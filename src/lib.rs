/// TODO: make state size aligned with chunk size of repCXL?
/// WARNING: currently assumes same memory layout and alignment across
/// all machines.
use log::{debug, error, info, warn};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, Instant};

mod algorithms;
mod safe_memio;
use safe_memio::ObjectMemoryEntry;
pub mod shmem;
mod timer;
pub mod utils;
pub mod request;
use request::{WriteRequest, ReadRequest, ReadReturn};
use shmem::object_index::ObjectInfo;
use shmem::{MemoryNode, SharedState};
pub mod config;
pub use config::RepCXLConfig;


/// The current membership of the group. Stores both the
/// processes and the memory nodes present in the system at a given time.
#[derive(Clone)]
pub struct GroupView {
    self_id: usize, // process ID of this instance
    pub processes: Vec<u32>,
    memory_nodes: Vec<MemoryNode>,
}

unsafe impl Send for GroupView {} // required because MemoryNode contains raw pointers
unsafe impl Sync for GroupView {}
impl GroupView {
    fn new(self_id: usize) -> Self {
        GroupView {
            self_id,
            processes: Vec::new(),
            memory_nodes: Vec::new(),
        }
    }

    fn add_process(&mut self, pid: u32) {
        if !self.processes.contains(&pid) {
            self.processes.push(pid);
        } else {
            info!("process {} already in group", pid);
        }
    }

    // Returns the process with the lowest ID as the coordinator
    fn get_coordinator(&self) -> Option<u32> {
        self.processes.iter().min().cloned()
    }

    // Returns the memory node with the lowest ID as the master node
    fn get_master_node(&self) -> Option<&MemoryNode> {
        self.memory_nodes.iter().min_by_key(|n| n.id)
    }
}
impl PartialEq for GroupView {
    fn eq(&self, other: &Self) -> bool {
        use std::collections::HashSet;

        let self_procs: HashSet<_> = self.processes.iter().collect();
        let other_procs: HashSet<_> = other.processes.iter().collect();

        let self_nodes: HashSet<_> = self.memory_nodes.iter().map(|n| n.id).collect();
        let other_nodes: HashSet<_> = other.memory_nodes.iter().map(|n| n.id).collect();

        self_procs == other_procs && self_nodes == other_nodes
    }
}
/// Shared replicated object across memory nodes
#[derive(Debug)]
pub struct RepCXLObject<T: Copy> {
    wreq_queue_tx: kanal::Sender<WriteRequest<T>>,
    rreq_queue_tx: kanal::Sender<ReadRequest<T>>,
    info: ObjectInfo,
}

impl<T: Copy> RepCXLObject<T> {
    const WRITE_TRACE_SAMPLE_RATE: u64 = 1024;

    pub fn new(
        id: usize,
        offset: usize,
        size: usize,
        wreq_queue_tx: kanal::Sender<WriteRequest<T>>,
        rreq_queue_tx: kanal::Sender<ReadRequest<T>>,
    ) -> Self {
        RepCXLObject {
            wreq_queue_tx,
            rreq_queue_tx,
            info: ObjectInfo::new(id, offset, size),
        }
    }

    pub fn write(&self, data: T) -> Result<(), String> {
        let client_start = Instant::now();
        let (ack_tx, ack_rx) = kanal::unbounded();
        let req = WriteRequest::new(self.info, data, ack_tx);
        let trace_id = req.trace_id;

        self.wreq_queue_tx
            .send(req)
            .map_err(|e| format!("Failed to send to object queue: {}", e))?;
        let send_to_worker = client_start.elapsed();
        let ack_wait_start = Instant::now();

        // std::thread::sleep(Duration::from_millis(10));
        // wait for ack
        let result = match ack_rx.recv() {
            Ok(true) => Ok(()),
            Ok(false) => Err("Failed write operation".into()),
            Err(e) => Err(format!("Failed to receive ack: {}", e)),
        };

        if trace_id % Self::WRITE_TRACE_SAMPLE_RATE == 0 {
            debug!(
                "[WRITE_TRACE][client] id={} send_to_worker={}ns ack_wait={}ns total={}ns",
                trace_id,
                send_to_worker.as_nanos(),
                ack_wait_start.elapsed().as_nanos(),
                client_start.elapsed().as_nanos(),
            );
        }

        result
    }

    pub fn read(&self) -> Result<ReadReturn<T>, String> {
        let (ack_tx, ack_rx) = kanal::unbounded();
        let req = ReadRequest::new(self.info, ack_tx);
        self.rreq_queue_tx
            .send(req)
            .map_err(|e| format!("Failed to send to object queue: {}", e))
            .unwrap();

        // wait for ack
        match ack_rx.recv() {
            Ok(return_val) => Ok(return_val),
            Err(e) => Err(format!("Failed to receive read ack: {}", e)),
        }
    }
}



/// Main RepCXL structure in local memory/cache for each process
/// current version only supports objects of type T
pub struct RepCXL<T> {
    pub config: RepCXLConfig,
    num_of_objects: usize,
    view: GroupView,
    wreq_queue_tx: kanal::Sender<WriteRequest<T>>,
    wreq_queue_rx: Option<kanal::Receiver<WriteRequest<T>>>,
    rreq_queue_tx: kanal::Sender<ReadRequest<T>>,
    rreq_queue_rx: Option<kanal::Receiver<ReadRequest<T>>>,
    start_instant: Instant,
    stop_flag: Arc<AtomicBool>,
    logger: Option<utils::ms_logger::MonsterStateLogger>,
}

impl<T: Send + Copy + PartialEq + std::fmt::Debug + 'static> RepCXL<T> {

    /// Create a new empty repCXL instance
    pub fn new(config: RepCXLConfig) -> Self {

        // config should be validated at arg parsing but paranoia
        if let Err(e) = config.validate() {
            panic!("Invalid configuration: {}", e);
        }
        // add processes to view
        let mut view = GroupView::new(config.id as usize);
        view.processes = config.processes.clone(); // add all processes to group view

        // open memory nodes
        for path in config.mem_nodes.iter() {
            let mnid = view.memory_nodes.len();
            let node = MemoryNode::from_file(mnid, path, config.mem_size);
            view.memory_nodes.push(node);
        }

        // init read and write request queues
        let (wtx, wrx) = kanal::unbounded();
        let (rtx, rrx) = kanal::unbounded();

        RepCXL {
            config,
            num_of_objects: 0,
            view,
            wreq_queue_tx: wtx,
            wreq_queue_rx: Some(wrx),
            rreq_queue_tx: rtx,
            rreq_queue_rx: Some(rrx),
            start_instant: std::time::Instant::now(),
            stop_flag: Arc::new(AtomicBool::new(false)),
            logger: None,
        }
    }

    /// Enable state logging to a file. Clears any existing log at the path.
    /// The algorithm thread will append state transitions to this file.
    pub fn enable_file_log(&mut self, path: &str) {
        let mut log = utils::ms_logger::MonsterStateLogger::new(path);
        log.clear();
        self.logger = Some(log);
    }

    pub fn register_process(&mut self, pid: u32) {
        self.view.add_process(pid);
    }

    pub fn is_coordinator(&self) -> bool {
        self.view.get_coordinator() == Some(self.config.id as u32)
    }

    pub fn add_memory_node_from_file(&mut self, path: &str) {
        let id = self.view.memory_nodes.len();
        let node = MemoryNode::from_file(id, path, self.config.mem_size);
        self.view.memory_nodes.push(node);
    }

    pub fn init_state(&mut self) {
        if !self.is_coordinator() {
            warn!("Only the coordinator should initialize state");
            return;
        }

        let state = SharedState::new(self.config.mem_size, self.config.chunk_size);

        // Write the shared state to each memory node
        for node in &self.view.memory_nodes {
            node.write_state(state);
        }
    }

    pub fn get_view(&self) -> GroupView {
        self.view.clone()
    }

    fn read_state_from_any(&self) -> Result<SharedState, &str> {
        for node in &self.view.memory_nodes {
            let state = node.read_state();
            return Ok(state);
        }
        Err("Could not read state from any memory node!")
    }

    // Get a mutable reference to the starting block from the master memory node
    fn get_state_from_master(&self) -> Result<&mut SharedState, &str> {
        if let Some(master) = self.view.get_master_node() {
            let state = master.get_state();
            return Ok(state);
        }
        Err("Could not read state from master node!")
    }

    pub fn dump_states(&mut self) {
        println!("#### state dump ####");
        for node in &self.view.memory_nodes {
            let state = node.read_state();
            println!("Memory node {}:\n{:?}", node.id, state);
        }
    }

    /// Attempts to create a new shared, replicated object of type T across
    /// all memory nodes.
    ///
    /// # Arguments
    /// * `id` - Unique identifier for the object.
    pub fn new_object(&mut self, id: usize) -> Option<RepCXLObject<T>> {
        if self.num_of_objects >= shmem::MAX_OBJECTS {
            warn!("Maximum number of objects reached");
            return None;
        }


        // TODO: do it more cleanly
        if id >= shmem::MAX_OBJECTS {
            warn!("Allowed IDs: 0-{}", shmem::MAX_OBJECTS - 1);
            return None;
        }

        if !self.is_coordinator() {
            warn!("Only the coordinator can create new objects");
            return None;
        }

        let size = std::mem::size_of::<ObjectMemoryEntry<T>>(); // padded and aligned

        let mut state = self.read_state_from_any().unwrap();

        // try to alloc object
        match state.object_index.alloc_object(id, size) {
            Some(offset) => {
                for node in &self.view.memory_nodes {
                    // write state to every memory node
                    node.write_state(state);
                }

                // clone the request queues
                let wtx = self.wreq_queue_tx.clone();
                let rtx = self.rreq_queue_tx.clone();
                // create the new RepCXLObject
                let obj = RepCXLObject::new(id, offset, size, wtx, rtx);

                self.num_of_objects += 1;
                return Some(obj);
            }
            None => {
                info!("Failed to allocate object with id {} of size {}", id, size);
                return None;
            }
        }
    }

    /// Creates a new object and initalizes it with a given value
    pub fn new_object_with_val(&mut self, id: usize, value: T) -> Option<RepCXLObject<T>> {
        if let Some(obj) = self.new_object(id) {
            
            // no write request ID for initialization
            let entry = ObjectMemoryEntry::new_nowid(value);

            // write to all memory nodes
            match safe_memio::mem_writeall(obj.info.offset, entry, &self.view.memory_nodes) {
                Ok(_) => Some(obj),
                Err(safe_memio::MemoryError(memory_node_id)) => {
                    error!("Failed to write object {} to memory node {}", id, memory_node_id);
                    None
                }
            }
        } else {
            None
        }
    }

    pub fn remove_object(&mut self, id: usize) {
        if !self.is_coordinator() {
            error!("Only the coordinator can remove objects");
            return;
        }

        let mut state = self.read_state_from_any().unwrap();
        state.object_index.dealloc_object(id);

        // Update the shared state in each memory node
        for node in &mut self.view.memory_nodes {
            node.write_state(state);
        }
    }

    /// Attempt to get an object reference by its ID first in the local cache
    /// and then in the shared state.
    pub fn get_object(&mut self, id: usize) -> Option<RepCXLObject<T>> {

        let state = self.read_state_from_any().unwrap();

        if let Some(oi) = state.object_index.lookup_object(id) {
            let obj = RepCXLObject::new(
                id,
                oi.offset,
                oi.size,
                self.wreq_queue_tx.clone(),
                self.rreq_queue_tx.clone(),
            );
            return Some(obj);
        }
        info!("Object {} not found in shared state", id);

        None
    }

    /// Use the config-specified write algorithm to write an object.
    /// Uses direct no-thread path when available, otherwise falls back to
    /// channel-based object write.
    pub fn write_object(&self, obj: &RepCXLObject<T>, data: T) -> Result<(), String> {
        match algorithms::write_nothread(&self.config.algorithm, &self.view, obj, data) {
            Ok(()) => Ok(()),
            Err(_) => obj.write(data),
        }
    }

    /// Use the config-specified read algorithm to read an object. 
    /// Retries the operation up to `config.read_retries` times if it returns a
    /// dirty read.
    pub fn read_object(&self, obj: &RepCXLObject<T>) -> Result<ReadReturn<T>, String> {
        
        // attempt best effort read first
        let mut res = algorithms::read_nothread(
                            &self.config.algorithm,
                            self.start_instant,
                            Duration::from_nanos(self.config.round_time),
                            self.config.read_offset,
                            &self.view, 
                            obj);

        // retry if dirty with offset time
        for _ in 0..self.config.read_retries{
            res = algorithms::read_nothread(
                            &self.config.algorithm,
                            self.start_instant,
                            Duration::from_nanos(self.config.round_time),
                            self.config.read_offset,
                            &self.view, 
                            obj); 
                
            match res {
                Ok(ReadReturn::ReadDirty(_)) => continue,
                _ => break,
            }
        }

        res
    }

    /// Start the repCXL protocol threads without initial synchronization (for async protocols)
    pub fn start(&mut self) {
        let algorithm = self.config.algorithm.clone();

        if self.config.pipeline {
            info!("Starting pipelined write thread for algorithm {}", algorithm);
        } else {
            info!("Starting non-pipelined write thread for algorithm {}", algorithm);
        }


        // pipeline mode uses threads and requests queues
        if self.config.pipeline {
            // for both read and write threads move the rx queue to the thread
            // and keep the tx queue in main state

            // WRITE thread
            let wactx = algorithms::WriteAlgorithmContext {
                group_view: self.view.clone(),
                start_instant: self.start_instant,
                round_time: Duration::from_nanos(self.config.round_time),
                req_queue: self.wreq_queue_rx.take().expect("Receiver already taken"),
                stop_flag: self.stop_flag.clone(),
                logger: self.logger.take(),
            };

            let core_affinity = self.config.core_affinity;
            std::thread::spawn(move || {
                if let Some(core) = core_affinity {
                        core_affinity::set_for_current(core_affinity::CoreId { id: core });
                }
                algorithms::write_thread(&algorithm, wactx);
            });

            // READ thread
            let algo = self.config.algorithm.clone();
            let ractx = algorithms::ReadAlgorithmContext {
                group_view: self.view.clone(),
                req_queue_rx: self.rreq_queue_rx.take().expect("Receiver already taken"),
                stop_flag: self.stop_flag.clone(),
            };

            std::thread::spawn(move || {
                // @TODO: pin thread for read?
                algorithms::read_thread(&algo, ractx);
            });
        }


    }

    /// Synchronize processes in the group and start repCXL rounds.
    /// **assumes sync'ed clocks**
    /// All processes must call this function with the same group view to
    /// ensure consistency.
    pub fn sync_start(&mut self) {
        if let Some(_coord) = self.view.get_coordinator() {
            let mstate = self.get_state_from_master().unwrap();
            let sblock = mstate.get_starting_block();
            let start_time;
            // mark self as ready
            sblock.mark_ready(self.config.id as usize);
            info!("Process {} ready and waiting to start", self.config.id);

            loop {
                if self.is_coordinator() {

                    // check if all processes are ready
                    if sblock.all_ready(self.view.processes.clone()) {
                        start_time = SystemTime::now() + Duration::from_nanos(self.config.startup_delay);
                        sblock.start_at(start_time);
                        info!("Rounds starting at {:?}", start_time);

                        break;
                    }
                } else if sblock.start_is_scheduled() {
                    start_time = sblock.get_start_time().unwrap();
                    info!(
                        "Process {} sees round starting time set to {:?}",
                        self.config.id, start_time
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
                debug!("Process {} waiting for start...", self.config.id);
            }

            let start_instant = timer::system_time_to_instant(start_time);
            self.start_instant = start_instant;

            timer::wait_start_time(start_instant, timer::ROUND_SLEEP_RATIO);

            self.start();

        } else {
            error!("FATAL: No coordinator found in group");
            return;
        }
    }


    // stop pipeline threads and exit process
    pub fn stop(&self) {
        info!("Stopping repCXL process {}. Goodbye...", self.config.id);
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

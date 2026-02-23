/// TODO: make state size aligned with chunk size of repCXL?
/// WARNING: currently assumes same memory layout and alignment across
/// all machines.
use log::{debug, error, info, warn};

use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

mod algorithms;
pub mod logger;
mod safe_memio;
mod shmem;
pub mod utils;
use shmem::object_index::ObjectInfo;
use shmem::{MemoryNode, SharedState};
pub mod config;
pub use config::RepCXLConfig;

// Limits
const MAX_PROCESSES: usize = 128; // Maximum number of processes
pub const MAX_OBJECTS: usize = 128; // Maximum number of objects


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
pub struct WriteRequest<T> {
    pub(crate) obj_info: ObjectInfo,
    pub data: T,
    pub ack_tx: mpsc::Sender<bool>,
}

impl<T> WriteRequest<T> {
    pub(crate) fn new(obj_info: ObjectInfo, data: T, ack_tx: mpsc::Sender<bool>) -> Self {
        WriteRequest {
            obj_info,
            data,
            ack_tx,
        }
    }

    pub(crate) fn to_tuple(self) -> (ObjectInfo, T, mpsc::Sender<bool>) {
        (self.obj_info, self.data, self.ack_tx)
    }
}

pub struct ReadRequest<T> {
    pub(crate) obj_info: ObjectInfo,
    pub ack_tx: mpsc::Sender<ReadReturn<T>>,
}

impl<T> ReadRequest<T> {
    pub(crate) fn new(obj_info: ObjectInfo, ack_tx: mpsc::Sender<ReadReturn<T>>) -> Self {
        ReadRequest { obj_info, ack_tx }
    }
}

#[derive(Debug)]
pub enum ReadReturn<T> {
    ReadSafe(T),
    ReadDirty(T),
}
/// Shared replicated object across memory nodes
#[derive(Debug)]
pub struct RepCXLObject<T: Copy> {
    wreq_queue_tx: mpsc::Sender<WriteRequest<T>>,
    rreq_queue_tx: mpsc::Sender<ReadRequest<T>>,
    info: ObjectInfo, // could also just store the offset
}

impl<T: Copy> RepCXLObject<T> {
    pub fn new(
        id: usize,
        offset: usize,
        size: usize,
        wreq_queue_tx: mpsc::Sender<WriteRequest<T>>,
        rreq_queue_tx: mpsc::Sender<ReadRequest<T>>,
    ) -> Self {
        RepCXLObject {
            wreq_queue_tx,
            rreq_queue_tx,
            info: ObjectInfo::new(id, offset, size),
        }
    }

    pub fn write(&self, data: T) -> Result<(), String> {
        let (ack_tx, ack_rx) = mpsc::channel();
        let req = WriteRequest::new(self.info, data, ack_tx);

        self.wreq_queue_tx
            .send(req)
            .map_err(|e| format!("Failed to send to object queue: {}", e))?;
        // std::thread::sleep(Duration::from_millis(10));
        // wait for ack
        match ack_rx.recv() {
            Ok(true) => Ok(()),
            Ok(false) => Err("Failed write operation".into()),
            Err(e) => Err(format!("Failed to receive ack: {}", e)),
        }
    }

    pub fn read(&self) -> Result<ReadReturn<T>, String> {
        let (ack_tx, ack_rx) = mpsc::channel();
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

/// RepCXL write request unique identifier. Stored next to every object
/// Comparison checks for largest round number and smallest process ID if
/// round numbers are equal.
#[derive(Debug, Clone, Copy)]
struct Wid {
    round_num: u64,
    process_id: usize,
}

impl PartialEq for Wid {
    fn eq(&self, other: &Self) -> bool {
        self.round_num == other.round_num && self.process_id == other.process_id
    }
}
impl Eq for Wid {}

impl PartialOrd for Wid {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Wid {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.round_num.cmp(&other.round_num) {
            std::cmp::Ordering::Greater => std::cmp::Ordering::Greater,
            std::cmp::Ordering::Less => std::cmp::Ordering::Less,
            std::cmp::Ordering::Equal => other.process_id.cmp(&self.process_id),
        }
    }
}

impl Hash for Wid {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.round_num.hash(state);
        self.process_id.hash(state);
    }
}

impl Wid {
    pub fn new(round_num: u64, process_id: usize) -> Self {
        Wid {
            round_num,
            process_id,
        }
    }
}

/// ObjectMemoryEntry. Stores the current write ID and the value of the object
/// in memory.
#[derive(Debug, Clone, Copy)]
struct ObjectMemoryEntry<T> {
    wid: Wid,
    value: T,
}

impl<T: Copy> ObjectMemoryEntry<T> {
    pub fn new(wid: Wid, value: T) -> Self {
        ObjectMemoryEntry { wid, value }
    }

    pub fn new_nowid(value: T) -> Self {
        ObjectMemoryEntry {
            wid: Wid::new(0, 0),
            value,
        }
    }
}

/// Main RepCXL structure in local memory/cache for each process
/// current version only supports objects of type T
pub struct RepCXL<T> {
    pub config: RepCXLConfig,
    num_of_objects: usize,
    view: GroupView,
    wreq_queue_tx: mpsc::Sender<WriteRequest<T>>,
    wreq_queue_rx: Option<mpsc::Receiver<WriteRequest<T>>>,
    rreq_queue_tx: mpsc::Sender<ReadRequest<T>>,
    rreq_queue_rx: Option<mpsc::Receiver<ReadRequest<T>>>,
    stop_flag: Arc<AtomicBool>,
    logger: Option<logger::Logger>,
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

        // init shared state
        let state = SharedState::new(config.mem_size, config.chunk_size);
        for node in &view.memory_nodes {
            node.write_state(state);
        }

        // init read and write request queues
        let (wtx, wrx) = mpsc::channel();
        let (rtx, rrx) = mpsc::channel();

        RepCXL {
            config,
            num_of_objects: 0,
            view,
            wreq_queue_tx: wtx,
            wreq_queue_rx: Some(wrx),
            rreq_queue_tx: rtx,
            rreq_queue_rx: Some(rrx),
            stop_flag: Arc::new(AtomicBool::new(false)),
            logger: None,
        }
    }


    /// Enable state logging to a file. Clears any existing log at the path.
    /// The algorithm thread will append state transitions to this file.
    pub fn enable_log(&mut self, path: &str) {
        let mut log = logger::Logger::new(path);
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
        if self.num_of_objects >= MAX_OBJECTS {
            warn!("Maximum number of objects reached");
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
        // info!(
        //     "Object with id {} not found in cache, looking in shared state",
        //     id
        // );

        let state = self.read_state_from_any().unwrap();

        if let Some(oi) = state.object_index.lookup_object(id) {
            info!("Object found in shared state");
            let obj = RepCXLObject::new(
                id,
                oi.offset,
                oi.size,
                self.wreq_queue_tx.clone(),
                self.rreq_queue_tx.clone(),
            );
            return Some(obj);
        }
        None
    }

    /// Synchronize processes in the group and start repCXL rounds.
    /// **assumes sync'ed clocks**
    /// All processes must call this function with the same group view to
    /// ensure consistency.
    pub fn sync_start(&mut self, algorithm: String, rt: Duration) {
        if let Some(_coord) = self.view.get_coordinator() {
            let mstate = self.get_state_from_master().unwrap();
            let sblock = mstate.get_starting_block();
            let start_time;
            // mark self as ready
            sblock.mark_ready(self.config.id as usize);
            info!("Process {} marked as ready.", self.config.id);

            loop {
                if self.is_coordinator() {
                    // info!("Process {} is the coordinator", self.id);

                    // check if all processes are ready
                    if sblock.all_ready(self.view.processes.clone()) {
                        start_time = std::time::SystemTime::now() + Duration::from_nanos(self.config.startup_delay);
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

            let v = self.view.clone();
            // let rt = self.round_time;

            // for both read and write threads move the rx queue to the thread
            // and keep the tx queue in state to assign to new objects

            // WRITE thread
            {
                let (algorithm, v, stop) = (algorithm.clone(), v.clone(), self.stop_flag.clone());
                let logger = self.logger.take();
                let rx = self.wreq_queue_rx.take().expect("Receiver already taken");
                std::thread::spawn(move || {
                    algorithms::get_write_algorithm(algorithm)(v, start_time, rt, rx, stop, logger);
                });
            }

            // READ thread
            {
                let stop = self.stop_flag.clone();
                let rx = self.rreq_queue_rx.take().expect("Receiver already taken");
                std::thread::spawn(move || {
                    algorithms::get_read_algorithm(algorithm)(v, start_time, rt, rx, stop);
                });
            }

            // block until after start time
            // std::thread::sleep(Duration::from_secs(2));
        } else {
            error!("FATAL: No coordinator found in group");
            return;
        }
    }

    pub fn stop(&self) {
        info!("Stopping repCXL process {}. Goodbye...", self.config.id);
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

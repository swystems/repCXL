/// TODO: make state size aligned with chunk size of repCXL?
/// WARNING: currently assumes same memory layout and alignment across
/// all machines.
use log::{debug, error, info, warn};

use std::hash::Hash;
use std::sync::mpsc;
use std::time::Duration;

mod algorithms;
mod safe_memio;
mod shmem;
use shmem::object_index::ObjectInfo;
use shmem::{MemoryNode, SharedState};

const MAX_PROCESSES: usize = 128; // Maximum number of processes
const MAX_OBJECTS: usize = 128; // Maximum number of objects

/// The current membership of the group. Stores both the
/// processes and the memory nodes present in the system at a given time.
#[derive(Clone)]
struct GroupView {
    self_id: usize, // process ID of this instance
    processes: Vec<usize>,
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

    fn add_process(&mut self, pid: usize) {
        if !self.processes.contains(&pid) {
            self.processes.push(pid);
        } else {
            info!("process {} already in group", pid);
        }
    }

    // Returns the process with the lowest ID as the coordinator
    fn get_coordinator(&self) -> Option<usize> {
        self.processes.iter().min().cloned()
    }

    // Returns the memory node with the lowest ID as the master node
    fn get_master_node(&self) -> Option<&MemoryNode> {
        self.memory_nodes.iter().min_by_key(|n| n.id)
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

/// Shared replicated object across memory nodes
#[derive(Debug)]
pub struct RepCXLObject<T: Copy> {
    req_queue_tx: mpsc::Sender<WriteRequest<T>>,
    info: ObjectInfo, // could also just store the offset
}

impl<T: Copy> RepCXLObject<T> {
    pub fn new(
        id: usize,
        offset: usize,
        size: usize,
        req_queue_tx: mpsc::Sender<WriteRequest<T>>,
    ) -> Self {
        RepCXLObject {
            req_queue_tx,
            info: ObjectInfo::new(id, offset, size),
        }
    }

    pub fn write(&self, data: T) -> Result<(), String> {
        let (ack_tx, ack_rx) = mpsc::channel();
        let req = WriteRequest::new(self.info, data, ack_tx);ObjectMemoryEntry

        self.req_queue_tx
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
    pub id: usize,
    pub size: usize,
    chunk_size: usize, // Size of each chunk in bytes
    num_of_objects: usize,
    view: GroupView,
    round_time: Duration,
    req_queue_tx: mpsc::Sender<WriteRequest<T>>,
    req_queue_rx: Option<mpsc::Receiver<WriteRequest<T>>>,
}

impl<T: Send + Copy + PartialEq + std::fmt::Debug + 'static> RepCXL<T> {
    pub fn new(id: usize, size: usize, chunk_size: usize, round_time: Duration) -> Self {
        let chunks = (size + chunk_size - 1) / chunk_size;
        let total_size = chunks * chunk_size;

        let mut view = GroupView::new(id);
        if id >= MAX_PROCESSES {
            panic!("Process ID must be between 0 and {}", MAX_PROCESSES - 1);
        }
        view.processes.push(id); // add self to group

        // dummy channel for init
        let (tx, rx) = mpsc::channel();

        RepCXL {
            id,
            size: total_size,
            chunk_size,
            num_of_objects: 0,
            view,
            round_time,
            req_queue_tx: tx,
            req_queue_rx: Some(rx),
        }
    }

    pub fn register_process(&mut self, pid: usize) {
        self.view.add_process(pid);
    }

    pub fn is_coordinator(&mut self) -> bool {
        self.view.get_coordinator() == Some(self.id)
    }

    pub fn add_memory_node_from_file(&mut self, path: &str) {
        let id = self.view.memory_nodes.len();
        let node = MemoryNode::from_file(id, path, self.size);
        self.view.memory_nodes.push(node);
    }

    pub fn init_state(&mut self) {
        let state = SharedState::new(self.size, self.chunk_size);

        // Write the shared state to each memory node
        for node in &self.view.memory_nodes {
            node.write_state(state);
        }
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

                // clone the object transmission queue
                let tx = self.req_queue_tx.clone();
                // create the new RepCXLObject
                let obj = RepCXLObject::new(id, offset, size, tx);

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
            let obj = RepCXLObject::new(id, oi.offset, oi.size, self.req_queue_tx.clone());
            return Some(obj);
        }
        None
    }

    /// Synchronize processes in the group and start repCXL rounds.
    /// **assumes sync'ed clocks**
    /// All processes must call this function with the same group view to
    /// ensure consistency.
    pub fn sync_start(&mut self, algorithm: String) {
        if let Some(coord) = self.view.get_coordinator() {
            let mstate = self.get_state_from_master().unwrap();
            let sblock = mstate.get_starting_block();
            let start_time;
            // mark self as ready
            sblock.mark_ready(self.id);
            info!("Process {} marked as ready.", self.id);

            loop {
                if coord == self.id {
                    // info!("Process {} is the coordinator", self.id);

                    // check if all processes are ready
                    if sblock.all_ready(self.view.processes.clone()) {
                        start_time = std::time::SystemTime::now() + Duration::from_secs(1);
                        sblock.start_at(start_time);
                        info!("Rounds starting at {:?}", start_time);

                        break;
                    }
                } else if sblock.start_is_scheduled() {
                    start_time = sblock.get_start_time().unwrap();
                    info!(
                        "Process {} sees round starting time set to {:?}",
                        self.id, start_time
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
                debug!("Process {} waiting for start...", self.id);
            }

            let v = self.view.clone();
            let rt = self.round_time;

            // create object queue channel: move the receiver to the thread
            // and keep the sender in state to assign to new objects
            // let (tx, rx) = mpsc::channel();
            let rx = self.req_queue_rx.take().expect("Receiver already taken");
            std::thread::spawn(move || {
                algorithms::from_string(algorithm)(v, start_time, rt, rx);
            });

            // block until after start time
            std::thread::sleep(Duration::from_secs(2));
        } else {
            error!("FATAL: No coordinator found in group");
            return;
        }
    }
}

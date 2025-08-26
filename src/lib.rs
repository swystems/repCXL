/// TODO: make state size aligned with chunk size of repCXL?
/// WARNING: currently assumes same memory layout and alignment across
/// all machines.
use libc::{c_int, c_void, mmap, munmap, MAP_SHARED, PROT_READ, PROT_WRITE};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::time::Duration;

const MAX_PROCESSES: usize = 128; // Maximum number of processes
const MAX_OBJECTS: usize = 32; // Maximum number of objects
const STATE_SIZE: usize = std::mem::size_of::<SharedState>();

#[link(name = "numa")]
extern "C" {
    pub fn numa_alloc_onnode(size: usize, node: c_int) -> *mut c_void;
    pub fn numa_free(mem: *mut c_void, size: usize);
}

/// Returns the system time in Duration since UNIX EPOCH
fn systime() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("Instant before UNIX EPOCH!")
}

#[derive(Debug, Clone, Copy)]
struct ObjectEntry {
    id: usize,
    offset: usize,
    size: usize,
}

impl ObjectEntry {
    fn new(id: usize, offset: usize, size: usize) -> Self {
        ObjectEntry { id, offset, size }
    }
}

/// Shared fixed-size array indexed by process ID
#[derive(Debug, Clone, Copy)]
struct StartingBlock {
    start_time: Duration,
    ready_processes: [bool; MAX_PROCESSES],
}

/// Memory allocation information. Process coordinator has write acess
/// while replicas have read-only access.
///
/// @TODO: add coordinator-only write checks
#[derive(Copy, Clone, Debug)]
struct AllocationInfo {
    total_size: usize,
    allocated_size: usize,
    chunk_size: usize,
    object_index: [Option<ObjectEntry>; MAX_OBJECTS],
}

impl AllocationInfo {
    fn new(total_size: usize, chunk_size: usize) -> Self {
        AllocationInfo {
            total_size,
            allocated_size: 0,
            chunk_size,
            object_index: [None; MAX_OBJECTS], // Initialize with None
        }
    }

    /// Get the object entry in from the index by its id.
    /// Returns Some<offset> if found, None otherwise.
    /// # Arguments
    /// * `id` - Unique identifier for the object.
    fn lookup_object(&self, id: usize) -> Option<ObjectEntry> {
        for entry in self.object_index {
            if let Some(obj) = entry {
                if obj.id == id {
                    return entry;
                }
            }
        }
        None
    }

    /// Allocates an object in the first free slot (first fit allocation) in the shared object index
    /// and returns Some<offset> if a suitable slot is found, otherwise None.
    ///
    /// @TODO: better allocation algorithm
    ///
    /// # Arguments
    /// * 'id' - Unique identifier for the object.
    /// * `size` - Size of the memory to allocate.
    fn alloc_object(&mut self, id: usize, size: usize) -> Option<usize> {
        let chunks = (size + self.chunk_size - 1) / self.chunk_size; // Round up to nearest chunk size
        let size = chunks * self.chunk_size;

        if self.allocated_size + size > self.total_size {
            warn!("Not enough space");
            return None;
        }

        if self.lookup_object(id).is_some() {
            info!("Object with id {} already exists", id);
            return None;
        }

        // bad allocation algorithm
        // loses space when a smaller object takes the place of a larger one which was freed
        for i in 0..MAX_OBJECTS {
            let entry = self.object_index[i];
            if entry.is_none() {
                let start = if i == 0 {
                    0
                } else {
                    self.object_index[i - 1]
                        .map(|e| e.offset as usize + e.size)
                        .expect("Previous entry should exist")
                };
                let end = if i == MAX_OBJECTS - 1 {
                    self.total_size
                } else {
                    self.object_index[i + 1]
                        .map(|e| e.offset as usize)
                        .unwrap_or(self.total_size)
                };
                if start + size <= end {
                    self.object_index[i] = Some(ObjectEntry::new(id, start, size));
                    self.allocated_size += size;
                    return Some(start);
                }
            }
        }
        warn!("Failed allocation: no free slot available");
        None
    }

    /// Removes an object from the state by its id
    fn dealloc_object(&mut self, id: usize) {
        self.object_index.iter_mut().for_each(|entry| {
            if let Some(obj) = entry {
                if obj.id == id {
                    self.allocated_size -= obj.size;
                    *entry = None; // Mark as free
                }
            }
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct SharedState {
    alloc_info: AllocationInfo,
    starting_block: StartingBlock,
}

impl SharedState {
    fn new(total_size: usize, chunk_size: usize) -> Self {
        SharedState {
            alloc_info: AllocationInfo::new(total_size, chunk_size),
            starting_block: StartingBlock {
                start_time: Duration::from_nanos(0),
                ready_processes: [false; MAX_PROCESSES],
            },
        }
    }
}

#[derive(PartialEq, Eq, Debug, Hash)]
enum MemoryType {
    Numa,
    File,
}

#[derive(PartialEq, Eq, Hash)]
pub struct MemoryNode {
    id: usize,
    type_: MemoryType,
    state_addr: *mut SharedState,
    addr: *mut u8,
    size: usize,
}

impl MemoryNode {
    // Create a MemoryNode from a file in tmpfs
    // Processes/VMs on same host will share the memory region, not guaranteed
    // across different hosts
    // assumes all processes/VMs use the same file path
    fn from_file(id: usize, path: &str, size: usize) -> Self {
        if size <= STATE_SIZE {
            panic!("Size must be greater than AllocationInfo size");
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("Failed to open shared memory");

        let ptr = unsafe {
            mmap(
                std::ptr::null_mut(),
                size,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            panic!(
                "Failed to mmap {}. Error: {}",
                path,
                std::io::Error::last_os_error()
            );
        }

        let ptr = ptr as *mut u8;

        MemoryNode {
            id,
            type_: MemoryType::File,
            state_addr: ptr as *mut SharedState,
            addr: unsafe { ptr.offset(STATE_SIZE as isize) },
            size,
        }
    }

    /// WARNING: placeholder only. memory is not shared, every node will its own memory region
    fn from_numa(id: usize, size: usize, numa_node: i32) -> Self {
        let ptr = unsafe { numa_alloc_onnode(size, numa_node) };
        if ptr.is_null() {
            panic!("numa_alloc_onnode failed");
        }
        let ptr = ptr as *mut u8;

        MemoryNode {
            id,
            type_: MemoryType::Numa,
            state_addr: ptr as *mut SharedState,
            addr: unsafe { ptr.offset(STATE_SIZE as isize) },
            size,
        }
    }

    fn addr_at(&self, offset: usize) -> *mut u8 {
        if offset >= self.size {
            panic!("Offset out of bounds");
        }
        unsafe { self.addr.offset(offset as isize) }
    }

    // copy of the shared state (which remains unchanged)
    fn read_state(&self) -> SharedState {
        unsafe { std::ptr::read(self.state_addr) } // WARNING: might want to read_unaligned
    }

    // mutable reference to the shared state
    fn get_state(&self) -> &mut SharedState {
        unsafe { &mut *self.state_addr }
    }

    fn write_state(&self, state: SharedState) {
        unsafe {
            std::ptr::write(self.state_addr, state); // WARNING: might want to write_unaligned
        }
    }
}

impl Drop for MemoryNode {
    fn drop(&mut self) {
        if self.type_ == MemoryType::Numa {
            unsafe {
                numa_free(self.addr as *mut c_void, self.size);
            }
        } else if self.type_ == MemoryType::File {
            unsafe {
                munmap(self.addr as *mut libc::c_void, self.size);
            }
            // File is automatically closed when it goes out of scope
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::remove_file;

    #[test]
    fn test_memory_node_from_file() {
        let mnid = 1;
        let path = "/dev/shm/repCXL_test";
        let size = 4096; // 1 KiB

        // Create and open the file with read/write permissions
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .expect("Failed to create/open file in tmpfs");

        // Resize the file to 4096 bytes (one page)
        file.set_len(4096).expect("Failed to set file length");

        let node = MemoryNode::from_file(mnid, path, size);
        assert_eq!(node.id, mnid);
        assert_eq!(node.type_, MemoryType::File);
        assert!(!node.addr.is_null());
        assert_eq!(node.size, size); // 1 KiB

        // Clean up: remove the tmpfs file
        remove_file(path).expect("Failed to remove tmpfs file");
    }

    #[test]
    fn test_memory_node_from_numa() {
        let mnid = 0;
        let size = 1024; // 1 KiB
        let numa_node = 0; // Node 0 should exist on most systems

        let node = MemoryNode::from_numa(mnid, size, numa_node);

        unsafe {
            *node.addr = 31;
            // Initialize the shared memory region to zero
            std::ptr::write_bytes(node.addr, 4, size);
        }

        assert_eq!(node.id, mnid);
        assert_eq!(node.type_, MemoryType::Numa);
        assert!(!node.addr.is_null());

        assert_eq!(node.size, size); // 1 KiB
    }
}

/// The current membership of the group. Stores both the
/// processes and the memory nodes present in the system at a given time.
struct GroupView {
    processes: Vec<usize>,
    memory_nodes: Vec<MemoryNode>,
}

impl GroupView {
    fn new() -> Self {
        GroupView {
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

/// Shared replicated object across memory nodes
pub struct RepCXLObject {
    pub id: usize,
    pub size: usize,
    addresses: HashMap<usize, *mut u8>, // MemoryNode id-> address in that node
}

/// Main RepCXL structure in local memory/cache for each process
pub struct RepCXL {
    pub id: usize,
    pub size: usize,
    chunk_size: usize, // Size of each chunk in bytes
    view: GroupView,
    objects: HashMap<usize, RepCXLObject>, // id -> object
}

impl RepCXL {
    pub fn new(id: usize, size: usize, chunk_size: usize) -> Self {
        let chunks = (size + chunk_size - 1) / chunk_size;
        let total_size = chunks * chunk_size;

        let mut view = GroupView::new();
        if id >= MAX_PROCESSES {
            panic!("Process ID must be between 0 and {}", MAX_PROCESSES - 1);
        }
        view.processes.push(id); // add self to group

        RepCXL {
            id,
            size: total_size,
            chunk_size,
            view,
            objects: HashMap::new(),
        }
    }

    pub fn add_process_to_group(&mut self, pid: usize) {
        self.view.add_process(pid);
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

    // fn read_state_from_master(&self) -> Result<SharedState, &str> {
    //     if let Some(master) = self.view.get_master_node() {
    //         master.read_state();
    //         return Ok(master.read_state());
    //     }
    //     Err("Could not read state from master node!")
    // }

    // fn write_state_to_master(&self, state: SharedState) {
    //     if let Some(master) = self.view.get_master_node() {
    //         master.write_state(state);
    //     } else {
    //         error!("Could not write state to master node!");
    //     }
    // }

    // Get a mutable reference to the starting block from the master node
    fn get_starting_block(&self) -> Result<&mut StartingBlock, &str> {
        if let Some(master) = self.view.get_master_node() {
            let state = master.get_state();
            return Ok(&mut state.starting_block);
        }
        Err("Could not read starting block from master node!")
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
    pub fn new_object<T>(&mut self, id: usize) -> Option<&RepCXLObject> {
        let mut addresses = HashMap::new();
        let size = std::mem::size_of::<T>(); // padded and aligned

        let mut state = self.read_state_from_any().unwrap();

        match state.alloc_info.alloc_object(id, size) {
            Some(offset) => {
                // Allocate memory in each memory node
                for node in &self.view.memory_nodes {
                    let addr = node.addr_at(offset);
                    addresses.insert(node.id, addr);

                    // write state to every memory node
                    node.write_state(state);
                }
            }
            None => {
                info!("Failed to allocate object with id {} of size {}", id, size);
                return None;
            }
        }

        self.objects.insert(
            id,
            RepCXLObject {
                id,
                addresses,
                size,
            },
        );

        self.objects.get(&id)
    }

    pub fn remove_object(&mut self, id: usize) {
        let mut state = self.read_state_from_any().unwrap();
        state.alloc_info.dealloc_object(id);

        // Update the shared state in each memory node
        for node in &mut self.view.memory_nodes {
            node.write_state(state);
        }
    }

    /// Attempt to get an object reference by its ID first in the local cache
    /// and then in the shared state.
    pub fn get_object(&mut self, id: usize) -> Option<&RepCXLObject> {
        if self.objects.contains_key(&id) {
            debug!("object found in repcxl local cache");
            return self.objects.get(&id);
        }

        info!(
            "Object with id {} not found in cache, looking in shared state",
            id
        );

        let state = self.read_state_from_any().unwrap();

        if let Some(oe) = state.alloc_info.lookup_object(id) {
            info!("Object found in shared state");
            let mut addresses = HashMap::new();
            for node in &self.view.memory_nodes {
                let addr = node.addr_at(oe.offset);
                addresses.insert(node.id, addr);
            }

            self.objects.insert(
                id,
                RepCXLObject {
                    id,
                    addresses,
                    size: oe.size,
                },
            );

            return self.objects.get(&id);
        }
        None
    }

    /// Synchronize processes in the group and start repCXL rounds.
    /// **assumes sync'ed clocks**
    /// All processes must call this function with the same group view to
    /// ensure consistency.
    pub fn sync_start(&self) {
        if let Some(coord) = self.view.get_coordinator() {
            let sblock = self.get_starting_block().unwrap();

            // mark self as ready
            sblock.ready_processes[self.id] = true;
            info!("Process {} marked as ready.", self.id);

            loop {
                if coord == self.id {
                    // info!("Process {} is the coordinator", self.id);

                    // check if all processes are ready
                    if self
                        .view
                        .processes
                        .iter()
                        .all(|&pid| sblock.ready_processes[pid])
                    {
                        let start_time = systime() + Duration::from_secs(2);
                        sblock.start_time = start_time;
                        info!("Rounds starting at {:?}", start_time);
                        break;
                    }
                    // break; //temp
                } else if sblock.start_time != Duration::from_nanos(0) {
                    info!(
                        "Process {} sees round starting time set to {:?}",
                        self.id, sblock.start_time
                    );
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
                debug!("Process {} waiting for start...", self.id);
            }
        } else {
            error!("FATAL: No coordinator found in group");
            return;
        }
    }
}

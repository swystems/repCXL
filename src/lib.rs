/// TODO: make state size aligned with chunk size of repCXL?
/// WARNING: currently assumes same memory layout and alignment across
/// all machines.
use libc::{c_int, c_void, mmap, munmap, MAP_SHARED, PROT_READ, PROT_WRITE};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

const MAX_OBJECTS: usize = 32; // Maximum number of objects
const STATE_SIZE: usize = std::mem::size_of::<SharedState>();

#[link(name = "numa")]
extern "C" {
    pub fn numa_alloc_onnode(size: usize, node: c_int) -> *mut c_void;
    pub fn numa_free(mem: *mut c_void, size: usize);
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
/// Shared replicated state across memory nodes.
#[derive(Copy, Clone, Debug)]
struct SharedState {
    total_size: usize,
    allocated_size: usize,
    chunk_size: usize,
    lock: bool, // Mutex for exclusive write
    object_index: [Option<ObjectEntry>; MAX_OBJECTS],
}

impl SharedState {
    fn new(total_size: usize, chunk_size: usize) -> Self {
        SharedState {
            total_size,
            allocated_size: 0,
            chunk_size,
            lock: false,                       // not used yet
            object_index: [None; MAX_OBJECTS], // Initialize with None
        }
    }

    fn lock(&mut self) {
        while self.lock {
            // Busy wait until the lock is released
        }
        self.lock = true; // Acquire the lock
    }

    fn unlock(&mut self) {
        self.lock = false; // Release the lock
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
            println!("Not enough space");
            return None;
        }

        if self.lookup_object(id).is_some() {
            println!("Object with id {} already exists", id);
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

#[derive(PartialEq, Eq, Debug, Hash)]
enum MemoryType {
    Numa,
    File,
}

#[derive(PartialEq, Eq, Hash)]
pub struct MemoryNode {
    id: usize,
    type_: MemoryType,
    // Pointer to the shared state in this memory node (start of the memory region)
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
            panic!("Size must be greater than SharedState size");
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

/// Shared replicated object across memory nodes
pub struct RepCXLObject {
    pub id: usize,
    pub size: usize,
    addresses: HashMap<usize, *mut u8>, // MemoryNode id-> address in that node
}

pub struct RepCXL {
    pub size: usize,
    chunk_size: usize, // Size of each chunk in bytes
    memory_nodes: Vec<MemoryNode>,
    objects: HashMap<usize, RepCXLObject>, // id -> object
}

impl RepCXL {
    pub fn new(size: usize, chunk_size: usize) -> Self {
        let chunks = (size + chunk_size - 1) / chunk_size;
        let total_size = chunks * chunk_size;

        RepCXL {
            size: total_size,
            chunk_size,
            memory_nodes: Vec::new(),
            objects: HashMap::new(),
        }
    }

    pub fn add_memory_node_from_file(&mut self, path: &str) {
        let id = self.memory_nodes.len();
        let node = MemoryNode::from_file(id, path, self.size);

        self.memory_nodes.push(node);
    }

    pub fn init_state(&mut self) {
        let state = SharedState::new(self.size, self.chunk_size);

        // Write the shared state to each memory node
        for node in &self.memory_nodes {
            unsafe {
                std::ptr::write(node.state_addr, state); // WARNING: might want to write_unaligned
            }
        }
    }

    fn read_state_from_any(&self) -> Result<SharedState, &str> {
        for node in &self.memory_nodes {
            unsafe {
                let state = std::ptr::read(node.state_addr);
                return Ok(state);
            }
        }
        Err("Could not read state from any memory node!")
    }

    pub fn dump_states(&mut self) {
        println!("#### state dump ####");
        for node in &self.memory_nodes {
            unsafe {
                let state = std::ptr::read(node.state_addr);
                println!("Memory node {}:\n{:?}", node.id, state);
            }
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

        match state.alloc_object(id, size) {
            Some(offset) => {
                // Allocate memory in each memory node
                for node in &self.memory_nodes {
                    let addr = node.addr_at(offset);
                    addresses.insert(node.id, addr);

                    // write state to every memory node
                    unsafe {
                        std::ptr::write(node.state_addr, state);
                    }
                }
            }
            None => {
                println!("Failed to allocate object with id {} of size {}", id, size);
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
        state.dealloc_object(id);

        // Update the shared state in each memory node
        for node in &mut self.memory_nodes {
            unsafe {
                std::ptr::write(node.state_addr, state);
            }
        }
    }

    /// Attempt to get an object reference by its ID first in the local cache
    /// and then in the shared state.
    pub fn get_object(&mut self, id: usize) -> Option<&RepCXLObject> {
        if self.objects.contains_key(&id) {
            println!("object found in repcxl local cache");
            return self.objects.get(&id);
        }

        println!("Object with id {} not found in cache, looking in shared state", id);

        let state = self.read_state_from_any().unwrap();
        
        if let Some(oe) = state.lookup_object(id) {
            println!("object found in shared state");
            let mut addresses = HashMap::new();
            for node in &self.memory_nodes {
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
}

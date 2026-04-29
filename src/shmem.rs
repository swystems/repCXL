use libc::{mmap, munmap, MAP_SHARED, PROT_READ, PROT_WRITE};
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;



// currently not used, requires the libnuma
// mod numa_mem_node;

pub mod object_index;
use object_index::ObjectIndex;
pub mod starting_block;
use starting_block::StartingBlock;
pub mod wcc;
use wcc::{ObjectWCC, FastWCC};
pub mod log;
use log::Log;


pub const MAX_OBJECTS: usize = 1024; // Maximum number of objects
pub const MAX_PROCESSES: usize = 512; // Maximum number of processes


pub fn mmap_daxdev(path: &str, size: usize) -> *mut u8 {
    let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_SYNC) // avoid page cache effects
            .open(path)
            .expect("Failed to open shared memory. Does the file exist?");

        /* DAX mapping requires a 2MiB alignment */
        let page = 2 * 1024 * 1024;
        if size < page {
            panic!("Size must be at least 2 MiB for DAX mapping");
        }

        let page_aligned_size = (size / page) * page + page;

        let ptr = unsafe {
            mmap(
                std::ptr::null_mut(),
                page_aligned_size,
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

        ptr as *mut u8
} 

#[derive(Debug)]
pub(crate) struct SharedState<T> {
    pub(crate) object_index: ObjectIndex,
    starting_block: StartingBlock,
    owcc: ObjectWCC,
    fwcc: FastWCC,
    log: Log<T>,
}

impl<T: Copy> SharedState<T> {
    pub(crate) fn new(total_size: usize) -> Self {
        SharedState {
            object_index: ObjectIndex::new(total_size),
            starting_block: StartingBlock::new(),
            owcc: ObjectWCC::new(),
            fwcc: FastWCC::new(),
            log: Log::new(),
        }
    }

    pub(crate) fn get_oi(&mut self) -> &mut ObjectIndex {
        &mut self.object_index
    }

    pub(crate) fn get_starting_block(&mut self) -> &mut StartingBlock {
        &mut self.starting_block
    }

    pub(crate) fn get_owcc(&mut self) -> &mut ObjectWCC {
        &mut self.owcc
    }

    pub(crate) fn get_fwcc(&mut self) -> &mut FastWCC  {
        &mut self.fwcc
    }

    pub(crate) fn get_log(&mut self) -> &mut Log<T> {
        &mut self.log
    }
}


#[derive(Hash, Clone)]
pub(crate) struct MemoryNode<T> {
    pub id: usize,
    state_addr: *mut SharedState<T>,
    obj_addr: *mut u8,
    size: usize,
}

// unsafe impl<T> Send for MemoryNode<T> {} // raw pointers are safe to send across threads
// unsafe impl<T> Sync for MemoryNode<T> {} // raw pointers are safe to share across threads

impl<T> MemoryNode<T> {
    // Create a MemoryNode from a file in tmpfs mapped to a CXL node or from
    // a CXL DAX device (e.g., /dev/dax0.0)
    // Processes/VMs on same host will share the memory region, not guaranteed
    // across different hosts
    // assumes all processes/VMs use the same file path
    pub(crate) fn from_file(id: usize, path: &str, size: usize) -> Self {
        if size <= std::mem::size_of::<SharedState<T>>() {
            panic!("Size must be greater than SharedState size: {}. Breakdown: \
            \n\tObjectIndex: {}\
            \n\tstarting_block: {}\
            \n\towcc: {}\
            \n\tfwcc: {}\
            \n\tlog: {}", 
                std::mem::size_of::<SharedState<T>>(), 
                std::mem::size_of::<ObjectIndex>(), 
                std::mem::size_of::<StartingBlock>(), 
                std::mem::size_of::<ObjectWCC>(),
                std::mem::size_of::<FastWCC>(),
                std::mem::size_of::<Log<T>>()
            );
        }

        let ptr = mmap_daxdev(path, size);

        // ensure the object area starts at a 64B aligned address after the state
        let offset_64aligned = std::mem::size_of::<SharedState<T>>() / 64 * 64 + 64; 
        let obj_addr = unsafe { ptr.offset(offset_64aligned as isize) };

        MemoryNode {
            id,
            state_addr: ptr as *mut SharedState<T>,
            obj_addr,
            size,
        }
    }

    pub(crate) fn addr_at(&self, offset: usize) -> *mut u8 {
        if offset >= self.size {
            panic!("Offset out of bounds");
        }
        unsafe { self.obj_addr.offset(offset as isize) }
    }

    // Heap-allocated copy of the shared state to avoid large stack allocations.
    // pub(crate) fn read_state_boxed(&self) -> Box<SharedState<T>> {
    //     let mut state = Box::new(std::mem::MaybeUninit::<SharedState<T>>::uninit());
    //     unsafe {
    //         std::ptr::copy_nonoverlapping(
    //             self.state_addr,
    //             state.as_mut_ptr() as *mut SharedState<T>,
    //             1,
    //         );
    //         state.assume_init()
    //     }
    // }

    // mutable reference to the shared state
    pub(crate) fn get_state(&self) -> &mut SharedState<T>    {
        unsafe { &mut *self.state_addr }
    }

    pub(crate) fn write_state(&self, state: &SharedState<T>) {
        unsafe {
            std::ptr::copy_nonoverlapping(state as *const SharedState<T>, self.state_addr, 1);
        }
    }
}

impl<T> Drop for MemoryNode<T> {
    fn drop(&mut self) {
        unsafe {
            munmap(self.state_addr as *mut libc::c_void, self.size);
        }
        // File is automatically closed when it goes out of scope
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
        let size: usize = 10 * 1024 * 1024; // 1 MiB

        // Create and open the file with read/write permissions
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .expect("Failed to create/open file in tmpfs");

        file.set_len(size as u64).expect("Failed to set file length");

        let node = MemoryNode::<u32>::from_file(mnid, path, size);
        assert_eq!(node.id, mnid);
        assert!(!node.obj_addr.is_null());
        assert_eq!(node.size, size);

        // Clean up: remove the tmpfs file
        remove_file(path).expect("Failed to remove tmpfs file");
    }

}

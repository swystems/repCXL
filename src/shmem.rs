use libc::{mmap, munmap, MAP_SHARED, PROT_READ, PROT_WRITE};
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;

// currently not used, requires the libnuma
// mod numa_mem_node;

pub mod allocator;
use allocator::Allocator;
mod starting_block;
use starting_block::StartingBlock;
pub mod wcc;
use wcc::WriteConflictChecker;

const STATE_SIZE: usize = std::mem::size_of::<SharedState>();



#[derive(Debug, Clone, Copy)]
pub(crate) struct SharedState {
    pub(crate) allocator: Allocator,
    starting_block: StartingBlock,
    wcc: WriteConflictChecker,
}

impl SharedState {
    pub(crate) fn new(total_size: usize, chunk_size: usize) -> Self {
        SharedState {
            allocator: Allocator::new(total_size, chunk_size),
            starting_block: StartingBlock::new(),
            wcc: WriteConflictChecker::new(),
        }
    }

    pub(crate) fn get_starting_block(&mut self) -> &mut StartingBlock {
        &mut self.starting_block
    }

    pub(crate) fn get_wcc(&mut self) -> &mut WriteConflictChecker {
        &mut self.wcc
    }
}


// @TODO: add type for addr since repcxl is currently type-specific?
#[derive(PartialEq, Eq, Hash, Clone)]
pub(crate) struct MemoryNode {
    pub id: usize,
    state_addr: *mut SharedState,
    obj_addr: *mut u8,
    size: usize,
}

impl MemoryNode {
    // Create a MemoryNode from a file in tmpfs mapped to a CXL node.
    // Processes/VMs on same host will share the memory region, not guaranteed
    // across different hosts
    // assumes all processes/VMs use the same file path
    pub(crate) fn from_file(id: usize, path: &str, size: usize) -> Self {
        if size <= STATE_SIZE {
            panic!("Size must be greater than Allocator size");
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("Failed to open shared memory. Does the file exist?");

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
            state_addr: ptr as *mut SharedState,
            obj_addr: unsafe { ptr.offset(STATE_SIZE as isize) },
            size,
        }
    }

    pub(crate) fn addr_at(&self, offset: usize) -> *mut u8 {
        if offset >= self.size {
            panic!("Offset out of bounds");
        }
        unsafe { self.obj_addr.offset(offset as isize) }
    }

    // copy of the shared state (which remains unchanged)
    pub(crate) fn read_state(&self) -> SharedState {
        unsafe { std::ptr::read(self.state_addr) } // WARNING: might want to read_unaligned
    }

    // mutable reference to the shared state
    pub(crate) fn get_state(&self) -> &mut SharedState {
        unsafe { &mut *self.state_addr }
    }

    pub(crate) fn write_state(&self, state: SharedState) {
        unsafe {
            std::ptr::write(self.state_addr, state); // WARNING: might want to write_unaligned
        }
    }
}

impl Drop for MemoryNode {
    fn drop(&mut self) {
        unsafe {
            munmap(self.obj_addr as *mut libc::c_void, self.size);
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
        assert!(!node.obj_addr.is_null());
        assert_eq!(node.size, size); // 1 KiB

        // Clean up: remove the tmpfs file
        remove_file(path).expect("Failed to remove tmpfs file");
    }

}

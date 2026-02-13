use libc::{c_int, c_void};
use crate::shmem::SharedState;
use crate::shmem::STATE_SIZE;


#[link(name = "numa")]
extern "C" {
    pub fn numa_alloc_onnode(size: usize, node: c_int) -> *mut c_void;
    pub fn numa_free(mem: *mut c_void, size: usize);
}


#[derive(PartialEq, Eq, Hash, Clone)]
pub(crate) struct NumaMemoryNode {
    pub id: usize,
    state_addr: *mut SharedState,
    obj_addr: *mut u8,
    size: usize,
}


impl NumaMemoryNode {


    /// WARNING: placeholder only. memory is not shared, every node will its own memory region
    fn _from_numa(id: usize, size: usize, numa_node: i32) -> Self {
        let ptr = unsafe { numa_alloc_onnode(size, numa_node) };
        if ptr.is_null() {
            panic!("numa_alloc_onnode failed");
        }
        let ptr = ptr as *mut u8;

        NumaMemoryNode {
            id,
            state_addr: ptr as *mut SharedState,
            obj_addr: unsafe { ptr.offset(STATE_SIZE as isize) },
            size,
        }
    }
}

impl Drop for NumaMemoryNode {
    fn drop(&mut self) {
            unsafe {
                numa_free(self.obj_addr as *mut c_void, self.size);
            }
        }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_node_from_numa() {
        let mnid = 0;
        let size = 1024; // 1 KiB
        let numa_node = 0; // Node 0 should exist on most systems

        let node = NumaMemoryNode::_from_numa(mnid, size, numa_node);

        unsafe {
            *node.obj_addr = 31;
            // Initialize the shared memory region to zero
            std::ptr::write_bytes(node.obj_addr, 4, size);
        }

        assert_eq!(node.id, mnid);
        assert!(!node.obj_addr.is_null());

        assert_eq!(node.size, size); // 1 KiB
    }
}
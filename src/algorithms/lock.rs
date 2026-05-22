use crate::{ObjectMemoryEntry,ReadReturn};
use crate::safe_memio::{mem_writeall, mem_readone, MemoryError};


/// Client-writer: clients perform write operation directly i.e. no write
/// thread request handling.
pub fn lock_write<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView<T>,
    robj: &crate::RepCXLObject<T>,  
    data: T,
) -> Result<(), String> {
    let obj_info = &robj.info;

    // fetch the shared object lock from SharedState's ObjectIndex
    let obj_index = view
        .get_master_node()
        .unwrap()
        .get_state()
        .get_oi();

    let obj_lock = obj_index
        .get_lock(robj.object_index_pos)
        .expect("Object lock not found, invalid object index position");

    // acquire write lock
    obj_lock.write_lock();

    let entry = ObjectMemoryEntry::new_nowid(data);
    let res = match mem_writeall(obj_info.offset, entry, &view.memory_nodes) {
        Ok(()) => Ok(()),
        Err(MemoryError(memory_node_id)) => {
            Err(format!("Memory node {} failed during write", memory_node_id))
        }
    };

    obj_lock.write_unlock();
    
    res
}



/// Client-reader: clients perform read operation directly i.e. no read thread
/// processing requests
pub fn lock_read<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView<T>,
    robj: &crate::RepCXLObject<T>,
) -> Result<ReadReturn<T>, String> {
    let obj_info = &robj.info;

    // fetch the shared object lock from SharedState's ObjectIndex
    let obj_index = view
        .get_master_node()
        .unwrap()
        .get_state()
        .get_oi();

    let obj_lock = obj_index
        .get_lock(robj.object_index_pos)
        .expect("Object lock not found, invalid object index position");

    // acquire read lock
    obj_lock.read_lock();

    // read the first memory node (arbitrary choice since locks ensure consistency)
    let res = match mem_readone(obj_info.offset, &view.memory_nodes[0]) {
        Ok(state) => {
        
            Ok(ReadReturn::ReadSafe(state.value))
        },
        Err(MemoryError(memory_node_id)) => {
            Err(format!("Memory node {} failed during read", memory_node_id))
        }
    };

    // release and return
    obj_lock.read_unlock();
    res
}


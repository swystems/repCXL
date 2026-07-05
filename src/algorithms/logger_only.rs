/// Use the logger for replication of every object.
/// Normally the logger should be used for writes only, while reads can
/// be performed directly at the end of the log. This implementation swaps it 
/// around to avoid fundamentally changing the structure of repCXL:
/// - Writes are performed directly to memory (bypassing the logger)
/// - Reads always return a dirty read, which is then logged. 
// This implementation is just to provide a baseline of the logger-only approach.
use crate::ReadReturn;
use crate::safe_memio::{mem_readone, MemoryError};
use crate::request::Wid;

/// Best-effort write
pub fn logger_only_write<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView<T>,
    obj_info: &crate::ObjectInfo,
    data: T,
) -> Result<(), String> {
    super::best_effort::async_best_effort_write(view, obj_info, data)
}


/// Read one and return dirty read. 
pub fn logger_only_read<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView<T>,
    robj: &crate::RepCXLObject<T>,
) -> Result<ReadReturn<T>, String> {
    let obj_info = &robj.info;

    // read the first memory node (arbitrary choice since locks ensure consistency)
    let res = match mem_readone(obj_info.offset, &view.memory_nodes[0]) {
        Ok(state) => {
        
            Ok(ReadReturn::ReadDirty(crate::request::ReadDirtyPayload { wid: Wid::default(), obj_info: obj_info.clone(), data: state.value }))
        },
        Err(MemoryError(memory_node_id)) => {
            Err(format!("Memory node {} failed during read", memory_node_id))
        }
    };

    res

}


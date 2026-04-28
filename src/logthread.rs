use crate::shmem::MemoryNode;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use crate::safe_memio::{mem_readends, MemoryError};
use crate::shmem::log::{LogRequestQueue, LogQueueEntry};

/// Check if a log entry is still dirty and return the dirty value if it exists.
/// This condition is evaluated when the value of log entry still exists some memory 
/// nodes, not all, still contain it
fn check_dirty<T: Copy>(memory_nodes: &Vec<MemoryNode<T>>, entry: &LogQueueEntry) -> Option<T> {
    match mem_readends(entry.obj_info.offset, memory_nodes) {
        Ok(states) => {

            // check if consistent
            if states[0].wid == states[1].wid {return None;} 
            // check if the dirty value has not been overwritten by a new write
            if states[0].wid == entry.wid {return Some(states[0].value);}
            if states[1].wid == entry.wid {return Some(states[1].value);}

            None
        },
        Err(MemoryError(e)) => { 
            log::error!("Failed to read object state for obj {} in memory node {}", 
                entry.obj_info.id, 
                e);
            None
        }
     }
}

/// Main loop that reads and processes log entries from the queue
pub fn run<T: Copy>(lrq_path: String, memory_nodes_paths: Vec<String>, mem_size: usize, stop_flag: Arc<AtomicBool>) {

    
    std::thread::spawn(move || {

        let mut memory_nodes = Vec::new();

        // open memory nodes (same as repCXL main thread)
        for path in memory_nodes_paths {
            let mnid = memory_nodes.len();
            let node = MemoryNode::<T>::from_file(mnid, &path, mem_size);
            memory_nodes.push(node);
        }

        // open log request queue
        let mut lrq = LogRequestQueue::from_file(lrq_path.as_str());


        loop {
            // stop with algorithms threads on rep_cxl.stop()
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
        
            // read log entry from queue
            if let (Some(entry), pid) = lrq.get_next() {

               if let Some(v) = check_dirty(&memory_nodes, &entry) {
                    for node in &memory_nodes {
                        // get log
                        let log = node.get_state().get_log();
                        // appnd to log
                        log.append(entry.wid, entry.obj_info, v);
                    }

                    lrq.clear_entry(pid);
               }
            }
            else {
                // sleep?
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    });
}
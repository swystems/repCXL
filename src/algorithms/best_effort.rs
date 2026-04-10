use std::time::{Duration, SystemTime, Instant};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use log::{info,error,debug};
use crate::{ObjectMemoryEntry,ReadReturn};
use crate::utils::ms_logger::MonsterStateLogger;
use crate::safe_memio::{safe_write, mem_readall, mem_writeall, mem_readends, MemoryError};
use crate::{GroupView, WriteRequest};
use crate::timer;
use super::ReadAlgorithmContext;

const WRITE_TRACE_SAMPLE_RATE: u64 = 1024;


pub fn async_best_effort_write<T: Copy + PartialEq + std::fmt::Debug>(
    view: GroupView,
    req_queue_rx: kanal::Receiver<WriteRequest<T>>,
    stop_flag: Arc<AtomicBool>,
) {

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        match req_queue_rx.recv() {
            Ok(req) => {
                let trace_id = req.trace_id;
                let queue_wait = req.enqueue_at.elapsed();
                let write_start = Instant::now();

                // write data to all memory nodes
                let (oi, data, ack_tx) = req.to_tuple();
                for node in &view.memory_nodes {
                    let addr = node.addr_at(oi.offset) as *mut ObjectMemoryEntry<T>;
                    let entry = ObjectMemoryEntry::new_nowid(data);
                    safe_write(addr, entry).unwrap_or_else(|e| {
                        error!(
                            "Safe write failed at node {} offset {}: {}",
                            node.id, oi.offset, e
                        );
                    });
                }

                let replicate_time = write_start.elapsed();

                if let Err(e) = ack_tx.send(true) {
                        error!("Failed to send ack: {}", e);
                }

                if trace_id % WRITE_TRACE_SAMPLE_RATE == 0 {
                    debug!(
                        "[WRITE_TRACE][worker] id={} queue_wait={}ns replicate={}ns",
                        trace_id,
                        queue_wait.as_nanos(),
                        replicate_time.as_nanos(),
                    );
                }
            },
            Err(e) => {
                info!("Object queue channel closed: {}", e);
                break; // exit thread
            }
        }
    }
}

/// Thread-reader: process read requests from repCXL object channels and sends
/// ReadReturn. inter-thread communication might lead to overhead, prefer 
/// _client version for better latency  
pub fn async_best_effort_read<T: Copy + PartialEq + std::fmt::Debug>(ractx: ReadAlgorithmContext<T>,) {
    let view = ractx.group_view;
    loop {
        if ractx.stop_flag.load(Ordering::Relaxed) {
            break;
        }
        match ractx.req_queue_rx.recv() {
            Ok(req) => {
                match mem_readall(req.obj_info.offset, &view.memory_nodes) {
                    Ok(states) => {
                        // check if all states are consistent by VALUE since best effort does not use WID

                        let consistent = states.iter().all(|s| s.value == states[0].value);
                        // return based on consistency
                        let result = if consistent {
                            ReadReturn::ReadSafe(states[0].value)
                        } else {
                            ReadReturn::ReadDirty(states[0].value)
                        };
                        if let Err(e) = req.ack_tx.send(result) {
                            error!("Failed to send read response: {}", e);
                        }
                    },
                    Err(MemoryError(memory_node_id)) => {
                        error!("Memory node {} failed during read", memory_node_id);
                    }
                }
            },
            Err(e) => {
                log::info!("[READ] Read request channel closed: {}", e);
                break; // exit thread
            }
        }
    }
}


/// Client-writer: clients perform write operation directly i.e. no write
/// thread request handling.
pub fn async_best_effort_write_client<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView,
    obj: &crate::RepCXLObject<T>,
    data: T,
) -> Result<(), String> {
    let entry = ObjectMemoryEntry::new_nowid(data);
    match mem_writeall(obj.info.offset, entry, &view.memory_nodes) {
        Ok(()) => Ok(()),
        Err(MemoryError(memory_node_id)) => {
            Err(format!("Memory node {} failed during write", memory_node_id))
        }
    }
}

/// Client-reader: clients perform read operation directly i.e. no read thread
/// processing requests
pub fn async_best_effort_read_client<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView,
    obj: &crate::RepCXLObject<T>,
) -> Result<ReadReturn<T>, String> {

    match mem_readends(obj.info.offset, &view.memory_nodes) {
        Ok(states) => {
            // check if all states are consistent by VALUE since best effort does not use WID

            let consistent = states.iter().all(|s: &ObjectMemoryEntry<T>| s.value == states[0].value);
            // return based on consistency
            let result = if consistent {
                ReadReturn::ReadSafe(states[0].value)
            } else {
                ReadReturn::ReadDirty(states[0].value)
            };
            Ok(result)
        },
        Err(MemoryError(memory_node_id)) => {
            Err(format!("Memory node {} failed during read", memory_node_id))
        }
    }
}


pub fn sync_best_effort<T: Copy + PartialEq + std::fmt::Debug>(
    view: crate::GroupView,
    start_instant: Instant,
    round_time: Duration,
    req_queue_rx: kanal::Receiver<WriteRequest<T>>,
    stop_flag: Arc<AtomicBool>,
    _logger: Option<MonsterStateLogger>,
) {
    
    let mut round_num = 0;

    // let start_instant = system_time_to_instant(start_time);
    let mut next_round = start_instant;
    timer::wait_start_time(start_instant, timer::ROUND_SLEEP_RATIO);

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        debug!(
            "Round #{round_num}, delay {:?}",
            Instant::now().duration_since(next_round)
        );

        match req_queue_rx.try_recv() {
            Ok(Some(req)) => {
                // write data to all memory nodes
                let (oi, data, ack_tx) = req.to_tuple();
                let ome = ObjectMemoryEntry::new_nowid(data);
                
                match mem_writeall(oi.offset, ome, &view.memory_nodes) {
                    Ok(()) => {
                        // send ack to client
                        if let Err(_) = ack_tx.send(true) {
                            error!("Failed to send ack");
                        }
                    },
                    Err(MemoryError(memory_node_id)) => {
                        error!("Memory node {} failed during write replication", memory_node_id);
                        break;
                    }
                }
            },
            Ok(None) => (), // no request, continue to next round
            Err(e) => {
                match e {
                    kanal::ReceiveError::Closed => {
                        info!("Object queue channel closed: {}", e);
                        break; // exit thread
                    },
                    kanal::ReceiveError::SendClosed => {
                        info!("Send object queue channel closed: {}", e);
                        break; // exit thread
                    }
                }
            }
        }

        (round_num, next_round) = timer::wait_next_round(
            start_instant, 
            round_time, 
            timer::ROUND_SLEEP_RATIO);
    }
}





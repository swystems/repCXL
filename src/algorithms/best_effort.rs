use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use log::{info,error,debug};
use crate::{ObjectMemoryEntry,ReadReturn};
use crate::utils::ms_logger::MonsterStateLogger;
use crate::safe_memio::{mem_writeall, mem_readends, MemoryError};
use crate::{GroupView, WriteRequest, ReadRequest};
use crate::timer;
use super::AlgorithmThreadContext;

const WRITE_TRACE_SAMPLE_RATE: u64 = 1024;



/// Client-writer: clients perform write operation directly i.e. no write
/// thread request handling.
pub fn async_best_effort_write<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView,
    obj_info: &crate::ObjectInfo,
    data: T,
) -> Result<(), String> {
    let entry = ObjectMemoryEntry::new_nowid(data);
    match mem_writeall(obj_info.offset, entry, &view.memory_nodes) {
        Ok(()) => Ok(()),
        Err(MemoryError(memory_node_id)) => {
            Err(format!("Memory node {} failed during write", memory_node_id))
        }
    }
}


pub fn async_best_effort_write_thread<T: Copy + PartialEq + std::fmt::Debug>(
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
                
                let trace_id = req.trace_id; //debug
                let queue_wait = req.enqueue_at.elapsed(); //debug
                let write_start = Instant::now(); //debug

                // write data to all memory nodes
                let (oi, data, ack_tx) = req.to_tuple();
                match async_best_effort_write(&view, &oi, data) {
                    Ok(()) => {
                        let replicate_time = write_start.elapsed(); //debug

                        // send ack to client
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
                        error!("Failed to write object: {}", e);
                        continue;
                    }
                }

                
            },
            Err(e) => {
                info!("Object queue channel closed: {}", e);
                break; // exit thread
            }
        }
    }
}


/// Client-reader: clients perform read operation directly i.e. no read thread
/// processing requests
pub fn async_best_effort_read<T: Copy + PartialEq + std::fmt::Debug>(
    view: &crate::GroupView,
    obj_info: &crate::ObjectInfo,
) -> Result<ReadReturn<T>, String> {

    match mem_readends(obj_info.offset, &view.memory_nodes) {
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


/// Thread-reader: process read requests from repCXL object channels and sends
/// ReadReturn. inter-thread communication might lead to overhead, prefer 
/// _client version for better latency  
pub fn async_best_effort_read_thread<T: Copy + PartialEq + std::fmt::Debug>(
    actx: AlgorithmThreadContext,
    req_queue: kanal::Receiver<ReadRequest<T>>,
) {
    let view = actx.group_view;
    loop {
        if actx.stop_flag.load(Ordering::Relaxed) {
            break;
        }
        match req_queue.recv() {
            Ok(req) => {
                match async_best_effort_read(&view, &req.obj_info) {
                    Ok(result) => {
                        if let Err(e) = req.ack_tx.send(result) {
                            error!("Failed to send read response: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to read object: {}", e);
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





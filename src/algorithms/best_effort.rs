use std::time::{Duration, SystemTime};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use log::{error, warn};
use crate::{ObjectMemoryEntry,ReadReturn};
use safe_memio::{mem_readone, MemoryError};

use super::*;


pub fn async_best_effort_write<T: Copy + PartialEq + std::fmt::Debug>(
    view: crate::GroupView,
    _start_time: SystemTime,
    _round_time: Duration,
    req_queue_rx: mpsc::Receiver<WriteRequest<T>>,
    stop_flag: Arc<AtomicBool>,
) {

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        match req_queue_rx.try_recv() {
            Ok(req) => {
                // write data to all memory nodes
                let (oi, data, ack_tx) = req.to_tuple();
                for node in &view.memory_nodes {
                    let addr = node.addr_at(oi.offset) as *mut ObjectMemoryEntry<T>;
                    let entry = ObjectMemoryEntry::new_nowid(data);
                    safe_memio::safe_write(addr, entry).unwrap_or_else(|e| {
                        error!(
                            "Safe write failed at node {} offset {}: {}",
                            node.id, oi.offset, e
                        );
                    });
                }

                if let Err(e) = ack_tx.send(true) {
                        error!("Failed to send ack: {}", e);
                }
            },
            Err(e) => {
                match e {
                    mpsc::TryRecvError::Empty => (),
                    mpsc::TryRecvError::Disconnected => {
                        warn!("Object queue channel closed: {}", e);
                        break; // exit thread
                    }
                }
            }
        }
    }
}

/// Return a value from one memory node at random ASAP
pub fn async_best_effort_read<T: Copy + PartialEq + std::fmt::Debug>(
    view: crate::GroupView,
    _start_time: SystemTime,
    _round_time: Duration,
    req_queue_rx: mpsc::Receiver<ReadRequest<T>>,
    stop_flag: Arc<AtomicBool>,
) {

     loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
        match req_queue_rx.recv() {
            Ok(req) => {
                match mem_readone(req.obj_info.offset, &view.memory_nodes) {
                    Ok(ome) => {
                        let result = ReadReturn::ReadDirty(ome.value);
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
            }
        }
    }
}


pub fn sync_best_effort<T: Copy + PartialEq + std::fmt::Debug>(
    view: crate::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<WriteRequest<T>>,
    stop_flag: Arc<AtomicBool>,
) {
    
    let mut round_num = 0;

    let mut next_round = start_time;
    wait_start_time(start_time, ROUND_SLEEP_RATIO);

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }

        debug!(
            "Round #{round_num}, delay {:?}",
            SystemTime::now().duration_since(next_round).unwrap()
        );

        match req_queue_rx.try_recv() {
            Ok(req) => {
                // write data to all memory nodes
                let (oi, data, ack_tx) = req.to_tuple();
                for node in &view.memory_nodes {
                    let addr = node.addr_at(oi.offset) as *mut ObjectMemoryEntry<T>;
                    let entry = ObjectMemoryEntry::new_nowid(data);
                    safe_memio::safe_write(addr, entry).unwrap_or_else(|e| {
                        error!(
                            "Safe write failed at node {}, obj id: {} offset {}: {}",
                            node.id, oi.id, oi.offset, e
                        );
                    });
                }

                if let Err(e) = ack_tx.send(true) {
                        error!("Failed to send ack: {}", e);
                }
            },
            Err(e) => {
                match e {
                    mpsc::TryRecvError::Empty => (),
                    mpsc::TryRecvError::Disconnected => {
                        warn!("Object queue channel closed: {}", e);
                        break; // exit thread
                    }
                }
            }
        }

        (round_num, next_round) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);
    }
}
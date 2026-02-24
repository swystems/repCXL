use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::*;
use crate::Wid;
use crate::safe_memio::{mem_writeall, mem_readall, MemoryError};
use crate::utils::ms_logger;
use crate::{ObjectMemoryEntry, ReadReturn};


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MonsterState {
    Try,
    Retry,
    Check,
    Replicate,
    Wait,
    PostConflictCheck,
}

impl std::fmt::Display for MonsterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MonsterState::Try => write!(f, "Try"),
            MonsterState::Retry => write!(f, "Retry"),
            MonsterState::Check => write!(f, "Check"),
            MonsterState::Replicate => write!(f, "Replicate"),
            MonsterState::Wait => write!(f, "Wait"),
            MonsterState::PostConflictCheck => write!(f, "PostConflictCheck"),
        }
    } 
}

struct MonsterStats {
    conflicts: u64,
    sync_failures: u64,
    empty_requests: u64,
    prev_round: u64,
}

impl MonsterStats {
    fn new() -> Self {
        Self {
            conflicts: 0,
            sync_failures: 0,
            empty_requests: 0,
            prev_round: 1,
        }
    }

    fn check_sync_failure(&mut self, round_num: u64) {
        if round_num > self.prev_round + 1 {
            self.sync_failures += 1;
        }
        self.prev_round = round_num;
    }
}

// 
// logging macro with phase tag
macro_rules! monster_info {
    ($tag:expr, $($arg:tt)*) => {
        log::debug!("[{} phase] {}", $tag, format_args!($($arg)*));
    };
}

macro_rules! monster_error {
    ($tag:expr, $($arg:tt)*) => {
        log::error!("[{}] {}", $tag, format_args!($($arg)*));
    };
}

pub fn monster_write<T: Copy + PartialEq + std::fmt::Debug>(
    view: super::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<WriteRequest<T>>,
    stop_flag: Arc<AtomicBool>,
    mut logger_opt: Option<ms_logger::MonsterStateLogger>,
) {
    let mut round_num = 1; // start from 1 to diff from zero-initialized ObjectMemoryEntry
    let mut monster_state = MonsterState::Try;

    // MONSTER loop vars
    let mut pending_req = None; // pending write request
    let mut wid = Wid::new(0,0); // write request id
    let mut oid = 0; // object id
    let mut stats = MonsterStats::new();


    // get shared write conflict checker
    let mnode_state = view.get_master_node().unwrap().get_state();
    let owcc = mnode_state.get_owcc();

    // wait to start
    let mut round_start = start_time;
    wait_start_time(start_time, ROUND_SLEEP_RATIO);

    loop {
        if stop_flag.load(Ordering::Relaxed) {
            monster_info!(monster_state, "Stop flag is set, exiting");
            log::info!("Monster stats: conflicts={}, sync_failures={}, empty_requests={}", stats.conflicts, stats.sync_failures, stats.empty_requests);
            break;
        }

        monster_info!(monster_state,
            "Round #{round_num}, delay {:?}, obj id: {}",
            SystemTime::now().duration_since(round_start).unwrap(),
            oid
        );

        stats.check_sync_failure(round_num);

        // Log state transition if logging is enabled
        if let Some(ref mut logger) = logger_opt {
            logger.log_monster(round_num, monster_state, oid);
        }

        match monster_state {
            MonsterState::Try => {
                match req_queue_rx.try_recv() {
                    Ok(req) => {
                        wid = Wid::new(round_num, view.self_id);
                        oid = req.obj_info.id;
                        owcc.write(oid, round_num, view.self_id);
                        monster_state = MonsterState::Check;

                        pending_req = Some(req);

                        
                    },
                    Err(e) => match e {
                        mpsc::TryRecvError::Empty => {
                            // no request, stay in Try state
                            stats.empty_requests += 1;
                        },
                        mpsc::TryRecvError::Disconnected => {
                            // the repcxl instance keeps the original sender, 
                            // so this should occur when the instance is dropped
                            monster_info!(monster_state, "request queue channel closed: {}", e);
                            break;
                        }
                    }
                }
            },

            // Same as Try but don't fetch new request, use the pending one
            MonsterState::Retry => {
                if pending_req.is_none() {
                    error!("No pending request in Retry state, disallowed state. Exiting.");
                    break;
                }

                let req = pending_req.as_ref().unwrap();
                wid = Wid::new(round_num, view.self_id);
                oid = req.obj_info.id;
                owcc.write(oid, round_num, view.self_id);

                monster_state = MonsterState::Check;
            },
            
            MonsterState::Check => {
                if owcc.is_last(oid, round_num, wid.round_num, wid.process_id) {
                    // current process is the last writer
                    monster_info!(monster_state, "Process {} is the last writer for object {} in round {}", view.self_id, oid, round_num);

                    if SystemTime::now().duration_since(round_start).unwrap() < round_time {
                        // on time, proceed to Replicate state
                        monster_state = MonsterState::Replicate;
                    } else {
                        // overtime (sync failure), wait for next round
                        monster_state = MonsterState::Check;
                    }
                }
                else {
                    // not the last writer
                    monster_state = MonsterState::Wait;
                }
            },

            MonsterState::Replicate => {
                if pending_req.is_none() {
                    error!("No pending request in Replicate state, disallowed state. Exiting.");
                    break;
                }
                
                let req = pending_req.as_ref().unwrap();
                let ome = ObjectMemoryEntry::new(wid, req.data);
                
                match mem_writeall(req.obj_info.offset, ome, &view.memory_nodes) {
                    Ok(()) => {
                        // send ack to client
                        if let Err(_) = req.ack_tx.send(true) {
                            error!("Failed to send ack");
                        }
                        pending_req = None;
                        monster_state = MonsterState::Try;
                    },
                    Err(MemoryError(memory_node_id)) => {
                        error!("Memory node {} failed during write replication", memory_node_id);
                        break;
                    }
                }
            },

            // wait for the replicate phase of the conflicting process to finish
            MonsterState::Wait => {
                monster_state = MonsterState::PostConflictCheck;
                stats.conflicts += 1;
            },

            // check if the conflicting write has been fully replicated, otherwise
            // retry the write.
            MonsterState::PostConflictCheck => {
                if pending_req.is_none() {
                    error!("No pending request in PostConflictCheck state, disallowed state. Exiting.");
                    break;
                }
                
                let req = pending_req.as_ref().unwrap();
                
                match mem_readall(req.obj_info.offset, &view.memory_nodes) {
                    Ok(omes) => {
                        // Check if any wid in omes is smaller than the current
                        // wid
                        let any_smaller = omes.iter().any(|ome: &ObjectMemoryEntry<T>| ome.wid < wid);

                        if any_smaller {                            
                            monster_info!(monster_state,
                                "Found wid smaller than current wid={:?} for object {}, retrying to write",
                                wid, req.obj_info.id
                            );


                            monster_state = MonsterState::Retry; 
                        } else {
                            monster_info!(monster_state, "State up to date");
                            // send ack to client
                            if let Err(_) = req.ack_tx.send(true) {
                                error!("Failed to send ack");
                            }   

                            pending_req = None;
                            monster_state = MonsterState::Try;
                        }
                    },
                    Err(MemoryError(memory_node_id)) => {
                        monster_error!(monster_state, "Memory node {} failed during post-conflict read", memory_node_id);
                        break;
                    }
                }
            }
        }

        (round_num, round_start) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);

    }
}

/// MONSTER READ: 
/// - pull read requests from queue (blocking) 
/// - async read all memory nodes and return ReadSafe or ReadDirty based
/// on state consistency
pub fn monster_read<T: Copy + PartialEq + std::fmt::Debug>(
    view: super::GroupView,
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
                match mem_readall(req.obj_info.offset, &view.memory_nodes) {
                    Ok(states) => {
                        // check if all states are consistent (have the same wid (i.e. value))
                        // and get the latest wid with one pass
                        // println!("{:?}", states);
                        let (consistent, latest) = states.iter().skip(1).fold(
                            (true, &states[0]),
                            |(cons, best), s| (cons && s.wid == states[0].wid, if s.wid > best.wid { s } else { best }),
                        );
                        // return based on consistency
                        let result = if consistent {
                            ReadReturn::ReadSafe(latest.value)
                        } else {
                            ReadReturn::ReadDirty(latest.value)
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
            Err(e) => { // the repcxl instance keeps the original sender, so this should occur when the instance is dropped 
                log::info!("[READ] Read request channel closed: {}", e);
                break; 
            }
        }
    }

}
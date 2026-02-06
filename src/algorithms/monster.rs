use super::*;
use crate::Wid;
use crate::safe_memio::{mem_writeall, mem_readall, MemoryError};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum MonsterState {
    Try,
    Check,
    Replicate,
    Wait,
    PostConflictCheck,
}

pub fn monster<T: Copy + PartialEq + std::fmt::Debug>(
    view: super::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<WriteRequest<T>>,
) {
    let mut round_num = 0;
    let mut monster_state = MonsterState::Try;

    // MONSTER loop vars
    let mut pending_req = None; // pending write request
    let mut wid = Wid::new(0,0); // write request id
    let mut oid = 0; // object id

    // get shared write conflict checker
    let mnode_state = view.get_master_node().unwrap().get_state();
    let owcc = mnode_state.get_owcc();

    // wait to start
    let mut round_start = start_time;
    wait_start_time(start_time, ROUND_SLEEP_RATIO);

    loop {
        debug!(
            "Round #{round_num}, delay {:?}, phase: {:?}, obj id: {}",
            SystemTime::now().duration_since(round_start).unwrap(),
            monster_state,
            oid
        );

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
                        },
                        mpsc::TryRecvError::Disconnected => {
                            warn!("request queue channel closed: {}", e);
                            break;
                        }
                    }
                }
            },
            
            MonsterState::Check => {
                if owcc.is_last(oid, round_num, wid.round_num, wid.process_id) {
                    // current process is the last writer
                    debug!("Process {} is the last writer for object {} in round {}", view.self_id, oid, round_num);

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

                if let None = pending_req {
                    error!("No pending request in Replicate state, disallowed state. Exiting.");
                    break;
                }
                
                let req = pending_req.unwrap();
                let ome = ObjectMemoryEntry::new(wid, req.data);
                
                match mem_writeall(req.obj_info.offset, ome, &view.memory_nodes) {
                    Ok(()) => {
                        // send ack to client
                        if let Err(_) = req.ack_tx.send(true) {
                            error!("Failed to send ack");
                        }

                        monster_state = MonsterState::Try;
                    },
                    Err(MemoryError(memory_node_id)) => {
                        error!("Memory node {} failed during write replication", memory_node_id);
                        break;
                    }

                }

                pending_req = None;
            },
            MonsterState::Wait => {
                monster_state = MonsterState::PostConflictCheck;
            },
            MonsterState::PostConflictCheck => {
                // TODO: implement post-conflict check logic

                monster_state = MonsterState::Try;
            }
        }

        (round_num, round_start) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);

    }
}
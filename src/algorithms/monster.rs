use crate::shmem::wcc::WCC;

use super::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum MonsterState {
    Try,
    Check,
    Replicate,
    Wait,
    PostConflictCheck,
}

// struct MonsterStateMachine {
//     state: MonsterState,
// }

// impl MonsterStateMachine {
//     fn new() -> Self {
//         MonsterStateMachine {
//             state: MonsterState::Init,
//         }
//     }

//     fn next(&mut self) -> MonsterState {
    

//         self.state = match self.state {
//             MonsterState::Init => MonsterState::Try,
//             MonsterState::Try => MonsterState::Check,
//             MonsterState::Check => MonsterState::Replicate,
//             MonsterState::Replicate => MonsterState::Wait,
//             MonsterState::Wait => MonsterState::PostConflictCheck,
//             MonsterState::PostConflictCheck => MonsterState::Try,
//         };

//         self.state
//     }
// }



pub fn monster<T: Copy + PartialEq + std::fmt::Debug>(
    view: super::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<WriteRequest<T>>,
) {
    let mut round_num = 0;
    let mut monster_state = MonsterState::Try;

    // MONSTER vars
    let mut wcc= &mut WCC::new(); // empty WCC for initialization
    let mut pending_req = None; // pending write request
    let mut wid = (0,0);

    // get shared write conflict checker
    let mnode_state = view.get_master_node().unwrap().get_state();
    let wcc_mo = mnode_state.get_wcc_mo();

    // wait to start
    let mut next_round = start_time;
    wait_start_time(start_time, ROUND_SLEEP_RATIO);

    loop {
        debug!(
            "Round #{round_num}, delay {:?}",
            SystemTime::now().duration_since(next_round).unwrap()
        );

        match monster_state {
            MonsterState::Try => {
                match req_queue_rx.try_recv() {
                    Ok(req) => {
                        if let Some(wcc_ref) = wcc_mo.get_object_wcc(req.object_id) { // @TODO optimize EXTRA MEM ACCESS!
                            wcc = wcc_ref;
                            wid = (round_num, view.self_id);
                            wcc.write(round_num, view.self_id);
                            monster_state = MonsterState::Check;

                            pending_req = Some(req);

                        } else {
                            error!("Object {} not found in WCC", req.object_id);
                        }
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
                wcc.is_last(round_num, wid.0 , wid.1);
                
                
                monster_state = MonsterState::Replicate;
            },

            MonsterState::Replicate => {

                if let None = pending_req {
                    error!("No pending request in Replicate state, disallowed state. Exiting.");
                    break;
                }

                match replicate(pending_req.unwrap(), &view) {
                    Ok(()) => {
                        monster_state = MonsterState::Try;
                    },
                    Err(e) => match e {
                        WriteError::MemoryNodeFailure(id) => {
                            error!("Memory node {} failed during write replication", id);
                            break;
                            },
                        WriteError::AckSendFailure => {
                                error!("Failed to send ack");
                            },
                    }
                }

                pending_req = None;
            },
            MonsterState::Wait => {},
            MonsterState::PostConflictCheck => {}
        }

        (round_num, next_round) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);

    }
}
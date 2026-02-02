use crate::shmem::MemoryNode;

use super::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum MonsterState {
    Try,
    Check,
    Replicate,
    Wait,
    PostConflictCheck
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
    req_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
) {
    let mut round_num = 0;

    let mut monster_state = MonsterState::Try;

    // get shared write conflict checker
    let mnode_state = view.get_master_node().unwrap().get_state();
    let wcc = mnode_state.get_wcc_mo();

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
                
            },
            MonsterState::Check => {},
            MonsterState::Replicate => {
                let req = req_queue_rx.try_recv();
                match replicate(req, &view) {
                    Ok(()) => {
                        monster_state = MonsterState::Wait;
                    },
                    Err(e) => match e {
                        WriteError::MemoryNodeFailure(id) => {
                            error!("Memory node {} failed during write replication", id);
                            break;
                            },
                        WriteError::AckSendFailure => {
                                error!("Failed to send ack");
                            },
                        WriteError::NoMoreClients => {
                            warn!("request queue channel closed");
                            break;
                        },
                    }
                }
            },
            MonsterState::Wait => {},
            MonsterState::PostConflictCheck => {}
        }

        (round_num, next_round) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);

    }
}
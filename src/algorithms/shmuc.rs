/// LEGACY module OUT OF TREE

const SHMUC_MEMBERSHIP_CHANGE_INTERVAL: u64 = 10; // every N rounds do a membership change


#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ShmucRound {
    Init,
    Write,
    Read,
    ViewChange1,
    ViewChange2,
    ViewChange3,
    ViewChange4,
    PendingViewChange,
    Quit,
}

struct ShmucStateMachine {
    state: ShmucRound,
}

impl ShmucStateMachine {
    fn new() -> Self {
        ShmucStateMachine {
            state: ShmucRound::Init,
        }
    }

    fn next(&mut self, round_num: u64) -> ShmucRound {
        // always membership view change periodically unless process has quit
        if round_num % SHMUC_MEMBERSHIP_CHANGE_INTERVAL == 0 && self.state != ShmucRound::Quit {
            self.state = ShmucRound::ViewChange1;
            return self.state;
        }

        self.state = match self.state {
            ShmucRound::Init => ShmucRound::Write,
            ShmucRound::Write => ShmucRound::Read,
            ShmucRound::Read => ShmucRound::Write,
            ShmucRound::ViewChange1 => ShmucRound::ViewChange2,
            ShmucRound::ViewChange2 => ShmucRound::ViewChange3,
            ShmucRound::ViewChange3 => ShmucRound::ViewChange4,
            ShmucRound::ViewChange4 => ShmucRound::Write,
            ShmucRound::PendingViewChange => ShmucRound::PendingViewChange,
            ShmucRound::Quit => ShmucRound::Quit,
        };

        self.state
    }
}

///
enum ShmucError {
    MemioError(&'static str),
    RoundOvertime,
}

/// Time-checked write. Fails if
/// - operation latency exceeds round time
/// - memory I/O operation fails
fn tchk_write<T: Copy>(round_end: SystemTime, addr: *mut T, data: T) -> Result<(), ShmucError> {
    safe_memio::safe_write(addr, data).map_err(|e| ShmucError::MemioError(e))?; // throws write error
    if SystemTime::now() > round_end {
        Err(ShmucError::RoundOvertime)
    } else {
        Ok(())
    }
}

/// Time-checked write. Fails if
/// - operation latency exceeds round time
/// - memory I/O operation fails
fn tchk_read<T: Copy>(round_end: SystemTime, addr: *mut T) -> Result<T, ShmucError> {
    let data = safe_memio::safe_read(addr).map_err(|e| ShmucError::MemioError(e))?; // throws read error
    if SystemTime::now() > round_end {
        Err(ShmucError::RoundOvertime)
    } else {
        Ok(data)
    }
}

/// Shared Memory Uniform Coordination
pub fn shmuc<T: Copy + PartialEq + std::fmt::Debug>(
    view: super::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
) {
    let mut round_num = 0;

    let mut shmuc_sm = ShmucStateMachine::new();
    // get shared write conflict referee
    let mstate = view.get_master_node().unwrap().get_state();
    let wcc = mstate.get_wcc();
    let mut pending_write_req = None;

    // wait to start
    let mut next_round = start_time;
    wait_start_time(start_time, ROUND_SLEEP_RATIO);

    loop {
        debug!(
            "Round #{round_num}, delay {:?}",
            SystemTime::now().duration_since(next_round).unwrap()
        );

        match shmuc_sm.next(round_num) {
            ShmucRound::Write => {
                debug!("Write round");

                // try to pop an object from the queue
                match req_queue_rx.try_recv() {
                    Ok((offset, data, ack_tx)) => {
                        // WCC check. We use the offset as request ID
                        wcc.push_request(offset, view.self_id);
                        // write data to all memory nodes
                        for node in &view.memory_nodes {
                            let addr = node.addr_at(offset) as *mut T;

                            // currently safe_write just simulates failures for testing purposes
                            if let Err(e) = tchk_write(next_round + round_time, addr, data) {
                                match e {
                                    ShmucError::MemioError(err) => {
                                        warn!(
                                            "Write failed on node {} at address {:p}: {}",
                                            node.id, addr, err
                                        );
                                    }
                                    ShmucError::RoundOvertime => {
                                        warn!(
                                            "Write operation overtime on node {}, retrying",
                                            node.id
                                        );
                                    }
                                }
                            }
                        }
                        // write successful, store object for read verification in next round
                        pending_write_req = Some((offset, data, ack_tx));
                    }
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
            ShmucRound::Read => {
                debug!("Read round");

                if let Some((offset, _, ack_tx)) = &pending_write_req {
                    // write data to all memory nodes
                    let mut success = true;

                    // WCC check
                    // @TODO: time check the operation!
                    if let Some(conflicting_pids) = wcc.check_conflicts(*offset) {
                        // handle write conflict
                        info!("conflict detected with pids: {:?}", conflicting_pids);
                        // min process ID wins
                        if view.self_id == *conflicting_pids.iter().min().unwrap() {
                            info!("this process wins the write conflict");
                        } else {
                            info!("write conflict lost");
                            // @TODO: retry instead of just failing
                            success = false;
                        }
                    }

                    // send ack
                    if let Err(e) = ack_tx.send(success) {
                        error!("Failed to send ack: {}", e);
                    }

                    pending_write_req = None; // clear pending write request

                    // for node in &view.memory_nodes {

                    // let addr = node.addr_at(*offset) as *mut T;

                    // match tchk_read(next_round + round_time, addr) {
                    //     Ok(val) => {
                    //         if val != *data {
                    //             warn!(
                    //                 "Read verification failed on node {}: wrote {:?}, read back {:?}",
                    //                 node.id, data, val
                    //             );
                    //             success = false;
                    //         } else {
                    //             debug!(
                    //                 "Successfully read back {:?} from node {}",
                    //                 val, node.id
                    //             );
                    //         }
                    //     }
                    //     Err(e) => {
                    //         match e {
                    //             ShmucError::MemioError(err) => {
                    //                 warn!(
                    //                     "Read failed on node {} at address {:p}: {}",
                    //                     node.id, addr, err
                    //                 );
                    //             }
                    //             ShmucError::RoundOvertime => {
                    //                 warn!("Read operation overtime on node {}", node.id);
                    //             }
                    //         }
                    //         success = false;
                    //     }
                    // }
                    // }
                }
            }
            ShmucRound::ViewChange1 => {
                info!("VC round, currently no-op");
            }
            ShmucRound::ViewChange2 => {
                info!("VC round, currently no-op");
            }
            ShmucRound::ViewChange3 => {
                info!("VC round, currently no-op");
            }
            ShmucRound::ViewChange4 => {
                info!("VC round, currently no-op");
            }
            _ => {
                warn!("Wait in PendingViewChange, Init or Quit state");
            }
        }

        (round_num, next_round) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);
    }
}

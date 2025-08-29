use log::{debug, error, info, warn};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use crate::safe_memio;

// CONFIGURATION
const ROUND_SLEEP_RATIO: f64 = 0.0; // Percentage of round time to sleep before busy-waiting
const SHMUC_MEMBERSHIP_CHANGE_INTERVAL: u64 = 10; // every N rounds do a membership change

/// Wait until the specified start time, sleeping for a portion of the time and busy-waiting for the rest
pub fn wait_start_time(start_time: SystemTime, sleep_ratio: f64) {
    // let treshold = next * 2;
    let round_time = start_time
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::from_secs(0));

    if sleep_ratio < 0.0 || sleep_ratio > 1.0 {
        panic!("sleep_ratio must be between 0.0 and 1.0");
    }
    let ns = round_time.as_nanos() as f64;
    let sleep_duration = Duration::from_nanos((ns * sleep_ratio) as u64);

    if round_time > sleep_duration {
        // uses nanosleep() syscall on linux
        std::thread::sleep(sleep_duration);

        // might sleep for more than the requested time
        if SystemTime::now() > start_time {
            return;
        }
    }

    while SystemTime::now() < start_time {
        std::hint::spin_loop();
        //std::thread::yield_now();
    }
}

/// Wait for the next round to start. Returns its number and start time.
/// Sleeps for a portion of the round time and busy-waits for the rest
pub fn wait_next_round(
    start_time: SystemTime,
    round_time: Duration,
    sleep_ratio: f64,
) -> (u64, SystemTime) {
    if sleep_ratio < 0.0 || sleep_ratio > 1.0 {
        panic!("sleep_ratio must be between 0.0 and 1.0");
    }

    let elapsed = SystemTime::now().duration_since(start_time).unwrap();
    let round_num = elapsed.div_duration_f64(round_time) as u64;
    let wake_up_time = round_time.mul_f64(sleep_ratio);
    let next_round =
        start_time + Duration::from_nanos(round_time.as_nanos() as u64 * (round_num + 1));

    // conversion required, operations with Duration accepts only u32 which
    // would give a max of ~4 billion rounds - not much considering round times of
    // microseconds or nanoseconds. from_nanos accepts u64 which gives us
    // a large enough round number to cover thousands of years
    let round_elapsed = Duration::from_nanos(
        (elapsed.as_nanos() - (round_time.as_nanos() * round_num as u128)) as u64,
    );

    // we could have already spent some time doing stuff in the round
    // so we have to take it into account (sleep amount is always relative to round start)
    if round_elapsed < wake_up_time {
        // uses nanosleep() syscall on linux
        std::thread::sleep(wake_up_time - round_elapsed);
    }

    while SystemTime::now() < next_round {
        std::hint::spin_loop();
        //std::thread::yield_now();
    }

    (round_num + 1, next_round)
}

/// Every round write 1 object to all memory nodes and read back to verify the write
/// was successful
pub fn _write_verify<T: Copy + PartialEq + std::fmt::Debug>(
    view: super::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    obj_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
) {
    let mut round_num = 0;
    // wait to start
    let mut next_round = start_time;
    wait_start_time(start_time, ROUND_SLEEP_RATIO);

    loop {
        // logic here
        debug!(
            "Round #{round_num}, delay {:?}",
            SystemTime::now().duration_since(next_round).unwrap()
        );

        match obj_queue_rx.try_recv() {
            Ok((offset, data, ack_tx)) => {
                // write data to all memory nodes
                let mut success = true;
                for node in &view.memory_nodes {
                    let addr = node.addr_at(offset) as *mut T;
                    unsafe {
                        std::ptr::write(addr, data);
                        // *addr = data;
                    }
                    // verify write
                    let read_back = unsafe { std::ptr::read(addr) };
                    if read_back != data {
                        warn!(
                            "Write verification failed on node {}: wrote {:?}, read back {:?}",
                            node.id, data, read_back
                        );
                        success = false;
                    }
                    debug!("Successfully wrote {:?} to node {}", data, node.id);
                }

                // send ack
                if let Err(e) = ack_tx.send(success) {
                    error!("Failed to send ack: {}", e);
                }
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

        (round_num, next_round) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);
    }
}

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
    obj_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
) {
    let mut round_num = 0;

    let mut shmuc_sm = ShmucStateMachine::new();
    // get shared write conflict referee
    let wcr = &mut view.get_master_node().unwrap().get_state().wcr;
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
                match obj_queue_rx.try_recv() {
                    Ok((offset, data, ack_tx)) => {
                        // WCR check. We use the offset as request ID
                        wcr.push_request(offset, view.self_id);
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

                    // WCR check
                    // @TODO: time check the operation!
                    if let Some(conflicting_pids) = wcr.check_conflicts(*offset) {
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

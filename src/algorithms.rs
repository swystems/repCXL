use log::{debug, error, warn};
use std::fmt::Write;
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use crate::safe_memio;
use crate::GroupView;
use crate::WriteRequest;


pub mod best_effort;
pub mod monster;

// CONFIGURATION
const ROUND_SLEEP_RATIO: f64 = 0.0; // Percentage of round time to sleep before busy-waiting
const ALGORITHM: &str = "sync_best_effort"; // default algorithm

pub fn from_config<T: Copy + PartialEq + std::fmt::Debug>(    
    view: GroupView,
    st: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<WriteRequest<T>>,
) {
    match ALGORITHM {
        "async_best_effort" => best_effort::async_best_effort(
            view,
            st,
            round_time,
            req_queue_rx,
        ),
        "sync_best_effort" => best_effort::sync_best_effort(
            view,
            st,
            round_time,
            req_queue_rx,
        ),
        "monster" => monster::monster(
            view,
            st,
            round_time,
            req_queue_rx,
        ),
        _ => {
            panic!("Unknown algorithm, check config: {}", ALGORITHM);
        }
    }
}

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
    req_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
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

        match req_queue_rx.try_recv() {
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

enum WriteError {
    MemoryNodeFailure(usize), // node id
    AckSendFailure,
}

fn replicate<T: Copy>(req: WriteRequest<T>, view: &GroupView) -> Result<(), WriteError> {
    let (offset, data, ack_tx) = req.to_tuple();

    // write data to all memory nodes
    for node in &view.memory_nodes {
        let addr = node.addr_at(offset) as *mut T;
        if let Err(e) = safe_memio::safe_write(addr, data) {
            error!(
                "Safe write failed at node {} offset {}: {}",
                node.id, offset, e
            );
            return Err(WriteError::MemoryNodeFailure(node.id));
        }
    }

    if let Err(_) = ack_tx.send(true) {
            return Err(WriteError::AckSendFailure);
    }
 
    Ok(())
}
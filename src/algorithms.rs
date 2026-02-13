use log::{debug, error};
use std::sync::atomic::{AtomicBool};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime};

use crate::safe_memio;
use crate::GroupView;
use crate::{WriteRequest,ReadRequest};


pub mod best_effort;
pub mod monster;

// CONFIGURATION
const ROUND_SLEEP_RATIO: f64 = 0.0; // Percentage of round time to sleep before busy-waiting
const _ALGORITHM: &str = "sync_best_effort"; // default algorithm


pub fn get_write_algorithm<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: String,
) -> fn(
    GroupView,
    SystemTime,
    Duration,
    mpsc::Receiver<WriteRequest<T>>,
    Arc<AtomicBool>,
) {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort,
        "sync_best_effort" => best_effort::sync_best_effort,
        "monster" => monster::monster_write,
        _ => panic!("Unknown algorithm, check config: {}", algorithm),
    }
}

pub fn get_read_algorithm<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: String,
) -> fn(
    GroupView,
    SystemTime,
    Duration,
    mpsc::Receiver<ReadRequest<T>>,
    Arc<AtomicBool>,
) {
    match algorithm.as_str() {
        "monster_read" => monster::monster_read,
        _ => panic!("Unknown algorithm, check config: {}", algorithm),
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

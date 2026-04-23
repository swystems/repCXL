
use core::panic;
use std::time::{Duration, Instant, SystemTime};

pub(crate) const ROUND_SLEEP_RATIO: f64 = 0.0; // Percentage of round time to sleep before busy-waiting

pub fn system_time_to_instant(start_time: SystemTime) -> Instant {
    let mut best_span = Duration::MAX;
    let mut best_mono_before = Instant::now();
    let mut best_mono_after = best_mono_before;
    let mut best_wall_now = SystemTime::now();

    for _ in 0..8 {
        let mono_before = Instant::now();
        let wall_now = SystemTime::now();
        let mono_after = Instant::now();

        let span = mono_after.duration_since(mono_before);
        if span < best_span {
            best_span = span;
            best_mono_before = mono_before;
            best_mono_after = mono_after;
            best_wall_now = wall_now;
        }

        if span <= Duration::from_micros(10) {
            break;
        }
    }

    let midpoint = best_mono_before + (best_mono_after.duration_since(best_mono_before) / 2);
    let delay = start_time
        .duration_since(best_wall_now)
        .unwrap_or(Duration::ZERO);
    midpoint + delay
}


/// Wait until a monotonic start instant, sleeping for a portion of the time and
/// busy-waiting for the rest.
pub fn wait_start_time(start_instant: Instant, sleep_ratio: f64) {
    let wait_duration = start_instant
        .checked_duration_since(Instant::now())
        .unwrap_or(Duration::from_secs(0));

    if sleep_ratio < 0.0 || sleep_ratio > 1.0 {
        panic!("sleep_ratio must be between 0.0 and 1.0");
    }

    let sleep_duration = wait_duration.mul_f64(sleep_ratio);

    if wait_duration > sleep_duration {
        std::thread::sleep(sleep_duration);
        if Instant::now() >= start_instant {
            return;
        }
    }

    while Instant::now() < start_instant {
        std::hint::spin_loop();
    }
}

/// Wait for the next round based on a monotonic start instant. Returns its
/// number and start instant.
pub fn wait_next_round(
    start_instant: Instant,
    round_time: Duration,
    sleep_ratio: f64,
) -> (u64, Instant) {
    if sleep_ratio < 0.0 || sleep_ratio > 1.0 {
        panic!("sleep_ratio must be between 0.0 and 1.0");
    }

    let elapsed = Instant::now().duration_since(start_instant);
    let round_time_ns = round_time.as_nanos();
    if round_time_ns == 0 {
        panic!("round_time must be greater than zero");
    }

    let round_num = (elapsed.as_nanos() / round_time_ns) as u64;
    let wake_up_time = round_time.mul_f64(sleep_ratio);
    let next_round = start_instant
        + Duration::from_nanos((round_time_ns * (round_num as u128 + 1)) as u64);
    let round_elapsed = Duration::from_nanos(
        (elapsed.as_nanos() - (round_time_ns * round_num as u128)) as u64,
    );

    if round_elapsed < wake_up_time {
        std::thread::sleep(wake_up_time - round_elapsed);
    }

    while Instant::now() < next_round {
        std::hint::spin_loop();
    }

    (round_num + 1, next_round)
}


/// Wait until the round has progressed by `round_progress` = between 0.0 (start) 
/// and 1.0 (end).
/// Sleeping for a portion of the time and busy-polls for the remaining. `sleep_ratio`
/// is relative to the full round, hence it must be less than `round_progress`.
pub fn wait_round_progress(
    round_progress: f64,
    start_instant: Instant,
    round_time: Duration,
    sleep_ratio: f64,
) {
    if sleep_ratio < 0.0 || sleep_ratio > 1.0 {
        panic!("sleep_ratio must be between 0.0 and 1.0");
    }

    if round_progress < 0.0 || round_progress > 1.0 {
        panic!("round_progress must be between 0.0 and 1.0");
    }

    if round_progress < sleep_ratio {
        panic!("round_progress must be greater than sleep_ratio to allow for sleeping");
    }

    let elapsed = Instant::now().duration_since(start_instant);
    let round_time_ns = round_time.as_nanos();
    if round_time_ns == 0 {
        panic!("round_time must be greater than zero");
    }

    let round_num = (elapsed.as_nanos() / round_time_ns) as u128;
    let wake_up_time = round_time.mul_f64(sleep_ratio);
    let round_elapsed = Duration::from_nanos(
        (elapsed.as_nanos() - (round_time_ns * round_num as u128)) as u64,
    );
    let target_time = start_instant +
        Duration::from_nanos((round_time_ns * round_num) as u64) // start of current round
        + round_time.mul_f64(round_progress); // target time within the round

    if round_elapsed < wake_up_time {
        std::thread::sleep(wake_up_time - round_elapsed);
    }

    while Instant::now() < target_time {
        std::hint::spin_loop();
    }
}


/// Simulated CXL switch delay of 300ns through a busy loop
pub fn cxl_switch_delay() {
    let start = Instant::now();
    let target = Duration::from_nanos(300);
    while Instant::now().duration_since(start) < target {
        std::hint::spin_loop();
    }
}
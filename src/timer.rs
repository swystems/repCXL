
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

/// Wait until the specified start time, sleeping for a portion of the time and busy-waiting for the rest
pub fn _wait_start_time(start_time: SystemTime, sleep_ratio: f64) {
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
pub fn _wait_next_round(
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

/// Wait until a monotonic start instant, sleeping for a portion of the time and
/// busy-waiting for the rest.
pub fn wait_start_instant(start_instant: Instant, sleep_ratio: f64) {
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
pub fn wait_next_round_instant(
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

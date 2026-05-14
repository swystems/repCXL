use std::time::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::config::RepCXLConfig;

pub mod arg_parser;
// pub mod mc_bench;
pub mod ycsb;
pub mod ms_logger;


pub fn percentile(lat_sorted: &Vec<u64>, p: f32) -> u64 {
    if lat_sorted.is_empty() {
        return 0;
    }
    let index = (p * lat_sorted.len() as f32).ceil() as usize - 1;
    lat_sorted[index]
}

fn downsample(lat_sorted: &Vec<u64>, samples: usize) -> Vec<u64> {
    lat_sorted.iter().step_by(lat_sorted.len() / samples).cloned().collect()
}

/// Format nanoseconds into the most human-readable unit.
pub fn fmt_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.2}s", ns as f64 / 1_000_000_000.0)
    } else if ns >= 1_000_000 {
        format!("{:.2}ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.2}μs", ns as f64 / 1_000.0)
    } else {
        format!("{}ns", ns)
    }
}

pub fn print_latency_stats(latencies: &Vec<Duration>) {
    let avg_ns = latencies.iter().sum::<Duration>().as_nanos() as u64 / latencies.len() as u64;
    let mut latencies_u64: Vec<u64> = latencies.iter().map(|d| d.as_nanos() as u64).collect();
    latencies_u64.sort_unstable(); // sort latencies for percentile calculation
    let p50 = percentile(&latencies_u64, 0.5);
    let p90 = percentile(&latencies_u64, 0.9);
    let p99 = percentile(&latencies_u64, 0.99);
    let p9999 = percentile(&latencies_u64, 0.9999);
    let p100 = *latencies_u64.iter().max().unwrap_or(&0);
    let mut vec100 = downsample(&latencies_u64, 100);
    vec100.push(p100); // ensure max latency is included in the vector
    println!("vec100: {:?}", vec100);
    println!("    avg:\t{}
    P50:\t{} (median)
    P90:\t{}
    P99:\t{}
    P99.99:\t{}
    P100:\t{}", fmt_ns(avg_ns), fmt_ns(p50), fmt_ns(p90), fmt_ns(p99), fmt_ns(p9999), fmt_ns(p100));
}



/// A compact read-write spinlock suitable for shared-memory use.
///
/// State encoding:
/// - bit0: writer held
/// - bits[1..]: reader count (each reader increments by 2)
#[repr(C, align(64))]
#[derive(Debug)]
pub struct RWSpinlock {
    state: AtomicUsize,
}

impl RWSpinlock {
    pub const fn new() -> Self {
        Self {
            state: AtomicUsize::new(0),
        }
    }

    pub fn read_lock(&self) {
        const WRITER_BIT: usize = 1;
        const READER_INC: usize = 2;

        loop {
            let state = self.state.load(Ordering::Relaxed);
            if (state & WRITER_BIT) != 0 {
                std::hint::spin_loop();
                continue;
            }

            // self.state = state checks whether a writer has acquired the lock since we read the state
            if self.state.compare_exchange(
                    state,
                    state + READER_INC,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ).is_ok()
            {
                return;
            }

            std::hint::spin_loop();
        }
    }

    pub fn read_unlock(&self) {
        const READER_INC: usize = 2;
        self.state.fetch_sub(READER_INC, Ordering::Release);
    }

    pub fn write_lock(&self) {
        const WRITER_BIT: usize = 1;
        loop {
            if self.state.compare_exchange(
                    0, 
                    WRITER_BIT, 
                    Ordering::Acquire, 
                    Ordering::Relaxed
                ).is_ok()
            {
                return;
            }
            std::hint::spin_loop();
        }
    }

    pub fn write_unlock(&self) {
        self.state.store(0, Ordering::Release);
    }
}

impl Default for RWSpinlock {
    fn default() -> Self {
        Self::new()
    }
}

// I know pls forgive my Trait sins
impl Clone for RWSpinlock {
    fn clone(&self) -> Self {
        Self::new()
    }
}


/// Set the core affinity for the current repcxl/logger process if a core_affinity 
/// range was provided in the configuration. 
pub fn set_core_affinity(config: &RepCXLConfig, is_logger: bool) {
    let rid = config.id as usize; // RepCXL id

    // no core pinning if empty
    if !config.core_affinity.is_empty() {    

        if config.core_affinity.len() < config.processes.len() + config.logger_cluster_size {
            println!("core_affinity: {:?}", config.core_affinity);
            panic!("broken config validation: core_affinity length must be \
            at least the total number of processes (including loggers)");
        }

        let core = if !is_logger {
            config.core_affinity[rid]
        } else {
            config.core_affinity[rid + config.processes.len()]
        };

        core_affinity::set_for_current(core_affinity::CoreId { id: core as usize});
    }
    
}
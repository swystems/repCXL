use std::time::Duration;

pub mod arg_parser;
// pub mod mc_bench;
pub mod ycsb;
pub mod ms_logger;


pub fn percentile(latencies: &Vec<u64>, p: f32) -> u64 {
    if latencies.is_empty() {
        return 0;
    }
    let mut sorted = latencies.clone();
    sorted.sort_unstable();
    let index = (p * sorted.len() as f32).ceil() as usize - 1;
    sorted[index]
}


/// Format nanoseconds into the most human-readable unit.
fn fmt_ns(ns: u64) -> String {
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
    let latencies_u64: Vec<u64> = latencies.iter().map(|d| d.as_nanos() as u64).collect();
    let p50 = percentile(&latencies_u64, 0.5);
    let p90 = percentile(&latencies_u64, 0.9);
    let p99 = percentile(&latencies_u64, 0.99);
    let p9999 = percentile(&latencies_u64, 0.9999);
    let p100 = *latencies_u64.iter().max().unwrap_or(&0);
    println!("    avg:\t{}
    P50:\t{} (median)
    P90:\t{}
    P99:\t{}
    P99.99:\t{}
    P100:\t{}", fmt_ns(avg_ns), fmt_ns(p50), fmt_ns(p90), fmt_ns(p99), fmt_ns(p9999), fmt_ns(p100));
}
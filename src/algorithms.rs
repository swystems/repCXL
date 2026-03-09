use log::{debug, error};
use std::sync::atomic::{AtomicBool};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::{RepCXLObject, safe_memio};
use crate::GroupView;
use crate::{WriteRequest,ReadRequest,ReadReturn};
use crate::utils::ms_logger::MonsterStateLogger;

pub mod best_effort;
pub mod monster;

// CONFIGURATION
const _ALGORITHM: &str = "sync_best_effort"; // default algorithm


pub fn get_write_algorithm<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: String,
) -> fn(
    GroupView,
    Instant,
    Duration,
    kanal::Receiver<WriteRequest<T>>,
    Arc<AtomicBool>,
    Option<MonsterStateLogger>,
) {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_write,
        "sync_best_effort" => best_effort::sync_best_effort,
        "monster" => monster::monster_write,
        _ => panic!("Unknown algorithm, check config: {}", algorithm),
    }
}

/// Get the read algorithm thread function.
/// Currently disabled.
#[allow(dead_code)]
pub fn get_read_algorithm<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: String,
) -> fn(
    GroupView,
    SystemTime,
    Duration,
    kanal::Receiver<ReadRequest<T>>,
    Arc<AtomicBool>,
) {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_read,
        "monster" => monster::monster_read,
        _ => panic!("Unknown algorithm, check config: {}", algorithm),
    }
}


pub fn get_read_algorithm_client<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: &String,
    start_instant: Instant,
    round_time: Duration,
    group_view: &GroupView,
    obj: &RepCXLObject<T>,
) -> Result<ReadReturn<T>, String> {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_read_client(group_view, obj),
        "monster" => monster::monster_read_client(
            start_instant, 
            round_time, 
            group_view, 
            obj),
        _ => panic!("Unknown algorithm, check config: {}", algorithm),
    }
}

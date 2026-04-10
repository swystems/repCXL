use std::sync::atomic::{AtomicBool};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{GroupView, RepCXLObject};
use crate::request::{WriteRequest,ReadRequest,ReadReturn};
use crate::utils::ms_logger::MonsterStateLogger;

pub mod best_effort;
pub mod monster;

pub(crate) struct ReadAlgorithmContext<T: Copy + PartialEq + std::fmt::Debug> {
    pub group_view: super::GroupView,
    pub req_queue_rx: kanal::Receiver<ReadRequest<T>>,
    pub stop_flag: Arc<AtomicBool>,
}

pub(crate) struct WriteAlgorithmContext<T: Copy + PartialEq + std::fmt::Debug> {
    pub group_view: GroupView,
    pub start_instant: Instant,
    pub round_time: Duration,
    pub req_queue: kanal::Receiver<WriteRequest<T>>,
    pub stop_flag: Arc<AtomicBool>,
    pub logger: Option<MonsterStateLogger>,
}

pub fn write_thread<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: &String,
    wactx: WriteAlgorithmContext<T>,
) {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_write(wactx.group_view, wactx.req_queue, wactx.stop_flag),
        "monster" => monster::monster_write(wactx),
        "fmonster" => monster::fmonster_write(wactx),
        _ => panic!("Unknown write algorithm, check config: {}", algorithm),
    }
}

pub fn read_thread<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: &String,
    ractx: ReadAlgorithmContext<T>,
) {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_read(ractx),
        "monster | fmonster" => monster::monster_read(ractx),
        _ => panic!("Unknown read algorithm, check config: {}", algorithm),
    }
}


/// Get the read algorithm thread function.
/// Currently disabled.
// pub fn get_read_algorithm<T: Copy + PartialEq + std::fmt::Debug>(
//     algorithm: String,
// ) -> fn(
//     GroupView,
//     SystemTime,
//     Duration,
//     kanal::Receiver<ReadRequest<T>>,
//     Arc<AtomicBool>,
// ) {
//     match algorithm.as_str() {
//         "async_best_effort" => best_effort::async_best_effort_read,
//         "monster" => monster::monster_read,
//         "fmonster" => monster::monster_read,
//         _ => panic!("Unknown read algorithm, check config: {}", algorithm),
//     }
// }


pub fn read_nothread<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: &String,
    start_instant: Instant,
    round_time: Duration,
    read_offset: Option<f64>,
    group_view: &GroupView,
    obj: &RepCXLObject<T>,
) -> Result<ReadReturn<T>, String> {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_read_client(group_view, obj),
        "monster" | "fmonster" => monster::monster_read_client(
            start_instant, 
            round_time, 
            read_offset,
            group_view, 
            obj),
        _ => panic!("Unknown read algorithm, check config: {}", algorithm),
    }
}

pub fn write_nothread<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: &String,
    group_view: &GroupView,
    obj: &RepCXLObject<T>,
    data: T,
) -> Result<(), String> {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_write_client(group_view, obj, data),
        _ => Err(format!("write_nothread not supported for algorithm '{}'", algorithm)),
    }
}

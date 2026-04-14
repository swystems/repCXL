use std::sync::atomic::{AtomicBool};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{GroupView, RepCXLObject};
use crate::request::{WriteRequest,ReadRequest,ReadReturn};

pub mod best_effort;
pub mod monster;

#[derive(Clone)]
pub(crate) struct AlgorithmThreadContext {
    pub group_view: super::GroupView,
    pub start_instant: Instant,
    pub round_time: Duration,
    pub read_offset: Option<f64>,
    pub stop_flag: Arc<AtomicBool>,
    pub logger: Option<String>,
}


impl AlgorithmThreadContext {
    pub fn to_call_context(&self, algorithm: &str, stats: monster::MonsterStats) -> AlgorithmCallContext {
        AlgorithmCallContext {
            algorithm: algorithm.to_string(),
            start_instant: self.start_instant,
            round_time: self.round_time,
            read_offset: self.read_offset,
            logger: self.logger.clone(),
            stats: stats,
        }
    }
}

pub(crate) struct AlgorithmCallContext {
    pub algorithm: String,
    pub start_instant: Instant,
    pub round_time: Duration,
    pub read_offset: Option<f64>,
    pub logger: Option<String>,
    pub stats: monster::MonsterStats,
}


pub fn write_thread<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: &String,
    actx: AlgorithmThreadContext,
    req_queue: kanal::Receiver<WriteRequest<T>>,
) {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_write_thread(actx.group_view, req_queue, actx.stop_flag),
        "monster" => monster::monster_write_thread(actx, req_queue),
        "fmonster" => monster::fmonster_write_thread(actx, req_queue),
        _ => panic!("Unknown write algorithm, check config: {}", algorithm),
    }
}

pub fn read_thread<T: Copy + PartialEq + std::fmt::Debug>(
    algorithm: &String,
    actx: AlgorithmThreadContext,
    req_queue: kanal::Receiver<ReadRequest<T>>,
) {
    match algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_read_thread(actx, req_queue),
        "monster" | "fmonster" => monster::monster_read_thread(actx, req_queue),
        _ => panic!("Unknown read algorithm, check config: {}", algorithm),
    }
}



pub fn read<T: Copy + PartialEq + std::fmt::Debug>(
    actx: &AlgorithmCallContext,
    view: &GroupView,
    obj: &RepCXLObject<T>,
) -> Result<ReadReturn<T>, String> {
    match actx.algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_read(&view, &obj.info),
        "monster" | "fmonster" => monster::monster_read(actx, view, &obj.info),
        _ => panic!("Unknown read algorithm, check config: {}", actx.algorithm),
    }
}

pub fn write<T: Copy + PartialEq + std::fmt::Debug>(
    actx: &mut AlgorithmCallContext,
    view: &GroupView,
    obj: &RepCXLObject<T>,
    data: T,
) -> Result<(), String> {
    match actx.algorithm.as_str() {
        "async_best_effort" => best_effort::async_best_effort_write(view, &obj.info, data),
        "monster"  => monster::monster_write(actx, view, &obj.info, data),
        "fmonster" => monster::fmonster_write(actx, view, &obj.info, data),
        _ => Err(format!("write not supported for algorithm '{}'", actx.algorithm)),
    }
}

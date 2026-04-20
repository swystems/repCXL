use std::hash::Hash;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use crate::shmem::object_index::ObjectInfo;

static WRITE_REQ_TRACE_ID: AtomicU64 = AtomicU64::new(1);

pub struct WriteRequest<T> {
    pub(crate) obj_info: ObjectInfo,
    pub data: T,
    pub ack_tx: kanal::Sender<bool>,
    pub trace_id: u64,
    pub enqueue_at: Instant,
}

impl<T> WriteRequest<T> {
    pub(crate) fn new(obj_info: ObjectInfo, data: T, ack_tx: kanal::Sender<bool>) -> Self {
        WriteRequest {
            obj_info,
            data,
            ack_tx,
            trace_id: WRITE_REQ_TRACE_ID.fetch_add(1, Ordering::Relaxed),
            enqueue_at: Instant::now(),
        }
    }

    pub(crate) fn to_tuple(self) -> (ObjectInfo, T, kanal::Sender<bool>) {
        (self.obj_info, self.data, self.ack_tx)
    }
}

pub struct ReadRequest<T> {
    pub(crate) obj_info: ObjectInfo,
    pub ack_tx: kanal::Sender<ReadReturn<T>>,
}

impl<T> ReadRequest<T> {
    pub(crate) fn new(obj_info: ObjectInfo, ack_tx: kanal::Sender<ReadReturn<T>>) -> Self {
        ReadRequest { obj_info, ack_tx }
    }
}

#[derive(Debug)]
pub enum ReadReturn<T> {
    ReadSafe(T),
    ReadDirty(T),
}

/// RepCXL write request unique identifier. Stored next to every object
/// Comparison checks for largest round number and smallest process ID if
/// round numbers are equal.
#[derive(Debug, Clone, Copy)]
pub struct Wid {
    pub round_num: u64,
    pub process_id: usize,
}

impl PartialEq for Wid {
    fn eq(&self, other: &Self) -> bool {
        self.round_num == other.round_num && self.process_id == other.process_id
    }
}
impl Eq for Wid {}

impl PartialOrd for Wid {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Wid {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.round_num.cmp(&other.round_num) {
            std::cmp::Ordering::Greater => std::cmp::Ordering::Greater,
            std::cmp::Ordering::Less => std::cmp::Ordering::Less,
            std::cmp::Ordering::Equal => other.process_id.cmp(&self.process_id), // smaller pid wins
        }
    }
}

impl Hash for Wid {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.round_num.hash(state);
        self.process_id.hash(state);
    }
}

impl Wid {
    pub fn new(round_num: u64, process_id: usize) -> Self {
        Wid {
            round_num,
            process_id,
        }
    }
}
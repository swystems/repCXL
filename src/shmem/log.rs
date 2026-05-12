use crate::request::Wid;
use super::object_index::ObjectInfo;
use crate::logger::LogQueueEntry;

pub const LOG_SIZE: usize = 1024; // Size of the log

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct LogEntry<T> {
    lqe: LogQueueEntry,
    data: T,
}

impl<T> LogEntry<T> {
    pub fn new(lqe: LogQueueEntry, data: T) -> Self {
        LogEntry { lqe, data }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Log<T> {
    entries: [Option<LogEntry<T>>; LOG_SIZE],
    size: usize,
}

impl<T: Copy> Log<T> {
    pub fn new() -> Self {
        Log {
            entries: [None; LOG_SIZE],
            size: 0,
        }
    }

    pub(crate) fn append(&mut self, wid: Wid, obj_info: ObjectInfo, data: T) {
        let entry = LogEntry::new(LogQueueEntry::new(wid, obj_info), data);
        self.entries[self.size % LOG_SIZE] = Some(entry);
        self.size += 1;
    }
}

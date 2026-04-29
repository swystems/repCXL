use super::MAX_PROCESSES;
use crate::request::Wid;
use super::object_index::ObjectInfo;

pub const LOG_SIZE: usize = 1024; // Size of the log

/// Log queue entry containing write identifier, object ID, and memory node ID
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct LogQueueEntry {
    pub wid: Wid,
    pub obj_info: ObjectInfo,
}

impl LogQueueEntry {
    pub fn new(wid: Wid, obj_info: ObjectInfo) -> Self {
        LogQueueEntry {
            wid,
            obj_info,
        }
    }
}

pub struct LogRequestQueue {
    entries: *mut [Option<LogQueueEntry>; MAX_PROCESSES],
    index: usize,
}

impl LogRequestQueue {
    pub fn from_file(lrq_path: &str) -> Self {
    
        let min_size = 2 * 1024 * 1024; // min required for DAX mapping
        let mut size = std::mem::size_of::<Option<LogQueueEntry>>() * MAX_PROCESSES;
        size = if size < min_size {
            min_size
        } else {
            size
        };
        let ptr = super::mmap_daxdev(lrq_path, size) as *mut [Option<LogQueueEntry>; MAX_PROCESSES];
                
        LogRequestQueue {
            entries: ptr,
            index: 0,
        }
    }

    pub fn push_wait(&mut self, wid: Wid, obj_info: ObjectInfo, pid: usize) {
        let entry = LogQueueEntry::new(wid, obj_info);
        unsafe {
            (*self.entries)[pid] = Some(entry);
        }

        // wait until the log thread processes the entry and clears it
        while unsafe { (*self.entries)[pid].is_some() } {
            std::thread::yield_now(); // Yield to allow log thread to process the entry
        }
    }

    pub fn get_next(&mut self) -> (Option<LogQueueEntry>, usize) {
        self.index = (self.index + 1) % MAX_PROCESSES;
        unsafe { ((*self.entries)[self.index], self.index) }
    }

    pub fn clear_entry(&mut self, pid: usize) {
        unsafe {
            (*self.entries)[pid] = None;
        }
    }
}


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

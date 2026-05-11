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

#[derive(Debug, Clone, Copy)]
struct ElectionVote {
    new_election: bool,
    term: usize,
    candidate_id: usize,
}

impl ElectionVote {
    fn init() -> Self {
        ElectionVote {
            new_election: false,
            term: 0,
            candidate_id: 0,
        }
    }
}

pub struct LoggerSharedState {
    election_board: [ElectionVote; MAX_PROCESSES],
    lrq: [Option<LogQueueEntry>; MAX_PROCESSES],
    lrq_index: usize,
}

impl LoggerSharedState {
    fn new() -> Self {
        LoggerSharedState {
            election_board: [ElectionVote::init(); MAX_PROCESSES],
            lrq: [None; MAX_PROCESSES],
            lrq_index: 0,
        }
    }
}

struct Logger {
    id: usize,
    cluster_size: usize,
    shmem: *mut LoggerSharedState,
}

impl Logger {
    pub fn new(id: usize, cluster_size: usize, shared_state_path: &str) -> Self {
    
        let min_size = 2 * 1024 * 1024; // min required for DAX mapping
        let mut size = std::mem::size_of::<Option<LoggerSharedState>>() * MAX_PROCESSES;
        size = if size < min_size {
            min_size
        } else {
            size
        };
        let ptr = super::mmap_daxdev(shared_state_path, size) as *mut LoggerSharedState;
        
        unsafe {
            (*ptr) = LoggerSharedState::new();
        }

        Logger {
            id,
            cluster_size,
            shmem: ptr,
        }
    }

    pub fn log_request(&mut self, wid: Wid, obj_info: ObjectInfo, pid: usize) {
        let entry = LogQueueEntry::new(wid, obj_info);
        unsafe {
            (*self.shmem).lrq[pid] = Some(entry);
        }

        // wait until the log thread processes the entry and clears it
        while unsafe { (*self.shmem).lrq[pid].is_some() } {
            std::thread::yield_now(); // Yield to allow log thread to process the entry
            // std::hint::spin_loop();
        }
    }

    pub fn poll_next_process_queue(&self) -> (Option<LogQueueEntry>, usize) {
        let shmem = unsafe { &mut *self.shmem };
        shmem.lrq_index = (shmem.lrq_index + 1) % MAX_PROCESSES;
        (shmem.lrq[shmem.lrq_index], shmem.lrq_index)
    }

    pub fn clear_process_queue(&mut self, pid: usize) {
        unsafe {
            (*self.shmem).lrq[pid] = None;
        }
    }

    /// Check if this process is the leader by counting votes in the election board.
    pub fn is_leader(&self) -> bool {

        let shmem = unsafe { &mut *self.shmem };

        let quorum = (self.cluster_size / 2) + 1;
        let mut votes = 0;
        for i in 0..self.cluster_size {
            if shmem.election_board[i].candidate_id == self.id {
                votes += 1;
            } 
            
            if votes >= quorum {
                return true;
            }
        }

        false
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

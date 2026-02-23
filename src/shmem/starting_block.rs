use crate::MAX_PROCESSES;
use std::time::SystemTime;

/// Shared fixed-size array indexed by process ID
#[derive(Debug, Clone, Copy)]
pub(crate) struct StartingBlock {
    start_time: Option<SystemTime>,
    ready_processes: [bool; MAX_PROCESSES],
}

impl StartingBlock {
    pub(crate) fn new() -> Self {
        StartingBlock {
            start_time: None,
            ready_processes: [false; MAX_PROCESSES],
        }
    }

    pub(crate) fn start_at(&mut self, time: SystemTime) {
        self.start_time = Some(time);
    }

    pub(crate) fn start_is_scheduled(&self) -> bool {
        self.start_time.is_some() && self.start_time.unwrap() > SystemTime::now()
    }

    pub(crate) fn get_start_time(&self) -> Option<SystemTime> {
        self.start_time
    }

    pub(crate) fn mark_ready(&mut self, pid: usize) {
        if pid < MAX_PROCESSES {
            self.ready_processes[pid] = true;
        } else {
            panic!("Process ID {} exceeds MAX_PROCESSES {}", pid, MAX_PROCESSES);
        }
    }

    pub(crate) fn all_ready(&self, processes: Vec<u32>) -> bool {
        processes.iter().all(|&pid| self.ready_processes[pid as usize])
    }
}

use crate::{MAX_OBJECTS, MAX_PROCESSES};

// @TODO: better data structure for tracking write conflicts
// use fixed-size hashmap
#[derive(Debug, Clone, Copy)]
pub(crate) struct WriteConflictReferee {
    requests: [(usize, ([usize; MAX_PROCESSES], usize)); MAX_OBJECTS], // (req_id, (array of process ids, current size))
    requests_count: usize,
}

impl WriteConflictReferee {
    pub(crate) fn new() -> Self {
        WriteConflictReferee {
            requests: [(0, ([0; MAX_PROCESSES], 0)); MAX_OBJECTS],
            requests_count: 0,
            // conflicts: [(0, ([0; MAX_PROCESSES], 0)); MAX_OBJECTS],
            // conflicts_count: 0,
        }
    }

    pub(crate) fn push_request(&mut self, req_id: usize, pid: usize) {
        for i in 0..self.requests_count {
            if self.requests[i].0 == req_id {
                // found existing request
                let pids_of_request = &mut self.requests[i].1 .0;
                let array_len = &mut self.requests[i].1 .1;
                for j in 0..*array_len {
                    // don't add pid to list if the same process has already requested a write to the same location
                    if pids_of_request[j] == pid {
                        return; // already requested
                    }
                }
                // add conflicts
                pids_of_request[*array_len] = pid;
                *array_len += 1;
                return;
            }
        }
    }

    pub(crate) fn check_conflicts(&self, req_id: usize) -> Option<Vec<usize>> {
        for i in 0..self.requests_count {
            if self.requests[i].0 == req_id {
                let array_len = self.requests[i].1 .1;
                if array_len > 1 {
                    let conflicting_pids = &self.requests[i].1 .0;
                    return Some(conflicting_pids[0..array_len].to_vec());
                }
            }
        }
        None
    }
}

// struct WriteConflictReferee {
//     requests: HashMap<usize, Vec<usize>>, // K = req_id, processes
//     conflicts: HashMap<usize, Vec<usize>>, // K = req_id, processes
//     conflicts_count: usize,
// }

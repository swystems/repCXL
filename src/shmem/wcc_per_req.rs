use crate::{MAX_OBJECTS, MAX_PROCESSES};

/// Write Conflict Checker (WCC) register to solve write conflicts
///
/// Uses a fixed-size hashmap with linear probing to store write requests.
/// Fixed-size is required to avoid overwriting object space in memory nodes.
/// Each entry maps a request ID to an array of process IDs that have requested
/// a write to that location.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WriteConflictChecker {
    // fixed-size hashmap: key = req_id, value = (array of pids, number of pids).
    // 2D array to handle collisions with linear probing
    requests: [[(usize, ([usize; MAX_PROCESSES], usize)); MAX_OBJECTS]; MAX_OBJECTS],
}

impl WriteConflictChecker {
    pub(crate) fn new() -> Self {
        WriteConflictChecker {
            requests: [[(0, ([0; MAX_PROCESSES], 0)); MAX_OBJECTS]; MAX_OBJECTS],
        }
    }

    fn hash(&self, key: usize) -> usize {
        key % MAX_OBJECTS
    }

    fn get_request(&mut self, key: usize) -> Option<&mut ([usize; MAX_PROCESSES], usize)> {
        let index = self.hash(key);
        for i in 0..MAX_OBJECTS {
            if self.requests[index][i].0 == key {
                return Some(&mut self.requests[index][i].1);
            }
            if self.requests[index][i].0 == 0 {
                return None; // empty slot means key not present
            }
        }
        None
    }

    pub(crate) fn push_request(&mut self, req_id: usize, pid: usize) {
        if let Some((pids_of_request, num_of_pids)) = self.get_request(req_id) {
            // found existing request
            for j in 0..*num_of_pids {
                // don't add pid to list if the same process has already requested a write to the same location
                if pids_of_request[j] == pid {
                    return; // already requested
                }
            }
            // add conflicting pids
            pids_of_request[*num_of_pids] = pid;
            *num_of_pids += 1;
            return;
        } else {
            // add new one
            let index = self.hash(req_id);
            self.requests[index][0] = (req_id, ([pid; MAX_PROCESSES], 1));
        }
    }

    pub(crate) fn check_conflicts(&mut self, req_id: usize) -> Option<Vec<usize>> {
        if let Some((conflicting_pids, num_of_pids)) = self.get_request(req_id) {
            if *num_of_pids > 1 {
                return Some(conflicting_pids[0..*num_of_pids].to_vec());
            }
        }
        None
    }

    pub(crate) fn clear(&mut self) {
        self.requests = [[(0, ([0; MAX_PROCESSES], 0)); MAX_OBJECTS]; MAX_OBJECTS];
    }
}

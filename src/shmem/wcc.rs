use crate::{MAX_OBJECTS, MAX_PROCESSES};

/// Write Conflict Checker (WCC) register to solve write conflicts
#[derive(Debug, Clone, Copy)]
pub(crate) struct WCC {
    // array of round values indexed by process ID
    p_round: ([u64; MAX_PROCESSES])
}

impl WCC {
    fn new() -> Self {
        WCC {
            p_round: [0; MAX_PROCESSES],
        }
    }

    fn write(&mut self, round: u64, pid: usize) {
        if pid > MAX_PROCESSES {
            return; // invalid pid
        }
        self.p_round[pid] = round;
    }

    /// Check if the given process is the last writer for the given round.
    /// Last writer criteria: 
    /// - the winning process has written in the highest round smaller than
    /// the current round
    /// - in case of conflicts, the smallest pid wins
    fn is_last(&self, current_round:u64, round: u64, pid: usize) -> bool {
        if pid > MAX_PROCESSES {
            return false; // invalid pid
        }

        for i in 0..MAX_PROCESSES {
            if current_round > self.p_round[i] && self.p_round[i] > round {
                return false; // another process has written in the same or a later round
            }
            if self.p_round[i] == round && i < pid {
                return false; // another process has lower pid
            }
        }
        self.p_round[pid] == round
    }
}



#[derive(Debug, Clone, Copy)]
pub(crate) struct WCCMultiObject {
    // fixed-size hashmap: key = req_id, value = (array of pids, number of pids).
    // 2D array to handle collisions with linear probing
    index_oids: [usize; MAX_OBJECTS],
    objects: [WCC; MAX_OBJECTS],
    objects_count: usize,
}

impl WCCMultiObject {
    pub(crate) fn new() -> Self {
        WCCMultiObject {
            index_oids: [0; MAX_OBJECTS],
            objects: [WCC::new(); MAX_OBJECTS],
            objects_count: 0,
        }
    }

    fn add_object(&mut self, oid: usize) {
        self.index_oids[self.objects_count] = oid;
        self.objects_count += 1;
    }

    fn get_object_wcc(&mut self, oid: usize) -> Option<&mut WCC> {
        for i in 0..MAX_OBJECTS {
            if self.index_oids[i] == oid {
                return Some(&mut self.objects[i]);
            }
        }
        None // object id not found
    }

    pub(crate) fn clear(&mut self) {
        self.objects = [WCC::new(); MAX_OBJECTS];
    }
}

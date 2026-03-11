#![allow(dead_code)]
use core::panic;

use super::{MAX_OBJECTS, MAX_PROCESSES};
use crate::safe_memio;
use crate::safe_memio::{CACHE_LINE_SIZE,clflushopt};

/// Write Conflict Checker (WCC) register to solve write conflicts
#[derive(Debug, Clone, Copy)]
pub(crate) struct WCC {
    // array of round values indexed by process ID
    p_round: [u64; MAX_PROCESSES]
}

impl WCC {
    pub fn new() -> Self {
        WCC {
            p_round: [0; MAX_PROCESSES],
        }
    }

    pub fn write(&mut self, round: u64, pid: usize) {
        if pid >= MAX_PROCESSES {  
            return; // invalid pid
        }
        self.p_round[pid] = round;
    }

    /// Check if the given process is the last writer for the given round.
    /// Last writer criteria: 
    /// - the winning process has written in the highest round smaller than
    /// the current round
    /// - in case of conflicts, the smallest pid wins
    pub fn is_last(&self, current_round:u64, round: u64, pid: usize) -> bool {
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

    pub fn get_object_wcc(&mut self, oid: usize) -> Option<&mut WCC> {
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


/// entry for ObjectWCC
/// contains object ID, round
#[derive(Debug, Clone, Copy)]
struct ObjectWCCEntry {
    oid: usize,
    round: u64,
}

impl ObjectWCCEntry {
    pub fn new(oid: usize, round: u64) -> Self {
        ObjectWCCEntry { oid, round }
    }
}

/// multi-object WCC with smaller memory footprint
#[derive(Debug, Clone, Copy)]
pub(crate) struct ObjectWCC {
    p_round: [ObjectWCCEntry; MAX_PROCESSES] // array of ObjectWCCEntry indexed by process ID
}

impl ObjectWCC {
    pub fn new() -> Self {
        ObjectWCC {
            p_round: [ObjectWCCEntry::new(0, 0); MAX_PROCESSES],
        }
    }

    pub fn write(&mut self, oid: usize, round: u64, pid: usize) {
        if pid >= MAX_PROCESSES {  
            return; // invalid pid
        }
        let entry = ObjectWCCEntry::new(oid, round);
        safe_memio::mem_write(&mut self.p_round[pid], entry);
    }

    /// Check if the given process is the last writer for the given object.
    /// 
    /// Last writer criteria: 
    /// - the winning process has written in the largest round smaller than
    /// the current round
    /// - in case of conflicts, the smallest pid wins
    pub fn is_last(&mut self, oid_in: usize, current_round:u64, round_in: u64, pid_in: usize) -> bool {
        if pid_in > MAX_PROCESSES {
            return false; // invalid pid
        }

        // single bulk flush of the entire p_round array + mfence, then
        // read_volatile per entry (avoids 128 individual flushes)
        unsafe {
            safe_memio::cache_flush_read(
                self.p_round.as_ptr() as *const u8,
                std::mem::size_of::<[ObjectWCCEntry; MAX_PROCESSES]>(),
            );
        }

        for i in 0..MAX_PROCESSES {
            let entry = unsafe { std::ptr::read_volatile(&self.p_round[i]) };
            // check only entries for the same object ID
            if entry.oid != oid_in {
                continue;
            }

            if current_round > entry.round && entry.round > round_in {
                return false; // another process has written in a larger round
            }
            if entry.round == round_in && i < pid_in {
                return false; // another process has lower pid
            }
        }
        true
    }
}

/// Bitmap to track processes that have written to an object for FastWCC implementation.
/// Assumes cache line of 64bytes, uses u64 + flush + mfence to ensure visibility
/// of CXL 2.0 read/writes across hosts.
#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy)]
struct ProcessBitmapUncached {
    data: [u64; MAX_PROCESSES / CACHE_LINE_SIZE],
}

impl ProcessBitmapUncached {
    fn new() -> Self {
        ProcessBitmapUncached { data: [0; MAX_PROCESSES / CACHE_LINE_SIZE] }
    }

    fn set(&mut self, pid: usize) {
        let byte_index = pid / CACHE_LINE_SIZE;
        let bit_index = pid % CACHE_LINE_SIZE;
        self.data[byte_index] |= 1 << bit_index;
        unsafe {
            clflushopt(self.data[byte_index] as *const u8);
            core::arch::x86_64::_mm_sfence();
        }
    }

    fn is_smallest(&self, pid: usize) -> bool {
        let byte_index = pid / CACHE_LINE_SIZE;
        let bit_index = pid % CACHE_LINE_SIZE;
        // Check if any lower bit is set
        for i in 0..byte_index {
            if self.data[i] != 0 {
                return false; // another process with smaller pid has written
            }
        }
        // Check bits in the same byte
        let mask = (1 << bit_index) - 1; // Mask for bits lower than bit_index
        (self.data[byte_index] & mask) == 0
    }

}

pub struct FastWCC {
    obm: [ProcessBitmapUncached; MAX_OBJECTS] // object bitmaps
}

impl FastWCC {
    fn new() -> Self {
        FastWCC { obm: [ProcessBitmapUncached::new(); MAX_OBJECTS] }
    }

    pub fn write(&mut self, oid: usize, pid: usize) {
        if oid >= MAX_OBJECTS || pid >= MAX_PROCESSES {
            panic!("Invalid object ID or process ID");
        }
        self.obm[oid].set(pid);
    }

    pub fn is_last(&mut self, oid: usize, pid: usize) -> bool {
        if oid >= MAX_OBJECTS || pid >= MAX_PROCESSES {
            panic!("Invalid object ID or process ID");
        }
        
        // Check if any other process has written to the same object
        self.obm[oid].is_smallest(pid)
    }
}

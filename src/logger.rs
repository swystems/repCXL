use crate::RepCXLConfig;
use crate::shmem::{MemoryNode, mmap_daxdev, MAX_PROCESSES};
use crate::shmem::object_index::ObjectInfo;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use crate::safe_memio;
use crate::request::Wid;

// whether to sleep in the follower loop to reduce CPU usage (at the cost of slower failure detection)
const ELECTION_SLEEP: bool = true; 

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
    process_election_trigger: [bool; MAX_PROCESSES], // List of flags to trigger election for each process
    election_board: [ElectionVote; MAX_PROCESSES],
    lrq: [Option<LogQueueEntry>; MAX_PROCESSES],
    lrq_index: usize,
}

impl LoggerSharedState {
    fn new() -> Self {
        LoggerSharedState {
            process_election_trigger: [false; MAX_PROCESSES],
            election_board: [ElectionVote::init(); MAX_PROCESSES],
            lrq: [None; MAX_PROCESSES],
            lrq_index: 0,
        }
    }
}

/// Logger shared-memory interface used for logger-logger and logger-RepCXL 
/// communication. 
pub struct LoggerInterface {
    cluster_size: usize,
    shmem: *mut LoggerSharedState,
}

impl LoggerInterface {

    pub fn new(config: &RepCXLConfig) -> Self {
        let min_size = 2 * 1024 * 1024; // min required for DAX mapping
        let mut size = std::mem::size_of::<LoggerSharedState>();
        size = if size < min_size {
            min_size
        } else {
            size
        };
        let ptr = mmap_daxdev(config.log_node.as_str(), size) as *mut LoggerSharedState;
        
        unsafe {
            (*ptr) = LoggerSharedState::new();
        }

        // logger id = repcxl id, inherited from the repcxl instance that creates
        // the logger 
        LoggerInterface {
            cluster_size: config.logger_cluster_size,
            shmem: ptr,
        }
    }

    /// [for RepCXL processes] Send a log request to the logger thread queue 
    /// and wait for it to be processed. 
    pub fn log_request(&mut self, wid: Wid, obj_info: ObjectInfo, pid: usize) {
        let entry = LogQueueEntry::new(wid, obj_info);
        unsafe {
            let mut lrq = &mut (*self.shmem).lrq[pid];
            *lrq = Some(entry);
            
            safe_memio::cache_flush_write(
                lrq as *const Option<LogQueueEntry> as *const u8, 
                std::mem::size_of::<Option<LogQueueEntry>>()
            );

            // wait until the log thread processes the entry and clears it
            while lrq.is_some() {

                std::thread::yield_now(); // Yield to allow log thread to process the entry

                safe_memio::cache_flush_read(
                    lrq as *const Option<LogQueueEntry> as *const u8, 
                    std::mem::size_of::<Option<LogQueueEntry>>()
                );  
                lrq = &mut (*self.shmem).lrq[pid];
                
            }
        }
    }

    /// [for RepCXL processes]: trigger a new election by candidating a random logger
    /// process
    pub fn start_new_election(&mut self) {
        
        // select a random logger process in the logger cluster
        let candidate_id = rand::random::<u32>() as usize % self.cluster_size;
        
        unsafe {
            let election_trigger = &mut (*self.shmem).process_election_trigger[candidate_id];
            *election_trigger = true;

            safe_memio::cache_flush_write(
                election_trigger as *const bool as *const u8, 
                std::mem::size_of::<bool>()
            );
        }
    }

    fn poll_next_process_queue(&self) -> (Option<LogQueueEntry>, usize) {
        
        unsafe {
            let shmem = &mut *self.shmem;
        
            // get shared index from memory
            safe_memio::cache_flush_read(
                &shmem.lrq_index as *const usize as *const u8, 
                std::mem::size_of::<usize>()
            );
            // update
            shmem.lrq_index = (shmem.lrq_index + 1) % MAX_PROCESSES;
            // push updated index to memory
            safe_memio::cache_flush_write(
                &shmem.lrq_index as *const usize as *const u8, 
                std::mem::size_of::<usize>()
            );

            // get log queue entry from memory
            safe_memio::cache_flush_read(
                &shmem.lrq[shmem.lrq_index] as *const Option<LogQueueEntry> as *const u8, 
                std::mem::size_of::<Option<LogQueueEntry>>()
            );
        (shmem.lrq[shmem.lrq_index], shmem.lrq_index)
        }
        
    }

    fn clear_process_queue(&mut self, pid: usize) {
        unsafe {
            // get process queue
            let lrq = &mut (*self.shmem).lrq[pid];
            // clear it
            *lrq = None;
            // push results to memory
            safe_memio::cache_flush_write(
                lrq as *const Option<LogQueueEntry> as *const u8, 
                std::mem::size_of::<Option<LogQueueEntry>>()
            );
        }
    }

    /// Check if this process is the leader by counting votes in the election board.
    fn is_leader(&self, pid: usize) -> bool {

        let election_board = unsafe { &mut (*self.shmem).election_board };

        let quorum = (self.cluster_size / 2) + 1;
        let mut votes = 0;
        
        // flush the election board to ensure read from memory
        unsafe {
            safe_memio::cache_flush_read(
                election_board as *const ElectionVote as *const u8, 
                std::mem::size_of::<ElectionVote>() * self.cluster_size
            );
        }

        for i in 0..self.cluster_size {
            if election_board[i].candidate_id == pid {
                votes += 1;
            } 
            
            if votes >= quorum {
                return true;
            }
        }

        false
    }


    /// Check if a a candidate started a new election and return its vote. In case
    /// of multiple candidates, return the one with the higher term or, in case of 
    /// tie, smaller candidate ID.
    fn check_new_election(&self) -> Option<ElectionVote> {
        let election_board = unsafe { &mut (*self.shmem).election_board };

        // flush the election board to ensure read from memory
        unsafe {
            safe_memio::cache_flush_read(
                election_board as *const ElectionVote as *const u8, 
                std::mem::size_of::<ElectionVote>() * self.cluster_size
            );
        }

        let mut best: Option<ElectionVote> = None;
        for i in 0..self.cluster_size {
            let vote = election_board[i];
            if !vote.new_election {
                continue;
            }

            best = match best {
                None => Some(vote),
                Some(current_best) => {
                    if vote.term > current_best.term
                        || (vote.term == current_best.term
                            && vote.candidate_id < current_best.candidate_id)
                    {
                        Some(vote)
                    } else {
                        Some(current_best)
                    }
                }
            };
        }

        best
    }

    fn vote(&mut self, pid: usize, vote: ElectionVote, ) {
        let election_board = unsafe { &mut (*self.shmem).election_board };
        election_board[pid] = vote;
        // write vote to memory
        unsafe {
            safe_memio::cache_flush_write(
                &mut election_board[pid] as *mut ElectionVote as *const u8, 
                std::mem::size_of::<ElectionVote>()
            );
        }
    }

    fn has_been_nominated(&self, lid: usize) -> bool {
        unsafe {
            let process_election_trigger = &(*self.shmem).process_election_trigger[lid];

            // flush the election trigger list to ensure read from memory
            safe_memio::cache_flush_read(
                process_election_trigger as *const bool as *const u8, 
                std::mem::size_of::<bool>()
            );

            *process_election_trigger
        }
    }

    fn uncandidate(&self, lid: usize) {
        unsafe {
            // clear election trigger
            let process_election_trigger = &mut (*self.shmem).process_election_trigger[lid];
            *process_election_trigger = false;

            // flush the election trigger list to ensure read from memory
            safe_memio::cache_flush_write(
                process_election_trigger as *const bool as *const u8, 
                std::mem::size_of::<bool>()
            );

            // terminate the election
            let my_vote = &mut (*self.shmem).election_board[lid];
            my_vote.new_election = false;

            safe_memio::cache_flush_write(
                my_vote as *const ElectionVote as *const u8, 
                std::mem::size_of::<ElectionVote>()
            );
        }
    }

}


/// Check if a log entry is still dirty and return the dirty value if it exists.
/// This condition is evaluated when the value of log entry still exists some memory 
/// nodes, not all, still contain it
fn check_dirty<T: Copy>(memory_nodes: &Vec<MemoryNode<T>>, entry: &LogQueueEntry) -> Option<T> {
    match safe_memio::mem_readends(entry.obj_info.offset, memory_nodes) {
        Ok(states) => {

            // check if consistent
            if states[0].wid == states[1].wid {return None;} 
            // check if the dirty value has not been overwritten by a new write
            if states[0].wid == entry.wid {return Some(states[0].value);}
            if states[1].wid == entry.wid {return Some(states[1].value);}

            None
        },
        Err(safe_memio::MemoryError(e)) => { 
            log::error!("Failed to read object state for obj {} in memory node {}", 
                entry.obj_info.id, 
                e);
            None
        }
     }
}

/// Replicated logger protocol. Uses a static set of logger processes with 
/// Raft-like leader election on shared memory to overcome failures of logger
/// processes.  
/// 
/// - A leader check if it's still the leader
/// - If it is, it polls the log request queue of the next repCXL process 
/// and, if present, logs it to all memory nodes and clears the queue entry.
/// - (TODO) RepCXL processes trigger election on log_request taking too long
/// using randomized timouts and picking a random candidate
/// - Other followers periodically read the election board and update
/// their vote to the latest leader candidate they see
/// - (TODO) When the quorum is reached, the new leader replicates the log request
/// on the _next_ available index to avoid the old leader to overwrite the log entry
/// We don't care about empty slots in the log, repcxl processes read the entire
/// log when recovery and discard empty entries
pub fn run<T: Copy>(lid: usize, config: RepCXLConfig, stop_flag: Arc<AtomicBool>) {

    std::thread::spawn(move || {

        let mut memory_nodes = Vec::new();

        // open memory nodes (same as repCXL main thread)
        for path in &config.mem_nodes {
            let mnid = memory_nodes.len();
            let node = MemoryNode::<T>::from_file(mnid, &path, config.mem_size);
            memory_nodes.push(node);
        }

        // open log request queue
        let mut lif = LoggerInterface::new(&config);
        // initial vote is always the default one (leader = pid0), no need to read
        let mut my_vote = ElectionVote::init();

        let mut running_for_election = false;
        // let lid = config.id as usize; // logger id = repcxl id

        loop {
            // stop with algorithms threads on rep_cxl.stop()
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }

            // LEADER logic
            if lif.is_leader(lid) {
                
                // p just became a leader
                if running_for_election {
                    log::debug!("[logger-election] {}: Election won, now leader for term {}", lid, my_vote.term);
                    running_for_election = false;
                    lif.uncandidate(lid);
                } 
                
                // p is (and was) the current active leader
                if !running_for_election {
                // read log entry from queue
                    if let (Some(entry), pid) = lif.poll_next_process_queue() {
                        log::debug!("[logger-election]: Processing log queue for obj {} from pid {}", 
                            entry.obj_info.id, pid);
                        if let Some(v) = check_dirty(&memory_nodes, &entry) {
                            for node in &memory_nodes {
                                // get log
                                let log = node.get_state().get_log();
                                // appnd to log
                                log.append(entry.wid, entry.obj_info, v);
                            }
                            log::debug!("[logger-election]: Appended dirty value to logs for obj {}", entry.obj_info.id);
                        } else {
                            log::debug!("[logger-election]: No dirty value found for obj {} - clearing entry", entry.obj_info.id);
                        }

                        // In all cases clear the queue entry so waiting processes won't hang.
                        lif.clear_process_queue(pid);
                        log::debug!("[logger-election]: Cleared log queue entry");
                    }
    
                }

                continue; // skip follower logic if active leader                

            }
            
            // FOLLOWER logic
            if let Some(candidate_vote) = lif.check_new_election() {
                
                log::debug!("[logger-election] {}: Detected election for candidate {}, term {}", 
                    lid, 
                    candidate_vote.candidate_id, 
                    candidate_vote.term
                );

                // note: could be self, check_new_election returns the best
                // candidate to vote for in the whole election board
                my_vote = candidate_vote;
                lif.vote(lid, my_vote);

            }
            if lif.has_been_nominated(lid) {
                log::debug!("[logger-election] starting election for logger {} with term {}", lid, my_vote.term + 1);
                // start election
                my_vote.new_election = true;
                // vote for self
                my_vote.term += 1;
                my_vote.candidate_id = lid;
                lif.vote(lid, my_vote);
                running_for_election = true;
            }
    
            if ELECTION_SLEEP {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

        }
    });
}

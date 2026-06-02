use crate::RepCXLConfig;
use crate::shmem::{MemoryNode, mmap_daxdev, MAX_PROCESSES};
use crate::shmem::object_index::ObjectInfo;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use crate::safe_memio;
use crate::request::Wid;
use crate::utils;

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

/// Per-process struct to vote for new log-index consensus proposals and
/// new leader elections.
#[derive(Debug, Clone, Copy)]
struct ProcessVote {
    log_index: usize,
    term: usize,
    candidate_id: usize,
}

impl ProcessVote {
    fn init() -> Self {
        ProcessVote {
            log_index: 0,
            term: 0,
            candidate_id: 0,
        }
    }
}
impl PartialEq for ProcessVote {
    fn eq(&self, other: &Self) -> bool {
        self.log_index == other.log_index &&
        self.term == other.term &&
        self.candidate_id == other.candidate_id
    }
}

/// Shared state of the logger cluster accessed by all loggers. 
/// Used for logger-logger and logger-RepCXL communication
pub struct LoggerSharedState {
    
    /// List of flags to trigger election for each process
    process_election_trigger: [bool; MAX_PROCESSES], 
    
    /// Board to store votes for consensus and leader election proposals.
    /// Equivalent to a broadcast channel in message-passing systems 
    consensus_board: [ProcessVote; MAX_PROCESSES],  
    
    /// Log request queue for each RepCXL process to send log requests to 
    /// the logger cluster. Implemented as a SPSC ring-buffer where each process has 
    /// a dedicated slot to avoid contention and false sharing (we cannot use
    /// locks to to lack of atomic operations).
    lrq: [Option<LogQueueEntry>; MAX_PROCESSES],

    /// Log reqeuest queue index. There might be contention here but we 
    /// don't care if we skip an entry and it will eventually be processed in 
    /// in the following rounds
    lrq_index: usize,
}

impl LoggerSharedState {
    fn new() -> Self {
        LoggerSharedState {
            process_election_trigger: [false; MAX_PROCESSES],
            consensus_board: [ProcessVote::init(); MAX_PROCESSES],
            lrq: [None; MAX_PROCESSES],
            lrq_index: 0,
        }
    }
}

/// Logger shared-memory interface stored in a CXL DAX-mapped memory region.
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
        let ptr = mmap_daxdev(config.logger_node.as_str(), size) as *mut LoggerSharedState;
        
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


    /// [for logger threads] Poll the next log request from the queue to be
    /// processed.
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

    /// [for logger threads] Clear the log request queue entry after processing it
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

    /// [for logger threads] check if this process is the leader by counting votes 
    /// in the consensus board.
    fn is_leader(&self, pid: usize) -> bool {

        let consensus_board = unsafe { &mut (*self.shmem).consensus_board };

        let quorum = (self.cluster_size / 2) + 1;
        let mut votes = 0;
        
        // flush the consensus board to ensure read from memory
        unsafe {
            safe_memio::cache_flush_read(
                consensus_board as *const ProcessVote as *const u8, 
                std::mem::size_of::<ProcessVote>() * self.cluster_size
            );
        }

        for i in 0..self.cluster_size {
            if consensus_board[i].candidate_id == pid {
                votes += 1;
            } 
            
            if votes >= quorum {
                return true;
            }
        }

        false
    }

    /// [for logger threads] Propose a log index for the current log entry to be
    /// appended to the log. Exclusive use by a single leader at a time guarantees
    /// Termination, in Raft this is only probabilistic
    fn propose_log_index(&self, id: usize, log_index: usize) {
        let consensus_board = unsafe { &mut (*self.shmem).consensus_board };

        consensus_board[id].log_index = log_index;

        // flush the consensus board to memory
        unsafe {
            safe_memio::cache_flush_write(
                &consensus_board[id].log_index as *const usize as *const u8, 
                std::mem::size_of::<usize>()
            ); 
        }
    }


    /// [for logger threads] Check if a quorum of votes is reached 
    /// for the _latest_ vote, i.e., highest term with highest log index. Used
    /// for both consensus and leader election. In the first case, the proposal
    /// is always the next log index.
    /// 
    /// Returns (quorum_reached, vote_with_highest_term)
    fn check_quorum(&self) -> (bool, ProcessVote) {
        let consensus_board = unsafe { &mut (*self.shmem).consensus_board };

        // flush the consensus board to ensure read from memory
        unsafe {
            safe_memio::cache_flush_read(
                consensus_board as *const ProcessVote as *const u8, 
                std::mem::size_of::<ProcessVote>() * self.cluster_size
            );
        }


        let mut quorum = 1; // count self vote
        let mut vote_with_highest_term = consensus_board[0];

        for i in 1..self.cluster_size {
            if consensus_board[i] == vote_with_highest_term {
                quorum += 1;
            }
            else if consensus_board[i].term > vote_with_highest_term.term {
                vote_with_highest_term = consensus_board[i];
                quorum = 1;
            }
            else if consensus_board[i].term == vote_with_highest_term.term && 
                    consensus_board[i].log_index > vote_with_highest_term.log_index {
                vote_with_highest_term = consensus_board[i];
                quorum = 1;
            }

        }

        let quorum_reached = quorum >= (self.cluster_size / 2) + 1;
        return (quorum_reached, vote_with_highest_term);

    }

    /// [for logger threads] Get the maximum log index committed in the cluster.
    fn max_log_index(&self) -> usize {
        let consensus_board = unsafe { &mut (*self.shmem).consensus_board };

        // flush the consensus board to ensure read from memory
        unsafe {
            safe_memio::cache_flush_read(
                consensus_board as *const ProcessVote as *const u8, 
                std::mem::size_of::<ProcessVote>() * self.cluster_size
            );
        }

        let mut max_log_index = 0;
        for i in 0..self.cluster_size {
            if consensus_board[i].log_index > max_log_index {
                max_log_index = consensus_board[i].log_index;
            }
        }
        max_log_index
    }


    /// [for logger threads] Check if a a candidate started by setting a term 
    /// larger than the current term. In case of multiple candidates, return the
    /// one with the higher term or, in case of tie, smaller candidate ID.
    fn check_new_election(&self, term: usize) -> Option<ProcessVote> {
        let consensus_board = unsafe { &mut (*self.shmem).consensus_board };

        // flush the consensus board to ensure read from memory
        unsafe {
            safe_memio::cache_flush_read(
                consensus_board as *const ProcessVote as *const u8, 
                std::mem::size_of::<ProcessVote>() * self.cluster_size
            );
        }

        let mut best: Option<ProcessVote> = None;
        for i in 0..self.cluster_size {
            let vote = consensus_board[i];
            if vote.term > term {

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
        }
        best
    }

    /// [for logger threads] Vote for a candidate and/or a log index in the 
    /// consensus board.
    fn vote(&mut self, pid: usize, vote: ProcessVote ) {
        let consensus_board = unsafe { &mut (*self.shmem).consensus_board };
        consensus_board[pid] = vote;
        // write vote to memory
        unsafe {
            safe_memio::cache_flush_write(
                &mut consensus_board[pid] as *mut ProcessVote as *const u8, 
                std::mem::size_of::<ProcessVote>()
            );
        }
    }

    /// [for logger processes] Check if a RepCXL process has triggered an election 
    /// for the given logger process through start_new_election()
    fn election_request(&self, logger_id: usize) -> bool {
        unsafe {
            let process_election_trigger = &(*self.shmem).process_election_trigger[logger_id];

            // flush the election trigger list to ensure read from memory
            safe_memio::cache_flush_read(
                process_election_trigger as *const bool as *const u8, 
                std::mem::size_of::<bool>()
            );

            *process_election_trigger
        }
    }

    /// [for logger processes] Clear the election request for the given logger 
    /// process after processing it
    fn clear_election_request(&self, logger_id: usize) {
        unsafe {
            // clear election trigger
            let process_election_trigger = &mut (*self.shmem).process_election_trigger[logger_id];
            *process_election_trigger = false;

            // flush the election trigger list to ensure read from memory
            safe_memio::cache_flush_write(
                process_election_trigger as *const bool as *const u8, 
                std::mem::size_of::<bool>()
            );

            // terminate the election
            let my_vote = &mut (*self.shmem).consensus_board[logger_id];
            // my_vote.new_election = false;

            safe_memio::cache_flush_write(
                my_vote as *const ProcessVote as *const u8, 
                std::mem::size_of::<ProcessVote>()
            );
        }
    }

}


/// Check if a log entry is still dirty and return the dirty value if it exists.
/// This condition is evaluated when the value of log entry still exists some memory 
/// nodes, not all, still contain it
fn check_dirty<T: Copy + PartialEq>(memory_nodes: &Vec<MemoryNode<T>>, entry: &LogQueueEntry) -> Option<T> {
    match safe_memio::mem_readends(entry.obj_info.offset, memory_nodes) {
        Ok(states) => {

            // check if consistent
            if states[0] == states[1] {return None;}

            // check if the dirty value has not been overwritten by a new write
            // if states[0].wid == entry.wid {return Some(states[0].value);}
            // if states[1].wid == entry.wid {return Some(states[1].value);}
            // None

            // return the value of the latest wid
            if states[0].wid > states[1].wid {
                return Some(states[0].value);
            } else {
                return Some(states[1].value);
            }

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
/// Assumptions: 
/// - quorum of logger processes is correct
/// - memory node containing the LoggerInterface (communication channel)
/// must not fail or safety is affected
/// 
/// Protocol:
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
pub fn _run_fast_passive_consensus<T: Copy + PartialEq>(logger_id: usize, config: RepCXLConfig, stop_flag: Arc<AtomicBool>) {

    std::thread::spawn(move || {

        utils::set_core_affinity(&config, true);

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
        let mut my_vote = ProcessVote::init();

        let mut running_for_election = false;
        // let leader_id = 0;

        let mut logger_latency_total = 0;
        let mut logger_latency_count = 0;
        loop {
            // stop with algorithms threads on rep_cxl.stop()
            if stop_flag.load(Ordering::Relaxed) {
                if logger_latency_count > 0 {
                    log::info!("Average logger latency: {}", utils::fmt_ns(logger_latency_total / logger_latency_count));
                }
                break;
            }

            // LEADER logic
            if lif.is_leader(logger_id) {
                
                // p just became a leader
                if running_for_election {
                    log::debug!("[election] {}: Election won, now leader for term {}", logger_id, my_vote.term);
                    running_for_election = false;
                    // lif.uncandidate(logger_id);
                } 
                
                // p is (and was) the current active leader
                if !running_for_election {
                    
                    // read log entry from queue
                    if let (Some(entry), pid) = lif.poll_next_process_queue() {
                        let logger_latency = std::time::Instant::now();
                        log::debug!("[leader]: Processing log queue for obj {} from pid {}", 
                            entry.obj_info.id, pid);
                        if let Some(v) = check_dirty(&memory_nodes, &entry) {
                            for node in &memory_nodes {
                                // get log
                                let log = node.get_state().get_log();
                                // append to log
                                log.append(entry.wid, entry.obj_info, v);
                            }
                            log::debug!("[leader]: Appended dirty value to logs for obj {}", entry.obj_info.id);
                        } else {
                            log::debug!("[leader]: No dirty value found for obj {} - clearing entry", entry.obj_info.id);
                        }

                        // In all cases clear the queue entry so waiting processes won't hang.
                        lif.clear_process_queue(pid);
                        log::debug!("[leader]: Cleared log queue entry");
                        logger_latency_total += logger_latency.elapsed().as_nanos() as u64;
                        logger_latency_count += 1;
                    }
    
                }

                continue; // skip follower logic if active leader                

            }
            
            // FOLLOWER logic
            if let Some(candidate_vote) = lif.check_new_election(my_vote.term) {
                
                log::debug!("[election] {}: Detected election for candidate {}, term {}", 
                    logger_id, 
                    candidate_vote.candidate_id, 
                    candidate_vote.term
                );

                // note: could be self, check_new_election returns the best
                // candidate to vote for in the whole election board
                my_vote = candidate_vote;
                lif.vote(logger_id, my_vote);

            }
            if lif.election_request(logger_id) {
                log::debug!("[election] {}: Starting election for candidate {}, term {}", 
                    logger_id, 
                    my_vote.candidate_id, 
                    my_vote.term + 1);
                // start election
                // my_vote.new_election = true;
                // vote for self
                my_vote.term += 1;
                my_vote.candidate_id = logger_id;
                lif.vote(logger_id, my_vote);
                running_for_election = true;
            }
    
            if ELECTION_SLEEP {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

        }
    });
}



/// Replicated logger protocol. Uses a static set of logger processes with 
/// shared-memory Raft.
/// 
/// Assumptions: 
/// - majority of non faulty logger processes. 
/// - Any number of memory nodes can fail including the one containing LoggerInterface
/// 
/// Protocol (Raft with some modifications):
/// - shared memory communication
/// - Leader polls for RepCXL log requests and proposes the next log index for 
/// the log entry to be appended to the log
/// - Followers vote for proposed log index and start new election.
/// 
/// TODO:
/// - test election / recovery
/// - randomized timeouts 
pub fn run_raft<T: Copy + PartialEq>(logger_id: usize, config: RepCXLConfig, stop_flag: Arc<AtomicBool>) {

    std::thread::spawn(move || {

        utils::set_core_affinity(&config, true);

        let mut memory_nodes = Vec::new();

        // open memory nodes (same as repCXL main thread)
        for path in &config.mem_nodes {
            let mnid = memory_nodes.len();
            let node = MemoryNode::<T>::from_file(mnid, &path, config.mem_size);
            memory_nodes.push(node);
        }

        // open log interface for communication with RepCXL processes and other loggers
        let mut lif = LoggerInterface::new(&config);
        
        // variables for latency measurement
        let mut logger_latency_total = 0;
        let mut logger_latency_count = 0;

        // loop variables
        let mut leader_id = 0;
        let mut log_index = 0;
        // initial vote is always the default one (leader = pid0), no need to read
        let mut my_vote: ProcessVote = ProcessVote::init();

        loop {
            // stop with algorithms threads on rep_cxl.stop()
            if stop_flag.load(Ordering::Relaxed) {
                if logger_latency_count > 0 {
                    log::info!("Average logger latency: {}", utils::fmt_ns(logger_latency_total / logger_latency_count));
                }
                break;
            }

            // LEADER logic
            if logger_id == leader_id {
                // let mut entry;
                // let mut pid;

                let (maybe_entry, pid) = lif.poll_next_process_queue();
                if maybe_entry.is_none() {
                    continue; // skip if no log entry to process
                }
                let entry = maybe_entry.unwrap();

                
                log_index += 1;
                my_vote.log_index = log_index;
                // propose the next log index (write to shared mem = broadcast)
                log::debug!("[leader]: Proposing log index {} for obj {} from pid {}", 
                    log_index, entry.obj_info.id, pid);
                lif.propose_log_index(logger_id, log_index);

                // leader loop for a given consensus instance (log index). check for 
                // - a majority of votes for the proposal
                // - a new leader being elected. 
                loop { 
                    let (quorum_reached, vote_with_highest_term) = lif.check_quorum();

                    let am_leader = my_vote == vote_with_highest_term;
                    match (quorum_reached, am_leader) {

                        // quorum reached and still leader, can safely append to 
                        // proposed log entry
                        (true, true) => { 
                            log::debug!("[leader]: Quorum reached for log index {}, appending...", 
                                log_index);
                            
                            // read log entry from queue
                            // if let (Some(entry), pid) = lif.poll_next_process_queue() {
                            let logger_latency = std::time::Instant::now();
                            log::debug!("[leader]: Processing log queue for obj {} from pid {}", 
                                entry.obj_info.id, pid);
                            if let Some(v) = check_dirty(&memory_nodes, &entry) {
                                for node in &memory_nodes {
                                    // get log
                                    let log = node.get_state().get_log();
                                    // append to log
                                    log.write(log_index, entry.wid, entry.obj_info, v);
                                }
                                log::debug!("[leader]: Appended dirty value to logs for obj {}", entry.obj_info.id);
                            } else {
                                log::debug!("[leader]: No dirty value found for obj {} - clearing entry", entry.obj_info.id);
                            }

                            // In all cases clear the queue entry so waiting processes won't hang.
                            lif.clear_process_queue(pid);
                            log::debug!("[leader]: Cleared log queue entry");
                            logger_latency_total += logger_latency.elapsed().as_nanos() as u64;
                            logger_latency_count += 1;
                            break; // exit leader loop and continue with next log entry
                        },

                        // still leader but no quorum on proposed log index
                        (false, true) => { 
                            log::debug!("[leader]: No quorum reached for log index {}, term {}. Retrying.", 
                                log_index, my_vote.term);
                            continue; // retry checking quorum for the same entry
                        },

                        // new leader was elected or proposed, step down and vote for it
                        (_, false) => { // 
                            log::debug!("[leader]: New leader elected or proposed: stepping down. Vote: {:?}", 
                                vote_with_highest_term);

                            my_vote = vote_with_highest_term;
                            leader_id = vote_with_highest_term.candidate_id;
                            lif.vote(logger_id, vote_with_highest_term);
                            break; // exit leader loop and continue with follower logic
                        }

                        // no quorum and new leader =>  election started, update vote
                        // (false, false) => {
                            
                        //     log::debug!("[leader]: No quorum reached for log index {}, term {}. Retrying.", 
                        //         log_index, term);
                        //     my_vote = vote_with_highest_term;
                        //     lif.vote(logger_id, my_vote);
                        // }
                    }
                }
                
            }
            
            // FOLLOWER logic
            else {

                // check for nominations
                if lif.election_request(logger_id) {
                    log::debug!("[election] {}: Starting election for candidate {}, term {}", 
                        logger_id, 
                        my_vote.candidate_id, 
                        my_vote.term + 1);

                    my_vote.term += 1;
                    my_vote.candidate_id = logger_id;
                    lif.vote(logger_id, my_vote);

                    lif.clear_election_request(logger_id); // only trigger election once, then wait for results
                }

                let (quorum_reached, vote_with_highest_term) = lif.check_quorum();

                leader_id = vote_with_highest_term.candidate_id;

                // I got elected as new leader, update local log index with the max
                // term and candidate id are already the highest if I got elected
                if leader_id == logger_id && quorum_reached {
                    log_index = lif.max_log_index();
                    continue;
                }

                // I am the candidate but no quorum yet, wait
                if leader_id == logger_id && !quorum_reached {
                    continue;
                }

                // else vote either for a new log index proposed by the current leader
                // or for a new leader if a new election started
                if my_vote != vote_with_highest_term {
                    log::debug!("[follower]: New leader candidate detected: {}, term {}. Updating vote: {:?}.", 
                        vote_with_highest_term.candidate_id, 
                        vote_with_highest_term.term, 
                        my_vote);
                    my_vote = vote_with_highest_term;
                    lif.vote(logger_id, my_vote);
                }

                // we are up to date, waiting for new proposals or elections
            }

        }
    });
}

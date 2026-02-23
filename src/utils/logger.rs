use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};

use crate::algorithms::monster::MonsterState;

/// Default log file path (in tmpfs for speed).
pub const DEFAULT_LOG_PATH: &str = "/tmp/repCXL.log";

/// A single entry in the state log, recording a monster state transition.
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterStateLogEntry {
    pub round_num: u64,
    pub state: String,
    pub object_id: usize,
}

pub struct Logger {
    log: File,
    // path: String,
}

impl Logger {
    pub fn new(path: &str) -> Self {
        Logger {
            log: OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)
            .expect("Failed to open state log file"),
            // path: path.to_string(),
        }
    }

    /// Append a MONSTER state transition line to the log file.
    /// Format: `<round_num>,<state>,<object_id>\n`
    pub fn log_monster(&mut self, round_num: u64, state: MonsterState, object_id: usize) {
        writeln!(self.log, "{},{},{}", round_num, state, object_id)
            .expect("Failed to write to state log file");
    }

    /// Read all entries from a state log file.
    /// Seeks to the beginning of the already-open file and reads from it.
    pub fn read_monster_log(&mut self) -> Vec<MonsterStateLogEntry> {
        self.log.seek(SeekFrom::Start(0)).expect("Failed to seek to beginning of log file");
        let reader = BufReader::new(&self.log);
        reader
            .lines()
            .filter_map(|line| {
                let line = line.ok()?;
                let line = line.trim();
                if line.is_empty() {
                    return None;
                }
                let parts: Vec<&str> = line.splitn(3, ',').collect();
                if parts.len() != 3 {
                    return None;
                }
                Some(MonsterStateLogEntry {
                    round_num: parts[0].parse().ok()?,
                    state: parts[1].to_string(),
                    object_id: parts[2].parse().ok()?,
                })
            })
            .collect()
    }

    /// Read only the state names in order from the log file.
    pub fn read_monster_states(&mut self) -> Vec<String> {
        self.read_monster_log().into_iter().map(|e| e.state).collect()
    }

    /// Clear the log by truncating the open file to zero length.
    pub fn clear(&mut self) {
        self.log.set_len(0).expect("Failed to truncate log file");
        self.log.seek(SeekFrom::Start(0)).expect("Failed to seek after truncation");
    }


}



/// Assert that the log file contains exactly the expected sequence of states.
pub fn assert_states(logger: &mut Logger, expected: &[&str]) {
    let actual = logger.read_monster_states();
    assert_eq!(
        actual.len(),
        expected.len(),
        "State log length mismatch: expected {}, got {}.\nActual: {:?}",
        expected.len(),
        actual.len(),
        actual
    );
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            a, e,
            "State mismatch at index {}: expected '{}', got '{}'.\nFull log: {:?}",
            i, e, a, actual
        );
    }
}


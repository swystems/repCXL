// Parse a TOML configuration file into DefaultArgs.
//
// The config file mirrors the CLI arguments defined in arg_parser so that
// the same set of parameters can be supplied via file instead of flags.

use std::fs;
use serde::Deserialize;

const DEFAULT_MEM_SIZE: usize = 1024 * 1024; // 1 MiB
const DEFAULT_CHUNK_SIZE: usize = 64; // 64 bytes
const DEFAULT_PROCESSES: u32 = 1;
const DEFAULT_ROUND_TIME_NS: u64 = 1000000; //1ms
const DEFAULT_ALGORITHM: &str = "monster";


/// Raw deserialized representation of the TOML config file.
/// All fields are optional during deserialization  missing fields keep their 
/// Can be checked with validate() 
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct RepCXLConfig {
    pub mem_nodes: Vec<String>,
    pub mem_size: usize,
    pub chunk_size: usize,
    pub round_time: u64,
    pub id: i32,
    pub processes: u32,
    pub algorithm: String,
}

impl Default for RepCXLConfig {
    fn default() -> Self {
        Self {
            mem_nodes: Vec::new(),
            mem_size: DEFAULT_MEM_SIZE,
            chunk_size: DEFAULT_CHUNK_SIZE,
            round_time: DEFAULT_ROUND_TIME_NS,
            id: -1, // -1 indicates no id provided in config file
            processes: DEFAULT_PROCESSES,
            algorithm: DEFAULT_ALGORITHM.to_string(),
        }
    }
}


impl RepCXLConfig {
    /// Read and parse a TOML config file from the given path.
    pub fn from_file(path: &str) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read config file '{}': {}", path, e))?;

        // deserialize the file into RepCXLConfig struct. Missing field will
        // take the default value
        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config file '{}': {}", path, e))
    }

    /// Validate the config values. Exits if any value is invalid.
    pub fn validate(&self) -> Result<(), String> {

        let err_prefix = "Invalid config:".to_string();
        // must specify id
        if self.id < 0 {
            return Err(format!("{} id must be provided in the config", err_prefix));
        }
        // id must be less than the number of processes
        if self.id as u32 >= self.processes {
            return Err(format!("{} id must be less than the number of processes", err_prefix));
        }

        // must specify at least one node
        if self.mem_nodes.is_empty() {
            return Err(format!("{} at least one memory node must be specified in the config", err_prefix));
        }

        Ok(())
    }

}

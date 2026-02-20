// Parse a TOML configuration file into DefaultArgs.
//
// The config file mirrors the CLI arguments defined in arg_parser so that
// the same set of parameters can be supplied via file instead of flags.

use std::fs;
use log::error;
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
    pub fn from_file(path: &str) -> Self {
        let content = fs::read_to_string(path)
            .unwrap_or_else(|e| {
                error!("Failed to read config file '{}': {}", path, e);
                std::process::exit(1);
            });

        // deserialize the file into RepCXLConfig struct. Missing field will
        // take the default value
        toml::from_str(&content)
            .unwrap_or_else(|e| {
                error!("Failed to parse config file '{}': {}", path, e);
                std::process::exit(1);
            })
    }

    /// Validate the config values. Exits if any value is invalid.
    pub fn validate(&self) {
        // must specify id
        if self.id < 0 {
            error!("id must be provided in the config");
            std::process::exit(1);
        }

        if self.id as u32 >= self.processes {
            error!("id must be less than the number of processes");
            error!("id: {}, # of processes: {}", self.id, self.processes);
            std::process::exit(1);
        }

        // must specify at least one node
        if self.mem_nodes.is_empty() {
            error!("at least one memory node must be specified in the config");
            std::process::exit(1);
        }
    }

}

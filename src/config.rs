// Parse a TOML configuration file into DefaultArgs.
//
// The config file mirrors the CLI arguments defined in arg_parser so that
// the same set of parameters can be supplied via file instead of flags.

use std::fs;
use serde::{Deserialize, Deserializer};

// default values for config parameters
const DEFAULT_MEM_SIZE: usize = 1024 * 1024; // 1 MiB
const DEFAULT_CHUNK_SIZE: usize = 64; // 64 bytes
const DEFAULT_STARTUP_DELAY: u64 = 1000000000; // 1s
const DEFAULT_ROUND_TIME_NS: u64 = 100000; //1ms
const DEFAULT_PROCESSES: &[u32] = &[0]; // default to single process with ID 0
const DEFAULT_ALGORITHM: &str = "monster";



/// Parse processes field which can be a number, array, or range string
fn parse_processes<'de, D>(deserializer: D) -> Result<Vec<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ProcessSpec {
        Number(u32),
        Array(Vec<u32>),
        Range(String),
    }
    
    match ProcessSpec::deserialize(deserializer)? {
        ProcessSpec::Number(n) => Ok((0..n).collect()),
        ProcessSpec::Array(arr) => Ok(arr),
        ProcessSpec::Range(s) => {
            // Parse "0-3" or "0..3" formats
            if let Some(pos) = s.find("..") {
                let start: u32 = s[..pos].trim().parse()
                    .map_err(|_| Error::custom(format!("Invalid range start in '{}'", s)))?;
                let end: u32 = s[pos+2..].trim().parse()
                    .map_err(|_| Error::custom(format!("Invalid range end in '{}'", s)))?;
                Ok((start..=end).collect())
            } else if let Some(pos) = s.find('-') {
                // Handle "0-3" format, but avoid negative numbers
                if pos > 0 {
                    let start: u32 = s[..pos].trim().parse()
                        .map_err(|_| Error::custom(format!("Invalid range start in '{}'", s)))?;
                    let end: u32 = s[pos+1..].trim().parse()
                        .map_err(|_| Error::custom(format!("Invalid range end in '{}'", s)))?;
                    Ok((start..=end).collect())
                } else {
                    // It's a negative number, try parsing as i32 then convert
                    let n = s.parse::<i32>()
                        .ok()
                        .and_then(|n| if n > 0 { Some(n as u32) } else { None })
                        .ok_or_else(|| Error::custom(format!("Invalid process count '{}'", s)))?;
                    Ok((0..n).collect())
                }
            } else {
                // Try parsing as plain number (count from 0)
                let n: u32 = s.trim().parse()
                    .map_err(|_| Error::custom(format!("Invalid process count '{}'", s)))?;
                Ok((0..n).collect())
            }
        }
    }
}

/// Raw deserialized representation of the TOML config file.
/// All fields are optional during deserialization  missing fields keep their 
/// Can be checked with validate() 
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct RepCXLConfig {
    pub mem_nodes: Vec<String>,
    pub mem_size: usize,
    pub chunk_size: usize,
    pub startup_delay: u64,
    pub round_time: u64,
    pub id: i32,
    #[serde(deserialize_with = "parse_processes")]
    pub processes: Vec<u32>,
    pub algorithm: String,
}

impl Default for RepCXLConfig {
    fn default() -> Self {
        Self {
            mem_nodes: Vec::new(),
            mem_size: DEFAULT_MEM_SIZE,
            chunk_size: DEFAULT_CHUNK_SIZE,
            startup_delay: DEFAULT_STARTUP_DELAY,
            round_time: DEFAULT_ROUND_TIME_NS,
            id: -1, // -1 indicates no id provided in config file
            processes: DEFAULT_PROCESSES.to_vec(), // default to single process with ID 0
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

        // id must be in the processes list
        if !self.processes.contains(&(self.id as u32)) {
            return Err(format!("{} id {} must be in the processes list {:?}", err_prefix, self.id, self.processes));
        }

        // must have less than MAX_PROCESSES
        if self.processes.len() > crate::MAX_PROCESSES as usize {
            return Err(format!("{} Maximum number of processes: {}", err_prefix, crate::MAX_PROCESSES));
        }

        // must specify at least one node
        if self.mem_nodes.is_empty() {
            return Err(format!("{} at least one memory node must be specified in the config", err_prefix));
        }

        Ok(())
    }

}

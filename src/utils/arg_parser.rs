// Parse command line arguments for RepCXL binaries and benchmarks

use clap::{Arg, value_parser};
use std::time::Duration;

use super::config::RepCXLConfig;


#[derive(Debug)]
pub struct ArgParser {
    program_name: String,
    about: String,
    pub config: RepCXLConfig,
    pub extra_args: Vec<clap::Arg>,
}

impl ArgParser {

    pub fn new(program_name: &str, about: &str) -> Self {
        Self {
            program_name: program_name.to_string(),
            about: about.to_string(),
            config: RepCXLConfig::default(),
            extra_args: Vec::new(),
        }
    }

    pub fn add_arg(&mut self, arg: clap::Arg) {
        self.extra_args.push(arg);
    }

    pub fn add_args(&mut self, args: &[clap::Arg]) {
        for arg in args {
            self.extra_args.push(arg.clone());
        }
    }
    
    /// Parse CLI arguments and populate the config struct. Returns the remaining
    /// arguments that are not part of RepCXLConfig (e.g. benchmark-specific args).
    pub fn parse(&mut self) -> clap::ArgMatches {
        let mut cmd = clap::Command::new(&self.program_name)
            .version("1.0")
            .about(&self.about)
            .arg(
                Arg::new("config")
                    .short('C')
                    .long("config")
                    .help("Path to a TOML config file. Mutually exclusive with all other default arguments")
                    .value_parser(value_parser!(String)),
            )
            .arg(
                Arg::new("round_time")
                    .short('r')
                    .long("round")
                    .help("Duration of the synchronous round of the replication protocol (in ns)")
                    .value_parser(value_parser!(u64)),
            )
            .arg(
                Arg::new("id")
                    .short('i')
                    .long("id")
                    .help("Unique identifier for the repCXL instance")
                    .value_parser(value_parser!(u32)),
            )
            .arg(
                Arg::new("processes")
                    .short('p')
                    .long("processes")
                    .help("Number of total repCXL processes")
                    .value_parser(value_parser!(u32)),
            )
            .arg(
                Arg::new("algorithm")
                    .short('A')
                    .long("algorithm")
                    .help("Replication algorithm to use")
                    .value_parser(value_parser!(String)),
            );

        for extra_arg in &self.extra_args {
            cmd = cmd.arg(extra_arg.clone());
        }

        let mut matches = cmd.get_matches();

        // Parse individual CLI arguments
        if let Some(round_time_ns) = matches.remove_one::<u64>("round_time") {
            self.config.round_time = Duration::from_nanos(round_time_ns);
        }
        if let Some(id) = matches.remove_one::<u32>("id") {
            self.config.id = id as i32;
        }
        if let Some(algorithm) = matches.remove_one::<String>("algorithm") {
            self.config.algorithm = algorithm;
        }
        if let Some(processes) = matches.remove_one::<u32>("processes") {
            self.config.processes = processes;
        }
        if let Some(ref path) = matches.remove_one::<String>("config") {
            println!("Config file provided, ignoring other CLI arguments");
            // overwrite other args
            self.config = RepCXLConfig::from_file(path);
        }

        self.config.validate(); // exits if config is invalid

        matches
    }

}
// evaluate raw replication performance
use clap::{value_parser, Arg};
use log::{debug, error};
use rand::Rng;
use rep_cxl::RepCXL;
use simple_logger;
use std::sync::Arc;
use std::time::{Duration, Instant};

// CONFIG
const NODE_PATHS: [&str; 3] = [
    "/dev/shm/repCXL_test1",
    "/dev/shm/repCXL_test2",
    "/dev/shm/repCXL_test3",
];
const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
const CHUNK_SIZE: usize = 64; // 64 bytes
const OBJ_VAL: u64 = 124; // use this value for all objects. Change size or type

// DEFAULTS
const DEFAULT_ATTEMPTS: &str = "100";
const DEFAULT_CLIENTS: &str = "1";
const DEFAULT_OBJECTS: &str = "100";
const DEFAULT_PROCESSES: &str = "1";
const DEFAULT_ROUND_TIME_NS: &str = "1000000"; //1ms
const DEFAULT_ALGORITHM: &str = "sync_best_effort";

pub fn percentile(latencies: &Vec<u128>, p: f32) -> u128 {
    if latencies.is_empty() {
        return 0;
    }
    let mut sorted = latencies.clone();
    sorted.sort_unstable();
    let index = (p * sorted.len() as f32).ceil() as usize - 1;
    sorted[index]
}

fn main() {
    // Initialize the logger
    simple_logger::init().unwrap();

    let matches = clap::Command::new("rep_bench")
        .version("1.0")
        .about("Consistent Latency replication over CXL")
        .arg(
            Arg::new("round_time")
                .short('r')
                .long("round")
                .help("Duration of the synchronous round of the replication protocol (in ns)")
                .default_value(DEFAULT_ROUND_TIME_NS) // 1 ms
                .value_parser(value_parser!(u64)),
        )
        .arg(
            Arg::new("attempts")
                .short('a')
                .long("attempts")
                .help("Number of tests")
                .default_value(DEFAULT_ATTEMPTS)
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("id")
                .short('i')
                .long("id")
                .help("Unique identifier for the repCXL instance")
                .required(true)
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("processes")
                .short('p')
                .long("processes")
                .help("Number of total repCXL processes")
                .default_value(DEFAULT_PROCESSES)
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("clients")
                .short('c')
                .long("clients")
                .help("Number of clients issuing requests to the current repCXL process. Must be the same for all repCXL processes")
                .default_value(DEFAULT_CLIENTS)
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("objects")
                .short('o')
                .long("objects")
                .help("Number of objects to create")
                .default_value(DEFAULT_OBJECTS)
                .value_parser(value_parser!(usize)),
        )
        .arg(
            Arg::new("algorithm")
                .short('A')
                .long("algorithm")
                .help("Replication algorithm to use")
                .default_value(DEFAULT_ALGORITHM)
                .value_parser(value_parser!(String)),
        )
        .get_matches();

    // Parse the command line arguments
    let round_time_ns = matches.get_one::<u64>("round_time").unwrap().clone();
    let round_time = Duration::from_nanos(round_time_ns);

    let id = matches.get_one::<u32>("id").unwrap().clone();
    let processes = matches.get_one::<u32>("processes").unwrap().clone();
    if id >= processes {
        error!("id must be less than the number of processes");
        error!("id: {id}, # of processes: {processes}");
        std::process::exit(1);
    }

    let attempts = matches.get_one::<u32>("attempts").unwrap().clone();
    let clients = matches.get_one::<u32>("clients").unwrap().clone();
    let num_of_objects = matches.get_one::<usize>("objects").unwrap().clone();

    let algorithm = matches.get_one::<String>("algorithm").unwrap().clone();

    // start repCXL process
    debug!("Starting RepCXL instance with id {}", id);
     let mut rcxl =
        RepCXL::<u64>::new(id as usize, MEMORY_SIZE, CHUNK_SIZE, round_time);

    // open memory nodes
    for path in NODE_PATHS.iter() {
        rcxl.add_memory_node_from_file(path);
    }

    // add processes to initial group view
    for process in 0..processes {
        rcxl.register_process(process as usize);
    }

    let mut objects = Vec::new();
    // only the coordinator manages the state
    if rcxl.is_coordinator() {
        debug!("Starting as coordinator with id {}", rcxl.id);
        rcxl.init_state();

        for i in 0..num_of_objects {
            debug!("Creating object {}", i);
            let obj = rcxl.new_object(i).expect("failed to create object");
            objects.push(obj);
        }
    }
    // Replica
    else {
        debug!("Starting as replica with id {}", rcxl.id);

        for i in 0..num_of_objects {
            // try until the coordinator creates the object
            let obj = loop {
                match rcxl.get_object(i) {
                    Some(obj) => break obj,
                    None => std::thread::sleep(Duration::from_millis(100)),
                }
            };
            objects.push(obj);
        }
    }


    rcxl.sync_start(algorithm);

    
    // start benchmark
    let objects = Arc::new(objects);
    let mut client_handles = Vec::new();

    // init metrics vectors
    let (lats_tx, lats_rx) = std::sync::mpsc::channel();
    let (tput_tx, tput_rx) = std::sync::mpsc::channel();

    for c in 0..clients {   
        let lats_tx = lats_tx.clone();
        let tput_tx = tput_tx.clone();

        let objects = Arc::clone(&objects);
        let handle = std::thread::spawn(move || {
            debug!("Starting client thread {}", c);

            let mut lats = Vec::new();
            let mut rng = rand::rng();
            
            

            let total_start = Instant::now();
            for _ in 0..attempts {
                let id = rng.random_range(0..num_of_objects);
                let obj = objects.get(id).unwrap();

                let start = Instant::now();
                // write is blocking
                match obj.write(OBJ_VAL) {
                    Ok(()) => (),
                    Err(e) => error!("{e}"),
                }
                lats.push(start.elapsed());
            }
            let total_elapsed_s = total_start.elapsed().as_secs_f64();
            lats_tx.send(lats).unwrap();
            tput_tx.send(attempts as f64 / total_elapsed_s).unwrap();
        }); // end of thread body

        client_handles.push(handle);
    }

    // collect and print stats (probably dumb way of doing it)
    let mut lats_ns = Vec::new();
    let mut total_tputs = Vec::new();

    // drop extra senders to make the recv loop exit
    drop(lats_tx);
    drop(tput_tx);

    loop {
        match (lats_rx.recv(), tput_rx.recv()) {
            (Ok(l), Ok(t)) => {
                lats_ns.append(&mut l.into_iter().map(|d| d.as_nanos()).collect());
                total_tputs.push(t);
            }
            _ => break,
        }
    }

    for handle in client_handles {
        handle.join().unwrap();
    }

    println!(
        "Throughput: {:.2} ops/sec",
        total_tputs.iter().sum::<f64>() / total_tputs.len() as f64
    );
    // let lats_ns: Vec<u128> = lats.into_iter().map(|d| d.as_nanos()).collect();
    let mem_avg = lats_ns.iter().sum::<u128>() as f64 / attempts as f64;
    let mem_max = lats_ns.iter().max().unwrap();
    let mem_min = lats_ns.iter().min().unwrap();

    println!("Round time {:?}, {attempts} runs", round_time);
    println!("Latency Avg: {:?}", Duration::from_nanos(mem_avg as u64));
    println!("Latency Min: {:?}", Duration::from_nanos(*mem_min as u64));
    println!(
        "Latency 50th percentile: {:?}",
        Duration::from_nanos(percentile(&lats_ns, 0.5) as u64)
    );
    println!(
        "Latency 95th percentile: {:?}",
        Duration::from_nanos(percentile(&lats_ns, 0.95) as u64)
    );
    println!(
        "Latency 99th percentile: {:?}",
        Duration::from_nanos(percentile(&lats_ns, 0.99) as u64)
    );
    println!(
        "Latency 99.99th percentile: {:?}",
        Duration::from_nanos(percentile(&lats_ns, 0.9999) as u64)
    );
    println!("Latency Max: {:?}", Duration::from_nanos(*mem_max as u64));
}

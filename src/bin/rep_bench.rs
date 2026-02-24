// evaluate raw replication performance
use clap::{value_parser, Arg};
use log::{debug, error};
use rand::Rng;
use rep_cxl::{RepCXL, utils};
use simple_logger;
use std::sync::Arc;
use std::time::{Duration, Instant};
use rep_cxl::utils::arg_parser::ArgParser;

const OBJ_VAL: u64 = 124; // use this value for all objects. Change size or type

// BENCHMARK DEFAULTS
const DEFAULT_ATTEMPTS: &str = "100000";
const DEFAULT_CLIENTS: &str = "1";
const DEFAULT_OBJECTS: &str = "100";

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

    // Parse repcxl and benchmark-specific args
    let mut ap = ArgParser::new("rep_bench", "test replication performance of repCXL");
    ap.add_args(&[
        Arg::new("attempts")
            .short('a')
            .long("attempts")
            .help("Number of tests")
            .default_value(DEFAULT_ATTEMPTS)
            .value_parser(value_parser!(u32)),
        Arg::new("clients")
            .short('C')
            .long("clients")
            .help("Number of clients issuing requests to the current repCXL process. Must be the same for all repCXL processes")
            .default_value(DEFAULT_CLIENTS)
            .value_parser(value_parser!(u32)),
        Arg::new("objects")
            .short('o')
            .long("objects")
            .help("Number of objects to create")
            .default_value(DEFAULT_OBJECTS)
            .value_parser(value_parser!(usize)),
    ]);

    let matches = ap.parse();
    
    let attempts = matches.get_one::<u32>("attempts").unwrap().clone();
    let clients = matches.get_one::<u32>("clients").unwrap().clone();
    let num_of_objects = matches.get_one::<usize>("objects").unwrap().clone();

    let config = ap.config;

    // start repCXL process
    debug!("Starting RepCXL instance with id {}", config.id);
     let mut rcxl =
        RepCXL::<u64>::new(config);



    let mut objects = Vec::new();
    // only the coordinator manages the state
    if rcxl.is_coordinator() {
        debug!("Starting as coordinator with id {}", rcxl.config.id);
        rcxl.init_state();

        for i in 0..num_of_objects {
            debug!("Creating object {}", i);
            let obj = rcxl.new_object(i).expect("failed to create object");
            objects.push(obj);
        }
    }
    // Replica
    else {
        debug!("Starting as replica with id {}", rcxl.config.id);

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

    rcxl.sync_start();

    // wait for all processes to start up before starting benchmark
    std::thread::sleep(Duration::from_nanos(rcxl.config.startup_delay));
    
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

    // drop extra senders to make the recv loop below exit later
    drop(lats_tx);
    drop(tput_tx);

    loop {
        match (lats_rx.recv(), tput_rx.recv()) {
            (Ok(mut lvec), Ok(t)) => {
                lats_ns.append(&mut lvec);
                total_tputs.push(t);
            }
            _ => break,
        }
    }

    for handle in client_handles {
        handle.join().unwrap();
    }

    rcxl.stop();

    println!(
        "Throughput: {:.2} ops/sec",
        total_tputs.iter().sum::<f64>() / total_tputs.len() as f64
    );
    // let lats_ns: Vec<u128> = lats.into_iter().map(|d| d.as_nanos()).collect();
    utils::print_latency_stats(&lats_ns);
}

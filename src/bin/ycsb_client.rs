// Run YCSB benchmark through the RepCXL client library. It imports
// a YCSB workload and executes it (rather than issuing reqs from YCSB Java bin).

use core::panic;
use rep_cxl::utils::ycsb::load_ycsb_workload;
use rep_cxl::utils::arg_parser::ArgParser;
use rep_cxl::{RepCXL, ReadReturn};
use clap::{Arg, value_parser};
use log::{debug, info, error};
use std::time::Duration;

/// Convert Vec<u8> to fixed-size array, truncating or padding with zeros as needed
fn vec_to_array<const N: usize>(vec: &Vec<u8>) -> [u8; N] {
    let mut arr = [0u8; N];
    let copy_len = vec.len().min(N);
    arr[..copy_len].copy_from_slice(&vec[..copy_len]);
    arr
}

fn main() {

    simple_logger::init_with_env().unwrap();

    let mut ap = ArgParser::new( "ycsb_client", "YCSB Client for RepCXL" ); 
    
    // Add benchmark-specific arguments
    ap.add_args(&[
        Arg::new("load_trace")
            .help("Path to the YCSB load trace file")
            .required(true)
            .index(1)
            .value_parser(value_parser!(String)),
        Arg::new("run_trace")
            .help("Path to the YCSB run trace file")
            .required(true)
            .index(2)
            .value_parser(value_parser!(String)),
    ]);
    
    let extra_args = ap.parse();
    
    let load_trace = extra_args.get_one::<String>("load_trace").unwrap();
    let run_trace = extra_args.get_one::<String>("run_trace").unwrap();

    // println!("\nParsed arguments: {:#?}", ap);

    let workload = load_ycsb_workload(load_trace, run_trace);
    workload.summary();

    debug!("First 5 load operations:");
    for (i, op) in workload.load_ops.iter().enumerate().take(5) {
        let val_preview: String = op.fields.first()
            .map(|(name, val)| format!(" {}=[{}B]", name, val.len()))
            .unwrap_or_default();
        debug!("  [{}] {:?} {}{}", i, op.op_type, op.key, val_preview);
    }

    // Initialize RepCXL client and local index
    let mut rcxl = RepCXL::<[u8; 64]>::new(ap.config);
    let mut index = std::collections::HashMap::new();

    // LOAD PHASE: populate index and memory nodes
    if rcxl.is_coordinator() {
        info!("This process is the coordinator. Executing YCSB load phase...");
        rcxl.init_state(); // only coordinator initializes the state

        let mut oid = 0;
        for op in workload.load_ops {
            match op.op_type {
                rep_cxl::utils::ycsb::OpType::Insert => {
                    
                    // truncate/pad to fixed-size
                    let value: [u8; 64] = vec_to_array(&op.fields[0].1);

                    if let Some(obj) = rcxl.new_object_with_val(oid, value) {
                        index.insert(op.key, obj);
                        oid += 1;
                    }
                    else {
                        panic!("Failed to create object for key {}", op.key);
                    }
                },
                _ => panic!("Unexpected operation type in load phase: {:?}", op.op_type),
            }
        }
    } else {
        info!("This process is a replica. Waiting for coordinator to execute YCSB workload...");
        
        // get objects created by coordinator and populate index
        let mut oid = 0;
        for op in workload.load_ops {
            match op.op_type {
                rep_cxl::utils::ycsb::OpType::Insert => {
                    if let Some(obj) = rcxl.get_object(oid) {
                        index.insert(op.key, obj);
                        oid += 1;
                    }
                    else {
                        // wait for the coordinator to create the object
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                },
                _ => panic!("Unexpected operation type in load phase: {:?}", op.op_type),
            }
        }
    }
    
    // start repcxl
    rcxl.sync_start();

    // wait for all processes to start up before starting benchmark
    std::thread::sleep(Duration::from_nanos(rcxl.config.startup_delay));


    // RUN PHASE: execute operations from run trace
    debug!("First 10 run operations:");
    for (i, op) in workload.run_ops.iter().enumerate().take(10) {
        let val_preview: String = op.fields.first()
            .map(|(name, val)| format!(" {}=[{}B]", name, val.len()))
            .unwrap_or_default();
        debug!("  [{}] {:?} {}{}", i, op.op_type, op.key, val_preview);
    }
    if workload.run_ops.len() > 10 {
        debug!("  ... ({} more)", workload.run_ops.len() - 10);
    }
    

    // metrics
    let mut read_latencies = Vec::new();
    let mut read_errors = 0;
    let mut write_latencies = Vec::new();
    let mut write_errors = 0;
    let mut dirty_reads = 0;
    let mut safe_reads = 0;

    info!("Executing YCSB run phase...");
    let start_total = std::time::Instant::now();
    for op in &workload.run_ops {
        match op.op_type {
            rep_cxl::utils::ycsb::OpType::Read => {
                let obj = index.get(&op.key).expect("Key not found in index");
                let start = std::time::Instant::now();
                match obj.read() {
                    Ok(rr) => {
                        match rr {
                            ReadReturn::ReadDirty(_) => dirty_reads += 1,
                            ReadReturn::ReadSafe(_) => safe_reads += 1,
                        }
                        read_latencies.push(start.elapsed());
                    },
                    Err(e) => {
                        error!("read error for object {}: {}", op.key, e);
                        read_errors += 1;
                    },
                }
            },
            rep_cxl::utils::ycsb::OpType::Update => {
                let value: [u8; 64] = vec_to_array(&op.fields[0].1);

                if let Some(obj) = index.get(&op.key) {
                    let start = std::time::Instant::now();
                    if let Err(e) = obj.write(value) {
                        error!("write error for object {}: {}", op.key, e);
                        write_errors += 1;
                    }
                    else {
                        write_latencies.push(start.elapsed());
                    }
                }
                else {
                    panic!("Key not found in index for update: {}", op.key);
                }
            },
            _ => panic!("Unexpected operation type in run phase: {:?}", op.op_type),
        }
    }    
    let total_elapsed = start_total.elapsed();

    rcxl.stop();


    // report metrics
    let tput = workload.run_ops.len() as f64 / total_elapsed.as_secs_f64();
    
    println!("YCSB run phase completed:");
    println!("  Total operations: {}", workload.run_ops.len());
    println!("  Total time: {}s", total_elapsed.as_secs_f64());
    println!("  Throughput: {} ops/sec", tput);
    println!("  Read errors: {}", read_errors);
    println!("  Write errors: {}", write_errors);
    println!("  Safe reads: {}", safe_reads);
    println!("  Dirty reads: {}", dirty_reads);
    if !read_latencies.is_empty() {
        let avg_read_latency = read_latencies.iter().sum::<Duration>() / read_latencies.len() as u32;
        println!("  Average read latency: {}μs", avg_read_latency.as_micros());
    }
    if !write_latencies.is_empty() {
        let avg_write_latency = write_latencies.iter().sum::<Duration>() / write_latencies.len() as u32;
        println!("  Average write latency: {}μs", avg_write_latency.as_micros());
    }

}
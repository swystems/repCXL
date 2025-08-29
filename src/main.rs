// benchmark skeleton right now
use clap::{value_parser, Arg};
use log::debug;
use rep_cxl::RepCXL;
use simple_logger;

const NODE_PATHS: [&str; 3] = [
    "/dev/shm/repCXL_test0",
    "/dev/shm/repCXL_test1",
    "/dev/shm/repCXL_test2",
];
const PROCESSES: [usize; 2] = [0, 1]; // smallest is leader

const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
const CHUNK_SIZE: usize = 64; // 64 bytes
fn main() {
    // Initialize the logger
    simple_logger::init().unwrap();

    let matches = clap::Command::new("repcxl")
        .version("1.0")
        .about("Consistent memory replication over CXL")
        .arg(
            Arg::new("round_time")
                .short('r')
                .long("round")
                .help("Duration of the synchronous round of the replication protocol (in ns)")
                .default_value("1000000") // 1 ms
                .value_parser(value_parser!(u64)),
        )
        .arg(
            Arg::new("attempts")
                .short('a')
                .long("attempts")
                .help("Number of tests")
                .default_value("1000")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            Arg::new("id")
                .long("id")
                .help("Unique identifier for the process")
                .required(true)
                .value_parser(value_parser!(u32)),
        )
        .get_matches();

    // Parse the command line arguments
    let round_time_ns = matches.get_one::<u64>("round_time").unwrap().clone();

    let repcxl_id = matches.get_one::<u32>("id").unwrap().clone();
    // // let ratio = ratio.parse::<f32>().expect("String not parsable");
    // let attempts = matches.get_one::<u32>("attempts").unwrap().clone();

    // let mut mem_latencies = Vec::<u64>::new();
    // let mut timer_latencies = Vec::<u64>::new();

    // for _ in 0..attempts {
    //     let start = std::time::Instant::now();

    //     // read from shared memory
    //     unsafe {
    //         std::ptr::read_volatile(ptr);
    //     }
    //     let mem_elapsed = start.elapsed().as_nanos() as u64;

    //     // sleep or busy poll until next round
    //     // if round_time > mem_elapsed {
    //     //     let time_left = round_time - mem_elapsed;
    //     //     let threshold = (time_left as f32 * (1.0 - ratio)) as u64;
    //     //     busy_poll_sleep(time_left, threshold);
    //     // }

    //     let end = start.elapsed().as_nanos() as u64;
    //     mem_latencies.push(mem_elapsed);
    //     // println!("mem lat {:?}ns, total elapsed {:?}ns", mem_elapsed, end);
    //     timer_latencies.push(end - mem_elapsed);
    // }

    let round_time = std::time::Duration::from_nanos(round_time_ns);
    let mut rcxl = RepCXL::new(repcxl_id as usize, MEMORY_SIZE, CHUNK_SIZE, round_time);

    debug!("mem: {}B", rcxl.size);

    // open memory nodes
    for path in NODE_PATHS.iter() {
        rcxl.add_memory_node_from_file(path);
    }

    // add processes to initial group view
    for process in PROCESSES.iter() {
        rcxl.register_process(*process);
    }

    // only the coordinator manages the state
    // TODO: add checks
    if rcxl.is_coordinator() {
        debug!("Starting as coordinator with id {}", rcxl.id);
        rcxl.init_state();

        // create test objects
        let obj3 = rcxl.new_object(3).expect("failed to create object");

        let obj7 = rcxl.new_object(7).expect("failed to create object");
    } else {
        debug!("Starting as replica with id {}", rcxl.id);

        let obj3 = rcxl.get_object(3).expect("failed to get object with id 3");
    }

    rcxl.dump_states();

    rcxl.sync_start();

    if !rcxl.is_coordinator() {
        let obj3 = rcxl.get_object(3).expect("failed to get object with id 3");
        // debug!("obj3 value: {}", obj3);
        obj3.write(33);
    }
    std::thread::sleep(std::time::Duration::from_secs(10));
}

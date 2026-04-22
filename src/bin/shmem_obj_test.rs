use rep_cxl::RepCXL;
use rep_cxl::utils::arg_parser::ArgParser;
use clap::Arg;

// const ID: i32 = 1;
// const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
// const CHUNK_SIZE: usize = 64; // 64 bytes
// const SHMEM_PATH: &str = "/sys/bus/pci/devices/0000:00:03.0/resource2";
// const ROUND_INTERVAL_NS: u64 = 1_000_000; // 1 ms

fn main() {

    simple_logger::SimpleLogger::new()
        .env()
        .without_timestamps()
        .init()
        .unwrap();

    let mut ap = ArgParser::new("shmem_obj_test_leader", 
    "test creating and removing objects with shmem backend");

    ap.add_args(&[
        Arg::new("role")
            .help("'c'/'coordinator' or 'r'/'replica'")
            .required(true)
            .index(1)
            // .value_parser(value_parser!(String))
            ]);

    let matches = ap.parse();
    let msg = *b"Hello, RepCXL!";
    let _msg = 10; // convert to fixed-size array for simplicity

    ap.config.algorithm = "async_best_effort".to_string();

    let mut rcxl = RepCXL::new(ap.config);

    rcxl.config.pipeline = false;

    match matches.get_one::<String>("role").map(|s| s.as_str()) {
        Some("c") | Some("coordinator") => {

            rcxl.init_state(); // coordinator inits state

            rcxl.start(); // start protocol threads

            std::thread::sleep(std::time::Duration::from_millis(10)); // wait for protocol to start

            let obj100 = rcxl.new_object(100).expect("failed to create object");
            rcxl.write_object(&obj100, msg).expect("failed to write to object");
            // obj100.write(*b"repcxl nocach1").expect("failed to write to object");
            // obj100.write(*b"repcxl nocach2").expect("failed to write to object");
            // obj100.write(*b"repcxl nocach3").expect("failed to write to object");
        },
        Some("r") | Some("replica") => {
            rcxl.start(); // start protocol threads
            std::thread::sleep(std::time::Duration::from_millis(10)); // wait for protocol to start

            let obj100 = rcxl.get_object(100).expect("failed to get object");
            match rcxl.read_object(&obj100).expect("failed to read from object") {
                rep_cxl::request::ReadReturn::ReadSafe(buf) => {
                    assert_eq!(buf, msg, "replica read incorrect data");
                    println!("Replica successfully read: {}", String::from_utf8_lossy(&buf));
                },
                rep_cxl::request::ReadReturn::ReadDirty(_) => {
                    println!("Read dirty detected, something went wrong");

                },
            }
                        
        },
        _ => println!("Usage: shmem_obj_test <role>\nrole: 'c'/'coordinator' or 'r'/'replica'"),
    }
    
    rcxl.stop(); // stop protocol threads
}

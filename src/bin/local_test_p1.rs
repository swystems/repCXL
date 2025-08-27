use rep_cxl::RepCXL;
// use simple_logger::SimpleLogger;
use std::fs::OpenOptions;

const ID: usize = 1;
const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
const CHUNK_SIZE: usize = 64; // 64 bytes
const NODES: usize = 3;
const ROUND_INTERVAL_NS: u64 = 1_000_000; // 1 ms
fn main() {
    // Initialize the logger
    simple_logger::init().unwrap();
    // simple_logger::init_with_env().unwrap();

    // create memory nodes as files in tmpfs
    for i in 0..NODES {
        let path = format!("/dev/shm/repCXL_test{}", i);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .expect("Failed to create/open file in tmpfs");

        file.set_len(MEMORY_SIZE as u64)
            .expect("Failed to set file length");
    }

    let mut rcxl = RepCXL::new(
        ID,
        MEMORY_SIZE,
        CHUNK_SIZE,
        std::time::Duration::from_nanos(ROUND_INTERVAL_NS),
    );

    println!("mem: {}", rcxl.size);
    for i in 0..NODES {
        rcxl.add_memory_node_from_file(&format!("/dev/shm/repCXL_test{}", i));
    }

    // add process 2
    rcxl.register_process(2);
    // test adding it again
    rcxl.register_process(2);

    rcxl.init_state();

    // rcxl.new_object::<[u16; 100]>(100)
    //     .expect("failed to create object");

    // rcxl.new_object::<String>(100);

    // rcxl.new_object::<String>(66);

    // rcxl.remove_object::<String>(100);

    rcxl.sync_start();
    std::thread::sleep(std::time::Duration::from_secs(1));
    rcxl.dump_states();
}

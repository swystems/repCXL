use rep_cxl::RepCXL;
// use simple_logger::SimpleLogger;
use std::fs::OpenOptions;

const ID: usize = 2;
const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
const CHUNK_SIZE: usize = 64; // 64 bytes
const NODES: usize = 3;
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

    let mut rcxl = RepCXL::new(ID, MEMORY_SIZE, CHUNK_SIZE);

    println!("mem: {}", rcxl.size);
    for i in 0..NODES {
        rcxl.add_memory_node_from_file(&format!("/dev/shm/repCXL_test{}", i));
    }

    // add process 1
    rcxl.add_process_to_group(1);

    rcxl.get_object(100).expect("failed to create object");

    // should fail
    rcxl.new_object::<String>(66);
    // should succeed
    rcxl.remove_object(66);

    // rcxl.remove_object::<String>(100);

    rcxl.sync_start();

    std::thread::sleep(std::time::Duration::from_secs(1));

    rcxl.dump_states();
    rcxl.remove_object(66);
}

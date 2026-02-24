// execute after shmem_.._leader for object and state init
use rep_cxl::{RepCXL, RepCXLConfig};

const ID: i32 = 2;
const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
const CHUNK_SIZE: usize = 64; // 64 bytes
const SHMEM_PATH: &str = "/sys/bus/pci/devices/0000:00:03.0/resource2";
// const ROUND_INTERVAL_NS: u64 = 1_000_000; // 1 ms

fn main() {
    let config = RepCXLConfig {
        id: ID,
        mem_nodes: vec![SHMEM_PATH.to_string()],
        mem_size: MEMORY_SIZE,
        chunk_size: CHUNK_SIZE,
        processes: vec![ID as u32], // only this process
        ..Default::default()
    };
    let mut rcxl = RepCXL::<u64>::new(config);
    println!("mem: {}", rcxl.config.mem_size);

    rcxl.add_memory_node_from_file(SHMEM_PATH);

    // should look for it in the shared state
    rcxl.get_object(100).expect("failed to create object");
    // should find object in cache
    rcxl.get_object(100).expect("failed to create object");
    // should not find this one (succesfully deleted)
    if rcxl.get_object(4).is_none() {
        println!("no object found)");
    }
    rcxl.dump_states();
}

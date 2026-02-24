use rep_cxl::RepCXL;
use rep_cxl::RepCXLConfig;

const ID: i32 = 1;
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
    rcxl.add_memory_node_from_file(SHMEM_PATH);

    rcxl.init_state();

    rcxl.new_object(100).expect("failed to create object");

    rcxl.new_object(4);
    rcxl.remove_object(4);
    rcxl.dump_states();
}

use rep_cxl::RepCXL;

const ID: usize = 1;
const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
const CHUNK_SIZE: usize = 64; // 64 bytes
const SHMEM_PATH: &str = "/sys/bus/pci/devices/0000:00:03.0/resource2";

fn main() {
    let mut rcxl = RepCXL::new(ID, MEMORY_SIZE, CHUNK_SIZE);

    println!("mem: {}", rcxl.size);
    rcxl.add_memory_node_from_file(SHMEM_PATH);

    rcxl.init_state();

    rcxl.new_object::<[u16; 100]>(100)
        .expect("failed to create object");

    rcxl.new_object::<u64>(4);
    rcxl.remove_object(4);
    rcxl.dump_states();
}

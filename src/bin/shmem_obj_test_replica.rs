// execute after shmem_.._leader for object and state init
use rep_cxl::RepCXL;

const MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
const CHUNK_SIZE: usize = 64; // 64 bytes
const SHMEM_PATH: &str = "/sys/bus/pci/devices/0000:00:03.0/resource2";

fn main() {

    let mut rcxl = RepCXL::new(MEMORY_SIZE, CHUNK_SIZE);
    println!("mem: {}", rcxl.size);

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

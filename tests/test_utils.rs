use std::fs::File;
use rep_cxl::RepCXL;

pub const TEST_MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
pub const TEST_CHUNK_SIZE: usize = 64;


pub fn setup_tmpfs_file(path: &str, size: usize) {
    let file = File::create(path).expect("Failed to create tmpfs file");
    file.set_len(size as u64).expect("Failed to set file size");
}

pub fn cleanup_tmpfs_file(path: &str) {
    let _ = std::fs::remove_file(path);
}

pub fn multi_rcxl(num: usize, node_path: &str) -> Vec<RepCXL<u64>> {
    let mut processes = Vec::new();
    for i in 0..num {
        let mut rcxl = RepCXL::<u64>::new(
            i,
            TEST_MEMORY_SIZE,
            TEST_CHUNK_SIZE,
        );
        rcxl.add_memory_node_from_file(node_path);
        rcxl.init_state();
        processes.push(rcxl);
    }
    processes
}

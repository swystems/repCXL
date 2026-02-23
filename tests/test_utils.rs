use std::fs::File;
use rep_cxl::RepCXL;
use rep_cxl::RepCXLConfig;

pub const TEST_MEMORY_SIZE: usize = 1024 * 1024; // 1 MiB
pub const TEST_CHUNK_SIZE: usize = 64;

pub fn test_config(node_paths: Vec<&str>) -> RepCXLConfig {
    RepCXLConfig {
        id: 0,
        mem_nodes: node_paths.into_iter().map(|s| s.to_string()).collect(),
        mem_size: TEST_MEMORY_SIZE,
        chunk_size: TEST_CHUNK_SIZE,
        processes: vec![],
        ..Default::default()
    }
}

pub fn setup_tmpfs_file(path: &str, size: usize) {
    let file = File::create(path).expect("Failed to create tmpfs file");
    file.set_len(size as u64).expect("Failed to set file size");
}

pub fn cleanup_tmpfs_file(path: &str) {
    let _ = std::fs::remove_file(path);
}


pub fn single_rcxl(id: usize, node_paths: Vec<&str>) -> RepCXL<u64> {
    let mut config = test_config(node_paths);
    config.id = id as i32;
    config.processes = vec![id as u32];
    RepCXL::<u64>::new(config)
}

pub fn multi_rcxl(num: usize, node_paths: Vec<&str>) -> Vec<RepCXL<u64>> {
    let mut processes = Vec::new();
    for i in 0..num {
        let rcxl = single_rcxl(i, node_paths.clone());
        processes.push(rcxl);
    }
    processes
}

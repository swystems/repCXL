use std::fs::File;
use rep_cxl::RepCXL;
use rep_cxl::RepCXLConfig;
use std::fs::{OpenOptions, metadata};

pub const TEST_MEMORY_SIZE: usize = 2 * 1024 * 1024; // 1 MiB
pub const TEST_ALGORITHM: &str = "monster";
pub const TEST_ROUND_TIME: u64 = 10_000_000; // 10 ms

pub fn test_config(node_paths: Vec<&'static str>) -> RepCXLConfig {
    let log_node = "/tmp/repcxl_test.log";


    if let Ok(_) = metadata(log_node) {
        // exists — open without truncating
        let _f = OpenOptions::new().read(true).write(true).open(log_node).unwrap();
    } else {
        // doesn't exist — create and set length
        let f = OpenOptions::new().read(true).write(true).create_new(true).open(log_node).unwrap();
        f.set_len(TEST_MEMORY_SIZE as u64).expect("Failed to set file size");
    }
    
    RepCXLConfig {
        id: 0,
        mem_nodes: node_paths.into_iter().map(|s| s.to_string()).collect(),
        mem_size: TEST_MEMORY_SIZE,
        processes: vec![], 
        algorithm: TEST_ALGORITHM.to_string(),
        round_time: TEST_ROUND_TIME,
        pipeline: false, // no threads
        log_node: log_node.to_string(),
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


pub fn single_rcxl(id: usize, node_paths: Vec<&'static str>) -> RepCXL<u64> {
    let mut config = test_config(node_paths);
    config.id = id as i32;
    config.processes = vec![id as u32];
    RepCXL::<u64>::new(config)
}


pub fn multi_rcxl(num: usize, node_paths: Vec<&'static str>) -> Vec<RepCXL<u64>> {
    let mut processes = Vec::new();
    for i in 0..num {
        let mut rcxl = single_rcxl(i, node_paths.clone());
        if i == 0 {
            rcxl.init_state(); // coordinator inits state
        }

        // register processes
        for j in 0..num {
            rcxl.register_process(j as u32);
        }

        // std::thread::spawn(move || {
        //     rcxl.sync_start();
        // });

        processes.push(rcxl);
    }
    processes
}

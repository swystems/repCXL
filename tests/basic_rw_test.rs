use std::time::Duration;
use rep_cxl::request::ReadReturn;

mod test_utils;
use test_utils::{single_rcxl,TEST_MEMORY_SIZE,setup_tmpfs_file,cleanup_tmpfs_file};

const ALGORITHM: &str = "async_best_effort";
const ROUND_TIME: Duration = Duration::from_millis(10);


#[test]
fn test_rw() {
    let node_path = "/dev/shm/repCXL_test_write";
    let val = 42;
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    
    std::thread::spawn(move || {

        let mut repcxl0 = single_rcxl(0, vec![node_path]);
        repcxl0.init_state(); // coordinator inits state
        repcxl0.register_process(1);
        repcxl0.config.algorithm = ALGORITHM.to_string();
        repcxl0.config.round_time = ROUND_TIME.as_nanos() as u64; 
    
        let obj5 = repcxl0.new_object(5).unwrap();

        repcxl0.start();
        // Perform write
        let result = repcxl0.write_object(&obj5,val);
        assert!(result.is_ok(), "Write should succeed");
    });
    std::thread::sleep(Duration::from_millis(100)); // wait for write to propagate
    
    // get owned repCXL instances to move to threads later
    let mut repcxl1 = single_rcxl(1, vec![node_path]);
    
    // register processes
    repcxl1.register_process(0);

    repcxl1.config.algorithm = ALGORITHM.to_string();
    repcxl1.config.round_time = ROUND_TIME.as_nanos() as u64;

    // coordinator creates object
    // other replica gets it
    let obj5replica = repcxl1.get_object(5).expect("failed to get obj with id 5");
    
    repcxl1.start();
    
    // verify the value was written correctly
    let read_val = repcxl1.read_object(&obj5replica).expect("Read should succeed");
    // assert!(matches!(read_val, ReadReturn::ReadDirty(_)), "Read should return ReadDirty variant");
    if let ReadReturn::ReadDirty(rdp) = read_val {
        assert_eq!(rdp.data, val, "Read value should match written value");
        assert_ne!(rdp.data, 0, "Read value should not be default value");
    }

    if let ReadReturn::ReadSafe(v) = read_val {
        assert_eq!(v, val, "Read value should match written value");
        assert_ne!(v, 0, "Read value should not be default value");
    }
    cleanup_tmpfs_file(node_path);

    // std::thread::sleep(Duration::from_secs(10));
 
    // write and read threads stop after the test ends and the repCXL instances 
    // are dropped. Cannot explicitly stop them here because threads own 
    // repcxl instances
}
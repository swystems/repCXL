use std::time::Duration;
use rep_cxl::ReadReturn;

mod test_utils;
use test_utils::*;

const ALGORITHM: &str = "async_best_effort";
const ROUND_TIME: Duration = Duration::from_millis(10);


#[test]
fn test_rw() {
    let node_path = "/dev/shm/repCXL_test_write";
    let val = 42;
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    let mut repcxls = multi_rcxl(2, vec![node_path]).into_iter();
    // get owned repCXL instances to move to threads later
    let mut repcxl0 = repcxls.next().unwrap();
    let mut repcxl1 = repcxls.next().unwrap();
    
    // register processes
    repcxl0.register_process(1);
    repcxl1.register_process(0);
    
    // coordinator creates object
    repcxl0.new_object(5);
    // other replica gets it
    let obj5 = repcxl1.get_object(5).expect("failed to get obj with id 5");
    
    // both start
    std::thread::spawn(move || {
        repcxl0.sync_start(ALGORITHM.to_string(), ROUND_TIME);
    });
    std::thread::spawn(move || {
        repcxl1.sync_start(ALGORITHM.to_string(), ROUND_TIME);
    });
    
    
    // Perform write
    let result = obj5.write(val);
    assert!(result.is_ok(), "Write should succeed");

    // verify the value was written correctly
    let read_val = obj5.read().expect("Read should succeed");
    assert!(matches!(read_val, ReadReturn::ReadDirty(_)), "Read should return ReadDirty variant");
    if let ReadReturn::ReadDirty(v) = read_val {
        assert_eq!(v, val, "Read value should match written value");
        assert_ne!(v, 0, "Read value should not be default value");
    }

    cleanup_tmpfs_file(node_path);

    std::thread::sleep(Duration::from_secs(10));
 
    // write and read threads stop after the test ends and the repCXL instances 
    // are dropped. Cannot explicitly stop them here because threads own 
    // repcxl instances
}
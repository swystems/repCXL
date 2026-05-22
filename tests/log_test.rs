mod test_utils;
use test_utils::{single_rcxl,TEST_MEMORY_SIZE,setup_tmpfs_file,cleanup_tmpfs_file};
use std::time::Duration;
use rep_cxl::request::ReadReturn;

#[test]
fn test_log() {
    let node_paths = vec![
        "/dev/shm/repCXL_test_dirty1",
        "/dev/shm/repCXL_test_dirty2",
        "/dev/shm/repCXL_test_dirty3",
    ];
    let val = 999;

    for path in &node_paths {
        setup_tmpfs_file(path, TEST_MEMORY_SIZE);
    }

    // a hacky way to simulate a write conflict using instances with different node sets.
    // the current implementation, instance 2 reads the state from the first node only
    // in the memorynode list, hence it is able to find the object. The
    // object is not present in the second node, the process reads the initialized
    // value 0

    // Start both instances
    let mn01 = vec![node_paths[0], node_paths[1]]; 
    let mn02 = vec![node_paths[0], node_paths[2]]; 
    let handle_a = std::thread::spawn(move || {
        // RepCXL instance 1: uses nodes 1 and 2
        let mut repcxl_a = single_rcxl(0, mn01);
        repcxl_a.init_state();
        repcxl_a.register_process(1);    
        let obj_a = repcxl_a.new_object(7).expect("failed to create object");
        repcxl_a.sync_start();

        // Write from instance A (replicates to nodes 1 and 2)
        let result = repcxl_a.write_object(&obj_a, val);
        assert!(result.is_ok(), "Write should succeed");
        // std::thread::sleep(Duration::from_millis(2000)); //  make sure to not finish earlier than other thread
        
    });

    std::thread::sleep(Duration::from_millis(100)); // wait for init

    // RepCXL instance 2: uses nodes 0 and 2
    let mut repcxl_b = single_rcxl(1, mn02);
    repcxl_b.register_process(0);
    repcxl_b.sync_start();
    handle_a.join().expect("Thread A panicked"); // wait for instance A to finish writing
    // std::thread::sleep(Duration::from_millis(100));
    println!("Instance B attempting to read object 7");
    let obj_b = repcxl_b.get_object(7).unwrap();
    let read_val = repcxl_b.read_object(&obj_b).expect("Read should succeed");
    
    // Read from instance B (has node 2 but not node 1)
    // Should return ReadDirty because instance B's view is incomplete
    assert!(
        matches!(read_val, ReadReturn::ReadDirty(_)),
        "Read should return ReadDirty due to incomplete node set"
    );
    if let ReadReturn::ReadDirty(rdp) = read_val {
        assert_eq!(
            rdp.data, val,
            "Read value should match written value despite being dirty"
        );
    }

    for path in &node_paths {
        cleanup_tmpfs_file(path);
    }
}
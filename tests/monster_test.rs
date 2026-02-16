use std::time::Duration;
use rep_cxl::ReadReturn;

mod test_utils;
use test_utils::*;

const ALGORITHM: &str = "monster";
const ROUND_TIME: Duration = Duration::from_millis(10);


fn start_two_nodes_and_create_object(node_paths: Vec<&str>) -> (rep_cxl::RepCXLObject<u64>, rep_cxl::RepCXLObject<u64>) {
    let mut repcxls = multi_rcxl(2, node_paths).into_iter();
    // get owned repCXL instances to move to threads later
    let mut repcxl0 = repcxls.next().unwrap();
    let mut repcxl1 = repcxls.next().unwrap();
    
    // register processes
    repcxl0.register_process(1);
    repcxl1.register_process(0);
    
    // coordinator creates object
    let obj5_coordinator = repcxl0.new_object(5).expect("failed to get obj with id 5");
    // other replica gets it
    let obj5_replica = repcxl1.get_object(5).expect("failed to get obj with id 5");
    
    // both start
    std::thread::spawn(move || {
        repcxl0.sync_start(ALGORITHM.to_string(), ROUND_TIME);
    });
    std::thread::spawn(move || {
        repcxl1.sync_start(ALGORITHM.to_string(), ROUND_TIME);
    });

    (obj5_coordinator, obj5_replica)
}



#[test]
fn test_rw_single_node() {
    let node_path = "/dev/shm/repCXL_test_rw";
    let val = 42;
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    // coordinator creates object
    let (obj5_coordinator, obj5_replica) = start_two_nodes_and_create_object(vec![node_path]);
    
    // Read returns the initial value
    let read_val = obj5_coordinator.read().expect("Read should succeed");
    assert!(matches!(read_val, ReadReturn::ReadSafe(_)), "Read should return ReadSafe (single node)");
    if let ReadReturn::ReadSafe(v) = read_val {
        assert_eq!(v, 0, "Initial value should be default (0)");
    }

    // Perform write
    let result = obj5_coordinator.write(val);
    assert!(result.is_ok(), "Write should succeed");

    // wait for the write to propagate in the background (should be at most 3 round)
    std::thread::sleep(ROUND_TIME*10); 

    // verify the value was written correctly
    let read_val = obj5_coordinator.read().expect("Read should succeed");
    assert!(matches!(read_val, ReadReturn::ReadSafe(_)), "Read should return ReadSafe (single node)");
    if let ReadReturn::ReadSafe(v) = read_val {
        assert_eq!(v, val, "Read value should match written value");
        assert_ne!(v, 0, "Read value should not be default value");
    }


    // Other way round
    let val = 58;
    let result = obj5_replica.write(val);
    assert!(result.is_ok(), "Write should succeed");


    // verify the value was written correctly
    let read_val = obj5_replica.read().expect("Read should succeed");
    assert!(matches!(read_val, ReadReturn::ReadSafe(_)), "Read should return ReadSafe (single node)");
    if let ReadReturn::ReadSafe(v) = read_val {
        assert_eq!(v, val, "Read value should match written value");
        assert_ne!(v, 0, "Read value should not be default value");
    }

    
    cleanup_tmpfs_file(node_path);
 
    // write and read threads stop after the test ends and the repCXL instances 
    // are dropped. Cannot explicitly stop them here because threads own 
    // repcxl instances
}


#[test]
fn test_readsafe_multi_node() {
    let node_path1 = "/dev/shm/repCXL_test_rw1";
    let node_path2 = "/dev/shm/repCXL_test_rw2";
    let val = 123213;

    setup_tmpfs_file(node_path1, TEST_MEMORY_SIZE);
    setup_tmpfs_file(node_path2, TEST_MEMORY_SIZE);

    let (obj, _obj_replica) = start_two_nodes_and_create_object(vec![node_path1, node_path2]);
    
    // Perform write
    let result = obj.write(val);
    assert!(result.is_ok(), "Write should succeed");


    // verify the value was written correctly
    let read_val = obj.read().expect("Read should succeed");
    assert!(matches!(read_val, ReadReturn::ReadSafe(_)), "Read should return ReadSafe (single node)");
    if let ReadReturn::ReadSafe(v) = read_val {
        assert_eq!(v, val, "Read value should match written value");
        assert_ne!(v, 0, "Read value should not be default value");
    }

    cleanup_tmpfs_file(node_path1);
    cleanup_tmpfs_file(node_path2);
 
    // write and read threads stop after the test ends and the repCXL instances 
    // are dropped. Cannot explicitly stop them here because threads own 
    // repcxl instances
}

#[test]
fn test_readdirty() {
    let node_paths = vec!["/dev/shm/repCXL_test_dirty1", "/dev/shm/repCXL_test_dirty2", "/dev/shm/repCXL_test_dirty3"];
    let val = 999;

    for path in &node_paths {
        setup_tmpfs_file(path, TEST_MEMORY_SIZE);
    }

    // RepCXL instance 1: uses nodes 1 and 2
    let mut repcxl_a = single_rcxl(0, vec![node_paths[0], node_paths[1]]);
    repcxl_a.register_process(1);
    
    // RepCXL instance 2: uses nodes 0 and 2
    let mut repcxl_b = single_rcxl(1, vec![node_paths[0], node_paths[2]]);
    repcxl_b.register_process(0);

    let obj_a = repcxl_a.new_object(7).expect("failed to create object");
    // a hacky way to simulate a write conflict. the current implementation 
    // reads the state from the first node
    // in the memorynode list, hence it is able to find the object. The
    // object is not present in the second node, the process reads the initialized
    // value 0 
    let obj_b = repcxl_b.get_object(7).expect("failed to get object");

    // Start both instances
    std::thread::spawn(move || {
        repcxl_a.sync_start(ALGORITHM.to_string(), ROUND_TIME);
    });
    // std::thread::spawn(move || {
        repcxl_b.sync_start(ALGORITHM.to_string(), ROUND_TIME);
    // });

    // Write from instance A (replicates to nodes 1 and 2)
    let result = obj_a.write(val);
    assert!(result.is_ok(), "Write should succeed");


    // Read from instance B (has node 2 but not node 1)
    // Should return ReadDirty because instance B's view is incomplete
    let read_val = obj_b.read().expect("Read should succeed");

    assert!(matches!(read_val, ReadReturn::ReadDirty(_)), 
            "Read should return ReadDirty due to incomplete node set");
    if let ReadReturn::ReadDirty(v) = read_val {
        assert_eq!(v, val, "Read value should match written value despite being dirty");
    }

for path in &node_paths {
        cleanup_tmpfs_file(path);
    }
    
    // write and read threads stop after the test ends and the repCXL instances
}

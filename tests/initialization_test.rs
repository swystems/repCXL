use rep_cxl::RepCXL;

mod test_utils;
use test_utils::*;


#[test]
fn test_repcxl_initialization() {
    let node_path = "/dev/shm/repCXL_test_init";
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    let mut rcxl = RepCXL::<u64>::new(
        0,
        TEST_MEMORY_SIZE,
        TEST_CHUNK_SIZE,
    );

    rcxl.add_memory_node_from_file(node_path);
    assert!(rcxl.is_coordinator());
    
    cleanup_tmpfs_file(node_path);
}

#[test]
fn test_object_creation_and_allocation() {
    let node_path = "/dev/shm/repCXL_test_obj_create";
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    let mut rcxl = RepCXL::<u64>::new(
        0,
        TEST_MEMORY_SIZE,
        TEST_CHUNK_SIZE,
    );

    rcxl.add_memory_node_from_file(node_path);
    rcxl.init_state();

    // Create multiple objects
    let obj1 = rcxl.new_object(1).expect("Failed to create object 1");
    let obj2 = rcxl.new_object(2).expect("Failed to create object 2");
    let obj3 = rcxl.new_object(3).expect("Failed to create object 3");

    // Verify objects are distinct
    assert_ne!(format!("{:?}", obj1), format!("{:?}", obj2));
    assert_ne!(format!("{:?}", obj2), format!("{:?}", obj3));

    cleanup_tmpfs_file(node_path);
}

#[test]
fn test_multiprocess_init() {
    let node_path = "/dev/shm/repCXL_test_multiprocess";
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    let mut repcxls = multi_rcxl(2, vec![node_path]);
    
    // check the process view is consistent across instances
    repcxls[0].register_process(1);
    assert!(repcxls[0].get_view().processes.contains(&1));
    repcxls[1].register_process(0);
    assert!(repcxls[0].get_view().processes.contains(&0));
    assert!(repcxls[0].get_view() == repcxls[1].get_view(), "Views are not equal");
    // no duplicates
    repcxls[0].register_process(1);
    assert!(repcxls[0].get_view().processes.iter().filter(|&&x| x == 1).count() == 1);

    // test coordinator
    assert!(repcxls[0].is_coordinator());
    assert!(!repcxls[1].is_coordinator());


    cleanup_tmpfs_file(node_path);
}

#[test]
fn test_object_lifecycle() {
    let node_path = "/dev/shm/repCXL_test_obj_lifecycle";
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    let mut repcxls = multi_rcxl(2, vec![node_path]);
    repcxls[0].register_process(1);
    repcxls[1].register_process(0);
    

    // only the coordinator (process with smallest ID) can create objects
    assert!(repcxls[1].new_object(100).is_none(), 
        "Non-coordinator process should not be able to create objects");

    // Create object as coordinator
    let _obj = repcxls[0].new_object(100).expect("Failed to create object");

    // re-creating object should return none
    assert!(repcxls[0].new_object(100).is_none(), 
        "Creating an object with an existing ID should return None");
    
    // Lookup object (simulates replica process)
    let found_obj = repcxls[1].get_object(100);
    assert!(found_obj.is_some(), "Object should be found");

    // Lookup non-existent object
    let missing_obj = repcxls[1].get_object(999);
    assert!(missing_obj.is_none(), "Non-existent object should return None");

    // Remove object and verify it's gone
    repcxls[0].remove_object(100);
    let removed_obj = repcxls[1].get_object(100);
    assert!(removed_obj.is_none(), "Removed object should return None");

    cleanup_tmpfs_file(node_path);
}

#[test]
fn test_object_limit() {
    let node_path = "/dev/shm/repCXL_test_limit";
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    let mut rcxl = RepCXL::<u8>::new(
        0,
        TEST_MEMORY_SIZE,
        TEST_CHUNK_SIZE,
    );

    rcxl.add_memory_node_from_file(node_path);
    rcxl.init_state();

    let max_objs = rep_cxl::MAX_OBJECTS;

    // Try to create more than MAX_OBJECTS (128)
    for i in 0..max_objs {
        rcxl.new_object(i);
    }

    assert!(rcxl.new_object(max_objs+1).is_none(), "Creating an object beyond the maximum limit should return None");

    cleanup_tmpfs_file(node_path);
}

// Note: some tests might be flaky and due to delays the round period which
// causes e.g. expected conflict to not occur and similar unlucky events. Run with
// at least 10ms round time or --test-threads=1 to reduce flakiness.
use rep_cxl::request::ReadReturn;
use rep_cxl::utils::ms_logger;
use std::time::Duration;

mod test_utils;
use test_utils::*;

fn wait_for_rounds(rounds: u32) {
    std::thread::sleep(Duration::from_nanos(TEST_ROUND_TIME) * rounds);
}

// fn start_two_nodes_with
/// Assert that a subsequence of states appears in order within the log file.
pub fn check_state_transitions(actual: &Vec<String>, expected: &[&str]) -> bool {
    let mut prev = "Try"; // initial state
    let mut ei = 0;
    for curr in actual {
        // println!("Checking state: {} against expected {}", curr, expected[ei]);
        if curr == expected[ei] {
            ei += 1;
            if ei == expected.len() {
                return true;
            }
        } else if prev != curr {
            ei = 0; // reset if the sequence breaks
        }

        prev = curr;
    }

    false
}

#[test]
fn test_rw_single_node() {
    let node_path = "/dev/shm/repCXL_test_rw";
    let val = 42;
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    // coordinator creates object
    // let (obj5_coordinator, obj5_replica) = start_two_nodes_and_create_object(vec![node_path]);

    // println!("here");
    let mut repcxls = multi_rcxl(2, vec![node_path]);
    
    // let obj5_coordinator = repcxls[0].new_object(5).expect("failed to get obj with id 5");
    // let obj5_replica = repcxls[1].get_object(5).expect("failed to get obj with id 5");

    let mut coordinator = repcxls.remove(0);

    std::thread::spawn(move || {
        coordinator.sync_start();
        let obj5 = coordinator.new_object(5).expect("failed to get obj with id 5");   
        let read_val = coordinator.read_object(&obj5).expect("Read should succeed");
        assert!(
            matches!(read_val, ReadReturn::ReadSafe(_)),
            "Read should return ReadSafe (single node)"
        );
        if let ReadReturn::ReadSafe(v) = read_val {
            assert_eq!(v, 0, "Initial value should be default (0)");
        }    
    
        let result = coordinator.write_object(&obj5, val);
        assert!(result.is_ok(), "Write should succeed");

    });
    // Read returns the initial value
    

    let mut replica = repcxls.remove(0);
    std::thread::spawn(move || {
        replica.sync_start();

        // wait for coordinator to finish
        std::thread::sleep(Duration::from_millis(100));
        let obj5 = replica.get_object(5).expect("failed to get obj with id 5");
        let read_val = replica.read_object(&obj5).expect("Read should succeed");

        assert!(
            matches!(read_val, ReadReturn::ReadSafe(_)),
            "Read should return ReadSafe (single node)"
        );
        if let ReadReturn::ReadSafe(v) = read_val {
            assert_eq!(v, val, "Read value should match written value");
            assert_ne!(v, 0, "Read value should not be default value");
        }

    });

    cleanup_tmpfs_file(node_path);
}

#[test]
fn test_readsafe_multi_node() {
    let node_path1 = "/dev/shm/repCXL_test_rw1";
    let node_path2 = "/dev/shm/repCXL_test_rw2";
    let val = 123213;

    setup_tmpfs_file(node_path1, TEST_MEMORY_SIZE);
    setup_tmpfs_file(node_path2, TEST_MEMORY_SIZE);

    // println!("here");
    let mut repcxls = multi_rcxl(2, vec![node_path1, node_path2]);
    
    // let obj5_coordinator = repcxls[0].new_object(5).expect("failed to get obj with id 5");
    // let obj5_replica = repcxls[1].get_object(5).expect("failed to get obj with id 5");

    let mut coordinator = repcxls.remove(0);

    std::thread::spawn(move || {
        coordinator.sync_start();
        let obj5 = coordinator.new_object(5).expect("failed to get obj with id 5");   
        let read_val = coordinator.read_object(&obj5).expect("Read should succeed");
        assert!(
            matches!(read_val, ReadReturn::ReadSafe(_)),
            "Read should return ReadSafe (single node)"
        );
        if let ReadReturn::ReadSafe(v) = read_val {
            assert_eq!(v, 0, "Initial value should be default (0)");
        }    
    
        let result = coordinator.write_object(&obj5, val);
        assert!(result.is_ok(), "Write should succeed");

    });
    // Read returns the initial value
    

    let mut replica = repcxls.remove(0);
    std::thread::spawn(move || {
        replica.sync_start();

        // wait for coordinator to finish
        std::thread::sleep(Duration::from_millis(100));
        let obj5 = replica.get_object(5).expect("failed to get obj with id 5");
        let read_val = replica.read_object(&obj5).expect("Read should succeed");

        assert!(
            matches!(read_val, ReadReturn::ReadSafe(_)),
            "Read should return ReadSafe (single node)"
        );
        if let ReadReturn::ReadSafe(v) = read_val {
            assert_eq!(v, val, "Read value should match written value");
            assert_ne!(v, 0, "Read value should not be default value");
        }

    });

    cleanup_tmpfs_file(node_path1);
    cleanup_tmpfs_file(node_path2);

    // write and read threads stop after the test ends and the repCXL instances
    // are dropped. Cannot explicitly stop them here because threads own
    // repcxl instances
}

#[test]
fn test_readdirty() {
    let node_paths = vec![
        "/dev/shm/repCXL_test_dirty1",
        "/dev/shm/repCXL_test_dirty2",
        "/dev/shm/repCXL_test_dirty3",
    ];
    let val = 999;

    for path in &node_paths {
        setup_tmpfs_file(path, TEST_MEMORY_SIZE);
    }
    // RepCXL instance 1: uses nodes 1 and 2
    let mut repcxl_a = single_rcxl(0, vec![node_paths[0], node_paths[1]]);
    repcxl_a.register_process(1);
    repcxl_a.init_state();

    // RepCXL instance 2: uses nodes 0 and 2
    let mut repcxl_b = single_rcxl(1, vec![node_paths[0], node_paths[2]]);
    repcxl_b.register_process(0);



    // a hacky way to simulate a write conflict using instances with different node sets.
    // the current implementation, instance 2 reads the state from the first node only
    // in the memorynode list, hence it is able to find the object. The
    // object is not present in the second node, the process reads the initialized
    // value 0

    // Start both instances
    std::thread::spawn(move || {
        repcxl_a.sync_start();
        let obj_a = repcxl_a.new_object(7).expect("failed to create object");

        // Write from instance A (replicates to nodes 1 and 2)
        let result = repcxl_a.write_object(&obj_a, val);
        assert!(result.is_ok(), "Write should succeed");
     
    });

    std::thread::spawn(move || {
        repcxl_b.sync_start();

        // wait for instance A to finish writing
        std::thread::sleep(Duration::from_millis(100));

        let obj_b = repcxl_b.get_object(7).expect("failed to get object");
        let read_val = repcxl_b.read_object(&obj_b).expect("Read should succeed");
        // Read from instance B (has node 2 but not node 1)
        // Should return ReadDirty because instance B's view is incomplete

        assert!(
            matches!(read_val, ReadReturn::ReadDirty(_)),
            "Read should return ReadDirty due to incomplete node set"
        );
        if let ReadReturn::ReadDirty(v) = read_val {
            assert_eq!(
                v, val,
                "Read value should match written value despite being dirty"
            );
        }
    });    

    for path in &node_paths {
        cleanup_tmpfs_file(path);
    }
}

/// Test that a single write follows the expected monster state sequence:
/// Try → Check → Replicate → Try (idle rounds)
#[test]
fn test_states_single_write() {
    let node_path = "/dev/shm/repCXL_test_statelog";
    let log_path = "/tmp/repcxl.log";
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);


    let mut rcxl = single_rcxl(0, vec![node_path]);
    rcxl.init_state();
    rcxl.enable_file_log(log_path);

    let obj = rcxl.new_object(1).expect("failed to create object");
    

    rcxl.sync_start();

    // Perform a single write and stop more than one round latency after to allow
    // the state machine to go back to the initial state (Try)
    let result = rcxl.write_object(&obj, 77);
    wait_for_rounds(2);
    rcxl.stop();


    assert!(result.is_ok(), "Write should succeed");

    // The log must contain the expected subsequence for a successful write:
    //   Try (picks up the request)  →  Check  →  Replicate  →  Try (back to idle)
    let states = ms_logger::MonsterStateLogger::new(log_path).read_monster_states();
    // println!("{:?}", states);
    let correct_transition = check_state_transitions(&states, &["Try", "Check", "Replicate"]);
    // panic!();
    assert!(
        correct_transition,
        "State transitions should match expected pattern"
    );
    let incorrect_transition = check_state_transitions(&states, &["Try", "Check", "Try", "Try"]);
    assert_eq!(
        incorrect_transition, false,
        "State transitions should not match incorrect pattern"
    );

    cleanup_tmpfs_file(node_path);
}

// Emulate a write conflict having a large enough round period and writing to
// the same object from two instances immediately after sync_start
// NOTE: sync failures might not lead to a conflict, hence fail the test
#[test]
fn test_states_write_conflict() {
    let node_path = "/dev/shm/repCXL_test_conflict";
    setup_tmpfs_file(node_path, TEST_MEMORY_SIZE);

    let log_path0 = "/tmp/repcxl0.log";
    let log_path1 = "/tmp/repcxl1.log";

    // init instance 1
    let mut rcxl0 = single_rcxl(0, vec![node_path]);
    rcxl0.register_process(1);
    rcxl0.init_state();
    rcxl0.enable_file_log(log_path0);

    // init instance 2
    let mut rcxl1 = single_rcxl(1, vec![node_path]);
    rcxl1.register_process(0);
    rcxl1.enable_file_log(log_path1);

    // create object

    // conflicting writes from both instances
    std::thread::spawn(move || {
        rcxl0.sync_start();
        let obj_coord = rcxl0.new_object(2).expect("failed to create object");

        let _ = rcxl0.write_object(&obj_coord, 88);
    });

    std::thread::spawn(move || {
        rcxl1.sync_start();
        let obj_replica = rcxl1.get_object(2).expect("failed to get object");
        let _ = rcxl1.write_object(&obj_replica, 99);
        let replica_states = ms_logger::MonsterStateLogger::new(log_path1).read_monster_states();
        // println!("{:?}", replica_states);
        let correct_transition = check_state_transitions(
            &replica_states,
            &["Try", "Check", "Wait", "PostConflictCheck"],
        );
        assert!(
            correct_transition,
            "Incorrect transition sequence in {}",
            replica_states.join(" -> ")
        );
        let incorrect_transition =
            check_state_transitions(&replica_states, &["Try", "Check", "Replicate", "Try"]);
        assert!(!incorrect_transition, "Should not Check -> Replicate");
    });

    cleanup_tmpfs_file(node_path);

}

// We simulate an error by having repcxl instance A writing to a subset of
// nodes, causing the other instance B to notice that one of the values was not
// successfully replicated due to a crash of A. We expect B to reattempt to
// write
#[test]
fn test_states_write_conflict_then_error() {
    let node_paths = vec![
        "/dev/shm/repCXL_test_conflict1",
        "/dev/shm/repCXL_test_conflict2",
    ];
    for path in &node_paths {
        setup_tmpfs_file(path, TEST_MEMORY_SIZE);
    }

    let log_path0 = "/tmp/repcxl00.log";
    let log_path1 = "/tmp/repcxl11.log";

    // init instance A (coordinator) with only the first memory node
    let mut rcxl0 = single_rcxl(0, vec![node_paths[0]]);
    rcxl0.register_process(1);
    rcxl0.init_state();
    rcxl0.enable_file_log(log_path0);

    // init instance B (replica) with both memory nodes
    let mut rcxl1 = single_rcxl(1, node_paths.clone());
    rcxl1.register_process(0);
    rcxl1.enable_file_log(log_path1);


    // conflicting writes from both instances
    std::thread::spawn(move || {
        rcxl0.sync_start();
        let obj_coord = rcxl0.new_object(2).expect("failed to create object");
        let _ = rcxl0.write_object(&obj_coord, 88);
    });

    
    std::thread::spawn(move || {
        rcxl1.sync_start();
    
        // sleep to make rcxl create the object but not too much to avoid missing
        // the conflict
        std::thread::sleep(Duration::from_nanos(TEST_ROUND_TIME/10)); 
        // create object (replica finds it in the memory first node)
        let obj_replica = rcxl1.get_object(2).expect("failed to get object");

        let _ = rcxl1.write_object(&obj_replica, 99);

        let replica_states = ms_logger::MonsterStateLogger::new(log_path1).read_monster_states();
        // println!("{:?}", replica_states);
        let correct_transition = check_state_transitions(
            &replica_states,
            &["Try", "Check", "Wait", "PostConflictCheck", "Retry"],
        );
        assert!(
            correct_transition,
            "Incorrect transition sequence in {}",
            replica_states.join(" -> ")
        );
        let incorrect_transition =
            check_state_transitions(&replica_states, &["Try", "Check", "Replicate", "Try"]);
        assert!(
            !incorrect_transition,
            "Incorrect transition should not occur {}",
            replica_states.join(" -> ")
        );

        // should read the value writte by the replica
        let read_val = rcxl1.read_object(&obj_replica).expect("Read should succeed");
        assert!(
            matches!(read_val, ReadReturn::ReadSafe(_)),
            "Read should return ReadSafe after retrying"
        );
        if let ReadReturn::ReadSafe(v) = read_val {
            assert_eq!(
                v, 99,
                "Read value should match the value written by the replica after retrying"
            );
        }
    });

    for path in &node_paths {
        cleanup_tmpfs_file(path);
    }
}

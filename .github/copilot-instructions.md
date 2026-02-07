# RepCXL Copilot Instructions

## Project Overview
**RepCXL** is a Rust-based replicated shared memory system for disaggregated CXL (Compute Express Link) memory experiments. It provides consistent latency replication across multiple NUMA memory nodes by implementing synchronous consensus protocols.

## Core Architecture

### The Replication Protocol
- **Synchronous rounds**: Time-divided rounds (configurable, e.g., 1ms) where all replicas synchronize writes
- **Write identifier (Wid)**: Tuple of `(round_num, process_id)` that uniquely identifies and orders all writes
- **Master node + coordinator process**: All replicas elect the lowest-ID memory node and lowest-ID process as authorities
- **Data flow**: Client writes → request queue → replication algorithm (best_effort/monster) → all memory nodes

### Key Type Generics
All replication happens over generic type `T: Copy + PartialEq + Debug`, allowing different object sizes. Objects are allocated at specific byte offsets in shared memory with alignment/size tracking.

### Module Structure
- `lib.rs`: Main `RepCXL<T>` controller, `RepCXLObject<T>` handles, `GroupView` (membership tracking)
- `shmem/`: Shared memory mapping via tmpfs files
  - `object_index.rs`: Tracks object allocations
  - `wcc.rs`: Object write consistency causality tracking
  - `starting_block.rs`: Initial state marker
- `algorithms/`: Replication strategies
  - `best_effort.rs`: async (fire-and-forget) and sync (round-synchronized) variants
  - `monster.rs`: Advanced consensus algorithm
- `safe_memio.rs`: Memory-safe I/O operations

## Critical Workflows

### Building & Running
```bash
cargo build --release
cargo run --bin rep_bench -- --round 1000000 --attempts 100 --objects 100
```

Default binaries:
- `rep_bench`: Performance benchmark with configurable rounds, clients, objects
- `local_test_p1`/`local_test_p2`: Multi-process local replication tests
- `shmem_obj_test_leader`/`shmem_obj_test_replica`: Inter-machine replication tests

### Shared Memory Setup
Tests create tmpfs files at `/dev/shm/repCXL_testN` for each memory node. These must:
1. Be initialized with correct size before `RepCXL::new()`
2. Be pre-allocated with `file.set_len()` to reserve space
3. Be passed to `add_memory_node_from_file()` before `init_state()`

### Cross-Machine Testing
- Use `deploy_host.sh`/`deploy_vms.sh` to set up remote QEMU VMs with CXL memory
- VMs run scripts from `scripts/` to mount CXL as NUMA nodes
- Tests map same tmpfs file path on multiple hosts → shared memory view

### VM NUMA Configuration
CXL memory mapped to NUMA node 2, local RAM to node 0. See `vm_configs.md` for example QEMU commands with `-object memory-backend-ram,host-nodes=2` for CXL binding.

## Important Patterns & Conventions

### Object Lifecycle
```rust
let rcxl = RepCXL::<u64>::new(id, MEMORY_SIZE, CHUNK_SIZE, round_duration);
rcxl.add_memory_node_from_file("/dev/shm/repCXL_test0");
rcxl.register_process(other_process_id);
rcxl.init_state();  // Initialize shared state
rcxl.sync_start("sync_best_effort".to_string());  // Start replication thread

let obj = rcxl.new_object(obj_id).expect("allocation failed");
obj.write(data)?;  // Blocks until ack from all replicas
rcxl.dump_states();  // Debug: print all node states
```

### Algorithm Pluggability
Select algorithm via `sync_start(algorithm_name)`. Must implement:
```rust
fn algorithm_fn<T: Copy + PartialEq + Debug>(
    view: GroupView,
    start_time: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<WriteRequest<T>>,
)
```

Receives write requests via channel, must replicate to all `view.memory_nodes` using `safe_memio::safe_write()`.

### Type-Specific Allocations
Objects store `ObjectInfo` (id, offset, size). Allocation algorithm in `object_index.rs` must handle T-specific sizing. All replicas must use identical allocation to maintain alignment across processes.

### Round Timing
Algorithms wait on `round_time` boundaries using `wait_next_round()`. Sleep ratio configured in `algorithms.rs` (0.0 = full busy-wait, >0.0 = hybrid). Critical for latency consistency.

### Unsafe Memory Details
- `MemoryNode` holds `*mut SharedState` and `*mut u8` (raw ptrs to mmap'd regions)
- `safe_memio::safe_write()` wraps unsafe writes with error handling
- All state struct layouts must be identical across machines (alignment warnings on size mismatches)

## Configuration Hotspots
- `rep_bench.rs`: `NODE_PATHS`, `MEMORY_SIZE`, `CHUNK_SIZE`, `DEFAULT_ROUND_TIME_NS`
- `algorithms.rs`: `ROUND_SLEEP_RATIO`, `_ALGORITHM` (unused; use `from_string()`)
- `local_test_p1.rs`: `NODES`, `ROUND_INTERVAL_NS` (hardcoded test setup)
- `shmem.rs`: `STATE_SIZE` assertion validates struct serialization

## Debugging Tips
- Enable logs: `RUST_LOG=debug cargo run`
- Check state consistency: `rcxl.dump_states()` prints all node object indices
- Verify file setup: `ls -l /dev/shm/repCXL_test*` before tests
- Timing issues: Use `SystemTime::now()` measurements around writes; check `ROUND_SLEEP_RATIO`
- Cross-machine sync: Ensure clock skew <100µs on host/VMs for round boundary detection

## Testing Data
`jupyter/round-accuracy/` contains ablation studies comparing algorithms. Results in `results/timer-accuracy/` show latency distributions with different round times and sleep ratios.

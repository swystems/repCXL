# RepCXL Copilot Instructions

## Big picture
- RepCXL is a Rust shared-memory replication system for CXL / NUMA experiments. The main entry point is `RepCXL<T>` in [src/lib.rs](src/lib.rs), backed by `RepCXLObject<T>` handles and a per-process `GroupView`.
- Replicas share tmpfs/DAX-backed files; `MemoryNode::from_file()` mmaps each file and the shared-state layout must match across all processes and hosts.
- The coordinator is always the lowest process ID; the master memory node is the lowest memory-node ID.

## Core flow
- Client calls `RepCXL::write_object()` / `read_object()` in [src/lib.rs](src/lib.rs) using `RepCXLObject` handles. Writes enqueue a `WriteRequest` and block for an ack unless the direct path is used.
- `sync_start()` waits for all processes to mark readiness in the shared `StartingBlock`, chooses a common start time, then launches the protocol threads.
- `stop()` only flips the atomic stop flag; MONSTER stats are printed on shutdown.

## Algorithms and consistency
- Supported algorithms are `monster`, `fmonster`, `async_best_effort`, `sync_best_effort`, and `lock` in [src/algorithms.rs](src/algorithms.rs).
- Replication ordering is tracked with `Wid { round_num, process_id }` in [src/request.rs](src/request.rs); smaller process ID wins when rounds tie.
- `monster` / `fmonster` use round scheduling plus shared write-conflict state (`owcc` / `fwcc`) from [src/shmem.rs](src/shmem.rs); best-effort writes directly to all memory nodes via `safe_memio::mem_writeall()`.
- Read paths may return `ReadDirty`; `read_retries` in config retries dirty reads before logging them.

## Shared-memory rules
- Always pre-create and size tmpfs files before constructing `RepCXL` (tests use `/dev/shm/repCXL_test*`). `setup_tmpfs_file()` in [tests/test_utils.rs](tests/test_utils.rs) shows the required `set_len()` step.
- `ObjectMemoryEntry<T>` is `#[repr(C, align(64))]` in [src/safe_memio.rs](src/safe_memio.rs); keep layout changes aligned with flush/fence code.
- `safe_memio.rs` centralizes volatile reads/writes and cache flushes (`clflushopt`, `cache_flush_write`, `cache_flush_read`) around otherwise unsafe raw pointers.

## Configuration and CLI
- CLI/config parsing is in [src/utils/arg_parser.rs](src/utils/arg_parser.rs) and [src/config.rs](src/config.rs). `-c/--config` loads TOML, and CLI flags override config-file values.
- `processes` and `core_affinity` accept counts, ranges, or arrays in TOML; core 0 is rejected, and `logger_cluster_size` must be odd and no larger than the process count.
- `Cargo.toml` sets `rep_bench` as the default binary.

## Developer workflows
- Build: `cargo build --release`.
- Run the benchmark: `cargo run --bin rep_bench -- -c config/local.toml` or pass explicit CLI flags.
- Tests: `cargo test -- --test-threads=1` for timing-sensitive MONSTER cases; several tests spawn threads/processes and rely on shared tmpfs paths.
- Benchmark analysis and deployment live in [ansible/README.md](ansible/README.md) and [ycsb/README.md](ycsb/README.md); run Ansible playbooks from the `ansible/` directory so `ansible.cfg` is picked up.

## Project-specific patterns
- `init_state()`, `new_object()`, and `remove_object()` are coordinator-only in [src/lib.rs](src/lib.rs).
- `new_object_with_val()` writes initial values directly to every node; `new_object()` only allocates metadata.
- `enable_monster_statelog()` writes the MONSTER phase log used by tests in [tests/monster_test.rs](tests/monster_test.rs).
- `RUST_LOG=debug` is the quickest way to inspect protocol timing, object allocation, and dirty-read behavior.

## What to inspect first when changing behavior
- `src/lib.rs` for lifecycle and public API changes.
- `src/algorithms/monster.rs` for round-based write ordering and conflict handling.
- `src/algorithms/best_effort.rs` for direct memory replication and read consistency.
- `src/shmem/` and [src/safe_memio.rs](src/safe_memio.rs) for layout, mapping, and persistence changes.
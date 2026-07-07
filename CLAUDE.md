# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

RepCXL is a Rust shared-memory replication system for CXL / NUMA disaggregated-memory
experiments (swystems <> IBM research), x86-only. Replicas run as separate processes that
map shared tmpfs/DAX-backed memory-node files and replicate objects across them using one
of several consistency algorithms.

- `src/` — RepCXL library
- `src/bin/` — executable programs (`rep_bench` is the default binary)
- `tests/` — integration tests, run with `cargo test`
- `config/` — TOML configuration files
- `ansible/` — automated QEMU VM + benchmark workflows, results collected in `bench-outputs/`
- `jupyter/` — benchmark result analysis/visualization notebooks
- `ycsb/` — YCSB benchmark workflows and traces
- `scripts/` — deployment/benchmark helper scripts

## Common commands

```sh
cargo build --release
cargo run --bin rep_bench -- -c config/local.toml   # or explicit CLI flags instead of -c
cargo test -- --test-threads=1                       # required: MONSTER tests are timing-sensitive
RUST_LOG=debug cargo run --bin rep_bench -- ...       # inspect protocol timing / allocation / dirty reads
```

Other binaries: `mem_test` (raw memory-backed-file access latency), `shmem_obj_test`
(coordinator/replica shared-memory smoke test — see `info.md` for a worked example),
`rwspinlock_mp_test`, `ycsb_client`.

Ansible playbooks (VM creation/provisioning/deploy) must be run from the `ansible/`
directory so `ansible/ansible.cfg` (sets `forks = 32`, needed for multi-VM-per-host
benchmarks) is picked up. See `ansible/README.md`.

## Architecture

- The main entry point is `RepCXL<T>` in `src/lib.rs`, backed by `RepCXLObject<T>` handles
  and a per-process `GroupView`. The coordinator is always the lowest process ID; the
  master memory node is the lowest memory-node ID.
- Replicas share tmpfs/DAX-backed files; `MemoryNode::from_file()` mmaps each file, and the
  shared-state layout must match across all processes and hosts.
- Client calls `RepCXL::write_object()` / `read_object()` using `RepCXLObject` handles.
  Writes enqueue a `WriteRequest` and block for an ack unless the direct path is used.
- `sync_start()` waits for all processes to mark readiness in the shared `StartingBlock`,
  picks a common start time, then launches the protocol threads. `stop()` only flips the
  atomic stop flag; MONSTER stats are printed on shutdown.
- `init_state()`, `new_object()`, and `remove_object()` are coordinator-only.
  `new_object_with_val()` writes initial values directly to every node; `new_object()` only
  allocates metadata.

### Algorithms and consistency

Supported algorithms (`src/algorithms.rs`): `monster`, `fmonster`, `async_best_effort`,
`sync_best_effort`, `lock`.

- Replication ordering uses `Wid { round_num, process_id }` (`src/request.rs`); on a round
  tie, the smaller process ID wins.
- `monster`/`fmonster` (`src/algorithms/monster.rs`) use round scheduling plus shared
  write-conflict state (`owcc`/`fwcc`) from `src/shmem.rs`.
- Best-effort (`src/algorithms/best_effort.rs`) writes directly to all memory nodes via
  `safe_memio::mem_writeall()`.
- Read paths may return `ReadDirty`; the config's `read_retries` retries dirty reads before
  logging them.

### Shared memory

- Always pre-create and size tmpfs files before constructing `RepCXL` (tests use
  `/dev/shm/repCXL_test*`); `setup_tmpfs_file()` in `tests/test_utils.rs` shows the required
  `set_len()` step.
- `ObjectMemoryEntry<T>` (`src/safe_memio.rs`) is `#[repr(C, align(64))]` — keep layout
  changes aligned with flush/fence code.
- `safe_memio.rs` centralizes volatile reads/writes and cache flushes (`clflushopt`,
  `cache_flush_write`, `cache_flush_read`) around otherwise-unsafe raw pointers.

### Configuration and CLI

- CLI/config parsing is in `src/utils/arg_parser.rs` and `src/config.rs`. `-c/--config`
  loads TOML; CLI flags override config-file values.
- `processes` and `core_affinity` accept counts, ranges, or arrays in TOML; core 0 is
  rejected, and `logger_cluster_size` must be odd and no larger than the process count.

### What to inspect first when changing behavior

- `src/lib.rs` — lifecycle and public API changes.
- `src/algorithms/monster.rs` — round-based write ordering and conflict handling.
- `src/algorithms/best_effort.rs` — direct memory replication and read consistency.
- `src/shmem/` and `src/safe_memio.rs` — layout, mapping, and persistence changes.

## CXL/NUMA test environment

Real CXL hardware access is limited, so VMs simulate it via QEMU with
`memory-backend-ram`/`memory-backend-file` mapped to specific NUMA `host-nodes`, or via
`daxctl`-created DAX devices for direct mmap. Details and example QEMU invocations are in
`info.md`; VM lifecycle automation is in `ansible/` (see `ansible/README.md`).

YCSB benchmarking workflow (build with the Redis binding for Python 3 support, generate
client workloads via `bench/gen_ycsb_workload.sh`) is documented in `ycsb/README.md`.

## Failure diagnostics

`ras_monitor.c` (build with `make`, requires `libsqlite3-dev`) watches a running repCXL
process by PID and, on abnormal exit, correlates it against rasdaemon's RAS event db to
identify which `mem_nodes`/`logger_node` entry in a given `config/*.toml` failed (e.g. from
poisoning, hot-unplug, or other hardware memory errors). See `RAS_MONITOR.md`.

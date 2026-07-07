# ras_monitor

A standalone C tool that watches a running repCXL process by PID and, if it
disappears, correlates the exit with hardware memory errors recorded by
[rasdaemon](https://github.com/mchehab/rasdaemon) (ECC errors, CXL
general-media/DRAM/module events, poisoning, hot-unplug, etc.) against the
memory nodes listed in a repCXL `config/*.toml` file, and prints which
configured node failed.

## Build

Requires `libsqlite3-dev` (and `pkg-config`, used to locate sqlite3's
cflags/libs):

```sh
sudo apt install libsqlite3-dev pkg-config
make
```

This builds `./ras_monitor`. Without `pkg-config`/dev headers available, the
manual build command is in the header comment of `ras_monitor.c`.

## Usage

```sh
./ras_monitor -p <pid> -c config/ansible.toml
```

Typical flow: start the repCXL process(es) under test, note the PID of the
instance you want to watch, run `ras_monitor` against it, then trigger the
fault (e.g. `daxctl disable-device`/`daxctl reconfigure-device` on the
backing `/dev/daxX.Y`, or a poison injection). `ras_monitor` blocks until the
PID disappears, waits a short grace period for rasdaemon to flush, then
queries the rasdaemon sqlite3 db (default
`/var/lib/rasdaemon/ras-mc_event.db`, override with `-d`) for every RAS event
table that exists, and reports which configured `mem_nodes` entry (or
`logger_node`) the failure correlates to.

Run `./ras_monitor -h` for all options (poll interval, grace period, timeout,
verbose row dump, and `-s "YYYY-MM-DD HH:MM:SS"` for post-hoc analysis
against a process that already exited).

Exit codes: `0` a configured node was matched to a RAS event, `1` no RAS
events found in the window, `2` RAS events found but none matched a
configured node, `3` usage/setup error.

## How node matching works

For each `mem_nodes`/`logger_node` path in the config, `ras_monitor` resolves
a best-effort hardware identity:

- `/dev/daxX.Y` paths: reads NUMA node (`target_node`), the physical address
  range (`mapping0/start` + `mapping0/end` or `mapping0/size`), and scans the
  device's resolved sysfs path for a `memN`-style CXL memdev component.
- `/sys/bus/pci/devices/<BDF>/...` paths (the ivshmem VM setup): extracts the
  PCI BDF.
- Plain files (e.g. `/dev/shm/...` used by `config/local.toml` for local dev
  without real CXL hardware): no hardware identity can be resolved, so RAS
  correlation isn't possible for that node — this is expected and reported.

Each row rasdaemon returns is scanned generically (column names, not a fixed
per-table schema, since available columns vary by rasdaemon/kernel version):
columns whose name contains `memdev`, `serial`, `label`, `dimm`, `location`,
`dev_name`, `host`, or `region` are treated as identity strings and
substring-matched (case-insensitive) against a node's `dax_name`/`memdev`/
`pci_bdf`; columns containing `dpa`, `hpa`, `addr`, or `pfn` are treated as
addresses and range-checked against a node's resolved physical address range.

## Known limitations

- **No parent/child relationship assumed.** `ras_monitor` treats the PID as
  an arbitrary, already-running process (per the intended usage), not a
  child it forked. This means it cannot retrieve the process's exit status
  or terminating signal (only the real parent can `wait()` for that) — it
  only detects *that* the process disappeared, not *why*, and relies
  entirely on the rasdaemon correlation to explain the "why".
- **DPA vs SPA address space.** CXL `general_media`/`dram` event records
  report a Device Physical Address (DPA, relative to the endpoint decoder),
  while the address range resolved from `/dev/dax` sysfs (`mapping0/start`)
  is a System Physical Address (SPA). These are only directly comparable
  when the CXL decoder is a simple 1:1 mapping. Address-range matches are
  reported at "medium" confidence for this reason; identity-string matches
  (`memdev` name) are reported at "high" confidence and are the more
  reliable signal for CXL error tables.
- **`memN` resolution is a best-effort sysfs path scan**, not a full
  CXL region/decoder/endpoint walk. If a dax device's sysfs realpath doesn't
  happen to include a `memN` path component, `memdev` is left empty for that
  node.
- **Timestamp filtering assumes a stable timezone offset** during the
  monitoring window (rasdaemon stores timestamps as text including a `%z`
  offset, compared lexicographically). Fine for a single test run; not
  robust across a DST transition.
- Requires rasdaemon to be installed, running, and built with the sqlite3
  backend (and CXL tracepoint support, for CXL-specific correlation) on the
  machine under test — not the case on a typical dev workstation.

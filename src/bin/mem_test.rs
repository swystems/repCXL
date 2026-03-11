/// Benchmark raw memory read/write speed on an mmap'd file (tmpfs, ivshmem, DAX, etc.)
/// Compares plain volatile access vs volatile + clflush.
///
/// Usage: mem_test <path> [--size <bytes>] [--iterations <n>] [--object-size <bytes>]
///   e.g. mem_test /dev/shm/repCXLnode0 --iterations 1000000

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::time::Instant;

use clap::{Arg, Command, value_parser};
use libc::{mmap, munmap, MAP_SHARED, PROT_READ, PROT_WRITE};

const CACHE_LINE_SIZE: usize = 64;

// ── cache helpers ───────────────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn cache_flush_fence(addr: *const u8, size: usize) {
    let mut ptr = addr as usize;
    let end = ptr + size;
    while ptr < end {
        core::arch::asm!("clflushopt [{}]", in(reg) ptr, options(nostack, preserves_flags));
        ptr += CACHE_LINE_SIZE;
    }
    core::arch::x86_64::_mm_sfence();
}

// ── write variants ──────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn write_volatile_only(addr: *mut u8, val: u8, size: usize) {
    // write `size` bytes of `val` one cache-line at a time
    for off in (0..size).step_by(CACHE_LINE_SIZE) {
        std::ptr::write_volatile(addr.add(off), val);
    }
}

#[inline(always)]
unsafe fn write_volatile_flush(addr: *mut u8, val: u8, size: usize) {
    for off in (0..size).step_by(CACHE_LINE_SIZE) {
        std::ptr::write_volatile(addr.add(off), val);
    }
    cache_flush_fence(addr, size);
}

#[inline(always)]
unsafe fn write_plain(addr: *mut u8, val: u8, size: usize) {
    for off in (0..size).step_by(CACHE_LINE_SIZE) {
        std::ptr::write(addr.add(off), val);
    }
}

// ── read variants ───────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn read_volatile_only(addr: *mut u8, size: usize) -> u8 {
    let mut v = 0u8;
    for off in (0..size).step_by(CACHE_LINE_SIZE) {
        v = std::ptr::read_volatile(addr.add(off));
    }
    v
}

#[inline(always)]
unsafe fn read_flush_volatile(addr: *mut u8, size: usize) -> u8 {
    cache_flush_fence(addr, size);
    let mut v = 0u8;
    for off in (0..size).step_by(CACHE_LINE_SIZE) {
        v = std::ptr::read_volatile(addr.add(off));
    }
    v
}

#[inline(always)]
unsafe fn read_plain(addr: *mut u8, size: usize) -> u8 {
    let mut v = 0u8;
    for off in (0..size).step_by(CACHE_LINE_SIZE) {
        v = std::ptr::read(addr.add(off));
    }
    v
}

// ── benchmark harness ───────────────────────────────────────────────────────

fn bench<F: Fn()>(name: &str, iters: u64, f: F) {
    // warmup
    for _ in 0..100 {
        f();
    }

    let mut latencies: Vec<u64> = Vec::with_capacity(iters as usize);
    let start = Instant::now();
    for _ in 0..iters {
        let t0 = Instant::now();
        f();
        latencies.push(t0.elapsed().as_nanos() as u64);
    }
    let total = start.elapsed();

    latencies.sort_unstable();
    let avg = latencies.iter().sum::<u64>() as f64 / latencies.len() as f64;
    let p50 = latencies[latencies.len() / 2];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];
    let p9999 = latencies[(latencies.len() as f64 * 0.9999).min(latencies.len() as f64 - 1.0) as usize];
    let p100 = *latencies.last().unwrap();

    println!(
        "  {:<30} avg {:>8.1}ns  P50 {:>7}ns  P99 {:>7}ns  P99.99 {:>7}ns  P100 {:>7}ns  ({:.2}s total)",
        name, avg, p50, p99, p9999, p100,
        total.as_secs_f64()
    );
}

// ── main ────────────────────────────────────────────────────────────────────

fn main() {
    let matches = Command::new("mem_test")
        .about("Benchmark raw memory I/O on an mmap'd device/file")
        .arg(Arg::new("path")
            .required(true)
            .help("Path to memory file (e.g. /dev/shm/repCXLnode0)"))
        .arg(Arg::new("size")
            .short('s')
            .long("size")
            .default_value("1048576")
            .value_parser(value_parser!(usize))
            .help("Mmap region size in bytes"))
        .arg(Arg::new("iterations")
            .short('n')
            .long("iterations")
            .default_value("1000000")
            .value_parser(value_parser!(u64))
            .help("Number of iterations per benchmark"))
        .arg(Arg::new("object_size")
            .short('o')
            .long("object-size")
            .default_value("64")
            .value_parser(value_parser!(usize))
            .help("Object size in bytes (how many bytes per read/write)"))
        .get_matches();

    let path: &String = matches.get_one("path").unwrap();
    let region_size: usize = *matches.get_one("size").unwrap();
    let iterations: u64 = *matches.get_one("iterations").unwrap();
    let obj_size: usize = *matches.get_one("object_size").unwrap();

    // open and mmap
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .custom_flags(libc::O_SYNC) // avoid page cache effects
        .open(path)
        .unwrap_or_else(|e| panic!("Failed to open '{}': {}", path, e));

    // ensure file is large enough
    let meta = file.metadata().unwrap();
    if meta.len() < region_size as u64 {
        println!("File is {} bytes, extending to {} bytes", meta.len(), region_size);
        file.set_len(region_size as u64).unwrap();
    }

    let ptr = unsafe {
        mmap(
            std::ptr::null_mut(),
            region_size,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };

    if ptr == libc::MAP_FAILED {
        panic!("mmap failed: {}", std::io::Error::last_os_error());
    }

    let base = ptr as *mut u8;

    println!("mem_test: {} (region={}B, obj={}B, iters={})\n",
        path, region_size, obj_size, iterations);

    // Use offset 4096 to avoid the SharedState header area
    let offset = 4096;
    let addr = unsafe { base.add(offset) };
    assert!(offset + obj_size <= region_size, "Object exceeds region");

    // ── WRITE benchmarks ────────────────────────────────────────────────

    println!("WRITE benchmarks:");
    bench("ptr::write (plain)", iterations, || unsafe {
        write_plain(addr, 0xAB, obj_size);
    });
    bench("write_volatile", iterations, || unsafe {
        write_volatile_only(addr, 0xAB, obj_size);
    });
    bench("write_volatile + clflush", iterations, || unsafe {
        write_volatile_flush(addr, 0xAB, obj_size);
    });

    println!();

    // ── READ benchmarks ─────────────────────────────────────────────────

    println!("READ benchmarks:");
    bench("ptr::read (plain)", iterations, || unsafe {
        std::hint::black_box(read_plain(addr, obj_size));
    });
    bench("read_volatile", iterations, || unsafe {
        std::hint::black_box(read_volatile_only(addr, obj_size));
    });
    bench("read_volatile + clflush", iterations, || unsafe {
        std::hint::black_box(read_flush_volatile(addr, obj_size));
    });

    // cleanup
    unsafe { munmap(ptr, region_size); }

    println!("\nDone.");
}

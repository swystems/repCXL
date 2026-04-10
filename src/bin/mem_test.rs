/// Benchmark raw memory read/write speed on an mmap'd file (tmpfs, ivshmem, DAX, etc.)
/// Compares plain volatile access vs volatile + clflush.
///
/// Usage: mem_test <path> [--size <bytes>] [--iterations <n>] [--object-size <bytes>] [--jump <bytes>] [--struct-read]
///   e.g. mem_test /dev/shm/repCXLnode0 --iterations 1000000 --jump 4096

use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::time::Instant;
use std::cell::Cell;

use clap::{Arg, Command, value_parser};
use libc::{mmap, munmap, MAP_SHARED, PROT_READ, PROT_WRITE};

const CACHE_LINE_SIZE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessPattern {
    Fixed,      // always access offset 0 (cache-friendly)
    Strided,    // access with fixed stride throughout region
    Random,     // random access within region
}

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
    core::arch::x86_64::_mm_mfence();
}

// ── write variants ──────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn write_volatile_only(addr: *mut u8, size: usize) {
    for off in 0..size {
        std::ptr::write_volatile(addr.add(off), 0xAB);
    }
}

#[inline(always)]
unsafe fn write_volatile_flush(addr: *mut u8, size: usize) {
    for off in 0..size {
        std::ptr::write_volatile(addr.add(off), 0xAB);
    }
    cache_flush_fence(addr, size);
}

#[inline(always)]
unsafe fn write_plain(addr: *mut u8, size: usize) {
    for off in 0..size {
        std::ptr::write(addr.add(off), 0xAB);
    }
}

// ── read variants ───────────────────────────────────────────────────────────

#[inline(always)]
unsafe fn read_volatile_only(addr: *mut u8, size: usize) -> u8 {
    let mut v = 0u8;
    for off in 0..size {
        v = std::ptr::read_volatile(addr.add(off));
    }
    v
}

#[inline(always)]
unsafe fn read_flush_volatile(addr: *mut u8, size: usize) -> u8 {
    cache_flush_fence(addr, size);
    let mut v = 0u8;
    for off in 0..size {
        v = std::ptr::read_volatile(addr.add(off));
    }
    v
}

#[inline(always)]
unsafe fn read_plain(addr: *mut u8, size: usize) -> u8 {
    let mut v = 0u8;
    for off in 0..size {
        v = std::ptr::read(addr.add(off));
    }
    v
}

// ── benchmark harness ───────────────────────────────────────────────────────

fn bench<F: Fn() -> u8>(name: &str, iters: u64, f: F) {
    // warmup
    for _ in 0..100 {
        let _ = f();
    }

    let mut latencies: Vec<u64> = Vec::with_capacity(iters as usize);
    let start = Instant::now();
    for _ in 0..iters {
        let t0 = Instant::now();
        let _ = f();
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


fn generate_offsets(region_size: usize, obj_size: usize, pattern: AccessPattern, jump_size: usize, iterations: u64) -> Vec<usize> {
    let mut addrs = Vec::with_capacity(iterations as usize);
    let max_offset = region_size.saturating_sub(obj_size);
    
    match pattern {
        AccessPattern::Fixed => {
            // Always return the same address (cache-friendly baseline)
            for _ in 0..iterations {
                addrs.push(0);
            }
        }
        AccessPattern::Strided => {
            // Access every jump_size bytes through the region
            let stride = jump_size.max(obj_size);
            let mut offset = 0;
            for _ in 0..iterations {
                if offset > max_offset {
                    offset = 0;
                }
                addrs.push(offset);
                offset = (offset + stride) % (max_offset + 1);
            }
        }
        AccessPattern::Random => {
            // Random access within region using a simple LCG
            let mut seed: u64 = 12345;
            for _ in 0..iterations {
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                let offset = ((seed >> 16) as usize) % (max_offset + 1);
                addrs.push(offset);
            }
        }
    }
    addrs
}

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
        .arg(Arg::new("jump")
            .short('j')
            .long("jump")
            .default_value("0")
            .value_parser(value_parser!(usize))
            .help("Jump/stride size in bytes for strided access (0 = fixed address, else = strided pattern)"))
        .arg(Arg::new("pattern")
            .short('p')
            .long("pattern")
            .default_value("fixed")
            .value_parser(["fixed", "strided", "random"])
            .help("Access pattern: fixed, strided, or random"))
        .get_matches();

    let path: &String = matches.get_one("path").unwrap();
    let region_size: usize = *matches.get_one("size").unwrap();
    let iterations: u64 = *matches.get_one("iterations").unwrap();
    let obj_size: usize = *matches.get_one("object_size").unwrap();
    let jump_size: usize = *matches.get_one("jump").unwrap();
    let pattern_str: &String = matches.get_one("pattern").unwrap();
    
    let pattern = match pattern_str.as_str() {
        "fixed" => AccessPattern::Fixed,
        "strided" => AccessPattern::Strided,
        "random" => AccessPattern::Random,
        _ => AccessPattern::Fixed,
    };

    // open and mmap
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // .custom_flags(libc::O_SYNC) // avoid page cache effects
        .open(path)
        .unwrap_or_else(|e| panic!("Failed to open '{}': {}", path, e));

    // ensure file is large enough
    // let meta = file.metadata().unwrap();
    // if meta.len() < region_size as u64 {
    //     println!("File is {} bytes, extending to {} bytes", meta.len(), region_size);
    //     file.set_len(region_size as u64).unwrap();
    // }

    // let offset = 0x280000000;
    let offset = 0;
    let page_size = 2 * 1024 * 1024;
    let page_aligned_size = ((region_size + page_size - 1) / page_size) * page_size;
    let ptr = unsafe {
        mmap(
            std::ptr::null_mut(),
            page_aligned_size,
            PROT_READ | PROT_WRITE,
            MAP_SHARED,
            file.as_raw_fd(),
            offset,
        )
    };

    if ptr == libc::MAP_FAILED {
        panic!("mmap failed: {}", std::io::Error::last_os_error());
    }

    let base = ptr as *mut u8;

    println!("mem_test: {} (region={}B, obj={}B, iters={}, pattern={:?}, jump={}B)\n",
        path, region_size, obj_size, iterations, pattern, jump_size);

    // Generate address sequence based on pattern
    let addresses = generate_offsets(region_size, obj_size, pattern, jump_size, iterations);


    // ── READ benchmarks ─────────────────────────────────────────────────

    println!("READ benchmarks:");
    
    {
        let counter = Cell::new(0usize);
        bench("ptr::read (plain)", iterations, || unsafe {
            let idx = counter.get();
            let offset = addresses[idx % addresses.len()];
            counter.set(idx + 1);
            read_plain(base.add(offset), obj_size)
        });
    }
    
    {
        let counter = Cell::new(0usize);
        bench("read_volatile", iterations, || unsafe {
            let idx = counter.get();
            let offset = addresses[idx % addresses.len()];
            counter.set(idx + 1);
            read_volatile_only(base.add(offset), obj_size)
        });
    }
    
    {
        let counter = Cell::new(0usize);
        bench("read_volatile + clflush", iterations, || unsafe {
            let idx = counter.get();
            let offset = addresses[idx % addresses.len()];
            counter.set(idx + 1);
            read_flush_volatile(base.add(offset), obj_size)
        });
    }

    
    // ── WRITE benchmarks ────────────────────────────────────────────────

    println!("WRITE benchmarks:");
    
    {
        let counter = Cell::new(0usize);
        bench("ptr::write (plain)", iterations, || unsafe {
            let idx = counter.get();
            let offset = addresses[idx % addresses.len()];
            counter.set(idx + 1);
            write_plain(base.add(offset), obj_size);
            0
        });
    }
    
    {
        let counter = Cell::new(0usize);
        bench("write_volatile", iterations, || unsafe {
            let idx = counter.get();
            let offset = addresses[idx % addresses.len()];
            counter.set(idx + 1);
            write_volatile_only(base.add(offset), obj_size);
            0
        });
    }
    
    {
        let counter = Cell::new(0usize);
        bench("write_volatile + clflush", iterations, || unsafe {
            let idx = counter.get();
            let offset = addresses[idx % addresses.len()];
            counter.set(idx + 1);
            write_volatile_flush(base.add(offset), obj_size);
            0
        });
    }

    println!();

    // cleanup
    unsafe { munmap(ptr, region_size); }

    println!("\nDone.");
}

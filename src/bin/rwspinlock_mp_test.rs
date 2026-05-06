use clap::{value_parser, Arg, Command};
use libc::{mmap, munmap, MAP_SHARED, PROT_READ, PROT_WRITE};
use rep_cxl::utils::RWSpinlock;
use std::fs::OpenOptions;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[repr(C, align(64))]
struct SharedData {
    lock: RWSpinlock,
    start_barrier: AtomicUsize,
    done_count: AtomicUsize,
    errors: AtomicUsize,
    writer_inside: AtomicUsize,
    counter: u64,
}

/// SAFETY: Only call when no other process is concurrently using the mapping.
unsafe fn reset_shared_region(shared: *mut SharedData) {
    std::ptr::write_bytes(
        shared.cast::<u8>(),
        0,
        std::mem::size_of::<SharedData>(),
    );
}

fn spin_wait_until<F: Fn() -> bool>(cond: F) {
    while !cond() {
        std::hint::spin_loop();
    }
}

fn main() {
    let matches = Command::new("rwspinlock_mp_test")
        .about("Multi-process sanity test for utils::RWSpinlock using a MAP_SHARED mmap")
        .arg(
            Arg::new("path")
                .short('p')
                .long("path")
                .default_value("/dev/shm/repCXL_rwspinlock_test")
                .help("Shared file path used for MAP_SHARED mmap"),
        )
        .arg(
            Arg::new("id")
                .short('i')
                .long("id")
                .required(true)
                .value_parser(value_parser!(usize))
                .help("Process id (0..procs-1). Use 0 for the initializer/leader."),
        )
        .arg(
            Arg::new("procs")
                .long("procs")
                .default_value("2")
                .value_parser(value_parser!(usize))
                .help("Number of participating processes"),
        )
        .arg(
            Arg::new("mode")
                .long("mode")
                .default_value("writer")
                .value_parser(["writer", "reader"])
                .help("writer: increments shared counter; reader: checks monotonic reads"),
        )
        .arg(
            Arg::new("iters")
                .long("iters")
                .default_value("100000")
                .value_parser(value_parser!(u64))
                .help("Iterations per process"),
        )
        .arg(
            Arg::new("hold_ns")
                .long("hold-ns")
                .default_value("0")
                .value_parser(value_parser!(u64))
                .help("Busy-hold time inside the lock critical section (ns)"),
        )
        .arg(
            Arg::new("init")
                .long("init")
                .action(clap::ArgAction::SetTrue)
                .help("Reset shared region (ONLY run once, typically from id=0, before starting others)"),
        )
        .get_matches();

    let path: &String = matches.get_one("path").unwrap();
    let id: usize = *matches.get_one("id").unwrap();
    let procs: usize = *matches.get_one("procs").unwrap();
    let mode: &String = matches.get_one("mode").unwrap();
    let iters: u64 = *matches.get_one("iters").unwrap();
    let hold_ns: u64 = *matches.get_one("hold_ns").unwrap();
    let init: bool = *matches.get_one("init").unwrap();

    if id >= procs {
        panic!("--id must be in [0, procs) (id={}, procs={})", id, procs);
    }

    let page_size = 4096usize;
    let region_size = std::mem::size_of::<SharedData>();
    let mmap_len = ((region_size + page_size - 1) / page_size) * page_size;

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(path)
        .unwrap_or_else(|e| panic!("Failed to open '{}': {}", path, e));

    file.set_len(mmap_len as u64)
        .unwrap_or_else(|e| panic!("Failed to set_len({}) on '{}': {}", mmap_len, path, e));

    let ptr = unsafe {
        mmap(
            std::ptr::null_mut(),
            mmap_len,
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
    let shared = base as *mut SharedData;

    let align = std::mem::align_of::<SharedData>();
    if (shared as usize) % align != 0 {
        panic!("mmap base not aligned for SharedData (base={:p}, align={})", shared, align);
    }

    if init {
        if id != 0 {
            eprintln!("warning: --init used with id != 0 (id={})", id);
        }
        // Do the reset before creating any shared references.
        unsafe { reset_shared_region(shared) };
        println!("initialized shared region at '{}' ({} bytes mapped)", path, mmap_len);
    }

    // SAFETY: `shared` points to a MAP_SHARED region sized for SharedData.
    let shared_ref: &SharedData = unsafe { &*shared };

    // Barrier: all processes call fetch_add(1), then spin until the count reaches `procs`.
    let before = shared_ref
        .start_barrier
        .fetch_add(1, Ordering::SeqCst);
    let _ = before;
    spin_wait_until(|| shared_ref.start_barrier.load(Ordering::SeqCst) >= procs);

    let start = Instant::now();
    let mut local_errors = 0usize;

    match mode.as_str() {
        "writer" => {
            for _ in 0..iters {
                shared_ref.lock.write_lock();

                // Verify mutual exclusion among writers.
                let prev = shared_ref.writer_inside.fetch_add(1, Ordering::SeqCst);
                if prev != 0 {
                    local_errors += 1;
                }

                // Critical section: increment shared counter.
                unsafe {
                    let p = std::ptr::addr_of!(shared_ref.counter) as *mut u64;
                    let v = std::ptr::read_volatile(p);
                    std::ptr::write_volatile(p, v.wrapping_add(1));
                }

                if hold_ns != 0 {
                    let hold_until = Instant::now() + Duration::from_nanos(hold_ns);
                    while Instant::now() < hold_until {
                        std::hint::spin_loop();
                    }
                }

                shared_ref.writer_inside.fetch_sub(1, Ordering::SeqCst);
                shared_ref.lock.write_unlock();
            }
        }
        "reader" => {
            let mut last = 0u64;
            for _ in 0..iters {
                shared_ref.lock.read_lock();
                let v = unsafe {
                    let p = std::ptr::addr_of!(shared_ref.counter);
                    std::ptr::read_volatile(p)
                };
                if v < last {
                    local_errors += 1;
                }
                last = v;
                shared_ref.lock.read_unlock();

                if hold_ns != 0 {
                    let hold_until = Instant::now() + Duration::from_nanos(hold_ns);
                    while Instant::now() < hold_until {
                        std::hint::spin_loop();
                    }
                }
            }
        }
        _ => unreachable!(),
    }

    if local_errors != 0 {
        shared_ref.errors.fetch_add(local_errors, Ordering::SeqCst);
    }

    let elapsed = start.elapsed();
    let done_prev = shared_ref.done_count.fetch_add(1, Ordering::SeqCst);

    // Leader prints summary.
    if id == 0 {
        spin_wait_until(|| shared_ref.done_count.load(Ordering::SeqCst) >= procs);
        let final_counter = unsafe { std::ptr::read_volatile(std::ptr::addr_of!(shared_ref.counter)) };
        let errors = shared_ref.errors.load(Ordering::SeqCst);
        println!(
            "done: procs={} mode={} iters={} hold_ns={} final_counter={} errors={} (leader waited; last_done_prev={})",
            procs,
            mode,
            iters,
            hold_ns,
            final_counter,
            errors,
            done_prev
        );

        if mode.as_str() == "writer" {
            let expected_min = iters * procs as u64;
            println!("writer-mode expected final_counter ≈ {} (exact if all procs are writers)", expected_min);
        }
    } else {
        println!(
            "proc {} done: mode={} iters={} hold_ns={} elapsed={:?}",
            id, mode, iters, hold_ns, elapsed
        );
    }

    unsafe {
        if munmap(ptr, mmap_len) != 0 {
            eprintln!("munmap failed: {}", std::io::Error::last_os_error());
        }
    }
}

use log::{debug, error, warn};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

const ROUND_SLEEP_RATIO: f64 = 0.5; // Percentage of round time to sleep before busy-waiting

pub fn wait_next_round(next_round: SystemTime, sleep_ratio: f64) {
    // let treshold = next * 2;
    let round_time = next_round
        .duration_since(SystemTime::now())
        .unwrap_or(Duration::from_secs(0));

    if sleep_ratio < 0.0 || sleep_ratio > 1.0 {
        panic!("sleep_ratio must be between 0.0 and 1.0");
    }
    let ns = round_time.as_nanos() as f64;
    let sleep_duration = Duration::from_nanos((ns * sleep_ratio) as u64);

    if round_time > sleep_duration {
        // uses nanosleep() syscall on linux
        std::thread::sleep(sleep_duration);

        // might sleep for more than the requested time
        if SystemTime::now() > next_round {
            return;
        }
    }

    while SystemTime::now() < next_round {
        std::hint::spin_loop();
        //std::thread::yield_now();
    }
    // Every round write  1 object to all memory nodes and read back to verify the write
    // was successful
}
pub fn write_verify<T: Copy + PartialEq + std::fmt::Debug>(
    view: super::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    obj_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
) {
    let mut round_num = 0;
    // wait to start
    let mut next_round = start_time;
    wait_next_round(next_round, ROUND_SLEEP_RATIO);

    loop {
        // logic here
        debug!(
            "Round #{round_num}, delay {:?}",
            SystemTime::now().duration_since(next_round).unwrap()
        );

        match obj_queue_rx.try_recv() {
            Ok((offset, data, ack_tx)) => {
                // write data to all memory nodes
                let mut success = true;
                for node in &view.memory_nodes {
                    let addr = node.addr_at(offset) as *mut T;
                    unsafe {
                        std::ptr::write(addr, data);
                        // *addr = data;
                    }
                    // verify write
                    let read_back = unsafe { std::ptr::read(addr) };
                    if read_back != data {
                        warn!(
                            "Write verification failed on node {}: wrote {:?}, read back {:?}",
                            node.id, data, read_back
                        );
                        success = false;
                    }
                    debug!("Successfully wrote {:?} to node {}", data, node.id);
                }

                // send ack
                if let Err(e) = ack_tx.send(success) {
                    error!("Failed to send ack: {}", e);
                }
            }
            Err(e) => {
                match e {
                    mpsc::TryRecvError::Empty => (),
                    mpsc::TryRecvError::Disconnected => {
                        warn!("Object queue channel closed: {}", e);
                        break; // exit thread
                    }
                }
            }
        }

        next_round += round_time;
        round_num += 1;
        wait_next_round(next_round, ROUND_SLEEP_RATIO);
    }
}

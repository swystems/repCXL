use std::time::{Duration, SystemTime};
use std::sync::mpsc;
use log::{error, warn};

use super::*;


pub fn async_best_effort<T: Copy + PartialEq + std::fmt::Debug>(
    view: crate::GroupView,
    _start_time: SystemTime,
    _round_time: Duration,
    req_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
) {

    loop {
        match req_queue_rx.try_recv() {
            Ok((offset, data, ack_tx)) => {
                // write data to all memory nodes
                for node in &view.memory_nodes {
                    let addr = node.addr_at(offset) as *mut T;
                    safe_memio::safe_write(addr, data).unwrap_or_else(|e| {
                        error!(
                            "Safe write failed at node {} offset {}: {}",
                            node.id, offset, e
                        );
                    });
                }

                if let Err(e) = ack_tx.send(true) {
                        error!("Failed to send ack: {}", e);
                }
            },
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
    }
}


pub fn sync_best_effort<T: Copy + PartialEq + std::fmt::Debug>(
    view: crate::GroupView,
    start_time: SystemTime,
    round_time: Duration,
    req_queue_rx: mpsc::Receiver<(usize, T, mpsc::Sender<bool>)>,
) {
    
    let mut round_num = 0;

    let mut next_round = start_time;
    wait_start_time(start_time, ROUND_SLEEP_RATIO);

    loop {

        debug!(
            "Round #{round_num}, delay {:?}",
            SystemTime::now().duration_since(next_round).unwrap()
        );

        match req_queue_rx.try_recv() {
            Ok((offset, data, ack_tx)) => {
                // write data to all memory nodes
                for node in &view.memory_nodes {
                    let addr = node.addr_at(offset) as *mut T;
                    safe_memio::safe_write(addr, data).unwrap_or_else(|e| {
                        error!(
                            "Safe write failed at node {} offset {}: {}",
                            node.id, offset, e
                        );
                    });
                }

                if let Err(e) = ack_tx.send(true) {
                        error!("Failed to send ack: {}", e);
                }
            },
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

        (round_num, next_round) = wait_next_round(start_time, round_time, ROUND_SLEEP_RATIO);
    }
}
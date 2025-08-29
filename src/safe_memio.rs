//! Safe memory I/O operations are required to prevent crash of a rep_cxl instance
//! when a failure occurs on a memory node.
//! This module relies on an external failure detector to notify the rep_cxl instance
//! and avoid writing to/reading from an invalid pointer.
//!
//! The failure detector mechanism is currently not implemented, this module is used to generate
//! errors for performance testing purposes only.
//!

use log::{debug, error, info, warn};
use rand::Rng;

const FAILURE_PROBABILITY: f32 = 0.0; // 1% chance of failure

pub fn safe_write<T: Copy>(addr: *mut T, data: T) -> Result<(), &'static str> {
    let mut rng = rand::rng();
    let roll: f32 = rng.random(); // random float between 0.0 and 1.0
    if roll < FAILURE_PROBABILITY {
        // error!("Simulated write failure at address {:p}", addr);
        return Err("Simulated write failure");
    }

    unsafe {
        std::ptr::write(addr, data);
    }
    Ok(())
}

pub fn safe_read<T: Copy>(addr: *mut T) -> Result<T, &'static str> {
    let mut rng = rand::rng();
    let roll: f32 = rng.random(); // random float between 0.0 and 1.0
    if roll < FAILURE_PROBABILITY {
        // error!("Simulated read failure at address {:p}", addr);
        return Err("Simulated read failure");
    }

    unsafe { Ok(std::ptr::read(addr)) }
}

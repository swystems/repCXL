//! Safe memory I/O operations are required to prevent crash of a rep_cxl instance
//! when a failure occurs on a memory node.
//! This module relies on an external failure detector to notify the rep_cxl instance
//! and avoid writing to/reading from an invalid pointer.
//!
//! The failure detector mechanism is currently not implemented, this module is used to generate
//! errors for performance testing purposes only.
//!

use rand::Rng;
use crate::ObjectMemoryEntry;
use crate::shmem::MemoryNode;
use log::error;

const FAILURE_PROBABILITY: f32 = 0.0; // 1% chance of failure

#[derive(Debug)]
pub struct MemoryError(pub usize);

pub fn safe_write<T: Copy>(addr: *mut ObjectMemoryEntry<T>, data: ObjectMemoryEntry<T>) -> Result<(), &'static str> {
    let mut rng = rand::rng();
    let roll: f32 = rng.random(); // random float between 0.0 and 1.0
    if roll < FAILURE_PROBABILITY {
        return Err("Simulated write failure");
    }

    unsafe {
        std::ptr::write(addr, data);
    }
    Ok(())
}

pub fn safe_read<T: Copy>(addr: *mut ObjectMemoryEntry<T>) -> Result<ObjectMemoryEntry<T>, &'static str> {
    let mut rng = rand::rng();
    let roll: f32 = rng.random(); // random float between 0.0 and 1.0
    if roll < FAILURE_PROBABILITY {
        return Err("Simulated read failure");
    }

    unsafe { Ok(std::ptr::read(addr)) }
}


/// Write the an ObjectMemoryEntry to all memory nodes at its given memory offset 
pub fn mem_writeall<T: Copy>(offset: usize, ome: ObjectMemoryEntry<T>, mem_nodes: &Vec<MemoryNode>) -> Result<(), MemoryError> {

    // write data to all memory nodes
    for node in mem_nodes {
        let addr = node.addr_at(offset) as *mut ObjectMemoryEntry<T>;
        if let Err(e) = safe_write(addr, ome) {
            error!(
                "Safe write failed at node {} offset {}: {}",
                node.id, offset, e
            );
            return Err(MemoryError(node.id));
        }
    }

    Ok(())
}
    

/// Read the value from all memory nodes for the given object
pub fn mem_readall<T: Copy>(offset: usize, mem_nodes: &Vec<MemoryNode>) -> Result<Vec<ObjectMemoryEntry<T>>, MemoryError> {
    let mut states = Vec::new();
    for node in mem_nodes {
        let addr = node.addr_at(offset) as *mut ObjectMemoryEntry<T>;
        match safe_read(addr) {
            Ok(data) => states.push(data),
            Err(e) => {
                error!(
                    "Safe read failed. Node {}, offset {}: {}",
                    node.id, offset, e
                );
                return Err(MemoryError(node.id));
            }
        }
    }
    Ok(states)
}

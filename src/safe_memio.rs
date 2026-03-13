//! Safe memory I/O operations are required to prevent crash of a rep_cxl instance
//! when a failure occurs on a memory node.
//! This module relies on an external failure detector to notify the rep_cxl instance
//! and avoid writing to/reading from an invalid pointer.
//!
//! The failure detector mechanism is currently not implemented, this module is used to generate
//! errors for performance testing purposes only.
//!

use rand::Rng;
use rand::prelude::IndexedRandom;  // Enables choose() on slices
use crate::request::Wid;
use crate::shmem::MemoryNode;
use log::error;
use std::mem::size_of;
use core::arch::x86_64::{_mm_mfence, _mm_sfence};

const FAILURE_PROBABILITY: f32 = 0.0;
pub const CACHE_LINE_SIZE: usize = 64;

#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub(crate) unsafe fn clflushopt(addr: *const u8) {
    core::arch::asm!("clflushopt [{}]", in(reg) addr, options(nostack, preserves_flags));
}

/// Flush cache lines covering `size` bytes starting at `addr` using pipelined
/// clflushopt. Does NOT issue a fence — caller must follow with the
/// appropriate fence (sfence for write path, mfence for read path).
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub(crate) unsafe fn clflushopt_range(addr: *const u8, size: usize) {
    let end = addr.add(size);
    let mut ptr = addr;
    while ptr < end {
        clflushopt(ptr);
        ptr = ptr.add(CACHE_LINE_SIZE);
    }
}

/// Flush + sfence: ensures writes are globally visible before subsequent stores.
/// Use after write_volatile.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub(crate) unsafe fn cache_flush_write(addr: *const u8, size: usize) {
    clflushopt_range(addr, size);
    _mm_sfence();
}

/// Flush + mfence: ensures cache lines are evicted before subsequent loads.
/// Use before read_volatile to guarantee reads come from memory.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub(crate) unsafe fn cache_flush_read(addr: *const u8, size: usize) {
    clflushopt_range(addr, size);
    _mm_mfence();
}

pub(crate) fn mem_write_flush<T: Copy>(addr: *mut T, data: T) {
    unsafe {
        std::ptr::write_volatile(addr, data);
        cache_flush_write(addr as *const u8, std::mem::size_of::<T>());
    }
}

#[derive(Debug)]
pub struct MemoryError(pub usize);


/// ObjectMemoryEntry. Stores the current write ID and the value of the object
/// in memory.
#[derive(Debug, Clone, Copy)]
pub struct ObjectMemoryEntry<T> {
    pub wid: Wid,
    pub value: T,
}

impl<T: Copy> ObjectMemoryEntry<T> {
    pub fn new(wid: Wid, value: T) -> Self {
        ObjectMemoryEntry { wid, value }
    }

    pub fn new_nowid(value: T) -> Self {
        ObjectMemoryEntry {
            wid: Wid::new(0, 0),
            value,
        }
    }
}

pub fn safe_write<T: Copy>(addr: *mut ObjectMemoryEntry<T>, data: ObjectMemoryEntry<T>) -> Result<(), &'static str> {
    if FAILURE_PROBABILITY > 0.0 {
        let mut rng = rand::rng();
        let roll: f32 = rng.random(); // random float between 0.0 and 1.0
        if roll < FAILURE_PROBABILITY {
            return Err("Simulated write failure");
        }
    }

    // mechanism to handle segfault here, signal catch plus backup process
    unsafe { std::ptr::write_volatile(addr, data); }
    Ok(())
}

pub fn safe_read<T: Copy>(addr: *mut ObjectMemoryEntry<T>) -> Result<ObjectMemoryEntry<T>, &'static str> {
    if FAILURE_PROBABILITY > 0.0 {
        let mut rng = rand::rng();
        let roll: f32 = rng.random(); // random float between 0.0 and 1.0
        if roll < FAILURE_PROBABILITY {
            return Err("Simulated read failure");
        }
    }
    // mechanism to handle segfault here, signal catch plus backup process

    Ok(unsafe { std::ptr::read_volatile(addr) })
}

/// Read the value from all memory nodes for the given object
pub fn _mem_readone<T: Copy>(offset: usize, mem_nodes: &Vec<MemoryNode>) -> Result<ObjectMemoryEntry<T>, MemoryError> {

    let node = mem_nodes.choose(&mut rand::rng()).unwrap();  // Returns Option<&T>

    let addr = node.addr_at(offset) as *mut ObjectMemoryEntry<T>;
    match safe_read(addr) {
        Ok(ome) => return Ok(ome),
        Err(e) => {
            error!(
                "Safe read failed. Node {}, offset {}: {}",
                node.id, offset, e
            );
            return Err(MemoryError(node.id));
        }
    }
}

/// Write the an ObjectMemoryEntry to all memory nodes at its given memory offset 
/// Flush&fence to ensure visibility
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
        // flush
        // unsafe { clflushopt_range(addr  as *const u8, size_of::<ObjectMemoryEntry<T>>()); }
    }

    // fence once only after all writes to all mem nodes are flushed
    // unsafe { _mm_mfence(); }

    Ok(())
}
    

/// Read the value from all memory nodes for the given object
pub fn mem_readall<T: Copy>(offset: usize, mem_nodes: &Vec<MemoryNode>) -> Result<Vec<ObjectMemoryEntry<T>>, MemoryError> {
    let mut states = Vec::with_capacity(mem_nodes.len());
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


/// Read the value from the first and last memory nodes only, exploiting the
/// fact that memory nodes are written to always in the same order. Used for
/// scalability improvements
pub fn mem_readends<T: Copy>(offset: usize, mem_nodes: &Vec<MemoryNode>) -> Result<[ObjectMemoryEntry<T>; 2], MemoryError> {
    
    // read the first node
    let first_node = &mem_nodes[0];
    let last_node = &mem_nodes[mem_nodes.len() - 1];

    // // flush nodes
    // let nodes = [first_node, last_node];
    // for node in nodes {
    //     let addr = node.addr_at(offset);
    //     unsafe { clflushopt_range(addr, size_of::<ObjectMemoryEntry<T>>()); }
    // }

    // unsafe { _mm_mfence(); }

    // now read both from memory
     
    let mut addr = first_node.addr_at(offset) as *mut ObjectMemoryEntry<T>;
    let first = match safe_read(addr) {
        Ok(data) => data,
        Err(e) => {
            error!(
                "Safe read failed. Node {}, offset {}: {}",
                first_node.id, offset, e
            );
            return Err(MemoryError(first_node.id));
        }
    };
    
    // read the last node
    addr = last_node.addr_at(offset) as *mut ObjectMemoryEntry<T>;
    let last = match safe_read(addr) {
        Ok(data) => data,
        Err(e) => {
            error!(
                "Safe read failed. Node {}, offset {}: {}",
                last_node.id, offset, e
            );
            return Err(MemoryError(last_node.id));
        }
    };

    Ok([first, last])
}


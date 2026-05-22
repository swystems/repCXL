use crate::{utils::RWSpinlock};

use super::MAX_OBJECTS;
use log::{info, warn};

const CHUNK_SIZE: usize = 64; // required for write operation alignment constraints


#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectInfo {
    pub id: usize,
    pub offset: usize,
    pub size: usize,
}

impl ObjectInfo {
    pub fn new(id: usize, offset: usize, size: usize) -> Self {
        ObjectInfo { id, offset, size }
    }
}

#[derive(Clone, Debug)]
pub struct ObjectIndexEntry {
    pub info: ObjectInfo,
    pub lock: RWSpinlock,
}

impl ObjectIndexEntry {
    pub fn new(info: ObjectInfo) -> Self {
        ObjectIndexEntry {
            info,
            lock: RWSpinlock::new(),
        }
    }
}

/// Memory allocation information. Process coordinator has write access
/// while replicas have read-only access.
///
/// Note: This index is stored inside shared memory; keep layout stable.
#[repr(C)]
#[derive(Clone, Debug)]
pub struct ObjectIndex {
    total_size: usize,
    allocated_size: usize,
    object_index: [Option<ObjectIndexEntry>; MAX_OBJECTS],
}


impl ObjectIndex {
    pub(crate) fn new(total_size: usize) -> Self {
        ObjectIndex {
            total_size,
            allocated_size: 0,
            object_index: std::array::from_fn(|_| None),
        }
    }

    /// Get the object info in from the index.
    /// Returns Some<offset> if found, None otherwise.
    /// ## Arguments
    /// * `id` - Unique identifier for the object.
    ///
    /// ## Returns
    /// * `Some((index, ObjectInfo))` if lookup is successful, where `index` \
    /// is the index in the object_index array where the object was allocated.
    /// * `None` if lookup fails due to insufficient space or duplicate id.
    pub(crate) fn lookup_object(&self, id: usize) -> Option<(usize, ObjectInfo)> {
        let mut i = 0;
        for entry in &self.object_index {
            if let Some(oie) = entry {
                if oie.info.id == id {
                    return Some((i, oie.info));
                }
            }
            i += 1;
        }
        None
    }

    /// Allocates an object in the first free slot (first fit allocation)
    /// Returns Some<offset> if a suitable slot is found, otherwise None.
    ///
    /// ## Arguments
    /// * `id` - Unique identifier for the object.
    /// * `size` - Size of the memory to allocate.
    /// 
    /// ## Returns
    /// * `Some((index, ObjectInfo))` if allocation is successful, where `index` \
    /// is the index in the object_index array where the object was allocated.
    /// * `None` if allocation fails due to insufficient space or duplicate id.
    pub(crate) fn alloc_object(&mut self, id: usize, size: usize) -> Option<(usize, ObjectInfo)> {
        let chunks = (size + CHUNK_SIZE - 1) / CHUNK_SIZE; // Round up to nearest chunk size
        let size = chunks * CHUNK_SIZE;

        if self.allocated_size + size > self.total_size {
            warn!("Not enough space");
            return None;
        }

        if self.lookup_object(id).is_some() {
            info!("Object with id {} already exists", id);
            return None;
        }

        // suboptimal allocation algorithm
        // loses space when a smaller object takes the place of a larger one which was freed
        for i in 0..MAX_OBJECTS {
            let entry = &self.object_index[i];
            if entry.is_none() {

                let start = if i == 0 {
                    0
                } else {
                    self.object_index[i - 1]
                        .as_ref()
                        .map(|e| e.info.offset as usize + e.info.size)
                        .expect("Previous entry should exist")
                };

                let mut end = self.total_size;
                for j in (i + 1)..MAX_OBJECTS {
                    if let Some(obj) = self.object_index[j].as_ref() {
                        end = obj.info.offset;
                        break;
                    }
                }

                if start + size <= end {
                    let oi = ObjectInfo::new(id, start, size);
                    self.object_index[i] = Some(ObjectIndexEntry::new(oi));
                    self.allocated_size += size;
                    return Some((i, oi));
                }
            }
        }

        warn!("Failed allocation: no free slot available");
        None
    }

    /// Removes an object from the state by its id
    pub(crate) fn dealloc_object(&mut self, id: usize) {
        self.object_index.iter_mut().for_each(|entry| {
            if let Some(obj) = entry {
                if obj.info.id == id {
                    self.allocated_size -= obj.info.size;
                    *entry = None; // Mark as free
                }
            }
        });
    }

    /// Get lock of a given object by its index position. 
    /// Returns Some(&RWSpinlock) if found, None otherwise.
    pub(crate) fn get_lock(&self, object_index_pos: usize) -> Option<&RWSpinlock> {
        self.object_index[object_index_pos]
            .as_ref()
            .map(|entry| &entry.lock)
    }
}

use crate::MAX_OBJECTS;
use log::{info, warn};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ObjectInfo {
    id: usize,
    pub(crate) offset: usize,
    pub size: usize,
}

impl ObjectInfo {
    pub(crate) fn new(id: usize, offset: usize, size: usize) -> Self {
        ObjectInfo { id, offset, size }
    }
}

/// Memory allocation information. Process coordinator has write acess
/// while replicas have read-only access.
///
/// @TODO: add coordinator-only write checks
#[derive(Copy, Clone, Debug)]
pub(crate) struct Allocator {
    total_size: usize,
    allocated_size: usize,
    chunk_size: usize,
    object_index: [Option<ObjectInfo>; MAX_OBJECTS],
}

impl Allocator {
    pub(crate) fn new(total_size: usize, chunk_size: usize) -> Self {
        Allocator {
            total_size,
            allocated_size: 0,
            chunk_size,
            object_index: [None; MAX_OBJECTS], // Initialize with None
        }
    }

    /// Get the object info in from the index.
    /// Returns Some<offset> if found, None otherwise.
    /// # Arguments
    /// * `id` - Unique identifier for the object.
    pub(crate) fn lookup_object(&self, id: usize) -> Option<ObjectInfo> {
        for entry in self.object_index {
            if let Some(obj) = entry {
                if obj.id == id {
                    return entry;
                }
            }
        }
        None
    }

    /// Allocates an object in the first free slot (first fit allocation)
    /// Returns Some<offset> if a suitable slot is found, otherwise None.
    ///
    /// @TODO: better allocation algorithm
    ///
    /// ## Arguments
    /// * 'id' - Unique identifier for the object.
    /// * `size` - Size of the memory to allocate.
    pub(crate) fn alloc_object(&mut self, id: usize, size: usize) -> Option<usize> {
        let chunks = (size + self.chunk_size - 1) / self.chunk_size; // Round up to nearest chunk size
        let size = chunks * self.chunk_size;

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
            let entry = self.object_index[i];
            if entry.is_none() {

                let start = if i == 0 {
                    0
                } else {
                    self.object_index[i - 1]
                        .map(|e| e.offset as usize + e.size)
                        .expect("Previous entry should exist")
                };

                let mut end = self.total_size;
                for j in (i + 1)..MAX_OBJECTS {
                    if let Some(obj) = self.object_index[j] {
                        end = obj.offset;
                        break;
                    }
                }

                if start + size <= end {
                    self.object_index[i] = Some(ObjectInfo::new(id, start, size));
                    self.allocated_size += size;
                    return Some(start);
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
                if obj.id == id {
                    self.allocated_size -= obj.size;
                    *entry = None; // Mark as free
                }
            }
        });
    }
}

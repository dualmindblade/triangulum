//! Edited chunks: the overlay the player writes.
//!
//! An edited LOD-0 chunk is authoritative: it is held in full (dense ids)
//! and is never regenerated. Generation stays a pure function; `World`
//! applies this overlay after it. The store also tracks which chunks are
//! unsaved (dirty) and which loaded chunks at any LOD must be remeshed
//! (invalidated), so saving and streaming stay out of the edit path.

use super::BlockId;
use super::chunk::{VOXELS, index};
use crate::grid::{ChunkKey, FastMap, FastSet};
use std::sync::RwLock;
use std::time::Instant;

#[derive(Default)]
struct Inner {
    chunks: FastMap<ChunkKey, Vec<BlockId>>,
    dirty: FastSet<ChunkKey>,
    invalidated: Vec<ChunkKey>,
    last_edit: Option<Instant>,
    first_dirty: Option<Instant>,
    /// Bumps on every change (cheap "anything new?" check).
    generation: u64,
}

#[derive(Default)]
pub struct EditStore {
    inner: RwLock<Inner>,
}

/// Save dirty chunks when the player has paused this long…
pub const SAVE_QUIET_S: f64 = 2.0;
/// …or at the latest this long after the first unsaved edit.
pub const SAVE_MAX_S: f64 = 15.0;

impl EditStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().chunks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.read().unwrap().chunks.len()
    }

    pub fn generation(&self) -> u64 {
        self.inner.read().unwrap().generation
    }

    pub fn contains(&self, key: ChunkKey) -> bool {
        self.inner.read().unwrap().chunks.contains_key(&key)
    }

    /// Block of an edited chunk; None when the chunk is not edited.
    pub fn get(&self, key: ChunkKey, x: usize, y: usize, z: usize) -> Option<BlockId> {
        self.inner.read().unwrap().chunks.get(&key).map(|d| d[index(x, y, z)])
    }

    /// Run `f` over an edited chunk's dense ids (under the read lock).
    pub fn with_chunk<R>(&self, key: ChunkKey, f: impl FnOnce(&[BlockId]) -> R) -> Option<R> {
        self.inner.read().unwrap().chunks.get(&key).map(|d| f(d))
    }

    /// Every edited chunk key (LOD 0).
    pub fn keys(&self) -> Vec<ChunkKey> {
        self.inner.read().unwrap().chunks.keys().copied().collect()
    }

    /// Set one block. `natural` produces the generator's dense chunk when
    /// this chunk is edited for the first time. Returns whether anything
    /// changed. Marks the chunk dirty; the caller computes invalidations.
    pub fn set(&self, key: ChunkKey, x: usize, y: usize, z: usize, id: BlockId, natural: impl FnOnce() -> Vec<BlockId>) -> bool {
        debug_assert_eq!(key.lod, 0);
        let mut g = self.inner.write().unwrap();
        let dense = g.chunks.entry(key).or_insert_with(|| {
            let d = natural();
            assert_eq!(d.len(), VOXELS);
            d
        });
        let slot = &mut dense[index(x, y, z)];
        if *slot == id {
            return false;
        }
        *slot = id;
        let now = Instant::now();
        g.dirty.insert(key);
        g.last_edit = Some(now);
        g.first_dirty.get_or_insert(now);
        g.generation += 1;
        true
    }

    /// Insert a whole chunk (loading a save). Not dirty unless asked.
    pub fn insert_chunk(&self, key: ChunkKey, dense: Vec<BlockId>, dirty: bool) {
        assert_eq!(dense.len(), VOXELS);
        let mut g = self.inner.write().unwrap();
        g.chunks.insert(key, dense);
        if dirty {
            let now = Instant::now();
            g.dirty.insert(key);
            g.last_edit = Some(now);
            g.first_dirty.get_or_insert(now);
        }
        g.generation += 1;
    }

    pub fn push_invalidated(&self, keys: impl IntoIterator<Item = ChunkKey>) {
        self.inner.write().unwrap().invalidated.extend(keys);
    }

    /// Chunks (any LOD) whose meshes went stale since the last call.
    pub fn take_invalidated(&self) -> Vec<ChunkKey> {
        let mut g = self.inner.write().unwrap();
        let mut v = std::mem::take(&mut g.invalidated);
        v.sort();
        v.dedup();
        v
    }

    pub fn pending_invalidations(&self) -> usize {
        self.inner.read().unwrap().invalidated.len()
    }

    pub fn dirty_count(&self) -> usize {
        self.inner.read().unwrap().dirty.len()
    }

    /// Debounce rule: dirty chunks and either a pause since the last edit
    /// or a long enough time since the first unsaved one.
    pub fn due_for_save(&self, now: Instant) -> bool {
        let g = self.inner.read().unwrap();
        if g.dirty.is_empty() {
            return false;
        }
        let quiet = g.last_edit.is_some_and(|t| now.duration_since(t).as_secs_f64() >= SAVE_QUIET_S);
        let long = g.first_dirty.is_some_and(|t| now.duration_since(t).as_secs_f64() >= SAVE_MAX_S);
        quiet || long
    }

    /// The dirty chunks' dense ids, clearing the dirty set.
    pub fn take_dirty(&self) -> Vec<(ChunkKey, Vec<BlockId>)> {
        let mut g = self.inner.write().unwrap();
        let keys: Vec<ChunkKey> = g.dirty.drain().collect();
        g.first_dirty = None;
        let mut out: Vec<(ChunkKey, Vec<BlockId>)> = keys.into_iter().filter_map(|k| g.chunks.get(&k).map(|d| (k, d.clone()))).collect();
        out.sort_by_key(|(k, _)| *k);
        out
    }

    /// Every edited chunk (for tests and whole-store saves).
    pub fn snapshot(&self) -> Vec<(ChunkKey, Vec<BlockId>)> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<(ChunkKey, Vec<BlockId>)> = g.chunks.iter().map(|(k, d)| (*k, d.clone())).collect();
        out.sort_by_key(|(k, _)| *k);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_tracks_change_and_dirt() {
        let s = EditStore::new();
        let key = ChunkKey { face: 1, lod: 0, cx: 3, cy: 4, cz: 5 };
        let natural = || vec![1u16; VOXELS];
        assert!(!s.set(key, 0, 0, 0, 1, natural), "same block is not a change");
        assert_eq!(s.len(), 1, "the chunk becomes authoritative on first touch");
        assert_eq!(s.dirty_count(), 0);
        assert!(s.set(key, 0, 0, 0, 0, natural));
        assert_eq!(s.get(key, 0, 0, 0), Some(0));
        assert_eq!(s.get(key, 0, 0, 1), Some(1));
        assert_eq!(s.dirty_count(), 1);
        assert!(!s.due_for_save(Instant::now()), "not yet quiet");
        assert!(s.due_for_save(Instant::now() + std::time::Duration::from_secs(3)));
        let d = s.take_dirty();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].1[0], 0);
        assert_eq!(s.dirty_count(), 0);
        s.push_invalidated([key, key, ChunkKey { lod: 1, ..key }]);
        assert_eq!(s.take_invalidated().len(), 2);
        assert!(s.take_invalidated().is_empty());
    }
}

//! Edited chunks: the overlay the player writes.
//!
//! An edited LOD-0 chunk is authoritative: it is held in full (dense ids)
//! and is never regenerated. Generation stays a pure function; `World`
//! applies this overlay after it. The store also tracks which chunks are
//! unsaved (dirty) and which loaded chunks at any LOD must be remeshed
//! (invalidated), so saving and streaming stay out of the edit path.
//!
//! A spatial index keyed by ancestor chunk (every LOD up to `INDEX_LODS`)
//! answers "which edited chunks lie under this coarse chunk?" without a
//! scan, so generating a coarse chunk costs the edits near it, not every
//! edit on the planet.

use super::BlockId;
use super::chunk::{VOXELS, index};
use crate::grid::{ChunkKey, FastMap, FastSet};
use std::sync::RwLock;
use std::time::Instant;

/// Ancestors indexed per edited chunk: LODs 1..=INDEX_LODS (a chunk at LOD
/// 15 is 2^20 blocks wide, more than any supported face).
const INDEX_LODS: u8 = 15;

#[derive(Default)]
struct Inner {
    chunks: FastMap<ChunkKey, Vec<BlockId>>,
    /// Edited LOD-0 chunks under each ancestor (LODs 1..=INDEX_LODS).
    under: FastMap<ChunkKey, Vec<ChunkKey>>,
    dirty: FastSet<ChunkKey>,
    invalidated: Vec<ChunkKey>,
    last_edit: Option<Instant>,
    first_dirty: Option<Instant>,
    /// Bumps on every change (cheap "anything new?" check).
    generation: u64,
}

impl Inner {
    fn index(&mut self, key: ChunkKey) {
        let mut anc = key;
        for _ in 0..INDEX_LODS {
            anc = anc.parent().expect("lod < 63");
            self.under.entry(anc).or_default().push(key);
        }
    }
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

    /// The edited LOD-0 chunks inside `anc` (itself, at LOD 0).
    pub fn edited_under(&self, anc: ChunkKey) -> Vec<ChunkKey> {
        let g = self.inner.read().unwrap();
        if anc.lod == 0 {
            return if g.chunks.contains_key(&anc) { vec![anc] } else { Vec::new() };
        }
        if anc.lod > INDEX_LODS {
            return g.chunks.keys().filter(|k| anc.contains(k)).copied().collect();
        }
        g.under.get(&anc).cloned().unwrap_or_default()
    }

    /// Set one block. `natural` produces the generator's dense chunk when
    /// this chunk is edited for the first time. Returns whether anything
    /// changed. Marks the chunk dirty; the caller computes invalidations.
    pub fn set(&self, key: ChunkKey, x: usize, y: usize, z: usize, id: BlockId, natural: impl FnOnce() -> Vec<BlockId>) -> bool {
        debug_assert_eq!(key.lod, 0);
        let mut g = self.inner.write().unwrap();
        if !g.chunks.contains_key(&key) {
            let d = natural();
            assert_eq!(d.len(), VOXELS);
            g.chunks.insert(key, d);
            g.index(key);
        }
        let dense = g.chunks.get_mut(&key).expect("just inserted");
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
        if g.chunks.insert(key, dense).is_none() {
            g.index(key);
        }
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
        let key = ChunkKey::neisor(1, 0, 3, 4, 5);
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

    #[test]
    fn the_ancestor_index_finds_edits_under_a_coarse_chunk() {
        let s = EditStore::new();
        let natural = || vec![1u16; VOXELS];
        let a = ChunkKey::neisor(2, 0, 100, 200, 17);
        let b = ChunkKey::neisor(2, 0, 101, 200, 17); // same LOD-1 parent as a
        let c = ChunkKey::neisor(2, 0, 160, 200, 17); // same LOD-8 ancestor, not LOD-7
        let d = ChunkKey::neisor(3, 0, 100, 200, 17); // another face
        for k in [a, b, c, d] {
            s.set(k, 0, 0, 0, 0, natural);
        }
        s.insert_chunk(a, natural(), false); // re-inserting does not double-index
        let under = |anc: ChunkKey| {
            let mut v = s.edited_under(anc);
            v.sort();
            v
        };
        assert_eq!(under(a.parent().unwrap()), vec![a, b]);
        assert_eq!(under(ChunkKey::neisor(2, 5, 100 >> 5, 200 >> 5, 17 >> 5)), vec![a, b]);
        assert_eq!(under(ChunkKey::neisor(2, 7, 100 >> 7, 200 >> 7, 17 >> 7)), vec![a, b]);
        assert_eq!(under(ChunkKey::neisor(2, 7, 160 >> 7, 200 >> 7, 17 >> 7)), vec![c]);
        assert_eq!(under(ChunkKey::neisor(2, 8, 0, 0, 0)), vec![a, b, c]);
        assert_eq!(under(ChunkKey::neisor(2, 6, 3, 200 >> 6, 17 >> 6)), Vec::<ChunkKey>::new());
        assert_eq!(under(ChunkKey::neisor(3, 9, 0, 0, 0)), vec![d]);
        assert_eq!(under(a), vec![a]);
        assert_eq!(under(ChunkKey::neisor(2, 0, 5, 5, 5)), Vec::<ChunkKey>::new());
        // Beyond the indexed LODs it falls back to a scan and still answers.
        assert_eq!(under(ChunkKey::neisor(2, 16, 0, 0, 0)), vec![a, b, c]);
    }
}

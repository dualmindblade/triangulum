//! Edited chunks: the authoritative overlay the player and the simulation
//! write.
//!
//! An edited LOD-0 chunk is held in full (dense ids, water levels and a
//! revision) and is never regenerated. Generation stays a pure function;
//! `World` applies this overlay after it. The store also tracks which chunks
//! are unsaved (dirty, by revision), which loaded chunks at any LOD must be
//! remeshed (invalidated) and which blocks changed (for the simulation's
//! neighbour notifications), so saving, streaming and the simulation stay
//! out of each other's way. All three are fed by one batch commit.
//!
//! Saving is acknowledged, not assumed (review #2): `dirty_snapshot` hands
//! out the dirty chunks without clearing anything, and `acknowledge` clears
//! only the revisions that were actually persisted, so a failed write is
//! retried on the next save and a chunk edited during a save stays dirty.
//!
//! A spatial index keyed by ancestor chunk (every LOD up to `INDEX_LODS`)
//! answers "which edited chunks lie under this coarse chunk?" without a
//! scan, so generating a coarse chunk costs the edits near it, not every
//! edit on the planet.

use super::BlockId;
use super::chunk::{VOXELS, index};
use crate::grid::{BlockPos, ChunkKey, FastMap, FastSet};
use std::sync::RwLock;
use std::time::Instant;

/// Ancestors indexed per edited chunk: LODs 1..=INDEX_LODS (a chunk at LOD
/// 15 is 2^20 blocks wide, more than any supported face).
const INDEX_LODS: u8 = 15;

/// Water level of a block that is water (M2b's levels replace this).
pub const WATER_FULL: u8 = 255;

/// One edited chunk, in full.
pub struct Edited {
    pub ids: Vec<BlockId>,
    /// Water level per voxel; None when no voxel holds water.
    pub water: Option<Vec<u8>>,
    /// Bumps on every change to this chunk.
    pub rev: u64,
}

/// One block write for `commit`: chunk, voxel, new id, and the water level
/// (None: full for a liquid block, empty otherwise).
#[derive(Clone, Copy, Debug)]
pub struct Write {
    pub key: ChunkKey,
    pub x: usize,
    pub y: usize,
    pub z: usize,
    pub id: BlockId,
    pub level: Option<u8>,
}

/// A dirty chunk as a save sees it.
pub struct DirtyChunk {
    pub key: ChunkKey,
    pub rev: u64,
    pub ids: Vec<BlockId>,
    pub water: Option<Vec<u8>>,
}

#[derive(Default)]
struct Inner {
    chunks: FastMap<ChunkKey, Edited>,
    /// Edited LOD-0 chunks under each ancestor (LODs 1..=INDEX_LODS).
    under: FastMap<ChunkKey, Vec<ChunkKey>>,
    dirty: FastSet<ChunkKey>,
    invalidated: Vec<ChunkKey>,
    /// Every block that changed, in write order, whoever changed it: the
    /// simulation drains this each tick to notify the neighbours.
    changed: Vec<BlockPos>,
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

    fn mark_dirty(&mut self, key: ChunkKey) {
        let now = Instant::now();
        self.dirty.insert(key);
        self.last_edit = Some(now);
        self.first_dirty.get_or_insert(now);
        self.generation += 1;
    }

    /// Apply one write to a chunk already in the map; whether it changed anything.
    fn write(&mut self, w: &Write, liquid: bool) -> bool {
        let e = self.chunks.get_mut(&w.key).expect("chunk present");
        let v = index(w.x, w.y, w.z);
        let level = w.level.unwrap_or(if liquid { WATER_FULL } else { 0 });
        let had_level = e.water.as_ref().map_or(0, |l| l[v]);
        if e.ids[v] == w.id && had_level == level {
            return false;
        }
        e.ids[v] = w.id;
        if level != had_level {
            if level != 0 && e.water.is_none() {
                e.water = Some(vec![0u8; VOXELS]);
            }
            if let Some(l) = e.water.as_mut() {
                l[v] = level;
                if level == 0 && l.iter().all(|&x| x == 0) {
                    e.water = None;
                }
            }
        }
        e.rev += 1;
        true
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
        self.inner.read().unwrap().chunks.get(&key).map(|e| e.ids[index(x, y, z)])
    }

    /// Water level of an edited chunk's voxel; None when the chunk is not edited.
    pub fn water_level(&self, key: ChunkKey, x: usize, y: usize, z: usize) -> Option<u8> {
        self.inner.read().unwrap().chunks.get(&key).map(|e| e.water.as_ref().map_or(0, |l| l[index(x, y, z)]))
    }

    /// Revision of an edited chunk; None when the chunk is not edited.
    pub fn revision(&self, key: ChunkKey) -> Option<u64> {
        self.inner.read().unwrap().chunks.get(&key).map(|e| e.rev)
    }

    /// Run `f` over an edited chunk's dense ids (under the read lock).
    pub fn with_chunk<R>(&self, key: ChunkKey, f: impl FnOnce(&[BlockId]) -> R) -> Option<R> {
        self.inner.read().unwrap().chunks.get(&key).map(|e| f(&e.ids))
    }

    /// Run `f` over an edited chunk's ids and water levels (under the read lock).
    pub fn with_edited<R>(&self, key: ChunkKey, f: impl FnOnce(&Edited) -> R) -> Option<R> {
        self.inner.read().unwrap().chunks.get(&key).map(f)
    }

    /// Every edited chunk key (LOD 0), sorted.
    pub fn keys(&self) -> Vec<ChunkKey> {
        let mut v: Vec<ChunkKey> = self.inner.read().unwrap().chunks.keys().copied().collect();
        v.sort();
        v
    }

    /// Is any chunk inside `anc` edited (itself, at LOD 0)?
    pub fn any_under(&self, anc: ChunkKey) -> bool {
        let g = self.inner.read().unwrap();
        if anc.lod == 0 {
            return g.chunks.contains_key(&anc);
        }
        if anc.lod > INDEX_LODS {
            return g.chunks.keys().any(|k| anc.contains(k));
        }
        g.under.get(&anc).is_some_and(|v| !v.is_empty())
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

    /// Apply a batch of writes under one lock. `natural` produces the
    /// generator's dense ids and water levels for a chunk edited for the
    /// first time; `liquid` says whether an id is water (its level defaults
    /// to full). Returns, per write, whether it changed anything; changed
    /// chunks are marked dirty. The caller computes invalidations and the
    /// change log from the result (`World::commit` does).
    pub fn commit(&self, writes: &[Write], mut natural: impl FnMut(ChunkKey) -> (Vec<BlockId>, Option<Vec<u8>>), liquid: impl Fn(BlockId) -> bool) -> Vec<bool> {
        let mut g = self.inner.write().unwrap();
        let mut out = Vec::with_capacity(writes.len());
        for w in writes {
            debug_assert_eq!(w.key.lod, 0);
            if !g.chunks.contains_key(&w.key) {
                let (ids, water) = natural(w.key);
                assert_eq!(ids.len(), VOXELS);
                if let Some(l) = &water {
                    assert_eq!(l.len(), VOXELS);
                }
                g.chunks.insert(w.key, Edited { ids, water, rev: 0 });
                g.index(w.key);
            }
            let changed = g.write(w, liquid(w.id));
            if changed {
                g.mark_dirty(w.key);
            }
            out.push(changed);
        }
        out
    }

    /// Set one block (tests and tools). `natural` produces the generator's
    /// dense ids when this chunk is edited for the first time; a liquid id
    /// gets a full water level. Returns whether anything changed.
    pub fn set(&self, key: ChunkKey, x: usize, y: usize, z: usize, id: BlockId, natural: impl FnOnce() -> Vec<BlockId>) -> bool {
        let mut natural = Some(natural);
        self.commit(&[Write { key, x, y, z, id, level: None }], |_| (natural.take().expect("one chunk")(), None), |_| false)[0]
    }

    /// Insert a whole chunk (loading a save). Not dirty unless asked.
    pub fn insert_chunk(&self, key: ChunkKey, ids: Vec<BlockId>, water: Option<Vec<u8>>, dirty: bool) {
        assert_eq!(ids.len(), VOXELS);
        if let Some(l) = &water {
            assert_eq!(l.len(), VOXELS);
        }
        let mut g = self.inner.write().unwrap();
        let water = water.filter(|l| l.iter().any(|&x| x != 0));
        if g.chunks.insert(key, Edited { ids, water, rev: 0 }).is_none() {
            g.index(key);
        }
        if dirty {
            g.mark_dirty(key);
        } else {
            g.generation += 1;
        }
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

    /// Record a changed block for the simulation (`World::commit` does this).
    pub fn push_changed(&self, bps: impl IntoIterator<Item = BlockPos>) {
        self.inner.write().unwrap().changed.extend(bps);
    }

    /// Blocks changed since the last call, in write order.
    pub fn take_changed(&self) -> Vec<BlockPos> {
        std::mem::take(&mut self.inner.write().unwrap().changed)
    }

    pub fn pending_changes(&self) -> usize {
        self.inner.read().unwrap().changed.len()
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

    /// The dirty chunks with their revisions, sorted by key, without
    /// clearing anything: a save writes them and then `acknowledge`s.
    pub fn dirty_snapshot(&self) -> Vec<DirtyChunk> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<DirtyChunk> = g.dirty.iter().filter_map(|k| g.chunks.get(k).map(|e| DirtyChunk { key: *k, rev: e.rev, ids: e.ids.clone(), water: e.water.clone() })).collect();
        out.sort_by_key(|d| d.key);
        out
    }

    /// A save persisted these revisions: the chunks stop being dirty unless
    /// they changed again in the meantime.
    pub fn acknowledge(&self, saved: &[(ChunkKey, u64)]) {
        let mut g = self.inner.write().unwrap();
        for (k, rev) in saved {
            if g.chunks.get(k).is_some_and(|e| e.rev == *rev) {
                g.dirty.remove(k);
            }
        }
        if g.dirty.is_empty() {
            g.first_dirty = None;
        }
    }

    /// Every edited chunk's ids (for tests and hashing), sorted by key.
    pub fn snapshot(&self) -> Vec<(ChunkKey, Vec<BlockId>)> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<(ChunkKey, Vec<BlockId>)> = g.chunks.iter().map(|(k, e)| (*k, e.ids.clone())).collect();
        out.sort_by_key(|(k, _)| *k);
        out
    }

    /// Every edited chunk in full (ids, water, revision), sorted by key.
    pub fn snapshot_full(&self) -> Vec<DirtyChunk> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<DirtyChunk> = g.chunks.iter().map(|(k, e)| DirtyChunk { key: *k, rev: e.rev, ids: e.ids.clone(), water: e.water.clone() }).collect();
        out.sort_by_key(|d| d.key);
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
        assert_eq!(s.revision(key), Some(0));
        assert!(s.set(key, 0, 0, 0, 0, natural));
        assert_eq!(s.get(key, 0, 0, 0), Some(0));
        assert_eq!(s.get(key, 0, 0, 1), Some(1));
        assert_eq!(s.revision(key), Some(1));
        assert_eq!(s.dirty_count(), 1);
        assert!(!s.due_for_save(Instant::now()), "not yet quiet");
        assert!(s.due_for_save(Instant::now() + std::time::Duration::from_secs(3)));
        let d = s.dirty_snapshot();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].ids[0], 0);
        assert_eq!(d[0].rev, 1);
        assert_eq!(s.dirty_count(), 1, "a snapshot clears nothing");
        s.acknowledge(&[(key, 1)]);
        assert_eq!(s.dirty_count(), 0);
        s.push_invalidated([key, key, ChunkKey { lod: 1, ..key }]);
        assert_eq!(s.take_invalidated().len(), 2);
        assert!(s.take_invalidated().is_empty());
    }

    /// Review #2: a chunk edited while a save is in flight stays dirty, and
    /// a save that fails leaves everything dirty for the next attempt.
    #[test]
    fn acknowledging_a_stale_revision_keeps_the_chunk_dirty() {
        let s = EditStore::new();
        let key = ChunkKey::neisor(1, 0, 3, 4, 5);
        let natural = || vec![1u16; VOXELS];
        s.set(key, 0, 0, 0, 2, natural);
        let d = s.dirty_snapshot();
        assert_eq!((d[0].key, d[0].rev), (key, 1));
        // "Written" while another edit lands: the acknowledgement is stale.
        s.set(key, 0, 0, 1, 3, natural);
        s.acknowledge(&[(key, 1)]);
        assert_eq!(s.dirty_count(), 1, "rev 2 is unsaved");
        s.acknowledge(&[(key, 2)]);
        assert_eq!(s.dirty_count(), 0);
        // A failed save acknowledges nothing: the dirty set is intact.
        s.set(key, 0, 0, 2, 4, natural);
        let _ = s.dirty_snapshot();
        assert_eq!(s.dirty_count(), 1);
        assert!(s.due_for_save(Instant::now() + std::time::Duration::from_secs(20)), "still due");
    }

    /// Review #15: water levels are stored, not reconstructed.
    #[test]
    fn the_store_holds_water_levels_and_a_batch_commits_under_one_lock() {
        let s = EditStore::new();
        let key = ChunkKey::neisor(2, 0, 1, 1, 1);
        let water: BlockId = 8;
        let liquid = |id: BlockId| id == water;
        let natural = |_: ChunkKey| (vec![1u16; VOXELS], None);
        let w = |x: usize, id: BlockId, level: Option<u8>| Write { key, x, y: 0, z: 0, id, level };
        let changed = s.commit(&[w(0, water, None), w(1, water, Some(17)), w(2, 1, None), w(3, 0, None)], natural, liquid);
        assert_eq!(changed, vec![true, true, false, true]);
        assert_eq!(s.water_level(key, 0, 0, 0), Some(WATER_FULL), "a liquid block defaults to full");
        assert_eq!(s.water_level(key, 1, 0, 0), Some(17), "an explicit level is kept");
        assert_eq!(s.water_level(key, 2, 0, 0), Some(0));
        assert_eq!(s.revision(key), Some(3));
        assert_eq!(s.dirty_count(), 1);
        // Changing only the level is a change; draining every level drops the layer.
        assert_eq!(s.commit(&[w(1, water, Some(16))], natural, liquid), vec![true]);
        assert_eq!(s.commit(&[w(0, 0, None), w(1, 0, None)], natural, liquid), vec![true, true]);
        assert!(s.with_edited(key, |e| e.water.is_none()).unwrap());
        // Loading keeps the levels it is given, dropping an all-zero layer.
        let key2 = ChunkKey::neisor(2, 0, 1, 1, 2);
        let mut levels = vec![0u8; VOXELS];
        levels[5] = 9;
        s.insert_chunk(key2, vec![0u16; VOXELS], Some(levels), false);
        assert_eq!(s.water_level(key2, 0, 0, 5), Some(9));
        s.insert_chunk(ChunkKey::neisor(2, 0, 1, 1, 3), vec![0u16; VOXELS], Some(vec![0u8; VOXELS]), false);
        assert!(s.with_edited(ChunkKey::neisor(2, 0, 1, 1, 3), |e| e.water.is_none()).unwrap());
        assert_eq!(s.dirty_count(), 1, "loading is not dirt");
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
        s.insert_chunk(a, natural(), None, false); // re-inserting does not double-index
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

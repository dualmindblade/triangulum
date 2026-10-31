//! Edited chunks: the authoritative overlay the player and the simulation
//! write.
//!
//! An edited LOD-0 chunk is held in full (its ids, packed at 4, 8 or 16
//! bits per voxel against a palette of the ids it holds; its water levels,
//! one byte per voxel, while any voxel holds water; a revision) and is
//! never regenerated. Generation stays a pure function;
//! `World` applies this overlay after it. The store also tracks which chunks
//! are unsaved (dirty, by revision), which loaded chunks at any LOD must be
//! remeshed (invalidated) and which blocks changed (for the simulation's
//! neighbour notifications), so saving, streaming and the simulation stay
//! out of each other's way. All three are fed by one batch commit.
//!
//! Saving is acknowledged, not assumed (review #2): `save_snapshot` hands
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

/// Ancestors indexed per edited chunk: LODs 1..=INDEX_LODS (a chunk at LOD
/// 15 is 2^20 blocks wide, more than any supported face).
const INDEX_LODS: u8 = 15;

/// Water level of a block that is water (M2b's levels replace this).
pub const WATER_FULL: u8 = 255;

/// An edited chunk's block ids (review 2 #17: they were a dense `u16` per
/// voxel, 64 KiB a chunk): a palette of the ids the chunk holds and one
/// index per voxel, 4 bits while the palette has at most 16 ids, 8 bits up
/// to 256, the ids themselves (16 bits) past that. A chunk is a few kinds
/// of rock, soil and air, so 16 KiB. The palette grows as new ids are
/// written and never shrinks; the dense form (`to_dense`) is canonical,
/// the packing is not, so digests and saves use the dense form.
#[derive(Clone, Debug)]
pub struct PackedIds {
    palette: Vec<BlockId>,
    bits: u8,
    data: Vec<u8>,
}

impl PackedIds {
    pub fn from_dense(ids: &[BlockId]) -> Self {
        assert_eq!(ids.len(), VOXELS);
        // Runs of one id are long (z is the innermost index): look up only
        // when the id changes.
        let mut palette: Vec<BlockId> = Vec::new();
        let mut last = None;
        for &id in ids {
            if last == Some(id) {
                continue;
            }
            last = Some(id);
            if !palette.contains(&id) {
                palette.push(id);
                if palette.len() > 256 {
                    break;
                }
            }
        }
        let bits = match palette.len() {
            0..=16 => 4,
            17..=256 => 8,
            _ => 16,
        };
        let mut p = Self { palette: if bits == 16 { Vec::new() } else { palette }, bits, data: vec![0u8; VOXELS * bits as usize / 8] };
        let mut last: Option<(BlockId, u16)> = None;
        for (v, &id) in ids.iter().enumerate() {
            let slot = match last {
                Some((l, s)) if l == id => s,
                _ => {
                    let s = p.slot_of(id).expect("the palette holds every id");
                    last = Some((id, s));
                    s
                }
            };
            p.put(v, slot);
        }
        p
    }

    /// The palette slot of an id (the id itself at 16 bits), if it has one.
    #[inline]
    fn slot_of(&self, id: BlockId) -> Option<u16> {
        if self.bits == 16 { Some(id) } else { self.palette.iter().position(|&p| p == id).map(|i| i as u16) }
    }

    #[inline]
    fn put(&mut self, v: usize, slot: u16) {
        match self.bits {
            4 => {
                let (b, sh) = (v >> 1, (v & 1) * 4);
                self.data[b] = (self.data[b] & !(0xF << sh)) | ((slot as u8) << sh);
            }
            8 => self.data[v] = slot as u8,
            _ => self.data[2 * v..2 * v + 2].copy_from_slice(&slot.to_le_bytes()),
        }
    }

    #[inline]
    pub fn get(&self, v: usize) -> BlockId {
        match self.bits {
            4 => self.palette[((self.data[v >> 1] >> ((v & 1) * 4)) & 0xF) as usize],
            8 => self.palette[self.data[v] as usize],
            _ => u16::from_le_bytes([self.data[2 * v], self.data[2 * v + 1]]),
        }
    }

    pub fn set(&mut self, v: usize, id: BlockId) {
        let slot = match self.slot_of(id) {
            Some(s) => s,
            None => {
                let cap = 1usize << self.bits;
                if self.palette.len() == cap {
                    // Widen: 4 -> 8 bits, or 8 -> the ids themselves.
                    let dense = self.to_dense();
                    let mut wider = Self { palette: if self.bits == 4 { self.palette.clone() } else { Vec::new() }, bits: if self.bits == 4 { 8 } else { 16 }, data: Vec::new() };
                    wider.data = vec![0u8; VOXELS * wider.bits as usize / 8];
                    for (u, &old) in dense.iter().enumerate() {
                        let s = wider.slot_of(old).expect("the wider palette holds every id");
                        wider.put(u, s);
                    }
                    *self = wider;
                    return self.set(v, id);
                }
                self.palette.push(id);
                (self.palette.len() - 1) as u16
            }
        };
        self.put(v, slot);
    }

    pub fn to_dense(&self) -> Vec<BlockId> {
        (0..VOXELS).map(|v| self.get(v)).collect()
    }

    /// Bits per voxel (4, 8 or 16).
    pub fn bits(&self) -> u8 {
        self.bits
    }

    pub fn heap_bytes(&self) -> usize {
        self.data.capacity() + self.palette.capacity() * 2
    }
}

/// One edited chunk, in full.
pub struct Edited {
    pub ids: PackedIds,
    /// Water level per voxel; None when no voxel holds water.
    pub water: Option<Vec<u8>>,
    /// Voxels whose level is not 0 (review 3 #12: emptying a cell used to
    /// rescan all 32,768 levels to decide whether the layer could go).
    wet: u32,
    /// Bumps on every change to this chunk.
    pub rev: u64,
    /// Whether the content equals the generator's chunk, as last worked
    /// out (`rev << 1 | equal`, `u64::MAX` unknown), and against which
    /// world's generator (`World::id`; shot markers make another): the state
    /// hash leaves such chunks out, and finding out regenerates the natural
    /// chunk, so it is kept until the chunk changes (review 2 #17).
    natural_eq: AtomicU64,
    natural_of: AtomicU64,
}

impl Edited {
    fn new(ids: &[BlockId], water: Option<Vec<u8>>) -> Self {
        let wet = water.as_ref().map_or(0, |l| l.iter().filter(|&&x| x != 0).count() as u32);
        let water = water.filter(|_| wet > 0);
        Self { ids: PackedIds::from_dense(ids), water, wet, rev: 0, natural_eq: AtomicU64::new(u64::MAX), natural_of: AtomicU64::new(0) }
    }

    /// Voxels holding any water.
    pub fn wet_cells(&self) -> u32 {
        self.wet
    }

    /// Block id of voxel `v` (`chunk::index`).
    #[inline]
    pub fn id(&self, v: usize) -> BlockId {
        self.ids.get(v)
    }

    /// Water level of voxel `v`.
    #[inline]
    pub fn level(&self, v: usize) -> u8 {
        self.water.as_ref().map_or(0, |l| l[v])
    }

    /// Heap bytes this chunk holds.
    pub fn heap_bytes(&self) -> usize {
        self.ids.heap_bytes() + self.water.as_ref().map_or(0, |l| l.capacity())
    }

    /// Whether the content equals generator `of`'s chunk, if that was
    /// worked out since the last change (`set_natural_eq`).
    pub fn natural_eq(&self, of: u64) -> Option<bool> {
        let c = self.natural_eq.load(Ordering::Relaxed);
        (c != u64::MAX && c >> 1 == self.rev && self.natural_of.load(Ordering::Relaxed) == of).then_some(c & 1 == 1)
    }

    pub fn set_natural_eq(&self, of: u64, equal: bool) {
        self.natural_of.store(of, Ordering::Relaxed);
        self.natural_eq.store(self.rev << 1 | equal as u64, Ordering::Relaxed);
    }
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

/// An owned save copy in the store's packed representation. Expansion and
/// disk compression belong to the writer, not the frame requesting a save.
pub struct SaveChunk {
    pub key: ChunkKey,
    pub rev: u64,
    pub ids: PackedIds,
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
        if e.ids.get(v) == w.id && had_level == level {
            return false;
        }
        e.ids.set(v, w.id);
        if level != had_level {
            if level != 0 && e.water.is_none() {
                e.water = Some(vec![0u8; VOXELS]);
            }
            if let Some(l) = e.water.as_mut() {
                l[v] = level;
            }
            match (had_level, level) {
                (0, _) => e.wet += 1,
                (_, 0) => e.wet -= 1,
                _ => {}
            }
            if e.wet == 0 {
                e.water = None;
            }
        }
        e.rev += 1;
        true
    }
}

#[derive(Default)]
pub struct EditStore {
    inner: RwLock<Inner>,
    /// `--powder levels` (B7): cells may hold partial powder, so the mesher
    /// and the player read the slope (`world/slope.rs`). Off, everything that
    /// asks costs one atomic load.
    slopes: AtomicBool,
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

    /// May cells hold partial powder (`--powder levels`)?
    #[inline]
    pub fn slopes_on(&self) -> bool {
        self.slopes.load(Ordering::Relaxed)
    }

    pub fn set_slopes(&self, on: bool) {
        self.slopes.store(on, Ordering::Relaxed);
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
        self.inner.read().unwrap().chunks.get(&key).map(|e| e.ids.get(index(x, y, z)))
    }

    /// Water level of an edited chunk's voxel; None when the chunk is not edited.
    pub fn water_level(&self, key: ChunkKey, x: usize, y: usize, z: usize) -> Option<u8> {
        self.inner.read().unwrap().chunks.get(&key).map(|e| e.water.as_ref().map_or(0, |l| l[index(x, y, z)]))
    }

    /// Block and water level of an edited chunk's voxel under one lock
    /// (the water rule reads seven cells per update); None when the chunk
    /// is not edited.
    pub fn get_with_level(&self, key: ChunkKey, x: usize, y: usize, z: usize) -> Option<(BlockId, u8)> {
        let g = self.inner.read().unwrap();
        let e = g.chunks.get(&key)?;
        let v = index(x, y, z);
        Some((e.ids.get(v), e.water.as_ref().map_or(0, |l| l[v])))
    }

    /// Does an edited chunk hold a water level other than empty or full?
    /// None when the chunk is not edited.
    pub fn has_partial_water(&self, key: ChunkKey) -> Option<bool> {
        self.inner.read().unwrap().chunks.get(&key).map(|e| e.water.as_ref().is_some_and(|w| w.iter().any(|&l| l != 0 && l != WATER_FULL)))
    }

    /// Revision of an edited chunk; None when the chunk is not edited.
    pub fn revision(&self, key: ChunkKey) -> Option<u64> {
        self.inner.read().unwrap().chunks.get(&key).map(|e| e.rev)
    }

    /// Run `f` over an edited chunk's dense ids (unpacked, under the read lock).
    pub fn with_chunk<R>(&self, key: ChunkKey, f: impl FnOnce(&[BlockId]) -> R) -> Option<R> {
        self.inner.read().unwrap().chunks.get(&key).map(|e| f(&e.ids.to_dense()))
    }

    /// Run `f` over an edited chunk's ids and water levels (under the read lock).
    pub fn with_edited<R>(&self, key: ChunkKey, f: impl FnOnce(&Edited) -> R) -> Option<R> {
        self.inner.read().unwrap().chunks.get(&key).map(f)
    }

    /// Visit every edited chunk in key order, without copying any of them
    /// (review 2 #17: the state hash cloned them all, 96 KiB each). `f` runs
    /// under the read lock, taken per chunk: it must not lock the store itself.
    pub fn for_each_sorted(&self, mut f: impl FnMut(ChunkKey, &Edited)) {
        for key in self.keys() {
            if let Some(e) = self.inner.read().unwrap().chunks.get(&key) {
                f(key, e);
            }
        }
    }

    /// Heap bytes the edited chunks hold (ids and water levels; not the
    /// maps around them).
    pub fn heap_bytes(&self) -> usize {
        self.inner.read().unwrap().chunks.values().map(|e| e.heap_bytes()).sum()
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
        // Chunks this commit materialised from the generator.
        let mut fresh: Vec<ChunkKey> = Vec::new();
        for w in writes {
            debug_assert_eq!(w.key.lod, 0);
            if !g.chunks.contains_key(&w.key) {
                let (ids, water) = natural(w.key);
                assert_eq!(ids.len(), VOXELS);
                if let Some(l) = &water {
                    assert_eq!(l.len(), VOXELS);
                }
                g.chunks.insert(w.key, Edited::new(&ids, water));
                fresh.push(w.key);
            }
            let changed = g.write(w, liquid(w.id));
            if changed {
                g.mark_dirty(w.key);
            }
            out.push(changed);
        }
        // A chunk no write changed is the generator's chunk: keep none of
        // it (review 3 #6: it would be in the state hash but never saved).
        // The others go into the ancestor index now (nothing reads it while
        // the write lock is held), so a no-op never touches the index.
        for key in fresh {
            if g.chunks.get(&key).is_some_and(|e| e.rev == 0) {
                g.chunks.remove(&key);
            } else {
                g.index(key);
            }
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
        let mut edited = Edited::new(&ids, water);
        if let Some(old) = g.chunks.get(&key) { edited.rev = old.rev + 1; }
        if g.chunks.insert(key, edited).is_none() {
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

    pub fn changed_snapshot(&self) -> Vec<BlockPos> {
        self.inner.read().unwrap().changed.clone()
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
        let mut out: Vec<DirtyChunk> = g.dirty.iter().filter_map(|k| g.chunks.get(k).map(|e| DirtyChunk { key: *k, rev: e.rev, ids: e.ids.to_dense(), water: e.water.clone() })).collect();
        out.sort_by_key(|d| d.key);
        out
    }

    /// Check the entire copy's bound under the same lock before allocating.
    /// Never truncate: chunks and simulation must describe one whole moment.
    pub fn save_snapshot(&self, limit: usize) -> Result<(Vec<SaveChunk>, usize), String> {
        let g = self.inner.read().unwrap();
        let bytes = g.dirty.len() * (std::mem::size_of::<SaveChunk>() + std::mem::size_of::<(ChunkKey, u64)>())
            + g.dirty.iter().map(|k| g.chunks[k].heap_bytes()).sum::<usize>();
        if bytes > limit { return Err("save snapshot exceeds its memory budget".into()); }
        let mut out = Vec::with_capacity(g.dirty.len());
        for k in &g.dirty {
            let e = &g.chunks[k];
            out.push(SaveChunk { key: *k, rev: e.rev, ids: e.ids.clone(), water: e.water.clone() });
        }
        out.sort_unstable_by_key(|d| d.key);
        Ok((out, bytes))
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
        let mut out: Vec<(ChunkKey, Vec<BlockId>)> = g.chunks.iter().map(|(k, e)| (*k, e.ids.to_dense())).collect();
        out.sort_by_key(|(k, _)| *k);
        out
    }

    /// Every edited chunk in full (ids, water, revision), sorted by key.
    pub fn snapshot_full(&self) -> Vec<DirtyChunk> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<DirtyChunk> = g.chunks.iter().map(|(k, e)| DirtyChunk { key: *k, rev: e.rev, ids: e.ids.to_dense(), water: e.water.clone() }).collect();
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
        // Review 3 #6: a write that changes nothing leaves no chunk behind
        // (it used to become authoritative on first touch, unsaved).
        assert_eq!(s.len(), 0, "an unchanged chunk is not edited");
        assert_eq!(s.dirty_count(), 0);
        assert_eq!(s.revision(key), None);
        assert!(!s.any_under(key.parent().unwrap()), "and not indexed");
        assert!(s.set(key, 0, 0, 0, 0, natural));
        assert_eq!(s.len(), 1);
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

    /// Review 3 #12: emptying a cell must not rescan the chunk's 32,768
    /// levels to decide whether its water layer can go. Draining a full
    /// chunk one cell at a time, in index order (so what remains is always
    /// at the end of the array), used to cost cells x remaining comparisons
    /// under the write lock.
    #[test]
    fn emptying_water_cell_by_cell_does_not_rescan_the_chunk() {
        let s = EditStore::new();
        let key = ChunkKey::neisor(2, 0, 7, 7, 7);
        let water: BlockId = 8;
        let liquid = |id: BlockId| id == water;
        let natural = |_: ChunkKey| (vec![0u16; VOXELS], None);
        let all: Vec<Write> = (0..VOXELS).map(|v| Write { key, x: v / 1024, y: (v / 32) % 32, z: v % 32, id: water, level: Some(200) }).collect();
        assert!(s.commit(&all, natural, liquid).iter().all(|&c| c));
        let t = Instant::now();
        for w in &all[..VOXELS - 1] {
            assert_eq!(s.commit(&[Write { id: 0, level: None, ..*w }], natural, liquid), vec![true]);
        }
        let ms = t.elapsed().as_secs_f64() * 1e3;
        assert!(s.with_edited(key, |e| e.water.is_some()).unwrap(), "one cell still holds water");
        let last = all[VOXELS - 1];
        s.commit(&[Write { id: 0, level: None, ..last }], natural, liquid);
        assert!(s.with_edited(key, |e| e.water.is_none()).unwrap(), "the last one empties the layer");
        println!("drained {} cells one at a time in {ms:.1} ms", VOXELS);
        assert!(ms < 60.0, "draining a chunk cell by cell took {ms:.1} ms: the emptiness check rescans the levels");
    }

    /// Review 2 #17: packed ids read back what was written, through
    /// palette growth and both widenings, against a dense model.
    #[test]
    fn packed_ids_hold_what_was_written_through_every_width() {
        let mut model: Vec<BlockId> = (0..VOXELS).map(|v| [0u16, 1, 2][v % 3]).collect();
        let mut p = PackedIds::from_dense(&model);
        assert_eq!((p.bits(), p.heap_bytes() >= VOXELS / 2), (4, true));
        assert!(p.heap_bytes() < VOXELS / 2 + 64, "16 KiB, not 64: {}", p.heap_bytes());
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for (round, ids) in [(0, 10u16), (1, 200), (2, 600)] {
            for _ in 0..20_000 {
                let v = (next() % VOXELS as u64) as usize;
                let id = (next() % ids as u64) as u16 * 7;
                p.set(v, id);
                model[v] = id;
            }
            assert_eq!(p.to_dense(), model, "round {round}");
            for v in (0..VOXELS).step_by(97) {
                assert_eq!(p.get(v), model[v]);
            }
        }
        assert_eq!(p.bits(), 16, "600 ids need the ids themselves");
        let q = PackedIds::from_dense(&model);
        assert_eq!(q.to_dense(), model);
        assert_eq!(PackedIds::from_dense(&vec![5u16; VOXELS]).get(1234), 5);
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

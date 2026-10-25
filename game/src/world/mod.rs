//! The world: block definitions, chunk storage, edits and saves, and
//! `World`, the generator plus the edit overlay that everything (streaming,
//! physics, picking) reads blocks from.

pub mod blocks;
pub mod chunk;
pub mod edits;
pub mod save;

pub use blocks::{BlockDef, BlockId, Registry};
pub use chunk::Chunk;
pub use edits::EditStore;

use crate::grid::{BlockPos, CHUNK, ChunkKey, FastMap, Planet};
use crate::worldgen::{Generated, Generator, PAD_VOXELS, pidx};
use chunk::{VOXELS, index};
use glam::DVec3;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Loaded chunks at every LOD, keyed by chunk (the streamer's view).
#[derive(Default)]
pub struct ChunkStore {
    chunks: FastMap<ChunkKey, Arc<Chunk>>,
}

impl ChunkStore {
    pub fn insert(&mut self, key: ChunkKey, chunk: Arc<Chunk>) {
        self.chunks.insert(key, chunk);
    }
    pub fn remove(&mut self, key: &ChunkKey) -> Option<Arc<Chunk>> {
        self.chunks.remove(key)
    }
    pub fn get(&self, key: &ChunkKey) -> Option<&Arc<Chunk>> {
        self.chunks.get(key)
    }
    pub fn len(&self) -> usize {
        self.chunks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
    /// Bytes held by palettes, indices and water layers.
    pub fn bytes(&self) -> usize {
        self.chunks.values().map(|c| c.bytes()).sum()
    }
}

/// Block lookup in lattice coordinates of one face, indices allowed a little
/// outside the face (they resolve onto the neighbouring face) and outside
/// the world (bedrock below, air above). Physics and picking run on this.
pub trait Voxels {
    fn block(&self, face: u8, i: i64, j: i64, k: i64) -> BlockId;
    fn is_solid(&self, id: BlockId) -> bool;
    fn is_liquid(&self, id: BlockId) -> bool;
    /// Water level (0..255) of a cell; a liquid cell is full unless the
    /// implementation knows better (M2b: the stored level).
    fn water_level(&self, _face: u8, _i: i64, _j: i64, _k: i64) -> u8 {
        edits::WATER_FULL
    }
    /// Is the point at fraction `frac` (0..1) up a cell under water? A cell
    /// with water above it counts as full (a falling column), otherwise the
    /// surface sits at level / 255.
    fn submerged(&self, face: u8, i: i64, j: i64, k: i64, frac: f64) -> bool {
        if !self.is_liquid(self.block(face, i, j, k)) {
            return false;
        }
        if self.is_liquid(self.block(face, i, j, k + 1)) {
            return true;
        }
        frac * f64::from(edits::WATER_FULL) < f64::from(self.water_level(face, i, j, k))
    }
}

/// Generator + edits. Chunks the generator produces are cached at LOD 0 for
/// physics and for the coarse-LOD edit rule; edited chunks come from the
/// store. `generate` is what the streamer's workers call.
pub struct World {
    pub planet: Planet,
    pub generator: Arc<Generator>,
    pub registry: Arc<Registry>,
    pub edits: Arc<EditStore>,
    natural: Mutex<NaturalCache>,
}

/// Natural chunks kept for physics, evicted oldest-first one at a time.
const NATURAL_CACHE_MAX: usize = 1024;

#[derive(Default)]
struct NaturalCache {
    map: FastMap<ChunkKey, Arc<Chunk>>,
    order: VecDeque<ChunkKey>,
}

impl World {
    pub fn new(generator: Arc<Generator>, registry: Arc<Registry>, edits: Arc<EditStore>) -> Self {
        Self { planet: generator.planet, generator, registry, edits, natural: Mutex::new(NaturalCache::default()) }
    }

    pub fn water_id(&self) -> BlockId {
        self.generator.blocks.water
    }

    /// The generator's LOD-0 chunk (pure, cached).
    pub fn natural(&self, key: ChunkKey) -> Arc<Chunk> {
        debug_assert_eq!(key.lod, 0);
        if let Some(c) = self.natural.lock().unwrap().map.get(&key) {
            return c.clone();
        }
        let c = Arc::new(self.generator.generate(key).chunk);
        let mut g = self.natural.lock().unwrap();
        if g.map.len() >= NATURAL_CACHE_MAX {
            if let Some(old) = g.order.pop_front() {
                g.map.remove(&old);
            }
        }
        if let Some(existing) = g.map.get(&key) {
            return existing.clone();
        }
        g.order.push_back(key);
        g.map.insert(key, c.clone());
        c
    }

    pub fn natural_cached(&self) -> usize {
        self.natural.lock().unwrap().map.len()
    }

    #[inline]
    pub fn chunk_of(bp: BlockPos) -> (ChunkKey, usize, usize, usize) {
        let c = CHUNK as u32;
        (ChunkKey::neisor(bp.face, 0, bp.i / c, bp.j / c, bp.k / c), (bp.i % c) as usize, (bp.j % c) as usize, (bp.k % c) as usize)
    }

    /// Every edited LOD-0 chunk that can touch `key`'s padded voxel grid:
    /// those under `key` and its 26 neighbours at `key`'s LOD on the same
    /// face, plus, when `key` sits on a face edge, those under the
    /// neighbouring faces' chunks along the padding strip past that edge.
    pub fn edited_near(&self, key: ChunkKey) -> Vec<ChunkKey> {
        let planet = &self.planet;
        let per_edge = planet.chunks_per_edge(key.lod) as i64;
        let per_col = planet.chunks_per_column(key.lod) as i64;
        let mut out = Vec::new();
        for dz in -1..=1i64 {
            let cz = key.cz as i64 + dz;
            if cz < 0 || cz >= per_col {
                continue;
            }
            for dy in -1..=1i64 {
                for dx in -1..=1i64 {
                    let (cx, cy) = (key.cx as i64 + dx, key.cy as i64 + dy);
                    if cx < 0 || cx >= per_edge || cy < 0 || cy >= per_edge {
                        continue;
                    }
                    out.extend(self.edits.edited_under(ChunkKey { cx: cx as u32, cy: cy as u32, cz: cz as u32, ..key }));
                }
            }
        }
        let at_edge = key.cx == 0 || key.cy == 0 || key.cx as i64 + 1 == per_edge || key.cy as i64 + 1 == per_edge;
        if at_edge {
            let e = key.extent_blocks() as f64;
            let v = key.voxel_blocks() as f64;
            let (i0, j0, _) = key.origin_blocks();
            let (i0, j0) = (i0 as f64, j0 as f64);
            let mut neighbours: Vec<ChunkKey> = Vec::new();
            // Centres of the padding voxels past each side, corners included.
            for t in -1..=CHUNK as i32 {
                let along = (t as f64 + 0.5) * v;
                for (i, j) in [(i0 - 0.5 * v, j0 + along), (i0 + e + 0.5 * v, j0 + along), (i0 + along, j0 - 0.5 * v), (i0 + along, j0 + e + 0.5 * v)] {
                    let (f, wi, wj) = planet.wrap(key.face, i, j);
                    if f == key.face {
                        continue;
                    }
                    let cx = ((wi / e).floor() as i64).clamp(0, per_edge - 1) as u32;
                    let cy = ((wj / e).floor() as i64).clamp(0, per_edge - 1) as u32;
                    for dz in -1..=1i64 {
                        let cz = key.cz as i64 + dz;
                        if cz < 0 || cz >= per_col {
                            continue;
                        }
                        let n = ChunkKey { face: f, cx, cy, cz: cz as u32, ..key };
                        if !neighbours.contains(&n) {
                            neighbours.push(n);
                        }
                    }
                }
            }
            for n in neighbours {
                out.extend(self.edits.edited_under(n));
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Block at a position inside the world (edits first).
    pub fn block_at(&self, bp: BlockPos) -> BlockId {
        let (key, x, y, z) = Self::chunk_of(bp);
        if let Some(id) = self.edits.get(key, x, y, z) {
            return id;
        }
        self.natural(key).get(x, y, z)
    }

    /// Water level (0..255) at a position (edits first; the generator's
    /// water is full). M2b's rule reads and writes this.
    pub fn water_level_at(&self, bp: BlockPos) -> u8 {
        let (key, x, y, z) = Self::chunk_of(bp);
        if let Some(l) = self.edits.water_level(key, x, y, z) {
            return l;
        }
        self.natural(key).water_level(x, y, z)
    }

    /// Block and water level at a position in one lookup (edits first).
    pub fn block_and_level_at(&self, bp: BlockPos) -> (BlockId, u8) {
        let (key, x, y, z) = Self::chunk_of(bp);
        if let Some(c) = self.edits.get_with_level(key, x, y, z) {
            return c;
        }
        let n = self.natural(key);
        (n.get(x, y, z), n.water_level(x, y, z))
    }

    /// The generator's block and water level at a position: what an
    /// unedited cell holds, and what the water rule treats a cell outside
    /// the active region as (M2b, `Sim::boundary_cell`). The generator's
    /// per-cell levels (M2-hydro's rivers) arrive through here.
    pub fn natural_cell(&self, bp: BlockPos) -> (BlockId, u8) {
        let (key, x, y, z) = Self::chunk_of(bp);
        let n = self.natural(key);
        (n.get(x, y, z), n.water_level(x, y, z))
    }

    /// Is a world position under water? Level-aware (`Voxels::submerged`).
    pub fn is_submerged(&self, pos: DVec3) -> bool {
        let (f, i, j, k) = self.planet.lattice_of_pos(pos);
        if k < 0.0 || k >= self.planet.height as f64 {
            return false;
        }
        <Self as Voxels>::submerged(self, f, i.floor() as i64, j.floor() as i64, k.floor() as i64, k - k.floor())
    }

    /// A solid block is about to take `bp`: the water there, if any, is
    /// pushed up its own column into the first cells above that can hold it
    /// (a full cell passes it on; the surface cell and the air above it take
    /// it), so a block dropped into a pool raises the pool instead of
    /// destroying water (M2b displacement). Returns the writes to commit
    /// together with the placement and the units that found no room (a
    /// solid ceiling): those are lost, and the caller counts them.
    pub fn displace_writes(&self, bp: BlockPos) -> (Vec<(BlockPos, BlockId, Option<u8>)>, u32) {
        let (id, level) = self.block_and_level_at(bp);
        if !self.registry.is_liquid(id) || level == 0 {
            return (Vec::new(), 0);
        }
        let water = self.water_id();
        let mut remaining = level as u32;
        let mut out = Vec::new();
        for k in bp.k + 1..self.planet.height {
            let up = BlockPos { k, ..bp };
            let (uid, ul) = self.block_and_level_at(up);
            if !self.registry.is_replaceable(uid) {
                break;
            }
            let cap = (edits::WATER_FULL - ul) as u32;
            if cap == 0 {
                continue;
            }
            let d = remaining.min(cap);
            out.push((up, water, Some((ul as u32 + d) as u8)));
            remaining -= d;
            if remaining == 0 {
                break;
            }
        }
        (out, remaining)
    }

    /// The generator's chunk as dense ids and water levels: what an edited
    /// chunk starts from.
    fn natural_dense(&self, key: ChunkKey) -> (Vec<BlockId>, Option<Vec<u8>>) {
        let c = self.natural(key);
        (c.to_dense(), c.water_dense())
    }

    /// Edited block at a position, None when its chunk is not edited.
    pub fn edit_at(&self, bp: BlockPos) -> Option<BlockId> {
        let (key, x, y, z) = Self::chunk_of(bp);
        self.edits.get(key, x, y, z)
    }

    /// Column (face, i, j) of lattice indices that may lie past the face
    /// edge: those snap onto the neighbouring face's own columns.
    pub fn resolve(&self, face: u8, ci: i64, cj: i64) -> (u8, u32, u32) {
        self.planet.wrap_cell(face, ci, cj)
    }

    /// `resolve` plus the layer: None outside the world's height.
    pub fn resolve_block(&self, face: u8, ci: i64, cj: i64, ck: i64) -> Option<BlockPos> {
        if ck < 0 || ck >= self.planet.height as i64 {
            return None;
        }
        let (f, i, j) = self.resolve(face, ci, cj);
        Some(BlockPos { face: f, i, j, k: ck as u32 })
    }

    pub fn block_lattice(&self, face: u8, ci: i64, cj: i64, ck: i64) -> BlockId {
        if ck < 0 {
            return self.generator.blocks.bedrock;
        }
        match self.resolve_block(face, ci, cj, ck) {
            Some(bp) => self.block_at(bp),
            None => self.generator.blocks.air,
        }
    }

    /// Set one block: `commit` of one write. Returns whether it changed.
    pub fn set_block(&self, bp: BlockPos, id: BlockId) -> bool {
        self.commit(&[(bp, id)]) == 1
    }

    /// Set a liquid block with a water level (M2b). Returns whether it changed.
    pub fn set_water(&self, bp: BlockPos, id: BlockId, level: u8) -> bool {
        self.commit_levels(&[(bp, id, Some(level))]) == 1
    }

    /// The one write path for the player and the simulation alike: apply a
    /// batch of block writes under one lock, then queue the remeshes of
    /// every chunk (at every LOD) they touch and log the changed blocks for
    /// the simulation's neighbour notifications. Writes outside the world
    /// are ignored. Returns how many writes changed something.
    pub fn commit(&self, writes: &[(BlockPos, BlockId)]) -> usize {
        let with_levels: Vec<(BlockPos, BlockId, Option<u8>)> = writes.iter().map(|&(bp, id)| (bp, id, None)).collect();
        self.commit_levels(&with_levels)
    }

    pub fn commit_levels(&self, writes: &[(BlockPos, BlockId, Option<u8>)]) -> usize {
        let planet = &self.planet;
        let valid: Vec<(BlockPos, BlockId, Option<u8>)> = writes.iter().copied().filter(|(bp, _, _)| bp.k < planet.height && bp.i < planet.n && bp.j < planet.n && bp.face < 6).collect();
        if valid.is_empty() {
            return 0;
        }
        let ws: Vec<edits::Write> = valid
            .iter()
            .map(|&(bp, id, level)| {
                let (key, x, y, z) = Self::chunk_of(bp);
                edits::Write { key, x, y, z, id, level }
            })
            .collect();
        let changed = self.edits.commit(&ws, |key| self.natural_dense(key), |id| self.registry.is_liquid(id));
        // The chunks to remesh, deduplicated as they come: a flood commits
        // thousands of cells a tick that share a few dozen chunks.
        let mut invalidated: crate::grid::FastSet<ChunkKey> = crate::grid::FastSet::default();
        let mut scratch = Vec::with_capacity(32);
        let mut log = Vec::new();
        for ((bp, _, _), c) in valid.iter().zip(&changed) {
            if *c {
                self.affected_chunks_into(*bp, &mut scratch);
                invalidated.extend(scratch.drain(..));
                log.push(*bp);
            }
        }
        if !log.is_empty() {
            let mut invalidated: Vec<ChunkKey> = invalidated.into_iter().collect();
            invalidated.sort();
            self.edits.push_invalidated(invalidated);
            self.edits.push_changed(log.iter().copied());
        }
        log.len()
    }

    /// The six face neighbours of a block that exist: across chunk borders
    /// and, through `Planet::wrap_cell`, across cube-face edges. A step of
    /// one column past a face's side is past exactly one side, so the
    /// corner policy (`Planet::wrap`) never has to decide here; above the
    /// world top and below layer 0 there is no neighbour.
    pub fn neighbours(&self, bp: BlockPos) -> Vec<BlockPos> {
        let (i, j, k) = (bp.i as i64, bp.j as i64, bp.k as i64);
        [(-1, 0, 0), (1, 0, 0), (0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1)].into_iter().filter_map(|(di, dj, dk)| self.resolve_block(bp.face, i + di, j + dj, k + dk)).collect()
    }

    /// Every chunk, at every LOD, whose padded voxel grid contains `bp`:
    /// the chunk column up the LOD tree, plus neighbours when the block
    /// sits in a border voxel (across face edges too). The step to a
    /// neighbour is one voxel of that LOD, not one block (review #11: at LOD
    /// 1, block 65 is in voxel 0 of chunk 1 and in the padding of chunk 0;
    /// stepping one block to 64 stayed in chunk 1).
    pub fn affected_chunks(&self, bp: BlockPos) -> Vec<ChunkKey> {
        let mut out = Vec::new();
        self.affected_chunks_into(bp, &mut out);
        out.sort();
        out.dedup();
        out
    }

    /// `affected_chunks` appended to `out`, unsorted and possibly with
    /// repeats, without allocating (a flood calls this thousands of times
    /// a tick).
    pub fn affected_chunks_into(&self, bp: BlockPos, out: &mut Vec<ChunkKey>) {
        let (i, j, k) = (bp.i as i64, bp.j as i64, bp.k as i64);
        for lod in 0..=self.planet.max_lod {
            let e = (CHUNK as i64) << lod;
            let v = 1i64 << lod;
            let per_edge = self.planet.chunks_per_edge(lod);
            let per_col = self.planet.chunks_per_column(lod);
            // The voxel offsets to the padding neighbours: none, -v, +v.
            let offsets = |c: i64| -> ([i64; 3], usize) {
                let local = (c % e) >> lod; // voxel index within the chunk
                let mut o = [0i64; 3];
                let mut n = 1;
                if local == 0 {
                    o[n] = -v;
                    n += 1;
                }
                if local == (CHUNK as i64) - 1 {
                    o[n] = v;
                    n += 1;
                }
                (o, n)
            };
            let (ox, nx) = offsets(i);
            let (oy, ny) = offsets(j);
            let (oz, nz) = offsets(k);
            for &dz in &oz[..nz] {
                let ck = k + dz;
                if ck < 0 || ck >= self.planet.height as i64 {
                    continue;
                }
                for &dx in &ox[..nx] {
                    for &dy in &oy[..ny] {
                        // A block one voxel past the face edge resolves onto the neighbouring face.
                        let (f, ri, rj) = self.resolve(bp.face, i + dx, j + dy);
                        let key = ChunkKey::neisor(f, lod, ri / e as u32, rj / e as u32, (ck / e) as u32);
                        if key.cx < per_edge && key.cy < per_edge && key.cz < per_col {
                            out.push(key);
                        }
                    }
                }
            }
        }
    }

    /// Generate one chunk at its LOD: the pure generator, then the edits.
    /// An edited LOD-0 chunk's compressed form carries its stored water
    /// levels; a coarse chunk's water is full wherever its voxel is water.
    pub fn generate(&self, key: ChunkKey) -> Generated {
        let mut g = self.generator.generate(key);
        if self.edits.is_empty() {
            return g;
        }
        let touched = if key.lod == 0 { self.overlay_lod0(key, &mut g.ids) } else { self.overlay_coarse(key, &mut g.ids) };
        if touched {
            let mut dense = vec![0 as BlockId; VOXELS];
            for x in 0..CHUNK {
                for y in 0..CHUNK {
                    for z in 0..CHUNK {
                        dense[index(x, y, z)] = g.ids[pidx(x as i32, y as i32, z as i32)];
                    }
                }
            }
            let levels = if key.lod == 0 { self.edits.with_edited(key, |e| e.water.clone()) } else { None };
            g.chunk = match levels {
                Some(water) => Chunk::from_dense(&dense, water.as_deref()),
                None => save::chunk_of_dense(&dense, self.water_id()),
            };
        }
        g
    }

    /// The water levels of a generated LOD-0 chunk's padded grid (34^3,
    /// index by `pidx`), for the mesher's sloped surfaces (M2b): None when
    /// every water cell in reach is full (the sea), so the flat path costs
    /// nothing extra. The generator's own padded levels (`Generated::levels`,
    /// which it computes for the border columns too: lakes and rivers across
    /// chunk borders) are the base; an edited chunk replaces the interior,
    /// and edited neighbours the border cells they hold.
    pub fn padded_levels(&self, key: ChunkKey, g: &Generated) -> Option<Vec<u8>> {
        if key.lod != 0 {
            return None;
        }
        let edited_interior: Option<Option<Vec<u8>>> = self.edits.with_edited(key, |e| e.water.clone());
        let near: Vec<ChunkKey> = self.edited_near(key).into_iter().filter(|e| *e != key).collect();
        let near_partial = near.iter().any(|n| self.edits.has_partial_water(*n).unwrap_or(false));
        let interior_partial = match &edited_interior {
            Some(w) => w.as_ref().is_some_and(|w| w.iter().any(|&l| l != 0 && l != edits::WATER_FULL)),
            None => g.levels.is_some(),
        };
        if g.levels.is_none() && !interior_partial && !near_partial {
            return None;
        }
        let c = CHUNK as i32;
        // The generator's padded levels, or full wherever its padded grid is water.
        let mut levels: Vec<u8> = match &g.levels {
            Some(l) => l.clone(),
            None => g.ids.iter().map(|&id| if self.registry.is_liquid(id) { edits::WATER_FULL } else { 0 }).collect(),
        };
        if let Some(w) = &edited_interior {
            for x in 0..CHUNK {
                for y in 0..CHUNK {
                    for z in 0..CHUNK {
                        levels[pidx(x as i32, y as i32, z as i32)] = w.as_ref().map_or(0, |w| w[index(x, y, z)]);
                    }
                }
            }
        }
        if !near.is_empty() {
            let (i0, j0, k0) = key.origin_blocks();
            let (i0, j0, k0) = (i0 as i64, j0 as i64, k0 as i64);
            for x in -1..=c {
                for y in -1..=c {
                    for z in -1..=c {
                        let border = x < 0 || y < 0 || z < 0 || x >= c || y >= c || z >= c;
                        if !border {
                            continue;
                        }
                        let Some(bp) = self.resolve_block(key.face, i0 + x as i64, j0 + y as i64, k0 + z as i64) else { continue };
                        let (ck, cx, cy, cz) = Self::chunk_of(bp);
                        if let Some(l) = self.edits.water_level(ck, cx, cy, cz) {
                            levels[pidx(x, y, z)] = l;
                        }
                    }
                }
            }
        }
        if !levels.iter().any(|&l| l != 0 && l != edits::WATER_FULL) {
            return None;
        }
        Some(levels)
    }

    /// LOD 0: an edited chunk replaces the interior wholesale; edited
    /// neighbours fill the one-voxel border so faces cull and shade against
    /// the real neighbours.
    fn overlay_lod0(&self, key: ChunkKey, ids: &mut [BlockId]) -> bool {
        debug_assert_eq!(ids.len(), PAD_VOXELS);
        let mut touched = false;
        if self
            .edits
            .with_chunk(key, |dense| {
                for x in 0..CHUNK {
                    for y in 0..CHUNK {
                        for z in 0..CHUNK {
                            ids[pidx(x as i32, y as i32, z as i32)] = dense[index(x, y, z)];
                        }
                    }
                }
            })
            .is_some()
        {
            touched = true;
        }
        let (i0, j0, k0) = key.origin_blocks();
        let (i0, j0, k0) = (i0 as i64, j0 as i64, k0 as i64);
        let near = self.edited_near(key).into_iter().any(|e| e != key);
        if !near {
            return touched;
        }
        let c = CHUNK as i32;
        for x in -1..=c {
            for y in -1..=c {
                for z in -1..=c {
                    let border = x < 0 || y < 0 || z < 0 || x >= c || y >= c || z >= c;
                    if !border {
                        continue;
                    }
                    let Some(bp) = self.resolve_block(key.face, i0 + x as i64, j0 + y as i64, k0 + z as i64) else { continue };
                    if let Some(id) = self.edit_at(bp) {
                        ids[pidx(x, y, z)] = id;
                        touched = true;
                    }
                }
            }
        }
        touched
    }

    /// Coarse LODs: a voxel covering edited blocks shows the topmost solid
    /// block in it when that block was placed by an edit (so a one-block
    /// bridge survives every downsample), becomes air or water when the
    /// whole voxel was dug out, and otherwise keeps the generator's value
    /// (a hole smaller than a voxel does not show from afar). Edited chunks
    /// on a neighbouring face map into the padding past the shared edge
    /// through `Planet::extended_coords`; the one diagonal padding column at
    /// a cube corner (three faces meet, no face is there) is left natural.
    fn overlay_coarse(&self, key: ChunkKey, ids: &mut [BlockId]) -> bool {
        #[derive(Default, Clone, Copy)]
        struct Agg {
            total: u64,
            nonsolid: u64,
            water: bool,
            /// Topmost solid block: (layer, id, placed by an edit).
            top: Option<(i64, BlockId, bool)>,
        }
        let lod = key.lod;
        let v = 1i64 << lod;
        let (i0, j0, k0) = key.origin_blocks();
        let (i0, j0, k0) = (i0 as i64, j0 as i64, k0 as i64);
        let c = CHUNK as i64;
        let lo = (i0 - v, j0 - v, k0 - v);
        let hi = (i0 + (c + 1) * v, j0 + (c + 1) * v, k0 + (c + 1) * v);
        let mut agg: FastMap<usize, Agg> = FastMap::default();
        let planet = &self.planet;
        for e in self.edited_near(key) {
            let (ei, ej, ek) = e.origin_blocks();
            let (ei, ej, ek) = (ei as i64, ej as i64, ek as i64);
            let cross = e.face != key.face;
            if ek + c <= lo.2 || ek >= hi.2 {
                continue;
            }
            if !cross && (ei + c <= lo.0 || ei >= hi.0 || ej + c <= lo.1 || ej >= hi.1) {
                continue;
            }
            // Column (x, y) of the edited chunk -> padded voxel (vx, vy) of this one.
            let voxel_xy = |x: usize, y: usize| -> Option<(i64, i64)> {
                let (bi, bj) = if cross {
                    let (i, j) = planet.extended_coords(key.face, e.face, (ei + x as i64) as f64 + 0.5, (ej + y as i64) as f64 + 0.5)?;
                    (i.floor() as i64, j.floor() as i64)
                } else {
                    (ei + x as i64, ej + y as i64)
                };
                let (vx, vy) = ((bi - i0).div_euclid(v), (bj - j0).div_euclid(v));
                ((-1..=c).contains(&vx) && (-1..=c).contains(&vy)).then_some((vx, vy))
            };
            let natural = self.natural(e);
            self.edits.with_chunk(e, |dense| {
                for x in 0..CHUNK {
                    for y in 0..CHUNK {
                        let Some((vx, vy)) = voxel_xy(x, y) else { continue };
                        for z in 0..CHUNK {
                            let gk = ek + z as i64;
                            let vz = (gk - k0).div_euclid(v);
                            if !(-1..=c).contains(&vz) {
                                continue;
                            }
                            let id = dense[index(x, y, z)];
                            let a = agg.entry(pidx(vx as i32, vy as i32, vz as i32)).or_default();
                            a.total += 1;
                            if self.registry.is_solid(id) {
                                let edited = natural.get(x, y, z) != id;
                                // Highest layer wins; on a tie (a deck flush with the
                                // field) the edit wins, or the bridge would vanish.
                                if a.top.is_none_or(|(tk, _, te)| gk > tk || (gk == tk && edited && !te)) {
                                    a.top = Some((gk, id, edited));
                                }
                            } else {
                                a.nonsolid += 1;
                                if self.registry.is_liquid(id) {
                                    a.water = true;
                                }
                            }
                        }
                    }
                }
            });
        }
        let mut touched = false;
        let full = (v * v * v) as u64;
        for (idx, a) in agg {
            match a.top {
                Some((_, id, true)) => {
                    ids[idx] = id;
                    touched = true;
                }
                Some(_) => {}
                None if a.total == full && a.nonsolid == full => {
                    ids[idx] = if a.water { self.water_id() } else { self.generator.blocks.air };
                    touched = true;
                }
                None => {}
            }
        }
        touched
    }
}

impl Voxels for World {
    #[inline]
    fn block(&self, face: u8, i: i64, j: i64, k: i64) -> BlockId {
        self.block_lattice(face, i, j, k)
    }
    #[inline]
    fn is_solid(&self, id: BlockId) -> bool {
        self.registry.is_solid(id)
    }
    #[inline]
    fn is_liquid(&self, id: BlockId) -> bool {
        self.registry.is_liquid(id)
    }
    #[inline]
    fn water_level(&self, face: u8, i: i64, j: i64, k: i64) -> u8 {
        match self.resolve_block(face, i, j, k) {
            Some(bp) => self.water_level_at(bp),
            None => 0,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::worldgen::bake;

    pub fn test_world() -> World {
        let registry = Arc::new(Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap());
        let bake = Arc::new(bake::open("C:/code/triangulum/game/assets/world/seed42").expect("world bake"));
        let planet = Planet::new(1.0, 2048, 512, 131072);
        let vscale = planet.radius / 6_371_000.0;
        let generator = Arc::new(Generator::new(planet, bake, 42, vscale, 1.0, 1.0, &registry).unwrap());
        World::new(generator, registry, Arc::new(EditStore::new()))
    }

    /// The hills anchor's ground block: (face, i, j, k of the top solid block).
    pub fn hills_ground(w: &World) -> BlockPos {
        let dir = crate::engine::gnomonic_dir(crate::engine::HILLS.0, crate::engine::HILLS.1, crate::engine::HILLS.2);
        let (f, i, j) = w.planet.lattice_of_dir(dir);
        let (i, j) = (i.floor() as u32, j.floor() as u32);
        let mut k = (w.planet.sea_level as f64 + w.generator.ground_height_m(dir) / w.planet.block_size) as u32 + 4;
        while !w.registry.is_solid(w.block_at(BlockPos { face: f, i, j, k })) {
            k -= 1;
        }
        BlockPos { face: f, i, j, k }
    }

    #[test]
    fn edits_are_authoritative_and_reach_neighbours() {
        let w = test_world();
        let g = hills_ground(&w);
        let stone = w.registry.id("stone").unwrap();
        let air = w.registry.id("air").unwrap();
        // Place a block in the air above the ground and dig the ground block.
        let above = BlockPos { k: g.k + 3, ..g };
        assert_eq!(w.block_at(above), air);
        assert!(w.set_block(above, stone));
        assert!(!w.set_block(above, stone), "no change the second time");
        assert!(w.set_block(g, air));
        assert_eq!(w.block_at(above), stone);
        assert_eq!(w.block_at(g), air);
        let (key, x, y, z) = World::chunk_of(above);
        let made = w.generate(key);
        assert_eq!(made.chunk.get(x, y, z), stone, "generate() carries the edit");
        assert_eq!(made.ids[pidx(x as i32, y as i32, z as i32)], stone);
        assert_eq!(w.generator.generate(key).chunk.get(x, y, z), air, "the generator itself stays pure");

        // A block on the chunk's x = 0 border appears in the -x neighbour's padding.
        let (i0, j0, k0) = key.origin_blocks();
        let edge = BlockPos { face: g.face, i: i0 as u32, j: j0 as u32 + 5, k: k0 as u32 + 5 };
        let marker = w.registry.id("marker_red").unwrap();
        assert!(w.set_block(edge, marker));
        let left = ChunkKey { cx: key.cx - 1, ..key };
        let gl = w.generate(left);
        assert_eq!(gl.ids[pidx(32, 5, 5)], marker, "neighbour padding sees the edit");
        let affected = w.affected_chunks(edge);
        assert!(affected.contains(&key) && affected.contains(&left), "{affected:?}");
        assert!(affected.contains(&key.parent().unwrap()));
        assert!(affected.iter().any(|k| k.lod == w.planet.max_lod));
        // An interior block touches one LOD-0 chunk and, at every LOD, its own
        // ancestor; a coarse neighbour only when the block lies in that
        // neighbour's padding voxel (review #11), which is then adjacent.
        let inner = BlockPos { face: g.face, i: i0 as u32 + 10, j: j0 as u32 + 10, k: k0 as u32 + 10 };
        let affected = w.affected_chunks(inner);
        assert_eq!(affected.iter().filter(|k| k.lod == 0).count(), 1, "{affected:?}");
        let mut anc = key;
        for lod in 0..=w.planet.max_lod {
            assert_eq!(anc.lod, lod);
            assert!(affected.contains(&anc), "LOD {lod}: own ancestor {anc:?} in {affected:?}");
            for k in affected.iter().filter(|k| k.lod == lod && **k != anc) {
                let d = (k.cx as i64 - anc.cx as i64).abs() + (k.cy as i64 - anc.cy as i64).abs() + (k.cz as i64 - anc.cz as i64).abs();
                assert_eq!(d, 1, "LOD {lod}: {k:?} is a padding neighbour of {anc:?}");
                let e = (CHUNK as i64) << lod;
                let (i, j, kk) = (inner.i as i64, inner.j as i64, inner.k as i64);
                let edge = |c: i64| ((c % e) >> lod) == 0 || ((c % e) >> lod) == CHUNK as i64 - 1;
                assert!(edge(i) || edge(j) || edge(kk), "LOD {lod}: the block is in a border voxel");
            }
            anc = anc.parent().unwrap();
        }
    }

    #[test]
    fn edits_propagate_to_coarse_lods() {
        let w = test_world();
        let g = hills_ground(&w);
        let stone = w.registry.id("stone").unwrap();
        let air = w.registry.id("air").unwrap();
        // A one-block-thick stone bridge, 12 m up, 8 blocks long in i.
        let k_bridge = g.k + 12;
        for di in 0..8 {
            assert!(w.set_block(BlockPos { face: g.face, i: g.i + di, j: g.j, k: k_bridge }, stone));
        }
        for lod in 1..=4u8 {
            let e = 32u32 << lod;
            let key = ChunkKey::neisor(g.face, lod, g.i / e, g.j / e, k_bridge / e);
            let (i0, j0, k0) = key.origin_blocks();
            let vx = ((g.i as u64 - i0) >> lod) as usize;
            let vy = ((g.j as u64 - j0) >> lod) as usize;
            let vz = ((k_bridge as u64 - k0) >> lod) as usize;
            let pure = w.generator.generate(key).chunk;
            let with = w.generate(key).chunk;
            assert_eq!(with.get(vx, vy, vz), stone, "LOD {lod}: the bridge voxel shows stone");
            if lod <= 2 {
                assert_eq!(pure.get(vx, vy, vz), air, "LOD {lod}: the generator alone has air there");
            }
            // Voxels beside the bridge are untouched.
            if vy + 2 < 32 {
                assert_eq!(with.get(vx, vy + 2, vz), pure.get(vx, vy + 2, vz));
            }
        }
        // A block placed flush with the natural surface (a deck across a
        // trench) ties with the grass beside it for the topmost layer; the
        // edit wins, so the deck shows at every LOD.
        let (fi, fj) = (g.i + 24, g.j + 2);
        let mut kt = g.k + 6;
        while !w.registry.is_solid(w.block_at(BlockPos { face: g.face, i: fi, j: fj, k: kt })) {
            kt -= 1;
        }
        let marker = w.registry.id("marker_white").unwrap();
        assert!(w.set_block(BlockPos { face: g.face, i: fi, j: fj, k: kt }, marker));
        for lod in 1..=3u8 {
            let e = 32u32 << lod;
            let key = ChunkKey::neisor(g.face, lod, fi / e, fj / e, kt / e);
            let (i0, j0, k0) = key.origin_blocks();
            let (vx, vy, vz) = (((fi as u64 - i0) >> lod) as usize, ((fj as u64 - j0) >> lod) as usize, ((kt as u64 - k0) >> lod) as usize);
            // The edit shows unless a natural column inside the voxel rises above it.
            let v = 1u32 << lod;
            let (ci0, cj0) = (fi & !(v - 1), fj & !(v - 1));
            let kv0 = kt & !(v - 1);
            let higher_natural = (ci0..ci0 + v).any(|i| (cj0..cj0 + v).any(|j| (kt + 1..kv0 + v).any(|k| (i, j) != (fi, fj) && w.registry.is_solid(w.block_at(BlockPos { face: g.face, i, j, k })))));
            let shown = w.generate(key).chunk.get(vx, vy, vz);
            if higher_natural {
                assert!(w.registry.is_solid(shown), "LOD {lod}: a higher natural block keeps the voxel solid");
            } else {
                assert_eq!(shown, marker, "LOD {lod}: a surface-level edit shows");
            }
        }
        // Dig a 2x2x2 hole aligned to LOD 1 voxels at the surface: that voxel
        // becomes air at LOD 1; a single dug block does not.
        let ai = (g.i + 16) & !1;
        let aj = (g.j + 16) & !1;
        let mut k_top = g.k + 6;
        while !w.registry.is_solid(w.block_at(BlockPos { face: g.face, i: ai, j: aj, k: k_top })) {
            k_top -= 1;
        }
        let ak = k_top & !1;
        // make sure both layers ak, ak+1 are solid in all four columns first
        let all_solid = (0..2).all(|dx| (0..2).all(|dy| (0..2).all(|dz| w.registry.is_solid(w.block_at(BlockPos { face: g.face, i: ai + dx, j: aj + dy, k: ak - 2 + dz })))));
        assert!(all_solid);
        let ak = ak - 2;
        let key1 = ChunkKey::neisor(g.face, 1, ai / 64, aj / 64, ak / 64);
        let (i0, j0, k0) = key1.origin_blocks();
        let (vx, vy, vz) = (((ai as u64 - i0) / 2) as usize, ((aj as u64 - j0) / 2) as usize, ((ak as u64 - k0) / 2) as usize);
        assert!(w.set_block(BlockPos { face: g.face, i: ai, j: aj, k: ak + 1 }, air));
        assert!(w.registry.is_solid(w.generate(key1).chunk.get(vx, vy, vz)), "one dug block keeps the coarse voxel solid");
        for dx in 0..2 {
            for dy in 0..2 {
                for dz in 0..2 {
                    w.set_block(BlockPos { face: g.face, i: ai + dx, j: aj + dy, k: ak + dz }, air);
                }
            }
        }
        assert_eq!(w.generate(key1).chunk.get(vx, vy, vz), air, "a fully dug voxel reads air at LOD 1");
    }

    /// Review #11: a block in the first or last voxel of a coarse chunk lies
    /// in the neighbour's padding voxel, so the neighbour must be
    /// invalidated: at LOD 1 chunk 0's padding spans blocks 64..66, and
    /// block 65 used to miss it (stepping one block landed in chunk 1).
    #[test]
    fn coarse_halo_invalidation_steps_a_whole_voxel() {
        let w = test_world();
        let bp = |i: u32| BlockPos { face: 2, i, j: 1000, k: 600 };
        let l1 = |cx: u32| ChunkKey::neisor(2, 1, cx, 1000 / 64, 600 / 64);
        let l2 = |cx: u32| ChunkKey::neisor(2, 2, cx, 1000 / 128, 600 / 128);
        for i in [64, 65] {
            let a = w.affected_chunks(bp(i));
            assert!(a.contains(&l1(1)) && a.contains(&l1(0)), "block {i}: {a:?}");
        }
        for i in [62, 63] {
            let a = w.affected_chunks(bp(i));
            assert!(a.contains(&l1(0)) && a.contains(&l1(1)), "block {i} (last voxel of chunk 0): {a:?}");
        }
        let a = w.affected_chunks(bp(66));
        assert!(a.contains(&l1(1)) && !a.contains(&l1(0)), "voxel 1 is nobody's padding: {a:?}");
        // LOD 2: voxels of 4 blocks, chunks of 128; blocks 128..132 are chunk 0's padding.
        for i in [128, 131] {
            let a = w.affected_chunks(bp(i));
            assert!(a.contains(&l2(0)) && a.contains(&l2(1)), "block {i} at LOD 2: {a:?}");
        }
        assert!(!w.affected_chunks(bp(132)).contains(&l2(0)));
        // Along k as well: layer 65 is in LOD-1 chunk cz = 1's first voxel.
        let a = w.affected_chunks(BlockPos { face: 2, i: 1000, j: 1000, k: 65 });
        assert!(a.contains(&ChunkKey::neisor(2, 1, 1000 / 64, 1000 / 64, 0)), "{a:?}");
    }

    /// Review #5: hiding water is a view flag; the world an edit is made in
    /// is the same world, so an edit beside water keeps the water.
    #[test]
    fn hiding_water_does_not_change_what_an_edit_makes_authoritative() {
        let w = test_world();
        let p = w.planet;
        // An ocean column: sea level block is water.
        let mut found = None;
        for i in (1000..p.n).step_by(2048) {
            let dir = p.dir(1, i as f64 + 0.5, 50000.5);
            if w.generator.column(dir, p.block_size).ocean {
                found = Some(i);
                break;
            }
        }
        let i = found.expect("an ocean column on face 1");
        let sea = BlockPos { face: 1, i, j: 50000, k: p.sea_level - 1 };
        assert_eq!(w.block_at(sea), w.water_id());
        // Edit a block far below the surface in this chunk (as if water were hidden on screen).
        let deep = BlockPos { k: sea.k - 20, ..sea };
        assert!(w.set_block(deep, w.registry.id("marker_red").unwrap()));
        let (key, x, y, z) = World::chunk_of(sea);
        assert_eq!(w.edits.get(key, x, y, z), Some(w.water_id()), "the authoritative chunk still holds the sea");
        assert_eq!(w.edits.water_level(key, x, y, z), Some(255));
        assert_eq!(w.generate(key).chunk.water_level(x, y, z), 255, "and the compressed chunk carries the level");
    }

    /// An edit in the last column of one face shows in the padding of the
    /// neighbouring face's coarse chunks (the M1b gap), and the natural
    /// cache evicts one chunk at a time instead of clearing.
    #[test]
    fn edits_cross_the_face_edge_into_coarse_padding() {
        let w = test_world();
        let p = w.planet;
        let n = p.n;
        // Face 0's i = N side meets face 2 (edge_maps test). Pick a land column there.
        let m = p.edges[0][1];
        assert_eq!(m.face, 2);
        let marker = w.registry.id("marker_red").unwrap();
        let mut found = None;
        for j in (1000..n).step_by(4096) {
            let dir = p.dir(0, n as f64 - 0.5, j as f64 + 0.5);
            let c = w.generator.column(dir, p.block_size);
            if !c.ocean && c.height_m > 5.0 {
                found = Some((j, c.height_m));
                break;
            }
        }
        let (j, h) = found.expect("land on the edge");
        let k = (p.sea_level as f64 + h as f64) as u32 + 12; // well above the ground: air naturally
        let bp = BlockPos { face: 0, i: n - 1, j, k };
        assert_eq!(w.block_at(bp), w.generator.blocks.air);
        assert!(w.set_block(bp, marker));
        // Where that block lies in face 2's extended lattice: just past one of its sides.
        let (xi, xj) = p.extended_coords(2, 0, (n - 1) as f64 + 0.5, j as f64 + 0.5).unwrap();
        assert!(!(0.0..=n as f64).contains(&xi) || !(0.0..=n as f64).contains(&xj), "past the side: ({xi}, {xj})");
        let affected = w.affected_chunks(bp);
        for lod in 1..=4u8 {
            let e = 32i64 << lod;
            let per_edge = p.chunks_per_edge(lod) as i64;
            let cx = ((xi.floor() as i64).div_euclid(e)).clamp(0, per_edge - 1);
            let cy = ((xj.floor() as i64).div_euclid(e)).clamp(0, per_edge - 1);
            let key = ChunkKey::neisor(2, lod, cx as u32, cy as u32, k / e as u32);
            let (i0, j0, k0) = key.origin_blocks();
            let v = 1i64 << lod;
            let vx = (xi.floor() as i64 - i0 as i64).div_euclid(v);
            let vy = (xj.floor() as i64 - j0 as i64).div_euclid(v);
            let vz = ((k as i64 - k0 as i64) >> lod) as i32;
            assert!(vx == -1 || vx == 32 || vy == -1 || vy == 32, "LOD {lod}: padding voxel ({vx}, {vy})");
            assert!(w.edited_near(key).contains(&World::chunk_of(bp).0), "LOD {lod}: the neighbour-face edit is found");
            let g = w.generate(key);
            assert_eq!(g.ids[pidx(vx as i32, vy as i32, vz)], marker, "LOD {lod}: face 2's padding shows face 0's edit");
            assert!(affected.contains(&key), "LOD {lod}: the face-2 chunk is invalidated by the edit");
        }
        // Cache eviction: filling past the limit keeps the newest and drops the oldest, one by one.
        for i in 0..(NATURAL_CACHE_MAX as u32 + 5) {
            w.natural(ChunkKey::neisor(4, 0, i % 4096, 7, 20));
        }
        assert_eq!(w.natural_cached(), NATURAL_CACHE_MAX);
    }
}

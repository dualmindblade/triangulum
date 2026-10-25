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
}

/// Generator + edits. Chunks the generator produces are cached at LOD 0 for
/// physics and for the coarse-LOD edit rule; edited chunks come from the
/// store. `generate` is what the streamer's workers call.
pub struct World {
    pub planet: Planet,
    pub generator: Arc<Generator>,
    pub registry: Arc<Registry>,
    pub edits: Arc<EditStore>,
    natural: Mutex<FastMap<ChunkKey, Arc<Chunk>>>,
}

/// Natural chunks kept for physics; cleared wholesale when exceeded.
const NATURAL_CACHE_MAX: usize = 1024;

impl World {
    pub fn new(generator: Arc<Generator>, registry: Arc<Registry>, edits: Arc<EditStore>) -> Self {
        Self { planet: generator.planet, generator, registry, edits, natural: Mutex::new(FastMap::default()) }
    }

    pub fn water_id(&self) -> BlockId {
        self.generator.blocks.water
    }

    /// The generator's LOD-0 chunk (pure, cached).
    pub fn natural(&self, key: ChunkKey) -> Arc<Chunk> {
        debug_assert_eq!(key.lod, 0);
        if let Some(c) = self.natural.lock().unwrap().get(&key) {
            return c.clone();
        }
        let c = Arc::new(self.generator.generate(key).chunk);
        let mut g = self.natural.lock().unwrap();
        if g.len() >= NATURAL_CACHE_MAX {
            g.clear();
        }
        g.entry(key).or_insert(c).clone()
    }

    pub fn natural_cached(&self) -> usize {
        self.natural.lock().unwrap().len()
    }

    #[inline]
    pub fn chunk_of(bp: BlockPos) -> (ChunkKey, usize, usize, usize) {
        let c = CHUNK as u32;
        (ChunkKey { face: bp.face, lod: 0, cx: bp.i / c, cy: bp.j / c, cz: bp.k / c }, (bp.i % c) as usize, (bp.j % c) as usize, (bp.k % c) as usize)
    }

    /// Block at a position inside the world (edits first).
    pub fn block_at(&self, bp: BlockPos) -> BlockId {
        let (key, x, y, z) = Self::chunk_of(bp);
        if let Some(id) = self.edits.get(key, x, y, z) {
            return id;
        }
        self.natural(key).get(x, y, z)
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

    /// Set one block. Returns whether it changed; queues the remeshes.
    pub fn set_block(&self, bp: BlockPos, id: BlockId) -> bool {
        if bp.k >= self.planet.height || bp.i >= self.planet.n || bp.j >= self.planet.n || bp.face > 5 {
            return false;
        }
        let (key, x, y, z) = Self::chunk_of(bp);
        let changed = self.edits.set(key, x, y, z, id, || self.natural(key).to_dense());
        if changed {
            self.edits.push_invalidated(self.affected_chunks(bp));
        }
        changed
    }

    /// Every chunk, at every LOD, whose padded voxel grid contains `bp`:
    /// the chunk column up the LOD tree, plus neighbours when the block
    /// sits on a chunk border (across face edges too).
    pub fn affected_chunks(&self, bp: BlockPos) -> Vec<ChunkKey> {
        let mut out = Vec::new();
        let (i, j, k) = (bp.i as i64, bp.j as i64, bp.k as i64);
        for lod in 0..=self.planet.max_lod {
            let e = (CHUNK as i64) << lod;
            let per_edge = self.planet.chunks_per_edge(lod);
            let per_col = self.planet.chunks_per_column(lod);
            let offsets = |c: i64| -> Vec<i64> {
                let local = (c % e) >> lod; // voxel index within the chunk
                let mut v = vec![0];
                if local == 0 {
                    v.push(-1);
                }
                if local == (CHUNK as i64) - 1 {
                    v.push(1);
                }
                v
            };
            for &dz in &offsets(k) {
                let ck = k + dz;
                if ck < 0 || ck >= self.planet.height as i64 {
                    continue;
                }
                for &dx in &offsets(i) {
                    for &dy in &offsets(j) {
                        // A block one past the face edge resolves onto the neighbouring face.
                        let (f, ri, rj) = self.resolve(bp.face, i + dx, j + dy);
                        let key = ChunkKey { face: f, lod, cx: ri / e as u32, cy: rj / e as u32, cz: (ck / e) as u32 };
                        if key.cx < per_edge && key.cy < per_edge && key.cz < per_col {
                            out.push(key);
                        }
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Generate one chunk at its LOD: the pure generator, then the edits.
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
            g.chunk = save::chunk_of_dense(&dense, self.water_id());
        }
        g
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
        let per_edge = self.planet.chunks_per_edge(0);
        let at_face_edge = key.cx == 0 || key.cy == 0 || key.cx + 1 == per_edge || key.cy + 1 == per_edge;
        let near = self.edits.keys().into_iter().any(|e| {
            if e.face == key.face {
                (e.cx as i64 - key.cx as i64).abs() <= 1 && (e.cy as i64 - key.cy as i64).abs() <= 1 && (e.cz as i64 - key.cz as i64).abs() <= 1 && e != key
            } else {
                at_face_edge
            }
        });
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
    /// (a hole smaller than a voxel does not show from afar).
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
        for e in self.edits.keys() {
            if e.face != key.face {
                continue;
            }
            let (ei, ej, ek) = e.origin_blocks();
            let (ei, ej, ek) = (ei as i64, ej as i64, ek as i64);
            if ei + c <= lo.0 || ei >= hi.0 || ej + c <= lo.1 || ej >= hi.1 || ek + c <= lo.2 || ek >= hi.2 {
                continue;
            }
            let natural = self.natural(e);
            self.edits.with_chunk(e, |dense| {
                for x in 0..CHUNK {
                    let vx = (ei + x as i64 - i0).div_euclid(v);
                    if !(-1..=c).contains(&vx) {
                        continue;
                    }
                    for y in 0..CHUNK {
                        let vy = (ej + y as i64 - j0).div_euclid(v);
                        if !(-1..=c).contains(&vy) {
                            continue;
                        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::bake;

    pub fn test_world() -> World {
        let registry = Arc::new(Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap());
        let bake = Arc::new(bake::open("C:/code/triangulum/game/assets/world/seed42").expect("world bake"));
        let planet = Planet::new(1.0, 2048, 512, 131072);
        let vscale = planet.radius / 6_371_000.0;
        let generator = Arc::new(Generator::new(planet, bake, 42, vscale, 1.0, &registry).unwrap());
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
        // An interior block only touches its own chunk column.
        let inner = BlockPos { face: g.face, i: i0 as u32 + 10, j: j0 as u32 + 10, k: k0 as u32 + 10 };
        let affected = w.affected_chunks(inner);
        assert_eq!(affected.len(), w.planet.max_lod as usize + 1, "{affected:?}");
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
            let key = ChunkKey { face: g.face, lod, cx: g.i / e, cy: g.j / e, cz: k_bridge / e };
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
            let key = ChunkKey { face: g.face, lod, cx: fi / e, cy: fj / e, cz: kt / e };
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
        let key1 = ChunkKey { face: g.face, lod: 1, cx: ai / 64, cy: aj / 64, cz: ak / 64 };
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
}

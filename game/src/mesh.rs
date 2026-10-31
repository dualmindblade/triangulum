//! Binary greedy meshing of a padded chunk into 8-byte quads.
//!
//! Quad layout: lo = axis:2 | sign:1 | slice:5 | u:5 | v:5 | w-1:5 | h-1:5;
//! hi = block id:16 | ao:8 (four 2-bit corner values, corner c = cu + 2 cv).
//! Opaque quads are emitted where a solid voxel meets a non-solid one; water
//! quads where a liquid voxel meets air. Quads never cross a curvature-grid
//! cell in the tangent directions, so the bilinear warp stays crack-free.
//!
//! Water quads (M2b) are 16 bytes: the same two words, then a word of four
//! 8-bit corner heights (corner c = cu + 2 cv again, 255 = the top of the
//! voxel) and a spare word. A top face is drawn at height level / 255 inside
//! its block, with each corner averaged over the water cells sharing that
//! lattice corner (Minecraft-style, `Surface::corner`), so neighbouring
//! cells' surfaces meet exactly; the top edge of a side face against air
//! takes the same two corners, so falls and partial columns are watertight.
//! A cell with water above it counts as full. Full water (the ocean, resting
//! lakes, every coarse LOD) carries 255s and is the geometry it always was.
//!
//! Grid record: 25 offsets (vec4, xyz = corner position at the chunk's
//! lattice floor relative to the chunk origin) then 25 unit directions
//! (vec4). `off[0].w` carries `r0`, the radius of the lattice floor, so a
//! vertex shader can place vertices at their exact radius `r0 + z` instead
//! of on the bilinear chord (review #11: at LOD 9 the chord sags 23 m below
//! the sphere mid-cell, so coarse and fine water surfaces did not meet).

use crate::grid::{CHUNK, ChunkKey, Planet};
use crate::world::Registry;
use crate::worldgen::{Generated, PAD, pidx};
use glam::DVec3;
use std::time::Instant;

/// Curvature grid: 4x4 cells (5x5 corners) per chunk footprint.
pub const GRID_CELLS: usize = 4;
pub const GRID_CORNERS: usize = GRID_CELLS + 1;
/// Longest tangent edge of a sea-surface quad, in metres. Every vertex
/// sits on its exact radius, but the edges between them are chords of the
/// sphere, sagging `L^2 / 8R` below it mid-edge: 23 m for an eight-voxel
/// quad at LOD 9, where the finer neighbour's vertices along the shared
/// edge sat on the sphere (review 2 #9, the seam). The sea is the one
/// surface that lies at exactly the same radius at every LOD, so every
/// quad on the sea-level plane (water, ice, the coarsest LOD's opaque
/// deep ocean, a beach at sea level) and every water quad is capped at
/// 256 m: the sag stays under 0.1 m, and at every LOD coarser than 5 the
/// edge vertices of neighbouring LODs coincide (the cap is a voxel
/// multiple at both), so there is no T-junction to sag against at all.
pub const SEA_EDGE_MAX_M: f64 = 256.0;
/// f32s per grid record: 25 offsets (vec4) + 25 directions (vec4).
pub const GRID_F32S: usize = GRID_CORNERS * GRID_CORNERS * 4 * 2;
pub const GRID_BYTES: usize = GRID_F32S * 4;
/// Index of `r0` (the lattice floor's radius) in the grid record.
pub const GRID_R0_INDEX: usize = 3;
/// Corner heights of a full (or coarse) water quad: 255 at every corner.
pub const WATER_FLAT: u32 = 0xFFFF_FFFF;

pub type Quad = [u32; 2];
/// A water quad: `Quad` plus the corner heights and a spare word.
pub type WaterQuad = [u32; 4];

pub struct MeshData {
    pub key: ChunkKey,
    pub opaque: Vec<Quad>,
    pub water: Vec<WaterQuad>,
    pub grid: Vec<f32>,
    /// World position of the chunk's lattice corner (i0, j0, k0).
    pub origin: DVec3,
    pub center: DVec3,
    pub radius: f32,
    pub mesh_us: u32,
}

#[derive(Clone, Copy)]
pub struct MeshOptions {
    /// Restrict greedy quads to curvature-grid cells (crack-free warp).
    pub split: bool,
    /// Per-vertex ambient occlusion (quads only merge across equal AO).
    pub ao: bool,
}

impl Default for MeshOptions {
    fn default() -> Self {
        Self { split: true, ao: true }
    }
}

const INNER: u64 = 0x1_FFFF_FFFE; // bits 1..=32 (interior z)

#[inline]
fn col(x: i32, y: i32) -> usize {
    ((x + 1) as usize) * PAD + (y + 1) as usize
}

/// Chunk-local voxel of (axis, slice, u, v).
#[inline]
fn voxel_of(axis: u32, slice: usize, u: usize, v: usize) -> (i32, i32, i32) {
    match axis {
        0 => (slice as i32, u as i32, v as i32),
        1 => (u as i32, slice as i32, v as i32),
        _ => (u as i32, v as i32, slice as i32),
    }
}

/// Column occupancy masks: bit (z + 1) set when voxel (x, y, z) is in the set.
struct Masks {
    solid: Vec<u64>,
    liquid: Vec<u64>,
}

impl Masks {
    #[inline]
    fn solid_at(&self, x: i32, y: i32, z: i32) -> bool {
        if !(-1..=CHUNK as i32).contains(&x) || !(-1..=CHUNK as i32).contains(&y) || !(-1..=CHUNK as i32).contains(&z) {
            return false;
        }
        (self.solid[col(x, y)] >> (z + 1)) & 1 == 1
    }
    #[inline]
    fn liquid_at(&self, x: i32, y: i32, z: i32) -> bool {
        if !(-1..=CHUNK as i32).contains(&x) || !(-1..=CHUNK as i32).contains(&y) || !(-1..=CHUNK as i32).contains(&z) {
            return false;
        }
        (self.liquid[col(x, y)] >> (z + 1)) & 1 == 1
    }
}

/// The water surface of a padded LOD-0 grid with per-cell levels.
struct Surface<'a> {
    masks: &'a Masks,
    levels: &'a [u8],
}

impl Surface<'_> {
    /// Height (0..=255 of a voxel) of layer `z`'s water at lattice corner
    /// (cx, cy), cx and cy in 0..=32: the weighted mean over the four cells
    /// around the corner, water cells by their level (weight 10 at 0.8 and
    /// above, else 1), air cells as 0 with weight 1, solid cells left out;
    /// a water cell with water above it makes the corner full.
    fn corner(&self, cx: i32, cy: i32, z: i32) -> u8 {
        let (mut sum, mut w) = (0u32, 0u32);
        for (x, y) in [(cx - 1, cy - 1), (cx, cy - 1), (cx - 1, cy), (cx, cy)] {
            if self.masks.liquid_at(x, y, z) {
                if self.masks.liquid_at(x, y, z + 1) {
                    return 255;
                }
                let l = self.levels[pidx(x, y, z)] as u32;
                if l >= 204 {
                    sum += 10 * l;
                    w += 10;
                } else {
                    sum += l;
                    w += 1;
                }
            } else if !self.masks.solid_at(x, y, z) {
                w += 1;
            }
        }
        if w == 0 { 255 } else { ((sum + w / 2) / w) as u8 }
    }

    /// The four corner heights of a face of water voxel (x, y, z), packed
    /// as corner c = cu + 2 cv (the AO convention): a top face takes the
    /// surface at its four lattice corners; a side face takes it at its two
    /// top corners (cv = 1) and 255 below; a bottom face is flat.
    fn heights(&self, axis: u32, sign: u32, x: i32, y: i32, z: i32) -> u32 {
        let s = sign as i32;
        let h = |cu: i32, cv: i32| -> u32 {
            let v = match axis {
                2 if sign == 1 => self.corner(x + cu, y + cv, z),
                2 => 255,
                0 if cv == 1 => self.corner(x + s, y + cu, z),
                1 if cv == 1 => self.corner(x + cu, y + s, z),
                _ => 255,
            };
            v as u32
        };
        h(0, 0) | (h(1, 0) << 8) | (h(0, 1) << 16) | (h(1, 1) << 24)
    }
}

/// Mesh a generated chunk whose water is full everywhere (natural chunks,
/// coarse LODs).
pub fn mesh_chunk(generated: &Generated, registry: &Registry, planet: &Planet, opt: MeshOptions) -> MeshData {
    mesh_chunk_levels(generated, None, registry, planet, opt)
}

/// Mesh a generated chunk; `levels` (padded, `World::padded_levels`) gives
/// LOD-0 water its sloped surfaces.
pub fn mesh_chunk_levels(generated: &Generated, levels: Option<&[u8]>, registry: &Registry, planet: &Planet, opt: MeshOptions) -> MeshData {
    let t0 = Instant::now();
    let key = generated.key;
    let ids = &generated.ids;
    let mut masks = Masks { solid: vec![0u64; PAD * PAD], liquid: vec![0u64; PAD * PAD] };
    for x in -1..=CHUNK as i32 {
        for y in -1..=CHUNK as i32 {
            let c = col(x, y);
            let (mut s, mut l) = (0u64, 0u64);
            for z in -1..=CHUNK as i32 {
                let id = ids[pidx(x, y, z)];
                let bit = 1u64 << (z + 1);
                if registry.is_solid(id) {
                    s |= bit;
                } else if registry.is_liquid(id) {
                    l |= bit;
                }
            }
            masks.solid[c] = s;
            masks.liquid[c] = l;
        }
    }
    let mut opaque = Vec::new();
    let mut water = Vec::new();
    let any_solid = masks.solid.iter().any(|&m| m & INNER != 0);
    let any_liquid = masks.liquid.iter().any(|&m| m & INNER != 0);
    let sea_edge = sea_edge_voxels(planet, key.lod);
    // The slice whose top face lies on the sea-level plane, if it is in this chunk.
    let sea_slice = {
        let k0 = key.origin_blocks().2;
        let vb = key.voxel_blocks();
        let sea = planet.sea_level as u64;
        (sea > k0 && (sea - k0) % vb == 0 && (sea - k0) / vb <= CHUNK as u64).then(|| ((sea - k0) / vb - 1) as usize)
    };
    if any_solid {
        let blocking: Vec<u64> = masks.solid.clone();
        let cap = |axis: u32, sign: u32, slice: usize| if axis == 2 && sign == 1 && Some(slice) == sea_slice { sea_edge } else { CHUNK };
        // Merge key: block id and the four corner AO values.
        let face_key = |axis: u32, sign: u32, u: usize, v: usize, slice: usize| -> u32 {
            let (x, y, z) = voxel_of(axis, slice, u, v);
            let id = ids[pidx(x, y, z)] as u32;
            if !opt.ao {
                return id | (0xFF << 16);
            }
            id | (ao_bits(&masks, axis, sign, x, y, z) << 16)
        };
        mesh_set(&masks.solid, &blocking, &blocking, opt, &cap, &face_key, &mut |lo, k| opaque.push([lo, k]));
    }
    if any_liquid {
        let blocking: Vec<u64> = masks.solid.iter().zip(&masks.liquid).map(|(s, l)| s | l).collect();
        let cap = |_: u32, _: u32, _: usize| sea_edge;
        let surface = levels.filter(|_| key.lod == 0).map(|levels| Surface { masks: &masks, levels });
        // A top face hides only under water, or under a solid it touches: a
        // surface flat at the voxel's top lies in the solid's underside, so
        // drawing it would z-fight with that face and tint a view from
        // below twice. One lower than that has air over it, and shows.
        let top_blocking: Vec<u64> = (0..PAD * PAD)
            .map(|c| {
                let mut b = masks.liquid[c];
                let mut covered = masks.liquid[c] & (masks.solid[c] >> 1) & INNER;
                while covered != 0 {
                    let bit = covered.trailing_zeros() as i32; // the cell is z = bit - 1
                    covered &= covered - 1;
                    let (x, y) = (((c / PAD) as i32) - 1, ((c % PAD) as i32) - 1);
                    let interior = (0..CHUNK as i32).contains(&x) && (0..CHUNK as i32).contains(&y);
                    if !interior || surface.as_ref().is_none_or(|s| s.heights(2, 1, x, y, bit - 1) == WATER_FLAT) {
                        b |= 1 << (bit + 1);
                    }
                }
                b
            })
            .collect();
        // Merge key: block id and the four corner heights (full water merges as before).
        let face_key = |axis: u32, sign: u32, u: usize, v: usize, slice: usize| -> (u32, u32) {
            let (x, y, z) = voxel_of(axis, slice, u, v);
            let id = ids[pidx(x, y, z)] as u32 | (0xFF << 16);
            let heights = surface.as_ref().map_or(WATER_FLAT, |s| s.heights(axis, sign, x, y, z));
            (id, heights)
        };
        mesh_set(&masks.liquid, &blocking, &top_blocking, MeshOptions { split: opt.split, ao: false }, &cap, &face_key, &mut |lo, k: (u32, u32)| water.push([lo, k.0, k.1, 0]));
    }

    // Curvature grid record relative to the chunk's lattice corner.
    let vox_blocks = key.voxel_blocks() as f64;
    let (i0, j0, k0) = key.origin_blocks();
    let (i0, j0) = (i0 as f64, j0 as f64);
    let r0 = planet.radius_at(k0 as f64);
    let mut dirs = [DVec3::ZERO; GRID_CORNERS * GRID_CORNERS];
    for gy in 0..GRID_CORNERS {
        for gx in 0..GRID_CORNERS {
            let i = i0 + (gx * CHUNK / GRID_CELLS) as f64 * vox_blocks;
            let j = j0 + (gy * CHUNK / GRID_CELLS) as f64 * vox_blocks;
            dirs[gy * GRID_CORNERS + gx] = planet.dir(key.face, i, j);
        }
    }
    let origin = dirs[0] * r0;
    let mut grid = vec![0f32; GRID_F32S];
    for g in 0..GRID_CORNERS * GRID_CORNERS {
        let off = dirs[g] * r0 - origin;
        grid[g * 4..g * 4 + 3].copy_from_slice(&[off.x as f32, off.y as f32, off.z as f32]);
        let base = (GRID_CORNERS * GRID_CORNERS + g) * 4;
        grid[base..base + 3].copy_from_slice(&[dirs[g].x as f32, dirs[g].y as f32, dirs[g].z as f32]);
    }
    grid[GRID_R0_INDEX] = r0 as f32;
    let bounds = planet.chunk_bounds(key);
    MeshData { key, opaque, water, grid, origin, center: bounds.centre, radius: bounds.radius as f32, mesh_us: t0.elapsed().as_micros() as u32 }
}

/// The longest tangent edge of a sea-surface quad at `lod`, in voxels: a
/// power of two from 1 to the chunk edge (a divisor of the grid cell below
/// it, so quads still never cross a cell when they are split at cells).
pub fn sea_edge_voxels(planet: &Planet, lod: u8) -> usize {
    let v = (SEA_EDGE_MAX_M / planet.voxel_m(lod)).floor().clamp(1.0, CHUNK as f64) as usize;
    1 << v.ilog2()
}

/// Emit greedy quads for every face of `occ` voxels that borders a
/// non-`blocking` voxel (`top_blocking` for +z faces). `cap(axis, sign,
/// slice)` is the longest tangent edge, in voxels, of the quads in that face
/// plane; `face_key(axis, sign, u, v, slice)` is what a face must share to
/// merge (its id and shading); `emit(lo, key)` receives each quad.
#[allow(clippy::too_many_arguments)]
fn mesh_set<K: Copy + PartialEq>(occ: &[u64], blocking: &[u64], top_blocking: &[u64], opt: MeshOptions, cap: &dyn Fn(u32, u32, usize) -> usize, face_key: &dyn Fn(u32, u32, usize, usize, usize) -> K, emit: &mut dyn FnMut(u32, K)) {
    // Transposed masks: xcol[y][z] bits over x+1; ycol[x][z] bits over y+1.
    let mut xocc = vec![0u64; PAD * PAD];
    let mut yocc = vec![0u64; PAD * PAD];
    let mut xblk = vec![0u64; PAD * PAD];
    let mut yblk = vec![0u64; PAD * PAD];
    for x in -1..=CHUNK as i32 {
        for y in -1..=CHUNK as i32 {
            let c = col(x, y);
            for (src, tx, ty) in [(occ[c], &mut xocc, &mut yocc), (blocking[c], &mut xblk, &mut yblk)] {
                let mut m = src;
                while m != 0 {
                    let b = m.trailing_zeros() as usize; // z = b - 1
                    m &= m - 1;
                    tx[(y + 1) as usize * PAD + b] |= 1u64 << (x + 1);
                    ty[(x + 1) as usize * PAD + b] |= 1u64 << (y + 1);
                }
            }
        }
    }
    // Face planes: [axis*2+sign][slice*32 + v] bits over u.
    let mut planes = vec![[0u32; CHUNK * CHUNK]; 6];
    // Z faces: slice z, u = x, v = y.
    for x in 0..CHUNK {
        for y in 0..CHUNK {
            let c = col(x as i32, y as i32);
            let (o, b) = (occ[c], blocking[c]);
            let mut up = o & !(top_blocking[c] >> 1) & INNER;
            while up != 0 {
                let z = up.trailing_zeros() as usize - 1;
                up &= up - 1;
                planes[5][z * CHUNK + y] |= 1 << x;
            }
            let mut dn = o & !(b << 1) & INNER;
            while dn != 0 {
                let z = dn.trailing_zeros() as usize - 1;
                dn &= dn - 1;
                planes[4][z * CHUNK + y] |= 1 << x;
            }
        }
    }
    // X faces: slice x, u = y, v = z.
    for y in 0..CHUNK {
        for z in 0..CHUNK {
            let i = (y + 1) * PAD + z + 1;
            let (o, b) = (xocc[i], xblk[i]);
            let mut px = o & !(b >> 1) & INNER;
            while px != 0 {
                let x = px.trailing_zeros() as usize - 1;
                px &= px - 1;
                planes[1][x * CHUNK + z] |= 1 << y;
            }
            let mut nx = o & !(b << 1) & INNER;
            while nx != 0 {
                let x = nx.trailing_zeros() as usize - 1;
                nx &= nx - 1;
                planes[0][x * CHUNK + z] |= 1 << y;
            }
        }
    }
    // Y faces: slice y, u = x, v = z.
    for x in 0..CHUNK {
        for z in 0..CHUNK {
            let i = (x + 1) * PAD + z + 1;
            let (o, b) = (yocc[i], yblk[i]);
            let mut py = o & !(b >> 1) & INNER;
            while py != 0 {
                let y = py.trailing_zeros() as usize - 1;
                py &= py - 1;
                planes[3][y * CHUNK + z] |= 1 << x;
            }
            let mut ny = o & !(b << 1) & INNER;
            while ny != 0 {
                let y = ny.trailing_zeros() as usize - 1;
                ny &= ny - 1;
                planes[2][y * CHUNK + z] |= 1 << x;
            }
        }
    }

    let cell = CHUNK / GRID_CELLS;
    for dirset in 0..6u32 {
        let axis = dirset / 2;
        let sign = dirset & 1;
        // Tangent directions are split at grid cells and capped; the radial
        // one is linear.
        let split_u = opt.split;
        let split_v = opt.split && axis == 2;
        let plane = &mut planes[dirset as usize];
        for slice in 0..CHUNK {
            let edge = cap(axis, sign, slice);
            for v in 0..CHUNK {
                let vlim = if axis != 2 { CHUNK } else { (if split_v { (v / cell + 1) * cell } else { CHUNK }).min((v / edge + 1) * edge) };
                loop {
                    let row = plane[slice * CHUNK + v];
                    if row == 0 {
                        break;
                    }
                    let u0 = row.trailing_zeros() as usize;
                    let ulim = (if split_u { (u0 / cell + 1) * cell } else { CHUNK }).min((u0 / edge + 1) * edge);
                    let k = face_key(axis, sign, u0, v, slice);
                    let mut w = 1usize;
                    while u0 + w < ulim && (row >> (u0 + w)) & 1 == 1 && face_key(axis, sign, u0 + w, v, slice) == k {
                        w += 1;
                    }
                    let wmask: u32 = if w == 32 { !0 } else { ((1u32 << w) - 1) << u0 };
                    plane[slice * CHUNK + v] = row & !wmask;
                    let mut h = 1usize;
                    'rows: while v + h < vlim {
                        let r2 = plane[slice * CHUNK + v + h];
                        if r2 & wmask != wmask {
                            break;
                        }
                        for u in u0..u0 + w {
                            if face_key(axis, sign, u, v + h, slice) != k {
                                break 'rows;
                            }
                        }
                        plane[slice * CHUNK + v + h] = r2 & !wmask;
                        h += 1;
                    }
                    let lo = axis
                        | (sign << 2)
                        | ((slice as u32) << 3)
                        | ((u0 as u32) << 8)
                        | ((v as u32) << 13)
                        | (((w - 1) as u32) << 18)
                        | (((h - 1) as u32) << 23);
                    emit(lo, k);
                }
            }
        }
    }
}

/// Classic voxel AO: for each corner of the face, the two edge-adjacent
/// and the diagonal neighbour in the layer in front of the face.
/// Returns four 2-bit values packed as corner c = cu + 2 cv.
#[inline]
fn ao_bits(masks: &Masks, axis: u32, sign: u32, x: i32, y: i32, z: i32) -> u32 {
    let s = if sign == 1 { 1 } else { -1 };
    let (nx, ny, nz) = match axis {
        0 => (x + s, y, z),
        1 => (x, y + s, z),
        _ => (x, y, z + s),
    };
    // U and V axes of the face.
    let (eu, ev): ((i32, i32, i32), (i32, i32, i32)) = match axis {
        0 => ((0, 1, 0), (0, 0, 1)),
        1 => ((1, 0, 0), (0, 0, 1)),
        _ => ((1, 0, 0), (0, 1, 0)),
    };
    let occ = |dx: i32, dy: i32, dz: i32| masks.solid_at(nx + dx, ny + dy, nz + dz) as u32;
    let mut bits = 0u32;
    for c in 0..4u32 {
        let du = if c & 1 == 1 { 1 } else { -1 };
        let dv = if c & 2 == 2 { 1 } else { -1 };
        let side1 = occ(du * eu.0, du * eu.1, du * eu.2);
        let side2 = occ(dv * ev.0, dv * ev.1, dv * ev.2);
        let corner = occ(du * eu.0 + dv * ev.0, du * eu.1 + dv * ev.1, du * eu.2 + dv * ev.2);
        let ao = if side1 == 1 && side2 == 1 { 0 } else { 3 - (side1 + side2 + corner) };
        bits |= ao << (2 * c);
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::BlockPos;
    use crate::worldgen::Generator;

    fn generator() -> (Generator, Registry) {
        let registry = Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap();
        let bake = std::sync::Arc::new(crate::worldgen::bake::open("C:/code/triangulum/game/assets/world/seed42").expect("world bake"));
        let planet = Planet::new(1.0, 2048, 512, 131072);
        let vscale = planet.radius / 6_371_000.0;
        (Generator::new(planet, bake, 42, vscale, 1.0, 1.0, &registry).unwrap(), registry)
    }

    #[test]
    fn quads_are_well_formed_and_ao_costs_are_reported() {
        let (g, registry) = generator();
        let dir = cubesphere::face_to_dir(g.planet.proj, 2, 0.5, -0.1);
        let (f, i, j) = g.planet.lattice_of_dir(dir);
        let k = g.planet.sea_level as f64 + g.ground_height_m(dir);
        let keys = [
            ChunkKey::neisor(f, 0, i as u32 / 32, j as u32 / 32, (k as u32) / 32),
            ChunkKey::neisor(f, 4, i as u32 / 512, j as u32 / 512, (k as u32) / 512),
            ChunkKey::neisor(0, 9, 0, 0, 0),
        ];
        let (mut with, mut without) = (0usize, 0usize);
        for key in keys {
            let generated = g.generate(key);
            let m = mesh_chunk(&generated, &registry, &g.planet, MeshOptions::default());
            let m2 = mesh_chunk(&generated, &registry, &g.planet, MeshOptions { split: true, ao: false });
            for q in m.opaque.iter().copied().chain(m.water.iter().map(|w| [w[0], w[1]])) {
                let id = (q[1] & 0xFFFF) as u16;
                assert!(id != 0 && (id as usize) < registry.len(), "block {id}");
                let u = (q[0] >> 8) & 31;
                let v = (q[0] >> 13) & 31;
                let w = ((q[0] >> 18) & 31) + 1;
                let h = ((q[0] >> 23) & 31) + 1;
                assert!(u + w <= 32 && v + h <= 32);
                assert!(u / 8 == (u + w - 1) / 8, "quad crosses a grid cell in u");
            }
            with += m.opaque.len();
            without += m2.opaque.len();
            assert!(m2.opaque.len() <= m.opaque.len());
        }
        assert!(with > 0);
        println!("opaque quads with AO {with}, without {without}");
    }

    /// The shader's vertex transform, in f64 on the f32 grid record the
    /// GPU gets: bilinear offset and direction over the 4x4 cells, or,
    /// with `exact` at voxels of 64 m and up, the quartic through the five
    /// corners per axis (review 2 #9). `exact` also adds the radial
    /// correction `r0 (1 - |m|)` along the normalised direction (review
    /// #11), which `grid[GRID_R0_INDEX]` is for.
    fn vertex_pos(grid: &[f32], origin: DVec3, lp: DVec3, voxel: f64, exact: bool) -> DVec3 {
        let g = lp.truncate() * 0.125;
        let off = |c: usize| DVec3::new(grid[c * 4] as f64, grid[c * 4 + 1] as f64, grid[c * 4 + 2] as f64);
        let dir = |c: usize| {
            let base = (GRID_CORNERS * GRID_CORNERS + c) * 4;
            DVec3::new(grid[base] as f64, grid[base + 1] as f64, grid[base + 2] as f64)
        };
        let (o, m) = if exact && voxel >= 64.0 {
            let lagrange5 = |t: f64| -> [f64; 5] {
                let (t0, t1, t2, t3, t4) = (t, t - 1.0, t - 2.0, t - 3.0, t - 4.0);
                [t1 * t2 * t3 * t4 / 24.0, t0 * t2 * t3 * t4 / -6.0, t0 * t1 * t3 * t4 / 4.0, t0 * t1 * t2 * t4 / -6.0, t0 * t1 * t2 * t3 / 24.0]
            };
            let (wx, wy) = (lagrange5(g.x), lagrange5(g.y));
            let (mut o, mut m) = (DVec3::ZERO, DVec3::ZERO);
            for j in 0..5 {
                for i in 0..5 {
                    let w = wx[i] * wy[j];
                    o += off(j * 5 + i) * w;
                    m += dir(j * 5 + i) * w;
                }
            }
            (o, m)
        } else {
            let gi = (g.x.floor().min(3.0) as usize, g.y.floor().min(3.0) as usize);
            let gf = (g.x - gi.0 as f64, g.y - gi.1 as f64);
            let b = gi.1 * 5 + gi.0;
            let mix = |a: DVec3, b: DVec3, t: f64| a + (b - a) * t;
            (mix(mix(off(b), off(b + 1), gf.0), mix(off(b + 5), off(b + 6), gf.0), gf.1), mix(mix(dir(b), dir(b + 1), gf.0), mix(dir(b + 5), dir(b + 6), gf.0), gf.1))
        };
        if !exact {
            return origin + o + m * (lp.z * voxel);
        }
        let len = m.length();
        let r0 = grid[GRID_R0_INDEX] as f64;
        origin + o + (m / len) * (lp.z * voxel + r0 * (1.0 - len))
    }

    /// Review #11: at a LOD-8/LOD-9 boundary the coarse chunk's sea surface
    /// is a chord 23 m below the fine chunk's mid-cell; with the exact-radius
    /// correction the two surfaces are at the same radius to a centimetre.
    #[test]
    fn water_surfaces_meet_across_lod_boundaries_with_the_radius_correction() {
        let (g, registry) = generator();
        let planet = g.planet;
        // LOD 9 chunk (4, 4) of face 0 and its LOD 8 child (8, 8): both
        // start at the face centre and share their left edge i = 65536.
        let coarse = ChunkKey::neisor(0, 9, 4, 4, 0);
        let fine = ChunkKey::neisor(0, 8, 8, 8, 0);
        let mc = mesh_chunk(&g.generate(coarse), &registry, &planet, MeshOptions::default());
        let mf = mesh_chunk(&g.generate(fine), &registry, &planet, MeshOptions::default());
        assert!((mc.grid[GRID_R0_INDEX] as f64 - planet.radius_at(0.0)).abs() < 0.02, "r0 rides in the grid record");
        // The same lattice point on the shared edge: 2048 blocks along it
        // (the coarse cell's midpoint, a fine grid corner), at sea level.
        let sea_k = planet.sea_level as f64;
        let lp_c = DVec3::new(0.0, 2048.0 / 512.0, sea_k / 512.0);
        let lp_f = DVec3::new(0.0, 2048.0 / 256.0, sea_k / 256.0);
        let (vc, vf) = (planet.voxel_m(9), planet.voxel_m(8));
        let (mut worst_before, mut worst_after, mut worst_lateral) = (0.0f64, 0.0f64, 0.0f64);
        // Along the whole shared edge at the coarse cells' midpoints.
        for cell in 0..4 {
            let along = (cell as f64 + 0.5) * 8.0;
            let lp_c = DVec3::new(lp_c.x, along, lp_c.z);
            let lp_f = DVec3::new(lp_f.x, along * 2.0, lp_f.z);
            if lp_f.y > 32.0 {
                break; // the fine chunk covers half the coarse edge
            }
            let before = (vertex_pos(&mc.grid, mc.origin, lp_c, vc, false).length() - vertex_pos(&mf.grid, mf.origin, lp_f, vf, false).length()).abs();
            let pc = vertex_pos(&mc.grid, mc.origin, lp_c, vc, true);
            let pf = vertex_pos(&mf.grid, mf.origin, lp_f, vf, true);
            let after = (pc.length() - pf.length()).abs();
            worst_before = worst_before.max(before);
            worst_after = worst_after.max(after);
            worst_lateral = worst_lateral.max((pc - pf).length());
        }
        println!("L9/L8 sea surface radial mismatch: {worst_before:.2} m before, {worst_after:.4} m after; lateral {worst_lateral:.2} m");
        assert!(worst_before > 20.0 && worst_before < 26.0, "the chord sags about 23 m mid-cell at LOD 9: {worst_before}");
        assert!(worst_after < 0.05, "surfaces meet radially: {worst_after} m");
        // Both chunks' sea-level vertices land on the sea-level sphere.
        let pc = vertex_pos(&mc.grid, mc.origin, lp_c, vc, true);
        assert!((pc.length() - planet.radius).abs() < 0.05, "{}", pc.length() - planet.radius);
        // At LOD 0 the correction is below a millimetre, so skipping it there costs nothing.
        let k0 = ChunkKey::neisor(0, 0, 2048, 2048, 16);
        let m0 = mesh_chunk(&g.generate(k0), &registry, &planet, MeshOptions::default());
        let lp = DVec3::new(4.0, 4.0, 1.0);
        let d = (vertex_pos(&m0.grid, m0.origin, lp, 1.0, true) - vertex_pos(&m0.grid, m0.origin, lp, 1.0, false)).length();
        assert!(d < 0.002, "LOD 0 sag {d} m");
    }

    /// Unpack a quad: (axis, sign, slice, u0, v0, w, h).
    fn unpack(q: &Quad) -> (u32, u32, u32, u32, u32, u32, u32) {
        let lo = q[0];
        (lo & 3, (lo >> 2) & 1, (lo >> 3) & 31, (lo >> 8) & 31, (lo >> 13) & 31, ((lo >> 18) & 31) + 1, ((lo >> 23) & 31) + 1)
    }

    /// Review 2 #9: the seam is between the triangle edges that are really
    /// emitted. Along the boundary between an LOD-9 chunk and the LOD-8
    /// child of its eastern neighbour, every vertex of the fine chunk's sea
    /// quads on that line is measured against the coarse chunk's emitted
    /// sea edge there (the straight line between its two corner vertices,
    /// placed as the shader places them). The old eight-voxel quads left
    /// 23 m radially, and the bilinear curvature grid another 30 m along
    /// the edge mid-cell; with sea-surface edges capped at 256 m and the
    /// quartic grid at coarse LODs the fine vertices sit within 2 m of the
    /// emitted edge (up to 0.6 m radially mid-edge, the chord of one LOD-9
    /// voxel, and up to 2 m along it, the warp's own wiggle over 16 km
    /// that five corners cannot follow; 0.05 px at the 41 km the default
    /// LOD factor draws LOD 9 from). (At LOD 9 the sea is the opaque
    /// deep-ocean voxel, so the cap has to hold for opaque quads on the
    /// sea-level plane as much as for water.)
    #[test]
    fn emitted_sea_edges_meet_across_an_lod_boundary() {
        let (g, registry) = generator();
        let planet = g.planet;
        let sea_k = planet.sea_level as f64;
        let per_edge = planet.chunks_per_edge(9);
        // Edge quads: the coarse chunk's +x side (x = 32) and the fine chunk's -x side (x = 0), sea-level top faces of any block.
        let top_at = |m: &MeshData, x_side: u32| -> Vec<(f64, f64)> {
            m.opaque
                .iter()
                .copied()
                .chain(m.water.iter().map(|w| [w[0], w[1]]))
                .map(|q| unpack(&q))
                .filter(|&(axis, sign, slice, u0, _, w, _)| axis == 2 && sign == 1 && (slice + 1) as f64 * (1u64 << m.key.lod) as f64 == sea_k && (if x_side == 0 { u0 == 0 } else { u0 + w == 32 }))
                .map(|(_, _, _, _, v0, _, h)| (v0 as f64, (v0 + h) as f64))
                .collect()
        };
        let mut compared = 0usize;
        let mut worst = 0.0f64;
        let mut worst_radial = 0.0f64;
        let mut pairs = 0usize;
        'faces: for face in 0..6u8 {
            for cy in 0..per_edge {
                for cx in 0..per_edge - 1 {
                    let coarse = ChunkKey::neisor(face, 9, cx, cy, 0);
                    let fine = ChunkKey::neisor(face, 8, 2 * (cx + 1), 2 * cy, 0);
                    let mc = mesh_chunk(&g.generate(coarse), &registry, &planet, MeshOptions::default());
                    let ce = top_at(&mc, 32);
                    if ce.is_empty() {
                        continue;
                    }
                    let mf = mesh_chunk(&g.generate(fine), &registry, &planet, MeshOptions::default());
                    let fe = top_at(&mf, 0);
                    if fe.is_empty() {
                        continue;
                    }
                    pairs += 1;
                    let (vc, vf) = (planet.voxel_m(9), planet.voxel_m(8));
                    // Fine vertices on the shared line, in coarse voxels along it (the fine chunk covers the first half).
                    let mut fine_vs: Vec<f64> = fe.iter().flat_map(|&(a, b)| [a, b]).map(|v| v * 0.5).collect();
                    fine_vs.sort_by(|a, b| a.total_cmp(b));
                    fine_vs.dedup();
                    println!("{coarse:?}: {} edge quads; {fine:?}: {} edge vertices", ce.len(), fine_vs.len());
                    for &(a, b) in &ce {
                        let pa = vertex_pos(&mc.grid, mc.origin, DVec3::new(32.0, a, sea_k / 512.0), vc, true);
                        let pb = vertex_pos(&mc.grid, mc.origin, DVec3::new(32.0, b, sea_k / 512.0), vc, true);
                        for &v in fine_vs.iter().filter(|&&v| v >= a && v <= b) {
                            let pf = vertex_pos(&mf.grid, mf.origin, DVec3::new(0.0, v * 2.0, sea_k / 256.0), vf, true);
                            // The coarse edge at this point: the chord between its two emitted vertices.
                            let t = if b > a { (v - a) / (b - a) } else { 0.0 };
                            let chord = pa + (pb - pa) * t;
                            let sep = (pf - chord).length();
                            if sep > 3.0 {
                                println!("  coarse edge {a}..{b} at v {v}: radial {:.3} m, total {sep:.3} m", pf.length() - chord.length());
                            }
                            worst = worst.max(sep);
                            worst_radial = worst_radial.max((pf.length() - chord.length()).abs());
                            compared += 1;
                        }
                    }
                    if pairs >= 3 {
                        break 'faces;
                    }
                }
            }
        }
        println!("{pairs} L9/L8 boundaries, {compared} fine vertices against emitted coarse edges: worst separation {worst:.3} m (radial {worst_radial:.3} m)");
        assert!(pairs >= 1 && compared >= 8, "found sea on both sides of a boundary ({pairs} pairs, {compared} comparisons)");
        assert!(worst < 3.0, "fine vertices sit on the coarse emitted edge: {worst:.3} m");
        assert!(worst_radial < 0.7, "no chord sags more than one LOD-9 voxel's: {worst_radial:.3} m");
        // The cap in voxels per LOD: 1 at LOD 9 and 8, doubling down to the grid cell at LOD 5, the whole chunk at LOD 2 and below.
        assert_eq!([9u8, 8, 7, 6, 5, 4, 3, 2, 0].map(|l| sea_edge_voxels(&planet, l)), [1, 1, 2, 4, 8, 16, 32, 32, 32]);
    }

    /// Every water top face of a mesh, by cell: (x, y, slice) -> the packed
    /// corner heights. Greedy quads merge only equal heights, so each cell
    /// of a quad has the quad's.
    fn water_tops(m: &MeshData) -> std::collections::HashMap<(u32, u32, u32), u32> {
        let mut out = std::collections::HashMap::new();
        for q in &m.water {
            let (axis, sign, slice, u0, v0, w, h) = unpack(&[q[0], q[1]]);
            if axis == 2 && sign == 1 {
                for x in u0..u0 + w {
                    for y in v0..v0 + h {
                        assert!(out.insert((x, y, slice), q[2]).is_none(), "two top faces at {x} {y} {slice}");
                    }
                }
            }
        }
        out
    }

    fn mesh_of(w: &crate::world::World, key: ChunkKey) -> MeshData {
        let made = w.generate(key);
        let levels = w.padded_levels(key, &made);
        mesh_chunk_levels(&made, levels.as_deref(), &w.registry, &w.planet, MeshOptions::default())
    }

    /// Andrew's #1, recording 040850: stones laid over a river. The cells
    /// under them hold level 115, half a block below the stone, and their
    /// top faces were culled because the cell above was solid, so the
    /// surface had a hole you looked straight through. A top face hides
    /// under a solid only when it lies in the solid's underside: there it
    /// is invisible from above and would z-fight from below.
    #[test]
    fn water_top_shows_under_a_solid_unless_it_touches_it() {
        let w = crate::world::tests::test_world();
        let stone = w.registry.id("stone").unwrap();
        let water = w.water_id();
        // High in the air, so nothing but these cells is in the chunk.
        let key = ChunkKey::neisor(2, 0, 3000, 1500, 30);
        let (i0, j0, k0) = key.origin_blocks();
        let at = |x: u32, y: u32, z: u32| BlockPos { face: 2, i: i0 as u32 + x, j: j0 as u32 + y, k: k0 as u32 + z };
        // A stone basin 3 x 3 cells inside, one deep, and a second pool so the chunk always has partial water.
        for x in 7..=11 {
            for y in 7..=11 {
                w.set_block(at(x, y, 9), stone);
                if x == 7 || x == 11 || y == 7 || y == 11 {
                    w.set_block(at(x, y, 10), stone);
                }
            }
        }
        for (x, y) in [(20, 20), (20, 21), (21, 20), (21, 21)] {
            w.set_block(at(x, y, 9), stone);
            w.set_water(at(x, y, 10), water, 60);
        }
        let fill = |level: u8| {
            for x in 8..=10 {
                for y in 8..=10 {
                    w.set_water(at(x, y, 10), water, level);
                }
            }
        };
        w.set_block(at(9, 9, 11), stone); // the one stone over the middle cell

        // Level 115: half a block of air under the stone, so the surface goes on under it.
        fill(115);
        let tops = water_tops(&mesh_of(&w, key));
        assert_eq!(tops.get(&(9, 9, 10)), Some(&0x7373_7373), "the cell under the stone has its surface, at its level");
        assert_eq!(tops.get(&(8, 8, 10)), Some(&0x7373_7373));

        // Full, with the same stone: the top would lie in the stone's underside.
        fill(255);
        let tops = water_tops(&mesh_of(&w, key));
        assert_eq!(tops.get(&(9, 9, 10)), None, "a full cell's top is coplanar with the solid on it: not drawn");
        assert_eq!(tops.get(&(8, 9, 10)), Some(&WATER_FLAT), "its uncovered neighbours keep theirs");

        // Full under the stone beside lower water: its corners are pulled down, so there is air
        // over them, and a top face has to close the gap.
        w.set_water(at(9, 8, 10), water, 100);
        let tops = water_tops(&mesh_of(&w, key));
        let h = *tops.get(&(9, 9, 10)).expect("a sloped surface under a solid shows");
        assert_ne!(h, WATER_FLAT);
        assert_eq!(h >> 16 & 0xFFFF, 0xFFFF, "and it still meets the stone on the far side");

        // A coarse LOD has no levels: its water is full, as it always was.
        assert!(mesh_chunk(&w.generate(key), &w.registry, &w.planet, MeshOptions::default()).water.iter().all(|q| q[2] == WATER_FLAT));
    }
}

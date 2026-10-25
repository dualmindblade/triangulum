//! Binary greedy meshing of a padded chunk into 8-byte quads.
//!
//! Quad layout: lo = axis:2 | sign:1 | slice:5 | u:5 | v:5 | w-1:5 | h-1:5;
//! hi = block id:16 | ao:8 (four 2-bit corner values, corner c = cu + 2 cv).
//! Opaque quads are emitted where a solid voxel meets a non-solid one; water
//! quads where a liquid voxel meets air. Quads never cross a curvature-grid
//! cell in the tangent directions, so the bilinear warp stays crack-free.
//!
//! Grid record: 25 offsets (vec4, xyz = corner position at the chunk's
//! lattice floor relative to the chunk origin) then 25 unit directions
//! (vec4). `off[0].w` carries `r0`, the radius of the lattice floor, so a
//! vertex shader can place vertices at their exact radius `r0 + z` instead
//! of on the bilinear chord (review #11: at LOD 9 the chord sags 23 m below
//! the sphere mid-cell, so coarse and fine water surfaces did not meet).

use crate::worldgen::{Generated, PAD, pidx};
use crate::grid::{CHUNK, ChunkKey, Planet};
use crate::world::{BlockId, Registry};
use glam::DVec3;
use std::time::Instant;

/// Curvature grid: 4x4 cells (5x5 corners) per chunk footprint.
pub const GRID_CELLS: usize = 4;
pub const GRID_CORNERS: usize = GRID_CELLS + 1;
/// f32s per grid record: 25 offsets (vec4) + 25 directions (vec4).
pub const GRID_F32S: usize = GRID_CORNERS * GRID_CORNERS * 4 * 2;
pub const GRID_BYTES: usize = GRID_F32S * 4;
/// Index of `r0` (the lattice floor's radius) in the grid record.
pub const GRID_R0_INDEX: usize = 3;

pub type Quad = [u32; 2];

pub struct MeshData {
    pub key: ChunkKey,
    pub opaque: Vec<Quad>,
    pub water: Vec<Quad>,
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
}

pub fn mesh_chunk(generated: &Generated, registry: &Registry, planet: &Planet, opt: MeshOptions) -> MeshData {
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
    if any_solid {
        let blocking: Vec<u64> = masks.solid.clone();
        mesh_set(&masks.solid, &blocking, ids, &masks, opt, true, &mut opaque);
    }
    if any_liquid {
        let blocking: Vec<u64> = masks.solid.iter().zip(&masks.liquid).map(|(s, l)| s | l).collect();
        mesh_set(&masks.liquid, &blocking, ids, &masks, MeshOptions { split: opt.split, ao: false }, false, &mut water);
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

/// Emit greedy quads for every face of `occ` voxels that borders a non-`blocking` voxel.
fn mesh_set(occ: &[u64], blocking: &[u64], ids: &[BlockId], masks: &Masks, opt: MeshOptions, with_ao: bool, out: &mut Vec<Quad>) {
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
            let mut up = o & !(b >> 1) & INNER;
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
        // Tangent directions are split at grid cells; the radial one is linear.
        let split_u = opt.split;
        let split_v = opt.split && axis == 2;
        // Merge key: block id and the four corner AO values.
        let face_key = |u: usize, v: usize, slice: usize| -> u32 {
            let (x, y, z) = voxel_of(axis, slice, u, v);
            let id = ids[pidx(x, y, z)] as u32;
            if !(with_ao && opt.ao) {
                return id | (0xFF << 16);
            }
            id | (ao_bits(masks, axis, sign, x, y, z) << 16)
        };
        let plane = &mut planes[dirset as usize];
        for slice in 0..CHUNK {
            for v in 0..CHUNK {
                let vlim = if split_v { (v / cell + 1) * cell } else { CHUNK };
                loop {
                    let row = plane[slice * CHUNK + v];
                    if row == 0 {
                        break;
                    }
                    let u0 = row.trailing_zeros() as usize;
                    let ulim = if split_u { (u0 / cell + 1) * cell } else { CHUNK };
                    let k = face_key(u0, v, slice);
                    let mut w = 1usize;
                    while u0 + w < ulim && (row >> (u0 + w)) & 1 == 1 && face_key(u0 + w, v, slice) == k {
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
                            if face_key(u, v + h, slice) != k {
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
                    out.push([lo, k]);
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
            for q in m.opaque.iter().chain(&m.water) {
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
    /// GPU gets: bilinear offset and direction over the 4x4 cells.
    /// `exact` adds the radial correction `r0 (1 - |m|)` along the
    /// normalised direction (review #11), which `grid[GRID_R0_INDEX]` is for.
    fn vertex_pos(grid: &[f32], origin: DVec3, lp: DVec3, voxel: f64, exact: bool) -> DVec3 {
        let g = lp.truncate() * 0.125;
        let gi = (g.x.floor().min(3.0) as usize, g.y.floor().min(3.0) as usize);
        let gf = (g.x - gi.0 as f64, g.y - gi.1 as f64);
        let b = gi.1 * 5 + gi.0;
        let off = |c: usize| DVec3::new(grid[c * 4] as f64, grid[c * 4 + 1] as f64, grid[c * 4 + 2] as f64);
        let dir = |c: usize| {
            let base = (GRID_CORNERS * GRID_CORNERS + c) * 4;
            DVec3::new(grid[base] as f64, grid[base + 1] as f64, grid[base + 2] as f64)
        };
        let mix = |a: DVec3, b: DVec3, t: f64| a + (b - a) * t;
        let o = mix(mix(off(b), off(b + 1), gf.0), mix(off(b + 5), off(b + 6), gf.0), gf.1);
        let m = mix(mix(dir(b), dir(b + 1), gf.0), mix(dir(b + 5), dir(b + 6), gf.0), gf.1);
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
}

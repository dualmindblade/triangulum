//! Planet parameters and the block lattice.
//!
//! A block is (face, i, j, k): columns i, j in [0, N) on a cube face, radial
//! layer k in [0, H). Layer k = sea_level sits at radius R; the bottom of the
//! world (k = 0) is R - sea_level * s. Every direction <-> lattice conversion
//! goes through the `cubesphere` crate (EqualAreaCobe, see DESIGN.md).
//!
//! Chunks are 32^3 voxels at every LOD. A voxel at LOD L is 2^L blocks, so a
//! chunk at LOD L spans 32 * 2^L blocks on every axis and tiles the face
//! exactly because N and H are powers of two multiples of 32.

use cubesphere::{self as cs, Projection};
use glam::DVec3;
use serde::{Deserialize, Serialize};

/// Voxels per chunk edge.
pub const CHUNK: usize = 32;
pub const CHUNK_U32: u32 = CHUNK as u32;

/// A cheap multiplicative hasher (Fx-style) for the chunk maps the streamer
/// rebuilds every frame; SipHash is several times slower on these small keys.
#[derive(Default, Clone, Copy)]
pub struct FastHasher(u64);

impl std::hash::Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        let mut h = self.0;
        h ^= h >> 32;
        h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^ (h >> 29)
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    #[inline]
    fn write_u8(&mut self, v: u8) {
        self.write_u64(v as u64);
    }
    #[inline]
    fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64);
    }
    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517C_C1B7_2722_0A95);
    }
}

pub type FastBuild = std::hash::BuildHasherDefault<FastHasher>;
pub type FastMap<K, V> = std::collections::HashMap<K, V, FastBuild>;
pub type FastSet<K> = std::collections::HashSet<K, FastBuild>;

/// World configuration file (`game/assets/world.json`), with CLI overrides
/// applied on top by `main`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldConfig {
    pub name: String,
    pub seed: u64,
    pub block_size_m: f64,
    pub world_height: u32,
    pub sea_level: u32,
    pub columns_per_face: u32,
    pub bake: String,
    /// None = R / 6,371,000 m (Earth slopes, ~1/70).
    pub vertical_scale: Option<f64>,
    /// Amplitude of the engine's fine relief (metre to ~50 m texture).
    pub roughness: f64,
    /// Strength of the engine's landforms between the bake's scale and
    /// ~50 m: ridges, valley networks, escarpments, mesas (0 = none).
    #[serde(default = "one")]
    pub landform: f64,
}

fn one() -> f64 {
    1.0
}

impl WorldConfig {
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Planet {
    /// Block size s in metres.
    pub block_size: f64,
    /// World height H in blocks (power of two).
    pub height: u32,
    /// Sea level in blocks above the bottom.
    pub sea_level: u32,
    /// Columns per face edge, N.
    pub n: u32,
    /// Sea-level radius in metres: R = s * N * sqrt(6 / 4 pi) (equal-area convention).
    pub radius: f64,
    /// Coarsest LOD in use (voxel = 2^max_lod blocks).
    pub max_lod: u8,
    pub proj: Projection,
}

impl Planet {
    pub fn new(block_size: f64, height: u32, sea_level: u32, n: u32) -> Self {
        assert!(height.is_power_of_two(), "world height H = {height} must be a power of two");
        assert!(n % height == 0, "N = {n} must be a multiple of H = {height}");
        assert!(height >= CHUNK_U32 && n >= CHUNK_U32, "H and N must be at least one chunk");
        assert!(sea_level > 0 && sea_level < height, "sea level {sea_level} must lie inside (0, H = {height})");
        assert!(block_size > 0.0);
        let radius = block_size * n as f64 * (6.0 / (4.0 * std::f64::consts::PI)).sqrt();
        // The coarsest voxel is H blocks (the cap). A voxel grid can only
        // place sea level exactly when the voxel size divides the sea level,
        // so LODs coarser than the largest power of two dividing sea_level
        // would show the sea either a voxel too high or not at all; they are
        // left out. With the defaults (H 2048, sea 512) this is LOD 9 (512 m).
        let max_lod = (height.trailing_zeros().min(sea_level.trailing_zeros())) as u8;
        Self { block_size, height, sea_level, n, radius, max_lod, proj: Projection::EqualAreaCobe }
    }

    /// Radius of the bottom of the world (layer 0).
    #[inline]
    pub fn bottom_radius(&self) -> f64 {
        self.radius - self.sea_level as f64 * self.block_size
    }

    /// Radius at continuous layer k.
    #[inline]
    pub fn radius_at(&self, k: f64) -> f64 {
        self.bottom_radius() + k * self.block_size
    }

    /// Voxel edge in metres at LOD `lod`.
    #[inline]
    pub fn voxel_m(&self, lod: u8) -> f64 {
        self.block_size * (1u64 << lod) as f64
    }

    /// Chunks per face edge at `lod` (at least 1: a chunk may stick out of the face).
    #[inline]
    pub fn chunks_per_edge(&self, lod: u8) -> u32 {
        (self.n / (CHUNK_U32 << lod)).max(1)
    }

    /// Chunks per column at `lod` (at least 1: a chunk may stick out of the world height).
    #[inline]
    pub fn chunks_per_column(&self, lod: u8) -> u32 {
        (self.height / (CHUNK_U32 << lod)).max(1)
    }

    /// Lattice square coordinate of continuous column index i (0..N).
    #[inline]
    pub fn p_of(&self, i: f64) -> f64 {
        i * 2.0 / self.n as f64 - 1.0
    }

    #[inline]
    pub fn i_of(&self, p: f64) -> f64 {
        (p + 1.0) * 0.5 * self.n as f64
    }

    /// Unit direction of continuous lattice point (i, j) on `face`. Indices a
    /// little outside [0, N] are allowed: they are snapped onto the
    /// neighbouring face's own lattice so chunk borders sample the same
    /// column centres their neighbours do.
    pub fn dir(&self, face: u8, i: f64, j: f64) -> DVec3 {
        let n = self.n as f64;
        if (0.0..=n).contains(&i) && (0.0..=n).contains(&j) {
            return cs::face_to_dir(self.proj, face, self.p_of(i), self.p_of(j));
        }
        // Past the face edge: `warp` is only defined on the face, so extend
        // linearly in gnomonic space from the edge (the gnomonic plane
        // continues onto the neighbour), then snap to that face's lattice.
        let (p, q) = (self.p_of(i), self.p_of(j));
        let (pc, qc) = (p.clamp(-1.0, 1.0), q.clamp(-1.0, 1.0));
        let eps = 1e-4;
        let (a0, b0) = cs::warp(self.proj, pc, qc);
        let (a1, _) = cs::warp(self.proj, pc - eps * pc.signum().max(0.0) + eps * (-pc.signum()).max(0.0), qc);
        let (_, b1) = cs::warp(self.proj, pc, qc - eps * qc.signum().max(0.0) + eps * (-qc.signum()).max(0.0));
        let a = a0 + (a0 - a1) / eps * (p - pc).abs() * pc.signum();
        let b = b0 + (b0 - b1) / eps * (q - qc).abs() * qc.signum();
        let (axis, right, up) = cs::FACES[face as usize];
        let d = (axis + a * right + b * up).normalize();
        let (f2, p2, q2) = cs::dir_to_face(self.proj, d);
        if f2 == face {
            return d;
        }
        // Snap to the same fractional offset within a column (centres stay centres).
        let fi = i.fract().abs();
        let fj = j.fract().abs();
        let snap = |x: f64, fr: f64| (self.i_of(x) - fr).round() + fr;
        cs::face_to_dir(self.proj, f2, self.p_of(snap(p2, fi)), self.p_of(snap(q2, fj)))
    }

    /// Continuous lattice coordinates (face, i, j) of a direction.
    pub fn lattice_of_dir(&self, dir: DVec3) -> (u8, f64, f64) {
        let (f, p, q) = cs::dir_to_face(self.proj, dir);
        (f, self.i_of(p), self.i_of(q))
    }

    /// World position of lattice point (face, i, j, k), planet-centred metres.
    pub fn pos(&self, face: u8, i: f64, j: f64, k: f64) -> DVec3 {
        self.dir(face, i, j) * self.radius_at(k)
    }

    /// Inverse of `pos`.
    pub fn lattice_of_pos(&self, p: DVec3) -> (u8, f64, f64, f64) {
        let r = p.length();
        let (f, i, j) = self.lattice_of_dir(p / r);
        (f, i, j, (r - self.bottom_radius()) / self.block_size)
    }

    /// Block containing a world position (clamped into the world).
    pub fn block_of_pos(&self, p: DVec3) -> BlockPos {
        let (f, i, j, k) = self.lattice_of_pos(p);
        let n = self.n as f64;
        BlockPos {
            face: f,
            i: i.floor().clamp(0.0, n - 1.0) as u32,
            j: j.floor().clamp(0.0, n - 1.0) as u32,
            k: k.floor().clamp(0.0, self.height as f64 - 1.0) as u32,
        }
    }

    /// Geodetic-style latitude/longitude in degrees for a direction (+Z is north).
    pub fn lat_lon_deg(dir: DVec3) -> (f64, f64) {
        let d = dir.normalize();
        (d.z.clamp(-1.0, 1.0).asin().to_degrees(), d.y.atan2(d.x).to_degrees())
    }

    pub fn dir_of_lat_lon_deg(lat: f64, lon: f64) -> DVec3 {
        let (la, lo) = (lat.to_radians(), lon.to_radians());
        DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin())
    }
}

/// One block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockPos {
    pub face: u8,
    pub i: u32,
    pub j: u32,
    pub k: u32,
}

/// A 32^3 chunk at LOD `lod` (voxel = 2^lod blocks). cx, cy index the
/// footprint on the face, cz the vertical slab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkKey {
    pub face: u8,
    pub lod: u8,
    pub cx: u32,
    pub cy: u32,
    pub cz: u32,
}

/// A chunk column footprint: every chunk with the same (face, lod, cx, cy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FootprintKey {
    pub face: u8,
    pub lod: u8,
    pub cx: u32,
    pub cy: u32,
}

impl ChunkKey {
    /// Blocks per voxel.
    #[inline]
    pub fn voxel_blocks(&self) -> u64 {
        1u64 << self.lod
    }

    /// Blocks per chunk edge.
    #[inline]
    pub fn extent_blocks(&self) -> u64 {
        (CHUNK as u64) << self.lod
    }

    /// Lattice corner (i0, j0, k0) in blocks.
    #[inline]
    pub fn origin_blocks(&self) -> (u64, u64, u64) {
        let e = self.extent_blocks();
        (self.cx as u64 * e, self.cy as u64 * e, self.cz as u64 * e)
    }

    #[inline]
    pub fn footprint(&self) -> FootprintKey {
        FootprintKey { face: self.face, lod: self.lod, cx: self.cx, cy: self.cy }
    }

    pub fn parent(&self) -> Option<ChunkKey> {
        (self.lod < 63).then(|| ChunkKey {
            face: self.face,
            lod: self.lod + 1,
            cx: self.cx / 2,
            cy: self.cy / 2,
            cz: self.cz / 2,
        })
    }

    /// The eight children at the next finer LOD (lod must be > 0).
    pub fn children(&self) -> [ChunkKey; 8] {
        let (f, l) = (self.face, self.lod - 1);
        let (x, y, z) = (self.cx * 2, self.cy * 2, self.cz * 2);
        let mut out = [ChunkKey { face: f, lod: l, cx: x, cy: y, cz: z }; 8];
        for (n, c) in out.iter_mut().enumerate() {
            c.cx = x + (n as u32 & 1);
            c.cy = y + ((n as u32 >> 1) & 1);
            c.cz = z + ((n as u32 >> 2) & 1);
        }
        out
    }
}

impl FootprintKey {
    #[inline]
    pub fn extent_blocks(&self) -> u64 {
        (CHUNK as u64) << self.lod
    }
    pub fn children(&self) -> [FootprintKey; 4] {
        let (f, l, x, y) = (self.face, self.lod - 1, self.cx * 2, self.cy * 2);
        [
            FootprintKey { face: f, lod: l, cx: x, cy: y },
            FootprintKey { face: f, lod: l, cx: x + 1, cy: y },
            FootprintKey { face: f, lod: l, cx: x, cy: y + 1 },
            FootprintKey { face: f, lod: l, cx: x + 1, cy: y + 1 },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planet() -> Planet {
        Planet::new(1.0, 2048, 512, 131072)
    }

    #[test]
    fn radius_follows_the_equal_area_convention() {
        let p = planet();
        // R = 131072 * sqrt(6 / 4 pi) = 90,569.2 m (DESIGN.md rounds it to 90.57 km).
        assert!((p.radius - 90_569.2).abs() < 1.0, "R = {}", p.radius);
        assert_eq!(p.max_lod, 9);
        assert_eq!(Planet::new(1.0, 512, 256, 131072).max_lod, 8);
        assert_eq!(Planet::new(0.5, 2048, 512, 262144).radius, p.radius);
    }

    #[test]
    #[should_panic]
    fn n_must_divide_by_h() {
        Planet::new(1.0, 2048, 512, 131072 + 32);
    }

    #[test]
    fn lattice_round_trip() {
        let g = planet();
        for &(f, i, j) in &[(0u8, 100.5, 7.5), (3, 70000.0, 131000.0), (5, 5.0, 131070.0)] {
            let p = g.pos(f, i, j, 600.0);
            let (f2, i2, j2, k2) = g.lattice_of_pos(p);
            assert_eq!(f, f2);
            assert!((i - i2).abs() < 1e-5 && (j - j2).abs() < 1e-5 && (k2 - 600.0).abs() < 1e-5);
        }
        let b = g.block_of_pos(g.pos(2, 10.5, 20.5, 512.5));
        assert_eq!(b, BlockPos { face: 2, i: 10, j: 20, k: 512 });
    }

    #[test]
    fn border_columns_snap_onto_the_neighbour_face() {
        let g = Planet::new(1.0, 512, 256, 4096);
        // column centre one past the i = N edge of face 0
        let d = g.dir(0, 4096.5, 300.5);
        let (f, i, j) = g.lattice_of_dir(d);
        assert_ne!(f, 0);
        let near_centre = |x: f64| (x - x.floor() - 0.5).abs() < 1e-6;
        assert!(near_centre(i) && near_centre(j), "i {i} j {j}");
    }

    #[test]
    fn coarse_voxel_covers_exactly_2_to_the_l_fine_columns() {
        // LOD alignment: voxel x of chunk (lod L, cx) covers lattice columns
        // [i0 + x 2^L, i0 + (x+1) 2^L), and the eight children tile the parent.
        for lod in 1..=9u8 {
            let k = ChunkKey { face: 2, lod, cx: 5, cy: 3, cz: 1 };
            let (i0, j0, k0) = k.origin_blocks();
            assert_eq!(i0, 5 * 32 << lod);
            assert_eq!(k.voxel_blocks(), 1 << lod);
            let mut covered = std::collections::HashSet::new();
            for c in k.children() {
                assert_eq!(c.parent(), Some(k));
                let (ci, cj, ck) = c.origin_blocks();
                assert!(ci >= i0 && ci + c.extent_blocks() <= i0 + k.extent_blocks());
                assert!(cj >= j0 && cj + c.extent_blocks() <= j0 + k.extent_blocks());
                assert!(ck >= k0 && ck + c.extent_blocks() <= k0 + k.extent_blocks());
                covered.insert((ci, cj, ck));
            }
            assert_eq!(covered.len(), 8);
            // voxel 7 of this chunk spans exactly 2^L fine columns
            let vox_i0 = i0 + 7 * k.voxel_blocks();
            let fine: Vec<u64> = (vox_i0..vox_i0 + k.voxel_blocks()).collect();
            assert_eq!(fine.len() as u64, 1u64 << lod);
            assert_eq!(fine[0] >> lod << lod, fine[0]);
        }
    }

    #[test]
    fn chunk_counts_tile_the_world() {
        let p = planet();
        assert_eq!(p.chunks_per_edge(0), 4096);
        assert_eq!(p.chunks_per_column(0), 64);
        assert_eq!(p.chunks_per_column(9), 1); // sticks out of H and is clipped
        assert_eq!(p.chunks_per_edge(9), 8);
    }

    #[test]
    fn lat_lon_round_trip() {
        for &(lat, lon) in &[(0.0, 0.0), (45.0, 90.0), (-60.0, -120.0), (89.0, 10.0)] {
            let d = Planet::dir_of_lat_lon_deg(lat, lon);
            let (la, lo) = Planet::lat_lon_deg(d);
            assert!((la - lat).abs() < 1e-9 && (lo - lon).abs() < 1e-9);
        }
    }
}

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
    pub roughness: f64,
}

impl WorldConfig {
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))
    }
}

/// One side of a face: 0 = i < 0, 1 = i > N, 2 = j < 0, 3 = j > N.
pub type Edge = u8;

/// How a face's side continues onto its neighbour (derived from `FACES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdgeMap {
    /// The neighbouring face.
    pub face: u8,
    /// Which of the neighbour's sides is the shared edge.
    pub edge: Edge,
    /// Whether the coordinate along the edge runs the other way (u' = N - u).
    pub flip: bool,
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
    /// `edges[face][side]`: the neighbour across each side.
    pub edges: [[EdgeMap; 4]; 6],
}

/// Which `FACES` entry has this axis.
fn face_with_axis(axis: DVec3) -> u8 {
    cs::FACES.iter().position(|(a, _, _)| a.distance(axis) < 1e-9).expect("every cube axis is a face") as u8
}

/// The side of face `g` whose outward direction is `v`, if any.
fn side_toward(g: u8, v: DVec3) -> Option<Edge> {
    let (_, right, up) = cs::FACES[g as usize];
    [(-right, 0u8), (right, 1), (-up, 2), (up, 3)].into_iter().find(|(d, _)| d.distance(v) < 1e-9).map(|(_, e)| e)
}

/// The face-edge adjacency of the cube, from the frames alone. Crossing
/// side `e` of face `f` lands on `map.face`, entering through `map.edge`;
/// the coordinate along the shared edge is kept or reversed (`flip`).
pub fn edge_maps() -> [[EdgeMap; 4]; 6] {
    let mut out = [[EdgeMap { face: 0, edge: 0, flip: false }; 4]; 6];
    for f in 0..6u8 {
        let (axis, right, up) = cs::FACES[f as usize];
        for e in 0..4u8 {
            let outward = match e {
                0 => -right,
                1 => right,
                2 => -up,
                _ => up,
            };
            let g = face_with_axis(outward);
            // On g, f's own axis points back toward f: that side of g is the shared edge.
            let edge = side_toward(g, axis).expect("neighbour side");
            let par_f = if e < 2 { up } else { right };
            let (_, gr, gu) = cs::FACES[g as usize];
            let par_g = if edge < 2 { gu } else { gr };
            let flip = if par_g.distance(par_f) < 1e-9 {
                false
            } else if par_g.distance(-par_f) < 1e-9 {
                true
            } else {
                panic!("edge-parallel axes of faces {f} and {g} are not aligned")
            };
            out[f as usize][e as usize] = EdgeMap { face: g, edge, flip };
        }
    }
    out
}

impl Planet {
    /// Panicking constructor for tests and callers with known-good parameters.
    pub fn new(block_size: f64, height: u32, sea_level: u32, n: u32) -> Self {
        Self::try_new(block_size, height, sea_level, n).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Validate the world parameters. Unsupported configurations are
    /// rejected here rather than hanging or leaving strips of the planet
    /// unvisited later.
    pub fn try_new(block_size: f64, height: u32, sea_level: u32, n: u32) -> Result<Self, String> {
        if !(block_size > 0.0) || !block_size.is_finite() {
            return Err(format!("block size {block_size} m must be positive"));
        }
        if !height.is_power_of_two() {
            return Err(format!("world height H = {height} must be a power of two"));
        }
        if height < CHUNK_U32 || n < CHUNK_U32 {
            return Err(format!("H = {height} and N = {n} must be at least one chunk ({CHUNK} blocks)"));
        }
        if n % height != 0 {
            return Err(format!("N = {n} must be a multiple of H = {height}"));
        }
        if sea_level == 0 || sea_level >= height {
            return Err(format!("sea level {sea_level} must lie inside (0, H = {height})"));
        }
        // The coarsest voxel is H blocks (the cap). A voxel grid can only
        // place sea level exactly when the voxel size divides the sea level,
        // so LODs coarser than the largest power of two dividing sea_level
        // would show the sea either a voxel too high or not at all; they are
        // left out. With the defaults (H 2048, sea 512) this is LOD 9 (512 m).
        // A sea level like 513 would leave only LOD 0 and billions of root
        // chunks, so the sea level must be a multiple of H / 8 at least.
        let sea_lod = (height.trailing_zeros().min(sea_level.trailing_zeros())) as u8;
        let min_lod = (height.trailing_zeros() as u8).saturating_sub(3);
        if sea_lod < min_lod {
            return Err(format!(
                "sea level {sea_level} must be a multiple of {} (H / 8) so the coarsest LOD stays at least {} blocks; {} leaves LOD {sea_lod}",
                height / 8,
                1u32 << min_lod,
                sea_level
            ));
        }
        // The coarsest chunk never exceeds the face (small test planets), and
        // coverage must be exact: root chunks tile the face only when N is a
        // multiple of the coarsest chunk edge.
        let face_lod = (n / CHUNK_U32).ilog2() as u8;
        let max_lod = sea_lod.min(face_lod);
        let coarse_chunk = CHUNK_U32 << max_lod;
        if n % coarse_chunk != 0 {
            return Err(format!("N = {n} must be a multiple of the coarsest chunk edge {coarse_chunk} blocks (32 x 2^{max_lod}); otherwise strips of the face are never visited"));
        }
        let radius = block_size * n as f64 * (6.0 / (4.0 * std::f64::consts::PI)).sqrt();
        Ok(Self { block_size, height, sea_level, n, radius, max_lod, proj: Projection::EqualAreaCobe, edges: edge_maps() })
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

    /// Carry continuous lattice coordinates that lie past a face's sides onto
    /// the face that is really there. Pure index arithmetic on the cube's
    /// edge adjacency, so it holds at every voxel spacing: a column centre
    /// half a voxel past the edge lands on the neighbour's own centre half a
    /// voxel inside its edge, at any LOD.
    ///
    /// Corner policy: past two sides at once there is no face (three faces
    /// meet at a cube corner). The side with the larger overshoot is crossed
    /// first, then the remaining overshoot is crossed again from the
    /// neighbour, so the diagonal continues onto the third face; whatever is
    /// still outside after that is clamped to the face.
    pub fn wrap(&self, face: u8, i: f64, j: f64) -> (u8, f64, f64) {
        let n = self.n as f64;
        let (mut f, mut i, mut j) = (face, i, j);
        for _ in 0..2 {
            let over_i = if i < 0.0 { -i } else if i > n { i - n } else { 0.0 };
            let over_j = if j < 0.0 { -j } else if j > n { j - n } else { 0.0 };
            if over_i == 0.0 && over_j == 0.0 {
                return (f, i, j);
            }
            let (side, t, u) = if over_i >= over_j { (if i < 0.0 { 0 } else { 1 }, over_i, j) } else { (if j < 0.0 { 2 } else { 3 }, over_j, i) };
            let m = self.edges[f as usize][side];
            let u = if m.flip { n - u } else { u };
            // Entering through the neighbour's side `m.edge`, `t` deep.
            let (ni, nj) = match m.edge {
                0 => (t, u),
                1 => (n - t, u),
                2 => (u, t),
                _ => (u, n - t),
            };
            f = m.face;
            i = ni;
            j = nj;
        }
        (f, i.clamp(0.0, n), j.clamp(0.0, n))
    }

    /// `wrap` for cell indices: the cell containing the centre of (ci, cj).
    pub fn wrap_cell(&self, face: u8, ci: i64, cj: i64) -> (u8, u32, u32) {
        let n = self.n as i64;
        if (0..n).contains(&ci) && (0..n).contains(&cj) {
            return (face, ci as u32, cj as u32);
        }
        let (f, i, j) = self.wrap(face, ci as f64 + 0.5, cj as f64 + 0.5);
        let nf = self.n as f64;
        (f, i.floor().clamp(0.0, nf - 1.0) as u32, j.floor().clamp(0.0, nf - 1.0) as u32)
    }

    /// Unit direction of continuous lattice point (i, j) on `face`. Indices
    /// outside [0, N] continue onto the neighbouring face (`wrap`), so chunk
    /// borders sample exactly the column centres their neighbours do.
    pub fn dir(&self, face: u8, i: f64, j: f64) -> DVec3 {
        let (f, i, j) = self.wrap(face, i, j);
        cs::face_to_dir(self.proj, f, self.p_of(i), self.p_of(j))
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

/// The body a lattice belongs to. Neisor is 0; the moon and vehicle grids
/// get their own ids later, so a chunk key never means two places.
pub type BodyId = u8;
pub const BODY_NEISOR: BodyId = 0;

/// A 32^3 chunk at LOD `lod` (voxel = 2^lod blocks) of body `body`. cx, cy
/// index the footprint on the face, cz the vertical slab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkKey {
    pub body: BodyId,
    pub face: u8,
    pub lod: u8,
    pub cx: u32,
    pub cy: u32,
    pub cz: u32,
}

/// A chunk column footprint: every chunk with the same (body, face, lod, cx, cy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FootprintKey {
    pub body: BodyId,
    pub face: u8,
    pub lod: u8,
    pub cx: u32,
    pub cy: u32,
}

impl ChunkKey {
    /// A key on Neisor.
    #[inline]
    pub const fn neisor(face: u8, lod: u8, cx: u32, cy: u32, cz: u32) -> Self {
        Self { body: BODY_NEISOR, face, lod, cx, cy, cz }
    }

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
        FootprintKey { body: self.body, face: self.face, lod: self.lod, cx: self.cx, cy: self.cy }
    }

    pub fn parent(&self) -> Option<ChunkKey> {
        (self.lod < 63).then(|| ChunkKey { lod: self.lod + 1, cx: self.cx / 2, cy: self.cy / 2, cz: self.cz / 2, ..*self })
    }

    /// The eight children at the next finer LOD (lod must be > 0).
    pub fn children(&self) -> [ChunkKey; 8] {
        let (x, y, z) = (self.cx * 2, self.cy * 2, self.cz * 2);
        let mut out = [ChunkKey { lod: self.lod - 1, cx: x, cy: y, cz: z, ..*self }; 8];
        for (n, c) in out.iter_mut().enumerate() {
            c.cx = x + (n as u32 & 1);
            c.cy = y + ((n as u32 >> 1) & 1);
            c.cz = z + ((n as u32 >> 2) & 1);
        }
        out
    }

    /// Is `other` this chunk or one of its descendants (same body and face)?
    pub fn contains(&self, other: &ChunkKey) -> bool {
        if other.body != self.body || other.face != self.face || other.lod > self.lod {
            return false;
        }
        let d = self.lod - other.lod;
        other.cx >> d == self.cx && other.cy >> d == self.cy && other.cz >> d == self.cz
    }
}

impl FootprintKey {
    #[inline]
    pub fn extent_blocks(&self) -> u64 {
        (CHUNK as u64) << self.lod
    }
    pub fn children(&self) -> [FootprintKey; 4] {
        let (l, x, y) = (self.lod - 1, self.cx * 2, self.cy * 2);
        [
            FootprintKey { lod: l, cx: x, cy: y, ..*self },
            FootprintKey { lod: l, cx: x + 1, cy: y, ..*self },
            FootprintKey { lod: l, cx: x, cy: y + 1, ..*self },
            FootprintKey { lod: l, cx: x + 1, cy: y + 1, ..*self },
        ]
    }
}

/// A conservative bounding sphere of a chunk's rendered geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub centre: DVec3,
    pub radius: f64,
}

impl Planet {
    /// Bounding sphere of everything a chunk can draw. The renderer warps a
    /// chunk through a bilinear curvature grid whose corners are the exact
    /// lattice points at the chunk's radial floor plus `up * z`; every
    /// rendered point therefore lies in the convex hull of the grid corners
    /// at the two radial extremes (lattice floor k0 and k0 + 32 voxels), and
    /// a sphere containing those corners contains the hull. The corners
    /// farthest from the chunk's axis are the footprint's own four corners
    /// (the footprint is a convex spherical quad), so the radius is the
    /// largest distance from the mid-radius centre to those eight points,
    /// with a small margin for the grid's interior corners and f32 rounding.
    /// A chunk that sticks out of the world height is clipped to it (the
    /// mesher emits nothing above the top layer).
    pub fn chunk_bounds(&self, key: ChunkKey) -> Bounds {
        let e = key.extent_blocks() as f64;
        let (i0, j0, k0) = key.origin_blocks();
        let (i0, j0, k0) = (i0 as f64, j0 as f64, k0 as f64);
        let (r0, r1) = (self.radius_at(k0), self.radius_at((k0 + e).min(self.height as f64)));
        let centre = self.dir(key.face, i0 + e * 0.5, j0 + e * 0.5) * (0.5 * (r0 + r1));
        let mut r2 = 0.0f64;
        for (di, dj) in [(0.0, 0.0), (e, 0.0), (0.0, e), (e, e)] {
            let d = self.dir(key.face, i0 + di, j0 + dj);
            r2 = r2.max((d * r0 - centre).length_squared()).max((d * r1 - centre).length_squared());
        }
        Bounds { centre, radius: r2.sqrt() * 1.002 + 0.5 }
    }

    /// Express a lattice point of the neighbouring face `other` in `face`'s
    /// own continuous coordinates, past the shared side: the inverse of one
    /// step of `wrap`. None when the faces do not share a side.
    pub fn extended_coords(&self, face: u8, other: u8, i: f64, j: f64) -> Option<(f64, f64)> {
        let n = self.n as f64;
        let side = (0..4u8).find(|&e| self.edges[face as usize][e as usize].face == other)?;
        let m = self.edges[face as usize][side as usize];
        // Depth into `other` from the shared side and the coordinate along it.
        let (t, u) = match m.edge {
            0 => (i, j),
            1 => (n - i, j),
            2 => (j, i),
            _ => (n - j, i),
        };
        let u = if m.flip { n - u } else { u };
        Some(match side {
            0 => (-t, u),
            1 => (n + t, u),
            2 => (u, -t),
            _ => (u, n + t),
        })
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
    fn edge_maps_are_mutual_and_cover_the_cube() {
        let maps = edge_maps();
        for f in 0..6u8 {
            for e in 0..4u8 {
                let m = maps[f as usize][e as usize];
                assert_ne!(m.face, f);
                // The neighbour's map back through the shared edge returns here.
                let back = maps[m.face as usize][m.edge as usize];
                assert_eq!(back.face, f, "face {f} side {e} -> {m:?} -> {back:?}");
                assert_eq!(back.edge, e);
                assert_eq!(back.flip, m.flip);
            }
        }
        // Face 0 (+X) is ringed by +Y (2), -Y (3), +Z (4), -Z (5).
        assert_eq!(maps[0][1].face, 2);
        assert_eq!(maps[0][0].face, 3);
        assert_eq!(maps[0][3].face, 4);
        assert_eq!(maps[0][2].face, 5);
    }

    #[test]
    fn border_columns_continue_onto_the_neighbour_at_every_edge_and_lod() {
        let g = Planet::new(1.0, 512, 256, 4096);
        let n = g.n as f64;
        let near_centre = |x: f64, v: f64| ((x / v) - (x / v).floor() - 0.5).abs() < 1e-6;
        for face in 0..6u8 {
            for (i, j) in [(n + 0.5, 300.5), (-0.5, 300.5), (300.5, n + 0.5), (300.5, -0.5)] {
                // L0: half a column past the edge is the neighbour's first column centre...
                let d = g.dir(face, i, j);
                let (f, wi, wj) = g.lattice_of_dir(d);
                assert_ne!(f, face, "face {face} ({i}, {j}) stayed home");
                assert!(near_centre(wi, 1.0) && near_centre(wj, 1.0), "face {face} ({i}, {j}) -> {f} ({wi}, {wj})");
                let depth = wi.min(n - wi).min(wj).min(n - wj);
                assert!((depth - 0.5).abs() < 1e-6, "half a column inside the neighbour's edge, got {depth}");
                // ...and the step across the edge is the same size as a step inside it
                // (the old extrapolation turned back into the face here).
                let inside = |di: f64, dj: f64| g.dir(face, i.clamp(0.0, n) + di, j.clamp(0.0, n) + dj);
                let (si, sj) = (if i > n { -1.0 } else if i < 0.0 { 1.0 } else { 0.0 }, if j > n { -1.0 } else if j < 0.0 { 1.0 } else { 0.0 });
                let step_in = inside(0.5 * si, 0.5 * sj).distance(inside(1.5 * si, 1.5 * sj));
                let step_across = d.distance(inside(0.5 * si, 0.5 * sj));
                assert!((step_across / step_in - 1.0).abs() < 0.1, "face {face} ({i}, {j}): across {step_across} vs inside {step_in}");
            }
            // A coarse LOD (voxel 64 blocks): half a coarse voxel past the edge
            // is a coarse centre half a voxel inside the neighbour.
            let v = 64.0;
            let d = g.dir(face, n + v / 2.0, 7.0 * v + v / 2.0);
            let (f, wi, wj) = g.lattice_of_dir(d);
            assert_ne!(f, face);
            assert!(near_centre(wi, v) && near_centre(wj, v), "coarse: {f} ({wi}, {wj})");
            assert!((wi.min(n - wi).min(wj).min(n - wj) - v / 2.0).abs() < 1e-6);
            // Integer cells map the same way.
            let (f2, ci, cj) = g.wrap_cell(face, g.n as i64, 300);
            assert_eq!(f2, f);
            let (_, wi, wj) = g.lattice_of_dir(g.dir(face, n + 0.5, 300.5));
            assert_eq!((ci, cj), (wi.floor() as u32, wj.floor() as u32));
        }
        // The shared edge itself is one line: approaching it from both faces agrees.
        let m = g.edges[0][1];
        let a = g.dir(0, n, 100.0);
        let (bi, bj) = match m.edge {
            0 => (0.0, if m.flip { n - 100.0 } else { 100.0 }),
            1 => (n, if m.flip { n - 100.0 } else { 100.0 }),
            2 => (if m.flip { n - 100.0 } else { 100.0 }, 0.0),
            _ => (if m.flip { n - 100.0 } else { 100.0 }, n),
        };
        assert!(a.distance(g.dir(m.face, bi, bj)) < 1e-12);
    }

    #[test]
    fn corners_continue_onto_the_third_face() {
        let g = Planet::new(1.0, 512, 256, 4096);
        let n = g.n as f64;
        // Past one side only: the neighbour across that side.
        let (f1, _, _) = g.wrap(0, n + 0.5, n - 0.5);
        let (f2, _, _) = g.wrap(0, n - 0.5, n + 0.5);
        assert_eq!(f1, g.edges[0][1].face);
        assert_eq!(f2, g.edges[0][3].face);
        assert_ne!(f1, f2);
        // Past both: the third face at the corner, a valid cell close to it.
        let (f3, i, j) = g.wrap(0, n + 0.5, n + 1.5);
        assert!(f3 != 0 && (f3 == f1 || f3 == f2), "the other neighbour at the corner (only three faces meet there), got {f3}");
        assert!((0.0..=n).contains(&i) && (0.0..=n).contains(&j));
        let depth = i.min(n - i).min(j).min(n - j);
        assert!(depth <= 1.5 + 1e-9, "stays near the corner: depth {depth}");
        // The corner direction itself is where the three faces meet.
        let corner = g.dir(0, n, n);
        let (fc, _, _) = g.wrap(0, n + 1.0, n + 1.0);
        let d = g.dir(fc, 0.0, 0.0).distance(corner).min(g.dir(fc, n, 0.0).distance(corner)).min(g.dir(fc, 0.0, n).distance(corner)).min(g.dir(fc, n, n).distance(corner));
        assert!(d < 1e-12, "third face shares the corner");
        // Far past a side (several columns) is still monotone: deeper in means further from the edge.
        let d1 = g.dir(0, n + 1.5, 200.5);
        let d2 = g.dir(0, n + 3.5, 200.5);
        assert!(g.dir(0, n, 200.5).distance(d2) > g.dir(0, n, 200.5).distance(d1));
    }

    #[test]
    fn unsupported_configurations_are_rejected_with_a_reason() {
        assert!(Planet::try_new(1.0, 2048, 513, 131072).unwrap_err().contains("multiple of 256"));
        assert!(Planet::try_new(1.0, 2048, 512, 131072 + 2048).unwrap_err().contains("coarsest chunk edge"));
        assert!(Planet::try_new(1.0, 2048, 512, 131072 + 32).unwrap_err().contains("multiple of H"));
        assert!(Planet::try_new(1.0, 2000, 512, 131072).unwrap_err().contains("power of two"));
        assert!(Planet::try_new(0.0, 2048, 512, 131072).unwrap_err().contains("positive"));
        assert!(Planet::try_new(1.0, 2048, 256, 131072).is_ok());
        assert!(Planet::try_new(1.0, 2048, 1024 + 512, 131072).is_ok(), "1536 = 6 x 256");
        assert!(Planet::try_new(0.5, 2048, 512, 262144).is_ok());
    }

    #[test]
    fn coarse_voxel_covers_exactly_2_to_the_l_fine_columns() {
        // LOD alignment: voxel x of chunk (lod L, cx) covers lattice columns
        // [i0 + x 2^L, i0 + (x+1) 2^L), and the eight children tile the parent.
        for lod in 1..=9u8 {
            let k = ChunkKey::neisor(2, lod, 5, 3, 1);
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

    /// Every curvature-grid corner the mesher emits (5x5 per footprint, at
    /// the chunk's radial floor and ceiling) lies inside the chunk's
    /// bounding sphere, including next to a cube corner where the lattice
    /// is most skewed (review #9: the old half*sqrt(3)*1.05 radius was
    /// 30.1 m there against a 33.8 m corner).
    #[test]
    fn chunk_bounds_contain_every_grid_corner_near_a_cube_corner() {
        let p = planet();
        let corner_l0 = ChunkKey::neisor(0, 0, 4095, 4095, 16);
        let mut keys = vec![
            corner_l0,
            ChunkKey::neisor(0, 0, 0, 4095, 16),
            ChunkKey::neisor(3, 0, 0, 0, 20),
            ChunkKey::neisor(0, 9, 7, 7, 0),
            ChunkKey::neisor(5, 9, 0, 0, 0),
            ChunkKey::neisor(2, 4, 255, 0, 2),
            ChunkKey::neisor(2, 6, 10, 12, 0),
            ChunkKey::neisor(1, 2, 1023, 500, 10),
        ];
        // A pseudo-random spread over every face and LOD.
        let mut s = 0x9E37_79B9u64;
        for _ in 0..200 {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let lod = ((s >> 33) % 10) as u8;
            let per_edge = p.chunks_per_edge(lod);
            let per_col = p.chunks_per_column(lod);
            keys.push(ChunkKey::neisor(((s >> 43) % 6) as u8, lod, ((s >> 13) as u32) % per_edge, ((s >> 23) as u32) % per_edge, ((s >> 3) as u32) % per_col));
        }
        let mut worst_slack = f64::MAX;
        for key in keys {
            let b = p.chunk_bounds(key);
            let e = key.extent_blocks() as f64;
            let (i0, j0, k0) = key.origin_blocks();
            let (i0, j0, k0) = (i0 as f64, j0 as f64, k0 as f64);
            let mut farthest = 0.0f64;
            for gy in 0..=4 {
                for gx in 0..=4 {
                    let d = p.dir(key.face, i0 + e * gx as f64 / 4.0, j0 + e * gy as f64 / 4.0);
                    for r in [p.radius_at(k0), p.radius_at((k0 + e).min(p.height as f64))] {
                        farthest = farthest.max((d * r - b.centre).length());
                    }
                }
            }
            assert!(farthest <= b.radius, "{key:?}: grid corner {farthest:.3} m from the centre, radius {:.3}", b.radius);
            worst_slack = worst_slack.min(b.radius - farthest);
            // Not absurdly loose either: within 2 % + 1 m of the tightest sphere about this centre.
            assert!(b.radius <= farthest * 1.02 + 1.0, "{key:?}: radius {:.3} vs farthest corner {farthest:.3}", b.radius);
        }
        println!("smallest slack {worst_slack:.4} m");
        // The review's numbers, reproduced: the old radius misses the corner.
        let old = 16.0 * 3f64.sqrt() * 1.05 + 1.0;
        let b = p.chunk_bounds(corner_l0);
        assert!(b.radius > 33.8 && old < 33.8, "old {old:.2}, new {:.2}", b.radius);
    }

    #[test]
    fn extended_coords_inverts_wrap_at_every_side() {
        let g = Planet::new(1.0, 512, 256, 4096);
        let n = g.n as f64;
        for face in 0..6u8 {
            for (i, j) in [(n + 3.5, 100.5), (-2.5, 100.5), (100.5, n + 0.5), (100.5, -7.5), (n + 1.5, n - 0.5)] {
                let (f, wi, wj) = g.wrap(face, i, j);
                assert_ne!(f, face);
                let (bi, bj) = g.extended_coords(face, f, wi, wj).expect("adjacent");
                assert!((bi - i).abs() < 1e-9 && (bj - j).abs() < 1e-9, "face {face} ({i}, {j}) -> {f} ({wi}, {wj}) -> ({bi}, {bj})");
            }
            // The opposite face shares no side.
            let opposite = (0..6u8).find(|&o| o != face && (0..4).all(|e| g.edges[face as usize][e].face != o)).unwrap();
            assert!(g.extended_coords(face, opposite, 1.0, 1.0).is_none());
        }
    }

    #[test]
    fn keys_know_their_descendants() {
        let k = ChunkKey::neisor(2, 3, 5, 6, 1);
        assert!(k.contains(&k));
        for c in k.children() {
            assert!(k.contains(&c));
            for g in c.children() {
                assert!(k.contains(&g) && c.contains(&g) && !g.contains(&k));
            }
        }
        assert!(!k.contains(&ChunkKey::neisor(2, 3, 4, 6, 1)));
        assert!(!k.contains(&ChunkKey { body: 1, ..k }));
        assert!(!k.contains(&ChunkKey::neisor(3, 2, 10, 12, 2)));
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

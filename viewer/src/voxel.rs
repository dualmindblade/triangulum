//! Near-field voxels: the diamond prisms become a landscape.
//!
//! Each cube face carries a 10,000,000-column lattice (~1 m column spacing —
//! the game-spec dimensions). A voxel is the cell between face-grid lines
//! (i, i+1) x (j, j+1), extruded between radial shells 1 m apart: a
//! diamond-shaped prism whose exact shape follows the gnomonic projection.
//!
//! Generation is column-based but no longer a single height: every column
//! carries a solid top, an optional water surface (sea / rivers / ponds),
//! and a band of cave carving below the surface (bit per block). Materials
//! come from the planet map's biome, temperature, and roughness, so deserts
//! are sand over sandstone, taiga is dark grass over dirt over stone, and
//! cliffs bare their rock. Trees are hash-placed full-block flora.
//!
//! Chunks are 32x32 columns, keyed (face, cx, cy) — the same "independent
//! node, any depth" property as the tiles: a chunk needs nothing but its key.

use crate::planet::{face_dir, ground_tint, Planet};
use crate::terrain::{sample, TileMesh, Vertex, VOXEL_OCTAVES};
use glam::DVec3;
use std::collections::HashMap;

pub const COLUMNS_PER_FACE: u64 = 10_000_000; // 1 m columns on a 10,000 km face
pub const CHUNK: u64 = 32;
pub const VOXEL_KM: f64 = 0.001;
/// How far below the nominal surface cave carving is evaluated (blocks).
const CAVE_DEPTH: i64 = 26;
/// Tree canopies reach 2 columns from the trunk, and each anchor checks
/// relief 2 columns around itself — the context grid carries 4 extra
/// columns so cross-chunk canopies mesh identically on both sides.
const TREE_MARGIN: i64 = 4;

/// Player edits: per-column height delta in blocks (break top = -1 each,
/// place on top = +1 each). Sparse — only touched columns are stored.
pub type Edits = HashMap<(u8, u64, u64), i64>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkKey {
    pub face: u8,
    pub cx: u64,
    pub cy: u64,
}

// ---------------------------------------------------------------- materials

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mat {
    Grass, // tinted by biome
    Dirt,
    Sand,
    Gravel,
    Stone,
    Rock,
    Snow,
    Ice,
    Water,
    Log,
    LeavesBroad,
    LeavesConifer,
    LeavesJungle,
    LeavesAcacia,
    Shrub,
}

impl Mat {
    /// Base linear-RGB color; Grass takes the biome ground tint.
    fn color(self, tint: [f32; 3]) -> [f32; 3] {
        match self {
            Mat::Grass => tint,
            Mat::Dirt => [0.23, 0.15, 0.085],
            Mat::Sand => [0.60, 0.51, 0.30],
            Mat::Gravel => [0.32, 0.31, 0.29],
            Mat::Stone => [0.30, 0.29, 0.28],
            Mat::Rock => [0.20, 0.195, 0.19],
            Mat::Snow => [0.83, 0.86, 0.91],
            Mat::Ice => [0.60, 0.72, 0.85],
            Mat::Water => [0.055, 0.17, 0.30],
            Mat::Log => [0.16, 0.10, 0.05],
            Mat::LeavesBroad => [0.065, 0.20, 0.035],
            Mat::LeavesConifer => [0.035, 0.11, 0.05],
            Mat::LeavesJungle => [0.04, 0.19, 0.03],
            Mat::LeavesAcacia => [0.14, 0.20, 0.045],
            Mat::Shrub => [0.10, 0.16, 0.06],
        }
    }
}

fn hash_u64(face: u8, ci: u64, cj: u64, salt: u64) -> u64 {
    // a linear combination alone has no avalanche: thresholding it puts
    // trees in lattice stripes ("orchard rows"). Finalize splitmix64-style
    // so nearby columns decorrelate.
    let mut x = (ci ^ ((face as u64) << 60))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(cj.wrapping_mul(0x85EB_CA77_C2B2_AE63))
        .wrapping_add(salt.wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn hash01(face: u8, ci: u64, cj: u64, salt: u64) -> f64 {
    ((hash_u64(face, ci, cj, salt) >> 11) & 0xFFFF_FFFF) as f64 / 4294967296.0
}

// ---------------------------------------------------------------- columns

/// Everything generation knows about one column, independent of neighbors.
#[derive(Clone, Copy)]
pub struct ColCtx {
    pub ground: i64,  // top solid block, including player edits
    /// Natural terrain top, before player edits. Caves, strata, steepness,
    /// and tree decisions anchor here so a player tower stays the material
    /// it was built on instead of turning into a stone cliff.
    pub ground0: i64,
    pub water: i64,     // water surface block; i64::MIN = dry column
    pub cave_bits: u32, // bit k set = block at z = ground0 - k carved out
    pub koppen: u8,
    pub e_raw: f32,
    pub temp: f32,
    pub precip: f32,
    pub rough: f32,
    pub carved: bool, // river/pond carving touched this column
    pub salt: bool,   // water here belongs to a salt lake
}

impl ColCtx {
    /// Is block z solid? (Below the cave band everything is solid.)
    pub fn filled(&self, z: i64) -> bool {
        if z > self.ground {
            return false;
        }
        if z > self.ground0 {
            return true; // player-built blocks are never cave-carved
        }
        let k = self.ground0 - z;
        if k < CAVE_DEPTH {
            (self.cave_bits >> k) & 1 == 0
        } else {
            true
        }
    }

    /// Highest solid block (cave breaches lower it — pits are real; digging
    /// down can open into the cave band).
    pub fn top_solid(&self) -> i64 {
        let z_min = self.ground0 - CAVE_DEPTH;
        let mut z = self.ground;
        while z > z_min && !self.filled(z) {
            z -= 1;
        }
        z
    }

    fn has_water(&self) -> bool {
        self.water != i64::MIN && self.water > self.top_solid()
    }
}

/// Column context by extended lattice index: indices past the face edge
/// resolve onto the neighbor face (cube-face lattices coincide along shared
/// edges). Shared by chunk meshing and the tree/physics queries so both
/// see identical columns.
pub fn col_ctx_ext(
    planet: &Planet,
    edits: &Edits,
    face: usize,
    i_ext: i64,
    j_ext: i64,
) -> ColCtx {
    let max = COLUMNS_PER_FACE as i64;
    if (0..max).contains(&i_ext) && (0..max).contains(&j_ext) {
        col_ctx(planet, edits, face, i_ext as u64, j_ext as u64)
    } else {
        let nn = COLUMNS_PER_FACE as f64;
        let u = -1.0 + 2.0 * (i_ext as f64 + 0.5) / nn;
        let v = -1.0 + 2.0 * (j_ext as f64 + 0.5) / nn;
        let (f2, u2, v2) = crate::planet::face_from_dir(face_dir(face, u, v));
        let (ci, cj) = column_of(u2, v2);
        col_ctx(planet, edits, f2, ci, cj)
    }
}

/// Generate one column from scratch: terrain sample, water, caves.
pub fn col_ctx(planet: &Planet, edits: &Edits, face: usize, ci: u64, cj: u64) -> ColCtx {
    let nn = COLUMNS_PER_FACE as f64;
    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
    let s = sample(planet, face, u, v, VOXEL_OCTAVES);
    let ground0 = (s.h_km * 1000.0).floor() as i64;
    let ground = ground0 + edits.get(&(face as u8, ci, cj)).copied().unwrap_or(0);
    let water = if s.water_km > s.h_km {
        (s.water_km * 1000.0).floor() as i64
    } else {
        i64::MIN
    };

    // caves: tubes along the intersection of two noise level-sets, in dry
    // land only. The radial offset gives the noise true vertical variation.
    // Anchored to the natural ground so edits don't drag the cave band.
    let mut cave_bits = 0u32;
    if water == i64::MIN && ground0 > 4 {
        let dir = face_dir(face, u, v);
        let seed = planet.seed;
        let region = crate::noise::gradient_noise(dir * 90.0, seed.wrapping_add(40961));
        if region > -0.05 {
            for k in 0..CAVE_DEPTH {
                let zm = (ground0 - k) as f64;
                let n1 = crate::noise::gradient_noise(
                    dir * (90000.0 + zm / 12.0),
                    seed.wrapping_add(31337),
                );
                if n1.abs() < 0.085 {
                    let n2 = crate::noise::gradient_noise(
                        dir * (76000.0 + zm / 9.0),
                        seed.wrapping_add(51413),
                    );
                    if n2.abs() < 0.085 {
                        cave_bits |= 1 << k;
                    }
                }
            }
        }
    }

    ColCtx {
        ground,
        ground0,
        water,
        cave_bits,
        koppen: planet.koppen(face, u, v),
        e_raw: s.e_raw as f32,
        temp: s.temp_c as f32,
        precip: s.precip as f32,
        rough: s.rough as f32,
        carved: s.carve_km > 0.001,
        salt: s.salt,
    }
}

/// Surface material given local steepness (max |height delta| to neighbors).
/// `jitter` (hash01 of the column) dithers the snow line so blocks pepper in
/// across the same -7.5..-10.5 C band the mesh's snow ramp covers.
fn surface_mat(c: &ColCtx, steep: i64, jitter: f64) -> Mat {
    // underwater floors
    if c.water != i64::MIN && c.ground < c.water {
        return if c.water - c.ground > 4 { Mat::Gravel } else { Mat::Sand };
    }
    if c.koppen == 29 || (c.temp as f64) < -9.0 + (jitter - 0.5) * 3.0 {
        return Mat::Snow;
    }
    if steep >= 5 {
        return Mat::Rock;
    }
    if steep >= 3 {
        return Mat::Stone;
    }
    // beaches: low ground near sea level (natural ground: a tower built on
    // a beach stays a sand tower)
    if c.e_raw < 0.012 && c.ground0 < 14 {
        return Mat::Sand;
    }
    match c.koppen {
        3 | 4 => Mat::Sand, // deserts
        255 => Mat::Sand,   // coastal strand: land under an ocean-class texel
        _ => Mat::Grass,
    }
}

fn sub_mat(surface: Mat) -> Mat {
    match surface {
        Mat::Sand => Mat::Sand,
        Mat::Snow => Mat::Dirt,
        Mat::Gravel => Mat::Gravel,
        Mat::Rock => Mat::Rock,
        Mat::Stone => Mat::Stone,
        _ => Mat::Dirt,
    }
}

/// Material of solid block z in a column: surface stratum on top, substrate
/// below it, stone at depth. Cave interiors thus read as stone naturally.
/// Strata measure from the *natural* ground: player-built blocks above it
/// are surface on top and substrate in the body (a grass-capped dirt tower),
/// never bare stone.
fn mat_at(c: &ColCtx, z: i64, surface: Mat) -> Mat {
    if z > c.ground0 {
        return if z >= c.ground { surface } else { sub_mat(surface) };
    }
    let d = c.ground.min(c.ground0) - z;
    if d <= 0 {
        surface
    } else if d <= 3 {
        sub_mat(surface)
    } else {
        Mat::Stone
    }
}

// ---------------------------------------------------------------- trees

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TreeKind {
    Conifer,
    Broadleaf,
    Jungle,
    Acacia,
    Shrub,
}

/// Deterministic tree placement: species + density by biome, no trees on
/// steep ground, water, beaches, or beyond the cold treeline.
pub fn tree_at(c: &ColCtx, face: u8, ci: u64, cj: u64, seed: i64) -> Option<(TreeKind, i64)> {
    // no trees in water, on beaches, or in river/pond carved ground (a
    // canopy anchored in a gully pokes out at rim level as leaf shards)
    if c.has_water() || c.water != i64::MIN || c.e_raw < 0.010 || c.carved {
        return None;
    }
    // densities are per-column; one canopy covers ~25 columns, so 0.010
    // is already a closed-canopy forest
    let (kind, density) = match c.koppen {
        0 | 1 => (TreeKind::Jungle, 0.011),
        2 => (TreeKind::Acacia, 0.0012),
        5 => (TreeKind::Acacia, 0.0005),
        6 => (TreeKind::Shrub, 0.0015),
        7 | 8 | 9 => (TreeKind::Broadleaf, 0.0025),
        10..=15 | 20 | 21 | 24 | 25 => (TreeKind::Broadleaf, 0.005),
        16..=19 => (TreeKind::Conifer, 0.003),
        22 | 23 | 26 | 27 => (TreeKind::Conifer, 0.007),
        28 => (TreeKind::Shrub, 0.002),
        _ => return None, // deserts, ice cap, ocean
    };
    // treeline: shrubs shiver on, trees give up
    if c.temp < -6.0 && kind != TreeKind::Shrub {
        return None;
    }
    if c.temp < -11.0 {
        return None;
    }
    if hash01(face, ci, cj, seed as u64 ^ 0x7255) >= density {
        return None;
    }
    let h_var = (hash01(face, ci, cj, 0x9E11) * 3.0) as i64;
    let trunk = match kind {
        TreeKind::Conifer => 4 + h_var,
        TreeKind::Broadleaf => 3 + h_var,
        TreeKind::Jungle => 6 + h_var,
        TreeKind::Acacia => 3 + (h_var).min(1),
        TreeKind::Shrub => 0,
    };
    Some((kind, trunk))
}

/// The full-block cells of a tree, relative to its anchor column's ground.
pub fn tree_cells(kind: TreeKind, trunk: i64, rnd: u64) -> Vec<(i64, i64, i64, Mat)> {
    let mut cells = Vec::new();
    let mut leaf = |dx: i64, dy: i64, dz: i64, m: Mat| cells.push((dx, dy, dz, m));
    match kind {
        TreeKind::Shrub => {
            leaf(0, 0, 1, Mat::Shrub);
            if rnd & 1 == 0 {
                leaf(1, 0, 1, Mat::Shrub);
            }
            if rnd & 2 == 0 {
                leaf(0, 1, 1, Mat::Shrub);
            }
        }
        TreeKind::Conifer => {
            // stacked diamonds narrowing to a tip
            for l in 0..=(trunk - 1) {
                let z = 2 + l;
                let r = ((trunk - l) / 2).clamp(0, 2);
                for dx in -r..=r {
                    for dy in -(r - dx.abs())..=(r - dx.abs()) {
                        if dx != 0 || dy != 0 {
                            leaf(dx, dy, z, Mat::LeavesConifer);
                        }
                    }
                }
            }
            leaf(0, 0, trunk + 1, Mat::LeavesConifer);
            leaf(0, 0, trunk + 2, Mat::LeavesConifer);
        }
        TreeKind::Broadleaf | TreeKind::Jungle => {
            let m = if kind == TreeKind::Jungle { Mat::LeavesJungle } else { Mat::LeavesBroad };
            for dz in -1..=1i64 {
                let r = if dz == 0 { 2 } else { 1 + (dz < 0) as i64 };
                for dx in -r..=r {
                    for dy in -r..=r {
                        // trim the square corners, with a little hash raggedness
                        let corner = dx.abs() + dy.abs() > r + 1 - (rnd >> ((dx + 2) * 5 + dy + 2) & 1) as i64;
                        if !corner {
                            leaf(dx, dy, trunk + dz, m);
                        }
                    }
                }
            }
            leaf(0, 0, trunk + 2, m);
        }
        TreeKind::Acacia => {
            for dx in -2..=2i64 {
                for dy in -2..=2i64 {
                    if dx.abs() + dy.abs() <= 3 {
                        leaf(dx, dy, trunk + 1, Mat::LeavesAcacia);
                    }
                }
            }
        }
    }
    // trunk last so it wins where canopy overlaps
    for z in 1..=trunk {
        cells.push((0, 0, z, Mat::Log));
    }
    cells
}

/// The tree standing at a column, applying the same slope/gully/cave-mouth
/// rejections as chunk meshing — physics and rendering must agree on which
/// trees exist. Returns (kind, trunk height in blocks).
pub fn tree_here(
    planet: &Planet,
    edits: &Edits,
    face: usize,
    ci: u64,
    cj: u64,
) -> Option<(TreeKind, i64)> {
    let c = col_ctx(planet, edits, face, ci, cj);
    if c.ground != c.ground0 {
        return None; // editing a tree's column chops the tree
    }
    let (i, j) = (ci as i64, cj as i64);
    let mut relief = 0i64;
    let mut carved_near = c.carved;
    for (di, dj) in [(2i64, 0i64), (-2, 0), (0, 2), (0, -2), (1, 1), (-1, -1), (1, -1), (-1, 1)] {
        let nb = col_ctx_ext(planet, edits, face, i + di, j + dj);
        // natural relief: a neighbor's player tower must not shake trees down
        relief = relief.max((c.ground0 - nb.ground0).abs());
        carved_near |= nb.carved;
    }
    if relief > 2 || carved_near || c.top_solid() != c.ground {
        return None;
    }
    tree_at(&c, face as u8, ci, cj, planet.seed)
}

// ---------------------------------------------------------------- queries

pub fn lift_km(exaggeration: f64) -> f64 {
    (1.6 * VOXEL_KM * exaggeration).max(0.0012)
}

fn column_of(u: f64, v: f64) -> (u64, u64) {
    let n = COLUMNS_PER_FACE as f64;
    let ci = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let cj = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    (ci, cj)
}

/// Height of the *solid* walkable surface (km, exaggerated, incl. the patch
/// lift) under a direction. Water is NOT walkable: you wade into ponds and
/// sink through rivers to their floor. Mirrors build_chunk's shell()/lift
/// so feet match the visible voxel tops.
pub fn surface_height_km(planet: &Planet, edits: &Edits, dir: DVec3, exaggeration: f64) -> f64 {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of(u, v);
    let c = col_ctx(planet, edits, face, ci, cj);
    c.top_solid() as f64 * VOXEL_KM * exaggeration + lift_km(exaggeration)
}

/// Highest solid block top at or below `at_km` in the column under `dir`
/// (same exaggerated+lift height units as surface_height_km). Cave-aware:
/// standing over a pit this is the pit floor, inside a tunnel the tunnel
/// floor — the physics query for gravity, landing, and step-up.
pub fn support_below_km(
    planet: &Planet,
    edits: &Edits,
    dir: DVec3,
    at_km: f64,
    exaggeration: f64,
) -> f64 {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of(u, v);
    let c = col_ctx(planet, edits, face, ci, cj);
    // tree trunks are solid (shrubs are not) — you bump into and can stand
    // on trunks; canopy leaves stay passable
    let trunk_top = tree_here(planet, edits, face, ci, cj)
        .filter(|(k, _)| *k != TreeKind::Shrub)
        .map(|(_, t)| c.ground + t);
    let solid = |z: i64| c.filled(z) || trunk_top.is_some_and(|t| z > c.ground && z <= t);
    let scale = VOXEL_KM * exaggeration;
    let lift = lift_km(exaggeration);
    let mut z = (((at_km - lift) / scale) + 1e-7).floor() as i64;
    z = z.min(trunk_top.unwrap_or(c.ground).max(c.ground));
    let z_min = c.ground - CAVE_DEPTH - 1;
    while z >= z_min {
        if solid(z) {
            return z as f64 * scale + lift;
        }
        z -= 1;
    }
    z_min as f64 * scale + lift
}

/// Lowest solid block *bottom* strictly above `at_km` in the column under
/// `dir`, or +inf under open sky — head collision for jumps, cave roofs.
pub fn ceiling_above_km(
    planet: &Planet,
    edits: &Edits,
    dir: DVec3,
    at_km: f64,
    exaggeration: f64,
) -> f64 {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of(u, v);
    let c = col_ctx(planet, edits, face, ci, cj);
    let trunk_top = tree_here(planet, edits, face, ci, cj)
        .filter(|(k, _)| *k != TreeKind::Shrub)
        .map(|(_, t)| c.ground + t);
    let solid = |z: i64| c.filled(z) || trunk_top.is_some_and(|t| z > c.ground && z <= t);
    let scale = VOXEL_KM * exaggeration;
    let lift = lift_km(exaggeration);
    // first block whose span could sit above at_km
    let mut z = (((at_km - lift) / scale) - 1e-7).floor() as i64 + 1;
    z = z.max(c.ground - CAVE_DEPTH);
    let z_top = trunk_top.unwrap_or(c.ground).max(c.ground);
    while z <= z_top {
        if solid(z) {
            return (z - 1) as f64 * scale + lift;
        }
        z += 1;
    }
    f64::INFINITY
}

/// Water surface height (km, exaggerated, incl. lift) under a direction,
/// if this column holds water. For the wading/underwater check.
pub fn water_surface_km(
    planet: &Planet,
    edits: &Edits,
    dir: DVec3,
    exaggeration: f64,
) -> Option<f64> {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of(u, v);
    let c = col_ctx(planet, edits, face, ci, cj);
    if c.has_water() {
        Some(c.water as f64 * VOXEL_KM * exaggeration + lift_km(exaggeration))
    } else {
        None
    }
}

/// March along the look ray until it dips below the walkable surface;
/// returns (hit column, last air column before it). Step is a third of a
/// block. The air column is where a placed block belongs: aiming at the
/// side of a tower builds next to it instead of pushing the tower up.
pub fn raycast_column(
    planet: &Planet,
    edits: &Edits,
    eye_km: DVec3,
    look: DVec3,
    max_m: f64,
    exaggeration: f64,
) -> Option<((u8, u64, u64), (u8, u64, u64))> {
    let col_under = |dir: DVec3| {
        let (face, u, v) = crate::planet::face_from_dir(dir);
        let (ci, cj) = column_of(u, v);
        (face as u8, ci, cj)
    };
    let mut prev = col_under(eye_km.normalize());
    let mut t_m = 0.4;
    while t_m < max_m {
        let p = eye_km + look * (t_m / 1000.0);
        let dir = p.normalize();
        let surf_r = planet.radius_km + surface_height_km(planet, edits, dir, exaggeration);
        let col = col_under(dir);
        if p.length() <= surf_r {
            return Some((col, prev));
        }
        prev = col;
        t_m += 0.33;
    }
    None
}

/// The chunk containing a column, plus neighbors if the column sits on the
/// chunk border (their ghost rings see it) — all need remeshing on edit.
pub fn chunks_touching_column(face: u8, ci: u64, cj: u64) -> Vec<ChunkKey> {
    let (cx, cy) = (ci / CHUNK, cj / CHUNK);
    let mut out = vec![ChunkKey { face, cx, cy }];
    let max_c = COLUMNS_PER_FACE / CHUNK - 1;
    if ci % CHUNK == 0 && cx > 0 {
        out.push(ChunkKey { face, cx: cx - 1, cy });
    }
    if ci % CHUNK == CHUNK - 1 && cx < max_c {
        out.push(ChunkKey { face, cx: cx + 1, cy });
    }
    if cj % CHUNK == 0 && cy > 0 {
        out.push(ChunkKey { face, cx, cy: cy - 1 });
    }
    if cj % CHUNK == CHUNK - 1 && cy < max_c {
        out.push(ChunkKey { face, cx, cy: cy + 1 });
    }
    out
}

/// Radius (km) of the disc that is *guaranteed* covered by built chunks, for
/// cutting the heightfield away underneath the voxel patch. select_chunks
/// covers a true metric disc (crossing face edges), so this is just the
/// selection radius minus a one-chunk safety margin.
pub fn safe_hole_radius_km(radius_m: f64) -> f64 {
    ((radius_m - 96.0) / 1000.0).max(0.0)
}

/// Chunks within `radius_m` of the camera's ground point. Selection samples
/// directions on a tangent-plane disc and asks face_from_dir which chunk owns
/// each sample — so the ring spills across cube-face edges for free instead
/// of clamping to the camera's face.
pub fn select_chunks(cam_pos: DVec3, planet: &Planet, radius_m: f64) -> Vec<ChunkKey> {
    let dir = cam_pos.normalize();
    let ref_axis = if dir.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let t1 = (ref_axis - dir * ref_axis.dot(dir)).normalize();
    let t2 = dir.cross(t1);
    let n = COLUMNS_PER_FACE as f64;
    // chunk size shrinks toward cube-face edges and corners (the gnomonic
    // cell's short axis scales as 1/(1+u^2+v^2): half size at edge middles,
    // a third at corners). Sample at 0.45x the local worst case so every
    // chunk overlapping the disc is hit — a fixed step tuned for the face
    // center skipped chunks near the edges, punching see-through holes in
    // the patch.
    let (_, u0, v0) = crate::planet::face_from_dir(dir);
    let chunk_min_km =
        CHUNK as f64 * (2.0 / n) * planet.radius_km / (1.0 + u0 * u0 + v0 * v0);
    let step_km = 0.45 * chunk_min_km;
    let r_km = radius_m / 1000.0;
    let steps = (r_km / step_km).ceil() as i64;
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(ChunkKey, i64)> = Vec::new();
    for sy in -steps..=steps {
        for sx in -steps..=steps {
            let (dx, dy) = (sx as f64 * step_km, sy as f64 * step_km);
            let d2 = dx * dx + dy * dy;
            if d2 > (r_km + step_km) * (r_km + step_km) {
                continue;
            }
            let p = (dir * planet.radius_km + t1 * dx + t2 * dy).normalize();
            let (face, u, v) = crate::planet::face_from_dir(p);
            let ci = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
            let cj = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
            let key = ChunkKey { face: face as u8, cx: ci / CHUNK, cy: cj / CHUNK };
            if seen.insert(key) {
                out.push((key, (d2 * 1e9) as i64));
            }
        }
    }
    out.sort_unstable_by_key(|(_, d)| *d);
    out.into_iter().map(|(k, _)| k).collect()
}

// ---------------------------------------------------------------- meshing

/// Mesh one chunk: solid columns (tops, cave ceilings/floors, strata sides),
/// water surfaces, and trees.
pub fn build_chunk(planet: &Planet, edits: &Edits, key: ChunkKey, exaggeration: f64) -> TileMesh {
    let n = CHUNK as i64;
    let face = key.face as usize;
    let nn = COLUMNS_PER_FACE as f64;
    let u_of = |i: i64| -1.0 + 2.0 * i as f64 / nn;
    let v_of = |j: i64| -1.0 + 2.0 * j as f64 / nn;

    // column contexts for the chunk plus TREE_MARGIN on each side. Columns
    // past the face edge resolve to the neighbor face via the extended
    // lattice direction — cube-face lattices coincide along shared edges.
    let m = TREE_MARGIN;
    let np = (n + 2 * m) as usize;
    let base_i = key.cx as i64 * n;
    let base_j = key.cy as i64 * n;
    let mut cols: Vec<ColCtx> = Vec::with_capacity(np * np);
    for gj in 0..np as i64 {
        for gi in 0..np as i64 {
            cols.push(col_ctx_ext(planet, edits, face, base_i + gi - m, base_j + gj - m));
        }
    }
    let at = |gi: i64, gj: i64| -> &ColCtx { &cols[(gj + m) as usize * np + (gi + m) as usize] };

    let radius = planet.radius_km;
    let lift = lift_km(exaggeration);
    let origin_dir = face_dir(face, u_of(base_i + n / 2), v_of(base_j + n / 2));
    let origin = origin_dir * radius;
    let shell = |z: i64| radius + (z as f64) * VOXEL_KM * exaggeration + lift;

    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    // per-corner colors: ambient occlusion darkens individual corners.
    // `dim` is the cave-darkness factor (1 = open sky): blocks carry it in
    // the water attribute's alpha so the shader — not the bake — applies
    // it, letting the player's torch light the rock back up near them.
    let mut quad = |corners: [DVec3; 4], normal: DVec3, cols: [[f32; 3]; 4], dim: f32| {
        let base = vertices.len() as u32;
        for (k, c) in corners.iter().enumerate() {
            let p = *c - origin;
            vertices.push(Vertex {
                pos: [p.x as f32, p.y as f32, p.z as f32],
                normal: [normal.x as f32, normal.y as f32, normal.z as f32],
                color: cols[k],
                water: [0.0, 0.0, 0.0, dim],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    let vary = |c: [f32; 3], t: f32| [c[0] * t, c[1] * t, c[2] * t];

    for j in 0..n {
        for i in 0..n {
            let c = at(i, j);
            let ci = (base_i + i) as u64;
            let cj = (base_j + j) as u64;
            let (u0, u1) = (u_of(base_i + i), u_of(base_i + i + 1));
            let (v0, v1) = (v_of(base_j + j), v_of(base_j + j + 1));
            let d00 = face_dir(face, u0, v0);
            let d10 = face_dir(face, u1, v0);
            let d11 = face_dir(face, u1, v1);
            let d01 = face_dir(face, u0, v1);
            let up = origin_dir;
            let tint = ground_tint(c.koppen);

            let nbs = [at(i + 1, j), at(i - 1, j), at(i, j + 1), at(i, j - 1)];
            // natural steepness: player towers are not cliffs
            let steep = nbs
                .iter()
                .map(|nb| (c.ground0 - nb.ground0).abs())
                .max()
                .unwrap_or(0);
            let surf = surface_mat(c, steep, hash01(face as u8, ci, cj, 0x5A0E));

            // per-block brightness hash: the subtle checkerboard that reads
            // "voxel" (keyed per column+height so sides vary too)
            let bright = |z: i64| {
                let h = hash_u64(face as u8, ci, cj, z as u64);
                0.9 + 0.2 * ((h >> 17 & 0xFF) as f32 / 255.0)
            };

            // ---- solid faces: z sweeps the cave band plus any cliff drop
            let lo_side = nbs.iter().map(|nb| nb.top_solid()).min().unwrap_or(c.ground);
            let z_lo = (c.ground - CAVE_DEPTH).min(lo_side).max(c.ground - 220);
            for z in z_lo..=c.ground {
                if !c.filled(z) {
                    continue;
                }
                let mat = mat_at(c, z, surf);
                let col = vary(mat.color(tint), bright(z));
                // cave dimming: faces buried under rock overhead darken
                // with depth below the walkable top (pit floors stay lit)
                let cave = (1.0 - 0.20 * (c.top_solid() - z).max(0) as f32).clamp(0.25, 1.0);
                if !c.filled(z + 1) {
                    let r = shell(z);
                    // per-corner ambient occlusion from the three blocks
                    // touching each corner one level up — the classic
                    // "soft shadow along walls" that makes blocks read 3D
                    let mut cols = [[0.0f32; 3]; 4];
                    for (k, &(a, b)) in
                        [(0i64, 0i64), (1, 0), (1, 1), (0, 1)].iter().enumerate()
                    {
                        let sx = at(i + (a * 2 - 1), j).filled(z + 1) as i32;
                        let sy = at(i, j + (b * 2 - 1)).filled(z + 1) as i32;
                        let dd = at(i + (a * 2 - 1), j + (b * 2 - 1)).filled(z + 1) as i32;
                        let lvl = if sx + sy == 2 { 3 } else { sx + sy + dd };
                        cols[k] = vary(col, 1.0 - 0.15 * lvl as f32);
                    }
                    quad([d00 * r, d10 * r, d11 * r, d01 * r], up, cols, cave);
                }
                if z > z_lo && !c.filled(z - 1) {
                    let r = shell(z - 1);
                    let cdark = vary(col, 0.55);
                    quad([d00 * r, d01 * r, d11 * r, d10 * r], -up, [cdark; 4], cave);
                }
            }
            // sides: contiguous exposed runs per neighbor, split by material
            let sides = [
                (0usize, d10, d11), // +i
                (1usize, d01, d00), // -i
                (2usize, d11, d01), // +j
                (3usize, d00, d10), // -j
            ];
            for (nbi, da, db) in sides {
                let nb = nbs[nbi];
                let out_n = (da + db).normalize() - up * (da + db).normalize().dot(up);
                let n_side = (out_n.normalize_or_zero() + up * 0.85).normalize();
                let mut run_start: Option<(i64, Mat)> = None;
                let mut z = z_lo;
                while z <= c.ground + 1 {
                    let exposed = z <= c.ground && c.filled(z) && !nb.filled(z);
                    let mat = if exposed { Some(mat_at(c, z, surf)) } else { None };
                    match (run_start, mat) {
                        (None, Some(mm)) => run_start = Some((z, mm)),
                        (Some((z0, m0)), other) if other != Some(m0) => {
                            let (r0, r1) = (shell(z0 - 1), shell(z - 1));
                            let cave = (1.0
                                - 0.20 * (nb.top_solid() - (z - 1)).max(0) as f32)
                                .clamp(0.25, 1.0);
                            let col = vary(vary(m0.color(tint), 0.72), bright(z0));
                            quad([da * r1, db * r1, db * r0, da * r0], n_side, [col; 4], cave);
                            run_start = other.map(|mm| (z, mm));
                        }
                        _ => {}
                    }
                    z += 1;
                }
            }

            // ---- water: top surface and exposed banks
            if c.has_water() {
                let w = c.water;
                let frozen = c.temp < -4.0;
                let wmat = if frozen { Mat::Ice } else { Mat::Water };
                let mut wcol = wmat.color(tint);
                if !frozen {
                    // same depth scale as the mesh's water_color, so the
                    // patch boundary doesn't jump shade: true ocean depth
                    // for the sea, carved depth (in km) for rivers/ponds
                    let depth_km = if c.e_raw < 0.0 {
                        -c.e_raw
                    } else {
                        (w - c.top_solid()) as f32 / 1000.0
                    };
                    let deep = [0.004, 0.013, 0.055];
                    let t = (depth_km / 2.5).clamp(0.0, 1.0);
                    wcol = [
                        wcol[0] + (deep[0] - wcol[0]) * t,
                        wcol[1] + (deep[1] - wcol[1]) * t,
                        wcol[2] + (deep[2] - wcol[2]) * t,
                    ];
                    if c.e_raw < 0.0 {
                        // shallow sea shoals teal — same ramp as the mesh's
                        // water_color so the patch boundary keeps its shade
                        let sh = (1.0 - depth_km / 0.02).clamp(0.0, 1.0) * 0.7;
                        let teal = [0.10, 0.32, 0.35];
                        wcol = [
                            wcol[0] + (teal[0] - wcol[0]) * sh,
                            wcol[1] + (teal[1] - wcol[1]) * sh,
                            wcol[2] + (teal[2] - wcol[2]) * sh,
                        ];
                    }
                    if c.salt {
                        // salt lakes: mineral-pale, matches the mesh tint
                        let pale = [0.45, 0.55, 0.52];
                        wcol = [
                            wcol[0] + (pale[0] - wcol[0]) * 0.55,
                            wcol[1] + (pale[1] - wcol[1]) * 0.55,
                            wcol[2] + (pale[2] - wcol[2]) * 0.55,
                        ];
                    }
                }
                let r = shell(w);
                quad([d00 * r, d10 * r, d11 * r, d01 * r], up, [wcol; 4], 1.0);
                for (nbi, da, db) in sides {
                    let nb = nbs[nbi];
                    let nb_surf = nb.top_solid().max(if nb.water == i64::MIN {
                        i64::MIN
                    } else {
                        nb.water
                    });
                    if nb_surf < w {
                        let out_n = (da + db).normalize() - up * (da + db).normalize().dot(up);
                        let n_side = (out_n.normalize_or_zero() + up * 0.85).normalize();
                        let (r0, r1) = (shell(nb_surf.max(c.top_solid())), shell(w));
                        quad(
                            [da * r1, db * r1, db * r0, da * r0],
                            n_side,
                            [vary(wcol, 0.8); 4],
                            1.0,
                        );
                    }
                }
            }
        }
    }

    // ---- trees: gather anchors in the margin, mesh cells inside the chunk
    let mut occ: HashMap<(i64, i64, i64), Mat> = HashMap::new();
    // anchors that can reach visible cells sit within canopy radius (2) of
    // the chunk; their relief probes reach 2 further — all inside the grid,
    // so every chunk makes identical decisions about shared trees
    for aj in (-m + 2)..(n + m - 2) {
        for ai in (-m + 2)..(n + m - 2) {
            let c = at(ai, aj);
            let aci = (base_i + ai) as u64;
            let acj = (base_j + aj) as u64;
            // relief across the whole canopy footprint: a tree planted on a
            // slope gets its crown buried and renders as floating shards.
            // Carved ground (river/pond gullies) anywhere under the canopy
            // disqualifies too — rim trees read as leaf scraps.
            if c.ground != c.ground0 {
                continue; // editing a tree's column chops the tree
            }
            let mut relief = 0i64;
            let mut carved_near = c.carved;
            for (di, dj) in [(2i64, 0i64), (-2, 0), (0, 2), (0, -2), (1, 1), (-1, -1), (1, -1), (-1, 1)] {
                let nb = at(ai + di, aj + dj);
                // natural relief — must mirror tree_here exactly
                relief = relief.max((c.ground0 - nb.ground0).abs());
                carved_near |= nb.carved;
            }
            if relief > 2 || carved_near || c.top_solid() != c.ground {
                continue; // no trees on slopes, gullies, or cave mouths
            }
            if let Some((kind, trunk)) = tree_at(c, key.face, aci, acj, planet.seed) {
                let rnd = hash_u64(key.face, aci, acj, 0xF0F0);
                for (dx, dy, dz, mat) in tree_cells(kind, trunk, rnd) {
                    occ.insert((ai + dx, aj + dy, c.ground + dz), mat);
                }
            }
        }
    }
    for (&(ti, tj, tz), &mat) in &occ {
        if !(0..n).contains(&ti) || !(0..n).contains(&tj) {
            continue;
        }
        let c = at(ti, tj);
        let ci = (base_i + ti) as u64;
        let cj = (base_j + tj) as u64;
        let tint = ground_tint(c.koppen);
        let h = hash_u64(face as u8, ci, cj, tz as u64);
        let bright = 0.88 + 0.24 * ((h >> 13 & 0xFF) as f32 / 255.0);
        let col = vary(mat.color(tint), bright);
        let (u0, u1) = (u_of(base_i + ti), u_of(base_i + ti + 1));
        let (v0, v1) = (v_of(base_j + tj), v_of(base_j + tj + 1));
        let d00 = face_dir(face, u0, v0);
        let d10 = face_dir(face, u1, v0);
        let d11 = face_dir(face, u1, v1);
        let d01 = face_dir(face, u0, v1);
        let up = origin_dir;
        let solid_at = |di: i64, dj: i64, z: i64| -> bool {
            occ.contains_key(&(ti + di, tj + dj, z))
                || at(ti + di, tj + dj).filled(z)
        };
        if !solid_at(0, 0, tz + 1) {
            let r = shell(tz);
            quad([d00 * r, d10 * r, d11 * r, d01 * r], up, [col; 4], 1.0);
        }
        if !solid_at(0, 0, tz - 1) {
            let r = shell(tz - 1);
            quad([d00 * r, d01 * r, d11 * r, d10 * r], -up, [vary(col, 0.6); 4], 1.0);
        }
        let sides = [
            (1i64, 0i64, d10, d11),
            (-1, 0, d01, d00),
            (0, 1, d11, d01),
            (0, -1, d00, d10),
        ];
        for (di, dj, da, db) in sides {
            if !solid_at(di, dj, tz) {
                let out_n = (da + db).normalize() - up * (da + db).normalize().dot(up);
                let n_side = (out_n.normalize_or_zero() + up * 0.85).normalize();
                let (r0, r1) = (shell(tz - 1), shell(tz));
                quad([da * r1, db * r1, db * r0, da * r0], n_side, [vary(col, 0.8); 4], 1.0);
            }
        }
    }

    TileMesh { origin_km: origin, vertices, indices }
}


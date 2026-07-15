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

use crate::planet::{
    climate_surface_with_biome, climate_surface_with_biome_at_season, face_dir, hash01, hash_u64,
    vegetation_surface, ClimateSurface, MainBlock, Planet,
};
use crate::moon::{MoonGenerator, MoonMaterial, MoonSample};
use crate::orbits::BodyId;
use crate::terrain::{
    sample_at_season, Sample, TileMesh, Vertex, NO_BIOME_PAYLOAD, VOXEL_OCTAVES,
};
use crate::weather::StructuralSeason;
use glam::DVec3;
use std::collections::{HashMap, HashSet};

pub use crate::planet::COLUMNS_PER_FACE;
pub const CHUNK: u64 = 32;
pub const VOXEL_KM: f64 = 0.001;
/// The configured moon is exactly 0.27 Neisor radii, so its angular lattice
/// has exactly 27/100 as many columns on each cube-face axis. 2,700,000 is
/// also exactly divisible by the 32-column chunk edge (84,375 chunks/face),
/// keeping face edges and chunk keys integral while matching Neisor's metric
/// block proportions. Area density falls by 1 / 0.27^2 = 13.717421... .
pub const LUNAR_COLUMNS_PER_FACE: u64 = COLUMNS_PER_FACE * 27 / 100;
/// How far below the nominal surface cave carving is evaluated (blocks).
const CAVE_DEPTH: i64 = 26;
/// Tree canopies reach 2 columns from the trunk, and each anchor checks
/// relief 2 columns around itself — the context grid carries 4 extra
/// columns so cross-chunk canopies mesh identically on both sides.
const TREE_MARGIN: i64 = 4;

/// Player edits: per-column height delta in blocks (break top = -1 each,
/// place on top = +1 each). Sparse — only touched columns are stored.
pub type Edits = HashMap<(u8, u64, u64), i64>;

/// Sparse edits are stored in independent column maps per physical body.
/// Keeping the familiar `Edits` map inside each slot preserves every Neisor
/// lookup and save record while making a body part of the authoritative key.
#[derive(Clone, Default)]
pub struct BodyEdits {
    sun: Edits,
    neisor: Edits,
    moon: Edits,
}

impl BodyEdits {
    pub fn for_body(&self, body: BodyId) -> &Edits {
        match body {
            BodyId::Sun => &self.sun,
            BodyId::Neisor => &self.neisor,
            BodyId::Moon => &self.moon,
        }
    }

    pub fn for_body_mut(&mut self, body: BodyId) -> &mut Edits {
        match body {
            BodyId::Sun => &mut self.sun,
            BodyId::Neisor => &mut self.neisor,
            BodyId::Moon => &mut self.moon,
        }
    }

    pub fn from_neisor(neisor: Edits) -> Self {
        Self {
            sun: Edits::default(),
            neisor,
            moon: Edits::default(),
        }
    }
}

/// Player-placed torches: a torch stands on the walkable top of its column
/// (it rides along if the column is edited). Persisted like edits.
pub type Torches = std::collections::HashSet<(u8, u64, u64)>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkKey {
    pub body: BodyId,
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
    RegolithHighland,
    RegolithMaria,
    RegolithRay,
    RegolithSubsurface,
    LunarRock,
}

impl Mat {
    /// Base linear-RGB color; main surface blocks (grass/sand/snow) take the
    /// shared climate tint. Public so the mesh-tile tree impostors wear
    /// EXACTLY the voxel palette (TRANSITIONS.md: one truth, two renderers).
    pub fn color(self, tint: [f32; 3]) -> [f32; 3] {
        match self {
            Mat::Grass | Mat::Sand | Mat::Snow => tint,
            // dry loam, not wet cellar soil: the old [0.23,0.15,0.085] was
            // the darkest ground material in the palette (darker than
            // stone), so every 2-block step's exposed dirt stratum read as
            // a black hole against grass tops in low light (the arrowed
            // faces in shot_lat20.810_lon28.922).
            Mat::Dirt => [0.33, 0.235, 0.135],
            Mat::Gravel => [0.32, 0.31, 0.29],
            Mat::Stone => [0.30, 0.29, 0.28],
            Mat::Rock => [0.20, 0.195, 0.19],
            Mat::Ice => [0.60, 0.72, 0.85],
            Mat::Water => [0.055, 0.17, 0.30],
            Mat::Log => [0.16, 0.10, 0.05],
            Mat::LeavesBroad => [0.065, 0.20, 0.035],
            Mat::LeavesConifer => [0.035, 0.11, 0.05],
            Mat::LeavesJungle => [0.04, 0.19, 0.03],
            Mat::LeavesAcacia => [0.14, 0.20, 0.045],
            Mat::Shrub => [0.22, 0.25, 0.10],
            Mat::RegolithHighland | Mat::RegolithMaria | Mat::RegolithRay => tint,
            Mat::RegolithSubsurface => [tint[0] * 0.72, tint[1] * 0.70, tint[2] * 0.68],
            Mat::LunarRock => [tint[0] * 0.48, tint[1] * 0.47, tint[2] * 0.46],
        }
    }
}

fn mix_color(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
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
    /// Continuous terrain height (km), pre-quantization — the same surface the
    /// far-LOD mesh shades with (ground0 = floor(h_km * 1000)). Block tops
    /// take their sun-shading normal from this smooth field so gentle slopes
    /// don't band into terrace-edge rings the way quantized heights do.
    pub h_km: f32,
    pub water: i64,     // water surface block; i64::MIN = dry column
    /// Continuous presentation level for the tightly-scoped A-5 handoff.
    /// Occupancy remains `water` and is never reclassified by this field.
    pub analog_water_km: f64,
    /// Liquid graph-river/lake overlap whose block ceiling must hand off to
    /// the lower continuous lake plane at the voxel-patch rim.
    pub rim_flush_water: bool,
    pub cave_bits: u32, // bit k set = block at z = ground0 - k carved out
    /// Local water table for FLOODED CAVES (BUGS.md W-6): any carved cave cell
    /// at z <= cave_water fills with water instead of air. Set only for dry-
    /// surface columns whose caves pass below a nearby river/lake/sea water
    /// table; i64::MIN when the caves (if any) stay dry. Distinct from `water`
    /// (the ONE open-air surface body) — this is an underground pool under
    /// solid rock, which the single-surface model can't otherwise express.
    pub cave_water: i64,
    /// The exact warped + locally-dithered surface field used by ground color.
    /// Tree density/category gates consume this same value rather than making
    /// an independent nearest-Koppen decision.
    /// Present only on climate-bearing bodies. Lunar columns carry the exact
    /// MoonSample-derived family/albedo below and never enter biome machinery.
    pub climate: Option<ClimateSurface>,
    pub lunar: Option<LunarColumn>,
    pub koppen: u8,
    /// Continuous climate at the same warped biome position. These select a
    /// tree silhouette without putting a nearest-class line back into density.
    pub biome_temp: f32,
    pub biome_precip: f32,
    /// Annual-only vegetation ownership. Seasonal snow/tints must not move
    /// tree anchors or change species/density.
    pub tree_main_block: MainBlock,
    pub tree_forest: f32,
    pub e_raw: f32,
    pub temp: f32,
    pub seasonal_temp: f32,
    pub seasonal_temp_trend: f32,
    pub frozen: bool,
    pub structural_season: StructuralSeason,
    pub precip: f32,
    pub rough: f32,
    pub carved: bool, // river/pond carving touched this column
    pub salt: bool,   // water here belongs to a salt lake
    /// TRUE ocean (the map's ocean mask), not merely e_raw < 0: dry basins
    /// and river mouths sit below sea level on purpose. The water color
    /// call must pass the same sea class the mesh does, or a below-sea-
    /// level river mouth renders a pale sea-tinted slab inside the patch
    /// against fresh mesh water (field hunt 3, 7.042 33.477).
    pub sea: bool,
    /// Shared terrain classification for an unrepresentable liquid-lake tie:
    /// dry ground raised flush with the water plane. Copied verbatim from the
    /// Sample so block truth and mesh truth name the same columns.
    pub lake_shoal: bool,
    /// Shared liquid-lake shore fraction; surface blocks dither on it.
    pub lake_shore_frac: f64,
    /// A finite local lake level makes the shared lake rule, rather than the
    /// generic low coastal beach rule, own surface sand in this territory.
    pub lake_material_region: bool,
    /// Preserve the pre-V-7 vegetation exclusion exactly. This is not a
    /// material decision: narrowing sand must not change tree placement.
    pub lake_level_band: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LunarColumn {
    pub material: MoonMaterial,
    pub albedo: f32,
    pub smoothness: f32,
    pub ray: f32,
}

/// The shared contract used by streaming, meshing, collision, edits, and
/// player gravity.  `Planet` remains the byte-identical Neisor implementation;
/// `LunarBody` supplies a dry/no-tree column law from `moon.rs`.
pub trait VoxelBody: Send + Sync {
    fn body_id(&self) -> BodyId;
    fn radius_km(&self) -> f64;
    fn seed(&self) -> i64;
    /// Authoritative angular column lattice for this physical body.
    fn columns_per_face(&self) -> u64;
    fn column_ctx(&self, edits: &Edits, face: usize, ci: u64, cj: u64) -> ColCtx;
    fn column_ctx_batch(&self, edits: &Edits, columns: &[(usize, u64, u64)]) -> Vec<ColCtx> {
        columns
            .iter()
            .map(|&(face, ci, cj)| self.column_ctx(edits, face, ci, cj))
            .collect()
    }
    fn ground_height_km(&self, dir: DVec3, exaggeration: f64) -> f64;
    fn tree_at_column(
        &self,
        _edits: &Edits,
        _face: usize,
        _ci: u64,
        _cj: u64,
    ) -> Option<(TreeKind, i64)> {
        None
    }
    fn has_trees(&self) -> bool {
        false
    }
}

#[derive(Clone)]
pub struct LunarBody {
    pub radius_km: f64,
    pub generator: std::sync::Arc<MoonGenerator>,
}

impl LunarBody {
    pub fn new(radius_km: f64, generator: std::sync::Arc<MoonGenerator>) -> Self {
        Self { radius_km, generator }
    }
}

/// Immutable Neisor body snapshot for one structural season bucket. Renderer
/// workers and player physics receive the same value, so an ice top cannot be
/// visible to one system and liquid to the other.
#[derive(Clone)]
pub struct SeasonalPlanet {
    pub planet: std::sync::Arc<Planet>,
    pub season: StructuralSeason,
}

impl SeasonalPlanet {
    pub fn new(planet: std::sync::Arc<Planet>, season: StructuralSeason) -> Self {
        Self { planet, season }
    }
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

    pub fn has_water(&self) -> bool {
        self.water != i64::MIN && self.water > self.top_solid()
    }

    /// Is block z a flooded-cave WATER cell? True for a carved void (cave air
    /// or a dug shaft) at or below the local cave water table — false for solid
    /// rock, for dry cave air above the table, and for open sky above ground.
    pub fn cave_flooded(&self, z: i64) -> bool {
        self.cave_water != i64::MIN && z <= self.cave_water && !self.filled(z)
    }

    /// A below-freezing water column renders as a solid ICE sheet (Mat::Ice,
    /// temp < -4 C — see build_chunk), so physics must treat its surface as
    /// walkable ground, not liquid: without this the player sinks through a
    /// visible ice sheet and swims. Returns the ice-surface block, or None.
    fn frozen_ice(&self) -> Option<i64> {
        if self.has_water() && self.frozen {
            Some(self.water)
        } else {
            None
        }
    }

    fn frozen_cave_ice(&self) -> Option<i64> {
        (self.frozen && self.cave_water != i64::MIN).then_some(self.cave_water)
    }
}

/// The block whose top the water surface RENDERS at for a liquid column:
/// clamped so the surface meets DRY shore neighbours (all 8) flush — the
/// mesher's shoreline rule, shared with the census `--lips` survey so the
/// survey measures exactly what renders. Frozen sheets are walkable and
/// never clamp. `nbs8` is the 8-neighbourhood in any order.
pub fn water_render_top(cc: &ColCtx, nbs8: &[ColCtx; 8]) -> i64 {
    let mut we = cc.water;
    if cc.frozen {
        return we;
    }
    for nb in nbs8 {
        if !nb.has_water() && nb.top_solid() < we {
            we = nb.top_solid();
        }
    }
    we.max(cc.top_solid() + 1)
}

/// Offset from an unclamped lattice top to the shared analog level, excluding
/// the patch lift. The shader removes that lift continuously at the rim.
fn rim_water_analog_delta_km(cc: &ColCtx, rendered_top: i64, exaggeration: f64) -> Option<f64> {
    (cc.rim_flush_water && rendered_top == cc.water).then_some(
        (cc.analog_water_km - rendered_top as f64 * VOXEL_KM) * exaggeration,
    )
}

fn same_rim_water_plane(a: &ColCtx, b: &ColCtx) -> bool {
    a.rim_flush_water
        && b.rim_flush_water
        && (a.analog_water_km - b.analog_water_km).abs() < 1e-9
}

#[cfg(test)]
fn rim_water_top_km(
    cc: &ColCtx,
    rendered_top: i64,
    exaggeration: f64,
    lift: f64,
    flush: f64,
) -> f64 {
    rendered_top as f64 * VOXEL_KM * exaggeration
        + lift
        + rim_water_analog_delta_km(cc, rendered_top, exaggeration).unwrap_or(0.0)
        - lift * flush.clamp(0.0, 1.0)
}

/// Canonical face/column for an extended lattice index. In-range indices keep
/// their identity; out-of-range indices follow the cube-face direction map.
pub fn canonical_column(face: usize, i_ext: i64, j_ext: i64) -> (u8, u64, u64) {
    canonical_column_on_lattice(COLUMNS_PER_FACE, face, i_ext, j_ext)
}

/// Body-parameterized form of [`canonical_column`]. All generic voxel paths
/// use this form; the short wrapper remains the Neisor-only public contract.
pub fn canonical_column_body(
    body: &dyn VoxelBody,
    face: usize,
    i_ext: i64,
    j_ext: i64,
) -> (u8, u64, u64) {
    canonical_column_on_lattice(body.columns_per_face(), face, i_ext, j_ext)
}

pub fn columns_per_face_for_body(body: BodyId) -> u64 {
    match body {
        BodyId::Moon => LUNAR_COLUMNS_PER_FACE,
        BodyId::Sun | BodyId::Neisor => COLUMNS_PER_FACE,
    }
}

fn canonical_column_on_lattice(
    columns_per_face: u64,
    face: usize,
    i_ext: i64,
    j_ext: i64,
) -> (u8, u64, u64) {
    let max = columns_per_face as i64;
    if (0..max).contains(&i_ext) && (0..max).contains(&j_ext) {
        (face as u8, i_ext as u64, j_ext as u64)
    } else {
        let nn = columns_per_face as f64;
        let u = -1.0 + 2.0 * (i_ext as f64 + 0.5) / nn;
        let v = -1.0 + 2.0 * (j_ext as f64 + 0.5) / nn;
        let (f2, u2, v2) = crate::planet::face_from_dir(face_dir(face, u, v));
        let (ci, cj) = column_of_lattice(u2, v2, columns_per_face);
        (f2 as u8, ci, cj)
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
    let (canon_face, ci, cj) = canonical_column(face, i_ext, j_ext);
    col_ctx(planet, edits, canon_face as usize, ci, cj)
}

pub fn col_ctx_ext_body(
    body: &dyn VoxelBody,
    edits: &Edits,
    face: usize,
    i_ext: i64,
    j_ext: i64,
) -> ColCtx {
    let (canon_face, ci, cj) = canonical_column_body(body, face, i_ext, j_ext);
    body.column_ctx(edits, canon_face as usize, ci, cj)
}

pub fn col_ctx_body(
    body: &dyn VoxelBody,
    edits: &Edits,
    face: usize,
    ci: u64,
    cj: u64,
) -> ColCtx {
    body.column_ctx(edits, face, ci, cj)
}

/// Generate one column from scratch: terrain sample, water, caves.
pub fn col_ctx(planet: &Planet, edits: &Edits, face: usize, ci: u64, cj: u64) -> ColCtx {
    col_ctx_at_season(
        planet,
        edits,
        face,
        ci,
        cj,
        StructuralSeason::annual(),
    )
}

pub fn col_ctx_at_season(
    planet: &Planet,
    edits: &Edits,
    face: usize,
    ci: u64,
    cj: u64,
    season: StructuralSeason,
) -> ColCtx {
    let nn = COLUMNS_PER_FACE as f64;
    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
    let s = sample_at_season(planet, face, u, v, VOXEL_OCTAVES, season);
    let ground0 = (s.h_km * 1000.0).floor() as i64;
    let mut water = if s.water_km > s.h_km {
        (s.water_km * 1000.0).floor() as i64
    } else {
        i64::MIN
    };
    let edit = edits.get(&(face as u8, ci, cj)).copied().unwrap_or(0);
    // A placed block targets the visible top. On seasonal water that top is
    // the liquid/ice surface, not the buried bed; anchor positive edits above
    // it so the edit wins across every later freeze/thaw bucket.
    let ground = if edit > 0 && water > ground0 {
        water + edit
    } else {
        ground0 + edit
    };
    let creek_film_sample = |ss: &Sample| {
        ss.water_km > ss.h_km
            && !ss.frozen
            && ss.river_hw_km > 0.0
            && ss.river_hw_km < 0.0015
            && ss.river_dist_km < ss.river_hw_km
            && !ss.sea
            && !ss.lake
    };
    let mut creek_film = creek_film_sample(&s);
    if !creek_film
        && !s.frozen
        && s.river_hw_km > 0.0
        && s.river_hw_km < 0.0015
        && s.river_dist_km < s.river_hw_km + 0.0010
    {
        let step = 2.0 / nn;
        'subcell: for ov in [-0.42, 0.0, 0.42] {
            for ou in [-0.42, 0.0, 0.42] {
                if ou == 0.0 && ov == 0.0 {
                    continue;
                }
                let ss = sample_at_season(
                    planet,
                    face,
                    u + ou * step,
                    v + ov * step,
                    VOXEL_OCTAVES,
                    season,
                );
                if creek_film_sample(&ss) {
                    creek_film = true;
                    break 'subcell;
                }
            }
        }
    }
    if creek_film && water <= ground && ground == ground0 {
        water = ground + 1;
    }

    // caves: tubes along the intersection of two noise level-sets, in dry
    // land only. The radial offset gives the noise true vertical variation.
    // Anchored to the natural ground so edits don't drag the cave band.
    let mut cave_bits = 0u32;
    if water == i64::MIN && ground0 > 4 {
        let dir = face_dir(face, u, v);
        let seed = planet.seed;
        let region = crate::noise::gradient_noise(dir * 90.0, seed.wrapping_add(40961));
        if region > -0.05 {
            // A shoal is structural sediment added specifically to make the
            // equal-block tie walkable. Keep its new cap solid while leaving
            // every underground cave bit deterministic and intact; otherwise
            // shifting the cave band's vertical anchor can turn the repaired
            // one-block pit into a deeper cave mouth at the same column.
            let first = if s.lake_shoal { 1 } else { 0 };
            for k in first..CAVE_DEPTH {
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

    // Flooded caves (BUGS.md W-6): where a carved cave passes below a nearby
    // water table its submerged cells fill with water. The earlier fix instead
    // SUPPRESSED caves near rivers/lakes, because a mouth breaching a bank
    // opened a bone-dry pit with its floor below the water table (photographed
    // at 3.726 63.065). Flooding those cells turns that pit into water, so the
    // suppression is lifted here. The table is a LATERAL groundwater level read
    // from the sample even on a dry bank: the river's graph level within a
    // bank-influence band, a lake's spill level within its shore band, or sea
    // level right on the coast. Groundwater beyond that influence is out of
    // scope (caves there stay dry). Only dry-surface columns carry caves (gate
    // above), so cave water never collides with the single open-air `water`.
    let mut cave_water = i64::MIN;
    if cave_bits != 0 {
        let mut table_km = f64::NEG_INFINITY;
        if s.river_level_km.is_finite() && s.river_dist_km < s.river_hw_km * 3.0 + 0.05 {
            table_km = table_km.max(s.river_level_km);
        }
        if s.lake_level_km.is_finite() && s.h_km < s.lake_level_km + 0.010 {
            table_km = table_km.max(s.lake_level_km);
        }
        if !s.sea && s.e_raw < 0.02 && planet.ocean(face, u, v) as f64 > 0.5 {
            table_km = table_km.max(0.0);
        }
        if table_km.is_finite() {
            let wt = (table_km * 1000.0).floor() as i64;
            // never perch the table ABOVE this column's own surface — a dry
            // column standing below the water line is the sample's perch case,
            // and flooding it would stand water over dry land. Flood only when
            // some carved cell actually sits at/below the (sub-surface) table.
            let floods =
                wt <= ground0 && (0..CAVE_DEPTH).any(|k| (cave_bits >> k) & 1 == 1 && ground0 - k <= wt);
            if floods {
                cave_water = wt;
            }
        }
    }

    let (climate, biome) = if season.enabled {
        climate_surface_with_biome_at_season(
            planet, face, u, v, s.temp_c, s.seasonal_temp_c, s.precip, s.sea, season,
        )
    } else {
        climate_surface_with_biome(planet, face, u, v, s.temp_c, s.precip, s.sea)
    };
    // Tree ownership and species deliberately stay on annual climate. Snow
    // and seasonal tints may repaint a canopy; they never reroll its anchor.
    let vegetation = vegetation_surface(planet, face, u, v, s.temp_c, s.precip);

    // A-5: the graph river is several metres above a local lake here. Keep
    // block occupancy intact, but rendering and swimming share the stable
    // nearby lake plane instead of exposing the river's lattice wall.
    let rim_flush_water = !s.frozen
        && s.has_water()
        && s.lake_level_km.is_finite()
        && s.river_level_km.is_finite()
        && s.river_level_km - s.lake_level_km > 0.002;
    let analog_water_km = if rim_flush_water {
        s.lake_level_km
    } else {
        s.water_km
    };

    ColCtx {
        ground,
        ground0,
        h_km: s.h_km as f32,
        water,
        analog_water_km,
        rim_flush_water,
        cave_bits,
        cave_water,
        climate: Some(climate),
        lunar: None,
        koppen: biome.koppen,
        biome_temp: biome.temp_c,
        biome_precip: biome.precip_mm_yr,
        e_raw: s.e_raw as f32,
        temp: s.temp_c as f32,
        seasonal_temp: s.seasonal_temp_c as f32,
        seasonal_temp_trend: s.seasonal_temp_trend as f32,
        frozen: s.frozen,
        structural_season: season,
        precip: s.precip as f32,
        rough: s.rough as f32,
        carved: s.carve_km > 0.001,
        salt: s.salt,
        sea: s.sea,
        lake_shoal: s.lake_shoal,
        lake_shore_frac: crate::terrain::lake_shore_frac_for_class(
            s.frozen,
            s.h_km,
            s.lake_level_km,
            s.lake_boundary_dist_km,
        ),
        lake_material_region: s.lake_level_km.is_finite(),
        lake_level_band: s.lake_level_km.is_finite()
            && s.h_km >= s.lake_level_km
            && s.h_km - s.lake_level_km <= 0.0015,
        tree_main_block: vegetation.main_block,
        tree_forest: vegetation.forest,
    }
}

fn lunar_col_ctx(
    moon: &LunarBody,
    edits: &Edits,
    face: usize,
    ci: u64,
    cj: u64,
) -> ColCtx {
    let nn = moon.columns_per_face() as f64;
    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
    let direction = face_dir(face, u, v);
    let sample = moon.generator.sample(direction);
    lunar_col_ctx_from_sample(moon, edits, face, ci, cj, sample)
}

fn lunar_col_ctx_from_sample(
    moon: &LunarBody,
    edits: &Edits,
    face: usize,
    ci: u64,
    cj: u64,
    sample: MoonSample,
) -> ColCtx {
    let h_km = sample.height_ratio * moon.radius_km;
    let ground0 = (h_km * 1000.0).floor() as i64;
    let ground = ground0 + edits.get(&(face as u8, ci, cj)).copied().unwrap_or(0);
    ColCtx {
        ground,
        ground0,
        h_km: h_km as f32,
        water: i64::MIN,
        analog_water_km: f64::NEG_INFINITY,
        rim_flush_water: false,
        cave_bits: 0,
        cave_water: i64::MIN,
        climate: None,
        lunar: Some(LunarColumn {
            material: sample.material(),
            albedo: sample.albedo as f32,
            smoothness: sample.smoothness as f32,
            ray: sample.ray as f32,
        }),
        koppen: 255,
        biome_temp: 0.0,
        biome_precip: 0.0,
        tree_main_block: MainBlock::Snow,
        tree_forest: 0.0,
        e_raw: h_km as f32,
        temp: 999.0,
        seasonal_temp: 999.0,
        seasonal_temp_trend: 0.0,
        frozen: false,
        structural_season: StructuralSeason::annual(),
        precip: 0.0,
        rough: (1.0 - sample.smoothness) as f32,
        carved: false,
        salt: false,
        sea: false,
        lake_shoal: false,
        lake_shore_frac: 0.0,
        lake_material_region: false,
        lake_level_band: false,
    }
}

impl VoxelBody for Planet {
    fn body_id(&self) -> BodyId { BodyId::Neisor }
    fn radius_km(&self) -> f64 { self.radius_km }
    fn seed(&self) -> i64 { self.seed }
    fn columns_per_face(&self) -> u64 { COLUMNS_PER_FACE }
    fn column_ctx(&self, edits: &Edits, face: usize, ci: u64, cj: u64) -> ColCtx {
        col_ctx(self, edits, face, ci, cj)
    }
    fn ground_height_km(&self, dir: DVec3, exaggeration: f64) -> f64 {
        crate::terrain::ground_height_km(self, dir, exaggeration)
    }
    fn tree_at_column(
        &self,
        edits: &Edits,
        face: usize,
        ci: u64,
        cj: u64,
    ) -> Option<(TreeKind, i64)> {
        tree_here(self, edits, face, ci, cj)
    }
    fn has_trees(&self) -> bool { true }
}

impl VoxelBody for SeasonalPlanet {
    fn body_id(&self) -> BodyId { BodyId::Neisor }
    fn radius_km(&self) -> f64 { self.planet.radius_km }
    fn seed(&self) -> i64 { self.planet.seed }
    fn columns_per_face(&self) -> u64 { COLUMNS_PER_FACE }
    fn column_ctx(&self, edits: &Edits, face: usize, ci: u64, cj: u64) -> ColCtx {
        col_ctx_at_season(&self.planet, edits, face, ci, cj, self.season)
    }
    fn ground_height_km(&self, dir: DVec3, exaggeration: f64) -> f64 {
        crate::terrain::ground_height_km_at_season(&self.planet, dir, exaggeration, self.season)
    }
    fn tree_at_column(
        &self,
        edits: &Edits,
        face: usize,
        ci: u64,
        cj: u64,
    ) -> Option<(TreeKind, i64)> {
        // Positions are intentionally annual and season-independent.
        tree_here(&self.planet, edits, face, ci, cj)
    }
    fn has_trees(&self) -> bool { true }
}

impl VoxelBody for LunarBody {
    fn body_id(&self) -> BodyId { BodyId::Moon }
    fn radius_km(&self) -> f64 { self.radius_km }
    fn seed(&self) -> i64 { self.generator.seed() }
    fn columns_per_face(&self) -> u64 { LUNAR_COLUMNS_PER_FACE }
    fn column_ctx(&self, edits: &Edits, face: usize, ci: u64, cj: u64) -> ColCtx {
        lunar_col_ctx(self, edits, face, ci, cj)
    }
    fn column_ctx_batch(&self, edits: &Edits, columns: &[(usize, u64, u64)]) -> Vec<ColCtx> {
        let directions: Vec<DVec3> = columns
            .iter()
            .map(|&(face, ci, cj)| dir_of_column_body(self, face, ci, cj))
            .collect();
        self.generator
            .sample_batch(&directions)
            .into_iter()
            .zip(columns.iter().copied())
            .map(|(sample, (face, ci, cj))| {
                lunar_col_ctx_from_sample(self, edits, face, ci, cj, sample)
            })
            .collect()
    }
    fn ground_height_km(&self, dir: DVec3, _exaggeration: f64) -> f64 {
        self.generator.height_km(dir, self.radius_km)
    }
}

/// Surface material given local steepness (max |height delta| to neighbors).
/// Climate-owned grass/sand/snow selection is shared with the far mesh;
/// hydrology and local slope remain higher-priority local facts. Generic
/// coastal beach sand uses the fractal ecotone comparator; the separately-
/// approved V-7 lake shore retains its established per-column dither. A lake
/// territory yields generic beach ownership to the shared lake-shore fraction
/// so a near-sea-level lake cannot fall through and become a sand province.
fn surface_mat(
    c: &ColCtx,
    steep: i64,
    climate_block: MainBlock,
    lake_jitter: f64,
    coastal_comparator: f64,
) -> Mat {
    // underwater floors
    if c.water != i64::MIN && c.ground < c.water {
        return if c.water - c.ground > 4 { Mat::Gravel } else { Mat::Sand };
    }
    // The analog point is submerged even though no distinct liquid cell can
    // coexist with its equal-block ground. Field-tested correction (Andrew,
    // 2026-07-11): water-colored walkable caps read as broken water - the
    // player can stand on 'water' they cannot swim in. Shoals now wear SAND:
    // an honest sandbar archipelago flush with the surface, connecting with
    // the lake-shore sand band.
    if c.lake_shoal {
        return Mat::Sand;
    }
    if lake_jitter < c.lake_shore_frac {
        return Mat::Sand;
    }
    if climate_block == MainBlock::Snow {
        return Mat::Snow;
    }
    if steep >= 5 {
        return Mat::Rock;
    }
    if steep >= 3 {
        return Mat::Stone;
    }
    // beaches: low ground near sea level, dithered on the SHARED fraction
    // (natural ground: a tower built on a beach stays a sand tower)
    if crate::terrain::coastal_beach_sand(
        c.e_raw as f64,
        c.h_km as f64,
        c.lake_material_region,
        coastal_comparator,
    ) {
        return Mat::Sand;
    }
    match climate_block {
        MainBlock::Sand => Mat::Sand,
        MainBlock::Grass => Mat::Grass,
        MainBlock::Snow => Mat::Snow, // handled above; exhaustive by contract
    }
}

/// Whether this exact canonical column wears generic coastal beach sand.
/// Surface materials, voxel trees, probes, and mesh impostors all call this
/// predicate, so vegetation cannot survive on a sand patch or stop at the
/// retired raw-elevation line.
pub fn coastal_beach_at(c: &ColCtx, face: u8, ci: u64, cj: u64, seed: i64) -> bool {
    crate::terrain::coastal_beach_sand(
        c.e_raw as f64,
        c.h_km as f64,
        c.lake_material_region,
        crate::terrain::beach_blend_comparator(face, ci, cj, seed, c.rough as f64),
    )
}

fn mat_main_block(mat: Mat, fallback: MainBlock) -> MainBlock {
    match mat {
        Mat::Grass => MainBlock::Grass,
        Mat::Sand => MainBlock::Sand,
        Mat::Snow => MainBlock::Snow,
        _ => fallback,
    }
}

fn sub_mat(surface: Mat) -> Mat {
    match surface {
        Mat::Sand => Mat::Sand,
        Mat::Snow => Mat::Dirt,
        Mat::Gravel => Mat::Gravel,
        Mat::Rock => Mat::Rock,
        Mat::Stone => Mat::Stone,
        Mat::RegolithHighland | Mat::RegolithMaria | Mat::RegolithRay => Mat::RegolithSubsurface,
        Mat::RegolithSubsurface => Mat::RegolithSubsurface,
        Mat::LunarRock => Mat::LunarRock,
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
    // depth below the NATURAL surface — strata always measure from ground0,
    // never from a dug-down top. Using the lowered top made a mined shaft
    // floor (z == c.ground) compute depth 0 and render as living grass several
    // meters underground; from ground0 it correctly reads dirt then stone.
    let d = c.ground0 - z;
    if c.lunar.is_some() {
        if d <= 0 {
            surface
        } else if d <= 4 {
            Mat::RegolithSubsurface
        } else {
            Mat::LunarRock
        }
    } else if d <= 0 {
        surface
    } else if d <= 3 {
        sub_mat(surface)
    } else {
        Mat::Stone
    }
}

fn lunar_surface_mat(column: LunarColumn) -> Mat {
    match column.material {
        MoonMaterial::Highland => Mat::RegolithHighland,
        MoonMaterial::Maria => Mat::RegolithMaria,
        MoonMaterial::Ray => Mat::RegolithRay,
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

/// Species + closed-canopy density per Köppen class — the ONE tree
/// distribution both renderers draw from (voxel trees here, mesh-tile
/// billboard impostors in terrain::build_tile; TRANSITIONS.md E).
/// Densities are per-column; one canopy covers ~25 columns, so 0.010 is
/// already a closed-canopy forest.
pub const MAX_TREE_DENSITY: f64 = 0.011;
/// Annual-mean treeline shared by voxel trees and mesh impostors. Candidate
/// enumeration may hoist this exact gate because seasonal state never changes
/// tree ownership or anchors.
pub const TREE_MIN_TEMP_C: f64 = -6.0;

pub fn tree_kind_density(koppen: u8) -> Option<(TreeKind, f64)> {
    match koppen {
        0 | 1 => Some((TreeKind::Jungle, 0.011)),
        2 => Some((TreeKind::Acacia, 0.0012)),
        5 => Some((TreeKind::Acacia, 0.0005)),
        6 => Some((TreeKind::Shrub, 0.0015)),
        7 | 8 | 9 => Some((TreeKind::Broadleaf, 0.0025)),
        10..=15 | 20 | 21 | 24 | 25 => Some((TreeKind::Broadleaf, 0.005)),
        16..=19 => Some((TreeKind::Conifer, 0.003)),
        22 | 23 | 26 | 27 => Some((TreeKind::Conifer, 0.007)),
        28 => Some((TreeKind::Shrub, 0.002)),
        _ => None, // deserts, ice cap, ocean
    }
}

/// Continuous forest signature -> established tree density. Every knot is an
/// existing biome interior, so joining the field does not globally retune
/// forests: only the space between those signatures changes continuously.
fn forest_tree_density(forest: f64) -> f64 {
    const KNOTS: [(f64, f64); 7] = [
        (0.0, 0.0),
        (0.15, 0.0012),
        (0.30, 0.0025),
        (0.40, 0.0030),
        (0.50, 0.0050),
        (0.60, 0.0070),
        (0.85, MAX_TREE_DENSITY),
    ];
    let forest = forest.clamp(0.0, 1.0);
    for pair in KNOTS.windows(2) {
        let (x0, y0) = pair[0];
        let (x1, y1) = pair[1];
        if forest <= x1 {
            let t = (forest - x0) / (x1 - x0);
            return y0 + (y1 - y0) * t;
        }
    }
    MAX_TREE_DENSITY
}

/// Species and density on the same warped/dithered field the ground uses.
///
/// `main_block` is the categorical fractal ecotone result: sand and snow
/// patches cannot grow a tree even when their nearest Koppen texel is wooded.
/// `forest` is the continuous signature at the same warped position, converted
/// to a density curve calibrated to retain the established biome interiors.
/// Existing vegetated classes retain their established species; continuous
/// warped climate supplies a species only on a dithered grass island whose
/// nearest class is barren. Classes whose forest signature is genuinely zero
/// keep their old sparse acacia/shrub populations.
pub fn tree_biome_profile(
    koppen: u8,
    main_block: MainBlock,
    forest: f32,
    biome_temp_c: f32,
    biome_precip_mm: f32,
) -> Option<(TreeKind, f64)> {
    if main_block != MainBlock::Grass {
        return None;
    }
    let forest = f64::from(forest.clamp(0.0, 1.0));
    if forest > 1e-4 {
        let inherited = tree_kind_density(koppen);
        let sparse = match koppen {
            5 | 6 | 28 => inherited,
            _ => None,
        };
        let forest_density = forest_tree_density(forest);
        if sparse.is_some_and(|(_, density)| density >= forest_density) {
            return sparse;
        }
        let climate_kind = if biome_temp_c < 5.0 {
            TreeKind::Conifer
        } else if biome_temp_c > 20.0 && biome_precip_mm > 1_400.0 {
            TreeKind::Jungle
        } else if biome_temp_c > 16.0 && biome_precip_mm < 1_400.0 {
            TreeKind::Acacia
        } else {
            TreeKind::Broadleaf
        };
        // Preserve established species wherever the warped nearest class is
        // already vegetated. Continuous climate supplies only the missing
        // species for a dithered grass island on the nominally barren side;
        // shrubs likewise hand over when a real forest signal overtakes them.
        let kind = match inherited {
            Some((TreeKind::Shrub, _)) | None => climate_kind,
            Some((kind, _)) => kind,
        };
        return Some((kind, forest_density));
    }
    tree_kind_density(koppen)
}

#[inline]
fn tree_profile(c: &ColCtx) -> Option<(TreeKind, f64)> {
    c.climate?;
    tree_biome_profile(
        c.koppen,
        c.tree_main_block,
        c.tree_forest,
        c.biome_temp,
        c.biome_precip,
    )
}

/// The placement lottery ticket for a column (uniform 0..1, compared
/// against the biome density). Public so impostors run the SAME lottery.
pub fn tree_hash01(face: u8, ci: u64, cj: u64, seed: i64) -> f64 {
    hash01(face, ci, cj, seed as u64 ^ 0x7255)
}

/// Trunk height for a winning column — shared with impostors so a tree's
/// silhouette height survives the voxel<->billboard handoff.
pub fn tree_trunk(kind: TreeKind, face: u8, ci: u64, cj: u64) -> i64 {
    let h_var = (hash01(face, ci, cj, 0x9E11) * 3.0) as i64;
    match kind {
        TreeKind::Conifer => 4 + h_var,
        TreeKind::Broadleaf => 3 + h_var,
        TreeKind::Jungle => 6 + h_var,
        TreeKind::Acacia => 3 + (h_var).min(1),
        TreeKind::Shrub => 0,
    }
}

/// Deterministic tree placement with a density multiplier for strided mesh
/// impostors. Scale 1 is voxel truth; a stride-s impostor represents s^2
/// columns and therefore uses scale s^2 against the same lottery/profile.
pub fn tree_at_scaled(
    c: &ColCtx,
    face: u8,
    ci: u64,
    cj: u64,
    seed: i64,
    density_scale: f64,
) -> Option<(TreeKind, i64)> {
    // No trees in water, on actual beach sand, in edited/cave-mouth columns,
    // or in river/pond carved ground (a canopy anchored in a gully pokes out
    // at rim level as leaf shards). The separately-approved lake vegetation
    // band stays unchanged; this mission changes generic coastal beaches.
    if c.has_water()
        || c.water != i64::MIN
        || c.lake_level_band
        || c.carved
        || c.ground != c.ground0
        || c.top_solid() != c.ground
        || coastal_beach_at(c, face, ci, cj, seed)
    {
        return None;
    }
    let Some((kind, density)) = tree_profile(c) else {
        return None;
    };
    // treeline: shrubs shiver on, trees give up
    if c.temp < TREE_MIN_TEMP_C as f32 && kind != TreeKind::Shrub {
        return None;
    }
    if c.temp < -11.0 {
        return None;
    }
    if tree_hash01(face, ci, cj, seed) >= density * density_scale {
        return None;
    }
    Some((kind, tree_trunk(kind, face, ci, cj)))
}

/// Voxel tree lottery (density scale 1). Public because probes and the voxel
/// mesher share this exact single-column decision with forest impostors.
pub fn tree_at(c: &ColCtx, face: u8, ci: u64, cj: u64, seed: i64) -> Option<(TreeKind, i64)> {
    tree_at_scaled(c, face, ci, cj, seed, 1.0)
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

pub fn column_of(u: f64, v: f64) -> (u64, u64) {
    column_of_lattice(u, v, COLUMNS_PER_FACE)
}

fn column_of_lattice(u: f64, v: f64, columns_per_face: u64) -> (u64, u64) {
    let n = columns_per_face as f64;
    let ci = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let cj = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    (ci, cj)
}

pub fn column_of_body(body: &dyn VoxelBody, u: f64, v: f64) -> (u64, u64) {
    column_of_lattice(u, v, body.columns_per_face())
}

/// Unit direction through the center of a body's canonical column.
pub fn dir_of_column_body(body: &dyn VoxelBody, face: usize, ci: u64, cj: u64) -> DVec3 {
    let n = body.columns_per_face() as f64;
    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / n;
    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / n;
    face_dir(face, u, v)
}

/// The face/column the given direction points at — the column identity used
/// by edits, chunk keys, and the "which column am I standing in" test.
pub fn column_id(dir: DVec3) -> (u8, u64, u64) {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of(u, v);
    (face as u8, ci, cj)
}

pub fn column_id_body(body: &dyn VoxelBody, dir: DVec3) -> (u8, u64, u64) {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of_body(body, u, v);
    (face as u8, ci, cj)
}

/// Height of the *solid* walkable surface (km, exaggerated, incl. the patch
/// lift) under a direction. Water is NOT walkable: you wade into ponds and
/// sink through rivers to their floor. Mirrors build_chunk's shell()/lift
/// so feet match the visible voxel tops.
pub fn surface_height_km(body: &dyn VoxelBody, edits: &Edits, dir: DVec3, exaggeration: f64) -> f64 {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of_body(body, u, v);
    let c = col_ctx_body(body, edits, face, ci, cj);
    // a frozen sheet is solid to EVERY world query, same rule as
    // support_below_km: aiming, placing, and torch height must see the ice
    // a player stands on, not the seabed beneath it
    let top = c.top_solid().max(c.frozen_ice().unwrap_or(i64::MIN));
    top as f64 * VOXEL_KM * exaggeration + lift_km(exaggeration)
}

/// Highest solid block top at or below `at_km` in the column under `dir`
/// (same exaggerated+lift height units as surface_height_km). Cave-aware:
/// standing over a pit this is the pit floor, inside a tunnel the tunnel
/// floor — the physics query for gravity, landing, and step-up.
pub fn support_below_km(
    body: &dyn VoxelBody,
    edits: &Edits,
    dir: DVec3,
    at_km: f64,
    exaggeration: f64,
) -> f64 {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of_body(body, u, v);
    let c = col_ctx_body(body, edits, face, ci, cj);
    // tree trunks are solid (shrubs are not) — you bump into and can stand
    // on trunks; canopy leaves stay passable
    let trunk_top = body.tree_at_column(edits, face, ci, cj)
        .filter(|(k, _)| *k != TreeKind::Shrub)
        .map(|(_, t)| c.ground + t);
    // frozen water is a solid ice sheet you stand ON (at the water surface)
    let ice = c.frozen_ice().or_else(|| c.frozen_cave_ice());
    let solid = |z: i64| {
        c.filled(z)
            || trunk_top.is_some_and(|t| z > c.ground && z <= t)
            || ice == Some(z)
    };
    let scale = VOXEL_KM * exaggeration;
    let lift = lift_km(exaggeration);
    let mut z = (((at_km - lift) / scale) + 1e-7).floor() as i64;
    z = z.min(
        trunk_top
            .unwrap_or(c.ground)
            .max(c.ground)
            .max(ice.unwrap_or(i64::MIN)),
    );
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
    body: &dyn VoxelBody,
    edits: &Edits,
    dir: DVec3,
    at_km: f64,
    exaggeration: f64,
) -> f64 {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of_body(body, u, v);
    let c = col_ctx_body(body, edits, face, ci, cj);
    let trunk_top = body.tree_at_column(edits, face, ci, cj)
        .filter(|(k, _)| *k != TreeKind::Shrub)
        .map(|(_, t)| c.ground + t);
    // frozen ice is a ceiling too: swimming up under a frozen sheet must
    // collide with it, not pass through into the "solid" ice
    let ice = c.frozen_ice().or_else(|| c.frozen_cave_ice());
    let solid = |z: i64| {
        c.filled(z) || trunk_top.is_some_and(|t| z > c.ground && z <= t) || ice == Some(z)
    };
    let scale = VOXEL_KM * exaggeration;
    let lift = lift_km(exaggeration);
    // first block whose span could sit above at_km
    let mut z = (((at_km - lift) / scale) - 1e-7).floor() as i64 + 1;
    z = z.max(c.ground - CAVE_DEPTH);
    let z_top = trunk_top.unwrap_or(c.ground).max(c.ground).max(ice.unwrap_or(i64::MIN));
    while z <= z_top {
        if solid(z) {
            return (z - 1) as f64 * scale + lift;
        }
        z += 1;
    }
    f64::INFINITY
}

/// Water surface height (km, exaggerated, incl. lift) the point `at_km` in the
/// column under `dir` is submerged beneath, if any — for wading/underwater.
///
/// The OPEN-AIR body (sea/river/lake/pond) is reported whenever the column
/// holds one, height-independent (so a camera hovering just above a lake still
/// reads "water here", as the surveys assert). A FLOODED CAVE (BUGS.md W-6) is
/// an underground pool capped by rock, so it is reported only when `at_km`
/// actually sits at/below its table — otherwise a player standing on dry land
/// over a flooded tunnel would read as swimming, and a dry bank near a lake
/// would spuriously gain a water surface.
pub fn water_surface_km(
    body: &dyn VoxelBody,
    edits: &Edits,
    dir: DVec3,
    at_km: f64,
    exaggeration: f64,
) -> Option<f64> {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (ci, cj) = column_of_body(body, u, v);
    let c = col_ctx_body(body, edits, face, ci, cj);
    let scale = VOXEL_KM * exaggeration;
    let lift = lift_km(exaggeration);
    // frozen columns are solid ice (walkable, handled by support_below_km),
    // NOT liquid — so wading/underwater physics must not see water here.
    if c.has_water() && c.frozen_ice().is_none() {
        if c.rim_flush_water {
            return Some(c.analog_water_km * exaggeration + lift);
        }
        return Some(c.water as f64 * scale + lift);
    }
    if c.cave_water != i64::MIN && c.frozen_cave_ice().is_none() {
        let surf = c.cave_water as f64 * scale + lift;
        if at_km < surf {
            return Some(surf);
        }
    }
    None
}

/// March along the look ray until it dips below the walkable surface;
/// returns (hit column, last air column before it). Step is a third of a
/// block. The air column is where a placed block belongs: aiming at the
/// side of a tower builds next to it instead of pushing the tower up.
pub fn raycast_column(
    body: &dyn VoxelBody,
    edits: &Edits,
    eye_km: DVec3,
    look: DVec3,
    max_m: f64,
    exaggeration: f64,
) -> Option<((u8, u64, u64), (u8, u64, u64))> {
    let col_under = |dir: DVec3| {
        let (face, u, v) = crate::planet::face_from_dir(dir);
        let (ci, cj) = column_of_body(body, u, v);
        (face as u8, ci, cj)
    };
    let mut prev = col_under(eye_km.normalize());
    let mut t_m = 0.4;
    while t_m < max_m {
        let p = eye_km + look * (t_m / 1000.0);
        let dir = p.normalize();
        let surf_r = body.radius_km() + surface_height_km(body, edits, dir, exaggeration);
        let col = col_under(dir);
        if p.length() <= surf_r {
            return Some((col, prev));
        }
        prev = col;
        t_m += 0.33;
    }
    None
}

/// Chunks whose ghost/tree context can observe a column need remeshing on edit.
pub fn chunks_touching_column(face: u8, ci: u64, cj: u64) -> Vec<ChunkKey> {
    chunks_touching_column_body(BodyId::Neisor, face, ci, cj)
}

pub fn chunks_touching_column_body(body: BodyId, face: u8, ci: u64, cj: u64) -> Vec<ChunkKey> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let (i, j) = (ci as i64, cj as i64);
    let mut push_key = |face, ci, cj| {
        let key = ChunkKey {
            body,
            face,
            cx: ci / CHUNK,
            cy: cj / CHUNK,
        };
        if seen.insert(key) {
            out.push(key);
        }
    };
    push_key(face, ci, cj);
    for dj in -TREE_MARGIN..=TREE_MARGIN {
        for di in -TREE_MARGIN..=TREE_MARGIN {
            let (canon_face, ci, cj) = canonical_column_on_lattice(
                columns_per_face_for_body(body),
                face as usize,
                i + di,
                j + dj,
            );
            push_key(canon_face, ci, cj);
        }
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
pub fn select_chunks(cam_pos: DVec3, body: &dyn VoxelBody, radius_m: f64) -> Vec<ChunkKey> {
    let dir = cam_pos.normalize();
    let ref_axis = if dir.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let t1 = (ref_axis - dir * ref_axis.dot(dir)).normalize();
    let t2 = dir.cross(t1);
    let n = body.columns_per_face() as f64;
    // chunk size shrinks toward cube-face edges and corners (the gnomonic
    // cell's short axis scales as 1/(1+u^2+v^2): half size at edge middles,
    // a third at corners). Sample at 0.45x the local worst case so every
    // chunk overlapping the disc is hit — a fixed step tuned for the face
    // center skipped chunks near the edges, punching see-through holes in
    // the patch.
    let (_, u0, v0) = crate::planet::face_from_dir(dir);
    let chunk_min_km =
        CHUNK as f64 * (2.0 / n) * body.radius_km() / (1.0 + u0 * u0 + v0 * v0);
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
            let p = (dir * body.radius_km() + t1 * dx + t2 * dy).normalize();
            let (face, u, v) = crate::planet::face_from_dir(p);
            let ci = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
            let cj = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
            let key = ChunkKey {
                body: body.body_id(),
                face: face as u8,
                cx: ci / CHUNK,
                cy: cj / CHUNK,
            };
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
/// water surfaces, trees, and player-placed torches.
pub fn build_chunk(
    body: &dyn VoxelBody,
    edits: &Edits,
    torches: &Torches,
    key: ChunkKey,
    exaggeration: f64,
) -> TileMesh {
    let n = CHUNK as i64;
    let face = key.face as usize;
    let nn = body.columns_per_face() as f64;
    let u_of = |i: i64| -1.0 + 2.0 * i as f64 / nn;
    let v_of = |j: i64| -1.0 + 2.0 * j as f64 / nn;

    // column contexts for the chunk plus TREE_MARGIN on each side. Columns
    // past the face edge resolve to the neighbor face via the extended
    // lattice direction — cube-face lattices coincide along shared edges.
    let m = TREE_MARGIN;
    let np = (n + 2 * m) as usize;
    let base_i = key.cx as i64 * n;
    let base_j = key.cy as i64 * n;
    let mut column_ids = Vec::with_capacity(np * np);
    for gj in 0..np as i64 {
        for gi in 0..np as i64 {
            let (canon_face, ci, cj) = canonical_column_body(
                body,
                face,
                base_i + gi - m,
                base_j + gj - m,
            );
            column_ids.push((canon_face as usize, ci, cj));
        }
    }
    // LunarBody overrides this batch to enumerate crater/ray candidates once
    // for the complete chunk footprint. Neisor's default preserves the exact
    // established per-column path.
    let cols = body.column_ctx_batch(edits, &column_ids);
    let at = |gi: i64, gj: i64| -> &ColCtx { &cols[(gj + m) as usize * np + (gi + m) as usize] };

    let radius = body.radius_km();
    let lift = lift_km(exaggeration);
    let origin_dir = face_dir(face, u_of(base_i + n / 2), v_of(base_j + n / 2));
    let origin = origin_dir * radius;
    let shell = |z: i64| radius + (z as f64) * VOXEL_KM * exaggeration + lift;

    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    // Optional per-corner analog offset and lift-removal weight for marked
    // A-5 water quads. Ordinary block vertices retain the old zero/dim pair.
    let quad_rim_delta = std::cell::Cell::new([0.0f32; 4]);
    let quad_rim_weight = std::cell::Cell::new(None::<[f32; 4]>);
    // The fourth byte of the packed range-color attribute is spare on every
    // vertex. Carry D-8's signed trough/peak residual there so blocks and the
    // heightfield consume the same rain-interpolation channel without growing
    // the established 72-byte vertex or GPU tile cache.
    let quad_rain_concavity = std::cell::Cell::new(0.0f32);
    let quad_lunar_surface = std::cell::Cell::new(None::<LunarColumn>);
    // per-corner colors: ambient occlusion darkens individual corners.
    // `dim` is the cave-darkness factor (1 = open sky): blocks carry it in
    // the water attribute's alpha so the shader — not the bake — applies
    // it, letting the player's torch light the rock back up near them.
    let mut quad = |corners: [DVec3; 4], normal: DVec3, cols: [[f32; 3]; 4], dim: f32, wflag: f32| {
        let base = vertices.len() as u32;
        for (k, c) in corners.iter().enumerate() {
            let p = *c - origin;
            vertices.push(Vertex {
                pos: [p.x as f32, p.y as f32, p.z as f32],
                normal: [normal.x as f32, normal.y as f32, normal.z as f32],
                color: cols[k],
                water: [
                    0.0,
                    0.0,
                    0.0,
                    quad_lunar_surface.get().map_or(dim, |l| l.smoothness),
                ],
                morph_dh: quad_rim_delta.get()[k],
                morph_wet: quad_rim_weight.get().map_or(dim, |weights| weights[k]),
                // 1.0 marks OPEN WATER surfaces: the shader's cold-dusting and
                // rain-darkening skip them (snow does not settle on liquid) -
                // block water dusted while the mesh's wet-mix masked it, a
                // +4 lum whole-sea divergence (review #2 aftermath)
                wflag: quad_lunar_surface.get().map_or(wflag, |l| l.ray),
                shore: -1.0, // blocks ARE the exact shoreline already
                biome: NO_BIOME_PAYLOAD,
                // payload-off, but beach.w still carries the D-8 rain
                // concavity (UNORM c/2 + 0.5) - block quads rain-bias too
                beach: [
                    0,
                    0,
                    0,
                    ((quad_rain_concavity.get().clamp(-1.0, 1.0) * 0.5 + 0.5)
                        * 255.0)
                        .round() as u8,
                ],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    let vary = |c: [f32; 3], t: f32| [c[0] * t, c[1] * t, c[2] * t];

    for j in 0..n {
        for i in 0..n {
            let c = at(i, j);
            quad_lunar_surface.set(c.lunar);
            quad_rain_concavity.set(if c.lunar.is_some() {
                0.0
            } else {
                crate::terrain::rain_concavity_proxy(c.e_raw as f64, c.h_km as f64)
            });
            let ci = (base_i + i) as u64;
            let cj = (base_j + j) as u64;
            let (u0, u1) = (u_of(base_i + i), u_of(base_i + i + 1));
            let (v0, v1) = (v_of(base_j + j), v_of(base_j + j + 1));
            let d00 = face_dir(face, u0, v0);
            let d10 = face_dir(face, u1, v0);
            let d11 = face_dir(face, u1, v1);
            let d01 = face_dir(face, u0, v1);
            let up = origin_dir;
            // `col_ctx` caches the exact climate surface because trees and
            // materials must consume the same warped/dithered field.
            let climate = c.climate;

            // Per-column surface normal for the block top. Without slope
            // self-shading the only relief cue is the baked-dark terrace risers,
            // which alias into fall-line light/dark stripes on any smooth slope.
            //
            // The gradient is taken from the CONTINUOUS terrain height (h_km) —
            // the same surface the far-LOD mesh shades with — not the quantized
            // block heights. Quantized central differences jump a whole block
            // across each 1-block terrace edge: on a GENTLE slope that one-column
            // ring tilts ~27 deg (at exagg 1) while the flat terrace tops between
            // stay radial-bright, reading as dark concentric contour rings. The
            // continuous gradient is smooth and small there (no rings) yet stays
            // large on steep ground (slope self-shading — the stripe fix — kept).
            // h_km is already in km, so its vertical scale drops the VOXEL_KM the
            // block-count path needs; both match shell()'s radial lift.
            //
            // h_km knows nothing of player edits or surface cave breaches, so any
            // column whose 4-neighbourhood is edited or carved-at-surface
            // (top_solid != ground0) falls back to the quantized difference —
            // towers, holes and pit rims still shade from real geometry.
            // Flat ground gives dz=0 -> radial normal.
            let top_n = {
                let r_top = shell(c.top_solid());
                let ei = (d10 - d00) * r_top; // +i horizontal world edge
                let ej = (d01 - d00) * r_top; // +j horizontal world edge
                let warped = [c, at(i + 1, j), at(i - 1, j), at(i, j + 1), at(i, j - 1)]
                    .iter()
                    .any(|k| k.top_solid() != k.ground0);
                let (dzi, dzj) = if warped {
                    let sc = 0.5 * VOXEL_KM * exaggeration; // block counts -> km
                    (
                        (at(i + 1, j).top_solid() - at(i - 1, j).top_solid()) as f64 * sc,
                        (at(i, j + 1).top_solid() - at(i, j - 1).top_solid()) as f64 * sc,
                    )
                } else {
                    let sc = 0.5 * exaggeration; // h_km is already km
                    (
                        (at(i + 1, j).h_km - at(i - 1, j).h_km) as f64 * sc,
                        (at(i, j + 1).h_km - at(i, j - 1).h_km) as f64 * sc,
                    )
                };
                let mut nrm = (ei + up * dzi).cross(ej + up * dzj);
                if nrm.dot(up) < 0.0 {
                    nrm = -nrm;
                }
                let nrm = nrm.normalize_or_zero();
                if nrm.length_squared() > 0.5 {
                    nrm
                } else {
                    up
                }
            };

            let nbs = [at(i + 1, j), at(i - 1, j), at(i, j + 1), at(i, j - 1)];
            // natural steepness: player towers are not cliffs
            let steep = nbs
                .iter()
                .map(|nb| (c.ground0 - nb.ground0).abs())
                .max()
                .unwrap_or(0);
            let (surf, tint) = if let Some(lunar) = c.lunar {
                (lunar_surface_mat(lunar), [lunar.albedo; 3])
            } else {
                let climate = climate.expect("Neisor column must carry climate");
                let surf = surface_mat(
                    c,
                    steep,
                    climate.main_block,
                    hash01(face as u8, ci, cj, 0xBEAC),
                    crate::terrain::beach_blend_comparator(
                        face as u8,
                        ci,
                        cj,
                        body.seed(),
                        c.rough as f64,
                    ),
                );
                (surf, climate.tint(mat_main_block(surf, climate.main_block)))
            };

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
                // shoal caps darken slightly: wet sand at the waterline
                let base = if c.lake_shoal && mat == Mat::Sand {
                    let c0 = mat.color(tint);
                    [c0[0] * 0.82, c0[1] * 0.82, c0[2] * 0.84]
                } else {
                    mat.color(tint)
                };
                let col = vary(base, bright(z));
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
                    // slope-lit only on the real surface top; buried exposed-up
                    // faces (cave floors) keep the radial normal
                    let n_top = if z == c.top_solid() { top_n } else { up };
                    quad([d00 * r, d10 * r, d11 * r, d01 * r], n_top, cols, cave, 0.0);
                }
                if z > z_lo && !c.filled(z - 1) {
                    let r = shell(z - 1);
                    let cdark = vary(col, 0.55);
                    quad([d00 * r, d01 * r, d11 * r, d10 * r], -up, [cdark; 4], cave, 0.0);
                }
            }
            // True horizontal axes of THIS column, for face normals. The old
            // `(da+db) - up` derivation left the face's POSITION relative to
            // the chunk center, not its orientation: the ~0.7 m face offset
            // drowned under up to ~28 m of column offset, so side normals
            // swept radially away from each chunk's center — same-orientation
            // faces lit differently by where they sat (Austin's annotated
            // one-block step), patterns repeating on the 32-column chunk
            // grid, and water/tree faces erratic for the same reason.
            let ei_dir = {
                let d = d10 - d00;
                (d - up * d.dot(up)).normalize_or_zero()
            };
            let ej_dir = {
                let d = d01 - d00;
                (d - up * d.dot(up)).normalize_or_zero()
            };
            // sides: contiguous exposed runs per neighbor, split by material
            let sides = [
                (0usize, d10, d11), // +i
                (1usize, d01, d00), // -i
                (2usize, d11, d01), // +j
                (3usize, d00, d10), // -j
            ];
            let out_dirs = [ei_dir, -ei_dir, ej_dir, -ej_dir];
            // air-side column offsets per side: the facing column, and the
            // lateral columns diagonal to the da/db corners — the occluders
            // for wall-face ambient occlusion (below)
            let nb_off = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)];
            let lat_off: [[(i64, i64); 2]; 4] = [
                [(1, -1), (1, 1)],   // +i: da is the lower-j corner
                [(-1, 1), (-1, -1)], // -i: da is the upper-j corner
                [(1, 1), (-1, 1)],   // +j: da is the upper-i corner
                [(-1, -1), (1, -1)], // -j: da is the lower-i corner
            ];
            for (nbi, da, db) in sides {
                let nb = nbs[nbi];
                let n_cube = (out_dirs[nbi] + up * 0.85).normalize();
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
                            // riser bake: 0.72 predates slope-lit tops and
                            // double-counted once they landed — step-dense
                            // terrain (terraced washes, meander banks) read
                            // as dark smears from any distance (the banding
                            // reports of 2026-07-08). Sun + sky-fill now do
                            // the modelling; keep only a whisper of bake.
                            let col = vary(vary(m0.color(tint), 0.90), bright(z0));
                            // Riser normals: a natural terrain step is a
                            // QUANTIZATION of a smooth slope, but a cube
                            // normal admits only four side orientations — so
                            // two faces meeting at a corner split light/dark
                            // under any directional sun/moon, and a zigzag
                            // contour line bands face-by-face (Austin's
                            // annotated shots, 2026-07-08 night). Surface-
                            // adjacent risers therefore inherit the SAME
                            // continuous surface normal the tops shade with,
                            // fading back to the cube normal a few blocks
                            // down (real cliff walls) — and player-built
                            // walls keep crisp cube shading outright.
                            let n_side = if c.ground != c.ground0 {
                                n_cube
                            } else {
                                let depth = (c.top_solid() - (z - 1)).max(0) as f64;
                                let k = (0.85 - 0.28 * depth).clamp(0.0, 0.85);
                                (top_n * k + n_cube * (1.0 - k)).normalize()
                            };
                            // Wall ambient occlusion — the missing half of
                            // the tops' corner shadow. Without it a crease
                            // darkens only on the floor side and the shadow
                            // stops dead at the crease line (Austin,
                            // 2026-07-08 late). Same occluder rule and
                            // falloff as the top faces: for each vertex,
                            // the air-side cells beyond the edge, lateral
                            // to the corner, and diagonal.
                            let (ox, oy) = nb_off[nbi];
                            let ao = |corner: usize, ze: i64, zr: i64| -> f32 {
                                let (lx, ly) = lat_off[nbi][corner];
                                let s1 = at(i + ox, j + oy).filled(ze) as i32;
                                let s2 = at(i + lx, j + ly).filled(zr) as i32;
                                let dd = at(i + lx, j + ly).filled(ze) as i32;
                                let lvl = if s1 + s2 == 2 { 3 } else { s1 + s2 + dd };
                                1.0 - 0.15 * lvl as f32
                            };
                            let (zt, zb) = (z - 1, z0);
                            let cols = [
                                vary(col, ao(0, zt + 1, zt)), // da, top
                                vary(col, ao(1, zt + 1, zt)), // db, top
                                vary(col, ao(1, zb - 1, zb)), // db, bottom
                                vary(col, ao(0, zb - 1, zb)), // da, bottom
                            ];
                            quad([da * r1, db * r1, db * r0, da * r0], n_side, cols, cave, 0.0);
                            run_start = other.map(|mm| (z, mm));
                        }
                        _ => {}
                    }
                    z += 1;
                }
            }

            // ---- water: top surface and exposed banks
            if c.has_water() {
                // Rendered water top, clamped to meet DRY shore neighbours
                // flush. Blended river levels tilt the water surface, and
                // quantizing a tilted surface against the terrain contour
                // otherwise leaves the water standing one block PROUD of the
                // bank along stretches of shoreline (floor(level) can exceed
                // floor(bank h) while the bank itself is genuinely above the
                // water). A dry neighbour is only ever lower than the water
                // block through that rounding mismatch, so meeting it flush
                // mis-renders by <1 m and reads as a real waterline. Both
                // sides of every seam compute the same clamp (pure function
                // of the column + 4-neighbourhood, all inside the ghost
                // margin), so faces agree across chunk borders.
                // (LIQUID only: a frozen sheet is walkable geometry — physics
                // stands on the unclamped block, so its visual must not sink.)
                let w_eff = |i: i64, j: i64| -> i64 {
                    let nbs8 = [
                        *at(i + 1, j),
                        *at(i - 1, j),
                        *at(i, j + 1),
                        *at(i, j - 1),
                        *at(i + 1, j + 1),
                        *at(i + 1, j - 1),
                        *at(i - 1, j + 1),
                        *at(i - 1, j - 1),
                    ];
                    water_render_top(at(i, j), &nbs8)
                };
                let w = w_eff(i, j);
                let frozen = c.frozen;
                let wmat = if frozen { Mat::Ice } else { Mat::Water };
                let mut wcol = wmat.color(tint);
                if frozen {
                    // a frozen sheet is flat and one color, so its top used to
                    // read as a featureless plane — indistinguishable from sky
                    // or liquid. Dust it with patchy snow per column so the
                    // solid surface reads as ground (brightness varied below).
                    let snow = Mat::Snow.color(
                        climate.expect("water exists only on a climate body").tint(MainBlock::Snow),
                    );
                    let f = hash01(face as u8, ci, cj, 0x1CE) as f32;
                    let s = f * f * 0.6;
                    wcol = [
                        wcol[0] + (snow[0] - wcol[0]) * s,
                        wcol[1] + (snow[1] - wcol[1]) * s,
                        wcol[2] + (snow[2] - wcol[2]) * s,
                    ];
                }
                if !frozen {
                    // ONE ramp with the mesh (terrain::water_surface_color,
                    // TRANSITIONS.md F): true ocean depth for the sea,
                    // carved depth for rivers/lakes/ponds
                    let depth_km = if c.sea {
                        -c.e_raw as f64
                    } else {
                        (w - c.top_solid()) as f64 / 1000.0
                    };
                    wcol = crate::terrain::water_surface_color(depth_km, c.sea, c.salt);
                }
                let r = shell(w);
                // frozen tops take the same per-column brightness checker as
                // land so the flat sheet reads as tiled ground, not a plane
                let wtop = if frozen { vary(wcol, bright(w)) } else { wcol };
                // same order as `nbs`: +i, -i, +j, -j
                let nb_off = [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)];
                let foam = [0.70, 0.82, 0.78];
                let foam_strength = |diff: i64, salt: u64| -> f32 {
                    let d = diff.clamp(1, 4) as f32;
                    let base = 0.10 + 0.06 * (d - 1.0);
                    let shimmer = 0.88 + 0.24 * hash01(face as u8, ci, cj, salt) as f32;
                    (base * shimmer).clamp(0.08, 0.30)
                };
                let lower_liquid_step = |nbi: usize| -> Option<f32> {
                    let nb = nbs[nbi];
                    if frozen || nb.frozen || !nb.has_water() {
                        return None;
                    }
                    if same_rim_water_plane(c, nb) {
                        return None;
                    }
                    let (di, dj) = nb_off[nbi];
                    let nb_w = w_eff(i + di, j + dj);
                    (nb_w < w).then(|| {
                        foam_strength(w - nb_w, 0xF04Du64 ^ ((nbi as u64) << 8) ^ w as u64)
                    })
                };
                let mut wtop_cols = [wtop; 4];
                let top_edge: [[usize; 2]; 4] = [
                    [1, 2], // +i: d10, d11
                    [3, 0], // -i: d01, d00
                    [2, 3], // +j: d11, d01
                    [0, 1], // -j: d00, d10
                ];
                for nbi in 0..4 {
                    if let Some(strength) = lower_liquid_step(nbi) {
                        for &corner in &top_edge[nbi] {
                            wtop_cols[corner] =
                                mix_color(wtop_cols[corner], foam, strength * 0.55);
                        }
                    }
                }
                let rim_delta = rim_water_analog_delta_km(c, w, exaggeration).map(|d| d as f32);
                if let Some(delta) = rim_delta {
                    quad_rim_delta.set([delta; 4]);
                    quad_rim_weight.set(Some([1.0; 4]));
                }
                quad(
                    [d00 * r, d10 * r, d11 * r, d01 * r],
                    up,
                    wtop_cols,
                    1.0,
                    if rim_delta.is_some() { 2.0 } else { 1.0 },
                );
                quad_rim_delta.set([0.0; 4]);
                quad_rim_weight.set(None);
                let wside = vary(wtop, 0.93);
                for (nbi, da, db) in sides {
                    let nb = nbs[nbi];
                    if same_rim_water_plane(c, nb) {
                        // Occupancy still steps from graph river to lake, but
                        // their presentation surface is one analog plane.
                        continue;
                    }
                    // the neighbour's water top must be ITS clamped value, or
                    // the two columns disagree about the seam and leak faces
                    let nb_surf = nb.top_solid().max(if nb.has_water() {
                        w_eff(i + nb_off[nbi].0, j + nb_off[nbi].1)
                    } else {
                        i64::MIN
                    });
                    if nb_surf < w {
                        // true face direction (see ei_dir/ej_dir above) — the
                        // old position-derived out_n corrupted these too
                        let n_side = (out_dirs[nbi] * 0.18 + up).normalize();
                        let (r0, r1) = (shell(nb_surf.max(c.top_solid())), shell(w));
                        let col = lower_liquid_step(nbi)
                            .map(|strength| mix_color(wside, foam, strength))
                            .unwrap_or(wside);
                        if let Some(delta) = rim_delta {
                            quad_rim_delta.set([delta, delta, 0.0, 0.0]);
                            quad_rim_weight.set(Some([1.0, 1.0, 0.0, 0.0]));
                        }
                        quad(
                            [da * r1, db * r1, db * r0, da * r0],
                            n_side,
                            [col; 4],
                            1.0,
                            if rim_delta.is_some() { 2.0 } else { 1.0 },
                        );
                        quad_rim_delta.set([0.0; 4]);
                        quad_rim_weight.set(None);
                    }
                }
            }

            // ---- flooded caves (BUGS.md W-6): carved cave cells at/below the
            // local water table hold water. A dry-surface column only (col_ctx
            // never sets cave_water where the open-air `water` exists), so this
            // never overlaps the block above. The rock walls/floor/ceiling are
            // already opaque from the solid pass; here we add the water itself:
            // a free TOP surface wherever dry air sits above the pool (an air
            // pocket to dive from, or an open water-filled pit), and SIDE faces
            // only where the pool meets a DRY cave passage in a neighbour.
            // Faces are NOT drawn against open sky over lower ground — that
            // would stand a wall of water above a dry neighbour (the very W-1/
            // shore-lip family we avoid); an open pit is instead enclosed by
            // its own rock walls and read from above through the top surface.
            if c.cave_water != i64::MIN {
                let cw = c.cave_water;
                let cave_frozen = c.frozen;
                let base = if cave_frozen { Mat::Ice } else { Mat::Water }.color(tint);
                let wz_lo = c.ground0 - CAVE_DEPTH;
                for z in wz_lo..=cw {
                    if !c.cave_flooded(z) {
                        continue;
                    }
                    // cave darkness (torch-relightable), same depth ramp as the
                    // rock: a shallow pit pool stays lit, a deep flood goes dark
                    let dim =
                        (1.0 - 0.20 * (c.top_solid() - z).max(0) as f32).clamp(0.25, 1.0);
                    // tint toward deep water with depth below the surface
                    let t = ((cw - z) as f32 / 2000.0).clamp(0.0, 1.0);
                    let deep = [0.004, 0.013, 0.055];
                    let wcol = if cave_frozen {
                        base
                    } else {
                        [
                            base[0] + (deep[0] - base[0]) * t,
                            base[1] + (deep[1] - base[1]) * t,
                            base[2] + (deep[2] - base[2]) * t,
                        ]
                    };
                    // free surface: cell above is dry air (not water, not rock)
                    if !c.filled(z + 1) && !c.cave_flooded(z + 1) {
                        let r = shell(z);
                        quad(
                            [d00 * r, d10 * r, d11 * r, d01 * r],
                            up,
                            [wcol; 4],
                            dim,
                            if cave_frozen { 0.0 } else { 1.0 },
                        );
                    }
                    // sides: only into a DRY cave passage (carved air within the
                    // neighbour's own band), never open sky above lower ground
                    let wside = vary(wcol, 0.93);
                    for (nbi, da, db) in sides {
                        let nb = nbs[nbi];
                        let nb_dry_cave =
                            z <= nb.ground && !nb.filled(z) && !nb.cave_flooded(z);
                        if nb_dry_cave {
                            let n_side = (out_dirs[nbi] * 0.18 + up).normalize();
                            let (r0, r1) = (shell(z - 1), shell(z));
                            quad(
                                [da * r1, db * r1, db * r0, da * r0],
                                n_side,
                                [wside; 4],
                                dim,
                                if cave_frozen { 0.0 } else { 1.0 },
                            );
                        }
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
            let (aface, aci, acj) = canonical_column_body(body, face, base_i + ai, base_j + aj);
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
            if let Some((kind, trunk)) = tree_at(c, aface, aci, acj, body.seed()) {
                let rnd = hash_u64(aface, aci, acj, 0xF0F0);
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
        let ci = (base_i + ti) as u64;
        let cj = (base_j + tj) as u64;
        let rain_c = at(ti, tj);
        quad_rain_concavity.set(crate::terrain::rain_concavity_proxy(
            rain_c.e_raw as f64,
            rain_c.h_km as f64,
        ));
        // Tree cells never use a ground-tinted material; keep their established
        // species palette without another climate raster pass per leaf block.
        let tint = [0.0; 3];
        let h = hash_u64(face as u8, ci, cj, tz as u64);
        let bright = 0.88 + 0.24 * ((h >> 13 & 0xFF) as f32 / 255.0);
        let base = mat.color(tint);
        let base = if mat == Mat::LeavesBroad {
            crate::weather::deciduous_tint(
                base,
                rain_c.seasonal_temp as f64,
                rain_c.seasonal_temp_trend as f64,
                rain_c.structural_season,
            )
        } else {
            base
        };
        let col = vary(base, bright);
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
            quad([d00 * r, d10 * r, d11 * r, d01 * r], up, [col; 4], 1.0, 0.0);
        }
        if !solid_at(0, 0, tz - 1) {
            let r = shell(tz - 1);
            quad([d00 * r, d01 * r, d11 * r, d10 * r], -up, [vary(col, 0.6); 4], 1.0, 0.0);
        }
        // true face directions (not position-derived — see the terrain
        // sides): tree faces were erratically lit by where the tree stood
        // in its chunk, which is what made twin shrubs read bright vs black
        let ei_dir = {
            let d = d10 - d00;
            (d - up * d.dot(up)).normalize_or_zero()
        };
        let ej_dir = {
            let d = d01 - d00;
            (d - up * d.dot(up)).normalize_or_zero()
        };
        let sides = [
            (1i64, 0i64, d10, d11, ei_dir),
            (-1, 0, d01, d00, -ei_dir),
            (0, 1, d11, d01, ej_dir),
            (0, -1, d00, d10, -ej_dir),
        ];
        for (di, dj, da, db, out_dir) in sides {
            if !solid_at(di, dj, tz) {
                let n_side = (out_dir + up * 0.85).normalize();
                let (r0, r1) = (shell(tz - 1), shell(tz));
                quad([da * r1, db * r1, db * r0, da * r0], n_side, [vary(col, 0.8); 4], 1.0, 0.0);
            }
        }
    }

    // ---- torches: a crossed pair of thin vertical quads standing on the
    // column's walkable top, wood below, emissive flame above (dim = 2.0
    // marks emissive for the shader). The actual LIGHT is a per-frame
    // point light the renderer collects from the same torch set.
    for &(tf, tci, tcj) in torches.iter() {
        if tf != key.face {
            continue;
        }
        let (ti, tj) = (tci as i64 - base_i, tcj as i64 - base_j);
        if !(0..n).contains(&ti) || !(0..n).contains(&tj) {
            continue;
        }
        let c = at(ti, tj);
        quad_rain_concavity.set(crate::terrain::rain_concavity_proxy(
            c.e_raw as f64,
            c.h_km as f64,
        ));
        // walkable top, ice included — a torch on a frozen lake stands on
        // the ice sheet, not drowned on the seabed below it
        let top = c.top_solid().max(c.frozen_ice().unwrap_or(i64::MIN));
        let (u0, u1) = (u_of(base_i + ti), u_of(base_i + ti + 1));
        let (v0, v1) = (v_of(base_j + tj), v_of(base_j + tj + 1));
        let d00 = face_dir(face, u0, v0);
        let d10 = face_dir(face, u1, v0);
        let d11 = face_dir(face, u1, v1);
        let d01 = face_dir(face, u0, v1);
        let up = origin_dir;
        let wood = [0.25f32, 0.15, 0.06];
        let flame = [1.0f32, 0.80, 0.42];
        let r0 = shell(top);
        let r1 = r0 + 0.62 * VOXEL_KM * exaggeration;
        let lp = |a: DVec3, b: DVec3, t: f64| (a + (b - a) * t).normalize();
        for (pa, pb) in [(d00, d11), (d10, d01)] {
            let e0 = lp(pa, pb, 0.40);
            let e1 = lp(pa, pb, 0.60);
            quad(
                [e0 * r0, e1 * r0, e1 * r1, e0 * r1],
                up,
                [wood, wood, flame, flame],
                2.0,
                0.0,
            );
        }
    }

    TileMesh { origin_km: origin, vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_columns_are_exact_deterministic_samples() {
        let radius = 8660.254037844386 * 0.27;
        let generator = std::sync::Arc::new(MoonGenerator::new(42));
        let moon = LunarBody::new(radius, std::sync::Arc::clone(&generator));
        let edits = Edits::default();
        for (face, ci, cj) in [
            (0usize, 1_350_000u64, 1_350_000u64),
            (2, 765_432, 2_123_456),
            (5, 2_388_888, 311_111),
        ] {
            let a = moon.column_ctx(&edits, face, ci, cj);
            let b = moon.column_ctx(&edits, face, ci, cj);
            assert_eq!(a.ground0, b.ground0);
            assert_eq!(a.h_km.to_bits(), b.h_km.to_bits());
            let n = moon.columns_per_face() as f64;
            let u = -1.0 + 2.0 * (ci as f64 + 0.5) / n;
            let v = -1.0 + 2.0 * (cj as f64 + 0.5) / n;
            let sample = generator.sample(face_dir(face, u, v));
            assert_eq!(a.ground0, (sample.height_ratio * radius * 1000.0).floor() as i64);
            let lunar = a.lunar.expect("moon column material");
            assert_eq!(lunar.material, sample.material());
            assert_eq!(lunar.albedo.to_bits(), (sample.albedo as f32).to_bits());
            assert!(a.climate.is_none());
            assert_eq!(a.water, i64::MIN);
            assert_eq!(a.cave_bits, 0);
        }
    }

    #[test]
    fn body_lattices_round_trip_random_directions_within_one_column() {
        let planet = test_planet();
        let moon = LunarBody::new(
            planet.radius_km * 0.27,
            std::sync::Arc::new(MoonGenerator::new(planet.seed)),
        );
        assert_eq!(LUNAR_COLUMNS_PER_FACE, 2_700_000);
        assert_eq!(LUNAR_COLUMNS_PER_FACE % CHUNK, 0);

        let mut state = 0x6A09_E667_F3BC_C909u64;
        let mut unit = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            ((z >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
        };
        for body in [&planet as &dyn VoxelBody, &moon as &dyn VoxelBody] {
            let n = body.columns_per_face() as f64;
            for _ in 0..2_000 {
                let z = unit() * 2.0 - 1.0;
                let a = unit() * std::f64::consts::TAU;
                let r = (1.0 - z * z).sqrt();
                let dir = DVec3::new(r * a.cos(), r * a.sin(), z);
                let id = column_id_body(body, dir);
                let center = dir_of_column_body(body, id.0 as usize, id.1, id.2);
                assert_eq!(column_id_body(body, center), id);
                let angular_error = dir.dot(center).clamp(-1.0, 1.0).acos();
                assert!(
                    angular_error <= 3.0 / n,
                    "{:?} angular error {angular_error} exceeded one-column bound {}",
                    body.body_id(),
                    3.0 / n,
                );
            }
        }
    }

    #[test]
    fn edit_keys_are_separated_per_body() {
        let key = (0u8, 12u64, 34u64);
        let mut edits = BodyEdits::default();
        edits.for_body_mut(BodyId::Sun).insert(key, 7);
        edits.for_body_mut(BodyId::Neisor).insert(key, -1);
        edits.for_body_mut(BodyId::Moon).insert(key, 3);
        assert_eq!(edits.for_body(BodyId::Sun).get(&key), Some(&7));
        assert_eq!(edits.for_body(BodyId::Neisor).get(&key), Some(&-1));
        assert_eq!(edits.for_body(BodyId::Moon).get(&key), Some(&3));
        assert_ne!(
            chunks_touching_column_body(BodyId::Neisor, key.0, key.1, key.2)[0],
            chunks_touching_column_body(BodyId::Moon, key.0, key.1, key.2)[0]
        );
    }

    fn test_planet() -> Planet {
        let assets = if std::path::Path::new("assets/meta.json").exists() {
            "assets"
        } else {
            "viewer/assets"
        };
        Planet::load(assets).expect("voxel gates require the baked viewer assets")
    }

    fn ctx_at(planet: &Planet, edits: &Edits, dir: DVec3) -> ColCtx {
        let (face, u, v) = crate::planet::face_from_dir(dir);
        let (ci, cj) = column_of(u, v);
        col_ctx(planet, edits, face, ci, cj)
    }

    #[test]
    fn placed_block_overrides_seasonal_water_class() {
        let planet = test_planet();
        let (lat, lon) = (73.486f64.to_radians(), -76.450f64.to_radians());
        let dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        let (face, u, v) = crate::planet::face_from_dir(dir);
        let (ci, cj) = column_of(u, v);
        let tuning = crate::weather::WeatherTuning::default();
        let winter = crate::weather::StructuralSeason::quantized(0.95, &tuning);
        let natural = col_ctx_at_season(&planet, &Edits::default(), face, ci, cj, winter);
        assert!(natural.has_water() && natural.frozen, "probe stopped exercising winter ice");
        let mut edits = Edits::default();
        edits.insert((face as u8, ci, cj), 1);
        let edited = col_ctx_at_season(&planet, &edits, face, ci, cj, winter);
        assert_eq!(edited.ground, natural.water + 1);
        assert!(!edited.has_water());
        assert_eq!(edited.frozen_ice(), None);
    }

    #[test]
    fn broadleaf_positions_are_season_independent() {
        let planet = std::sync::Arc::new(test_planet());
        let tuning = crate::weather::WeatherTuning::default();
        let winter = SeasonalPlanet::new(
            std::sync::Arc::clone(&planet),
            crate::weather::StructuralSeason::quantized(0.95, &tuning),
        );
        let summer = SeasonalPlanet::new(
            std::sync::Arc::clone(&planet),
            crate::weather::StructuralSeason::quantized(0.50, &tuning),
        );
        let (lat, lon) = (-0.906f64.to_radians(), -67.804f64.to_radians());
        let dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        let (face, u, v) = crate::planet::face_from_dir(dir);
        let (ci, cj) = column_of(u, v);
        let edits = Edits::default();
        let mut trees = 0;
        for dj in -24i64..=24 {
            for di in -24i64..=24 {
                let (f, i, j) = canonical_column(face, ci as i64 + di, cj as i64 + dj);
                let a = winter.tree_at_column(&edits, f as usize, i, j);
                let b = summer.tree_at_column(&edits, f as usize, i, j);
                assert_eq!(a, b);
                trees += usize::from(a.is_some());
            }
        }
        assert!(trees > 0, "probe stopped exercising tree positions");
    }

    #[test]
    fn rim_water_handoff_is_flush_without_changing_occupancy() {
        let planet = test_planet();
        let edits = Edits::default();
        let (lat, lon) = (13.013f64.to_radians(), -4.546f64.to_radians());
        let center_dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        let axis = if center_dir.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
        let tangent = (axis - center_dir * axis.dot(center_dir)).normalize();
        let river_dir = (center_dir - tangent * 0.10 / planet.radius_km).normalize();
        let lake = ctx_at(&planet, &edits, center_dir);
        let river = ctx_at(&planet, &edits, river_dir);

        assert!(lake.rim_flush_water && river.rim_flush_water);
        assert_eq!(lake.water, 122);
        assert_eq!(river.water, 125, "graph-river occupancy unexpectedly changed");
        assert!(same_rim_water_plane(&lake, &river));
        assert!((lake.analog_water_km - 0.122_408_948_838_710_78).abs() < 1e-12);

        let lift = lift_km(1.0);
        assert!((rim_water_top_km(&river, river.water, 1.0, lift, 0.0)
            - (river.analog_water_km + lift)).abs() < 1e-12);
        assert!((rim_water_top_km(&river, river.water, 1.0, lift, 1.0)
            - river.analog_water_km).abs() < 1e-12);
        let physics = water_surface_km(&planet, &edits, river_dir, 1.0, 1.0).unwrap();
        assert!((physics - (river.analog_water_km + lift)).abs() < 1e-12);

        // Same fractional lake level, no higher-river overlap: the standing
        // Difficulty Lake control keeps its exact lattice path.
        let (dl_lat, dl_lon) = (13.346f64.to_radians(), -4.807f64.to_radians());
        let dl_dir = DVec3::new(
            dl_lat.cos() * dl_lon.cos(),
            dl_lat.cos() * dl_lon.sin(),
            dl_lat.sin(),
        );
        let dl = ctx_at(&planet, &edits, dl_dir);
        assert!(dl.has_water() && !dl.rim_flush_water);
        assert_eq!(
            water_surface_km(&planet, &edits, dl_dir, 1.0, 1.0).unwrap(),
            dl.water as f64 * VOXEL_KM + lift
        );
    }

    #[test]
    fn tree_profile_follows_shared_surface_not_nearest_class() {
        let wooded = tree_biome_profile(13, MainBlock::Grass, 0.5, 15.0, 1_000.0)
            .expect("grass forest should grow trees");
        let sentinel_patch = tree_biome_profile(255, MainBlock::Grass, 0.5, 15.0, 1_000.0)
            .expect("a dithered grass island must not inherit sentinel barrenness");
        assert_eq!(wooded.0, sentinel_patch.0);
        assert!((wooded.1 - sentinel_patch.1).abs() < f64::EPSILON);
        assert!(
            tree_biome_profile(13, MainBlock::Sand, 0.5, 15.0, 1_000.0).is_none(),
            "a dithered sand island must not inherit forest eligibility"
        );
    }

    #[test]
    fn tree_density_is_continuous_in_warped_forest_signal() {
        let densities: Vec<f64> = [0.15, 0.30, 0.50, 0.60, 0.85]
            .into_iter()
            .map(|forest| {
                tree_biome_profile(13, MainBlock::Grass, forest, 12.0, 1_000.0)
                    .unwrap()
                    .1
            })
            .collect();
        assert!(densities.windows(2).all(|w| w[0] < w[1]), "{densities:?}");
        assert!((densities[4] - MAX_TREE_DENSITY).abs() < 0.000_1);
        // True zero-forest steppe/tundra retains its established sparse flora.
        assert_eq!(
            tree_biome_profile(6, MainBlock::Grass, 0.0, 2.0, 300.0),
            Some((TreeKind::Shrub, 0.0015))
        );
    }
}

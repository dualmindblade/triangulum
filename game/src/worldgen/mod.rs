//! Generator v2: a pure function of (seed, position, LOD voxel size).
//!
//! Height = the smooth bake surface times the vertical scale, plus the
//! engine's landforms (`landform.rs`: ridges, valley networks, mesas and a
//! fine roughness), band-limited to the voxel size so every LOD samples the
//! same surface. Materials come from climate; strata are real blocks.
//! A chunk is generated directly at its LOD from the column samples at its
//! voxel centres, with a one-voxel border so the mesher can cull and shade
//! faces against the neighbours without loading them.
//!
//! Two entry points for the streamer:
//! - `classify(key)`: cheap Air / Enclosed / Mixed from the footprint's
//!   height bounds (cache `footprint_height_range_m` per footprint and use
//!   `classify_in`); a renderer can skip Air and Enclosed chunks.
//! - `generate(key)`: always the full, correct contents (strata, bedrock,
//!   water, markers) whatever the classification.
//!
//! Voxel rules, in order (`block_at`): above the world is air, below it
//! bedrock; in an ocean column the voxel holding sea level is water at every
//! LOD (a coarse voxel must not lose the sea to the rock below it); layer 0
//! is bedrock (at coarse LODs the bottom voxel is solid, bedrock where only
//! the floor makes it so); otherwise solid below the surface, water below
//! sea level in ocean columns, air above.

pub mod bake;
pub mod inspect;
pub mod landform;
pub mod noise;
pub mod strata;

use crate::grid::{CHUNK, ChunkKey, FootprintKey, Planet};
use crate::world::chunk::VOXELS;
use crate::world::{BlockId, Chunk, Registry};
use bake::Bake;
use glam::DVec3;
use landform::{Landform, Local};
use noise::gradient_noise;
use std::sync::Arc;
use strata::{GeoHint, RockCache, Strata};

/// Padded voxel grid edge (one voxel border on every side).
pub const PAD: usize = CHUNK + 2;
pub const PAD_VOXELS: usize = PAD * PAD * PAD;

/// Index into a padded grid for voxel coordinates in [-1, 32].
#[inline]
pub fn pidx(x: i32, y: i32, z: i32) -> usize {
    (((x + 1) as usize * PAD) + (y + 1) as usize) * PAD + (z + 1) as usize
}

/// Ocean-mask value by which a column whose surface ends below sea level is
/// free to flood (coves, drowned valleys); further inland such columns are
/// held at LAND_FLOOR_M instead, so no dry pit sits next to the sea.
const SEA_MASK: f64 = 0.15;
const LAND_FLOOR_M: f64 = 0.3;
/// Lapse rate applied to the landform's height (deg C per Earth km).
const LAPSE_C_PER_KM: f64 = 6.5;

/// Block ids the generator places, resolved from the registry by name.
#[derive(Clone, Copy, Debug)]
pub struct Blocks {
    pub air: BlockId,
    pub stone: BlockId,
    pub dirt: BlockId,
    pub grass: BlockId,
    pub dry_grass: BlockId,
    pub sand: BlockId,
    pub gravel: BlockId,
    pub snow: BlockId,
    pub ice: BlockId,
    pub water: BlockId,
    pub bedrock: BlockId,
    pub sandstone: BlockId,
    pub marker_red: BlockId,
    pub marker_white: BlockId,
}

impl Blocks {
    pub fn resolve(r: &Registry) -> Result<Self, String> {
        Ok(Self {
            air: r.id("air")?,
            stone: r.id("stone")?,
            dirt: r.id("dirt")?,
            grass: r.id("grass")?,
            dry_grass: r.id("dry_grass")?,
            sand: r.id("sand")?,
            gravel: r.id("gravel")?,
            snow: r.id("snow")?,
            ice: r.id("ice")?,
            water: r.id("water")?,
            bedrock: r.id("bedrock")?,
            sandstone: r.id("sandstone")?,
            marker_red: r.id("marker_red")?,
            marker_white: r.id("marker_white")?,
        })
    }
}

/// A debug scale reference: a column of blocks standing on the ground.
#[derive(Clone, Copy, Debug)]
pub struct Marker {
    pub dir: DVec3,
    pub height_blocks: u32,
    pub width_blocks: u32,
    pub block: BlockId,
    /// Stamp downward from the ground instead of upward (with `block` = air
    /// this cuts a pit whose walls expose the strata).
    pub below: bool,
}

/// One terrain column sampled at a voxel centre.
#[derive(Clone, Copy, Debug, Default)]
pub struct Column {
    /// Surface height, metres above sea level (relief included).
    pub height_m: f32,
    /// Ocean column: water fills [height, 0).
    pub ocean: bool,
    pub temp_c: f32,
    pub precip: f32,
    /// Ocean-mask fraction (near-shore indicator).
    pub shore: f32,
    /// Dither noise in [-1, 1] at ~40 m scale, for soft material thresholds.
    pub dither: f32,
    /// Strata column offset (m).
    pub strata_off: f32,
    /// planetgen soil/sediment depth (km, Earth scale; 0 when the bake lacks it).
    pub soil_km: f32,
    /// Geology hints for the strata (zero when the bake lacks them).
    pub geo: GeoHint,
    /// 0..1: river channel bed (gravel).
    pub river: f32,
}

/// Surface and soil of a column once the slope is known.
#[derive(Clone, Copy, Debug)]
struct Surface {
    top: BlockId,
    soil: BlockId,
    /// Soil thickness in metres below the top block.
    soil_m: f32,
}

/// What a chunk holds, as far as a renderer needs to know before generating it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkClass {
    /// Every voxel (border included) is air: nothing to draw.
    Air,
    /// Every voxel (border included) is solid: no face can show.
    Enclosed,
    /// Anything else: generate and mesh it.
    Mixed,
}

pub struct Generator {
    pub planet: Planet,
    pub bake: Arc<dyn Bake>,
    pub seed: u64,
    /// Metres of terrain per metre of bake elevation.
    pub vscale: f64,
    /// Andrew's fine-relief dial (1.0 = default).
    pub roughness: f64,
    /// Andrew's landform dial (1.0 = default).
    pub landform: f64,
    /// The landform model (holds copies of the dials above).
    pub lf: Landform,
    pub blocks: Blocks,
    pub strata: Strata,
    /// Border columns are lowered by this many voxels so chunk edges grow
    /// a few side faces that hide LOD-seam gaps.
    pub skirt: u32,
    pub markers: Vec<Marker>,
    /// Debug: generate no water (ocean columns stay dry so the floor shows).
    pub hide_water: bool,
    /// Largest |landform| anywhere (m), for global bounds.
    max_relief_m: f64,
}

/// Result of generating one chunk.
pub struct Generated {
    pub key: ChunkKey,
    /// Padded block ids, 34^3, index by `pidx`.
    pub ids: Vec<BlockId>,
    /// The compressed interior chunk.
    pub chunk: Chunk,
    pub gen_us: u32,
}

impl Generator {
    pub fn new(planet: Planet, bake: Arc<dyn Bake>, seed: u64, vscale: f64, roughness: f64, landform: f64, registry: &Registry) -> Result<Self, String> {
        let blocks = Blocks::resolve(registry)?;
        let beds = ["shale", "mudstone", "umber", "sandstone", "limestone"]
            .iter()
            .map(|n| registry.id(n))
            .collect::<Result<Vec<_>, _>>()?;
        let strata = Strata::new(beds, planet.block_size, planet.radius, seed);
        let lf = Landform { seed: seed ^ 0x1A4D_F0E1, radius: planet.radius, vscale, landform, roughness };
        let elev_max = (0..6u8).map(|f| bake.elev_range_km(f, -1.0, -1.0, 1.0, 1.0).1).fold(f32::MIN, f32::max) as f64;
        let max_relief_m = lf.bound_m(lf.relief_bound_m(bake.rough_max_km() as f64, elev_max));
        Ok(Self { planet, bake, seed, vscale, roughness, landform, lf, blocks, strata, skirt: 2, markers: Vec::new(), hide_water: false, max_relief_m })
    }

    /// One line naming the relief model and its dials (captions, logs).
    pub fn describe_relief(&self) -> String {
        format!("landforms v2, roughness {}, landform {}", self.lf.roughness, self.lf.landform)
    }

    /// Conservative padding (m) above and below the bake's height range,
    /// for the worst relief anywhere (the horizon occluder uses it).
    pub fn height_margin_m(&self) -> f64 {
        self.max_relief_m + (self.skirt as f64 + 2.0) * self.planet.voxel_m(self.planet.max_lod).min(64.0 * self.planet.block_size)
    }

    /// Sample the column at a direction, band-limited for `voxel_m`-sized voxels.
    pub fn column(&self, dir: DVec3, voxel_m: f64) -> Column {
        self.column_full(dir, voxel_m).0
    }

    /// The column plus the landform's displacement and local parameters.
    fn column_full(&self, dir: DVec3, voxel_m: f64) -> (Column, f64, Local) {
        self.column_from(dir, voxel_m, None, None)
    }

    /// A column, with the landform's coarse part optionally given
    /// (interpolated from a chunk's grid; see `chunk_columns`).
    fn column_from(&self, dir: DVec3, voxel_m: f64, coarse: Option<(f64, f64, f64, f64)>, strata_off: Option<f64>) -> (Column, f64, Local) {
        let s = self.bake.sample(dir);
        let base = s.elev_km * 1000.0 * self.vscale;
        let strata_off = strata_off.unwrap_or_else(|| self.strata.column_offset_m(dir));
        let lf = self.lf.sample_with(dir, &s, voxel_m, strata_off, coarse);
        let mut h = base + lf.dh;
        // Inland, ground pushed below sea level is held just above it; toward
        // the sea that hold fades out (continuously in the ocean mask, so no
        // cliff follows the mask's texels) and such ground floods: coves,
        // drowned valleys.
        if base >= -1.0 && h < LAND_FLOOR_M + 1.0 {
            let hold = 1.0 - smoothstep(0.0, SEA_MASK, s.ocean);
            h += hold * (soft_floor(h, LAND_FLOOR_M, 1.0) - h);
        }
        let ocean = h < 0.0 && s.ocean > 0.0;
        // Lapse rate on the landform's own height, in Earth-equivalent km.
        let lapse = (LAPSE_C_PER_KM * lf.dh / self.vscale / 1000.0).clamp(-12.0, 12.0);
        let p = dir * self.planet.radius;
        let dither = gradient_noise(p / 41.0 + DVec3::splat(3.1), self.seed ^ 0xD17) as f32;
        let col = Column {
            height_m: h as f32,
            ocean,
            temp_c: (s.temp_c - lapse) as f32,
            precip: s.precip_mm as f32,
            shore: s.ocean as f32,
            dither,
            strata_off: strata_off as f32,
            soil_km: s.soil_km as f32,
            geo: GeoHint {
                craton: s.continental as f32,
                volcanic: s.volcanic as f32,
                hardness: if s.erodibility > 0.0 { (1.0 / s.erodibility).clamp(0.0, 1.0) as f32 } else { 0.0 },
            },
            river: lf.river as f32,
        };
        (col, lf.dh, lf.local)
    }

    /// Ground height (m above sea level) under a direction; water counts as 0.
    pub fn ground_height_m(&self, dir: DVec3) -> f64 {
        let c = self.column(dir, self.planet.block_size);
        if c.ocean && !self.hide_water { 0.0 } else { c.height_m as f64 }
    }

    /// Conservative (min, max) of the bake elevation (km) over the columns a
    /// chunk of this footprint samples, its one-voxel border included. The
    /// padded footprint is sampled on a 9 x 9 lattice grid through
    /// `Planet::dir` (so border columns that snap onto a neighbouring face
    /// land there), each face's gnomonic box is grown by the largest step
    /// between neighbouring samples, and the bake bounds each box from its
    /// min/max pyramid with the full 2D Catmull-Rom overshoot.
    fn bake_range_km(&self, fp: FootprintKey) -> (f64, f64, f64) {
        let planet = &self.planet;
        let vb = (1u64 << fp.lod) as f64;
        let size = fp.extent_blocks() as f64;
        let (i0, j0) = (fp.cx as f64 * size - vb, fp.cy as f64 * size - vb);
        let span = size + 2.0 * vb;
        const G: usize = 9;
        // Per face: (a0, b0, a1, b1, max step).
        let mut boxes = [(f64::MAX, f64::MAX, f64::MIN, f64::MIN, 0.0f64); 6];
        let mut prev_row: Vec<Option<(u8, f64, f64)>> = vec![None; G];
        for gy in 0..G {
            let mut prev: Option<(u8, f64, f64)> = None;
            for gx in 0..G {
                let i = i0 + span * gx as f64 / (G - 1) as f64;
                let j = j0 + span * gy as f64 / (G - 1) as f64;
                let d = planet.dir(fp.face, i, j);
                let f = dominant_face(d);
                let (axis, right, up) = cubesphere::FACES[f as usize];
                let z = d.dot(axis);
                let (a, b) = (d.dot(right) / z, d.dot(up) / z);
                let bx = &mut boxes[f as usize];
                bx.0 = bx.0.min(a);
                bx.1 = bx.1.min(b);
                bx.2 = bx.2.max(a);
                bx.3 = bx.3.max(b);
                for n in [prev, prev_row[gx]].into_iter().flatten() {
                    if n.0 == f {
                        bx.4 = bx.4.max((n.1 - a).abs()).max((n.2 - b).abs());
                    } else {
                        // A step across a face edge: grow both sides by a voxel's worth.
                        bx.4 = bx.4.max(4.0 * vb * planet.block_size / planet.radius);
                    }
                }
                prev = Some((f, a, b));
                prev_row[gx] = Some((f, a, b));
            }
        }
        let (mut emin, mut emax, mut rmax) = (f64::MAX, f64::MIN, 0.0f64);
        for (f, bx) in boxes.iter().enumerate() {
            if bx.0 > bx.2 {
                continue;
            }
            let g = bx.4 * 1.5;
            let (a0, b0, a1, b1) = (bx.0 - g, bx.1 - g, bx.2 + g, bx.3 + g);
            let (mn, mx) = self.bake.elev_range_km(f as u8, a0, b0, a1, b1);
            emin = emin.min(mn as f64);
            emax = emax.max(mx as f64);
            rmax = rmax.max(self.bake.rough_max_range_km(f as u8, a0, b0, a1, b1) as f64);
        }
        (emin, emax, rmax)
    }

    /// Conservative (min, max) surface height in metres over every column a
    /// chunk of this footprint samples (its one-voxel border included), water
    /// surface and markers included. The bake part comes from the bake's
    /// pyramid; the landform part from a 5 x 5 sample of the real landform at
    /// this LOD, widened by the largest difference between neighbouring
    /// samples and by a bound on every octave finer than the sample grid can
    /// see. Costs about 25 landform evaluations (~80 us); cache it per footprint.
    pub fn footprint_height_range_m(&self, fp: FootprintKey) -> (f64, f64) {
        let planet = &self.planet;
        let vb = (1u64 << fp.lod) as f64;
        let voxel_m = planet.voxel_m(fp.lod);
        let size = fp.extent_blocks() as f64;
        let (i0, j0) = (fp.cx as f64 * size, fp.cy as f64 * size);
        let (emin, emax, _) = self.bake_range_km(fp);
        const N: usize = 5;
        let step = (CHUNK as f64 + 1.0) / (N - 1) as f64; // voxels between samples
        let mut dh = [[0.0f64; N]; N];
        let mut sub = 0.0f64;
        for a in 0..N {
            for b in 0..N {
                let x = -1.0 + step * a as f64;
                let y = -1.0 + step * b as f64;
                let dir = planet.dir(fp.face, i0 + (x + 0.5) * vb, j0 + (y + 0.5) * vb);
                let s = self.bake.sample(dir);
                let l = self.lf.sample(dir, &s, voxel_m, self.strata.column_offset_m(dir));
                dh[a][b] = l.dh;
                sub = sub.max(self.lf.sub_amp_bound_m(&l.local, 4.0 * step * voxel_m));
            }
        }
        let (mut lo, mut hi, mut dmax) = (f64::MAX, f64::MIN, 0.0f64);
        for a in 0..N {
            for b in 0..N {
                lo = lo.min(dh[a][b]);
                hi = hi.max(dh[a][b]);
                if a + 1 < N {
                    dmax = dmax.max((dh[a + 1][b] - dh[a][b]).abs());
                }
                if b + 1 < N {
                    dmax = dmax.max((dh[a][b + 1] - dh[a][b]).abs());
                }
                if a + 1 < N && b + 1 < N {
                    dmax = dmax.max((dh[a + 1][b + 1] - dh[a][b]).abs()).max((dh[a + 1][b] - dh[a][b + 1]).abs());
                }
            }
        }
        let margin = dmax + 1.5 * sub + 2.0 * voxel_m;
        let mut hmin = emin * 1000.0 * self.vscale + lo - margin;
        let mut hmax = emax * 1000.0 * self.vscale + hi + margin;
        // The land floor lifts low dry land to just above sea level; the sea surface is at 0.
        if hmin < LAND_FLOOR_M + 1.0 {
            hmax = hmax.max(LAND_FLOOR_M + 1.0);
        }
        if hmin < 0.0 {
            hmax = hmax.max(0.0);
        }
        for m in &self.markers {
            let (f, i, j) = planet.lattice_of_dir(m.dir);
            let reach_i = (m.width_blocks as f64) + 2.0 * vb;
            if f == fp.face && i >= i0 - reach_i && i < i0 + size + reach_i && j >= j0 - reach_i && j < j0 + size + reach_i {
                let ground = self.ground_height_m(m.dir);
                let reach = (m.height_blocks as f64 + 2.0) * planet.block_size;
                if m.below {
                    hmin = hmin.min(ground - reach - margin);
                    hmax = hmax.max(ground + margin);
                } else {
                    hmax = hmax.max(ground + reach);
                    hmin = hmin.min(self.column(m.dir, planet.block_size).height_m as f64 - margin);
                }
            }
        }
        (hmin, hmax)
    }

    /// Cheap classification from the footprint bounds (call
    /// `footprint_height_range_m` once per footprint and use `classify_in`
    /// when classifying many chunks of one footprint).
    pub fn classify(&self, key: ChunkKey) -> ChunkClass {
        self.classify_in(key, self.footprint_height_range_m(key.footprint()))
    }

    /// Classification against known footprint bounds (hmin, hmax) in metres.
    pub fn classify_in(&self, key: ChunkKey, (hmin, hmax): (f64, f64)) -> ChunkClass {
        let planet = &self.planet;
        let vb = key.voxel_blocks() as f64;
        let k0 = key.origin_blocks().2 as f64;
        // Padded vertical extent in layers: voxels -1 ..= 32.
        let (k_lo, k_hi) = (k0 - vb, k0 + (CHUNK as f64 + 1.0) * vb);
        if k_lo >= planet.height as f64 {
            return ChunkClass::Air;
        }
        let sea = planet.sea_level as f64 * planet.block_size;
        let (z_lo, z_hi) = (k_lo * planet.block_size - sea, k_hi * planet.block_size - sea);
        // Bedrock floor: a chunk holding layer 0 or below is never all air.
        if k_lo > 0.0 && z_lo >= hmax {
            return ChunkClass::Air;
        }
        let skirt_m = self.skirt as f64 * planet.voxel_m(key.lod);
        if k_hi <= 0.0 || z_hi <= hmin - skirt_m {
            return ChunkClass::Enclosed;
        }
        ChunkClass::Mixed
    }

    /// Surface block and the soil under it. Soil is 1-4 blocks measured
    /// from the natural ground: thicker on flat, warm, wet ground, thinner
    /// on steep rock.
    fn surface(&self, c: &Column, slope: f32) -> Surface {
        let b = &self.blocks;
        let d = c.dither;
        let s = self.planet.block_size as f32;
        let t = c.temp_c + 1.5 * d;
        let pr = c.precip + 40.0 * d;
        // Soil thickness in blocks: 2.5 base, +1 wet, +0.5 warm, -2 per unit of slope.
        let base = if c.soil_km > 0.0 {
            // planetgen's sediment depth (0..0.2 km) maps onto 1..4 blocks.
            1.0 + 3.0 * (c.soil_km / 0.1).min(1.0)
        } else {
            2.5 + if pr > 600.0 { 1.0 } else { 0.0 } + if t > 10.0 { 0.5 } else { 0.0 }
        };
        let soil_blocks = (base + 0.6 * d - 2.5 * slope.min(1.2)).clamp(1.0, 4.0);
        let soil_m = soil_blocks.round() * s;
        if c.ocean {
            let deep = -c.height_m > 6.0 * s + 2.0 * d;
            let floor = if deep { b.gravel } else { b.sand };
            return Surface { top: floor, soil: floor, soil_m: 2.0 * s };
        }
        if c.river > 0.5 + 0.2 * d {
            // A river bed: gravel over sand.
            return Surface { top: b.gravel, soil: b.sand, soil_m: 2.0 * s };
        }
        if slope > 0.95 + 0.25 * d {
            return Surface { top: b.stone, soil: b.stone, soil_m: 0.0 };
        }
        if t < -3.0 {
            return Surface { top: b.snow, soil: b.dirt, soil_m };
        }
        if c.height_m < (2.5 + d) * s && c.shore > 0.05 {
            return Surface { top: b.sand, soil: b.sand, soil_m: soil_m.max(2.0 * s) };
        }
        if pr < 130.0 && t > 8.0 {
            // Desert: a sand skin over real sandstone.
            return Surface { top: b.sand, soil: b.sandstone, soil_m: soil_m.max(2.0 * s) };
        }
        if pr < 320.0 {
            Surface { top: b.dry_grass, soil: b.dirt, soil_m }
        } else {
            Surface { top: b.grass, soil: b.dirt, soil_m }
        }
    }

    /// Block for the voxel covering layers [k_lo, k_lo + 2^lod) of a column
    /// (k_lo >= 0 and below the world top; the caller handles the rest).
    /// `z_lo` is the voxel's bottom in metres above sea level.
    #[inline]
    fn block_at(&self, c: &Column, sf: &Surface, z_lo: f64, voxel_m: f64, k_lo: f64, lod: u8, rock: &mut RockCache) -> BlockId {
        let b = &self.blocks;
        let h = c.height_m as f64;
        let z_hi = z_lo + voxel_m;
        let centre = z_lo + 0.5 * voxel_m;
        let water = c.ocean && !self.hide_water;
        // The sea surface survives every LOD: the voxel holding sea level in
        // an ocean column is water, however much of it lies below the floor.
        if water && z_lo < 0.0 && z_hi >= 0.0 && !(lod == 0 && k_lo == 0.0) {
            // The top water voxel freezes in the cold.
            return if c.temp_c + 1.5 * c.dither < -4.0 { b.ice } else { b.water };
        }
        // World floor: layer 0 is bedrock; a coarse bottom voxel is solid.
        if k_lo == 0.0 && (lod == 0 || h <= centre) {
            return b.bedrock;
        }
        if h > centre {
            let depth = h - centre;
            return if depth < voxel_m {
                sf.top
            } else if depth < voxel_m + sf.soil_m as f64 {
                sf.soil
            } else {
                self.strata.rock_at_cached(centre, c.strata_off as f64, c.geo, rock)
            };
        }
        if water && centre < 0.0 {
            return b.water;
        }
        b.air
    }

    /// The padded 34 x 34 column grid of a chunk at its LOD.
    ///
    /// The landform's long octaves (8 or more grid steps per wavelength) are
    /// evaluated on a grid of nodes at every 4th voxel centre, aligned to the
    /// face lattice so neighbouring chunks share nodes, and interpolated
    /// (Catmull-Rom, height and steering gradient); everything shorter, and
    /// every nonlinear step, is per column. Chunks whose nodes reach a cube
    /// edge's chart cross-fade take the direct per-column path. The two
    /// paths agree to a few centimetres (see the tests).
    pub fn chunk_columns(&self, key: ChunkKey) -> Vec<Column> {
        let planet = &self.planet;
        let vox_blocks = key.voxel_blocks() as f64;
        let voxel_m = planet.voxel_m(key.lod);
        let (i0, j0, _) = key.origin_blocks();
        let (i0, j0) = (i0 as f64, j0 as f64);
        let col_dir = |x: i32, y: i32| planet.dir(key.face, i0 + (x as f64 + 0.5) * vox_blocks, j0 + (y as f64 + 0.5) * vox_blocks);
        let grid = Landform::grid_cut(voxel_m).and_then(|cut| self.coarse_grid(key, cut, &col_dir).map(|n| (cut, n)));
        let mut cols = vec![Column::default(); PAD * PAD];
        for x in -1..=CHUNK as i32 {
            for y in -1..=CHUNK as i32 {
                let (coarse, off) = match grid.as_ref() {
                    Some((cut, nodes)) => {
                        let v = interp_grid(nodes, x, y);
                        (Some((*cut, v[0], v[1], v[2])), Some(v[3]))
                    }
                    None => (None, None),
                };
                cols[((x + 1) as usize) * PAD + (y + 1) as usize] = self.column_from(col_dir(x, y), voxel_m, coarse, off).0;
            }
        }
        cols
    }

    /// The coarse landform (height, steering gradient) and the strata
    /// offset at a chunk's grid nodes: voxel offsets -8, -4, ..., 40 on each
    /// axis (13 x 13), or None if any node leaves the face or lies in a
    /// chart cross-fade.
    fn coarse_grid(&self, key: ChunkKey, cut: f64, col_dir: &dyn Fn(i32, i32) -> DVec3) -> Option<Vec<[f64; 4]>> {
        let voxel_m = self.planet.voxel_m(key.lod);
        let mut nodes = Vec::with_capacity(GRID_N * GRID_N);
        for a in 0..GRID_N as i32 {
            for b in 0..GRID_N as i32 {
                let dir = col_dir(-8 + 4 * a, -8 + 4 * b);
                let s = self.bake.sample(dir);
                if s.face != key.face {
                    return None;
                }
                let (h, gx, gy) = self.lf.coarse_node(dir, &s, voxel_m, cut)?;
                nodes.push([h, gx, gy, self.strata.column_offset_m(dir)]);
            }
        }
        Some(nodes)
    }

    /// Generate one chunk at its own LOD: always the full contents.
    pub fn generate(&self, key: ChunkKey) -> Generated {
        let t0 = std::time::Instant::now();
        let planet = &self.planet;
        let lod = key.lod;
        let vox_blocks = key.voxel_blocks() as f64;
        let voxel_m = planet.voxel_m(lod);
        let k0 = key.origin_blocks().2 as f64;
        let sea = planet.sea_level as f64 * planet.block_size;
        let mut cols = self.chunk_columns(key);
        let water_on = !self.hide_water;
        let top_max = cols.iter().map(|c| if c.ocean && water_on { c.height_m.max(0.0) } else { c.height_m }).fold(f32::MIN, f32::max);
        // All air (border included): the padded bottom voxel is above every
        // surface and the water, and above the bedrock floor. Markers are
        // still stamped below, so this is the full result, not a shortcut.
        let k_pad_lo = k0 - vox_blocks;
        let all_air = k_pad_lo > 0.0 && k_pad_lo * planet.block_size - sea >= top_max as f64;
        let mut ids = vec![self.blocks.air; PAD_VOXELS];
        let mut water: Option<Vec<u8>> = None;
        if !all_air {
            // Surface material per column from the local slope (rise over run),
            // classified on the true heights before the skirt lowers the border.
            let mut surf = vec![Surface { top: self.blocks.stone, soil: self.blocks.stone, soil_m: 0.0 }; PAD * PAD];
            for x in -1..=CHUNK as i32 {
                for y in -1..=CHUNK as i32 {
                    let at = |xx: i32, yy: i32| cols[((xx.clamp(-1, CHUNK as i32) + 1) as usize) * PAD + (yy.clamp(-1, CHUNK as i32) + 1) as usize].height_m;
                    let dhx = at(x + 1, y) - at(x - 1, y);
                    let dhy = at(x, y + 1) - at(x, y - 1);
                    let slope = (dhx * dhx + dhy * dhy).sqrt() / (2.0 * voxel_m as f32);
                    let p = ((x + 1) as usize) * PAD + (y + 1) as usize;
                    surf[p] = self.surface(&cols[p], slope);
                }
            }
            // Skirt: border columns pretend to be lower, so the chunk's edge
            // voxels grow side faces that hide LOD-seam gaps.
            let skirt_m = (self.skirt as f64 * voxel_m) as f32;
            for x in -1..=CHUNK as i32 {
                for y in -1..=CHUNK as i32 {
                    if x < 0 || y < 0 || x >= CHUNK as i32 || y >= CHUNK as i32 {
                        cols[((x + 1) as usize) * PAD + (y + 1) as usize].height_m -= skirt_m;
                    }
                }
            }
            // Fill the padded grid: voxel z covers layers [k0 + z v, k0 + (z+1) v).
            let world_top = planet.height as f64;
            for x in -1..=CHUNK as i32 {
                for y in -1..=CHUNK as i32 {
                    let p = ((x + 1) as usize) * PAD + (y + 1) as usize;
                    let c = &cols[p];
                    let sf = &surf[p];
                    let mut rock = RockCache::default();
                    for z in -1..=CHUNK as i32 {
                        let k_lo = k0 + z as f64 * vox_blocks;
                        if k_lo >= world_top {
                            continue; // above the world: air
                        }
                        let id = if k_lo < 0.0 {
                            self.blocks.bedrock // below the world
                        } else {
                            self.block_at(c, sf, k_lo * planet.block_size - sea, voxel_m, k_lo, lod, &mut rock)
                        };
                        ids[pidx(x, y, z)] = id;
                        if id == self.blocks.water && (0..CHUNK as i32).contains(&x) && (0..CHUNK as i32).contains(&y) && (0..CHUNK as i32).contains(&z) {
                            water.get_or_insert_with(|| vec![0u8; VOXELS])[crate::world::chunk::index(x as usize, y as usize, z as usize)] = 255;
                        }
                    }
                }
            }
        }
        self.stamp_markers(key, &mut ids);
        // Compress the interior.
        let mut dense = vec![self.blocks.air; VOXELS];
        for x in 0..CHUNK {
            for y in 0..CHUNK {
                for z in 0..CHUNK {
                    dense[crate::world::chunk::index(x, y, z)] = ids[pidx(x as i32, y as i32, z as i32)];
                }
            }
        }
        let chunk = Chunk::from_dense(&dense, water.as_deref());
        Generated { key, ids, chunk, gen_us: t0.elapsed().as_micros() as u32 }
    }

    fn stamp_markers(&self, key: ChunkKey, ids: &mut [BlockId]) {
        if self.markers.is_empty() {
            return;
        }
        let planet = &self.planet;
        let vb = key.voxel_blocks() as i64;
        let (i0, j0, k0) = key.origin_blocks();
        let (i0, j0, k0) = (i0 as i64, j0 as i64, k0 as i64);
        for m in &self.markers {
            let (f, i, j) = planet.lattice_of_dir(m.dir);
            if f != key.face {
                continue;
            }
            let ground_k = planet.sea_level as i64 + (self.ground_height_m(m.dir) / planet.block_size).floor() as i64;
            let half = (m.width_blocks as i64) / 2;
            let (mi0, mi1) = (i.floor() as i64 - half, i.floor() as i64 - half + m.width_blocks as i64);
            let (mj0, mj1) = (j.floor() as i64 - half, j.floor() as i64 - half + m.width_blocks as i64);
            let (mk0, mk1) = if m.below {
                ((ground_k + 1 - m.height_blocks as i64).max(1), ground_k + 1)
            } else {
                (ground_k + 1, ground_k + 1 + m.height_blocks as i64)
            };
            // Voxel ranges (padded coords) that intersect the marker box.
            let vr = |lo: i64, hi: i64, origin: i64| -> (i32, i32) {
                let a = ((lo - origin).div_euclid(vb)).clamp(-1, CHUNK as i64) as i32;
                let b = ((hi - 1 - origin).div_euclid(vb)).clamp(-1, CHUNK as i64) as i32;
                (a, b)
            };
            let (xa, xb) = vr(mi0, mi1, i0);
            let (ya, yb) = vr(mj0, mj1, j0);
            let (za, zb) = vr(mk0, mk1, k0);
            if mi1 <= i0 - vb || mi0 >= i0 + (CHUNK as i64 + 1) * vb || mj1 <= j0 - vb || mj0 >= j0 + (CHUNK as i64 + 1) * vb || mk1 <= k0 - vb || mk0 >= k0 + (CHUNK as i64 + 1) * vb {
                continue;
            }
            for x in xa..=xb {
                for y in ya..=yb {
                    for z in za..=zb {
                        ids[pidx(x, y, z)] = m.block;
                    }
                }
            }
        }
    }
}

/// Grid nodes per axis for a chunk's coarse landform.
const GRID_N: usize = 13;

/// Catmull-Rom interpolation of a chunk's coarse grid at column (x, y):
/// [height, gradient x, gradient y, strata offset].
#[inline]
fn interp_grid(nodes: &[[f64; 4]], x: i32, y: i32) -> [f64; 4] {
    let w = |t: f64| {
        let (t2, t3) = (t * t, t * t * t);
        [0.5 * (-t3 + 2.0 * t2 - t), 0.5 * (3.0 * t3 - 5.0 * t2 + 2.0), 0.5 * (-3.0 * t3 + 4.0 * t2 + t), 0.5 * (t3 - t2)]
    };
    let (tx, ty) = ((x + 8) as f64 / 4.0, (y + 8) as f64 / 4.0);
    let (nx, ny) = (tx.floor() as usize, ty.floor() as usize);
    let (wx, wy) = (w(tx - nx as f64), w(ty - ny as f64));
    let mut out = [0.0; 4];
    for (da, wa) in wx.iter().enumerate() {
        for (db, wb) in wy.iter().enumerate() {
            let n = nodes[(nx + da - 1) * GRID_N + (ny + db - 1)];
            let k = wa * wb;
            for (o, v) in out.iter_mut().zip(n) {
                *o += k * v;
            }
        }
    }
    out
}

/// The cube face a direction points into (largest component, FACES order).
#[inline]
fn dominant_face(d: DVec3) -> u8 {
    let a = d.abs();
    if a.x >= a.y && a.x >= a.z {
        if d.x >= 0.0 { 0 } else { 1 }
    } else if a.y >= a.z {
        if d.y >= 0.0 { 2 } else { 3 }
    } else if d.z >= 0.0 {
        4
    } else {
        5
    }
}

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// C1 soft floor: `h` above `floor + k` is unchanged; below, it eases
/// toward `floor` and never reaches it.
#[inline]
fn soft_floor(h: f64, floor: f64, k: f64) -> f64 {
    if h >= floor + k { h } else { floor + k * ((h - floor) / k - 1.0).exp() }
}

#[cfg(test)]
mod tests;

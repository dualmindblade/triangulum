//! Generator v1: a pure function of (seed, position, LOD voxel size).
//!
//! Height comes from smooth bake sampling times the vertical scale, plus
//! gradient-noise relief modulated by the bake's roughness field and the
//! roughness dial, band-limited to the voxel size so every LOD samples the
//! same surface. Materials come from climate; strata are real blocks.
//! A chunk is generated directly at its LOD from the column samples at its
//! voxel centres, with a one-voxel border so the mesher can cull and shade
//! faces against the neighbours without loading them.

pub mod bake;
pub mod inspect;
pub mod noise;
pub mod strata;

use crate::grid::{CHUNK, ChunkKey, FootprintKey, Planet};
use crate::world::chunk::VOXELS;
use crate::world::{BlockId, Chunk, Registry};
use bake::Bake;
use glam::DVec3;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use noise::{fbm, gradient_noise};
use strata::{GeoHint, Strata};

/// The landform dial every new `Generator` starts with (1.0 unless `main`
/// sets it from world.json / --landform). `Engine::build` passes only the
/// roughness dial to `Generator::new`, and engine.rs belongs to another
/// mission right now; passing the dial explicitly there is a one-line
/// follow-up that makes this global unnecessary.
static DEFAULT_LANDFORM_BITS: AtomicU64 = AtomicU64::new(0x3FF0_0000_0000_0000);

pub fn set_default_landform(dial: f64) {
    DEFAULT_LANDFORM_BITS.store(dial.to_bits(), Ordering::Relaxed);
}

pub fn default_landform() -> f64 {
    f64::from_bits(DEFAULT_LANDFORM_BITS.load(Ordering::Relaxed))
}

/// Padded voxel grid edge (one voxel border on every side).
pub const PAD: usize = CHUNK + 2;
pub const PAD_VOXELS: usize = PAD * PAD * PAD;

/// Index into a padded grid for voxel coordinates in [-1, 32].
#[inline]
pub fn pidx(x: i32, y: i32, z: i32) -> usize {
    (((x + 1) as usize * PAD) + (y + 1) as usize) * PAD + (z + 1) as usize
}

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
}

/// Surface and soil of a column once the slope is known.
#[derive(Clone, Copy, Debug)]
struct Surface {
    top: BlockId,
    soil: BlockId,
    /// Soil thickness in metres below the top block.
    soil_m: f32,
}

pub struct Generator {
    pub planet: Planet,
    pub bake: Arc<dyn Bake>,
    pub seed: u64,
    /// Metres of terrain per metre of bake elevation.
    pub vscale: f64,
    /// Andrew's relief dial (1.0 = default).
    pub roughness: f64,
    /// Andrew's landform dial (1.0 = default).
    pub landform: f64,
    pub blocks: Blocks,
    pub strata: Strata,
    /// Border columns are lowered by this many voxels so chunk edges grow
    /// a few side faces that hide LOD-seam gaps.
    pub skirt: u32,
    pub markers: Vec<Marker>,
    /// Debug: generate no water (ocean columns stay dry so the floor shows).
    pub hide_water: bool,
    /// Largest relief the noise can add (m), for conservative bounds.
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
    pub fn new(planet: Planet, bake: Arc<dyn Bake>, seed: u64, vscale: f64, roughness: f64, registry: &Registry) -> Result<Self, String> {
        let blocks = Blocks::resolve(registry)?;
        let beds = ["shale", "mudstone", "umber", "sandstone", "limestone"]
            .iter()
            .map(|n| registry.id(n))
            .collect::<Result<Vec<_>, _>>()?;
        let strata = Strata::new(beds, planet.block_size, planet.radius, seed);
        let max_relief_m = relief_amp_m(bake.rough_max_km() as f64, vscale, roughness) * 2.0;
        Ok(Self {
            planet,
            bake,
            seed,
            vscale,
            roughness,
            landform: default_landform(),
            blocks,
            strata,
            skirt: 2,
            markers: Vec::new(),
            hide_water: false,
            max_relief_m,
        })
    }

    /// One line naming the relief model and its dials (captions, logs).
    pub fn describe_relief(&self) -> String {
        format!("relief v1 (gradient fBm), roughness {}", self.roughness)
    }

    /// Conservative padding (m) above and below the bake's height range,
    /// for the worst roughness anywhere (the horizon occluder uses it).
    pub fn height_margin_m(&self) -> f64 {
        self.max_relief_m + (self.skirt as f64 + 2.0) * self.planet.block_size
    }

    /// The same padding for a known local roughness (km).
    fn local_margin_m(&self, rough_km: f64) -> f64 {
        // Six fBm octaves at gain 0.5 sum to just under twice the first.
        relief_amp_m(rough_km, self.vscale, self.roughness) * 2.0 + (self.skirt as f64 + 2.0) * self.planet.block_size
    }

    /// Sample the column at a direction, band-limited for `voxel_m`-sized voxels.
    pub fn column(&self, dir: DVec3, voxel_m: f64) -> Column {
        let s = self.bake.sample(dir);
        let elev = s.elev_km * 1000.0 * self.vscale;
        let ocean = s.ocean > 0.5 && elev < 0.0;
        let p = dir * self.planet.radius;
        // Relief: gradient fBm from a 10 km wavelength (the bake carries no
        // structure between its ~30 km cells and the texel size) down to
        // 16 m, amplitude from the roughness field and the dial, faded out
        // near the shoreline so coasts stay clean, reduced on the sea floor.
        // Octaves finer than four voxels are faded out, so every LOD samples
        // the same band-limited surface.
        let amp = relief_amp_m(s.rough_km, self.vscale, self.roughness);
        let land_fac = if ocean { 0.35 } else { 0.25 + 0.75 * smoothstep(0.0, 10.0 * self.planet.block_size, elev) };
        let cutoff = voxel_m * 4.0;
        // Each octave's slope is capped at RELIEF_MAX_EARTH_SLOPE (times the
        // vertical scale and the dial): the roughness field is relief at
        // ~30 km, and extrapolating it down to metres unchecked makes
        // mountains a thicket of 1 m cliffs.
        let slope_cap = RELIEF_MAX_EARTH_SLOPE * self.vscale * self.roughness / std::f64::consts::TAU;
        let relief = fbm(p, RELIEF_OCTAVES, RELIEF_WL_M, 2.5, RELIEF_GAIN, self.seed, |o, wl| {
            let band = ((wl / cutoff) - 1.0).clamp(0.0, 1.0);
            let octave_amp = amp * RELIEF_GAIN.powi(o as i32);
            band * (slope_cap * wl / octave_amp).min(1.0)
        }) * amp
            * land_fac;
        let height = elev + relief;
        let dither = gradient_noise(p / 41.0 + DVec3::splat(3.1), self.seed ^ 0xD17) as f32;
        Column {
            height_m: height as f32,
            ocean,
            temp_c: s.temp_c as f32,
            precip: s.precip_mm as f32,
            shore: s.ocean as f32,
            dither,
            strata_off: self.strata.column_offset_m(dir) as f32,
            soil_km: s.soil_km as f32,
            geo: GeoHint {
                craton: s.continental as f32,
                volcanic: s.volcanic as f32,
                hardness: if s.erodibility > 0.0 { (1.0 / s.erodibility).clamp(0.0, 1.0) as f32 } else { 0.0 },
            },
        }
    }

    /// Ground height (m above sea level) under a direction; water counts as 0.
    pub fn ground_height_m(&self, dir: DVec3) -> f64 {
        let c = self.column(dir, self.planet.block_size);
        if c.ocean && !self.hide_water { 0.0 } else { c.height_m as f64 }
    }

    /// Conservative (min, max) surface height in metres over a footprint
    /// (water surface included, markers included).
    pub fn footprint_height_range_m(&self, fp: FootprintKey) -> (f64, f64) {
        let planet = &self.planet;
        let size = fp.extent_blocks() as f64;
        let (i0, j0) = (fp.cx as f64 * size, fp.cy as f64 * size);
        let (i1, j1) = ((i0 + size).min(planet.n as f64), (j0 + size).min(planet.n as f64));
        let (a0, b0) = cubesphere::warp(planet.proj, planet.p_of(i0), planet.p_of(j0));
        let (a1, b1) = cubesphere::warp(planet.proj, planet.p_of(i1), planet.p_of(j1));
        let (emin, emax) = self.bake.elev_range_km(fp.face, a0, b0, a1, b1);
        let margin = self.local_margin_m(self.bake.rough_max_range_km(fp.face, a0, b0, a1, b1) as f64);
        let mut hmin = emin as f64 * 1000.0 * self.vscale - margin;
        let mut hmax = (emax as f64 * 1000.0 * self.vscale).max(0.0) + margin;
        for m in &self.markers {
            let (f, i, j) = planet.lattice_of_dir(m.dir);
            if f == fp.face && i >= i0 && i < i1 && j >= j0 && j < j1 {
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

    /// Surface block and the soil under it. Soil is 1-4 blocks measured
    /// from the natural ground: thicker on flat, warm, wet ground, thinner
    /// on steep rock (planetgen's soil_km will replace this with the new bake).
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

    /// Block at voxel centre elevation `e_m` (above sea level) in a column.
    #[inline]
    fn block_at(&self, c: &Column, sf: &Surface, e_m: f64, voxel_m: f64, k_bottom: bool) -> BlockId {
        let b = &self.blocks;
        let h = c.height_m as f64;
        if e_m >= h {
            if c.ocean && !self.hide_water && e_m < 0.0 {
                // Top water voxel freezes in the cold.
                if c.temp_c + 1.5 * c.dither < -4.0 && e_m + voxel_m >= 0.0 {
                    return b.ice;
                }
                return b.water;
            }
            return b.air;
        }
        if k_bottom {
            return b.bedrock;
        }
        let depth = h - e_m;
        if depth < voxel_m {
            sf.top
        } else if depth < voxel_m + sf.soil_m as f64 {
            sf.soil
        } else {
            self.strata.rock_at(e_m, c.strata_off as f64, c.geo)
        }
    }

    /// Generate one chunk at its own LOD.
    pub fn generate(&self, key: ChunkKey) -> Generated {
        let t0 = std::time::Instant::now();
        let planet = &self.planet;
        let lod = key.lod;
        let vox_blocks = key.voxel_blocks() as f64;
        let voxel_m = planet.voxel_m(lod);
        let (i0, j0, k0) = key.origin_blocks();
        let (i0, j0, k0) = (i0 as f64, j0 as f64, k0 as f64);
        let sea = planet.sea_level as f64 * planet.block_size;

        // Column samples with a one-voxel border.
        let mut cols = vec![Column::default(); PAD * PAD];
        let (mut top_max, mut height_min) = (f32::MIN, f32::MAX);
        let water_on = !self.hide_water;
        for x in -1..=CHUNK as i32 {
            for y in -1..=CHUNK as i32 {
                let i = i0 + (x as f64 + 0.5) * vox_blocks;
                let j = j0 + (y as f64 + 0.5) * vox_blocks;
                let dir = planet.dir(key.face, i, j);
                let c = self.column(dir, voxel_m);
                let p = ((x + 1) as usize) * PAD + (y + 1) as usize;
                top_max = top_max.max(if c.ocean && water_on { c.height_m.max(0.0) } else { c.height_m });
                height_min = height_min.min(c.height_m);
                cols[p] = c;
            }
        }
        // Chunk vertical extent in metres above sea level.
        let z0 = k0 * planet.block_size - sea;
        let z1 = (k0 + CHUNK as f64 * vox_blocks) * planet.block_size - sea;
        let skirt_m = self.skirt as f64 * voxel_m;
        // Early out: a chunk entirely above every surface is air, one entirely
        // below every (skirted) surface is enclosed rock. Neither has a face.
        if self.markers.is_empty() && (z0 >= top_max as f64 || z1 <= height_min as f64 - skirt_m) {
            let id = if z0 >= top_max as f64 { self.blocks.air } else { self.blocks.stone };
            return Generated { key, ids: vec![id; PAD_VOXELS], chunk: Chunk::uniform(id), gen_us: t0.elapsed().as_micros() as u32 };
        }
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
        for x in -1..=CHUNK as i32 {
            for y in -1..=CHUNK as i32 {
                if x < 0 || y < 0 || x >= CHUNK as i32 || y >= CHUNK as i32 {
                    cols[((x + 1) as usize) * PAD + (y + 1) as usize].height_m -= skirt_m as f32;
                }
            }
        }
        // Fill the padded grid: voxel z covers layers [k0 + z v, k0 + (z+1) v).
        let mut ids = vec![self.blocks.air; PAD_VOXELS];
        let mut water: Option<Vec<u8>> = None;
        let world_top = planet.height as f64;
        for x in -1..=CHUNK as i32 {
            for y in -1..=CHUNK as i32 {
                let p = ((x + 1) as usize) * PAD + (y + 1) as usize;
                let c = &cols[p];
                let sf = &surf[p];
                for z in -1..=CHUNK as i32 {
                    let k_lo = k0 + z as f64 * vox_blocks;
                    if k_lo >= world_top {
                        continue; // clipped: above the world (air)
                    }
                    if k_lo < 0.0 {
                        ids[pidx(x, y, z)] = self.blocks.bedrock; // below the world: nothing to see
                        continue;
                    }
                    let e_m = (k_lo + 0.5 * vox_blocks) * planet.block_size - sea;
                    let id = self.block_at(c, sf, e_m, voxel_m, k_lo == 0.0 && lod == 0);
                    ids[pidx(x, y, z)] = id;
                    if id == self.blocks.water && (0..CHUNK as i32).contains(&x) && (0..CHUNK as i32).contains(&y) && (0..CHUNK as i32).contains(&z) {
                        water.get_or_insert_with(|| vec![0u8; VOXELS])[crate::world::chunk::index(x as usize, y as usize, z as usize)] = 255;
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

/// Relief octaves: 10 km, 4 km, 1.6 km, 640 m, 256 m, 102 m, 41 m, 16 m.
const RELIEF_OCTAVES: u32 = 8;
const RELIEF_WL_M: f64 = 10_000.0;
/// Amplitude ratio per octave (lacunarity 2.5): slopes stay about equal
/// across octaves instead of growing toward the fine ones.
const RELIEF_GAIN: f64 = 0.42;
/// Largest Earth-scale slope one octave may add (rise over run).
const RELIEF_MAX_EARTH_SLOPE: f64 = 0.7;

/// First-octave relief amplitude (m) for a roughness value (km, Earth
/// scale). The bake's roughness is relief at the ~30 km cell scale; at the
/// 10 km first octave a self-affine landscape keeps about half of it. The
/// 1.5 m floor keeps plains from being dead flat. Six to eight octaves at
/// gain 0.5 sum to just under twice this.
fn relief_amp_m(rough_km: f64, vscale: f64, dial: f64) -> f64 {
    (rough_km * 1000.0 * vscale * 0.5 + 1.5) * dial
}

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn test_generator() -> Generator {
        let registry = Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap();
        let bake = Arc::new(bake::open("C:/code/triangulum/game/assets/world/seed42").expect("world bake"));
        let planet = Planet::new(1.0, 2048, 512, 131072);
        let vscale = planet.radius / 6_371_000.0;
        Generator::new(planet, bake, 42, vscale, 1.0, &registry).unwrap()
    }

    #[test]
    fn determinism_across_runs_and_thread_counts() {
        let g = test_generator();
        let keys = [
            ChunkKey { face: 2, lod: 0, cx: 3295, cy: 1813, cz: 16 },
            ChunkKey { face: 2, lod: 3, cx: 400, cy: 220, cz: 2 },
            ChunkKey { face: 0, lod: 9, cx: 3, cy: 5, cz: 0 },
        ];
        let once: Vec<Vec<u8>> = keys.iter().map(|k| g.generate(*k).chunk.to_bytes()).collect();
        for threads in [1usize, 8] {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
            let again: Vec<Vec<u8>> = pool.install(|| {
                use rayon::prelude::*;
                keys.par_iter().map(|k| g.generate(*k).chunk.to_bytes()).collect()
            });
            assert_eq!(once, again, "chunk bytes differ with {threads} threads");
        }
        let ids_a = g.generate(keys[0]).ids;
        let ids_b = g.generate(keys[0]).ids;
        assert_eq!(ids_a, ids_b);
    }

    #[test]
    fn surface_chunk_has_air_above_and_rock_below() {
        let g = test_generator();
        let dir = cubesphere::face_to_dir(g.planet.proj, 2, 0.5, -0.1);
        let (f, i, j) = g.planet.lattice_of_dir(dir);
        let h = g.ground_height_m(dir);
        let k = g.planet.sea_level as f64 + h;
        let key = ChunkKey { face: f, lod: 0, cx: i as u32 / 32, cy: j as u32 / 32, cz: (k as u32) / 32 };
        let generated = g.generate(key);
        let top = generated.ids[pidx(0, 0, 32)];
        let bottom = generated.ids[pidx(0, 0, -1)];
        assert_eq!(top, g.blocks.air);
        assert!(bottom != g.blocks.air && bottom != g.blocks.water, "bottom {bottom}");
        assert!(!generated.chunk.is_uniform());
    }
}

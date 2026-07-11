//! Planet data: baked cube-face rasters + cube-sphere math.
//!
//! Face convention must match scripts/bake_faces.py:
//!   direction(u, v) = normalize(axis + u*right + v*up),  u, v in [-1, 1]

use anyhow::{Context, Result};
use glam::DVec3;

use crate::noise::normal_value_noise;

pub const FACES: [(DVec3, DVec3, DVec3); 6] = [
    (DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(-1.0, 0.0, 0.0), DVec3::new(0.0, -1.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, -1.0, 0.0), DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, 0.0, 1.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0)),
    (DVec3::new(0.0, 0.0, -1.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(1.0, 0.0, 0.0)),
];

/// The canonical one-metre-ish column lattice. Biome dithering snaps to this
/// identity in both renderers; changing it would move every ecotone column.
pub const COLUMNS_PER_FACE: u64 = 10_000_000;

/// Total width of a cross-material ecotone (half on either side of the baked
/// Koppen edge). Same-material classes never consult this band.
pub const CROSS_BLOCK_ECOTONE_KM: f64 = 0.300;
/// Koppen contributes a small, spatially blended hue memory; continuous
/// temperature and precipitation remain the dominant color coordinates.
pub const KOPPEN_HUE_NUDGE: f32 = 0.14;

// ---- Andrew's continental biome-warp art-direction knobs ---------------
// The baked climate raster is categorical, so its otherwise straight texel
// edges are displaced in climate-lookup space before class and tint reads.
// Amplitude is in baked texels. Wavelength is also in baked texels at a face
// center; LACUNARITY divides it and PERSISTENCE scales amplitude per octave.
// On Neisor the defaults are approximately:
//   wavelength 68 / 15 / 3.4 km, amplitude 6.8 / 2.0 / 0.61 km before the
//   0.70-texel cap. The last octave is the intermediate-flight layer: broad
//   enough to read around 1 km altitude, but still hands off cleanly to the
//   300 m categorical ecotone below it.
// One inexpensive smooth vector-noise evaluation supplies each broad octave;
// the final octave weaves two projections. The class-interior fast path skips
// the whole stack when it cannot matter.
pub const BIOME_WARP_OCTAVES: u32 = 3;
pub const BIOME_WARP_BASE_AMPLITUDE_TEXELS: f64 = 0.40;
pub const BIOME_WARP_BASE_WAVELENGTH_TEXELS: f64 = 4.0;
pub const BIOME_WARP_LACUNARITY: f64 = 4.5;
pub const BIOME_WARP_PERSISTENCE: f64 = 0.30;
pub const BIOME_WARP_MAX_DISPLACEMENT_TEXELS: f64 = 0.70;

const BIOME_WARP_SEED_OFFSET: i64 = 0x0B10_0A2E;
const BIOME_WARP_WOVEN_OCTAVE: usize = 2;

// ---- Andrew's fractal ecotone art-direction knobs -----------------------
// OCTAVES is the number of nested spatial layers (and noise evaluations).
// BASE_PATCH_COLUMNS is the broadest layer's approximate wavelength on the
// canonical face lattice: 4096 columns is ~0.4 of a 1024-bake texel and
// ~7 km at a face center on Neisor.
// LACUNARITY divides wavelength between layers. At 16, the four defaults are
// about 4096, 256, 16, and 1 column: map-scale islands down to voxel speckle.
// PERSISTENCE multiplies amplitude per finer layer. 0.5 lets broad islands
// own the silhouette while each smaller layer can perforate their margins.
pub const ECOTONE_FIELD_OCTAVES: u32 = 4;
pub const ECOTONE_BASE_PATCH_COLUMNS: f64 = 4_096.0;
pub const ECOTONE_LACUNARITY: f64 = 16.0;
pub const ECOTONE_PERSISTENCE: f64 = 0.5;

const ECOTONE_FIELD_SEED_OFFSET: i64 = 0x0EC0_70AE;
const SNOW_FIELD_SEED_OFFSET: i64 = 0x0000_5A0E;
const SNOWLINE_CENTER_C: f64 = -9.0;
const SNOWLINE_HALF_RANGE_C: f64 = 1.5;

/// The categories Koppen actually selects in today's surface material code.
/// Rock and stone are local-slope overrides, not biome main blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MainBlock {
    Grass,
    Sand,
    Snow,
}

pub fn koppen_main_block(id: u8) -> MainBlock {
    match id {
        3 | 4 | 255 => MainBlock::Sand,
        29 => MainBlock::Snow,
        _ => MainBlock::Grass,
    }
}

/// The shared climate/material result evaluated once per mesh vertex or voxel
/// column. All three tints are carried because a local beach/cliff rule may
/// override the biome's main block without another raster pass.
#[derive(Clone, Copy, Debug)]
pub struct ClimateSurface {
    pub main_block: MainBlock,
    grass: [f32; 3],
    sand: [f32; 3],
    snow: [f32; 3],
    /// Existing far-canopy approximation, spatially blended so it cannot put
    /// a second hard line back across a smooth same-block transition.
    pub forest: f32,
}

/// Climate values sampled at the one shared, domain-warped biome position.
/// Terrain/weather/water callers deliberately continue to use `temp()` and
/// `precip()` at the original world position; this type is for biome visuals.
#[derive(Clone, Copy, Debug)]
pub struct BiomeClimate {
    pub koppen: u8,
    pub temp_c: f32,
    pub precip_mm_yr: f32,
    /// Physical sea ownership at the original, unwarped world position.
    /// A dry coastal column may legitimately carry the baked `255` sentinel;
    /// consumers must use this bit, not the class byte, to identify water.
    pub sea: bool,
}

impl ClimateSurface {
    pub fn tint(self, block: MainBlock) -> [f32; 3] {
        match block {
            MainBlock::Grass => self.grass,
            MainBlock::Sand => self.sand,
            MainBlock::Snow => self.snow,
        }
    }
}

/// Shared splitmix-style column hash. Its exact output remains stable for
/// beaches, decorations, and other per-column consumers; categorical climate
/// transitions use the correlated field below instead.
pub fn hash_u64(face: u8, ci: u64, cj: u64, salt: u64) -> u64 {
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

pub fn hash01(face: u8, ci: u64, cj: u64, salt: u64) -> f64 {
    ((hash_u64(face, ci, cj, salt) >> 11) & 0xFFFF_FFFF) as f64 / 4294967296.0
}

/// Unit direction for face-local coordinates (u, v) in [-1, 1].
pub fn face_dir(face: usize, u: f64, v: f64) -> DVec3 {
    let (axis, right, up) = FACES[face];
    (axis + u * right + v * up).normalize()
}

/// Inverse gnomonic projection: which face a direction hits, and where.
pub fn face_from_dir(dir: DVec3) -> (usize, f64, f64) {
    let mut best = (0usize, f64::MIN);
    for (i, (axis, _, _)) in FACES.iter().enumerate() {
        let d = dir.dot(*axis);
        if d > best.1 {
            best = (i, d);
        }
    }
    let (axis, right, up) = FACES[best.0];
    let p = dir / dir.dot(axis);
    (best.0, p.dot(right), p.dot(up))
}

#[inline]
fn uv_from_dir_on_face(face: usize, dir: DVec3) -> (f64, f64) {
    let (axis, right, up) = FACES[face];
    let p = dir / dir.dot(axis);
    (p.dot(right), p.dot(up))
}

/// Canonical face ownership is needed only on shared cube edges. It makes a
/// world position presented through either adjacent face follow the exact same
/// floating-point path before any procedural field is evaluated.
#[inline]
fn canonical_face_uv(face: usize, u: f64, v: f64) -> (usize, f64, f64) {
    if u.abs() >= 1.0 || v.abs() >= 1.0 {
        face_from_dir(face_dir(face, u, v))
    } else {
        (face, u, v)
    }
}

/// The physical open-sea classifier shared with `terrain::sample`.
///
/// This deliberately remains a function of UNWARPED landscape rasters. The
/// biome transform may move climate appearance, never water ownership.
#[inline]
pub fn sea_from_fields(e_raw_km: f64, water_mask: f64, ocean_frac: f64) -> bool {
    e_raw_km <= 0.0
        && (water_mask >= 0.5 || (e_raw_km <= -0.1 && ocean_frac > 0.35))
}

// Seed-independent irrational-looking directions for the three vector-noise
// channels. Each row rotates the domain at the next octave; all are normalized
// integer triples, so no cube-face or world-axis direction owns the result.
const BIOME_WARP_AXES: [[DVec3; 3]; 3] = [
    [
        DVec3::new(0.811_107_105_653_812_7, 0.324_442_842_261_525_1, -0.486_664_263_392_287_6),
        DVec3::new(-0.235_702_260_395_515_9, 0.942_809_041_582_063_4, 0.235_702_260_395_515_9),
        DVec3::new(0.371_390_676_354_103_7, -0.557_086_014_531_155_6, 0.742_781_352_708_207_4),
    ],
    [
        DVec3::new(-0.685_994_340_570_035_4, 0.514_495_755_427_526_5, 0.514_495_755_427_526_5),
        DVec3::new(0.696_310_623_822_791_4, 0.696_310_623_822_791_4, -0.174_077_655_955_697_9),
        DVec3::new(-0.365_148_371_670_110_7, 0.182_574_185_835_055_4, 0.912_870_929_175_276_9),
    ],
    [
        DVec3::new(0.486_664_263_392_287_6, -0.811_107_105_653_812_7, -0.324_442_842_261_525_1),
        DVec3::new(0.169_030_850_945_703_3, 0.507_092_552_837_110_0, 0.845_154_254_728_516_6),
        DVec3::new(0.745_355_992_499_929_9, -0.298_142_396_999_972_0, 0.596_284_793_999_943_9),
    ],
];

/// Two C2-continuous alternating value ramps in phase quadrature. One domain
/// projection and floor supplies both vector channels, keeping each octave
/// substantially cheaper than three independent scalar-noise evaluations.
#[inline]
fn smooth_noise_wave_pair(x: f64) -> (f64, f64) {
    let cell = x.floor();
    let t = x - cell;
    let wave = |cell: i64, t: f64| {
        let fade = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        if cell & 1 == 0 { 2.0 * fade - 1.0 } else { 1.0 - 2.0 * fade }
    };
    let cell = cell as i64;
    let first = wave(cell, t);
    let second = if t < 0.5 {
        wave(cell, t + 0.5)
    } else {
        wave(cell + 1, t - 0.5)
    };
    (first, second)
}

#[inline]
fn biome_vector_noise(dir: DVec3, frequency: f64, seed_phase: f64, octave: usize) -> DVec3 {
    let axes = BIOME_WARP_AXES[octave % BIOME_WARP_AXES.len()];
    let octave_phase = seed_phase + octave as f64 * 0.618_033_988_749_894_9;
    let (a, b) = smooth_noise_wave_pair(dir.dot(axes[0]) * frequency + octave_phase);
    if octave == BIOME_WARP_WOVEN_OCTAVE {
        // The broad approved layers keep their original inexpensive stripe
        // field. The added flight-scale octave weaves a second projection into
        // it, so a boundary parallel to one projection (notably lon -45) still
        // receives variation along its length. RMS amplitude stays unchanged.
        let (c, d) = smooth_noise_wave_pair(
            dir.dot(axes[1]) * frequency
                + octave_phase * 1.732_050_807_568_877_2
                + 0.414_213_562_373_095_0,
        );
        (axes[1] * (a + d) + axes[2] * (b - c))
            * (1.15 * std::f64::consts::FRAC_1_SQRT_2)
    } else {
        (axes[1] * a + axes[2] * b) * 1.15
    }
}

/// Smallest gnomonic angular derivative at this face position. Scaling the
/// direction-space displacement by it makes the maximum a true local raster-
/// texel bound even at cube corners, where one face-center angular texel would
/// otherwise span more than two raster texels. The expression is symmetric
/// across shared face representations.
#[inline]
fn biome_warp_metric_scale(u: f64, v: f64) -> f64 {
    let denom = 1.0 + u * u + v * v;
    // 1/denom is a conservative lower bound on both exact derivatives. It
    // keeps the proof strict without adding square roots to every edge lookup.
    1.0 / denom
}

/// Pure, direction-space domain warp. The vector field is sampled in 3-D and
/// projected onto the sphere tangent, so it has neither cube-face axes nor
/// seams. The final displacement is angular and scaled in baked texels.
fn biome_warp_dir(dir: DVec3, res: usize, seed: i64, metric_scale: f64) -> DVec3 {
    debug_assert!(res > 1);
    let texel_angle = 2.0 / (res - 1) as f64;
    // smooth_noise_wave_pair has a two-cell period.
    let mut frequency = 2.0 / (texel_angle * BIOME_WARP_BASE_WAVELENGTH_TEXELS);
    let mut amplitude = BIOME_WARP_BASE_AMPLITUDE_TEXELS;
    let mut offset = DVec3::ZERO;
    let mut phase_bits = (seed.wrapping_add(BIOME_WARP_SEED_OFFSET) as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    phase_bits ^= phase_bits >> 30;
    phase_bits = phase_bits.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    phase_bits ^= phase_bits >> 27;
    let seed_phase = (phase_bits >> 11) as f64 * (2.0 / 9_007_199_254_740_992.0);
    for octave in 0..BIOME_WARP_OCTAVES as usize {
        let vector = biome_vector_noise(dir, frequency, seed_phase, octave);
        offset += amplitude * (vector - dir * vector.dot(dir));
        frequency *= BIOME_WARP_LACUNARITY;
        amplitude *= BIOME_WARP_PERSISTENCE;
    }
    let length = offset.length();
    if length > BIOME_WARP_MAX_DISPLACEMENT_TEXELS {
        offset *= BIOME_WARP_MAX_DISPLACEMENT_TEXELS / length;
    }
    // Projection back to a cube face is homogeneous, so normalization here
    // would be a wasted square root in the hot path.
    dir + texel_angle * metric_scale * offset
}

/// Fast standard-normal CDF (Abramowitz-Stegun 26.2.17, max error about
/// 7.5e-8). The ecotone stack is normalized to N(0,1), so this probability
/// integral transform makes its comparator approximately uniform on [0,1].
#[inline]
fn standard_normal_cdf(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.231_641_9 * z);
    let tail = 0.398_942_280_401_432_7
        * (-0.5 * z * z).exp()
        * t
        * (0.319_381_530
            + t * (-0.356_563_782
                + t * (1.781_477_937 + t * (-1.821_255_978 + t * 1.330_274_429))));
    if x < 0.0 { tail } else { 1.0 - tail }
}

/// The one categorical-dither truth used by mesh vertices and voxel columns.
/// Input is snapped to the canonical column center before any noise is read,
/// so every sample within a column gets the same answer. Noise is evaluated
/// on the 3-D unit direction, making the field continuous across cube faces.
/// `seed` selects an independent deterministic stream for each consumer.
fn ecotone_comparator(face: usize, u: f64, v: f64, seed: i64) -> f64 {
    let (face, u, v) = if u.abs() >= 1.0 || v.abs() >= 1.0 {
        face_from_dir(face_dir(face, u, v))
    } else {
        (face, u, v)
    };
    let n = COLUMNS_PER_FACE as f64;
    let ci = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let cj = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let uc = -1.0 + 2.0 * (ci as f64 + 0.5) / n;
    let vc = -1.0 + 2.0 * (cj as f64 + 0.5) / n;
    let dir = face_dir(face, uc, vc);

    let mut frequency = n / (2.0 * ECOTONE_BASE_PATCH_COLUMNS);
    let mut amplitude = 1.0;
    let mut sum = 0.0;
    let mut variance = 0.0;
    for octave in 0..ECOTONE_FIELD_OCTAVES as i64 {
        let octave_seed = seed.wrapping_mul(7_919).wrapping_add(octave * 131);
        sum += amplitude * normal_value_noise(dir * frequency, octave_seed);
        variance += amplitude * amplitude;
        frequency *= ECOTONE_LACUNARITY;
        amplitude *= ECOTONE_PERSISTENCE;
    }
    standard_normal_cdf(sum / variance.sqrt()).clamp(0.0, 1.0)
}

pub struct FaceRaster {
    pub res: usize,
    pub elev_km: Vec<f32>,
    pub koppen: Vec<u8>, // 255 = ocean
    /// 1 where this texel's 8-neighbor ring contains another class. A
    /// sub-texel domain warp cannot change class outside this conservative
    /// strip, so those interiors skip every procedural warp evaluation.
    climate_edge: Vec<u64>,
    pub rough_km: Vec<f32>,     // mean |elevation delta| between map cells
    /// Interleaved [annual mean temperature C, annual precipitation mm].
    /// These are read together by terrain and biome tint paths, so one cache
    /// stream serves both without changing either field's coordinates.
    climate: Vec<[f32; 2]>,
    pub flow_log10: Vec<f32>,   // log10(1 + river flow accumulation m3/s)
    /// Blurred is-ocean mask (0 = interior land, 1 = open sea), derived from
    /// koppen==255 at load. "Below sea level" alone is NOT ocean: the map has
    /// genuine dry depressions, and elevation dips a few meters under zero
    /// all along the coasts. The blur (radius 2 texels) keeps one mislabeled
    /// texel from drying out a strait or flooding an inland dip.
    pub ocean: Vec<f32>,
    /// UNBLURRED is-ocean mask (koppen==255 as 0/1). Bilinear over this is
    /// the map's own cell-resolution coastline — the authority on which side
    /// of the shore a point is. Sub-sea-level interpolation tongues reaching
    /// inland of it are undershoot artifacts, not water.
    pub water: Vec<f32>,
}

pub struct Planet {
    pub radius_km: f64,
    pub seed: i64,
    pub faces: Vec<FaceRaster>,
    /// River courses + lakes from the drainage graph (empty if rivers.bin
    /// is missing — run scripts/bake_rivers.py).
    pub rivers: crate::rivers::RiverIndex,
}

#[derive(Clone, Copy, Debug)]
struct RasterPosition {
    face: usize,
    u: f64,
    v: f64,
    x: f64,
    y: f64,
    here: u8,
    climate_edge: bool,
}

#[derive(Clone, Copy, Debug)]
struct ClimatePosition {
    raster: RasterPosition,
    warped: bool,
}

impl Planet {
    pub fn load(dir: &str) -> Result<Self> {
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{dir}/meta.json"))
                .context("missing viewer/assets/meta.json - run scripts/bake_faces.py")?,
        )?;
        let res = meta["resolution"].as_u64().unwrap() as usize;
        let radius_km = meta["radius_km"].as_f64().unwrap();
        let seed = meta["seed"].as_i64().unwrap_or(42);
        let mut faces = Vec::new();
        for fi in 0..6 {
            let raw = std::fs::read(format!("{dir}/face_{fi}.bin"))?;
            let n = res * res;
            anyhow::ensure!(
                raw.len() == n * 21,
                "face_{fi}.bin has unexpected size - rerun scripts/bake_faces.py (format now carries rough/precip/temp/flow layers)"
            );
            let f32s = |off: usize| -> Vec<f32> {
                raw[off..off + n * 4]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect()
            };
            let elev_km = f32s(0);
            let koppen = raw[n * 4..n * 5].to_vec();
            let climate_edge = climate_edge_mask(&koppen, res);
            let rough_km = f32s(n * 5);
            let precip_mm_yr = f32s(n * 9);
            let temp_c = f32s(n * 13);
            let climate = temp_c.into_iter().zip(precip_mm_yr).map(|(t, p)| [t, p]).collect();
            let flow_log10 = f32s(n * 17);
            let ocean = blur_mask(&koppen, res, 2);
            let water = koppen.iter().map(|&k| (k == 255) as u8 as f32).collect();
            faces.push(FaceRaster {
                res,
                elev_km,
                koppen,
                climate_edge,
                rough_km,
                climate,
                flow_log10,
                ocean,
                water,
            });
        }
        // The per-face blur above is edge-CLAMPED, so each face averages only
        // its own interior near a shared cube edge — the derived ocean value
        // then DISAGREES between two faces at the very same world direction
        // (BUGS.md: 0.32 via face 0 vs 0.56 via face 3 at lat -14.457 lon -45,
        // a half-screen navy/sand split at the seam). Re-derive the near-edge
        // band over true neighbor data and force shared texels bit-identical.
        seam_exact_ocean(&mut faces, 2);
        let rivers = match crate::rivers::RiverIndex::load(&format!("{dir}/rivers.bin"), radius_km)
        {
            Ok(r) => {
                println!("rivers: {} segments, {} lake cells", r.segments.len(), r.lakes.len());
                r
            }
            Err(e) => {
                eprintln!(
                    "no river data ({e}) - rivers/lakes disabled; run scripts/bake_rivers.py"
                );
                crate::rivers::RiverIndex::empty(radius_km)
            }
        };
        Ok(Self { radius_km, seed, faces, rivers })
    }

    /// Bilinear sample of a per-face f32 layer at (u, v) in [-1, 1].
    /// Rasters are edge-inclusive: texel 0 and res-1 lie exactly on the face
    /// edge, making adjacent faces agree along shared cube edges.
    fn bilinear(&self, face: usize, layer: impl Fn(&FaceRaster) -> &[f32], u: f64, v: f64) -> f32 {
        let r = &self.faces[face];
        let data = layer(r);
        let res = r.res as f64;
        let x = ((u * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let y = ((v * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(r.res - 1), (y0 + 1).min(r.res - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let at = |xx: usize, yy: usize| data[yy * r.res + xx];
        let a = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let b = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        a * (1.0 - fy) + b * fy
    }

    #[inline]
    fn climate_bilinear_xy(r: &FaceRaster, x: f64, y: f64) -> [f32; 2] {
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(r.res - 1), (y0 + 1).min(r.res - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let a = r.climate[y0 * r.res + x0];
        let b = r.climate[y0 * r.res + x1];
        let c = r.climate[y1 * r.res + x0];
        let d = r.climate[y1 * r.res + x1];
        let top = [a[0] * (1.0 - fx) + b[0] * fx, a[1] * (1.0 - fx) + b[1] * fx];
        let bottom = [c[0] * (1.0 - fx) + d[0] * fx, c[1] * (1.0 - fx) + d[1] * fx];
        [
            top[0] * (1.0 - fy) + bottom[0] * fy,
            top[1] * (1.0 - fy) + bottom[1] * fy,
        ]
    }

    #[inline]
    fn climate_bilinear(&self, face: usize, u: f64, v: f64) -> [f32; 2] {
        let r = &self.faces[face];
        let d = (r.res - 1) as f64;
        let x = ((u * 0.5 + 0.5) * d).clamp(0.0, d);
        let y = ((v * 0.5 + 0.5) * d).clamp(0.0, d);
        Self::climate_bilinear_xy(r, x, y)
    }

    #[inline]
    fn climate_bilinear_at(&self, p: RasterPosition) -> [f32; 2] {
        Self::climate_bilinear_xy(&self.faces[p.face], p.x, p.y)
    }

    /// Bilinear elevation (km).
    pub fn elevation(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.elev_km, u, v)
    }

    /// Map-scale roughness (km of elevation delta between ~30 km cells).
    pub fn rough(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.rough_km, u, v)
    }

    /// Annual precipitation (mm/yr).
    pub fn precip(&self, face: usize, u: f64, v: f64) -> f32 {
        self.climate_bilinear(face, u, v)[1]
    }

    /// Annual mean temperature (deg C).
    pub fn temp(&self, face: usize, u: f64, v: f64) -> f32 {
        self.climate_bilinear(face, u, v)[0]
    }

    /// Unwarped annual mean temperature and precipitation in one raster pass.
    pub fn temp_precip(&self, face: usize, u: f64, v: f64) -> (f32, f32) {
        let climate = self.climate_bilinear(face, u, v);
        (climate[0], climate[1])
    }

    /// log10(1 + river flow accumulation m3/s), dilated one map cell.
    pub fn flow(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.flow_log10, u, v)
    }

    /// Blurred open-ocean fraction at (u, v): ~1 on the sea, ~0 in interior
    /// land — including dry below-sea-level basins the map keeps as land.
    pub fn ocean(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.ocean, u, v)
    }

    /// Sharp (unblurred) ocean fraction at (u, v). >= 0.5 means this point
    /// is on the ocean side of the map's cell-resolution coastline.
    pub fn water_frac(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.water, u, v)
    }

    /// Physical sea ownership at an unwarped raster position. The positive-
    /// elevation fast path matters for dry coastal `255` areas: it proves land
    /// with one raster read and avoids consulting either ocean mask.
    #[inline]
    fn true_sea_at(&self, face: usize, u: f64, v: f64) -> bool {
        let e_raw = self.elevation(face, u, v) as f64;
        if e_raw > 0.0 {
            return false;
        }
        sea_from_fields(
            e_raw,
            self.water_frac(face, u, v) as f64,
            self.ocean(face, u, v) as f64,
        )
    }

    /// Unwarped nearest-texel class read. This is private because public biome
    /// consumers must all pass through `climate_position`; it remains the
    /// authority for categorical neighbor reads and regression probes.
    fn raw_koppen(&self, face: usize, u: f64, v: f64) -> u8 {
        let r = &self.faces[face];
        let res = r.res as f64;
        let x = (((u * 0.5 + 0.5) * (res - 1.0)).round().max(0.0) as usize).min(r.res - 1);
        let y = (((v * 0.5 + 0.5) * (res - 1.0)).round().max(0.0) as usize).min(r.res - 1);
        r.koppen[y * r.res + x]
    }

    /// Raster coordinates cached across the signature, class-weight, and
    /// warp-gate reads made by one surface lookup.
    #[inline]
    fn raster_position(&self, face: usize, u: f64, v: f64) -> RasterPosition {
        let r = &self.faces[face];
        let d = (r.res - 1) as f64;
        let x = ((u * 0.5 + 0.5) * d).clamp(0.0, d);
        let y = ((v * 0.5 + 0.5) * d).clamp(0.0, d);
        let xi = x.round() as i64;
        let yi = y.round() as i64;
        let index = yi as usize * r.res + xi as usize;
        RasterPosition {
            face,
            u,
            v,
            x,
            y,
            here: r.koppen[index],
            climate_edge: (r.climate_edge[index >> 6] >> (index & 63)) & 1 != 0,
        }
    }

    /// The ONE position transform for every biome-class and biome-tint read.
    ///
    /// Physical sea ownership is checked first at the unwarped world position;
    /// true sea never warps. A land warp touching the categorical `255`
    /// sentinel is rejected only when its target is also physical sea. Thus the
    /// water boundary cannot move, while dry `255` coastal land still receives
    /// the same appearance transform as every other biome.
    fn climate_position_with_sea(
        &self,
        face: usize,
        u: f64,
        v: f64,
        source_is_sea: bool,
    ) -> ClimatePosition {
        let (face, u, v) = canonical_face_uv(face, u, v);
        let source = self.raster_position(face, u, v);
        if source_is_sea {
            return ClimatePosition { raster: source, warped: false };
        }

        // With a hard <1-texel displacement bound, an equal 8-neighbor ring
        // proves that the categorical result cannot change. This is the main
        // per-vertex fast path: broad biome interiors retain the identity
        // transform, consistently for class and tint, without evaluating
        // procedural noise that could have no visible categorical effect.
        if !source.climate_edge {
            return ClimatePosition { raster: source, warped: false };
        }

        let res = self.faces[face].res;
        let warped_dir = biome_warp_dir(
            face_dir(face, u, v),
            res,
            self.seed,
            biome_warp_metric_scale(u, v),
        );
        let face_margin = 4.0 / (res - 1) as f64;
        let (warped_face, warped_u, warped_v) = if u.abs() < 1.0 - face_margin
            && v.abs() < 1.0 - face_margin
        {
            let (u, v) = uv_from_dir_on_face(face, warped_dir);
            (face, u, v)
        } else {
            face_from_dir(warped_dir)
        };
        let warped = self.raster_position(warped_face, warped_u, warped_v);
        // `255` is a climate sentinel, not a water decision. It occurs over
        // positive-elevation coastal land throughout the bake (the -44.8 deg
        // repro is the largest example). Only reject a sentinel-crossing warp
        // when its TARGET is physically sea; otherwise dry sand/forest edges
        // participate in the same domain warp as every other biome boundary.
        let touches_sentinel = source.here == 255 || warped.here == 255;
        if touches_sentinel && self.true_sea_at(warped_face, warped_u, warped_v) {
            ClimatePosition { raster: source, warped: false }
        } else {
            ClimatePosition { raster: warped, warped: true }
        }
    }

    #[inline]
    fn climate_position(&self, face: usize, u: f64, v: f64) -> ClimatePosition {
        let (face, u, v) = canonical_face_uv(face, u, v);
        let source_is_sea = self.true_sea_at(face, u, v);
        self.climate_position_with_sea(face, u, v, source_is_sea)
    }

    /// Nearest-texel domain-warped Koppen class id. `255` is the baked climate
    /// sentinel and can occur on dry land; use `BiomeClimate::sea` for water.
    pub fn koppen(&self, face: usize, u: f64, v: f64) -> u8 {
        self.climate_position(face, u, v).raster.here
    }

    /// Domain-warped class when the caller already owns the authoritative
    /// unwarped surface sample. Mesh and voxel hot paths use this to avoid
    /// re-reading the three sea-classification rasters.
    pub fn koppen_with_sea(&self, face: usize, u: f64, v: f64, sea: bool) -> u8 {
        self.climate_position_with_sea(face, u, v, sea).raster.here
    }

    /// Class plus continuous tint coordinates at the exact same warped point.
    /// This is intentionally separate from `temp()` / `precip()`: those public
    /// fields remain unwarped for terrain, weather, ecology, and water rules.
    pub fn biome_climate(&self, face: usize, u: f64, v: f64) -> BiomeClimate {
        let (face, u, v) = canonical_face_uv(face, u, v);
        let sea = self.true_sea_at(face, u, v);
        let p = self.climate_position_with_sea(face, u, v, sea);
        let r = p.raster;
        let climate = self.climate_bilinear_at(r);
        BiomeClimate {
            koppen: r.here,
            temp_c: climate[0],
            precip_mm_yr: climate[1],
            sea,
        }
    }

    /// Categorical texel access with a real neighbor across cube-face edges.
    /// Interior transition checks are four direct byte reads; only the outer
    /// raster ring pays for a direction remap.
    fn koppen_texel(&self, face: usize, x: i64, y: i64) -> u8 {
        let r = &self.faces[face];
        if x >= 0 && x < r.res as i64 && y >= 0 && y < r.res as i64 {
            return r.koppen[y as usize * r.res + x as usize];
        }
        let d = (r.res - 1) as f64;
        let u = -1.0 + 2.0 * x as f64 / d;
        let v = -1.0 + 2.0 * y as f64 / d;
        let (f2, u2, v2) = face_from_dir(face_dir(face, u, v));
        self.raw_koppen(f2, u2, v2)
    }

    /// Smooth the old palette anchor and far-forest weight over the categorical
    /// raster lattice. This signal is only a small nudge; unlike nearest-texel
    /// Koppen it is continuous at cell edges.
    fn koppen_signature(&self, p: RasterPosition) -> ([f32; 3], f32) {
        let r = &self.faces[p.face];
        let (x, y) = (p.x, p.y);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(r.res - 1), (y0 + 1).min(r.res - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let sample = |xx: usize, yy: usize| {
            let k = r.koppen[yy * r.res + xx];
            (ground_tint(k), koppen_forest(k))
        };
        let (a, fa) = sample(x0, y0);
        let (b, fb) = sample(x1, y0);
        let (c, fc) = sample(x0, y1);
        let (d, fd) = sample(x1, y1);
        let top = mix3(a, b, fx);
        let bottom = mix3(c, d, fx);
        (
            mix3(top, bottom, fy),
            (fa + (fb - fa) * fx) * (1.0 - fy) + (fc + (fd - fc) * fx) * fy,
        )
    }

    /// Continuous main-block weights for the short cross-material band.
    ///
    /// The categorical texels live at integer raster coordinates and their
    /// nearest-sample boundaries at half-integers. Each axis gets a smooth
    /// 0..1 ramp only within 150 m of its nearest boundary; the tensor product
    /// of those two ramps weights all four surrounding texels. Unlike choosing
    /// one of the nearest texel's four edges, the resulting probabilities have
    /// the same limit from every quadrant of a texel corner.
    ///
    /// Returns the nearest class (for the no-blend fast path), accumulated
    /// weights in Grass/Sand/Snow order, and one representative class per
    /// block. The representative is immaterial to rendering: callers use its
    /// main block, while the long-range Koppen tint signal remains separately
    /// bilinear in `koppen_signature`.
    #[cfg(test)]
    fn koppen_block_weights(&self, face: usize, u: f64, v: f64) -> (u8, [f64; 3], [u8; 3]) {
        self.koppen_block_weights_at(self.raster_position(face, u, v))
    }

    fn koppen_block_weights_at(&self, p: RasterPosition) -> (u8, [f64; 3], [u8; 3]) {
        let r = &self.faces[p.face];
        let d = (r.res - 1) as f64;
        let (x, y) = (p.x, p.y);
        let here = p.here;

        // Gnomonic metric: angular derivative of normalize([u,v,1]). This
        // keeps the artist-facing width in kilometres across face centers,
        // edges, and corners instead of treating a raster texel as constant.
        let denom = 1.0 + p.u * p.u + p.v * p.v;
        let texel_uv = 2.0 / d;
        let km_x = self.radius_km * texel_uv * (1.0 + p.v * p.v).sqrt() / denom;
        let km_y = self.radius_km * texel_uv * (1.0 + p.u * p.u).sqrt() / denom;
        let half = CROSS_BLOCK_ECOTONE_KM * 0.5;
        let axis_weight = |coord: f64, edge: f64, km_per_texel: f64| {
            let t = (0.5 + (coord - edge) * km_per_texel / (2.0 * half)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let (x0, y0) = (x.floor() as i64, y.floor() as i64);
        let wx = axis_weight(x, x0 as f64 + 0.5, km_x);
        let wy = axis_weight(y, y0 as f64 + 0.5, km_y);
        let samples = [
            (self.koppen_texel(p.face, x0, y0), (1.0 - wx) * (1.0 - wy)),
            (self.koppen_texel(p.face, x0 + 1, y0), wx * (1.0 - wy)),
            (self.koppen_texel(p.face, x0, y0 + 1), (1.0 - wx) * wy),
            (self.koppen_texel(p.face, x0 + 1, y0 + 1), wx * wy),
        ];

        let block_index = |block| match block {
            MainBlock::Grass => 0,
            MainBlock::Sand => 1,
            MainBlock::Snow => 2,
        };
        let mut weights = [0.0; 3];
        let mut representatives = [here; 3];
        let mut seen = [false; 3];
        for (class, weight) in samples {
            let slot = block_index(koppen_main_block(class));
            weights[slot] += weight;
            if weight > 0.0 && !seen[slot] {
                representatives[slot] = class;
                seen[slot] = true;
            }
        }
        (here, weights, representatives)
    }

    /// Dither against the continuous four-texel weights in a stable block
    /// order. Same-block neighborhoods return the nearest class exactly and
    /// therefore retain all climate/material statistics outside ecotones.
    #[cfg(test)]
    fn dithered_koppen(
        &self,
        face: usize,
        u: f64,
        v: f64,
        comparator: impl FnOnce() -> f64,
    ) -> u8 {
        self.dithered_koppen_at(self.raster_position(face, u, v), comparator)
    }

    fn dithered_koppen_at(
        &self,
        p: RasterPosition,
        comparator: impl FnOnce() -> f64,
    ) -> u8 {
        let (here, weights, representatives) = self.koppen_block_weights_at(p);
        if weights.iter().filter(|&&weight| weight > 0.0).count() <= 1 {
            return here;
        }

        let comparator = comparator();
        let mut cumulative = 0.0;
        let mut last = here;
        for (weight, class) in weights.into_iter().zip(representatives) {
            if weight == 0.0 {
                continue;
            }
            cumulative += weight;
            last = class;
            if comparator < cumulative {
                return class;
            }
        }
        // The CDF can round to exactly one in its extreme tail. This also
        // covers a caller-supplied one or a final-ULP accumulation shortfall.
        last
    }
}

/// Conservative same-class interior mask for the domain-warp fast path.
/// Outer-ring texels stay marked because their true neighbors live on another
/// cube face; they are a negligible fraction and must never be guessed from a
/// clamped edge.
fn climate_edge_mask(koppen: &[u8], res: usize) -> Vec<u64> {
    let mut edge = vec![0u64; koppen.len().div_ceil(64)];
    for y in 0..res {
        for x in 0..res {
            let mut differs = x == 0 || y == 0 || x + 1 == res || y + 1 == res;
            if !differs {
                let here = koppen[y * res + x];
                for dy in -1..=1isize {
                    for dx in -1..=1isize {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let xx = (x as isize + dx) as usize;
                        let yy = (y as isize + dy) as usize;
                        differs |= koppen[yy * res + xx] != here;
                    }
                }
            }
            if differs {
                let index = y * res + x;
                edge[index >> 6] |= 1u64 << (index & 63);
            }
        }
    }
    edge
}

/// Separable box blur of the (koppen == 255) ocean mask, edge-clamped.
fn blur_mask(koppen: &[u8], res: usize, radius: i32) -> Vec<f32> {
    let mask: Vec<f32> = koppen.iter().map(|&k| (k == 255) as u8 as f32).collect();
    let span = (2 * radius + 1) as f32;
    let mut tmp = vec![0f32; mask.len()];
    for y in 0..res {
        for x in 0..res {
            let mut s = 0.0;
            for d in -radius..=radius {
                let xx = (x as i32 + d).clamp(0, res as i32 - 1) as usize;
                s += mask[y * res + xx];
            }
            tmp[y * res + x] = s / span;
        }
    }
    let mut out = vec![0f32; mask.len()];
    for y in 0..res {
        for x in 0..res {
            let mut s = 0.0;
            for d in -radius..=radius {
                let yy = (y as i32 + d).clamp(0, res as i32 - 1) as usize;
                s += tmp[yy * res + x];
            }
            out[y * res + x] = s / span;
        }
    }
    out
}

/// Make the derived ocean mask agree across shared cube edges.
///
/// `blur_mask` runs per face with EDGE-CLAMPED taps: near a shared edge a
/// face averages only its own interior, so face A and face B derive different
/// ocean fractions for the identical world direction. Downstream
/// `terrain::sample` classifies sea from that fraction, so the coastline can
/// flip across a seam (the reported navy/sand half-screen split). Two-step,
/// load-time, raster-level fix (no bake-format change):
///
///  1. INTERIOR texels (further than `radius` from every edge) keep the fast
///     separable blur already in `faces[*].ocean` — bit-identical to before,
///     so coastlines away from seams do not move.
///  2. BORDER texels (within `radius` of an edge) are re-blurred with a direct
///     (2r+1)² gather whose off-face taps resolve by DIRECTION onto the
///     neighbor face's raw mask (the ghost-ring trick `voxel::canonical_column`
///     uses at column level), so they average TRUE neighbor data instead of
///     clamped own-face data — this kills the 0.32-vs-0.56 magnitude error.
///  3. SHARED texels (on an edge/corner) are then overwritten with their owner
///     face's value. `face_from_dir` is the codebase's canonical tie-breaker,
///     so every face's copy of a shared texel converges on one owner value —
///     bit-identical, corners (three faces) included. Bilinear exactly on an
///     edge uses only edge texels, so this makes the sea classification
///     seam-free by construction.
///
/// All six border re-blurs run BEFORE the canonical copy so the copy reads
/// finished (ghost-ring) values.
fn seam_exact_ocean(faces: &mut [FaceRaster], radius: i32) {
    let n = faces.len();
    let res = faces[0].res;
    let resf = res as f64;
    let r = radius as usize;
    let span2 = ((2 * radius + 1) * (2 * radius + 1)) as f32;

    // raw 0/1 ocean masks (koppen == 255) for every face, for the gather.
    let masks: Vec<Vec<f32>> = faces
        .iter()
        .map(|f| f.koppen.iter().map(|&k| (k == 255) as u8 as f32).collect())
        .collect();
    // start from the existing per-face separable blur (interior already final).
    let mut blurred: Vec<Vec<f32>> = faces.iter().map(|f| f.ocean.clone()).collect();

    // Edge-inclusive raster index -> texel-center face coordinate: texel k
    // centers at u = -1 + 2k/(res-1), so texel 0 and res-1 sit ON the edge.
    let uv_of = |x: i64, y: i64| (-1.0 + 2.0 * x as f64 / (resf - 1.0), -1.0 + 2.0 * y as f64 / (resf - 1.0));
    // nearest texel index for a face coordinate (matches koppen()/bilinear).
    let texel = |u: f64, v: f64| -> usize {
        let x = (((u * 0.5 + 0.5) * (resf - 1.0)).round().max(0.0) as usize).min(res - 1);
        let y = (((v * 0.5 + 0.5) * (resf - 1.0)).round().max(0.0) as usize).min(res - 1);
        y * res + x
    };
    // raw mask at a (possibly off-face) integer texel: off-face indices are
    // resolved by direction onto the neighbor face's nearest texel.
    let raw_at = |face: usize, x: i64, y: i64| -> f32 {
        if x >= 0 && x < res as i64 && y >= 0 && y < res as i64 {
            masks[face][y as usize * res + x as usize]
        } else {
            let (u, v) = uv_of(x, y);
            let (f2, u2, v2) = face_from_dir(face_dir(face, u, v));
            masks[f2][texel(u2, v2)]
        }
    };

    // 2. re-blur the border band over direction-resolved neighbor data.
    for face in 0..n {
        for y in 0..res {
            for x in 0..res {
                let border = x < r || x + r >= res || y < r || y + r >= res;
                if !border {
                    continue;
                }
                let mut s = 0.0f32;
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        s += raw_at(face, x as i64 + dx as i64, y as i64 + dy as i64);
                    }
                }
                blurred[face][y * res + x] = s / span2;
            }
        }
    }

    // 3. canonicalize shared texels to their owner face's (finished) value.
    let mut out = blurred.clone();
    for face in 0..n {
        for y in 0..res {
            for x in 0..res {
                if !(x == 0 || x == res - 1 || y == 0 || y == res - 1) {
                    continue;
                }
                let (u, v) = uv_of(x as i64, y as i64);
                let (owner, u2, v2) = face_from_dir(face_dir(face, u, v));
                if owner == face {
                    continue;
                }
                out[face][y * res + x] = blurred[owner][texel(u2, v2)];
            }
        }
    }

    for (f, o) in faces.iter_mut().zip(out) {
        f.ocean = o;
    }
}

/// Legacy Koppen palette anchor (linear RGB). The minimap still uses it
/// directly; world surfaces only take a small, bilinearly-smoothed hue nudge
/// from it through `climate_surface`.
pub fn ground_tint(id: u8) -> [f32; 3] {
    match id {
        0 | 1 => [0.050, 0.230, 0.040],  // Af/Am tropical rainforest
        2 => [0.200, 0.240, 0.055],      // Aw savanna gold-green
        3 => [0.480, 0.360, 0.190],      // BWh hot desert sand
        4 => [0.380, 0.310, 0.210],      // BWk cold desert
        5 => [0.300, 0.260, 0.095],      // BSh hot steppe
        6 => [0.260, 0.240, 0.110],      // BSk cold steppe
        7 | 8 | 9 => [0.180, 0.230, 0.070],   // Cs* mediterranean olive
        10 | 11 | 12 => [0.090, 0.240, 0.055], // Cw* subtropical highland
        13 | 14 | 15 => [0.085, 0.250, 0.048], // Cf* temperate green
        16 | 17 | 18 | 19 => [0.150, 0.210, 0.085], // Ds* dry continental
        20 | 21 | 22 | 23 => [0.095, 0.210, 0.060], // Dw* monsoon continental
        24 | 25 => [0.085, 0.220, 0.052],      // Dfa/Dfb humid continental
        26 | 27 => [0.055, 0.150, 0.062],      // Dfc/Dfd taiga blue-green
        28 => [0.220, 0.215, 0.150],           // ET tundra grey-green
        29 => [0.780, 0.820, 0.880],           // EF ice cap
        // 255 (ocean) on LAND happens routinely: elevation interpolation
        // overshoots above zero across texels whose nearest map cell is
        // ocean — coastal strands. Blocks fall back to sand there; the old
        // navy "ocean floor" tint here painted the same strands as blue
        // plates on every desert coast.
        _ => [0.52, 0.45, 0.27],
    }
}

fn koppen_forest(id: u8) -> f32 {
    match id {
        0 | 1 => 0.85,
        10..=15 | 20 | 21 | 24 | 25 => 0.5,
        22 | 23 | 26 | 27 => 0.6,
        16..=19 => 0.4,
        7..=9 => 0.3,
        2 => 0.15,
        _ => 0.0,
    }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smoothstepf(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Four temperature anchors for provisional climate colors. Smooth segments
/// avoid putting a new contour at an anchor temperature.
fn temperature_ramp(temp_c: f32, colors: [[f32; 3]; 4]) -> [f32; 3] {
    const T: [f32; 4] = [-4.0, 4.0, 16.0, 26.0];
    if temp_c <= T[1] {
        return mix3(colors[0], colors[1], smoothstepf(T[0], T[1], temp_c));
    }
    if temp_c <= T[2] {
        return mix3(colors[1], colors[2], smoothstepf(T[1], T[2], temp_c));
    }
    mix3(colors[2], colors[3], smoothstepf(T[2], T[3], temp_c))
}

fn climate_grass(temp_c: f32, precip_mm: f32) -> [f32; 3] {
    // Annual rain must work harder in heat. This is deliberately a simple
    // smooth 2-D ramp, not a second climate classifier; seasonal color waits
    // for texture anchors as Andrew directed.
    let dry_limit = 100.0 + temp_c.max(0.0) * 35.0;
    let moisture = smoothstepf(dry_limit, dry_limit + 900.0, precip_mm.max(0.0));
    let dry = temperature_ramp(
        temp_c,
        [
            [0.220, 0.215, 0.150],
            [0.260, 0.240, 0.110],
            [0.180, 0.230, 0.070],
            [0.205, 0.240, 0.055],
        ],
    );
    let wet = temperature_ramp(
        temp_c,
        [
            [0.070, 0.160, 0.070],
            [0.075, 0.205, 0.060],
            [0.090, 0.250, 0.050],
            [0.050, 0.230, 0.040],
        ],
    );
    mix3(dry, wet, moisture)
}

fn climate_sand(temp_c: f32, precip_mm: f32) -> [f32; 3] {
    let heat = smoothstepf(-4.0, 26.0, temp_c);
    let mut sand = mix3([0.545, 0.470, 0.290], [0.585, 0.485, 0.255], heat);
    let damp = smoothstepf(250.0, 1400.0, precip_mm.max(0.0)) * 0.10;
    sand = mix3(sand, [0.485, 0.430, 0.270], damp);
    sand
}

fn climate_snow(temp_c: f32, precip_mm: f32) -> [f32; 3] {
    let warmth = smoothstepf(-30.0, -4.0, temp_c);
    let wet = smoothstepf(50.0, 900.0, precip_mm.max(0.0));
    let snow = mix3([0.795, 0.835, 0.900], [0.835, 0.865, 0.905], warmth);
    mix3(snow, [0.850, 0.875, 0.915], wet * 0.12)
}

fn hue_nudge(base: [f32; 3], anchor: [f32; 3]) -> [f32; 3] {
    // Match luminance before mixing: Koppen remembers hue/chroma without
    // regaining ownership of brightness (which belongs to climate + material).
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let scale = luma(base) / luma(anchor).max(1e-4);
    let same_value = [
        (anchor[0] * scale).clamp(0.0, 1.0),
        (anchor[1] * scale).clamp(0.0, 1.0),
        (anchor[2] * scale).clamp(0.0, 1.0),
    ];
    mix3(base, same_value, KOPPEN_HUE_NUDGE)
}

/// One truth for the two ground renderers: domain-warped climate supplies all
/// provisional material tints, smoothly sampled Koppen only nudges grass hue,
/// and the approved unwarped world-column field chooses cross-block and snow
/// categories. The caller supplies unwarped physical-sea ownership solely as
/// a guard; elevation, weather, hydrology, and water geometry never warp.
pub fn climate_surface(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
) -> ClimateSurface {
    // Canonicalize the real world position once. The climate raster reads use
    // its warped counterpart; metre-scale comparator phases stay anchored to
    // this original position so their approved character does not change.
    let (world_face, world_u, world_v) = canonical_face_uv(face, u, v);
    let lookup =
        planet.climate_position_with_sea(world_face, world_u, world_v, unwarped_sea);
    let p = lookup.raster;
    let (koppen_anchor, forest) = planet.koppen_signature(p);
    let chosen = planet.dithered_koppen_at(p, || {
        ecotone_comparator(
            world_face,
            world_u,
            world_v,
            planet.seed.wrapping_add(ECOTONE_FIELD_SEED_OFFSET),
        )
    });
    let (temp_c, precip_mm) = if lookup.warped {
        let climate = planet.climate_bilinear_at(p);
        (climate[0] as f64, climate[1] as f64)
    } else {
        (unwarped_temp_c, unwarped_precip_mm)
    };
    let mut main_block = koppen_main_block(chosen);
    let snow_low = SNOWLINE_CENTER_C - SNOWLINE_HALF_RANGE_C;
    let snow_high = SNOWLINE_CENTER_C + SNOWLINE_HALF_RANGE_C;
    if main_block == MainBlock::Snow || temp_c < snow_low {
        main_block = MainBlock::Snow;
    } else if temp_c < snow_high {
        let snow_comparator = ecotone_comparator(
            world_face,
            world_u,
            world_v,
            planet.seed.wrapping_add(SNOW_FIELD_SEED_OFFSET),
        );
        let snow_threshold = SNOWLINE_CENTER_C
            + (snow_comparator - 0.5) * (SNOWLINE_HALF_RANGE_C * 2.0);
        if temp_c < snow_threshold {
            main_block = MainBlock::Snow;
        }
    }

    let temp = temp_c as f32;
    let precip = precip_mm as f32;
    ClimateSurface {
        main_block,
        grass: hue_nudge(climate_grass(temp, precip), koppen_anchor),
        sand: climate_sand(temp, precip),
        snow: climate_snow(temp, precip),
        forest,
    }
}

/// Koppen class -> base color (matches planetgen/biomes.py palette, linearized-ish).
#[allow(dead_code)]
pub fn koppen_color(id: u8) -> [f32; 3] {
    const HEX: [u32; 30] = [
        0x0000fe, 0x0078ff, 0x46aafa, 0xff0000, 0xff9696, 0xf5a500, 0xffdc64, 0xffff00,
        0xc8c800, 0x969600, 0x96ff96, 0x64c864, 0x329632, 0xc8ff50, 0x64ff50, 0x32c800,
        0xff00fe, 0xc800c8, 0x963296, 0x966496, 0xabb1ff, 0x5a77db, 0x4b50b4, 0x320087,
        0x00ffff, 0x37c8ff, 0x007d7d, 0x00465f, 0xb2b2b2, 0x686868,
    ];
    if (id as usize) < HEX.len() {
        let h = HEX[id as usize];
        let s = |c: u32| (c as f32 / 255.0).powf(2.2); // rough sRGB -> linear
        [s((h >> 16) & 255), s((h >> 8) & 255), s(h & 255)]
    } else {
        [0.02, 0.09, 0.18] // ocean fallback (unused: water colored by depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain;

    fn climate_test_planet(right_class: u8) -> Planet {
        let res = 4usize;
        let n = res * res;
        let koppen: Vec<u8> = (0..res)
            .flat_map(|_| [6, 6, right_class, right_class])
            .collect();
        let face = || FaceRaster {
            res,
            elev_km: vec![0.2; n],
            koppen: koppen.clone(),
            climate_edge: climate_edge_mask(&koppen, res),
            rough_km: vec![0.0; n],
            climate: vec![[8.0, 500.0]; n],
            flow_log10: vec![0.0; n],
            ocean: vec![0.0; n],
            water: vec![0.0; n],
        };
        Planet {
            radius_km: 1.0,
            seed: 42,
            faces: (0..6).map(|_| face()).collect(),
            rivers: crate::rivers::RiverIndex::empty(1.0),
        }
    }

    #[test]
    fn koppen_main_blocks_match_surface_materials() {
        for k in 0..=29 {
            let expected = match k {
                3 | 4 => MainBlock::Sand,
                29 => MainBlock::Snow,
                _ => MainBlock::Grass,
            };
            assert_eq!(koppen_main_block(k), expected, "Koppen class {k}");
        }
        assert_eq!(koppen_main_block(255), MainBlock::Sand);
    }

    #[test]
    fn biome_warp_is_deterministic_and_exact_across_cube_faces() {
        let seed = 42;
        let dir = face_dir(2, 0.271_828_182_8, -0.618_033_988_7);
        let expected = biome_warp_dir(dir, 1024, seed, 1.0);
        let repeated = biome_warp_dir(dir, 1024, seed, 1.0);
        assert_eq!(
            [expected.x.to_bits(), expected.y.to_bits(), expected.z.to_bits()],
            [repeated.x.to_bits(), repeated.y.to_bits(), repeated.z.to_bits()]
        );
        assert!((expected - dir).length() > 1e-8, "configured warp must move the lookup");

        // Present identical edge/corner directions through every face that
        // owns them. Production canonicalization must lead to one bit-exact
        // warped direction, not merely visually close copies.
        for dir in [
            face_dir(0, 1.0, -1.0),
            face_dir(0, 1.0, -0.234_567_89),
            face_dir(0, 1.0, 0.618_033_99),
            face_dir(0, 1.0, 1.0),
        ] {
            let mut expected = None;
            let mut copies = 0;
            for face in 0..FACES.len() {
                let (u, v, on) = project(face, dir);
                if !on || (u.abs() < 1.0 - 1e-6 && v.abs() < 1.0 - 1e-6) {
                    continue;
                }
                let (f, u, v) = canonical_face_uv(face, u, v);
                let got = biome_warp_dir(
                    face_dir(f, u, v),
                    1024,
                    seed,
                    biome_warp_metric_scale(u, v),
                );
                let bits = [got.x.to_bits(), got.y.to_bits(), got.z.to_bits()];
                if let Some(expected) = expected {
                    assert_eq!(bits, expected, "face {face} differs at {dir:?}");
                } else {
                    expected = Some(bits);
                }
                copies += 1;
            }
            assert!(copies >= 2, "expected a shared edge/corner at {dir:?}");
        }
    }

    #[test]
    fn biome_warp_has_an_intermediate_flight_octave() {
        // Art-direction gate for Neisor's production bake. This is the octave
        // that must remain visible between the approved continental warp and
        // the 300 m local ecotone field.
        const NEISOR_RADIUS_KM: f64 = 8_660.254_037_844_386;
        const BAKE_RES: f64 = 1_024.0;
        let texel_km = NEISOR_RADIUS_KM * 2.0 / (BAKE_RES - 1.0);
        let octave = (BIOME_WARP_OCTAVES - 1) as i32;
        let wavelength_km = texel_km * BIOME_WARP_BASE_WAVELENGTH_TEXELS
            / BIOME_WARP_LACUNARITY.powi(octave);
        let amplitude_km = texel_km * BIOME_WARP_BASE_AMPLITUDE_TEXELS
            * BIOME_WARP_PERSISTENCE.powi(octave);
        assert!(
            (2.0..=4.0).contains(&wavelength_km),
            "finest warp wavelength is {wavelength_km:.3} km"
        );
        assert!(
            (0.2..=0.7).contains(&amplitude_km),
            "finest warp amplitude is {amplitude_km:.3} km"
        );
    }

    #[test]
    fn dry_ocean_sentinel_participates_in_biome_warp() {
        // Production has dry, positive-elevation coastal land whose nearest
        // climate byte is 255. It is sand appearance, not physical ocean, and
        // therefore must move with the forest/sand domain warp.
        let dry_sentinel = climate_test_planet(255);
        let mut changed = 0usize;
        for face in 0..FACES.len() {
            for j in 0..128 {
                for i in 0..128 {
                    let u = -1.0 + 2.0 * (i as f64 + 0.5) / 128.0;
                    let v = -1.0 + 2.0 * (j as f64 + 0.5) / 128.0;
                    assert!(!dry_sentinel.true_sea_at(face, u, v));
                    changed += usize::from(
                        dry_sentinel.raw_koppen(face, u, v)
                            != dry_sentinel.koppen(face, u, v),
                    );
                }
            }
        }
        assert!(changed > 0, "dry 255 boundary remained completely unwarped");
        assert!(!dry_sentinel.biome_climate(0, 0.25, 0.0).sea);
    }

    #[test]
    fn biome_warp_cannot_cross_true_sea() {
        let mut ocean_edge = climate_test_planet(255);
        for face in &mut ocean_edge.faces {
            for y in 0..face.res {
                for x in 2..face.res {
                    let index = y * face.res + x;
                    face.elev_km[index] = -0.2;
                    face.water[index] = 1.0;
                    face.ocean[index] = 1.0;
                }
            }
        }
        for face in 0..FACES.len() {
            for j in 0..128 {
                for i in 0..128 {
                    let u = -1.0 + 2.0 * (i as f64 + 0.5) / 128.0;
                    let v = -1.0 + 2.0 * (j as f64 + 0.5) / 128.0;
                    let true_sea = ocean_edge.true_sea_at(face, u, v);
                    let warped_is_ocean = ocean_edge.koppen(face, u, v) == 255;
                    assert_eq!(warped_is_ocean, true_sea, "face={face} u={u} v={v}");
                }
            }
        }
    }

    #[test]
    fn category_dither_only_crosses_main_blocks() {
        // In a 4x4 raster the class edge is x=1.5 -> u=0. Just inside the
        // left cell, cumulative Grass/Sand weights select the two blocks at
        // opposite ends of the hash interval.
        let cross = climate_test_planet(4);
        let u_near = -0.006;
        assert_eq!(cross.dithered_koppen(0, u_near, 0.0, || 0.0), 6);
        assert_eq!(cross.dithered_koppen(0, u_near, 0.0, || 0.999), 4);
        // More than half the 300 m band away, no hash can cross the edge.
        assert_eq!(cross.dithered_koppen(0, -0.30, 0.0, || 0.0), 6);

        // Classes 6 and 14 are both grass: even at the exact edge the
        // category remains nearest-texel and no random decision is made.
        let same = climate_test_planet(14);
        assert_eq!(same.dithered_koppen(0, u_near, 0.0, || 0.0), 6);
        assert_eq!(same.dithered_koppen(0, u_near, 0.0, || 0.999), 6);

        // The hue nudge itself is bilinear, so same-block color is continuous
        // across that nearest-class edge.
        let a = climate_surface(&same, 0, -1e-6, 0.0, 8.0, 900.0, false)
            .tint(MainBlock::Grass);
        let b = climate_surface(&same, 0, 1e-6, 0.0, 8.0, 900.0, false)
            .tint(MainBlock::Grass);
        assert!(a.into_iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5));
    }

    #[test]
    fn category_dither_fraction_is_continuous_at_texel_corner() {
        // Around the x=1.5, y=1.5 corner, sand occupies only the lower-left
        // quadrant. The old four-edge query reported ~1/2 sand on the two
        // adjacent grass sides and exactly zero in the diagonal grass texel.
        let mut corner = climate_test_planet(6);
        for face in &mut corner.faces {
            face.koppen[1 * face.res + 1] = 4;
        }

        let epsilon = 1e-12;
        let quantum = 1.0 / 4294967296.0; // hash01's 32-bit dither quantum
        let mut fractions = Vec::new();
        for (u, v) in [
            (-epsilon, -epsilon),
            (epsilon, -epsilon),
            (-epsilon, epsilon),
            (epsilon, epsilon),
        ] {
            let (_, weights, _) = corner.koppen_block_weights(0, u, v);
            fractions.push(weights[1]); // Sand in Grass/Sand/Snow order.
        }
        let lo = fractions.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = fractions.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            hi - lo <= quantum,
            "corner limits differ by {} (> one hash quantum): {fractions:?}",
            hi - lo
        );

        let (_, at_corner, _) = corner.koppen_block_weights(0, 0.0, 0.0);
        assert!((at_corner[1] - 0.25).abs() < 1e-12);
    }

    #[test]
    fn ecotone_comparator_preserves_area_fractions() {
        // Sample canonical columns over all faces and both categorical seed
        // streams. If the probability integral transform is doing its job,
        // threshold occupancy equals the requested blend weight.
        const SAMPLES_PER_SEED: usize = 65_536;
        const FRACTIONS: [f64; 3] = [0.25, 0.50, 0.75];
        const TOLERANCE: f64 = 0.008;
        for seed in [
            42i64.wrapping_add(ECOTONE_FIELD_SEED_OFFSET),
            42i64.wrapping_add(SNOW_FIELD_SEED_OFFSET),
        ] {
            let mut occupied = [0usize; 3];
            for sample in 0..SAMPLES_PER_SEED {
                let face = sample % FACES.len();
                let a = hash_u64(
                    face as u8,
                    sample as u64,
                    (sample as u64).wrapping_mul(0x9E37_79B9),
                    0xA11C_E001,
                );
                let b = hash_u64(
                    face as u8,
                    a,
                    (sample as u64).wrapping_mul(0x85EB_CA77),
                    0xA11C_E002,
                );
                let ci = a % COLUMNS_PER_FACE;
                let cj = b % COLUMNS_PER_FACE;
                let n = COLUMNS_PER_FACE as f64;
                let u = -1.0 + 2.0 * (ci as f64 + 0.5) / n;
                let v = -1.0 + 2.0 * (cj as f64 + 0.5) / n;
                let comparator = ecotone_comparator(face, u, v, seed);
                for (count, fraction) in occupied.iter_mut().zip(FRACTIONS) {
                    *count += usize::from(comparator < fraction);
                }
            }
            for (count, expected) in occupied.into_iter().zip(FRACTIONS) {
                let measured = count as f64 / SAMPLES_PER_SEED as f64;
                eprintln!(
                    "ecotone occupancy seed={seed} blend={expected:.2} measured={measured:.5}"
                );
                assert!(
                    (measured - expected).abs() <= TOLERANCE,
                    "seed {seed}: occupancy {measured:.5} at blend {expected:.2} exceeds {TOLERANCE:.3} tolerance"
                );
            }
        }
    }

    #[test]
    fn ecotone_comparator_is_one_value_per_canonical_column() {
        let n = COLUMNS_PER_FACE as f64;
        let (ci, cj) = (4_321_987u64, 7_654_321u64);
        let u = -1.0 + 2.0 * (ci as f64 + 0.5) / n;
        let v = -1.0 + 2.0 * (cj as f64 + 0.5) / n;
        let seed = 42i64.wrapping_add(ECOTONE_FIELD_SEED_OFFSET);
        let expected = ecotone_comparator(2, u, v, seed).to_bits();
        for (du, dv) in [(-0.20, -0.20), (0.20, -0.20), (-0.20, 0.20), (0.20, 0.20)] {
            let got = ecotone_comparator(2, u + 2.0 * du / n, v + 2.0 * dv / n, seed);
            assert_eq!(got.to_bits(), expected);
        }
        assert_eq!(ecotone_comparator(2, u, v, seed).to_bits(), expected);
    }

    /// Project a world direction onto a specific face's gnomonic plane.
    /// `on` is true when the direction lies within this face's [-1,1]^2
    /// domain (a hair of slack so an exact edge direction registers on both
    /// of the faces that share it — three, at a corner).
    fn project(face: usize, dir: DVec3) -> (f64, f64, bool) {
        let (axis, right, up) = FACES[face];
        let d = dir.dot(axis);
        if d <= 0.0 {
            return (0.0, 0.0, false);
        }
        let p = dir / d;
        let (u, v) = (p.dot(right), p.dot(up));
        let slack = 1.0 + 1e-6;
        (u, v, u.abs() <= slack && v.abs() <= slack)
    }

    /// Snap a coordinate that should sit on the shared lattice to its exact
    /// value: an ULP-band around ±1 goes to exactly ±1 (the edge column),
    /// everything else to its nearest edge-inclusive texel center. This makes
    /// both faces read the SAME shared lattice point, where `face_dir` is
    /// bit-identical (each world axis gets exactly one signed-unit term, so
    /// the pre-normalize vector is order-independent).
    fn snap(x: f64, res: usize) -> f64 {
        if x.abs() > 1.0 - 1e-6 {
            x.signum()
        } else {
            let k = ((x * 0.5 + 0.5) * (res as f64 - 1.0)).round();
            -1.0 + 2.0 * k / (res as f64 - 1.0)
        }
    }

    #[test]
    fn ecotone_comparator_is_exact_across_cube_faces() {
        let seed = 42i64.wrapping_add(ECOTONE_FIELD_SEED_OFFSET);
        // Interior points on an edge plus both adjacent cube corners.
        for dir in [
            face_dir(0, 1.0, -1.0),
            face_dir(0, 1.0, -0.234_567_89),
            face_dir(0, 1.0, 0.618_033_99),
            face_dir(0, 1.0, 1.0),
        ] {
            let mut expected = None;
            let mut copies = 0;
            for face in 0..FACES.len() {
                let (u, v, on) = project(face, dir);
                if !on || (u.abs() < 1.0 - 1e-6 && v.abs() < 1.0 - 1e-6) {
                    continue;
                }
                let got = ecotone_comparator(face, u, v, seed).to_bits();
                if let Some(expected) = expected {
                    assert_eq!(got, expected, "face {face} differs at {dir:?}");
                } else {
                    expected = Some(got);
                }
                copies += 1;
            }
            assert!(copies >= 2, "expected a shared edge/corner at {dir:?}");
        }
    }

    /// The property the review asked for: on every shared cube edge and corner
    /// the derived ocean mask — and the sea classification `terrain::sample`
    /// draws from it — must be identical no matter which face samples the
    /// point. Loads the real baked planet; skips (does not fail) without
    /// assets so CI-less clones stay green.
    #[test]
    fn ocean_mask_seam_exact() {
        let planet = match Planet::load("assets") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("skipping ocean_mask_seam_exact: no assets ({e})");
                return;
            }
        };
        let res = planet.faces[0].res;
        // (axis_kind, edge_value): 0 = u fixed, v marches; 1 = v fixed, u marches.
        let edges: [(u8, f64); 4] = [(0, -1.0), (0, 1.0), (1, -1.0), (1, 1.0)];
        let steps = 512usize;
        let mut checked = 0u64;
        for fa in 0..6usize {
            for &(kind, edge_val) in &edges {
                for s in 0..=steps {
                    let t = -1.0 + 2.0 * s as f64 / steps as f64;
                    let (ua, va) = if kind == 0 { (edge_val, t) } else { (t, edge_val) };
                    let (ua, va) = (snap(ua, res), snap(va, res));
                    let dir = face_dir(fa, ua, va);
                    let ocean_a = planet.ocean(fa, ua, va);
                    let samp_a = terrain::sample(&planet, fa, ua, va, 5);
                    for fb in 0..6usize {
                        if fb == fa {
                            continue;
                        }
                        let (ub, vb, on) = project(fb, dir);
                        if !on {
                            continue;
                        }
                        let (ubs, vbs) = (snap(ub, res), snap(vb, res));
                        // must genuinely be an edge/corner of fb, not interior
                        if ubs.abs() < 0.9999 && vbs.abs() < 0.9999 {
                            continue;
                        }
                        // sanity: the two representations must resolve the same
                        // world point. (They agree to the bit on identity edges;
                        // on reversal edges the tangential lattice index is
                        // reconstructed as -1+2(res-1-k)/(res-1), a real ULP off
                        // -(-1+2k/(res-1)), so dir can differ by 1 ULP — which
                        // the nearest-texel rounding downstream absorbs.)
                        let dir_b = face_dir(fb, ubs, vbs);
                        assert!(
                            dir.distance(dir_b) < 1e-9,
                            "seam faces disagree on the point: f{fa} {dir:?} vs f{fb} {dir_b:?}"
                        );
                        // (1) derived ocean fraction bit-identical
                        let ocean_b = planet.ocean(fb, ubs, vbs);
                        assert_eq!(
                            ocean_a.to_bits(),
                            ocean_b.to_bits(),
                            "ocean seam: f{fa}({ua},{va})={ocean_a} vs f{fb}({ubs},{vbs})={ocean_b}"
                        );
                        // (2) sea classification and water surface agree
                        let samp_b = terrain::sample(&planet, fb, ubs, vbs, 5);
                        assert_eq!(samp_a.sea, samp_b.sea, "sea seam f{fa} vs f{fb}");
                        let (wa, wb) = (samp_a.water_km, samp_b.water_km);
                        assert!(
                            wa.to_bits() == wb.to_bits() || (!wa.is_finite() && !wb.is_finite()),
                            "water_km seam f{fa}={wa} vs f{fb}={wb}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "no seam pairs sampled");
        eprintln!("ocean_mask_seam_exact: {checked} seam pairs verified");
    }
}

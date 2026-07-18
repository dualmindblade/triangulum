//! Planet data: baked cube-face rasters + cube-sphere math.
//!
//! Face convention must match scripts/bake_faces.py:
//!   direction(u, v) = normalize(axis + u*right + v*up),  u, v in [-1, 1]

use anyhow::{Context, Result};
use glam::DVec3;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::noise::{gradient_noise, normal_value_noise};

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

// Forest-impostor candidate enumeration is much finer than the baked climate
// raster and used to repeat the expensive annual vegetation profile for every
// level-11..14 tile. Partition only the sparse evaluated profiles into
// level-14 regions (~800 m on Neisor). A request always walks its own exact
// lattice and applies its own stride-scaled lottery gate first; it never
// front-loads a whole region or evaluates the more-permissive gates of other
// LODs. Profiles that survive that cheap gate are shared across LODs/rebuilds.
const IMPOSTOR_CANDIDATE_REGION_LEVEL: u32 = 14;
const IMPOSTOR_CANDIDATE_REGIONS_PER_FACE: u64 = 1 << IMPOSTOR_CANDIDATE_REGION_LEVEL;
// First test a cheap level-16 grid. If none of those cells can be rejected,
// the warm dense region falls straight through to enumeration. Only cells
// beside a proven boundary recurse four more levels, to level 20 (~12 m).
const IMPOSTOR_CANDIDATE_PROOF_INITIAL_DIVISIONS: u64 = 4;
const IMPOSTOR_CANDIDATE_PROOF_REFINEMENT_DEPTH: u32 = 4;
// A whole render tile may combine disjoint early and late rejections. Refine
// that union before entering the cache; short-circuiting keeps productive
// warm tiles cheap while cold climate boundaries can prove an empty output.
const IMPOSTOR_CANDIDATE_TILE_PROOF_DEPTH: u32 = 8;
const IMPOSTOR_CANDIDATE_CACHE_SHARDS: usize = 16;
const IMPOSTOR_CANDIDATE_CACHE_BYTES: usize = 64 * 1024 * 1024;
const IMPOSTOR_CANDIDATE_CACHE_BYTES_PER_SHARD: usize =
    IMPOSTOR_CANDIDATE_CACHE_BYTES / IMPOSTOR_CANDIDATE_CACHE_SHARDS;
// Empty and proof-only regions can stay far below the payload cap, so they
// need a separate bound to prevent unbounded map growth.
const IMPOSTOR_CANDIDATE_CACHE_ENTRIES_PER_SHARD: usize = 512;
const IMPOSTOR_CANDIDATE_STRIDES: [u64; 4] = [1, 2, 4, 8];
const _: () = assert!(IMPOSTOR_CANDIDATE_REGIONS_PER_FACE <= u16::MAX as u64);

/// Total width of a cross-material ecotone (half on either side of the baked
/// Koppen edge) at voxel/local scale. Same-material classes never consult this
/// band. Retained as the metric minimum and narrow-band test oracle; production
/// cross-block surfaces use the all-range boundary zone below.
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
//   wavelength 85 / 24 / 6.5 / 1.8 / 0.50 km,
//   amplitude 11 / 4.0 / 1.4 / 0.51 / 0.18 km before the 1.20-texel cap.
// Every octave samples seamless hashed 3-D value noise in a rotated domain;
// octave-varying carrier directions keep the displacement non-axis-aligned.
// The >1 texel reach is deliberate: sub-texel displacement can only decorate
// each baked cell, while this reach can break recognizable cell topology.
// The class-interior fast path skips the whole stack when it cannot matter.
pub const BIOME_WARP_OCTAVES: u32 = 5;
pub const BIOME_WARP_BASE_AMPLITUDE_TEXELS: f64 = 0.65;
pub const BIOME_WARP_BASE_WAVELENGTH_TEXELS: f64 = 5.0;
pub const BIOME_WARP_LACUNARITY: f64 = 3.6;
pub const BIOME_WARP_PERSISTENCE: f64 = 0.36;
pub const BIOME_WARP_MAX_DISPLACEMENT_TEXELS: f64 = 1.20;

const BIOME_WARP_SEED_OFFSET: i64 = 0x0B10_0A2E;

// ---- Andrew's all-range boundary-zone knobs -----------------------------
// Cross-material boundaries use one 8 km probability zone in voxels and every
// mesh LOD. The old range path added two independent coarse layers to a local
// four-layer/300 m treatment; the shader then cross-faded between disagreeing
// silhouettes at 4..12 km. The categorical stack shares the domain warp's
// rotated domains, frequencies, and seed schedule; its gradient lattice also
// has an exact WGSL twin, so orbit, flight, and ground follow one field.
pub const BIOME_BOUNDARY_ZONE_KM: f64 = 8.0;
pub const BIOME_BOUNDARY_ZONE_MAX_TEXEL_FRACTION: f64 = 0.90;
/// Symmetric categorical prefilter applied to the one all-range path.
/// The center weight is `1 - 2*side`; normalization and symmetry preserve the
/// expected area of each main block while extending islands into both sides.
pub const BIOME_BOUNDARY_PREFILTER_SIDE_WEIGHT: f64 = 0.15;
const BIOME_BOUNDARY_PREFILTER_SUPPORT_TEXELS: isize = 2;

// Five shared octaves are exactly the warp's 85..0.5 km scalar carriers; five
// continuations reach ~0.8 m. Boundary amplitudes preserve round 3's visual
// hierarchy: two broad layers, then a strong range-visible reset and a 0.5
// cascade. One normalized stack at every altitude makes continuity a
// construction, not a blend between separately normalized fields.
pub const BIOME_BOUNDARY_FIELD_OCTAVES: u32 = 10;
pub const BIOME_BOUNDARY_FIELD_SECOND_COARSE_AMPLITUDE: f64 = 0.55;
pub const BIOME_BOUNDARY_FIELD_FINE_PERSISTENCE: f64 = 0.5;
/// Prefix rendered as explicit far-mesh patches. On Neisor these are the
/// approximately 85, 24, 6.5, 1.8, and 0.5 km layers. Fragment evaluation
/// (rather than vertex sampling) keeps the last layers lattice-safe. Finer
/// layers stay in the exact local decision and contribute their variance as
/// the range mean below.
pub const BIOME_RANGE_RESOLVED_OCTAVES: u32 = 5;

const fn boundary_field_amplitude(octave: u32) -> f64 {
    if octave == 0 {
        return 1.0;
    }
    if octave == 1 {
        return BIOME_BOUNDARY_FIELD_SECOND_COARSE_AMPLITUDE;
    }
    let mut amplitude = 1.0;
    let mut at = 2;
    while at < octave {
        amplitude *= BIOME_BOUNDARY_FIELD_FINE_PERSISTENCE;
        at += 1;
    }
    amplitude
}

const fn boundary_field_variance(first_octave: u32) -> f64 {
    let mut variance = 0.0;
    let mut octave = first_octave;
    while octave < BIOME_BOUNDARY_FIELD_OCTAVES {
        let amplitude = boundary_field_amplitude(octave);
        variance += amplitude * amplitude;
        octave += 1;
    }
    variance
}

/// Fraction of categorical variance below the explicit range-patch lattice.
/// Each packed range endpoint retains exactly this much of the smooth expected
/// color. It is derived from the same octave amplitudes, not an independent
/// fixed-contrast art knob (approximately 0.1264 with the defaults).
pub const BIOME_RANGE_UNRESOLVED_MEAN: f32 =
    (boundary_field_variance(BIOME_RANGE_RESOLVED_OCTAVES)
        / boundary_field_variance(0)) as f32;
/// `gradient_noise` is narrower than N(0,1). The shared CPU/WGSL comparator
/// applies this calibration before the normal CDF so requested coverage stays
/// area-neutral. Locked by the categorical occupancy test below.
pub const BIOME_BOUNDARY_GRADIENT_NORMALIZE: f64 = 2.70;
pub const BIOME_RANGE_FAMILIES: usize = 8;

fn climate_range_family(class: u8, climate: [f32; 2]) -> usize {
    match koppen_main_block(class) {
        MainBlock::Sand => 6,
        MainBlock::Snow => 7,
        MainBlock::Grass => {
            let color = climate_grass(climate[0], climate[1]);
            let greenness = color[1] - 0.5 * (color[0] + color[2]);
            if greenness < 0.060 {
                0
            } else if greenness < 0.085 {
                1
            } else if greenness < 0.110 {
                2
            } else if greenness < 0.135 {
                3
            } else if greenness < 0.160 {
                4
            } else {
                5
            }
        }
    }
}

const BIOME_RANGE_FAMILY_REPRESENTATIVE: [u8; BIOME_RANGE_FAMILIES] =
    [28, 6, 18, 25, 13, 0, 3, 29];
const BIOME_RANGE_FAMILY_TEMP_C: [f32; BIOME_RANGE_FAMILIES] =
    [-8.0, 8.0, 8.0, 8.0, 8.0, 24.0, 18.0, -20.0];
const BIOME_RANGE_FAMILY_PRECIP_MM: [f32; BIOME_RANGE_FAMILIES] =
    [100.0, 450.0, 700.0, 900.0, 1300.0, 1800.0, 100.0, 300.0];
/// Comparator polarity is otherwise arbitrary (block order assigns the two
/// sides). Negative retains the approved grass ownership at the pond dossier
/// while using the exact same warp lobes and unchanged area statistics.
pub const BIOME_BOUNDARY_FIELD_POLARITY: f64 = -1.0;

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

#[cfg(test)]
const ECOTONE_FIELD_SEED_OFFSET: i64 = 0x0EC0_70AE;
const SNOW_FIELD_SEED_OFFSET: i64 = 0x0000_5A0E;
const SNOWLINE_CENTER_C: f64 = -9.0;
const SNOWLINE_HALF_RANGE_C: f64 = 1.5;
/// Below this annual temperature `apply_snow_override` unconditionally owns
/// the vegetation material, before its transition-band comparator is needed.
pub(crate) const VEGETATION_UNCONDITIONAL_SNOW_C: f64 =
    SNOWLINE_CENTER_C - SNOWLINE_HALF_RANGE_C;

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
    /// Grass/Sand/Snow ownership weights. Exact and fixed range-family
    /// endpoints are one-hot; range coverage itself lives beside the palette.
    block_weights: [f32; 3],
    /// Existing far-canopy approximation, spatially blended so it cannot put
    /// a second hard line back across a smooth same-block transition.
    pub forest: f32,
}

/// Fixed semantic Koppen-family channels for the far mesh. Every vertex uses
/// the same channel identity, preventing candidate-slot changes from exposing
/// the triangle lattice while retaining class-level climate anchors.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClimateRangeSurface {
    pub candidates: [ClimateSurface; BIOME_RANGE_FAMILIES],
    pub weights: [f32; BIOME_RANGE_FAMILIES],
    /// Original continuous climate mean, retained as the common-mode color.
    /// Categorical endpoints are corrected around it, so one-family interiors
    /// keep their approved tint without letting this mean own the interface.
    pub mean: ClimateSurface,
    /// Cumulative candidate coverages; the fragment shader compares its
    /// world-anchored resolved field against these thresholds.
    pub thresholds: [f32; BIOME_RANGE_FAMILIES],
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

/// The subset of the shared biome surface needed by tree candidate
/// enumeration. Keeping this separate from `ClimateSurface` avoids building
/// three display tints for columns that only need vegetation ownership.
#[derive(Clone, Copy, Debug)]
pub struct VegetationSurface {
    pub main_block: MainBlock,
    pub forest: f32,
    pub koppen: u8,
    pub temp_c: f32,
    pub precip_mm_yr: f32,
}

impl ClimateSurface {
    pub fn tint(self, block: MainBlock) -> [f32; 3] {
        match block {
            MainBlock::Grass => self.grass,
            MainBlock::Sand => self.sand,
            MainBlock::Snow => self.snow,
        }
    }

    pub(crate) fn display_weights(self, categorical_contrast: f32) -> [f32; 3] {
        let mut weights = self.block_weights;
        let slot = match self.main_block {
            MainBlock::Grass => 0,
            MainBlock::Sand => 1,
            MainBlock::Snow => 2,
        };
        for (i, weight) in weights.iter_mut().enumerate() {
            *weight *= 1.0 - categorical_contrast;
            if i == slot {
                *weight += categorical_contrast;
            }
        }
        weights
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

#[inline]
fn biome_scalar_noise(dir: DVec3, frequency: f64, seed: i64, octave: usize) -> f64 {
    let axes = BIOME_WARP_AXES[octave % BIOME_WARP_AXES.len()];
    let domain = DVec3::new(dir.dot(axes[0]), dir.dot(axes[1]), dir.dot(axes[2]));
    let octave_seed = seed
        .wrapping_add(BIOME_WARP_SEED_OFFSET)
        .wrapping_mul(7_919)
        .wrapping_add(octave as i64 * 131);
    normal_value_noise(domain * frequency, octave_seed)
}

/// Shader-evaluable twin used for categorical boundary ownership. It keeps
/// the warp's rotated domains, frequencies, and seed schedule, but uses the
/// repository's gradient lattice (`kgnoise` in WGSL) so fragments can recover
/// contours below the vertex lattice instead of interpolating biome colors.
#[inline]
fn boundary_scalar_noise(dir: DVec3, frequency: f64, seed: i64, octave: usize) -> f64 {
    let axes = BIOME_WARP_AXES[octave % BIOME_WARP_AXES.len()];
    let domain = DVec3::new(dir.dot(axes[0]), dir.dot(axes[1]), dir.dot(axes[2]));
    let octave_seed = seed
        .wrapping_add(BIOME_WARP_SEED_OFFSET)
        .wrapping_mul(7_919)
        .wrapping_add(octave as i64 * 131);
    gradient_noise(domain * frequency, octave_seed)
}

pub(crate) fn boundary_shader_seedmul(seed: i64) -> u32 {
    let octave_zero = seed
        .wrapping_add(BIOME_WARP_SEED_OFFSET)
        .wrapping_mul(7_919);
    (octave_zero.wrapping_mul(0x9E37_79B1) & 0xFFFF_FFFF) as u32
}

#[inline]
fn biome_vector_noise(dir: DVec3, frequency: f64, seed: i64, octave: usize) -> DVec3 {
    let axes = BIOME_WARP_AXES[octave % BIOME_WARP_AXES.len()];
    axes[1] * (1.15 * biome_scalar_noise(dir, frequency, seed, octave))
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
    let mut frequency = 1.0 / (texel_angle * BIOME_WARP_BASE_WAVELENGTH_TEXELS);
    let mut amplitude = BIOME_WARP_BASE_AMPLITUDE_TEXELS;
    let mut offset = DVec3::ZERO;
    for octave in 0..BIOME_WARP_OCTAVES as usize {
        let vector = biome_vector_noise(dir, frequency, seed, octave);
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
fn ecotone_column_dir(face: usize, u: f64, v: f64) -> DVec3 {
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
    face_dir(face, uc, vc)
}

fn ecotone_comparator(face: usize, u: f64, v: f64, seed: i64) -> f64 {
    let dir = ecotone_column_dir(face, u, v);
    let n = COLUMNS_PER_FACE as f64;
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

/// One boundary comparator at every range. Its first five scalar samples use
/// the domain warp's exact geometric/seed schedule on the shader-compatible
/// gradient lattice; the remaining five continue down to column scale.
/// Normalization keeps the probability transform area-neutral.
#[derive(Clone, Copy)]
struct BoundaryZoneSignal {
    full_comparator: f64,
}

fn boundary_zone_signal(face: usize, u: f64, v: f64, res: usize, seed: i64) -> BoundaryZoneSignal {
    let dir = ecotone_column_dir(face, u, v);
    let texel_angle = 2.0 / (res - 1) as f64;
    let mut frequency = 1.0 / (texel_angle * BIOME_WARP_BASE_WAVELENGTH_TEXELS);
    let mut sum = 0.0;
    let mut variance = 0.0;
    for octave in 0..BIOME_BOUNDARY_FIELD_OCTAVES as usize {
        let amplitude = boundary_field_amplitude(octave as u32);
        let value = amplitude * boundary_scalar_noise(dir, frequency, seed, octave);
        sum += value;
        variance += amplitude * amplitude;
        frequency *= BIOME_WARP_LACUNARITY;
    }
    BoundaryZoneSignal {
        full_comparator: standard_normal_cdf(
            BIOME_BOUNDARY_FIELD_POLARITY * BIOME_BOUNDARY_GRADIENT_NORMALIZE
                * sum / variance.sqrt(),
        )
        .clamp(0.0, 1.0),
    }
}

fn boundary_zone_comparator(face: usize, u: f64, v: f64, res: usize, seed: i64) -> f64 {
    boundary_zone_signal(face, u, v, res, seed).full_comparator
}

#[cfg(test)]
fn boundary_range_comparator(face: usize, u: f64, v: f64, res: usize, seed: i64) -> f64 {
    let dir = ecotone_column_dir(face, u, v);
    let texel_angle = 2.0 / (res - 1) as f64;
    let mut frequency = 1.0 / (texel_angle * BIOME_WARP_BASE_WAVELENGTH_TEXELS);
    let mut sum = 0.0;
    let mut variance = 0.0;
    for octave in 0..BIOME_RANGE_RESOLVED_OCTAVES as usize {
        let amplitude = boundary_field_amplitude(octave as u32);
        sum += amplitude * boundary_scalar_noise(dir, frequency, seed, octave);
        variance += amplitude * amplitude;
        frequency *= BIOME_WARP_LACUNARITY;
    }
    standard_normal_cdf(
        BIOME_BOUNDARY_FIELD_POLARITY * BIOME_BOUNDARY_GRADIENT_NORMALIZE
            * sum / variance.sqrt(),
    )
    .clamp(0.0, 1.0)
}

#[cfg(test)]
fn production_boundary_zone_comparator(face: usize, u: f64, v: f64, seed: i64) -> f64 {
    boundary_zone_comparator(face, u, v, 1_024, seed)
}

#[cfg(test)]
fn production_boundary_range_comparator(face: usize, u: f64, v: f64, seed: i64) -> f64 {
    boundary_range_comparator(face, u, v, 1_024, seed)
}

pub struct FaceRaster {
    pub res: usize,
    pub elev_km: Vec<f32>,
    pub koppen: Vec<u8>, // 255 = ocean
    /// 1 where the conservative warp + range-filter neighborhood contains
    /// another class. Broad same-class interiors skip procedural evaluation.
    climate_edge: Vec<u64>,
    /// 1 where the range-appearance family changes inside the 4x4 coverage
    /// support. This is deliberately separate: one Koppen class can span a
    /// visible dry/wet tint transition.
    range_edge: Vec<u64>,
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

/// The annual, edit-independent expensive half of one forest-impostor
/// decision. `kind == None` caches an exact phase-one rejection. The lottery
/// stays request-local so the requesting LOD's comp-scaled gate runs before
/// this profile is ever evaluated.
#[derive(Clone, Copy)]
struct LeanImpostorProfile {
    density: f64,
    /// Two checked u16 region offsets packed without layout padding.
    site: u32,
    kind: Option<crate::voxel::TreeKind>,
}

const _: () = assert!(std::mem::size_of::<LeanImpostorProfile>() <= 16);

pub(crate) type ImpostorCandidate = (u64, u64, crate::voxel::TreeKind, f64, f64);

/// Sparse expensive half of one rock decision. It deliberately mirrors the
/// tree profile's 16-byte layout and lives in the same bounded region entry;
/// the requesting LOD still applies its own stride-scaled lottery first.
#[derive(Clone, Copy)]
pub(crate) struct LeanRockProfile {
    pub(crate) density: f64,
    pub(crate) site: u32,
    pub(crate) family: Option<crate::voxel::RockFamily>,
}

const _: () = assert!(std::mem::size_of::<LeanRockProfile>() <= 16);

#[derive(Clone, Copy)]
pub(crate) struct RockPlacement {
    /// Neisor: kilometres above datum. Moon: height/radius ratio.
    pub(crate) height: f64,
    /// Moon surface payload; zero on Neisor.
    pub(crate) albedo: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct LeanRockPlacement {
    pub(crate) height: f64,
    pub(crate) site: u32,
    /// Moon albedo, zero on Neisor, negative for an exact late rejection.
    /// The sentinel keeps this second sparse record at the tree profile's
    /// 16-byte size without losing the moon root's f64 precision.
    pub(crate) albedo: f32,
}

const _: () = assert!(std::mem::size_of::<LeanRockPlacement>() <= 16);

pub(crate) type RockImpostorCandidate = (
    u64,
    u64,
    crate::voxel::RockKind,
    crate::voxel::RockFamily,
    f64,
    f64,
    Option<RockPlacement>,
);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ImpostorCandidateRegionKey {
    pub(crate) face: u8,
    pub(crate) ri: u16,
    pub(crate) rj: u16,
}

struct ImpostorCandidateRegion {
    /// Sorted sparse profiles. Misses are computed outside this lock and
    /// merged afterward, so unrelated Rayon workers never wait behind a
    /// monolithic region build. Concurrent duplicates are harmless pure work
    /// and collapse to one byte-identical record at merge time.
    profiles: Mutex<Vec<LeanImpostorProfile>>,
    /// Second species stream through the same region/LRU. Tree records stay
    /// untouched and therefore retain their original bit-for-bit behavior.
    rock_profiles: Mutex<Vec<LeanRockProfile>>,
    /// Exact annual surface eligibility/root payload, populated only after a
    /// site's density gate wins. This removes repeated ColCtx/crater samples
    /// from tile rebuilds without front-loading the permissive max-density set.
    rock_placements: Mutex<Vec<LeanRockPlacement>>,
    rejects_all: OnceLock<bool>,
    /// Disjoint retained cells used only when the coarser region proof
    /// declines. Rejected cells are omitted; immutable boundary leaves are
    /// shared by every tier.
    proof_cells: OnceLock<Arc<[ImpostorCandidateProofCell]>>,
    last_use: AtomicU64,
    resident_bytes: AtomicUsize,
}

impl ImpostorCandidateRegion {
    fn new(last_use: u64) -> Self {
        Self {
            profiles: Mutex::new(Vec::new()),
            rock_profiles: Mutex::new(Vec::new()),
            rock_placements: Mutex::new(Vec::new()),
            rejects_all: OnceLock::new(),
            proof_cells: OnceLock::new(),
            last_use: AtomicU64::new(last_use),
            resident_bytes: AtomicUsize::new(0),
        }
    }
}

#[derive(Clone, Copy)]
struct ImpostorCandidateProofCell {
    // Absolute columns remove every offset cast/index reconstruction from the
    // proof path. There are normally one to a few cells, so the extra bytes
    // are immaterial beside the cached profiles.
    ci0: u64,
    ci1: u64,
    cj0: u64,
    cj1: u64,
}

#[derive(Default)]
struct ImpostorCandidateCacheShard {
    entries: HashMap<ImpostorCandidateRegionKey, Arc<ImpostorCandidateRegion>>,
    resident_bytes: usize,
}

pub(crate) struct ImpostorCandidateCache {
    shards: [Mutex<ImpostorCandidateCacheShard>; IMPOSTOR_CANDIDATE_CACHE_SHARDS],
    clock: AtomicU64,
}

impl Default for ImpostorCandidateCache {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Mutex::new(ImpostorCandidateCacheShard::default())),
            clock: AtomicU64::new(1),
        }
    }
}

impl ImpostorCandidateCache {
    #[inline]
    pub(crate) fn shard_index(key: ImpostorCandidateRegionKey) -> usize {
        let mixed = u64::from(key.face).wrapping_mul(0x9E37_79B9)
            ^ u64::from(key.ri).wrapping_mul(0x85EB_CA77)
            ^ u64::from(key.rj).wrapping_mul(0xC2B2_AE3D);
        mixed as usize & (IMPOSTOR_CANDIDATE_CACHE_SHARDS - 1)
    }

    fn lock_shard(&self, index: usize) -> std::sync::MutexGuard<'_, ImpostorCandidateCacheShard> {
        self.shards[index]
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn enforce_bound(shard: &mut ImpostorCandidateCacheShard) {
        while shard.entries.len() > IMPOSTOR_CANDIDATE_CACHE_ENTRIES_PER_SHARD
            || shard.resident_bytes > IMPOSTOR_CANDIDATE_CACHE_BYTES_PER_SHARD
        {
            // An entry held by a worker is pinned. This permits only a
            // transient in-flight overshoot; the next access trims it after
            // workers release their Arcs.
            let victim = shard
                .entries
                .iter()
                .filter(|(_, entry)| Arc::strong_count(entry) == 1)
                .min_by_key(|(key, entry)| (entry.last_use.load(Ordering::Relaxed), **key))
                .map(|(key, _)| *key);
            let Some(victim) = victim else {
                break;
            };
            if let Some(entry) = shard.entries.remove(&victim) {
                shard.resident_bytes = shard
                    .resident_bytes
                    .saturating_sub(entry.resident_bytes.load(Ordering::Relaxed));
            }
        }
    }

    fn entry(&self, key: ImpostorCandidateRegionKey) -> Arc<ImpostorCandidateRegion> {
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        let index = Self::shard_index(key);
        let mut shard = self.lock_shard(index);
        if let Some(entry) = shard.entries.get(&key) {
            entry.last_use.store(tick, Ordering::Relaxed);
            return Arc::clone(entry);
        }
        let entry = Arc::new(ImpostorCandidateRegion::new(tick));
        shard.entries.insert(key, Arc::clone(&entry));
        Self::enforce_bound(&mut shard);
        entry
    }

    fn record_bytes(
        &self,
        key: ImpostorCandidateRegionKey,
        entry: &Arc<ImpostorCandidateRegion>,
        bytes: usize,
    ) {
        let index = Self::shard_index(key);
        let mut shard = self.lock_shard(index);
        let still_resident = shard
            .entries
            .get(&key)
            .is_some_and(|resident| Arc::ptr_eq(resident, entry));
        if still_resident {
            entry.resident_bytes.fetch_add(bytes, Ordering::Relaxed);
            shard.resident_bytes = shard.resident_bytes.saturating_add(bytes);
            Self::enforce_bound(&mut shard);
        }
    }

    #[cfg(test)]
    fn trim(&self) {
        for index in 0..IMPOSTOR_CANDIDATE_CACHE_SHARDS {
            let mut shard = self.lock_shard(index);
            Self::enforce_bound(&mut shard);
        }
    }

    pub(crate) fn trim_shards(&self, shard_mask: u16) {
        for index in 0..IMPOSTOR_CANDIDATE_CACHE_SHARDS {
            if shard_mask & (1u16 << index) == 0 {
                continue;
            }
            let mut shard = self.lock_shard(index);
            Self::enforce_bound(&mut shard);
        }
    }

    /// Resolve one request-gated rock slice. Cached lookups happen under the
    /// short region lock, misses are evaluated as one caller-controlled batch
    /// outside it (the moon amortizes crater prefetch here), and the sorted
    /// merge makes cache state unobservable just like the tree stream.
    pub(crate) fn resolve_rock_profiles<F>(
        &self,
        key: ImpostorCandidateRegionKey,
        gated: &[(u64, u64, Option<u32>, f64)],
        evaluate_batch: F,
    ) -> Vec<LeanRockProfile>
    where
        F: FnOnce(&[(u64, u64, u32)]) -> Vec<LeanRockProfile>,
    {
        let entry = self.entry(key);
        let cached: Vec<Option<LeanRockProfile>> = {
            let profiles = entry
                .rock_profiles
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            gated
                .iter()
                .map(|&(_, _, site, _)| {
                    let site = site?;
                    profiles
                        .binary_search_by_key(&site, |profile| profile.site)
                        .ok()
                        .and_then(|index| profiles.get(index).copied())
                })
                .collect()
        };
        let missing: Vec<(usize, u64, u64, u32)> = cached
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| {
                profile.is_none().then(|| {
                    let (ci, cj, site, _) = gated[index];
                    (index, ci, cj, site.unwrap_or(u32::MAX))
                })
            })
            .collect();
        let requests: Vec<(u64, u64, u32)> = missing
            .iter()
            .map(|&(_, ci, cj, site)| (ci, cj, site))
            .collect();
        let evaluated = evaluate_batch(&requests);
        assert_eq!(
            evaluated.len(),
            requests.len(),
            "rock profile evaluator changed request cardinality"
        );

        let mut fresh = vec![None; gated.len()];
        let mut computed = Vec::new();
        for ((index, _, _, site), profile) in missing.into_iter().zip(evaluated) {
            fresh[index] = Some(profile);
            if site != u32::MAX {
                computed.push(profile);
            }
        }
        if !computed.is_empty() {
            let added_capacity = {
                let mut profiles = entry
                    .rock_profiles
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let old_capacity = profiles.capacity();
                profiles.extend(computed);
                profiles.sort_unstable_by_key(|profile| profile.site);
                profiles.dedup_by_key(|profile| profile.site);
                profiles.capacity().saturating_sub(old_capacity)
            };
            self.record_bytes(
                key,
                &entry,
                added_capacity * std::mem::size_of::<LeanRockProfile>(),
            );
        }
        cached
            .into_iter()
            .zip(fresh)
            .map(|(cached, fresh)| cached.or(fresh).expect("every rock profile resolved"))
            .collect()
    }

    /// Resolve the exact surface/root half for density winners only. An
    /// ineligible record is retained just like an eligible one, so repeated
    /// tile builds cannot turn a wet/cave rejection into repeated terrain
    /// work. Sites that cannot be represented in a region are still evaluated
    /// exactly but deliberately remain uncached.
    pub(crate) fn resolve_rock_placements<F>(
        &self,
        key: ImpostorCandidateRegionKey,
        gated: &[(u64, u64, Option<u32>)],
        evaluate_batch: F,
    ) -> Vec<Option<RockPlacement>>
    where
        F: FnOnce(&[(u64, u64, u32)]) -> Vec<LeanRockPlacement>,
    {
        let entry = self.entry(key);
        let cached: Vec<Option<LeanRockPlacement>> = {
            let placements = entry
                .rock_placements
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            gated
                .iter()
                .map(|&(_, _, site)| {
                    let site = site?;
                    placements
                        .binary_search_by_key(&site, |placement| placement.site)
                        .ok()
                        .and_then(|index| placements.get(index).copied())
                })
                .collect()
        };
        let missing: Vec<(usize, u64, u64, u32)> = cached
            .iter()
            .enumerate()
            .filter_map(|(index, placement)| {
                placement.is_none().then(|| {
                    let (ci, cj, site) = gated[index];
                    (index, ci, cj, site.unwrap_or(u32::MAX))
                })
            })
            .collect();
        let requests: Vec<(u64, u64, u32)> = missing
            .iter()
            .map(|&(_, ci, cj, site)| (ci, cj, site))
            .collect();
        let evaluated = evaluate_batch(&requests);
        assert_eq!(
            evaluated.len(),
            requests.len(),
            "rock placement evaluator changed request cardinality"
        );

        let mut fresh = vec![None; gated.len()];
        let mut computed = Vec::new();
        for ((index, _, _, site), placement) in missing.into_iter().zip(evaluated) {
            fresh[index] = Some(placement);
            if site != u32::MAX {
                computed.push(placement);
            }
        }
        if !computed.is_empty() {
            let added_capacity = {
                let mut placements = entry
                    .rock_placements
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let old_capacity = placements.capacity();
                placements.extend(computed);
                placements.sort_unstable_by_key(|placement| placement.site);
                placements.dedup_by_key(|placement| placement.site);
                placements.capacity().saturating_sub(old_capacity)
            };
            self.record_bytes(
                key,
                &entry,
                added_capacity * std::mem::size_of::<LeanRockPlacement>(),
            );
        }
        cached
            .into_iter()
            .zip(fresh)
            .map(|(cached, fresh)| {
                let placement = cached
                    .or(fresh)
                    .expect("every rock placement resolved");
                (placement.albedo >= 0.0).then_some(RockPlacement {
                    height: placement.height,
                    albedo: placement.albedo,
                })
            })
            .collect()
    }
}

pub struct Planet {
    pub radius_km: f64,
    pub seed: i64,
    pub faces: Vec<FaceRaster>,
    /// River courses + lakes from the drainage graph (empty if rivers.bin
    /// is missing — run scripts/bake_rivers.py).
    pub rivers: crate::rivers::RiverIndex,
    /// Shared baked climatology used by every W4 structural consumer. The
    /// renderer borrows this same Arc; there is no second seasonal field.
    pub weather: Option<std::sync::Arc<crate::weather::WeatherField>>,
    /// Bounded annual forest-candidate stream shared by concurrent tile
    /// builds. Seasonal color/temperature and authoritative column gates are
    /// deliberately not retained here.
    impostor_candidates: ImpostorCandidateCache,
}

/// Lightweight all-land fixture for sibling-module unit tests that exercise
/// weather against the real `Planet` sampling API without loading the 160 MiB
/// baked asset set once more in the parallel test process.
#[cfg(test)]
pub(crate) fn weather_test_planet(seed: i64) -> Planet {
    let res = 4usize;
    let n = res * res;
    let koppen = vec![6u8; n];
    let face = || FaceRaster {
        res,
        elev_km: vec![0.2; n],
        koppen: koppen.clone(),
        climate_edge: climate_edge_mask(&koppen, res),
        range_edge: climate_range_edge_mask(&koppen, &vec![[8.0, 500.0]; n], res),
        rough_km: vec![0.0; n],
        climate: vec![[8.0, 500.0]; n],
        flow_log10: vec![0.0; n],
        ocean: vec![0.0; n],
        water: vec![0.0; n],
    };
    Planet {
        radius_km: 6371.0,
        seed,
        faces: (0..6).map(|_| face()).collect(),
        rivers: crate::rivers::RiverIndex::empty(6371.0),
        weather: None,
        impostor_candidates: ImpostorCandidateCache::default(),
    }
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
    range_edge: bool,
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
            let climate: Vec<[f32; 2]> =
                temp_c.into_iter().zip(precip_mm_yr).map(|(t, p)| [t, p]).collect();
            let range_edge = climate_range_edge_mask(&koppen, &climate, res);
            let flow_log10 = f32s(n * 17);
            let ocean = blur_mask(&koppen, res, 2);
            let water = koppen.iter().map(|&k| (k == 255) as u8 as f32).collect();
            faces.push(FaceRaster {
                res,
                elev_km,
                koppen,
                climate_edge,
                range_edge,
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
        let rivers = match crate::rivers::RiverIndex::load(
            &format!("{dir}/rivers.bin"),
            radius_km,
            seed,
        )
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
        let weather = match crate::weather::WeatherField::load(dir) {
            Ok(field) => Some(std::sync::Arc::new(field)),
            Err(error) => {
                eprintln!("structural seasons unavailable ({error})");
                None
            }
        };
        Ok(Self {
            radius_km,
            seed,
            faces,
            rivers,
            weather,
            impostor_candidates: ImpostorCandidateCache::default(),
        })
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

    /// W4's single seasonal temperature entry point for world positions.
    /// Weather-off and missing-bake runs return the old annual raster exactly.
    pub fn seasonal_temp_c(
        &self,
        pos: DVec3,
        season: crate::weather::StructuralSeason,
    ) -> f64 {
        let (face, u, v) = face_from_dir(pos);
        self.seasonal_temp_c_face(face, u, v, season)
    }

    #[inline]
    pub fn seasonal_temp_c_face(
        &self,
        face: usize,
        u: f64,
        v: f64,
        season: crate::weather::StructuralSeason,
    ) -> f64 {
        if !season.enabled {
            return self.temp(face, u, v) as f64;
        }
        let annual = self.temp(face, u, v) as f64;
        self.weather.as_ref().map_or(annual, |field| {
            field
                .seasonal_temp_state_from_phase(
                    annual,
                    face,
                    u,
                    v,
                    season.sin_phase,
                    season.cos_phase,
                    season.is_canonical(),
                )
                .0
        })
    }

    #[inline]
    pub fn seasonal_temp_trend_face(
        &self,
        face: usize,
        u: f64,
        v: f64,
        season: crate::weather::StructuralSeason,
    ) -> f64 {
        if !season.enabled {
            return 0.0;
        }
        self.weather.as_ref().map_or(0.0, |field| {
            field.seasonal_temp_trend(face, u, v, season.season_frac)
        })
    }

    /// One stateless hysteresis loop. Cooling uses the freeze edge; warming
    /// uses the thaw edge. A world-anchored ecotone comparator dithers each
    /// contour over one degree, preventing a whole shore from flipping as a
    /// single raster-shaped line.
    pub fn water_frozen(
        &self,
        face: usize,
        u: f64,
        v: f64,
        sea: bool,
        season: crate::weather::StructuralSeason,
    ) -> bool {
        let annual = self.temp(face, u, v) as f64;
        self.seasonal_water_state_face(face, u, v, annual, sea, season).2
    }

    /// Hot structural path: temperature, derivative, and water class from
    /// one pair of Fourier coefficient reads. `annual` comes from the caller's
    /// already-resident temp/precip raster sample.
    pub fn seasonal_water_state_face(
        &self,
        face: usize,
        u: f64,
        v: f64,
        annual: f64,
        sea: bool,
        season: crate::weather::StructuralSeason,
    ) -> (f64, f64, bool) {
        let (temp, trend) = self.seasonal_temp_state_face(face, u, v, annual, season);
        let frozen = self.water_frozen_from_state(
            face, u, v, annual, temp, trend, sea, season,
        );
        (temp, trend, frozen)
    }

    pub fn seasonal_temp_state_face(
        &self,
        face: usize,
        u: f64,
        v: f64,
        annual: f64,
        season: crate::weather::StructuralSeason,
    ) -> (f64, f64) {
        if !season.enabled || season.is_canonical() || self.weather.is_none() {
            return (annual, 0.0);
        }
        self.weather.as_ref().expect("checked above").seasonal_temp_state_from_phase(
            annual,
            face,
            u,
            v,
            season.sin_phase,
            season.cos_phase,
            false,
        )
    }

    pub fn water_frozen_from_state(
        &self,
        face: usize,
        u: f64,
        v: f64,
        annual: f64,
        temp: f64,
        trend: f64,
        sea: bool,
        season: crate::weather::StructuralSeason,
    ) -> bool {
        if !season.enabled || season.is_canonical() || self.weather.is_none() {
            return annual < -4.0;
        }
        let field = self.weather.as_ref().expect("checked above");
        let comparator = ecotone_comparator(
            face,
            u,
            v,
            self.seed.wrapping_add(0x1CE5_EA),
        );
        if sea && let Some(ice) = field.sea_ice_fraction(face, u, v, season.season_frac)
        {
            return ice > comparator;
        }
        let frozen = crate::weather::hysteretic_frozen(
            temp,
            trend,
            comparator,
            season.freeze_c,
            season.thaw_c,
        );
        frozen
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
            range_edge: (r.range_edge[index >> 6] >> (index & 63)) & 1 != 0,
        }
    }

    /// Constant vegetation signature for a categorical interior. A clear
    /// climate-edge bit proves that the warp and every local dither/signature
    /// read stay on this class. Callers may use this only for monotone
    /// rejection (snow can suppress vegetation but cannot create a tree).
    #[inline]
    pub(crate) fn vegetation_interior(
        &self,
        face: usize,
        u: f64,
        v: f64,
    ) -> Option<(u8, f32)> {
        let (face, u, v) = canonical_face_uv(face, u, v);
        let source = self.raster_position(face, u, v);
        (!source.climate_edge).then(|| (source.here, koppen_forest(source.here)))
    }

    /// Prove a whole same-face rectangle has only categorical interiors whose
    /// constant vegetation signatures satisfy `predicate`. The nearest-raster
    /// bounds are conservative: every texel reachable by any point in the
    /// rectangle is checked, and any climate-edge texel fails the proof.
    ///
    /// This is the region form of `vegetation_interior`; callers may hoist a
    /// monotone rejection out of a dense column loop only when this returns
    /// true. Inputs are expected to be canonical candidate centers strictly
    /// inside one cube face, so no face-edge remapping is needed here.
    #[inline]
    pub(crate) fn vegetation_region_all_interior(
        &self,
        face: usize,
        u0: f64,
        u1: f64,
        v0: f64,
        v1: f64,
        mut predicate: impl FnMut(u8, f32) -> bool,
    ) -> bool {
        debug_assert!(face < self.faces.len());
        debug_assert!((-1.0..=1.0).contains(&u0) && (-1.0..=1.0).contains(&u1));
        debug_assert!((-1.0..=1.0).contains(&v0) && (-1.0..=1.0).contains(&v1));
        let raster = &self.faces[face];
        let d = (raster.res - 1) as f64;
        let nearest = |value: f64| {
            ((value * 0.5 + 0.5) * d).clamp(0.0, d).round() as usize
        };
        let (x0, x1) = (nearest(u0.min(u1)), nearest(u0.max(u1)));
        let (y0, y1) = (nearest(v0.min(v1)), nearest(v0.max(v1)));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let index = y * raster.res + x;
                if (raster.climate_edge[index >> 6] >> (index & 63)) & 1 != 0 {
                    return false;
                }
                let koppen = raster.koppen[index];
                if !predicate(koppen, koppen_forest(koppen)) {
                    return false;
                }
            }
        }
        true
    }

    /// Prove that every vegetation lookup in a same-face rectangle is colder
    /// than the unconditional snowline. The checked climate rectangle expands
    /// the input by the domain warp's strict raster-texel displacement bound,
    /// covering both the source and every possible warped lookup. Near a cube
    /// seam the proof declines instead of reasoning across faces. A small
    /// temperature margin absorbs f32 bilinear rounding, so `true` guarantees
    /// `apply_snow_override` selects snow and no tree candidate can survive.
    #[inline]
    pub(crate) fn vegetation_region_always_snow(
        &self,
        face: usize,
        u0: f64,
        u1: f64,
        v0: f64,
        v1: f64,
    ) -> bool {
        debug_assert!(face < self.faces.len());
        debug_assert!((-1.0..=1.0).contains(&u0) && (-1.0..=1.0).contains(&u1));
        debug_assert!((-1.0..=1.0).contains(&v0) && (-1.0..=1.0).contains(&v1));
        let raster = &self.faces[face];
        let d = (raster.res - 1) as f64;
        let coordinate = |value: f64| ((value * 0.5 + 0.5) * d).clamp(0.0, d);
        let (source_xa, source_xb) = (coordinate(u0.min(u1)), coordinate(u0.max(u1)));
        let (source_ya, source_yb) = (coordinate(v0.min(v1)), coordinate(v0.max(v1)));
        // `climate_position_with_sea` may switch cube faces inside this
        // two-texel source margin. Keep this proof deliberately same-face.
        let face_margin = 2.0;
        if source_xa <= face_margin
            || source_xb >= d - face_margin
            || source_ya <= face_margin
            || source_yb >= d - face_margin
        {
            return false;
        }
        let expand = BIOME_WARP_MAX_DISPLACEMENT_TEXELS + 1e-6;
        let (xa, xb) = (source_xa - expand, source_xb + expand);
        let (ya, yb) = (source_ya - expand, source_yb + expand);
        let mut xs = vec![xa, xb];
        xs.extend((xa.ceil() as usize..=xb.floor() as usize).map(|x| x as f64));
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        let mut ys = vec![ya, yb];
        ys.extend((ya.ceil() as usize..=yb.floor() as usize).map(|y| y as f64));
        ys.sort_by(f64::total_cmp);
        ys.dedup();
        let proof_limit = VEGETATION_UNCONDITIONAL_SNOW_C as f32 - 0.05;
        for &y in &ys {
            for &x in &xs {
                let temp = Self::climate_bilinear_xy(raster, x, y)[0];
                if !temp.is_finite() || temp >= proof_limit {
                    return false;
                }
            }
        }
        true
    }

    /// Prove that the unwarped annual temperature at every point in a
    /// same-face rectangle is below the authoritative non-shrub treeline.
    /// Candidate enumeration has already rejected shrubs, so this is the
    /// exact ColCtx/cheap-root rejection hoisted ahead of the dense lattice.
    /// Bilinear extrema occur at the rectangle corners and crossed texel
    /// boundaries; the small margin absorbs f32 interpolation rounding.
    #[inline]
    pub(crate) fn vegetation_region_below_treeline(
        &self,
        face: usize,
        u0: f64,
        u1: f64,
        v0: f64,
        v1: f64,
    ) -> bool {
        debug_assert!(face < self.faces.len());
        let raster = &self.faces[face];
        let d = (raster.res - 1) as f64;
        let coordinate = |value: f64| ((value * 0.5 + 0.5) * d).clamp(0.0, d);
        let (xa, xb) = (coordinate(u0.min(u1)), coordinate(u0.max(u1)));
        let (ya, yb) = (coordinate(v0.min(v1)), coordinate(v0.max(v1)));
        let mut xs = vec![xa, xb];
        xs.extend((xa.ceil() as usize..=xb.floor() as usize).map(|x| x as f64));
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        let mut ys = vec![ya, yb];
        ys.extend((ya.ceil() as usize..=yb.floor() as usize).map(|y| y as f64));
        ys.sort_by(f64::total_cmp);
        ys.dedup();
        let proof_limit = crate::voxel::TREE_MIN_TEMP_C as f32 - 0.01;
        for &y in &ys {
            for &x in &xs {
                let temp = Self::climate_bilinear_xy(raster, x, y)[0];
                if !temp.is_finite() || temp >= proof_limit {
                    return false;
                }
            }
        }
        true
    }

    #[inline]
    pub(crate) fn impostor_candidate_partition_index(column: u64, partitions: u64) -> u16 {
        debug_assert!(column < COLUMNS_PER_FACE);
        debug_assert!(partitions > 0 && partitions <= u16::MAX as u64);
        ((column * partitions) / COLUMNS_PER_FACE) as u16
    }

    #[inline]
    pub(crate) fn impostor_candidate_partition_bounds(
        region: u16,
        partitions: u64,
    ) -> (u64, u64) {
        debug_assert!(partitions > 0 && partitions <= u16::MAX as u64);
        debug_assert!(u64::from(region) < partitions);
        let region = u64::from(region);
        let start = (region * COLUMNS_PER_FACE).div_ceil(partitions);
        let end = ((region + 1) * COLUMNS_PER_FACE)
            .div_ceil(partitions)
            .saturating_sub(1);
        debug_assert!(start <= end && end < COLUMNS_PER_FACE);
        (start, end)
    }

    #[inline]
    fn impostor_candidate_region_index(column: u64) -> u16 {
        Self::impostor_candidate_partition_index(column, IMPOSTOR_CANDIDATE_REGIONS_PER_FACE)
    }

    #[inline]
    fn impostor_candidate_region_bounds(region: u16) -> (u64, u64) {
        Self::impostor_candidate_partition_bounds(region, IMPOSTOR_CANDIDATE_REGIONS_PER_FACE)
    }

    #[inline]
    pub(crate) fn impostor_candidate_region_index_on_lattice(
        column: u64,
        columns_per_face: u64,
    ) -> u16 {
        debug_assert!(column < columns_per_face);
        ((column * IMPOSTOR_CANDIDATE_REGIONS_PER_FACE) / columns_per_face) as u16
    }

    #[inline]
    pub(crate) fn impostor_candidate_region_bounds_on_lattice(
        region: u16,
        columns_per_face: u64,
    ) -> (u64, u64) {
        let region = u64::from(region);
        let start = (region * columns_per_face).div_ceil(IMPOSTOR_CANDIDATE_REGIONS_PER_FACE);
        let end = ((region + 1) * columns_per_face)
            .div_ceil(IMPOSTOR_CANDIDATE_REGIONS_PER_FACE)
            .saturating_sub(1);
        debug_assert!(start <= end && end < columns_per_face);
        (start, end)
    }

    fn impostor_candidate_bounds_phase_reject_all(
        &self,
        face: u8,
        ci0: u64,
        ci1: u64,
        cj0: u64,
        cj1: u64,
    ) -> bool {
        let nnf = COLUMNS_PER_FACE as f64;
        let col_uv = |column: u64| -1.0 + 2.0 * (column as f64 + 0.5) / nnf;
        let barren_interior = |koppen, forest| {
            forest <= 1e-4
                && !matches!(
                    crate::voxel::tree_kind_density(koppen),
                    Some((kind, _)) if kind != crate::voxel::TreeKind::Shrub
                )
        };
        self.vegetation_region_all_interior(
            face as usize,
            col_uv(ci0),
            col_uv(ci1),
            col_uv(cj0),
            col_uv(cj1),
            barren_interior,
        ) || self.vegetation_region_always_snow(
            face as usize,
            col_uv(ci0),
            col_uv(ci1),
            col_uv(cj0),
            col_uv(cj1),
        )
    }

    fn impostor_candidate_bounds_emit_none(
        &self,
        face: u8,
        ci0: u64,
        ci1: u64,
        cj0: u64,
        cj1: u64,
        depth: u32,
    ) -> bool {
        if self.impostor_candidate_bounds_phase_reject_all(face, ci0, ci1, cj0, cj1) {
            return true;
        }
        let nnf = COLUMNS_PER_FACE as f64;
        let col_uv = |column: u64| -1.0 + 2.0 * (column as f64 + 0.5) / nnf;
        if self.vegetation_region_below_treeline(
            face as usize,
            col_uv(ci0),
            col_uv(ci1),
            col_uv(cj0),
            col_uv(cj1),
        ) {
            return true;
        }
        if depth == 0 || (ci0 == ci1 && cj0 == cj1) {
            return false;
        }

        let ci_width = ci1 - ci0 + 1;
        let cj_width = cj1 - cj0 + 1;
        let ci_parts = if ci_width > 1 { 2 } else { 1 };
        let cj_parts = if cj_width > 1 { 2 } else { 1 };
        for pi in 0..ci_parts {
            let child_ci0 = ci0 + (pi * ci_width).div_ceil(ci_parts);
            let child_ci1 = ci0 + ((pi + 1) * ci_width).div_ceil(ci_parts) - 1;
            for pj in 0..cj_parts {
                let child_cj0 = cj0 + (pj * cj_width).div_ceil(cj_parts);
                let child_cj1 = cj0 + ((pj + 1) * cj_width).div_ceil(cj_parts) - 1;
                if !self.impostor_candidate_bounds_emit_none(
                    face,
                    child_ci0,
                    child_ci1,
                    child_cj0,
                    child_cj1,
                    depth - 1,
                ) {
                    return false;
                }
            }
        }
        true
    }

    /// Prove that every old phase-one candidate in this exact tile lattice is
    /// rejected either by phase one itself or by the authoritative annual
    /// treeline. This proof is legal only for the whole tile: partially
    /// removing late-rejected candidates would change cap/boost accounting.
    pub(crate) fn impostor_tile_emits_none(
        &self,
        face: u8,
        ci0: u64,
        ci1: u64,
        cj0: u64,
        cj1: u64,
    ) -> bool {
        // This is a conservative proof API: malformed bounds prove nothing.
        // Reject them before any face slice access or recursive arithmetic.
        if usize::from(face) >= self.faces.len()
            || ci0 > ci1
            || cj0 > cj1
            || ci1 >= COLUMNS_PER_FACE
            || cj1 >= COLUMNS_PER_FACE
        {
            return false;
        }
        self.impostor_candidate_bounds_emit_none(
            face,
            ci0,
            ci1,
            cj0,
            cj1,
            IMPOSTOR_CANDIDATE_TILE_PROOF_DEPTH,
        )
    }

    fn impostor_candidate_region_rejects_all(&self, key: ImpostorCandidateRegionKey) -> bool {
        let (ci0, ci1) = Self::impostor_candidate_region_bounds(key.ri);
        let (cj0, cj1) = Self::impostor_candidate_region_bounds(key.rj);
        self.impostor_candidate_bounds_phase_reject_all(key.face, ci0, ci1, cj0, cj1)
    }

    fn retain_impostor_candidate_proof_cell(
        &self,
        key: ImpostorCandidateRegionKey,
        ci0: u64,
        ci1: u64,
        cj0: u64,
        cj1: u64,
        depth: u32,
        cells: &mut Vec<ImpostorCandidateProofCell>,
    ) {
        if depth == 0 {
            cells.push(ImpostorCandidateProofCell { ci0, ci1, cj0, cj1 });
            return;
        }

        let ci_width = ci1 - ci0 + 1;
        let cj_width = cj1 - cj0 + 1;
        let ci_parts = if ci_width > 1 { 2 } else { 1 };
        let cj_parts = if cj_width > 1 { 2 } else { 1 };
        let mut children = Vec::with_capacity((ci_parts * cj_parts) as usize);
        let mut rejected_any = false;
        for pi in 0..ci_parts {
            let child_ci0 = ci0 + (pi * ci_width).div_ceil(ci_parts);
            let child_ci1 = ci0 + ((pi + 1) * ci_width).div_ceil(ci_parts) - 1;
            for pj in 0..cj_parts {
                let child_cj0 = cj0 + (pj * cj_width).div_ceil(cj_parts);
                let child_cj1 = cj0 + ((pj + 1) * cj_width).div_ceil(cj_parts) - 1;
                let rejects = self.impostor_candidate_bounds_phase_reject_all(
                    key.face, child_ci0, child_ci1, child_cj0, child_cj1,
                );
                rejected_any |= rejects;
                children.push((child_ci0, child_ci1, child_cj0, child_cj1, rejects));
            }
        }
        // If this subdivision proved nothing, deeper checks are optional for
        // correctness and would only tax an ordinary warm dense region.
        if !rejected_any {
            cells.push(ImpostorCandidateProofCell { ci0, ci1, cj0, cj1 });
            return;
        }
        for (child_ci0, child_ci1, child_cj0, child_cj1, rejects) in children {
            if !rejects {
                self.retain_impostor_candidate_proof_cell(
                    key,
                    child_ci0,
                    child_ci1,
                    child_cj0,
                    child_cj1,
                    depth - 1,
                    cells,
                );
            }
        }
    }

    fn build_impostor_candidate_proof_cells(
        &self,
        key: ImpostorCandidateRegionKey,
    ) -> Arc<[ImpostorCandidateProofCell]> {
        let (region_ci0, region_ci1) = Self::impostor_candidate_region_bounds(key.ri);
        let (region_cj0, region_cj1) = Self::impostor_candidate_region_bounds(key.rj);
        let ci_width = region_ci1 - region_ci0 + 1;
        let cj_width = region_cj1 - region_cj0 + 1;
        let divisions = IMPOSTOR_CANDIDATE_PROOF_INITIAL_DIVISIONS;
        let mut tested = Vec::with_capacity((divisions * divisions) as usize);
        let mut rejected_any = false;
        for pi in 0..divisions {
            let ci0 = region_ci0 + (pi * ci_width).div_ceil(divisions);
            let ci1 = region_ci0 + ((pi + 1) * ci_width).div_ceil(divisions) - 1;
            for pj in 0..divisions {
                let cj0 = region_cj0 + (pj * cj_width).div_ceil(divisions);
                let cj1 = region_cj0 + ((pj + 1) * cj_width).div_ceil(divisions) - 1;
                let rejects =
                    self.impostor_candidate_bounds_phase_reject_all(key.face, ci0, ci1, cj0, cj1);
                rejected_any |= rejects;
                tested.push((ci0, ci1, cj0, cj1, rejects));
            }
        }

        let mut cells = Vec::new();
        if !rejected_any {
            cells.push(ImpostorCandidateProofCell {
                ci0: region_ci0,
                ci1: region_ci1,
                cj0: region_cj0,
                cj1: region_cj1,
            });
        } else {
            for (ci0, ci1, cj0, cj1, rejects) in tested {
                if !rejects {
                    self.retain_impostor_candidate_proof_cell(
                        key,
                        ci0,
                        ci1,
                        cj0,
                        cj1,
                        IMPOSTOR_CANDIDATE_PROOF_REFINEMENT_DEPTH,
                        &mut cells,
                    );
                }
            }
        }
        Arc::from(cells.into_boxed_slice())
    }

    #[inline]
    pub(crate) fn impostor_candidate_site(
        region_ci0: u64,
        region_cj0: u64,
        ci: u64,
        cj: u64,
    ) -> Option<u32> {
        let ci = u16::try_from(ci.checked_sub(region_ci0)?).ok()?;
        let cj = u16::try_from(cj.checked_sub(region_cj0)?).ok()?;
        Some((u32::from(ci) << 16) | u32::from(cj))
    }

    /// Evaluate the exact old phase-one profile after the caller has applied
    /// its own comp-scaled lottery gate. Negative results are retained too;
    /// they are just as expensive and just as annual as positive profiles.
    fn evaluate_impostor_candidate_profile(
        &self,
        face: u8,
        ci: u64,
        cj: u64,
        site: u32,
    ) -> LeanImpostorProfile {
        let face_index = usize::from(face);
        let nnf = COLUMNS_PER_FACE as f64;
        let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nnf;
        let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nnf;
        let barren_interior = |koppen, forest| {
            forest <= 1e-4
                && !matches!(
                    crate::voxel::tree_kind_density(koppen),
                    Some((kind, _)) if kind != crate::voxel::TreeKind::Shrub
                )
        };
        let interior = self.vegetation_interior(face_index, u, v);
        if interior.is_some_and(|(koppen, forest)| barren_interior(koppen, forest)) {
            return LeanImpostorProfile {
                density: 0.0,
                site,
                kind: None,
            };
        }
        let (temp, precip) = self.temp_precip(face_index, u, v);
        // This was the direct loop's second exact cheap gate. The v1 cache
        // accidentally dropped it along with the request-scaled lot gate.
        if interior.is_some() && (temp as f64) < VEGETATION_UNCONDITIONAL_SNOW_C {
            return LeanImpostorProfile {
                density: 0.0,
                site,
                kind: None,
            };
        }
        let vegetation = vegetation_surface(self, face_index, u, v, temp as f64, precip as f64);
        let profile = crate::voxel::tree_biome_profile(
            vegetation.koppen,
            vegetation.main_block,
            vegetation.forest,
            vegetation.temp_c,
            vegetation.precip_mm_yr,
        )
        .filter(|(kind, _)| *kind != crate::voxel::TreeKind::Shrub);
        match profile {
            Some((kind, density)) => LeanImpostorProfile {
                density,
                site,
                kind: Some(kind),
            },
            None => LeanImpostorProfile {
                density: 0.0,
                site,
                kind: None,
            },
        }
    }

    /// Return the exact annual candidate stream for one render-tile lattice.
    /// Every miss is incremental and request-bounded: enumerate only the
    /// requested intersection, apply this LOD's cheap lottery first, reuse or
    /// compute the sparse expensive profiles, then merge misses without
    /// holding a region lock during climate work. The final sort restores the
    /// direct loop's ci-major/cj-minor order, making cache state unobservable.
    pub(crate) fn impostor_candidates(
        &self,
        face: u8,
        ci_start: u64,
        ci_end: u64,
        cj_start: u64,
        cj_end: u64,
        stride: u64,
    ) -> Vec<ImpostorCandidate> {
        if ci_start > ci_end || cj_start > cj_end {
            return Vec::new();
        }
        if usize::from(face) >= self.faces.len()
            || ci_end >= COLUMNS_PER_FACE
            || cj_end >= COLUMNS_PER_FACE
            || !IMPOSTOR_CANDIDATE_STRIDES.contains(&stride)
        {
            return Vec::new();
        }
        let ri0 = Self::impostor_candidate_region_index(ci_start);
        let ri1 = Self::impostor_candidate_region_index(ci_end);
        let rj0 = Self::impostor_candidate_region_index(cj_start);
        let rj1 = Self::impostor_candidate_region_index(cj_end);
        let ri_count = usize::from(ri1) - usize::from(ri0) + 1;
        let rj_count = usize::from(rj1) - usize::from(rj0) + 1;
        let mut keys = Vec::with_capacity(ri_count * rj_count);
        let mut touched_shards = 0u16;
        for ri in ri0..=ri1 {
            for rj in rj0..=rj1 {
                let key = ImpostorCandidateRegionKey { face, ri, rj };
                touched_shards |= 1u16 << ImpostorCandidateCache::shard_index(key);
                keys.push(key);
            }
        }

        let comp = (stride * stride) as f64;
        let mut candidates: Vec<ImpostorCandidate> = Vec::new();
        for key in keys {
            let entry = self.impostor_candidates.entry(key);
            if *entry
                .rejects_all
                .get_or_init(|| self.impostor_candidate_region_rejects_all(key))
            {
                continue;
            }
            let proof_cells = entry.proof_cells.get_or_init(|| {
                let cells = self.build_impostor_candidate_proof_cells(key);
                self.impostor_candidates.record_bytes(
                    key,
                    &entry,
                    cells.len() * std::mem::size_of::<ImpostorCandidateProofCell>(),
                );
                cells
            });
            let (region_ci0, _) = Self::impostor_candidate_region_bounds(key.ri);
            let (region_cj0, _) = Self::impostor_candidate_region_bounds(key.rj);
            let mut gated = Vec::new();
            for cell in proof_cells.iter() {
                let cell_ci0 = cell.ci0.max(ci_start);
                let cell_ci1 = cell.ci1.min(ci_end);
                let cell_cj0 = cell.cj0.max(cj_start);
                let cell_cj1 = cell.cj1.min(cj_end);
                if cell_ci0 > cell_ci1 || cell_cj0 > cell_cj1 {
                    continue;
                }
                let first_ci = cell_ci0.div_ceil(stride) * stride;
                let first_cj = cell_cj0.div_ceil(stride) * stride;
                if first_ci > cell_ci1 || first_cj > cell_cj1 {
                    continue;
                }
                for ci in (first_ci..=cell_ci1).step_by(stride as usize) {
                    for cj in (first_cj..=cell_cj1).step_by(stride as usize) {
                        let lot = crate::voxel::tree_hash01(face, ci, cj, self.seed);
                        if lot >= crate::voxel::MAX_TREE_DENSITY * comp {
                            continue;
                        }
                        // Region widths are ~611 columns, but keep this fully
                        // checked: a malformed partition can fall back to an
                        // uncached exact profile instead of panicking.
                        let site = Self::impostor_candidate_site(region_ci0, region_cj0, ci, cj);
                        gated.push((ci, cj, site, lot));
                    }
                }
            }

            let cached: Vec<Option<LeanImpostorProfile>> = {
                let profiles = entry
                    .profiles
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                gated
                    .iter()
                    .map(|&(_, _, site, _)| {
                        let site = site?;
                        profiles
                            .binary_search_by_key(&site, |profile| profile.site)
                            .ok()
                            .and_then(|index| profiles.get(index).copied())
                    })
                    .collect()
            };
            let mut computed = Vec::new();
            let resolved: Vec<LeanImpostorProfile> = gated
                .iter()
                .zip(cached)
                .map(|(&(ci, cj, site, _), cached)| {
                    cached.unwrap_or_else(|| {
                        let site = site.unwrap_or(u32::MAX);
                        let profile = self.evaluate_impostor_candidate_profile(face, ci, cj, site);
                        if site != u32::MAX {
                            computed.push(profile);
                        }
                        profile
                    })
                })
                .collect();

            if !computed.is_empty() {
                let added_capacity = {
                    let mut profiles = entry
                        .profiles
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let old_capacity = profiles.capacity();
                    profiles.extend(computed);
                    profiles.sort_unstable_by_key(|profile| profile.site);
                    profiles.dedup_by_key(|profile| profile.site);
                    profiles.capacity().saturating_sub(old_capacity)
                };
                self.impostor_candidates.record_bytes(
                    key,
                    &entry,
                    added_capacity * std::mem::size_of::<LeanImpostorProfile>(),
                );
            }

            for ((ci, cj, _, lot), profile) in gated.into_iter().zip(resolved) {
                let Some(kind) = profile.kind else {
                    continue;
                };
                if lot < profile.density * comp {
                    candidates.push((ci, cj, kind, lot, profile.density));
                }
            }
        }
        // All per-region entry Arcs from this request are gone here. Trim
        // after the request so concurrent builds may overshoot only while
        // their workers are actively consuming entries.
        self.impostor_candidates.trim_shards(touched_shards);
        candidates.sort_unstable_by_key(|&(ci, cj, _, _, _)| (ci, cj));
        candidates
    }

    fn evaluate_rock_candidate_profile(
        &self,
        face: u8,
        ci: u64,
        cj: u64,
        site: u32,
    ) -> LeanRockProfile {
        let face_index = usize::from(face);
        let nnf = COLUMNS_PER_FACE as f64;
        let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nnf;
        let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nnf;
        let elevation = self.elevation(face_index, u, v);
        if sea_from_fields(
            f64::from(elevation),
            f64::from(self.water_frac(face_index, u, v)),
            f64::from(self.ocean(face_index, u, v)),
        ) {
            return LeanRockProfile {
                density: 0.0,
                site,
                family: None,
            };
        }
        let (temp, precip) = self.temp_precip(face_index, u, v);
        let geology = vegetation_surface(
            self,
            face_index,
            u,
            v,
            f64::from(temp),
            f64::from(precip),
        );
        let (family, density) = crate::voxel::neisor_rock_profile(
            geology.main_block,
            geology.forest,
            elevation,
            self.rough(face_index, u, v),
        );
        LeanRockProfile {
            density,
            site,
            family: Some(family),
        }
    }

    /// The second B-4a species stream. It shares the tree cache's regions,
    /// sharding, request-first lottery, miss-outside-lock merge, byte bound,
    /// and deterministic final order. Tree profiles and proofs are separate,
    /// so no rock request can alter a tree decision.
    pub(crate) fn rock_impostor_candidates(
        &self,
        face: u8,
        ci_start: u64,
        ci_end: u64,
        cj_start: u64,
        cj_end: u64,
        stride: u64,
    ) -> Vec<RockImpostorCandidate> {
        if ci_start > ci_end || cj_start > cj_end {
            return Vec::new();
        }
        if usize::from(face) >= self.faces.len()
            || ci_end >= COLUMNS_PER_FACE
            || cj_end >= COLUMNS_PER_FACE
            || !IMPOSTOR_CANDIDATE_STRIDES.contains(&stride)
        {
            return Vec::new();
        }
        let ri0 = Self::impostor_candidate_region_index(ci_start);
        let ri1 = Self::impostor_candidate_region_index(ci_end);
        let rj0 = Self::impostor_candidate_region_index(cj_start);
        let rj1 = Self::impostor_candidate_region_index(cj_end);
        let comp = (stride * stride) as f64;
        let mut touched_shards = 0u16;
        let mut candidates = Vec::new();
        let no_edits = crate::voxel::Edits::default();

        for ri in ri0..=ri1 {
            for rj in rj0..=rj1 {
                let key = ImpostorCandidateRegionKey { face, ri, rj };
                touched_shards |= 1u16 << ImpostorCandidateCache::shard_index(key);
                let (region_ci0, region_ci1) = Self::impostor_candidate_region_bounds(ri);
                let (region_cj0, region_cj1) = Self::impostor_candidate_region_bounds(rj);
                let cell_ci0 = region_ci0.max(ci_start);
                let cell_ci1 = region_ci1.min(ci_end);
                let cell_cj0 = region_cj0.max(cj_start);
                let cell_cj1 = region_cj1.min(cj_end);
                if cell_ci0 > cell_ci1 || cell_cj0 > cell_cj1 {
                    continue;
                }
                let first_ci = cell_ci0.div_ceil(stride) * stride;
                let first_cj = cell_cj0.div_ceil(stride) * stride;
                if first_ci > cell_ci1 || first_cj > cell_cj1 {
                    continue;
                }
                let mut gated = Vec::new();
                for ci in (first_ci..=cell_ci1).step_by(stride as usize) {
                    for cj in (first_cj..=cell_cj1).step_by(stride as usize) {
                        let lot = crate::voxel::rock_hash01(face, ci, cj, self.seed);
                        if lot >= crate::voxel::rock_tuning::MAX_DENSITY * comp {
                            continue;
                        }
                        let site = Self::impostor_candidate_site(region_ci0, region_cj0, ci, cj);
                        gated.push((ci, cj, site, lot));
                    }
                }
                let resolved = self.impostor_candidates.resolve_rock_profiles(
                    key,
                    &gated,
                    |requests| {
                        requests
                            .iter()
                            .map(|&(ci, cj, site)| {
                                self.evaluate_rock_candidate_profile(face, ci, cj, site)
                            })
                            .collect()
                    },
                );
                let mut winners = Vec::new();
                for ((ci, cj, site, lot), profile) in gated.into_iter().zip(resolved) {
                    let Some(family) = profile.family else {
                        continue;
                    };
                    if lot >= profile.density * comp {
                        continue;
                    }
                    winners.push((ci, cj, site, lot, profile.density, family));
                }
                let placement_requests: Vec<_> = winners
                    .iter()
                    .map(|&(ci, cj, site, _, _, _)| (ci, cj, site))
                    .collect();
                let placements = self.impostor_candidates.resolve_rock_placements(
                    key,
                    &placement_requests,
                    |requests| {
                        requests
                            .iter()
                            .map(|&(ci, cj, site)| {
                                let column = crate::voxel::col_ctx(
                                    self,
                                    &no_edits,
                                    usize::from(face),
                                    ci,
                                    cj,
                                );
                                LeanRockPlacement {
                                    height: f64::from(column.h_km),
                                    site,
                                    albedo: if crate::voxel::rock_surface_eligible(&column) {
                                        0.0
                                    } else {
                                        -1.0
                                    },
                                }
                            })
                            .collect()
                    },
                );
                for ((ci, cj, _, lot, density, family), placement) in
                    winners.into_iter().zip(placements)
                {
                    candidates.push((
                        ci,
                        cj,
                        crate::voxel::rock_kind(family, face, ci, cj, self.seed),
                        family,
                        lot,
                        density,
                        placement,
                    ));
                }
            }
        }
        self.impostor_candidates.trim_shards(touched_shards);
        candidates.sort_unstable_by_key(|&(ci, cj, _, _, _, _, _)| (ci, cj));
        candidates
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

        // A same-class neighbor ring as wide as the displacement bound proves
        // that the categorical result cannot change. This is the main per-
        // vertex fast path: broad biome interiors retain the identity transform
        // without evaluating noise that could have no visible class effect.
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

    /// Class and climate at one categorical texel, with the same real
    /// cross-face neighbor rule as `koppen_texel`.
    fn koppen_climate_texel(&self, face: usize, x: i64, y: i64) -> (u8, [f32; 2]) {
        let r = &self.faces[face];
        if x >= 0 && x < r.res as i64 && y >= 0 && y < r.res as i64 {
            let index = y as usize * r.res + x as usize;
            return (r.koppen[index], r.climate[index]);
        }
        let d = (r.res - 1) as f64;
        let u = -1.0 + 2.0 * x as f64 / d;
        let v = -1.0 + 2.0 * y as f64 / d;
        let (f2, u2, v2) = face_from_dir(face_dir(face, u, v));
        let p = self.raster_position(f2, u2, v2);
        let rr = &self.faces[p.face];
        let index = p.y.round() as usize * rr.res + p.x.round() as usize;
        (rr.koppen[index], rr.climate[index])
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

    /// Forest half of `koppen_signature`, preserving its exact interpolation
    /// order without calculating display colors. Candidate-only biome lookups
    /// use this on proven same-class interiors.
    #[inline]
    fn koppen_forest_signature(&self, p: RasterPosition) -> f32 {
        let r = &self.faces[p.face];
        let (x0, y0) = (p.x.floor() as usize, p.y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(r.res - 1), (y0 + 1).min(r.res - 1));
        let (fx, fy) = ((p.x - x0 as f64) as f32, (p.y - y0 as f64) as f32);
        let at = |x: usize, y: usize| koppen_forest(r.koppen[y * r.res + x]);
        let (fa, fb, fc, fd) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        (fa + (fb - fa) * fx) * (1.0 - fy) + (fc + (fd - fc) * fx) * fy
    }

    /// Continuous main-block weights for a cross-material band.
    ///
    /// The categorical texels live at integer raster coordinates and their
    /// nearest-sample boundaries at half-integers. Each axis gets a smooth
    /// 0..1 ramp around its nearest boundary; the tensor product weights the
    /// surrounding texels. Production supplies the one all-range 8 km width.
    /// Tests can still request the legacy 300 m support as an area-invariance
    /// oracle. Both widths have the same limit from every corner quadrant.
    ///
    /// Returns the nearest class (for the no-blend fast path), accumulated
    /// weights in Grass/Sand/Snow order, and one representative class per
    /// block. The representative is immaterial to rendering: callers use its
    /// main block, while the long-range Koppen tint signal remains separately
    /// bilinear in `koppen_signature`.
    #[cfg(test)]
    fn koppen_block_weights(&self, face: usize, u: f64, v: f64) -> (u8, [f64; 3], [u8; 3]) {
        self.koppen_block_weights_at(
            self.raster_position(face, u, v),
            CROSS_BLOCK_ECOTONE_KM,
        )
    }

    fn koppen_block_weights_at(
        &self,
        p: RasterPosition,
        requested_width_km: f64,
    ) -> (u8, [f64; 3], [u8; 3]) {
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
        let width_km = requested_width_km
            .max(CROSS_BLOCK_ECOTONE_KM)
            .min(BIOME_BOUNDARY_ZONE_MAX_TEXEL_FRACTION * km_x.min(km_y));
        let half = width_km * 0.5;
        let axis_weight = |coord: f64, edge: f64, km_per_texel: f64| {
            let t = (0.5 + (coord - edge) * km_per_texel / (2.0 * half)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let (x0, y0) = (x.floor() as i64, y.floor() as i64);
        let wx = axis_weight(x, x0 as f64 + 0.5, km_x);
        let wy = axis_weight(y, y0 as f64 + 0.5, km_y);

        let block_index = |block| match block {
            MainBlock::Grass => 0,
            MainBlock::Sand => 1,
            MainBlock::Snow => 2,
        };
        let mut weights = [0.0; 3];
        let mut representatives = [here; 3];
        let mut seen = [false; 3];
        let mut add_sample = |class, weight| {
            let slot = block_index(koppen_main_block(class));
            weights[slot] += weight;
            if weight > 0.0 && !seen[slot] {
                representatives[slot] = class;
                seen[slot] = true;
            }
        };

        if requested_width_km <= CROSS_BLOCK_ECOTONE_KM {
            for (class, weight) in [
                (self.koppen_texel(p.face, x0, y0), (1.0 - wx) * (1.0 - wy)),
                (self.koppen_texel(p.face, x0 + 1, y0), wx * (1.0 - wy)),
                (self.koppen_texel(p.face, x0, y0 + 1), (1.0 - wx) * wy),
                (self.koppen_texel(p.face, x0 + 1, y0 + 1), wx * wy),
            ] {
                add_sample(class, weight);
            }
        } else if !p.climate_edge {
            // The load-time mask covers the warp reach and this filter's full
            // support. A clear bit proves all 16 reads would be `here`.
            add_sample(here, 1.0);
        } else {
            let side = BIOME_BOUNDARY_PREFILTER_SIDE_WEIGHT;
            let center = 1.0 - 2.0 * side;
            let filtered_axis = |w: f64| {
                [
                    (1.0 - w) * side,
                    (1.0 - w) * center + w * side,
                    (1.0 - w) * side + w * center,
                    w * side,
                ]
            };
            let x_weights = filtered_axis(wx);
            let y_weights = filtered_axis(wy);
            for (dy, y_weight) in y_weights.into_iter().enumerate() {
                for (dx, x_weight) in x_weights.into_iter().enumerate() {
                    add_sample(
                        self.koppen_texel(p.face, x0 + dx as i64 - 1, y0 + dy as i64 - 1),
                        x_weight * y_weight,
                    );
                }
            }
        }
        (here, weights, representatives)
    }

    /// Appearance-family counterpart to `koppen_block_weights_at` for range color.
    /// The material path deliberately aggregates to three main blocks; doing
    /// that to display color erased temperate/steppe/tundra identity and left
    /// a smooth tint wash even when material ownership was categorical. Fold
    /// the taps into eight globally stable visual/material families.
    fn koppen_range_weights_at(
        &self,
        p: RasterPosition,
        requested_width_km: f64,
    ) -> [f32; BIOME_RANGE_FAMILIES] {
        let r = &self.faces[p.face];
        // The semantic mask covers the complete 4x4 range-filter footprint.
        // Most vertices are therefore a proven one-family interior: return a
        // one-hot payload before doing footprint metric/filter work. Besides
        // being exact, this keeps the tile-build cost near the round-3 path.
        if requested_width_km > CROSS_BLOCK_ECOTONE_KM && !p.range_edge {
            let x = p.x.round().clamp(0.0, (r.res - 1) as f64) as usize;
            let y = p.y.round().clamp(0.0, (r.res - 1) as f64) as usize;
            let index = y * r.res + x;
            let family = climate_range_family(r.koppen[index], r.climate[index]);
            return std::array::from_fn(|slot| if slot == family { 1.0 } else { 0.0 });
        }
        let d = (r.res - 1) as f64;
        let denom = 1.0 + p.u * p.u + p.v * p.v;
        let texel_uv = 2.0 / d;
        let km_x = self.radius_km * texel_uv * (1.0 + p.v * p.v).sqrt() / denom;
        let km_y = self.radius_km * texel_uv * (1.0 + p.u * p.u).sqrt() / denom;
        let width_km = requested_width_km
            .max(CROSS_BLOCK_ECOTONE_KM)
            .min(BIOME_BOUNDARY_ZONE_MAX_TEXEL_FRACTION * km_x.min(km_y));
        let half = width_km * 0.5;
        let axis_weight = |coord: f64, edge: f64, km_per_texel: f64| {
            let t = (0.5 + (coord - edge) * km_per_texel / (2.0 * half)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let (x0, y0) = (p.x.floor() as i64, p.y.floor() as i64);
        let wx = axis_weight(p.x, x0 as f64 + 0.5, km_x);
        let wy = axis_weight(p.y, y0 as f64 + 0.5, km_y);

        let mut family_weight = [0.0f64; BIOME_RANGE_FAMILIES];
        let mut add = |class: u8, climate: [f32; 2], weight: f64| {
            if weight <= 0.0 {
                return;
            }
            let family = climate_range_family(class, climate);
            family_weight[family] += weight;
        };
        let mut add_texel = |x: i64, y: i64, weight: f64| {
            let (class, climate) = self.koppen_climate_texel(p.face, x, y);
            add(class, climate, weight);
        };

        if requested_width_km <= CROSS_BLOCK_ECOTONE_KM {
            for (x, y, weight) in [
                (x0, y0, (1.0 - wx) * (1.0 - wy)),
                (x0 + 1, y0, wx * (1.0 - wy)),
                (x0, y0 + 1, (1.0 - wx) * wy),
                (x0 + 1, y0 + 1, wx * wy),
            ] {
                add_texel(x, y, weight);
            }
        } else {
            let side = BIOME_BOUNDARY_PREFILTER_SIDE_WEIGHT;
            let center = 1.0 - 2.0 * side;
            let filtered_axis = |w: f64| {
                [
                    (1.0 - w) * side,
                    (1.0 - w) * center + w * side,
                    (1.0 - w) * side + w * center,
                    w * side,
                ]
            };
            let x_weights = filtered_axis(wx);
            let y_weights = filtered_axis(wy);
            for (dy, y_weight) in y_weights.into_iter().enumerate() {
                for (dx, x_weight) in x_weights.into_iter().enumerate() {
                    add_texel(
                        x0 + dx as i64 - 1,
                        y0 + dy as i64 - 1,
                        x_weight * y_weight,
                    );
                }
            }
        }

        debug_assert!((family_weight.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        family_weight.map(|weight| weight as f32)
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
        self.dithered_koppen_at(
            self.raster_position(face, u, v),
            CROSS_BLOCK_ECOTONE_KM,
            comparator,
        )
    }

    #[cfg(test)]
    fn dithered_koppen_at(
        &self,
        p: RasterPosition,
        width_km: f64,
        comparator: impl FnOnce() -> f64,
    ) -> u8 {
        let (here, weights, representatives) = self.koppen_block_weights_at(p, width_km);
        Self::choose_dithered_koppen(here, weights, representatives, comparator)
    }

    fn choose_dithered_koppen(
        here: u8,
        weights: [f64; 3],
        representatives: [u8; 3],
        comparator: impl FnOnce() -> f64,
    ) -> u8 {
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

/// Conservative same-class interior mask for the domain-warp fast path. Its
/// radius follows the artist-facing displacement cap. Outer-ring texels stay
/// marked because their true neighbors live on another cube face; they must
/// never be guessed from a clamped edge.
fn climate_edge_mask(koppen: &[u8], res: usize) -> Vec<u64> {
    let mut edge = vec![0u64; koppen.len().div_ceil(64)];
    let radius = BIOME_WARP_MAX_DISPLACEMENT_TEXELS.ceil() as isize
        + BIOME_BOUNDARY_PREFILTER_SUPPORT_TEXELS;
    for y in 0..res {
        for x in 0..res {
            let mut differs = x < radius as usize
                || y < radius as usize
                || x + radius as usize >= res
                || y + radius as usize >= res;
            if !differs {
                let here = koppen[y * res + x];
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
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

/// Conservative interior mask for the range appearance quantizer. Koppen's
/// byte alone is insufficient: temperature/precipitation can cross a visual
/// bin inside one class. The radius encloses the 4x4 coverage prefilter; face
/// borders remain marked for real neighbor resolution.
fn climate_range_edge_mask(koppen: &[u8], climate: &[[f32; 2]], res: usize) -> Vec<u64> {
    let mut edge = vec![0u64; koppen.len().div_ceil(64)];
    let families: Vec<u8> = koppen
        .iter()
        .zip(climate)
        .map(|(&class, &values)| climate_range_family(class, values) as u8)
        .collect();
    let radius = BIOME_BOUNDARY_PREFILTER_SUPPORT_TEXELS;
    for y in 0..res {
        for x in 0..res {
            let mut differs = x < radius as usize
                || y < radius as usize
                || x + radius as usize >= res
                || y + radius as usize >= res;
            if !differs {
                let index = y * res + x;
                let here = families[index];
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let xx = (x as isize + dx) as usize;
                        let yy = (y as isize + dy) as usize;
                        let at = yy * res + xx;
                        differs |= families[at] != here;
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

#[derive(Clone, Copy)]
struct ClimateSurfaceBasis {
    p: RasterPosition,
    world_face: usize,
    world_u: f64,
    world_v: f64,
    temp_c: f64,
    /// Temperature used only by the snow override and display tints. The
    /// annual `temp_c` above continues to own biome ranges and vegetation.
    surface_temp_c: f64,
    precip_mm: f64,
    koppen_anchor: [f32; 3],
    forest: f32,
}

fn climate_surface_basis(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
) -> ClimateSurfaceBasis {
    climate_surface_basis_at_season(
        planet,
        face,
        u,
        v,
        unwarped_temp_c,
        unwarped_temp_c,
        unwarped_precip_mm,
        unwarped_sea,
        crate::weather::StructuralSeason::annual(),
    )
}

fn climate_surface_basis_at_season(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_surface_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
    season: crate::weather::StructuralSeason,
) -> ClimateSurfaceBasis {
    let (world_face, world_u, world_v) = canonical_face_uv(face, u, v);
    let lookup =
        planet.climate_position_with_sea(world_face, world_u, world_v, unwarped_sea);
    let p = lookup.raster;
    let (koppen_anchor, forest) = planet.koppen_signature(p);
    let (temp_c, precip_mm) = if lookup.warped {
        let climate = planet.climate_bilinear_at(p);
        (climate[0] as f64, climate[1] as f64)
    } else {
        (unwarped_temp_c, unwarped_precip_mm)
    };
    ClimateSurfaceBasis {
        p,
        world_face,
        world_u,
        world_v,
        temp_c,
        surface_temp_c: if season.enabled && lookup.warped {
            planet.seasonal_temp_c_face(p.face, p.u, p.v, season)
        } else if season.enabled {
            unwarped_surface_temp_c
        } else {
            temp_c
        },
        precip_mm,
        koppen_anchor,
        forest,
    }
}

fn climate_cross_block(planet: &Planet, basis: ClimateSurfaceBasis) -> MainBlock {
    climate_boundary_selection(planet, basis)
}

fn climate_boundary_selection(
    planet: &Planet,
    basis: ClimateSurfaceBasis,
) -> MainBlock {
    let (here, weights, representatives) =
        planet.koppen_block_weights_at(basis.p, BIOME_BOUNDARY_ZONE_KM);
    let chosen = Planet::choose_dithered_koppen(here, weights, representatives, || {
        boundary_zone_comparator(
            basis.world_face,
            basis.world_u,
            basis.world_v,
            planet.faces[basis.p.face].res,
            planet.seed,
        )
    });
    koppen_main_block(chosen)
}

/// Exact local material selection plus eight stable appearance-family weights.
/// Thresholds are their cumulative coverages in fixed semantic order.
/// The fragment shader evaluates the resolved comparator directly, defining
/// categorical contours even when all candidates share one main block (the
/// specimen's Dfb/tundra wash).
fn climate_boundary_pair_selection(
    planet: &Planet,
    basis: ClimateSurfaceBasis,
) -> (
    MainBlock,
    [f32; BIOME_RANGE_FAMILIES],
    [f32; BIOME_RANGE_FAMILIES],
    [f32; 3],
) {
    let (here, weights, representatives) =
        planet.koppen_block_weights_at(basis.p, BIOME_BOUNDARY_ZONE_KM);
    let range_weights = planet.koppen_range_weights_at(basis.p, BIOME_BOUNDARY_ZONE_KM);
    let block_mixed = weights.iter().filter(|&&weight| weight > 0.0).count() > 1;
    let signal = block_mixed.then(|| {
        boundary_zone_signal(
            basis.world_face,
            basis.world_u,
            basis.world_v,
            planet.faces[basis.p.face].res,
            planet.seed,
        )
    });
    let chosen = if block_mixed {
        let signal = signal.expect("mixed block weights require a boundary signal");
        Planet::choose_dithered_koppen(here, weights, representatives, || {
            signal.full_comparator
        })
    } else {
        here
    };
    let mut cumulative = 0.0f32;
    let thresholds = std::array::from_fn(|family| {
        cumulative += range_weights[family];
        cumulative.min(1.0)
    });
    (
        koppen_main_block(chosen),
        range_weights,
        thresholds,
        weights.map(|weight| weight as f32),
    )
}

fn apply_snow_override(
    planet: &Planet,
    basis: ClimateSurfaceBasis,
    blocks: &mut [MainBlock],
) -> bool {
    let snow_low = SNOWLINE_CENTER_C - SNOWLINE_HALF_RANGE_C;
    let snow_high = SNOWLINE_CENTER_C + SNOWLINE_HALF_RANGE_C;
    if basis.surface_temp_c < snow_low {
        blocks.fill(MainBlock::Snow);
        return true;
    } else if basis.surface_temp_c < snow_high
        && blocks.iter().any(|&b| b != MainBlock::Snow)
        && snow_transition_forces_snow(planet, basis)
    {
        blocks.fill(MainBlock::Snow);
        return true;
    }
    false
}

#[inline]
fn snow_transition_forces_snow(planet: &Planet, basis: ClimateSurfaceBasis) -> bool {
    let snow_comparator = ecotone_comparator(
        basis.world_face,
        basis.world_u,
        basis.world_v,
        planet.seed.wrapping_add(SNOW_FIELD_SEED_OFFSET),
    );
    let snow_threshold = SNOWLINE_CENTER_C
        + (snow_comparator - 0.5) * (SNOWLINE_HALF_RANGE_C * 2.0);
    basis.surface_temp_c < snow_threshold
}

#[inline]
fn vegetation_forces_snow(planet: &Planet, basis: ClimateSurfaceBasis) -> bool {
    basis.surface_temp_c < VEGETATION_UNCONDITIONAL_SNOW_C
        || (basis.surface_temp_c < SNOWLINE_CENTER_C + SNOWLINE_HALF_RANGE_C
            && snow_transition_forces_snow(planet, basis))
}

fn finish_climate_surface(
    basis: ClimateSurfaceBasis,
    main_block: MainBlock,
    block_weights: [f32; 3],
) -> ClimateSurface {
    let temp = basis.surface_temp_c as f32;
    let precip = basis.precip_mm as f32;
    ClimateSurface {
        main_block,
        grass: hue_nudge(climate_grass(temp, precip), basis.koppen_anchor),
        sand: climate_sand(temp, precip),
        snow: climate_snow(temp, precip),
        block_weights,
        forest: basis.forest,
    }
}

fn finish_range_candidate(family: usize, force_snow: bool) -> ClimateSurface {
    let representative = BIOME_RANGE_FAMILY_REPRESENTATIVE[family];
    let temp = BIOME_RANGE_FAMILY_TEMP_C[family];
    let precip = BIOME_RANGE_FAMILY_PRECIP_MM[family];
    let main_block = if force_snow {
        MainBlock::Snow
    } else {
        koppen_main_block(representative)
    };
    let one_hot = match main_block {
        MainBlock::Grass => [1.0, 0.0, 0.0],
        MainBlock::Sand => [0.0, 1.0, 0.0],
        MainBlock::Snow => [0.0, 0.0, 1.0],
    };
    ClimateSurface {
        main_block,
        grass: hue_nudge(climate_grass(temp, precip), ground_tint(representative)),
        sand: climate_sand(temp, precip),
        snow: climate_snow(temp, precip),
        block_weights: one_hot,
        forest: koppen_forest(representative),
    }
}

fn range_candidate_palette(force_snow: bool) -> &'static [ClimateSurface; BIOME_RANGE_FAMILIES] {
    static NORMAL: std::sync::OnceLock<[ClimateSurface; BIOME_RANGE_FAMILIES]> =
        std::sync::OnceLock::new();
    static SNOW: std::sync::OnceLock<[ClimateSurface; BIOME_RANGE_FAMILIES]> =
        std::sync::OnceLock::new();
    let palette = if force_snow { &SNOW } else { &NORMAL };
    palette.get_or_init(|| {
        std::array::from_fn(|family| finish_range_candidate(family, force_snow))
    })
}

#[inline]
fn biome_climate_from_basis(basis: ClimateSurfaceBasis, sea: bool) -> BiomeClimate {
    BiomeClimate {
        koppen: basis.p.here,
        temp_c: basis.temp_c as f32,
        precip_mm_yr: basis.precip_mm as f32,
        sea,
    }
}

/// Tree-candidate view of the exact local climate surface. This performs the
/// same warp, categorical dither, snow override, signature, and physical-sea
/// classification as `biome_climate` + `climate_surface`, but shares their
/// climate position and does not calculate display-only colors.
pub fn vegetation_surface(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_precip_mm: f64,
) -> VegetationSurface {
    let (world_face, world_u, world_v) = canonical_face_uv(face, u, v);
    let source = planet.raster_position(world_face, world_u, world_v);

    // The load-time interior bit proves that the warp, the local four-texel
    // dither, and the signature's bilinear footprint all see one class. Build
    // their exact result directly: candidate enumeration does not consume
    // physical-sea ownership or display tints. Boundary columns fall through
    // to the complete shared field below.
    if !source.climate_edge {
        let basis = ClimateSurfaceBasis {
            p: source,
            world_face,
            world_u,
            world_v,
            temp_c: unwarped_temp_c,
            surface_temp_c: unwarped_temp_c,
            precip_mm: unwarped_precip_mm,
            koppen_anchor: [0.0; 3],
            forest: planet.koppen_forest_signature(source),
        };
        if vegetation_forces_snow(planet, basis) {
            return VegetationSurface {
                main_block: MainBlock::Snow,
                forest: basis.forest,
                koppen: source.here,
                temp_c: unwarped_temp_c as f32,
                precip_mm_yr: unwarped_precip_mm as f32,
            };
        }
        return VegetationSurface {
            main_block: koppen_main_block(source.here),
            forest: basis.forest,
            koppen: source.here,
            temp_c: unwarped_temp_c as f32,
            precip_mm_yr: unwarped_precip_mm as f32,
        };
    }

    let sea = planet.true_sea_at(world_face, world_u, world_v);
    let basis = climate_surface_basis(
        planet,
        world_face,
        world_u,
        world_v,
        unwarped_temp_c,
        unwarped_precip_mm,
        sea,
    );
    if vegetation_forces_snow(planet, basis) {
        return VegetationSurface {
            main_block: MainBlock::Snow,
            forest: basis.forest,
            koppen: basis.p.here,
            temp_c: basis.temp_c as f32,
            precip_mm_yr: basis.precip_mm as f32,
        };
    }
    let biome = biome_climate_from_basis(basis, sea);
    VegetationSurface {
        main_block: climate_cross_block(planet, basis),
        forest: basis.forest,
        koppen: biome.koppen,
        temp_c: biome.temp_c,
        precip_mm_yr: biome.precip_mm_yr,
    }
}

/// Full local surface plus its biome coordinates from one climate-position
/// lookup. Voxel columns consume both, so evaluating them independently would
/// repeat the five-octave domain warp on every climate edge.
pub(crate) fn climate_surface_with_biome(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
) -> (ClimateSurface, BiomeClimate) {
    climate_surface_with_biome_at_season(
        planet,
        face,
        u,
        v,
        unwarped_temp_c,
        unwarped_temp_c,
        unwarped_precip_mm,
        unwarped_sea,
        crate::weather::StructuralSeason::annual(),
    )
}

pub(crate) fn climate_surface_with_biome_at_season(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_surface_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
    season: crate::weather::StructuralSeason,
) -> (ClimateSurface, BiomeClimate) {
    let basis = climate_surface_basis_at_season(
        planet,
        face,
        u,
        v,
        unwarped_temp_c,
        unwarped_surface_temp_c,
        unwarped_precip_mm,
        unwarped_sea,
        season,
    );
    let mut blocks = [climate_cross_block(planet, basis)];
    apply_snow_override(planet, basis, &mut blocks);
    let one_hot = match blocks[0] {
        MainBlock::Grass => [1.0, 0.0, 0.0],
        MainBlock::Sand => [0.0, 1.0, 0.0],
        MainBlock::Snow => [0.0, 0.0, 1.0],
    };
    (
        finish_climate_surface(basis, blocks[0], one_hot),
        biome_climate_from_basis(basis, unwarped_sea),
    )
}

/// One local truth for both ground renderers: domain-warped climate supplies
/// provisional tints, smoothly sampled Koppen nudges grass hue, and the shared
/// 8 km/ten-octave field chooses cross-block categories at every altitude.
/// The caller supplies unwarped physical-sea ownership solely as a guard;
/// elevation, weather, hydrology, and water geometry never warp.
pub fn climate_surface(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
) -> ClimateSurface {
    let basis = climate_surface_basis(
        planet,
        face,
        u,
        v,
        unwarped_temp_c,
        unwarped_precip_mm,
        unwarped_sea,
    );
    let mut blocks = [climate_cross_block(planet, basis)];
    apply_snow_override(planet, basis, &mut blocks);
    let one_hot = match blocks[0] {
        MainBlock::Grass => [1.0, 0.0, 0.0],
        MainBlock::Sand => [0.0, 1.0, 0.0],
        MainBlock::Snow => [0.0, 0.0, 1.0],
    };
    finish_climate_surface(basis, blocks[0], one_hot)
}

/// Local + range appearances from one raster/warp lookup and ONE categorical
/// field. Terrain vertices carry a one-hot exact appearance plus eight fixed
/// appearance-family endpoints and cumulative thresholds for fragment
/// patches. Candidate climates are categorical texel averages, so continuous
/// temperature/precipitation interpolation cannot restore the old tint wash.
/// Voxel callers consume the same exact boundary choice through the public
/// local API above.
pub(crate) fn climate_surface_pair(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
) -> (ClimateSurface, ClimateRangeSurface) {
    climate_surface_pair_at_season(
        planet,
        face,
        u,
        v,
        unwarped_temp_c,
        unwarped_temp_c,
        unwarped_precip_mm,
        unwarped_sea,
        crate::weather::StructuralSeason::annual(),
    )
}

pub(crate) fn climate_surface_pair_at_season(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    unwarped_temp_c: f64,
    unwarped_surface_temp_c: f64,
    unwarped_precip_mm: f64,
    unwarped_sea: bool,
    season: crate::weather::StructuralSeason,
) -> (ClimateSurface, ClimateRangeSurface) {
    let basis = climate_surface_basis_at_season(
        planet,
        face,
        u,
        v,
        unwarped_temp_c,
        unwarped_surface_temp_c,
        unwarped_precip_mm,
        unwarped_sea,
        season,
    );
    let (boundary, range_weights, range_thresholds, mut mean_weights) =
        climate_boundary_pair_selection(planet, basis);
    let mut blocks = [boundary];
    let force_snow = apply_snow_override(planet, basis, &mut blocks);
    if force_snow {
        mean_weights = [0.0, 0.0, 1.0];
    }
    let local_weights = match blocks[0] {
        MainBlock::Grass => [1.0, 0.0, 0.0],
        MainBlock::Sand => [0.0, 1.0, 0.0],
        MainBlock::Snow => [0.0, 0.0, 1.0],
    };
    (
        finish_climate_surface(basis, blocks[0], local_weights),
        ClimateRangeSurface {
            candidates: *range_candidate_palette(force_snow),
            weights: range_weights,
            mean: finish_climate_surface(basis, blocks[0], mean_weights),
            thresholds: range_thresholds,
        },
    )
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
    use crate::camera::Camera;
    use crate::terrain;

    /// Audit the primary round-4 specimen through the exact mesh climate path.
    /// Ignored because it loads the full baked planet and exists as an
    /// evidence probe rather than a unit gate. Run with:
    /// `cargo test --release --lib reported_9km_range_transect -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn reported_9km_range_transect() {
        let assets = if std::path::Path::new("assets/meta.json").exists() {
            "assets"
        } else {
            "viewer/assets"
        };
        let planet = Planet::load(assets).expect("range probe requires baked viewer assets");
        let camera = Camera {
            body: crate::orbits::BodyId::Neisor,
            center_km: DVec3::ZERO,
            lat: 52.991f64.to_radians(),
            lon: 100.016f64.to_radians(),
            altitude_km: 9.070,
            radius_km: planet.radius_km,
            ground_km: 0.003_321_985_203_788_121_3,
            yaw: -84.0f64.to_radians(),
            pitch: -83.0f64.to_radians(),
            roll: 0.0,
        };
        let eye = camera.position();
        let forward = camera.look_dir();
        let up_hint = camera.frame().0;
        let right = forward.cross(up_hint).normalize();
        let view_up = right.cross(forward).normalize();
        let tan_half_fov = (65.0f64.to_radians() * 0.5).tan();
        let aspect = 1280.0 / 720.0;
        let surface_dir_at_pixel = |pixel_x: f64, pixel_y: f64| {
            let ndc_x = 2.0 * (pixel_x + 0.5) / 1280.0 - 1.0;
            let ndc_y = 1.0 - 2.0 * (pixel_y + 0.5) / 720.0;
            let ray = (forward
                + right * (ndc_x * aspect * tan_half_fov)
                + view_up * (ndc_y * tan_half_fov))
                .normalize();
            let mut radius = planet.radius_km;
            let mut direction = eye.normalize();
            for _ in 0..4 {
                let b = eye.dot(ray);
                let c = eye.length_squared() - radius * radius;
                let disc = b * b - c;
                assert!(disc >= 0.0, "pixel ({pixel_x},{pixel_y}) misses planet");
                let distance = -b - disc.sqrt();
                assert!(distance > 0.0, "pixel ({pixel_x},{pixel_y}) hits behind eye");
                direction = (eye + ray * distance).normalize();
                radius = planet.radius_km + terrain::ground_height_km(&planet, direction, 1.0);
            }
            direction
        };

        println!("| # | pixel | lat | lon | Koppen | family coverages 0..7 | temp C | precip mm | path | beach coverage / gain | unlit RGB byte | active endpoint span |");
        println!("|---:|---:|---:|---:|---:|---|---:|---:|---|---:|---|---:|");
        for sample_index in 0..10 {
            let t = sample_index as f64 / 9.0;
            let pixel_x = 650.0 + (950.0 - 650.0) * t;
            let pixel_y = 120.0 + (500.0 - 120.0) * t;
            let dir = surface_dir_at_pixel(pixel_x, pixel_y);
            let lat = dir.z.asin().to_degrees();
            let lon = dir.y.atan2(dir.x).to_degrees();
            let (face, u, v) = face_from_dir(dir);
            let sampled = terrain::sample(&planet, face, u, v, terrain::VOXEL_OCTAVES);
            let basis = climate_surface_basis(
                &planet,
                face,
                u,
                v,
                sampled.temp_c,
                sampled.precip,
                sampled.sea,
            );
            let weights = planet.koppen_range_weights_at(basis.p, BIOME_BOUNDARY_ZONE_KM);
            let path = if !basis.p.range_edge { "interior fast" } else { "range_edge complete" };
            let coverages = weights
                .map(|weight| format!("{weight:.4}"))
                .join(" / ");
            let (ground, endpoints, beach) =
                terrain::debug_range_ground_payload(&planet, face, u, v, &sampled);
            let ground_bytes = ground.map(|channel| (channel * 255.0).round() as u8);
            let active: Vec<[u8; 3]> = endpoints
                .into_iter()
                .zip(weights)
                .enumerate()
                .filter(|(family, (_, weight))| *weight > 0.0 || (*family == 6 && beach[0] > 0))
                .map(|(_, (endpoint, _))| [endpoint[0], endpoint[1], endpoint[2]])
                .collect();
            let endpoint_span = (0..3)
                .map(|channel| {
                    let lo = active.iter().map(|color| color[channel]).min().unwrap();
                    let hi = active.iter().map(|color| color[channel]).max().unwrap();
                    hi - lo
                })
                .max()
                .unwrap();
            println!(
                "| {} | ({:.0},{:.0}) | {:.6} | {:.6} | {} | {} | {:.3} | {:.1} | {} | {:.4} / {:.4} | {:?} | {} |",
                sample_index + 1,
                pixel_x,
                pixel_y,
                lat,
                lon,
                basis.p.here,
                coverages,
                basis.temp_c,
                basis.precip_mm,
                path,
                beach[0] as f32 / 255.0,
                beach[1] as f32 / 255.0,
                ground_bytes,
                endpoint_span,
            );
        }
    }

    fn climate_test_planet_res(right_class: u8, res: usize) -> Planet {
        assert!(res >= 4 && res.is_multiple_of(2));
        let n = res * res;
        let koppen: Vec<u8> = (0..res)
            .flat_map(|_| {
                (0..res).map(|x| if x < res / 2 { 6 } else { right_class })
            })
            .collect();
        let face = || FaceRaster {
            res,
            elev_km: vec![0.2; n],
            koppen: koppen.clone(),
            climate_edge: climate_edge_mask(&koppen, res),
            range_edge: climate_range_edge_mask(&koppen, &vec![[8.0, 500.0]; n], res),
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
            weather: None,
            impostor_candidates: ImpostorCandidateCache::default(),
        }
    }

    fn climate_test_planet(right_class: u8) -> Planet {
        climate_test_planet_res(right_class, 4)
    }

    fn uncached_impostor_candidates(
        planet: &Planet,
        face: u8,
        ci_start: u64,
        ci_end: u64,
        cj_start: u64,
        cj_end: u64,
        stride: u64,
    ) -> Vec<ImpostorCandidate> {
        let nnf = COLUMNS_PER_FACE as f64;
        let comp = (stride * stride) as f64;
        let barren_interior = |koppen, forest| {
            forest <= 1e-4
                && !matches!(
                    crate::voxel::tree_kind_density(koppen),
                    Some((kind, _)) if kind != crate::voxel::TreeKind::Shrub
                )
        };
        let mut candidates = Vec::new();
        for ci in (ci_start..=ci_end).step_by(stride as usize) {
            for cj in (cj_start..=cj_end).step_by(stride as usize) {
                let lot = crate::voxel::tree_hash01(face, ci, cj, planet.seed);
                if lot >= crate::voxel::MAX_TREE_DENSITY * comp {
                    continue;
                }
                let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nnf;
                let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nnf;
                let interior = planet.vegetation_interior(face as usize, u, v);
                if interior.is_some_and(|(koppen, forest)| barren_interior(koppen, forest)) {
                    continue;
                }
                let (temp, precip) = planet.temp_precip(face as usize, u, v);
                let vegetation =
                    vegetation_surface(planet, face as usize, u, v, temp as f64, precip as f64);
                let Some((kind, density)) = crate::voxel::tree_biome_profile(
                    vegetation.koppen,
                    vegetation.main_block,
                    vegetation.forest,
                    vegetation.temp_c,
                    vegetation.precip_mm_yr,
                ) else {
                    continue;
                };
                if kind == crate::voxel::TreeKind::Shrub || lot >= density * comp {
                    continue;
                }
                candidates.push((ci, cj, kind, lot, density));
            }
        }
        candidates
    }

    fn assert_impostor_candidates_bits_eq(
        actual: &[ImpostorCandidate],
        expected: &[ImpostorCandidate],
    ) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(
                (actual.0, actual.1, actual.2),
                (expected.0, expected.1, expected.2)
            );
            assert_eq!(actual.3.to_bits(), expected.3.to_bits());
            assert_eq!(actual.4.to_bits(), expected.4.to_bits());
        }
    }

    fn uncached_rock_impostor_candidates(
        planet: &Planet,
        face: u8,
        ci_start: u64,
        ci_end: u64,
        cj_start: u64,
        cj_end: u64,
        stride: u64,
    ) -> Vec<RockImpostorCandidate> {
        let nnf = COLUMNS_PER_FACE as f64;
        let comp = (stride * stride) as f64;
        let mut candidates = Vec::new();
        let no_edits = crate::voxel::Edits::default();
        for ci in (ci_start..=ci_end).step_by(stride as usize) {
            for cj in (cj_start..=cj_end).step_by(stride as usize) {
                let lot = crate::voxel::rock_hash01(face, ci, cj, planet.seed);
                if lot >= crate::voxel::rock_tuning::MAX_DENSITY * comp {
                    continue;
                }
                let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nnf;
                let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nnf;
                let elevation = planet.elevation(face as usize, u, v);
                if sea_from_fields(
                    f64::from(elevation),
                    f64::from(planet.water_frac(face as usize, u, v)),
                    f64::from(planet.ocean(face as usize, u, v)),
                ) {
                    continue;
                }
                let (temp, precip) = planet.temp_precip(face as usize, u, v);
                let geology = vegetation_surface(
                    planet,
                    face as usize,
                    u,
                    v,
                    f64::from(temp),
                    f64::from(precip),
                );
                let (family, density) = crate::voxel::neisor_rock_profile(
                    geology.main_block,
                    geology.forest,
                    elevation,
                    planet.rough(face as usize, u, v),
                );
                if lot >= density * comp {
                    continue;
                }
                let column = crate::voxel::col_ctx(
                    planet,
                    &no_edits,
                    usize::from(face),
                    ci,
                    cj,
                );
                let placement = crate::voxel::rock_surface_eligible(&column).then_some(
                    RockPlacement {
                        height: f64::from(column.h_km),
                        albedo: 0.0,
                    },
                );
                candidates.push((
                    ci,
                    cj,
                    crate::voxel::rock_kind(family, face, ci, cj, planet.seed),
                    family,
                    lot,
                    density,
                    placement,
                ));
            }
        }
        candidates
    }

    fn assert_rock_candidates_bits_eq(
        actual: &[RockImpostorCandidate],
        expected: &[RockImpostorCandidate],
    ) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(
                (actual.0, actual.1, actual.2, actual.3),
                (expected.0, expected.1, expected.2, expected.3)
            );
            assert_eq!(actual.4.to_bits(), expected.4.to_bits());
            assert_eq!(actual.5.to_bits(), expected.5.to_bits());
            match (actual.6, expected.6) {
                (Some(actual), Some(expected)) => {
                    assert_eq!(actual.height.to_bits(), expected.height.to_bits());
                    assert_eq!(actual.albedo.to_bits(), expected.albedo.to_bits());
                }
                (None, None) => {}
                _ => panic!("rock placement eligibility differs"),
            }
        }
    }

    fn assert_surface_bits_eq(left: ClimateSurface, right: ClimateSurface) {
        assert_eq!(left.main_block, right.main_block);
        for (label, left, right) in [
            ("grass", left.grass, right.grass),
            ("sand", left.sand, right.sand),
            ("snow", left.snow, right.snow),
            ("weights", left.block_weights, right.block_weights),
        ] {
            assert_eq!(
                left.map(f32::to_bits),
                right.map(f32::to_bits),
                "{label} differs"
            );
        }
        assert_eq!(left.forest.to_bits(), right.forest.to_bits());
    }

    #[test]
    fn range_edge_mask_detects_tint_boundaries_inside_one_koppen_class() {
        let res = 32;
        let n = res * res;
        let koppen = vec![25; n]; // Dfb on both sides: class mask sees no edge.
        let climate: Vec<[f32; 2]> = (0..res)
            .flat_map(|_| {
                (0..res).map(|x| {
                    if x < res / 2 { [8.0, 700.0] } else { [8.0, 1_300.0] }
                })
            })
            .collect();
        assert_ne!(
            climate_range_family(25, [8.0, 700.0]),
            climate_range_family(25, [8.0, 1_300.0]),
        );
        let class_edge = climate_edge_mask(&koppen, res);
        let range_edge = climate_range_edge_mask(&koppen, &climate, res);
        let marked = |mask: &[u64], x: usize, y: usize| {
            let index = y * res + x;
            (mask[index >> 6] >> (index & 63)) & 1 != 0
        };
        assert!(!marked(&class_edge, res / 2 - 1, res / 2));
        assert!(marked(&range_edge, res / 2 - 1, res / 2));
        assert!(!marked(&range_edge, res / 4, res / 2));
    }

    #[test]
    fn range_family_weights_are_normalized_and_thresholds_monotone() {
        let planet = climate_test_planet_res(4, 32);
        for (u, v) in [(-0.60, 0.12), (-0.08, -0.31), (0.0, 0.0), (0.08, 0.27), (0.60, -0.19)] {
            let p = planet.raster_position(0, u, v);
            let weights = planet.koppen_range_weights_at(p, BIOME_BOUNDARY_ZONE_KM);
            assert!(weights.iter().all(|weight| *weight >= 0.0));
            let sum = weights.iter().sum::<f32>();
            assert!((sum - 1.0).abs() <= 2e-6, "weights sum to {sum} at {u},{v}");
            let mut cumulative = 0.0;
            let mut previous = 0.0;
            for weight in weights {
                cumulative += weight;
                assert!(cumulative + 1e-7 >= previous);
                previous = cumulative;
            }
            assert!((cumulative - 1.0).abs() <= 2e-6);
        }
    }

    #[test]
    fn one_family_range_mean_preserves_the_exact_local_appearance() {
        let planet = climate_test_planet_res(6, 32);
        for (temp, precip) in [(8.0, 500.0), (-20.0, 300.0)] {
            let (local, range) =
                climate_surface_pair(&planet, 0, -0.35, 0.20, temp, precip, false);
            assert_surface_bits_eq(local, range.mean);
        }
    }

    #[test]
    fn public_local_surface_matches_pair_local_bit_for_bit() {
        for right_class in [4, 29] {
            let planet = climate_test_planet_res(right_class, 8);
            for (face, u, v, temp_c, precip_mm, sea) in [
                (0, -0.80, 0.20, 17.0, 800.0, false),
                (3, 0.03, -0.17, -4.0, 300.0, false),
                (5, 0.40, 0.50, -20.0, 100.0, true),
            ] {
                let public = climate_surface(
                    &planet, face, u, v, temp_c, precip_mm, sea,
                );
                let (paired_local, _) = climate_surface_pair(
                    &planet, face, u, v, temp_c, precip_mm, sea,
                );
                assert_surface_bits_eq(public, paired_local);
            }
        }
    }

    #[test]
    fn optimized_vegetation_and_voxel_views_match_full_field_bit_for_bit() {
        for right_class in [4, 13, 28, 29] {
            let planet = climate_test_planet_res(right_class, 32);
            for (face, u, v) in [
                (0, -0.80, 0.20),
                (0, -0.20, -0.37),
                (3, -0.01, 0.17),
                (3, 0.01, -0.17),
                (5, 0.40, 0.50),
                (5, 0.92, -0.73),
            ] {
                let (temp, precip) = planet.temp_precip(face, u, v);
                let biome = planet.biome_climate(face, u, v);
                let full = climate_surface(
                    &planet,
                    face,
                    u,
                    v,
                    temp as f64,
                    precip as f64,
                    biome.sea,
                );
                let vegetation = vegetation_surface(
                    &planet,
                    face,
                    u,
                    v,
                    temp as f64,
                    precip as f64,
                );
                assert_eq!(vegetation.main_block, full.main_block);
                assert_eq!(vegetation.forest.to_bits(), full.forest.to_bits());
                assert_eq!(vegetation.koppen, biome.koppen);
                assert_eq!(vegetation.temp_c.to_bits(), biome.temp_c.to_bits());
                assert_eq!(
                    vegetation.precip_mm_yr.to_bits(),
                    biome.precip_mm_yr.to_bits()
                );

                let (combined_surface, combined_biome) = climate_surface_with_biome(
                    &planet,
                    face,
                    u,
                    v,
                    temp as f64,
                    precip as f64,
                    biome.sea,
                );
                assert_surface_bits_eq(combined_surface, full);
                assert_eq!(combined_biome.koppen, biome.koppen);
                assert_eq!(combined_biome.temp_c.to_bits(), biome.temp_c.to_bits());
                assert_eq!(
                    combined_biome.precip_mm_yr.to_bits(),
                    biome.precip_mm_yr.to_bits()
                );
                assert_eq!(combined_biome.sea, biome.sea);
            }
        }
    }

    #[test]
    fn vegetation_snow_hoist_matches_full_surface_across_thresholds() {
        for temp in [-12.0, -10.4, -9.0, -7.6, -7.0] {
            let mut planet = climate_test_planet_res(13, 64);
            for face in &mut planet.faces {
                face.climate.fill([temp, 500.0]);
            }
            for (face, u, v) in [
                (0, -0.60, 0.20),
                (0, -0.01, -0.17),
                (0, 0.01, 0.17),
                (3, 0.60, -0.20),
            ] {
                let (sample_temp, precip) = planet.temp_precip(face, u, v);
                let biome = planet.biome_climate(face, u, v);
                let full = climate_surface(
                    &planet,
                    face,
                    u,
                    v,
                    sample_temp as f64,
                    precip as f64,
                    biome.sea,
                );
                let vegetation = vegetation_surface(
                    &planet,
                    face,
                    u,
                    v,
                    sample_temp as f64,
                    precip as f64,
                );
                assert_eq!(vegetation.main_block, full.main_block);
                assert_eq!(vegetation.forest.to_bits(), full.forest.to_bits());
                assert_eq!(vegetation.koppen, biome.koppen);
                assert_eq!(vegetation.temp_c.to_bits(), biome.temp_c.to_bits());
                assert_eq!(
                    vegetation.precip_mm_yr.to_bits(),
                    biome.precip_mm_yr.to_bits()
                );
            }
        }
    }

    #[test]
    fn vegetation_region_proof_is_conservative_at_climate_edges() {
        let planet = climate_test_planet_res(13, 128);
        let barren = |koppen, forest| {
            forest <= 1e-4
                && !matches!(
                    crate::voxel::tree_kind_density(koppen),
                    Some((kind, _)) if kind != crate::voxel::TreeKind::Shrub
                )
        };
        assert!(planet.vegetation_region_all_interior(
            0, -0.6, -0.4, -0.2, 0.2, barren
        ));
        assert!(planet.vegetation_region_all_interior(
            0, -0.4, -0.6, 0.2, -0.2, barren
        ));
        assert!(!planet.vegetation_region_all_interior(
            0, 0.4, 0.6, -0.2, 0.2, barren
        ));
        assert!(!planet.vegetation_region_all_interior(
            0, -0.2, 0.2, -0.2, 0.2, barren
        ));

        let mut cold = climate_test_planet_res(13, 128);
        for face in &mut cold.faces {
            face.climate.fill([-20.0, 500.0]);
        }
        assert!(cold.vegetation_region_always_snow(0, 0.4, 0.6, -0.2, 0.2));
        assert!(cold.vegetation_region_always_snow(0, -0.2, 0.2, -0.2, 0.2));
        assert!(!cold.vegetation_region_always_snow(0, -0.999, -0.98, -0.2, 0.2));
        for face in &mut cold.faces {
            face.climate.fill([8.0, 500.0]);
        }
        assert!(!cold.vegetation_region_always_snow(0, 0.4, 0.6, -0.2, 0.2));
    }

    #[test]
    fn impostor_candidate_cache_matches_uncached_stream_across_strides() {
        let planet = climate_test_planet_res(13, 128);
        let (ci0, ci1) = (4_999_680, 5_000_191);
        let (cj0, cj1) = (5_123_200, 5_123_711);
        let strides = [8, 4, 2, 1];
        let expected: Vec<Vec<ImpostorCandidate>> = strides
            .iter()
            .map(|&stride| uncached_impostor_candidates(&planet, 0, ci0, ci1, cj0, cj1, stride))
            .collect();
        for (&stride, expected) in strides.iter().zip(&expected) {
            let miss = planet.impostor_candidates(0, ci0, ci1, cj0, cj1, stride);
            assert_impostor_candidates_bits_eq(&miss, expected);
            let hit = planet.impostor_candidates(0, ci0, ci1, cj0, cj1, stride);
            assert_impostor_candidates_bits_eq(&hit, expected);
        }

        // A fresh cache may receive overlapping sparse misses from different
        // Rayon workers. Climate work happens outside the profile lock and
        // duplicate pure results collapse during merge; every caller still
        // observes the same ordered stream.
        use rayon::prelude::*;
        let concurrent = climate_test_planet_res(13, 128);
        let actual: Vec<Vec<ImpostorCandidate>> = strides
            .par_iter()
            .map(|&stride| concurrent.impostor_candidates(0, ci0, ci1, cj0, cj1, stride))
            .collect();
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_impostor_candidates_bits_eq(actual, expected);
        }
    }

    #[test]
    fn rock_candidate_cache_matches_uncached_stream_across_strides() {
        let planet = climate_test_planet_res(13, 128);
        let (ci0, ci1) = (4_999_680, 5_000_191);
        let (cj0, cj1) = (5_123_200, 5_123_711);
        let strides = [8, 4, 2, 1];
        let expected: Vec<Vec<RockImpostorCandidate>> = strides
            .iter()
            .map(|&stride| {
                uncached_rock_impostor_candidates(
                    &planet, 0, ci0, ci1, cj0, cj1, stride,
                )
            })
            .collect();
        for (&stride, expected) in strides.iter().zip(&expected) {
            let miss = planet.rock_impostor_candidates(0, ci0, ci1, cj0, cj1, stride);
            assert_rock_candidates_bits_eq(&miss, expected);
            let hit = planet.rock_impostor_candidates(0, ci0, ci1, cj0, cj1, stride);
            assert_rock_candidates_bits_eq(&hit, expected);
        }

        use rayon::prelude::*;
        let concurrent = climate_test_planet_res(13, 128);
        let actual: Vec<Vec<RockImpostorCandidate>> = strides
            .par_iter()
            .map(|&stride| {
                concurrent.rock_impostor_candidates(0, ci0, ci1, cj0, cj1, stride)
            })
            .collect();
        for (actual, expected) in actual.iter().zip(&expected) {
            assert_rock_candidates_bits_eq(actual, expected);
        }
    }

    #[test]
    fn rock_cache_requests_cannot_change_tree_candidate_bits() {
        let planet = climate_test_planet_res(13, 128);
        let (ci0, ci1) = (4_999_680, 5_000_191);
        let (cj0, cj1) = (5_123_200, 5_123_711);
        let expected = uncached_impostor_candidates(&planet, 0, ci0, ci1, cj0, cj1, 2);
        let _ = planet.rock_impostor_candidates(0, ci0, ci1, cj0, cj1, 8);
        let _ = planet.rock_impostor_candidates(0, ci0, ci1, cj0, cj1, 1);
        let actual = planet.impostor_candidates(0, ci0, ci1, cj0, cj1, 2);
        assert_impostor_candidates_bits_eq(&actual, &expected);
    }

    #[test]
    fn rock_candidates_keep_exact_kind_through_voxel_late_gates() {
        let planet = climate_test_planet_res(13, 128);
        let (ci0, ci1) = (4_999_680, 5_000_191);
        let (cj0, cj1) = (5_123_200, 5_123_711);
        let candidates = planet.rock_impostor_candidates(0, ci0, ci1, cj0, cj1, 1);
        assert!(!candidates.is_empty());
        let edits = crate::voxel::Edits::default();
        let mut survived = 0usize;
        for (ci, cj, kind, family, _, _, placement) in candidates {
            let column = crate::voxel::col_ctx(&planet, &edits, 0, ci, cj);
            if let Some(exact) = crate::voxel::rock_at(&column, 0, ci, cj, planet.seed) {
                assert_eq!(exact, (kind, family));
                assert!(placement.is_some());
                survived += 1;
            } else {
                assert!(placement.is_none());
            }
        }
        assert!(survived > 0, "fixture stopped exercising exact rock survivors");
    }

    #[test]
    fn impostor_cache_preserves_pre_decimation_treeline_candidates() {
        let (ci0, ci1) = (5_200_000, 5_200_255);
        let (cj0, cj1) = (5_300_000, 5_300_255);
        let stride = 4;
        for temp in [-6.1_f32, -6.0, -5.9] {
            let mut planet = climate_test_planet_res(13, 128);
            for face in &mut planet.faces {
                face.climate.fill([temp, 500.0]);
            }
            let old_phase = uncached_impostor_candidates(&planet, 0, ci0, ci1, cj0, cj1, stride);
            let cached = planet.impostor_candidates(0, ci0, ci1, cj0, cj1, stride);
            assert_impostor_candidates_bits_eq(&cached, &old_phase);
            if temp < crate::voxel::TREE_MIN_TEMP_C as f32 {
                assert!(!old_phase.is_empty());
                assert!(planet.vegetation_region_below_treeline(
                    0,
                    -1.0 + 2.0 * (ci0 as f64 + 0.5) / COLUMNS_PER_FACE as f64,
                    -1.0 + 2.0 * (ci1 as f64 + 0.5) / COLUMNS_PER_FACE as f64,
                    -1.0 + 2.0 * (cj0 as f64 + 0.5) / COLUMNS_PER_FACE as f64,
                    -1.0 + 2.0 * (cj1 as f64 + 0.5) / COLUMNS_PER_FACE as f64,
                ));
            } else {
                assert!(!cached.is_empty());
            }
        }
    }

    #[test]
    fn impostor_whole_tile_proof_combines_phase_and_late_rejections() {
        let res = 128;
        let mut planet = climate_test_planet_res(13, res);
        for face in &mut planet.faces {
            for y in 0..res {
                for x in 0..res {
                    // The warm left side is shrub-only. Make the climate cold
                    // well before the forest/class edge so every ambiguous
                    // edge lookup is below the late non-shrub treeline.
                    face.climate[y * res + x][0] = if x < res / 2 - 12 { 8.0 } else { -20.0 };
                }
            }
        }
        let to_col = |uv: f64| {
            (((uv + 1.0) * 0.5 * COLUMNS_PER_FACE as f64)
                .floor()
                .clamp(0.0, COLUMNS_PER_FACE as f64 - 1.0)) as u64
        };
        let (ci0, ci1) = (to_col(-0.8), to_col(0.8));
        let (cj0, cj1) = (to_col(-0.4), to_col(0.4));
        assert!(!planet.impostor_candidate_bounds_phase_reject_all(0, ci0, ci1, cj0, cj1,));
        assert!(!planet.vegetation_region_below_treeline(0, -0.8, 0.8, -0.4, 0.4));
        assert!(planet.impostor_tile_emits_none(0, ci0, ci1, cj0, cj1));

        for face in &mut planet.faces {
            face.climate.fill([8.0, 500.0]);
        }
        assert!(!planet.impostor_tile_emits_none(0, ci0, ci1, cj0, cj1));
    }

    #[test]
    fn impostor_candidate_cache_enforces_entry_and_byte_bounds() {
        let cache = ImpostorCandidateCache::default();
        let target_shard = 0;
        let mut inserted = 0;
        'keys: for ri in 0..u16::MAX {
            for rj in 0..u16::MAX {
                let key = ImpostorCandidateRegionKey { face: 0, ri, rj };
                if ImpostorCandidateCache::shard_index(key) != target_shard {
                    continue;
                }
                drop(cache.entry(key));
                inserted += 1;
                if inserted > IMPOSTOR_CANDIDATE_CACHE_ENTRIES_PER_SHARD + 32 {
                    break 'keys;
                }
            }
        }
        cache.trim();
        assert!(
            cache.lock_shard(target_shard).entries.len()
                <= IMPOSTOR_CANDIDATE_CACHE_ENTRIES_PER_SHARD
        );

        let heavy_key = (0..u16::MAX)
            .flat_map(|ri| (0..32).map(move |rj| ImpostorCandidateRegionKey { face: 1, ri, rj }))
            .find(|&key| ImpostorCandidateCache::shard_index(key) == target_shard)
            .unwrap();
        let heavy = cache.entry(heavy_key);
        cache.record_bytes(
            heavy_key,
            &heavy,
            IMPOSTOR_CANDIDATE_CACHE_BYTES_PER_SHARD + 1,
        );
        drop(heavy);
        cache.trim();
        let shard = cache.lock_shard(target_shard);
        assert!(shard.resident_bytes <= IMPOSTOR_CANDIDATE_CACHE_BYTES_PER_SHARD);
        assert!(!shard.entries.contains_key(&heavy_key));
    }

    #[test]
    fn impostor_candidate_partitions_and_sites_are_exact_at_both_face_edges() {
        let partitions = IMPOSTOR_CANDIDATE_REGIONS_PER_FACE;
        let mut next = 0u64;
        for region in 0..partitions {
            let region = u16::try_from(region).unwrap();
            let (start, end) = Planet::impostor_candidate_region_bounds(region);
            assert_eq!(start, next);
            assert!(start <= end && end < COLUMNS_PER_FACE);
            assert_eq!(Planet::impostor_candidate_region_index(start), region);
            assert_eq!(Planet::impostor_candidate_region_index(end), region);

            let first = Planet::impostor_candidate_site(start, start, start, start).unwrap();
            assert_eq!(first, 0);
            let last = Planet::impostor_candidate_site(start, start, end, end).unwrap();
            assert_eq!(u64::from(last >> 16), end - start);
            assert_eq!(u64::from(last & 0xffff), end - start);
            next = end + 1;
        }
        assert_eq!(next, COLUMNS_PER_FACE);
        assert_eq!(Planet::impostor_candidate_region_index(0), 0);
        assert_eq!(
            Planet::impostor_candidate_region_index(COLUMNS_PER_FACE - 1),
            u16::try_from(partitions - 1).unwrap()
        );

        // Checked packing refuses an invalid offset instead of truncating it.
        assert!(Planet::impostor_candidate_site(0, 0, u64::from(u16::MAX) + 1, 0).is_none());
        assert!(Planet::impostor_candidate_site(0, 0, 0, u64::from(u16::MAX) + 1).is_none());
    }

    #[test]
    fn impostor_candidate_cache_matches_direct_stream_at_every_face_edge() {
        let planet = climate_test_planet_res(13, 128);
        let edge = COLUMNS_PER_FACE - 128;
        let ranges = [
            (0, 127, 0, 127),
            (0, 127, edge, COLUMNS_PER_FACE - 1),
            (edge, COLUMNS_PER_FACE - 1, 0, 127),
            (edge, COLUMNS_PER_FACE - 1, edge, COLUMNS_PER_FACE - 1),
        ];
        for face in 0..6 {
            for &(ci0, ci1, cj0, cj1) in &ranges {
                for stride in IMPOSTOR_CANDIDATE_STRIDES {
                    let expected =
                        uncached_impostor_candidates(&planet, face, ci0, ci1, cj0, cj1, stride);
                    let actual = planet.impostor_candidates(face, ci0, ci1, cj0, cj1, stride);
                    assert_impostor_candidates_bits_eq(&actual, &expected);
                }
            }
        }
    }

    #[test]
    fn rock_candidate_cache_matches_direct_stream_at_every_face_edge() {
        let planet = climate_test_planet_res(13, 128);
        let edge = COLUMNS_PER_FACE - 128;
        let ranges = [
            (0, 127, 0, 127),
            (0, 127, edge, COLUMNS_PER_FACE - 1),
            (edge, COLUMNS_PER_FACE - 1, 0, 127),
            (edge, COLUMNS_PER_FACE - 1, edge, COLUMNS_PER_FACE - 1),
        ];
        for face in 0..6 {
            for &(ci0, ci1, cj0, cj1) in &ranges {
                for stride in IMPOSTOR_CANDIDATE_STRIDES {
                    let expected = uncached_rock_impostor_candidates(
                        &planet, face, ci0, ci1, cj0, cj1, stride,
                    );
                    let actual =
                        planet.rock_impostor_candidates(face, ci0, ci1, cj0, cj1, stride);
                    assert_rock_candidates_bits_eq(&actual, &expected);
                }
            }
        }
    }

    #[test]
    fn impostor_candidate_entry_points_reject_malformed_bounds_without_panicking() {
        let planet = climate_test_planet_res(13, 128);
        for args in [
            (6, 0, 0, 0, 0, 1),
            (0, 2, 1, 0, 0, 1),
            (0, 0, 0, 2, 1, 1),
            (0, 0, COLUMNS_PER_FACE, 0, 0, 1),
            (0, 0, 0, 0, COLUMNS_PER_FACE, 1),
            (0, 0, 0, 0, 0, 0),
            (0, 0, 0, 0, 0, 3),
        ] {
            assert!(
                planet
                    .impostor_candidates(args.0, args.1, args.2, args.3, args.4, args.5)
                    .is_empty()
            );
        }
        assert!(!planet.impostor_tile_emits_none(6, 0, 0, 0, 0));
        assert!(!planet.impostor_tile_emits_none(0, 1, 0, 0, 0));
        assert!(!planet.impostor_tile_emits_none(0, 0, COLUMNS_PER_FACE, 0, 0));
    }

    #[test]
    fn rock_candidate_entry_points_reject_malformed_bounds_without_panicking() {
        let planet = climate_test_planet_res(13, 128);
        for args in [
            (6, 0, 0, 0, 0, 1),
            (0, 2, 1, 0, 0, 1),
            (0, 0, 0, 2, 1, 1),
            (0, 0, COLUMNS_PER_FACE, 0, 0, 1),
            (0, 0, 0, 0, COLUMNS_PER_FACE, 1),
            (0, 0, 0, 0, 0, 0),
            (0, 0, 0, 0, 0, 3),
        ] {
            assert!(
                planet
                    .rock_impostor_candidates(
                        args.0, args.1, args.2, args.3, args.4, args.5,
                    )
                    .is_empty()
            );
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
    fn biome_warp_spans_intermediate_flight_to_local_scales() {
        // Art-direction gate for Neisor's production bake. Octave two remains
        // visible at intermediate flight range; the finest octave now reaches
        // inside the 300 m local ecotone instead of ending kilometres above it.
        const NEISOR_RADIUS_KM: f64 = 8_660.254_037_844_386;
        const BAKE_RES: f64 = 1_024.0;
        let texel_km = NEISOR_RADIUS_KM * 2.0 / (BAKE_RES - 1.0);
        let scale = |octave: i32| {
            (
                texel_km * BIOME_WARP_BASE_WAVELENGTH_TEXELS
                    / BIOME_WARP_LACUNARITY.powi(octave),
                texel_km * BIOME_WARP_BASE_AMPLITUDE_TEXELS
                    * BIOME_WARP_PERSISTENCE.powi(octave),
            )
        };
        let (wavelength_km, amplitude_km) = scale(2);
        assert!(
            (5.0..=8.0).contains(&wavelength_km),
            "intermediate warp wavelength is {wavelength_km:.3} km"
        );
        assert!(
            (1.0..=2.0).contains(&amplitude_km),
            "intermediate warp amplitude is {amplitude_km:.3} km"
        );
        let (wavelength_km, amplitude_km) = scale((BIOME_WARP_OCTAVES - 1) as i32);
        assert!((0.35..=0.70).contains(&wavelength_km));
        assert!((0.10..=0.30).contains(&amplitude_km));
    }

    #[test]
    fn biome_warp_scalar_and_vector_noise_share_exact_carriers() {
        assert!(BIOME_BOUNDARY_FIELD_OCTAVES >= BIOME_WARP_OCTAVES);
        let seed = 42;
        let res = 1_024;
        let texel_angle = 2.0 / (res - 1) as f64;
        for sample in 0..512u64 {
            let face = sample as usize % FACES.len();
            let ci = hash_u64(face as u8, sample, sample.wrapping_mul(17), 0xA11C_E101)
                % COLUMNS_PER_FACE;
            let cj = hash_u64(face as u8, sample, sample.wrapping_mul(31), 0xA11C_E102)
                % COLUMNS_PER_FACE;
            let n = COLUMNS_PER_FACE as f64;
            let dir = ecotone_column_dir(
                face,
                -1.0 + 2.0 * (ci as f64 + 0.5) / n,
                -1.0 + 2.0 * (cj as f64 + 0.5) / n,
            );
            let mut frequency = 1.0 / (texel_angle * BIOME_WARP_BASE_WAVELENGTH_TEXELS);
            for octave in 0..BIOME_WARP_OCTAVES as usize {
                let shared = biome_scalar_noise(dir, frequency, seed, octave);
                let carrier = biome_vector_noise(dir, frequency, seed, octave);
                let axis = BIOME_WARP_AXES[octave % BIOME_WARP_AXES.len()][1];
                let recovered = carrier.dot(axis) / (1.15 * axis.length_squared());
                assert!((shared - recovered).abs() < 1e-12);
                frequency *= BIOME_WARP_LACUNARITY;
            }
        }
    }

    #[test]
    fn range_boundary_zone_is_wide_symmetric_and_area_neutral() {
        let mut cross = climate_test_planet_res(4, 8);
        cross.radius_km = 8_660.254_037_844_386;
        let weights_at_x = |x: f64| {
            let u = -1.0 + 2.0 * x / 7.0;
            cross
                .koppen_block_weights_at(
                    cross.raster_position(0, u, 0.0),
                    BIOME_BOUNDARY_ZONE_KM,
                )
                .1[1]
        };

        // The 4x4 support extends the x=3.5 transition across its two adjacent
        // texels, with pure ownership beyond them.
        assert!(weights_at_x(2.0) < 1e-12);
        assert!((weights_at_x(3.5) - 0.5).abs() < 1e-12);
        assert!(1.0 - weights_at_x(5.0) < 1e-12);

        // Both the smooth ramp and categorical prefilter are antisymmetric
        // around the old edge, so neither biases expected biome area.
        for i in 0..=64 {
            let dx = 1.5 * i as f64 / 64.0;
            assert!(
                (weights_at_x(3.5 - dx) + weights_at_x(3.5 + dx) - 1.0).abs()
                    < 1e-12
            );
        }
    }

    #[test]
    fn near_and_range_surfaces_keep_one_boundary_silhouette() {
        let cross = climate_test_planet_res(4, 8);
        for j in 0..128 {
            for i in 0..128 {
                let u = -0.75 + 1.5 * (i as f64 + 0.5) / 128.0;
                let v = -0.75 + 1.5 * (j as f64 + 0.5) / 128.0;
                let (near, range) =
                    climate_surface_pair(&cross, 0, u, v, 8.0, 900.0, false);
                assert!(
                    range.candidates.into_iter().zip(range.weights)
                        .any(|(candidate, weight)| weight > 0.0
                            && candidate.main_block == near.main_block),
                    "exact material disappeared from range candidates at u={u} v={v}"
                );
            }
        }
    }

    #[test]
    fn pond_dossier_keeps_its_approved_grass_surface() {
        let assets = if std::path::Path::new("assets/meta.json").exists() {
            "assets"
        } else {
            "viewer/assets"
        };
        let planet = Planet::load(assets).expect("dossier gate requires baked viewer assets");
        let (lat, lon) = (12.199f64.to_radians(), -44.827f64.to_radians());
        let dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        let (face, u, v) = face_from_dir(dir);
        let sample = terrain::sample(&planet, face, u, v, terrain::VOXEL_OCTAVES);
        let basis = climate_surface_basis(
            &planet,
            face,
            u,
            v,
            sample.temp_c,
            sample.precip,
            sample.sea,
        );
        let (_, weights, _) =
            planet.koppen_block_weights_at(basis.p, BIOME_BOUNDARY_ZONE_KM);
        let comparator = boundary_zone_comparator(
            basis.world_face,
            basis.world_u,
            basis.world_v,
            planet.faces[basis.p.face].res,
            planet.seed,
        );
        let surface = climate_surface(
            &planet,
            face,
            u,
            v,
            sample.temp_c,
            sample.precip,
            sample.sea,
        );
        assert_eq!(
            surface.main_block,
            MainBlock::Grass,
            "dossier changed: weights={weights:?} comparator={comparator}"
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
    fn categorical_comparators_preserve_area_fractions() {
        // Sample canonical columns over all faces and both categorical seed
        // streams. If the probability integral transform is doing its job,
        // threshold occupancy equals the requested blend weight.
        const SAMPLES_PER_SEED: usize = 65_536;
        const FRACTIONS: [f64; 3] = [0.25, 0.50, 0.75];
        const TOLERANCE: f64 = 0.008;
        let local_seeds = [
            42i64.wrapping_add(ECOTONE_FIELD_SEED_OFFSET),
            42i64.wrapping_add(SNOW_FIELD_SEED_OFFSET),
        ];
        let boundary_seeds = [42i64];
        for (label, field, seeds) in [
            (
                "local",
                ecotone_comparator as fn(usize, f64, f64, i64) -> f64,
                local_seeds.as_slice(),
            ),
            (
                "boundary",
                production_boundary_zone_comparator,
                boundary_seeds.as_slice(),
            ),
            (
                "range",
                production_boundary_range_comparator,
                boundary_seeds.as_slice(),
            ),
        ] {
            for &seed in seeds {
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
                    let comparator = field(face, u, v, seed);
                    for (count, fraction) in occupied.iter_mut().zip(FRACTIONS) {
                        *count += usize::from(comparator < fraction);
                    }
                }
                for (count, expected) in occupied.into_iter().zip(FRACTIONS) {
                    let measured = count as f64 / SAMPLES_PER_SEED as f64;
                    eprintln!(
                        "{label} occupancy seed={seed} blend={expected:.2} measured={measured:.5}"
                    );
                    assert!(
                        (measured - expected).abs() <= TOLERANCE,
                        "{label} seed {seed}: occupancy {measured:.5} at blend {expected:.2} exceeds {TOLERANCE:.3} tolerance"
                    );
                }
            }
        }
    }

    #[test]
    fn range_prefix_is_deterministic_and_tracks_the_full_boundary_field() {
        const SAMPLES: usize = 32_768;
        let seed = 42;
        let mut same_half = 0usize;
        for sample in 0..SAMPLES {
            let face = sample % FACES.len();
            let a = hash_u64(
                face as u8,
                sample as u64,
                (sample as u64).wrapping_mul(0x9E37_79B9),
                0xA11C_E101,
            );
            let b = hash_u64(
                face as u8,
                a,
                (sample as u64).wrapping_mul(0x85EB_CA77),
                0xA11C_E102,
            );
            let n = COLUMNS_PER_FACE as f64;
            let u = -1.0 + 2.0 * ((a % COLUMNS_PER_FACE) as f64 + 0.5) / n;
            let v = -1.0 + 2.0 * ((b % COLUMNS_PER_FACE) as f64 + 0.5) / n;
            let range = production_boundary_range_comparator(face, u, v, seed);
            let full = production_boundary_zone_comparator(face, u, v, seed);
            assert_eq!(
                range.to_bits(),
                production_boundary_range_comparator(face, u, v, seed).to_bits(),
            );
            same_half += usize::from((range < 0.5) == (full < 0.5));
        }
        let agreement = same_half as f64 / SAMPLES as f64;
        assert!(
            agreement >= 0.82,
            "resolved prefix/full category agreement fell to {agreement:.5}"
        );
    }

    #[test]
    fn categorical_comparators_are_one_value_per_canonical_column() {
        let n = COLUMNS_PER_FACE as f64;
        let (ci, cj) = (4_321_987u64, 7_654_321u64);
        let u = -1.0 + 2.0 * (ci as f64 + 0.5) / n;
        let v = -1.0 + 2.0 * (cj as f64 + 0.5) / n;
        let seed = 42i64.wrapping_add(ECOTONE_FIELD_SEED_OFFSET);
        for (label, field) in [
            ("local", ecotone_comparator as fn(usize, f64, f64, i64) -> f64),
            ("boundary", production_boundary_zone_comparator),
            ("range", production_boundary_range_comparator),
        ] {
            let expected = field(2, u, v, seed).to_bits();
            for (du, dv) in [(-0.20, -0.20), (0.20, -0.20), (-0.20, 0.20), (0.20, 0.20)] {
                let got = field(2, u + 2.0 * du / n, v + 2.0 * dv / n, seed);
                assert_eq!(got.to_bits(), expected, "{label} comparator changed within a column");
            }
            assert_eq!(field(2, u, v, seed).to_bits(), expected, "{label} repeat differed");
        }
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
    fn categorical_comparators_are_exact_across_cube_faces() {
        let seed = 42i64.wrapping_add(ECOTONE_FIELD_SEED_OFFSET);
        for (label, field) in [
            ("local", ecotone_comparator as fn(usize, f64, f64, i64) -> f64),
            ("boundary", production_boundary_zone_comparator),
            ("range", production_boundary_range_comparator),
        ] {
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
                    let got = field(face, u, v, seed).to_bits();
                    if let Some(expected) = expected {
                        assert_eq!(got, expected, "{label} face {face} differs at {dir:?}");
                    } else {
                        expected = Some(got);
                    }
                    copies += 1;
                }
                assert!(copies >= 2, "expected a shared edge/corner at {dir:?}");
            }
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

//! River courses and lakes from the planetgen drainage graph.
//!
//! scripts/bake_rivers.py exports the cell->receiver polyline segments (with
//! flow and monotonic-downstream water levels) and lake cells (with spill
//! levels). The terrain generator measures exact distance to the nearest
//! segment, so channels follow the map's real valleys and reach the sea —
//! the course is data, only the sub-cell meander is noise.
//!
//! Spatial lookup is a per-face uv bucket grid. Segments are inserted into
//! every bucket within their influence radius (channel + floodplain +
//! meander), so a query only ever reads the single bucket containing the
//! query point — an empty bucket is a near-free "no river here".

use std::collections::{HashMap, HashSet};

use crate::planet::FACES;
use glam::DVec3;

/// Andrew-facing river art knobs. Geometry constants use kilometres because
/// the terrain sampler does; flow thresholds are m^3/s and gradients are
/// dimensionless rise/run. Keep every Track-B dial here so art iteration does
/// not require hunting through the hot path, voxel mesher, and shader bridge.
pub mod river_tuning {
    // Hydraulic geometry and banks.
    pub const CHANNEL_HALF_WIDTH_PER_SQRT_FLOW_KM: f64 = 0.0015;
    pub const CHANNEL_DEPTH_GAIN_KM: f64 = 0.00027;
    pub const CHANNEL_DEPTH_MAX_KM: f64 = 0.012;
    pub const HEADWATER_TAPER_START_M3S: f64 = 120.0;
    pub const HEADWATER_TAPER_END_M3S: f64 = 400.0;
    pub const WIDTH_VARIATION: f64 = 0.34;
    pub const BANK_WIDTH_BASE: f64 = 0.82;
    pub const BANK_WIDTH_FLOW_GAIN: f64 = 0.95;
    pub const BANK_WIDTH_VARIATION: f64 = 0.31;
    pub const BANK_EDGE_WOBBLE_KM: f64 = 0.016;
    pub const BANK_EDGE_WAVELENGTH_KM: f64 = 0.95;
    pub const BANK_ASYMMETRY: f64 = 0.18;
    pub const BANK_ASYMMETRY_CUT_GAIN: f64 = 0.65;
    pub const BANK_PROFILE_NOISE_GAIN: f64 = 0.16;
    pub const RIVER_SHAPE_MIN_SCALE: f64 = 0.55;
    pub const BANK_MIN_WIDTH_KM: f64 = 0.020;
    pub const BANK_CUT_DEPTH_GAIN: f64 = 1.20;
    pub const BANK_PROFILE_POWER_BASE: f64 = 0.90;
    pub const BANK_PROFILE_POWER_FLOW_GAIN: f64 = 0.65;

    // Arc-length centerline noise.
    pub const MEANDER_AMPLITUDE_KM: f64 = 0.280;
    pub const MEANDER_WIDTH_GAIN: f64 = 0.10;
    pub const MEANDER_MAX_AMPLITUDE_KM: f64 = 0.620;
    pub const MEANDER_WAVELENGTH_KM: f64 = 4.2;
    pub const MEANDER_PHASE_OFFSET_CYCLES: f64 = 0.75;
    pub const MEANDER_HEADWATER_GAIN: f64 = 0.35;
    pub const MEANDER_FULL_FLOW_M3S: f64 = 800.0;
    pub const MEANDER_PIN_RAMP_KM: f64 = 5.0;
    pub const WIDTH_NOISE_WAVELENGTH_GAIN: f64 = 0.72;
    pub const BANK_NOISE_WAVELENGTH_GAIN: f64 = 1.35;

    // Bank material band.
    pub const BANK_MATERIAL_FULL_HEIGHT_KM: f64 = 0.0018;
    pub const BANK_MATERIAL_END_HEIGHT_KM: f64 = 0.0045;
    pub const BANK_MATERIAL_REACH: f64 = 0.58;
    pub const BANK_MATERIAL_INNER_REACH: f64 = 0.72;
    pub const BANK_MATERIAL_COHERENT_START: f64 = 0.16;
    pub const BANK_MATERIAL_COHERENT_FULL: f64 = 0.46;
    pub const BANK_MATERIAL_VOXEL_THRESHOLD: f64 = 0.50;
    pub const BANK_MATERIAL_TYPE_PATCH_SHIFT: u32 = 6;
    pub const BANK_MATERIAL_DRY_START_KM: f64 = -0.0006;
    pub const BANK_MATERIAL_DRY_END_KM: f64 = 0.0002;
    pub const BANK_GRAVEL_FLOW_START_M3S: f64 = 1_200.0;
    pub const BANK_GRAVEL_FLOW_END_M3S: f64 = 18_000.0;
    pub const BANK_GRAVEL_COLD_FULL_C: f64 = 1.0;
    pub const BANK_GRAVEL_COLD_END_C: f64 = 18.0;
    pub const BANK_GRAVEL_WET_START_MM: f64 = 650.0;
    pub const BANK_GRAVEL_WET_FULL_MM: f64 = 1_600.0;
    pub const BANK_GRAVEL_BASE: f64 = 0.08;
    pub const BANK_GRAVEL_FLOW_GAIN: f64 = 0.52;
    pub const BANK_GRAVEL_COLD_GAIN: f64 = 0.24;
    pub const BANK_GRAVEL_WET_GAIN: f64 = 0.10;
    pub const BANK_GRAVEL_ROUGH_GAIN: f64 = 0.22;

    // Braided reaches and walkable bars.
    pub const ISLAND_DENSITY: f64 = 0.42;
    pub const ISLAND_MIN_FLOW_M3S: f64 = 2_200.0;
    pub const ISLAND_FULL_FLOW_M3S: f64 = 12_000.0;
    pub const ISLAND_MAX_GRADIENT: f64 = 0.0009;
    pub const ISLAND_BAR_SPACING_KM: f64 = 2.4;
    pub const ISLAND_BAR_DENSITY: f64 = 0.68;
    pub const ISLAND_HALF_LENGTH_MIN_KM: f64 = 0.42;
    pub const ISLAND_HALF_LENGTH_MAX_KM: f64 = 1.15;
    pub const ISLAND_HALF_WIDTH_MIN: f64 = 0.16;
    pub const ISLAND_HALF_WIDTH_MAX: f64 = 0.31;
    pub const ISLAND_LATERAL_WANDER: f64 = 0.22;
    pub const ISLAND_LOW_GRADE_GAIN: f64 = 0.35;
    pub const ISLAND_CENTER_JITTER_MIN: f64 = 0.18;
    pub const ISLAND_CENTER_JITTER_RANGE: f64 = 0.64;
    pub const ISLAND_LENGTH_CORE: f64 = 0.58;
    pub const ISLAND_WIDTH_CORE: f64 = 0.52;
    pub const ISLAND_SUBMERGED_MARGIN_KM: f64 = 0.0016;
    pub const ISLAND_EMERGENCE_KM: f64 = 0.0042;
    pub const ISLAND_WET_FADE_START: f64 = 0.42;
    pub const ISLAND_WET_FADE_END: f64 = 0.82;

    // Falls, falling sheet, plunge foam, and restrained base mist.
    pub const FALL_MIN_FLOW_M3S: f64 = 300.0;
    pub const FALL_FULL_FLOW_M3S: f64 = 1_500.0;
    pub const FALL_MIN_DROP_KM: f64 = 0.050;
    pub const FALL_FULL_DROP_KM: f64 = 0.180;
    pub const FALL_MIN_GRADIENT: f64 = 0.006;
    pub const FALL_FULL_GRADIENT: f64 = 0.014;
    pub const FALL_SITE_MIN_STRENGTH: f64 = 0.08;
    pub const FALL_VISUAL_DROP_FRACTION: f64 = 0.55;
    pub const FALL_VISUAL_DROP_MAX_KM: f64 = 0.120;
    pub const FALL_DRAWDOWN_SHARE: f64 = 0.12;
    pub const FALL_DRAWDOWN_REACH_KM: f64 = 0.75;
    pub const FALL_PROFILE_HALF_ARC_KM: f64 = 0.0025;
    pub const FALL_PROFILE_MIN_HALF_T: f64 = 0.00004;
    pub const FALL_FOAM_UPSTREAM_REACH_KM: f64 = 3.0;
    pub const FALL_FOAM_DOWNSTREAM_REACH_KM: f64 = 0.90;
    pub const FALL_SHEET_SHADE_REACH_KM: f64 = 0.11;
    pub const FALL_VISUAL_STRENGTH_GAIN: f64 = 1.85;
    pub const FALL_BANK_MATERIAL_SUPPRESS_START: f64 = 0.02;
    pub const FALL_BANK_MATERIAL_SUPPRESS_FULL: f64 = 0.10;
    pub const FALL_SHEET_WIDTH_GAIN: f64 = 1.18;
    pub const FALL_SHEET_STRENGTH: f64 = 1.55;
    pub const FALL_SURFACE_FOAM_GAIN: f64 = 0.92;
    pub const FALL_FOAM_COLOR: [f32; 3] = [0.86, 0.94, 0.97];
    pub const FOAM_INTENSITY: f64 = 1.10;

    // B-9: procedural ponds are native at eight detail octaves; carry their
    // exact class at seven so the existing parent morph owns a graceful LOD
    // handoff instead of a hard water/no-water tile border.
    pub const POND_NATIVE_MIN_OCTAVES: u32 = 8;
    pub const POND_LOD_CARRY_OCTAVES: u32 = 7;
    pub const LIQUID_LAKE_INNER_APRON_CLEARANCE_KM: f64 = 0.0012;
    pub const LIQUID_LAKE_INNER_APRON_GRADE: f64 = 0.030;
}

pub struct Segment {
    pub a: DVec3, // unit vector, upstream end
    pub b: DVec3, // unit vector, downstream (receiver) end
    pub flow_log: f32,
    pub level_a_km: f32,
    pub level_b_km: f32,
    /// Cached metric facts used by every query. These are derived from the
    /// serialized fields and therefore do not change the RIV1 format.
    pub length_km: f32,
    pub flow_m3s: f32,
    /// 0 for an ordinary reach; otherwise load-time waterfall confidence.
    pub fall_strength: f32,
    /// Junction-to-junction course metadata derived at load. Internal graph
    /// nodes share reach arc length and endpoint tangents, so lateral noise
    /// stays continuous instead of restarting at every drainage cell.
    pub reach_id: u32,
    pub reach_arc_start_km: f64,
    pub reach_length_km: f64,
    pub tangent_a: DVec3,
    pub tangent_b: DVec3,
    pub meander_amplitude_km: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct FallSite {
    pub segment_id: u32,
    pub center: DVec3,
    pub tangent: DVec3,
    pub across: DVec3,
    pub half_width_km: f64,
    pub top_level_km: f64,
    pub bottom_level_km: f64,
    pub strength: f64,
}

pub struct LakeCell {
    pub center: DVec3, // unit vector
    pub radius_km: f32,
    pub level_km: f32,
    pub salt: bool,
    /// A rim cell: dry land bordering the lake. Water only exists where the
    /// nearest cell is a lake cell — the Voronoi footprint of the actual
    /// map lake, instead of blobby per-cell discs.
    pub rim: bool,
}

pub struct RiverHit {
    pub dist_km: f64,
    pub signed_dist_km: f64,
    pub level_km: f64,
    pub flow: f64, // m3/s at the segment
    pub flow_log10: f64,
    pub half_width_km: f64,
    pub depth_km: f64,
    pub bank_width_scale: f64,
    pub bank_profile_power: f64,
    pub island_field: f64,
    /// Aerated approach/plunge influence and narrow falling-sheet influence
    /// are separate so horizontal white water does not inherit sheet streaks.
    pub fall_strength: f64,
    pub fall_sheet_strength: f64,
    pub segment_id: u32,
    pub segment_t: f64,
}

/// Lake candidate at a query point — everything the terrain sampler needs to
/// decide whether this column floods. The decision itself lives in
/// terrain::sample (it needs the planet rasters); this is pure geometry.
pub struct LakeHit {
    pub level_km: f64,
    pub salt: bool,
    /// nearest true lake cell: distance, center, radius
    pub d_lake_km: f64,
    pub lake_center: DVec3,
    pub radius_km: f64,
    /// Distance to the baked lake footprint's actual edge: the bisector
    /// between the nearest true lake cell and nearest dry rim cell. Unlike
    /// `past_boundary_km`, this is symmetric, so it stays meaningful on dry
    /// islands and shoals inside the flood-eligible territory as well as in
    /// the rim territory outside it.
    pub boundary_dist_km: f64,
    /// whether the nearest cell overall (lake AND rim competing) is a lake
    /// cell — i.e. the query is inside the lake's own Voronoi territory
    pub in_lake_voronoi: bool,
    /// how far past the lake/rim Voronoi boundary the query sits (0 inside
    /// lake territory; grows toward the rim cell's far side)
    pub past_boundary_km: f64,
    /// distance past the NEAREST flood frontier (the flood is a union of
    /// the lake's Voronoi territory and every dam rim's 1.15 r shore band;
    /// the shore apron grades from whichever frontier is closest). The
    /// Voronoi metric alone left a 170 m wall where a non-dam rim's
    /// territory sits beside a dam rim's flooded band (16.569 -32.262).
    pub apron_past_km: f64,
    /// Distance inward from the active flood predicate's nearest frontier.
    /// This is bathymetry metadata only: `flood_eligible` remains the sole
    /// wet/dry authority. In particular it follows radius-scaled dam bands,
    /// whose edge can differ from the raw lake/rim Voronoi bisector.
    pub flood_edge_margin_km: f64,
    /// Nearest Voronoi bisector to a different baked lake key. A merged
    /// high/low lake pair can switch levels here even while both winners are
    /// far from their own lake/rim boundary.
    pub competing_lake_boundary_km: f64,
    /// Higher competing lake and distance to its bisector, or -INF/+INF.
    /// Terrain uses this only to grade a dry bank toward the higher surface;
    /// it never changes which lake wins.
    pub higher_competing_level_km: f64,
    pub higher_competing_boundary_km: f64,
    /// the nearest rim (when the nearest cell is one) is a TRUE DAM: its own
    /// elevation reaches the lake level. Shore-band flood-through is only
    /// sound over dam-height rims — a below-level rim (e.g. a peeled
    /// conduit cell down a mountain flank) must not pass the flood through
    /// its territory.
    pub rim_is_dam: bool,
}

impl LakeHit {
    /// Octave-independent geometric half of the lake flood predicate. The
    /// terrain sampler still decides liquid versus dry from detailed ground.
    #[inline]
    pub fn flood_eligible(&self) -> bool {
        let d_any = self.d_lake_km - self.past_boundary_km;
        self.d_lake_km < self.radius_km * 2.6
            && (self.in_lake_voronoi || (d_any < self.radius_km * 1.15 && self.rim_is_dam))
    }
}

const GRID: usize = 128;
/// Influence radius (km) a query may care about: channel half-width (<=0.4)
/// + floodplain damping (<=2.5) + meander (<=0.3), rounded up.
pub const SEG_INFLUENCE_KM: f64 = 3.5;

pub struct RiverIndex {
    pub segments: Vec<Segment>,
    pub lakes: Vec<LakeCell>,
    pub fall_sites: Vec<FallSite>,
    radius_km: f64,
    seg_buckets: Vec<Vec<u32>>,  // 6 * GRID * GRID
    lake_buckets: Vec<Vec<u32>>,
}

fn bucket_of(face: usize, u: f64, v: f64) -> usize {
    let x = (((u + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
    let y = (((v + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
    (face * GRID + y) * GRID + x
}

/// Project a unit direction onto one face's gnomonic plane (no clamping).
fn face_uv(face: usize, dir: DVec3) -> Option<(f64, f64)> {
    let (axis, right, up) = FACES[face];
    let d = dir.dot(axis);
    if d < 0.30 {
        return None; // far behind this face; projection blows up
    }
    let p = dir / d;
    Some((p.dot(right), p.dot(up)))
}

type NodeKey = [u64; 3];

#[inline]
fn node_key(p: DVec3) -> NodeKey {
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
}

fn endpoint_tangent(a: DVec3, b: DVec3, at_b: bool) -> DVec3 {
    let center = if at_b { b } else { a }.normalize_or_zero();
    let chord = b - a;
    (chord - center * chord.dot(center)).normalize_or_zero()
}

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    if a == b {
        return (x >= b) as u8 as f64;
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[inline]
fn river_hash01(seed: i64, segment_id: u32, lane: i64, salt: u64) -> f64 {
    let key = (seed as u64)
        ^ u64::from(segment_id).wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ (lane as u64).wrapping_mul(0xA076_1D64_78BD_642F)
        ^ salt;
    let bits = splitmix64(key) >> 11;
    bits as f64 * (1.0 / ((1u64 << 53) as f64))
}

fn arc_noise(seed: i64, segment_id: u32, arc_km: f64, wavelength_km: f64, salt: u64) -> f64 {
    let phase = river_hash01(seed, segment_id, 0, salt ^ 0x91E1_0DA5_C79E_7B1D);
    let x = arc_km / wavelength_km.max(1e-6) + phase;
    let i = x.floor() as i64;
    let f = x - i as f64;
    let s = f * f * (3.0 - 2.0 * f);
    let a = river_hash01(seed, segment_id, i, salt) * 2.0 - 1.0;
    let b = river_hash01(seed, segment_id, i + 1, salt) * 2.0 - 1.0;
    a + (b - a) * s
}

/// A seeded, arc-length-attached bend carrier. Pure value noise can spend an
/// entire photographed reach near zero, making a large amplitude dial
/// visually inert. The warped sinusoid guarantees developed bends while two
/// much longer value-noise fields keep their cadence and magnitude organic.
fn meander_wave(seed: i64, reach_id: u32, arc_km: f64) -> f64 {
    use std::f64::consts::TAU;
    use river_tuning::MEANDER_WAVELENGTH_KM;
    let phase = river_hash01(seed, reach_id, 0, 0x4D45_414E_5048_4153)
        + river_tuning::MEANDER_PHASE_OFFSET_CYCLES;
    let phase_warp = 0.18
        * arc_noise(
            seed,
            reach_id,
            arc_km,
            MEANDER_WAVELENGTH_KM * 3.4,
            0x4D45_414E_5741_5250,
        );
    let magnitude = 0.82
        + 0.18
            * arc_noise(
                seed,
                reach_id,
                arc_km,
                MEANDER_WAVELENGTH_KM * 5.1,
                0x4D45_414E_414D_5053,
            )
            .abs();
    (TAU * (arc_km / MEANDER_WAVELENGTH_KM + phase + phase_warp)).sin() * magnitude
}

/// Established hydraulic width/depth law, centralized so the displaced
/// course, island field, terrain carve, and fall geometry cannot drift.
pub fn hydraulic_geometry(flow_m3s: f64) -> (f64, f64) {
    use river_tuning::*;
    let taper = smoothstep(HEADWATER_TAPER_START_M3S, HEADWATER_TAPER_END_M3S, flow_m3s);
    if taper <= 0.0 {
        return (0.0, 0.0);
    }
    let half_width = CHANNEL_HALF_WIDTH_PER_SQRT_FLOW_KM * flow_m3s.sqrt() * taper;
    let depth = (CHANNEL_DEPTH_GAIN_KM * flow_m3s.powf(0.39)).min(CHANNEL_DEPTH_MAX_KM) * taper;
    (half_width, depth)
}

fn meander_amplitude_km(flow_m3s: f64) -> f64 {
    use river_tuning::*;
    let flow_gain = smoothstep(HEADWATER_TAPER_START_M3S, MEANDER_FULL_FLOW_M3S, flow_m3s);
    let (base_hw, _) = hydraulic_geometry(flow_m3s);
    (MEANDER_AMPLITUDE_KM
        * (MEANDER_HEADWATER_GAIN + (1.0 - MEANDER_HEADWATER_GAIN) * flow_gain)
        + base_hw * MEANDER_WIDTH_GAIN)
        .min(MEANDER_MAX_AMPLITUDE_KM)
}

fn fall_strength(flow_m3s: f64, drop_km: f64, length_km: f64) -> f64 {
    use river_tuning::*;
    if length_km <= 0.0 || drop_km <= 0.0 {
        return 0.0;
    }
    let gradient = drop_km / length_km;
    let strength = smoothstep(FALL_MIN_FLOW_M3S, FALL_FULL_FLOW_M3S, flow_m3s)
        * smoothstep(FALL_MIN_DROP_KM, FALL_FULL_DROP_KM, drop_km)
        * smoothstep(FALL_MIN_GRADIENT, FALL_FULL_GRADIENT, gradient);
    if strength >= FALL_SITE_MIN_STRENGTH { strength } else { 0.0 }
}

fn fall_half_t(length_km: f64) -> f64 {
    (river_tuning::FALL_PROFILE_HALF_ARC_KM / length_km.max(1e-6))
        .clamp(river_tuning::FALL_PROFILE_MIN_HALF_T, 0.22)
}

fn fall_foam_influence(s: &Segment, t: f64) -> f64 {
    use river_tuning::*;
    let arc_km = (t.clamp(0.0, 1.0) - 0.5) * f64::from(s.length_km);
    let reach = if arc_km <= 0.0 {
        FALL_FOAM_UPSTREAM_REACH_KM
    } else {
        FALL_FOAM_DOWNSTREAM_REACH_KM
    };
    1.0 - smoothstep(FALL_PROFILE_HALF_ARC_KM, reach, arc_km.abs())
}

fn fall_sheet_influence(s: &Segment, t: f64) -> f64 {
    use river_tuning::*;
    let arc_km = (t.clamp(0.0, 1.0) - 0.5).abs() * f64::from(s.length_km);
    1.0 - smoothstep(FALL_PROFILE_HALF_ARC_KM, FALL_SHEET_SHADE_REACH_KM, arc_km)
}

/// Endpoint-preserving monotone profile. Only a bounded portion of a detected
/// reach's existing drop is concentrated into the chute; the residual remains
/// a linear downstream grade.
fn segment_level_km(s: &Segment, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let a = f64::from(s.level_a_km);
    let b = f64::from(s.level_b_km);
    // Preserve the serialized graph nodes bit-for-bit. Besides making the
    // monotone invariant explicit, this prevents independent reaches from
    // acquiring a one-ulp seam through different arithmetic at their join.
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let drop = (a - b).max(0.0);
    if s.fall_strength <= 0.0 || drop <= 0.0 {
        return a + (b - a) * t;
    }
    let visual = (drop * river_tuning::FALL_VISUAL_DROP_FRACTION)
        .min(river_tuning::FALL_VISUAL_DROP_MAX_KM);
    let residual = drop - visual;
    let half = fall_half_t(f64::from(s.length_km));
    let drawdown_t = (river_tuning::FALL_DRAWDOWN_REACH_KM
        / f64::from(s.length_km).max(1e-6))
        .clamp(half, 0.28);
    let drawdown = smoothstep(0.5 - drawdown_t, 0.5 - half, t);
    let sheet = smoothstep(0.5 - half, 0.5 + half, t);
    let concentrated = river_tuning::FALL_DRAWDOWN_SHARE * drawdown
        + (1.0 - river_tuning::FALL_DRAWDOWN_SHARE) * sheet;
    a - residual * t - visual * concentrated
}

fn segment_frame(s: &Segment, t: f64) -> (DVec3, DVec3, DVec3) {
    let chord = s.b - s.a;
    let center = (s.a + chord * t).normalize();
    let blended = s.tangent_a.lerp(s.tangent_b, t);
    let mut tangent = (blended - center * blended.dot(center)).normalize_or_zero();
    if tangent.length_squared() < 0.5 {
        tangent = (chord - center * chord.dot(center)).normalize_or_zero();
    }
    let across = center.cross(tangent).normalize_or_zero();
    (center, tangent, across)
}

#[inline]
fn reach_arc_km(s: &Segment, t: f64) -> f64 {
    s.reach_arc_start_km + t.clamp(0.0, 1.0) * f64::from(s.length_km)
}

#[inline]
fn reach_envelope(s: &Segment, t: f64) -> f64 {
    let arc = reach_arc_km(s, t).clamp(0.0, s.reach_length_km);
    let pin_distance = arc.min((s.reach_length_km - arc).max(0.0));
    smoothstep(0.0, river_tuning::MEANDER_PIN_RAMP_KM, pin_distance)
}

fn meander_offset_km(s: &Segment, t: f64, seed: i64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    // Position and first derivative are pinned only at hydraulic reach ends
    // (sources, confluences, mouths, and lake transitions). Internal graph
    // cells share one arc/noise phase and one continuous tangent frame.
    let envelope = reach_envelope(s, t);
    let arc = reach_arc_km(s, t);
    envelope
        * f64::from(s.meander_amplitude_km)
        * meander_wave(seed, s.reach_id, arc)
}

#[derive(Clone, Copy)]
struct SegmentShape {
    t: f64,
    dist_km: f64,
    signed_dist_km: f64,
    width_scale: f64,
    bank_scale: f64,
    bank_profile_power: f64,
}

fn segment_shape(s: &Segment, p: DVec3, radius_km: f64, seed: i64) -> SegmentShape {
    use river_tuning::*;
    let a = s.a * radius_km;
    let ab = s.b * radius_km - a;
    let ab2 = ab.length_squared().max(1e-12);
    let mut t = ((p - a).dot(ab) / ab2).clamp(0.0, 1.0);
    // Two inexpensive projection corrections cover the bounded sub-cell
    // displacement without a subdivision loop in the terrain hot path.
    for _ in 0..2 {
        let (_, _, across) = segment_frame(s, t);
        let center = a + ab * t + across * meander_offset_km(s, t, seed);
        t = (t + (p - center).dot(ab) / ab2).clamp(0.0, 1.0);
    }
    let (_, _, across) = segment_frame(s, t);
    let center = a + ab * t + across * meander_offset_km(s, t, seed);
    let delta = p - center;
    let arc = reach_arc_km(s, t);
    let endpoint_envelope = reach_envelope(s, t);
    let width_n = arc_noise(
        seed,
        s.reach_id,
        arc,
        MEANDER_WAVELENGTH_KM * WIDTH_NOISE_WAVELENGTH_GAIN,
        0x5749_4454_4800_0002,
    );
    let bank_n = arc_noise(
        seed,
        s.reach_id,
        arc,
        MEANDER_WAVELENGTH_KM * BANK_NOISE_WAVELENGTH_GAIN,
        0x4241_4E4B_0000_0003,
    );
    let edge_n = arc_noise(
        seed,
        s.reach_id,
        arc,
        BANK_EDGE_WAVELENGTH_KM,
        0x4544_4745_0000_000A,
    );
    let signed_dist_km = delta.dot(across);
    let side = if signed_dist_km < 0.0 { -1.0 } else { 1.0 };
    let flow_gain = smoothstep(
        HEADWATER_TAPER_END_M3S,
        ISLAND_FULL_FLOW_M3S,
        f64::from(s.flow_m3s),
    );
    SegmentShape {
        t,
        dist_km: (delta.length() - BANK_EDGE_WOBBLE_KM * endpoint_envelope * edge_n).max(0.0),
        signed_dist_km,
        width_scale: (1.0
            + WIDTH_VARIATION * endpoint_envelope * width_n
            + BANK_ASYMMETRY * endpoint_envelope * edge_n * side)
            .max(RIVER_SHAPE_MIN_SCALE),
        bank_scale: (1.0
            + BANK_WIDTH_VARIATION * endpoint_envelope * bank_n
            + BANK_ASYMMETRY * BANK_ASYMMETRY_CUT_GAIN * endpoint_envelope * width_n * side)
            .max(RIVER_SHAPE_MIN_SCALE),
        bank_profile_power: BANK_PROFILE_POWER_BASE
            + BANK_PROFILE_POWER_FLOW_GAIN * flow_gain
            + BANK_PROFILE_NOISE_GAIN * endpoint_envelope * bank_n,
    }
}

fn braided_segment(s: &Segment, segment_id: u32, seed: i64) -> bool {
    use river_tuning::*;
    let flow = f64::from(s.flow_m3s);
    let gradient = (f64::from(s.level_a_km) - f64::from(s.level_b_km)).max(0.0)
        / f64::from(s.length_km).max(1e-6);
    if flow < ISLAND_MIN_FLOW_M3S || gradient >= ISLAND_MAX_GRADIENT {
        return false;
    }
    let flow_gate = smoothstep(ISLAND_MIN_FLOW_M3S, ISLAND_FULL_FLOW_M3S, flow);
    let grade_gate = 1.0
        - smoothstep(
            ISLAND_MAX_GRADIENT * ISLAND_LOW_GRADE_GAIN,
            ISLAND_MAX_GRADIENT,
            gradient,
        );
    river_hash01(seed, segment_id, 0, 0x4252_4149_4453_0004)
        < ISLAND_DENSITY * flow_gate * grade_gate
}

fn island_field(
    s: &Segment,
    segment_id: u32,
    t: f64,
    signed_dist_km: f64,
    half_width_km: f64,
    seed: i64,
) -> f64 {
    use river_tuning::*;
    if half_width_km <= 0.0 || !braided_segment(s, segment_id, seed) {
        return 0.0;
    }
    let along = t * f64::from(s.length_km);
    let cell = (along / ISLAND_BAR_SPACING_KM).floor() as i64;
    let mut field = 0.0f64;
    for k in (cell - 1)..=(cell + 1) {
        if river_hash01(seed, segment_id, k, 0x4241_5247_4154_4505) >= ISLAND_BAR_DENSITY {
            continue;
        }
        let jitter = river_hash01(seed, segment_id, k, 0x4C4F_4E47_0000_0006);
        let center = (k as f64 + ISLAND_CENTER_JITTER_MIN + ISLAND_CENTER_JITTER_RANGE * jitter)
            * ISLAND_BAR_SPACING_KM;
        let hl = ISLAND_HALF_LENGTH_MIN_KM
            + (ISLAND_HALF_LENGTH_MAX_KM - ISLAND_HALF_LENGTH_MIN_KM)
                * river_hash01(seed, segment_id, k, 0x4C45_4E47_5448_0007);
        let long = 1.0 - smoothstep(hl * ISLAND_LENGTH_CORE, hl, (along - center).abs());
        if long <= 0.0 {
            continue;
        }
        let lateral_center = (river_hash01(seed, segment_id, k, 0x4C41_5445_5241_4C08) * 2.0 - 1.0)
            * half_width_km
            * ISLAND_LATERAL_WANDER;
        let hw = half_width_km
            * (ISLAND_HALF_WIDTH_MIN
                + (ISLAND_HALF_WIDTH_MAX - ISLAND_HALF_WIDTH_MIN)
                    * river_hash01(seed, segment_id, k, 0x5749_4454_4800_0009));
        let across = 1.0
            - smoothstep(
                hw * ISLAND_WIDTH_CORE,
                hw,
                (signed_dist_km - lateral_center).abs(),
            );
        field = field.max(long * across);
    }
    field
}

impl RiverIndex {
    pub fn empty(radius_km: f64) -> Self {
        Self {
            segments: Vec::new(),
            lakes: Vec::new(),
            fall_sites: Vec::new(),
            radius_km,
            seg_buckets: vec![Vec::new(); 6 * GRID * GRID],
            lake_buckets: vec![Vec::new(); 6 * GRID * GRID],
        }
    }

    pub fn load(path: &str, radius_km: f64, seed: i64) -> anyhow::Result<Self> {
        let raw = std::fs::read(path)?;
        anyhow::ensure!(raw.len() >= 12 && &raw[0..4] == b"RIV1", "bad rivers.bin header");
        let n_seg = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let n_lake = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
        anyhow::ensure!(
            raw.len() == 12 + n_seg * 36 + n_lake * 24,
            "rivers.bin size mismatch - rerun scripts/bake_rivers.py"
        );
        let f = |off: usize| f32::from_le_bytes(raw[off..off + 4].try_into().unwrap());
        let mut out = Self::empty(radius_km);
        for i in 0..n_seg {
            let o = 12 + i * 36;
            let a = DVec3::new(f(o) as f64, f(o + 4) as f64, f(o + 8) as f64);
            let b = DVec3::new(f(o + 12) as f64, f(o + 16) as f64, f(o + 20) as f64);
            let flow_log = f(o + 24);
            let flow_m3s = (10f64.powf(f64::from(flow_log)) - 1.0) as f32;
            let level_a_km = f(o + 28);
            let level_b_km = f(o + 32);
            let length_km = (a.dot(b).clamp(-1.0, 1.0).acos() * radius_km) as f32;
            let tangent_a = endpoint_tangent(a, b, false);
            let tangent_b = endpoint_tangent(a, b, true);
            out.segments.push(Segment {
                a,
                b,
                flow_log,
                level_a_km,
                level_b_km,
                length_km,
                flow_m3s,
                fall_strength: fall_strength(
                    f64::from(flow_m3s),
                    (f64::from(level_a_km) - f64::from(level_b_km)).max(0.0),
                    f64::from(length_km),
                ) as f32,
                reach_id: i as u32,
                reach_arc_start_km: 0.0,
                reach_length_km: f64::from(length_km),
                tangent_a,
                tangent_b,
                meander_amplitude_km: meander_amplitude_km(f64::from(flow_m3s)) as f32,
            });
        }
        let lake_base = 12 + n_seg * 36;
        for i in 0..n_lake {
            let o = lake_base + i * 24;
            let flags = f(o + 20);
            out.lakes.push(LakeCell {
                center: DVec3::new(f(o) as f64, f(o + 4) as f64, f(o + 8) as f64),
                radius_km: f(o + 12),
                level_km: f(o + 16),
                salt: (flags - 1.0).abs() < 0.25,
                rim: flags > 1.5,
            });
        }
        out.build_reaches();
        out.build_fall_sites(seed);
        out.build_buckets();
        Ok(out)
    }

    /// Derive junction-to-junction reaches without changing `RIV1`. Exact
    /// serialized endpoint bits are node identity, so this is deterministic
    /// and cannot accidentally join merely-near drainage cells.
    fn build_reaches(&mut self) {
        let mut incoming: HashMap<NodeKey, Vec<usize>> = HashMap::new();
        let mut outgoing: HashMap<NodeKey, Vec<usize>> = HashMap::new();
        for (id, s) in self.segments.iter().enumerate() {
            outgoing.entry(node_key(s.a)).or_default().push(id);
            incoming.entry(node_key(s.b)).or_default().push(id);
        }

        let mut pins = HashSet::new();
        for key in incoming.keys().chain(outgoing.keys()) {
            if incoming.get(key).map_or(0, Vec::len) != 1
                || outgoing.get(key).map_or(0, Vec::len) != 1
            {
                pins.insert(*key);
            }
        }

        // A lake may contain a long degree-1 graph chain. Pin only where a
        // segment crosses into/out of its serialized cell set, not at every
        // submerged cell, so the lake entry remains connected without
        // reintroducing per-cell resets.
        let lake_nodes: HashSet<NodeKey> = self
            .lakes
            .iter()
            .filter(|l| !l.rim)
            .map(|l| node_key(l.center))
            .collect();
        for s in &self.segments {
            let a_lake = lake_nodes.contains(&node_key(s.a));
            let b_lake = lake_nodes.contains(&node_key(s.b));
            if a_lake != b_lake {
                pins.insert(if a_lake { node_key(s.a) } else { node_key(s.b) });
            }
        }

        let mut assigned = vec![false; self.segments.len()];
        let mut reach_id = 0u32;
        for start in 0..self.segments.len() {
            if assigned[start] || !pins.contains(&node_key(self.segments[start].a)) {
                continue;
            }
            let chain = Self::trace_reach(start, &outgoing, &pins, &mut assigned, &self.segments);
            self.assign_reach(&chain, reach_id);
            reach_id += 1;
        }
        // Defensive cycle/orphan handling. The baked drainage graph should
        // have no unpinned cycles, but stable segment order still gives any
        // malformed component deterministic metadata instead of a panic.
        for start in 0..self.segments.len() {
            if assigned[start] {
                continue;
            }
            let chain = Self::trace_reach(start, &outgoing, &pins, &mut assigned, &self.segments);
            self.assign_reach(&chain, reach_id);
            reach_id += 1;
        }
    }

    fn trace_reach(
        start: usize,
        outgoing: &HashMap<NodeKey, Vec<usize>>,
        pins: &HashSet<NodeKey>,
        assigned: &mut [bool],
        segments: &[Segment],
    ) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut current = start;
        loop {
            if assigned[current] {
                break;
            }
            assigned[current] = true;
            chain.push(current);
            let end = node_key(segments[current].b);
            if pins.contains(&end) {
                break;
            }
            let Some(next) = outgoing.get(&end).and_then(|ids| ids.first()).copied() else {
                break;
            };
            current = next;
        }
        chain
    }

    fn assign_reach(&mut self, chain: &[usize], reach_id: u32) {
        if chain.is_empty() {
            return;
        }
        let total = chain
            .iter()
            .map(|&id| f64::from(self.segments[id].length_km))
            .sum::<f64>()
            .max(1e-6);
        let reach_flow = chain
            .iter()
            .map(|&id| f64::from(self.segments[id].flow_m3s))
            .fold(0.0, f64::max);
        let reach_amplitude = meander_amplitude_km(reach_flow) as f32;
        let mut tangents = Vec::with_capacity(chain.len() + 1);
        let first = &self.segments[chain[0]];
        tangents.push(endpoint_tangent(first.a, first.b, false));
        for pair in chain.windows(2) {
            let previous = &self.segments[pair[0]];
            let next = &self.segments[pair[1]];
            let incoming = endpoint_tangent(previous.a, previous.b, true);
            let outgoing = endpoint_tangent(next.a, next.b, false);
            let joined = (incoming + outgoing).normalize_or_zero();
            tangents.push(if joined.length_squared() > 0.5 { joined } else { outgoing });
        }
        let last = &self.segments[*chain.last().unwrap()];
        tangents.push(endpoint_tangent(last.a, last.b, true));

        let mut arc = 0.0;
        for (index, &id) in chain.iter().enumerate() {
            let s = &mut self.segments[id];
            s.reach_id = reach_id;
            s.reach_arc_start_km = arc;
            s.reach_length_km = total;
            s.tangent_a = tangents[index];
            s.tangent_b = tangents[index + 1];
            s.meander_amplitude_km = reach_amplitude;
            arc += f64::from(s.length_km);
        }
    }

    fn build_fall_sites(&mut self, seed: i64) {
        use river_tuning::*;
        for (id, s) in self.segments.iter().enumerate() {
            let strength = f64::from(s.fall_strength);
            if strength <= 0.0 {
                continue;
            }
            let t = 0.5;
            let (base, _, across0) = segment_frame(s, t);
            let center = (base
                + across0 * (meander_offset_km(s, t, seed) / self.radius_km))
                .normalize();
            let chord = s.b - s.a;
            let tangent = (chord - center * chord.dot(center)).normalize_or_zero();
            let across = center.cross(tangent).normalize_or_zero();
            let half_t = fall_half_t(f64::from(s.length_km));
            let top_level_km = segment_level_km(s, t - half_t);
            let bottom_level_km = segment_level_km(s, t + half_t);
            let (base_hw, _) = hydraulic_geometry(f64::from(s.flow_m3s));
            let shape = segment_shape(
                s,
                center * self.radius_km,
                self.radius_km,
                seed,
            );
            self.fall_sites.push(FallSite {
                segment_id: id as u32,
                center,
                tangent,
                across,
                half_width_km: base_hw * shape.width_scale * FALL_SHEET_WIDTH_GAIN,
                top_level_km,
                bottom_level_km,
                strength,
            });
        }
    }

    fn insert(buckets: &mut [Vec<u32>], face: usize, lo: (f64, f64), hi: (f64, f64), id: u32) {
        let clamp01 = |t: f64| t.clamp(-1.0, 1.0);
        let (u0, v0) = (clamp01(lo.0), clamp01(lo.1));
        let (u1, v1) = (clamp01(hi.0), clamp01(hi.1));
        let bx0 = ((u0 + 1.0) * 0.5 * GRID as f64) as usize;
        let bx1 = (((u1 + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
        let by0 = ((v0 + 1.0) * 0.5 * GRID as f64) as usize;
        let by1 = (((v1 + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
        for y in by0..=by1 {
            for x in bx0..=bx1 {
                buckets[(face * GRID + y) * GRID + x].push(id);
            }
        }
    }

    fn build_buckets(&mut self) {
        // uv padding that covers the metric influence radius anywhere on a
        // face (worst-case gnomonic compression at corners is 3x)
        let pad_seg = SEG_INFLUENCE_KM / self.radius_km * 3.2;
        for (i, s) in self.segments.iter().enumerate() {
            for face in 0..6 {
                let (pa, pb) = match (face_uv(face, s.a), face_uv(face, s.b)) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                };
                let lo = (pa.0.min(pb.0) - pad_seg, pa.1.min(pb.1) - pad_seg);
                let hi = (pa.0.max(pb.0) + pad_seg, pa.1.max(pb.1) + pad_seg);
                if lo.0 > 1.0 || lo.1 > 1.0 || hi.0 < -1.0 || hi.1 < -1.0 {
                    continue;
                }
                Self::insert(&mut self.seg_buckets, face, lo, hi, i as u32);
            }
        }
        for (i, l) in self.lakes.iter().enumerate() {
            for face in 0..6 {
                let Some(c) = face_uv(face, l.center) else { continue };
                // pad past the cell spacing: the Voronoi test needs every
                // competitor (lake AND rim) visible from any query point
                // whose nearest cell might be this one (3.4 matches the
                // extended hit gate in lake_at)
                let pad = (l.radius_km as f64 * 3.4 + 2.0) / self.radius_km * 3.2;
                let lo = (c.0 - pad, c.1 - pad);
                let hi = (c.0 + pad, c.1 + pad);
                if lo.0 > 1.0 || lo.1 > 1.0 || hi.0 < -1.0 || hi.1 < -1.0 {
                    continue;
                }
                Self::insert(&mut self.lake_buckets, face, lo, hi, i as u32);
            }
        }
    }

    /// Any river segments near this bucket at all? Near-free pre-check so
    /// the caller can skip meander noise on the vast riverless majority.
    pub fn maybe_river(&self, face: usize, u: f64, v: f64) -> bool {
        !self.seg_buckets[bucket_of(face, u, v)].is_empty()
    }

    /// Nearest river segment within SEG_INFLUENCE_KM of `dir` — geometry
    /// (distance, flow) from the winner, but the water LEVEL blended across
    /// every in-influence segment by inverse closeness. Winner-take-all
    /// levels are discontinuous along the Voronoi bisector between two
    /// reaches (a hairpin bend's arms, a tributary beside the main stem),
    /// which rendered as a staircase water CLIFF running mid-channel
    /// (BUGS.md W-2). Levels along one course are continuous by export, so
    /// blending only acts where independent reaches meet — the cliff
    /// becomes a smooth ramp confined to the overlap zone, reading as
    /// rapids. Weights peak inside a channel (w -> 1 at dist 0) and die at
    /// the influence edge, so a lone segment's level is exactly its own.
    pub fn river_near(
        &self,
        face: usize,
        u: f64,
        v: f64,
        dir: DVec3,
        seed: i64,
    ) -> Option<RiverHit> {
        let ids = &self.seg_buckets[bucket_of(face, u, v)];
        if ids.is_empty() {
            return None;
        }
        let p = dir * self.radius_km;
        let mut best: Option<RiverHit> = None;
        let mut wsum = 0.0f64;
        let mut lsum = 0.0f64;
        for &id in ids {
            let s = &self.segments[id as usize];
            // Bucket padding is deliberately conservative. Cull a straight
            // reach before paying for arc-length noise when even the maximum
            // possible course displacement and edge wobble cannot bring it
            // into the influence band. This is an exact bound, not an LOD:
            // candidates capable of affecting the shared field still take
            // the identical path below.
            let a = s.a * self.radius_km;
            let ab = s.b * self.radius_km - a;
            let base_t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            let base_dist = (p - (a + ab * base_t)).length();
            if base_dist
                > SEG_INFLUENCE_KM
                    + river_tuning::MEANDER_MAX_AMPLITUDE_KM
                    + river_tuning::BANK_EDGE_WOBBLE_KM
            {
                continue;
            }
            let shape = segment_shape(s, p, self.radius_km, seed);
            let d = shape.dist_km;
            if d >= SEG_INFLUENCE_KM {
                continue;
            }
            let level = segment_level_km(s, shape.t);
            // closeness^2 so the owning channel dominates its own water
            // level, but a lower neighbouring reach still bends the surface
            // down before the bisector instead of cliffing at it
            let w = ((1.0 - d / SEG_INFLUENCE_KM) * (1.0 - d / SEG_INFLUENCE_KM)).max(1e-6);
            wsum += w;
            lsum += w * level;
            if best.as_ref().is_none_or(|b| d < b.dist_km) {
                let flow = f64::from(s.flow_m3s);
                let (base_hw, depth_km) = hydraulic_geometry(flow);
                let half_width_km = base_hw * shape.width_scale;
                let flow_gain = smoothstep(
                    river_tuning::HEADWATER_TAPER_END_M3S,
                    river_tuning::ISLAND_FULL_FLOW_M3S,
                    flow,
                );
                let visual_fall = (f64::from(s.fall_strength)
                    * river_tuning::FALL_VISUAL_STRENGTH_GAIN)
                    .min(1.0);
                let fall_strength = visual_fall * fall_foam_influence(s, shape.t);
                let fall_sheet_strength = visual_fall * fall_sheet_influence(s, shape.t);
                best = Some(RiverHit {
                    dist_km: d,
                    signed_dist_km: shape.signed_dist_km,
                    level_km: level,
                    flow,
                    flow_log10: f64::from(s.flow_log),
                    half_width_km,
                    depth_km,
                    bank_width_scale: (river_tuning::BANK_WIDTH_BASE
                        + river_tuning::BANK_WIDTH_FLOW_GAIN * flow_gain)
                        * shape.bank_scale,
                    bank_profile_power: shape.bank_profile_power.max(0.35),
                    island_field: island_field(
                        s,
                        id,
                        shape.t,
                        shape.signed_dist_km,
                        half_width_km,
                        seed,
                    ),
                    fall_strength,
                    fall_sheet_strength,
                    segment_id: id,
                    segment_t: shape.t,
                });
            }
        }
        best.map(|mut b| {
            b.level_km = lsum / wsum;
            b
        })
    }

    /// Lake candidate at `dir`. Pure geometry: the nearest lake cell (its
    /// level/salt/center/radius) plus where the query sits relative to the
    /// lake's Voronoi territory and its RIM ring (the sim's dry cells around
    /// the lake — by construction of the spill level, every rim cell's
    /// elevation is >= the lake level, so the rim is the dam).
    ///
    /// The flood decision belongs to terrain::sample, which combines this
    /// with the planet rasters. History, because this geometry has been
    /// wrong in two opposite ways: the original hard rule (flood only inside
    /// lake-cell Voronoi) left below-level noise dips just outside the
    /// footprint dry — vertical water walls at the shore (lake 414). The
    /// 586a828 fix flooded the whole 3-radius disc regardless of the rim,
    /// which let a +70 m coastal lake spill across its dam and stand against
    /// the open sea (BUGS.md W-1). The truth needs both distances, so both
    /// are returned.
    pub fn lake_at(&self, face: usize, u: f64, v: f64, dir: DVec3) -> Option<LakeHit> {
        let ids = &self.lake_buckets[bucket_of(face, u, v)];
        if ids.is_empty() {
            return None;
        }
        let p = dir * self.radius_km;
        let mut best_lake: Option<(f64, &LakeCell)> = None; // nearest true lake cell
        let mut best_rim: Option<(f64, &LakeCell)> = None; // nearest dry rim cell
        let mut d_any = f64::INFINITY; // nearest cell of either kind
        let mut any_is_lake = false;
        let mut any_rim_elev = f64::NEG_INFINITY; // rim rows carry elevation
        for &id in ids {
            let l = &self.lakes[id as usize];
            let d = (p - l.center * self.radius_km).length();
            if d < d_any {
                d_any = d;
                any_is_lake = !l.rim;
                any_rim_elev = if l.rim { l.level_km as f64 } else { f64::NEG_INFINITY };
            }
            if !l.rim && best_lake.as_ref().is_none_or(|b| d < b.0) {
                best_lake = Some((d, l));
            }
            if l.rim && best_rim.as_ref().is_none_or(|b| d < b.0) {
                best_rim = Some((d, l));
            }
        }
        match best_lake {
            // 3.4 r: hit coverage must OUTLAST the flood (bounded at 2.6 r in
            // terrain::sample) by the apron's grading reach, or the water and
            // its bank get clipped mid-basin by the search radius itself —
            // that truncation stood as a 170 m wall at 16.569 -32.262
            Some((d, l)) if d < l.radius_km as f64 * 3.4 => {
                // Exact distance to the perpendicular bisector of the two
                // cells that define the lake-vs-rim Voronoi decision. Chord
                // space is already lake_at's metric; at ~30 km cells on an
                // 8,660 km planet its difference from surface distance is
                // negligible. This is color/material metadata only.
                let boundary_dist = best_rim.map_or(f64::INFINITY, |(dr, r)| {
                    let centers_km = (l.center - r.center).length() * self.radius_km;
                    ((dr * dr - d * d).abs() / (2.0 * centers_km.max(1e-9))).max(0.0)
                });
                let mut competing_lake_boundary = f64::INFINITY;
                let mut higher_competing_level = f64::NEG_INFINITY;
                let mut higher_competing_boundary = f64::INFINITY;
                for &id in ids {
                    let c = &self.lakes[id as usize];
                    if c.rim
                        || (c.salt == l.salt
                            && c.level_km.to_bits() == l.level_km.to_bits())
                    {
                        continue;
                    }
                    let dc = (p - c.center * self.radius_km).length();
                    let centers_km = (l.center - c.center).length() * self.radius_km;
                    let boundary = ((dc * dc - d * d).abs()
                        / (2.0 * centers_km.max(1e-9)))
                        .max(0.0);
                    competing_lake_boundary = competing_lake_boundary.min(boundary);
                    if c.level_km > l.level_km && boundary < higher_competing_boundary {
                        higher_competing_level = f64::from(c.level_km);
                        higher_competing_boundary = boundary;
                    }
                }
                let past_boundary = if any_is_lake { 0.0 } else { (d - d_any).max(0.0) };
                let rim_is_dam = any_is_lake
                    || any_rim_elev + 0.03 >= l.level_km as f64;
                // distance past every dam rim's shore band, minimized: each
                // dam rim floods its own 1.15 r disc, so the apron must cone
                // out from EVERY flooded frontier, not just the lake-Voronoi
                // one (rim rows carry their elevation in level_km; only rims
                // at dam height for THIS lake's level count — a lower
                // cascade neighbor's rims must not hoist its ground)
                let mut apron_past = past_boundary;
                if !any_is_lake && apron_past > 0.0 {
                    let band = l.radius_km as f64 * 1.15;
                    for &id in ids {
                        let r = &self.lakes[id as usize];
                        if !r.rim || (r.level_km as f64) + 0.03 < l.level_km as f64 {
                            continue;
                        }
                        let dr = (p - r.center * self.radius_km).length();
                        apron_past = apron_past.min((dr - band).max(0.0));
                        if apron_past == 0.0 {
                            break;
                        }
                    }
                }
                // the flood region is (voronoi ∪ dam bands) ∩ {d < 2.6 r},
                // so the apron distance is the MAX of the two violations:
                // past the voronoi/band frontiers (computed above) and past
                // the 2.6 r lake-distance bound. Min-ing the bound in
                // instead let a far rim's band zero the distance inside a
                // phantom pool the 2.6 r rule had just removed — the apron
                // then built a 170 m dirt mesa where the fiction water was.
                apron_past = apron_past.max((d - l.radius_km as f64 * 2.6).max(0.0));
                // W-7: adjacent members of one merged lake can have
                // different radii. Flood authority remains winner-take-all,
                // but its dry apron must see a same-lake larger cell whose
                // frontier is only metres away; otherwise this distance
                // jumps kilometres at the member-cell bisector. Exact baked
                // level+salt is a unique lake key in RIV1.
                if !any_is_lake && rim_is_dam && apron_past > 0.0 {
                    for &id in ids {
                        let c = &self.lakes[id as usize];
                        if c.rim
                            || c.salt != l.salt
                            || c.level_km.to_bits() != l.level_km.to_bits()
                        {
                            continue;
                        }
                        let dc = (p - c.center * self.radius_km).length();
                        let candidate_past = (d_any - c.radius_km as f64 * 1.15)
                            .max(0.0)
                            .max(dc - c.radius_km as f64 * 2.6);
                        apron_past = apron_past.min(candidate_past);
                        if apron_past == 0.0 {
                            break;
                        }
                    }
                }
                let d_any_from_winner = d - past_boundary;
                let union_margin = if any_is_lake {
                    boundary_dist
                } else if rim_is_dam {
                    l.radius_km as f64 * 1.15 - d_any_from_winner
                } else {
                    -past_boundary
                };
                let flood_edge_margin = (l.radius_km as f64 * 2.6 - d)
                    .min(union_margin)
                    .min(competing_lake_boundary)
                    .max(0.0);
                Some(LakeHit {
                    level_km: l.level_km as f64,
                    salt: l.salt,
                    d_lake_km: d,
                    lake_center: l.center,
                    radius_km: l.radius_km as f64,
                    boundary_dist_km: boundary_dist,
                    in_lake_voronoi: any_is_lake,
                    past_boundary_km: past_boundary,
                    apron_past_km: apron_past,
                    flood_edge_margin_km: flood_edge_margin,
                    competing_lake_boundary_km: competing_lake_boundary,
                    higher_competing_level_km: higher_competing_level,
                    higher_competing_boundary_km: higher_competing_boundary,
                    rim_is_dam,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_segment(
        radius_km: f64,
        angle: f64,
        flow_m3s: f64,
        level_a_km: f64,
        level_b_km: f64,
    ) -> Segment {
        let a = DVec3::X;
        let b = DVec3::new(angle.cos(), angle.sin(), 0.0);
        let length_km = angle * radius_km;
        let tangent_a = endpoint_tangent(a, b, false);
        let tangent_b = endpoint_tangent(a, b, true);
        Segment {
            a,
            b,
            flow_log: (1.0 + flow_m3s).log10() as f32,
            level_a_km: level_a_km as f32,
            level_b_km: level_b_km as f32,
            length_km: length_km as f32,
            flow_m3s: flow_m3s as f32,
            fall_strength: fall_strength(
                flow_m3s,
                (level_a_km - level_b_km).max(0.0),
                length_km,
            ) as f32,
            reach_id: 0,
            reach_arc_start_km: 0.0,
            reach_length_km: length_km,
            tangent_a,
            tangent_b,
            meander_amplitude_km: meander_amplitude_km(flow_m3s) as f32,
        }
    }

    #[test]
    fn bank_field_is_seed_position_deterministic_and_reach_endpoint_pinned() {
        let radius = 1_000.0;
        let s = fixture_segment(radius, 0.04, 8_000.0, 0.120, 0.110);
        let p = (s.a + s.b).normalize() * radius + DVec3::Z * 0.04;
        let a = segment_shape(&s, p, radius, 42);
        let b = segment_shape(&s, p, radius, 42);
        assert_eq!(a.t.to_bits(), b.t.to_bits());
        assert_eq!(a.dist_km.to_bits(), b.dist_km.to_bits());
        assert_eq!(a.width_scale.to_bits(), b.width_scale.to_bits());
        assert_eq!(a.bank_scale.to_bits(), b.bank_scale.to_bits());
        assert_eq!(a.bank_profile_power.to_bits(), b.bank_profile_power.to_bits());
        assert_eq!(meander_offset_km(&s, 0.0, 42), 0.0);
        assert_eq!(meander_offset_km(&s, 1.0, 42), 0.0);
        assert_ne!(
            meander_offset_km(&s, 0.43, 42).to_bits(),
            meander_offset_km(&s, 0.43, 43).to_bits(),
        );
    }

    #[test]
    fn reach_meander_crosses_internal_graph_node_continuously() {
        let radius = 1_000.0;
        let mut first = fixture_segment(radius, 0.04, 8_000.0, 0.120, 0.116);
        let mut second = fixture_segment(radius, 0.04, 8_100.0, 0.116, 0.112);
        second.a = first.b;
        second.b = DVec3::new(0.08f64.cos(), 0.08f64.sin(), 0.0);
        second.length_km = 40.0;
        second.reach_length_km = 40.0;
        second.tangent_a = endpoint_tangent(second.a, second.b, false);
        second.tangent_b = endpoint_tangent(second.a, second.b, true);
        first.fall_strength = 0.0;
        second.fall_strength = 0.0;

        let mut index = RiverIndex::empty(radius);
        index.segments = vec![first, second];
        index.build_reaches();
        let a = &index.segments[0];
        let b = &index.segments[1];
        assert_eq!(a.reach_id, b.reach_id);
        assert_eq!(b.reach_arc_start_km, f64::from(a.length_km));
        assert_eq!(a.reach_length_km, b.reach_length_km);

        let oa = meander_offset_km(a, 1.0, 42);
        let ob = meander_offset_km(b, 0.0, 42);
        assert_eq!(oa.to_bits(), ob.to_bits());
        assert!(oa.abs() > 1e-6, "internal graph node was incorrectly pinned");
        let (ca, _, xa) = segment_frame(a, 1.0);
        let (cb, _, xb) = segment_frame(b, 0.0);
        let pa = ca + xa * (oa / radius);
        let pb = cb + xb * (ob / radius);
        assert!(pa.distance(pb) < 1e-12, "displaced reach cracked at its internal node");
        assert_eq!(meander_offset_km(a, 0.0, 42), 0.0);
        assert_eq!(meander_offset_km(b, 1.0, 42), 0.0);
    }

    #[test]
    fn island_bar_never_changes_monotone_downstream_level() {
        let radius = 1_000.0;
        let mut s = fixture_segment(radius, 0.04, 12_000.0, 0.120, 0.112);
        // This is a low-gradient reach, not a fall.
        s.fall_strength = 0.0;
        let seed = (0..512)
            .find(|&seed| braided_segment(&s, 91, seed))
            .expect("fixture must find a deterministic braided lottery winner");
        let (hw, _) = hydraulic_geometry(f64::from(s.flow_m3s));
        let mut previous = segment_level_km(&s, 0.0);
        let mut saw_bar = false;
        for i in 1..=2_000 {
            let t = i as f64 / 2_000.0;
            let bar = island_field(&s, 91, t, 0.0, hw, seed);
            saw_bar |= bar > 0.5;
            let level = segment_level_km(&s, t);
            assert!(level <= previous + 1e-12, "level rose at t={t}: {previous} -> {level}");
            // Across-course bar evaluation has no hydraulic output at all.
            assert_eq!(level.to_bits(), segment_level_km(&s, t).to_bits());
            previous = level;
        }
        assert!(saw_bar, "braided fixture never exercised an emergent-bar field");
        assert_eq!(segment_level_km(&s, 0.0).to_bits(), f64::from(s.level_a_km).to_bits());
        assert_eq!(segment_level_km(&s, 1.0).to_bits(), f64::from(s.level_b_km).to_bits());
    }

    #[test]
    fn fall_site_detection_requires_flow_drop_and_gradient_and_stays_monotone() {
        let radius = 1_000.0;
        let fall = fixture_segment(radius, 0.02, 2_000.0, 0.240, 0.020);
        let slow = fixture_segment(radius, 0.02, 200.0, 0.240, 0.020);
        let flat = fixture_segment(radius, 0.08, 2_000.0, 0.080, 0.060);
        assert!(fall.fall_strength > 0.0);
        assert_eq!(slow.fall_strength, 0.0);
        assert_eq!(flat.fall_strength, 0.0);
        assert_eq!(segment_level_km(&fall, 0.0), f64::from(fall.level_a_km));
        assert_eq!(segment_level_km(&fall, 1.0), f64::from(fall.level_b_km));
        let mut previous = segment_level_km(&fall, 0.0);
        for i in 1..=1_000 {
            let level = segment_level_km(&fall, i as f64 / 1_000.0);
            assert!(level <= previous + 1e-12);
            previous = level;
        }
        let half = fall_half_t(f64::from(fall.length_km));
        assert!(
            segment_level_km(&fall, 0.5 - half)
                > segment_level_km(&fall, 0.5 + half)
        );
        let drawdown_t = river_tuning::FALL_DRAWDOWN_REACH_KM
            / f64::from(fall.length_km);
        assert!(
            segment_level_km(&fall, 0.5 - drawdown_t)
                > segment_level_km(&fall, 0.5 - half),
            "approach drawdown did not precede the main sheet"
        );
        let approach_t = 0.5 - 1.0 / f64::from(fall.length_km);
        assert!(fall_foam_influence(&fall, approach_t) > 0.0);
        assert_eq!(fall_sheet_influence(&fall, approach_t), 0.0);
    }
}

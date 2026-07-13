//! Deterministic lunar terrain shared by adaptive mesh tiles and voxel columns.
//!
//! [`MoonGenerator::sample`] is the one terrain/material law: a pure function
//! of unit direction and world seed. Craters are not stored in a global fold.
//! Ten size octaves use deterministic jittered cube-sphere cells; a sample
//! visits a bounded subset of its 3x3 neighborhood at each octave, then the
//! 3x3 neighborhood in one
//! sparse Tycho-size ray-carrier lattice. The hard budget is therefore
//! `10 * 9 + 9 = 99` cell visits per sample, independent of the hundreds of
//! thousands of conceptual sub-kilometre impacts over the whole sphere.
//!
//! Octaves compose coarse-to-fine. That supplies a deterministic impact order
//! and lets fine saturation craters erase older basin floors and ray material.
//! Maria are the only pre-expanded records: a small seeded spawn field places
//! clustered large cratons and smaller plains around their edges.

use glam::DVec3;
use std::collections::{BTreeMap, BTreeSet};

use crate::noise::{fbm, gradient_noise, ridged_band};
use crate::planet::{FACES, face_dir, face_from_dir};
use crate::terrain::{TILE_QUADS, TileKey, TileMesh, Vertex};

pub mod tuning {
    /// Radius halves and conceptual count grows about fourfold per octave.
    pub const CRATER_OCTAVES: usize = 10;
    pub const CRATER_OCCUPANCY: f64 = 0.31;
    /// Radius is `(fraction / grid_width) * cube_metric` radians.
    /// The ratio is deliberately greater than two, so adjacent half-scale
    /// octaves overlap instead of leaving a visible size comb. Log-uniform
    /// draws preserve equal opportunity across each interval; occupancy is
    /// reduced from 0.34 to 0.31 to keep covered area and N(D) near D^-2.
    pub const CRATER_RADIUS_CELL: (f64, f64) = (0.20, 0.46);
    pub const CRATER_OUTER_RADII: f64 = 1.78;
    /// Almost the full +/- half-cell interval. One cell still owns each draw,
    /// but its center no longer advertises the lattice row.
    pub const CRATER_JITTER: f64 = 0.47;
    pub const MAX_CRATER_ELONGATION: f64 = 1.20;
    /// At the worst cube metric, the largest elongated outer rim reaches
    /// `0.46/2 * 1.78 * 1.20 * 1.08 = 0.530` cells from its center. Full-cell
    /// jitter can therefore extend 0.500 cells through a cell edge; 0.54
    /// retains a conservative cut-line margin while the 3x3 window remains
    /// sound (`0.47 + 0.530 < 1.5` cells from the nominal center).
    pub const CRATER_CELL_MARGIN: f64 = 0.54;
    /// The five coarse visible bins are immutable indexed cell caches. This
    /// removes repeated feature expansion during tile fill without creating a
    /// global fold; lookup still visits only the sample's nearby lattice cells.
    pub const CACHED_CRATER_OCTAVES: usize = 5;
    pub const MAX_CRATER_CELL_VISITS: usize = CRATER_OCTAVES * 9;

    /// A separate sparse mid-size cohort carries all rays. Large basin
    /// octaves never enter this lattice.
    pub const RAY_GRID: u32 = 3;
    pub const RAY_OCCUPANCY: f64 = 0.20;
    pub const RAY_NEIGHBOR_RADIUS: i32 = 1;
    pub const RAY_CELL_VISITS: usize = 9;
    pub const RAY_LINES: usize = 24;
    pub const RAY_MAX_RADII: f64 = 14.0;
    pub const MAX_LATTICE_CELL_VISITS: usize = MAX_CRATER_CELL_VISITS + RAY_CELL_VISITS;

    pub const LARGE_MARE_GRID: u32 = 3;
    pub const MID_MARE_GRID: u32 = 7;

    /// With 32 quads/tile and level 14 this reaches about seven-metre vertex
    /// spacing on the configured moon.
    pub const LOD_ERROR_TARGET: f64 = 0.18;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonSample {
    /// Signed relief as a fraction of the physical moon radius.
    pub height_ratio: f64,
    /// Scalar reflectance before the renderer's lunar/copper tint.
    pub albedo: f64,
    /// 0 highlands/rough ejecta, 1 smooth mare or proximal ray halo.
    pub smoothness: f64,
    /// Surviving bright ray material (0..1).
    pub ray: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoonMaterial {
    Highland,
    Maria,
    Ray,
}

/// Read-only evidence probe for deterministic mission scripts. Rendering and
/// voxel generation never consume this enumeration; both continue to call
/// [`MoonGenerator::sample`] directly.
#[derive(Clone, Copy, Debug)]
pub struct MoonFeatureProbe {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub radius_deg: f64,
}

impl MoonSample {
    pub fn material(self) -> MoonMaterial {
        if self.ray >= 0.08 {
            MoonMaterial::Ray
        } else if self.smoothness >= 0.45 {
            MoonMaterial::Maria
        } else {
            MoonMaterial::Highland
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct MareCluster {
    center: DVec3,
    radius: f64,
    strength: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct Mare {
    center: DVec3,
    major: DVec3,
    minor: DVec3,
    radius: f64,
    elongation: f64,
    albedo_target: f64,
    floor_drop: f64,
    phase: f64,
    seed: i64,
    large: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct CellKey {
    face: u8,
    i: u16,
    j: u16,
}

impl CellKey {
    const ZERO: Self = Self {
        face: 0,
        i: 0,
        j: 0,
    };
}

#[derive(Clone, Copy, Debug)]
struct Crater {
    center: DVec3,
    major: DVec3,
    minor: DVec3,
    radius: f64,
    elongation: f64,
    depth_ratio: f64,
    roughness: f64,
    rim_phase: f64,
    rim_lobes: f64,
    fresh_albedo: f64,
    floor_tone_delta: f64,
    /// Local low datum composed from terrain coarser than this impact. This is
    /// spatially constant across the bowl, unlike the inherited relief that
    /// the impact is meant to erase.
    floor_datum: f64,
    octave: u8,
    age_key: u64,
    noise_seed: i64,
}

impl Crater {
    const EMPTY: Self = Self {
        center: DVec3::ZERO,
        major: DVec3::X,
        minor: DVec3::Y,
        radius: 0.0,
        elongation: 1.0,
        depth_ratio: 0.0,
        roughness: 0.0,
        rim_phase: 0.0,
        rim_lobes: 0.0,
        fresh_albedo: 0.0,
        floor_tone_delta: 0.0,
        floor_datum: 0.0,
        octave: 0,
        age_key: 0,
        noise_seed: 0,
    };
}

#[derive(Clone, Copy)]
struct BaseSample {
    height: f64,
    albedo: f64,
    smoothness: f64,
}

#[derive(Clone, Copy)]
struct SurfaceState {
    height: f64,
    albedo: f64,
    smoothness: f64,
    ray: f64,
}

#[derive(Clone)]
pub struct MoonGenerator {
    seed: i64,
    clusters: [MareCluster; 4],
    maria: Vec<Mare>,
    crater_cache: Vec<Vec<Option<Crater>>>,
    ray_cache: Vec<Option<Crater>>,
    legacy_floors: bool,
}

#[derive(Clone, Copy)]
struct PrefetchedCrater {
    crater: Crater,
    impact: BaseSample,
}

/// Crater candidates shared by every sample in one chunk/tile build. The
/// union is gathered once from the build footprint; applying a candidate
/// outside its exact reach is a no-op in the unchanged crater/ray equations.
struct MoonCraterPrefetch {
    octaves: [Vec<PrefetchedCrater>; tuning::CRATER_OCTAVES],
    ray_carriers: Vec<PrefetchedCrater>,
    samples: Vec<MoonSamplePrefetch>,
    skip_octave: Option<usize>,
}

struct MoonSampleKeys {
    octaves: [[CellKey; tuning::RAY_CELL_VISITS]; tuning::CRATER_OCTAVES],
    octave_counts: [u8; tuning::CRATER_OCTAVES],
    rays: [CellKey; tuning::RAY_CELL_VISITS],
    ray_count: u8,
}

impl MoonSampleKeys {
    fn empty() -> Self {
        Self {
            octaves: [[CellKey::ZERO; tuning::RAY_CELL_VISITS]; tuning::CRATER_OCTAVES],
            octave_counts: [0; tuning::CRATER_OCTAVES],
            rays: [CellKey::ZERO; tuning::RAY_CELL_VISITS],
            ray_count: 0,
        }
    }
}

struct MoonSamplePrefetch {
    octaves: [[u32; tuning::RAY_CELL_VISITS]; tuning::CRATER_OCTAVES],
    octave_counts: [u8; tuning::CRATER_OCTAVES],
    rays: [u32; tuning::RAY_CELL_VISITS],
    ray_count: u8,
}

impl MoonSamplePrefetch {
    fn empty() -> Self {
        Self {
            octaves: [[0; tuning::RAY_CELL_VISITS]; tuning::CRATER_OCTAVES],
            octave_counts: [0; tuning::CRATER_OCTAVES],
            rays: [0; tuning::RAY_CELL_VISITS],
            ray_count: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct SeedStream(u64);

impl SeedStream {
    fn new(seed: i64, salt: u64) -> Self {
        Self((seed as u64) ^ salt)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        splitmix64(self.0)
    }

    fn unit(&mut self) -> f64 {
        unit_from_hash(self.next_u64())
    }

    fn direction(&mut self) -> DVec3 {
        let z = self.unit() * 2.0 - 1.0;
        let a = self.unit() * std::f64::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        DVec3::new(r * a.cos(), r * a.sin(), z)
    }
}

#[inline]
fn splitmix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn unit_from_hash(hash: u64) -> f64 {
    ((hash >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

#[inline]
fn packed_unit(hash: u64, lane: u32) -> f64 {
    ((hash >> (lane * 16)) & 0xFFFF) as f64 * (1.0 / 65_536.0)
}

#[inline]
fn feature_hash(seed: i64, key: CellKey, domain: u64, channel: u64) -> u64 {
    let packed = (key.face as u64) << 58
        ^ (key.i as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ (key.j as u64).wrapping_mul(0xA5A3_564E_27F8_862D);
    splitmix64((seed as u64) ^ domain ^ packed ^ channel.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

#[inline]
fn feature_unit(seed: i64, key: CellKey, domain: u64, channel: u64) -> f64 {
    unit_from_hash(feature_hash(seed, key, domain, channel))
}

fn tangent_axes(center: DVec3, angle: f64) -> (DVec3, DVec3) {
    let reference = if center.z.abs() < 0.88 {
        DVec3::Z
    } else {
        DVec3::X
    };
    let a = (reference - center * reference.dot(center)).normalize();
    let b = center.cross(a).normalize();
    let (s, c) = angle.sin_cos();
    let major = a * c + b * s;
    let minor = center.cross(major).normalize();
    (major, minor)
}

fn lat_lon_dir(lat_deg: f64, lon_deg: f64) -> DVec3 {
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
}

fn log_range(lo: f64, hi: f64, u: f64) -> f64 {
    lo * (hi / lo).powf(u)
}

#[inline]
fn crater_radius_fraction(octave: usize, u: f64) -> f64 {
    // Preserve the shipped absolute largest/smallest bounds at the two ends;
    // only interior octave bands widen enough to overlap their neighbors.
    let lo = if octave + 1 == tuning::CRATER_OCTAVES {
        0.22
    } else {
        tuning::CRATER_RADIUS_CELL.0
    };
    let hi = if octave == 0 {
        0.38
    } else {
        tuning::CRATER_RADIUS_CELL.1
    };
    log_range(lo, hi, u)
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    if (b - a).abs() < f64::EPSILON {
        return (x >= b) as u8 as f64;
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn smooth_min(a: f64, b: f64, width: f64) -> f64 {
    0.5 * (a + b - ((a - b) * (a - b) + width * width).sqrt())
}

/// Elliptical angular radius and bearing around a feature center.
fn feature_coords(
    direction: DVec3,
    center: DVec3,
    major: DVec3,
    minor: DVec3,
    angular_radius: f64,
    elongation: f64,
) -> (f64, f64, f64) {
    let dot = direction.dot(center).clamp(-1.0, 1.0);
    let theta = dot.acos();
    if theta < 1e-14 {
        return (0.0, 0.0, theta);
    }
    let tangent_len = (1.0 - dot * dot).max(0.0).sqrt().max(1e-14);
    let x = direction.dot(major) / tangent_len;
    let y = direction.dot(minor) / tangent_len;
    let q = theta / angular_radius * ((x / elongation).powi(2) + (y * elongation).powi(2)).sqrt();
    (q, y.atan2(x), theta)
}

fn lattice_point(
    seed: i64,
    key: CellKey,
    grid: u32,
    domain: u64,
    jitter: f64,
) -> (DVec3, f64, f64) {
    let jx = (feature_unit(seed, key, domain, 1) * 2.0 - 1.0) * jitter;
    let jy = (feature_unit(seed, key, domain, 2) * 2.0 - 1.0) * jitter;
    let u = -1.0 + (key.i as f64 + 0.5 + jx) * 2.0 / grid as f64;
    let v = -1.0 + (key.j as f64 + 0.5 + jy) * 2.0 / grid as f64;
    (face_dir(key.face as usize, u, v), u, v)
}

/// Hot crater path: one mixed word supplies occupancy, both jitters, and
/// octave-local radius. Sixteen bits per lane is far below any renderable
/// spatial precision at these scales and avoids three extra hashes/candidate.
fn packed_lattice_point(
    key: CellKey,
    grid: u32,
    primary: u64,
    jitter: f64,
) -> (DVec3, f64, f64, f64) {
    let (u, v) = packed_lattice_uv(key, grid, primary, jitter);
    let (axis, right, up) = FACES[key.face as usize];
    let inverse_length = (1.0 + u * u + v * v).sqrt().recip();
    (
        (axis + u * right + v * up) * inverse_length,
        u,
        v,
        inverse_length,
    )
}

#[inline]
fn packed_lattice_uv(key: CellKey, grid: u32, primary: u64, jitter: f64) -> (f64, f64) {
    let jx = (packed_unit(primary, 1) * 2.0 - 1.0) * jitter;
    let jy = (packed_unit(primary, 2) * 2.0 - 1.0) * jitter;
    let u = -1.0 + (key.i as f64 + 0.5 + jx) * 2.0 / grid as f64;
    let v = -1.0 + (key.j as f64 + 0.5 + jy) * 2.0 / grid as f64;
    (u, v)
}

fn cell_index(coord: f64, grid: u32) -> u16 {
    (((coord + 1.0) * 0.5 * grid as f64).floor() as i32).clamp(0, grid as i32 - 1) as u16
}

/// Canonical nearby cube-sphere cells. Out-of-face offsets are projected to
/// their owning face, then the fixed output is deduplicated and sorted.
fn nearby_cells(
    direction: DVec3,
    grid: u32,
    radius: i32,
    out: &mut [CellKey; tuning::RAY_CELL_VISITS],
) -> usize {
    if grid == 1 {
        for face in 0..6 {
            out[face] = CellKey {
                face: face as u8,
                i: 0,
                j: 0,
            };
        }
        return 6;
    }

    let (face, u, v) = face_from_dir(direction);
    let bi = cell_index(u, grid) as i32;
    let bj = cell_index(v, grid) as i32;
    let mut len = 0usize;
    for dj in -radius..=radius {
        for di in -radius..=radius {
            let ri = bi + di;
            let rj = bj + dj;
            let key = if (0..grid as i32).contains(&ri) && (0..grid as i32).contains(&rj) {
                CellKey {
                    face: face as u8,
                    i: ri as u16,
                    j: rj as u16,
                }
            } else {
                let cu = -1.0 + (ri as f64 + 0.5) * 2.0 / grid as f64;
                let cv = -1.0 + (rj as f64 + 0.5) * 2.0 / grid as f64;
                let (owner, ou, ov) = face_from_dir(face_dir(face, cu, cv));
                CellKey {
                    face: owner as u8,
                    i: cell_index(ou, grid),
                    j: cell_index(ov, grid),
                }
            };
            if !out[..len].contains(&key) {
                out[len] = key;
                len += 1;
            }
        }
    }
    len
}

/// Ordinary crater neighborhood with a conservative geometric edge test.
/// Interior samples do not pay for cells whose jittered feature cannot reach
/// them; seams still project to canonical adjacent-face owners.
fn nearby_crater_cells(
    locator: (usize, f64, f64),
    grid: u32,
    out: &mut [CellKey; tuning::RAY_CELL_VISITS],
) -> usize {
    if grid == 1 {
        for face in 0..6 {
            out[face] = CellKey {
                face: face as u8,
                i: 0,
                j: 0,
            };
        }
        return 6;
    }
    let (face, u, v) = locator;
    let gu = (u + 1.0) * 0.5 * grid as f64;
    let gv = (v + 1.0) * 0.5 * grid as f64;
    let bi = gu.floor().clamp(0.0, grid as f64 - 1.0) as i32;
    let bj = gv.floor().clamp(0.0, grid as f64 - 1.0) as i32;
    let fu = gu - gu.floor();
    let fv = gv - gv.floor();
    let mut len = 0usize;
    for dj in -1..=1 {
        if (dj < 0 && fv >= tuning::CRATER_CELL_MARGIN)
            || (dj > 0 && fv <= 1.0 - tuning::CRATER_CELL_MARGIN)
        {
            continue;
        }
        for di in -1..=1 {
            if (di < 0 && fu >= tuning::CRATER_CELL_MARGIN)
                || (di > 0 && fu <= 1.0 - tuning::CRATER_CELL_MARGIN)
            {
                continue;
            }
            let ri = bi + di;
            let rj = bj + dj;
            let key = if (0..grid as i32).contains(&ri) && (0..grid as i32).contains(&rj) {
                CellKey {
                    face: face as u8,
                    i: ri as u16,
                    j: rj as u16,
                }
            } else {
                let cu = -1.0 + (ri as f64 + 0.5) * 2.0 / grid as f64;
                let cv = -1.0 + (rj as f64 + 0.5) * 2.0 / grid as f64;
                let (owner, ou, ov) = face_from_dir(face_dir(face, cu, cv));
                CellKey {
                    face: owner as u8,
                    i: cell_index(ou, grid),
                    j: cell_index(ov, grid),
                }
            };
            if !out[..len].contains(&key) {
                out[len] = key;
                len += 1;
            }
        }
    }
    len
}

impl MoonGenerator {
    pub fn new(seed: i64) -> Self {
        let mut stream = SeedStream::new(seed, 0x4D41_5245_5F56_325F);
        const ANCHORS: [(f64, f64); 3] = [(-16.0, -18.0), (20.0, 24.0), (5.0, 61.0)];
        let clusters = std::array::from_fn(|i| {
            let center = if i < ANCHORS.len() {
                let lat = ANCHORS[i].0 + (stream.unit() - 0.5) * 14.0;
                let lon = ANCHORS[i].1 + (stream.unit() - 0.5) * 18.0;
                lat_lon_dir(lat, lon)
            } else {
                stream.direction()
            };
            MareCluster {
                center,
                radius: (24.0 + 12.0 * stream.unit()).to_radians(),
                strength: 0.82 + 0.18 * stream.unit(),
            }
        });

        let mut maria = Vec::new();
        for &(large, grid, domain) in &[
            (true, tuning::LARGE_MARE_GRID, 0x4D41_5245_5F4C_4152),
            (false, tuning::MID_MARE_GRID, 0x4D41_5245_5F4D_4944),
        ] {
            for face in 0..6u8 {
                for j in 0..grid as u16 {
                    for i in 0..grid as u16 {
                        let key = CellKey { face, i, j };
                        let (center, _, _) = lattice_point(seed, key, grid, domain, 0.31);
                        let spawn = mare_spawn_field(&clusters, center, seed);
                        let probability = if large {
                            0.025 + 0.70 * smoothstep(0.30, 0.80, spawn)
                        } else {
                            let edge = (-((spawn - 0.42) / 0.24).powi(2)).exp();
                            0.018 + 0.20 * edge + 0.08 * smoothstep(0.58, 0.92, spawn)
                        };
                        if feature_unit(seed, key, domain, 3) >= probability {
                            continue;
                        }
                        let phase = feature_unit(seed, key, domain, 4) * std::f64::consts::TAU;
                        let (major, minor) = tangent_axes(center, phase);
                        // The two spawn families overlap broadly in size.
                        // Their different rarity fields still make great
                        // maria rare, without a hard 7-degree size class.
                        let radius_deg = if large {
                            log_range(5.5, 17.0, feature_unit(seed, key, domain, 5))
                        } else {
                            log_range(1.7, 8.5, feature_unit(seed, key, domain, 5))
                        };
                        maria.push(Mare {
                            center,
                            major,
                            minor,
                            radius: radius_deg.to_radians(),
                            elongation: if large {
                                1.05 + 0.42 * feature_unit(seed, key, domain, 6)
                            } else {
                                1.0 + 0.30 * feature_unit(seed, key, domain, 6)
                            },
                            albedo_target: if large {
                                0.315 + 0.020 * feature_unit(seed, key, domain, 7)
                            } else {
                                0.325 + 0.022 * feature_unit(seed, key, domain, 7)
                            },
                            floor_drop: if large {
                                0.00012 + 0.00024 * feature_unit(seed, key, domain, 8)
                            } else {
                                0.00004 + 0.00012 * feature_unit(seed, key, domain, 8)
                            },
                            phase,
                            seed: feature_hash(seed, key, domain, 9) as i64,
                            large,
                        });
                    }
                }
            }
        }

        let mut moon = Self {
            seed,
            clusters,
            maria,
            crater_cache: Vec::with_capacity(tuning::CACHED_CRATER_OCTAVES),
            ray_cache: Vec::new(),
            legacy_floors: std::env::var_os("TRI_MOON_LEGACY_FLOORS").is_some(),
        };
        for octave in 0..tuning::CACHED_CRATER_OCTAVES {
            let grid = 1u32 << octave;
            let mut bin = Vec::with_capacity(6 * grid as usize * grid as usize);
            for face in 0..6u8 {
                for j in 0..grid as u16 {
                    for i in 0..grid as u16 {
                        bin.push(moon.generate_crater_from_cell(
                            octave,
                            CellKey { face, i, j },
                            None,
                        ));
                    }
                }
            }
            moon.crater_cache.push(bin);
        }
        moon.ray_cache = Vec::with_capacity(
            6 * tuning::RAY_GRID as usize * tuning::RAY_GRID as usize,
        );
        for face in 0..6u8 {
            for j in 0..tuning::RAY_GRID as u16 {
                for i in 0..tuning::RAY_GRID as u16 {
                    let carrier =
                        moon.generate_ray_carrier_from_cell(CellKey { face, i, j }, None);
                    moon.ray_cache.push(carrier);
                }
            }
        }
        moon
    }

    pub fn seed(&self) -> i64 {
        self.seed
    }

    pub fn mare_counts(&self) -> (usize, usize) {
        let large = self.maria.iter().filter(|m| m.large).count();
        (large, self.maria.len() - large)
    }

    /// Exact occupied-cell counts on canonical cube face 0. This fixed probe
    /// region is cheap enough for evidence tooling and directly exposes the
    /// octave power law without enumerating the whole finest sphere.
    pub fn crater_probe_counts(&self) -> Vec<usize> {
        (0..tuning::CRATER_OCTAVES)
            .map(|octave| {
                let grid = 1u32 << octave;
                let domain = 0x4352_4154_4552_0000 ^ octave as u64;
                let mut count = 0usize;
                for j in 0..grid as u16 {
                    for i in 0..grid as u16 {
                        let key = CellKey { face: 0, i, j };
                        let primary = feature_hash(self.seed, key, domain, 0);
                        if octave == 0 {
                            count += self.crater_from_cell(octave, key, None).is_some() as usize;
                        } else {
                            count += (unit_from_hash(primary) < tuning::CRATER_OCCUPANCY) as usize;
                        }
                    }
                }
                count
            })
            .collect()
    }

    /// Exact generated radii on one canonical cube face, used by the headless
    /// histogram evidence. This enumerates records, never terrain samples, so
    /// the finest face remains inexpensive and cannot affect render behavior.
    pub fn crater_radius_samples_on_face(&self, face: u8) -> Vec<f64> {
        let mut radii = Vec::new();
        for octave in 0..tuning::CRATER_OCTAVES {
            let grid = 1u32 << octave;
            let domain = 0x4352_4154_4552_0000 ^ octave as u64;
            for j in 0..grid as u16 {
                for i in 0..grid as u16 {
                    let key = CellKey { face, i, j };
                    let primary = feature_hash(self.seed, key, domain, 0);
                    let occupancy = unit_from_hash(primary);
                    if octave > 0 && occupancy >= tuning::CRATER_OCCUPANCY {
                        continue;
                    }
                    let (center, _, _, inverse_length) =
                        packed_lattice_point(key, grid, primary, tuning::CRATER_JITTER);
                    if octave == 0 {
                        let spawn = mare_spawn_field(&self.clusters, center, self.seed);
                        if occupancy >= 0.42 + 0.32 * spawn {
                            continue;
                        }
                    }
                    let fraction = crater_radius_fraction(octave, packed_unit(primary, 3));
                    let cube_metric = inverse_length * inverse_length;
                    radii.push((fraction / grid as f64 * cube_metric).to_degrees());
                }
            }
        }
        radii
    }

    /// Young visible craters whose inner bowls cross an older crater crest.
    /// This deterministic evidence probe is the direct regression case for
    /// inherited-rim flattening.
    pub fn rim_straddling_crater_probes(&self) -> Vec<MoonFeatureProbe> {
        let mut probes = Vec::new();
        let mut keys = [CellKey::ZERO; tuning::RAY_CELL_VISITS];
        for octave in 1..tuning::CACHED_CRATER_OCTAVES {
            let grid = 1u32 << octave;
            for face in 0..6u8 {
                for j in 0..grid as u16 {
                    for i in 0..grid as u16 {
                        let Some(child) =
                            self.crater_from_cell(octave, CellKey { face, i, j }, None)
                        else {
                            continue;
                        };
                        let locator = face_from_dir(child.center);
                        let mut straddles = false;
                        for older_octave in 0..octave {
                            let count = nearby_crater_cells(
                                locator,
                                1u32 << older_octave,
                                &mut keys,
                            );
                            for &key in &keys[..count] {
                                let Some(parent) = self.crater_from_cell(
                                    older_octave,
                                    key,
                                    Some((child.center, locator)),
                                ) else {
                                    continue;
                                };
                                let (parent_q, _, _) = feature_coords(
                                    child.center,
                                    parent.center,
                                    parent.major,
                                    parent.minor,
                                    parent.radius,
                                    parent.elongation,
                                );
                                let child_span =
                                    child.radius * 0.64 / parent.radius.max(child.radius);
                                if (parent_q - 1.0).abs() < child_span {
                                    straddles = true;
                                    break;
                                }
                            }
                            if straddles {
                                break;
                            }
                        }
                        if straddles {
                            probes.push(feature_probe(child));
                        }
                    }
                }
            }
        }
        probes.sort_by(|a, b| b.radius_deg.total_cmp(&a.radius_deg));
        probes
    }

    pub fn ray_carrier_probes(&self) -> Vec<MoonFeatureProbe> {
        let mut probes = Vec::new();
        for face in 0..6u8 {
            for j in 0..tuning::RAY_GRID as u16 {
                for i in 0..tuning::RAY_GRID as u16 {
                    if let Some(carrier) = self.ray_carrier_from_cell(CellKey { face, i, j }, None)
                    {
                        probes.push(feature_probe(carrier));
                    }
                }
            }
        }
        probes
    }

    pub fn largest_crater_probes(&self) -> Vec<MoonFeatureProbe> {
        (0..6u8)
            .filter_map(|face| {
                self.crater_from_cell(0, CellKey { face, i: 0, j: 0 }, None)
                    .map(feature_probe)
            })
            .collect()
    }

    pub fn mare_edge_crater_probes(&self) -> Vec<MoonFeatureProbe> {
        let mut probes = Vec::new();
        for octave in 1..=4 {
            let grid = 1u32 << octave;
            for face in 0..6u8 {
                for j in 0..grid as u16 {
                    for i in 0..grid as u16 {
                        let Some(crater) =
                            self.crater_from_cell(octave, CellKey { face, i, j }, None)
                        else {
                            continue;
                        };
                        let a = crater.radius * 0.82;
                        let p0 = crater.center * a.cos() + crater.major * a.sin();
                        let p1 = crater.center * a.cos() - crater.major * a.sin();
                        let m0 = self.base_surface(p0).smoothness;
                        let m1 = self.base_surface(p1).smoothness;
                        if (m0 > 0.62 && m1 < 0.20) || (m1 > 0.62 && m0 < 0.20) {
                            probes.push(feature_probe(crater));
                        }
                    }
                }
            }
        }
        probes.sort_by(|a, b| b.radius_deg.total_cmp(&a.radius_deg));
        probes
    }

    fn base_surface(&self, direction: DVec3) -> BaseSample {
        let broad = fbm(direction, 4, 1.15, self.seed.wrapping_add(1_101));
        let soft_ridges = ridged_band(direction, 0, 2, 3.2, self.seed.wrapping_add(1_337));
        let local_soft = fbm(direction, 2, 74.0, self.seed.wrapping_add(1_619));
        let mut height = broad * 0.00058 + soft_ridges * 0.00010 + local_soft * 0.000014;
        let albedo_noise = fbm(direction, 3, 2.1, self.seed.wrapping_add(2_003));
        let grain = gradient_noise(direction * 17.0, self.seed.wrapping_add(2_111));
        let fine_grain = gradient_noise(direction * 92.0, self.seed.wrapping_add(2_303));
        let mut albedo = 0.695 + 0.064 * albedo_noise + 0.020 * grain + 0.017 * fine_grain;
        let mut smoothness: f64 = 0.0;

        let mut mare_mask = 0.0f64;
        let mut mare_albedo = 1.0f64;
        let mut mare_smooth = 0.0f64;
        for mare in &self.maria {
            // The reach cutoff must cover the mask's WORST-CASE support:
            // `irregular` bottoms out at 0.665, stretching the height
            // feather to q = 1.10/0.665 = 1.66. The old 1.24 cutoff sliced
            // through live mask values and dropped up to ~20% of the floor
            // in one step - Andrew's 492 m "discrete cliff" arcs, with the
            // sin(5b)/sin(11b) terms drawing his squished sine stripes
            // along the cut (transect + 16x16 field scan, 2026-07-12).
            let reach = mare.radius * mare.elongation * 1.70;
            if direction.dot(mare.center) < reach.cos() {
                continue;
            }
            let (q, bearing, _) = feature_coords(
                direction,
                mare.center,
                mare.major,
                mare.minor,
                mare.radius,
                mare.elongation,
            );
            let irregular = 1.0
                + 0.24 * gradient_noise(direction * 4.5, mare.seed)
                + 0.070 * (5.0 * bearing + mare.phase).sin()
                + 0.025 * (11.0 * bearing - mare.phase * 0.4).sin();
            // HEIGHT and ALBEDO separate here (Andrew's art direction):
            // geometry keeps the wide feather - basalt floods a basin, it
            // does not build walls - while the color edge is a tight band,
            // so the mare interior reads one consistent dark tone with
            // variation only at the border, like the real near side.
            let height_mask = 1.0 - smoothstep(0.74, 1.10, q * irregular);
            let albedo_mask = 1.0 - smoothstep(0.965, 1.075, q * irregular);
            if height_mask <= 0.0 {
                continue;
            }
            height = height * (1.0 - 0.84 * height_mask) - mare.floor_drop * height_mask;
            // overlapping maria SATURATE: basalt over basalt is the same
            // basalt (Andrew: intersections rendered darker than either)
            mare_mask = mare_mask.max(albedo_mask);
            mare_albedo = mare_albedo.min(mare.albedo_target);
            mare_smooth = mare_smooth.max(albedo_mask * if mare.large { 0.96 } else { 0.86 });
        }
        // Near-solid interior coverage suppresses the highland grain rather
        // than merely subtracting a constant from it. Only the tight edge
        // band exposes a meaningful fraction of the inherited variation.
        albedo = mix(albedo, mare_albedo, mare_mask * 0.985);
        smoothness = smoothness.max(mare_smooth);

        BaseSample {
            height,
            albedo,
            smoothness,
        }
    }

    /// Height at `direction` after the cached crater octaves strictly coarser
    /// than `max_octave`. Cache construction proceeds coarse-to-fine, so every
    /// record consulted here already owns its stable local floor datum.
    fn composed_coarse_height(&self, direction: DVec3, max_octave: usize) -> f64 {
        let base = self.base_surface(direction);
        let mut state = SurfaceState {
            height: base.height,
            albedo: base.albedo,
            smoothness: base.smoothness,
            ray: 0.0,
        };
        let locator = face_from_dir(direction);
        let mut keys = [CellKey::ZERO; tuning::RAY_CELL_VISITS];
        let mut craters = [Crater::EMPTY; tuning::RAY_CELL_VISITS];
        for octave in 0..max_octave.min(self.crater_cache.len()) {
            let key_count = nearby_crater_cells(locator, 1u32 << octave, &mut keys);
            let mut crater_count = 0usize;
            for &key in &keys[..key_count] {
                if let Some(crater) = self.crater_from_cell(octave, key, Some((direction, locator)))
                {
                    craters[crater_count] = crater;
                    crater_count += 1;
                }
            }
            craters[..crater_count].sort_unstable_by_key(|c| c.age_key);
            for &crater in &craters[..crater_count] {
                // Only geometry is consumed. Supplying the local base avoids
                // reevaluating irrelevant material at each old impact center.
                self.apply_crater_with_impact(crater, direction, &mut state, Some(base));
            }
        }
        state.height
    }

    /// Pick a low, local excavation datum from terrain that existed before
    /// this impact. Visible cached cohorts probe across the inner bowl, which
    /// is what makes a young crater erase an older rim it straddles. Fine
    /// saturation cohorts use the composed center value to keep their hot path
    /// bounded; the no-raise guard in `apply_crater` remains the pillar trap's
    /// final backstop.
    fn impact_floor_datum(
        &self,
        center: DVec3,
        major: DVec3,
        minor: DVec3,
        radius: f64,
        octave: usize,
    ) -> f64 {
        let mut heights = [self.composed_coarse_height(center, octave); 9];
        if octave < tuning::CACHED_CRATER_OCTAVES {
            let angle = radius * 0.62;
            for (index, slot) in heights[1..].iter_mut().enumerate() {
                let bearing = index as f64 * std::f64::consts::TAU / 8.0;
                let tangent = major * bearing.cos() + minor * bearing.sin();
                let point = center * angle.cos() + tangent * angle.sin();
                *slot = self.composed_coarse_height(point, octave);
            }
            heights.sort_unstable_by(f64::total_cmp);
            // A low excavation datum means the pointwise no-raise ceiling is
            // normally dormant, including when this bowl crosses an old rim.
            heights[0]
        } else {
            heights[0]
        }
    }

    fn crater_from_cell(
        &self,
        octave: usize,
        key: CellKey,
        sample: Option<(DVec3, (usize, f64, f64))>,
    ) -> Option<Crater> {
        if let Some(bin) = self.crater_cache.get(octave) {
            let grid = 1usize << octave;
            let index = key.face as usize * grid * grid + key.j as usize * grid + key.i as usize;
            let crater = bin.get(index).copied().flatten()?;
            if let Some((direction, _)) = sample {
                let reach = crater.radius * crater.elongation * tuning::CRATER_OUTER_RADII;
                if direction.distance_squared(crater.center) > reach * reach {
                    return None;
                }
            }
            return Some(crater);
        }
        self.generate_crater_from_cell(octave, key, sample)
    }

    fn generate_crater_from_cell(
        &self,
        octave: usize,
        key: CellKey,
        sample: Option<(DVec3, (usize, f64, f64))>,
    ) -> Option<Crater> {
        let domain = 0x4352_4154_4552_0000 ^ octave as u64;
        let primary = feature_hash(self.seed, key, domain, 0);
        let occupancy = unit_from_hash(primary);
        if octave > 0 && occupancy >= tuning::CRATER_OCCUPANCY {
            return None;
        }
        let grid = 1u32 << octave;
        let radius_fraction = crater_radius_fraction(octave, packed_unit(primary, 3));
        let (u, v) = packed_lattice_uv(key, grid, primary, tuning::CRATER_JITTER);
        if let Some((_, (sample_face, sample_u, sample_v))) = sample
            && key.face as usize == sample_face
        {
            let dx = (u - sample_u) * grid as f64 * 0.5;
            let dy = (v - sample_v) * grid as f64 * 0.5;
            let max_cell_reach = radius_fraction
                * 0.5
                * tuning::CRATER_OUTER_RADII
                * tuning::MAX_CRATER_ELONGATION
                * 1.08;
            if dx * dx + dy * dy > max_cell_reach * max_cell_reach {
                return None;
            }
        }
        let (center, _u, _v, inverse_length) =
            packed_lattice_point(key, grid, primary, tuning::CRATER_JITTER);
        if octave == 0 {
            let spawn = mare_spawn_field(&self.clusters, center, self.seed);
            let probability = 0.42 + 0.32 * spawn;
            if occupancy >= probability {
                return None;
            }
        }

        // The smaller singular value of the gnomonic cube projection keeps
        // crater reach bounded in cell units even at cube corners.
        let cube_metric = inverse_length * inverse_length;
        let radius = radius_fraction / grid as f64 * cube_metric;
        if let Some((direction, _)) = sample {
            let max_reach = radius * 1.72 * tuning::CRATER_OUTER_RADII;
            if direction.distance_squared(center) > max_reach * max_reach {
                return None;
            }
        }
        let axis_angle = feature_unit(self.seed, key, domain, 4) * std::f64::consts::TAU;
        let (major, minor) = tangent_axes(center, axis_angle);
        // Obliquity is continuous, but strongly biased toward near-normal
        // impacts so highly egg-shaped bowls retain their old rarity.
        let impact_angle = feature_unit(self.seed, key, domain, 5).powf(7.0);
        let elongation = 1.0 + (tuning::MAX_CRATER_ELONGATION - 1.0) * impact_angle;
        let scale = octave as f64 / (tuning::CRATER_OCTAVES - 1) as f64;
        let depth_factor = mix(0.024, 0.132, scale.sqrt())
            * (0.82 + 0.30 * feature_unit(self.seed, key, domain, 7));
        let roughness = mix(0.115, 0.014, scale.sqrt())
            * (0.72 + 0.42 * feature_unit(self.seed, key, domain, 8));
        let freshness = feature_unit(self.seed, key, domain, 9);
        let floor_datum = self.impact_floor_datum(center, major, minor, radius, octave);
        Some(Crater {
            center,
            major,
            minor,
            radius,
            elongation,
            depth_ratio: radius * depth_factor,
            roughness,
            rim_phase: feature_unit(self.seed, key, domain, 10) * std::f64::consts::TAU,
            rim_lobes: 5.0 + (feature_unit(self.seed, key, domain, 11) * 10.0).floor(),
            fresh_albedo: 0.72 + 0.12 * freshness,
            floor_tone_delta: (feature_unit(self.seed, key, domain, 12) - 0.5) * 0.038,
            floor_datum,
            octave: octave as u8,
            age_key: feature_hash(self.seed, key, domain, 13),
            noise_seed: feature_hash(self.seed, key, domain, 14) as i64,
        })
    }

    fn ray_carrier_from_cell(
        &self,
        key: CellKey,
        sample_direction: Option<DVec3>,
    ) -> Option<Crater> {
        if !self.ray_cache.is_empty() {
            let grid = tuning::RAY_GRID as usize;
            let index = key.face as usize * grid * grid + key.j as usize * grid + key.i as usize;
            let carrier = self.ray_cache.get(index).copied().flatten()?;
            if let Some(direction) = sample_direction {
                let max_reach = carrier.radius * tuning::RAY_MAX_RADII;
                if direction.distance_squared(carrier.center) > max_reach * max_reach {
                    return None;
                }
            }
            return Some(carrier);
        }
        self.generate_ray_carrier_from_cell(key, sample_direction)
    }

    fn generate_ray_carrier_from_cell(
        &self,
        key: CellKey,
        sample_direction: Option<DVec3>,
    ) -> Option<Crater> {
        const DOMAIN: u64 = 0x5459_4348_4F5F_5241;
        let primary = feature_hash(self.seed, key, DOMAIN, 0);
        if packed_unit(primary, 0) >= tuning::RAY_OCCUPANCY {
            return None;
        }
        let (center, _, _, _) = packed_lattice_point(key, tuning::RAY_GRID, primary, 0.30);
        let radius = log_range(0.55, 1.25, packed_unit(primary, 3)).to_radians();
        if let Some(direction) = sample_direction {
            let max_reach = radius * tuning::RAY_MAX_RADII;
            if direction.distance_squared(center) > max_reach * max_reach {
                return None;
            }
        }
        let axis_angle = feature_unit(self.seed, key, DOMAIN, 4) * std::f64::consts::TAU;
        let (major, minor) = tangent_axes(center, axis_angle);
        let floor_datum = self.impact_floor_datum(center, major, minor, radius, 4);
        Some(Crater {
            center,
            major,
            minor,
            radius,
            elongation: 1.0
                + (tuning::MAX_CRATER_ELONGATION - 1.0)
                    * feature_unit(self.seed, key, DOMAIN, 4).powf(7.0),
            depth_ratio: radius * (0.088 + 0.030 * feature_unit(self.seed, key, DOMAIN, 5)),
            roughness: 0.026 + 0.020 * feature_unit(self.seed, key, DOMAIN, 6),
            rim_phase: feature_unit(self.seed, key, DOMAIN, 7) * std::f64::consts::TAU,
            rim_lobes: 7.0 + (feature_unit(self.seed, key, DOMAIN, 8) * 8.0).floor(),
            fresh_albedo: 0.80 + 0.08 * feature_unit(self.seed, key, DOMAIN, 9),
            floor_tone_delta: (feature_unit(self.seed, key, DOMAIN, 10) - 0.5) * 0.025,
            floor_datum,
            octave: 4,
            age_key: feature_hash(self.seed, key, DOMAIN, 11),
            noise_seed: feature_hash(self.seed, key, DOMAIN, 12) as i64,
        })
    }

    fn apply_crater(&self, crater: Crater, direction: DVec3, state: &mut SurfaceState) {
        self.apply_crater_with_impact(crater, direction, state, None);
    }

    fn apply_crater_with_impact(
        &self,
        crater: Crater,
        direction: DVec3,
        state: &mut SurfaceState,
        prefetched_impact: Option<BaseSample>,
    ) {
        let reach = crater.radius * crater.elongation * tuning::CRATER_OUTER_RADII;
        if reach < std::f64::consts::PI && direction.dot(crater.center) < reach.cos() {
            return;
        }
        let (q0, bearing, _) = feature_coords(
            direction,
            crater.center,
            crater.major,
            crater.minor,
            crater.radius,
            crater.elongation,
        );
        if q0 >= tuning::CRATER_OUTER_RADII * 1.18 {
            return;
        }

        // Angular noise perturbs the rim radius itself. The amplitude falls
        // by almost an order of magnitude from basins to fine craters.
        let noise_scale = (3.5 / crater.radius).clamp(9.0, 16_000.0);
        let angular_noise = 0.68 * (crater.rim_lobes * bearing + crater.rim_phase).sin()
            + 0.24 * ((crater.rim_lobes * 2.0 + 3.0) * bearing - crater.rim_phase * 0.71).sin()
            + 0.18 * gradient_noise(direction * noise_scale, crater.noise_seed);
        let rim_gate = smoothstep(0.38, 0.86, q0) * (1.0 - smoothstep(1.52, 1.94, q0));
        let rim_radius = (1.0 + crater.roughness * rim_gate * angular_noise).clamp(0.78, 1.22);
        let q = q0 / rim_radius;
        if q >= tuning::CRATER_OUTER_RADII {
            return;
        }

        let scale = crater.octave as f64 / (tuning::CRATER_OCTAVES - 1) as f64;
        let floor_weight = 1.0 - smoothstep(0.70, 0.975, q);
        if floor_weight > 0.0 {
            // One pre-impact local datum and material target owns the complete
            // floor. Geometry is driven toward that constant instead of
            // subtracting the same depth from whatever old rim happens to be
            // under each sample.
            let impact = match prefetched_impact {
                Some(impact) => impact,
                None => self.base_surface(crater.center),
            };
            let floor_shape = 0.96 - 0.075 * (q / 0.70).min(1.0).powi(2);
            let texture_scale = (2.8 / crater.radius).clamp(18.0, 22_000.0);
            let floor_texture = crater.depth_ratio
                * 0.012
                * gradient_noise(
                    direction * texture_scale,
                    crater.noise_seed.wrapping_add(91),
                );
            let mut target = if self.legacy_floors {
                // Evidence-only A/B lens: the shipped subtractive floor keeps
                // all inherited contrast and makes a rim run through a young
                // bowl. Never enabled by production scripts or rendering.
                state.height - crater.depth_ratio * floor_shape + floor_texture
            } else {
                let excavated =
                    crater.floor_datum - crater.depth_ratio * floor_shape + floor_texture;
                // An impact may lower or leave inherited terrain, never lift
                // a deep old basin toward a stale datum. The small compulsory
                // cut also prevents an anomalously low inherited point from
                // becoming an unexcavated island while retaining the pillar
                // guarantee.
                let no_raise_ceiling = state.height - crater.depth_ratio * floor_shape * 0.08
                    + floor_texture.min(0.0);
                // A hard min stamped a contour wherever the two surfaces
                // crossed. This conservative smooth minimum stays below both
                // inputs while feathering that transition over impact depth.
                smooth_min(excavated, no_raise_ceiling, crater.depth_ratio * 0.55)
            };
            if crater.octave <= 2 {
                let peak =
                    crater.depth_ratio * mix(0.48, 0.30, scale) * (-(q / 0.21).powi(2)).exp();
                let dimple = crater.depth_ratio * 0.15 * (-(q / 0.052).powi(2)).exp();
                target += peak - dimple;
            }
            state.height = mix(state.height, target, floor_weight);
            let floor_albedo = (impact.albedo + crater.floor_tone_delta).clamp(0.20, 0.84);
            state.albedo = mix(state.albedo, floor_albedo, floor_weight);
            let floor_smooth = (impact.smoothness * 0.78 + 0.055).clamp(0.035, 0.80);
            state.smoothness = mix(state.smoothness, floor_smooth, floor_weight);
            state.ray *= 1.0 - floor_weight * 0.98;
        }

        // The inside wall is narrow and steep; outside relief decays over
        // almost a full radius. Basin rims use little uplift, keeping their
        // flat near adjacent ground while fine fresh rims stand higher.
        let inside_width = 0.060;
        let outside_width = 0.255;
        let rim_core = if q <= 1.0 {
            (-((q - 1.0) / inside_width).powi(2)).exp()
        } else {
            (-((q - 1.0) / outside_width).powi(2)).exp()
        };
        // The ejecta blanket RAMPS from zero at the crest: the old branch
        // jumped 0 -> 1 exactly at q = 1, stamping a discontinuous ledge
        // (0.42*rim_lift + the disturbed term) along every crater's crest
        // line - 492 m on the largest basin, Andrew's "discrete cliff
        // faces... affect all craters at the rim" (traced 2026-07-12).
        let outer_falloff =
            smoothstep(1.0, 1.10, q) * (1.0 - smoothstep(1.0, tuning::CRATER_OUTER_RADII, q));
        let ridge = (0.80
            + 0.20 * (crater.rim_lobes * bearing + crater.rim_phase).sin()
            + 0.08 * ((crater.rim_lobes * 2.0 + 3.0) * bearing - crater.rim_phase * 0.71).sin())
        .clamp(0.52, 1.18);
        let rim_lift = crater.depth_ratio * mix(0.055, 0.245, scale.sqrt());
        state.height += rim_lift * (rim_core + 0.42 * outer_falloff) * ridge;
        let disturbed = outer_falloff
            * (0.48 + 0.52 * (17.0 * q + crater.rim_phase + 3.0 * bearing).sin().abs());
        state.height += crater.depth_ratio * 0.014 * disturbed;

        // Universal bright rims read cartoonish (Andrew: "significantly
        // curb this effect") - only genuinely FRESH craters flash their
        // rims now; the rest keep a whisper. Rim contrast should come
        // from lighting, not paint.
        let fresh_gate = 0.12 + 0.88 * smoothstep(0.80, 0.96, (crater.fresh_albedo - 0.72) / 0.12);
        let exposed = (rim_core * 0.72 + disturbed * 0.13).clamp(0.0, 0.88) * fresh_gate;
        state.albedo = mix(state.albedo, crater.fresh_albedo, exposed);
        state.albedo += 0.012 * rim_core * ridge * fresh_gate;
        state.smoothness *= 1.0 - (rim_core * 0.68 + disturbed * 0.16).clamp(0.0, 0.84);
        state.ray *= 1.0 - (rim_core * 0.75 + disturbed * 0.30).clamp(0.0, 0.92);
    }

    fn apply_ray_field(&self, crater: Crater, direction: DVec3, state: &mut SurfaceState) {
        let max_reach = crater.radius * tuning::RAY_MAX_RADII;
        if direction.dot(crater.center) < max_reach.cos() {
            return;
        }
        let (_, bearing, theta) = feature_coords(
            direction,
            crater.center,
            crater.major,
            crater.minor,
            crater.radius,
            1.0,
        );
        let radial = theta / crater.radius;
        if radial <= 0.98 || radial >= tuning::RAY_MAX_RADII {
            return;
        }

        // Tycho's proximal blanket: dark and smoother immediately outside
        // the rim, gone by two radii before the bright line field dominates.
        let halo = smoothstep(0.99, 1.08, radial) * (1.0 - smoothstep(1.08, 2.12, radial));
        if halo > 0.0 {
            let dark_target = (state.albedo * 0.58).clamp(0.24, 0.48);
            state.albedo = mix(state.albedo, dark_target, halo * 0.88);
            state.smoothness = state.smoothness.max(halo * 0.86);
            state.ray *= 1.0 - halo * 0.94;
        }
        if radial < 1.24 {
            return;
        }

        let shared_rough = gradient_noise(
            direction * 180.0 + crater.center * 37.0,
            crater.noise_seed.wrapping_add(4_201),
        );
        let mut ray: f64 = 0.0;
        for line in 0..tuning::RAY_LINES {
            let h = splitmix64(
                crater.noise_seed as u64 ^ (line as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
            );
            let u0 = unit_from_hash(h);
            let u1 = unit_from_hash(splitmix64(h ^ 0xA5A3_564E_27F8_862D));
            let u2 = unit_from_hash(splitmix64(h ^ 0xD6E8_FEB8_6659_FD93));
            let u3 = unit_from_hash(splitmix64(h ^ 0x94D0_49BB_1331_11EB));
            let u4 = unit_from_hash(splitmix64(h ^ 0xBF58_476D_1CE4_E5B9));
            let long = line < 7;
            let angle = u0 * std::f64::consts::TAU;
            let length = if long {
                mix(7.0, tuning::RAY_MAX_RADII, u1)
            } else {
                mix(3.1, 8.2, u1)
            };
            let start = 1.24 + 0.28 * u2;
            if radial <= start || radial >= length {
                continue;
            }
            let progress = ((radial - start) / (length - start)).clamp(0.0, 1.0);
            let taper = (1.0 - progress).powf(0.72);
            let width0 = crater.radius
                * if long {
                    mix(0.12, 0.25, u2)
                } else {
                    mix(0.045, 0.115, u2)
                };
            let width = width0 * (0.08 + 0.92 * taper) * (0.88 + 0.16 * shared_rough);
            let warp = (0.012 + 0.034 * u3) * (radial * mix(0.45, 1.35, u4) + u2 * 9.0).sin()
                + 0.010 * shared_rough;
            let delta = (bearing - angle - warp + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            let perpendicular = theta * delta.sin().abs();
            let core = 1.0 - smoothstep(width * 0.38, width, perpendicular);
            if core <= 0.0 {
                continue;
            }
            let begin = smoothstep(start, start + 0.32, radial);
            let end = 1.0 - smoothstep(length * 0.76, length, radial);
            let broken_wave = (radial * mix(2.2, 5.8, u3) + u4 * 17.0).sin() + shared_rough * 0.62;
            let broken = mix(0.32, 1.0, smoothstep(-0.48, 0.38, broken_wave));
            let strength = if long {
                mix(0.55, 1.0, u4)
            } else {
                mix(0.25, 0.72, u4)
            };
            ray = ray.max(core * begin * end * broken * strength);
        }
        if ray > 0.0 {
            state.albedo += (0.96 - state.albedo) * ray * 0.84;
            state.height += crater.radius * 0.00011 * ray;
            state.smoothness *= 1.0 - ray * 0.24;
            state.ray = state.ray.max(ray);
        }
    }

    /// The sole surface law. Input is normalized defensively so map, tests,
    /// mesh tiles, and voxel columns cannot create subtly different faces.
    pub fn sample(&self, direction: DVec3) -> MoonSample {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 || !direction.is_finite() {
            return MoonSample {
                height_ratio: 0.0,
                albedo: 0.5,
                smoothness: 0.0,
                ray: 0.0,
            };
        }

        let base = self.base_surface(direction);
        let mut state = SurfaceState {
            height: base.height,
            albedo: base.albedo,
            smoothness: base.smoothness,
            ray: 0.0,
        };
        let mut keys = [CellKey::ZERO; tuning::RAY_CELL_VISITS];
        let mut craters = [Crater::EMPTY; tuning::RAY_CELL_VISITS];
        let locator = face_from_dir(direction);

        // diagnostic: TRI_MOON_SKIP_OCTAVE=N removes one crater octave -
        // the bisection tool that found the ejecta-ledge cliff (B-3)
        let skip_octave: Option<usize> = std::env::var("TRI_MOON_SKIP_OCTAVE")
            .ok()
            .and_then(|v| v.parse().ok());
        for octave in 0..tuning::CRATER_OCTAVES {
            if skip_octave == Some(octave) {
                continue;
            }
            let grid = 1u32 << octave;
            let key_count = nearby_crater_cells(locator, grid, &mut keys);
            let mut crater_count = 0usize;
            for &key in &keys[..key_count] {
                if let Some(crater) = self.crater_from_cell(octave, key, Some((direction, locator)))
                {
                    craters[crater_count] = crater;
                    crater_count += 1;
                }
            }
            craters[..crater_count].sort_unstable_by_key(|c| c.age_key);
            for &crater in &craters[..crater_count] {
                self.apply_crater(crater, direction, &mut state);
            }

            // Tycho-class impacts are younger than the broad fields but older
            // than the fine saturation layers that naturally pile over rays.
            if octave == 4 {
                let key_count = nearby_cells(
                    direction,
                    tuning::RAY_GRID,
                    tuning::RAY_NEIGHBOR_RADIUS,
                    &mut keys,
                );
                let mut carrier_count = 0usize;
                for &key in &keys[..key_count] {
                    if let Some(carrier) = self.ray_carrier_from_cell(key, Some(direction)) {
                        craters[carrier_count] = carrier;
                        carrier_count += 1;
                    }
                }
                craters[..carrier_count].sort_unstable_by_key(|c| c.age_key);
                for &carrier in &craters[..carrier_count] {
                    self.apply_crater(carrier, direction, &mut state);
                    self.apply_ray_field(carrier, direction, &mut state);
                }
            }
        }

        MoonSample {
            height_ratio: state.height.clamp(-0.014, 0.010),
            albedo: state.albedo.clamp(0.15, 0.96),
            smoothness: state.smoothness.clamp(0.0, 1.0),
            ray: state.ray.clamp(0.0, 1.0),
        }
    }

    fn prefetch_for_directions(&self, directions: &[DVec3]) -> MoonCraterPrefetch {
        let skip_octave: Option<usize> = std::env::var("TRI_MOON_SKIP_OCTAVE")
            .ok()
            .and_then(|v| v.parse().ok());
        let mut octave_keys: [BTreeSet<CellKey>; tuning::CRATER_OCTAVES] =
            std::array::from_fn(|_| BTreeSet::new());
        let mut ray_keys = BTreeSet::new();
        let mut keys = [CellKey::ZERO; tuning::RAY_CELL_VISITS];
        let mut sample_keys = Vec::with_capacity(directions.len());
        for &raw_direction in directions {
            let mut lookup = MoonSampleKeys::empty();
            let direction = raw_direction.normalize_or_zero();
            if direction.length_squared() < 0.5 || !direction.is_finite() {
                sample_keys.push(lookup);
                continue;
            }
            let locator = face_from_dir(direction);
            for (octave, set) in octave_keys.iter_mut().enumerate() {
                if skip_octave == Some(octave) {
                    continue;
                }
                if octave == 4 {
                    let key_count = nearby_cells(
                        direction,
                        tuning::RAY_GRID,
                        tuning::RAY_NEIGHBOR_RADIUS,
                        &mut keys,
                    );
                    lookup.rays[..key_count].copy_from_slice(&keys[..key_count]);
                    lookup.ray_count = key_count as u8;
                    ray_keys.extend(keys[..key_count].iter().copied());
                }
                let key_count = nearby_crater_cells(locator, 1u32 << octave, &mut keys);
                lookup.octaves[octave][..key_count].copy_from_slice(&keys[..key_count]);
                lookup.octave_counts[octave] = key_count as u8;
                set.extend(keys[..key_count].iter().copied());
            }
            sample_keys.push(lookup);
        }
        let mut octave_indices: [BTreeMap<CellKey, u32>; tuning::CRATER_OCTAVES] =
            std::array::from_fn(|_| BTreeMap::new());
        let octaves = std::array::from_fn(|octave| {
            let mut craters = Vec::new();
            for &key in &octave_keys[octave] {
                if let Some(crater) = self.crater_from_cell(octave, key, None) {
                    octave_indices[octave].insert(key, craters.len() as u32);
                    craters.push(PrefetchedCrater {
                        crater,
                        impact: self.base_surface(crater.center),
                    });
                }
            }
            craters
        });
        let mut ray_indices = BTreeMap::new();
        let mut ray_carriers = Vec::new();
        for key in ray_keys {
            if let Some(carrier) = self.ray_carrier_from_cell(key, None) {
                ray_indices.insert(key, ray_carriers.len() as u32);
                ray_carriers.push(PrefetchedCrater {
                    crater: carrier,
                    impact: self.base_surface(carrier.center),
                });
            }
        }
        let samples = sample_keys
            .into_iter()
            .map(|lookup| {
                let mut sample = MoonSamplePrefetch::empty();
                for octave in 0..tuning::CRATER_OCTAVES {
                    let mut count = 0usize;
                    for key in &lookup.octaves[octave][..lookup.octave_counts[octave] as usize] {
                        if let Some(&index) = octave_indices[octave].get(key) {
                            sample.octaves[octave][count] = index;
                            count += 1;
                        }
                    }
                    sample.octaves[octave][..count].sort_unstable_by_key(|&index| {
                        octaves[octave][index as usize].crater.age_key
                    });
                    sample.octave_counts[octave] = count as u8;
                }
                let mut count = 0usize;
                for key in &lookup.rays[..lookup.ray_count as usize] {
                    if let Some(&index) = ray_indices.get(key) {
                        sample.rays[count] = index;
                        count += 1;
                    }
                }
                sample.rays[..count]
                    .sort_unstable_by_key(|&index| ray_carriers[index as usize].crater.age_key);
                sample.ray_count = count as u8;
                sample
            })
            .collect();
        MoonCraterPrefetch {
            octaves,
            ray_carriers,
            samples,
            skip_octave,
        }
    }

    fn sample_prefetched(
        &self,
        direction: DVec3,
        prefetch: &MoonCraterPrefetch,
        sample_index: usize,
    ) -> MoonSample {
        let direction = direction.normalize_or_zero();
        if direction.length_squared() < 0.5 || !direction.is_finite() {
            return MoonSample {
                height_ratio: 0.0,
                albedo: 0.5,
                smoothness: 0.0,
                ray: 0.0,
            };
        }
        let base = self.base_surface(direction);
        let mut state = SurfaceState {
            height: base.height,
            albedo: base.albedo,
            smoothness: base.smoothness,
            ray: 0.0,
        };
        let sample = &prefetch.samples[sample_index];
        for (octave, crater_cache) in prefetch.octaves.iter().enumerate() {
            if prefetch.skip_octave == Some(octave) {
                continue;
            }
            for &index in &sample.octaves[octave][..sample.octave_counts[octave] as usize] {
                let prefetched = crater_cache[index as usize];
                self.apply_crater_with_impact(
                    prefetched.crater,
                    direction,
                    &mut state,
                    Some(prefetched.impact),
                );
            }
            if octave == 4 {
                for &index in &sample.rays[..sample.ray_count as usize] {
                    let prefetched = prefetch.ray_carriers[index as usize];
                    self.apply_crater_with_impact(
                        prefetched.crater,
                        direction,
                        &mut state,
                        Some(prefetched.impact),
                    );
                    self.apply_ray_field(prefetched.crater, direction, &mut state);
                }
            }
        }
        MoonSample {
            height_ratio: state.height.clamp(-0.014, 0.010),
            albedo: state.albedo.clamp(0.15, 0.96),
            smoothness: state.smoothness.clamp(0.0, 1.0),
            ray: state.ray.clamp(0.0, 1.0),
        }
    }

    /// Sample a chunk/tile footprint with one amortized crater enumeration.
    pub fn sample_batch(&self, directions: &[DVec3]) -> Vec<MoonSample> {
        // Broad coarse-LOD tiles share few cells; their immutable generator
        // cache is faster than building a sparse per-tile index. Compact
        // landed 33x33 tiles and voxel chunks enumerate all crater/ray cells
        // once and apply indexed lists per sample.
        let anchor = directions
            .first()
            .copied()
            .unwrap_or(DVec3::ZERO)
            .normalize_or_zero();
        let compact = directions
            .iter()
            .all(|&direction| (direction.normalize_or_zero() - anchor).length_squared() <= 0.0005);
        if !compact {
            return directions
                .iter()
                .map(|&direction| self.sample(direction))
                .collect();
        }
        let prefetch = self.prefetch_for_directions(directions);
        directions
            .iter()
            .enumerate()
            .map(|(index, &direction)| self.sample_prefetched(direction, &prefetch, index))
            .collect()
    }

    pub fn height_km(&self, direction: DVec3, radius_km: f64) -> f64 {
        self.sample(direction).height_ratio * radius_km
    }
}

fn mare_spawn_field(clusters: &[MareCluster; 4], direction: DVec3, seed: i64) -> f64 {
    let mut field = 0.0f64;
    for cluster in clusters {
        let theta = direction.dot(cluster.center).clamp(-1.0, 1.0).acos();
        let contribution = cluster.strength * (-0.5 * (theta / cluster.radius).powi(2)).exp();
        field = 1.0 - (1.0 - field) * (1.0 - contribution.clamp(0.0, 1.0));
    }
    (field + 0.075 * gradient_noise(direction * 2.4, seed.wrapping_add(18_811))).clamp(0.0, 1.0)
}

fn feature_probe(crater: Crater) -> MoonFeatureProbe {
    MoonFeatureProbe {
        lat_deg: crater.center.z.asin().to_degrees(),
        lon_deg: crater.center.y.atan2(crater.center.x).to_degrees(),
        radius_deg: crater.radius.to_degrees(),
    }
}

/// Convenience form for probes/tests. Hot paths construct one generator and
/// share it through an `Arc`.
pub fn sample(direction: DVec3, seed: i64) -> MoonSample {
    MoonGenerator::new(seed).sample(direction)
}

/// Build one adaptive cube-sphere moon tile. Positions and normals are
/// computed in f64 moon-local kilometres; the renderer adds the f64 body
/// center and subtracts the f64 camera before the f32 upload boundary.
pub fn build_tile(generator: &MoonGenerator, key: TileKey, radius_km: f64) -> TileMesh {
    let n = TILE_QUADS + 1;
    let np2 = n + 2;
    let (u0, v0, size) = key.uv_range();
    let face = key.face as usize;
    let origin = key.center_dir() * radius_km;
    let mut directions = Vec::with_capacity(np2 * np2);
    for gj in 0..np2 {
        for gi in 0..np2 {
            let u = u0 + size * (gi as f64 - 1.0) / TILE_QUADS as f64;
            let v = v0 + size * (gj as f64 - 1.0) / TILE_QUADS as f64;
            directions.push(face_dir(face, u, v));
        }
    }
    let samples = generator.sample_batch(&directions);
    let mut world = vec![DVec3::ZERO; np2 * np2];
    for (index, (&dir, sample)) in directions.iter().zip(&samples).enumerate() {
        world[index] = dir * (radius_km * (1.0 + sample.height_ratio));
    }

    let half = TILE_QUADS / 2 + 1;
    let mut parent_h = vec![0.0f64; half * half];
    if key.level > 0 {
        for pj in 0..half {
            for pi in 0..half {
                parent_h[pj * half + pi] = samples[(2 * pj + 1) * np2 + 2 * pi + 1].height_ratio;
            }
        }
    }
    let parent_value = |i: usize, j: usize| -> f64 {
        let (pi, fi) = (i / 2, (i % 2) as f64 * 0.5);
        let (pj, fj) = (j / 2, (j % 2) as f64 * 0.5);
        let (pi1, pj1) = ((pi + 1).min(half - 1), (pj + 1).min(half - 1));
        let (a, b, c, d) = (
            parent_h[pj * half + pi],
            parent_h[pj * half + pi1],
            parent_h[pj1 * half + pi],
            parent_h[pj1 * half + pi1],
        );
        if fi + fj <= 1.0 {
            a * (1.0 - fi - fj) + b * fi + c * fj
        } else {
            b * (1.0 - fj) + d * (fi + fj - 1.0) + c * (1.0 - fi)
        }
    };

    let mut vertices = Vec::with_capacity(n * n + 4 * n);
    for j in 0..n {
        for i in 0..n {
            let (gi, gj) = (i + 1, j + 1);
            let l = world[gj * np2 + gi - 1];
            let r = world[gj * np2 + gi + 1];
            let d = world[(gj - 1) * np2 + gi];
            let u = world[(gj + 1) * np2 + gi];
            let normal = (r - l).cross(u - d).normalize_or_zero();
            let p = world[gj * np2 + gi] - origin;
            let sample = samples[gj * np2 + gi];
            let dh = if key.level > 0 {
                (parent_value(i, j) - sample.height_ratio) * radius_km
            } else {
                0.0
            };
            let a = sample.albedo as f32;
            vertices.push(Vertex {
                pos: [p.x as f32, p.y as f32, p.z as f32],
                normal: [normal.x as f32, normal.y as f32, normal.z as f32],
                color: [a, a, a],
                water: [0.0, 0.0, 0.0, sample.smoothness as f32],
                morph_dh: dh as f32,
                morph_wet: 0.0,
                wflag: sample.ray as f32,
                shore: 0.0,
                biome: crate::terrain::NO_BIOME_PAYLOAD,
                beach: [0, 0, 0, 127],
            });
        }
    }

    let idx = |i: usize, j: usize| (j * n + i) as u32;
    let mut indices = Vec::with_capacity(TILE_QUADS * TILE_QUADS * 6 + 8 * TILE_QUADS * 6);
    for j in 0..TILE_QUADS {
        for i in 0..TILE_QUADS {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i, j + 1), idx(i + 1, j + 1));
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }

    let drop = (key.size_km(radius_km) * 0.018).clamp(0.002, 1.5);
    let border: Vec<u32> = (0..n)
        .map(|i| idx(i, 0))
        .chain((0..n).map(|i| idx(i, n - 1)))
        .chain((0..n).map(|j| idx(0, j)))
        .chain((0..n).map(|j| idx(n - 1, j)))
        .collect();
    for &b in &border {
        let v = vertices[b as usize];
        let p = DVec3::new(v.pos[0] as f64, v.pos[1] as f64, v.pos[2] as f64) + origin;
        let pulled = p - p.normalize() * drop - origin;
        vertices.push(Vertex {
            pos: pulled.to_array().map(|x| x as f32),
            ..v
        });
    }
    let skirt_base = (n * n) as u32;
    let seg = n as u32;
    for side in 0..4u32 {
        for t in 0..(n - 1) as u32 {
            let (t0, t1) = (side * seg + t, side * seg + t + 1);
            let (o0, o1) = (border[t0 as usize], border[t1 as usize]);
            let (s0, s1) = (skirt_base + t0, skirt_base + t1);
            indices.extend_from_slice(&[o0, o1, s0, o1, s1, s0]);
        }
    }

    TileMesh {
        origin_km: origin,
        vertices,
        indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(lat: f64, lon: f64) -> DVec3 {
        lat_lon_dir(lat, lon)
    }

    #[test]
    fn crater_lattice_is_deterministic_and_seeded() {
        let a = MoonGenerator::new(42);
        let b = MoonGenerator::new(42);
        for d in [
            dir(0.0, 0.0),
            dir(23.5, -71.25),
            dir(-68.0, 144.0),
            face_dir(0, 1.0, 0.237),
        ] {
            let sa = a.sample(d);
            let sb = b.sample(d);
            assert_eq!(sa.height_ratio.to_bits(), sb.height_ratio.to_bits());
            assert_eq!(sa.albedo.to_bits(), sb.albedo.to_bits());
            assert_eq!(sa.smoothness.to_bits(), sb.smoothness.to_bits());
            assert_eq!(sa.ray.to_bits(), sb.ray.to_bits());
        }
        assert_ne!(
            a.sample(dir(23.5, -71.25)),
            MoonGenerator::new(43).sample(dir(23.5, -71.25))
        );
    }

    #[test]
    fn prefetched_batch_is_bit_identical_to_scalar_sampling() {
        let moon = MoonGenerator::new(42);
        let mut directions = Vec::new();
        // Exercise an ordinary tile footprint, a cube-face rim, and several
        // unrelated directions so union candidates include harmless extras.
        for j in 0..35 {
            for i in 0..35 {
                let u = -0.31 + 0.08 * i as f64 / 34.0;
                let v = 0.77 + 0.08 * j as f64 / 34.0;
                directions.push(face_dir(0, u, v));
            }
        }
        for k in 0..64 {
            let v = -0.95 + 1.9 * k as f64 / 63.0;
            directions.push(face_dir(0, 1.0, v));
        }
        directions.extend([
            dir(-36.2094, -1.0),
            dir(37.0385, 101.1598),
            dir(-29.5553, 33.9783),
        ]);
        let prefetch = moon.prefetch_for_directions(&directions);
        for (index, &direction) in directions.iter().enumerate() {
            let prefetched = moon.sample_prefetched(direction, &prefetch, index);
            let scalar = moon.sample(direction);
            assert_eq!(
                prefetched.height_ratio.to_bits(),
                scalar.height_ratio.to_bits()
            );
            assert_eq!(prefetched.albedo.to_bits(), scalar.albedo.to_bits());
            assert_eq!(prefetched.smoothness.to_bits(), scalar.smoothness.to_bits());
            assert_eq!(prefetched.ray.to_bits(), scalar.ray.to_bits());
        }
    }

    #[test]
    fn lattice_visit_budget_is_bounded_unique_and_reproducible() {
        let d = face_dir(0, 1.0, -0.314_159_265);
        let mut a = [CellKey::ZERO; tuning::RAY_CELL_VISITS];
        let mut b = [CellKey::ZERO; tuning::RAY_CELL_VISITS];
        let mut ordinary_visits = 0usize;
        let locator = face_from_dir(d);
        for octave in 0..tuning::CRATER_OCTAVES {
            let n = nearby_crater_cells(locator, 1 << octave, &mut a);
            let n2 = nearby_crater_cells(locator, 1 << octave, &mut b);
            assert_eq!(n, n2);
            assert_eq!(&a[..n], &b[..n]);
            assert!(n <= 9);
            for i in 0..n {
                assert!(!a[..i].contains(&a[i]));
            }
            ordinary_visits += n;
        }
        let ray_visits = nearby_cells(d, tuning::RAY_GRID, tuning::RAY_NEIGHBOR_RADIUS, &mut a);
        assert!(ordinary_visits <= tuning::MAX_CRATER_CELL_VISITS);
        assert!(ray_visits <= tuning::RAY_CELL_VISITS);
        assert!(ordinary_visits + ray_visits <= tuning::MAX_LATTICE_CELL_VISITS);
    }

    fn active_count_on_face(seed: i64, octave: usize, face: u8) -> usize {
        let grid = 1u32 << octave;
        let domain = 0x4352_4154_4552_0000 ^ octave as u64;
        let mut count = 0usize;
        for j in 0..grid as u16 {
            for i in 0..grid as u16 {
                let key = CellKey { face, i, j };
                count += (unit_from_hash(feature_hash(seed, key, domain, 0))
                    < tuning::CRATER_OCCUPANCY) as usize;
            }
        }
        count
    }

    #[test]
    fn probe_region_follows_power_law_size_frequency() {
        // One complete cube face is the fixed probe region. Halving diameter
        // doubles lattice resolution in each axis, so N grows near 4x.
        let counts: Vec<usize> = (2..=7)
            .map(|octave| active_count_on_face(42, octave, 0))
            .collect();
        for pair in counts.windows(2).skip(1) {
            let ratio = pair[1] as f64 / pair[0] as f64;
            assert!((3.20..5.00).contains(&ratio), "counts={counts:?}");
        }
        let span = counts[5] as f64 / counts[0] as f64;
        assert!((650.0..1450.0).contains(&span), "counts={counts:?}");

        let moon = MoonGenerator::new(42);
        let largest = (0..6u8)
            .filter(|&face| {
                moon.crater_from_cell(0, CellKey { face, i: 0, j: 0 }, None)
                    .is_some()
            })
            .count();
        assert!((2..=5).contains(&largest), "largest={largest}");
        let small_global = (0..6u8)
            .map(|face| active_count_on_face(42, 6, face))
            .sum::<usize>();
        assert!(small_global >= largest * 1_000);
    }

    #[test]
    fn overlapping_radius_bands_have_no_size_comb() {
        let moon = MoonGenerator::new(42);
        let radii = moon.crater_radius_samples_on_face(0);
        // Ten equal log bins across the well-populated interior of the face
        // sample. A D^-2 law predicts a 10^(2*0.15)=2.0 count increase per
        // bin toward smaller radii. Cube metric and finite sampling broaden
        // that target, but an empty/gapped octave comb cannot pass.
        let mut counts = [0usize; 10];
        let log_lo = -1.50f64; // 0.0316 degrees
        let width = 0.15f64;
        for radius in radii {
            let bin = ((radius.log10() - log_lo) / width).floor() as isize;
            if (0..counts.len() as isize).contains(&bin) {
                counts[bin as usize] += 1;
            }
        }
        assert!(counts.iter().all(|&count| count >= 15), "counts={counts:?}");
        for pair in counts.windows(2) {
            let smaller_to_larger = pair[0] as f64 / pair[1] as f64;
            assert!(
                (1.15..3.8).contains(&smaller_to_larger),
                "counts={counts:?}, adjacent ratio={smaller_to_larger:.3}"
            );
        }
        eprintln!("radius-decade counts {counts:?}");
    }

    #[test]
    fn random_transects_obey_lunar_slope_bound() {
        let moon = MoonGenerator::new(42);
        let mut stream = SeedStream::new(42, 0x5452_414E_5345_4354);
        let mut max_slope = 0.0f64;
        let mut worst = (0usize, 0usize, 0.0f64);
        const STEPS: usize = 128;
        const SPAN: f64 = 0.24;
        for transect in 0..100 {
            let center = stream.direction();
            let random = stream.direction();
            let tangent = (random - center * random.dot(center)).normalize();
            let mut previous: Option<f64> = None;
            for step in 0..=STEPS {
                let angle = -SPAN * 0.5 + SPAN * step as f64 / STEPS as f64;
                let point = center * angle.cos() + tangent * angle.sin();
                let height = moon.sample(point).height_ratio;
                if let Some(old_height) = previous {
                    let slope = (height - old_height).abs() / (SPAN / STEPS as f64);
                    if slope > max_slope {
                        max_slope = slope;
                        worst = (transect, step, slope);
                    }
                }
                previous = Some(height);
            }
        }
        assert!(
            max_slope < 1.10,
            "100-transect max slope {max_slope:.6} at {worst:?}"
        );
        eprintln!("100-transect max slope {max_slope:.6} at {worst:?}");
    }

    #[test]
    fn jitter_reach_margin_is_sound_and_cut_lines_are_continuous() {
        let worst_reach = tuning::CRATER_RADIUS_CELL.1
            * 0.5
            * tuning::CRATER_OUTER_RADII
            * tuning::MAX_CRATER_ELONGATION
            * 1.08;
        let edge_extension = tuning::CRATER_JITTER + worst_reach - 0.5;
        assert!(tuning::CRATER_CELL_MARGIN >= edge_extension + 0.03);
        assert!(tuning::CRATER_JITTER + worst_reach < 1.5);

        let moon = MoonGenerator::new(42);
        let eps = 1.0e-10;
        for octave in 1..tuning::CRATER_OCTAVES {
            let grid = 1u32 << octave;
            let cell = (grid / 2).min(grid - 1) as f64;
            for fraction in [
                tuning::CRATER_CELL_MARGIN,
                1.0 - tuning::CRATER_CELL_MARGIN,
            ] {
                let u = -1.0 + 2.0 * (cell + fraction) / grid as f64;
                for v in [-0.83, -0.37, 0.19, 0.71] {
                    let a = moon.sample(face_dir(0, u - eps, v));
                    let b = moon.sample(face_dir(0, u + eps, v));
                    assert!(
                        (a.height_ratio - b.height_ratio).abs() < 1.0e-7,
                        "octave={octave} u={u} v={v}"
                    );
                    assert!((a.albedo - b.albedo).abs() < 1.0e-5);
                }
            }
        }
    }

    #[test]
    fn younger_impact_levels_an_older_rim_without_a_pillar() {
        let moon = MoonGenerator::new(42);
        let mut keys = [CellKey::ZERO; tuning::RAY_CELL_VISITS];
        let mut best: Option<(Crater, f64, f64)> = None;
        for octave in 1..tuning::CACHED_CRATER_OCTAVES {
            let grid = 1u32 << octave;
            for face in 0..6u8 {
                for j in 0..grid as u16 {
                    for i in 0..grid as u16 {
                        let Some(child) =
                            moon.crater_from_cell(octave, CellKey { face, i, j }, None)
                        else {
                            continue;
                        };
                        let locator = face_from_dir(child.center);
                        for older_octave in 0..octave {
                            let count = nearby_crater_cells(
                                locator,
                                1u32 << older_octave,
                                &mut keys,
                            );
                            for &key in &keys[..count] {
                                let Some(parent) = moon.crater_from_cell(
                                    older_octave,
                                    key,
                                    Some((child.center, locator)),
                                ) else {
                                    continue;
                                };
                                let (parent_q, _, _) = feature_coords(
                                    child.center,
                                    parent.center,
                                    parent.major,
                                    parent.minor,
                                    parent.radius,
                                    parent.elongation,
                                );
                                let child_span =
                                    child.radius * 0.64 / parent.radius.max(child.radius);
                                if (parent_q - 1.0).abs() >= child_span {
                                    continue;
                                }
                                let radial = (parent.center
                                    - child.center * parent.center.dot(child.center))
                                    .normalize();
                                let mut before_lo = f64::INFINITY;
                                let mut before_hi = f64::NEG_INFINITY;
                                let mut after_lo = f64::INFINITY;
                                let mut after_hi = f64::NEG_INFINITY;
                                for step in 0..=12 {
                                    let angle = child.radius * (-0.58 + 1.16 * step as f64 / 12.0);
                                    let point =
                                        child.center * angle.cos() + radial * angle.sin();
                                    let base = moon.base_surface(point);
                                    let before = moon.composed_coarse_height(point, octave);
                                    let mut state = SurfaceState {
                                        height: before,
                                        albedo: base.albedo,
                                        smoothness: base.smoothness,
                                        ray: 0.0,
                                    };
                                    moon.apply_crater_with_impact(
                                        child,
                                        point,
                                        &mut state,
                                        Some(base),
                                    );
                                    before_lo = before_lo.min(before);
                                    before_hi = before_hi.max(before);
                                    after_lo = after_lo.min(state.height);
                                    after_hi = after_hi.max(state.height);
                                }
                                let before_range = before_hi - before_lo;
                                let after_range = after_hi - after_lo;
                                if best.is_none_or(|(_, old_range, _)| before_range > old_range) {
                                    best = Some((child, before_range, after_range));
                                }
                            }
                        }
                    }
                }
            }
        }
        let (child, before, after) = best.expect("seed 42 cached rim-straddling crater");
        assert!(before > 2.0e-5, "weak proof case: before={before:.8}");
        assert!(
            after < before * 0.35,
            "rim floor radius={}deg retained relief: before={before:.8} after={after:.8}",
            child.radius.to_degrees()
        );
        eprintln!(
            "rim-straddling floor lat={:.4} lon={:.4} radius={:.4}deg relief before={before:.8} after={after:.8}",
            child.center.z.asin().to_degrees(),
            child.center.y.atan2(child.center.x).to_degrees(),
            child.radius.to_degrees(),
        );
        // The excavation guard is encoded pointwise above: its target never
        // exceeds inherited height, so this flattening cannot be a raised
        // impact-center pillar.
    }

    #[test]
    fn mare_spawn_field_and_records_are_reproducible() {
        let a = MoonGenerator::new(42);
        let b = MoonGenerator::new(42);
        assert_eq!(a.clusters, b.clusters);
        assert_eq!(a.maria, b.maria);
        assert_ne!(a.clusters, MoonGenerator::new(43).clusters);
        let (large, mid) = a.mare_counts();
        assert!(large >= 5, "seed 42 large maria={large}");
        assert!(mid >= 15, "seed 42 mid maria={mid}");
    }

    #[test]
    fn placement_angle_and_mare_sizes_are_continuous() {
        let moon = MoonGenerator::new(42);

        let mut elongations: Vec<u64> = moon
            .crater_cache
            .iter()
            .flatten()
            .filter_map(|crater| crater.map(|crater| crater.elongation.to_bits()))
            .collect();
        elongations.sort_unstable();
        elongations.dedup();
        assert!(elongations.len() > 100, "elongation values={}", elongations.len());
        let max_elongation = elongations
            .iter()
            .map(|bits| f64::from_bits(*bits))
            .fold(1.0f64, f64::max);
        assert!(max_elongation > 1.15 && max_elongation <= 1.20);

        let large_min = moon
            .maria
            .iter()
            .filter(|mare| mare.large)
            .map(|mare| mare.radius.to_degrees())
            .fold(f64::INFINITY, f64::min);
        let mid_max = moon
            .maria
            .iter()
            .filter(|mare| !mare.large)
            .map(|mare| mare.radius.to_degrees())
            .fold(0.0f64, f64::max);
        assert!(large_min < mid_max, "large_min={large_min} mid_max={mid_max}");

        let octave = 6;
        let grid = 1u32 << octave;
        let domain = 0x4352_4154_4552_0000 ^ octave as u64;
        let mut jitter_lo = 1.0f64;
        let mut jitter_hi = -1.0f64;
        for j in 0..grid as u16 {
            for i in 0..grid as u16 {
                let primary = feature_hash(42, CellKey { face: 0, i, j }, domain, 0);
                let jitter = (packed_unit(primary, 1) * 2.0 - 1.0) * tuning::CRATER_JITTER;
                jitter_lo = jitter_lo.min(jitter);
                jitter_hi = jitter_hi.max(jitter);
            }
        }
        assert!(jitter_lo < -0.44 && jitter_hi > 0.44, "jitter={jitter_lo}..{jitter_hi}");
    }

    #[test]
    fn mare_interior_is_dark_and_tightly_consistent() {
        let moon = MoonGenerator::new(42);
        let mare = moon
            .maria
            .iter()
            .filter(|mare| mare.large)
            .max_by(|a, b| a.radius.total_cmp(&b.radius))
            .expect("seed 42 large mare");
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for ring in 0..=3 {
            let angle = mare.radius * 0.10 * ring as f64;
            for bearing in 0..16 {
                let phase = bearing as f64 * std::f64::consts::TAU / 16.0;
                let tangent = mare.major * phase.cos() + mare.minor * phase.sin();
                let point = mare.center * angle.cos() + tangent * angle.sin();
                let albedo = moon.base_surface(point).albedo;
                lo = lo.min(albedo);
                hi = hi.max(albedo);
            }
        }
        assert!(hi < 0.36, "mare albedo={lo:.6}..{hi:.6}");
        assert!(hi - lo < 0.012, "mare albedo={lo:.6}..{hi:.6}");
        eprintln!("mare interior albedo {lo:.6}..{hi:.6}");
    }

    #[test]
    fn albedo_stays_physical_and_has_lunar_range() {
        let moon = MoonGenerator::new(42);
        let mut lo = 1.0f64;
        let mut hi = 0.0f64;
        for lat in (-90..=90).step_by(10) {
            for lon in (-180..180).step_by(10) {
                let s = moon.sample(dir(lat as f64, lon as f64));
                assert!(s.albedo.is_finite());
                assert!((0.0..=1.0).contains(&s.albedo), "{lat} {lon}: {s:?}");
                lo = lo.min(s.albedo);
                hi = hi.max(s.albedo);
            }
        }
        assert!(lo < 0.58, "maria never became visibly dark: {lo}..{hi}");
        assert!(
            hi > 0.78,
            "rims/rays never became visibly bright: {lo}..{hi}"
        );
    }

    #[test]
    fn lattice_cell_and_cube_face_boundaries_are_continuous() {
        let moon = MoonGenerator::new(42);
        let eps = 1.0e-10;
        for octave in 1..tuning::CRATER_OCTAVES {
            let grid = 1u32 << octave;
            for i in [1, grid / 2, grid - 1] {
                let u = -1.0 + 2.0 * i as f64 / grid as f64;
                for v in [-0.73, -0.11, 0.42, 0.81] {
                    let a = moon.sample(face_dir(0, u - eps, v));
                    let b = moon.sample(face_dir(0, u + eps, v));
                    assert!((a.height_ratio - b.height_ratio).abs() < 1.0e-7);
                    assert!((a.albedo - b.albedo).abs() < 1.0e-5);
                }
            }
        }
        for z in [-0.72, -0.15, 0.37, 0.84] {
            let seam = DVec3::new(1.0, 1.0, z).normalize();
            let across = DVec3::new(1.0, -1.0, 0.0).normalize();
            let a = moon.sample((seam - across * eps).normalize());
            let b = moon.sample((seam + across * eps).normalize());
            assert!((a.height_ratio - b.height_ratio).abs() < 1.0e-7);
            assert!((a.albedo - b.albedo).abs() < 1.0e-5);
        }
    }

    #[test]
    fn impact_center_owns_one_floor_material_and_ray_halo_is_dark_smooth() {
        let moon = MoonGenerator::new(42);
        let crater = 'found_crater: {
            for face in 0..6u8 {
                for j in 0..4u16 {
                    for i in 0..4u16 {
                        if let Some(crater) = moon.crater_from_cell(2, CellKey { face, i, j }, None)
                        {
                            break 'found_crater crater;
                        }
                    }
                }
            }
            panic!("seed 42 octave-2 crater")
        };
        let angle = crater.radius * 0.30;
        let p0 = crater.center * angle.cos() + crater.major * angle.sin();
        let p1 = crater.center * angle.cos() - crater.major * angle.sin();
        let mut a = SurfaceState {
            height: -0.004,
            albedo: 0.28,
            smoothness: 0.88,
            ray: 0.7,
        };
        let mut b = SurfaceState {
            height: 0.002,
            albedo: 0.82,
            smoothness: 0.02,
            ray: 0.1,
        };
        moon.apply_crater(crater, p0, &mut a);
        moon.apply_crater(crater, p1, &mut b);
        assert!((a.albedo - b.albedo).abs() < 1.0e-12);
        assert!((a.smoothness - b.smoothness).abs() < 1.0e-12);

        let carrier = 'found_carrier: {
            for face in 0..6u8 {
                for j in 0..tuning::RAY_GRID as u16 {
                    for i in 0..tuning::RAY_GRID as u16 {
                        if let Some(carrier) =
                            moon.ray_carrier_from_cell(CellKey { face, i, j }, None)
                        {
                            break 'found_carrier carrier;
                        }
                    }
                }
            }
            panic!("seed 42 ray carrier")
        };
        let center = carrier.center;
        let (_, tangent) = tangent_axes(center, 0.0);
        let halo_angle = carrier.radius * 1.12;
        let point = center * halo_angle.cos() + tangent * halo_angle.sin();
        let mut halo = SurfaceState {
            height: 0.0,
            albedo: 0.70,
            smoothness: 0.05,
            ray: 0.0,
        };
        moon.apply_ray_field(carrier, point, &mut halo);
        assert!(
            halo.albedo < 0.60,
            "halo={halo_albedo}",
            halo_albedo = halo.albedo
        );
        assert!(
            halo.smoothness > 0.35,
            "halo smoothness={}",
            halo.smoothness
        );
    }
}

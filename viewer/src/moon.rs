//! Deterministic mesh-phase moon generation.
//!
//! [`MoonGenerator::sample`] is the one terrain/material law: a pure function
//! of a unit direction and the world seed.  The immutable `MoonGenerator`
//! merely pre-expands the seed stream into crater/mare records so a mesh tile
//! or map raster does not regenerate that list for every pixel.  It carries no
//! clock or simulation state.
//!
//! The fold is chronological.  Craters are drawn oldest to newest; a younger
//! bowl replaces the terrain/material inside its impact floor while its rim
//! and ejecta disturb the terrain already there.  Rays are an albedo channel
//! first (with only a very small relief echo), and maria flatten the broad
//! base while darkening it.
//!
//! P3/P4 seams, intentionally data-free in this mesh-only phase:
//! `polar_ice_deposition`, `permanent_crater_shadow_ice`,
//! `underground_ice`, and `resource_reservoirs` belong in a future material
//! overlay evaluated *after* this sample.  None is guessed here.

use glam::DVec3;

use crate::noise::{fbm, gradient_noise, ridged_band};
use crate::planet::face_dir;
use crate::terrain::{TILE_QUADS, TileKey, TileMesh, Vertex};

/// Art/coverage constants for the seed stream.  Angular radii keep the law a
/// function of `(direction, seed)` and make the same face scale correctly if
/// a world uses a different physical Neisor radius.
pub mod tuning {
    /// Large basins: roughly 73--306 km radius on the configured moon.
    pub const LARGE_CRATERS: usize = 18;
    pub const LARGE_RADIUS_DEG: (f64, f64) = (1.8, 7.5);
    /// Mid-size craters: roughly 14--73 km radius.
    pub const MEDIUM_CRATERS: usize = 72;
    pub const MEDIUM_RADIUS_DEG: (f64, f64) = (0.35, 1.8);
    /// Local craters resolved by the near-moon tile levels: ~2.4--14 km.
    pub const SMALL_CRATERS: usize = 288;
    pub const SMALL_RADIUS_DEG: (f64, f64) = (0.06, 0.35);
    pub const TOTAL_CRATERS: usize = LARGE_CRATERS + MEDIUM_CRATERS + SMALL_CRATERS;

    /// Broad irregular dark plains.  Seven gives a readable face without
    /// turning the whole surface into mare.
    pub const MARIA: usize = 7;
    pub const MARE_RADIUS_DEG: (f64, f64) = (8.0, 19.0);

    /// Incidental shallow-angle strikes are deliberately uncommon.  At 2.5%
    /// the seed-42 field has only a handful over the full sphere.
    pub const ELONGATED_FRACTION: f64 = 0.025;
    /// Only the youngest 28% can retain rays; size and a second lottery thin
    /// that cohort further.
    pub const RAY_YOUNG_FRACTION: f64 = 0.28;
    pub const RAY_MAX_RADII: f64 = 7.5;

    /// Mesh LOD screen-space error.  With 32 quads/tile and level 14 this
    /// reaches about seven-metre vertex spacing on the configured moon.
    pub const LOD_ERROR_TARGET: f64 = 0.18;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonSample {
    /// Signed relief as a fraction of the physical moon radius.
    pub height_ratio: f64,
    /// Scalar reflectance before the renderer's lunar/copper tint.
    pub albedo: f64,
    /// 0 highlands/rough ejecta, 1 smooth mare floor.
    pub smoothness: f64,
    /// Surviving bright ray material (0..1), useful to later material stacks.
    pub ray: f64,
}

#[derive(Clone)]
struct Mare {
    center: DVec3,
    major: DVec3,
    minor: DVec3,
    radius: f64,
    elongation: f64,
    darkness: f64,
    floor_drop: f64,
    phase: f64,
    seed: i64,
}

#[derive(Clone)]
struct Crater {
    center: DVec3,
    major: DVec3,
    minor: DVec3,
    radius: f64,
    elongation: f64,
    depth_ratio: f64,
    rim_phase: f64,
    rim_lobes: f64,
    fresh_albedo: f64,
    large: bool,
    ray_strength: f64,
    ray_phase: f64,
    ray_arms: f64,
}

/// Immutable expansion of the moon seed.  Cloning is deterministic and has
/// no semantic effect; render workers normally share it through an `Arc`.
#[derive(Clone)]
pub struct MoonGenerator {
    seed: i64,
    maria: Vec<Mare>,
    craters: Vec<Crater>,
}

#[derive(Clone, Copy)]
enum CraterTier {
    Large,
    Medium,
    Small,
}

#[derive(Clone, Copy)]
struct SeedStream(u64);

impl SeedStream {
    fn new(seed: i64, salt: u64) -> Self {
        Self((seed as u64) ^ salt)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        // Exact 53-bit conversion, always in [0, 1).
        ((self.next_u64() >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
    }

    fn direction(&mut self) -> DVec3 {
        let z = self.unit() * 2.0 - 1.0;
        let a = self.unit() * std::f64::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        DVec3::new(r * a.cos(), r * a.sin(), z)
    }
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

fn log_range(lo: f64, hi: f64, u: f64) -> f64 {
    lo * (hi / lo).powf(u)
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

impl MoonGenerator {
    pub fn new(seed: i64) -> Self {
        let mut maria_stream = SeedStream::new(seed, 0x4D41_5249_415F_5032);
        let mut maria = Vec::with_capacity(tuning::MARIA);
        for i in 0..tuning::MARIA {
            // Three seed-jittered near-side anchors make the from-Neisor face
            // deliberately readable; the remaining plains stay uniform over
            // the sphere.  Both paths consume exactly two stream words.
            let center = if i < 3 {
                const ANCHORS: [(f64, f64); 3] = [(-18.0, -12.0), (19.0, 28.0), (3.0, 61.0)];
                let lat = (ANCHORS[i].0 + (maria_stream.unit() - 0.5) * 12.0).to_radians();
                let lon = (ANCHORS[i].1 + (maria_stream.unit() - 0.5) * 16.0).to_radians();
                DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
            } else {
                maria_stream.direction()
            };
            let phase = maria_stream.unit() * std::f64::consts::TAU;
            let (major, minor) = tangent_axes(center, phase);
            let radius = log_range(
                tuning::MARE_RADIUS_DEG.0,
                tuning::MARE_RADIUS_DEG.1,
                maria_stream.unit(),
            )
            .to_radians();
            maria.push(Mare {
                center,
                major,
                minor,
                radius,
                elongation: 1.05 + 0.35 * maria_stream.unit(),
                darkness: 0.18 + 0.13 * maria_stream.unit(),
                floor_drop: 0.00010 + 0.00020 * maria_stream.unit(),
                phase,
                seed: seed.wrapping_add(20_000 + i as i64 * 977),
            });
        }

        // A single chronological stream chooses among the three remaining
        // size cohorts.  Counts are exact, but sizes are interleaved rather
        // than making every small crater artificially younger than a basin.
        let mut stream = SeedStream::new(seed, 0x4352_4154_4552_5032);
        let mut remaining = [
            tuning::LARGE_CRATERS,
            tuning::MEDIUM_CRATERS,
            tuning::SMALL_CRATERS,
        ];
        let mut craters = Vec::with_capacity(tuning::TOTAL_CRATERS);
        for ordinal in 0..tuning::TOTAL_CRATERS {
            let total = remaining.iter().sum::<usize>();
            let mut pick = (stream.unit() * total as f64).floor() as usize;
            let mut tier_idx = 2usize;
            for (i, &count) in remaining.iter().enumerate() {
                if pick < count {
                    tier_idx = i;
                    break;
                }
                pick -= count;
            }
            remaining[tier_idx] -= 1;
            let tier = match tier_idx {
                0 => CraterTier::Large,
                1 => CraterTier::Medium,
                _ => CraterTier::Small,
            };
            let range = match tier {
                CraterTier::Large => tuning::LARGE_RADIUS_DEG,
                CraterTier::Medium => tuning::MEDIUM_RADIUS_DEG,
                CraterTier::Small => tuning::SMALL_RADIUS_DEG,
            };
            let center = stream.direction();
            let radius = log_range(range.0, range.1, stream.unit()).to_radians();
            let axis_angle = stream.unit() * std::f64::consts::TAU;
            let (major, minor) = tangent_axes(center, axis_angle);
            let elongated = stream.unit() < tuning::ELONGATED_FRACTION;
            let elongation = if elongated {
                1.25 + 0.50 * stream.unit()
            } else {
                1.0
            };
            let age = ordinal as f64 / (tuning::TOTAL_CRATERS - 1) as f64;
            let ray_lottery = stream.unit();
            let ray_strength = if age >= 1.0 - tuning::RAY_YOUNG_FRACTION
                && radius >= 0.24f64.to_radians()
                && ray_lottery < 0.72
            {
                (0.35 + 0.65 * stream.unit()) * smoothstep(0.24, 1.4, radius.to_degrees())
            } else {
                // Consume the same word on every branch: editing the ray gate
                // never re-rolls later crater locations.
                let _ = stream.unit();
                0.0
            };
            let depth_ratio = radius
                * match tier {
                    CraterTier::Large => 0.052 + 0.022 * stream.unit(),
                    CraterTier::Medium => 0.075 + 0.035 * stream.unit(),
                    CraterTier::Small => 0.105 + 0.050 * stream.unit(),
                };
            craters.push(Crater {
                center,
                major,
                minor,
                radius,
                elongation,
                depth_ratio,
                rim_phase: stream.unit() * std::f64::consts::TAU,
                rim_lobes: 6.0 + (stream.unit() * 8.0).floor(),
                fresh_albedo: 0.67 + 0.10 * age + 0.035 * (stream.unit() * 2.0 - 1.0),
                large: matches!(tier, CraterTier::Large),
                ray_strength,
                ray_phase: stream.unit() * std::f64::consts::TAU,
                ray_arms: 5.0 + (stream.unit() * 7.0).floor(),
            });
        }

        Self {
            seed,
            maria,
            craters,
        }
    }

    pub fn seed(&self) -> i64 {
        self.seed
    }

    pub fn crater_count(&self) -> usize {
        self.craters.len()
    }

    /// The sole surface law.  `direction` is normalized defensively so map,
    /// tests, and mesh callers cannot create subtly different faces.
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

        // Only broad wavelengths: the base is intentionally calmer than
        // Neisor.  Fine character comes from impacts, not mountain noise.
        let broad = fbm(direction, 4, 1.15, self.seed.wrapping_add(1_101));
        let soft_ridges = ridged_band(direction, 0, 2, 3.2, self.seed.wrapping_add(1_337));
        let local_soft = fbm(direction, 2, 74.0, self.seed.wrapping_add(1_619));
        let mut macro_height = broad * 0.00058 + soft_ridges * 0.00010 + local_soft * 0.000014;
        let albedo_noise = fbm(direction, 3, 2.1, self.seed.wrapping_add(2_003));
        let grain = gradient_noise(direction * 17.0, self.seed.wrapping_add(2_111));
        let fine_grain = gradient_noise(direction * 92.0, self.seed.wrapping_add(2_303));
        let mut albedo = 0.705 + 0.068 * albedo_noise + 0.022 * grain + 0.020 * fine_grain;
        let mut smoothness: f64 = 0.0;

        // Maria are old flooded basins: broad base relief is flattened before
        // younger craters fold over it, while albedo gets the stronger signal.
        for mare in &self.maria {
            let reach = mare.radius * mare.elongation * 1.22;
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
                + 0.12 * gradient_noise(direction * 7.0, mare.seed)
                + 0.055 * (5.0 * bearing + mare.phase).sin();
            let mask = 1.0 - smoothstep(0.76, 1.08, q * irregular);
            if mask <= 0.0 {
                continue;
            }
            macro_height = macro_height * (1.0 - 0.82 * mask) - mare.floor_drop * mask;
            albedo -= mare.darkness * mask;
            smoothness = smoothness.max(mask * 0.94);
        }

        let mut height = macro_height;
        let mut ray_seen: f64 = 0.0;
        // Oldest -> newest.  Interior override uses the pre-impact macro
        // datum, intentionally erasing an older bowl where a younger one hit;
        // rim/ejecta are additive over whatever survived at the edge.
        for crater in &self.craters {
            let max_radii = if crater.ray_strength > 0.0 {
                tuning::RAY_MAX_RADII
            } else {
                1.72
            };
            let reach = crater.radius * crater.elongation * max_radii;
            if reach < std::f64::consts::PI && direction.dot(crater.center) < reach.cos() {
                continue;
            }
            let (q0, bearing, theta) = feature_coords(
                direction,
                crater.center,
                crater.major,
                crater.minor,
                crater.radius,
                crater.elongation,
            );
            // Impact rims are never perfect circles: two deterministic
            // azimuthal harmonics perturb the wall/rim radius itself, while
            // the explicit elongation above remains reserved for rare
            // shallow-angle strikes.
            let rim_influence = smoothstep(0.58, 1.02, q0);
            let q = q0
                * (1.0
                    + rim_influence
                        * (0.035 * (crater.rim_lobes * bearing + crater.rim_phase).sin()
                            + 0.015
                                * ((crater.rim_lobes * 2.0 + 3.0) * bearing
                                    - crater.rim_phase * 0.7)
                                    .sin()));

            if q < 1.72 {
                let floor_shape = if q <= 0.56 {
                    0.96 - 0.08 * (q / 0.56).powi(2)
                } else {
                    0.88 * (1.0 - smoothstep(0.56, 1.02, q))
                };
                let mut target = macro_height - crater.depth_ratio * floor_shape;
                if crater.large {
                    // Broad central uplift with a small summit dimple: from
                    // orbit it reads as a peak; in flyby the center is not a
                    // mathematically perfect spike.
                    let peak = crater.depth_ratio * 0.58 * (-(q / 0.20).powi(2)).exp();
                    let dimple = crater.depth_ratio * 0.17 * (-(q / 0.050).powi(2)).exp();
                    target += peak - dimple;
                }
                let floor_texture = crater.depth_ratio
                    * 0.018
                    * gradient_noise(direction * 78.0, self.seed.wrapping_add(3_301))
                    * (1.0 - smoothstep(0.42, 0.84, q));
                target += floor_texture;
                let override_weight = 1.0 - smoothstep(0.70, 1.02, q);
                height = mix(height, target, override_weight);

                let ridge = (0.75
                    + 0.25 * (crater.rim_lobes * bearing + crater.rim_phase).sin()
                    + 0.10
                        * ((crater.rim_lobes * 2.0 + 3.0) * bearing - crater.rim_phase * 0.7)
                            .sin())
                .clamp(0.35, 1.15);
                let rim = (-((q - 1.0) / 0.105).powi(2)).exp();
                height += crater.depth_ratio * 0.28 * rim * ridge;
                let disturbed = smoothstep(1.72, 1.02, q)
                    * (0.55 + 0.45 * (17.0 * q + crater.rim_phase + 3.0 * bearing).sin());
                height += crater.depth_ratio * 0.032 * disturbed;

                let exposed = (override_weight * 0.70 + rim * 0.55).clamp(0.0, 1.0);
                albedo = mix(albedo, crater.fresh_albedo, exposed);
                albedo += 0.055 * rim * ridge;
                smoothness *= 1.0 - (rim * 0.62 + disturbed * 0.22).clamp(0.0, 0.82);
            }

            if crater.ray_strength > 0.0 {
                let radial = theta / crater.radius;
                if (1.05..tuning::RAY_MAX_RADII).contains(&radial) {
                    let angular_warp = 0.055 * (3.0 * bearing + crater.ray_phase * 0.7).sin()
                        + 0.025 * (7.0 * bearing - crater.ray_phase).sin();
                    let primary = (crater.ray_arms * (bearing + angular_warp) + crater.ray_phase)
                        .cos()
                        .abs()
                        .powf(10.0)
                        * (0.52
                            + 0.48
                                * ((crater.ray_arms * 2.0 + 3.0) * bearing
                                    - crater.ray_phase * 0.4)
                                    .cos()
                                    .abs()
                                    .powf(3.0));
                    let fork = ((crater.ray_arms + 2.0) * bearing - crater.ray_phase * 1.7)
                        .cos()
                        .abs()
                        .powf(14.0)
                        * 0.42;
                    let broken = (0.78
                        + 0.22
                            * gradient_noise(
                                direction * 54.0 + crater.center * 17.0,
                                self.seed.wrapping_add(4_201),
                            ))
                    .clamp(0.42, 1.0);
                    let fade = 1.0 - smoothstep(1.15, tuning::RAY_MAX_RADII, radial);
                    let ray = crater.ray_strength * primary.max(fork) * broken * fade;
                    // The visible identity is reflectance.  Relief is only a
                    // faint ejecta blanket, so rays never become mountain
                    // spokes when seen edge-on.
                    albedo += (0.96 - albedo) * ray * 0.90;
                    height += crater.radius * 0.00015 * ray;
                    ray_seen = ray_seen.max(ray);
                }
            }
        }

        MoonSample {
            height_ratio: height.clamp(-0.012, 0.009),
            albedo: albedo.clamp(0.16, 0.96),
            smoothness: smoothness.clamp(0.0, 1.0),
            ray: ray_seen.clamp(0.0, 1.0),
        }
    }

    pub fn height_km(&self, direction: DVec3, radius_km: f64) -> f64 {
        self.sample(direction).height_ratio * radius_km
    }
}

/// Convenience form for probes/tests.  Hot rendering paths should construct
/// one [`MoonGenerator`] and reuse its immutable seed expansion.
pub fn sample(direction: DVec3, seed: i64) -> MoonSample {
    MoonGenerator::new(seed).sample(direction)
}

/// Build one adaptive cube-sphere moon tile.  Positions and normals are
/// computed in f64 moon-local kilometres; the renderer later adds the f64
/// body center and subtracts the f64 camera before the f32 upload boundary.
pub fn build_tile(generator: &MoonGenerator, key: TileKey, radius_km: f64) -> TileMesh {
    let n = TILE_QUADS + 1;
    let np2 = n + 2;
    let (u0, v0, size) = key.uv_range();
    let face = key.face as usize;
    let origin = key.center_dir() * radius_km;
    let mut world = vec![DVec3::ZERO; np2 * np2];
    let mut samples = Vec::with_capacity(np2 * np2);
    for gj in 0..np2 {
        for gi in 0..np2 {
            let u = u0 + size * (gi as f64 - 1.0) / TILE_QUADS as f64;
            let v = v0 + size * (gj as f64 - 1.0) / TILE_QUADS as f64;
            let dir = face_dir(face, u, v);
            let sample = generator.sample(dir);
            world[gj * np2 + gi] = dir * (radius_km * (1.0 + sample.height_ratio));
            samples.push(sample);
        }
    }

    // Parent-triangle targets use the nested even child lattice.  Thus all
    // levels sample the exact same law, while new interior vertices slide in
    // without a visible split pop.
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
                // moon tiles skip biome reconstruction (payload-off) and
                // carry a neutral rain concavity in beach.w
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

    // Small inward skirts cover cross-level T-junctions.  They are moon-local
    // and therefore retain precision even when the body is 100,000 km away.
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
        let (lat, lon) = (lat.to_radians(), lon.to_radians());
        DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
    }

    #[test]
    fn crater_fold_is_deterministic() {
        let a = MoonGenerator::new(42);
        let b = MoonGenerator::new(42);
        assert_eq!(a.crater_count(), tuning::TOTAL_CRATERS);
        for d in [dir(0.0, 0.0), dir(23.5, -71.25), dir(-68.0, 144.0)] {
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
        assert!(lo < 0.62, "maria never became visibly dark: {lo}..{hi}");
        assert!(
            hi > 0.76,
            "rims/rays never became visibly bright: {lo}..{hi}"
        );
    }
}

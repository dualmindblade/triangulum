//! The coast: where the sea ends, as one curve, and the ground either side.
//!
//! planetgen's coastline is the 0.5 contour of `ocean_frac` (smooth at the
//! scale of its cells, about 140 m here). The ground near it used to decide
//! wet or dry column by column: sea level cut through the landform's noise,
//! and at 1/70 the bake's own height near a coast is a metre or so, less
//! than that noise, so the shoreline was the noise's contour. Gullies drew
//! stars and saw teeth, and a shallow sea shredded into sand and water blobs.
//!
//! Here the shoreline comes first and the ground follows it:
//! - `distance_field`: the signed distance from the coastline, worked out
//!   once from the bake's mask when it loads (no bake change), on the
//!   sphere, so it meets itself across cube edges. Interpolated Catmull-Rom,
//!   its zero is a smooth curve with no texels in it.
//! - `wiggle`: bounded detail added to that distance, so the curve is not
//!   glassy up close. Its slope stays under 1 and the distance's own slope
//!   is 1, so their sum has no local maximum or minimum: the wiggle moves the
//!   shoreline but can never make an island, a pond or a fold. Every piece
//!   of land and sea in the world is one the planet has.
//! - `shape`: the ground as a function of that distance, in metres, so it is
//!   the same beach at every vertical scale and block size. Land stays
//!   above the sea and rises at a beach's slope; the sea floor stays below
//!   it and falls at the same slope, to at least `SEA_DEPTH_M`; the natural
//!   relief fades back in over `REACH_M` either side.
//!
//! All of it is a pure function of (seed, direction, voxel size).

use super::noise::gradient_noise;
use cubesphere::FACES;
use glam::DVec3;
use rayon::prelude::*;
use std::f64::consts::FRAC_PI_4;

/// How far the distance field reaches either side of the coastline, in bake
/// texels (about 1.1 km on Neisor); past it, it holds that value.
const FIELD_TEXELS: i64 = 8;

/// The beach's slope (rise over run), above and below the water.
pub const BEACH_SLOPE: f64 = 0.035;
/// Height the beach levels out at (m).
pub const BEACH_TOP_M: f64 = 1.6;
/// Shallowest the sea gets away from its shore (m).
pub const SEA_DEPTH_M: f64 = 2.0;
/// Distance from the shoreline at which the natural relief is fully back (m).
pub const REACH_M: f64 = 140.0;
/// The land's floor holds to here and is gone here (m): low coasts get a
/// beach ridge, and whatever lies behind it takes over.
const RIDGE_START_M: f64 = 160.0;
const RIDGE_END_M: f64 = 320.0;
/// Beach width (m) and how far it varies along the coast (a fraction).
const BEACH_WIDTH_M: f64 = 70.0;
const BEACH_VARY: f64 = 0.45;
/// How far the sand runs out under the water, as a share of the beach's width
/// (past it the sea floor is gravel: sand under 0.62 water reads grey, and a
/// shelf of it would turn the sea grey from orbit).
const SHALLOWS_SHARE: f64 = 0.6;
/// Sand stops at this height above the sea (m): a cliff has no beach up it.
pub const BEACH_SAND_TOP_M: f64 = 4.0;
/// The wiggle: longest wavelength (m), octaves (each half the last) and
/// amplitude as a fraction of the wavelength. `wiggle_stays_under_a_slope_of_one`
/// holds these to the promise above.
const WIGGLE_TOP_M: f64 = 1024.0;
const WIGGLE_OCTAVES: u32 = 7;
pub const WIGGLE_AMP: f64 = 0.04;
/// Largest |wiggle| anywhere (m).
pub const WIGGLE_MAX_M: f64 = WIGGLE_AMP * 2.0 * WIGGLE_TOP_M;
/// Beyond this distance from the coastline, before the wiggle, nothing here
/// needs working out (m).
pub const BAND_M: f64 = RIDGE_END_M + WIGGLE_MAX_M;

/// A column's place relative to the shore.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shore {
    /// Distance from the shoreline (m): positive at sea, negative inland;
    /// `BAND_M` or more means "far from it".
    pub d: f64,
    /// On the sea's side of the shoreline.
    pub sea: bool,
    /// On the beach: dry sand above the waterline, the sand of the shallows
    /// below it.
    pub beach: bool,
}

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Octave fade for voxels of `voxel_m`, as the landform's.
#[inline]
fn band(wl: f64, voxel_m: f64) -> f64 {
    (wl / (4.0 * voxel_m) - 1.0).clamp(0.0, 1.0)
}

/// Detail added to the distance from the coastline (m), band-limited.
pub fn wiggle(p: DVec3, voxel_m: f64, seed: u64) -> f64 {
    let mut sum = 0.0;
    let mut wl = WIGGLE_TOP_M;
    for k in 0..WIGGLE_OCTAVES {
        let fade = band(wl, voxel_m);
        if fade <= 0.0 {
            break;
        }
        sum += WIGGLE_AMP * wl * fade * gradient_noise(p / wl + DVec3::splat(k as f64 * 7.31), seed.wrapping_add(k as u64 * 0x3C5));
        wl *= 0.5;
    }
    sum
}

/// Where a column lies relative to the shore. `coast_m` is the bake's signed
/// distance in metres; `p` the column's position on the sphere (m).
pub fn shore_at(coast_m: f64, p: DVec3, voxel_m: f64, seed: u64) -> Shore {
    if coast_m.abs() >= BAND_M.max(2.0 * voxel_m) {
        return Shore { d: coast_m, sea: coast_m > 0.0, beach: false };
    }
    let d = coast_m + wiggle(p, voxel_m, seed ^ 0xC0A5);
    let sea = d > 0.0;
    // Wider along some stretches of coast than others, with a soft inland
    // edge, and never so narrow that a coarse voxel steps over it.
    let n = gradient_noise(p / 700.0, seed ^ 0xBEAC) + 0.35 * gradient_noise(p / 130.0, seed ^ 0xBEAD) + 0.15 * gradient_noise(p / 24.0, seed ^ 0xBEAE);
    let width = (BEACH_WIDTH_M * (1.0 + BEACH_VARY * n)).max(1.5 * voxel_m);
    let beach = if sea { d < SHALLOWS_SHARE * width } else { -d < width };
    Shore { d, sea, beach }
}

/// The ground at distance `d` from the shoreline, given the natural height.
#[inline]
pub fn shape(h: f64, d: f64) -> f64 {
    if d > 0.0 {
        if d > REACH_M {
            return h.min(-SEA_DEPTH_M * (1.0 - (-d * BEACH_SLOPE / SEA_DEPTH_M).exp()));
        }
        let c = -SEA_DEPTH_M * (1.0 - (-d * BEACH_SLOPE / SEA_DEPTH_M).exp());
        c + smoothstep(0.0, REACH_M, d) * (h - c).min(0.0)
    } else {
        let x = -d;
        if x >= RIDGE_END_M {
            return h;
        }
        let rim = BEACH_TOP_M * (1.0 - (-x * BEACH_SLOPE / BEACH_TOP_M).exp()) * (1.0 - smoothstep(RIDGE_START_M, RIDGE_END_M, x));
        rim + smoothstep(0.0, REACH_M, x) * (h - rim).max(0.0)
    }
}

/// The signed angle (radians) from the coastline at every texel of every
/// face: positive at sea. `ocean[f]` is face f's mask, `res` x `res` texels,
/// uniform in the equal-angle parameter when `equiangular` and in the gnomonic
/// coordinate otherwise. The coastline is the mask's 0.5 contour, by marching
/// squares; the distance is taken on the sphere to its segments from every
/// face, so the field agrees with itself along cube edges.
pub fn distance_field(ocean: &[&[f32]], res: usize, equiangular: bool) -> Vec<Vec<f32>> {
    let m = (res - 1) as f64;
    let unparam = |p: f64| if equiangular { (p * FRAC_PI_4).tan() } else { p };
    let param = |a: f64| if equiangular { a.atan() / FRAC_PI_4 } else { a };
    let coords: Vec<f64> = (0..res).map(|t| unparam(t as f64 / m * 2.0 - 1.0)).collect();
    let dir_at = |f: usize, x: f64, y: f64| {
        let (axis, right, up) = FACES[f];
        (axis + unparam(x / m * 2.0 - 1.0) * right + unparam(y / m * 2.0 - 1.0) * up).normalize()
    };
    // Segments of the 0.5 contour, face by face.
    let segs: Vec<(DVec3, DVec3)> = (0..ocean.len())
        .into_par_iter()
        .flat_map_iter(|f| {
            let o = ocean[f];
            let mut out = Vec::new();
            let at = |x: usize, y: usize| o[y * res + x] as f64 - 0.5;
            for y in 0..res - 1 {
                for x in 0..res - 1 {
                    let (v00, v10, v01, v11) = (at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1));
                    let up = |v: f64| v >= 0.0;
                    if up(v00) == up(v10) && up(v00) == up(v01) && up(v00) == up(v11) {
                        continue;
                    }
                    let (fx, fy) = (x as f64, y as f64);
                    // Crossings in order round the cell: bottom, right, top, left.
                    let cross = |a: f64, b: f64| a / (a - b);
                    let mut p: [Option<(f64, f64)>; 4] = [None; 4];
                    if up(v00) != up(v10) {
                        p[0] = Some((fx + cross(v00, v10), fy));
                    }
                    if up(v10) != up(v11) {
                        p[1] = Some((fx + 1.0, fy + cross(v10, v11)));
                    }
                    if up(v01) != up(v11) {
                        p[2] = Some((fx + cross(v01, v11), fy + 1.0));
                    }
                    if up(v00) != up(v01) {
                        p[3] = Some((fx, fy + cross(v00, v01)));
                    }
                    let mut join = |a: (f64, f64), b: (f64, f64)| out.push((dir_at(f, a.0, a.1), dir_at(f, b.0, b.1)));
                    match (p[0], p[1], p[2], p[3]) {
                        (Some(b), Some(r), Some(t), Some(l)) => {
                            // A saddle: the centre decides which corners join.
                            if up(0.25 * (v00 + v10 + v01 + v11)) == up(v00) {
                                join(b, r);
                                join(t, l);
                            } else {
                                join(b, l);
                                join(r, t);
                            }
                        }
                        _ => {
                            let found: Vec<(f64, f64)> = p.iter().flatten().copied().collect();
                            if let [a, b] = found[..] {
                                join(a, b);
                            }
                        }
                    }
                }
            }
            out
        })
        .collect();
    let cap = FIELD_TEXELS as f64 * std::f64::consts::FRAC_PI_2 / m;
    (0..ocean.len())
        .into_par_iter()
        .map(|f| {
            let (axis, right, up) = FACES[f];
            let mut best = vec![cap * cap; res * res];
            for &(a, b) in &segs {
                let mid = (a + b).normalize();
                let z = mid.dot(axis);
                if z <= 0.2 {
                    continue;
                }
                let (cx, cy) = ((param(mid.dot(right) / z) * 0.5 + 0.5) * m, (param(mid.dot(up) / z) * 0.5 + 0.5) * m);
                let reach = FIELD_TEXELS as f64 + 1.0;
                if cx < -reach || cy < -reach || cx > m + reach || cy > m + reach {
                    continue;
                }
                let (x0, x1) = ((cx - reach).ceil().max(0.0) as usize, (cx + reach).floor().min(m) as usize);
                let (y0, y1) = ((cy - reach).ceil().max(0.0) as usize, (cy + reach).floor().min(m) as usize);
                let e = b - a;
                let ee = e.length_squared();
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let d = (axis + coords[x] * right + coords[y] * up).normalize();
                        let t = if ee > 0.0 { ((d - a).dot(e) / ee).clamp(0.0, 1.0) } else { 0.0 };
                        let v = (d - (a + e * t)).length_squared();
                        let c = &mut best[y * res + x];
                        if v < *c {
                            *c = v;
                        }
                    }
                }
            }
            let o = ocean[f];
            best.iter().zip(o).map(|(v, s)| if *s >= 0.5 { v.sqrt() as f32 } else { -v.sqrt() as f32 }).collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ground function is continuous, keeps land above the sea and the
    /// sea floor below it, and gives the natural height back away from the shore.
    #[test]
    fn the_shore_profile_is_continuous_and_keeps_its_sides() {
        for h in [-80.0, -5.0, -1.0, -0.2, 0.0, 0.3, 1.0, 4.0, 30.0, 400.0] {
            let mut prev = shape(h, -RIDGE_END_M - 5.0);
            let mut d = -RIDGE_END_M - 5.0 + 0.25;
            while d < 600.0 {
                let v = shape(h, d);
                let step = 0.25 * (BEACH_SLOPE + 2.0 * (h.abs() + 3.0) / REACH_M);
                assert!((v - prev).abs() < step, "h {h}: jump {} at {d}", (v - prev).abs());
                if d > 0.0 {
                    assert!(v < 0.0, "h {h}: ground {v} at sea, {d} m out");
                } else if d < 0.0 {
                    assert!(v > 0.0 || d < -RIDGE_END_M, "h {h}: ground {v} on land, {} m in", -d);
                }
                prev = v;
                d += 0.25;
            }
            assert_eq!(shape(h, -RIDGE_END_M - 1.0), h, "inland the ground is the natural ground");
            if h < -SEA_DEPTH_M {
                assert_eq!(shape(h, 2000.0), h, "out at sea the floor is the natural floor");
            }
        }
    }

    /// The promise in the module header: the wiggle's slope stays under 1, so
    /// it cannot fold the shoreline or make an island.
    #[test]
    fn wiggle_stays_under_a_slope_of_one() {
        let r = 90_569.0;
        let mut worst: f64 = 0.0;
        let step = 0.5;
        for k in 0..200_000u32 {
            let t = k as f64 * 0.618_033_988_749_895;
            let (a, b) = (t.fract() * std::f64::consts::TAU, (t * 0.381_966).fract() * 2.0 - 1.0);
            let s = (1.0 - b * b).sqrt();
            let d = DVec3::new(s * a.cos(), s * a.sin(), b);
            let e1 = d.cross(DVec3::new(0.3, 0.5, 0.8)).normalize();
            let e2 = d.cross(e1);
            let at = |u: f64, v: f64| wiggle((d + e1 * (u / r) + e2 * (v / r)).normalize() * r, 1.0, 42);
            let (gx, gy) = ((at(step, 0.0) - at(-step, 0.0)) / (2.0 * step), (at(0.0, step) - at(0.0, -step)) / (2.0 * step));
            worst = worst.max((gx * gx + gy * gy).sqrt());
        }
        println!("wiggle: steepest slope {worst:.3}");
        assert!(worst < 1.0, "wiggle slope {worst}");
    }

    /// A straight coast along a face's middle: the field is the distance to
    /// it, with the right sign, on the sphere.
    #[test]
    fn the_distance_field_measures_from_the_contour() {
        let res = 65usize;
        let m = (res - 1) as f64;
        // Face 0: sea at columns past the middle.
        let mask: Vec<Vec<f32>> = (0..6)
            .map(|f| {
                (0..res * res)
                    .map(|i| {
                        let x = (i % res) as f64;
                        if f == 0 { ((x - 0.5 * m) / 2.0 + 0.5).clamp(0.0, 1.0) as f32 } else { 0.0 }
                    })
                    .collect()
            })
            .collect();
        let refs: Vec<&[f32]> = mask.iter().map(|v| v.as_slice()).collect();
        let field = distance_field(&refs, res, true);
        let texel = std::f64::consts::FRAC_PI_2 / m;
        let row = 32 * res;
        for dx in [-3i64, -2, -1, 1, 2, 3] {
            let v = field[0][row + (32 + dx) as usize] as f64 / texel;
            assert!((v - dx as f64).abs() < 0.2, "{dx} texels from the coast: {v} texels");
        }
        // Far from any coast it holds its cap, on the land's side.
        assert!((field[0][row] as f64 / texel + FIELD_TEXELS as f64).abs() < 1e-3);
        assert!(field[3].iter().all(|v| *v < 0.0), "face 3 is all land");
    }
}

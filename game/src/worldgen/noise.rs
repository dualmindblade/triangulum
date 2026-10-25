//! Seeded gradient (Perlin) noise on 3D positions. Gradient noise has no
//! lattice-aligned plateaus, unlike value noise, so it leaves no grid
//! artifacts in relief.

use glam::DVec3;

#[inline]
fn hash(x: i64, y: i64, z: i64, seed: u64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ seed.wrapping_mul(0x85EB_CA77_C2B2_AE63);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

/// Uniform [0, 1) from a lattice point.
#[inline]
pub fn hash01(x: i64, y: i64, z: i64, seed: u64) -> f64 {
    (hash(x, y, z, seed) >> 11) as f64 / (1u64 << 53) as f64
}

/// Perlin's 12 cube-edge gradients (plus 4 repeats), picked by the low hash bits.
#[inline]
fn grad(h: u64, d: DVec3) -> f64 {
    match h & 15 {
        0 => d.x + d.y,
        1 => -d.x + d.y,
        2 => d.x - d.y,
        3 => -d.x - d.y,
        4 => d.x + d.z,
        5 => -d.x + d.z,
        6 => d.x - d.z,
        7 => -d.x - d.z,
        8 => d.y + d.z,
        9 => -d.y + d.z,
        10 => d.y - d.z,
        11 => -d.y - d.z,
        12 => d.y + d.x,
        13 => -d.y + d.z,
        14 => d.y - d.x,
        _ => -d.y - d.z,
    }
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Gradient noise at `p`, roughly in [-1, 1].
pub fn gradient_noise(p: DVec3, seed: u64) -> f64 {
    let pi = p.floor();
    let pf = p - pi;
    let (ix, iy, iz) = (pi.x as i64, pi.y as i64, pi.z as i64);
    let (fx, fy, fz) = (fade(pf.x), fade(pf.y), fade(pf.z));
    let mut total = 0.0;
    for dx in 0..2i64 {
        let wx = if dx == 1 { fx } else { 1.0 - fx };
        for dy in 0..2i64 {
            let wy = if dy == 1 { fy } else { 1.0 - fy };
            for dz in 0..2i64 {
                let wz = if dz == 1 { fz } else { 1.0 - fz };
                let h = hash(ix + dx, iy + dy, iz + dz, seed);
                let d = pf - DVec3::new(dx as f64, dy as f64, dz as f64);
                total += wx * wy * wz * grad(h, d);
            }
        }
    }
    total * 1.35
}

/// Fractal sum: `octaves` gradient octaves starting at wavelength `wl`,
/// each `lacunarity` times finer and `gain` times weaker. Returns the sum
/// of amp_i * noise_i (unnormalised; the first octave has amplitude 1).
/// `octave_weight(octave, wavelength)` scales each octave (LOD band
/// limits, slope caps).
pub fn fbm(p: DVec3, octaves: u32, wl: f64, lacunarity: f64, gain: f64, seed: u64, octave_weight: impl Fn(u32, f64) -> f64) -> f64 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut w = wl;
    for o in 0..octaves {
        let k = octave_weight(o, w);
        if k > 0.0 {
            let q = p / w + DVec3::splat(o as f64 * 19.17);
            sum += gradient_noise(q, seed.wrapping_add(o as u64 * 0x9E37_79B9)) * amp * k;
        }
        amp *= gain;
        w /= lacunarity;
    }
    sum
}

/// 32 unit gradients for 2D noise, evenly spread in angle and offset from
/// the axes so no direction is favoured.
const GRAD2: [(f64, f64); 32] = {
    let mut g = [(0.0, 0.0); 32];
    let mut i = 0;
    while i < 32 {
        // cos/sin by Taylor series (const context): angle = (i + 0.37) * 2 pi / 32.
        let a = (i as f64 + 0.37) * std::f64::consts::TAU / 32.0;
        let a = if a > std::f64::consts::PI { a - std::f64::consts::TAU } else { a };
        let mut c = 0.0;
        let mut s = 0.0;
        let mut term = 1.0;
        let mut k = 0;
        while k < 30 {
            // term = a^k / k!
            match k % 4 {
                0 => c += term,
                1 => s += term,
                2 => c -= term,
                _ => s -= term,
            }
            k += 1;
            term *= a / k as f64;
        }
        g[i] = (c, s);
        i += 1;
    }
    g
};

#[inline]
fn hash2(x: i64, y: i64, seed: u64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed;
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 32;
    h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^ (h >> 32)
}

/// 2D gradient noise with its analytic derivative: (value, d/dx, d/dy),
/// value roughly in [-1, 1] (standard deviation ~0.29).
#[inline]
pub fn noise2d(x: f64, y: f64, seed: u64) -> (f64, f64, f64) {
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let (ix, iy) = (xf as i64, yf as i64);
    let u = fade(fx);
    let v = fade(fy);
    let du = 30.0 * fx * fx * (fx - 1.0) * (fx - 1.0);
    let dv = 30.0 * fy * fy * (fy - 1.0) * (fy - 1.0);
    let g00 = GRAD2[(hash2(ix, iy, seed) & 31) as usize];
    let g10 = GRAD2[(hash2(ix + 1, iy, seed) & 31) as usize];
    let g01 = GRAD2[(hash2(ix, iy + 1, seed) & 31) as usize];
    let g11 = GRAD2[(hash2(ix + 1, iy + 1, seed) & 31) as usize];
    let n00 = g00.0 * fx + g00.1 * fy;
    let n10 = g10.0 * (fx - 1.0) + g10.1 * fy;
    let n01 = g01.0 * fx + g01.1 * (fy - 1.0);
    let n11 = g11.0 * (fx - 1.0) + g11.1 * (fy - 1.0);
    let k = n00 - n10 - n01 + n11;
    let val = n00 + u * (n10 - n00) + v * (n01 - n00) + u * v * k;
    let dx = g00.0 + u * (g10.0 - g00.0) + v * (g01.0 - g00.0) + u * v * (g00.0 - g10.0 - g01.0 + g11.0) + du * (n10 - n00 + v * k);
    let dy = g00.1 + u * (g10.1 - g00.1) + v * (g01.1 - g00.1) + u * v * (g00.1 - g10.1 - g01.1 + g11.1) + dv * (n01 - n00 + u * k);
    const S: f64 = std::f64::consts::SQRT_2;
    (val * S, dx * S, dy * S)
}

/// sin and cos to ~1e-7: quadrant reduction to [-pi/4, pi/4] and short
/// Taylor polynomials. Plain arithmetic, so as deterministic as the rest.
#[inline]
pub fn fast_sin_cos(x: f64) -> (f64, f64) {
    use std::f64::consts::FRAC_PI_2;
    let q = (x * (1.0 / FRAC_PI_2)).round();
    let r = x - q * FRAC_PI_2;
    let r2 = r * r;
    let s = r * (1.0 - r2 * (1.0 / 6.0 - r2 * (1.0 / 120.0 - r2 * (1.0 / 5040.0))));
    let c = 1.0 - r2 * (0.5 - r2 * (1.0 / 24.0 - r2 * (1.0 / 720.0 - r2 * (1.0 / 40320.0))));
    match (q as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// Slope-aligned gully noise ("erosion noise", after clayjohn's and Fewes'
/// eroded-terrain shaders): every cell of a jittered grid carries a cosine
/// stripe pattern through its point whose crests run along the downhill
/// direction; neighbouring cells blend with a compact C1 kernel (1 - r^2/R^2)^2, so the
/// stripes merge and split where their phases disagree, which reads as
/// branching drainage. `(tx, ty)` is the unit contour direction (across
/// the slope) in the same units as `(x, y)` (cells). Returns (value in
/// [-1, 1], d/dx, d/dy); +1 is a spur crest, -1 a gully floor. The
/// derivative ignores the kernel's own slope (it steers finer octaves).
/// Exact coverage: points jitter +-0.25 around cell centres and the kernel
/// radius is 1.25 cells, so the 3x3 neighbourhood holds every point that
/// can reach (x, y) and the point of the cell containing (x, y) always does.
#[inline]
pub fn gully2d(x: f64, y: f64, tx: f64, ty: f64, seed: u64) -> (f64, f64, f64) {
    use std::f64::consts::TAU;
    let (xf, yf) = (x.floor(), y.floor());
    let (ix, iy) = (xf as i64, yf as i64);
    let (mut sw, mut sv, mut sd) = (0.0, 0.0, 0.0);
    const INV_R2: f64 = 1.0 / (1.25 * 1.25);
    for j in -1..=1i64 {
        for i in -1..=1i64 {
            let h = hash2(ix + i, iy + j, seed);
            let jx = ((h & 0xFFFF) as f64 * (1.0 / 65535.0) - 0.5) * 0.5;
            let jy = (((h >> 16) & 0xFFFF) as f64 * (1.0 / 65535.0) - 0.5) * 0.5;
            let dx = x - (xf + i as f64 + 0.5 + jx);
            let dy = y - (yf + j as f64 + 0.5 + jy);
            let r2 = (dx * dx + dy * dy) * INV_R2;
            if r2 >= 1.0 {
                continue;
            }
            let a = 1.0 - r2;
            let w = a * a;
            let (s, c) = (TAU * (dx * tx + dy * ty)).sin_cos();
            sw += w;
            sv += w * c;
            sd -= w * s;
        }
    }
    let inv = 1.0 / sw;
    (sv * inv, sd * inv * TAU * tx, sd * inv * TAU * ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise2d_derivative_matches_finite_differences_and_is_bounded() {
        let mut mx: f64 = 0.0;
        let mut sum2 = 0.0;
        let n = 20_000;
        for i in 0..n {
            let (x, y) = (i as f64 * 0.1371 + 0.05, (i as f64 * 0.0917).sin() * 40.0 + i as f64 * 0.013);
            let (v, dx, dy) = noise2d(x, y, 9);
            mx = mx.max(v.abs());
            sum2 += v * v;
            let h = 1e-6;
            let fx = (noise2d(x + h, y, 9).0 - noise2d(x - h, y, 9).0) / (2.0 * h);
            let fy = (noise2d(x, y + h, 9).0 - noise2d(x, y - h, 9).0) / (2.0 * h);
            assert!((fx - dx).abs() < 1e-5 && (fy - dy).abs() < 1e-5, "at ({x}, {y}): {dx} vs {fx}, {dy} vs {fy}");
        }
        assert!(mx <= 1.0 && mx > 0.6, "max |noise2d| {mx}");
        let sd = (sum2 / n as f64).sqrt();
        assert!((0.2..0.4).contains(&sd), "noise2d std {sd}");
        assert_eq!(noise2d(3.0, -4.0, 1).0, 0.0);
    }

    #[test]
    fn fast_sin_cos_is_accurate() {
        let mut worst: f64 = 0.0;
        for k in -200_000..200_000 {
            let x = k as f64 * 1.37e-4;
            let (s, c) = fast_sin_cos(x);
            worst = worst.max((s - x.sin()).abs()).max((c - x.cos()).abs());
        }
        assert!(worst < 1e-6, "worst error {worst}");
    }

    #[test]
    fn gully2d_is_bounded_continuous_and_follows_the_stripe_direction() {
        // Continuity across cell borders: tiny steps change the value a little.
        let (tx, ty) = (0.6, 0.8);
        let mut prev = gully2d(-3.0, 0.3 - 3.0 * 0.37, tx, ty, 5).0;
        let mut worst: f64 = 0.0;
        for k in 1..60_000 {
            let x = -3.0 + k as f64 * 1e-4;
            let v = gully2d(x, 0.3 + x * 0.37, tx, ty, 5).0;
            assert!(v.abs() <= 1.0 + 1e-12);
            worst = worst.max((v - prev).abs());
            prev = v;
        }
        assert!(worst < 5e-3, "largest jump over 1e-4 cells: {worst}");
        // Along the stripes (perpendicular to t) the value varies much less than across them.
        let (mut along, mut across) = (0.0, 0.0);
        for k in 0..4000 {
            let (x, y) = (k as f64 * 0.731 % 37.0, k as f64 * 0.377 % 41.0);
            let (_, dx, dy) = gully2d(x, y, tx, ty, 5);
            across += (dx * tx + dy * ty).abs();
            along += (dx * -ty + dy * tx).abs();
        }
        assert!(along < 1e-9 * across + 1e-12, "the analytic derivative lies across the stripes");
    }

    #[test]
    fn noise_is_bounded_zero_at_lattice_and_seeded() {
        let mut mx: f64 = 0.0;
        for i in 0..2000 {
            let p = DVec3::new(i as f64 * 0.37, (i as f64 * 0.11).sin() * 5.0, i as f64 * 0.071);
            let v = gradient_noise(p, 7);
            mx = mx.max(v.abs());
        }
        assert!(mx <= 1.0 && mx > 0.5, "max |noise| {mx}");
        assert_eq!(gradient_noise(DVec3::new(3.0, 4.0, 5.0), 7), 0.0);
        assert_ne!(gradient_noise(DVec3::new(3.3, 4.1, 5.7), 7), gradient_noise(DVec3::new(3.3, 4.1, 5.7), 8));
        assert_eq!(gradient_noise(DVec3::new(3.3, 4.1, 5.7), 7), gradient_noise(DVec3::new(3.3, 4.1, 5.7), 7));
    }
}

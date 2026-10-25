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
/// `octave_weight(i, wavelength)` scales each octave (for LOD band limits).
pub fn fbm(p: DVec3, octaves: u32, wl: f64, lacunarity: f64, gain: f64, seed: u64, octave_weight: impl Fn(f64) -> f64) -> f64 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut w = wl;
    for o in 0..octaves {
        let k = octave_weight(w);
        if k > 0.0 {
            let q = p / w + DVec3::splat(o as f64 * 19.17);
            sum += gradient_noise(q, seed.wrapping_add(o as u64 * 0x9E37_79B9)) * amp * k;
        }
        amp *= gain;
        w /= lacunarity;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

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

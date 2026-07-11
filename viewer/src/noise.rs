//! Exact Rust port of planetgen/noise.py — same hash, same gradient table,
//! same octave scheme, so the same seed produces the same planet in both
//! worlds. Equality is enforced by tests/golden.rs against values generated
//! by the Python original (including its int64 wrap-around behavior, which
//! `wrapping_mul`/`wrapping_add` reproduce bit-exactly).

use crate::noise_grad::GRAD;
use glam::DVec3;

const MASK: i64 = 0xFFFF_FFFF;

#[inline]
fn hash(ix: i64, iy: i64, iz: i64, seed: i64) -> usize {
    let mut h = ix
        .wrapping_mul(0x8DA6_B343)
        .wrapping_add(iy.wrapping_mul(0xD816_3841))
        .wrapping_add(iz.wrapping_mul(0xCB1A_B31F))
        .wrapping_add(seed.wrapping_mul(0x9E37_79B1))
        & MASK;
    h = (h ^ (h >> 13)).wrapping_mul(0xC2B2_AE35) & MASK;
    h ^= h >> 16;
    (h & 255) as usize
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// A separate 64-bit lattice hash for procedural fields that need scalar
/// values rather than the legacy 256-entry gradient table. Keeping this out
/// of `hash` preserves the Python/Rust terrain-noise goldens byte-for-byte.
#[inline]
fn scalar_hash(ix: i64, iy: i64, iz: i64, seed: i64) -> u64 {
    let mut h = (ix as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (iz as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ (seed as u64).wrapping_mul(0x85EB_CA77_C2B2_AE63);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 31)
}

/// One cheap, approximately standard-normal lattice variate. The population
/// count of a well-mixed 64-bit word is Binomial(64, 1/2), with mean 32 and
/// variance 16. It is close to Gaussian and compiles to one popcount instead
/// of spending a dozen additions at every corner of every octave.
#[inline]
fn lattice_normal(bits: u64) -> f64 {
    (bits.count_ones() as f64 - 32.0) * 0.25
}

/// Seamless scalar value noise with an approximately N(0,1) marginal.
///
/// Trilinear interpolation normally has lower variance in the middle of a
/// lattice cell than at its corners. Dividing by the exact L2 norm of the
/// eight interpolation weights removes that phase-dependent variance. This
/// matters when a caller applies a CDF: every position should have the same
/// occupancy statistics, not just the lattice points.
pub fn normal_value_noise(p: DVec3, seed: i64) -> f64 {
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
                total += wx
                    * wy
                    * wz
                    * lattice_normal(scalar_hash(ix + dx, iy + dy, iz + dz, seed));
            }
        }
    }
    let norm2 = ((1.0 - fx).powi(2) + fx.powi(2))
        * ((1.0 - fy).powi(2) + fy.powi(2))
        * ((1.0 - fz).powi(2) + fz.powi(2));
    total / norm2.sqrt()
}

/// Perlin-style gradient noise at p; output roughly [-1, 1].
pub fn gradient_noise(p: DVec3, seed: i64) -> f64 {
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
                let g = GRAD[hash(ix + dx, iy + dy, iz + dz, seed)];
                let d = pf - DVec3::new(dx as f64, dy as f64, dz as f64);
                total += wx * wy * wz * (g[0] * d.x + g[1] * d.y + g[2] * d.z);
            }
        }
    }
    total * 1.9
}

pub fn fbm(p: DVec3, octaves: u32, freq: f64, seed: i64) -> f64 {
    let (mut total, mut amp, mut f, mut norm) = (0.0, 1.0, freq, 0.0);
    for o in 0..octaves as i64 {
        total += amp * gradient_noise(p * f, seed.wrapping_mul(7919).wrapping_add(o * 131));
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    total / norm
}

pub fn ridged(p: DVec3, octaves: u32, freq: f64, seed: i64) -> f64 {
    let (mut total, mut amp, mut f, mut norm) = (0.0, 1.0, freq, 0.0);
    for o in 0..octaves as i64 {
        let n = 1.0
            - gradient_noise(p * f, seed.wrapping_mul(104_729).wrapping_add(o * 131)).abs();
        total += amp * n * n;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    total / norm
}

/// fbm evaluated only over octaves [first, first+count) of the full stack —
/// the per-LOD "band" trick: a quadtree node adds exactly the octaves its
/// resolution can carry, coarser nodes carry fewer. Same seeds per octave
/// index as `fbm`, so bands from different levels sum consistently.
pub fn fbm_band(p: DVec3, first: u32, count: u32, base_freq: f64, seed: i64) -> f64 {
    let mut total = 0.0;
    let mut amp = 0.5f64.powi(first as i32);
    let mut f = base_freq * 2.0f64.powi(first as i32);
    for o in first as i64..(first + count) as i64 {
        total += amp * gradient_noise(p * f, seed.wrapping_mul(7919).wrapping_add(o * 131));
        amp *= 0.5;
        f *= 2.0;
    }
    total
}

pub fn ridged_band(p: DVec3, first: u32, count: u32, base_freq: f64, seed: i64) -> f64 {
    let mut total = 0.0;
    let mut amp = 0.5f64.powi(first as i32);
    let mut f = base_freq * 2.0f64.powi(first as i32);
    for o in first as i64..(first + count) as i64 {
        let n = 1.0
            - gradient_noise(p * f, seed.wrapping_mul(104_729).wrapping_add(o * 131)).abs();
        total += amp * (n * n - 0.5);
        amp *= 0.5;
        f *= 2.0;
    }
    total
}

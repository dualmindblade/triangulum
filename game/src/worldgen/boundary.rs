//! Boundaries: every line between two kinds of ground is one line, rough at
//! every scale from the size of the thing it bounds down to a block, that
//! does not break up into scattered pieces.
//!
//! The climate fields (and the lake field) are smooth, so their contours are
//! smooth curves; the old rule added one 41 m dither to the value before the
//! threshold. Across a gentle gradient a fixed number of degrees moves a line
//! a long way (5.6 km at the polar ice edge), so the dither decided a band
//! kilometres wide: scatter. Where the gradient is steep the line stayed the
//! bake's contour: round from afar.
//!
//! Here nothing is added to a value. The fields are read somewhere else: a
//! column at x takes the climate of phi(x), a warp of the sphere built from
//! octaves of uniformly decreasing size, shortest first, each read at the
//! place the ones before it led to:
//!
//!   q0 = x,  q(k+1) = q(k) + A(k) N(q(k) / L(k)),  phi(x) = q(n)
//!
//! - **It moves lines by metres, not degrees.** How far a line moves does not
//!   depend on how gentle the field is, so a gentle gradient cannot shred it.
//! - **It cannot make a piece.** A(k) / L(k) is small enough that every step
//!   is one-to-one (`steps_are_one_to_one`), and a chain of one-to-one maps is
//!   one-to-one however many there are, so phi only bends the sphere. The
//!   contours of F(phi(x)) are the contours of F, bent: exactly the pieces the
//!   bake has, however strong the warp. This is `coast.rs`'s promise (a slope
//!   under 1 can't make an island) without the limit on the sum of the
//!   octaves, so the top octaves can be strong enough to matter.
//! - **It is rough at every scale.** A(k) / L(k) is the same at every octave
//!   and each octave bends what the others made, so there is no gap in it: a
//!   line's roughness in a box of any size (`lines_are_rough_at_every_scale`)
//!   changes smoothly from kilometres to metres. Short octaves take two or
//!   three steps through their own field, which keeps them one-to-one and
//!   carries the roughness down to the last block.
//! - **It is the same line at every LOD.** A voxel leaves out the octaves it
//!   can't resolve (`band`). They come first in the chain, so what the voxel
//!   sees is the finer line less their own small move, however the long
//!   octaves stretch things after them. There is no noise value to alias into
//!   speckle: a coarse voxel gets a position, not a threshold test.
//! - **Global and local.** Octaves over `LOCAL_TOP_M` are positions on the
//!   sphere: they stretch with the planet, like the fields they bend.
//!   Octaves up to it are metres of ground, the same on any planet.
//!
//! All of it is a pure function of (seed, direction, voxel size), and it lives
//! in 3D with no chart, so it has no seam at a cube edge.

use super::noise::vector_noise;
use glam::DVec3;

/// Longest octave that belongs to the ground rather than the planet (m).
pub const LOCAL_TOP_M: f64 = 1024.0;
/// Each step's size as a fraction of its wavelength. `steps_are_one_to_one`
/// holds this to the promise above.
pub const STEP_AMP: f64 = 0.17;
/// Longest octave of the climate's warp, as a fraction of the radius (16 km
/// on Neisor, about a third of a continent's breadth there).
pub const CLIMATE_TOP_RADII: f64 = 0.177;
/// Steps taken through each octave's field: one above `MIDDLE_TOP_M`, then more
/// on the way down. One step of a short octave moves a line under a block, so
/// with one step each a line looks like smooth noise close up; with these its
/// roughness holds to the last block. (Long octaves take one: more shears
/// the climate into streaks.)
const MIDDLE_TOP_M: f64 = 128.0;
const MIDDLE_STEPS: u32 = 2;
const SHORT_TOP_M: f64 = 16.0;
const SHORT_STEPS: u32 = 3;
/// Shortest octave anywhere, in blocks: rough down to a block's edge.
const FINEST_BLOCKS: f64 = 2.0;
/// Longest octave of a lake outline, as a share of the lake field's reach.
const LAKE_TOP_SHARE: f64 = 0.6;
/// Longest octave of the bare-rock line (m): the slope it bends has nothing
/// shorter than about this in it (`Generator::rock_map`). Local: rock is ground.
const ROCK_TOP_M: f64 = 64.0;
/// Largest length of `vector_noise` (measured: `vector_noise_is_bounded`).
pub const NOISE_MAX: f64 = 1.8;

/// Octave fade for voxels of `voxel_m`: full at 3 voxels per wavelength, gone
/// at 1.5. Finer than the landform's rule, because an octave here moves a line
/// by a fraction of its wavelength rather than setting a value: at the fade
/// it moves the line by a fraction of a voxel, which can't alias into speckle.
#[inline]
fn band(wl: f64, voxel_m: f64) -> f64 {
    (wl / (1.5 * voxel_m) - 1.0).clamp(0.0, 1.0)
}

#[derive(Clone, Copy, Debug)]
struct Octave {
    wl_m: f64,
    /// Noise lattice cells per unit of direction (radius / wavelength).
    freq: f64,
    /// Displacement per unit of noise, in units of direction.
    step: f64,
    seed: u64,
    /// Lattice offset, so no two octaves share a lattice point at the origin.
    offset: f64,
    /// Steps taken through this octave's field.
    steps: u32,
}

/// A warp of the sphere, see the module header. Octaves finest first.
#[derive(Clone, Debug)]
pub struct Warp {
    octaves: Vec<Octave>,
}

impl Warp {
    /// A warp whose longest octave is `top_m` (m on this planet), of strength
    /// `amp` (a step's size per wavelength), for blocks of `block_m`.
    pub fn new(radius: f64, block_m: f64, top_m: f64, amp: f64, seed: u64) -> Self {
        let mut octaves = Vec::new();
        let mut push = |wl_m: f64, k: u64| {
            let steps = if wl_m <= SHORT_TOP_M {
                SHORT_STEPS
            } else if wl_m <= MIDDLE_TOP_M {
                MIDDLE_STEPS
            } else {
                1
            };
            octaves.push(Octave { wl_m, freq: radius / wl_m, step: amp * wl_m / radius, seed: seed.wrapping_add(k.wrapping_mul(0x9E37_79B9)), offset: k as f64 * 7.31, steps });
        };
        // Local: the same wavelengths on every planet.
        let mut wl = LOCAL_TOP_M * (FINEST_BLOCKS * block_m / LOCAL_TOP_M).log2().ceil().exp2();
        let mut k = 64u64 + (wl / FINEST_BLOCKS).log2().round() as u64;
        while wl <= LOCAL_TOP_M && wl <= top_m * 1.41 {
            push(wl, k);
            wl *= 2.0;
            k += 1;
        }
        // Global: halves of the top, while they stay clear of the local ones.
        let mut wl = top_m;
        let mut k = 0u64;
        let mut global = Vec::new();
        while wl > 1.41 * LOCAL_TOP_M {
            global.push((wl, k));
            wl *= 0.5;
            k += 1;
        }
        for (wl, k) in global.into_iter().rev() {
            push(wl, k);
        }
        Self { octaves }
    }

    /// The warp of the climate fields.
    pub fn climate(radius: f64, block_m: f64, seed: u64) -> Self {
        Self::new(radius, block_m, CLIMATE_TOP_RADII * radius, STEP_AMP, seed ^ 0xC11A)
    }

    /// The warp of the coast's distance field (`coast.rs`).
    pub fn coast(radius: f64, block_m: f64, seed: u64) -> Self {
        Self::new(radius, block_m, super::coast::WARP_TOP_RADII * radius, STEP_AMP, seed ^ 0xC0A5)
    }

    /// The warp of the lake field. `r_field` is the field's reach: a lake is
    /// at least about half that wide, so the longest octave bends a small
    /// lake's whole shape and can't carry it off its rivers.
    pub fn shore(radius: f64, block_m: f64, r_field: f64, seed: u64) -> Self {
        Self::new(radius, block_m, LAKE_TOP_SHARE * r_field, STEP_AMP, seed ^ 0x5A0E)
    }

    /// The warp of the bare-rock line.
    pub fn rock(radius: f64, block_m: f64, seed: u64) -> Self {
        Self::new(radius, block_m, ROCK_TOP_M, STEP_AMP, seed ^ 0x90C4)
    }

    /// Where the column at `dir` reads its fields, for voxels of `voxel_m`.
    #[inline]
    pub fn at(&self, dir: DVec3, voxel_m: f64) -> DVec3 {
        self.split_at(dir, voxel_m, 0.0).1
    }

    /// `at` as a pair: the position after only the octaves of wavelength up to
    /// `short_m`, and after all of them. The ground's own features read the
    /// first: the long octaves would carry them off the ground they belong to.
    #[inline]
    pub fn split_at(&self, dir: DVec3, voxel_m: f64, short_m: f64) -> (DVec3, DVec3) {
        let mut q = dir;
        let mut short = None;
        for o in &self.octaves {
            if short.is_none() && o.wl_m > short_m {
                short = Some(q);
            }
            let fade = band(o.wl_m, voxel_m);
            if fade > 0.0 {
                // Back on the sphere at every step: a step is one-to-one on the
                // sphere, and a radial part would pile wrinkles over each other.
                for _ in 0..o.steps {
                    q = (q + vector_noise(q * o.freq + DVec3::splat(o.offset), o.seed) * (o.step * fade)).normalize();
                }
            }
        }
        (short.unwrap_or(q), q)
    }

    /// Upper bound on how far `at` can move a column (m).
    pub fn reach_m(&self, radius: f64) -> f64 {
        self.octaves.iter().map(|o| o.steps as f64 * o.step * radius * NOISE_MAX).sum()
    }

    /// Wavelengths, finest first (m).
    #[cfg(test)]
    pub fn wavelengths(&self) -> Vec<f64> {
        self.octaves.iter().map(|o| o.wl_m).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 90_569.0;

    fn sphere_points(n: u32) -> impl Iterator<Item = DVec3> {
        (0..n).map(|k| {
            let t = k as f64 * 0.618_033_988_749_895;
            let (a, b) = (t.fract() * std::f64::consts::TAU, (t * 0.381_966).fract() * 2.0 - 1.0);
            let s = (1.0 - b * b).sqrt();
            DVec3::new(s * a.cos(), s * a.sin(), b)
        })
    }

    #[test]
    fn the_ladder_runs_from_the_top_to_a_block_and_stretches_only_above_the_crossover() {
        let a = Warp::climate(R, 1.0, 42).wavelengths();
        let b = Warp::climate(4.0 * R, 1.0, 42).wavelengths();
        assert_eq!(Warp::climate(R, 0.5, 42).wavelengths()[0], 1.0, "half-metre blocks go a block finer");
        println!("{a:?}\n{b:?}");
        assert!((a.last().unwrap() - 16_030.0).abs() < 1.0 && (b.last().unwrap() - 4.0 * 16_030.0).abs() < 4.0, "the top stretches with the radius");
        assert_eq!(a[0], 2.0);
        assert_eq!(b[0], 2.0);
        let local: Vec<&f64> = a.iter().filter(|w| **w <= LOCAL_TOP_M).collect();
        assert_eq!(local, b.iter().filter(|w| **w <= LOCAL_TOP_M).collect::<Vec<_>>(), "local octaves are the same metres on any planet");
        assert_eq!(b.len(), a.len() + 2, "a bigger planet has more global octaves, and no gap");
        for w in a.windows(2).chain(b.windows(2)) {
            assert!((1.4..=2.9).contains(&(w[1] / w[0])), "gap between octaves {} and {}", w[0], w[1]);
        }
    }

    /// The promise in the module header: every step is one-to-one on the
    /// sphere. A step is x + s N(x / w): its tangent Jacobian is I + J, and
    /// while |J| < 1 (largest singular value) it can't fold.
    #[test]
    fn steps_are_one_to_one() {
        let w = Warp::climate(R, 1.0, 42);
        let mut worst: f64 = 0.0;
        for o in &w.octaves {
            for d in sphere_points(40_000) {
                let e1 = d.any_orthonormal_vector();
                let e2 = d.cross(e1);
                let h = 1e-4 / o.freq;
                let at = |x: DVec3| vector_noise(x * o.freq + DVec3::splat(o.offset), o.seed) * o.step;
                let (g1, g2) = ((at(d + e1 * h) - at(d - e1 * h)) / (2.0 * h), (at(d + e2 * h) - at(d - e2 * h)) / (2.0 * h));
                // The 2 x 2 Jacobian on the tangent plane (the radial part only moves the lookup).
                let (a, b, c, e) = (g1.dot(e1), g2.dot(e1), g1.dot(e2), g2.dot(e2));
                // Largest singular value of [[a, b], [c, e]].
                let (s1, s2) = (a * a + b * b + c * c + e * e, ((a * a + b * b - c * c - e * e).powi(2) + 4.0 * (a * c + b * e).powi(2)).sqrt());
                worst = worst.max((0.5 * (s1 + s2)).sqrt());
            }
        }
        println!("steepest step: {worst:.3}");
        assert!(worst < 1.0, "a step's slope reached {worst:.3}: it can fold");
        // One octave much harder: the steepest spots are rare.
        use rayon::prelude::*;
        let o = w.octaves[3];
        let pts: Vec<DVec3> = sphere_points(4_000_000).collect();
        let hard = pts
            .par_iter()
            .map(|&d| {
                let e1 = d.any_orthonormal_vector();
                let e2 = d.cross(e1);
                let h = 1e-4 / o.freq;
                let at = |x: DVec3| vector_noise(x * o.freq + DVec3::splat(o.offset), o.seed) * o.step;
                let (g1, g2) = ((at(d + e1 * h) - at(d - e1 * h)) / (2.0 * h), (at(d + e2 * h) - at(d - e2 * h)) / (2.0 * h));
                let (a, b, c, e) = (g1.dot(e1), g2.dot(e1), g1.dot(e2), g2.dot(e2));
                let (s1, s2) = (a * a + b * b + c * c + e * e, ((a * a + b * b - c * c - e * e).powi(2) + 4.0 * (a * c + b * e).powi(2)).sqrt());
                (0.5 * (s1 + s2)).sqrt()
            })
            .reduce(|| 0.0, f64::max);
        println!("steepest of one octave over 4 million points: {hard:.3}");
        assert!(hard < 1.0, "a step's slope reached {hard:.3}: it can fold");
    }

    /// The whole warp stays one-to-one at every voxel size: a small square
    /// stays a positively oriented quadrilateral through the whole chain. The
    /// square has to be small against what the chain stretches it by.
    #[test]
    fn the_warp_never_turns_the_sphere_inside_out() {
        use rayon::prelude::*;
        let w = Warp::climate(R, 1.0, 42);
        for voxel in [1.0, 4.0, 32.0, 256.0] {
            let pts: Vec<DVec3> = sphere_points(400_000).collect();
            let at = |h: f64| {
                pts.par_iter()
                    .map(|&d| {
                        let e1 = d.any_orthonormal_vector();
                        let e2 = d.cross(e1);
                        let (a, b, c) = (w.at(d, voxel), w.at((d + e1 * h).normalize(), voxel), w.at((d + e2 * h).normalize(), voxel));
                        (b - a).cross(c - a).dot(a) / (h * h)
                    })
                    .reduce(|| f64::MAX, f64::min)
            };
            let (wide, tight, finest) = (at(0.05 * voxel / R), at(0.005 * voxel / R), at(0.0005 * voxel / R));
            println!("voxel {voxel}: smallest area scale over a square of {:.3} m: {wide:.3}; of {:.4} m: {tight:.3}; of {:.5} m: {finest:.3}", 0.05 * voxel, 0.005 * voxel, 0.0005 * voxel);
            assert!(finest > 0.0, "voxels of {voxel} m: the warp folds ({finest})");
        }
    }

    /// Reach: the bound holds, and the warp really moves things that far.
    #[test]
    fn reach_bounds_the_displacement() {
        let w = Warp::climate(R, 1.0, 42);
        let reach = w.reach_m(R);
        let mut worst: f64 = 0.0;
        for d in sphere_points(100_000) {
            worst = worst.max((w.at(d, 1.0) - d).length() * R);
        }
        println!("largest move {worst:.0} m, bound {reach:.0} m");
        assert!(worst < reach && worst > 0.2 * reach, "moved {worst}, bound {reach}");
    }

    /// `Generator::rock_map` reads a lattice wide enough for `ROCK_WARP_M`.
    #[test]
    fn the_rock_warp_stays_inside_the_lattice_it_reads() {
        use rayon::prelude::*;
        let w = Warp::rock(R, 1.0, 42);
        let pts: Vec<DVec3> = sphere_points(4_000_000).collect();
        let worst = pts.par_iter().map(|d| (w.at(*d, 1.0) - *d).length() * R).reduce(|| 0.0, f64::max);
        println!("largest move of 4 million columns {worst:.1} m (bound {:.1} m)", w.reach_m(R));
        assert!(worst < crate::worldgen::ROCK_WARP_M, "moved {worst}");
    }

    /// Band limit: the octaves a voxel leaves out come first in the chain, so a
    /// line at that LOD is the finer line less their own small move, however
    /// the long octaves stretch things after them.
    #[test]
    fn what_a_coarse_voxel_leaves_out_moves_a_line_under_a_voxel() {
        let w = Warp::climate(R, 1.0, 42);
        for voxel in [2.0, 8.0, 64.0, 512.0] {
            let mut worst: f64 = 0.0;
            for d in sphere_points(50_000) {
                worst = worst.max((w.split_at(d, 1.0, 3.0 * voxel).0 - d).length() * R);
            }
            println!("voxel {voxel}: octaves out of reach move a line {:.2} voxels at most", worst / voxel);
            assert!(worst < 2.0 * voxel, "voxels of {voxel} m: {worst} m");
        }
    }
}

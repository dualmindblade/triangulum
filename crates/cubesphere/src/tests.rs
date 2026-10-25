use super::*;
use std::hint::black_box;
use std::time::Instant;

const ALL: [Projection; 5] = [
    Projection::Gnomonic,
    Projection::Equiangular,
    Projection::EqualAreaCrease,
    Projection::Cobe,
    Projection::EqualArea,
];

struct Rng(u64);
impl Rng {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545f4914f6cdd1d) >> 11) as f64 / (1_u64 << 53) as f64
    }
    fn signed(&mut self) -> f64 {
        2.0 * self.uniform() - 1.0
    }
    fn near_edge(&mut self) -> f64 {
        // Between 1e-12 and 1e-9 from an edge, never an exact boundary tie.
        let distance = 10.0_f64.powf(-12.0 + 3.0 * self.uniform());
        (1.0 - distance).copysign(self.signed())
    }
}

#[test]
fn round_trips_100_000_per_projection() {
    for proj in ALL {
        let mut rng = Rng(0x6d315f67726964);
        let mut worst = 0.0_f64;
        for i in 0..100_000 {
            let (p, q) = match i % 8 {
                0 => (rng.near_edge(), rng.signed()),
                1 => (rng.signed(), rng.near_edge()),
                2 => (rng.near_edge(), rng.near_edge()),
                3 => {
                    let s = rng.signed();
                    (
                        s,
                        (s + 1e-10 * rng.signed()).clamp(-1.0 + 1e-12, 1.0 - 1e-12),
                    )
                }
                4 => (rng.signed() * 1e-10, rng.signed() * 1e-10),
                _ => (rng.signed(), rng.signed()),
            };
            let face = (i % 6) as u8;
            let dir = face_to_dir(proj, face, p, q);
            assert!((dir.length_squared() - 1.0).abs() < 8e-16);
            let (f, u, v) = dir_to_face(proj, dir);
            assert_eq!(f, face, "{proj:?} ({p},{q})");
            let error = (p - u).abs().max((q - v).abs());
            worst = worst.max(error);
            assert!(error < 1e-12, "{proj:?} ({p},{q}) -> ({u},{v}), {error:e}");
            // Also start from tangent coordinates, independently of warp.
            let (u, v) = unwarp(proj, p, q);
            let (a, b) = warp(proj, u, v);
            assert!((p - a).abs().max((q - b).abs()) < 1e-12);
        }
        println!("round trip {proj:?}: max coordinate error {worst:.3e}");
    }
}

#[test]
fn cobe_matches_python_reference() {
    let data = include_bytes!("../data/csc_reference.bin");
    let mut worst = 0.0_f64;
    for record in data.chunks_exact(32) {
        let v: Vec<_> = record
            .chunks_exact(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let (a, b) = warp(Projection::Cobe, v[0], v[1]);
        let error = (a - v[2]).abs().max((b - v[3]).abs());
        worst = worst.max(error);
        assert!(error < 8e-16);
    }
    println!(
        "CSC Python reference: {} points, max error {worst:.3e}",
        data.len() / 32
    );
}

#[test]
fn frames_edges_corners_and_symmetries() {
    // Independent expected integer axes guard the data convention.
    let expected = [
        ([1, 0, 0], [0, 1, 0], [0, 0, 1]),
        ([-1, 0, 0], [0, -1, 0], [0, 0, 1]),
        ([0, 1, 0], [-1, 0, 0], [0, 0, 1]),
        ([0, -1, 0], [1, 0, 0], [0, 0, 1]),
        ([0, 0, 1], [0, 1, 0], [-1, 0, 0]),
        ([0, 0, -1], [0, 1, 0], [1, 0, 0]),
    ];
    for ((axis, right, up), (a, r, u)) in FACES.into_iter().zip(expected) {
        assert_eq!(axis, DVec3::from_array(a.map(f64::from)));
        assert_eq!(right, DVec3::from_array(r.map(f64::from)));
        assert_eq!(up, DVec3::from_array(u.map(f64::from)));
    }
    for proj in ALL {
        let mut worst = 0.0_f64;
        for (face, (axis, right, up)) in FACES.into_iter().enumerate() {
            for edge in 0..4 {
                let normal = match edge {
                    0 => right,
                    1 => -right,
                    2 => up,
                    _ => -up,
                };
                let neighbour = FACES.iter().position(|&(a, _, _)| a == normal).unwrap();
                for k in 0..=1024 {
                    let t = -1.0 + 2.0 * k as f64 / 1024.0;
                    let (p, q) = match edge {
                        0 => (1.0, t),
                        1 => (-1.0, t),
                        2 => (t, 1.0),
                        _ => (t, -1.0),
                    };
                    let cube = axis + p * right + q * up;
                    let (_, nr, nu) = FACES[neighbour];
                    let a = face_to_dir(proj, face as u8, p, q);
                    let b = face_to_dir(proj, neighbour as u8, cube.dot(nr), cube.dot(nu));
                    worst = worst.max((a - b).abs().max_element());
                    assert_eq!(
                        a.to_array().map(f64::to_bits),
                        b.to_array().map(f64::to_bits),
                        "{proj:?}: shared edge {face}/{neighbour}, t={t}"
                    );
                    let canonical = FACES
                        .iter()
                        .position(|&(axis, _, _)| cube.dot(axis) == 1.0)
                        .unwrap();
                    let (f, u, v) = dir_to_face(proj, a);
                    assert_eq!(f as usize, canonical);
                    assert!((face_to_dir(proj, f, u, v) - a).length() < 1e-12);
                    let (u, v) = unwarp(proj, warp(proj, p, q).0, warp(proj, p, q).1);
                    assert!((u - p).abs().max((v - q).abs()) < 1e-12);
                }
            }
        }
        let mut rng = Rng(0xd4);
        for _ in 0..4096 {
            let (p, q) = (rng.uniform(), rng.uniform());
            let (a, b) = warp(proj, p, q);
            assert_eq!(warp(proj, q, p), (b, a));
            assert_eq!(warp(proj, -p, q), (-a, b));
            assert_eq!(warp(proj, p, -q), (a, -b));
        }
        println!("edges {proj:?}: max component error {worst:.3e} (bit-identical)");
    }
}

fn triangle(a: DVec3, b: DVec3, c: DVec3) -> f64 {
    // Algebraically the usual spherical excess determinant, evaluated in
    // difference form to avoid catastrophic cancellation for tiny cells.
    2.0 * a
        .dot((b - a).cross(c - a))
        .abs()
        .atan2(1.0 + a.dot(b) + b.dot(c) + c.dot(a))
}

fn area_metrics(proj: Projection, n: usize) -> (f64, f64, f64) {
    let mut previous: Vec<_> = (0..=n)
        .map(|i| face_to_dir(proj, 0, -1.0 + 2.0 * i as f64 / n as f64, -1.0))
        .collect();
    let mut row = vec![DVec3::ZERO; n + 1];
    let mut min = f64::INFINITY;
    let mut max = 0.0_f64;
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for j in 1..=n {
        for (i, v) in row.iter_mut().enumerate() {
            *v = face_to_dir(
                proj,
                0,
                -1.0 + 2.0 * i as f64 / n as f64,
                -1.0 + 2.0 * j as f64 / n as f64,
            );
        }
        for i in 0..n {
            let area = triangle(previous[i], previous[i + 1], row[i + 1])
                + triangle(previous[i], row[i + 1], row[i]);
            min = min.min(area);
            max = max.max(area);
            let corrected = area - compensation;
            let next = sum + corrected;
            compensation = (next - sum) - corrected;
            sum = next;
        }
        std::mem::swap(&mut row, &mut previous);
    }
    let mean = sum / (n * n) as f64;
    assert!((sum - 4.0 * FRAC_PI_6).abs() < 2e-13);
    (
        min / mean,
        max / mean,
        (max / mean - 1.0).max(1.0 - min / mean),
    )
}

#[test]
fn every_cell_area_at_1024() {
    for proj in ALL {
        let (min, max, error) = area_metrics(proj, 1024);
        println!(
            "N=1024 areas {proj:?}: min/mean={min:.12}, max/mean={max:.12}, spread={:.12}, max relative error={error:.3e}",
            max / min
        );
        if proj == Projection::EqualArea {
            assert!(error <= 1e-6);
        }
    }
}

#[test]
fn smooth_map_jacobian_and_knot_continuity() {
    let mut rng = Rng(0x534d4f4f5448);
    let mut worst = 0.0_f64;
    for i in 0..100_000 {
        let (p, q) = if i % 3 == 0 {
            (rng.near_edge(), rng.near_edge())
        } else {
            (rng.signed(), rng.signed())
        };
        let r = smooth_warp::<true>(Projection::EqualArea, p, q);
        let rho = (r[2] * r[5] - r[3] * r[4]) / (1.0 + r[0] * r[0] + r[1] * r[1]).powf(1.5);
        worst = worst.max((rho / FRAC_PI_6 - 1.0).abs());
        assert!((rho / FRAC_PI_6 - 1.0).abs() < 1e-8);
        if i < 1000 {
            let h = 1e-5;
            let p = p * 0.99;
            let q = q * 0.99;
            let r = smooth_warp::<true>(Projection::EqualArea, p, q);
            let f = |x| warp(Projection::EqualArea, x, q);
            let d = |part: usize| {
                let get = |t: (f64, f64)| if part == 0 { t.0 } else { t.1 };
                (get(f(p - 2.0 * h)) - 8.0 * get(f(p - h)) + 8.0 * get(f(p + h))
                    - get(f(p + 2.0 * h)))
                    / (12.0 * h)
            };
            assert!((d(0) - r[2]).abs() < 5e-10);
            assert!((d(1) - r[4]).abs() < 5e-10);
        }
    }
    // Derivatives approach one another across every spline knot, diagonal,
    // and mirrored axis; this would detect a hidden diagonal crease.
    for k in 0..=128 {
        let p = (k as f64 / 128.0).sqrt();
        for q in [0.0, 0.3, 0.8, p] {
            if p == 1.0 {
                continue;
            }
            let l = smooth_warp::<true>(Projection::EqualArea, p - 1e-10, q);
            let r = smooth_warp::<true>(Projection::EqualArea, p + 1e-10, q);
            for d in 2..6 {
                assert!((l[d] - r[d]).abs() < 2e-8);
            }
        }
    }
    println!("EqualArea sampled analytic Jacobian: max relative error {worst:.3e}");
}

fn width_jump(proj: Projection, n: usize) -> f64 {
    let mut previous = vec![0.0_f64; n];
    let mut worst = 0.0_f64;
    let jump = |a: f64, b: f64| a.max(b) / a.min(b) - 1.0;
    for j in 0..n {
        let q = -1.0 + 2.0 * (j as f64 + 0.5) / n as f64;
        let mut left = face_to_dir(proj, 0, -1.0, q);
        let mut last_width = 0.0;
        for (i, prev) in previous.iter_mut().enumerate() {
            let right = face_to_dir(proj, 0, -1.0 + 2.0 * (i + 1) as f64 / n as f64, q);
            let width = 2.0 * (0.5 * (right - left).length()).asin();
            if i > 0 {
                worst = worst.max(jump(width, last_width));
            }
            if j > 0 {
                worst = worst.max(jump(width, *prev));
            }
            *prev = width;
            last_width = width;
            left = right;
        }
    }
    // Transpose symmetry means the q-width comparison is identical.
    worst
}

#[test]
fn cell_width_smoothness() {
    for proj in [Projection::EqualArea, Projection::EqualAreaCrease] {
        let mut previous = f64::INFINITY;
        for n in [128, 256, 512, 1024] {
            let jump = width_jump(proj, n);
            println!(
                "width jump {proj:?}, N={n}: {jump:.9} ({:.6}%)",
                100.0 * jump
            );
            if proj == Projection::EqualArea {
                assert!(
                    jump < 0.56 * previous,
                    "smooth variation should shrink linearly with grid spacing"
                );
                if n == 1024 {
                    assert!(jump < 0.004);
                }
            } else if n == 1024 {
                assert!(jump > 0.04);
            }
            previous = jump;
        }
    }
}

#[test]
fn one_sided_diagonal_widths() {
    // A cell centered ON the crease averages both sides, splitting the
    // intrinsic ~12% discontinuity into two ~6% neighbour jumps. Measure
    // the limiting one-sided widths separately so the kink is unambiguous.
    for proj in [Projection::EqualArea, Projection::EqualAreaCrease] {
        let mut worst = 0.0_f64;
        for s in [0.3, 0.6, 0.9, 1.0 - 1e-6] {
            let eps = 1e-7;
            let h = 1e-9;
            let width = |p, q| {
                (face_to_dir(proj, 0, p + h, q) - face_to_dir(proj, 0, p - h, q)).length()
                    / (2.0 * h)
            };
            let a = width(s, s - eps);
            let b = width(s - eps, s);
            worst = worst.max(a.max(b) / a.min(b) - 1.0);
        }
        println!(
            "one-sided diagonal width jump {proj:?}: {:.6}%",
            100.0 * worst
        );
        if proj == Projection::EqualArea {
            assert!(worst < 1e-5);
        } else {
            assert!(worst > 0.11 && worst < 0.13);
        }
    }
}

#[test]
fn curved_cell_areas_at_arbitrary_resolutions() {
    // Gaussian quadrature of the actual curved footprint. This is different
    // from replacing a coarse cell's four curved sides by great-circle arcs.
    const NODES: [f64; 4] = [
        -0.8611363115940526,
        -0.3399810435848563,
        0.3399810435848563,
        0.8611363115940526,
    ];
    const WEIGHTS: [f64; 4] = [
        0.3478548451374538,
        0.6521451548625461,
        0.6521451548625461,
        0.3478548451374538,
    ];
    let mut rng = Rng(0x41524541);
    for n in [1, 2, 3, 7, 16, 31, 64, 127, 1024, 131_072] {
        let mut worst = 0.0_f64;
        let cells: Vec<_> = if n <= 31 {
            (0..n).flat_map(|i| (0..n).map(move |j| (i, j))).collect()
        } else {
            let mut cells = vec![
                (0, 0),
                (n - 1, n - 1),
                (0, n - 1),
                (n / 2, n / 2),
                (0, n / 2),
            ];
            cells.extend((0..100).map(|_| {
                (
                    (rng.uniform() * n as f64) as usize,
                    (rng.uniform() * n as f64) as usize,
                )
            }));
            cells
        };
        for (i, j) in cells {
            let p = -1.0 + (2 * i + 1) as f64 / n as f64;
            let q = -1.0 + (2 * j + 1) as f64 / n as f64;
            let mut mean_density = 0.0;
            for k in 0..4 {
                for l in 0..4 {
                    let r = smooth_warp::<true>(
                        Projection::EqualArea,
                        p + NODES[k] / n as f64,
                        q + NODES[l] / n as f64,
                    );
                    let rho =
                        (r[2] * r[5] - r[3] * r[4]) / (1.0 + r[0] * r[0] + r[1] * r[1]).powf(1.5);
                    mean_density += WEIGHTS[k] * WEIGHTS[l] * rho / 4.0;
                }
            }
            worst = worst.max((mean_density / FRAC_PI_6 - 1.0).abs());
        }
        println!("curved cell quadrature N={n}: max relative error {worst:.3e}");
        assert!(worst < 1e-8);
    }
}

#[test]
fn shape_metrics() {
    // Midpoint quadrature of the continuous Jacobian, weighted by spherical
    // area. Finite differences stay on one side of crease diagonals.
    let n = 400;
    for proj in ALL {
        let mut total = 0.0;
        let mut off_square = 0.0;
        let mut worst_skew = 0.0_f64;
        let mut worst_stretch = 0.0_f64;
        for j in 0..n {
            for i in 0..n {
                let p = -1.0 + 2.0 * (i as f64 + 0.5) / n as f64;
                let mut q = -1.0 + 2.0 * (j as f64 + 0.5) / n as f64;
                // Choose a one-sided representative for measure-zero creases.
                if p.abs() == q.abs() {
                    q += 1e-6;
                }
                let h = 1e-7;
                let dp =
                    (face_to_dir(proj, 0, p + h, q) - face_to_dir(proj, 0, p - h, q)) / (2.0 * h);
                let dq =
                    (face_to_dir(proj, 0, p, q + h) - face_to_dir(proj, 0, p, q - h)) / (2.0 * h);
                let area = dp.cross(dq).length();
                let skew = (dp.dot(dq) / (dp.length() * dq.length()))
                    .clamp(-1.0, 1.0)
                    .asin()
                    .abs()
                    .to_degrees();
                let tr = dp.length_squared() + dq.length_squared();
                let disc = ((dp.length_squared() - dq.length_squared()).powi(2)
                    + 4.0 * dp.dot(dq).powi(2))
                .sqrt();
                let stretch = ((tr + disc) / (tr - disc)).sqrt();
                total += area;
                if skew > 10.0 {
                    off_square += area;
                }
                worst_skew = worst_skew.max(skew);
                worst_stretch = worst_stretch.max(stretch);
            }
        }
        println!(
            "shape {proj:?}: surface >10deg={:.6}%, sampled worst skew={worst_skew:.6}deg, max singular ratio={worst_stretch:.6}",
            100.0 * off_square / total
        );
    }
}

#[test]
fn call_timings() {
    // Run this test alone with --release -- --nocapture --test-threads=1
    // for useful CPU timing; timings in a parallel test run are provisional.
    let mut rng = Rng(0x5350454544);
    let samples: Vec<_> = (0..8192)
        .map(|i| ((i % 6) as u8, rng.signed(), rng.signed()))
        .collect();
    const REPEAT: usize = 8;
    for proj in ALL {
        let dirs: Vec<_> = samples
            .iter()
            .map(|&(f, p, q)| face_to_dir(proj, f, p, q))
            .collect();
        let start = Instant::now();
        for _ in 0..REPEAT {
            for &(f, p, q) in &samples {
                black_box(face_to_dir(
                    black_box(proj),
                    black_box(f),
                    black_box(p),
                    black_box(q),
                ));
            }
        }
        let forward = start.elapsed().as_nanos() as f64 / (samples.len() * REPEAT) as f64;
        let start = Instant::now();
        for _ in 0..REPEAT {
            for &dir in &dirs {
                black_box(dir_to_face(black_box(proj), black_box(dir)));
            }
        }
        let inverse = start.elapsed().as_nanos() as f64 / (samples.len() * REPEAT) as f64;
        println!(
            "speed {proj:?}: face_to_dir {forward:.1} ns, dir_to_face {inverse:.1} ns (provisional wall-clock)"
        );
    }
}

#[test]
fn scale_invariant_directions_and_tiny_coordinates() {
    for proj in ALL {
        for scale in [1e-300, 1.0, 1e300] {
            let (f, p, q) = dir_to_face(proj, DVec3::new(0.3, -0.5, 0.7) * scale);
            let d = face_to_dir(proj, f, p, q);
            assert!((d - DVec3::new(0.3, -0.5, 0.7).normalize()).length() < 1e-12);
        }
        for p in [0.0, 1e-200, 1e-100, 1e-20, 1e-10] {
            let (a, b) = warp(proj, p, -p * 0.5);
            let (u, v) = unwarp(proj, a, b);
            assert!(a.is_finite() && b.is_finite());
            // Absolute inverse tolerance also applies close to the origin.
            assert!((u - p).abs().max((v + p * 0.5).abs()) < 1e-12);
        }
    }
}

#[test]
#[should_panic(expected = "finite and nonzero")]
fn zero_direction_is_rejected() {
    dir_to_face(Projection::EqualArea, DVec3::ZERO);
}

#[test]
#[should_panic(expected = "finite and nonzero")]
fn nonfinite_direction_is_rejected() {
    dir_to_face(Projection::EqualArea, DVec3::splat(f64::NAN));
}

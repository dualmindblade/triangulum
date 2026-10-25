//! Pure, allocation-free maps between cube-face grids and unit directions.
//!
//! Face parameters and tangent coordinates are in `[-1, 1]`. Column `(i,j)`
//! on an `N` by `N` face has `p = -1 + 2*(i+0.5)/N` and similarly for `q`.
//! See the crate README for the numerical construction, measured errors and
//! the distinction between curved cell boundaries and great-circle chords.

use glam::DVec3;
use std::f64::consts::{FRAC_PI_4, FRAC_PI_6};

mod table;

/// `(axis, right, up)`, exactly matching `viewer/src/planet.rs`.
pub const FACES: [(DVec3, DVec3, DVec3); 6] = [
    (
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    ),
    (
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, -1.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    ),
    (
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    ),
    (
        DVec3::new(0.0, -1.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
        DVec3::new(0.0, 0.0, 1.0),
    ),
    (
        DVec3::new(0.0, 0.0, 1.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(-1.0, 0.0, 0.0),
    ),
    (
        DVec3::new(0.0, 0.0, -1.0),
        DVec3::new(0.0, 1.0, 0.0),
        DVec3::new(1.0, 0.0, 0.0),
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Radial cube projection; grid lines are great circles, areas vary widely.
    Gnomonic,
    /// Uniform angular steps along both tangent axes.
    Equiangular,
    /// Closed-form equal area; its first derivatives jump on face diagonals.
    EqualAreaCrease,
    /// NASA COBE CSC's 28-coefficient approximate equal-area polynomial.
    Cobe,
    /// Smooth Moser equal-area map, represented by an embedded quintic spline.
    EqualArea,
}

// Rows are powers of y; columns are powers of x, with x=p*p and y=q*q.
// Coefficients are from the supplied, wcslib-checked reference/csc_check.py.
const CSC: [[f64; 7]; 7] = [
    [
        -0.27292696,
        -0.07629969,
        -0.22797056,
        0.54852384,
        -0.62930065,
        0.25795794,
        0.02584375,
    ],
    [
        -0.02819452,
        -0.01471565,
        0.48051509,
        -1.74114454,
        1.71547508,
        -0.53022337,
        0.0,
    ],
    [
        0.27058160,
        -0.56800938,
        0.30803317,
        0.98938102,
        -0.83180469,
        0.0,
        0.0,
    ],
    [
        -0.60441560,
        1.50880086,
        -0.93678576,
        0.08693841,
        0.0,
        0.0,
        0.0,
    ],
    [0.93412077, -1.41601920, 0.33887446, 0.0, 0.0, 0.0, 0.0],
    [-0.63915306, 0.52032238, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.14381585, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
];

#[inline]
fn cobe_residual<const DERIV: bool>(x: f64, y: f64) -> [f64; 3] {
    let mut g = [0.0; 3];
    for j in (0..7).rev() {
        let mut row = 0.0;
        let mut dx = 0.0;
        for i in (0..7 - j).rev() {
            if DERIV {
                dx = dx * x + row;
            }
            row = row * x + CSC[j][i];
        }
        if DERIV {
            g[2] = g[2] * y + g[0];
            g[1] = g[1] * y + dx;
        }
        g[0] = g[0] * y + row;
    }
    g
}

// Returns (a,b, da/dp, da/dq, db/dp, db/dq). DERIV=false removes derivative work.
#[inline]
fn smooth_warp<const DERIV: bool>(proj: Projection, p: f64, q: f64) -> [f64; 6] {
    let x = p * p;
    let y = q * q;
    let (g, h) = if proj == Projection::Cobe {
        (cobe_residual::<DERIV>(x, y), cobe_residual::<DERIV>(y, x))
    } else {
        table::residual::<DERIV>(x, y)
    };
    let mut r = [
        p + p * (1.0 - x) * g[0],
        q + q * (1.0 - y) * h[0],
        0.0,
        0.0,
        0.0,
        0.0,
    ];
    if DERIV {
        r[2] = 1.0 + (1.0 - 3.0 * x) * g[0] + 2.0 * x * (1.0 - x) * g[1];
        r[3] = 2.0 * p * q * (1.0 - x) * g[2];
        r[4] = 2.0 * p * q * (1.0 - y) * h[2];
        r[5] = 1.0 + (1.0 - 3.0 * y) * h[0] + 2.0 * y * (1.0 - y) * h[1];
    }
    r
}

#[inline]
fn crease(p: f64, q: f64, inverse: bool) -> (f64, f64) {
    let swap = q.abs() > p.abs();
    let (u, v) = if swap { (q, p) } else { (p, q) };
    let s = u.abs();
    if s == 0.0 {
        return (p, q);
    }
    let t = v.abs() / s;
    let (a, b) = if inverse {
        let r = (1.0 + 2.0 * s * s).sqrt();
        let z = s * s / r;
        let atan_over_z = if z == 0.0 { 1.0 } else { z.atan() / z };
        let a = if s == 1.0 {
            1.0
        } else {
            s * (atan_over_z / (FRAC_PI_6 * r)).sqrt()
        };
        let b = if t == 1.0 {
            a
        } else {
            t * a * ((1.0 + 2.0 * s * s) / (1.0 + s * s + v * v)).sqrt()
        };
        (a, b)
    } else {
        let z = FRAC_PI_6 * s * s;
        let tz = z.tan();
        let tan_over_z = if z == 0.0 { 1.0 } else { tz / z };
        let a = if s == 1.0 {
            1.0
        } else {
            s * (FRAC_PI_6 * tan_over_z * (tz + (1.0 + tz * tz).sqrt())).sqrt()
        };
        let b = if t == 1.0 {
            a
        } else {
            t * a * ((1.0 + a * a) / (1.0 + a * a * (2.0 - t * t))).sqrt()
        };
        (a, b)
    };
    let (a, b) = (a.copysign(u), b.copysign(v));
    if swap { (b, a) } else { (a, b) }
}

/// Face parameters to gnomonic tangent coordinates, all in `[-1,1]`.
///
/// The input must be finite and within that interval. No allocations,
/// initialization, mutable global state, or runtime data files are used.
#[inline]
pub fn warp(proj: Projection, p: f64, q: f64) -> (f64, f64) {
    match proj {
        Projection::Gnomonic => (p, q),
        Projection::Equiangular => {
            let angle = |x: f64| {
                if x.abs() == 1.0 {
                    x
                } else {
                    (FRAC_PI_4 * x).tan()
                }
            };
            (angle(p), angle(q))
        }
        Projection::EqualAreaCrease => crease(p, q, false),
        Projection::Cobe | Projection::EqualArea => {
            let r = smooth_warp::<false>(proj, p, q);
            (r[0], r[1])
        }
    }
}

/// Inverse of [`warp`], to floating-point accuracy (not a fitted inverse).
///
/// Input coordinates must be finite and in `[-1,1]`. COBE and the smooth
/// equal-area map use Newton iterations with their analytic Jacobians.
#[inline]
pub fn unwarp(proj: Projection, a: f64, b: f64) -> (f64, f64) {
    match proj {
        Projection::Gnomonic => (a, b),
        Projection::Equiangular => (a.atan() / FRAC_PI_4, b.atan() / FRAC_PI_4),
        Projection::EqualAreaCrease => crease(a, b, true),
        Projection::Cobe | Projection::EqualArea => {
            let mut p = a.atan() / FRAC_PI_4;
            let mut q = b.atan() / FRAC_PI_4;
            for _ in 0..12 {
                let r = smooth_warp::<true>(proj, p, q);
                let da = r[0] - a;
                let db = r[1] - b;
                if da.abs().max(db.abs()) <= 2.0e-16 {
                    break;
                }
                let det = r[2] * r[5] - r[3] * r[4];
                let dp = (r[5] * da - r[3] * db) / det;
                let dq = (r[2] * db - r[4] * da) / det;
                p = (p - dp).clamp(-1.0, 1.0);
                q = (q - dq).clamp(-1.0, 1.0);
                if dp.abs().max(dq.abs()) <= 2.0e-16 {
                    break;
                }
            }
            (p, q)
        }
    }
}

/// Convert face `0..6` and parameters `[-1,1]` to a unit direction.
/// Panics if the face index is invalid.
#[inline]
pub fn face_to_dir(proj: Projection, face: u8, p: f64, q: f64) -> DVec3 {
    let (axis, right, up) = FACES[face as usize];
    let (a, b) = warp(proj, p, q);
    // Symmetric summation makes an edge's normalization bit-identical on
    // either incident face, even when the frame exchanges a and b.
    (axis + a * right + b * up) * (1.0 + (a * a + b * b)).sqrt().recip()
}

/// Return a face and its parameters for any finite, nonzero direction.
///
/// Largest absolute direction component wins. Exact ties choose X before
/// Y before Z, equivalently the lowest eligible index in [`FACES`]. Thus a
/// point on a shared boundary has one canonical face; an input face cannot
/// in general be recovered on an exact edge or corner. Interior points do
/// recover their original face. The direction need not be normalized.
///
/// Panics for zero or non-finite directions.
#[inline]
pub fn dir_to_face(proj: Projection, dir: DVec3) -> (u8, f64, f64) {
    assert!(
        dir.is_finite() && dir.abs().max_element() > 0.0,
        "direction must be finite and nonzero"
    );
    let d = dir.abs();
    let face = if d.x >= d.y && d.x >= d.z {
        if dir.x >= 0.0 { 0 } else { 1 }
    } else if d.y >= d.z {
        if dir.y >= 0.0 { 2 } else { 3 }
    } else if dir.z >= 0.0 {
        4
    } else {
        5
    };
    let (axis, right, up) = FACES[face as usize];
    let scale = dir.dot(axis);
    let (p, q) = unwarp(proj, dir.dot(right) / scale, dir.dot(up) / scale);
    (face, p, q)
}

#[cfg(test)]
mod tests;

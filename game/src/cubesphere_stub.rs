//! Stand-in for the `crates/cubesphere` library (Astra's, in progress).
//!
//! It exposes exactly that crate's API so every caller in the engine is
//! already written against it; when the real crate lands this file is
//! replaced by `use cubesphere::*` and the projection switches to
//! `EqualArea`. Only `Equiangular` (and the trivial `Gnomonic`) are
//! implemented here; the other projections panic.
//!
//! Conventions (shared with the real crate):
//! - A face's lattice square is `(p, q)` in `[-1, 1]^2`, uniform in the
//!   lattice: column `i` of `N` spans `p` in `[2i/N - 1, 2(i+1)/N - 1]`.
//! - `warp` maps the lattice square to the gnomonic square `(a, b)`, also
//!   `[-1, 1]^2`, where direction = normalize(axis + a*right + b*up) using
//!   the frames in `FACES` (identical to `viewer/src/planet.rs`).
//! - `warp` must accept arguments slightly outside `[-1, 1]` (the engine
//!   samples one voxel past a face edge when meshing chunk borders).

use glam::DVec3;
use std::f64::consts::FRAC_PI_4;

/// Face frames (axis, right, up), copied verbatim from `viewer/src/planet.rs`.
pub const FACES: [(DVec3, DVec3, DVec3); 6] = [
    (DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(-1.0, 0.0, 0.0), DVec3::new(0.0, -1.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, -1.0, 0.0), DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, 0.0, 1.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0)),
    (DVec3::new(0.0, 0.0, -1.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(1.0, 0.0, 0.0)),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Gnomonic,
    Equiangular,
    EqualAreaCrease,
    Cobe,
    EqualArea,
}

/// Lattice square (p, q) -> gnomonic square (a, b).
#[inline]
pub fn warp(proj: Projection, p: f64, q: f64) -> (f64, f64) {
    match proj {
        Projection::Gnomonic => (p, q),
        Projection::Equiangular => ((p * FRAC_PI_4).tan(), (q * FRAC_PI_4).tan()),
        other => unimplemented!("projection {other:?} is provided by crates/cubesphere"),
    }
}

/// Gnomonic square (a, b) -> lattice square (p, q).
#[inline]
pub fn unwarp(proj: Projection, a: f64, b: f64) -> (f64, f64) {
    match proj {
        Projection::Gnomonic => (a, b),
        Projection::Equiangular => (a.atan() / FRAC_PI_4, b.atan() / FRAC_PI_4),
        other => unimplemented!("projection {other:?} is provided by crates/cubesphere"),
    }
}

/// Unit direction of lattice point (p, q) on `face`.
#[inline]
pub fn face_to_dir(proj: Projection, face: u8, p: f64, q: f64) -> DVec3 {
    let (a, b) = warp(proj, p, q);
    let (axis, right, up) = FACES[face as usize];
    (axis + a * right + b * up).normalize()
}

/// Which face a direction hits, and the lattice point (p, q) there.
#[inline]
pub fn dir_to_face(proj: Projection, dir: DVec3) -> (u8, f64, f64) {
    let mut best = (0usize, f64::MIN);
    for (i, (axis, _, _)) in FACES.iter().enumerate() {
        let d = dir.dot(*axis);
        if d > best.1 {
            best = (i, d);
        }
    }
    let (axis, right, up) = FACES[best.0];
    let g = dir / dir.dot(axis);
    let (p, q) = unwarp(proj, g.dot(right), g.dot(up));
    (best.0 as u8, p, q)
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Projection = Projection::Equiangular;

    #[test]
    fn warp_unwarp_round_trip() {
        for i in 0..=40 {
            for j in 0..=40 {
                let (p, q) = (i as f64 / 20.0 - 1.0, j as f64 / 20.0 - 1.0);
                let (a, b) = warp(P, p, q);
                assert!(a.abs() <= 1.0 + 1e-12 && b.abs() <= 1.0 + 1e-12);
                let (p2, q2) = unwarp(P, a, b);
                assert!((p - p2).abs() < 1e-12 && (q - q2).abs() < 1e-12, "{p} {q} -> {p2} {q2}");
            }
        }
    }

    #[test]
    fn face_round_trip() {
        for face in 0..6u8 {
            for i in 0..=20 {
                for j in 0..=20 {
                    // stay strictly inside the face so the face choice is unambiguous
                    let (p, q) = (i as f64 / 10.0 - 1.0, j as f64 / 10.0 - 1.0);
                    let (p, q) = (p * 0.999, q * 0.999);
                    let d = face_to_dir(P, face, p, q);
                    assert!((d.length() - 1.0).abs() < 1e-12);
                    let (f2, p2, q2) = dir_to_face(P, d);
                    assert_eq!(face, f2, "face {face} at {p} {q}");
                    assert!((p - p2).abs() < 1e-9 && (q - q2).abs() < 1e-9, "{p} {q} -> {p2} {q2}");
                }
            }
        }
    }

    #[test]
    fn edges_are_shared_and_angles_uniform() {
        // A point on the p = 1 edge of face 0 is the same direction as some
        // point on an edge of the neighbouring face, and equiangular columns
        // subtend equal angles along the edge.
        let d = face_to_dir(P, 0, 1.0, 0.3);
        let (f, p, q) = dir_to_face(Projection::Gnomonic, d + DVec3::new(0.0, 1e-9, 0.0));
        assert_ne!(f, 0);
        assert!((p.abs() - 1.0).abs() < 1e-6 || (q.abs() - 1.0).abs() < 1e-6, "{p} {q}");
        let a0 = face_to_dir(P, 0, 0.0, 0.0);
        let a1 = face_to_dir(P, 0, 0.1, 0.0);
        let a2 = face_to_dir(P, 0, 0.9, 0.0);
        let a3 = face_to_dir(P, 0, 1.0, 0.0);
        let ang = |x: DVec3, y: DVec3| x.dot(y).clamp(-1.0, 1.0).acos();
        assert!((ang(a0, a1) - ang(a2, a3)).abs() < 1e-12);
    }

    #[test]
    fn one_column_past_the_edge_lands_in_the_neighbour_edge_column() {
        // Equiangular: stepping half a column beyond p = 1 lands exactly half
        // a column inside the neighbouring face across the edge (the angle
        // past the edge equals the angle inside it). Along the edge the
        // lattice kinks, so q only lands within the same column, not at its
        // centre; `Planet::dir` snaps it.
        let n = 4096.0;
        let step = 2.0 / n;
        let beyond = face_to_dir(P, 0, 1.0 + 0.5 * step, 0.25);
        let (f, p, q) = dir_to_face(P, beyond);
        assert_ne!(f, 0);
        assert!((p.abs() - (1.0 - 0.5 * step)).abs() < 1e-12, "p = {p}");
        let q_col = (q + 1.0) / step;
        assert!((q_col - q_col.round()).abs() < 0.5 && (q_col - ((0.25 + 1.0) / step)).abs() < 0.5, "q column {q_col}");
    }
}

//! Traverse the world ray, splitting at cube-face planes. Within each face,
//! locate integer lattice crossings by solving the *exact inverse* along
//! the ray. Neither the ray nor a hit is extended in another face's lattice.
use super::{Hit, Planet};
use crate::world::Voxels;
use glam::{DVec3, IVec3};

const COORD_EPS: f64 = 1e-8;

struct Ray<'a> {
    vox: &'a dyn Voxels,
    planet: &'a Planet,
    eye: DVec3,
    dir: DVec3,
    liquids: bool,
    initial: (u8, [i64; 3]),
    previous: (u8, [i64; 3]),
}

impl Ray<'_> {
    // At a shared edge the usual inverse chooses one canonical face. Both
    // endpoints of a span must instead be expressed on THAT span's face.
    fn lattice(&self, face: u8, t: f64) -> DVec3 {
        let pos = self.eye + self.dir * t;
        let (axis, right, up) = cubesphere::FACES[face as usize];
        let scale = pos.dot(axis);
        let (p, q) = cubesphere::unwarp(self.planet.proj,
            (pos.dot(right) / scale).clamp(-1.0, 1.0),
            (pos.dot(up) / scale).clamp(-1.0, 1.0));
        DVec3::new(self.planet.i_of(p), self.planet.i_of(q),
            (pos.length() - self.planet.bottom_radius()) / self.planet.block_size)
    }

    fn visit(&mut self, face: u8, point: DVec3, t: f64) -> Option<Hit> {
        let n = self.planet.n as i64;
        let cell = [(point.x.floor() as i64).clamp(0, n - 1),
            (point.y.floor() as i64).clamp(0, n - 1), point.z.floor() as i64];
        let (old_face, old) = self.previous;
        self.previous = (face, cell);
        let id = self.vox.block(face, cell[0], cell[1], cell[2]);
        if !(self.vox.is_solid(id) || (self.liquids && (face, cell) != self.initial && self.vox.is_liquid(id)))
            || cell[2] < 0 || cell[2] >= self.planet.height as i64 { return None; }
        let mut normal = IVec3::ZERO;
        if old_face != face {
            // The entered side belongs to the HIT's frame, including when
            // the eye is two cube faces away. Wrapping that neighbour is
            // index adjacency, not an extrapolation of the world ray.
            let edge = self.planet.edges[face as usize].iter().position(|e| e.face == old_face).expect("adjacent ray faces");
            normal[edge / 2] = if edge % 2 == 0 { -1 } else { 1 };
        } else if let Some(axis) = (0..3).find(|&a| cell[a] != old[a]) {
            // An exact voxel edge/corner has several possible faces. Pick
            // the first axis deterministically; each names a real neighbour.
            normal[axis] = (old[axis] - cell[axis]).signum() as i32;
        }
        Some(Hit { block: crate::grid::BlockPos { face, i: cell[0] as u32, j: cell[1] as u32, k: cell[2] as u32 },
            id, face, cell: (cell[0], cell[1], cell[2]), normal, t })
    }

    fn crossing(&self, face: u8, axis: usize, level: f64, mut a: f64, mut b: f64, mut va: f64, mut vb: f64) -> f64 {
        // Bracketed secant: the affine chord is only a root *guess*. Every
        // correction evaluates the world ray, including radial curvature.
        for _ in 0..32 {
            let t = a + (b - a) * ((level - va) / (vb - va)).clamp(0.01, 0.99);
            let v = self.lattice(face, t)[axis];
            if (v - level).abs() < 1e-10 || b - a < 1e-9 { return t; }
            if (v < level) == (va < level) { a = t; va = v; } else { b = t; vb = v; }
        }
        (a + b) * 0.5
    }

    fn span(&mut self, face: u8, a: f64, b: f64, pa: DVec3, pb: DVec3, depth: u32) -> Option<Hit> {
        let mid = (a + b) * 0.5;
        let pm = self.lattice(face, mid);
        // A near-tangent coordinate can turn and enter/leave a cell with
        // identical endpoint cells. Split such spans before seeking roots.
        // Also limit angular span for small test planets / long reaches.
        let bend = (pm - (pa + pb) * 0.5).abs() * 8.0 + DVec3::splat(COORD_EPS);
        let turning = (0..3).any(|axis| {
            let lo = pa[axis].min(pb[axis]).min(pm[axis]) - bend[axis];
            let hi = pa[axis].max(pb[axis]).max(pm[axis]) + bend[axis];
            // A coordinate exactly on a face side or a radial ray on a
            // lattice line has no crossing along that coordinate.
            let constant = (pa[axis] - pb[axis]).abs() < COORD_EPS && (pa[axis] - pm[axis]).abs() < COORD_EPS;
            !constant && lo.floor() != hi.floor() && (pb[axis] - pa[axis]).abs() <= 4.0 * bend[axis]
        });
        let r = (self.eye + self.dir * mid).length();
        if depth < 40 && b - a > self.planet.block_size * 1e-7 && (turning || b - a > r * 0.002) {
            return self.span(face, a, mid, pa, pm, depth + 1)
                .or_else(|| self.span(face, mid, b, pm, pb, depth + 1));
        }
        let mut crossings = Vec::with_capacity(16);
        crossings.push(a);
        for axis in 0..3 {
            let lo = pa[axis].min(pb[axis]);
            let hi = pa[axis].max(pb[axis]);
            for level in (lo.floor() as i64 + 1)..=(hi.ceil() as i64 - 1) {
                crossings.push(self.crossing(face, axis, level as f64, a, b, pa[axis], pb[axis]));
            }
        }
        crossings.push(b);
        crossings.sort_unstable_by(f64::total_cmp);
        for ts in crossings.windows(2) {
            if ts[1] - ts[0] < 1e-9 { continue; }
            let point = if crossings.len() == 2 { pm } else { self.lattice(face, (ts[0] + ts[1]) * 0.5) };
            if let Some(hit) = self.visit(face, point, ts[0]) { return Some(hit); }
        }
        None
    }
}

/// First solid (or bucket liquid) on the true world ray, for the entire reach.
pub fn pick(vox: &dyn Voxels, planet: &Planet, eye: DVec3, dir: DVec3, reach_m: f64, liquids: bool) -> Option<Hit> {
    if !eye.is_finite() || !dir.is_finite() || eye.length_squared() == 0.0 || dir.length_squared() == 0.0 || !reach_m.is_finite() || reach_m < 0.0 { return None; }
    let dir = dir.normalize();
    let (f, i, j, k) = planet.lattice_of_pos(eye);
    let n = planet.n as i64;
    let initial = (f, [(i.floor() as i64).clamp(0, n - 1), (j.floor() as i64).clamp(0, n - 1), k.floor() as i64]);
    let mut ray = Ray { vox, planet, eye, dir, liquids, initial, previous: initial };
    if let Some(hit) = ray.visit(f, DVec3::new(i, j, k), 0.0) { return Some(hit); }
    // A face is selected by the largest absolute world coordinate. Its
    // boundaries are x=+/-y, x=+/-z, y=+/-z: intersect those planes exactly.
    let mut cuts = vec![0.0, reach_m];
    for a in 0..3 {
        for b in a + 1..3 {
            for sign in [-1.0, 1.0] {
                let denom = dir[a] - sign * dir[b];
                if denom != 0.0 {
                    let t = -(eye[a] - sign * eye[b]) / denom;
                    if t > 0.0 && t < reach_m { cuts.push(t); }
                }
            }
        }
    }
    // Radius reverses at closest approach, even if both ends are in one
    // layer (a grazing ray can briefly dip into the layer beneath it).
    let closest = -eye.dot(dir);
    if closest > 0.0 && closest < reach_m { cuts.push(closest); }
    cuts.sort_unstable_by(f64::total_cmp);
    for ts in cuts.windows(2) {
        if ts[1] - ts[0] < 1e-9 { continue; }
        let (face, _, _) = planet.lattice_of_dir(eye + dir * ((ts[0] + ts[1]) * 0.5));
        let pa = ray.lattice(face, ts[0]);
        let pb = ray.lattice(face, ts[1]);
        if let Some(hit) = ray.span(face, ts[0], ts[1], pa, pb, 0) { return Some(hit); }
    }
    // Reach is inclusive at a cell face. Classify its forward limit (ten
    // nanometres for metre blocks) so an exactly-on-boundary hit is not
    // discarded merely because its open interval begins at reach_m.
    if reach_m > 0.0 {
        let point = eye + dir * (reach_m + planet.block_size * 1e-8);
        let (face, i, j, k) = planet.lattice_of_pos(point);
        if let Some(hit) = ray.visit(face, DVec3::new(i, j, k), reach_m) { return Some(hit); }
    }
    None
}

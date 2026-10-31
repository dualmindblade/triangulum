//! The player: walk mode with radial gravity and box collision against the
//! voxel lattice, swimming, a fly mode, and block picking.
//!
//! Collision runs in the local lattice frame: around the player the lattice
//! (i, j, k) of the current face is a rectilinear grid of unit cells, so a
//! box in lattice units sweeps against cells axis by axis, and the metric
//! (metres per lattice unit along i, j and k) turns the player's size in
//! metres into lattice units. Positions are kept in planet-centred metres;
//! each step converts in and out, so crossing a face edge only changes
//! which lattice the next step is computed in.

use crate::camera::Camera;
use crate::grid::{BlockPos, Planet};
use crate::world::{BlockId, Voxels};
use glam::{DMat3, DVec3, IVec3};

#[cfg(test)]
#[path = "picking_tests.rs"]
mod picking_tests;

/// Fixed simulation step (the harness and the app both use it).
pub const STEP: f64 = 1.0 / 60.0;
pub const WIDTH_M: f64 = 0.6;
pub const HEIGHT_M: f64 = 1.8;
pub const EYE_M: f64 = 1.62;
pub const REACH_M: f64 = 6.0;
const GRAVITY: f64 = 25.0;
const JUMP_SPEED: f64 = 8.0;
const WALK_SPEED: f64 = 4.3;
const SPRINT_SPEED: f64 = 6.5;
/// Sneaking (Ctrl on the ground): slow, and the feet never leave the edge
/// of whatever they stand on, so you can build a bridge by backing up.
const SNEAK_SPEED: f64 = 1.3;
const SWIM_SPEED: f64 = 2.2;
const TERMINAL: f64 = 70.0;
/// Skin in lattice units: boxes stop this short of a face and ignore cells
/// they only touch.
const EPS: f64 = 1e-5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Walk,
    Fly,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    /// Ctrl: down in fly mode and in water; sneak on the ground.
    pub down: bool,
    pub sprint: bool,
}

/// Local lattice frame at a position.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub face: u8,
    /// Continuous lattice coordinates of the position.
    pub i: f64,
    pub j: f64,
    pub k: f64,
    /// Metres per lattice unit along i, j, k (columns are ~block_size wide, layers exactly).
    pub mi: f64,
    pub mj: f64,
    pub mk: f64,
    /// Unit world directions of +i, +j, +k.
    pub ei: DVec3,
    pub ej: DVec3,
    pub ek: DVec3,
    /// World metres per lattice unit (columns ei·mi, ej·mj, ek·mk) and its inverse.
    m: DMat3,
    inv: DMat3,
}

impl Frame {
    pub fn at(planet: &Planet, pos: DVec3) -> Frame {
        let (face, i, j, k) = planet.lattice_of_pos(pos);
        Self::from_lattice(planet, face, i, j, k)
    }

    /// The frame of a lattice point given explicitly: how a move that
    /// crosses a face edge continues in the neighbour's lattice.
    pub fn from_lattice(planet: &Planet, face: u8, i: f64, j: f64, k: f64) -> Frame {
        let n = planet.n as f64;
        let r = planet.radius_at(k);
        let h = 0.01;
        let (ia, ib) = ((i - h).max(0.0), (i + h).min(n));
        let (ja, jb) = ((j - h).max(0.0), (j + h).min(n));
        let di = (planet.dir(face, ib, j) - planet.dir(face, ia, j)) * r / (ib - ia);
        let dj = (planet.dir(face, i, jb) - planet.dir(face, i, ja)) * r / (jb - ja);
        let (mi, mj) = (di.length(), dj.length());
        let ei = di / mi;
        let ej = dj / mj;
        let ek = planet.dir(face, i, j);
        let mk = planet.block_size;
        let m = DMat3::from_cols(ei * mi, ej * mj, ek * mk);
        Frame { face, i, j, k, mi, mj, mk, ei, ej, ek, m, inv: m.inverse() }
    }

    /// World position of the frame's own lattice point.
    pub fn pos(&self, planet: &Planet) -> DVec3 {
        self.ek * planet.radius_at(self.k)
    }

    /// The cell the feet are in. A body resting on a floor sits at k = n
    /// to within float noise either way, so the layer is rounded with a
    /// micrometre of tolerance: standing on layer n's top is "in layer n".
    pub fn feet_cell(&self) -> (i64, i64, i64) {
        (self.i.floor() as i64, self.j.floor() as i64, (self.k + 1e-6).floor() as i64)
    }

    /// World metres -> lattice units.
    #[inline]
    pub fn to_lattice(&self, v: DVec3) -> DVec3 {
        self.inv * v
    }

    /// Lattice units -> world metres.
    #[inline]
    pub fn to_world(&self, dl: DVec3) -> DVec3 {
        self.m * dl
    }
}

/// A picked block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The block, resolved onto its own face.
    pub block: BlockPos,
    pub id: BlockId,
    /// The hit's own face and canonical cell, never the eye's extrapolated lattice.
    pub face: u8,
    pub cell: (i64, i64, i64),
    /// Lattice-axis normal in the hit's face (points out of the block).
    pub normal: IVec3,
    /// Distance along the ray (m).
    pub t: f64,
}

impl Hit {
    /// The neighbour in the hit's lattice; wrap this one cell across an edge if needed.
    pub fn place_cell(&self) -> (i64, i64, i64) {
        (self.cell.0 + self.normal.x as i64, self.cell.1 + self.normal.y as i64, self.cell.2 + self.normal.z as i64)
    }
}

mod picking;
pub use picking::pick;

#[derive(Clone, Debug)]
pub struct Player {
    /// Feet centre, planet-centred metres.
    pub pos: DVec3,
    /// World velocity (m/s).
    pub vel: DVec3,
    pub heading: f64,
    pub pitch: f64,
    pub mode: Mode,
    pub on_ground: bool,
    pub in_water: bool,
    pub head_in_water: bool,
    /// Fly speed (m/s), scroll-adjustable.
    pub fly_speed: f64,
    pub selected: usize,
    pub block_size: f64,
}

impl Player {
    pub fn new(pos: DVec3, heading_deg: f64, pitch_deg: f64, mode: Mode, block_size: f64) -> Self {
        Self { pos, vel: DVec3::ZERO, heading: heading_deg.to_radians(), pitch: pitch_deg.to_radians(), mode, on_ground: false, in_water: false, head_in_water: false, fly_speed: 20.0, selected: 0, block_size }
    }

    pub fn eye(&self) -> DVec3 {
        self.pos + self.pos.normalize() * EYE_M
    }

    pub fn camera(&self) -> Camera {
        Camera { pos: self.eye(), heading: self.heading, pitch: self.pitch, fov_y: 60f64.to_radians() }
    }

    /// Camera forward direction.
    pub fn look_dir(&self) -> DVec3 {
        self.camera().vectors().0
    }

    /// Does the player's box overlap a cell? Used to refuse placing into oneself.
    pub fn overlaps_cell(&self, planet: &Planet, face: u8, cell: (i64, i64, i64)) -> bool {
        let f = Frame::at(planet, self.pos);
        let (face, i, j) = planet.wrap_cell(face, cell.0, cell.1);
        // Test the actual neighbour's corners in the body's frame. A cell
        // centre on another face says nothing about whether its side cuts
        // into the body at an edge or a three-face corner.
        let corners: [DVec3; 8] = std::array::from_fn(|c| {
            let p = planet.pos(face, i as f64 + (c & 1) as f64,
                j as f64 + ((c >> 1) & 1) as f64, (cell.2 + ((c >> 2) & 1) as i64) as f64);
            f.to_lattice(p - self.pos)
        });
        let half = DVec3::new(WIDTH_M * 0.5 / f.mi, WIDTH_M * 0.5 / f.mj, HEIGHT_M * 0.5 / f.mk);
        let centre = DVec3::new(0.0, 0.0, half.z);
        let separated = |axis: DVec3| {
            if axis.length_squared() < 1e-20 { return false; }
            let axis = axis.normalize();
            let extent = (half - DVec3::splat(EPS)).dot(axis.abs());
            let lo = corners.iter().map(|v| (*v - centre).dot(axis)).fold(f64::INFINITY, f64::min);
            let hi = corners.iter().map(|v| (*v - centre).dot(axis)).fold(f64::NEG_INFINITY, f64::max);
            lo >= extent || hi <= -extent
        };
        let axes = [DVec3::X, DVec3::Y, DVec3::Z];
        if axes.iter().any(|&axis| separated(axis)) { return false; }
        // Separating-axis test: box axes, cell triangle normals, and edge
        // crosses. Triangulate the slightly non-planar radial prism sides.
        for [a, b, c, d] in [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4],
            [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]] {
            for [x, y, z] in [[a, b, c], [a, c, d]] {
                if separated((corners[y] - corners[x]).cross(corners[z] - corners[x])) { return false; }
                for (u, v) in [(x, y), (y, z), (z, x)] {
                    for axis in axes {
                        if separated((corners[v] - corners[u]).cross(axis)) { return false; }
                    }
                }
            }
        }
        true
    }


    fn lattice_box(&self, f: &Frame) -> (DVec3, DVec3) {
        let hi = 0.5 * WIDTH_M / f.mi;
        let hj = 0.5 * WIDTH_M / f.mj;
        let hk = HEIGHT_M / f.mk;
        (DVec3::new(f.i - hi, f.j - hj, f.k), DVec3::new(f.i + hi, f.j + hj, f.k + hk))
    }

    /// One fixed step.
    pub fn step(&mut self, vox: &dyn Voxels, planet: &Planet, input: &Input, dt: f64) {
        match self.mode {
            Mode::Fly => self.step_fly(planet, input, dt),
            Mode::Walk => self.step_walk(vox, planet, input, dt),
        }
    }

    fn step_fly(&mut self, planet: &Planet, input: &Input, dt: f64) {
        let cam = self.camera();
        let (fwd, _, up) = cam.vectors();
        let right = cam.right();
        let mut mv = DVec3::ZERO;
        if input.forward {
            mv += fwd;
        }
        if input.back {
            mv -= fwd;
        }
        if input.left {
            mv -= right;
        }
        if input.right {
            mv += right;
        }
        if input.jump {
            mv += up;
        }
        if input.down {
            mv -= up;
        }
        let speed = if input.sprint { self.fly_speed * 4.0 } else { self.fly_speed };
        self.vel = if mv != DVec3::ZERO { mv.normalize() * speed } else { DVec3::ZERO };
        self.pos += self.vel * dt;
        let min_r = planet.bottom_radius() + 2.0;
        if self.pos.length() < min_r {
            self.pos = self.pos.normalize() * min_r;
        }
        self.on_ground = false;
        self.in_water = false;
        self.head_in_water = false;
    }

    fn step_walk(&mut self, vox: &dyn Voxels, planet: &Planet, input: &Input, dt: f64) {
        let f = Frame::at(planet, self.pos);
        // Where the body is: water at the feet or the hips means swimming.
        // Level-aware (M2b): a point above a partial cell's surface is dry.
        let cell_at = |dk_m: f64| (f.i.floor() as i64, f.j.floor() as i64, (f.k + dk_m / f.mk).floor() as i64);
        let liquid = |dk_m: f64| {
            let c = cell_at(dk_m);
            let kk = f.k + dk_m / f.mk;
            vox.submerged(f.face, c.0, c.1, c.2, kk - kk.floor())
        };
        self.in_water = liquid(0.1) || liquid(0.9);
        self.head_in_water = liquid(EYE_M);
        // Push out of a solid cell (a block placed into us, a teleport into
        // rock): climb to the top of the solid column.
        {
            let feet = cell_at(0.05);
            if vox.is_solid(vox.block(f.face, feet.0, feet.1, feet.2)) {
                let mut k = feet.2;
                while vox.is_solid(vox.block(f.face, feet.0, feet.1, k)) && k < feet.2 + 64 {
                    k += 1;
                }
                self.pos = self.pos.normalize() * planet.radius_at(k as f64 + EPS);
                self.vel = DVec3::ZERO;
                return;
            }
        }

        // Desired horizontal velocity in the tangent plane.
        let (north, east, up) = Camera::local_frame(self.pos);
        let fwd = north * self.heading.cos() + east * self.heading.sin();
        let right = fwd.cross(up).normalize();
        let mut wish = DVec3::ZERO;
        if input.forward {
            wish += fwd;
        }
        if input.back {
            wish -= fwd;
        }
        if input.left {
            wish -= right;
        }
        if input.right {
            wish += right;
        }
        let sneak = input.down && self.on_ground && !self.in_water;
        let speed = if self.in_water {
            SWIM_SPEED
        } else if sneak {
            SNEAK_SPEED
        } else if input.sprint {
            SPRINT_SPEED
        } else {
            WALK_SPEED
        };
        let wish = if wish != DVec3::ZERO { wish.normalize() * speed } else { DVec3::ZERO };
        let mut vh = self.vel - up * self.vel.dot(up);
        let accel = if self.on_ground { 60.0 } else if self.in_water { 12.0 } else { 10.0 };
        let dv = wish - vh;
        vh += dv.clamp_length_max(accel * dt);
        let mut vz = self.vel.dot(up);
        if self.in_water {
            // Buoyancy: net upward when submerged, a gentle sink at the surface, strong drag.
            let buoy = if self.head_in_water { GRAVITY * 1.15 } else { GRAVITY * 0.9 };
            vz += (buoy - GRAVITY) * dt;
            if input.jump {
                vz += (3.0 - vz) * (6.0 * dt).min(1.0);
            } else if input.down {
                vz += (-3.0 - vz) * (6.0 * dt).min(1.0);
            }
            vz *= (1.0 - 2.5 * dt).max(0.0);
            vz = vz.clamp(-4.0, 4.0);
        } else {
            vz -= GRAVITY * dt;
            if input.jump && self.on_ground {
                vz = JUMP_SPEED;
            }
            vz = vz.max(-TERMINAL);
        }
        self.vel = vh + up * vz;

        // Sweep the box through the lattice: k, then i, then j, in this
        // face's lattice.
        let dl = f.to_lattice(self.vel * dt);
        let (mut bmin, mut bmax) = self.lattice_box(&f);
        let dz = sweep(vox, f.face, bmin, bmax, 2, dl.z);
        let hit_z = dz != dl.z;
        bmin.z += dz;
        bmax.z += dz;
        let (dx, dy, hit_x, hit_y) = sweep_horizontal(vox, f.face, bmin, bmax, dl.x, dl.y, sneak);
        self.on_ground = hit_z && dl.z < 0.0;
        if hit_z {
            self.vel -= up * self.vel.dot(up);
        }
        if hit_x {
            self.vel -= f.ei * self.vel.dot(f.ei);
        }
        if hit_y {
            self.vel -= f.ej * self.vel.dot(f.ej);
        }
        let (ni, nj, nk) = (f.i + dx, f.j + dy, f.k + dz);
        let n = planet.n as f64;
        let r = planet.radius_at(nk);
        let inside = |i: f64, j: f64| (0.0..=n).contains(&i) && (0.0..=n).contains(&j);
        if inside(ni, nj) {
            // Still on this face: the position is exact on its lattice.
            self.pos = planet.dir(f.face, ni, nj) * r;
            return;
        }
        // The move leaves the face (review 2 #10). Split it at the edge: go
        // as far as the edge in this lattice, then sweep the rest in the
        // neighbour's own lattice, where the cells really are. The two
        // lattices meet at the edge with a kink, so the box re-expressed in
        // the neighbour's frame can lean into a cell this frame kept it
        // clear of; the whole box is checked there and nudged out.
        let s = edge_fraction(f.i, f.j, dx, dy, n);
        let (ei, ej) = (f.i + s * dx, f.j + s * dy);
        let rest = f.to_world(DVec3::new((1.0 - s) * dx, (1.0 - s) * dy, 0.0));
        // Just past the edge, on the neighbour (the corner policy of `wrap`
        // picks the face past two sides at once).
        let (face2, i2, j2) = planet.wrap(f.face, ei + (ni - ei) * 1e-6, ej + (nj - ej) * 1e-6);
        if face2 == f.face {
            // Nothing past there (clamped at a cube corner): stop at the edge.
            self.pos = planet.dir(f.face, ei, ej) * r;
            self.vel -= f.ei * self.vel.dot(f.ei) + f.ej * self.vel.dot(f.ej);
            return;
        }
        let f2 = Frame::from_lattice(planet, face2, i2, j2, nk);
        let dl2 = f2.to_lattice(rest);
        let (b2min, b2max) = self.lattice_box(&f2);
        let (dx2, dy2, hit_x2, hit_y2) = sweep_horizontal(vox, face2, b2min, b2max, dl2.x, dl2.y, sneak);
        if hit_x2 {
            self.vel -= f2.ei * self.vel.dot(f2.ei);
        }
        if hit_y2 {
            self.vel -= f2.ej * self.vel.dot(f2.ej);
        }
        let (ni2, nj2) = (f2.i + dx2, f2.j + dy2);
        self.pos = if inside(ni2, nj2) {
            planet.dir(face2, ni2, nj2) * r
        } else {
            // A second edge in one step (a corner): a tangent step.
            (f2.pos(planet) + f2.to_world(DVec3::new(dx2, dy2, 0.0))).normalize() * r
        };
        // Validate the whole box where it ended up, in its own frame.
        let fv = Frame::at(planet, self.pos);
        if let Some(push) = free_box(vox, &fv, self.lattice_box(&fv)) {
            if push != DVec3::ZERO {
                let (pi, pj) = (fv.i + push.x, fv.j + push.y);
                self.pos = if inside(pi, pj) { planet.dir(fv.face, pi, pj) * r } else { (self.pos + fv.to_world(push)).normalize() * r };
                let fc = Frame::at(planet, self.pos);
                if free_box(vox, &fc, self.lattice_box(&fc)) != Some(DVec3::ZERO) {
                    self.refuse_horizontal(planet, &f, up, r);
                }
            }
        } else {
            self.refuse_horizontal(planet, &f, up, r);
        }
    }

    /// No horizontal move this step: the vertical one only, on the lattice
    /// the step started in, and the horizontal velocity is dropped.
    fn refuse_horizontal(&mut self, planet: &Planet, f: &Frame, up: DVec3, r: f64) {
        self.pos = planet.dir(f.face, f.i, f.j) * r;
        self.vel = up * self.vel.dot(up);
    }

    /// Is the box free of solid cells in the lattice of its own position?
    /// (The collision invariant every step keeps.)
    pub fn box_is_free(&self, vox: &dyn Voxels, planet: &Planet) -> bool {
        let f = Frame::at(planet, self.pos);
        free_box(vox, &f, self.lattice_box(&f)) == Some(DVec3::ZERO)
    }

    /// Stand on the ground under `dir`: feet on the first air block above the
    /// highest solid block at or below `k_hint` (searching down, then up).
    pub fn place_on_ground(&mut self, vox: &dyn Voxels, planet: &Planet, dir: DVec3, k_hint: f64) {
        let (f, i, j) = planet.lattice_of_dir(dir);
        let (ci, cj) = (i.floor() as i64, j.floor() as i64);
        let mut k = k_hint.floor() as i64;
        let solid = |k: i64| vox.is_solid(vox.block(f, ci, cj, k));
        // Climb out of rock, then fall onto the first solid.
        while solid(k) && k < planet.height as i64 {
            k += 1;
        }
        while k > 0 && !solid(k - 1) {
            k -= 1;
        }
        self.pos = planet.dir(f, i, j) * planet.radius_at(k as f64 + EPS);
        self.vel = DVec3::ZERO;
        self.on_ground = true;
    }
}

/// Is there something solid under any part of the box's footprint?
fn supported(vox: &dyn Voxels, face: u8, bmin: DVec3, bmax: DVec3) -> bool {
    // Resting on a floor puts the feet at the layer boundary to within
    // float noise either way; the layer below is one down from there.
    let below = (bmin.z + 1e-6).floor() as i64 - 1;
    let (i0, i1) = ((bmin.x + EPS).floor() as i64, (bmax.x - EPS).floor() as i64);
    let (j0, j1) = ((bmin.y + EPS).floor() as i64, (bmax.y - EPS).floor() as i64);
    (i0..=i1).any(|i| (j0..=j1).any(|j| vox.is_solid(vox.block(face, i, j, below))))
}

/// Sneaking: shorten a horizontal move so the box keeps something solid
/// under it, by bisection on the displacement (Minecraft's sneak rule).
fn hold_edge(vox: &dyn Voxels, face: u8, bmin: DVec3, bmax: DVec3, axis: usize, d: f64) -> f64 {
    if d == 0.0 {
        return 0.0;
    }
    let shifted = |t: f64| {
        let mut off = DVec3::ZERO;
        off[axis] = d * t;
        (bmin + off, bmax + off)
    };
    let (a, b) = shifted(1.0);
    if supported(vox, face, a, b) {
        return d;
    }
    // Largest fraction that still has support (0 always has: we are on the ground).
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        let (a, b) = shifted(mid);
        if supported(vox, face, a, b) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    d * lo
}

/// Sweep i, then j, holding the edge when sneaking. Returns the allowed
/// displacements and whether each axis was stopped short.
fn sweep_horizontal(vox: &dyn Voxels, face: u8, mut bmin: DVec3, mut bmax: DVec3, dlx: f64, dly: f64, sneak: bool) -> (f64, f64, bool, bool) {
    let mut dx = sweep(vox, face, bmin, bmax, 0, dlx);
    if sneak {
        dx = hold_edge(vox, face, bmin, bmax, 0, dx);
    }
    bmin.x += dx;
    bmax.x += dx;
    let mut dy = sweep(vox, face, bmin, bmax, 1, dly);
    if sneak {
        dy = hold_edge(vox, face, bmin, bmax, 1, dy);
    }
    (dx, dy, dx != dlx, dy != dly)
}

/// The fraction of the horizontal move (dx, dy) from (i, j) at which the
/// point reaches a side of the face (1 when it stays inside).
fn edge_fraction(i: f64, j: f64, dx: f64, dy: f64, n: f64) -> f64 {
    let mut s = 1.0f64;
    for (c, d) in [(i, dx), (j, dy)] {
        if d > 0.0 && c + d > n {
            s = s.min((n - c) / d);
        }
        if d < 0.0 && c + d < 0.0 {
            s = s.min(c / -d);
        }
    }
    s.clamp(0.0, 1.0)
}

/// The first solid cell the box overlaps in `face`'s lattice, as its corners.
fn first_solid(vox: &dyn Voxels, face: u8, bmin: DVec3, bmax: DVec3) -> Option<(DVec3, DVec3)> {
    let lo = |v: f64| (v + EPS).floor() as i64;
    let hi = |v: f64| (v - EPS).floor() as i64;
    for k in lo(bmin.z)..=hi(bmax.z) {
        for i in lo(bmin.x)..=hi(bmax.x) {
            for j in lo(bmin.y)..=hi(bmax.y) {
                if vox.is_solid(vox.block(face, i, j, k)) {
                    let c0 = DVec3::new(i as f64, j as f64, k as f64);
                    return Some((c0, c0 + DVec3::ONE));
                }
            }
        }
    }
    None
}

/// Check the box against the solid cells of its frame: Some(zero) when it
/// is clear, Some(push) when a sideways nudge of that much (lattice units,
/// at most half a cell) clears it, None when nothing small does.
fn free_box(vox: &dyn Voxels, f: &Frame, (bmin, bmax): (DVec3, DVec3)) -> Option<DVec3> {
    let (mut bmin, mut bmax) = (bmin, bmax);
    let mut total = DVec3::ZERO;
    for _ in 0..4 {
        let Some((c0, c1)) = first_solid(vox, f.face, bmin, bmax) else { return Some(total) };
        let candidates = [
            DVec3::new(c1.x - bmin.x + 2.0 * EPS, 0.0, 0.0),
            DVec3::new(-(bmax.x - c0.x + 2.0 * EPS), 0.0, 0.0),
            DVec3::new(0.0, c1.y - bmin.y + 2.0 * EPS, 0.0),
            DVec3::new(0.0, -(bmax.y - c0.y + 2.0 * EPS), 0.0),
        ];
        let push = candidates.into_iter().min_by(|a, b| a.length().total_cmp(&b.length())).unwrap();
        if push.length() > 0.5 {
            return None;
        }
        bmin += push;
        bmax += push;
        total += push;
    }
    None
}

/// Sweep an axis-aligned box along `axis` by `d` lattice units; return the
/// displacement allowed before the first solid cell.
fn sweep(vox: &dyn Voxels, face: u8, bmin: DVec3, bmax: DVec3, axis: usize, d: f64) -> f64 {
    if d == 0.0 {
        return 0.0;
    }
    let (a1, a2) = match axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let lo1 = (bmin[a1] + EPS).floor() as i64;
    let hi1 = (bmax[a1] - EPS).floor() as i64;
    let lo2 = (bmin[a2] + EPS).floor() as i64;
    let hi2 = (bmax[a2] - EPS).floor() as i64;
    let mut cell = [0i64; 3];
    let mut allowed = d;
    if d > 0.0 {
        let lead = bmax[axis];
        let c0 = (lead + EPS).floor() as i64;
        let c1 = (lead + d - EPS).floor() as i64;
        'outer: for c in c0..=c1 {
            for u in lo1..=hi1 {
                for v in lo2..=hi2 {
                    cell[axis] = c;
                    cell[a1] = u;
                    cell[a2] = v;
                    if vox.is_solid(vox.block(face, cell[0], cell[1], cell[2])) {
                        allowed = (c as f64 - lead).max(0.0);
                        break 'outer;
                    }
                }
            }
        }
        allowed.min(d)
    } else {
        let lead = bmin[axis];
        let c0 = (lead - EPS).floor() as i64;
        let c1 = (lead + d + EPS).floor() as i64;
        'outer: for c in (c1..=c0).rev() {
            for u in lo1..=hi1 {
                for v in lo2..=hi2 {
                    cell[axis] = c;
                    cell[a1] = u;
                    cell[a2] = v;
                    if vox.is_solid(vox.block(face, cell[0], cell[1], cell[2])) {
                        allowed = ((c + 1) as f64 - lead).min(0.0);
                        break 'outer;
                    }
                }
            }
        }
        allowed.max(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const STONE: BlockId = 1;
    const WATER: BlockId = 8;

    /// A flat world: solid below `floor_k`, explicit cells on top. Cells are
    /// addressed in any face's lattice without resolving, which is exactly
    /// what crossing an edge needs: the floor continues everywhere.
    struct Flat {
        floor_k: i64,
        cells: HashMap<(i64, i64, i64), BlockId>,
    }

    impl Voxels for Flat {
        fn block(&self, _face: u8, i: i64, j: i64, k: i64) -> BlockId {
            if k < self.floor_k {
                return STONE;
            }
            self.cells.get(&(i, j, k)).copied().unwrap_or(0)
        }
        fn is_solid(&self, id: BlockId) -> bool {
            id != 0 && id != WATER
        }
        fn is_liquid(&self, id: BlockId) -> bool {
            id == WATER
        }
    }

    fn planet(block_size: f64) -> Planet {
        Planet::new(block_size, 2048, 512, (131072.0 / block_size) as u32)
    }

    /// A player standing on the floor at lattice (face, i, j), facing +i.
    fn standing(p: &Planet, face: u8, i: f64, j: f64, floor_k: i64) -> Player {
        let pos = p.dir(face, i, j) * p.radius_at(floor_k as f64 + 0.001);
        let f = Frame::at(p, pos);
        let (north, east, _) = Camera::local_frame(pos);
        let heading = f.ei.dot(east).atan2(f.ei.dot(north));
        let mut pl = Player::new(pos, heading.to_degrees(), 0.0, Mode::Walk, p.block_size);
        pl.on_ground = true;
        pl
    }

    fn run(pl: &mut Player, w: &dyn Voxels, p: &Planet, input: Input, steps: usize) {
        for _ in 0..steps {
            pl.step(w, p, &input, STEP);
        }
    }

    #[test]
    fn falls_onto_the_floor_and_rests_exactly_on_it() {
        let p = planet(1.0);
        let w = Flat { floor_k: 600, cells: HashMap::new() };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        pl.pos = pl.pos.normalize() * p.radius_at(605.0);
        pl.on_ground = false;
        run(&mut pl, &w, &p, Input::default(), 120);
        let f = Frame::at(&p, pl.pos);
        assert!(pl.on_ground);
        assert!((f.k - 600.0).abs() < 1e-6, "k = {}", f.k);
        assert!(pl.vel.length() < 1e-9);
        // and it stays there
        run(&mut pl, &w, &p, Input::default(), 60);
        let f = Frame::at(&p, pl.pos);
        assert!(pl.on_ground && (f.k - 600.0).abs() < 1e-6);
        assert!((f.i - 1000.5).abs() < 1e-9 && (f.j - 2000.5).abs() < 1e-9, "no drift: {} {}", f.i, f.j);
    }

    #[test]
    fn walls_stop_the_box_and_a_jump_clears_a_block() {
        let p = planet(1.0);
        let mut cells = HashMap::new();
        // a wall at i = 1003, two blocks high, across our path
        for dj in -2..=2 {
            cells.insert((1003, 2000 + dj, 600), STONE);
            cells.insert((1003, 2000 + dj, 601), STONE);
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 180);
        let f = Frame::at(&p, pl.pos);
        // Columns here are ~1.08 m wide (equal-area grid), so the 0.3 m half width is 0.3 / mi columns.
        assert!((f.i + 0.3 / f.mi - 1003.0).abs() < 1e-4, "stopped at the wall: i = {} (mi {})", f.i, f.mi);
        assert!(pl.on_ground);
        // a one-block step, three columns deep: walking fails, a running jump lands on it
        let mut cells = HashMap::new();
        for di in 1003..1006 {
            for dj in -2..=2 {
                cells.insert((di, 2000 + dj, 600), STONE);
            }
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 120);
        let f = Frame::at(&p, pl.pos);
        assert!(f.i < 1003.0 && (f.k - 600.0).abs() < 1e-6, "blocked: i {} k {}", f.i, f.k);
        // A running jump (air control alone will not carry a standing jump 1.7 m).
        let mut pl = standing(&p, 2, 999.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 30);
        run(&mut pl, &w, &p, Input { forward: true, jump: true, ..Default::default() }, 1);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 45);
        let f = Frame::at(&p, pl.pos);
        assert!(f.i > 1003.0 && f.i < 1006.0, "jumped onto the step: i = {}", f.i);
        assert!((f.k - 601.0).abs() < 1e-6 && pl.on_ground, "standing on the step: k = {}", f.k);
    }

    #[test]
    fn half_metre_blocks_size_the_box_in_blocks() {
        let p = planet(0.5);
        let f0 = Frame::at(&p, p.dir(2, 1000.5, 2000.5) * p.radius_at(600.0));
        assert!((f0.mi - 0.5).abs() < 0.05 && (f0.mk - 0.5).abs() < 1e-12, "mi {} mk {}", f0.mi, f0.mk);
        // A gap one block (0.5 m) wide does not admit a 0.6 m body; two blocks do.
        let mut cells = HashMap::new();
        for dj in -6..=6 {
            if dj != 0 {
                for k in 600..604 {
                    cells.insert((1003, 2000 + dj, k), STONE);
                }
            }
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 120);
        assert!(Frame::at(&p, pl.pos).i < 1003.0, "0.5 m gap blocks a 0.6 m body");
        let mut cells = HashMap::new();
        for dj in -6..=6 {
            if dj != 0 && dj != 1 {
                for k in 600..604 {
                    cells.insert((1003, 2000 + dj, k), STONE);
                }
            }
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2001.0, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 120);
        assert!(Frame::at(&p, pl.pos).i > 1004.0, "1 m gap admits it");
        // A 1.8 m body (3.6 blocks) needs a 4-block ceiling, not 3.
        let mut cells = HashMap::new();
        for di in 1002..1010 {
            for dj in -3..=3 {
                cells.insert((di, 2000 + dj, 603), STONE);
            }
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 120);
        assert!(Frame::at(&p, pl.pos).i < 1002.0, "a 1.5 m tunnel keeps a 1.8 m body out");
        let mut cells = HashMap::new();
        for di in 1002..1010 {
            for dj in -3..=3 {
                cells.insert((di, 2000 + dj, 604), STONE);
            }
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 120);
        assert!(Frame::at(&p, pl.pos).i > 1003.0, "a 2 m tunnel admits it");
        // A 1 m step (two half-metre blocks, six columns deep) is cleared by the jump.
        let mut cells = HashMap::new();
        for di in 1003..1009 {
            for dj in -3..=3 {
                cells.insert((di, 2000 + dj, 600), STONE);
                cells.insert((di, 2000 + dj, 601), STONE);
            }
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1001.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, jump: true, ..Default::default() }, 1);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 45);
        let f = Frame::at(&p, pl.pos);
        assert!(f.i > 1003.0 && f.i < 1009.0 && (f.k - 602.0).abs() < 1e-6 && pl.on_ground, "on the 1 m step: i {} k {}", f.i, f.k);
    }

    #[test]
    fn walks_across_a_face_edge_and_round_a_corner() {
        let p = planet(1.0);
        let w = Flat { floor_k: 600, cells: HashMap::new() };
        let n = p.n as f64;
        // Edge: start 3 columns from the i = N edge of face 0, walk +i for 4 s.
        let mut pl = standing(&p, 0, n - 3.5, 40000.5, 600);
        let start = pl.pos;
        let mut max_step = 0.0f64;
        let mut faces = std::collections::BTreeSet::new();
        for _ in 0..240 {
            let before = pl.pos;
            pl.step(&w, &p, &Input { forward: true, ..Default::default() }, STEP);
            max_step = max_step.max((pl.pos - before).length());
            let f = Frame::at(&p, pl.pos);
            faces.insert(f.face);
            assert!((f.k - 600.0).abs() < 1e-6, "stays on the floor across the edge: k = {}", f.k);
            assert!(pl.on_ground);
        }
        assert!(faces.len() == 2 && faces.contains(&0), "crossed onto a neighbour: {faces:?}");
        let dist = (pl.pos - start).length();
        assert!((dist - 4.0 * WALK_SPEED).abs() < 0.6, "walked {dist:.2} m in 4 s");
        assert!(max_step < WALK_SPEED * STEP * 1.5, "no jumps at the edge: {max_step}");
        // Corner: start near (N, N) of face 0 and walk diagonally through it.
        let mut pl = standing(&p, 0, n - 2.5, n - 2.5, 600);
        let f0 = Frame::at(&p, pl.pos);
        let (north, east, _) = Camera::local_frame(pl.pos);
        let diag = (f0.ei + f0.ej).normalize();
        pl.heading = diag.dot(east).atan2(diag.dot(north));
        let start = pl.pos;
        let mut faces = std::collections::BTreeSet::new();
        for _ in 0..240 {
            let before = pl.pos;
            pl.step(&w, &p, &Input { forward: true, ..Default::default() }, STEP);
            let d = (pl.pos - before).length();
            assert!(d < WALK_SPEED * STEP * 1.5, "no jump at the corner: {d}");
            let f = Frame::at(&p, pl.pos);
            faces.insert(f.face);
            assert!((f.k - 600.0).abs() < 1e-6 && pl.on_ground, "k = {}", f.k);
        }
        assert!(faces.len() >= 2, "left face 0 past the corner: {faces:?}");
        let dist = (pl.pos - start).length();
        assert!(dist > 4.0 * WALK_SPEED * 0.8, "kept walking through the corner: {dist:.2} m");
    }

    #[test]
    fn swims_in_water_and_floats_up() {
        let p = planet(1.0);
        let mut cells = HashMap::new();
        for di in 990..1010 {
            for dj in 1990..2010 {
                for k in 600..606 {
                    cells.insert((di, dj, k), WATER);
                }
            }
        }
        let w = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        pl.pos = pl.pos.normalize() * p.radius_at(601.0);
        pl.on_ground = false;
        run(&mut pl, &w, &p, Input::default(), 300);
        let f = Frame::at(&p, pl.pos);
        assert!(pl.in_water, "in the pool");
        assert!(f.k > 602.0 && f.k < 606.5, "floats near the surface, not on the floor: k = {}", f.k);
        assert!(!pl.on_ground);
        // Swimming up with Space reaches the surface and bobs there.
        run(&mut pl, &w, &p, Input { jump: true, ..Default::default() }, 180);
        let f = Frame::at(&p, pl.pos);
        assert!(f.k > 604.5, "swam up: k = {}", f.k);
    }

    /// A pool one layer deep whose water sits at a level (M2b).
    struct Film {
        level: u8,
    }

    impl Voxels for Film {
        fn block(&self, _face: u8, _i: i64, _j: i64, k: i64) -> BlockId {
            if k < 600 {
                STONE
            } else if k == 600 {
                WATER
            } else {
                0
            }
        }
        fn is_solid(&self, id: BlockId) -> bool {
            id != 0 && id != WATER
        }
        fn is_liquid(&self, id: BlockId) -> bool {
            id == WATER
        }
        fn water_level(&self, _face: u8, _i: i64, _j: i64, _k: i64) -> u8 {
            self.level
        }
    }

    /// M2b: a film of water two levels deep is walked on, a full cell is swum in.
    #[test]
    fn standing_in_a_thin_film_is_not_swimming() {
        let p = planet(1.0);
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        run(&mut pl, &Film { level: 2 }, &p, Input::default(), 30);
        assert!(!pl.in_water, "a 2/255 film is under the feet, not around them");
        assert!(pl.on_ground);
        let f = Frame::at(&p, pl.pos);
        assert!((f.k - 600.0).abs() < 0.01, "stands on the floor at k = {}", f.k);
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        run(&mut pl, &Film { level: 255 }, &p, Input::default(), 30);
        assert!(pl.in_water, "a full cell is swum in");
        // The eye at 1.62 m is above a one-layer pool either way.
        assert!(!pl.head_in_water);
    }

    /// A four-column stone platform over nothing.
    struct Platform;
    impl Voxels for Platform {
        fn block(&self, _face: u8, i: i64, _j: i64, k: i64) -> BlockId {
            if k < 600 && (1000..=1003).contains(&i) { STONE } else { 0 }
        }
        fn is_solid(&self, id: BlockId) -> bool {
            id != 0 && id != WATER
        }
        fn is_liquid(&self, id: BlockId) -> bool {
            id == WATER
        }
    }

    #[test]
    fn sneaking_holds_the_edge_and_walking_falls_off() {
        let p = planet(1.0);
        let w = Platform;
        // Walking off the end: airborne within two seconds, falling.
        let mut pl = standing(&p, 2, 1001.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 120);
        let f = Frame::at(&p, pl.pos);
        assert!(!pl.on_ground && f.k < 599.0, "walked off: k = {}", f.k);
        // Sneaking: slower, and the box's leading edge stops at the platform's edge.
        let mut pl = standing(&p, 2, 1001.5, 2000.5, 600);
        run(&mut pl, &w, &p, Input { forward: true, down: true, ..Default::default() }, 60);
        let f = Frame::at(&p, pl.pos);
        assert!(pl.on_ground && (f.k - 600.0).abs() < 1e-6);
        assert!(f.i > 1001.5 && f.i < 1003.0, "sneaking moves, slowly: i = {}", f.i);
        run(&mut pl, &w, &p, Input { forward: true, down: true, ..Default::default() }, 240);
        let f = Frame::at(&p, pl.pos);
        assert!(pl.on_ground && (f.k - 600.0).abs() < 1e-6, "still on the platform: k = {}", f.k);
        // Like Minecraft, the box may overhang until its trailing edge is at the platform's edge.
        let trail = f.i - 0.3 / f.mi;
        assert!(trail <= 1004.0 + 1e-6 && trail > 1003.99, "trailing edge held at the platform's edge: {trail}");
        // Sideways and backwards too.
        for input in [Input { back: true, down: true, ..Default::default() }, Input { left: true, down: true, ..Default::default() }, Input { right: true, down: true, ..Default::default() }] {
            let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
            run(&mut pl, &w, &p, input, 240);
            let f = Frame::at(&p, pl.pos);
            assert!(pl.on_ground && (f.k - 600.0).abs() < 1e-6, "{input:?}: k = {}", f.k);
        }
        // Releasing Ctrl at the edge walks off.
        run(&mut pl, &w, &p, Input { forward: true, ..Default::default() }, 120);
        assert!(!pl.on_ground);
        // In water Ctrl still swims down.
        let mut cells = HashMap::new();
        for di in 990..1010 {
            for dj in 1990..2010 {
                for k in 600..610 {
                    cells.insert((di, dj, k), WATER);
                }
            }
        }
        let pool = Flat { floor_k: 600, cells };
        let mut pl = standing(&p, 2, 1000.5, 2000.5, 600);
        pl.pos = pl.pos.normalize() * p.radius_at(607.0);
        pl.on_ground = false;
        run(&mut pl, &pool, &p, Input { down: true, ..Default::default() }, 120);
        let f = Frame::at(&p, pl.pos);
        assert!(pl.in_water && f.k < 604.0, "swam down: k = {}", f.k);
    }

    #[test]
    fn picking_finds_the_facing_block_and_its_normal() {
        let p = planet(1.0);
        let mut cells = HashMap::new();
        cells.insert((1004, 2000, 600), STONE);
        cells.insert((1004, 2000, 601), STONE);
        let w = Flat { floor_k: 600, cells };
        let pl = standing(&p, 2, 1000.5, 2000.5, 600);
        let hit = pick(&w, &p, pl.eye(), pl.look_dir(), REACH_M, false).expect("hit");
        assert_eq!(hit.cell, (1004, 2000, 601));
        assert_eq!(hit.normal, IVec3::new(-1, 0, 0));
        assert_eq!(hit.place_cell(), (1003, 2000, 601));
        let mi = Frame::at(&p, pl.eye()).mi;
        assert!((hit.t - 3.5 * mi).abs() < 0.01, "distance {} for 3.5 columns of {mi} m", hit.t);
        // Looking down finds the floor under the feet.
        let mut down = pl.clone();
        down.pitch = -1.5;
        let hit = pick(&w, &p, down.eye(), down.look_dir(), REACH_M, false).expect("floor");
        assert_eq!(hit.cell.2, 599);
        assert_eq!(hit.normal, IVec3::new(0, 0, 1));
        // Out of reach: nothing.
        let w2 = Flat { floor_k: 600, cells: HashMap::new() };
        assert!(pick(&w2, &p, pl.eye(), pl.look_dir(), REACH_M, false).is_none());
        // A placed block inside the player's box is detected.
        assert!(pl.overlaps_cell(&p, 2, (1000, 2000, 600)));
        assert!(!pl.overlaps_cell(&p, 2, (1002, 2000, 600)));
    }

    /// A world addressed the way `World` is: a cell past a face's side is
    /// the neighbouring face's own cell (`Planet::wrap_cell`), so a block
    /// on face 2 is one block however it is reached.
    struct Canon {
        planet: Planet,
        floor_k: i64,
        cells: HashMap<BlockPos, BlockId>,
    }

    impl Canon {
        fn canonical(&self, face: u8, i: i64, j: i64, k: i64) -> Option<BlockPos> {
            if k < 0 || k >= self.planet.height as i64 {
                return None;
            }
            let (f, ci, cj) = self.planet.wrap_cell(face, i, j);
            Some(BlockPos { face: f, i: ci, j: cj, k: k as u32 })
        }
    }

    impl Voxels for Canon {
        fn block(&self, face: u8, i: i64, j: i64, k: i64) -> BlockId {
            if k < 0 {
                return STONE; // the virtual bedrock floor, as World::block_lattice
            }
            if k < self.floor_k {
                return STONE;
            }
            self.canonical(face, i, j, k).and_then(|bp| self.cells.get(&bp).copied()).unwrap_or(0)
        }
        fn is_solid(&self, id: BlockId) -> bool {
            id != 0 && id != WATER
        }
        fn is_liquid(&self, id: BlockId) -> bool {
            id == WATER
        }
    }

    /// Review 2 #6: a ray from face 0 that hits a block past the edge, on
    /// face 2, names that block on face 2, not face 0's clamped boundary
    /// column; and a solid below layer 0 (the virtual floor) is no hit.
    #[test]
    fn picking_across_a_face_edge_names_the_neighbours_block() {
        let p = planet(1.0);
        let n = p.n as i64;
        let mut w = Canon { planet: p, floor_k: 600, cells: HashMap::new() };
        // Two blocks past face 0's i = N side, at the player's eye height.
        let target = w.canonical(0, n + 1, 40000, 601).unwrap();
        assert_eq!(target.face, 2, "face 0's +i side meets face 2");
        w.cells.insert(target, STONE);
        let pl = standing(&p, 0, n as f64 - 1.5, 40000.5, 600);
        // Aim at the neighbour's actual centre. Continuing the eye's +i
        // tangent does not follow this wrapped index row (B3).
        let centre = p.pos(target.face, target.i as f64 + 0.5, target.j as f64 + 0.5, target.k as f64 + 0.5);
        let hit = pick(&w, &p, pl.eye(), centre - pl.eye(), REACH_M, false).expect("hit across the edge");
        assert_eq!(hit.id, STONE);
        assert_eq!(hit.face, target.face);
        assert_eq!(hit.cell, (target.i as i64, target.j as i64, target.k as i64), "the hit is in its own face lattice");
        assert_eq!(hit.block, target, "the block is the face-2 block the ray hit, not {:?}", BlockPos { face: 0, i: p.n - 1, j: 40000, k: 601 });
        // The old clamped column must not be what breaking would change.
        assert_ne!(hit.block.face, 0);
        // Below the world: standing on the bottom layer's top with nothing
        // under it, looking down finds the virtual floor and reports no hit
        // rather than layer 0.
        let w0 = Canon { planet: p, floor_k: 0, cells: HashMap::new() };
        let mut down = standing(&p, 2, 1000.5, 2000.5, 0);
        down.pitch = -1.5;
        assert!(pick(&w0, &p, down.eye(), down.look_dir(), REACH_M, false).is_none(), "no hit on the floor below layer 0");
        // ...while a real block at layer 0 is picked as layer 0.
        let mut w1 = Canon { planet: p, floor_k: 0, cells: HashMap::new() };
        w1.cells.insert(BlockPos { face: 2, i: 1000, j: 2000, k: 0 }, STONE);
        let mut up = standing(&p, 2, 1000.5, 2000.5, 1);
        up.pitch = -1.5;
        let hit = pick(&w1, &p, up.eye(), up.look_dir(), REACH_M, false).expect("layer 0");
        assert_eq!(hit.block, BlockPos { face: 2, i: 1000, j: 2000, k: 0 });
    }

    /// Review 2 #10: the two lattices meet at a face edge with a kink, so
    /// a box swept clear of a wall in face 0's lattice can, re-expressed
    /// in face 2's lattice after the crossing, overlap that wall. Astra's
    /// case: feet at face 0 (N-0.02, N-0.70, 600) walking +i along a wall
    /// in the next row. The box must never overlap a solid cell in the
    /// lattice of wherever it is, and the walk must get through.
    #[test]
    fn walking_across_a_face_edge_along_a_wall_keeps_the_box_out_of_it() {
        let p = planet(1.0);
        let n = p.n as i64;
        let nf = n as f64;
        let mut w = Canon { planet: p, floor_k: 600, cells: HashMap::new() };
        // The wall: row N-2 of face 0, continued past the edge as the same
        // row of face 2 (the cells face 0's lattice reaches at j = N-2),
        // two blocks high.
        for di in -8..=8 {
            for k in 600..602 {
                let bp = w.canonical(0, n + di, n - 2, k).unwrap();
                w.cells.insert(bp, STONE);
            }
        }
        let mut pl = standing(&p, 0, nf - 0.02, nf - 0.70, 600);
        let start = pl.pos;
        let f0 = Frame::at(&p, pl.pos);
        assert_eq!(f0.face, 0);
        assert!(pl.box_is_free(&w, &p), "starts clear of the wall");
        let mut faces = std::collections::BTreeSet::new();
        for step in 0..120 {
            pl.step(&w, &p, &Input { forward: true, ..Default::default() }, STEP);
            let f = Frame::at(&p, pl.pos);
            faces.insert(f.face);
            let (bmin, bmax) = pl.lattice_box(&f);
            // How far into a solid cell the box reaches in its own lattice.
            if let Some((c0, c1)) = first_solid(&w, f.face, bmin, bmax) {
                let depth = (bmax.y - c0.y).min(c1.y - bmin.y).min(bmax.x - c0.x).min(c1.x - bmin.x);
                panic!("step {step} on face {}: the box ({:.4}..{:.4}, {:.4}..{:.4}) overlaps the solid cell {:?} by {depth:.4} lattice units", f.face, bmin.x, bmax.x, bmin.y, bmax.y, c0);
            }
            assert!((f.k - 600.0).abs() < 1e-6 && pl.on_ground, "step {step}: k = {}", f.k);
        }
        assert!(faces.contains(&2), "crossed onto face 2: {faces:?}");
        let dist = (pl.pos - start).length();
        assert!((dist - 2.0 * WALK_SPEED).abs() < 0.3, "kept walking along the wall through the edge: {dist:.2} m in 2 s");
    }

    /// The whole-box check also catches a wall straight ahead past the edge
    /// (the sweep sees it through `wrap_cell`, the kink shifts it a little):
    /// walking +i into face 2's second column stops short of it and never
    /// overlaps it.
    #[test]
    fn a_wall_just_past_the_edge_stops_the_walk_without_overlap() {
        let p = planet(1.0);
        let n = p.n as i64;
        let nf = n as f64;
        let mut w = Canon { planet: p, floor_k: 600, cells: HashMap::new() };
        for dj in -6..=6 {
            for k in 600..602 {
                let bp = w.canonical(0, n + 1, n - 4 + dj, k).unwrap();
                w.cells.insert(bp, STONE);
            }
        }
        let mut pl = standing(&p, 0, nf - 1.5, nf - 4.5, 600);
        for step in 0..120 {
            pl.step(&w, &p, &Input { forward: true, ..Default::default() }, STEP);
            assert!(pl.box_is_free(&w, &p), "step {step}: the box overlaps the wall");
        }
        let f = Frame::at(&p, pl.pos);
        assert_eq!(f.face, 2, "stopped against the wall on face 2 (i = {}, j = {})", f.i, f.j);
        assert!(pl.on_ground);
    }
}

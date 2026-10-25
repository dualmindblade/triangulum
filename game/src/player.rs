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
        let n = planet.n as f64;
        let r = pos.length();
        let h = 0.01;
        let (ia, ib) = ((i - h).max(0.0), (i + h).min(n));
        let (ja, jb) = ((j - h).max(0.0), (j + h).min(n));
        let di = (planet.dir(face, ib, j) - planet.dir(face, ia, j)) * r / (ib - ia);
        let dj = (planet.dir(face, i, jb) - planet.dir(face, i, ja)) * r / (jb - ja);
        let (mi, mj) = (di.length(), dj.length());
        let ei = di / mi;
        let ej = dj / mj;
        let ek = pos / r;
        let mk = planet.block_size;
        let m = DMat3::from_cols(ei * mi, ej * mj, ek * mk);
        Frame { face, i, j, k, mi, mj, mk, ei, ej, ek, m, inv: m.inverse() }
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
    /// The cell in the frame's lattice (may lie past the face edge).
    pub face: u8,
    pub cell: (i64, i64, i64),
    /// Lattice-axis normal of the entered face (points out of the block).
    pub normal: IVec3,
    /// Distance along the ray (m).
    pub t: f64,
}

impl Hit {
    /// The cell a placed block would go into (frame lattice).
    pub fn place_cell(&self) -> (i64, i64, i64) {
        (self.cell.0 + self.normal.x as i64, self.cell.1 + self.normal.y as i64, self.cell.2 + self.normal.z as i64)
    }
}

/// March a ray through the lattice (Amanatides-Woo) for `reach_m` metres,
/// returning the first solid cell. Linear in lattice space: over a few
/// metres the lattice is straight to well under a millimetre.
pub fn pick(vox: &dyn Voxels, planet: &Planet, eye: DVec3, dir: DVec3, reach_m: f64) -> Option<Hit> {
    let frame = Frame::at(planet, eye);
    let o = DVec3::new(frame.i, frame.j, frame.k);
    let d = frame.to_lattice(dir.normalize()); // lattice units per metre
    let mut cell = [o.x.floor() as i64, o.y.floor() as i64, o.z.floor() as i64];
    let mut step = [0i64; 3];
    let mut t_max = [f64::INFINITY; 3];
    let mut t_delta = [f64::INFINITY; 3];
    for a in 0..3 {
        if d[a] > 0.0 {
            step[a] = 1;
            t_max[a] = ((cell[a] + 1) as f64 - o[a]) / d[a];
            t_delta[a] = 1.0 / d[a];
        } else if d[a] < 0.0 {
            step[a] = -1;
            t_max[a] = (cell[a] as f64 - o[a]) / d[a];
            t_delta[a] = -1.0 / d[a];
        }
    }
    let mut t = 0.0;
    let mut normal = IVec3::ZERO;
    for _ in 0..512 {
        let id = vox.block(frame.face, cell[0], cell[1], cell[2]);
        if vox.is_solid(id) {
            let bp = BlockPos { face: frame.face, i: cell[0].clamp(0, planet.n as i64 - 1) as u32, j: cell[1].clamp(0, planet.n as i64 - 1) as u32, k: cell[2].clamp(0, planet.height as i64 - 1) as u32 };
            return Some(Hit { block: bp, id, face: frame.face, cell: (cell[0], cell[1], cell[2]), normal, t });
        }
        let a = if t_max[0] < t_max[1] { if t_max[0] < t_max[2] { 0 } else { 2 } } else if t_max[1] < t_max[2] { 1 } else { 2 };
        t = t_max[a];
        if t > reach_m {
            return None;
        }
        cell[a] += step[a];
        t_max[a] += t_delta[a];
        normal = IVec3::ZERO;
        normal[a] = -step[a] as i32;
    }
    None
}

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

    /// Does the player's box overlap any solid cell? (Used to refuse placing a block into oneself.)
    pub fn overlaps_cell(&self, planet: &Planet, face: u8, cell: (i64, i64, i64)) -> bool {
        let f = Frame::at(planet, self.pos);
        if f.face != face {
            // Resolve the cell onto this frame by comparing world positions of the cell centre.
            let centre = planet.dir(face, cell.0 as f64 + 0.5, cell.1 as f64 + 0.5) * planet.radius_at(cell.2 as f64 + 0.5);
            let fc = Frame::at(planet, centre);
            if fc.face != f.face {
                return false;
            }
            return self.box_overlaps(&f, (fc.i.floor() as i64, fc.j.floor() as i64, fc.k.floor() as i64));
        }
        self.box_overlaps(&f, cell)
    }

    fn box_overlaps(&self, f: &Frame, cell: (i64, i64, i64)) -> bool {
        let (bmin, bmax) = self.lattice_box(f);
        let (c0, c1) = (DVec3::new(cell.0 as f64, cell.1 as f64, cell.2 as f64), DVec3::new(cell.0 as f64 + 1.0, cell.1 as f64 + 1.0, cell.2 as f64 + 1.0));
        (0..3).all(|a| bmin[a] < c1[a] - EPS && bmax[a] > c0[a] + EPS)
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
        let cell_at = |dk_m: f64| (f.i.floor() as i64, f.j.floor() as i64, (f.k + dk_m / f.mk).floor() as i64);
        let liquid = |c: (i64, i64, i64)| vox.is_liquid(vox.block(f.face, c.0, c.1, c.2));
        self.in_water = liquid(cell_at(0.1)) || liquid(cell_at(0.9));
        self.head_in_water = liquid(cell_at(EYE_M));
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
        let speed = if self.in_water { SWIM_SPEED } else if input.sprint { SPRINT_SPEED } else { WALK_SPEED };
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

        // Sweep the box through the lattice: k, then i, then j.
        let dl = f.to_lattice(self.vel * dt);
        let (mut bmin, mut bmax) = self.lattice_box(&f);
        let dz = sweep(vox, f.face, bmin, bmax, 2, dl.z);
        let hit_z = dz != dl.z;
        bmin.z += dz;
        bmax.z += dz;
        let dx = sweep(vox, f.face, bmin, bmax, 0, dl.x);
        let hit_x = dx != dl.x;
        bmin.x += dx;
        bmax.x += dx;
        let dy = sweep(vox, f.face, bmin, bmax, 1, dl.y);
        let hit_y = dy != dl.y;
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
        // New position: exact on this face's lattice when still inside it,
        // a tangent step when the move crosses the face edge.
        let (ni, nj, nk) = (f.i + dx, f.j + dy, f.k + dz);
        let n = planet.n as f64;
        let r = planet.radius_at(nk);
        self.pos = if (0.0..=n).contains(&ni) && (0.0..=n).contains(&nj) {
            planet.dir(f.face, ni, nj) * r
        } else {
            (self.pos + f.to_world(DVec3::new(dx, dy, 0.0))).normalize() * r
        };
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

    fn run(pl: &mut Player, w: &Flat, p: &Planet, input: Input, steps: usize) {
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

    #[test]
    fn picking_finds_the_facing_block_and_its_normal() {
        let p = planet(1.0);
        let mut cells = HashMap::new();
        cells.insert((1004, 2000, 600), STONE);
        cells.insert((1004, 2000, 601), STONE);
        let w = Flat { floor_k: 600, cells };
        let pl = standing(&p, 2, 1000.5, 2000.5, 600);
        let hit = pick(&w, &p, pl.eye(), pl.look_dir(), REACH_M).expect("hit");
        assert_eq!(hit.cell, (1004, 2000, 601));
        assert_eq!(hit.normal, IVec3::new(-1, 0, 0));
        assert_eq!(hit.place_cell(), (1003, 2000, 601));
        let mi = Frame::at(&p, pl.eye()).mi;
        assert!((hit.t - 3.5 * mi).abs() < 0.01, "distance {} for 3.5 columns of {mi} m", hit.t);
        // Looking down finds the floor under the feet.
        let mut down = pl.clone();
        down.pitch = -1.5;
        let hit = pick(&w, &p, down.eye(), down.look_dir(), REACH_M).expect("floor");
        assert_eq!(hit.cell.2, 599);
        assert_eq!(hit.normal, IVec3::new(0, 0, 1));
        // Out of reach: nothing.
        let w2 = Flat { floor_k: 600, cells: HashMap::new() };
        assert!(pick(&w2, &p, pl.eye(), pl.look_dir(), REACH_M).is_none());
        // A placed block inside the player's box is detected.
        assert!(pl.overlaps_cell(&p, 2, (1000, 2000, 600)));
        assert!(!pl.overlaps_cell(&p, 2, (1002, 2000, 600)));
    }
}

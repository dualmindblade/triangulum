//! The surface of partial powder (B7): what the mesher draws and what the
//! player stands on, computed in one place so the two cannot disagree.
//!
//! A pile at 35 degrees falls 0.7 of a block per cell, so its surface crosses
//! layers inside a column, and water's way of drawing a level (a sloped top
//! that stays inside its cell) would leave a stair of risers. So the slope is
//! a height field over columns, in units of 1/255 of a layer:
//!
//! - A column's height is the top of its surface (`Top`). Everything here is a
//!   pure function of the cells of the 3 x 3 columns around it, so a change
//!   reaches every chunk that shows it through the same one-cell padding as any
//!   other (`World::commit_levels` adds the layers above and below).
//! - A lattice corner's height depends only on the columns around it, so every
//!   column, chunk and cube face that shares it gets the same integer and
//!   neighbouring tops meet exactly (`corner`). Among powder it is their mean.
//!   Ground lower than the powder there holds it down to the ground, so a pile
//!   feathers out instead of ending in a kerb. Ground higher than all of it is
//!   a wall, and it and a drop past the window are left out.
//! - The middle of a column stands at the column's own height, so a top is
//!   four triangles around it. That keeps what a cell holds on show: a thin
//!   strip whose corners are all on the ground would otherwise vanish, and a
//!   lone partial cell is a little tent, not a flat tile. It also has no
//!   diagonal of its own, so the surface has no grain along either axis.
//! - A column is drawn this way (`Slope`) when its top is partial or touches a
//!   partial top: untouched sand stays cubes, and the first ring of it blends
//!   into a pile standing on it.
//! - The cells a slope cuts through are `floor..=top`. They leave the cube mesh,
//!   and read as air to the player's sweeps (`Soft` in player.rs): the top cell
//!   always, and the whole cells below it that the slope's low corners go under.
//!
//! - Water beside a slope goes on across its cells, for the eye only
//!   (`water_beside`).
//! - A partial cell in mid-air is on its way down: a flat tile, which the
//!   surface around it takes no notice of (`column_top`).
//!
//! Watertight, by cases. Two slope columns share their corners. A slope column
//! beside anything else draws a skirt from its surface down to `floor`, on any
//! side where the neighbour stands lower (hidden when that neighbour is itself
//! a slope, since it lies under its surface; drawing it anyway keeps the skirt
//! a function of the 3 x 3). Below `floor` are ordinary cubes with ordinary
//! faces. A neighbour standing higher is a solid whose side face the cube mesh
//! draws, because the slope's cells no longer hide it. A corner is raised to
//! the top of the column's own non-powder ground, so a surface never dives into
//! stone; if that parts two columns at a corner, the stone's side fills the gap.

use super::{BlockId, World};
use crate::grid::{CHUNK, ChunkKey};
use crate::mesh::SlopeQuad;
use crate::worldgen::{Generated, pidx};
use std::collections::BTreeMap;

/// How many layers above and below its reference a column is searched. The
/// steepest angle allowed is 2.7 layers per cell, so a corner lies within 3
/// layers of its column and 6 reach anything that can matter.
pub const WINDOW: i64 = 6;
/// A whole top blends with a partial neighbour within this height.
const BLEND: i32 = 2 * 255;
/// Units in a layer, as heights count them.
const LAYER: i32 = 255;

/// The drawn top of one column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slope {
    pub id: BlockId,
    /// The layer of the column's top powder cell.
    pub top: i64,
    /// The lowest layer the slope replaces: `floor..=top` leave the cube mesh.
    pub floor: i64,
    /// Heights of the four corners, absolute, in 1/255 of a layer, indexed
    /// cu + 2 cv as in the quad format.
    pub corners: [i32; 4],
    /// The column's own height, at the middle of the cell.
    pub centre: i32,
    /// The surface's gradient at the corners and last at the middle, in
    /// layers per cell over 32, for smooth shading: at a corner it comes from
    /// the columns around it, so the cells sharing a corner agree.
    pub slopes: [(i8, i8); 5],
    /// Which sides need a skirt: -i, +i, -j, +j. Not one beside a partial top
    /// (a slope, whose surface meets this one) or anything as high as the edge.
    pub open: [bool; 4],
}

impl Slope {
    /// The surface over fraction (u, v) of the cell, in layers: on the four
    /// triangles it is really drawn as, around the middle of the cell.
    pub fn height_at(&self, u: f64, v: f64) -> f64 {
        let [a, b, c, d] = self.corners.map(f64::from);
        let m = 2.0 * f64::from(self.centre);
        let (du, dv) = (u - 0.5, v - 0.5);
        let units = if du.abs() >= dv.abs() {
            if du < 0.0 { a + (c - a) * v + (m - a - c) * u } else { b + (d - b) * v + (m - b - d) * (1.0 - u) }
        } else if dv < 0.0 {
            a + (b - a) * u + (m - a - b) * v
        } else {
            c + (d - c) * u + (m - c - d) * (1.0 - v)
        };
        units / f64::from(LAYER)
    }
}

/// What a column shows near a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Top {
    /// Past two sides of a face: at a cube corner this lattice column does
    /// not exist.
    Missing,
    /// Solid all the way up the window.
    High,
    /// Nothing all the way down it.
    Low,
    Ground(i32),
    Powder { h: i32, partial: bool },
}

/// The padded grid's slope, for the mesher (`mesh_chunk_slopes`).
pub struct PaddedPowder {
    /// Padded cells (x, y, z) that leave the cube mesh.
    pub soft: Vec<(i8, i8, i8)>,
    /// Those of them water stands beside, and at what level (`water_beside`).
    pub wet: Vec<(i8, i8, i8, u8)>,
    pub water: BlockId,
    /// This chunk's quads: tops, skirts and undersides.
    pub quads: Vec<SlopeQuad>,
}

/// One corner's height, from the four columns around it (the module doc).
fn corner(cols: [Top; 4], own: i32) -> i32 {
    let high = cols.iter().filter_map(|c| if let Top::Powder { h, .. } = c { Some(*h) } else { None }).max().unwrap_or(own);
    let (mut sum, mut n, mut ground) = (0i64, 0i64, None);
    for c in cols {
        match c {
            Top::Powder { h, .. } => {
                sum += h as i64;
                n += 1;
            }
            Top::Ground(h) if h <= high => ground = ground.max(Some(h)),
            _ => {}
        }
    }
    match ground {
        Some(h) => h,
        None if n == 0 => own,
        None => (sum + n / 2).div_euclid(n) as i32,
    }
}

/// The gradient over the four columns around a corner, in layers per cell
/// over 32; a column that has no height there stands at the corner.
fn gradient(cols: [Top; 4], at: i32) -> (i8, i8) {
    let [a, b, c, d] = cols.map(|t| match t {
        Top::Powder { h, .. } | Top::Ground(h) => h,
        _ => at,
    });
    grad(a, b, c, d)
}

/// The gradient of four heights on a unit square, in layers per cell over 32.
fn grad(a: i32, b: i32, c: i32, d: i32) -> (i8, i8) {
    let scale = |x: i32| ((x as f64) * 32.0 / (2.0 * f64::from(LAYER))).round().clamp(-127.0, 127.0) as i8;
    (scale(b + d - a - c), scale(c + d - a - b))
}

impl World {
    /// A cell in `face`'s lattice, which may run past the face: bedrock below
    /// the world, air above it.
    #[inline]
    fn lattice_cell(&self, face: u8, ci: i64, cj: i64, ck: i64) -> (BlockId, u8) {
        if ck < 0 {
            return (self.generator.blocks.bedrock, 0);
        }
        match self.resolve_block(face, ci, cj, ck) {
            Some(bp) => self.block_and_level_at(bp),
            None => (self.generator.blocks.air, 0),
        }
    }

    /// What column (ci, cj) shows near layer `near`: from the first cell above
    /// that layer, up through what fills it, or down through what does not.
    fn column_top(&self, face: u8, ci: i64, cj: i64, near: i64) -> Top {
        let n = self.planet.n as i64;
        if !(0..n).contains(&ci) && !(0..n).contains(&cj) {
            return Top::Missing;
        }
        let reg = &self.registry;
        // A partial cell with nothing under it is not ground yet.
        let free = |k: i64| {
            let (id, level) = self.lattice_cell(face, ci, cj, k);
            reg.is_replaceable(id) || (level != 0 && reg.has_gravity(id) && reg.is_replaceable(self.lattice_cell(face, ci, cj, k - 1).0))
        };
        let mut k = near + 1;
        if free(k) {
            loop {
                k -= 1;
                if k < near - WINDOW {
                    return Top::Low;
                }
                if !free(k) {
                    break;
                }
            }
        } else {
            loop {
                if free(k + 1) {
                    break;
                }
                k += 1;
                if k > near + WINDOW {
                    return Top::High;
                }
            }
        }
        let (id, level) = self.lattice_cell(face, ci, cj, k);
        if reg.has_gravity(id) {
            let u = crate::sim::powder::units(level) as i32;
            Top::Powder { h: LAYER * k as i32 + u, partial: u != LAYER }
        } else {
            Top::Ground(LAYER * (k as i32 + 1))
        }
    }

    /// The slope of a column whose powder's top is cell (ci, cj, top), if it
    /// is drawn that way. All of `face`'s lattice, even past its sides: `wrap`
    /// puts the neighbouring face's columns where they belong, so two faces
    /// reach the same corner height from either side.
    pub fn slope_column(&self, face: u8, ci: i64, cj: i64, top: i64) -> Option<Slope> {
        let reg = &self.registry;
        let (id, level) = self.lattice_cell(face, ci, cj, top);
        if !reg.has_gravity(id) || !reg.is_replaceable(self.lattice_cell(face, ci, cj, top + 1).0) {
            return None;
        }
        let u = crate::sim::powder::units(level) as i32;
        let own = LAYER * top as i32 + u;
        if u != LAYER && reg.is_replaceable(self.lattice_cell(face, ci, cj, top - 1).0) {
            return Some(Slope { id, top, floor: top, corners: [own; 4], centre: own, slopes: [(0, 0); 5], open: [true; 4] });
        }
        let mut around = [Top::Missing; 9];
        let mut blends = u != LAYER;
        for dy in 0..3i64 {
            for dx in 0..3i64 {
                around[(dy * 3 + dx) as usize] = if dx == 1 && dy == 1 {
                    Top::Powder { h: own, partial: u != LAYER }
                } else {
                    let t = self.column_top(face, ci + dx - 1, cj + dy - 1, top);
                    blends |= matches!(t, Top::Powder { h, partial: true } if (h - own).abs() <= BLEND);
                    t
                };
            }
        }
        if !blends {
            return None;
        }
        // The non-powder ground under the stack: a corner stays on it.
        let mut low = top;
        while low > top - WINDOW && reg.has_gravity(self.lattice_cell(face, ci, cj, low - 1).0) {
            low -= 1;
        }
        let support = LAYER * low as i32;
        let cols = |c: usize| {
            let (cu, cv) = (c & 1, c >> 1);
            let at = |x: usize, y: usize| around[(cv + y) * 3 + cu + x];
            [at(0, 0), at(1, 0), at(0, 1), at(1, 1)]
        };
        let corners: [i32; 4] = std::array::from_fn(|c| corner(cols(c), own).max(support));
        let lowest = corners.into_iter().min().expect("four corners").min(own);
        // One further down when the surface only touches a layer's top, so no
        // cube's top lies in the surface.
        let floor = ((lowest - 1).div_euclid(LAYER) as i64).clamp(low, top);
        let at = |c: usize| gradient(cols(c), corners[c]);
        let slopes = [at(0), at(1), at(2), at(3), grad(corners[0], corners[1], corners[2], corners[3])];
        let open = |side: Top, a: usize, b: usize| {
            let edge = corners[a].max(corners[b]);
            match side {
                Top::Powder { partial: true, .. } | Top::High => false,
                Top::Powder { h, .. } | Top::Ground(h) => h < edge,
                Top::Low | Top::Missing => true,
            }
        };
        let open = [open(around[3], 0, 2), open(around[5], 1, 3), open(around[1], 0, 1), open(around[7], 2, 3)];
        Some(Slope { id, top, floor, corners, centre: own, slopes, open })
    }

    /// The slope that layer `k` of a column is in, or just over: for the
    /// player, who asks about cells and not about tops.
    pub fn slope_at(&self, face: u8, ci: i64, cj: i64, k: i64) -> Option<Slope> {
        if !self.edits.slopes_on() {
            return None;
        }
        let reg = &self.registry;
        let mut t = k + WINDOW;
        while t >= k - 3 {
            let (id, _) = self.lattice_cell(face, ci, cj, t);
            if reg.has_gravity(id) && reg.is_replaceable(self.lattice_cell(face, ci, cj, t + 1).0) {
                if let Some(s) = self.slope_column(face, ci, cj, t)
                    && s.floor <= k
                    && k <= s.top + 3
                {
                    return Some(s);
                }
                while reg.has_gravity(self.lattice_cell(face, ci, cj, t).0) {
                    t -= 1;
                }
                continue;
            }
            t -= 1;
        }
        None
    }

    /// The slope of a padded LOD-0 grid: None unless partial powder lies in
    /// reach (so a chunk near none costs one scan of its edited neighbours'
    /// levels, and the normal game one atomic load).
    pub fn padded_powder(&self, key: ChunkKey, g: &Generated) -> Option<PaddedPowder> {
        if key.lod != 0 || !self.edits.slopes_on() {
            return None;
        }
        let reg = self.registry.clone();
        let c = CHUNK as i64;
        let (i0, j0, k0) = key.origin_blocks();
        let (i0, j0, k0) = (i0 as i64, j0 as i64, k0 as i64);
        // Which columns can hold a slope: those within a column of a partial
        // cell. A chunk on another face can't be placed in this lattice cheaply,
        // and marks every column.
        let side = (c + 4) as usize;
        let mut near = vec![false; side * side];
        let mut any = false;
        for e in self.edited_near(key) {
            let (ei, ej, ek) = e.origin_blocks();
            let (ei, ej, ek) = (ei as i64, ej as i64, ek as i64);
            self.edits.with_edited(e, |ed| {
                let Some(w) = ed.water.as_ref() else { return };
                for (v, &l) in w.iter().enumerate() {
                    if l == 0 || l == 255 || !reg.has_gravity(ed.id(v)) {
                        continue;
                    }
                    any = true;
                    if e.face != key.face {
                        near.fill(true);
                        return;
                    }
                    let (x, y, z) = ((v / (CHUNK * CHUNK)) as i64, ((v / CHUNK) % CHUNK) as i64, (v % CHUNK) as i64);
                    let (x, y) = (ei + x - i0, ej + y - j0);
                    let k = ek + z;
                    if k < k0 - WINDOW || k > k0 + c + WINDOW {
                        continue;
                    }
                    for nx in x - 1..=x + 1 {
                        for ny in y - 1..=y + 1 {
                            if (-2..c + 2).contains(&nx) && (-2..c + 2).contains(&ny) {
                                near[(nx + 2) as usize * side + (ny + 2) as usize] = true;
                            }
                        }
                    }
                }
            });
        }
        if !any {
            return None;
        }
        let mut out = PaddedPowder { soft: Vec::new(), wet: Vec::new(), water: self.water_id(), quads: Vec::new() };
        for x in -1..=c {
            for y in -1..=c {
                if !near[(x + 2) as usize * side + (y + 2) as usize] {
                    continue;
                }
                let (ci, cj) = (i0 + x, j0 + y);
                let interior = (0..c).contains(&x) && (0..c).contains(&y);
                // The padded ids are the world, but for the skirt the generator
                // cut into a border column.
                let cell = |z: i64| {
                    if interior && (-1..=c).contains(&z) { g.ids[pidx(x as i32, y as i32, z as i32)] } else { self.lattice_cell(key.face, ci, cj, k0 + z).0 }
                };
                // A stack that reaches the top of the padded grid may go on
                // above it; one that does not has nothing to do with this chunk.
                let mut z = c - 1;
                if reg.has_gravity(cell(c)) {
                    z = c;
                    while z < c + WINDOW && reg.has_gravity(cell(z + 1)) {
                        z += 1;
                    }
                }
                while z >= -1 {
                    if !reg.has_gravity(cell(z)) {
                        z -= 1;
                        continue;
                    }
                    if reg.is_replaceable(cell(z + 1))
                        && let Some(s) = self.slope_column(key.face, ci, cj, k0 + z)
                    {
                        for k in s.floor..=s.top {
                            if (-1..=c).contains(&(k - k0)) {
                                out.soft.push((x as i8, y as i8, (k - k0) as i8));
                            }
                        }
                        if interior && (0..c).contains(&z) {
                            self.emit_slope(&s, key.face, ci, cj, (x as u32, y as u32, z as u32), k0, &mut out.quads);
                        }
                    }
                    while z >= -1 && reg.has_gravity(cell(z)) {
                        z -= 1;
                    }
                }
            }
        }
        out.wet = self.water_beside(key, &out.soft);
        Some(out)
    }

    /// A cell holds powder or water, not both, so to the rule water stops at
    /// partial powder. Drawn that way, a pool would end in a wall of water at
    /// each cell of a slope, with a hole in its surface over the pile. So for
    /// the eye alone the water beside a replaced cell goes on across it, through
    /// the replaced cells of that layer: the slope, being opaque and drawn
    /// first, then shows through or stands out of it, and where the two cross
    /// is the shoreline.
    fn water_beside(&self, key: ChunkKey, soft: &[(i8, i8, i8)]) -> Vec<(i8, i8, i8, u8)> {
        let (i0, j0, k0) = key.origin_blocks();
        let (i0, j0, k0) = (i0 as i64, j0 as i64, k0 as i64);
        let cells: std::collections::BTreeSet<(i8, i8, i8)> = soft.iter().copied().collect();
        let mut wet: BTreeMap<(i8, i8, i8), u8> = BTreeMap::new();
        let mut front: Vec<(i8, i8, i8)> = Vec::new();
        for &(x, y, z) in &cells {
            let mut level = 0;
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (id, l) = self.lattice_cell(key.face, i0 + x as i64 + dx, j0 + y as i64 + dy, k0 + z as i64);
                if self.registry.is_liquid(id) {
                    level = level.max(l);
                }
            }
            if level > 0 {
                wet.insert((x, y, z), level);
                front.push((x, y, z));
            }
        }
        while let Some((x, y, z)) = front.pop() {
            let level = wet[&(x, y, z)];
            for (dx, dy) in [(-1i8, 0i8), (1, 0), (0, -1), (0, 1)] {
                let n = (x + dx, y + dy, z);
                if cells.contains(&n) && wet.get(&n).is_none_or(|l| *l < level) {
                    wet.insert(n, level);
                    front.push(n);
                }
            }
        }
        wet.into_iter().map(|((x, y, z), l)| (x, y, z, l)).collect()
    }

    /// One column's quads (`SlopeQuad`). A top is two quads, each a fan of
    /// two triangles around the middle of the cell: vertex 1 is the middle, in
    /// the index pattern of render/mod.rs. The shader's `FAN` is these orders.
    fn emit_slope(&self, s: &Slope, face: u8, ci: i64, cj: i64, (x, y, z): (u32, u32, u32), k0: i64, out: &mut Vec<SlopeQuad>) {
        let hi = s.id as u32 | (0xFF << 16);
        let base = LAYER * k0 as i32;
        let rel = |h: i32| (h - base) as i16 as u16 as u32;
        let words = |h: [i32; 4]| [rel(h[0]) | (rel(h[1]) << 16), rel(h[2]) | (rel(h[3]) << 16)];
        let lean = |g: (i8, i8)| g.0 as u8 as u32 | ((g.1 as u8 as u32) << 8);
        let quad = |axis: u32, sign: u32, slice: u32, u: u32, v: u32, part: u32, h: [i32; 4], g: [(i8, i8); 4]| -> SlopeQuad {
            let [a, b] = words(h);
            let n = [lean(g[0]) | (lean(g[1]) << 16), lean(g[2]) | (lean(g[3]) << 16)];
            [axis | (sign << 2) | (slice << 3) | (u << 8) | (v << 13) | (part << 18), hi, a, b, n[0], n[1]]
        };
        let c = s.corners;
        let g = s.slopes;
        let flat = [(0, 0); 4];
        out.push(quad(2, 1, z, x, y, 0, [c[1], s.centre, c[0], c[2]], [g[1], g[4], g[0], g[2]]));
        out.push(quad(2, 1, z, x, y, 1, [c[2], s.centre, c[3], c[1]], [g[2], g[4], g[3], g[1]]));
        let bottom = LAYER * s.floor as i32;
        // The sides: axis, sign, and the two corners along it.
        for (side, (axis, sign, a, b)) in [(0u32, 0u32, c[0], c[2]), (0, 1, c[1], c[3]), (1, 0, c[0], c[1]), (1, 1, c[2], c[3])].into_iter().enumerate() {
            if !s.open[side] || a.max(b) <= bottom {
                continue;
            }
            let (slice, u) = if axis == 0 { (x, y) } else { (y, x) };
            out.push(quad(axis, sign, slice, u, 0, 0, [bottom, bottom, a, b], flat));
        }
        // A cell in mid-air, falling.
        if self.registry.is_replaceable(self.lattice_cell(face, ci, cj, s.floor - 1).0) {
            out.push(quad(2, 0, z, x, y, 0, [bottom; 4], flat));
        }
    }
}

#[cfg(test)]
#[path = "slope_tests.rs"]
mod tests;

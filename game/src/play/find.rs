//! `find flat|shore|river|lake|waterfall` (M2c): stand the player at a
//! suitable site near where they are, deterministically, so a script (or
//! a replay test) can go to "a river bank" without hand-copied coordinates.
//!
//! The search is a pure function of the player's column and the generator:
//! a coarse step finds a candidate area (the nearest river course or fall
//! from the hydrology, the nearest lake field cell, the nearest coastline
//! in the bake, or here for flat ground), then columns are tested nearest
//! first, in a fixed order, in square rings around it. A site is a dry
//! column with the wanted water one to three blocks from it along a lattice
//! axis (or, for flat, a 5 x 5 patch of dry ground at one height). The
//! player is put on it in walk mode, looking at the water (or along +i).

use crate::grid::{BlockPos, FastMap, Planet};
use crate::player::Mode;
use crate::session::{Action, Session};
use crate::worldgen::Generator;
use crate::worldgen::hydro::WaterKind;
use glam::DVec3;

/// A site `find` chose.
pub struct Site {
    pub kind: String,
    /// The top solid block the player stands on.
    pub stand: BlockPos,
    /// The cell looked at: the water surface cell, or the flat patch's centre.
    pub target: BlockPos,
    /// Lattice axis from the stand column to the target, and how many
    /// columns along it.
    pub axis: &'static str,
    pub gap: i64,
    /// Distance from where the player was (m) and columns sampled.
    pub dist_m: f64,
    pub columns: usize,
    /// Water drop at a fall (blocks), 0 elsewhere.
    pub drop: i64,
}

impl Site {
    pub fn describe(&self) -> String {
        let s = self.stand;
        let t = self.target;
        format!(
            "find {}: standing on face {} i {} j {} k {} ({:.0} m from where the player was; {} columns sampled), looking {} {} column(s) to face {} i {} j {} k {}{}",
            self.kind,
            s.face,
            s.i,
            s.j,
            s.k,
            self.dist_m,
            self.columns,
            self.axis,
            self.gap,
            t.face,
            t.i,
            t.j,
            t.k,
            if self.drop > 0 { format!(" (a fall of {} blocks beside it)", self.drop) } else { String::new() }
        )
    }

    /// State fields for asserts.
    pub fn fields(&self) -> serde_json::Value {
        serde_json::json!({
            "find_kind": self.kind,
            "find_face": self.stand.face, "find_i": self.stand.i, "find_j": self.stand.j, "find_k": self.stand.k,
            "find_target_face": self.target.face, "find_target_i": self.target.i, "find_target_j": self.target.j, "find_target_k": self.target.k,
            "find_axis": self.axis, "find_gap": self.gap, "find_dist_m": self.dist_m, "find_columns": self.columns, "find_drop": self.drop,
        })
    }
}

/// What a column holds, from the generator.
#[derive(Clone, Copy)]
struct Col {
    /// floor(ground height / block size): the ground layer above sea level.
    ground: i64,
    wet: bool,
    kind: WaterKind,
    /// floor(water surface / block size) when wet.
    water: i64,
}

/// Columns sampled once each, in the lattice of the face being searched
/// (`face`; coordinates past its sides wrap onto the neighbouring face).
struct Cols<'a> {
    g: &'a Generator,
    planet: Planet,
    face: u8,
    cache: FastMap<(u8, i64, i64), Col>,
}

impl<'a> Cols<'a> {
    fn get(&mut self, i: i64, j: i64) -> Col {
        if let Some(c) = self.cache.get(&(self.face, i, j)) {
            return *c;
        }
        let (f, wi, wj) = self.planet.wrap_cell(self.face, i, j);
        let dir = self.planet.dir(f, wi as f64 + 0.5, wj as f64 + 0.5);
        let c = self.g.column(dir, self.planet.block_size);
        let b = self.planet.block_size;
        let col = Col { ground: (c.height_m as f64 / b).floor() as i64, wet: c.wet(), kind: c.water_kind, water: if c.wet() { (c.water_m / b).floor() as i64 } else { i64::MIN } };
        self.cache.insert((self.face, i, j), col);
        col
    }
}

const AXES: [(&str, i64, i64); 4] = [("+i", 1, 0), ("-i", -1, 0), ("+j", 0, 1), ("-j", 0, -1)];

/// Offsets in square rings around the origin, nearest first, in a fixed order.
fn ring(r: i64) -> Vec<(i64, i64)> {
    if r == 0 {
        return vec![(0, 0)];
    }
    let mut v: Vec<(i64, i64)> = Vec::with_capacity(8 * r as usize);
    for d in -r..=r {
        v.push((d, -r));
        v.push((d, r));
    }
    for d in -r + 1..r {
        v.push((-r, d));
        v.push((r, d));
    }
    v.sort_by_key(|&(a, b)| (a * a + b * b, a, b));
    v
}

/// A site found at a stand column.
struct Hit {
    axis: &'static str,
    gap: i64,
    /// The water column looked at.
    target: (i64, i64),
    /// A fall: the drop (blocks) and the column below the lip.
    drop: i64,
    low: Option<(i64, i64)>,
}

/// Is (i, j) a site of this kind?
fn site_at(cols: &mut Cols, kind: &str, i: i64, j: i64) -> Option<Hit> {
    let here = cols.get(i, j);
    if here.wet {
        return None;
    }
    if kind == "flat" {
        for di in -2..=2 {
            for dj in -2..=2 {
                let c = cols.get(i + di, j + dj);
                if c.wet || c.ground != here.ground {
                    return None;
                }
            }
        }
        return Some(Hit { axis: "+i", gap: 0, target: (i, j), drop: 0, low: None });
    }
    let want = match kind {
        "shore" => WaterKind::Ocean,
        "lake" => WaterKind::Lake,
        _ => WaterKind::River,
    };
    for (axis, ai, aj) in AXES {
        for gap in 1..=3 {
            let (ti, tj) = (i + ai * gap, j + aj * gap);
            let t = cols.get(ti, tj);
            if !t.wet {
                continue;
            }
            if t.kind != want || (here.ground - t.water).abs() > 4 {
                break;
            }
            if kind == "waterfall" {
                // A river cell with a neighbouring river cell at least three
                // blocks lower: the lip of a fall.
                let mut best: Option<(i64, (i64, i64))> = None;
                for &(_, bi, bj) in &AXES {
                    let n = cols.get(ti + bi, tj + bj);
                    if n.wet && n.kind == WaterKind::River && best.is_none_or(|b| t.water - n.water > b.0) {
                        best = Some((t.water - n.water, (ti + bi, tj + bj)));
                    }
                }
                match best {
                    Some((drop, low)) if drop >= 3 => return Some(Hit { axis, gap, target: (ti, tj), drop, low: Some(low) }),
                    _ => break,
                }
            }
            return Some(Hit { axis, gap, target: (ti, tj), drop: 0, low: None });
        }
    }
    None
}

/// Search rings 0..=rmax around (ci, cj) for a site: (stand column, hit).
fn search(cols: &mut Cols, kind: &str, ci: i64, cj: i64, rmax: i64) -> Option<((i64, i64), Hit)> {
    for r in 0..=rmax {
        for (di, dj) in ring(r) {
            let (i, j) = (ci + di, cj + dj);
            if let Some(hit) = site_at(cols, kind, i, j) {
                return Some(((i, j), hit));
            }
        }
    }
    None
}

/// Candidate areas for a kind, nearest first: (face, i, j) columns to
/// search around, each in its own face's lattice.
fn seeds(s: &Session, kind: &str, here: DVec3) -> Result<Vec<(u8, i64, i64)>, String> {
    let planet = s.engine.planet;
    let g = &s.engine.generator;
    let to_lattice = |d: DVec3| -> (u8, i64, i64) {
        let (f, i, j) = planet.lattice_of_dir(d);
        (f, i.floor() as i64, j.floor() as i64)
    };
    match kind {
        "flat" => Ok(vec![to_lattice(here)]),
        "river" | "waterfall" | "lake" => {
            let hy = g.hydro.as_ref().ok_or("this bake has no hydro.bin (no lakes or rivers)")?;
            let falls = kind == "waterfall";
            for r in [300.0, 1200.0, 5000.0, 20000.0, 80000.0] {
                let c = hy.cands(here, r);
                let mut best: Vec<(f64, DVec3)> = Vec::new();
                if kind == "lake" {
                    for &k in &c.cells {
                        let cell = &hy.field[k as usize];
                        if cell.lake >= 0 {
                            best.push(((cell.pos - here).length() * planet.radius, cell.pos));
                        }
                    }
                } else {
                    for &k in &c.segs {
                        let seg = &hy.segs[k as usize];
                        if falls {
                            let p = hy.profile(k, planet.block_size);
                            if !p.falls || p.steps == 0 {
                                continue;
                            }
                            // Where the surface drops by a block or more within a metre.
                            let mut s0 = 0.0;
                            while s0 + 1.0 < seg.len {
                                let a = hy.w_water(k, &p, s0);
                                let b = hy.w_water(k, &p, s0 + 1.0);
                                if a - b >= planet.block_size {
                                    let at = seg.point(s0 + 0.5);
                                    best.push(((at - here).length() * planet.radius, at));
                                }
                                s0 += 1.0;
                            }
                        } else {
                            for &at in &seg.pts {
                                best.push(((at - here).length() * planet.radius, at));
                            }
                        }
                    }
                }
                if best.is_empty() {
                    continue;
                }
                best.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.x.total_cmp(&b.1.x)));
                if kind == "lake" {
                    // A lake cell can be deep inside a big lake: walk from it
                    // toward the player, 8 m at a time, to where the lake ends.
                    for e in best.iter_mut().take(24) {
                        let (from, to) = (e.1, here);
                        let dist = e.0;
                        let mut t = 0.0;
                        while t < dist {
                            let p = (from * (1.0 - t / dist) + to * (t / dist)).normalize();
                            let c = g.column(p, planet.block_size);
                            if !(c.wet() && c.water_kind == WaterKind::Lake) {
                                e.1 = p;
                                break;
                            }
                            t += 8.0;
                        }
                    }
                }
                let mut out: Vec<(u8, i64, i64)> = Vec::new();
                for (_, d) in best {
                    let c = to_lattice(d);
                    if out.iter().all(|o| o.0 != c.0 || (o.1 - c.1).abs() > 48 || (o.2 - c.2).abs() > 48) {
                        out.push(c);
                    }
                    if out.len() >= 6 {
                        break;
                    }
                }
                return Ok(out);
            }
            Err(format!("no {kind} within 80 km"))
        }
        "shore" => {
            // The coastline in the bake: the first sample, ring by ring at a
            // 16-block stride, where land meets the sea mask.
            let (face, hi, hj) = to_lattice(here);
            // Sea in the bake: the sea's side of the coastline.
            let sea = |i: i64, j: i64| -> bool {
                let (f, wi, wj) = planet.wrap_cell(face, i, j);
                s.engine.bake.sample(planet.dir(f, wi as f64 + 0.5, wj as f64 + 0.5)).coast > 0.0
            };
            let stride = 16;
            let mut out = Vec::new();
            for r in 0..=400i64 {
                for (di, dj) in ring(r) {
                    let (i, j) = (hi + di * stride, hj + dj * stride);
                    if sea(i, j) {
                        continue;
                    }
                    // Land here: is there sea a stride away?
                    if AXES.iter().any(|&(_, ai, aj)| sea(i + ai * stride, j + aj * stride)) {
                        out.push((face, i, j));
                        if out.len() >= 4 {
                            return Ok(out);
                        }
                    }
                }
            }
            if out.is_empty() { Err("no coastline within 6 km".into()) } else { Ok(out) }
        }
        _ => Err(format!("find {kind}: the kinds are flat, shore, river, lake and waterfall")),
    }
}

/// Find a site and stand the player there.
pub fn find(s: &mut Session, kind: &str) -> Result<Site, String> {
    let kind = kind.to_lowercase();
    let planet = s.engine.planet;
    let here = s.player.pos.normalize();
    let seeds = seeds(s, &kind, here)?;
    let engine = s.engine.clone();
    let mut cols = Cols { g: &engine.generator, planet, face: 0, cache: FastMap::default() };
    let rmax = if kind == "flat" { 160 } else { 64 };
    let mut found = None;
    for (face, ci, cj) in seeds {
        cols.face = face;
        if let Some(hit) = search(&mut cols, &kind, ci, cj, rmax) {
            found = Some(hit);
            break;
        }
    }
    let columns = cols.cache.len();
    let face = cols.face;
    let Some(((i, j), Hit { axis, gap, target: (ti, tj), drop, low })) = found else {
        return Err(format!("no {kind} site found near here ({columns} columns sampled)"));
    };
    let world = engine.world.as_ref();
    let reg = &engine.registry;
    // The top solid block of a column, from the world (edits included).
    let top = |i: i64, j: i64, hint: i64| -> BlockPos {
        let (f, wi, wj) = planet.wrap_cell(face, i, j);
        let mut k = (planet.sea_level as i64 + hint + 6).clamp(0, planet.height as i64 - 1);
        while k > 0 && !reg.is_solid(world.block_at(BlockPos { face: f, i: wi, j: wj, k: k as u32 })) {
            k -= 1;
        }
        BlockPos { face: f, i: wi, j: wj, k: k as u32 }
    };
    let stand = top(i, j, cols.get(i, j).ground);
    let target = if kind == "flat" {
        stand
    } else {
        let t = cols.get(ti, tj);
        let (f, wi, wj) = planet.wrap_cell(face, ti, tj);
        BlockPos { face: f, i: wi, j: wj, k: (planet.sea_level as i64 + t.water).clamp(0, planet.height as i64 - 1) as u32 }
    };
    let _ = s.apply(Action::SetMode(Mode::Walk));
    s.teleport_block(stand);
    // Look at the target cell's top centre (flat: along +i, level; a
    // fall: down at the water below its lip).
    let eye = s.player.eye();
    let (north, east, up) = crate::camera::Camera::local_frame(eye);
    let aim = if kind == "flat" {
        let (f, wi, wj) = planet.wrap_cell(face, i + 4, j);
        planet.pos(f, wi as f64 + 0.5, wj as f64 + 0.5, stand.k as f64 + 1.0 + crate::player::EYE_M / planet.block_size)
    } else if let Some((li, lj)) = low {
        let l = cols.get(li, lj);
        let (f, wi, wj) = planet.wrap_cell(face, li, lj);
        planet.pos(f, wi as f64 + 0.5, wj as f64 + 0.5, (planet.sea_level as i64 + l.water) as f64 + 1.0)
    } else {
        planet.pos(target.face, target.i as f64 + 0.5, target.j as f64 + 0.5, target.k as f64 + 1.0)
    };
    let d = (aim - eye).normalize();
    s.player.heading = d.dot(east).atan2(d.dot(north));
    s.player.pitch = d.dot(up).clamp(-1.0, 1.0).asin().clamp(-1.55, 1.55);
    let dist_m = (s.player.pos.normalize() - here).length() * planet.radius;
    Ok(Site { kind, stand, target, axis, gap, dist_m, columns, drop })
}

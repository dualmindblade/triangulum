//! Powder that holds a level (B7, Andrew's #8: "Experiment with alternate
//! powder mechanics similar to water levels but with an angle of
//! inclination"). Off by default: `--powder levels`, `sim powder levels`. The
//! rule in force without it is the block slide in `sim/mod.rs`, and this file
//! is never reached.
//!
//! **What a cell holds.** One byte, the same byte water uses (it costs no new
//! layer, and saves, hashes and replays carry it already). For a powder cell it
//! says how full the cell is: 1 to 254 units of 255. A stored 0 is a whole
//! block, because that is what every sand cell the generator and the old rules
//! make holds, so generated sand is whole until something disturbs it and needs
//! no converting. Units here are always 0..=255; `units` and `Cells::put`
//! translate.
//!
//! **The height of a column.** A top cell at layer k holding L units stands at
//! H = 255 k + L. Each tick, every top cell near a change compares itself with
//! its eight neighbouring columns (the surface of each: down through free cells
//! to the first thing that holds powder up, as far as the slope can matter).
//! Its excess E = D - S on a way is how far the drop D there exceeds what the
//! slope allows: S = 255 tan(angle) along an axis, S * sqrt(2) along a diagonal,
//! because a diagonal neighbour is sqrt(2) cells away. It wants to send
//! ceil(E / 2) units down every way with an excess, so each pair ends about at
//! the slope and never past it; if that is more than it holds, each way gets
//! its share. Going one way only (the steepest) was my first version, and the
//! heaps came out as four-armed stars: a cell that gives everything to one
//! neighbour leaves a lobe, and nothing evens a lobe out, since a rough surface
//! within the slope is at rest. Steepest first, ties by `choice`; each share is
//! checked against the cell as the shares before it left it. Over a drop a cell
//! gives at most `POUR` a tick, half a block, so a tower pours rather than
//! sheds all of itself at once.
//!
//! Why eight ways and not four: four would make a pile a diamond, steeper down
//! its corners than down its faces (B4 measured 55 degrees against 45 for the
//! block rule). Eight make it close to a cone. A diagonal move needs one of the
//! two cells it passes between to be open, so powder does not go through the
//! corner between two walls.
//!
//! Where the units go:
//!   - a neighbour on this layer that is partial, the same powder: into it;
//!   - a free cell with a floor under it (whole, solid or another powder):
//!     into it, resting on that floor;
//!   - a free cell over a partial cell of the same powder: into that cell;
//!   - a free cell over another free cell: straight into the lower free cell,
//!     from where it falls. That is the slide of `water.rs`, and it is one
//!     move for the same reason: a sideways step of a cell's last unit would
//!     leave the potential where it was.
//!
//! Anything with room below falls first, and does not spread. A partial cell
//! falls by levels: `FALL` cells a tick through air, one through water. (At
//! one cell a tick what a tower sheds stays level with what it sheds next,
//! and consecutive partial cells draw as a stack, a solid sleeve sliding down
//! the tower. Two leaves air between them.) A whole block over a free cell is
//! the `Falling` entity it always was (`fall_rule`), which lands whole.
//! A whole block landing, or placed, on a partial cell needs nothing special:
//! it sits on it for one tick, then fills it from above (a fall by levels), and
//! what is left over stays in the cell it landed in. Falling powder that meets
//! water pushes the whole cell's water up its column, through any powder above
//! (`World::displace_writes` does the same for a landing block), and partial
//! powder is a wall and a floor to water: one byte cannot hold both.
//!
//! Two powders do not mix. A partial cell of another powder is a wall on its
//! layer and a floor above: a cell holds one id, and the alternatives were
//! turning sand into gravel or inventing a ratio.
//!
//! **A generated slope holds.** A cell still exactly as the generator made it
//! keeps the block rule's limit (one block per block; two along a diagonal) if
//! that is steeper than the angle. Without this, one dig on a beach would set
//! every natural slope within the active region flowing down to the angle,
//! which is not what "stands until touched" means. Everything disturbed, and
//! everything the player placed, follows the angle. So a pit in a real beach
//! keeps natural walls up to 45 degrees, with loose sand at the angle on them.
//!
//! **Why it comes to rest.** The potential is the same shape as water's, with
//! the true weight of a layer in units:
//!   Phi = sum over powder cells of (510 k + L) L
//! which adds up twice the height of every unit, less one. Taking d units from
//! a cell standing at Ha and putting them in a cell standing at Hb changes it by
//!   2 d (Hb - Ha + d)
//! so it falls strictly exactly when d < Ha - Hb. Every move here meets that:
//!   - sideways or onto a partial cell: Ha - Hb = D > S >= 1 and
//!     d <= ceil((D - S) / 2) < D, with D as this share finds it;
//!   - into a free cell on a floor, on this layer: the same, with D = L;
//!   - into a free cell below this layer: Ha - Hb >= 255 + L > d, since d <= L;
//!   - a fall by levels into a free cell: Ha - Hb >= 255 + L > d;
//!   - a fall by levels into a partial cell: Ha - Hb = 255 + L - M, and
//!     d <= min(L, 255 - M), so d = 255 + L - M would need M = 255, a full cell.
//! A `Falling` block only goes down. Phi is a non-negative integer, so only
//! finitely many moves can happen: no cycle, no creep. That S is at least 1 is
//! why the angle has a floor: with S = 0 a difference of 1 would swap for ever.
//! `powder_potential` and the tests check this tick by tick. The natural limit
//! only ever forbids moves, so it changes nothing here.
//!
//! **Determinism.** The moves of a tick are made one after another, each seeing
//! the ones before it, in the order of a hash of the cell and the tick
//! (`choice`), so neither the lattice's axes nor the order the queue held them
//! in decide anything. Nothing iterates a hash map except to collect what is
//! then sorted. All of a tick's moves go into the world as one batch, before
//! the water rule runs, so water sees where powder went.
//!
//! **The active region.** Nothing outside it is written. A cell whose way lies
//! through a chunk outside waits under that chunk (`Sim::wait_for`) as the block
//! slide's does, at no cost, and goes when the region arrives.

use super::{PosKey, Sim, choice};
use crate::grid::{BlockPos, ChunkKey, FastMap, FastSet};
use crate::world::{BlockId, EditStore, Registry, World};

/// Units in a whole cell.
pub const WHOLE: u32 = 255;
/// Weight of the layer in `powder_potential`.
pub const POWDER_LAYER_WEIGHT: u64 = 2 * WHOLE as u64;
/// The least a free cell on a floor is given: 3 cm. Without it a pile's foot
/// ends in tiles of dust, thin water's problem over again. So the last cell of
/// a toe stands a few units past the slope instead. It only forbids moves.
pub const LEAST_SPREAD: u32 = 8;
/// The most a cell gives in a tick when any of it goes over a drop.
pub const POUR: u32 = 128;
/// Cells a tick a partial cell falls through air.
const FALL: u32 = 2;
/// How far a change is looked for up a stack of powder, in layers.
const CLIMB: u32 = 64;
/// How far down an empty column is searched for the top of its powder.
const DESCEND: u32 = 4;
/// The eight ways, in `Way` order: the four sides, then the diagonals.
const WAYS: [(i64, i64); 8] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)];

/// How powder behaves. Part of `sim::Flow`, so saved, hashed and replayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Powder {
    /// Whole blocks that slide off ledges (B4): the game.
    Blocks,
    /// Cells that hold a level and rest at an angle (this file).
    Levels(Repose),
}

/// The angle of repose, and the height differences it allows, in units.
/// Integers from here on, so a world's future is a function of what is saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repose {
    tenths: u16,
    /// Most a column may stand above a neighbour along an axis.
    pub straight: u32,
    /// And along a diagonal: `straight` * sqrt(2).
    pub diagonal: u32,
}

impl Repose {
    /// Dry sand is 30 to 35 degrees.
    pub const DEFAULT_DEG: f64 = 35.0;
    /// tan(1 degree) * 255 is 4, so S is well above the 1 the proof needs.
    pub const MIN_DEG: f64 = 1.0;
    /// The mesher's window (`world/slope.rs`) reaches 2.7 layers per cell here.
    pub const MAX_DEG: f64 = 70.0;

    pub fn of_degrees(deg: f64) -> Result<Self, String> {
        if !deg.is_finite() || !(Self::MIN_DEG..=Self::MAX_DEG).contains(&deg) {
            return Err(format!("bad powder angle {deg}: {} to {} degrees", Self::MIN_DEG, Self::MAX_DEG));
        }
        let tenths = (deg * 10.0).round() as u16;
        let slope = (f64::from(tenths) / 10.0).to_radians().tan() * f64::from(WHOLE);
        Ok(Self { tenths, straight: slope.round() as u32, diagonal: (slope * std::f64::consts::SQRT_2).round() as u32 })
    }

    pub fn degrees(self) -> f64 {
        f64::from(self.tenths) / 10.0
    }
}

impl Default for Repose {
    fn default() -> Self {
        Self::of_degrees(Self::DEFAULT_DEG).expect("the default angle is in range")
    }
}

/// The units in a powder cell, from its stored level (0 is a whole block).
#[inline]
pub fn units(level: u8) -> u32 {
    if level == 0 { WHOLE } else { u32::from(level) }
}

/// One way a top cell can send powder.
struct Way {
    /// Its excess, then a hash for ties, highest first.
    rank: (i64, u64),
    to: BlockPos,
    surface: i64,
    /// The drop it stands at: the slope, or a natural cell's limit.
    hold: i64,
    /// It goes into a free cell over another free cell, and falls from there.
    over: bool,
}

/// A tick's writes and reads through them.
struct Cells<'a> {
    world: &'a World,
    writes: FastMap<PosKey, (BlockId, u8)>,
    /// Cells powder came into this tick: they do not pass it on until the
    /// next, so powder moves one cell sideways per tick from any source.
    received: FastSet<PosKey>,
    /// The cells blocks in flight are in. Powder does not move into one, and
    /// a whole cell does not start to fall into one: a block that found its
    /// cell taken would settle above whatever stands over it (`advance_falling`),
    /// which is a rise, and the proof has none. What waits tries again next
    /// tick; the block will have gone.
    flying: FastSet<PosKey>,
}

impl Cells<'_> {
    #[inline]
    fn get(&self, p: BlockPos) -> (BlockId, u8) {
        match self.writes.get(&PosKey::of(p)) {
            Some(&c) => c,
            None => self.world.block_and_level_at(p),
        }
    }

    #[inline]
    fn touched(&self, p: BlockPos) -> bool {
        self.writes.contains_key(&PosKey::of(p))
    }

    /// Write a cell of powder: 0 units leave air, 255 a whole block.
    fn put(&mut self, p: BlockPos, id: BlockId, u: u32) {
        let c = match u {
            0 => (self.world.generator.blocks.air, 0),
            WHOLE => (id, 0),
            _ => (id, u as u8),
        };
        self.writes.insert(PosKey::of(p), c);
    }
}

impl Sim {
    /// One tick of the rule: `gravity` is this tick's powder cells, ascending,
    /// `others` the air and water cells that were notified. Both say where
    /// things changed; what moves is the top of the powder there.
    pub(super) fn powder_levels(&mut self, world: &World, gravity: &[BlockPos], others: &[(u64, PosKey)], rule: Repose) {
        world.edits.set_slopes(true);
        let reg = world.registry.clone();
        let flying = self.falling.iter().map(|f| PosKey::of(BlockPos { face: f.face, i: f.i, j: f.j, k: f.cell().max(0) as u32 })).collect();
        let mut c = Cells { world, writes: FastMap::default(), received: FastSet::default(), flying };
        for &p in gravity {
            self.powder_fall(&mut c, &reg, p);
        }
        // The tops near a change: a column's own, and those of the eight
        // around it, which heard nothing (a side neighbour hears, a diagonal
        // does not, and neither does the top of a stack cut from below).
        let mut seen: FastSet<PosKey> = FastSet::default();
        let mut tops: Vec<PosKey> = Vec::new();
        let seeds = gravity.iter().copied().chain(others.iter().map(|(_, key)| key.pos()));
        for p in seeds {
            let (i, j, k) = (p.i as i64, p.j as i64, p.k as i64);
            for (di, dj) in [(0, 0)].into_iter().chain(WAYS) {
                let Some(q) = way_to(world, p.face, i + di, j + dj, k) else { continue };
                if seen.insert(PosKey::of(q))
                    && let Some(top) = surface_of(&c, &reg, q, &mut seen)
                {
                    tops.push(PosKey::of(top));
                }
            }
        }
        tops.sort_unstable();
        tops.dedup();
        let tick = self.tick;
        let mut order: Vec<(u64, PosKey)> = tops.into_iter().map(|key| (choice(key, key, tick), key)).collect();
        order.sort_unstable();
        for (_, key) in order {
            let p = key.pos();
            if self.is_active_block(p) {
                self.powder_step(&mut c, &reg, p, rule);
            } else {
                self.wait_for(World::chunk_of(p).0, key);
            }
        }
        if c.writes.is_empty() {
            return;
        }
        let mut writes: Vec<(PosKey, (BlockId, u8))> = c.writes.into_iter().collect();
        writes.sort_unstable_by_key(|w| w.0);
        let batch: Vec<(BlockPos, BlockId, Option<u8>)> = writes.iter().map(|(k, (id, l))| (k.pos(), *id, Some(*l))).collect();
        world.commit_levels(&batch);
        self.dirty = true;
    }

    /// Down first (the module doc). `p` is a due cell, anywhere in a stack.
    fn powder_fall(&mut self, c: &mut Cells, reg: &Registry, p: BlockPos) {
        let (id, level) = c.get(p);
        if !reg.has_gravity(id) || p.k == 0 {
            return;
        }
        let below = BlockPos { k: p.k - 1, ..p };
        let (bid, bl) = c.get(below);
        if !self.is_active_block(below) {
            if reg.is_replaceable(bid) || (bid == id && bl != 0) {
                self.wait_for(World::chunk_of(below).0, PosKey::of(p));
            }
            return;
        }
        let u = units(level);
        if reg.is_replaceable(bid) {
            if c.flying.contains(&PosKey::of(below)) {
                self.queue.insert((self.tick + 1, PosKey::of(p)));
                self.dirty = true;
                return;
            }
            if u == WHOLE && !c.touched(p) && !c.touched(below) {
                self.fall_rule(c.world, p);
                c.flying.insert(PosKey::of(p));
                return;
            }
            // On through air, while nothing is in the way.
            let mut to = below;
            for _ in 1..FALL {
                if to.k == 0 || c.get(to).1 != 0 {
                    break;
                }
                let next = BlockPos { k: to.k - 1, ..to };
                let (nid, nl) = c.get(next);
                if !reg.is_replaceable(nid) || nl != 0 || !self.is_active_block(next) || c.flying.contains(&PosKey::of(next)) {
                    break;
                }
                to = next;
            }
            c.put(p, id, 0);
            if self.powder_enter(c, reg, to, id, u) {
                self.stats.powder_moved += u as u64;
            } else {
                c.put(p, id, u);
            }
        } else if bid == id && bl != 0 {
            let d = u.min(WHOLE - u32::from(bl));
            c.put(below, id, u32::from(bl) + d);
            c.put(p, id, u - d);
            self.stats.powder_moved += d as u64;
        }
    }

    /// One top cell's move: where it goes and how much (the module doc).
    fn powder_step(&mut self, c: &mut Cells, reg: &Registry, p: BlockPos, rule: Repose) {
        let key = PosKey::of(p);
        let (id, level) = c.get(p);
        if !reg.has_gravity(id) || c.received.contains(&key) {
            return;
        }
        // It may have stopped being a top, or have room below, since it was found.
        if p.k + 1 < c.world.planet.height && reg.has_gravity(c.get(BlockPos { k: p.k + 1, ..p }).0) {
            return;
        }
        if p.k > 0 {
            let (bid, bl) = c.get(BlockPos { k: p.k - 1, ..p });
            if reg.is_replaceable(bid) || (bid == id && bl != 0) {
                return;
            }
        }
        let u = units(level);
        let k = p.k as i64;
        let here = 255 * k + u as i64;
        let natural = u == WHOLE && c.world.natural_cell(p) == (id, 0);
        let (i, j) = (p.i as i64, p.j as i64);
        let beside = WAYS.map(|(di, dj)| way_to(c.world, p.face, i + di, j + dj, k));
        let mut ways: Vec<Way> = Vec::with_capacity(8);
        let mut outside: Vec<ChunkKey> = Vec::new();
        let mut busy = false;
        for (way, q) in beside.iter().enumerate() {
            let Some(q) = *q else { continue };
            let diagonal = way >= 4;
            if diagonal {
                let (di, dj) = WAYS[way];
                let open = |side: Option<BlockPos>| {
                    side.is_some_and(|s| {
                        let (sid, sl) = c.get(s);
                        reg.is_replaceable(sid) || (sid == id && sl != 0)
                    })
                };
                if !open(beside[usize::from(di > 0)]) && !open(beside[2 + usize::from(dj > 0)]) {
                    continue;
                }
            }
            if !self.is_active_block(q) {
                outside.push(World::chunk_of(q).0);
                continue;
            }
            let limit = i64::from(if diagonal { rule.diagonal } else { rule.straight });
            let Some((to, surface, over)) = self.powder_way(c, reg, q, id, limit, &mut outside) else { continue };
            let hold = if natural { limit.max(if diagonal { 510 } else { 255 }) } else { limit };
            let excess = here - surface - hold;
            if excess <= 0 {
                continue;
            }
            if c.flying.contains(&PosKey::of(to)) {
                busy = true;
                continue;
            }
            ways.push(Way { rank: (excess, u64::MAX - choice(key, PosKey::of(to), self.tick)), to, surface, hold, over });
        }
        if ways.is_empty() {
            outside.sort_unstable();
            outside.dedup();
            for chunk in outside {
                self.wait_for(chunk, key);
            }
            if busy {
                self.queue.insert((self.tick + 1, key));
                self.dirty = true;
            }
            return;
        }
        ways.sort_unstable_by(|a, b| b.rank.cmp(&a.rank));
        let wanted = |e: i64| (e.min(2 * WHOLE as i64) as u32).div_ceil(2);
        let total: u32 = ways.iter().map(|w| wanted(w.rank.0)).sum();
        let can = if ways.iter().any(|w| w.over) { u.min(POUR) } else { u };
        let mut left = u;
        for way in ways {
            let share = if total <= can { wanted(way.rank.0) } else { (wanted(way.rank.0) * can / total).max(1) };
            // As the shares before this one left the cell.
            let excess = 255 * k + left as i64 - way.surface - way.hold;
            if excess <= 0 {
                continue;
            }
            let (tid, tl) = c.get(way.to);
            let room = if reg.is_replaceable(tid) { WHOLE } else { WHOLE - u32::from(tl) };
            let d = share.min(wanted(excess)).min(left).min(room);
            if d == 0 || (d < LEAST_SPREAD && way.to.k == p.k && reg.is_replaceable(tid)) {
                continue;
            }
            c.put(p, id, left - d);
            if reg.is_replaceable(tid) {
                if !self.powder_enter(c, reg, way.to, id, d) {
                    c.put(p, id, left);
                    continue;
                }
            } else {
                c.put(way.to, id, u32::from(tl) + d);
            }
            left -= d;
            c.received.insert(PosKey::of(way.to));
            self.stats.powder_moves += 1;
            self.stats.powder_moved += d as u64;
            if left == 0 {
                break;
            }
        }
    }

    /// Where powder going through cell `q` beside it ends up: the cell its
    /// units go into, the height of the surface there, and whether it goes
    /// over a drop. None if that way is shut; `limit` says how far down can
    /// matter.
    fn powder_way(&self, c: &Cells, reg: &Registry, q: BlockPos, id: BlockId, limit: i64, outside: &mut Vec<ChunkKey>) -> Option<(BlockPos, i64, bool)> {
        let (qid, ql) = c.get(q);
        let k = q.k as i64;
        if reg.has_gravity(qid) {
            // A partial cell of this powder takes units; anything else here
            // stands at least as high as this cell can.
            return (qid == id && ql != 0).then(|| (q, 255 * k + i64::from(ql), false));
        }
        if !reg.is_replaceable(qid) {
            return None;
        }
        if q.k == 0 {
            return Some((q, 0, false));
        }
        let under = BlockPos { k: q.k - 1, ..q };
        if !self.is_active_block(under) {
            outside.push(World::chunk_of(under).0);
            return None;
        }
        let (uid, ul) = c.get(under);
        if !reg.is_replaceable(uid) {
            // The floor is right here.
            return Some(if uid == id && ul != 0 { (under, 255 * (k - 1) + i64::from(ul), false) } else { (q, 255 * k, false) });
        }
        // A free cell over a free cell: the units go into the lower one and
        // fall from there. How far they go on falling decides whether they go.
        let mut m = k - 2;
        let stop = (k - limit / 255 - 3).max(-1);
        while m > stop {
            let b = BlockPos { k: m as u32, ..q };
            if !self.is_active_block(b) {
                outside.push(World::chunk_of(b).0);
                return None;
            }
            let (bid, bl) = c.get(b);
            if !reg.is_replaceable(bid) {
                let surface = if bid == id && bl != 0 { 255 * m + i64::from(bl) } else { 255 * (m + 1) };
                return Some((under, surface, true));
            }
            m -= 1;
        }
        Some((under, if m < 0 { 0 } else { 255 * (m + 1) - 1 }, true))
    }

    /// `d` units of powder take a free cell (the source is already written).
    /// Water there goes up its column, as under a landing block. False, and
    /// nothing changed, if it would have to rise out of the active region.
    fn powder_enter(&mut self, c: &mut Cells, reg: &Registry, to: BlockPos, id: BlockId, d: u32) -> bool {
        let (_, water) = c.get(to);
        let mut rest = u32::from(water);
        if rest > 0 {
            let wet = c.world.water_id();
            let mut lifted: Vec<(BlockPos, u8)> = Vec::new();
            for k in to.k + 1..c.world.planet.height {
                let up = BlockPos { k, ..to };
                let (uid, ul) = c.get(up);
                if reg.has_gravity(uid) {
                    continue;
                }
                if !reg.is_replaceable(uid) {
                    break;
                }
                let cap = WHOLE - u32::from(ul);
                if cap == 0 {
                    continue;
                }
                if !self.is_active_block(up) {
                    return false;
                }
                let m = rest.min(cap);
                lifted.push((up, ul + m as u8));
                rest -= m;
                if rest == 0 {
                    break;
                }
            }
            for (up, l) in lifted {
                c.writes.insert(PosKey::of(up), (wet, l));
            }
            self.stats.water_displaced += u64::from(water) - rest as u64;
            self.stats.water_lost += rest as u64;
        }
        c.put(to, id, d);
        true
    }
}

/// The cell at these coordinates of `face`'s lattice, or None: outside the
/// world's height, or past two sides of a face, where a cube corner has no cell.
#[inline]
fn way_to(world: &World, face: u8, i: i64, j: i64, k: i64) -> Option<BlockPos> {
    let n = world.planet.n as i64;
    if !(0..n).contains(&i) && !(0..n).contains(&j) {
        return None;
    }
    world.resolve_block(face, i, j, k)
}

/// The top of the powder a due cell belongs to: up a stack from inside it,
/// down an open column from above it. The cells it passes on the way up have
/// the same top, so they go in `seen` (a collapsing block of N columns h high
/// would otherwise climb h * h cells a column, on every tick).
fn surface_of(c: &Cells, reg: &Registry, p: BlockPos, seen: &mut FastSet<PosKey>) -> Option<BlockPos> {
    let (id, _) = c.get(p);
    let height = c.world.planet.height;
    if reg.has_gravity(id) {
        let mut top = p;
        for _ in 0..CLIMB {
            if top.k + 1 >= height {
                break;
            }
            let up = BlockPos { k: top.k + 1, ..top };
            if !reg.has_gravity(c.get(up).0) {
                break;
            }
            top = up;
            seen.insert(PosKey::of(up));
        }
        return Some(top);
    }
    if !reg.is_replaceable(id) {
        return None;
    }
    let mut q = p;
    for _ in 0..DESCEND {
        if q.k == 0 {
            return None;
        }
        q.k -= 1;
        let (qid, _) = c.get(q);
        if reg.has_gravity(qid) {
            return Some(q);
        }
        if !reg.is_replaceable(qid) {
            return None;
        }
    }
    None
}

/// Phi of the module doc over every edited chunk's powder.
pub fn powder_potential(edits: &EditStore, reg: &Registry) -> u64 {
    let mut phi = 0u64;
    for c in edits.snapshot_full() {
        let k0 = c.key.origin_blocks().2;
        for (v, &id) in c.ids.iter().enumerate() {
            if reg.has_gravity(id) {
                let l = units(c.water.as_ref().map_or(0, |w| w[v])) as u64;
                phi += (POWDER_LAYER_WEIGHT * (k0 + (v % crate::grid::CHUNK) as u64) + l) * l;
            }
        }
    }
    phi
}

/// Total powder (units) over every edited chunk.
pub fn powder_total(edits: &EditStore, reg: &Registry) -> u64 {
    let mut total = 0u64;
    for c in edits.snapshot_full() {
        for (v, &id) in c.ids.iter().enumerate() {
            if reg.has_gravity(id) {
                total += units(c.water.as_ref().map_or(0, |w| w[v])) as u64;
            }
        }
    }
    total
}

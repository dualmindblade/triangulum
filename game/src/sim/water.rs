//! Andrew's water rule (M2b, DESIGN.md "Water").
//!
//! One byte of level per cell, 0 (no water; the block is air) to 255
//! (`FULL`); a cell is water exactly when its level is at least 1. Each
//! tick, every scheduled replaceable cell (air or water) runs two steps:
//!
//! 1. **Fall.** Water moves straight down into the cell below when that cell
//!    is replaceable and not full: as much as fits. The due cells are taken
//!    bottom-up (ascending `PosKey`, so k ascends within a column), which
//!    lets a free-standing column drop one cell per tick as a unit.
//! 2. **Spread.** What is left spreads sideways, only downhill and only to
//!    lateral neighbours at least 2 levels lower (Andrew's rule). The cell
//!    and those neighbours are levelled: the lowest neighbours are included
//!    while each is at or below the running mean of the included set
//!    (water-filling), then the included cells receive the floor of the
//!    mean, the cell keeps the floor, and the remainder (fewer units than
//!    cells) goes one unit each to the lowest included neighbours. So after
//!    a spread the cell is never more than 1 above or below any neighbour
//!    it levelled with, and water that nothing moves differs from every
//!    lateral neighbour by at most 1: "resting water may lean up to 1/256".
//!
//! 3. **Slide** (B4, Andrew's #7). A cell that can neither fall nor spread,
//!    with a free cell beside it whose floor is open (replaceable and not
//!    full), moves half its level, rounded up, straight into the cell below
//!    that free cell, as much as fits. This is what moves the level-1 cell at
//!    a ledge, which is only 1 above the air beside it, so the spread can
//!    never move it. Anything thicker gets there by spreading into the free
//!    cell and falling out of it next tick. Of several ledges the lowest of
//!    `choice` goes.
//! 4. **The film** (B4, Andrew's #9). What is left at level 1 is not at a
//!    ledge, and what the rule does with it is `Flow::film`:
//!    - `seek`: it looks along the four straight lines from the cell, through
//!      dry cells, up to `film_reach` cells, and its unit goes to the nearest
//!      drop, straight into the cell below it: the slide, from further off.
//!    - `merge`: no cell is wetted with fewer than 2 units, so a spread never
//!      thins an edge to 1, and a level-1 cell gives its unit to its wettest
//!      neighbour. Water beads up.
//!    - `dry`: it dries, one to three `film_dry_ticks` after it came to rest
//!      (`Sim::drying`), unless it leans on water that is not a puddle: the
//!      thicker water it touches reaches the boundary or runs to more than
//!      `PUDDLE_CELLS` cells (`Sim::may_dry`). So a puddle dries away from
//!      its edge, and a lake or a fed trickle keeps its edge and comes to
//!      rest. The units are counted (`SimStats::water_dried`).
//!    - `seek+dry`, the default: what has nowhere to go dries.
//!
//! Updates are applied in a deterministic order with a tick-local overlay:
//! a cell processed later in the same tick sees the writes of earlier ones,
//! and every write is committed in one batch at the end of the tick. A cell
//! that a neighbour spread into this tick does not spread itself when its
//! turn comes (it runs next tick, having changed), so water moves one cell
//! sideways per tick from any source and never chains across the region in
//! a single tick. A cell that water fell into does spread in the same tick
//! (down first, then sideways), so a pool under a fall is never starved of
//! its own update, unless the cell below it also received a fall this tick:
//! then it is the middle of a column in motion and waits, so a column drops
//! as a unit instead of tearing sideways. A cell with room below it never
//! spreads; it falls next tick. The spread pass alternates direction every
//! tick to cancel the directional bias a fixed order would leave while
//! water is in motion.
//! Every cell written in a tick is scheduled again for the next, whether or
//! not its level changed: the middle of a falling column gives and receives
//! the same amount and must still run. So is a cell that exchanged water
//! with the boundary and ended where it started (review 3 #9: a steady
//! throughflow from a reservoir into a drain used to fall asleep).
//!
//! **Why it settles.** Define the potential over every water cell
//!   Phi = sum over cells of (A * k + L) * L,   A = `POTENTIAL_LAYER_WEIGHT` = 1024,
//! with k the layer and L the level. A fall of d units from layer k to
//! k - 1 changes the layer term by -A d and the square term by
//! 2 d (L_below - L + d) <= 2 d * 254, so Phi drops by at least
//! (1024 - 508) d > 0. A spread keeps the layer term (same layer) and
//! replaces the levels of the cell and its included neighbours by the most
//! even integer distribution of the same total, which has the strictly
//! smallest sum of squares when the original had a pair differing by 2 or
//! more (it always has: the lowest included neighbour is at least 2 below
//! the cell). Phi is a non-negative integer, so only finitely many moves
//! can happen: no cycle, no squirming. The tests check Phi strictly falls
//! on every tick in which any unit moves (`water_potential`).
//!
//! A slide moves d units from layer k to layer k - 1, exactly like a fall:
//! that they land one column over changes nothing in Phi, which sees only
//! layers and levels. With B the level below, d <= L and B + d <= 255, so
//! Phi changes by -A d + 2 d (B - L + d) <= -A d + 2 d * 254, the same bound,
//! and drops by at least (1024 - 508) d > 0. So the reason water comes to
//! rest is unchanged: every move either goes down a layer or evens out a
//! layer. That is why a slide is one move into the lower cell and not a
//! sideways step and then a fall: a sideways step of the last unit leaves
//! the sum of squares as it was, and Phi would stand still on that tick.
//! A `seek` is the same move from further along a dry line, so the same
//! bound holds for it: that is why it goes straight to the lower cell.
//!
//! Drying removes a unit from a level-1 cell at layer k: Phi drops by
//! A k + 1. It is the only move that wakes neighbours without going down or
//! levelling, and it cannot run forever: a cell dries only from a puddle
//! that the boundary does not feed, so all the water it can ever take was
//! already there.
//!
//! `merge` moves water uphill, so it needs its own potential: `film_potential`
//! adds `FILM_WEIGHT` = 510 to Phi for every level-1 cell. A merge of level 1
//! into level l drops that by 510 for the cell it empties (and 510 more if
//! l = 1) and adds at most 2 * 254 + 1 = 509 for the squares. A spread never
//! makes more level-1 cells than it removes, since none is wetted with less
//! than 2 and a level-1 cell it levels with is raised. A fall can add one (a
//! remainder of 1), at most 510 + 505 over a drop of 1024. So that potential
//! strictly falls in `merge`, and the tests check it.
//!
//! The argument is for a closed body; the active region's boundary (below) can supply
//! and absorb water, and a river flowing in at one side and out at the
//! other is a steady flow by design, not a failure to settle.
//!
//! **The boundary** (`Sim::boundary_cell`). A neighbour in an inactive
//! chunk is frozen: it is never written. Where the generator has water
//! there, it acts as a reservoir at the level it holds now (its edit's if it
//! has one, so a lake the player drained near the edge does not refill
//! itself when the region shifts, and a lake the generator filled stays
//! full): a cell beside it that is 2 or more lower pulls half the
//! difference, a cell below it that is not full fills from it. Only
//! generated water that is still exactly as generated and below full (a
//! river cell stepping down, once M2-hydro produces levels) also absorbs:
//! the inside levels down to it. Everything else at the boundary is a
//! wall, including an edited channel dug past the edge, so a lake cannot
//! drain into frozen nothing. Units crossing the boundary are counted in
//! `SimStats::water_in` / `water_out`; the tests check that the water inside
//! changes by exactly in - out.

use super::{PosKey, Sim, choice, lateral};
use crate::grid::BlockPos;
use crate::world::edits::WATER_FULL;
use crate::world::{BlockId, EditStore, World};

/// A full cell.
pub const FULL: u8 = WATER_FULL;
/// Weight of the layer in the potential: more than 2 * 254 (see the module doc).
pub const POTENTIAL_LAYER_WEIGHT: u64 = 1024;
/// What a level-1 cell adds to the potential under `Film::Merge`.
pub const FILM_WEIGHT: u64 = 510;
/// The largest body a film may dry from (`Sim::may_dry`): a poured block is
/// about 60 cells of level 2 or more, a pond past this is not a puddle.
pub const PUDDLE_CELLS: usize = 256;

/// How an active cell sees a neighbour in an inactive chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// Nothing flows either way: rock, air, or water the player removed.
    Wall,
    /// Generated water, frozen at `level`: it supplies water to lower cells
    /// beside it and to a cell below it; when `drains`, it also absorbs
    /// water from higher cells, down to `level`.
    Water { level: u8, drains: bool },
}

/// How a written cell came by its water this tick.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Receipt {
    /// Its own update changed it.
    Own,
    /// A neighbour beside it spread into it.
    Spread,
    /// The cell above fell into it.
    Fall,
}

/// The potential of the module doc over every edited chunk's water (a
/// closed body of water that started as edits, for the tests).
pub fn water_potential(edits: &EditStore) -> u64 {
    let mut phi = 0u64;
    for c in edits.snapshot_full() {
        let Some(w) = &c.water else { continue };
        let k0 = c.key.cz as u64 * crate::grid::CHUNK as u64;
        for x in 0..crate::grid::CHUNK {
            for y in 0..crate::grid::CHUNK {
                for z in 0..crate::grid::CHUNK {
                    let l = w[crate::world::chunk::index(x, y, z)] as u64;
                    if l > 0 {
                        phi += (POTENTIAL_LAYER_WEIGHT * (k0 + z as u64) + l) * l;
                    }
                }
            }
        }
    }
    phi
}

/// `water_potential` plus `FILM_WEIGHT` per level-1 cell: the quantity that
/// strictly falls under `Film::Merge` (the module doc).
pub fn film_potential(edits: &EditStore) -> u64 {
    let film: u64 = edits.snapshot_full().iter().filter_map(|c| c.water.as_ref()).map(|w| w.iter().filter(|&&l| l == 1).count() as u64).sum();
    water_potential(edits) + FILM_WEIGHT * film
}

/// Total water (units) over every edited chunk.
pub fn water_total(edits: &EditStore) -> u64 {
    edits.snapshot_full().iter().filter_map(|c| c.water.as_ref()).map(|w| w.iter().map(|&l| l as u64).sum::<u64>()).sum()
}

/// The next cell on a straight line from `prev` through `cur`, across
/// chunk borders, cube edges and cube corners. Over a cube edge the line's
/// way in the new face's frame is the one opposite the way it came in.
fn straight_on(world: &World, prev: BlockPos, cur: BlockPos) -> Option<BlockPos> {
    if prev.face == cur.face {
        return world.resolve_block(cur.face, 2 * cur.i as i64 - prev.i as i64, 2 * cur.j as i64 - prev.j as i64, cur.k as i64);
    }
    let around = lateral(world, cur);
    let came_from = around.iter().position(|n| *n == Some(prev))?;
    around[came_from ^ 1]
}

impl Sim {
    /// Is the chunk holding this block simulated right now?
    #[inline]
    pub(super) fn is_active_block(&self, bp: BlockPos) -> bool {
        self.active.contains(&World::chunk_of(bp).0)
    }

    /// A cell as this tick sees it: the overlay first, then the world.
    #[inline]
    pub(super) fn cell(&self, world: &World, bp: BlockPos) -> (BlockId, u8) {
        if let Some(&c) = self.overlay.writes.get(&PosKey::of(bp)) {
            return c;
        }
        world.block_and_level_at(bp)
    }

    /// Record a write for the end-of-tick commit, noting how the cell got
    /// its water when another cell gave it (see `Overlay`).
    fn write(&mut self, world: &World, bp: BlockPos, level: u8, how: Receipt) {
        let id = if level > 0 { world.water_id() } else { world.generator.blocks.air };
        let key = PosKey::of(bp);
        self.overlay.writes.insert(key, (id, level));
        match how {
            Receipt::Own => {}
            Receipt::Spread => {
                self.overlay.spread_into.insert(key);
            }
            Receipt::Fall => {
                self.overlay.fell_into.insert(key);
            }
        }
    }

    /// What an active cell sees in an inactive neighbour (module doc).
    pub fn boundary_cell(&self, world: &World, bp: BlockPos) -> Boundary {
        #[cfg(test)]
        if let Some(f) = &self.boundary_override
            && let Some(b) = f(bp)
        {
            return b;
        }
        let (nid, nl) = world.natural_cell(bp);
        if !world.registry.is_liquid(nid) {
            return Boundary::Wall;
        }
        let (cid, cl) = world.block_and_level_at(bp);
        if !world.registry.is_liquid(cid) || cl == 0 {
            return Boundary::Wall;
        }
        Boundary::Water { level: cl, drains: cid == nid && cl == nl && nl < FULL }
    }

    /// Step 1 for one cell: fill from a reservoir above, then fall.
    pub(super) fn water_fall(&mut self, world: &World, p: BlockPos) {
        let reg = world.registry.clone();
        let (id, start) = self.cell(world, p);
        if !reg.is_replaceable(id) {
            return;
        }
        if start == 0 && self.config.flow.slide {
            self.wake_beside_drop(world, p);
        }
        let flux = self.stats.water_in + self.stats.water_out;
        let mut level = start;
        if level < FULL && p.k + 1 < world.planet.height {
            let above = BlockPos { k: p.k + 1, ..p };
            if !self.is_active_block(above)
                && let Boundary::Water { level: la, .. } = self.boundary_cell(world, above)
            {
                let d = la.min(FULL - level);
                if d > 0 {
                    level += d;
                    self.stats.water_in += d as u64;
                    self.overlay.moved += d as u64;
                }
            }
        }
        if level > 0 && p.k > 0 {
            let below = BlockPos { k: p.k - 1, ..p };
            if self.is_active_block(below) {
                let (bid, bl) = self.cell(world, below);
                if reg.is_replaceable(bid) && bl < FULL {
                    let d = level.min(FULL - bl);
                    level -= d;
                    self.write(world, below, bl + d, Receipt::Fall);
                    self.overlay.moved += d as u64;
                }
            } else if let Boundary::Water { level: bl, drains: true } = self.boundary_cell(world, below)
                && bl < FULL
            {
                let d = level.min(FULL - bl);
                level -= d;
                self.stats.water_out += d as u64;
                self.overlay.moved += d as u64;
            }
        }
        self.finish_cell(world, p, start, level, flux);
    }

    /// An empty cell has been notified. If it has room below it may just have
    /// become a way down for what rests beside it, which is diagonal to the
    /// change and heard nothing: wake that water and powder for next tick.
    fn wake_beside_drop(&mut self, world: &World, p: BlockPos) {
        if p.k == 0 {
            return;
        }
        let (bid, bl) = self.cell(world, BlockPos { k: p.k - 1, ..p });
        if !world.registry.is_replaceable(bid) || bl >= FULL {
            return;
        }
        let next = self.tick + 1;
        for n in lateral(world, p).into_iter().flatten() {
            let (id, level) = self.cell(world, n);
            if level > 0 || world.registry.has_gravity(id) {
                self.queue.insert((next, PosKey::of(n)));
                self.dirty = true;
            }
        }
    }

    /// Steps 3 and 4: the level left after a slide off a ledge, or from a
    /// level-1 cell under `seek` after a run to one further off along a
    /// straight dry line (the module doc): the nearest wins, then the lowest
    /// of `choice`. A line that leaves the region waits for that chunk.
    fn water_slide(&mut self, world: &World, p: BlockPos, beside: &[Option<BlockPos>; 4], level: u8) -> u8 {
        if p.k == 0 {
            return level;
        }
        let flow = self.config.flow;
        let reach = if level == 1 && flow.film.seeks() { flow.film_reach.max(1) } else { 1 };
        let key = PosKey::of(p);
        let reg = world.registry.clone();
        let mut best: Option<((u32, u64), BlockPos, u8)> = None;
        let mut outside = [None; 4];
        for (way, first) in beside.iter().enumerate() {
            let Some(mut cur) = *first else { continue };
            let mut prev = p;
            for dist in 1..=reach {
                if best.is_some_and(|b| b.0.0 < dist) {
                    break;
                }
                if !self.is_active_block(cur) {
                    // At one step `wake_new_pairs` takes this pair when the
                    // chunk joins; further on, the cells between are dry.
                    if dist > 1 {
                        outside[way] = Some(World::chunk_of(cur).0);
                    }
                    break;
                }
                let (nid, nl) = self.cell(world, cur);
                if !reg.is_replaceable(nid) || nl >= if dist == 1 { level } else { 1 } {
                    break;
                }
                let low = BlockPos { k: cur.k - 1, ..cur };
                if !self.is_active_block(low) {
                    outside[way] = Some(World::chunk_of(low).0);
                    break;
                }
                let (bid, bl) = self.cell(world, low);
                if reg.is_replaceable(bid) && bl < FULL {
                    let rank = (dist, choice(key, PosKey::of(low), self.tick));
                    if best.is_none_or(|b| rank < b.0) {
                        best = Some((rank, low, bl));
                    }
                    break;
                }
                let Some(next) = straight_on(world, prev, cur) else { break };
                prev = cur;
                cur = next;
            }
        }
        let Some((_, low, bl)) = best else {
            for c in outside.into_iter().flatten() {
                self.wait_for(c, key);
            }
            return level;
        };
        let d = level.div_ceil(2).min(FULL - bl);
        self.write(world, low, bl + d, Receipt::Fall);
        self.overlay.moved += d as u64;
        level - d
    }

    /// `Film::Merge`: the level-1 cell gives its unit to its wettest
    /// neighbour with room, ties by `choice`. The level left.
    fn water_merge(&mut self, world: &World, p: BlockPos, beside: &[Option<BlockPos>; 4]) -> u8 {
        let key = PosKey::of(p);
        let mut best: Option<((u8, u64), BlockPos)> = None;
        for n in beside.iter().flatten() {
            if !self.is_active_block(*n) {
                continue;
            }
            let (id, nl) = self.cell(world, *n);
            if world.registry.is_replaceable(id) && nl >= 1 && nl < FULL {
                let rank = (nl, u64::MAX - choice(key, PosKey::of(*n), self.tick));
                if best.is_none_or(|b| rank > b.0) {
                    best = Some((rank, *n));
                }
            }
        }
        let Some(((nl, _), n)) = best else { return 1 };
        self.write(world, n, nl + 1, Receipt::Spread);
        self.overlay.moved += 1;
        0
    }

    /// `Film::Dry`: a level-1 cell at rest waits to dry. Due between one and
    /// three drying times from now, on a beat of its own, so a film dries in
    /// patches over time instead of all in one tick, and a cell notified
    /// again in the same stretch files the same entry.
    fn plan_drying(&mut self, p: BlockPos) {
        let t = self.config.flow.film_dry_ticks.max(1);
        let key = PosKey::of(p);
        let due = (self.tick + t).div_ceil(t) * t + choice(key, key, 0) % t;
        if self.drying.insert((due, key)) {
            self.dirty = true;
        }
    }

    /// May this level-1 cell dry? Not if the thicker water it leans on is a
    /// body rather than a puddle: walking the cells of level 2 and more that
    /// connect to it on this layer, it may not reach the boundary's water or
    /// run past `PUDDLE_CELLS`. That keeps the edge of a lake and of a fed
    /// trickle, where drying would draw water in for ever.
    fn may_dry(&self, world: &World, p: BlockPos) -> bool {
        let mut cells = vec![p];
        let mut seen: crate::grid::FastSet<PosKey> = Default::default();
        let mut at = 0;
        while at < cells.len() {
            let c = cells[at];
            at += 1;
            for n in lateral(world, c).into_iter().flatten() {
                if !self.is_active_block(n) {
                    if matches!(self.boundary_cell(world, n), Boundary::Water { level, .. } if level >= 2) {
                        return false;
                    }
                    continue;
                }
                let (id, l) = self.cell(world, n);
                if world.registry.is_replaceable(id) && l >= 2 && seen.insert(PosKey::of(n)) {
                    if cells.len() > PUDDLE_CELLS {
                        return false;
                    }
                    cells.push(n);
                }
            }
        }
        true
    }

    /// Dry the films whose time has come. A cell outside the region goes
    /// with the frozen updates and files again when it wakes.
    pub(super) fn dry_films(&mut self, world: &World) {
        let due: Vec<(u64, PosKey)> = self.drying.range(..=(self.tick, PosKey(u64::MAX))).copied().collect();
        for e in &due {
            self.drying.remove(e);
        }
        self.dirty = true;
        for (_, key) in due {
            let p = key.pos();
            if !self.is_active_block(p) {
                self.wait_for(World::chunk_of(p).0, key);
                continue;
            }
            let (_, level) = self.cell(world, p);
            if level == 1 && !self.room_below(world, p) && self.may_dry(world, p) {
                self.write(world, p, 0, Receipt::Own);
                self.stats.water_dried += 1;
            }
        }
    }

    /// Water here would fall: the cell below takes it, or drains it at the
    /// boundary.
    fn room_below(&self, world: &World, p: BlockPos) -> bool {
        if p.k == 0 {
            return false;
        }
        let below = BlockPos { k: p.k - 1, ..p };
        if self.is_active_block(below) {
            let (bid, bl) = self.cell(world, below);
            world.registry.is_replaceable(bid) && bl < FULL
        } else {
            matches!(self.boundary_cell(world, below), Boundary::Water { level: bl, drains: true } if bl < FULL)
        }
    }

    /// Record a cell's own new level; a cell that exchanged water with the
    /// boundary and came out where it started (a steady throughflow: in
    /// from a reservoir, on into a drain) writes nothing but still runs
    /// next tick (review 3 #9: it used to fall asleep, and the flow and its
    /// accounting stopped while the source and the drain were unchanged).
    fn finish_cell(&mut self, world: &World, p: BlockPos, start: u8, level: u8, flux: u64) {
        if level != start {
            self.write(world, p, level, Receipt::Own);
        } else if self.stats.water_in + self.stats.water_out != flux {
            self.overlay.exchanged.insert(PosKey::of(p));
        }
    }

    /// Step 2 for one cell: pull from reservoirs beside it, then spread.
    pub(super) fn water_spread(&mut self, world: &World, p: BlockPos) {
        let key = PosKey::of(p);
        if self.overlay.spread_into.contains(&key) {
            return;
        }
        // The middle of a falling column: water fell into this cell and into
        // the one below it this tick, so the water below is itself falling
        // and this cell waits instead of spreading over it.
        if self.overlay.fell_into.contains(&key) && p.k > 0 && self.overlay.fell_into.contains(&PosKey::of(BlockPos { k: p.k - 1, ..p })) {
            return;
        }
        let reg = world.registry.clone();
        let (id, start) = self.cell(world, p);
        if !reg.is_replaceable(id) {
            return;
        }
        let flux = self.stats.water_in + self.stats.water_out;
        let mut level = start;
        let beside = lateral(world, p);
        // Reservoirs beside the cell: pairwise, half the difference.
        for n in beside.iter().flatten() {
            if !self.is_active_block(*n)
                && let Boundary::Water { level: ln, .. } = self.boundary_cell(world, *n)
                && ln >= level.saturating_add(2)
            {
                let d = (ln - level) / 2;
                level += d;
                self.stats.water_in += d as u64;
                self.overlay.moved += d as u64;
            }
        }
        // Down first: a cell that water fell into this tick may still have
        // room below (its own fall step ran before it was filled); it falls
        // next tick instead of spreading now.
        let can_fall = self.room_below(world, p);
        let mut spread = false;
        if level >= 2 && !can_fall {
            // Candidates: (level, neighbour order, cell, is a boundary drain).
            let mut cands: Vec<(u8, usize, BlockPos, bool)> = Vec::with_capacity(4);
            for (idx, n) in beside.iter().enumerate() {
                let Some(n) = n else { continue };
                if self.is_active_block(*n) {
                    let (nid, nl) = self.cell(world, *n);
                    if reg.is_replaceable(nid) && nl <= level - 2 {
                        cands.push((nl, idx, *n, false));
                    }
                } else if let Boundary::Water { level: nl, drains: true } = self.boundary_cell(world, *n)
                    && nl <= level - 2
                {
                    cands.push((nl, idx, *n, true));
                }
            }
            cands.sort_by_key(|c| (c.0, c.1));
            // Water-filling: the lowest neighbours, while each is at or below
            // the running mean.
            let fill = |cands: &[(u8, usize, BlockPos, bool)]| {
                let (mut total, mut count, mut included) = (level as u32, 1u32, 0usize);
                for c in cands {
                    if c.0 as u32 * count <= total {
                        total += c.0 as u32;
                        count += 1;
                        included += 1;
                    } else {
                        break;
                    }
                }
                (total, count, included)
            };
            // Merge: a dry cell is only ever wetted with 2 or more, so nothing
            // spreads that would have to leave it less.
            if self.config.flow.film.merges() && (fill(&cands).0 / fill(&cands).1) < 2 {
                cands.retain(|c| c.0 > 0);
            }
            if !cands.is_empty() {
                let (total, count, included) = fill(&cands);
                let m = total / count;
                let r = total - m * count; // < count, so at most `included` units
                for (n, c) in cands[..included].iter().enumerate() {
                    let nl = m as u8 + u8::from((n as u32) < r);
                    let d = nl - c.0;
                    if c.3 {
                        self.stats.water_out += d as u64;
                    } else {
                        self.write(world, c.2, nl, Receipt::Spread);
                    }
                    self.overlay.moved += d as u64;
                }
                level = m as u8;
                spread = true;
            }
        }
        if level > 0 && !can_fall && !spread && self.config.flow.slide {
            level = self.water_slide(world, p, &beside, level);
        }
        if level == 1 && !can_fall {
            let film = self.config.flow.film;
            if film.merges() {
                level = self.water_merge(world, p, &beside);
            } else if film.dries() {
                self.plan_drying(p);
            }
        }
        self.finish_cell(world, p, start, level, flux);
    }

    /// Commit this tick's water writes as one batch, in key order.
    pub(super) fn commit_water(&mut self, world: &World) {
        self.stats.water_moved += self.overlay.moved;
        self.overlay.moved = 0;
        self.overlay.spread_into.clear();
        self.overlay.fell_into.clear();
        let next = self.tick + 1;
        if !self.overlay.exchanged.is_empty() {
            for key in self.overlay.exchanged.drain() {
                self.queue.insert((next, key));
            }
            self.dirty = true;
        }
        if self.overlay.writes.is_empty() {
            return;
        }
        let mut writes: Vec<(PosKey, (BlockId, u8))> = self.overlay.writes.drain().collect();
        writes.sort_unstable_by_key(|w| w.0);
        let batch: Vec<(BlockPos, BlockId, Option<u8>)> = writes.iter().map(|(k, (id, l))| (k.pos(), *id, Some(*l))).collect();
        self.stats.water_writes += world.commit_levels(&batch) as u64;
        // A cell that gave and received the same amount (the middle of a
        // falling column) has not changed, so the change log will not
        // schedule it; water moved through it, so it runs again next tick.
        for (key, _) in writes {
            self.queue.insert((next, key));
        }
        self.dirty = true;
    }
}

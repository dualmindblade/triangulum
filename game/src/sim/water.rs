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
//! on every tick in which any unit moves (`water_potential`). The argument
//! is for a closed body; the active region's boundary (below) can supply
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

use super::{PosKey, Sim};
use crate::grid::BlockPos;
use crate::world::edits::WATER_FULL;
use crate::world::{BlockId, EditStore, World};

/// A full cell.
pub const FULL: u8 = WATER_FULL;
/// Weight of the layer in the potential: more than 2 * 254 (see the module doc).
pub const POTENTIAL_LAYER_WEIGHT: u64 = 1024;

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

/// Total water (units) over every edited chunk.
pub fn water_total(edits: &EditStore) -> u64 {
    edits.snapshot_full().iter().filter_map(|c| c.water.as_ref()).map(|w| w.iter().map(|&l| l as u64).sum::<u64>()).sum()
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
        let (i, j, k) = (p.i as i64, p.j as i64, p.k as i64);
        let lateral: [Option<BlockPos>; 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)].map(|(di, dj)| world.resolve_block(p.face, i + di, j + dj, k));
        // Reservoirs beside the cell: pairwise, half the difference.
        for n in lateral.iter().flatten() {
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
        let can_fall = p.k > 0 && {
            let below = BlockPos { k: p.k - 1, ..p };
            if self.is_active_block(below) {
                let (bid, bl) = self.cell(world, below);
                reg.is_replaceable(bid) && bl < FULL
            } else {
                matches!(self.boundary_cell(world, below), Boundary::Water { level: bl, drains: true } if bl < FULL)
            }
        };
        if level >= 2 && !can_fall {
            // Candidates: (level, neighbour order, cell, is a boundary drain).
            let mut cands: Vec<(u8, usize, BlockPos, bool)> = Vec::with_capacity(4);
            for (idx, n) in lateral.iter().enumerate() {
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
            if !cands.is_empty() {
                cands.sort_by_key(|c| (c.0, c.1));
                let mut total = level as u32;
                let mut count = 1u32;
                let mut included = 0usize;
                for c in &cands {
                    // Include while the neighbour is at or below the running mean.
                    if c.0 as u32 * count <= total {
                        total += c.0 as u32;
                        count += 1;
                        included += 1;
                    } else {
                        break;
                    }
                }
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

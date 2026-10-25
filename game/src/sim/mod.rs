//! The simulation core (M2a): a fixed 20 Hz tick, a deterministic
//! block-update queue with neighbour notifications across chunk borders and
//! cube-face edges, an active region around the player, and the rules that
//! run on it: falling blocks (sand and gravel, by flag) and, since M2b,
//! Andrew's 256-level water (`water.rs`).
//!
//! Clocks. The tick is tied to the player's fixed 60 Hz steps (every third
//! step, `STEPS_PER_TICK`), not to wall time and not to the sky clock. The
//! `[` and `]` keys scale the sky's seconds only: a 3600x day does not make
//! sand fall faster, because a sped-up sky is a look, and a sped-up physics
//! would make every scripted session depend on how the clock was set. The
//! tick runs on the main thread inside `Session::step`; nothing in it uses
//! rayon or wall time for anything but the cost measurement.
//!
//! Determinism. Nothing here iterates a HashMap. The queue is a BTreeSet
//! ordered by (due tick, canonical position key); frozen updates sit in a
//! BTreeMap by chunk; falling blocks are sorted by (column, height) before
//! they move. Rules read and write the world through `World`, whose edit
//! store is the one authority for player and simulation writes alike:
//! `World::set_block` logs every change and the tick drains the log and
//! schedules the changed block and its six neighbours, whoever made the
//! change. `state_hash` covers the edited chunks, the queue, the frozen
//! updates and the falling blocks, and the tests show it is the same across
//! two runs, across rayon pools of 1 and 8 threads, and across a save and a
//! reload in the middle.
//!
//! Active region. LOD-0 chunks whose bounding sphere comes within
//! `SimConfig::radius_m` of the player are active; it is recomputed when the
//! player crosses a chunk border. An update whose chunk is not active is
//! moved to `frozen` under its chunk and comes back into the queue, due now,
//! when that chunk is active again; it is never dropped and it is saved.
//! When chunks join the region, both cells of every neighbour pair across
//! their sides that could move water and is not as generated are woken
//! too (review 3 #3: water at rest against the old edge used to stay there).
//! A falling block outside the region hangs where it is, and one inside
//! never moves into, lands in or displaces water into a chunk outside: it
//! waits at the edge (review 3 #7). Rendering never decides any of this:
//! the region is a function of the player's position.
//!
//! Falling blocks. A block whose definition has `gravity` and whose cell
//! below is `replaceable` (air, water) leaves the grid and becomes a
//! `Falling` entity in its own column, accelerating under the player's
//! gravity to a terminal speed, and lands in the first cell above a
//! non-replaceable one, written back through `World::set_block`. Columns
//! keep their shape (DESIGN.md, "Things that move" 1): the entity never
//! leaves (face, i, j). Removing the block notifies the one above, which
//! falls a tick later, so a column drains from the bottom in a chain and
//! lands in order. Falling blocks do not collide with each other or the
//! player in flight; if something is built into the cell a block is in, it
//! settles in the first free cell above.

pub mod draw;
pub mod water;

use crate::grid::{BlockPos, CHUNK, ChunkKey, FastMap, FastSet};
use crate::sha256::{Sha256, hex};
use crate::world::{BlockId, World};
use glam::DVec3;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;
pub use water::Boundary;

/// The version of the simulation's rules: what a replay recorded with this
/// build expects them to do. A replay names it and a build with other rules
/// refuses to play it rather than diverge. 1: M2a to M2c. 2: M2-hardening
/// (review 3): a region that grows wakes both sides of the pairs it takes
/// in (#3), a steady throughflow at the boundary stays scheduled (#9), and
/// falling blocks wait at the region's edge (#7).
pub const RULES_VERSION: u32 = 2;

/// Simulation rate.
pub const TICK_HZ: u32 = 20;
pub const TICK_S: f64 = 1.0 / TICK_HZ as f64;
/// Player steps (`player::STEP`, 60 Hz) per tick.
pub const STEPS_PER_TICK: u32 = 3;
/// The player's gravity and terminal speed (player.rs), so a block and a
/// body dropped together land together.
pub const GRAVITY_M_S2: f64 = 25.0;
pub const TERMINAL_M_S: f64 = 70.0;
/// A landed block is still drawn as a cube for this many ticks, while the
/// chunk it landed in is remeshed (cosmetic; not state, not saved).
const GHOST_TICKS: u64 = 5;
/// Ticks kept for the average cost.
const COST_WINDOW: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimConfig {
    /// Radius of the active region around the player, metres.
    pub radius_m: f64,
    /// `sim off` in the harness, `--no-sim` on the command line.
    pub enabled: bool,
    /// Ticks per water step (M2b): water updates run on ticks that are a
    /// multiple of this and wait for the next one otherwise. 1 is a cell
    /// per tick (20 cells per second); Andrew's dial for the flow speed.
    pub water_period: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { radius_m: 112.0, enabled: true, water_period: 1 }
    }
}

/// This tick's water writes, applied as one commit after the drain (water.rs).
#[derive(Default)]
struct Overlay {
    writes: FastMap<PosKey, (BlockId, u8)>,
    /// Cells a neighbour spread into this tick: they do not spread
    /// themselves until the next, so water moves one cell sideways per tick.
    spread_into: FastSet<PosKey>,
    /// Cells water fell into this tick: one whose own lower neighbour also
    /// received a fall is the middle of a moving column and waits too.
    fell_into: FastSet<PosKey>,
    /// Cells that exchanged water with the boundary this tick but ended
    /// where they started: they run again next tick (review 3 #9).
    exchanged: FastSet<PosKey>,
    /// Units moved this tick (falls, spreads, boundary flows).
    moved: u64,
}

/// The canonical position key the queue sorts by: face, i, j, k packed so
/// that integer order is lexicographic order. N up to 2^20 columns, H up to
/// 2^16 layers; `Planet::try_new` refuses larger worlds at startup (review 3
/// #10), so no key aliases another. The key is body-local: a `Sim` is one
/// body's simulation (Neisor's; its frozen chunk keys carry the body), and
/// another body (M4's vehicles, on their own grid) gets its own `Sim` and
/// its own key space rather than a body field here.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PosKey(pub u64);

impl PosKey {
    pub const MAX_N: u32 = 1 << 20;
    pub const MAX_H: u32 = 1 << 16;

    #[inline]
    pub fn of(bp: BlockPos) -> Self {
        debug_assert!(bp.i < Self::MAX_N && bp.j < Self::MAX_N && bp.k < Self::MAX_H && bp.face < 6);
        Self(((bp.face as u64) << 56) | ((bp.i as u64) << 36) | ((bp.j as u64) << 16) | bp.k as u64)
    }

    #[inline]
    pub fn pos(self) -> BlockPos {
        BlockPos { face: (self.0 >> 56) as u8, i: ((self.0 >> 36) & 0xF_FFFF) as u32, j: ((self.0 >> 16) & 0xF_FFFF) as u32, k: (self.0 & 0xFFFF) as u32 }
    }

    /// The key of this position's column (layer 0).
    #[inline]
    pub fn column(self) -> Self {
        Self(self.0 & !0xFFFF)
    }
}

/// A block in flight, in its own column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Falling {
    pub id: BlockId,
    pub face: u8,
    pub i: u32,
    pub j: u32,
    /// Continuous layer of the block's bottom (layers, `>= 0`).
    pub z: f64,
    /// Layers per second, `<= 0`.
    pub vel: f64,
}

impl Falling {
    #[inline]
    pub fn column(&self) -> PosKey {
        PosKey::of(BlockPos { face: self.face, i: self.i, j: self.j, k: 0 })
    }
    /// The cell the block's bottom is in.
    #[inline]
    pub fn cell(&self) -> i64 {
        self.z.floor() as i64
    }
    fn sort_key(&self) -> (PosKey, u64) {
        (self.column(), self.z.to_bits())
    }
}

/// Everything a save needs to carry about the simulation.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub tick: u64,
    pub queue: Vec<(u64, PosKey)>,
    pub frozen: Vec<(ChunkKey, Vec<(u64, PosKey)>)>,
    pub falling: Vec<Falling>,
    /// Version 2 (review 3 #4): the settings the state ran under, where the
    /// active region was computed (it depends on where in its chunk the
    /// player stood, so a restore brings it back rather than recomputing it
    /// around wherever the player is), and the digest of the block rules
    /// (`rules_digest`). None in a version-1 file.
    pub config: Option<SimConfig>,
    pub focus: Option<DVec3>,
    pub rules: Option<[u8; 32]>,
}

const SNAPSHOT_MAGIC: &[u8; 4] = b"TRSM";
const SNAPSHOT_VERSION: u32 = 2;

/// The SHA-256 of the block definitions the simulation's rules read: each
/// id's name and its solid, liquid, gravity and replaceable flags. Colours
/// are left out: they are look, and changing one must not change the hash
/// (Andrew's live look panel edits them while the game runs).
pub fn rules_digest(reg: &crate::world::Registry) -> [u8; 32] {
    rules_digest_upto(reg, usize::MAX)
}

/// M3b: the rules digest of the first `n` blocks. Blocks.json only ever
/// grows (ids are append-only), and a block's identity is its first name
/// (a renamed block keeps the old one in `was`), so the digest of a
/// registry's first blocks is the digest a save or a replay recorded
/// before blocks were added or renamed.
pub fn rules_digest_upto(reg: &crate::world::Registry, n: usize) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"triangulum-block-rules-1");
    for d in reg.defs().iter().take(n) {
        let name = d.first_name();
        h.update(&d.id.to_le_bytes());
        h.update(&(name.len() as u32).to_le_bytes());
        h.update(name.as_bytes());
        h.update(&[d.solid as u8 | (d.liquid as u8) << 1 | (d.gravity as u8) << 2 | (d.replaceable as u8) << 3]);
    }
    h.finish()
}

/// M3b: were `digest`'s rules this registry's, or the rules of the blocks
/// it began with before blocks were appended?
pub fn rules_extend(reg: &crate::world::Registry, digest: &[u8; 32]) -> bool {
    (1..=reg.len()).rev().any(|n| rules_digest_upto(reg, n) == *digest)
}

impl Snapshot {
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty() && self.frozen.is_empty() && self.falling.is_empty()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Vec::new();
        w.extend_from_slice(SNAPSHOT_MAGIC);
        w.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        w.extend_from_slice(&self.tick.to_le_bytes());
        let entries = |w: &mut Vec<u8>, es: &[(u64, PosKey)]| {
            w.extend_from_slice(&(es.len() as u64).to_le_bytes());
            for (due, key) in es {
                w.extend_from_slice(&due.to_le_bytes());
                w.extend_from_slice(&key.0.to_le_bytes());
            }
        };
        entries(&mut w, &self.queue);
        w.extend_from_slice(&(self.frozen.len() as u64).to_le_bytes());
        for (ck, es) in &self.frozen {
            w.extend_from_slice(&[ck.body, ck.face, ck.lod]);
            w.extend_from_slice(&ck.cx.to_le_bytes());
            w.extend_from_slice(&ck.cy.to_le_bytes());
            w.extend_from_slice(&ck.cz.to_le_bytes());
            entries(&mut w, es);
        }
        w.extend_from_slice(&(self.falling.len() as u64).to_le_bytes());
        for f in &self.falling {
            w.extend_from_slice(&f.id.to_le_bytes());
            w.push(f.face);
            w.extend_from_slice(&f.i.to_le_bytes());
            w.extend_from_slice(&f.j.to_le_bytes());
            w.extend_from_slice(&f.z.to_bits().to_le_bytes());
            w.extend_from_slice(&f.vel.to_bits().to_le_bytes());
        }
        // Version 2: a presence byte, then each part present.
        w.push(self.config.is_some() as u8 | (self.focus.is_some() as u8) << 1 | (self.rules.is_some() as u8) << 2);
        if let Some(c) = &self.config {
            w.extend_from_slice(&c.radius_m.to_bits().to_le_bytes());
            w.push(c.enabled as u8);
            w.extend_from_slice(&c.water_period.to_le_bytes());
        }
        if let Some(p) = self.focus {
            for x in [p.x, p.y, p.z] {
                w.extend_from_slice(&x.to_bits().to_le_bytes());
            }
        }
        if let Some(r) = &self.rules {
            w.extend_from_slice(r);
        }
        w
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, String> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != SNAPSHOT_MAGIC {
            return Err("not a simulation state file".into());
        }
        let version = r.u32()?;
        if version != 1 && version != SNAPSHOT_VERSION {
            return Err(format!("simulation state version {version}, this build reads 1 and {SNAPSHOT_VERSION}"));
        }
        let tick = r.u64()?;
        let queue = r.entries()?;
        let nf = r.count()?;
        let mut frozen = Vec::with_capacity(nf);
        for _ in 0..nf {
            let h = r.take(3)?;
            let (body, face, lod) = (h[0], h[1], h[2]);
            let cx = r.u32()?;
            let cy = r.u32()?;
            let cz = r.u32()?;
            if lod != 0 || face > 5 {
                return Err(format!("bad frozen chunk key face {face} lod {lod}"));
            }
            let es = r.entries()?;
            frozen.push((ChunkKey { body, face, lod, cx, cy, cz }, es));
        }
        let nb = r.count()?;
        let mut falling = Vec::with_capacity(nb);
        for _ in 0..nb {
            let id = u16::from_le_bytes(r.take(2)?.try_into().unwrap());
            let face = r.take(1)?[0];
            let i = r.u32()?;
            let j = r.u32()?;
            let z = f64::from_bits(r.u64()?);
            let vel = f64::from_bits(r.u64()?);
            if face > 5 || !z.is_finite() || !vel.is_finite() || z < 0.0 {
                return Err(format!("bad falling block face {face} z {z} vel {vel}"));
            }
            falling.push(Falling { id, face, i, j, z, vel });
        }
        let (mut config, mut focus, mut rules) = (None, None, None);
        if version >= 2 {
            let parts = r.take(1)?[0];
            if parts & !7 != 0 {
                return Err(format!("bad simulation state parts {parts:#x}"));
            }
            if parts & 1 != 0 {
                let radius_m = f64::from_bits(r.u64()?);
                let enabled = match r.take(1)?[0] {
                    0 => false,
                    1 => true,
                    x => return Err(format!("bad simulation enabled flag {x}")),
                };
                let water_period = r.u64()?;
                if !radius_m.is_finite() || radius_m < 0.0 || water_period == 0 {
                    return Err(format!("bad simulation settings: radius {radius_m}, water period {water_period}"));
                }
                config = Some(SimConfig { radius_m, enabled, water_period });
            }
            if parts & 2 != 0 {
                let p = DVec3::new(f64::from_bits(r.u64()?), f64::from_bits(r.u64()?), f64::from_bits(r.u64()?));
                if !p.is_finite() {
                    return Err(format!("bad active-region focus {p}"));
                }
                focus = Some(p);
            }
            if parts & 4 != 0 {
                rules = Some(r.take(32)?.try_into().unwrap());
            }
        }
        if r.at != b.len() {
            return Err(format!("{} trailing bytes after simulation state", b.len() - r.at));
        }
        Ok(Self { tick, queue, frozen, falling, config, focus, rules })
    }
}

/// A bounds-checked little-endian cursor for `Snapshot::from_bytes`.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.at..self.at + n).ok_or_else(|| format!("simulation state truncated at byte {}", self.at))?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn count(&mut self) -> Result<usize, String> {
        let n = self.u64()?;
        if n > 1 << 32 {
            return Err(format!("implausible count {n} in simulation state"));
        }
        Ok(n as usize)
    }
    fn entries(&mut self) -> Result<Vec<(u64, PosKey)>, String> {
        let n = self.count()?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            let due = self.u64()?;
            let key = self.u64()?;
            v.push((due, PosKey(key)));
        }
        Ok(v)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SimStats {
    pub ticks: u64,
    pub last_us: u32,
    pub max_us: u32,
    recent_us: VecDeque<u32>,
    /// Updates whose rule ran.
    pub processed: u64,
    pub falls: u64,
    pub landings: u64,
    /// Blocks that found no free cell above when their cell was taken.
    pub vanished: u64,
    /// Cost of the last active-region recompute.
    pub refocus_us: u32,
    pub refocuses: u64,
    /// Neighbour pairs woken because the region grew over them (review 3 #3).
    pub pairs_woken: u64,
    /// Water (M2b), all in level units (255 = one full cell).
    /// Units moved by the rule (falls, spreads, boundary flows).
    pub water_moved: u64,
    /// Cell writes the water rule committed.
    pub water_writes: u64,
    /// Units that entered the active region from generated water at its
    /// boundary, and units that left into generated river cells there.
    pub water_in: u64,
    pub water_out: u64,
    /// Units the player placed and removed.
    pub water_placed: u64,
    pub water_removed: u64,
    /// Units pushed up by a block taking a water cell, and units that found
    /// no room above (lost under a solid ceiling).
    pub water_displaced: u64,
    pub water_lost: u64,
    /// Water cell updates run last tick.
    pub water_updates_last: u32,
    /// Last tick's cost by phase (microseconds): the water rule's two
    /// passes, its commit (the edit store, mesh invalidations), and the
    /// end-of-tick change-log drain that schedules the next tick.
    pub water_rule_us: u32,
    pub water_commit_us: u32,
    pub absorb_us: u32,
}

impl SimStats {
    /// Water created or destroyed by anything but the rule: what the water
    /// inside the active region changed by, in units, if the rule itself
    /// conserved it (the tests check it did).
    pub fn water_balance(&self) -> i64 {
        self.water_in as i64 - self.water_out as i64 + self.water_placed as i64 - self.water_removed as i64 - self.water_lost as i64
    }
}

impl SimStats {
    pub fn avg_us(&self) -> f64 {
        if self.recent_us.is_empty() {
            return 0.0;
        }
        self.recent_us.iter().map(|&u| u as f64).sum::<f64>() / self.recent_us.len() as f64
    }
    fn record(&mut self, us: u32) {
        self.ticks += 1;
        self.last_us = us;
        self.max_us = self.max_us.max(us);
        self.recent_us.push_back(us);
        while self.recent_us.len() > COST_WINDOW {
            self.recent_us.pop_front();
        }
    }
}

pub struct Sim {
    pub config: SimConfig,
    tick: u64,
    /// (due tick, position), drained in order up to the current tick.
    queue: BTreeSet<(u64, PosKey)>,
    /// Updates whose chunk was outside the active region when they came
    /// due: one per cell (its earliest due tick), since a frozen update
    /// runs when its chunk wakes whatever its due tick was, and a cell at
    /// the region's edge beside flowing water is notified every tick.
    frozen: BTreeMap<ChunkKey, BTreeMap<PosKey, u64>>,
    /// Sorted by (column, height) after every tick.
    falling: Vec<Falling>,
    active: FastSet<ChunkKey>,
    focus_chunk: Option<ChunkKey>,
    /// The exact position the active region was last computed for: the
    /// region depends on where in its chunk the player stood when it was
    /// computed (review 3 #4), so it is saved, hashed and restored.
    focus_pos: Option<DVec3>,
    /// A restored focus: the next tick computes the region here first,
    /// as it was when the state was saved.
    pending_focus: Option<DVec3>,
    /// `active` is a region computed since `new` or `restore`, so the next
    /// recompute can tell which chunks joined (`wake_new_pairs`).
    region_known: bool,
    ghosts: Vec<(Falling, u64)>,
    /// Queue, frozen or falling changed since the last snapshot.
    dirty: bool,
    pub stats: SimStats,
    /// The water rule's tick-local writes (water.rs).
    overlay: Overlay,
    /// Tests inject what the boundary holds (a generated river's levels
    /// before M2-hydro produces them).
    #[cfg(test)]
    pub boundary_override: Option<Box<dyn Fn(BlockPos) -> Option<Boundary>>>,
}

impl Sim {
    pub fn new(config: SimConfig) -> Self {
        Self {
            config,
            tick: 0,
            queue: BTreeSet::new(),
            frozen: BTreeMap::new(),
            falling: Vec::new(),
            active: FastSet::default(),
            focus_chunk: None,
            focus_pos: None,
            pending_focus: None,
            region_known: false,
            ghosts: Vec::new(),
            dirty: false,
            stats: SimStats::default(),
            overlay: Overlay::default(),
            #[cfg(test)]
            boundary_override: None,
        }
    }

    pub fn from_snapshot(config: SimConfig, snap: Snapshot) -> Self {
        let mut s = Self::new(config);
        s.restore(snap);
        s
    }

    /// Replace the state with a saved one (a reload). The active region is
    /// computed on the next tick where the saved one was (a version-1 state
    /// has no focus: there, around the player). The settings stay the
    /// caller's (`from_snapshot`'s config): what the command line or the
    /// running session says.
    pub fn restore(&mut self, snap: Snapshot) {
        self.tick = snap.tick;
        self.queue = snap.queue.into_iter().collect();
        self.frozen = BTreeMap::new();
        for (ck, es) in snap.frozen {
            let cells = self.frozen.entry(ck).or_default();
            for (due, key) in es {
                cells.entry(key).and_modify(|d| *d = (*d).min(due)).or_insert(due);
            }
        }
        self.falling = snap.falling;
        self.falling.sort_by_key(Falling::sort_key);
        self.active.clear();
        self.focus_chunk = None;
        self.focus_pos = None;
        self.pending_focus = snap.focus;
        self.region_known = false;
        self.ghosts.clear();
        self.overlay = Overlay::default();
        self.dirty = false;
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn falling(&self) -> &[Falling] {
        &self.falling
    }
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
    pub fn frozen_count(&self) -> usize {
        self.frozen.values().map(|s| s.len()).sum()
    }
    pub fn active_chunks(&self) -> usize {
        self.active.len()
    }
    /// Is this chunk simulated right now?
    pub fn is_active(&self, key: ChunkKey) -> bool {
        self.active.contains(&key)
    }
    /// State changed since the last snapshot (saves use this).
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn has_state(&self) -> bool {
        !(self.queue.is_empty() && self.frozen.is_empty() && self.falling.is_empty())
    }

    pub fn set_radius(&mut self, radius_m: f64) {
        self.config.radius_m = radius_m;
        self.focus_chunk = None;
        self.pending_focus = None;
    }

    /// Where the active region was (or, restored, will be) computed. None:
    /// it is computed on the next tick from wherever the player is then.
    pub fn focus(&self) -> Option<DVec3> {
        self.pending_focus.or(self.focus_chunk.and(self.focus_pos))
    }

    /// A cheap digest of the tick state for the replay's periodic checks
    /// (M2c), and the queue length it saw: the tick, the configuration,
    /// the active region's focus, the queue, the frozen updates and the
    /// falling blocks. It changes nothing, and it counts the changes still
    /// waiting in the world's log as the updates they become (due next
    /// tick, exactly as `absorb` schedules them), so it does not depend on
    /// when they are absorbed: a save absorbs them early, a replay that
    /// does not save absorbs them at the next tick, and both hash the
    /// same. The edited chunks are left out; the replay digests them chunk
    /// by chunk.
    pub fn state_digest(&self, world: &World) -> (u64, usize) {
        let changed = world.edits.take_changed();
        world.edits.push_changed(changed.iter().copied());
        let mut virtual_queue;
        let queue = if changed.is_empty() {
            &self.queue
        } else {
            virtual_queue = self.queue.clone();
            let due = self.tick + 1;
            for bp in changed {
                virtual_queue.insert((due, PosKey::of(bp)));
                for n in world.neighbours(bp) {
                    virtual_queue.insert((due, PosKey::of(n)));
                }
            }
            &virtual_queue
        };
        let mut h = crate::replay::Digest::new(b"sim-state");
        h.u64(self.tick);
        h.u64(self.config.radius_m.to_bits());
        h.u64(self.config.enabled as u64);
        h.u64(self.config.water_period);
        match self.focus() {
            Some(p) => {
                h.u64(1);
                h.u64(p.x.to_bits());
                h.u64(p.y.to_bits());
                h.u64(p.z.to_bits());
            }
            None => h.u64(0),
        }
        h.u64(queue.len() as u64);
        for (due, key) in queue {
            h.u64(*due);
            h.u64(key.0);
        }
        h.u64(self.frozen.len() as u64);
        for (ck, es) in &self.frozen {
            h.bytes(&[ck.body, ck.face, ck.lod]);
            h.u64(ck.cx as u64);
            h.u64(ck.cy as u64);
            h.u64(ck.cz as u64);
            h.u64(es.len() as u64);
            for (key, due) in es {
                h.u64(*due);
                h.u64(key.0);
            }
        }
        let mut falling = self.falling.clone();
        falling.sort_by_key(Falling::sort_key);
        h.u64(falling.len() as u64);
        for f in &falling {
            h.u64(f.id as u64);
            h.u64(f.face as u64);
            h.u64(f.i as u64);
            h.u64(f.j as u64);
            h.u64(f.z.to_bits());
            h.u64(f.vel.to_bits());
        }
        (h.finish(), queue.len())
    }

    /// The blocks to draw: those in flight plus, briefly, those that just
    /// landed (their chunk is being remeshed).
    pub fn view(&self) -> Vec<Falling> {
        let mut v = self.falling.clone();
        v.extend(self.ghosts.iter().map(|(f, _)| *f));
        v
    }

    /// One line for the HUD and the harness transcript.
    pub fn describe(&self) -> String {
        format!(
            "sim {}  tick {}  falling {}  queued {}  frozen {}  active {} chunks  water {} updates, {} moved, in {} out {}  tick cost {:.3} ms (avg {:.3}, max {:.3})",
            if self.config.enabled { "on" } else { "off" },
            self.tick,
            self.falling.len(),
            self.queue.len(),
            self.frozen_count(),
            self.active.len(),
            self.stats.water_updates_last,
            self.stats.water_moved,
            self.stats.water_in,
            self.stats.water_out,
            self.stats.last_us as f64 / 1e3,
            self.stats.avg_us() / 1e3,
            self.stats.max_us as f64 / 1e3
        )
    }

    /// Schedule an update for a block.
    pub fn schedule(&mut self, bp: BlockPos, due: u64) {
        if self.queue.insert((due, PosKey::of(bp))) {
            self.dirty = true;
        }
    }

    /// Drain the world's change log: every changed block and its six
    /// neighbours get an update at `due`.
    fn absorb(&mut self, world: &World, due: u64) {
        for bp in world.edits.take_changed() {
            self.schedule(bp, due);
            for n in world.neighbours(bp) {
                self.schedule(n, due);
            }
        }
    }

    /// Everything the tick state owes to pending changes, then the state:
    /// what a save writes. Changes logged since the last tick are due at
    /// the next one, exactly as `step` would schedule them.
    pub fn snapshot(&mut self, world: &World) -> Snapshot {
        self.absorb(world, self.tick + 1);
        self.dirty = false;
        let mut falling = self.falling.clone();
        falling.sort_by_key(Falling::sort_key);
        Snapshot {
            tick: self.tick,
            queue: self.queue.iter().copied().collect(),
            frozen: self.frozen.iter().map(|(k, v)| (*k, v.iter().map(|(key, due)| (*due, *key)).collect())).collect(),
            falling,
            config: Some(self.config),
            focus: self.focus(),
            rules: Some(rules_digest(&world.registry)),
        }
    }

    /// SHA-256 over everything that decides the future (review 3 #4): the
    /// tick, the settings (radius, on or off, water period), where the
    /// active region was computed, the block rules (`rules_digest`; not
    /// the colours), the queue, the frozen updates, the falling blocks,
    /// and the content of every edited chunk that differs from the
    /// generator's (review 3 #6: content, not history, so a chunk dug and
    /// filled back hashes as untouched). Two simulations that hash the same
    /// run the same from here. Pending (unabsorbed) changes are not part of
    /// it; call it after a tick or a snapshot.
    pub fn state_hash(&self, world: &World) -> String {
        let edits = &world.edits;
        let mut h = Sha256::new();
        h.update(b"triangulum-sim-state-2");
        h.update(&self.tick.to_le_bytes());
        h.update(&self.config.radius_m.to_bits().to_le_bytes());
        h.update(&[self.config.enabled as u8]);
        h.update(&self.config.water_period.to_le_bytes());
        match self.focus() {
            Some(p) => {
                h.update(&[1]);
                for x in [p.x, p.y, p.z] {
                    h.update(&x.to_bits().to_le_bytes());
                }
            }
            None => h.update(&[0]),
        }
        // M3b: the rules of the blocks this state can hold, the generator's
        // and any later block an edit or a fall holds. A block appended to
        // blocks.json and not used yet decides nothing, so adding one
        // changes no hash (a replay recorded before still matches).
        let used = edits.keys().into_iter().filter_map(|k| edits.with_chunk(k, |ids| ids.iter().copied().max()).flatten()).chain(self.falling.iter().map(|f| f.id));
        let used = used.max().unwrap_or(0).max(world.generator.max_block_id());
        h.update(&rules_digest_upto(&world.registry, used as usize + 1));
        let entries = |h: &mut Sha256, es: &BTreeSet<(u64, PosKey)>| {
            h.update(&(es.len() as u64).to_le_bytes());
            for (due, key) in es {
                h.update(&due.to_le_bytes());
                h.update(&key.0.to_le_bytes());
            }
        };
        entries(&mut h, &self.queue);
        h.update(&(self.frozen.len() as u64).to_le_bytes());
        for (ck, es) in &self.frozen {
            h.update(&[ck.body, ck.face, ck.lod]);
            h.update(&ck.cx.to_le_bytes());
            h.update(&ck.cy.to_le_bytes());
            h.update(&ck.cz.to_le_bytes());
            h.update(&(es.len() as u64).to_le_bytes());
            for (key, due) in es {
                h.update(&due.to_le_bytes());
                h.update(&key.0.to_le_bytes());
            }
        }
        let mut falling = self.falling.clone();
        falling.sort_by_key(Falling::sort_key);
        h.update(&(falling.len() as u64).to_le_bytes());
        for f in &falling {
            h.update(&f.id.to_le_bytes());
            h.update(&[f.face]);
            h.update(&f.i.to_le_bytes());
            h.update(&f.j.to_le_bytes());
            h.update(&f.z.to_bits().to_le_bytes());
            h.update(&f.vel.to_bits().to_le_bytes());
        }
        // Only chunks whose content differs from the generator's: an edited
        // chunk equal to its natural one is the same world. Whether it is
        // is kept per chunk until the chunk changes (the comparison
        // regenerates the natural chunk), and no chunk is copied (review 2
        // #17: this cloned every edited chunk, 96 KiB each).
        let no_water = |l: Option<&[u8]>| l.is_none_or(|v| v.iter().all(|&x| x == 0));
        let mut differing: Vec<ChunkKey> = Vec::new();
        let generator = world.id();
        edits.for_each_sorted(|key, e| {
            let equal = e.natural_eq(generator).unwrap_or_else(|| {
                let n = world.natural(key);
                let ids_equal = (0..crate::world::chunk::VOXELS).all(|v| e.id(v) == n.get(v / 1024, (v / 32) % 32, v % 32));
                let nw = n.water_dense();
                let equal = ids_equal
                    && match (no_water(e.water.as_deref()), no_water(nw.as_deref())) {
                        (true, true) => true,
                        (false, false) => e.water == nw,
                        _ => false,
                    };
                e.set_natural_eq(generator, equal);
                equal
            });
            if !equal {
                differing.push(key);
            }
        });
        h.update(&(differing.len() as u64).to_le_bytes());
        let mut buf = Vec::with_capacity(crate::world::chunk::VOXELS * 2);
        for ck in differing {
            edits.with_edited(ck, |e| {
                h.update(&[ck.body, ck.face, ck.lod]);
                h.update(&ck.cx.to_le_bytes());
                h.update(&ck.cy.to_le_bytes());
                h.update(&ck.cz.to_le_bytes());
                buf.clear();
                for v in 0..crate::world::chunk::VOXELS {
                    buf.extend_from_slice(&e.id(v).to_le_bytes());
                }
                h.update(&buf);
                match &e.water {
                    Some(levels) if !no_water(Some(levels)) => {
                        h.update(&[1]);
                        h.update(levels);
                    }
                    _ => h.update(&[0]),
                }
            });
        }
        hex(&h.finish())
    }

    /// Recompute the active region when the player has moved to another
    /// chunk, wake the frozen updates of chunks that are active now, and
    /// (review 3 #3) wake both sides of every neighbour pair the change of
    /// membership put inside the region (`wake_new_pairs`). The first
    /// region after `new` or `restore` has nothing to compare with: a
    /// restored state gets back the region it was saved with, and nothing
    /// in it ran under other boundaries since.
    fn refocus(&mut self, world: &World, focus: DVec3) {
        let planet = &world.planet;
        let here = World::chunk_of(planet.block_of_pos(focus)).0;
        if self.focus_chunk == Some(here) {
            return;
        }
        let t0 = Instant::now();
        self.focus_chunk = Some(here);
        self.focus_pos = Some(focus);
        let previous = std::mem::take(&mut self.active);
        let r = self.config.radius_m;
        let c = CHUNK as i64;
        let span = (r / (c as f64 * planet.block_size)).ceil() as i64 + 1;
        let per_col = planet.chunks_per_column(0) as i64;
        for dz in -span..=span {
            let cz = here.cz as i64 + dz;
            if cz < 0 || cz >= per_col {
                continue;
            }
            for dx in -span..=span {
                for dy in -span..=span {
                    if ((dx * dx + dy * dy + dz * dz) as f64).sqrt() > span as f64 + 0.75 {
                        continue;
                    }
                    // The chunk's centre column in this face's lattice; past a
                    // side it lands on the neighbouring face's own chunk.
                    let ci = (here.cx as i64 + dx) * c + c / 2;
                    let cj = (here.cy as i64 + dy) * c + c / 2;
                    let (f, wi, wj) = planet.wrap_cell(here.face, ci, cj);
                    let key = ChunkKey::neisor(f, 0, wi / c as u32, wj / c as u32, cz as u32);
                    let b = planet.chunk_bounds(key);
                    if b.centre.distance(focus) <= r + b.radius {
                        self.active.insert(key);
                    }
                }
            }
        }
        // The chunks that joined. Frozen updates only ever sit under
        // chunks outside the region, so only these can have any (review 2
        // #17: this used to scan every frozen chunk on the planet).
        let mut joined: Vec<ChunkKey> = self.active.iter().filter(|k| !previous.contains(k)).copied().collect();
        joined.sort_unstable();
        for k in &joined {
            if let Some(es) = self.frozen.remove(k) {
                for (key, _) in es {
                    self.queue.insert((self.tick, key));
                }
                self.dirty = true;
            }
        }
        if self.region_known {
            self.wake_new_pairs(world, &joined);
        }
        self.region_known = true;
        self.stats.refocus_us = t0.elapsed().as_micros() as u32;
        self.stats.refocuses += 1;
    }

    /// Review 3 #3. A cell beside a chunk outside the region ran its rule
    /// against the boundary (`Sim::boundary_cell`): air there is a wall, and
    /// the outside is never written. When that chunk joins the region the
    /// pair is under the full rule, but only the outside cell had an update
    /// waiting (frozen), and an empty cell cannot pull water: water that had
    /// come to rest against the old edge stayed there, an invisible dam.
    /// So for every pair of face neighbours across the side of a chunk that
    /// joined, both now in the region, both cells are scheduled now when
    /// the pair could move water (both replaceable, and their levels differ
    /// or the upper one could fall into the lower) and at least one of the
    /// two is not as the generator made it. A pair exactly as generated is
    /// left alone: an untouched river stays held at its generated levels
    /// (DESIGN.md) however the region moves, and a side between two
    /// unedited chunks is skipped without looking at its cells.
    fn wake_new_pairs(&mut self, world: &World, joined: &[ChunkKey]) {
        if joined.is_empty() || world.edits.is_empty() {
            return;
        }
        let reg = world.registry.clone();
        let c = CHUNK as i64;
        let joined_set: FastSet<ChunkKey> = joined.iter().copied().collect();
        let mut woken = 0u64;
        // What a chunk's side can hold, without generating it where the
        // generator's footprint bounds already say: an unedited chunk that
        // is all rock has nothing to move, one that is all air can only
        // take water from the other side (generating a natural chunk costs
        // about 1.7 ms; the bounds about 80 us per column of chunks).
        #[derive(Clone, Copy, PartialEq)]
        enum Holds {
            Edited { wet: bool },
            Air,
            Rock,
            Natural,
        }
        let mut bounds: FastMap<crate::grid::FootprintKey, (f64, f64)> = FastMap::default();
        let mut holds = |k: ChunkKey| -> Holds {
            if let Some(wet) = world.edits.with_edited(k, |e| e.wet_cells() > 0) {
                return Holds::Edited { wet };
            }
            let range = *bounds.entry(k.footprint()).or_insert_with(|| world.generator.footprint_height_range_m(k.footprint()));
            match world.generator.classify_in(k, range) {
                crate::worldgen::ChunkClass::Air => Holds::Air,
                crate::worldgen::ChunkClass::Enclosed => Holds::Rock,
                crate::worldgen::ChunkClass::Mixed => Holds::Natural,
            }
        };
        for &b in joined {
            let (i0, j0, k0) = b.origin_blocks();
            let (i0, j0, k0) = (i0 as i64, j0 as i64, k0 as i64);
            // The six sides: a step (di, dj, dk) and the cells of b on that side.
            for (di, dj, dk) in [(-1i64, 0i64, 0i64), (1, 0, 0), (0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1)] {
                let side_cell = |u: i64, v: i64| -> (i64, i64, i64) {
                    let at = |d: i64, a: i64| if d < 0 { 0 } else if d > 0 { c - 1 } else { a };
                    if di != 0 {
                        (at(di, 0), u, v)
                    } else if dj != 0 {
                        (u, at(dj, 0), v)
                    } else {
                        (u, v, at(dk, 0))
                    }
                };
                // The chunk across this side (through a face edge if need be).
                let (x, y, z) = side_cell(c / 2, c / 2);
                let Some(probe) = world.resolve_block(b.face, i0 + x + di, j0 + y + dj, k0 + z + dk) else { continue };
                let n = World::chunk_of(probe).0;
                if !self.active.contains(&n) {
                    continue;
                }
                // A side between two chunks that both joined is seen from both: once.
                if joined_set.contains(&n) && n < b {
                    continue;
                }
                // Both as generated (the common case): held, and nothing to look at.
                if !world.edits.contains(b) && !world.edits.contains(n) {
                    continue;
                }
                let (hb, hn) = (holds(b), holds(n));
                match (hb, hn) {
                    // Rock on one side: nothing on this side is replaceable.
                    (Holds::Rock, _) | (_, Holds::Rock) => continue,
                    // Both as generated: held, whatever the water does there.
                    (Holds::Air | Holds::Natural, Holds::Air | Holds::Natural) => continue,
                    // Air beside a dry edited chunk: nothing to move.
                    (Holds::Air, Holds::Edited { wet: false }) | (Holds::Edited { wet: false }, Holds::Air) => continue,
                    _ => {}
                }
                // An all-air side is read as air without generating it. With
                // one side edited and the other as generated, a pair to wake
                // needs a changed cell on the edited side, so that side is
                // read first and the other side's natural chunk is generated
                // only when there is one.
                let air = (world.generator.blocks.air, 0u8);
                let read = |bp: BlockPos, all_air: bool| if all_air { air } else { world.block_and_level_at(bp) };
                let (b_air, n_air) = (hb == Holds::Air, hn == Holds::Air);
                let (b_ed, n_ed) = (matches!(hb, Holds::Edited { .. }), matches!(hn, Holds::Edited { .. }));
                for u in 0..c {
                    for v in 0..c {
                        let (x, y, z) = side_cell(u, v);
                        let p = BlockPos { face: b.face, i: (i0 + x) as u32, j: (j0 + y) as u32, k: (k0 + z) as u32 };
                        let Some(q) = world.resolve_block(b.face, i0 + x + di, j0 + y + dj, k0 + z + dk) else { continue };
                        if !self.active.contains(&World::chunk_of(q).0) {
                            continue;
                        }
                        // The changed side of the pair, then the other.
                        let (first, second, first_air, second_air) = if b_ed { (p, q, b_air, n_air) } else { (q, p, n_air, b_air) };
                        let fc = read(first, first_air);
                        if !reg.is_replaceable(fc.0) {
                            continue;
                        }
                        let first_natural = world.natural_cell(first) == fc;
                        if first_natural && !(b_ed && n_ed) {
                            continue; // the other side is unedited: the pair is as generated
                        }
                        let sc = read(second, second_air);
                        if !reg.is_replaceable(sc.0) {
                            continue;
                        }
                        let ((_, pl), (_, ql)) = if b_ed { (fc, sc) } else { (sc, fc) };
                        let (upper, lower) = if dk > 0 { (ql, pl) } else { (pl, ql) };
                        let moves = pl != ql || (dk != 0 && upper > 0 && lower < water::FULL);
                        if !moves || (first_natural && world.natural_cell(second) == sc) {
                            continue;
                        }
                        self.queue.insert((self.tick, PosKey::of(p)));
                        self.queue.insert((self.tick, PosKey::of(q)));
                        woken += 1;
                    }
                }
            }
        }
        if woken > 0 {
            self.dirty = true;
            self.stats.pairs_woken += woken;
        }
    }

    /// One tick: absorb changes, advance the clock, drain due updates in
    /// (due, position) order, move the falling blocks, absorb again.
    pub fn step(&mut self, world: &World, focus: DVec3) {
        let t0 = Instant::now();
        let next = self.tick + 1;
        self.absorb(world, next);
        self.tick = next;
        self.ghosts.retain(|(_, t)| next - *t < GHOST_TICKS);
        // A restored state first gets back the region it was saved with.
        if let Some(p) = self.pending_focus.take() {
            self.refocus(world, p);
        }
        self.refocus(world, focus);

        let upto = (self.tick, PosKey(u64::MAX));
        let due: Vec<(u64, PosKey)> = self.queue.range(..=upto).copied().collect();
        if !due.is_empty() {
            self.dirty = true;
        }
        for e in &due {
            self.queue.remove(e);
        }
        // Sort the due, active cells by rule: blocks with gravity fall first
        // (their removals are visible to the water rule in the same tick);
        // replaceable cells (air and water) run the water rule. The gravity
        // pass keeps ascending order (a column notified at once starts
        // together, bottom first).
        let reg = world.registry.clone();
        let mut gravity: Vec<BlockPos> = Vec::new();
        let mut water: Vec<(u64, PosKey)> = Vec::new();
        for (d, key) in due {
            let bp = key.pos();
            let (ck, ..) = World::chunk_of(bp);
            if !self.active.contains(&ck) {
                self.frozen.entry(ck).or_default().entry(key).and_modify(|due| *due = (*due).min(d)).or_insert(d);
                continue;
            }
            self.stats.processed += 1;
            let id = world.block_at(bp);
            if reg.has_gravity(id) {
                gravity.push(bp);
            } else if reg.is_replaceable(id) {
                water.push((d, key));
            }
        }
        for bp in gravity {
            self.fall_rule(world, bp);
        }
        self.stats.water_updates_last = 0;
        self.stats.water_rule_us = 0;
        self.stats.water_commit_us = 0;
        if !water.is_empty() {
            let period = self.config.water_period.max(1);
            if self.tick % period == 0 {
                // Pass 1, bottom-up: falls. Pass 2, alternating direction
                // every water step: spreads (water.rs).
                let t1 = Instant::now();
                self.stats.water_updates_last = water.len() as u32;
                for &(_, key) in &water {
                    self.water_fall(world, key.pos());
                }
                if (self.tick / period) % 2 == 1 {
                    water.reverse();
                }
                for &(_, key) in &water {
                    self.water_spread(world, key.pos());
                }
                let t2 = Instant::now();
                self.commit_water(world);
                self.stats.water_rule_us = (t2 - t1).as_micros() as u32;
                self.stats.water_commit_us = t2.elapsed().as_micros() as u32;
            } else {
                // Not a water step: wait for the next one.
                let next = (self.tick / period + 1) * period;
                for (_, key) in water {
                    self.queue.insert((next, key));
                }
            }
        }
        self.advance_falling(world);
        let t3 = Instant::now();
        self.absorb(world, self.tick + 1);
        self.stats.absorb_us = t3.elapsed().as_micros() as u32;
        self.stats.record(t0.elapsed().as_micros() as u32);
    }

    /// The gravity rule, driven by block flags: a block with `gravity` over
    /// a replaceable cell leaves the grid and falls.
    fn fall_rule(&mut self, world: &World, bp: BlockPos) {
        let reg = &world.registry;
        let id = world.block_at(bp);
        if !reg.has_gravity(id) {
            return;
        }
        // Layer 0 rests on the floor of the world.
        if bp.k == 0 {
            return;
        }
        let below = world.block_at(BlockPos { k: bp.k - 1, ..bp });
        if reg.is_replaceable(below) && world.set_block(bp, world.generator.blocks.air) {
            self.falling.push(Falling { id, face: bp.face, i: bp.i, j: bp.j, z: bp.k as f64, vel: 0.0 });
            self.stats.falls += 1;
            self.dirty = true;
        }
    }

    fn advance_falling(&mut self, world: &World) {
        if self.falling.is_empty() {
            return;
        }
        let planet = &world.planet;
        let reg = &world.registry;
        let g = GRAVITY_M_S2 / planet.block_size;
        let terminal = TERMINAL_M_S / planet.block_size;
        let height = planet.height as i64;
        let mut list = std::mem::take(&mut self.falling);
        list.sort_by_key(Falling::sort_key);
        let mut keep = Vec::with_capacity(list.len());
        // Landings in one column are written as they happen, so the next block
        // up in the same column lands on top of them; everything else goes
        // into one batch commit at the end of the tick. A landing in water
        // pushes the water up the column (`World::displace_writes`), in the
        // same batch.
        let mut landings: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
        let mut landed_in: Option<PosKey> = None;
        for mut f in list {
            let (face, i, j) = (f.face, f.i, f.j);
            let k0 = f.cell().clamp(0, height - 1);
            let ck = World::chunk_of(BlockPos { face, i, j, k: k0 as u32 }).0;
            if !self.active.contains(&ck) {
                keep.push(f);
                continue;
            }
            if landed_in.is_some_and(|c| c == f.column()) {
                // A block below in this column landed this tick: make it visible to our checks.
                world.commit_levels(&landings);
                landings.clear();
            }
            landed_in = None;
            let free = |k: i64| reg.is_replaceable(world.block_at(BlockPos { face, i, j, k: k as u32 }));
            // Review 3 #7: a falling block never changes a chunk outside the
            // region. Its sweep stops at the region's floor, its search for a
            // free cell above stops at the region's top, and a landing whose
            // displaced water would have to rise out of the region waits; a
            // block that waits is left exactly as it was (it moves on when
            // the region reaches further, as a frozen update does).
            let inside = |k: i64| self.active.contains(&World::chunk_of(BlockPos { face, i, j, k: k as u32 }).0);
            // Something was built into the cell we are in: settle in the first free cell above.
            if !free(k0) {
                match (k0 + 1..height).find(|&k| !inside(k) || free(k)) {
                    Some(k) if inside(k) => {
                        if self.land_into(world, f, k, &mut landings) {
                            landed_in = Some(f.column());
                            self.dirty = true;
                        } else {
                            keep.push(f);
                        }
                    }
                    Some(_) => keep.push(f),
                    None => {
                        self.stats.vanished += 1;
                        self.dirty = true;
                    }
                }
                continue;
            }
            let vel = (f.vel - g * TICK_S).max(-terminal);
            let z1 = f.z + vel * TICK_S;
            let mut k = k0;
            let mut landed = None;
            let mut floor = false;
            while (k as f64) > z1 {
                if k == 0 || !free(k - 1) {
                    landed = Some(k);
                    break;
                }
                if !inside(k - 1) {
                    floor = true;
                    break;
                }
                k -= 1;
            }
            match landed {
                Some(k) => {
                    if self.land_into(world, Falling { vel, ..f }, k, &mut landings) {
                        landed_in = Some(f.column());
                        self.dirty = true;
                    } else {
                        keep.push(f);
                    }
                }
                None if floor => {
                    // At the bottom of the lowest cell in the region.
                    if f.z != k as f64 {
                        f.z = k as f64;
                        f.vel = vel;
                        self.dirty = true;
                    }
                    keep.push(f);
                }
                None => {
                    f.z = z1;
                    f.vel = vel;
                    keep.push(f);
                    self.dirty = true;
                }
            }
        }
        if !landings.is_empty() {
            world.commit_levels(&landings);
        }
        self.falling = keep;
    }

    /// Record a landing: the block's write and, when it lands in water, the
    /// displaced water's writes above it (batched by the caller). False,
    /// and nothing recorded, when the displaced water would have to rise
    /// into a chunk outside the region (review 3 #7): the block waits.
    fn land_into(&mut self, world: &World, f: Falling, k: i64, batch: &mut Vec<(BlockPos, BlockId, Option<u8>)>) -> bool {
        let bp = BlockPos { face: f.face, i: f.i, j: f.j, k: k as u32 };
        let Some((displaced, lost)) = world.displace_writes_within(bp, |b| self.active.contains(&World::chunk_of(b).0)) else { return false };
        self.stats.landings += 1;
        self.ghosts.push((Falling { z: k as f64, vel: 0.0, ..f }, self.tick));
        let level = world.water_level_at(bp) as u64;
        self.stats.water_displaced += level - lost as u64;
        self.stats.water_lost += lost as u64;
        batch.push((bp, f.id, None));
        batch.extend(displaced);
        true
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod water_tests;

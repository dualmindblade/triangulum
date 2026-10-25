//! The simulation core (M2a): a fixed 20 Hz tick, a deterministic
//! block-update queue with neighbour notifications across chunk borders and
//! cube-face edges, an active region around the player, and the first rule
//! that runs on it: falling blocks (sand and gravel, by flag).
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
//! when that chunk is active again; it is never dropped and it is saved. A
//! falling block outside the region hangs where it is. Rendering never
//! decides any of this: the region is a function of the player's position.
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

use crate::grid::{BlockPos, CHUNK, ChunkKey, FastSet};
use crate::sha256::{Sha256, hex};
use crate::world::{BlockId, EditStore, World};
use glam::DVec3;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

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
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { radius_m: 112.0, enabled: true }
    }
}

/// The canonical position key the queue sorts by: face, i, j, k packed so
/// that integer order is lexicographic order. N up to 2^20 columns, H up to
/// 2^16 layers.
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
}

const SNAPSHOT_MAGIC: &[u8; 4] = b"TRSM";
const SNAPSHOT_VERSION: u32 = 1;

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
        w
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, String> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != SNAPSHOT_MAGIC {
            return Err("not a simulation state file".into());
        }
        let version = r.u32()?;
        if version != SNAPSHOT_VERSION {
            return Err(format!("simulation state version {version}, this build reads {SNAPSHOT_VERSION}"));
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
        if r.at != b.len() {
            return Err(format!("{} trailing bytes after simulation state", b.len() - r.at));
        }
        Ok(Self { tick, queue, frozen, falling })
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
    /// Updates whose chunk was outside the active region when they came due.
    frozen: BTreeMap<ChunkKey, BTreeSet<(u64, PosKey)>>,
    /// Sorted by (column, height) after every tick.
    falling: Vec<Falling>,
    active: FastSet<ChunkKey>,
    focus_chunk: Option<ChunkKey>,
    ghosts: Vec<(Falling, u64)>,
    /// Queue, frozen or falling changed since the last snapshot.
    dirty: bool,
    pub stats: SimStats,
}

impl Sim {
    pub fn new(config: SimConfig) -> Self {
        Self { config, tick: 0, queue: BTreeSet::new(), frozen: BTreeMap::new(), falling: Vec::new(), active: FastSet::default(), focus_chunk: None, ghosts: Vec::new(), dirty: false, stats: SimStats::default() }
    }

    pub fn from_snapshot(config: SimConfig, snap: Snapshot) -> Self {
        let mut s = Self::new(config);
        s.restore(snap);
        s
    }

    /// Replace the state with a saved one (a reload). The active region is
    /// recomputed on the next tick.
    pub fn restore(&mut self, snap: Snapshot) {
        self.tick = snap.tick;
        self.queue = snap.queue.into_iter().collect();
        self.frozen = snap.frozen.into_iter().map(|(k, v)| (k, v.into_iter().collect())).collect();
        self.falling = snap.falling;
        self.falling.sort_by_key(Falling::sort_key);
        self.active.clear();
        self.focus_chunk = None;
        self.ghosts.clear();
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
            "sim {}  tick {}  falling {}  queued {}  frozen {}  active {} chunks  tick cost {:.3} ms (avg {:.3}, max {:.3})",
            if self.config.enabled { "on" } else { "off" },
            self.tick,
            self.falling.len(),
            self.queue.len(),
            self.frozen_count(),
            self.active.len(),
            self.stats.last_us as f64 / 1e3,
            self.stats.avg_us() / 1e3,
            self.stats.max_us as f64 / 1e3
        )
    }

    /// The hook M2b's water rule reads boundary cells through: outside the
    /// active region a cell takes the generator's value, so rivers keep
    /// flowing in and out and lakes stay full (DESIGN.md, Water). Whether
    /// an edited cell at the boundary should read its edit instead is
    /// M2b's call; this returns the generator's block.
    pub fn boundary_block(&self, world: &World, bp: BlockPos) -> BlockId {
        let (key, x, y, z) = World::chunk_of(bp);
        world.natural(key).get(x, y, z)
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
            frozen: self.frozen.iter().map(|(k, v)| (*k, v.iter().copied().collect())).collect(),
            falling,
        }
    }

    /// SHA-256 over the tick, the queue, the frozen updates, the falling
    /// blocks and every edited chunk: two simulations in the same state
    /// hash the same. Pending (unabsorbed) changes are not part of it;
    /// call it after a tick or a snapshot.
    pub fn state_hash(&self, edits: &EditStore) -> String {
        let mut h = Sha256::new();
        h.update(b"triangulum-sim-state-1");
        h.update(&self.tick.to_le_bytes());
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
            entries(&mut h, es);
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
        let chunks = edits.snapshot_full();
        h.update(&(chunks.len() as u64).to_le_bytes());
        let mut buf = Vec::with_capacity(crate::world::chunk::VOXELS * 2);
        for c in &chunks {
            let ck = c.key;
            h.update(&[ck.body, ck.face, ck.lod]);
            h.update(&ck.cx.to_le_bytes());
            h.update(&ck.cy.to_le_bytes());
            h.update(&ck.cz.to_le_bytes());
            buf.clear();
            for id in &c.ids {
                buf.extend_from_slice(&id.to_le_bytes());
            }
            h.update(&buf);
            match &c.water {
                Some(levels) => {
                    h.update(&[1]);
                    h.update(levels);
                }
                None => h.update(&[0]),
            }
        }
        hex(&h.finish())
    }

    /// Recompute the active region when the player has moved to another
    /// chunk, and wake the frozen updates of chunks that are active now.
    fn refocus(&mut self, world: &World, focus: DVec3) {
        let planet = &world.planet;
        let here = World::chunk_of(planet.block_of_pos(focus)).0;
        if self.focus_chunk == Some(here) {
            return;
        }
        let t0 = Instant::now();
        self.focus_chunk = Some(here);
        self.active.clear();
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
        let woken: Vec<ChunkKey> = self.frozen.keys().filter(|k| self.active.contains(k)).copied().collect();
        for k in woken {
            if let Some(es) = self.frozen.remove(&k) {
                for (_, key) in es {
                    self.queue.insert((self.tick, key));
                }
                self.dirty = true;
            }
        }
        self.stats.refocus_us = t0.elapsed().as_micros() as u32;
        self.stats.refocuses += 1;
    }

    /// One tick: absorb changes, advance the clock, drain due updates in
    /// (due, position) order, move the falling blocks, absorb again.
    pub fn step(&mut self, world: &World, focus: DVec3) {
        let t0 = Instant::now();
        let next = self.tick + 1;
        self.absorb(world, next);
        self.tick = next;
        self.ghosts.retain(|(_, t)| next - *t < GHOST_TICKS);
        self.refocus(world, focus);

        let upto = (self.tick, PosKey(u64::MAX));
        let due: Vec<(u64, PosKey)> = self.queue.range(..=upto).copied().collect();
        if !due.is_empty() {
            self.dirty = true;
        }
        for e in &due {
            self.queue.remove(e);
        }
        for (d, key) in due {
            let bp = key.pos();
            let (ck, ..) = World::chunk_of(bp);
            if self.active.contains(&ck) {
                self.apply(world, bp);
                self.stats.processed += 1;
            } else {
                self.frozen.entry(ck).or_default().insert((d, key));
            }
        }
        self.advance_falling(world);
        self.absorb(world, self.tick + 1);
        self.stats.record(t0.elapsed().as_micros() as u32);
    }

    /// The rules, driven by block flags. Water (M2b) plugs in here as
    /// another flag-driven rule, reading cells outside the active region
    /// through `boundary_block`.
    fn apply(&mut self, world: &World, bp: BlockPos) {
        let reg = &world.registry;
        let id = world.block_at(bp);
        if reg.has_gravity(id) {
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
        // into one batch commit at the end of the tick.
        let mut landings: Vec<(BlockPos, BlockId)> = Vec::new();
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
                world.commit(&landings);
                landings.clear();
            }
            landed_in = None;
            let free = |k: i64| reg.is_replaceable(world.block_at(BlockPos { face, i, j, k: k as u32 }));
            // Something was built into the cell we are in: settle in the first free cell above.
            if !free(k0) {
                match (k0 + 1..height).find(|&k| free(k)) {
                    Some(k) => {
                        landings.push(self.land(f, k));
                        landed_in = Some(f.column());
                    }
                    None => self.stats.vanished += 1,
                }
                self.dirty = true;
                continue;
            }
            f.vel = (f.vel - g * TICK_S).max(-terminal);
            let z1 = f.z + f.vel * TICK_S;
            let mut k = k0;
            let mut landed = None;
            while (k as f64) > z1 {
                if k == 0 || !free(k - 1) {
                    landed = Some(k);
                    break;
                }
                k -= 1;
            }
            match landed {
                Some(k) => {
                    landings.push(self.land(f, k));
                    landed_in = Some(f.column());
                }
                None => {
                    f.z = z1;
                    keep.push(f);
                }
            }
            self.dirty = true;
        }
        if !landings.is_empty() {
            world.commit(&landings);
        }
        self.falling = keep;
    }

    /// Record a landing (the write itself is batched by the caller).
    fn land(&mut self, f: Falling, k: i64) -> (BlockPos, BlockId) {
        self.stats.landings += 1;
        self.ghosts.push((Falling { z: k as f64, vel: 0.0, ..f }, self.tick));
        (BlockPos { face: f.face, i: f.i, j: f.j, k: k as u32 }, f.id)
    }
}

#[cfg(test)]
mod tests;

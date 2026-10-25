//! Streaming: a persistent, non-overlapping coverage tree per face, refined
//! and coarsened as the camera crosses distance thresholds, with complete
//! covers switched atomically; horizon culling against the world's opaque
//! core; job dispatch to rayon workers; and the per-frame draw lists.
//!
//! The tree (`Coverage`) holds one node per chunk it has decided exists:
//! the roots at the coarsest LOD and, under them, children down to wherever
//! the camera has pulled detail. Chunks that lie entirely above or below
//! the terrain band of their footprint are never created, so a taller
//! world adds no chunks. Each root's subtree carries a *cut* of `drawn`
//! nodes: the chunks currently rendered. The cut is complete (every live
//! node is on it, above it or below it) and never overlaps (no drawn node
//! has a drawn ancestor), and every drawn node that is not beyond the
//! horizon is resident. The cut only moves in two ways:
//!
//! - *down*: a drawn node that wants finer detail splits; its children are
//!   dispatched while it keeps drawing; when every child is resident (or
//!   hidden by the horizon) the children become drawn and it stops, in
//!   one step. It keeps its mesh as long as a child has none, so a child
//!   that comes over the horizon unloaded makes the parent draw again
//!   instead of leaving a hole.
//! - *up*: an interior node whose children are drawn leaves and whose
//!   distance says the detail is no longer wanted gets a mesh of its own;
//!   when it arrives the node becomes drawn and the subtree is deleted.
//!   Nothing deeper is ever freed before the coarser mesh that replaces it
//!   is resident.
//!
//! Jobs read blocks through `World` (generator + edits) and carry the
//! node's slot, generation and revision. Edits bump the revision; a result
//! from an older revision is rejected and regenerated, a result for a slot
//! whose node is gone is dropped, so a late generation never overwrites an
//! edit and never resurrects a freed chunk. Caches are bounded by the tree
//! itself: a node's bounds and height range die with it, and the worker
//! timing history is recorded only when the bench asks for it.
//!
//! Whether a chunk exists is worldgen's call: the streamer hands the tree
//! `Generator::classify_in(key, range)`, so Air and Enclosed chunks (all
//! solid, border included) are never nodes; physics still materialises
//! them through `World::generate` when a player stands on or digs into
//! them, and a chunk with edits always counts as existing: an edit to a
//! chunk that is not a node creates it under its nearest split ancestor
//! (`invalidate`), and the tree refines the rest.
//! Footprint height ranges cost hundreds of microseconds each with
//! landforms, so a node that wants to split first asks for its children's
//! footprint ranges (`take_range_wants`, run by workers) and keeps drawing
//! until they are back (`supply_range`); the split itself then costs
//! microseconds. Ranges live in a bounded, oldest-first cache.

use crate::grid::{Bounds, CHUNK, ChunkKey, FastMap, FootprintKey, Planet};
use crate::mesh::{MeshData, MeshOptions, mesh_chunk};
use crate::render::{DrawRecord, GpuMesh, Renderer};
use crate::world::{Registry, World};
use glam::{DVec3, Mat4, Vec3, Vec4};

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, Debug)]
pub struct LodParams {
    /// Subdivide a chunk when the camera is closer than `factor * chunk edge`.
    pub factor: f64,
    pub max_inflight: usize,
}

/// A split node merges back only when the camera is this much further than
/// the split distance, so the cut does not flap at the threshold.
pub const MERGE_HYSTERESIS: f64 = 1.25;

/// A wave of splits in one frame, after a teleport or when a whole level
/// arrives at once, is spread over frames by stopping once this much time
/// has gone into splits. Deferred nodes keep drawing their current mesh
/// and split on a later frame.
pub const SPLIT_BUDGET_S: f64 = 0.001;

/// Meshes that found no room in the arena are kept this long before being
/// dropped (their nodes are dispatched again afterwards).
const MAX_AWAITING_UPLOAD: usize = 512;

pub const NONE: u32 = u32::MAX;

/// A node's mesh residency.
#[derive(Clone, Copy, Debug)]
pub enum Mesh {
    /// Nothing resident.
    Absent,
    /// Generated and found to have no faces: resident, nothing to draw.
    Empty,
    /// Quads and a grid slot on the GPU.
    Gpu(GpuMesh),
}

impl PartialEq for Mesh {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Mesh::Absent, Mesh::Absent) | (Mesh::Empty, Mesh::Empty) => true,
            (Mesh::Gpu(a), Mesh::Gpu(b)) => (a.slot, a.opaque_base, a.opaque_count, a.water_base, a.water_count) == (b.slot, b.opaque_base, b.opaque_count, b.water_base, b.water_count),
            _ => false,
        }
    }
}
impl Eq for Mesh {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    None,
    /// A worker is generating it.
    Inflight,
    /// Finished, waiting for room in the quad arena.
    Awaiting,
}

/// One chunk the tree has decided exists.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub key: ChunkKey,
    pub centre: DVec3,
    pub radius: f32,
    /// Chunk edge in metres.
    pub edge_m: f32,
    /// World position of the lattice corner (from the mesh), for draws.
    pub origin: DVec3,
    pub parent: u32,
    /// Index into `Coverage::kids`, or NONE for a leaf.
    pub kids: u32,
    pub mesh: Mesh,
    pub job: Job,
    /// Bumped by every invalidation; jobs carry the value they started with.
    pub rev: u32,
    /// Slot generation: a result for another generation is for a dead node.
    pub epoch: u32,
    /// Distance from the camera to the bounding sphere (last step).
    pub dist: f32,
    pub live: bool,
    /// On the cut: this chunk is what the renderer shows for its region.
    pub drawn: bool,
    /// Entirely below the horizon of the opaque core (last step).
    pub hidden: bool,
    /// Last policy verdict: finer detail wanted here.
    pub want: bool,
    /// A root whose subtree has drawn something at some point.
    pub ever_drawn: bool,
    /// Created by an edit under an already split parent: a region nothing
    /// rendered before, so not a gap until it has drawn once.
    pub fresh: bool,
}

impl Node {
    #[inline]
    pub fn split(&self) -> bool {
        self.kids != NONE
    }
    #[inline]
    pub fn resident(&self) -> bool {
        self.mesh != Mesh::Absent
    }
    /// Ready to take its place on the cut: resident, or nothing to show yet.
    #[inline]
    fn ready(&self) -> bool {
        self.resident() || self.hidden
    }
}

/// What `Coverage::step` did and found this frame.
#[derive(Default, Clone, Debug)]
pub struct StepStats {
    pub nodes: usize,
    pub drawn: usize,
    pub resident: usize,
    pub hidden: usize,
    pub splits: usize,
    /// Splits wanted but deferred to a later frame by the time budget.
    pub splits_deferred: usize,
    /// Splits waiting for their children's footprint ranges.
    pub splits_waiting: usize,
    /// Footprint ranges computed inline this step (sync mode) and their time.
    pub range_misses: usize,
    pub range_ms: f64,
    /// Time spent splitting this step (classification, bounds).
    pub split_ms: f64,
    /// The step's phases: policy pass, structure pass, counts.
    pub policy_ms: f64,
    pub structure_ms: f64,
    pub counts_ms: f64,
    pub switches: usize,
    pub collapses: usize,
    pub merges: usize,
    pub reverts: usize,
    /// Interior nodes still holding their mesh for an unloaded child.
    pub retained: usize,
    /// Things the tree still wants to change: splits, switches, merges and
    /// meshes pending. Zero means the cover matches the camera.
    pub unsettled: usize,
    pub drawn_per_lod: [u32; 16],
    pub resident_per_lod: [u32; 16],
}

/// The invariants, as counted over the live tree.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct CoverReport {
    /// Drawn nodes with a drawn ancestor.
    pub overlaps: usize,
    /// Visible drawn nodes without a mesh: a region rendering nothing.
    pub gaps: usize,
    /// Visible leaves that no drawn node covers, under a root that has
    /// drawn before (a root waiting for its first mesh is not a gap).
    pub uncovered: usize,
    pub drawn: usize,
    pub nodes: usize,
}

impl CoverReport {
    pub fn ok(&self) -> bool {
        self.overlaps == 0 && self.gaps == 0 && self.uncovered == 0
    }
}

/// A job the tree wants run: generate and mesh `key` for node `slot`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub slot: u32,
    pub epoch: u32,
    pub rev: u32,
    pub key: ChunkKey,
}

/// What became of an installed result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    Ok,
    /// The node changed since the job started; `Want` says what to run now.
    Stale(Want),
    /// The node is gone.
    Dropped,
}

pub type RangeFn = Box<dyn FnMut(FootprintKey) -> (f64, f64) + Send>;
/// Does a chunk hold any surface, given its footprint's height range?
pub type BandFn = Box<dyn FnMut(ChunkKey, (f64, f64)) -> bool + Send>;

/// Footprint ranges remembered (oldest evicted first).
const RANGE_CACHE_MAX: usize = 1 << 16;

pub struct Coverage {
    pub planet: Planet,
    pub factor: f64,
    /// Radius of the sphere nothing can ever see through.
    pub occluder_radius: f64,
    nodes: Vec<Node>,
    kids: Vec<[u32; 8]>,
    free_nodes: Vec<u32>,
    free_kids: Vec<u32>,
    index: FastMap<ChunkKey, u32>,
    /// Computes a footprint's range inline: for the roots, and for every
    /// footprint in sync mode (tests).
    range: RangeFn,
    /// Sync mode: ranges are computed inline instead of requested.
    sync_ranges: bool,
    in_band: BandFn,
    range_cache: FastMap<FootprintKey, (f64, f64)>,
    range_order: std::collections::VecDeque<FootprintKey>,
    /// Requested from the owner and not yet supplied.
    range_pending: FastMap<FootprintKey, ()>,
    range_wants: Vec<FootprintKey>,
    range_misses: usize,
    range_s: f64,
    next_gen: u32,
    /// GPU meshes the tree let go of; the owner frees them.
    pub freed: Vec<GpuMesh>,
    /// Nodes that want a job, filled by `step`, consumed by `take_wants`.
    wants: Vec<u32>,
    stack: Vec<u32>,
    pub stats: StepStats,
    /// Seconds of splitting allowed per step.
    pub split_budget_s: f64,
}

impl Coverage {
    /// A tree with its roots (every in-band chunk at the coarsest LOD).
    /// With `sync_ranges` every footprint range is computed inline through
    /// `range`; otherwise only the roots' are, and the rest are asked for
    /// through `take_range_wants` and answered with `supply_range`.
    pub fn new(planet: Planet, factor: f64, occluder_radius: f64, range: RangeFn, in_band: BandFn, sync_ranges: bool) -> Self {
        let mut c = Self {
            planet,
            factor,
            occluder_radius,
            nodes: Vec::new(),
            kids: Vec::new(),
            free_nodes: Vec::new(),
            free_kids: Vec::new(),
            index: FastMap::default(),
            range,
            sync_ranges,
            in_band,
            range_cache: FastMap::default(),
            range_order: std::collections::VecDeque::new(),
            range_pending: FastMap::default(),
            range_wants: Vec::new(),
            range_misses: 0,
            range_s: 0.0,
            next_gen: 1,
            freed: Vec::new(),
            wants: Vec::new(),
            stack: Vec::new(),
            stats: StepStats::default(),
            split_budget_s: SPLIT_BUDGET_S,
        };
        c.make_roots();
        c
    }

    /// The plain band test: the chunk's radial extent overlaps the
    /// footprint's height band (metres above sea level). The streamer uses
    /// worldgen's `classify_in` instead; tests use this.
    pub fn band_test(planet: Planet) -> BandFn {
        Box::new(move |key: ChunkKey, (hmin, hmax): (f64, f64)| {
            let e_m = key.extent_blocks() as f64 * planet.block_size;
            let z0 = key.cz as f64 * e_m - planet.sea_level as f64 * planet.block_size;
            let z1 = z0 + e_m;
            !(z1 <= hmin || z0 >= hmax)
        })
    }

    /// A footprint's height range, computed inline (roots, sync mode).
    fn range_of(&mut self, fp: FootprintKey) -> (f64, f64) {
        if let Some(r) = self.range_cache.get(&fp) {
            return *r;
        }
        let t = std::time::Instant::now();
        let r = (self.range)(fp);
        self.range_s += t.elapsed().as_secs_f64();
        self.range_misses += 1;
        self.supply_range(fp, r);
        r
    }

    /// A footprint range the owner computed (or one from `take_range_wants`).
    pub fn supply_range(&mut self, fp: FootprintKey, r: (f64, f64)) {
        self.range_pending.remove(&fp);
        if self.range_cache.contains_key(&fp) {
            return;
        }
        if self.range_cache.len() >= RANGE_CACHE_MAX
            && let Some(old) = self.range_order.pop_front()
        {
            self.range_cache.remove(&old);
        }
        self.range_cache.insert(fp, r);
        self.range_order.push_back(fp);
    }

    /// Footprints whose ranges the tree is waiting for (each once).
    pub fn take_range_wants(&mut self) -> Vec<FootprintKey> {
        std::mem::take(&mut self.range_wants)
    }

    pub fn ranges_pending(&self) -> usize {
        self.range_pending.len()
    }

    /// Are the children's footprint ranges here? If not, ask for the
    /// missing ones (or compute them now in sync mode).
    fn children_ranges_ready(&mut self, key: ChunkKey) -> bool {
        let per_edge = self.planet.chunks_per_edge(key.lod - 1);
        let mut ready = true;
        for fp in key.footprint().children() {
            if fp.cx >= per_edge || fp.cy >= per_edge || self.range_cache.contains_key(&fp) {
                continue;
            }
            if self.sync_ranges {
                self.range_of(fp);
            } else {
                ready = false;
                if !self.range_pending.contains_key(&fp) {
                    self.range_pending.insert(fp, ());
                    self.range_wants.push(fp);
                }
            }
        }
        ready
    }

    fn make_roots(&mut self) {
        let planet = self.planet;
        let max_lod = planet.max_lod;
        let per_edge = planet.chunks_per_edge(max_lod);
        let per_col = planet.chunks_per_column(max_lod);
        for face in 0..6u8 {
            for cy in 0..per_edge {
                for cx in 0..per_edge {
                    let fp = FootprintKey { body: crate::grid::BODY_NEISOR, face, lod: max_lod, cx, cy };
                    let r = self.range_of(fp);
                    for cz in 0..per_col {
                        let key = ChunkKey::neisor(face, max_lod, cx, cy, cz);
                        if (self.in_band)(key, r) {
                            self.alloc(key, NONE);
                        }
                    }
                }
            }
        }
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| n.live)
    }

    pub fn node(&self, slot: u32) -> Option<&Node> {
        self.nodes.get(slot as usize).filter(|n| n.live)
    }

    pub fn slot_of(&self, key: ChunkKey) -> Option<u32> {
        self.index.get(&key).copied()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    fn alloc(&mut self, key: ChunkKey, parent: u32) -> u32 {
        let Bounds { centre, radius } = self.planet.chunk_bounds(key);
        let epoch = self.next_gen;
        self.next_gen = self.next_gen.wrapping_add(1).max(1);
        let node = Node {
            key,
            centre,
            radius: radius as f32,
            edge_m: (key.extent_blocks() as f64 * self.planet.block_size) as f32,
            origin: DVec3::ZERO,
            parent,
            kids: NONE,
            mesh: Mesh::Absent,
            job: Job::None,
            rev: 0,
            epoch,
            dist: f32::MAX,
            live: true,
            drawn: false,
            hidden: false,
            want: false,
            ever_drawn: false,
            fresh: false,
        };
        let slot = match self.free_nodes.pop() {
            Some(s) => {
                self.nodes[s as usize] = node;
                s
            }
            None => {
                self.nodes.push(node);
                (self.nodes.len() - 1) as u32
            }
        };
        self.index.insert(key, slot);
        slot
    }

    /// Release one node: its mesh goes to `freed`, its slot to the free list.
    fn release(&mut self, slot: u32) {
        let n = &mut self.nodes[slot as usize];
        debug_assert!(n.live);
        if let Mesh::Gpu(gm) = n.mesh {
            self.freed.push(gm);
        }
        n.mesh = Mesh::Absent;
        n.live = false;
        n.drawn = false;
        n.epoch = 0;
        if n.kids != NONE {
            self.free_kids.push(n.kids);
            n.kids = NONE;
        }
        self.index.remove(&n.key);
        self.free_nodes.push(slot);
    }

    /// Delete everything under `slot` (not `slot` itself); it becomes a leaf.
    fn delete_subtree(&mut self, slot: u32) {
        let kids = self.nodes[slot as usize].kids;
        if kids == NONE {
            return;
        }
        self.stack.clear();
        self.stack.extend(self.kids[kids as usize].iter().copied().filter(|&c| c != NONE));
        self.free_kids.push(kids);
        self.nodes[slot as usize].kids = NONE;
        while let Some(s) = self.stack.pop() {
            let k = self.nodes[s as usize].kids;
            if k != NONE {
                let children = self.kids[k as usize];
                self.stack.extend(children.iter().copied().filter(|&c| c != NONE));
                self.free_kids.push(k);
                self.nodes[s as usize].kids = NONE;
            }
            self.release(s);
        }
    }

    /// Create the in-band children of a drawn leaf.
    fn split(&mut self, slot: u32) {
        let key = self.nodes[slot as usize].key;
        debug_assert!(key.lod > 0 && !self.nodes[slot as usize].split());
        let child_lod = key.lod - 1;
        let per_edge = self.planet.chunks_per_edge(child_lod);
        let per_col = self.planet.chunks_per_column(child_lod);
        let mut children = [NONE; 8];
        // Four footprints, one height range each, shared by the two children in a column.
        let mut ranges = [(0.0f64, 0.0f64); 4];
        for (q, fp) in key.footprint().children().iter().enumerate() {
            if fp.cx < per_edge && fp.cy < per_edge {
                ranges[q] = self.range_of(*fp);
            }
        }
        debug_assert!(self.range_misses == 0 || self.sync_ranges, "split before its ranges arrived");
        for (n, child) in key.children().iter().enumerate() {
            if child.cx >= per_edge || child.cy >= per_edge || child.cz >= per_col {
                continue;
            }
            let q = (n & 1) | ((n >> 1) & 1) << 1;
            if !(self.in_band)(*child, ranges[q]) {
                continue;
            }
            children[n] = self.alloc(*child, slot);
        }
        let kids = match self.free_kids.pop() {
            Some(k) => {
                self.kids[k as usize] = children;
                k
            }
            None => {
                self.kids.push(children);
                (self.kids.len() - 1) as u32
            }
        };
        self.nodes[slot as usize].kids = kids;
    }

    fn children(&self, slot: u32) -> [u32; 8] {
        let k = self.nodes[slot as usize].kids;
        if k == NONE { [NONE; 8] } else { self.kids[k as usize] }
    }

    fn all_children(&self, slot: u32, f: impl Fn(&Node) -> bool) -> bool {
        self.children(slot).iter().all(|&c| c == NONE || f(&self.nodes[c as usize]))
    }

    fn any_child(&self, slot: u32, f: impl Fn(&Node) -> bool) -> bool {
        self.children(slot).iter().any(|&c| c != NONE && f(&self.nodes[c as usize]))
    }

    fn has_drawn_ancestor(&self, slot: u32) -> bool {
        let mut p = self.nodes[slot as usize].parent;
        while p != NONE {
            let a = &self.nodes[p as usize];
            if a.drawn {
                return true;
            }
            p = a.parent;
        }
        false
    }

    /// An edited chunk that is not a node (its footprint said Air or
    /// Enclosed): create it under its nearest existing split ancestor so
    /// it gets drawn; the levels between follow as the tree refines, the
    /// band test admitting edited chunks. Nothing to do when the nearest
    /// ancestor is a leaf (it draws, or will split through that test).
    fn materialise(&mut self, key: ChunkKey) {
        let mut child = key;
        let mut anc = key.parent();
        while let Some(a) = anc {
            if a.lod > self.planet.max_lod {
                return;
            }
            if let Some(&aslot) = self.index.get(&a) {
                let n = self.nodes[aslot as usize];
                if !n.split() {
                    return;
                }
                let idx = ((child.cx & 1) | ((child.cy & 1) << 1) | ((child.cz & 1) << 2)) as usize;
                if self.kids[n.kids as usize][idx] != NONE {
                    return;
                }
                let slot = self.alloc(child, aslot);
                self.nodes[slot as usize].fresh = true;
                self.kids[n.kids as usize][idx] = slot;
                return;
            }
            child = a;
            anc = a.parent();
        }
    }

    /// Conservative: true when no point of the sphere can be above the
    /// horizon of the opaque core as seen from `cam`.
    pub fn hidden_by_horizon(&self, cam: DVec3, centre: DVec3, radius: f64) -> bool {
        hidden_by_horizon(self.occluder_radius, cam, centre, radius)
    }

    /// One frame: re-evaluate every node against the camera, then move the
    /// cuts. Nodes created by a split are evaluated in the same frame.
    pub fn step(&mut self, cam: DVec3) -> &StepStats {
        let factor = self.factor;
        let ro = self.occluder_radius;
        let mut st = StepStats::default();
        let t_policy = std::time::Instant::now();
        // 1. Policy: distance, horizon, verdict.
        for n in self.nodes.iter_mut() {
            if !n.live {
                continue;
            }
            let d = n.centre - cam;
            let dist = (d.length() - n.radius as f64).max(0.0);
            n.dist = dist as f32;
            n.hidden = hidden_by_horizon(ro, cam, n.centre, n.radius as f64);
            let split_at = factor * n.edge_m as f64;
            n.want = if n.split() { dist < split_at * MERGE_HYSTERESIS } else { dist < split_at };
        }
        st.policy_ms = t_policy.elapsed().as_secs_f64() * 1e3;
        // 2. Structure. Splits append nodes; they are visited in this same loop.
        self.wants.clear();
        let t_split = std::time::Instant::now();
        let mut split_time = 0.0f64;
        self.range_misses = 0;
        self.range_s = 0.0;
        let mut i = 0usize;
        while i < self.nodes.len() {
            let slot = i as u32;
            i += 1;
            let n = self.nodes[slot as usize];
            if !n.live {
                continue;
            }
            if n.drawn {
                if n.split() {
                    if !n.want {
                        // The camera left before the children arrived.
                        self.delete_subtree(slot);
                        st.collapses += 1;
                    } else if self.all_children(slot, |c| c.ready()) {
                        // Switch the cover down, atomically.
                        for c in self.children(slot) {
                            if c != NONE {
                                self.nodes[c as usize].drawn = true;
                            }
                        }
                        let m = &mut self.nodes[slot as usize];
                        m.drawn = false;
                        if self.all_children(slot, |c| c.resident()) {
                            self.free_mesh(slot);
                        } else {
                            st.retained += 1;
                        }
                        st.switches += 1;
                    } else {
                        st.unsettled += 1;
                    }
                } else if n.want && n.key.lod > 0 && !n.hidden && n.resident() {
                    if split_time > self.split_budget_s {
                        st.splits_deferred += 1;
                        st.unsettled += 1;
                        continue;
                    }
                    if !self.children_ranges_ready(n.key) {
                        st.splits_waiting += 1;
                        st.unsettled += 1;
                        continue;
                    }
                    let t0 = t_split.elapsed().as_secs_f64();
                    self.split(slot);
                    split_time += t_split.elapsed().as_secs_f64() - t0;
                    // Evaluate the children now so they can be dispatched this frame.
                    for c in self.children(slot) {
                        if c == NONE {
                            continue;
                        }
                        let k = &mut self.nodes[c as usize];
                        let d = k.centre - cam;
                        k.dist = ((d.length() - k.radius as f64).max(0.0)) as f32;
                        k.hidden = hidden_by_horizon(ro, cam, k.centre, k.radius as f64);
                        k.want = false;
                        if !k.hidden {
                            self.wants.push(c);
                        }
                    }
                    st.splits += 1;
                    st.unsettled += 1;
                } else if !n.resident() && !n.hidden {
                    // Came over the horizon unloaded: its parent draws again
                    // (below) while this loads.
                    if n.job == Job::None {
                        self.wants.push(slot);
                    }
                    st.unsettled += 1;
                }
            } else if n.split() {
                // Interior, above the cut.
                if n.resident() {
                    if !n.want {
                        // Merge: the coarser mesh is here, the children go.
                        self.delete_subtree(slot);
                        self.nodes[slot as usize].drawn = true;
                        st.merges += 1;
                    } else if self.any_child(slot, |c| !c.hidden && !c.resident()) {
                        // A child came over the horizon unloaded: draw this
                        // again until it arrives. The children go back to
                        // being leaves (a child that had split was still
                        // waiting for its own children; nothing drawn is
                        // lost), so no split node ever sits below the cut.
                        for c in self.children(slot) {
                            if c != NONE {
                                self.nodes[c as usize].drawn = false;
                                self.delete_subtree(c);
                            }
                        }
                        self.nodes[slot as usize].drawn = true;
                        st.reverts += 1;
                        st.unsettled += 1;
                    } else if self.all_children(slot, |c| c.resident()) {
                        // Every child has its mesh: the fallback is no longer needed.
                        if n.job == Job::None {
                            self.free_mesh(slot);
                        }
                    } else {
                        st.retained += 1;
                    }
                } else if !n.want && self.all_children(slot, |c| !c.split()) {
                    // Merge wanted, from the bottom up: get a mesh first.
                    if n.job == Job::None {
                        self.wants.push(slot);
                    }
                    st.unsettled += 1;
                }
            } else {
                // A leaf below the cut (waiting for siblings), a root before
                // its first mesh, or a chunk an edit created under a split
                // parent whose cut is at its siblings: the last two draw as
                // soon as they are resident.
                if n.resident() {
                    if n.parent == NONE || (n.fresh && !self.has_drawn_ancestor(slot)) {
                        let m = &mut self.nodes[slot as usize];
                        m.drawn = true;
                        m.ever_drawn = true;
                        m.fresh = false;
                    }
                } else if !n.hidden {
                    if n.job == Job::None {
                        self.wants.push(slot);
                    }
                    st.unsettled += 1;
                }
            }
        }
        st.split_ms = split_time * 1e3;
        st.structure_ms = t_split.elapsed().as_secs_f64() * 1e3;
        st.range_misses = self.range_misses;
        st.range_ms = self.range_s * 1e3;
        // 3. Counts.
        let t_counts = std::time::Instant::now();
        for n in self.nodes.iter() {
            if !n.live {
                continue;
            }
            st.nodes += 1;
            let lod = (n.key.lod as usize).min(15);
            if n.drawn {
                st.drawn += 1;
                if !n.hidden {
                    st.drawn_per_lod[lod] += 1;
                }
            }
            if n.resident() {
                st.resident += 1;
                st.resident_per_lod[lod] += 1;
            }
            if n.hidden {
                st.hidden += 1;
            }
            if n.job != Job::None {
                st.unsettled += 1;
            }
        }
        st.counts_ms = t_counts.elapsed().as_secs_f64() * 1e3;
        self.stats = st;
        &self.stats
    }

    fn free_mesh(&mut self, slot: u32) {
        let n = &mut self.nodes[slot as usize];
        if let Mesh::Gpu(gm) = n.mesh {
            self.freed.push(gm);
        }
        n.mesh = Mesh::Absent;
    }

    /// The nearest `room` nodes that want a job, marked in flight.
    pub fn take_wants(&mut self, room: usize, out: &mut Vec<Want>) {
        // Dedup (a node can be pushed once per rule) and drop anything that
        // got a job meanwhile.
        self.wants.retain(|&s| self.nodes[s as usize].live && self.nodes[s as usize].job == Job::None);
        self.wants.sort_unstable();
        self.wants.dedup();
        if self.wants.len() > room {
            let nodes = &self.nodes;
            self.wants.select_nth_unstable_by(room, |&a, &b| nodes[a as usize].dist.total_cmp(&nodes[b as usize].dist));
            self.wants.truncate(room);
        }
        for &s in &self.wants {
            let n = &mut self.nodes[s as usize];
            n.job = Job::Inflight;
            out.push(Want { slot: s, epoch: n.epoch, rev: n.rev, key: n.key });
        }
        self.wants.clear();
    }

    /// How many nodes asked for a job this frame (before `take_wants`).
    pub fn wanting(&self) -> usize {
        self.wants.len()
    }

    /// A result arrived. `mesh` replaces whatever the node had.
    pub fn install(&mut self, slot: u32, epoch: u32, rev: u32, mesh: Mesh, origin: DVec3) -> Installed {
        let Some(n) = self.nodes.get_mut(slot as usize) else { return Installed::Dropped };
        if !n.live || n.epoch != epoch {
            return Installed::Dropped;
        }
        if n.job == Job::Inflight || n.job == Job::Awaiting {
            n.job = Job::None;
        }
        if n.rev != rev {
            n.job = Job::Inflight;
            return Installed::Stale(Want { slot, epoch: n.epoch, rev: n.rev, key: n.key });
        }
        if let Mesh::Gpu(old) = n.mesh {
            self.freed.push(old);
        }
        n.mesh = mesh;
        n.origin = origin;
        Installed::Ok
    }

    /// The arena had no room: the node waits with its result parked.
    pub fn park(&mut self, slot: u32, epoch: u32) {
        if let Some(n) = self.nodes.get_mut(slot as usize)
            && n.live
            && n.epoch == epoch
        {
            n.job = Job::Awaiting;
        }
    }

    /// Forget a parked result (it was dropped): the node asks again.
    pub fn unpark(&mut self, slot: u32, epoch: u32) {
        if let Some(n) = self.nodes.get_mut(slot as usize)
            && n.live
            && n.epoch == epoch
            && n.job == Job::Awaiting
        {
            n.job = Job::None;
        }
    }

    /// Is this (slot, epoch) still a live node at this revision?
    pub fn current(&self, slot: u32, epoch: u32, rev: u32) -> bool {
        self.nodes.get(slot as usize).is_some_and(|n| n.live && n.epoch == epoch && n.rev == rev)
    }

    /// An edit touched `key`: bump its revision. Returns the job to run now
    /// when the node holds (or is waiting to hold) a mesh that is now wrong.
    pub fn invalidate(&mut self, key: ChunkKey) -> Option<Want> {
        let Some(&slot) = self.index.get(&key) else {
            self.materialise(key);
            return None;
        };
        let n = &mut self.nodes[slot as usize];
        n.rev = n.rev.wrapping_add(1);
        match n.job {
            // The result will come back stale and be re-dispatched then.
            Job::Inflight => None,
            Job::Awaiting | Job::None => {
                if n.job == Job::None && !n.resident() {
                    // Nothing resident and nothing parked: a future job reads the edit.
                    return None;
                }
                n.job = Job::Inflight;
                Some(Want { slot, epoch: n.epoch, rev: n.rev, key })
            }
        }
    }

    /// Drop every node and start again from the roots.
    pub fn reset(&mut self) {
        for s in 0..self.nodes.len() as u32 {
            if self.nodes[s as usize].live {
                self.release(s);
            }
        }
        self.nodes.clear();
        self.kids.clear();
        self.free_nodes.clear();
        self.free_kids.clear();
        self.index.clear();
        self.wants.clear();
        self.make_roots();
    }

    /// Count violations of the cover invariants (O(nodes * depth)).
    pub fn check(&self) -> CoverReport {
        let mut r = CoverReport { nodes: self.len(), ..Default::default() };
        for n in self.nodes.iter().filter(|n| n.live) {
            // Up the chain: a drawn ancestor and the root.
            let mut drawn_above = false;
            let mut root = n;
            let mut p = n.parent;
            while p != NONE {
                let a = &self.nodes[p as usize];
                drawn_above |= a.drawn;
                root = a;
                p = a.parent;
            }
            if n.drawn {
                r.drawn += 1;
                if drawn_above {
                    r.overlaps += 1;
                }
                if !n.hidden && !n.resident() {
                    r.gaps += 1;
                }
            } else if !n.split() && !drawn_above && !n.hidden && root.ever_drawn && !n.fresh {
                r.uncovered += 1;
            }
        }
        r
    }
}

/// True when the whole sphere (centre, radius) lies below the horizon of
/// the opaque sphere of radius `ro` as seen from `cam`: the segment from the
/// camera to the sphere's centre passes through the core with the sphere's
/// radius to spare.
#[inline]
pub fn hidden_by_horizon(ro: f64, cam: DVec3, centre: DVec3, radius: f64) -> bool {
    let ro = ro - radius;
    if ro <= 0.0 {
        return false;
    }
    let d = centre - cam;
    let dd = d.dot(d);
    let t = if dd > 0.0 { (-cam.dot(d) / dd).clamp(0.0, 1.0) } else { 0.0 };
    let closest = cam + d * t;
    closest.dot(closest) < ro * ro
}

// ---------------------------------------------------------------- streamer

struct JobResult {
    mesh: MeshData,
    gen_us: u32,
    slot: u32,
    epoch: u32,
    rev: u32,
}

struct RangeResult {
    fp: FootprintKey,
    range: (f64, f64),
    us: u32,
}

#[derive(Default, Clone, Debug)]
pub struct StreamStats {
    /// Drawn nodes (the cut), hidden ones included.
    pub desired: usize,
    /// Resident nodes.
    pub loaded: usize,
    pub inflight: usize,
    /// Nodes that wanted a job this frame and did not get one (room).
    pub pending: usize,
    pub drawn_per_lod: [u32; 16],
    pub loaded_per_lod: [u32; 16],
    /// Opaque draws issued.
    pub drawn: usize,
    pub water_drawn: usize,
    pub culled_frustum: usize,
    /// Interior nodes drawing again for a child that came over the horizon.
    pub fallback_draws: usize,
    pub dropped_results: u64,
    pub stale_results: u64,
    pub uploaded_this_frame: usize,
    pub alloc_failures: u64,
    /// Meshes waiting for room in the quad arena.
    pub awaiting_upload: usize,
    pub remeshed: u64,
    /// Policy + structure (the tree step).
    pub traverse_ms: f64,
    /// Edits and results.
    pub recv_ms: f64,
    /// Draw lists (frustum, sort).
    pub cull_ms: f64,
    /// Live nodes.
    pub visited: usize,
    pub splits: u64,
    pub switches: u64,
    pub merges: u64,
    pub collapses: u64,
    pub reverts: u64,
    pub retained: usize,
    pub hidden: usize,
    pub unsettled: usize,
    pub splits_deferred: usize,
    pub splits_waiting: usize,
    pub range_misses: usize,
    pub range_ms: f64,
    pub split_ms: f64,
    pub policy_ms: f64,
    pub structure_ms: f64,
    pub counts_ms: f64,
    /// Footprint range jobs in flight and finished (cumulative).
    pub ranges_inflight: usize,
    pub ranges_done: u64,
    /// Worker time per range job, last seen (µs).
    pub range_job_us: u32,
}

/// Per-LOD timing samples from the workers (gen µs, mesh µs), recorded only
/// when `enabled` (the bench); play leaves it off so memory does not grow
/// with regeneration.
#[derive(Default)]
pub struct Timing {
    pub enabled: bool,
    pub samples: Vec<Vec<(u32, u32)>>,
}

pub struct Streamer {
    pub world: Arc<World>,
    pub registry: Arc<Registry>,
    pub params: LodParams,
    pub mesh_opts: MeshOptions,
    pub cover: Coverage,
    /// Finished meshes the arena could not take yet (retried after frees).
    awaiting: Vec<JobResult>,
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    range_tx: Sender<RangeResult>,
    range_rx: Receiver<RangeResult>,
    range_inflight: usize,
    inflight: usize,
    wants: Vec<Want>,
    order: Vec<(f32, DrawRecord)>,
    worder: Vec<(f32, DrawRecord)>,
    pub stats: StreamStats,
    pub timing: Timing,
}

impl Streamer {
    pub fn new(world: Arc<World>, registry: Arc<Registry>, params: LodParams, mesh_opts: MeshOptions) -> Self {
        let (tx, rx) = channel();
        let (range_tx, range_rx) = channel();
        let planet = world.planet;
        // Review #8: the occluder is the world's bottom, the one sphere that
        // is opaque whatever the terrain does and wherever anyone digs.
        let occluder_radius = planet.bottom_radius();
        let range_world = world.clone();
        let band_world = world.clone();
        let cover = Coverage::new(
            planet,
            params.factor,
            occluder_radius,
            Box::new(move |fp| range_world.generator.footprint_height_range_m(fp)),
            Box::new(move |key, r| band_world.generator.classify_in(key, r) == crate::worldgen::ChunkClass::Mixed || band_world.edits.any_under(key)),
            false,
        );
        Self {
            world,
            registry,
            params,
            mesh_opts,
            cover,
            awaiting: Vec::new(),
            tx,
            rx,
            range_tx,
            range_rx,
            range_inflight: 0,
            inflight: 0,
            wants: Vec::new(),
            order: Vec::new(),
            worder: Vec::new(),
            stats: StreamStats::default(),
            timing: Timing { enabled: false, samples: vec![Vec::new(); 16] },
        }
    }

    pub fn planet(&self) -> &Planet {
        &self.world.planet
    }

    /// Nothing in flight, nothing waiting for the arena, no edits pending,
    /// and the cover matches the camera.
    pub fn settled(&self) -> bool {
        self.inflight == 0 && self.range_inflight == 0 && self.awaiting.is_empty() && self.world.edits.pending_invalidations() == 0 && self.cover.stats.unsettled == 0
    }

    /// Keys of every resident chunk.
    pub fn resident_keys(&self) -> Vec<ChunkKey> {
        self.cover.nodes().filter(|n| n.resident()).map(|n| n.key).collect()
    }

    /// The invariants over the current tree.
    pub fn check_cover(&self) -> CoverReport {
        self.cover.check()
    }

    fn dispatch(&mut self, w: Want) {
        self.inflight += 1;
        let world = self.world.clone();
        let registry = self.registry.clone();
        let opts = self.mesh_opts;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let g = world.generate(w.key);
            let mesh = mesh_chunk(&g, &registry, &world.planet, opts);
            let _ = tx.send(JobResult { mesh, gen_us: g.gen_us, slot: w.slot, epoch: w.epoch, rev: w.rev });
        });
    }

    /// Upload a finished mesh into its node; when the arena is full the
    /// result waits.
    fn install(&mut self, res: JobResult, gpu: &mut Renderer) {
        let m = &res.mesh;
        let mesh = if m.opaque.is_empty() && m.water.is_empty() {
            Mesh::Empty
        } else {
            match gpu.add_mesh(&m.opaque, &m.water, &m.grid) {
                Some(up) => Mesh::Gpu(up),
                None => {
                    self.stats.alloc_failures += 1;
                    self.cover.park(res.slot, res.epoch);
                    if self.awaiting.len() >= MAX_AWAITING_UPLOAD {
                        let old = self.awaiting.remove(0);
                        self.cover.unpark(old.slot, old.epoch);
                    }
                    self.awaiting.push(res);
                    return;
                }
            }
        };
        match self.cover.install(res.slot, res.epoch, res.rev, mesh, m.origin) {
            Installed::Ok => self.stats.uploaded_this_frame += 1,
            Installed::Stale(w) => {
                self.stats.stale_results += 1;
                if let Mesh::Gpu(gm) = mesh {
                    gpu.free_mesh(gm);
                }
                self.dispatch(w);
            }
            Installed::Dropped => {
                self.stats.dropped_results += 1;
                if let Mesh::Gpu(gm) = mesh {
                    gpu.free_mesh(gm);
                }
            }
        }
    }

    fn free_released(&mut self, gpu: &mut Renderer) {
        for gm in self.cover.freed.drain(..) {
            gpu.free_mesh(gm);
        }
    }

    /// One streaming step: apply edits, ingest finished jobs, move the
    /// cover, dispatch new jobs, and build the draw lists.
    pub fn update(&mut self, cam: DVec3, view_proj: Mat4, gpu: &mut Renderer, opaque: &mut Vec<DrawRecord>, water: &mut Vec<DrawRecord>) {
        let t0 = std::time::Instant::now();
        let planet = *self.planet();
        self.stats.uploaded_this_frame = 0;

        // 1. Edits: bump revisions and remesh what is resident.
        for key in self.world.edits.take_invalidated() {
            if let Some(w) = self.cover.invalidate(key) {
                self.awaiting.retain(|r| !(r.slot == w.slot && r.epoch == w.epoch));
                self.stats.remeshed += 1;
                self.dispatch(w);
            }
        }
        // 2. Ingest results: footprint ranges, then meshes.
        while let Ok(r) = self.range_rx.try_recv() {
            self.range_inflight -= 1;
            self.stats.ranges_done += 1;
            self.stats.range_job_us = r.us;
            self.cover.supply_range(r.fp, r.range);
        }
        while let Ok(res) = self.rx.try_recv() {
            self.inflight -= 1;
            if self.timing.enabled {
                let lod = res.mesh.key.lod as usize;
                if lod < 16 {
                    self.timing.samples[lod].push((res.gen_us, res.mesh.mesh_us));
                }
            }
            self.install(res, gpu);
        }
        let t1 = std::time::Instant::now();

        // 3. The tree, then the footprint ranges it asked for.
        let st = self.cover.step(cam).clone();
        self.free_released(gpu);
        for fp in self.cover.take_range_wants() {
            self.range_inflight += 1;
            let world = self.world.clone();
            let tx = self.range_tx.clone();
            rayon::spawn(move || {
                let t = std::time::Instant::now();
                let range = world.generator.footprint_height_range_m(fp);
                let _ = tx.send(RangeResult { fp, range, us: t.elapsed().as_micros() as u32 });
            });
        }
        // 3b. Retry parked results now that stale meshes are freed.
        if !self.awaiting.is_empty() {
            let waiting = std::mem::take(&mut self.awaiting);
            for res in waiting {
                if !self.cover.current(res.slot, res.epoch, res.rev) {
                    self.cover.unpark(res.slot, res.epoch);
                    self.stats.dropped_results += 1;
                    continue;
                }
                self.install(res, gpu);
            }
        }
        // 4. Dispatch nearest-first.
        let wanting = self.cover.wanting();
        let room = self.params.max_inflight.saturating_sub(self.inflight);
        self.wants.clear();
        self.cover.take_wants(room, &mut self.wants);
        self.stats.pending = wanting.saturating_sub(self.wants.len());
        let wants = std::mem::take(&mut self.wants);
        for w in &wants {
            self.dispatch(*w);
        }
        self.wants = wants;
        let t2 = std::time::Instant::now();

        // 5. Draw lists: frustum cull in camera-relative space; opaque front
        // to back, water back to front.
        opaque.clear();
        water.clear();
        let planes = frustum_planes(view_proj);
        let mut culled = 0usize;
        self.order.clear();
        self.worder.clear();
        for n in self.cover.nodes() {
            if !n.drawn || n.hidden {
                continue;
            }
            let Mesh::Gpu(gm) = n.mesh else { continue };
            let rel = (n.centre - cam).as_vec3();
            if !sphere_in_frustum(&planes, rel, n.radius) {
                culled += 1;
                continue;
            }
            let o = (n.origin - cam).as_vec3();
            let voxel = planet.voxel_m(n.key.lod) as f32;
            let d2 = rel.length_squared();
            if gm.opaque_count > 0 {
                self.order.push((d2, DrawRecord { origin: o.to_array(), voxel, slot: gm.slot, quad_base: gm.opaque_base, quad_count: gm.opaque_count, pad: 0 }));
            }
            if gm.water_count > 0 {
                self.worder.push((d2, DrawRecord { origin: o.to_array(), voxel, slot: gm.slot, quad_base: gm.water_base, quad_count: gm.water_count, pad: 0 }));
            }
        }
        self.order.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        self.worder.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        opaque.extend(self.order.iter().map(|(_, d)| *d));
        water.extend(self.worder.iter().map(|(_, d)| *d));
        let t3 = std::time::Instant::now();

        let s = &mut self.stats;
        s.desired = st.drawn;
        s.loaded = st.resident;
        s.inflight = self.inflight;
        s.awaiting_upload = self.awaiting.len();
        s.drawn_per_lod = st.drawn_per_lod;
        s.loaded_per_lod = st.resident_per_lod;
        s.drawn = opaque.len();
        s.water_drawn = water.len();
        s.culled_frustum = culled;
        s.fallback_draws = st.reverts;
        s.traverse_ms = (t2 - t1).as_secs_f64() * 1e3;
        s.recv_ms = (t1 - t0).as_secs_f64() * 1e3;
        s.cull_ms = (t3 - t2).as_secs_f64() * 1e3;
        s.visited = st.nodes;
        s.splits += st.splits as u64;
        s.switches += st.switches as u64;
        s.merges += st.merges as u64;
        s.collapses += st.collapses as u64;
        s.reverts += st.reverts as u64;
        s.retained = st.retained;
        s.hidden = st.hidden;
        s.unsettled = st.unsettled;
        s.splits_deferred = st.splits_deferred;
        s.splits_waiting = st.splits_waiting;
        s.policy_ms = st.policy_ms;
        s.structure_ms = st.structure_ms;
        s.counts_ms = st.counts_ms;
        s.range_misses = st.range_misses;
        s.range_ms = st.range_ms;
        s.split_ms = st.split_ms;
        s.ranges_inflight = self.range_inflight;
    }

    /// Drop every chunk and start again from the roots.
    pub fn clear(&mut self, gpu: &mut Renderer) {
        self.cover.reset();
        self.free_released(gpu);
        self.awaiting.clear();
    }
}

/// Gribb/Hartmann planes (left, right, bottom, top, near) from a view-projection matrix.
fn frustum_planes(m: Mat4) -> [Vec4; 5] {
    let r0 = m.row(0);
    let r1 = m.row(1);
    let r2 = m.row(2);
    let r3 = m.row(3);
    let norm = |p: Vec4| p / p.truncate().length();
    // Near plane for reversed-Z: z_clip <= w_clip.
    [norm(r3 + r0), norm(r3 - r0), norm(r3 + r1), norm(r3 - r1), norm(r3 - r2)]
}

fn sphere_in_frustum(planes: &[Vec4; 5], c: Vec3, r: f32) -> bool {
    planes.iter().all(|p| p.truncate().dot(c) + p.w + r >= 0.0)
}

/// Chunk edge in metres at a LOD (helper for callers formatting stats).
pub fn chunk_edge_m(planet: &Planet, lod: u8) -> f64 {
    CHUNK as f64 * planet.voxel_m(lod)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A small planet: six roots at LOD 7 (R = 2.83 km, H = 512, sea 256).
    fn planet() -> Planet {
        Planet::new(1.0, 512, 256, 4096)
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 11
        }
        fn f(&mut self) -> f64 {
            (self.next() % 1_000_000) as f64 / 1_000_000.0
        }
    }

    /// Flat terrain: every footprint spans -20..40 m.
    fn flat_range() -> RangeFn {
        Box::new(|_| (-20.0, 40.0))
    }

    /// A fake GPU: hands out slot ids and checks every one is freed once.
    #[derive(Default)]
    struct FakeGpu {
        next: u32,
        live: HashSet<u32>,
        freed_twice: usize,
    }
    impl FakeGpu {
        fn alloc(&mut self) -> GpuMesh {
            self.next += 1;
            self.live.insert(self.next);
            GpuMesh { slot: self.next, opaque_base: 0, opaque_count: 1, water_base: 0, water_count: 0 }
        }
        fn free(&mut self, gm: GpuMesh) {
            if !self.live.remove(&gm.slot) {
                self.freed_twice += 1;
            }
        }
    }

    /// The LOD the policy wants at a drawn node once everything settles.
    fn lod_correct(c: &Coverage) -> bool {
        c.nodes().filter(|n| n.drawn).all(|n| {
            let leaf_ok = n.hidden || n.key.lod == 0 || !n.want;
            let parent_ok = n.parent == NONE || c.node(n.parent).unwrap().want;
            leaf_ok && parent_ok
        })
    }

    #[test]
    fn roots_cover_the_band_and_nothing_else() {
        let p = planet();
        let c = Coverage::new(p, 5.0, p.bottom_radius(), flat_range(), Coverage::band_test(p), true);
        // LOD 7 chunks are 4096 blocks tall: one per face column, all in band.
        assert_eq!(c.len(), 6);
        assert!(c.nodes().all(|n| n.key.lod == 7 && n.parent == NONE && !n.drawn));
        // A chunk 100 m above the band or below it does not exist at LOD 0.
        let above = ChunkKey::neisor(0, 0, 10, 10, (256 + 64) / 32);
        let below = ChunkKey::neisor(0, 0, 10, 10, (256 - 64) / 32);
        let at = ChunkKey::neisor(0, 0, 10, 10, 256 / 32);
        let mut band = Coverage::band_test(p);
        assert!(!band(above, (-20.0, 40.0)) && !band(below, (-20.0, 40.0)) && band(at, (-20.0, 40.0)));
    }

    /// Random camera paths, random job completion order and latency: at
    /// every step the cut never overlaps, never leaves a visible drawn node
    /// without a mesh, never uncovers a region it covered, and frees every
    /// GPU mesh exactly once; when the camera stops and the jobs drain, the
    /// cover is complete and at the LOD the policy asks for.
    #[test]
    fn coverage_tree_invariants_under_random_camera_paths() {
        let p = planet();
        for seed in 1..=6u64 {
            let mut rng = Lcg(seed * 7919);
            // Odd seeds compute ranges inline, even seeds get them a frame late.
            let mut c = Coverage::new(p, 3.0, p.bottom_radius(), flat_range(), Coverage::band_test(p), seed % 2 == 1);
            let mut gpu = FakeGpu::default();
            let mut inflight: Vec<(Want, u32)> = Vec::new(); // job, frames until done
            let mut wants = Vec::new();
            // Camera: a random walk on the sphere, altitude 2 m .. 2 km, speed up to 400 m/step.
            let mut dir = DVec3::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5).normalize();
            let mut alt = 2.0;
            let mut max_nodes = 0;
            let mut max_drawn = 0;
            let mut switches = 0u64;
            let mut merges = 0u64;
            let check = |c: &Coverage, when: &str| {
                let r = c.check();
                assert!(r.ok(), "seed {seed} {when}: {r:?}");
            };
            for frame in 0..1500 {
                // Move: mostly smooth, sometimes a teleport or an altitude jump.
                let r = rng.f();
                if r < 0.02 {
                    dir = DVec3::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5).normalize();
                } else if r < 0.05 {
                    alt = 2.0 + rng.f().powi(3) * 2000.0;
                } else {
                    let step = rng.f() * 400.0 / p.radius;
                    let t = DVec3::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5);
                    let t = (t - dir * t.dot(dir)).normalize();
                    dir = (dir * step.cos() + t * step.sin()).normalize();
                    alt = (alt * (1.0 + (rng.f() - 0.5) * 0.2)).clamp(2.0, 2000.0);
                }
                let cam = dir * (p.radius + alt);
                c.step(cam);
                for fp in c.take_range_wants() {
                    c.supply_range(fp, (-20.0, 40.0));
                }
                for gm in c.freed.drain(..) {
                    gpu.free(gm);
                }
                check(&c, &format!("frame {frame} after step"));
                switches += c.stats.switches as u64;
                merges += c.stats.merges as u64;
                max_nodes = max_nodes.max(c.len());
                max_drawn = max_drawn.max(c.stats.drawn);
                // Dispatch up to 12 jobs, each taking 1..6 frames.
                wants.clear();
                c.take_wants(12, &mut wants);
                for w in &wants {
                    inflight.push((*w, 1 + (rng.next() % 6) as u32));
                }
                // Complete whatever is due, in random order.
                for k in (0..inflight.len()).rev() {
                    let j = (rng.next() as usize) % inflight.len();
                    inflight.swap(k, j);
                }
                let mut still = Vec::new();
                for (w, left) in inflight.drain(..) {
                    if left > 1 {
                        still.push((w, left - 1));
                        continue;
                    }
                    let mesh = if rng.f() < 0.3 { Mesh::Empty } else { Mesh::Gpu(gpu.alloc()) };
                    match c.install(w.slot, w.epoch, w.rev, mesh, DVec3::ZERO) {
                        Installed::Ok => {}
                        Installed::Stale(w2) => {
                            if let Mesh::Gpu(gm) = mesh {
                                gpu.free(gm);
                            }
                            still.push((w2, 2));
                        }
                        Installed::Dropped => {
                            if let Mesh::Gpu(gm) = mesh {
                                gpu.free(gm);
                            }
                        }
                    }
                }
                inflight = still;
                for gm in c.freed.drain(..) {
                    gpu.free(gm);
                }
                check(&c, &format!("frame {frame} after results"));
                // Occasionally an edit lands on a random live node.
                if rng.f() < 0.1 {
                    let keys: Vec<ChunkKey> = c.nodes().map(|n| n.key).collect();
                    let key = keys[(rng.next() as usize) % keys.len()];
                    if let Some(w) = c.invalidate(key) {
                        inflight.push((w, 1 + (rng.next() % 3) as u32));
                    }
                }
            }
            // Stop and drain: the cover converges to the policy's LODs.
            let cam = dir * (p.radius + alt);
            let mut quiet = 0;
            for _ in 0..400 {
                c.step(cam);
                for fp in c.take_range_wants() {
                    c.supply_range(fp, (-20.0, 40.0));
                }
                for gm in c.freed.drain(..) {
                    gpu.free(gm);
                }
                check(&c, "draining");
                let settled = c.stats.unsettled == 0 && inflight.is_empty();
                wants.clear();
                c.take_wants(64, &mut wants);
                let mut installed = 0;
                let due: Vec<Want> = wants.drain(..).chain(inflight.drain(..).map(|(w, _)| w)).collect();
                for w in due {
                    let mesh = Mesh::Gpu(gpu.alloc());
                    match c.install(w.slot, w.epoch, w.rev, mesh, DVec3::ZERO) {
                        Installed::Ok => installed += 1,
                        Installed::Stale(w2) => {
                            gpu.free(mesh_gpu(mesh));
                            inflight.push((w2, 1));
                        }
                        Installed::Dropped => gpu.free(mesh_gpu(mesh)),
                    }
                }
                for gm in c.freed.drain(..) {
                    gpu.free(gm);
                }
                quiet = if settled && installed == 0 { quiet + 1 } else { 0 };
                if quiet >= 3 {
                    break;
                }
            }
            assert_eq!(c.stats.unsettled, 0, "seed {seed}: settled");
            let r = c.check();
            assert!(r.ok() && r.drawn > 0, "seed {seed}: {r:?}");
            assert!(lod_correct(&c), "seed {seed}: every drawn node is at the policy's LOD");
            // Every visible node is covered (no root left undrawn).
            assert!(c.nodes().all(|n| n.parent != NONE || n.ever_drawn), "seed {seed}: every root has drawn");
            assert_eq!(gpu.freed_twice, 0, "seed {seed}: no double free");
            // Live GPU meshes are exactly the resident Gpu nodes.
            let resident: HashSet<u32> = c.nodes().filter_map(|n| if let Mesh::Gpu(g) = n.mesh { Some(g.slot) } else { None }).collect();
            assert_eq!(gpu.live, resident, "seed {seed}: no leaked GPU meshes");
            println!("seed {seed}: max nodes {max_nodes}, max drawn {max_drawn}, switches {switches}, merges {merges}, final nodes {}", c.len());
            assert!(switches > 0 && merges > 0, "seed {seed}: the path exercised both directions ({switches} switches, {merges} merges)");
        }
    }

    /// Item 4: a chunk the band test excludes (an Enclosed one) that gets
    /// edited is created under its split ancestor, drawn once resident,
    /// and never counts as a cover violation; the tree refines it further
    /// because the band test admits edited chunks.
    #[test]
    fn an_edited_enclosed_chunk_becomes_a_node() {
        use std::sync::{Arc, Mutex};
        let p = planet();
        // Band: the surface only, unless the chunk has an "edit".
        let edited: Arc<Mutex<Vec<ChunkKey>>> = Arc::new(Mutex::new(Vec::new()));
        let e2 = edited.clone();
        let mut band = Coverage::band_test(p);
        let band: BandFn = Box::new(move |key, r| band(key, r) || e2.lock().unwrap().iter().any(|e| key.contains(e)));
        let mut c = Coverage::new(p, 3.0, p.bottom_radius(), flat_range(), band, true);
        let mut gpu = FakeGpu::default();
        let mut wants = Vec::new();
        let ground = DVec3::X * (p.radius + 2.0);
        for _ in 0..40 {
            c.step(ground);
            c.take_wants(10_000, &mut wants);
            for w in wants.drain(..) {
                c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
            }
            for gm in c.freed.drain(..) {
                gpu.free(gm);
            }
        }
        assert_eq!(c.stats.unsettled, 0);
        // A chunk 100 m below the band, right under the camera: not a node.
        let (f, i, j) = p.lattice_of_dir(DVec3::X);
        let deep = ChunkKey::neisor(f, 0, i as u32 / 32, j as u32 / 32, (256 - 128) / 32);
        assert!(c.slot_of(deep).is_none(), "an Enclosed chunk is not a node");
        let drawn_before = c.stats.drawn;
        // Dig into it: the edit invalidates it (and its ancestors, as World::affected_chunks does).
        edited.lock().unwrap().push(deep);
        let mut k = deep;
        loop {
            c.invalidate(k);
            match k.parent() {
                Some(pk) if pk.lod <= p.max_lod => k = pk,
                _ => break,
            }
        }
        // It (or an ancestor on the way to it) now exists under a split parent, is drawn once
        // resident, and the tree refines down to it without a violation at any step.
        let mut seen_deep = false;
        for _ in 0..40 {
            c.step(ground);
            let r = c.check();
            assert!(r.ok(), "{r:?}");
            c.take_wants(10_000, &mut wants);
            for w in wants.drain(..) {
                c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
            }
            for gm in c.freed.drain(..) {
                gpu.free(gm);
            }
            if let Some(s) = c.slot_of(deep) {
                seen_deep |= c.node(s).unwrap().drawn;
            }
        }
        assert!(seen_deep, "the dug chunk is drawn at LOD 0");
        assert_eq!(c.stats.unsettled, 0);
        assert!(c.check().ok() && c.stats.drawn > drawn_before);
        assert!(c.nodes().all(|n| !n.fresh), "nothing stays fresh once drawn");
    }

    fn mesh_gpu(m: Mesh) -> GpuMesh {
        match m {
            Mesh::Gpu(g) => g,
            _ => unreachable!(),
        }
    }

    /// A split never draws parent and children together; a merge frees the
    /// children only after the parent's mesh is resident; a parent keeps
    /// its mesh while a child is hidden and unloaded, and draws again when
    /// that child comes into view.
    #[test]
    fn switches_are_atomic_and_fallbacks_are_kept() {
        let p = planet();
        // Factor 0.5: from 2.5 km above face 0's centre only that face's
        // root is within its split distance (2048 m), and its children
        // (split at 1024 m) are not.
        let mut c = Coverage::new(p, 0.5, p.bottom_radius(), flat_range(), Coverage::band_test(p), true);
        let mut gpu = FakeGpu::default();
        let mut wants = Vec::new();
        let root_key = ChunkKey::neisor(0, 7, 0, 0, 0);
        let root = c.slot_of(root_key).unwrap();
        // Far away: only the roots.
        let far = DVec3::X * (p.radius + 500_000.0);
        c.step(far);
        c.take_wants(64, &mut wants);
        assert!(wants.iter().any(|w| w.slot == root));
        for w in wants.drain(..) {
            assert_eq!(c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO), Installed::Ok);
        }
        c.step(far);
        assert!(c.node(root).unwrap().drawn && c.check().ok());
        assert_eq!(c.stats.unsettled, 0);
        // Close in over face 0's centre: the root splits and keeps drawing.
        let near = DVec3::X * (p.radius + 2500.0);
        c.step(near);
        assert_eq!(c.stats.splits, 1, "only face 0's root splits");
        let n = c.node(root).unwrap();
        assert!(n.drawn && n.split(), "the parent keeps drawing while the children load");
        let kids: Vec<u32> = c.children(root).iter().copied().filter(|&k| k != NONE).collect();
        assert!(!kids.is_empty());
        // Half the children arrive: still the parent only.
        c.take_wants(64, &mut wants);
        let (first, rest) = wants.split_at(wants.len() / 2);
        for w in first {
            c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
        }
        c.step(near);
        let r = c.check();
        assert!(r.ok() && c.node(root).unwrap().drawn && kids.iter().all(|&k| !c.node(k).unwrap().drawn), "{r:?}");
        // The rest arrive: one step switches the cut to the children and frees the parent's mesh.
        for w in rest {
            c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
        }
        wants.clear();
        let before = c.node(root).unwrap().mesh;
        c.step(near);
        assert_eq!((c.stats.switches, c.stats.splits), (1, 0));
        assert!(!c.node(root).unwrap().drawn && kids.iter().all(|&k| c.node(k).unwrap().drawn));
        assert!(c.check().ok());
        for gm in c.freed.drain(..) {
            gpu.free(gm);
        }
        assert!(!gpu.live.contains(&mesh_gpu(before).slot), "the parent's mesh was freed after the switch");
        assert!(!c.node(root).unwrap().resident());
        // Back far away: the root wants a mesh; the children stay drawn until it has one.
        c.step(far);
        assert!(c.node(root).unwrap().split() && !c.node(root).unwrap().drawn && kids.iter().all(|&k| c.node(k).unwrap().drawn));
        c.take_wants(64, &mut wants);
        assert_eq!(wants.len(), 1);
        assert_eq!(wants[0].slot, root);
        c.step(far);
        assert!(kids.iter().all(|&k| c.node(k).unwrap().drawn && c.node(k).unwrap().live), "children still drawn while the parent loads");
        let w = wants[0];
        c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
        c.step(far);
        assert_eq!(c.stats.merges, 1);
        assert!(c.node(root).unwrap().drawn && !c.node(root).unwrap().split());
        assert!(kids.iter().all(|&k| c.node(k).is_none()), "children deleted after the merge");
        for gm in c.freed.drain(..) {
            gpu.free(gm);
        }
        assert_eq!(gpu.live.len(), 6, "one mesh per root remains");
        assert!(c.check().ok());

        // Horizon: standing on face 0, the far side's roots are hidden and
        // not dispatched; a split whose child is hidden switches anyway and
        // the parent retains its mesh; when that child comes into view the
        // parent draws again until the child arrives.
        let mut c = Coverage::new(p, 3.0, p.bottom_radius(), flat_range(), Coverage::band_test(p), true);
        let ground = DVec3::X * (p.radius + 2.0);
        c.step(ground);
        let hidden_roots = c.nodes().filter(|n| n.hidden).count();
        // (A root of this 2.8 km planet is wider than its opaque core, so none can hide.)
        assert!(hidden_roots <= 5, "{hidden_roots} roots hidden from the ground");
        wants.clear();
        c.take_wants(64, &mut wants);
        assert_eq!(wants.len(), 6 - hidden_roots, "hidden roots are not dispatched");
        for w in wants.drain(..) {
            c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
        }
        // Drive the tree down to the ground with everything delivered at once.
        for _ in 0..40 {
            c.step(ground);
            c.take_wants(10_000, &mut wants);
            for w in wants.drain(..) {
                c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
            }
            assert!(c.check().ok());
        }
        assert_eq!(c.stats.unsettled, 0);
        let retained = c.nodes().filter(|n| n.split() && !n.drawn && n.resident()).count();
        assert!(retained > 0, "some parents keep their mesh for hidden, unloaded children");
        // Pick such a parent and its hidden children; put the camera where they show.
        let parent = c.nodes().find(|n| n.split() && !n.drawn && n.resident()).unwrap();
        let pslot = c.slot_of(parent.key).unwrap();
        let hidden_child = c.children(pslot).iter().copied().find(|&k| k != NONE && c.node(k).unwrap().hidden && !c.node(k).unwrap().resident()).expect("a hidden unloaded child");
        let child_centre = c.node(hidden_child).unwrap().centre;
        let above_child = child_centre.normalize() * (p.radius + 300.0);
        c.step(above_child);
        let pn = c.node(pslot).unwrap();
        let cn = c.node(hidden_child).unwrap();
        assert!(!cn.hidden, "the child is visible from above it");
        assert!(pn.drawn && !cn.drawn && pn.resident(), "the parent draws again for the unloaded child");
        assert!(c.check().ok());
        // The child is asked for; when it arrives the cut moves down again.
        wants.clear();
        c.take_wants(10_000, &mut wants);
        assert!(wants.iter().any(|w| w.slot == hidden_child), "the newly visible child is dispatched");
        for w in wants.drain(..) {
            c.install(w.slot, w.epoch, w.rev, Mesh::Gpu(gpu.alloc()), DVec3::ZERO);
        }
        c.step(above_child);
        assert!(!c.node(pslot).unwrap().drawn && c.node(hidden_child).unwrap().drawn);
        assert!(c.check().ok());
    }

    /// Review #8: the occluder is the world bottom. A chunk on the floor of
    /// the deepest trench, seen from the surface above it, is never culled,
    /// where the old land-minimum occluder hid it at vertical scale 1/8.
    #[test]
    fn occluder_never_hides_the_trench_floor() {
        // Synthetic first: the lowest possible chunk, straight below the camera
        // and a kilometre along, on the small planet (128 L0 chunks per edge).
        let p = planet();
        let cam = DVec3::X * (p.radius + 2.0);
        for cx in [64u32, 64 + 31] {
            let b = p.chunk_bounds(ChunkKey::neisor(0, 0, cx, 64, 0));
            assert!(!hidden_by_horizon(p.bottom_radius(), cam, b.centre, b.radius), "a bottom chunk at cx {cx} is visible");
        }
        // ...and the far side is hidden.
        let far = p.chunk_bounds(ChunkKey::neisor(1, 3, 8, 8, 1));
        assert!(hidden_by_horizon(p.bottom_radius(), cam, far.centre, far.radius));

        // The real bake at 1/8 with H 4096, sea 1536 (the review's numbers).
        use crate::worldgen::bake::Bake;
        let registry = Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap();
        let bake = Arc::new(crate::worldgen::bake::open("C:/code/triangulum/game/assets/world/seed42").expect("world bake"));
        let planet = Planet::new(1.0, 4096, 1536, 131072);
        let g = crate::worldgen::Generator::new(planet, bake.clone(), 42, 0.125, 1.0, 1.0, &registry).unwrap();
        let places = crate::engine::find_places(bake.as_ref());
        let trench = places.trench.0;
        let floor_m = g.column(trench, planet.block_size).height_m as f64;
        let bottom_m = -(planet.sea_level as f64) * planet.block_size;
        assert!(floor_m < -1000.0 && floor_m > bottom_m, "trench floor {floor_m:.0} m, world bottom {bottom_m:.0} m");
        let (f, i, j) = planet.lattice_of_dir(trench);
        let k = (planet.sea_level as f64 + floor_m / planet.block_size).floor() as u32;
        let key = ChunkKey::neisor(f, 0, i as u32 / 32, j as u32 / 32, k / 32);
        let b = planet.chunk_bounds(key);
        // Camera on the sea surface 2 km away, looking down into the trench.
        let (north, _, _) = crate::camera::Camera::local_frame(trench);
        let cam_dir = (trench * (2000.0f64 / planet.radius).cos() + north * (2000.0f64 / planet.radius).sin()).normalize();
        let cam = cam_dir * (planet.radius + 1.7);
        let new_ro = planet.bottom_radius();
        // The M1a occluder (land minimum minus the relief margin): with the
        // M1a generator it sat 1167 m below sea and hid this chunk (the
        // review's finding, reproduced before landforms v2 widened the
        // margin); the number is printed, the world bottom is what counts.
        let old_ro = planet.radius + (bake.land_min_km() as f64 * 1000.0 * 0.125).min(0.0) - g.height_margin_m();
        println!(
            "trench floor {floor_m:.0} m; M1a-style occluder {:.0} m below sea (hides the floor: {}), world bottom {:.0} m",
            planet.radius - old_ro,
            hidden_by_horizon(old_ro, cam, b.centre, b.radius),
            planet.radius - new_ro
        );
        assert!(!hidden_by_horizon(new_ro, cam, b.centre, b.radius), "the world-bottom occluder shows it");
        // And every chunk in the trench's column, down to the bottom.
        for cz in 0..=(k / 32) {
            let b = planet.chunk_bounds(ChunkKey { cz, ..key });
            assert!(!hidden_by_horizon(new_ro, cam, b.centre, b.radius), "cz {cz}");
        }
    }
}

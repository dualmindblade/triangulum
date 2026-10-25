//! Streaming: LOD selection (an octree of cubic chunks per face, chosen by
//! 3D distance), horizon culling, job dispatch to rayon workers, retention
//! of stale LODs until replacements arrive, and the per-frame draw lists.
//!
//! Chunks that lie entirely above or below the terrain band of their
//! footprint (from the bake's min/max pyramid) are never visited, so a
//! taller world adds no chunks: only the surface shell is ever loaded.

use crate::grid::{CHUNK, ChunkKey, FastMap, FastSet, FootprintKey, Planet};
use crate::mesh::{MeshData, MeshOptions, mesh_chunk};
use crate::render::{DrawRecord, GpuMesh, Renderer};
use crate::world::{Chunk, ChunkStore, Registry};
use crate::worldgen::Generator;
use glam::{DVec3, Mat4, Vec3, Vec4};

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Copy, Debug)]
pub struct LodParams {
    /// Subdivide a chunk when the camera is closer than `factor * chunk edge`.
    pub factor: f64,
    pub max_inflight: usize,
}

pub struct LoadedChunk {
    pub mesh: Option<GpuMesh>,
    pub origin: DVec3,
    pub center: DVec3,
    pub radius: f32,
}

struct JobResult {
    mesh: MeshData,
    chunk: Arc<Chunk>,
    gen_us: u32,
}

#[derive(Default, Clone, Debug)]
pub struct StreamStats {
    pub desired: usize,
    pub loaded: usize,
    pub inflight: usize,
    pub pending: usize,
    pub drawn_per_lod: [u32; 16],
    pub loaded_per_lod: [u32; 16],
    pub drawn: usize,
    pub water_drawn: usize,
    pub culled_frustum: usize,
    pub fallback_draws: usize,
    pub dropped_results: u64,
    pub uploaded_this_frame: usize,
    pub alloc_failures: u64,
    pub traverse_ms: f64,
    pub recv_ms: f64,
    pub cull_ms: f64,
    pub visited: usize,
}

/// Per-LOD timing samples from the workers (gen µs, mesh µs).
#[derive(Default)]
pub struct Timing {
    pub samples: Vec<Vec<(u32, u32)>>,
}

pub struct Streamer {
    pub generator: Arc<Generator>,
    pub registry: Arc<Registry>,
    pub params: LodParams,
    pub mesh_opts: MeshOptions,
    range_cache: FastMap<FootprintKey, (f64, f64)>,
    pub loaded: FastMap<ChunkKey, LoadedChunk>,
    geom_cache: FastMap<ChunkKey, (DVec3, f64)>,
    pub chunks: ChunkStore,
    inflight: FastSet<ChunkKey>,
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    desired: Vec<(ChunkKey, f64)>,
    desired_set: FastSet<ChunkKey>,
    render_set: Vec<ChunkKey>,
    pub stats: StreamStats,
    pub timing: Timing,
    occluder_radius: f64,
    visited: usize,
}

impl Streamer {
    pub fn new(generator: Arc<Generator>, registry: Arc<Registry>, params: LodParams, mesh_opts: MeshOptions) -> Self {
        let (tx, rx) = channel();
        let planet = generator.planet;
        let land_min_m = generator.bake.land_min_km() as f64 * 1000.0 * generator.vscale;
        let occluder_radius = planet.radius + land_min_m.min(0.0) - generator.height_margin_m();
        Self {
            generator,
            registry,
            params,
            mesh_opts,
            range_cache: FastMap::default(),
            loaded: FastMap::default(),
            geom_cache: FastMap::default(),
            chunks: ChunkStore::default(),
            inflight: FastSet::default(),
            tx,
            rx,
            desired: Vec::new(),
            desired_set: FastSet::default(),
            render_set: Vec::new(),
            stats: StreamStats::default(),
            timing: Timing { samples: vec![Vec::new(); 16] },
            occluder_radius,
            visited: 0,
        }
    }

    pub fn planet(&self) -> &Planet {
        &self.generator.planet
    }

    pub fn settled(&self) -> bool {
        self.inflight.is_empty() && self.desired.iter().all(|(k, _)| self.loaded.contains_key(k))
    }

    fn height_range(&mut self, fp: FootprintKey) -> (f64, f64) {
        if let Some(r) = self.range_cache.get(&fp) {
            return *r;
        }
        let r = self.generator.footprint_height_range_m(fp);
        self.range_cache.insert(fp, r);
        r
    }

    /// Bounding sphere of a chunk.
    fn geom(&mut self, key: ChunkKey) -> (DVec3, f64) {
        if let Some(g) = self.geom_cache.get(&key) {
            return *g;
        }
        let g = self.compute_geom(key);
        if self.geom_cache.len() > 1 << 20 {
            self.geom_cache.clear();
        }
        self.geom_cache.insert(key, g);
        g
    }

    fn compute_geom(&self, key: ChunkKey) -> (DVec3, f64) {
        let planet = self.planet();
        let e = key.extent_blocks() as f64;
        let (i0, j0, k0) = key.origin_blocks();
        let half = e * 0.5;
        let centre = planet.dir(key.face, i0 as f64 + half, j0 as f64 + half) * planet.radius_at(k0 as f64 + half);
        let radius = half * planet.block_size * 3f64.sqrt() * 1.05 + 1.0;
        (centre, radius)
    }

    /// Conservative: true when no point of the sphere can be above the horizon.
    fn hidden_by_horizon(&self, cam: DVec3, centre: DVec3, radius: f64) -> bool {
        let ro = self.occluder_radius - radius;
        if ro <= 0.0 {
            return false;
        }
        let d = centre - cam;
        let dd = d.dot(d);
        let t = if dd > 0.0 { (-cam.dot(d) / dd).clamp(0.0, 1.0) } else { 0.0 };
        let closest = cam + d * t;
        closest.dot(closest) < ro * ro
    }

    fn visit(&mut self, key: ChunkKey, cam: DVec3) {
        self.visited += 1;
        let planet = *self.planet();
        let (hmin, hmax) = self.height_range(key.footprint());
        // Vertical extent of the chunk in metres above sea level.
        let e_m = key.extent_blocks() as f64 * planet.block_size;
        let z0 = key.cz as f64 * e_m - planet.sea_level as f64 * planet.block_size;
        let z1 = z0 + e_m;
        if z1 <= hmin || z0 >= hmax {
            return; // entirely below (solid) or above (air) the terrain band
        }
        let (centre, radius) = self.geom(key);
        if self.hidden_by_horizon(cam, centre, radius) {
            return;
        }
        let dist = ((centre - cam).length() - radius).max(0.0);
        if key.lod > 0 && dist < self.params.factor * e_m {
            let child_lod = key.lod - 1;
            let per_edge = planet.chunks_per_edge(child_lod);
            let per_col = planet.chunks_per_column(child_lod);
            for c in key.children() {
                if c.cx < per_edge && c.cy < per_edge && c.cz < per_col {
                    self.visit(c, cam);
                }
            }
        } else {
            self.desired.push((key, dist));
        }
    }

    /// One streaming step: decide the desired set, ingest finished jobs,
    /// dispatch new ones, free stale chunks, and build the draw lists.
    pub fn update(&mut self, cam: DVec3, view_proj: Mat4, gpu: &mut Renderer, opaque: &mut Vec<DrawRecord>, water: &mut Vec<DrawRecord>) {
        let t0 = std::time::Instant::now();
        let planet = *self.planet();
        // 1. Desired set.
        self.desired.clear();
        self.visited = 0;
        let max_lod = planet.max_lod;
        let per_edge = planet.chunks_per_edge(max_lod);
        let per_col = planet.chunks_per_column(max_lod);
        for face in 0..6u8 {
            for cy in 0..per_edge {
                for cx in 0..per_edge {
                    for cz in 0..per_col {
                        self.visit(ChunkKey { face, lod: max_lod, cx, cy, cz }, cam);
                    }
                }
            }
        }
        self.desired_set.clear();
        self.desired_set.extend(self.desired.iter().map(|(k, _)| *k));
        let t1 = std::time::Instant::now();

        // 2. Ingest results.
        self.stats.uploaded_this_frame = 0;
        while let Ok(res) = self.rx.try_recv() {
            let key = res.mesh.key;
            self.inflight.remove(&key);
            let lod = key.lod as usize;
            if lod < 16 {
                self.timing.samples[lod].push((res.gen_us, res.mesh.mesh_us));
            }
            if !self.desired_set.contains(&key) {
                self.stats.dropped_results += 1;
                continue;
            }
            let m = &res.mesh;
            let mesh = if m.opaque.is_empty() && m.water.is_empty() {
                None
            } else {
                let up = gpu.add_mesh(&m.opaque, &m.water, &m.grid);
                if up.is_none() {
                    self.stats.alloc_failures += 1;
                }
                up
            };
            self.stats.uploaded_this_frame += 1;
            let loaded = LoadedChunk { mesh, origin: m.origin, center: m.center, radius: m.radius };
            if let Some(old) = self.loaded.insert(key, loaded) {
                if let Some(gm) = old.mesh {
                    gpu.free_mesh(gm);
                }
            }
            self.chunks.insert(key, res.chunk);
        }
        let t2 = std::time::Instant::now();

        // 3. Render set: desired & loaded, plus fallbacks for desired & missing.
        self.render_set.clear();
        let mut render_marked: FastSet<ChunkKey> = FastSet::default();
        let mut pending: Vec<(ChunkKey, f64)> = Vec::new();
        let mut fallback_draws = 0usize;
        for &(key, dist) in &self.desired {
            if self.loaded.contains_key(&key) {
                if render_marked.insert(key) {
                    self.render_set.push(key);
                }
                continue;
            }
            if !self.inflight.contains(&key) {
                pending.push((key, dist));
            }
            // Nearest loaded ancestor…
            let mut anc = key.parent();
            let mut found = false;
            while let Some(a) = anc {
                if a.lod > max_lod {
                    break;
                }
                if self.loaded.contains_key(&a) {
                    if render_marked.insert(a) {
                        self.render_set.push(a);
                        fallback_draws += 1;
                    }
                    found = true;
                    break;
                }
                anc = a.parent();
            }
            if found || key.lod == 0 {
                continue;
            }
            // …or loaded descendants, up to 2 levels down.
            let mut frontier = vec![key];
            for _ in 0..2 {
                let mut next = Vec::new();
                for k in frontier {
                    if k.lod == 0 {
                        continue;
                    }
                    for c in k.children() {
                        if self.loaded.contains_key(&c) {
                            if render_marked.insert(c) {
                                self.render_set.push(c);
                                fallback_draws += 1;
                            }
                        } else {
                            next.push(c);
                        }
                    }
                }
                frontier = next;
                if frontier.is_empty() {
                    break;
                }
            }
        }
        self.stats.fallback_draws = fallback_draws;

        // 4. Free everything neither desired nor used as a fallback.
        let stale: Vec<ChunkKey> = self.loaded.keys().filter(|k| !self.desired_set.contains(k) && !render_marked.contains(k)).copied().collect();
        for k in stale {
            if let Some(s) = self.loaded.remove(&k) {
                if let Some(gm) = s.mesh {
                    gpu.free_mesh(gm);
                }
            }
            self.chunks.remove(&k);
        }

        // 5. Dispatch nearest-first.
        pending.sort_by(|a, b| a.1.total_cmp(&b.1));
        self.stats.pending = pending.len();
        let room = self.params.max_inflight.saturating_sub(self.inflight.len());
        for &(key, _) in pending.iter().take(room) {
            self.inflight.insert(key);
            let generator = self.generator.clone();
            let registry = self.registry.clone();
            let opts = self.mesh_opts;
            let tx = self.tx.clone();
            rayon::spawn(move || {
                let g = generator.generate(key);
                let mesh = mesh_chunk(&g, &registry, &generator.planet, opts);
                let _ = tx.send(JobResult { mesh, chunk: Arc::new(g.chunk), gen_us: g.gen_us });
            });
        }

        // 6. Draw lists: frustum cull in camera-relative space; opaque front
        // to back, water back to front.
        opaque.clear();
        water.clear();
        let planes = frustum_planes(view_proj);
        let mut culled = 0usize;
        let mut per_lod = [0u32; 16];
        let mut order: Vec<(f32, DrawRecord)> = Vec::new();
        let mut worder: Vec<(f32, DrawRecord)> = Vec::new();
        for key in &self.render_set {
            let c = &self.loaded[key];
            let Some(gm) = c.mesh else { continue };
            let rel = (c.center - cam).as_vec3();
            if !sphere_in_frustum(&planes, rel, c.radius) {
                culled += 1;
                continue;
            }
            per_lod[(key.lod as usize).min(15)] += 1;
            let o = (c.origin - cam).as_vec3();
            let voxel = planet.voxel_m(key.lod) as f32;
            let d2 = rel.length_squared();
            if gm.opaque_count > 0 {
                order.push((d2, DrawRecord { origin: o.to_array(), voxel, slot: gm.slot, quad_base: gm.opaque_base, quad_count: gm.opaque_count, pad: 0 }));
            }
            if gm.water_count > 0 {
                worder.push((d2, DrawRecord { origin: o.to_array(), voxel, slot: gm.slot, quad_base: gm.water_base, quad_count: gm.water_count, pad: 0 }));
            }
        }
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        worder.sort_by(|a, b| b.0.total_cmp(&a.0));
        opaque.extend(order.into_iter().map(|(_, d)| d));
        water.extend(worder.into_iter().map(|(_, d)| d));
        let t3 = std::time::Instant::now();

        let mut loaded_per_lod = [0u32; 16];
        for k in self.loaded.keys() {
            loaded_per_lod[(k.lod as usize).min(15)] += 1;
        }
        self.stats.desired = self.desired.len();
        self.stats.loaded = self.loaded.len();
        self.stats.inflight = self.inflight.len();
        self.stats.drawn_per_lod = per_lod;
        self.stats.loaded_per_lod = loaded_per_lod;
        self.stats.drawn = opaque.len();
        self.stats.water_drawn = water.len();
        self.stats.culled_frustum = culled;
        self.stats.traverse_ms = (t1 - t0).as_secs_f64() * 1e3;
        self.stats.recv_ms = (t2 - t1).as_secs_f64() * 1e3;
        self.stats.cull_ms = (t3 - t2).as_secs_f64() * 1e3;
        self.stats.visited = self.visited;
    }

    /// Drop every loaded chunk.
    pub fn clear(&mut self, gpu: &mut Renderer) {
        for (_, s) in self.loaded.drain() {
            if let Some(gm) = s.mesh {
                gpu.free_mesh(gm);
            }
        }
        self.chunks = ChunkStore::default();
    }

    /// Chunks at CHUNK granularity: how many desired chunks per LOD.
    pub fn desired_per_lod(&self) -> [u32; 16] {
        let mut out = [0u32; 16];
        for (k, _) in &self.desired {
            out[(k.lod as usize).min(15)] += 1;
        }
        out
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

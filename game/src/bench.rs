//! Headless measurement: static poses (settle, then N frames), the fixed
//! 60 s flight, PNG captures with captions, AO cost, and the per-chunk
//! generation/meshing costs. Also Andrew's vertical-scale render shots.

use crate::camera::Camera;
use crate::engine::{Engine, HILLS, MOUNTAINS, gnomonic_dir};
use crate::render::{DrawRecord, Renderer};
use crate::sim::Falling;
use crate::sim::draw::FallingDraws;
use crate::stream::Streamer;
use crate::ui::{EguiPaint, draw_caption, paint_offscreen};
use crate::worldgen::Marker;
use glam::DVec3;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct Harness {
    pub engine: Arc<Engine>,
    pub streamer: Streamer,
    pub gpu: Renderer,
    pub out_dir: String,
    opaque: Vec<DrawRecord>,
    water: Vec<DrawRecord>,
    egui_ctx: egui::Context,
    egui_paint: EguiPaint,
    /// Blocks in flight to draw this frame (the play harness sets it from
    /// its session; the bench has none).
    pub falling: Vec<Falling>,
    falling_draw: FallingDraws,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct FrameSample {
    pub cpu_ms: f64,
    pub gpu_ms: f64,
    pub wall_ms: f64,
    pub tris: u64,
    pub water_tris: u64,
    pub draws: u32,
}

pub struct Pose {
    pub name: String,
    pub dir: DVec3,
    pub alt_agl: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn stats(v: &[f64]) -> (f64, f64, f64, f64) {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let avg = s.iter().sum::<f64>() / s.len().max(1) as f64;
    (avg, pct(&s, 0.5), pct(&s, 0.95), pct(&s, 0.99))
}

impl Harness {
    pub fn new(engine: Arc<Engine>, width: u32, height: u32, out_dir: &str, backend: Option<&str>) -> Self {
        let instance = crate::render::make_instance(backend);
        let gpu = Renderer::new(&instance, None, width, height, engine.quad_mb, engine.slot_capacity, &engine.colours);
        let egui_paint = EguiPaint::new(&gpu.device, gpu.format);
        let streamer = engine.streamer();
        let _ = std::fs::create_dir_all(out_dir);
        Self { engine, streamer, gpu, out_dir: out_dir.to_string(), opaque: Vec::new(), water: Vec::new(), egui_ctx: egui::Context::default(), egui_paint, falling: Vec::new(), falling_draw: FallingDraws::default() }
    }

    /// Swap in a new engine. A different world (markers, a reload) drops
    /// every chunk; a view-only change (water hidden) keeps them.
    pub fn set_engine(&mut self, engine: Arc<Engine>) {
        if !Arc::ptr_eq(&self.engine.world, &engine.world) {
            self.falling_draw.clear(&mut self.gpu);
            self.streamer.clear(&mut self.gpu);
            self.streamer = engine.streamer();
        }
        self.engine = engine;
    }

    /// One frame: streaming update + render; waits for the GPU for its timing.
    pub fn frame(&mut self, cam: &Camera) -> FrameSample {
        let t0 = Instant::now();
        let aspect = self.gpu.width as f64 / self.gpu.height as f64;
        let vp = cam.view_proj(aspect);
        self.streamer.update(cam.pos, vp, &mut self.gpu, &mut self.opaque, &mut self.water);
        self.falling_draw.emit(&mut self.gpu, &self.engine.planet, &self.falling, cam.pos, &mut self.opaque);
        let uni = cam.uniform(aspect, self.engine.sun_at(cam.pos), &self.engine.planet, self.engine.water_alpha);
        // Water is hidden at draw time, never in generation (review #5).
        let water: &[DrawRecord] = if self.engine.hide_water { &[] } else { &self.water };
        let fs = self.gpu.render(&uni, &self.opaque, water, None);
        let cpu_ms = t0.elapsed().as_secs_f64() * 1e3;
        let gpu_ms = self.gpu.wait_gpu_ms();
        FrameSample { cpu_ms, gpu_ms, wall_ms: t0.elapsed().as_secs_f64() * 1e3, tris: fs.triangles, water_tris: fs.water_triangles, draws: fs.draws + fs.water_draws }
    }

    /// Run frames until streaming is settled for 10 consecutive frames.
    pub fn settle(&mut self, cam: &Camera, timeout: Duration) -> (f64, usize, bool) {
        let t0 = Instant::now();
        let mut frames = 0usize;
        let mut stable = 0;
        loop {
            self.frame(cam);
            frames += 1;
            if self.streamer.settled() {
                stable += 1;
                if stable >= 10 {
                    return (t0.elapsed().as_secs_f64(), frames, true);
                }
            } else {
                stable = 0;
            }
            if t0.elapsed() > timeout {
                return (t0.elapsed().as_secs_f64(), frames, false);
            }
        }
    }

    pub fn save(&mut self, path: &str, caption: Option<&str>) {
        if let Some(text) = caption {
            let text = text.to_string();
            paint_offscreen(&self.egui_ctx, &mut self.egui_paint, &self.gpu, |ctx| draw_caption(ctx, &text));
        }
        self.gpu.save_png(path);
    }

    /// Save the frame with an arbitrary egui overlay (the harness draws the game HUD).
    pub fn save_with_ui(&mut self, path: &str, build: &dyn Fn(&egui::Context)) {
        paint_offscreen(&self.egui_ctx, &mut self.egui_paint, &self.gpu, build);
        self.gpu.save_png(path);
    }

    pub fn run_static(&mut self, poses: &[Pose], frames: usize) -> String {
        let mut report = String::new();
        report.push_str("| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |\n");
        report.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
        for p in poses {
            let cam = self.engine.camera_at(p.dir, p.alt_agl, p.heading_deg, p.pitch_deg);
            let (settle_s, settle_frames, ok) = self.settle(&cam, Duration::from_secs(240));
            let mut cpu = Vec::new();
            let mut gpu = Vec::new();
            let mut last = FrameSample::default();
            for _ in 0..frames {
                let s = self.frame(&cam);
                cpu.push(s.cpu_ms);
                gpu.push(s.gpu_ms);
                last = s;
            }
            let (cavg, _, c95, _) = stats(&cpu);
            let (gavg, _, g95, _) = stats(&gpu);
            let st = self.streamer.stats.clone();
            let per_lod: Vec<String> = st.drawn_per_lod.iter().enumerate().filter(|(_, n)| **n > 0).map(|(l, n)| format!("L{l}:{n}")).collect();
            let png = format!("{}/pose_{}.png", self.out_dir, p.name);
            self.save(&png, None);
            let vram_mb = self.gpu.resident_bytes() as f64 / 1e6;
            let row = format!(
                "| {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2}M | {:.2}M | {} | {} | {} | {:.1} | {:.1}{} | {} |\n",
                p.name,
                gavg,
                g95,
                cavg,
                c95,
                last.tris as f64 / 1e6,
                last.water_tris as f64 / 1e6,
                last.draws,
                per_lod.join(" "),
                st.loaded,
                vram_mb,
                settle_s,
                if ok { "" } else { " (TIMEOUT)" },
                png
            );
            print!("{row}");
            println!(
                "    resident {} nodes {} drawn {} hidden {} retained {} culled {} settle frames {} cpu split: tree {:.3} edits+results {:.3} draw lists {:.3} ms; alloc peak {:.1} MB; drops {} stale {} alloc failures {}",
                st.loaded, st.visited, st.desired, st.hidden, st.retained, st.culled_frustum, settle_frames, st.traverse_ms, st.recv_ms, st.cull_ms,
                self.gpu.alloc.peak as f64 * 8.0 / 1e6, st.dropped_results, st.stale_results, st.alloc_failures
            );
            report.push_str(&row);
        }
        report
    }

    /// The project's one performance number: 60 s, 50 m AGL at 50 m/s for
    /// 30 s over varied terrain, climb to 5 km over 15 s, dive back over 15 s.
    pub fn run_flight(&mut self, start: DVec3, heading_deg: f64) -> (String, Vec<FrameSample>) {
        let mut cam = self.engine.camera_at(start, 50.0, heading_deg, -5.0);
        self.settle(&cam, Duration::from_secs(120));
        let mut samples = Vec::new();
        let t0 = Instant::now();
        let mut last = Instant::now();
        let mut dir = start;
        let mut png_mid = false;
        loop {
            let t = t0.elapsed().as_secs_f64();
            if t >= 60.0 {
                break;
            }
            let now = Instant::now();
            let dt = (now - last).as_secs_f64().min(0.1);
            last = now;
            let (_, fwd_h, _) = cam.vectors();
            dir = (dir * self.engine.planet.radius + fwd_h * (50.0 * dt)).normalize();
            let (alt_agl, pitch) = if t < 30.0 {
                (50.0, -5.0)
            } else if t < 45.0 {
                let u = smooth((t - 30.0) / 15.0);
                (50.0 + 4950.0 * u, -5.0 - 25.0 * u)
            } else {
                let u = smooth((t - 45.0) / 15.0);
                (5000.0 - 4950.0 * u, -30.0 + 25.0 * u)
            };
            let h = self.engine.ground(dir);
            cam.pos = dir * (self.engine.planet.radius + h + alt_agl);
            cam.pitch = (pitch as f64).to_radians();
            samples.push(self.frame(&cam));
            if t >= 15.0 && !png_mid {
                png_mid = true;
                self.save(&format!("{}/flight_low.png", self.out_dir), None);
            }
        }
        self.save(&format!("{}/flight_end.png", self.out_dir), None);
        let wall: Vec<f64> = samples.iter().map(|s| s.wall_ms).collect();
        let (wavg, w50, _, w99) = stats(&wall);
        let mut sorted = wall.clone();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let n1 = (sorted.len() / 100).max(1);
        let low1_ms = sorted[..n1].iter().sum::<f64>() / n1 as f64;
        let worst = sorted[0];
        let cpu: Vec<f64> = samples.iter().map(|s| s.cpu_ms).collect();
        let gpu: Vec<f64> = samples.iter().map(|s| s.gpu_ms).collect();
        let (cavg, _, _, c99) = stats(&cpu);
        let (gavg, _, _, g99) = stats(&gpu);
        let report = format!(
            "| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |\n|---|---|---|---|---|---|---|\n| {} | {:.1} | {:.1} | {:.2} | {:.2} / {:.2} | {:.2} / {:.2} | {:.2} / {:.2} |\n",
            samples.len(),
            1000.0 / wavg,
            1000.0 / low1_ms,
            worst,
            w50,
            w99,
            cavg,
            c99,
            gavg,
            g99
        );
        (report, samples)
    }

    /// Re-mesh every loaded chunk with and without AO (CPU only) and
    /// compare quad counts: the triangle cost of per-vertex AO.
    pub fn ao_cost(&self) -> String {
        use rayon::prelude::*;
        let keys: Vec<_> = self.streamer.resident_keys();
        let engine = &self.engine;
        let (with, without): (usize, usize) = keys
            .par_iter()
            .map(|k| {
                let g = engine.generator.generate(*k);
                let a = crate::mesh::mesh_chunk(&g, &engine.registry, &engine.planet, crate::mesh::MeshOptions { split: true, ao: true });
                let b = crate::mesh::mesh_chunk(&g, &engine.registry, &engine.planet, crate::mesh::MeshOptions { split: true, ao: false });
                (a.opaque.len(), b.opaque.len())
            })
            .reduce(|| (0, 0), |x, y| (x.0 + y.0, x.1 + y.1));
        format!(
            "AO cost over {} loaded chunks: {} opaque quads with AO vs {} without (+{:.1}%), i.e. {:.2}M vs {:.2}M triangles\n",
            keys.len(),
            with,
            without,
            (with as f64 / without.max(1) as f64 - 1.0) * 100.0,
            with as f64 * 2.0 / 1e6,
            without as f64 * 2.0 / 1e6
        )
    }

    pub fn timing_report(&self) -> String {
        let mut s = String::new();
        s.push_str("| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |\n|---|---|---|---|---|---|\n");
        for (lod, v) in self.streamer.timing.samples.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            let g: Vec<f64> = v.iter().map(|x| x.0 as f64).collect();
            let m: Vec<f64> = v.iter().map(|x| x.1 as f64).collect();
            let (_, gm, g95, _) = stats(&g);
            let (_, mm, m95, _) = stats(&m);
            s.push_str(&format!("| {lod} | {} | {gm:.0} | {g95:.0} | {mm:.0} | {m95:.0} |\n", v.len()));
        }
        s
    }
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub struct BenchOptions {
    pub out: String,
    pub frames: usize,
    pub skip_flight: bool,
    pub only: Option<String>,
    pub backend: Option<String>,
    pub label: String,
}

pub fn run_bench(engine: Arc<Engine>, opts: BenchOptions) {
    let mut h = Harness::new(engine.clone(), 1920, 1080, &opts.out, opts.backend.as_deref());
    // Worker timing history is only kept here (review #15).
    h.streamer.timing.enabled = true;
    let hills = gnomonic_dir(HILLS.0, HILLS.1, HILLS.2);
    let mountains = gnomonic_dir(MOUNTAINS.0, MOUNTAINS.1, MOUNTAINS.2);
    let c = engine.generator.column(hills, engine.planet.block_size);
    println!("hills anchor: ground {:.1} m, ocean {}, temp {:.1} C, precip {:.0} mm; max_lod {}", c.height_m, c.ocean, c.temp_c, c.precip, engine.planet.max_lod);
    let mk = |name: &str, dir: DVec3, alt: f64, heading: f64, pitch: f64| Pose { name: name.into(), dir, alt_agl: alt, heading_deg: heading, pitch_deg: pitch };
    let all = vec![
        mk("stand_hills", hills, 1.7, -60.0, 0.0),
        mk("stand_mountains", mountains, 1.7, -155.0, 0.0),
        mk("alt_100m", hills, 100.0, -60.0, -10.0),
        mk("alt_1km", hills, 1_000.0, -60.0, -20.0),
        mk("alt_10km", hills, 10_000.0, -60.0, -35.0),
        mk("alt_100km", hills, 100_000.0, -60.0, -60.0),
        mk("alt_500km", hills, 500_000.0, -60.0, -89.0),
    ];
    let poses: Vec<Pose> = all.into_iter().filter(|p| opts.only.as_deref().is_none_or(|o| o.split(',').any(|n| n == p.name))).collect();
    println!("\n== static poses (1920x1080, {} frames each) ==", opts.frames);
    let table = h.run_static(&poses, opts.frames);
    let ao = if poses.iter().any(|p| p.name == "stand_hills") {
        // Settle the standing pose again so the AO comparison covers it.
        let cam = engine.camera_at(hills, 1.7, -60.0, 0.0);
        h.settle(&cam, Duration::from_secs(120));
        let s = h.ao_cost();
        print!("{s}");
        s
    } else {
        String::new()
    };
    let mut flight = String::new();
    if !opts.skip_flight {
        println!("\n== fixed 60 s flight ==");
        let (r, _) = h.run_flight(hills, -60.0);
        print!("{r}");
        flight = r;
    }
    let timing = h.timing_report();
    println!("\n== per-chunk generation / meshing cost (worker threads) ==\n{timing}");
    let p = &engine.planet;
    let summary = format!(
        "# bench: {}\n\nTiming is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.\n\nplanet: s = {} m, H = {} blocks, sea level {} blocks, N = {}, R = {:.1} km, vertical scale {:.5} (1/{:.1}), roughness {}, max LOD {} ({} m voxels), lod factor {}, rayon threads {}, AO {}, skirt {}\n\n## static poses\n\n{table}\n{ao}\n## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)\n\n{flight}\n## worker costs\n\n{timing}",
        opts.label,
        p.block_size,
        p.height,
        p.sea_level,
        p.n,
        p.radius / 1000.0,
        engine.vscale(),
        1.0 / engine.vscale(),
        engine.config.roughness,
        p.max_lod,
        p.voxel_m(p.max_lod),
        engine.lod.factor,
        rayon::current_num_threads(),
        engine.mesh_opts.ao,
        engine.skirt,
    );
    let path = format!("{}/results.md", opts.out);
    std::fs::write(&path, summary).unwrap();
    println!("wrote {path}");
}

// ---------------------------------------------------------------- shots

pub struct ShotOptions {
    pub out: String,
    pub backend: Option<String>,
    pub which: Option<String>,
}

/// Andrew's vertical-scale renders: the same places at the current engine
/// settings, water hidden where the sea floor is the subject, with 2 m and
/// 100 m marker columns, captioned.
pub fn run_shots(engine: Arc<Engine>, opts: ShotOptions) {
    let mut h = Harness::new(engine.clone(), 1920, 1080, &opts.out, opts.backend.as_deref());
    let p = engine.planet;
    let vs = engine.vscale();
    let places = engine.places;
    let b = engine.generator.blocks;
    let marker_pair = |dir: DVec3, e: &Engine| -> Vec<Marker> {
        let beside = e.offset_dir(dir, 90.0, 8.0 * p.block_size);
        vec![
            Marker { dir, height_blocks: (100.0 / p.block_size) as u32, width_blocks: (4.0 / p.block_size).max(1.0) as u32, block: b.marker_red, below: false },
            Marker { dir: beside, height_blocks: (2.0 / p.block_size) as u32, width_blocks: 1, block: b.marker_white, below: false },
        ]
    };
    let scale_label = format!("vertical scale 1/{:.0} ({:.4})", 1.0 / vs, vs);
    let world_label = format!("H {} blocks, sea level {}, s = {} m", p.height, p.sea_level, p.block_size);

    // Terrain heights at this scale (relief included).
    let peak_dir = places.peak.0;
    let peak_h = engine.generator.column(peak_dir, p.block_size).height_m as f64;
    let trench_dir = places.trench.0;
    let trench_h = engine.generator.column(trench_dir, p.block_size).height_m as f64;
    let coast_dir = places.coast.0;
    let bottom_m = -(p.sea_level as f64) * p.block_size;
    println!("places: peak {:.1} m ({:.2} km Earth), trench {:.1} m ({:.2} km Earth, world bottom at {:.0} m), coast gradient {:.3}", peak_h, places.peak.1, trench_h, places.trench.1, bottom_m, places.coast.1);

    // Pick the viewing direction toward the peak with the lowest ground 4 km out.
    let best_heading = |centre: DVec3, dist: f64, lowest: bool| -> f64 {
        let mut best = (0.0, if lowest { f64::MAX } else { f64::MIN });
        for k in 0..16 {
            let hd = k as f64 * 22.5;
            let d = engine.offset_dir(centre, hd, dist);
            let g = engine.generator.column(d, p.block_size).height_m as f64;
            if (lowest && g < best.1) || (!lowest && g > best.1) {
                best = (hd, g);
            }
        }
        best.0
    };
    let approach = best_heading(peak_dir, 4000.0, true);
    let horizon = |h: f64| (2.0 * p.radius * h.max(0.0)).sqrt();

    struct Shot {
        name: &'static str,
        cam: Camera,
        hide_water: bool,
        markers: Vec<Marker>,
        caption: String,
    }
    let mut shots: Vec<Shot> = Vec::new();

    // (a) the highest range from the ground, 4 km away, eye height 1.7 m.
    {
        let cam_dir = engine.offset_dir(peak_dir, approach, 4000.0);
        let (heading, dist) = engine.heading_to(cam_dir, peak_dir);
        let ground = engine.ground(cam_dir);
        let pitch = ((peak_h - ground - 1.7) / dist).atan().to_degrees() * 0.5;
        let mut m = marker_pair(peak_dir, &engine);
        m.extend(marker_pair(engine.offset_dir(cam_dir, heading + 8.0, 60.0), &engine));
        shots.push(Shot {
            name: "a_range_ground",
            cam: engine.camera_at(cam_dir, 1.7, heading, pitch),
            hide_water: false,
            markers: m,
            caption: format!("(a) highest range from the ground | {scale_label} | {world_label} | peak {peak_h:.0} m above sea | camera {:.1} km from peak, eye 1.7 m AGL, ground {ground:.0} m | markers: red 100 m, white 2 m (at peak and 60 m ahead)", dist / 1000.0),
        });
    }
    // (b) the same range from 1 km altitude, looking across it.
    {
        let cam_dir = engine.offset_dir(peak_dir, approach, 6000.0);
        let (heading, dist) = engine.heading_to(cam_dir, peak_dir);
        let alt = 1000.0;
        let pitch = ((peak_h - alt) / dist).atan().to_degrees();
        shots.push(Shot {
            name: "b_range_1km",
            cam: engine.camera_asl(cam_dir, alt, heading, pitch),
            hide_water: false,
            markers: marker_pair(peak_dir, &engine),
            caption: format!("(b) highest range from 1 km altitude | {scale_label} | {world_label} | peak {peak_h:.0} m above sea | camera {:.1} km from peak at 1000 m ASL | markers at peak: red 100 m, white 2 m", dist / 1000.0),
        });
    }
    // (c) side-on silhouette against the sky from near sea level.
    {
        let side = approach + 90.0;
        let reach = (0.85 * (horizon(1.7) + horizon(peak_h))).min(25_000.0);
        let cam_dir = engine.offset_dir(peak_dir, side, reach);
        let (heading, dist) = engine.heading_to(cam_dir, peak_dir);
        let ground = engine.ground(cam_dir);
        shots.push(Shot {
            name: "c_range_silhouette",
            cam: engine.camera_at(cam_dir, 1.7, heading, 1.5),
            hide_water: false,
            markers: marker_pair(peak_dir, &engine),
            caption: format!("(c) range silhouette from near sea level | {scale_label} | {world_label} | peak {peak_h:.0} m above sea | camera {:.1} km from peak, eye 1.7 m AGL, ground {ground:.0} m | markers at peak: red 100 m, white 2 m", dist / 1000.0),
        });
    }
    // (d) the deepest trench, water hidden, low angle: the camera sits on the
    // trench wall, 3 km from the deepest point, 40 m above the floor there.
    {
        let floor = trench_h.max(bottom_m);
        let depth = -floor;
        // The basin is tens of km wide (planetgen cells are ~30 km), so the
        // camera backs off in proportion to the depth it has to show.
        let reach = (depth * 6.0).clamp(3000.0, 15_000.0);
        let hd = best_heading(trench_dir, reach, false);
        let cam_dir = engine.offset_dir(trench_dir, hd, reach);
        let (heading, dist) = engine.heading_to(cam_dir, trench_dir);
        let ground = engine.generator.column(cam_dir, p.block_size).height_m.max(bottom_m as f32) as f64;
        // High enough that the floor is above the curvature horizon: the
        // drop over `reach` on this planet is reach^2 / 2R, plus a margin.
        let alt = ground + reach * reach / (2.0 * p.radius) + 150.0;
        let pitch = ((floor - alt) / dist).atan().to_degrees();
        let clipped = if trench_h < bottom_m { format!(", floor clipped at bedrock {bottom_m:.0} m (true {trench_h:.0} m)") } else { String::new() };
        shots.push(Shot {
            name: "d_trench",
            cam: engine.camera_asl(cam_dir, alt, heading, pitch),
            hide_water: true,
            markers: marker_pair(trench_dir, &engine),
            caption: format!("(d) deepest trench, water hidden | {scale_label} | {world_label} | floor {floor:.0} m below sea{clipped} | camera {:.1} km from the deepest point at {alt:.0} m ASL, ground there {ground:.0} m | markers at floor: red 100 m, white 2 m", dist / 1000.0),
        });
    }
    // (e) a coast with its shelf and slope, water hidden.
    {
        let hd = best_heading(coast_dir, 3000.0, true);
        let cam_dir = engine.offset_dir(coast_dir, hd, 3000.0);
        let (heading, dist) = engine.heading_to(cam_dir, coast_dir);
        let coast_h = engine.generator.column(coast_dir, p.block_size).height_m as f64;
        let sea_floor = engine.generator.column(cam_dir, p.block_size).height_m as f64;
        let alt = 400.0;
        shots.push(Shot {
            name: "e_coast_shelf",
            cam: engine.camera_asl(cam_dir, alt, heading, -14.0),
            hide_water: true,
            markers: marker_pair(coast_dir, &engine),
            caption: format!("(e) steep coast with shelf and slope, water hidden | {scale_label} | {world_label} | coast point {coast_h:.0} m, sea floor under camera {sea_floor:.0} m | camera {:.1} km offshore at {alt:.0} m ASL | markers at coast: red 100 m, white 2 m", dist / 1000.0),
        });
    }

    // (f) a quarry cut on the hills: a 48 m pit, 28 m deep, seen from its
    // floor so the wall shows the soil and the beds as real blocks.
    {
        let hills = gnomonic_dir(HILLS.0, HILLS.1, HILLS.2);
        let pit = engine.offset_dir(hills, 0.0, 120.0);
        let side = (48.0 / p.block_size) as u32;
        let depth = (28.0 / p.block_size) as u32;
        let cam_dir = engine.offset_dir(pit, 180.0, 14.0);
        let ground = engine.ground(pit);
        let (heading, _) = engine.heading_to(cam_dir, pit);
        let mut m = vec![Marker { dir: pit, height_blocks: depth, width_blocks: side, block: b.air, below: true }];
        m.extend(marker_pair(engine.offset_dir(pit, 0.0, 30.0), &engine));
        shots.push(Shot {
            name: "f_strata_cut",
            cam: engine.camera_asl(cam_dir, ground - 28.0 + 1.7, heading, 8.0),
            hide_water: false,
            markers: m,
            caption: format!("(f) quarry cut on the hills, {side} m square and {depth} m deep | {scale_label} | {world_label} | camera on the pit floor, eye 1.7 m, facing the north wall 38 m away | soil over hashed 2-6 block beds, folded; marker red 100 m, white 2 m behind the wall"),
        });
    }

    for s in shots.into_iter().filter(|s| opts.which.as_deref().is_none_or(|w| w.split(',').any(|n| s.name.starts_with(n)))) {
        let e = Arc::new(engine.with_markers(s.markers.clone(), s.hide_water, true));
        h.set_engine(e);
        let (secs, _, ok) = h.settle(&s.cam, Duration::from_secs(300));
        let path = format!("{}/{}.png", opts.out, s.name);
        h.save(&path, Some(&s.caption));
        let (lat, lon, alt) = s.cam.lat_lon_alt(&p);
        println!("{} -> {path} (settled {secs:.1} s{}; cam lat {lat:.4} lon {lon:.4} alt {alt:.0} m; {} draws)", s.name, if ok { "" } else { " TIMEOUT" }, h.streamer.stats.drawn);
    }
}

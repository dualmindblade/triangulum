//! Scripted play harness: drive the game's own player, world and renderer
//! from a plain-text script, headless, on a fixed 60 Hz step. Scripts are
//! reproducible play sessions that leave frames and machine-readable state
//! behind, and fail loudly (exit code 1) when an assert does not hold.
//!
//!   cargo run --release -- --play game/scripts/m1-done.play [--out DIR] [--keep-save]
//!
//! Output: `game/out/play/<script stem>/` with NAME.png frames, NAME.json
//! state sidecars and run.log (the transcript). The run uses its own save
//! directory, `game/saves/_play/<stem>/`, wiped at the start unless
//! `--keep-save`, so runs repeat exactly.
//!
//! THIS HEADER IS THE COMMAND LIST. A command missing here is a bug.
//!
//! ---- placement ------------------------------------------------------
//!   at LAT LON [ALT_M]      teleport. With ALT_M (metres above sea level)
//!                           the player hovers (fly) or falls (walk) from
//!                           there; without it, stand on the ground.
//!   atblock FACE I J K      stand on top of lattice block (FACE, I, J, K)
//!   look HEADING PITCH      absolute view angles, degrees (0 = north, +east)
//!   lookaxis AXIS PITCH     face exactly along a lattice axis: +i -i +j -j
//!   aim DI DJ DK            look at the lattice point (feet cell centre +
//!                           DI, + DJ, feet layer + DK): the way to target a
//!                           particular block face deterministically
//!   turn DH DP              relative view change, degrees
//!   mode walk|fly           like the G key
//!
//! ---- time and input (fixed 60 Hz) -----------------------------------
//!   hold KEYS SECONDS       keys held: w a s d space shift ctrl, written
//!                           as "w", "wa", "w,space" or "w+shift" (ctrl
//!                           sneaks on the ground, dives in water, descends
//!                           in fly mode)
//!   tap break|place|jump|g|f|1..9
//!                           one action: left click, right click, a jump
//!                           step, walk/fly toggle, water toggle, hotbar
//!   wait SECONDS            time passes with no input (gravity acts)
//!   select N                hotbar slot 1-9
//!   speed M_PER_S           fly speed (scroll wheel), 1..5000
//!   travel SECONDS [KEYS]   like hold (default "w"), but every step also
//!                           streams and renders a frame and checks the
//!                           coverage tree (no overlapping draws, no
//!                           visible drawn chunk without a mesh, nothing
//!                           uncovered); reports travel_frames,
//!                           travel_violations, travel_overlaps_max,
//!                           travel_gaps_max, travel_uncovered_max,
//!                           travel_faces, travel_cpu_avg_ms / p99 / max
//!
//! ---- world ---------------------------------------------------------
//!   water on|off            show or hide water (rebuilds the generator)
//!   save                    write unsaved chunks now
//!   reload                  save, drop every chunk and the edit store, and
//!                           load the save from disk: quit and restart
//!
//! ---- output and checks ---------------------------------------------
//!   shot NAME               settled frame (streaming drained) with the HUD
//!                           as NAME.png, plus NAME.json state
//!   state NAME              NAME.json only
//!   bench N                 render N frames at the current pose; reports
//!                           avg / p95 / worst wall ms and the GPU time
//!   log TEXT...             annotate the transcript
//!   column DI DJ            print the blocks of a column near the feet
//!                           (layers -4..+3 relative to the feet block)
//!   assert FIELD [OP] VALUE [TOL]
//!                           OP: == != < <= > >= ~ (approx, TOL default
//!                           0.01); omitted means ==. VALUE: number,
//!                           true/false, a string, or none.
//!                           FIELD is any key of the state sidecar:
//!                             mode lat lon alt alt_agl ground face i j k
//!                             fi fj fk x y z heading pitch on_ground
//!                             in_water head_in_water vel_up speed selected
//!                             selected_block block_below edits dirty
//!                             saved_chunks hide_water crosshair
//!                             crosshair_face/i/j/k crosshair_dist
//!                             loaded inflight drawn (after a shot/bench)
//!                           or block[F,I,J,K]  the block at lattice coords
//!                           or block_rel[DI,DJ,DK]  relative to the feet block

use crate::bench::Harness;
use crate::grid::BlockPos;
use crate::player::{Input, Mode, STEP};
use crate::session::Session;
use crate::ui::{Hud, draw_hud, outline_segments};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

pub struct PlayOptions {
    pub script: String,
    pub out: String,
    pub width: u32,
    pub height: u32,
    pub backend: Option<String>,
}

struct Run {
    session: Session,
    harness: Harness,
    log: String,
    failed: usize,
    asserts: usize,
    last_bench: serde_json::Value,
    last_stream: serde_json::Value,
    out: String,
}

impl Run {
    fn say(&mut self, s: impl AsRef<str>) {
        let s = s.as_ref();
        println!("{s}");
        self.log.push_str(s);
        self.log.push('\n');
    }

    fn steps(&mut self, input: Input, seconds: f64) {
        let n = (seconds * 60.0).round().max(0.0) as usize;
        for _ in 0..n {
            self.session.step(&input, STEP);
        }
    }

    fn state(&self) -> serde_json::Value {
        let mut v = self.session.state();
        if let serde_json::Value::Object(m) = &mut v {
            if let serde_json::Value::Object(s) = &self.last_stream {
                for (k, val) in s {
                    m.insert(k.clone(), val.clone());
                }
            }
            if let serde_json::Value::Object(b) = &self.last_bench {
                for (k, val) in b {
                    m.insert(k.clone(), val.clone());
                }
            }
        }
        v
    }

    fn record_stream(&mut self) {
        let st = &self.harness.streamer.stats;
        self.last_stream = serde_json::json!({
            "loaded": st.loaded, "inflight": st.inflight, "drawn": st.drawn, "pending": st.pending,
            "fallback_draws": st.fallback_draws, "awaiting_upload": st.awaiting_upload, "remeshed": st.remeshed,
            "stale_results": st.stale_results, "alloc_failures": st.alloc_failures,
        });
    }

    fn write_state(&mut self, name: &str) {
        let v = self.state();
        let path = format!("{}/{name}.json", self.out);
        std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let brief = format!(
            "  state {name}: {} lat {:.5} lon {:.5} alt {:.2} m (agl {:.2}) face {} i {} j {} k {} ground {} water {} crosshair {} edits {} dirty {}",
            v["mode"].as_str().unwrap_or("?"),
            v["lat"].as_f64().unwrap_or(0.0),
            v["lon"].as_f64().unwrap_or(0.0),
            v["alt"].as_f64().unwrap_or(0.0),
            v["alt_agl"].as_f64().unwrap_or(0.0),
            v["face"],
            v["i"],
            v["j"],
            v["k"],
            v["on_ground"],
            v["in_water"],
            v["crosshair"],
            v["edits"],
            v["dirty"]
        );
        self.say(brief);
    }

    fn shot(&mut self, name: &str) {
        let cam = self.session.player.camera();
        let (secs, frames, ok) = self.harness.settle(&cam, Duration::from_secs(180));
        self.harness.frame(&cam);
        self.record_stream();
        let engine = self.session.engine.clone();
        let (w, h) = (self.harness.gpu.width, self.harness.gpu.height);
        let aspect = w as f64 / h as f64;
        let pick = self.session.pick();
        let p = &self.session.player;
        let ground = engine.ground(p.pos.normalize());
        let hud = Hud {
            lat: crate::grid::Planet::lat_lon_deg(p.pos).0,
            lon: crate::grid::Planet::lat_lon_deg(p.pos).1,
            alt: cam.altitude(&engine.planet),
            alt_agl: cam.altitude(&engine.planet) - ground,
            block: pick.map(|h| (h.block, engine.registry.def(h.id).name.clone(), h.t)),
            speed: p.fly_speed,
            loaded: self.harness.streamer.stats.loaded,
            drawn_per_lod: self.harness.streamer.stats.drawn_per_lod,
            loaded_per_lod: self.harness.streamer.stats.loaded_per_lod,
            preset: "script".into(),
            hide_water: engine.hide_water,
            mode: if p.mode == Mode::Walk { "walk".into() } else { "fly".into() },
            on_ground: p.on_ground,
            in_water: p.in_water,
            hotbar: engine.hotbar.iter().map(|&id| (engine.registry.def(id).name.clone(), engine.registry.def(id).color)).collect(),
            selected: p.selected,
            outline: pick.map_or(Vec::new(), |hit| outline_segments(&engine.planet, &cam, aspect, w, h, hit.face, hit.cell)),
            edits: engine.edits.len(),
            dirty: engine.edits.dirty_count(),
            ..Default::default()
        };
        let path = format!("{}/{name}.png", self.out);
        self.harness.save_with_ui(&path, &|ctx| draw_hud(ctx, &hud, true));
        self.say(format!("  shot {name}: {path} (settled in {secs:.1} s / {frames} frames{}; {} draws)", if ok { "" } else { " TIMEOUT" }, self.harness.streamer.stats.drawn));
        self.write_state(name);
    }

    /// Move with `input` held for `seconds`, streaming and rendering a
    /// frame every step and checking the cover after each one.
    fn travel(&mut self, input: Input, seconds: f64) {
        let n = (seconds * 60.0).round().max(0.0) as usize;
        let mut violations = 0usize;
        let (mut overlaps, mut gaps, mut uncovered) = (0usize, 0usize, 0usize);
        let mut faces = std::collections::BTreeSet::new();
        let mut cpu = Vec::with_capacity(n);
        let mut nodes_max = 0usize;
        let (mut tree_max, mut recv_max, mut lists_max) = (0.0f64, 0.0f64, 0.0f64);
        let (mut split_max, mut range_max, mut misses_max) = (0.0f64, 0.0f64, 0usize);
        let (mut ranges_inflight_max, mut range_job_max) = (0usize, 0u32);
        let (mut policy_max, mut structure_max, mut counts_max) = (0.0f64, 0.0f64, 0.0f64);
        let t0 = Instant::now();
        for _ in 0..n {
            self.session.step(&input, STEP);
            let cam = self.session.player.camera();
            let s = self.harness.frame(&cam);
            cpu.push(s.cpu_ms);
            let st = &self.harness.streamer.stats;
            tree_max = tree_max.max(st.traverse_ms);
            recv_max = recv_max.max(st.recv_ms);
            lists_max = lists_max.max(st.cull_ms);
            split_max = split_max.max(st.split_ms);
            range_max = range_max.max(st.range_ms);
            misses_max = misses_max.max(st.range_misses);
            ranges_inflight_max = ranges_inflight_max.max(st.ranges_inflight);
            policy_max = policy_max.max(st.policy_ms);
            structure_max = structure_max.max(st.structure_ms);
            counts_max = counts_max.max(st.counts_ms);
            range_job_max = range_job_max.max(st.range_job_us);
            let r = self.harness.streamer.check_cover();
            if !r.ok() {
                violations += 1;
            }
            overlaps = overlaps.max(r.overlaps);
            gaps = gaps.max(r.gaps);
            uncovered = uncovered.max(r.uncovered);
            nodes_max = nodes_max.max(r.nodes);
            faces.insert(crate::player::Frame::at(&self.session.engine.planet, self.session.player.pos).face);
        }
        self.record_stream();
        let mut sorted = cpu.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let avg = cpu.iter().sum::<f64>() / n.max(1) as f64;
        let p99 = sorted.get(((n as f64 - 1.0) * 0.99).round() as usize).copied().unwrap_or(0.0);
        let max = sorted.last().copied().unwrap_or(0.0);
        let faces_s: Vec<String> = faces.iter().map(|f| f.to_string()).collect();
        if let serde_json::Value::Object(m) = &mut self.last_stream {
            m.insert("travel_frames".into(), n.into());
            m.insert("travel_violations".into(), violations.into());
            m.insert("travel_overlaps_max".into(), overlaps.into());
            m.insert("travel_gaps_max".into(), gaps.into());
            m.insert("travel_uncovered_max".into(), uncovered.into());
            m.insert("travel_faces".into(), faces_s.join(",").into());
            m.insert("travel_nodes_max".into(), nodes_max.into());
            m.insert("travel_cpu_avg_ms".into(), avg.into());
            m.insert("travel_cpu_p99_ms".into(), p99.into());
            m.insert("travel_cpu_max_ms".into(), max.into());
            m.insert("travel_tree_max_ms".into(), tree_max.into());
            m.insert("travel_results_max_ms".into(), recv_max.into());
            m.insert("travel_lists_max_ms".into(), lists_max.into());
            m.insert("travel_range_jobs_max".into(), ranges_inflight_max.into());
        }
        let wall = t0.elapsed().as_secs_f64();
        let faces_s = faces_s.join(",");
        let job_ms = range_job_max as f64 / 1e3;
        self.say(format!(
            "  travel {n} frames in {wall:.1} s: faces {faces_s}; cover violations {violations} (overlaps max {overlaps}, gaps max {gaps}, uncovered max {uncovered}); nodes max {nodes_max}; cpu avg {avg:.2} ms p99 {p99:.2} max {max:.2}; worst phases: tree {tree_max:.2} (policy {policy_max:.2}, structure {structure_max:.2} of which splits {split_max:.2}, counts {counts_max:.2}; inline ranges {range_max:.2} / {misses_max}), results {recv_max:.2}, draw lists {lists_max:.2}; range jobs in flight max {ranges_inflight_max}, worst {job_ms:.2} ms"
        ));
    }

    fn bench(&mut self, n: usize) {
        let cam = self.session.player.camera();
        self.harness.settle(&cam, Duration::from_secs(180));
        let mut wall = Vec::with_capacity(n);
        let mut gpu = Vec::with_capacity(n);
        for _ in 0..n {
            let s = self.harness.frame(&cam);
            wall.push(s.wall_ms);
            gpu.push(s.gpu_ms);
        }
        self.record_stream();
        let mut sorted = wall.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let avg = wall.iter().sum::<f64>() / n.max(1) as f64;
        let p95 = sorted[((n as f64 - 1.0) * 0.95).round() as usize];
        let worst = *sorted.last().unwrap_or(&0.0);
        let gavg = gpu.iter().sum::<f64>() / n.max(1) as f64;
        self.last_bench = serde_json::json!({ "bench_frames": n, "bench_avg_ms": avg, "bench_p95_ms": p95, "bench_worst_ms": worst, "bench_gpu_avg_ms": gavg, "bench_fps": 1000.0 / avg });
        self.say(format!("  bench {n} frames: wall avg {avg:.2} ms (p95 {p95:.2}, worst {worst:.2}) = {:.0} fps; gpu avg {gavg:.2} ms; {} draws", 1000.0 / avg, self.harness.streamer.stats.drawn));
    }

    /// Evaluate `assert FIELD [OP] VALUE [TOL]`.
    fn assert(&mut self, args: &[&str], line_no: usize) {
        self.asserts += 1;
        let ops = ["==", "!=", "<", "<=", ">", ">=", "~"];
        let (field, op, value, tol) = match args {
            [f, o, v, rest @ ..] if ops.contains(o) => (*f, *o, *v, rest.first().and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.01)),
            [f, v] => (*f, "==", *v, 0.01),
            _ => {
                self.failed += 1;
                self.say(format!("ASSERT FAILED (line {line_no}): cannot parse {:?}", args));
                return;
            }
        };
        let actual = self.field(field);
        let Some(actual) = actual else {
            self.failed += 1;
            self.say(format!("ASSERT FAILED (line {line_no}): no field {field}"));
            return;
        };
        let pass = compare(&actual, op, value, tol);
        let shown = match &actual {
            serde_json::Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        if pass {
            self.say(format!("  assert ok: {field} {op} {value} (actual {shown})"));
        } else {
            self.failed += 1;
            self.say(format!("ASSERT FAILED (line {line_no}): {field} {op} {value}, actual {shown}"));
        }
    }

    fn field(&self, field: &str) -> Option<serde_json::Value> {
        if let Some(inner) = field.strip_prefix("block[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 4 {
                return None;
            }
            return Some(serde_json::Value::String(self.session.block_name(v[0] as u8, v[1], v[2], v[3])));
        }
        if let Some(inner) = field.strip_prefix("block_rel[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 3 {
                return None;
            }
            return Some(serde_json::Value::String(self.session.block_name_rel(v[0], v[1], v[2])));
        }
        self.state().get(field).cloned()
    }
}

fn compare(actual: &serde_json::Value, op: &str, value: &str, tol: f64) -> bool {
    use serde_json::Value as V;
    match actual {
        V::Null => match op {
            "==" => value == "none" || value == "null",
            "!=" => value != "none" && value != "null",
            _ => false,
        },
        V::Bool(b) => {
            let want = match value {
                "true" => true,
                "false" => false,
                _ => return false,
            };
            match op {
                "==" => *b == want,
                "!=" => *b != want,
                _ => false,
            }
        }
        V::Number(n) => {
            let a = n.as_f64().unwrap_or(f64::NAN);
            let Ok(w) = value.parse::<f64>() else { return false };
            match op {
                "==" => a == w,
                "!=" => a != w,
                "<" => a < w,
                "<=" => a <= w,
                ">" => a > w,
                ">=" => a >= w,
                "~" => (a - w).abs() <= tol,
                _ => false,
            }
        }
        V::String(s) => match op {
            "==" => s == value,
            "!=" => s != value,
            _ => false,
        },
        _ => false,
    }
}

fn parse_keys(spec: &str) -> Result<Input, String> {
    let mut input = Input::default();
    let tokens: Vec<String> = if spec.contains([',', '+']) {
        spec.split([',', '+']).map(|s| s.trim().to_lowercase()).collect()
    } else if ["space", "shift", "ctrl", "jump", "sprint", "down"].contains(&spec.to_lowercase().as_str()) {
        vec![spec.to_lowercase()]
    } else {
        spec.chars().map(|c| c.to_lowercase().to_string()).collect()
    };
    for t in tokens {
        match t.as_str() {
            "w" => input.forward = true,
            "s" => input.back = true,
            "a" => input.left = true,
            "d" => input.right = true,
            "space" | "jump" => input.jump = true,
            "shift" | "sprint" => input.sprint = true,
            "ctrl" | "down" => input.down = true,
            "" => {}
            other => return Err(format!("unknown key {other}")),
        }
    }
    Ok(input)
}

pub fn run_play(session: Session, opts: PlayOptions) -> i32 {
    let text = match std::fs::read_to_string(&opts.script) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("read {}: {e}", opts.script);
            return 2;
        }
    };
    let _ = std::fs::create_dir_all(&opts.out);
    let engine = session.engine.clone();
    let harness = Harness::new(engine, opts.width, opts.height, &opts.out, opts.backend.as_deref());
    let mut run = Run { session, harness, log: String::new(), failed: 0, asserts: 0, last_bench: serde_json::Value::Null, last_stream: serde_json::Value::Null, out: opts.out.clone() };
    run.say(format!("play {} -> {}", opts.script, opts.out));
    let t0 = Instant::now();
    let mut errors = 0usize;
    for (n, raw) in text.lines().enumerate() {
        let line_no = n + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();
        let args = &parts[1..];
        run.say(format!("> {line}"));
        let num = |i: usize| -> Option<f64> { args.get(i).and_then(|s| s.parse::<f64>().ok()) };
        let mut bad = |run: &mut Run, msg: &str| {
            errors += 1;
            run.say(format!("ERROR (line {line_no}): {msg}"));
        };
        match cmd.as_str() {
            "at" | "teleport" => match (num(0), num(1)) {
                (Some(lat), Some(lon)) => {
                    let dir = crate::grid::Planet::dir_of_lat_lon_deg(lat, lon);
                    run.session.teleport(dir, num(2));
                }
                _ => bad(&mut run, "at LAT LON [ALT_M]"),
            },
            "atblock" => match (num(0), num(1), num(2), num(3)) {
                (Some(f), Some(i), Some(j), Some(k)) => run.session.teleport_block(BlockPos { face: f as u8, i: i as u32, j: j as u32, k: k as u32 }),
                _ => bad(&mut run, "atblock FACE I J K"),
            },
            "look" => match (num(0), num(1)) {
                (Some(h), Some(p)) => {
                    run.session.player.heading = h.to_radians();
                    run.session.player.pitch = p.to_radians().clamp(-1.55, 1.55);
                }
                _ => bad(&mut run, "look HEADING PITCH"),
            },
            "lookaxis" => match (args.first(), num(1)) {
                (Some(axis), Some(p)) => {
                    let f = crate::player::Frame::at(&run.session.engine.planet, run.session.player.pos);
                    let v = match *axis {
                        "+i" => f.ei,
                        "-i" => -f.ei,
                        "+j" => f.ej,
                        "-j" => -f.ej,
                        _ => {
                            bad(&mut run, "lookaxis +i|-i|+j|-j PITCH");
                            continue;
                        }
                    };
                    let (north, east, _) = crate::camera::Camera::local_frame(run.session.player.pos);
                    run.session.player.heading = v.dot(east).atan2(v.dot(north));
                    run.session.player.pitch = p.to_radians().clamp(-1.55, 1.55);
                }
                _ => bad(&mut run, "lookaxis AXIS PITCH"),
            },
            "aim" => match (num(0), num(1), num(2)) {
                (Some(di), Some(dj), Some(dk)) => {
                    let planet = run.session.engine.planet;
                    let f = crate::player::Frame::at(&planet, run.session.player.pos);
                    let (ci, cj, ck) = f.feet_cell();
                    let target = planet.pos(f.face, ci as f64 + 0.5 + di, cj as f64 + 0.5 + dj, ck as f64 + dk);
                    let eye = run.session.player.eye();
                    let d = (target - eye).normalize();
                    let (north, east, up) = crate::camera::Camera::local_frame(eye);
                    run.session.player.heading = d.dot(east).atan2(d.dot(north));
                    run.session.player.pitch = d.dot(up).clamp(-1.0, 1.0).asin().clamp(-1.55, 1.55);
                }
                _ => bad(&mut run, "aim DI DJ DK"),
            },
            "column" => match (num(0), num(1)) {
                (Some(di), Some(dj)) => {
                    let f = crate::player::Frame::at(&run.session.engine.planet, run.session.player.pos);
                    let (fi, fj, ck) = f.feet_cell();
                    let (ci, cj) = (fi + di as i64, fj + dj as i64);
                    let names: Vec<String> = (-4..=3).rev().map(|dk| format!("{dk:+}:{}", run.session.block_name(f.face, ci, cj, ck + dk))).collect();
                    run.say(format!("  column face {} i {} j {} (feet layer {}): {}", f.face, ci, cj, ck, names.join(" ")));
                }
                _ => bad(&mut run, "column DI DJ"),
            },
            "turn" => match (num(0), num(1)) {
                (Some(h), Some(p)) => {
                    run.session.player.heading += h.to_radians();
                    run.session.player.pitch = (run.session.player.pitch + p.to_radians()).clamp(-1.55, 1.55);
                }
                _ => bad(&mut run, "turn DH DP"),
            },
            "mode" => match args.first().map(|s| s.to_lowercase()).as_deref() {
                Some("walk") => run.session.set_mode(Mode::Walk),
                Some("fly") => run.session.set_mode(Mode::Fly),
                _ => bad(&mut run, "mode walk|fly"),
            },
            "hold" => match (args.first(), num(1)) {
                (Some(keys), Some(secs)) => match parse_keys(keys) {
                    Ok(input) => run.steps(input, secs),
                    Err(e) => bad(&mut run, &e),
                },
                _ => bad(&mut run, "hold KEYS SECONDS"),
            },
            "wait" => match num(0) {
                Some(secs) => run.steps(Input::default(), secs),
                None => bad(&mut run, "wait SECONDS"),
            },
            "speed" => match num(0) {
                Some(v) if (1.0..=5000.0).contains(&v) => run.session.player.fly_speed = v,
                _ => bad(&mut run, "speed M_PER_S (1..5000)"),
            },
            "travel" => match num(0) {
                Some(secs) => match parse_keys(args.get(1).copied().unwrap_or("w")) {
                    Ok(input) => run.travel(input, secs),
                    Err(e) => bad(&mut run, &e),
                },
                None => bad(&mut run, "travel SECONDS [KEYS]"),
            },
            "tap" => match args.first().map(|s| s.to_lowercase()).as_deref() {
                Some("break") | Some("lmb") => {
                    let r = run.session.break_block();
                    run.say(format!("  break: {}", r.map_or("nothing in reach".to_string(), |b| format!("face {} i {} j {} k {}", b.face, b.i, b.j, b.k))));
                }
                Some("place") | Some("rmb") => {
                    let r = run.session.place_block();
                    run.say(format!("  place: {}", r.map_or("not placed".to_string(), |b| format!("face {} i {} j {} k {}", b.face, b.i, b.j, b.k))));
                }
                Some("jump") | Some("space") => run.steps(Input { jump: true, ..Default::default() }, STEP),
                Some("g") => run.session.toggle_mode(),
                Some("f") => {
                    let hide = !run.session.engine.hide_water;
                    let e = run.session.set_hide_water(hide);
                    run.harness.set_engine(e);
                }
                Some(d) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => run.session.select(d.parse::<usize>().unwrap() - 1),
                _ => bad(&mut run, "tap break|place|jump|g|f|1..9"),
            },
            "select" => match num(0) {
                Some(n) if (1.0..=9.0).contains(&n) => run.session.select(n as usize - 1),
                _ => bad(&mut run, "select 1..9"),
            },
            "water" => match args.first().map(|s| s.to_lowercase()).as_deref() {
                Some(v @ ("on" | "off")) => {
                    let e = run.session.set_hide_water(v == "off");
                    run.harness.set_engine(e);
                }
                _ => bad(&mut run, "water on|off"),
            },
            "save" => match run.session.save_now() {
                Ok(n) => run.say(format!("  saved {n} chunk(s)")),
                Err(e) => bad(&mut run, &format!("save: {e}")),
            },
            "reload" => match run.session.reload() {
                Ok((engine, report)) => {
                    run.harness.set_engine(engine);
                    run.say(format!("  reloaded {} chunk(s) from disk{}", report.chunks, if report.warnings.is_empty() { String::new() } else { format!("; warnings: {}", report.warnings.join("; ")) }));
                }
                Err(e) => bad(&mut run, &format!("reload: {e}")),
            },
            "shot" => match args.first() {
                Some(name) => run.shot(name),
                None => bad(&mut run, "shot NAME"),
            },
            "state" => match args.first() {
                Some(name) => run.write_state(name),
                None => bad(&mut run, "state NAME"),
            },
            "bench" => match num(0) {
                Some(n) if n >= 1.0 => run.bench(n as usize),
                _ => bad(&mut run, "bench N"),
            },
            "log" => {}
            "assert" => run.assert(args, line_no),
            other => bad(&mut run, &format!("unknown command {other}")),
        }
    }
    match run.session.save_now() {
        Ok(n) if n > 0 => run.say(format!("saved {n} chunk(s) at exit")),
        Ok(_) => {}
        Err(e) => {
            errors += 1;
            run.say(format!("ERROR: save at exit: {e}"));
        }
    }
    let mut summary = String::new();
    let _ = write!(summary, "done in {:.1} s: {} asserts, {} failed, {} errors", t0.elapsed().as_secs_f64(), run.asserts, run.failed, errors);
    run.say(summary);
    std::fs::write(format!("{}/run.log", opts.out), &run.log).unwrap();
    if run.failed > 0 || errors > 0 { 1 } else { 0 }
}

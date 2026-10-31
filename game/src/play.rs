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
//!   find flat|shore|river|lake|waterfall
//!                           stand (walk mode) at the nearest such site,
//!                           deterministically: a dry column with that water
//!                           1-3 columns away along a lattice axis (a fall:
//!                           beside a lip with a 3+ block drop), or a 5 x 5
//!                           patch of level dry ground; looks at the water
//!                           (a fall: below its lip; flat: along +i). Sets
//!                           find_kind, find_face/i/j/k (the block stood
//!                           on), find_target_face/i/j/k, find_axis,
//!                           find_gap, find_drop, find_dist_m, find_columns
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
//!   tick N                  N simulation ticks (20 Hz: 3 player steps each,
//!                           no input); "tick 20" is one second
//!   sim on|off              run or pause the simulation (ticks stop; edits
//!                           still log, and all notify when it resumes)
//!   sim radius M            active region radius in metres (--sim-radius)
//!   sim period N            ticks per water step (--water-period; 1)
//!   sim slide on|off        powder and water go over ledges. Off is the rules
//!                           before B4 (--no-slide): the film rule goes to
//!                           keep too, and on leaves it there
//!   sim film MODE           what becomes of level-1 water (--film): keep,
//!                           seek, merge, dry, seek+dry
//!   sim reach N             how far a film runs to a drop (--film-reach)
//!   sim dry SECONDS         how long a film waits to dry (--film-dry-s)
//!   select N                hotbar slot 1-9
//!   hotbar A,B,...          what keys 1-9 hold, by block name (M3b; the
//!                           harness otherwise keeps the default hotbar,
//!                           whatever blocks.json says)
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
//!   fill DI DJ DK DI2 DJ2 DK2 BLOCK [LEVEL]
//!                           set a box of cells relative to the feet block,
//!                           corners inclusive, at once (LEVEL: water's, 255
//!                           if left out). A scene-building shortcut, written
//!                           straight into the world: not a player input, so
//!                           a replay being recorded notes it
//!   water on|off            show or hide water (rebuilds the generator)
//!   save                    write unsaved chunks now
//!   reload                  save, drop every chunk and the edit store, and
//!                           load the save from disk: quit and restart
//!
//! ---- replays (M2c, replay.rs) ----------------------------------------
//!   replay record FILE      start recording a replay to FILE (under the out
//!                           directory), exactly as F10 does in the game:
//!                           every command after this is in it
//!   replay stop             finish it; sets replay_steps, replay_events,
//!                           replay_checks_written, replay_notes (things the
//!                           recorder could not capture: must be 0),
//!                           replay_bytes
//!   replay check FILE       play FILE in a fresh session built from its own
//!                           snapshot (scratch save saves/_replay/<stem>),
//!                           here, and compare; sets replay_ok, replay_checks,
//!                           replay_matched, replay_steps_played,
//!                           replay_ended, replay_end_match,
//!                           replay_divergence_step (none when it matched)
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
//!   levels DI DJ DI2 DJ2 DK print the water levels of one layer as a grid
//!   heights DI DJ DI2 DJ2 DK_LOW DK_HIGH
//!                           print how far above DK_LOW the top sand or
//!                           gravel of each column stands, as a grid
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
//!                             sim_enabled sim_tick falling queued frozen
//!                             sim_active_chunks falls slides landings vanished
//!                             tick_ms_last tick_ms_avg tick_ms_max
//!                             sim_hash (the world-state hash; computed on demand)
//!                             water_moved water_writes water_in water_out
//!                             water_placed water_removed water_displaced
//!                             water_lost water_dried drying water_balance
//!                             water_updates_last
//!                             water_period flow sim_radius (units: 255 = a cell)
//!                             sky_seconds sky_speed (the session's sky
//!                             clock) steps recording
//!                             video_frames video_falling_drawn_max (after
//!                             a recording's stop)
//!                           or block[F,I,J,K]  the block at lattice coords
//!                           or block_rel[DI,DJ,DK]  relative to the feet block
//!                           or level[F,I,J,K] / level_rel[DI,DJ,DK]  the water
//!                              level 0..255 of a cell
//!                           or water_rel[DI,DJ,DK,DI2,DJ2,DK2]  the total water
//!                              (units) in that box of cells, corners inclusive
//!                           or wet_rel[DI,DJ,DK,DI2,DJ2,DK2]  the water cells in
//!                              that box, and film_rel[...] those at level 1
//!                           or falling_rel[DI,DJ]  blocks in flight in that column
//!                           or cover[F,I,J,K] / cover_rel[DI,DJ,DK]  what the
//!                              streamer's cut shows at that block, after a
//!                              shot: drawn:L (a drawn chunk at LOD L with a
//!                              mesh), empty:L, gap:L, hidden:L, or none
//!
//! ---- video (play/video.rs) ------------------------------------------
//!   record FILE [FPS]       start writing FILE (H.264 .mp4 under the out
//!                           directory, via ffmpeg on PATH or
//!                           TRIANGULUM_FFMPEG) at FPS (30): from now on
//!                           every 60 Hz step (hold, wait, tap jump, fly,
//!                           pan) is a video tick, and every 60/FPS-th tick
//!                           writes a frame. The first frame is written at once.
//!   stop                    finish the file; reports frames, size and
//!                           settle timeouts (an error if any frame timed out)
//!   caption [+S] TEXT...    a caption cue from this frame (or S seconds of
//!                           video later, to place cues inside a fly) until
//!                           the next cue ("caption -" ends it); written on stop as
//!                           FILE.captions.json [{start, end, text}] for the
//!                           editor (scripts/showcase.py burns them in)
//!   recmode settle|live     settle (default): drain streaming before each
//!                           written frame, as `shot` does. live: one
//!                           streaming update + render per tick, paced to
//!                           real time (what a 60 fps player sees)
//!   hud on|off              draw the game HUD (crosshair, block outline,
//!                           hotbar; no stats) on written frames (default off)
//!   settle                  drain streaming at the current pose (pre-warm)
//!   skytime HH:MM           local solar time at the eye's longitude
//!   sun ELEV [am|pm]        seek the clock to when the sun is ELEV degrees
//!                           above the horizon here (pm: afternoon, default)
//!   skyspeed F              sky clock speed on every step, game s per s
//!                           (default 0: the sky holds, as before). The
//!                           clock is the session's and advances on every
//!                           60 Hz step, whatever command takes it (hold,
//!                           wait, tick, travel, fly, pan, zoom; M2c)
//!   target FACE I J K | rel DI DJ DK | ll LAT LON ALT | off
//!                           a look-at point: a block centre, a point
//!                           relative to the feet cell (as aim), or lat/lon/alt
//!   key T LAT LON ALT [HEADING PITCH]
//!                           a camera key T seconds after the current pose,
//!                           eye ALT metres above sea level; "-" or an
//!                           omitted angle repeats the previous key's
//!   keyrel T DIST BEARING HEIGHT [HEADING PITCH]
//!                           a key DIST m from the target along BEARING
//!                           (degrees from north, or +i -i +j -j there),
//!                           HEIGHT m above the target
//!   fly [ease|in|out|none] [keys|path|target]
//!                           move the eye through the keys (cubic Hermite;
//!                           direction on the sphere, altitude in log
//!                           space), easing in/out (default both), looking
//!                           by the keys' angles, along the path (keys'
//!                           pitch), or at the target; leaves fly mode on.
//!                           Every frame is a step of the session (the
//!                           world simulates and the sky turns meanwhile).
//!                           Fails if the eye comes within 1 m of the ground
//!   pan SECONDS look H P | turn DH DP | lookaxis AXIS P | aim DI DJ DK | target
//!                           turn the view smoothly to where that command
//!                           would point it, stepping with no input
//!   fov DEGREES             vertical field of view of recorded frames (60).
//!                           Level of detail follows distance only, so a
//!                           narrow view shows far terrain as it is drawn there
//!   zoom SECONDS DEGREES    change the field of view smoothly, stepping
//!                           with no input

mod find;
mod video;

use crate::bench::Harness;
use crate::grid::BlockPos;
use crate::player::{Input, Mode, STEP};
use crate::replay::{self, Replay};
use crate::session::{Action, Button, Session};
use crate::sim::STEPS_PER_TICK;
use crate::ui::{Hud, draw_hud, outline_segments};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;
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
    /// The last `find` and `replay` results, as state fields.
    last_find: serde_json::Value,
    last_replay: serde_json::Value,
    out: String,
    video: video::Video,
}

impl Run {
    fn new(session: Session, harness: Harness, out: &str) -> Self {
        Run { session, harness, log: String::new(), failed: 0, asserts: 0, last_bench: serde_json::Value::Null, last_stream: serde_json::Value::Null, last_find: serde_json::Value::Null, last_replay: serde_json::Value::Null, out: out.to_string(), video: video::Video::default() }
    }

    /// `replay record FILE | replay stop | replay check FILE` (M2c): the
    /// recorder the app's F10 uses, and a playback of the file in a fresh
    /// session built from its snapshot (in a scratch save), in this process.
    fn replay_command(&mut self, args: &[&str]) -> Result<(), String> {
        match (args.first().copied(), args.get(1).copied()) {
            (Some("record"), Some(file)) => {
                let path = Path::new(&self.out).join(file);
                let p = self.session.start_recording(&path, "recorded by a harness script")?;
                self.say(format!("  recording a replay to {}", p.display()));
                Ok(())
            }
            (Some("stop"), _) => {
                let s = self.session.stop_recording()?;
                self.say(format!("  replay {}: {} steps, {} events, {} checks, {} notes; {:.1} KB ({:.1} KB of it the snapshot)", s.path.display(), s.steps, s.events, s.checks, s.notes, s.bytes as f64 / 1e3, s.snapshot_bytes as f64 / 1e3));
                self.last_replay = serde_json::json!({ "replay_steps": s.steps, "replay_events": s.events, "replay_checks_written": s.checks, "replay_notes": s.notes, "replay_bytes": s.bytes });
                Ok(())
            }
            (Some("check"), Some(file)) => {
                let path = Path::new(&self.out).join(file);
                let rep = replay::read(&path)?;
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("replay").to_string();
                let t0 = Instant::now();
                let mut s = replay::session_from(&self.session, &rep, &replay::scratch_dir(&stem))?;
                let report = replay::Runner::new(&rep, &s).run(&mut s);
                for l in report.text().lines() {
                    self.say(format!("  {l}"));
                }
                let secs = t0.elapsed().as_secs_f64();
                self.say(format!("  (played in {secs:.1} s)"));
                self.last_replay = serde_json::json!({
                    "replay_ok": report.ok(),
                    "replay_checks": report.checks,
                    "replay_matched": report.matched,
                    "replay_steps_played": report.steps,
                    "replay_ended": report.ended,
                    "replay_end_match": report.end_sha.as_ref().is_some_and(|(a, b)| a == b),
                    "replay_divergence_step": report.first.as_ref().map(|d| d.step),
                    "replay_secs": secs,
                });
                Ok(())
            }
            _ => Err("replay record FILE | replay stop | replay check FILE".into()),
        }
    }

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
            self.video_tick();
        }
    }

    /// `tick N`: exactly N simulation ticks with no input.
    fn ticks(&mut self, n: usize) {
        let before = self.session.sim.tick();
        for _ in 0..n * STEPS_PER_TICK as usize {
            self.session.step(&Input::default(), STEP);
        }
        self.say(format!("  ticked {} -> {}: {}", before, self.session.sim.tick(), self.session.sim.describe()));
    }

    /// Hand the harness what the next frames draw from the session: the
    /// blocks in flight and the sky clock (the session advances it on its
    /// steps; M2c).
    fn sync_falling(&mut self) {
        self.harness.falling = self.session.sim.view();
        self.harness.gpu.sky.clock = self.session.sky;
    }

    fn state(&self) -> serde_json::Value {
        let mut v = self.session.state();
        if let serde_json::Value::Object(m) = &mut v {
            if let serde_json::Value::Object(s) = &self.last_stream {
                for (k, val) in s {
                    m.insert(k.clone(), val.clone());
                }
            }
            for extra in [&self.last_bench, &self.last_find, &self.last_replay] {
                if let serde_json::Value::Object(b) = extra {
                    for (k, val) in b {
                        m.insert(k.clone(), val.clone());
                    }
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
        self.sync_falling();
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
            hotbar: engine.hotbar.iter().map(|&id| (engine.registry.def(id).label(), engine.registry.colour(id))).collect(),
            selected: p.selected,
            outline: pick.map_or(Vec::new(), |hit| outline_segments(&engine.planet, &cam, aspect, w, h, hit.face, hit.cell)),
            edits: engine.edits.len(),
            dirty: engine.edits.dirty_count(),
            sim_line: self.session.sim.describe(),
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
            self.sync_falling();
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
        self.sync_falling();
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

    fn field(&mut self, field: &str) -> Option<serde_json::Value> {
        if field == "sim_hash" {
            return Some(serde_json::Value::String(self.session.sim_hash()));
        }
        if let Some(inner) = field.strip_prefix("falling_rel[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 2 {
                return None;
            }
            return Some(serde_json::Value::from(self.session.falling_in_column_rel(v[0], v[1])));
        }
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
        if let Some(inner) = field.strip_prefix("level[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 4 {
                return None;
            }
            return Some(serde_json::Value::from(self.session.level_at(v[0] as u8, v[1], v[2], v[3])));
        }
        if let Some(inner) = field.strip_prefix("level_rel[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 3 {
                return None;
            }
            return Some(serde_json::Value::from(self.session.level_rel(v[0], v[1], v[2])));
        }
        if let Some(inner) = field.strip_prefix("water_rel[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 6 {
                return None;
            }
            return Some(serde_json::Value::from(self.session.water_in_box_rel((v[0], v[1], v[2]), (v[3], v[4], v[5]))));
        }
        for (prefix, film) in [("wet_rel[", false), ("film_rel[", true)] {
            if let Some(inner) = field.strip_prefix(prefix).and_then(|r| r.strip_suffix(']')) {
                let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
                if v.len() != 6 {
                    return None;
                }
                let (wet, level_one) = self.session.wet_in_box_rel((v[0], v[1], v[2]), (v[3], v[4], v[5]));
                return Some(serde_json::Value::from(if film { level_one } else { wet }));
            }
        }
        if let Some(inner) = field.strip_prefix("cover[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 4 {
                return None;
            }
            let bp = self.session.engine.world.resolve_block(v[0] as u8, v[1], v[2], v[3]);
            return Some(serde_json::Value::String(bp.map_or("none".to_string(), |bp| self.harness.streamer.cover_at(bp))));
        }
        if let Some(inner) = field.strip_prefix("cover_rel[").and_then(|r| r.strip_suffix(']')) {
            let v: Vec<i64> = inner.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
            if v.len() != 3 {
                return None;
            }
            let f = crate::player::Frame::at(&self.session.engine.planet, self.session.player.pos);
            let (ci, cj, ck) = f.feet_cell();
            let bp = self.session.engine.world.resolve_block(f.face, ci + v[0], cj + v[1], ck + v[2]);
            return Some(serde_json::Value::String(bp.map_or("none".to_string(), |bp| self.harness.streamer.cover_at(bp))));
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
    let mut run = Run::new(session, harness, &opts.out);
    // The harness's sky holds unless a script sets `skyspeed` (as before
    // M2c); the session advances it on its steps.
    run.session.sky.speed = 0.0;
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
            "levels" => match (num(0), num(1), num(2), num(3), num(4)) {
                (Some(i0), Some(j0), Some(i1), Some(j1), Some(dk)) => {
                    let (i0, j0, i1, j1, dk) = (i0 as i64, j0 as i64, i1 as i64, j1 as i64, dk as i64);
                    run.say(format!("  water levels, layer {dk:+}, i {i0:+}..{i1:+} across, j {j1:+}..{j0:+} down:"));
                    for dj in (j0..=j1).rev() {
                        let row: Vec<String> = (i0..=i1).map(|di| format!("{:3}", run.session.level_rel(di, dj, dk))).collect();
                        run.say(format!("  {}", row.join(" ")));
                    }
                }
                _ => bad(&mut run, "levels DI DJ DI2 DJ2 DK"),
            },
            "heights" => match (num(0), num(1), num(2), num(3), num(4), num(5)) {
                (Some(i0), Some(j0), Some(i1), Some(j1), Some(lo), Some(hi)) => {
                    let (i0, j0, i1, j1, lo, hi) = (i0 as i64, j0 as i64, i1 as i64, j1 as i64, lo as i64, hi as i64);
                    run.say(format!("  top of the powder above layer {lo:+}, i {i0:+}..{i1:+} across, j {j1:+}..{j0:+} down:"));
                    for dj in (j0..=j1).rev() {
                        let row: Vec<String> = (i0..=i1)
                            .map(|di| {
                                let top = (lo + 1..=hi).rev().find(|&dk| matches!(run.session.block_name_rel(di, dj, dk).as_str(), "sand" | "gravel"));
                                top.map_or("  .".to_string(), |dk| format!("{:3}", dk - lo))
                            })
                            .collect();
                        run.say(format!("  {}", row.join(" ")));
                    }
                }
                _ => bad(&mut run, "heights DI DJ DI2 DJ2 DK_LOW DK_HIGH"),
            },
            "fill" => match (num(0), num(1), num(2), num(3), num(4), num(5), args.get(6)) {
                (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f), Some(name)) => match run.session.engine.registry.id(name) {
                    Ok(id) => {
                        let level = match num(7) {
                            Some(l) => Some(l as u8),
                            None if run.session.engine.registry.is_liquid(id) => Some(crate::sim::water::FULL),
                            None => None,
                        };
                        let n = run.session.fill_rel((a as i64, b as i64, c as i64), (d as i64, e as i64, f as i64), id, level);
                        run.say(format!("  fill: {n} cells of {name} changed"));
                    }
                    Err(e) => bad(&mut run, &e),
                },
                _ => bad(&mut run, "fill DI DJ DK DI2 DJ2 DK2 BLOCK [LEVEL]"),
            },
            "turn" => match (num(0), num(1)) {
                (Some(h), Some(p)) => {
                    run.session.player.heading += h.to_radians();
                    run.session.player.pitch = (run.session.player.pitch + p.to_radians()).clamp(-1.55, 1.55);
                }
                _ => bad(&mut run, "turn DH DP"),
            },
            "mode" => match args.first().map(|s| s.to_lowercase()).as_deref() {
                Some("walk") => {
                    let _ = run.session.apply(Action::SetMode(Mode::Walk));
                }
                Some("fly") => {
                    let _ = run.session.apply(Action::SetMode(Mode::Fly));
                }
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
            "tick" => match num(0) {
                Some(n) if n >= 0.0 => run.ticks(n as usize),
                _ => bad(&mut run, "tick N"),
            },
            "sim" => match (args.first().map(|s| s.to_lowercase()).as_deref(), num(1)) {
                (Some(v @ ("on" | "off")), _) => {
                    let _ = run.session.apply(Action::Sim(v == "on"));
                    let d = run.session.sim.describe();
                    run.say(format!("  {d}"));
                }
                (Some("radius"), Some(m)) if m >= 0.0 => {
                    let _ = run.session.apply(Action::SimRadius(m));
                    let d = run.session.sim.describe();
                    run.say(format!("  radius {m} m: {d}"));
                }
                (Some("period"), Some(n)) if n >= 1.0 => {
                    let _ = run.session.apply(Action::WaterPeriod(n as u64));
                    run.say(format!("  water period {} tick(s)", n as u64));
                }
                (Some("slide"), _) if matches!(args.get(1).copied(), Some("on" | "off")) => {
                    let now = run.session.sim.config.flow;
                    let flow = if args[1] == "on" { crate::sim::Flow { slide: true, ..now } } else { crate::sim::Flow { slide: false, film: crate::sim::Film::Keep, ..now } };
                    let _ = run.session.apply(Action::Flow(flow));
                    run.say(format!("  {}", flow.text()));
                }
                (Some("film"), _) if args.get(1).is_some_and(|n| crate::sim::Film::parse(n).is_ok()) => {
                    let flow = crate::sim::Flow { film: crate::sim::Film::parse(args[1]).unwrap(), ..run.session.sim.config.flow };
                    let _ = run.session.apply(Action::Flow(flow));
                    run.say(format!("  {}", flow.text()));
                }
                (Some("reach"), Some(n)) if n >= 1.0 => {
                    let flow = crate::sim::Flow { film_reach: n as u32, ..run.session.sim.config.flow };
                    let _ = run.session.apply(Action::Flow(flow));
                    run.say(format!("  {}", flow.text()));
                }
                (Some("dry"), Some(s)) if s > 0.0 => {
                    let flow = crate::sim::Flow { film_dry_ticks: ((s * crate::sim::TICK_HZ as f64) as u64).max(1), ..run.session.sim.config.flow };
                    let _ = run.session.apply(Action::Flow(flow));
                    run.say(format!("  {}", flow.text()));
                }
                _ => bad(&mut run, "sim on|off | sim radius M | sim period N | sim slide on|off | sim film keep|seek|merge|dry|seek+dry | sim reach N | sim dry SECONDS"),
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
                    let r = run.session.apply(Action::Click(Button::Break)).ok().flatten();
                    run.say(format!("  break: {}", r.map_or("nothing in reach".to_string(), |b| format!("face {} i {} j {} k {}", b.face, b.i, b.j, b.k))));
                }
                Some("place") | Some("rmb") => {
                    let r = run.session.apply(Action::Click(Button::Place)).ok().flatten();
                    run.say(format!("  place: {}", r.map_or("not placed".to_string(), |b| format!("face {} i {} j {} k {}", b.face, b.i, b.j, b.k))));
                }
                Some("jump") | Some("space") => run.steps(Input { jump: true, ..Default::default() }, STEP),
                Some("g") => {
                    let _ = run.session.apply(Action::ToggleMode);
                }
                Some("f") => {
                    let _ = run.session.apply(Action::ToggleWater);
                    run.harness.set_engine(run.session.engine.clone());
                }
                Some(d) if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => {
                    let _ = run.session.apply(Action::Select(d.parse::<usize>().unwrap() - 1));
                }
                _ => bad(&mut run, "tap break|place|jump|g|f|1..9"),
            },
            "select" => match num(0) {
                Some(n) if (1.0..=9.0).contains(&n) => {
                    let _ = run.session.apply(Action::Select(n as usize - 1));
                }
                _ => bad(&mut run, "select 1..9"),
            },
            "hotbar" => match args.first().map(|names| run.session.engine.registry.hotbar_from(names)) {
                Some(Ok(_)) if run.session.recording() => bad(&mut run, "hotbar: not while a replay is being recorded"),
                Some(Ok(ids)) => {
                    run.session.engine = Arc::new(run.session.engine.with_hotbar(ids));
                    run.harness.set_engine(run.session.engine.clone());
                }
                Some(Err(e)) => bad(&mut run, &format!("hotbar: {e}")),
                None => bad(&mut run, "hotbar NAME,NAME,... (1 to 9 block names)"),
            },
            "water" => match args.first().map(|s| s.to_lowercase()).as_deref() {
                Some(v @ ("on" | "off")) => {
                    let _ = run.session.apply(Action::HideWater(v == "off"));
                    run.harness.set_engine(run.session.engine.clone());
                }
                _ => bad(&mut run, "water on|off"),
            },
            "save" => match run.session.save_now() {
                Ok(n) => run.say(format!("  saved {n} chunk(s)")),
                Err(e) => bad(&mut run, &format!("save: {e}")),
            },
            "reload" => match run.session.apply(Action::Reload) {
                Ok(_) => {
                    run.harness.set_engine(run.session.engine.clone());
                    let report = run.session.last_reload.take().expect("a reload report");
                    run.say(format!("  reloaded {} chunk(s) from disk{}", report.chunks, if report.warnings.is_empty() { String::new() } else { format!("; warnings: {}", report.warnings.join("; ")) }));
                }
                Err(e) => bad(&mut run, &format!("reload: {e}")),
            },
            "find" => match args.first() {
                Some(kind) => match find::find(&mut run.session, kind) {
                    Ok(site) => {
                        run.say(format!("  {}", site.describe()));
                        run.last_find = site.fields();
                    }
                    Err(e) => bad(&mut run, &format!("find: {e}")),
                },
                None => bad(&mut run, "find flat|shore|river|lake|waterfall"),
            },
            "replay" => match run.replay_command(args) {
                Ok(()) => {}
                Err(e) => bad(&mut run, &format!("replay: {e}")),
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
            other => match run.video_command(other, args) {
                Some(Ok(())) => {}
                Some(Err(e)) => bad(&mut run, &e),
                None => bad(&mut run, &format!("unknown command {other}")),
            },
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

pub struct ReplayOptions {
    pub out: String,
    /// The scratch save's name (`saves/_replay/<name>`).
    pub name: String,
    /// `--shot-at` values: step numbers, or "end".
    pub shots: Vec<String>,
    /// Play in the window at real time instead.
    pub watch: bool,
    pub width: u32,
    pub height: u32,
    pub backend: Option<String>,
    pub vsync: bool,
    /// With `watch`: save a frame and quit after this many seconds.
    pub screenshot: Option<String>,
    pub exit_after: Option<f64>,
}

/// `--replay FILE` (M2c): play a replay from its snapshot in a scratch save,
/// check it as it goes, take the settled shots asked for, report the first
/// divergence. Exit code 0 when it reproduced, 1 when it diverged, 2 when it
/// could not play.
pub fn run_replay(template: Session, rep: Replay, opts: ReplayOptions) -> i32 {
    let _ = std::fs::create_dir_all(&opts.out);
    let h = &rep.header;
    println!("replay {}: recorded {} UTC by {} ({}); {} steps ({:.1} s), {} events, {} snapshot files ({:.1} KB), {} edited chunks at the start", rep.path.display(), h.created_utc, h.engine, h.note, rep.steps(), rep.steps() as f64 / 60.0, rep.lines.len(), h.files.len(), h.files.iter().map(|f| f.bytes).sum::<u64>() as f64 / 1e3, h.start_edits);
    let scratch = replay::scratch_dir(&opts.name);
    let session = match replay::session_from(&template, &rep, &scratch) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    drop(template);
    println!("playing in the scratch save {} (your own save is not touched)", scratch.display());
    let mut shots = std::collections::BTreeSet::new();
    let mut shot_end = false;
    for s in &opts.shots {
        match s.as_str() {
            "end" => shot_end = true,
            n => match n.parse::<u64>() {
                Ok(n) => {
                    shots.insert(n);
                }
                Err(_) => {
                    eprintln!("error: --shot-at takes a step number or \"end\", not {n:?}");
                    return 2;
                }
            },
        }
    }
    if opts.watch {
        return crate::app::run_replay(session, rep, crate::app::AppOptions { width: opts.width, height: opts.height, vsync: opts.vsync, backend: opts.backend, screenshot: opts.screenshot, exit_after: opts.exit_after, record: None });
    }
    let t0 = Instant::now();
    let mut log = String::new();
    let report = if shots.is_empty() && !shot_end {
        let mut s = session;
        replay::Runner::new(&rep, &s).run(&mut s)
    } else {
        let engine = session.engine.clone();
        let harness = Harness::new(engine.clone(), opts.width, opts.height, &opts.out, opts.backend.as_deref());
        let mut run = Run::new(session, harness, &opts.out);
        let mut seen = engine;
        let mut runner = replay::Runner::new(&rep, &run.session);
        loop {
            if !Arc::ptr_eq(&seen, &run.session.engine) {
                // A reload or a water toggle made a new engine: draw from it.
                seen = run.session.engine.clone();
                run.harness.set_engine(seen.clone());
            }
            if shots.remove(&runner.step()) {
                run.shot(&format!("step{:06}", runner.step()));
            }
            if !runner.advance(&mut run.session) {
                break;
            }
        }
        if !Arc::ptr_eq(&seen, &run.session.engine) {
            run.harness.set_engine(run.session.engine.clone());
        }
        if shot_end {
            run.shot(&format!("end_step{:06}", runner.step()));
        }
        for s in &shots {
            run.say(format!("  no shot at step {s}: the replay ends at step {}", runner.step()));
        }
        log = std::mem::take(&mut run.log);
        runner.report
    };
    let text = report.text();
    println!("{text}");
    println!("(played in {:.1} s)", t0.elapsed().as_secs_f64());
    log.push_str(&text);
    log.push('\n');
    let json = serde_json::json!({
        "replay": rep.path.display().to_string(),
        "ok": report.ok(),
        "steps": report.steps,
        "checks": report.checks,
        "matched": report.matched,
        "ended": report.ended,
        "end_sha": report.end_sha,
        "divergence": report.first.as_ref().map(|d| serde_json::json!({ "step": d.step, "event_line": d.line_no, "what": d.what })),
        "notes": report.notes,
        "warnings": report.warnings,
        "errors": report.errors,
    });
    let _ = std::fs::write(format!("{}/report.json", opts.out), serde_json::to_string_pretty(&json).unwrap());
    let _ = std::fs::write(format!("{}/run.log", opts.out), log);
    println!("report: {}/report.json", opts.out);
    if report.ok() { 0 } else { 1 }
}

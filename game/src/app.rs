//! The windowed app: the player (walk or fly), streaming, rendering, the
//! egui HUD, block breaking and placing, photos, the T window, and autosave.
//!
//! F9 (B6): the next frame is drawn clean and read back as a `Moment`, the mouse
//! comes free, and the annotator takes the screen. Steps stop (`Gate`), the
//! world isn't drawn, so a pen stroke costs one egui pass and nothing else, and
//! closing it puts everything back. Nothing it does reaches the session except
//! the marker, written when it's saved, at the step it was grabbed.

use crate::annotate::Gate;
use crate::camera::Camera;
use crate::controls::{Controls, Route, route_key};
use crate::engine::{Engine, Preset};
use crate::grid::Planet;
use crate::menu::{self, Go, Menu, MenuEnv};
use crate::photos::{self, Shot, ShotInfo, ShotWriter};
use crate::player::Mode;
use crate::render::{DrawRecord, Renderer};
use crate::replay::ChunkDigests;
use crate::report::{Grab, Moment, Ticker};
use crate::session::Session;
use crate::sim::draw::FallingDraws;
use crate::stream::Streamer;
use crate::ui::{EguiPaint, Hud, draw_hud, outline_segments};
use crate::visual::{self, Verdict, Visual};
use crate::workshop::Workshop;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

pub struct AppOptions {
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
    pub backend: Option<String>,
    /// Save the frame to this PNG and quit after `exit_after` seconds (for headless checks).
    pub screenshot: Option<String>,
    pub exit_after: Option<f64>,
    /// Start recording a replay to this file at launch, as F10 does (M2c;
    /// with `exit_after` it is stopped and written at the exit).
    pub record: Option<String>,
}

/// A settled photo wants the world finished for this many frames in a row
/// (the harness's rule), and stops waiting after this long.
const SETTLE_FRAMES: u32 = 10;
const SETTLE_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the line about a saved photo stays up.
const BANNER_LIFE: Duration = Duration::from_secs(4);
/// While recording, how often to see whether the look or a texture changed.
const VISUAL_EVERY: Duration = Duration::from_millis(500);

/// P was pressed: the world holds still, and streaming runs, until this view
/// has finished loading. No steps pass, so neither the simulation nor a replay
/// being recorded can tell; the event loop keeps running, so nothing hangs.
struct PendingPhoto {
    cam: Camera,
    started: Instant,
    frames: u32,
    stable: u32,
}

/// How a frame came to be a photo.
struct Taken {
    kind: &'static str,
    settled: bool,
    settle_s: f64,
    settle_frames: u32,
    hud: bool,
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Renderer,
    egui_state: egui_winit::State,
    egui_paint: EguiPaint,
}

struct App {
    session: Session,
    opts: Option<AppOptions>,
    gfx: Option<Gfx>,
    streamer: Option<Streamer>,
    egui_ctx: egui::Context,
    /// Window events in, step-positioned game inputs out (M2c).
    controls: Controls,
    last: Instant,
    /// The sky speed the window title shows.
    title_speed: f64,
    /// A Shift key is down (Shift+M, Shift+P).
    shift: bool,
    /// The T window: map, photos, teleport and places.
    menu: Menu,
    /// P, waiting for the world to finish loading.
    photo: Option<PendingPhoto>,
    /// Shift+P: the next frame, as drawn.
    live_shot: bool,
    /// F9 was pressed: grab the next frame and open the annotator.
    mark_wanted: bool,
    /// The game's side of the annotator (stopped steps, swallowed input), and
    /// whether the mouse goes back to the game afterwards.
    gate: Gate,
    mark_recapture: bool,
    visual_checked: Instant,
    writer: ShotWriter,
    digests: ChunkDigests,
    banner: Option<(Instant, String)>,
    opaque: Vec<DrawRecord>,
    water: Vec<DrawRecord>,
    captured: bool,
    vsync: bool,
    show_stats: bool,
    presets: Vec<Preset>,
    /// (time, frame ms) over the last five seconds.
    frames: VecDeque<(Instant, f64)>,
    cpu_ms: f64,
    last_tris: (u64, u64, usize),
    started: Instant,
    screenshot: Option<String>,
    exit_after: Option<f64>,
    want_exit: bool,
    save_note: String,
    falling_draw: FallingDraws,
    /// Andrew's look panel (F1) and the look files it saves and watches (M3a).
    workshop: Workshop,
    /// `--replay FILE --watch` (M2c): the replay drives the steps instead
    /// of the keyboard and mouse.
    replay: Option<WatchedReplay>,
}

struct WatchedReplay {
    runner: crate::replay::Runner<'static>,
    acc: f64,
    reported: bool,
    ticker: Ticker,
    /// The look, against the recording's.
    visual: Verdict,
}

impl App {
    fn engine(&self) -> &Arc<Engine> {
        &self.session.engine
    }

    fn configure_surface(&mut self) {
        let Some(gfx) = &self.gfx else { return };
        let size = gfx.window.inner_size();
        let mut config = gfx.config.clone();
        config.width = size.width.max(1);
        config.height = size.height.max(1);
        config.present_mode = if self.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync };
        gfx.surface.configure(&gfx.gpu.device, &config);
        let gfx = self.gfx.as_mut().unwrap();
        gfx.config = config;
        gfx.gpu.resize(size.width, size.height);
    }

    fn set_capture(&mut self, on: bool) {
        let Some(gfx) = &self.gfx else { return };
        if on {
            let ok = gfx.window.set_cursor_grab(CursorGrabMode::Confined).or_else(|_| gfx.window.set_cursor_grab(CursorGrabMode::Locked)).is_ok();
            gfx.window.set_cursor_visible(!ok);
            self.captured = ok;
        } else {
            let _ = gfx.window.set_cursor_grab(CursorGrabMode::None);
            gfx.window.set_cursor_visible(true);
            self.captured = false;
        }
    }

    /// F1: open the look panel (freeing the mouse) or close it (giving the
    /// mouse back to the game if it had it).
    fn toggle_panel(&mut self) {
        self.workshop.open = !self.workshop.open;
        if self.workshop.open {
            self.workshop.recapture = self.captured;
            // A held mouse button lets go: the panel has the mouse now.
            self.controls.button(winit::event::MouseButton::Left, false);
            self.set_capture(false);
        } else if self.workshop.recapture {
            self.set_capture(true);
        }
    }

    /// T: open the window. It owns all input until it shuts (§17.1): held
    /// keys and the mouse button let go, and the mouse comes free.
    fn open_menu(&mut self) {
        crate::ui::apply_panel_style(&self.egui_ctx);
        self.menu.recapture = self.captured;
        self.controls.release_all();
        self.set_capture(false);
        self.menu.show();
    }

    fn close_menu(&mut self) {
        self.menu.hide();
        if self.menu.recapture {
            self.set_capture(true);
        }
    }

    /// The window sent the player somewhere: recorded inputs at the next step.
    fn arrive(&mut self, go: Go) {
        let Some(gfx) = &self.gfx else { return };
        let actions = menu::actions(&go, &self.session, &gfx.gpu.sky.settings);
        if let Go::Place(i) = go {
            self.controls.set_place(i);
        }
        for a in actions {
            self.controls.queue(a);
        }
        if self.menu.recapture {
            self.set_capture(true);
        }
    }

    /// P: a photo of the settled world. Shift+P: this frame, as it stands.
    fn take_photo(&mut self) {
        if self.shift {
            self.live_shot = true;
        } else if self.photo.is_none() {
            let cam = self.controls.camera(&self.session.player);
            self.photo = Some(PendingPhoto { cam, started: Instant::now(), frames: 0, stable: 0 });
        }
    }

    /// Write a frame as a photo, off this thread. `path`: where, or a new name
    /// in the photo folder.
    fn store_photo(&mut self, path: Option<PathBuf>, rgba: Vec<u8>, width: u32, height: u32, cam: &Camera, how: Taken) {
        let Some(gfx) = &self.gfx else { return };
        let (lat, lon) = Planet::lat_lon_deg(cam.pos);
        let alt = cam.pos.length() - self.session.engine.planet.radius;
        let png = path.unwrap_or_else(|| photos::free_path(&photos::dir(), &photos::shot_stem(lat, lon, alt, cam.heading.to_degrees(), cam.pitch.to_degrees(), how.kind != "settled")));
        let frame_ms = self.frame_stats();
        let info = ShotInfo { kind: how.kind, settled: how.settled, settle_s: how.settle_s, settle_frames: how.settle_frames, width, height, hud: how.hud, mesh_view: self.workshop.mesh_view, preview_world: self.workshop.previewing(), frame_ms };
        let sidecar = photos::sidecar(&self.session, cam, &gfx.gpu.sky.settings, self.workshop.look(), &mut self.digests, &png, &info);
        self.writer.write(Shot { png, width, height, rgba, sidecar });
    }

    /// (average, 95th percentile) frame time over the last few seconds.
    fn frame_stats(&self) -> Option<(f64, f64)> {
        let mut ms: Vec<f64> = self.frames.iter().map(|f| f.1).collect();
        ms.sort_by(|a, b| a.total_cmp(b));
        (!ms.is_empty()).then(|| (ms.iter().sum::<f64>() / ms.len() as f64, ms[((ms.len() - 1) as f64 * 0.95).round() as usize]))
    }

    /// What the picture is made of now, for the recording (`visual.rs`): a new
    /// value only when something changed, and a recording writes that down.
    fn refresh_visual(&mut self, now: bool) {
        if !now && self.visual_checked.elapsed() < VISUAL_EVERY {
            return;
        }
        self.visual_checked = Instant::now();
        let e = &self.session.engine;
        let v = Visual::of(self.workshop.look(), &e.registry, visual::texture_ids(self.workshop.texture_set()), &self.workshop.mesh_view.name(), e.mesh_opts.ao, e.mesh_opts.split, e.skirt);
        if self.session.visual.as_deref() != Some(&v) {
            self.session.visual = Some(Arc::new(v));
        }
    }

    /// F9, a frame later: the moment, grabbed, and the annotator open on it.
    fn open_mark(&mut self, rgba: Vec<u8>, width: u32, height: u32, cam: &Camera) {
        self.mark_wanted = false;
        self.refresh_visual(true);
        let frame_ms = self.frame_stats();
        let Some(gfx) = &self.gfx else { return };
        let moment = Moment::grab(Grab { session: &self.session, cam, settings: &gfx.gpu.sky.settings, look: self.workshop.look(), digests: &mut self.digests, rgba, width, height, mesh_view: self.workshop.mesh_view, preview_world: self.workshop.previewing(), frame_ms });
        self.gate.open();
        self.mark_recapture = self.captured;
        self.set_capture(false);
        crate::ui::apply_panel_style(&self.egui_ctx);
        self.menu.mark_moment(moment);
    }

    /// The annotator closed: back to the game, with the marker if it was saved.
    fn finish_mark(&mut self, f: menu::Finished) {
        self.gate.close();
        if self.mark_recapture {
            self.set_capture(true);
        }
        let now = Instant::now();
        let Some((marks, comment)) = f.saved else {
            self.banner = Some((now, "Nothing saved".to_string()));
            return;
        };
        let text = match &f.moment.recording {
            Some((_, n)) => format!("Marker {n} saved, step {}", f.moment.step),
            None => "Marked photo saved".to_string(),
        };
        let (shot, marks) = f.moment.commit(&mut self.session, marks, comment);
        self.writer.write_marked(shot, marks);
        self.banner = Some((now, text));
    }

    /// How many frames may queue up for the screen. One with the annotator
    /// open: the pen is under the player's eye, next to the mouse pointer.
    fn set_frame_latency(&mut self, frames: u32) {
        let Some(gfx) = &mut self.gfx else { return };
        if gfx.config.desired_maximum_frame_latency != frames {
            gfx.config.desired_maximum_frame_latency = frames;
            self.configure_surface();
        }
    }

    /// A frame with the annotator open: only that. No world is drawn behind a
    /// full-screen panel, so a stroke costs one egui pass.
    fn annotate_frame(&mut self) {
        self.set_frame_latency(1);
        let engine = self.session.engine.clone();
        let here = Planet::lat_lon_deg(self.session.player.pos);
        let recording = self.session.recorder.as_ref().map(|r| r.path().to_path_buf());
        let Some(gfx) = &mut self.gfx else { return };
        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                self.configure_surface();
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        gfx.gpu.clear(&view);
        let raw = gfx.egui_state.take_egui_input(&gfx.window);
        let env = MenuEnv { engine: &engine, bake_sha256: &self.session.bake_sha256, presets: &self.presets, here, can_move: self.replay.is_none(), day_length_s: gfx.gpu.sky.settings.day_length_s, recording: recording.as_deref() };
        let menu = &mut self.menu;
        let full = self.egui_ctx.run_ui(raw, |ui| {
            menu.ui(ui.ctx(), &env);
        });
        gfx.egui_state.handle_platform_output(&gfx.window, full.platform_output);
        let prims = self.egui_ctx.tessellate(full.shapes, full.pixels_per_point);
        gfx.egui_paint.paint(&gfx.gpu.device, &gfx.gpu.queue, &view, (gfx.config.width, gfx.config.height), full.pixels_per_point, &prims, &full.textures_delta);
        gfx.window.pre_present_notify();
        gfx.gpu.queue.present(frame);
    }

    /// What the line on screen says now.
    fn banner_text(&mut self) -> Option<String> {
        if self.photo.is_some() {
            let left = self.streamer.as_ref().map_or(0, |s| s.stats.pending + s.stats.inflight);
            return Some(if left > 0 { format!("Taking a photo: {left} chunks to load") } else { "Taking a photo".to_string() });
        }
        self.banner.take_if(|(t, _)| t.elapsed() >= BANNER_LIFE);
        self.banner.as_ref().map(|(_, s)| s.clone())
    }

    /// F10: start or stop recording a replay (M2c). The file goes to
    /// `game/replays/`; the console, the window title and the stats panel
    /// say where.
    fn toggle_recording(&mut self) {
        self.toggle_recording_to(None);
    }

    /// `path`: where a new recording goes (None: `game/replays/<world>-<time>.replay`).
    fn toggle_recording_to(&mut self, path: Option<std::path::PathBuf>) {
        if self.session.recording() {
            match self.session.stop_recording() {
                Ok(s) => {
                    self.save_note = format!("replay saved: {} ({:.1} s, {:.1} MB)", s.path.display(), s.steps as f64 / 60.0, s.bytes as f64 / 1e6);
                    println!("replay saved to {} ({} steps = {:.1} s, {} checks, {:.1} MB; play it with --replay)", s.path.display(), s.steps, s.steps as f64 / 60.0, s.checks, s.bytes as f64 / 1e6);
                }
                Err(e) => {
                    self.save_note = format!("REPLAY FAILED: {e}");
                    eprintln!("replay failed: {e}");
                }
            }
        } else {
            let path = path.unwrap_or_else(|| {
                let name = crate::replay::default_path(&self.session.engine.config.name);
                photos::replays_dir().join(name.file_name().unwrap_or_default())
            });
            self.refresh_visual(true);
            match self.session.start_recording(&path, "recorded in the app (F10)") {
                Ok(p) => {
                    self.save_note = format!("RECORDING replay (F9 marks a moment, F10 stops): {}", p.display());
                    println!("recording a replay to {} (F9 marks a moment, F10 stops)", p.display());
                }
                Err(e) => {
                    self.save_note = format!("REPLAY FAILED: {e}");
                    eprintln!("replay failed: {e}");
                }
            }
        }
        self.update_title();
    }

    fn update_title(&mut self) {
        let Some(gfx) = &self.gfx else { return };
        let speed = self.session.sky.speed;
        self.title_speed = speed;
        let rec = if self.session.recording() { " | REC replay (F9 marks, F10 stops)" } else { "" };
        gfx.window.set_title(&format!("Triangulum | time {speed}x ([ slower, ] faster){rec}"));
    }

    fn save_on_exit(&mut self) {
        // The window closed on an open annotator: what's there is kept.
        if let Some(f) = self.menu.finish_now() {
            self.finish_mark(f);
        }
        if self.session.recording() {
            self.toggle_recording();
        }
        match self.session.save_now() {
            Ok(n) if n > 0 => println!("saved {n} chunk(s) to {}", self.session.save.as_ref().map_or("-".into(), |s| s.path.display().to_string())),
            Ok(_) => {}
            Err(e) => eprintln!("save failed: {e}"),
        }
        self.writer.finish();
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.25);
        self.last = now;
        // Fixed 60 Hz steps (the same steps the harness and a replay take):
        // the queued inputs go in at the next step, in order (M2c). None while
        // a photo waits for the world to settle, or a moment is being marked.
        if self.photo.is_some() || self.mark_wanted {
        } else if self.gate.is_open() {
            self.gate.frame(&mut self.controls, &mut self.session, dt);
        } else if let Some(w) = &mut self.replay {
            // A watched replay: its own inputs, at real time.
            w.acc += dt;
            let mut n = 0;
            while w.acc >= crate::player::STEP && n < crate::controls::MAX_STEPS_PER_FRAME && w.runner.advance(&mut self.session) {
                w.acc -= crate::player::STEP;
                n += 1;
            }
            if n == crate::controls::MAX_STEPS_PER_FRAME {
                w.acc = 0.0;
            }
            if w.runner.done() && !w.reported {
                w.reported = true;
                let r = &w.runner.report;
                println!("{}", r.text());
                println!("{}", w.visual.text());
                let look = format!("look {}", w.visual.reproduced.name());
                self.save_note = if r.ok() { format!("replay finished at step {}: simulation reproduced exactly; {look}", r.steps) } else { format!("replay DIVERGED at step {} (see the console)", r.first.as_ref().map_or(0, |d| d.step)) };
            }
        } else {
            self.gate.frame(&mut self.controls, &mut self.session, dt);
        }
        if self.session.recording() {
            self.refresh_visual(false);
        }
        if let Some(gfx) = &mut self.gfx {
            gfx.gpu.sky.clock = self.session.sky;
        }
        if self.session.sky.speed != self.title_speed {
            println!("time speed: {}x", self.session.sky.speed);
            self.update_title();
        }
        if let Some(e) = self.session.recorder.as_ref().and_then(|r| r.error.clone()) {
            self.save_note = format!("REPLAY FAILED: {e}");
        }
        // Autosave (debounced).
        match self.session.save_if_due(now) {
            Ok(n) if n > 0 => self.save_note = format!("saved {n} chunk(s)"),
            Ok(_) => {}
            Err(e) => {
                self.save_note = format!("SAVE FAILED: {e}");
                eprintln!("save failed: {e}");
            }
        }
        if let Some(done) = self.writer.poll() {
            let text = match done {
                Ok(p) => format!("Photo saved: {}", p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())),
                Err(e) => format!("PHOTO FAILED: {e}"),
            };
            println!("{text}");
            self.workshop.say(text.clone());
            self.banner = Some((now, text));
        }
        if self.menu.annotating() {
            self.annotate_frame();
            if let Some(f) = self.menu.take_finished() {
                self.finish_mark(f);
            }
            return;
        }
        self.set_frame_latency(2);
        let cam = self.photo.as_ref().map_or_else(|| self.controls.camera(&self.session.player), |p| p.cam);

        let (Some(gfx), Some(streamer)) = (&mut self.gfx, &mut self.streamer) else { return };
        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                self.configure_surface();
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        // The look panel's changes and the look files (M3a): render dials,
        // block colours, distance detail, terrain previews.
        self.workshop.update(&mut self.session, streamer, &mut self.falling_draw, &mut gfx.gpu);
        let t0 = Instant::now();
        let aspect = gfx.gpu.width as f64 / gfx.gpu.height as f64;
        let vp = cam.view_proj(aspect);
        streamer.update(cam.pos, vp, &mut gfx.gpu, &mut self.opaque, &mut self.water);
        self.falling_draw.emit(&mut gfx.gpu, &self.session.engine.planet, &self.session.sim.view(), cam.pos, &mut self.opaque);
        let engine = self.session.engine.clone();
        let uni = cam.uniform(aspect, engine.sun_at(cam.pos), &engine.planet, engine.water_alpha);
        // Water is hidden here, at draw time, never in generation. The eye
        // is under water when it is below the level of the cell it is in (M2b).
        gfx.gpu.sky.set_observer(cam.pos, !engine.hide_water && engine.world.is_submerged(cam.pos));
        let water: &[DrawRecord] = if engine.hide_water { &[] } else { &self.water };
        let fs = gfx.gpu.render(&uni, &self.opaque, water, Some(&view));
        self.cpu_ms = t0.elapsed().as_secs_f64() * 1e3;
        self.last_tris = (fs.triangles, fs.water_triangles, (fs.draws + fs.water_draws) as usize);
        let gpu_ms = gfx.gpu.poll_gpu_ms();
        // P: when this view has finished loading, the frame again, clean,
        // into the offscreen target (the player keeps seeing the HUD).
        let mut clean = None;
        if let Some(p) = &mut self.photo {
            p.frames += 1;
            p.stable = if streamer.settled() { p.stable + 1 } else { 0 };
            let (settled, waited) = (p.stable >= SETTLE_FRAMES, p.started.elapsed());
            if settled || waited >= SETTLE_TIMEOUT {
                gfx.gpu.sky.set_observer(cam.pos, !engine.hide_water && engine.world.is_submerged(cam.pos));
                gfx.gpu.render(&uni, &self.opaque, water, None);
                clean = Some((gfx.gpu.read_rgba(), Taken { kind: "settled", settled, settle_s: waited.as_secs_f64(), settle_frames: p.frames, hud: false }));
            }
        }
        // F9: this frame again, clean, as it stands.
        let mut marked = None;
        if self.mark_wanted {
            gfx.gpu.sky.set_observer(cam.pos, !engine.hide_water && engine.world.is_submerged(cam.pos));
            gfx.gpu.render(&uni, &self.opaque, water, None);
            marked = Some(gfx.gpu.read_rgba());
        }

        // HUD (crosshair, outline and hotbar always; stats on H).
        let (w, h) = (gfx.config.width, gfx.config.height);
        let raw = gfx.egui_state.take_egui_input(&gfx.window);
        let day_length_s = gfx.gpu.sky.settings.day_length_s;
        let mut go = None;
        {
            let mut hud = self.hud_info(gpu_ms, &cam, aspect, w, h);
            hud.hotbar_tex = self.workshop.hotbar_thumbs(&self.egui_ctx, &self.session.engine.hotbar);
            hud.banner = self.banner_text();
            hud.marker = self.replay.as_ref().and_then(|r| r.ticker.line(r.runner.step()));
            let stats = self.show_stats;
            let live = self.live_shot;
            let recording = self.session.recorder.as_ref().map(|r| r.path().to_path_buf());
            let (workshop, menu) = (&mut self.workshop, &mut self.menu);
            let env = MenuEnv { engine: &engine, bake_sha256: &self.session.bake_sha256, presets: &self.presets, here: Planet::lat_lon_deg(cam.pos), can_move: self.replay.is_none(), day_length_s, recording: recording.as_deref() };
            let full = self.egui_ctx.run_ui(raw, |ui| {
                draw_hud(ui.ctx(), &hud, stats);
                // The T window owns the screen: the panel waits behind it.
                if !menu.open && !live {
                    workshop.ui(ui.ctx());
                }
                if let Some(g) = menu.ui(ui.ctx(), &env) {
                    go = Some(g);
                }
            });
            let gfx = self.gfx.as_mut().unwrap();
            gfx.egui_state.handle_platform_output(&gfx.window, full.platform_output);
            let prims = self.egui_ctx.tessellate(full.shapes, full.pixels_per_point);
            gfx.egui_paint.paint(&gfx.gpu.device, &gfx.gpu.queue, &view, (gfx.config.width, gfx.config.height), full.pixels_per_point, &prims, &full.textures_delta);
        }
        if let Some(go) = go {
            self.arrive(go);
        }
        if let Some((rgba, how)) = clean {
            self.photo = None;
            self.store_photo(None, rgba, w, h, &cam, how);
        }
        if let Some(rgba) = marked {
            self.open_mark(rgba, w, h, &cam);
        }
        // Shift+P, the look panel's button and --screenshot: the frame as
        // drawn, HUD and all, windows aside.
        let quit_shot = self.exit_after.is_some_and(|s| self.started.elapsed().as_secs_f64() >= s);
        let path = if quit_shot { self.screenshot.clone().map(PathBuf::from) } else { None };
        let live = std::mem::take(&mut self.live_shot);
        let panel = self.workshop.shot_pending();
        if path.is_some() || live || panel {
            let rgba = self.gfx.as_ref().unwrap().gpu.read_texture(&frame.texture, w, h);
            let how = Taken { kind: if panel { "panel" } else { "live" }, settled: false, settle_s: 0.0, settle_frames: 0, hud: true };
            self.store_photo(path, rgba, w, h, &cam, how);
        }
        if quit_shot {
            self.want_exit = true;
        }
        let gfx = self.gfx.as_mut().unwrap();
        gfx.window.pre_present_notify();
        gfx.gpu.queue.present(frame);
        let frame_ms = now.elapsed().as_secs_f64() * 1e3;
        self.frames.push_back((now, frame_ms));
        while self.frames.front().is_some_and(|(t, _)| now.duration_since(*t).as_secs_f64() > 5.0) {
            self.frames.pop_front();
        }
    }

    /// The legend of the mesh view, when it is on.
    fn mesh_legend(&self, drawn: &[u32; 16]) -> Option<crate::render::meshview::Legend> {
        crate::render::meshview::legend_for(self.workshop.mesh_view, drawn, &self.engine().registry, &self.workshop.regions_state)
    }

    fn hud_info(&self, gpu_ms: f64, cam: &Camera, aspect: f64, width: u32, height: u32) -> Hud {
        let now = Instant::now();
        let last_1s: Vec<f64> = self.frames.iter().filter(|(t, _)| now.duration_since(*t).as_secs_f64() <= 1.0).map(|(_, ms)| *ms).collect();
        let fps = if last_1s.is_empty() { 0.0 } else { 1000.0 / (last_1s.iter().sum::<f64>() / last_1s.len() as f64) };
        let mut all: Vec<f64> = self.frames.iter().map(|(_, ms)| *ms).collect();
        all.sort_by(|a, b| b.total_cmp(a));
        let n1 = (all.len() / 100).max(1).min(all.len().max(1));
        let low1 = if all.is_empty() { 0.0 } else { 1000.0 / (all[..n1].iter().sum::<f64>() / n1 as f64) };
        let streamer = self.streamer.as_ref().unwrap();
        let st = &streamer.stats;
        let engine = self.engine();
        let p = &self.session.player;
        let (lat, lon, alt) = cam.lat_lon_alt(&engine.planet);
        let ground = engine.ground(p.pos.normalize());
        let pick = self.session.pick();
        let outline = pick.map_or(Vec::new(), |h| outline_segments(&engine.planet, cam, aspect, width, height, h.block));
        Hud {
            fps,
            low1_fps: low1,
            gpu_ms,
            cpu_ms: self.cpu_ms,
            frame_ms: self.frames.back().map_or(0.0, |f| f.1),
            drawn_per_lod: st.drawn_per_lod,
            loaded_per_lod: st.loaded_per_lod,
            lat,
            lon,
            alt,
            alt_agl: alt - ground,
            block: pick.map(|h| (h.block, engine.registry.def(h.id).name.clone(), h.t)),
            speed: p.fly_speed,
            vsync: self.vsync,
            loaded: st.loaded,
            inflight: st.inflight,
            pending: st.pending,
            tris: self.last_tris.0,
            water_tris: self.last_tris.1,
            draws: self.last_tris.2,
            vram_mb: self.gfx.as_ref().map_or(0.0, |g| g.gpu.resident_bytes() as f64 / 1e6),
            preset: self.presets.get(self.controls.place()).map_or("-".into(), |p| p.name.clone()),
            hide_water: engine.hide_water,
            captured: self.captured,
            mode: if p.mode == Mode::Walk { "walk".into() } else { "fly".into() },
            on_ground: p.on_ground,
            in_water: p.in_water,
            hotbar: engine.hotbar.iter().map(|&id| (engine.registry.def(id).label(), engine.registry.colour(id))).collect(),
            hotbar_tex: Vec::new(),
            selected: p.selected,
            outline,
            edits: engine.edits.len(),
            dirty: engine.edits.dirty_count(),
            save_note: self.save_note.clone(),
            sim_line: self.session.sim.describe(),
            mesh_legend: self.mesh_legend(&st.drawn_per_lod),
            banner: None,
            marker: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let opts = self.opts.take().unwrap();
        let window = Arc::new(
            el.create_window(Window::default_attributes().with_title("Triangulum").with_inner_size(winit::dpi::PhysicalSize::new(opts.width, opts.height)))
                .unwrap(),
        );
        let instance = crate::render::make_instance(opts.backend.as_deref());
        let surface = instance.create_surface(window.clone()).unwrap();
        let engine = self.engine().clone();
        let gpu = Renderer::new(&instance, Some(&surface), opts.width, opts.height, engine.quad_mb, engine.slot_capacity, &engine.colours);
        let caps = surface.get_capabilities(
            &pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            }))
            .unwrap(),
        );
        assert!(caps.formats.contains(&gpu.format), "surface does not support {:?}", gpu.format);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: gpu.format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: if opts.vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync },
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&gpu.device, &config);
        let egui_state = egui_winit::State::new(self.egui_ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, None);
        let egui_paint = EguiPaint::new(&gpu.device, gpu.format);
        self.streamer = Some(engine.streamer());
        self.vsync = opts.vsync;
        self.gfx = Some(Gfx { window, surface, config, gpu, egui_state, egui_paint });
        el.set_control_flow(ControlFlow::Poll);
        if let Some(p) = opts.record {
            self.toggle_recording_to(Some(p.into()));
        }
        println!("app: WASD move, Space jump/up, Ctrl sneak (fly: down), Shift sprint, mouse look (click to capture, Esc releases), LMB break, RMB place, 1-9 block, G walk/fly, scroll fly speed, P photo, Shift+P live photo, T map photos teleport, R to ground, V vsync, F water, M mesh colours, Shift+M mesh lines, H hud, [ slower time, ] faster time, F9 mark what's wrong, F10 record a replay, Q quit");
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(gfx) = self.gfx.as_mut() {
            let _ = gfx.egui_state.on_window_event(&gfx.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                self.save_on_exit();
                el.exit()
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    // Game keys queue their input for the next step (controls.rs);
                    // the app's own keys act here.
                    let pressed = event.state == ElementState::Pressed;
                    // F9's annotator owns the keys: egui has them already, and
                    // the game hears only that a held key was let go.
                    if self.gate.is_open() {
                        self.gate.key(&mut self.controls, code, pressed);
                        return;
                    }
                    // An open T window owns all input (§17.1).
                    let typing = self.egui_ctx.egui_wants_keyboard_input();
                    match route_key(code, pressed, event.repeat, self.menu.open, typing) {
                        Route::Window => return,
                        Route::OpenMenu => return self.open_menu(),
                        Route::CloseMenu if self.menu.annotating() => return,
                        Route::CloseMenu => return self.close_menu(),
                        Route::MenuBack => {
                            if !self.menu.escape(typing) {
                                self.close_menu();
                            }
                            return;
                        }
                        Route::Game => {}
                    }
                    // The look panel (M3a): F1 opens and closes it; keys
                    // typed into one of its fields are not game input.
                    if pressed && code == KeyCode::F1 && !event.repeat {
                        self.toggle_panel();
                    }
                    if pressed && self.workshop.typing(&self.egui_ctx) {
                        return;
                    }
                    // Watching a replay, only the app's own keys work.
                    let first = if self.replay.is_some() { pressed && !event.repeat } else { self.controls.key(code, pressed) };
                    if first {
                        match code {
                            // One layer at a time: the panel, then the mouse.
                            KeyCode::Escape if self.workshop.open && !egui::Popup::is_any_open(&self.egui_ctx) => self.toggle_panel(),
                            KeyCode::Escape => self.set_capture(false),
                            KeyCode::KeyP => self.take_photo(),
                            KeyCode::KeyQ => {
                                self.save_on_exit();
                                el.exit()
                            }
                            KeyCode::KeyV => {
                                self.vsync = !self.vsync;
                                self.configure_surface();
                            }
                            KeyCode::KeyH => self.show_stats = !self.show_stats,
                            KeyCode::KeyM => {
                                let v = &mut self.workshop.mesh_view;
                                if self.shift {
                                    v.lines = v.lines.next();
                                } else {
                                    v.tint = v.tint.next();
                                }
                                self.save_note = format!("mesh view: {}", v.label());
                            }
                            KeyCode::F9 => self.mark_wanted = true,
                            KeyCode::F10 if self.replay.is_none() => self.toggle_recording(),
                            _ => {}
                        }
                    }
                }
            }
            // The annotator has the mouse, but a button held at F9 lets go.
            WindowEvent::MouseInput { state, button, .. } if self.gate.is_open() => self.gate.button(&mut self.controls, button, state == ElementState::Pressed),
            WindowEvent::MouseWheel { .. } if self.gate.is_open() => {}
            // The T window has the mouse; so does a watched replay's window.
            WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } if self.replay.is_some() || self.menu.open => {}
            // While the look panel is open the mouse is its (M3a).
            WindowEvent::MouseInput { state: ElementState::Pressed, .. } if self.workshop.open => {}
            WindowEvent::MouseWheel { .. } if self.workshop.open && self.egui_ctx.is_pointer_over_egui() => {}
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Pressed && !self.captured {
                    // The capturing click is not game input.
                    self.set_capture(true);
                } else if self.captured || state == ElementState::Released {
                    self.gate.button(&mut self.controls, button, state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.gate.scroll(&mut self.controls, lines);
            }
            WindowEvent::ModifiersChanged(m) => self.shift = m.state().shift_key(),
            WindowEvent::Focused(false) => {
                self.shift = false;
                self.controls.focus_lost();
            }
            WindowEvent::Resized(_) => self.configure_surface(),
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.captured && self.replay.is_none() {
                self.gate.motion(&mut self.controls, delta.0, delta.1);
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.want_exit {
            self.save_on_exit();
            el.exit();
            return;
        }
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }
}

pub fn run(session: Session, opts: AppOptions) {
    run_with(session, opts, None);
}

/// `--replay FILE --watch` (M2c): play a replay in the window at real
/// time, checking it as it goes. Returns the exit code (0: reproduced).
pub fn run_replay(session: Session, rep: crate::replay::Replay, opts: AppOptions, visual: Verdict) -> i32 {
    // The window owns the runner until the process ends.
    let rep: &'static crate::replay::Replay = Box::leak(Box::new(rep));
    let runner = crate::replay::Runner::new(rep, &session);
    let ok = run_with(session, opts, Some(WatchedReplay { runner, acc: 0.0, reported: false, ticker: Ticker::new(rep.markers()), visual }));
    if ok { 0 } else { 1 }
}

fn run_with(session: Session, opts: AppOptions, replay: Option<WatchedReplay>) -> bool {
    let el = EventLoop::new().unwrap();
    let presets = session.engine.presets();
    let (screenshot, exit_after) = (opts.screenshot.clone(), opts.exit_after);
    let egui_ctx = egui::Context::default();
    egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
    let title_speed = session.sky.speed;
    let workshop = Workshop::new(&session.engine);
    let mut app = App {
        session,
        opts: Some(opts),
        gfx: None,
        streamer: None,
        egui_ctx,
        controls: Controls::default(),
        last: Instant::now(),
        title_speed,
        shift: false,
        menu: Menu::new(photos::dir()),
        photo: None,
        live_shot: false,
        mark_wanted: false,
        gate: Gate::default(),
        mark_recapture: false,
        visual_checked: Instant::now(),
        writer: ShotWriter::default(),
        digests: ChunkDigests::default(),
        banner: None,
        opaque: Vec::new(),
        water: Vec::new(),
        captured: false,
        vsync: true,
        show_stats: true,
        presets,
        frames: VecDeque::new(),
        cpu_ms: 0.0,
        last_tris: (0, 0, 0),
        started: Instant::now(),
        screenshot,
        exit_after,
        want_exit: false,
        save_note: String::new(),
        falling_draw: FallingDraws::default(),
        workshop,
        replay,
    };
    el.run_app(&mut app).unwrap();
    app.replay.as_ref().is_none_or(|w| w.runner.report.ok())
}

//! The windowed app: the player (walk or fly), streaming, rendering, the
//! egui HUD, block breaking and placing, and autosave.

use crate::camera::Camera;
use crate::controls::Controls;
use crate::engine::{Engine, Preset};
use crate::player::Mode;
use crate::render::{DrawRecord, Renderer};
use crate::session::Session;
use crate::sim::draw::FallingDraws;
use crate::stream::Streamer;
use crate::ui::{EguiPaint, Hud, draw_hud, outline_segments};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
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
    /// `--replay FILE --watch` (M2c): the replay drives the steps instead
    /// of the keyboard and mouse.
    replay: Option<WatchedReplay>,
}

struct WatchedReplay {
    runner: crate::replay::Runner<'static>,
    acc: f64,
    reported: bool,
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
            let path = path.unwrap_or_else(|| crate::replay::default_path(&self.session.engine.config.name));
            match self.session.start_recording(&path, "recorded in the app (F10)") {
                Ok(p) => {
                    self.save_note = format!("RECORDING replay (F10 stops): {}", p.display());
                    println!("recording a replay to {} (F10 stops)", p.display());
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
        let rec = if self.session.recording() { " | REC replay (F10 stops)" } else { "" };
        gfx.window.set_title(&format!("Triangulum | time {speed}x ([ slower, ] faster){rec}"));
    }

    fn save_on_exit(&mut self) {
        if self.session.recording() {
            self.toggle_recording();
        }
        match self.session.save_now() {
            Ok(n) if n > 0 => println!("saved {n} chunk(s) to {}", self.session.save.as_ref().map_or("-".into(), |s| s.path.display().to_string())),
            Ok(_) => {}
            Err(e) => eprintln!("save failed: {e}"),
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.25);
        self.last = now;
        // Fixed 60 Hz steps (the same steps the harness and a replay take):
        // the queued inputs go in at the next step, in order (M2c).
        if let Some(w) = &mut self.replay {
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
                self.save_note = if r.ok() { format!("replay finished at step {}: reproduced exactly", r.steps) } else { format!("replay DIVERGED at step {} (see the console)", r.first.as_ref().map_or(0, |d| d.step)) };
            }
        } else {
            self.controls.frame(&mut self.session, dt);
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
        let cam = self.controls.camera(&self.session.player);

        let (Some(gfx), Some(streamer)) = (&mut self.gfx, &mut self.streamer) else { return };
        let frame = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                self.configure_surface();
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
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

        // HUD (crosshair, outline and hotbar always; stats on H).
        {
            let raw = gfx.egui_state.take_egui_input(&gfx.window);
            let (w, h) = (gfx.config.width, gfx.config.height);
            let hud = self.hud_info(gpu_ms, &cam, aspect, w, h);
            let stats = self.show_stats;
            let full = self.egui_ctx.run_ui(raw, |ui| draw_hud(ui.ctx(), &hud, stats));
            let gfx = self.gfx.as_mut().unwrap();
            gfx.egui_state.handle_platform_output(&gfx.window, full.platform_output);
            let prims = self.egui_ctx.tessellate(full.shapes, full.pixels_per_point);
            gfx.egui_paint.paint(&gfx.gpu.device, &gfx.gpu.queue, &view, (gfx.config.width, gfx.config.height), full.pixels_per_point, &prims, &full.textures_delta);
        }
        let gfx = self.gfx.as_mut().unwrap();
        if self.exit_after.is_some_and(|s| self.started.elapsed().as_secs_f64() >= s) {
            if let Some(path) = &self.screenshot {
                let rgba = gfx.gpu.read_texture(&frame.texture, gfx.config.width, gfx.config.height);
                crate::render::write_png(path, gfx.config.width, gfx.config.height, &rgba);
                println!("screenshot saved to {path}");
            }
            self.want_exit = true;
        }
        gfx.window.pre_present_notify();
        gfx.gpu.queue.present(frame);
        let frame_ms = now.elapsed().as_secs_f64() * 1e3;
        self.frames.push_back((now, frame_ms));
        while self.frames.front().is_some_and(|(t, _)| now.duration_since(*t).as_secs_f64() > 5.0) {
            self.frames.pop_front();
        }
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
        let outline = pick.map_or(Vec::new(), |h| outline_segments(&engine.planet, cam, aspect, width, height, h.face, h.cell));
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
            preset: self.presets.get(self.controls.preset()).map_or("-".into(), |p| p.name.clone()),
            hide_water: engine.hide_water,
            captured: self.captured,
            mode: if p.mode == Mode::Walk { "walk".into() } else { "fly".into() },
            on_ground: p.on_ground,
            in_water: p.in_water,
            hotbar: engine.hotbar.iter().map(|&id| (engine.registry.def(id).name.clone(), engine.registry.def(id).color)).collect(),
            selected: p.selected,
            outline,
            edits: engine.edits.len(),
            dirty: engine.edits.dirty_count(),
            save_note: self.save_note.clone(),
            sim_line: self.session.sim.describe(),
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
        println!("app: WASD move, Space jump/up, Ctrl sneak (fly: down), Shift sprint, mouse look (click to capture, Esc releases), LMB break, RMB place, 1-9 block, G walk/fly, scroll fly speed, P preset, R to ground, V vsync, F water, H hud, [ slower time, ] faster time, F10 record a replay, Q quit");
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
                    // Watching a replay, only the app's own keys work.
                    let first = if self.replay.is_some() { pressed && !event.repeat } else { self.controls.key(code, pressed, self.presets.len()) };
                    if first {
                        match code {
                            KeyCode::Escape => self.set_capture(false),
                            KeyCode::KeyQ => {
                                self.save_on_exit();
                                el.exit()
                            }
                            KeyCode::KeyV => {
                                self.vsync = !self.vsync;
                                self.configure_surface();
                            }
                            KeyCode::KeyH => self.show_stats = !self.show_stats,
                            KeyCode::F10 if self.replay.is_none() => self.toggle_recording(),
                            _ => {}
                        }
                    }
                }
            }
            WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } if self.replay.is_some() => {}
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Pressed && !self.captured {
                    // The capturing click is not game input.
                    self.set_capture(true);
                } else if self.captured || state == ElementState::Released {
                    self.controls.button(button, state == ElementState::Pressed);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.controls.scroll(lines);
            }
            WindowEvent::Focused(false) => self.controls.focus_lost(),
            WindowEvent::Resized(_) => self.configure_surface(),
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.captured && self.replay.is_none() {
                self.controls.motion(delta.0, delta.1);
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
pub fn run_replay(session: Session, rep: crate::replay::Replay, opts: AppOptions) -> i32 {
    // The window owns the runner until the process ends.
    let rep: &'static crate::replay::Replay = Box::leak(Box::new(rep));
    let runner = crate::replay::Runner::new(rep, &session);
    let ok = run_with(session, opts, Some(WatchedReplay { runner, acc: 0.0, reported: false }));
    if ok { 0 } else { 1 }
}

fn run_with(session: Session, opts: AppOptions, replay: Option<WatchedReplay>) -> bool {
    let el = EventLoop::new().unwrap();
    let presets = session.engine.presets();
    let (screenshot, exit_after) = (opts.screenshot.clone(), opts.exit_after);
    let egui_ctx = egui::Context::default();
    egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
    let title_speed = session.sky.speed;
    let mut app = App {
        session,
        opts: Some(opts),
        gfx: None,
        streamer: None,
        egui_ctx,
        controls: Controls::default(),
        last: Instant::now(),
        title_speed,
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
        replay,
    };
    el.run_app(&mut app).unwrap();
    app.replay.as_ref().is_none_or(|w| w.runner.report.ok())
}

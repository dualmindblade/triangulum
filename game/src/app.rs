//! The windowed app: the player (walk or fly), streaming, rendering, the
//! egui HUD, block breaking and placing, and autosave.

use crate::camera::Camera;
use crate::engine::{Engine, Preset};
use crate::player::{Input, Mode, STEP};
use crate::render::{DrawRecord, Renderer};
use crate::session::Session;
use crate::stream::Streamer;
use crate::ui::{EguiPaint, Hud, draw_hud, outline_segments};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
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
}

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Renderer,
    egui_state: egui_winit::State,
    egui_paint: EguiPaint,
}

/// Holding a mouse button repeats its action this often.
const REPEAT_S: f64 = 0.25;

struct App {
    session: Session,
    opts: Option<AppOptions>,
    gfx: Option<Gfx>,
    streamer: Option<Streamer>,
    egui_ctx: egui::Context,
    keys: HashSet<KeyCode>,
    last: Instant,
    /// Fixed-step accumulator (s).
    acc: f64,
    opaque: Vec<DrawRecord>,
    water: Vec<DrawRecord>,
    captured: bool,
    vsync: bool,
    show_stats: bool,
    preset: usize,
    presets: Vec<Preset>,
    /// (time, frame ms) over the last five seconds.
    frames: VecDeque<(Instant, f64)>,
    cpu_ms: f64,
    last_tris: (u64, u64, usize),
    started: Instant,
    screenshot: Option<String>,
    exit_after: Option<f64>,
    want_exit: bool,
    mouse_held: Option<(MouseButton, Instant)>,
    save_note: String,
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

    fn apply_preset(&mut self, i: usize) {
        if self.presets.is_empty() {
            return;
        }
        self.preset = i % self.presets.len();
        let p = self.presets[self.preset].clone();
        self.session.player.heading = p.heading_deg.to_radians();
        self.session.player.pitch = p.pitch_deg.to_radians();
        if p.alt_agl > 50.0 {
            self.session.set_mode(Mode::Fly);
            let ground = self.engine().ground(p.dir);
            self.session.teleport(p.dir, Some(ground + p.alt_agl));
        } else {
            self.session.set_mode(Mode::Walk);
            self.session.teleport(p.dir, None);
        }
        println!("preset {}: {}", self.preset, p.name);
    }

    /// Swap the streamer for the session's current engine (water toggle, reload).
    fn reset_streamer(&mut self) {
        if let (Some(s), Some(g)) = (&mut self.streamer, &mut self.gfx) {
            s.clear(&mut g.gpu);
        }
        self.streamer = Some(self.engine().streamer());
    }

    fn input(&self) -> Input {
        let k = &self.keys;
        Input {
            forward: k.contains(&KeyCode::KeyW),
            back: k.contains(&KeyCode::KeyS),
            left: k.contains(&KeyCode::KeyA),
            right: k.contains(&KeyCode::KeyD),
            jump: k.contains(&KeyCode::Space),
            down: k.contains(&KeyCode::ControlLeft) || k.contains(&KeyCode::ControlRight),
            sprint: k.contains(&KeyCode::ShiftLeft) || k.contains(&KeyCode::ShiftRight),
        }
    }

    fn mouse_action(&mut self, button: MouseButton) {
        match button {
            MouseButton::Left => {
                self.session.break_block();
            }
            MouseButton::Right => {
                self.session.place_block();
            }
            _ => {}
        }
    }

    fn save_on_exit(&mut self) {
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
        // Fixed-step player physics (the same step the harness uses).
        self.acc += dt;
        let input = self.input();
        let mut steps = 0;
        while self.acc >= STEP && steps < 8 {
            self.session.step(&input, STEP);
            self.acc -= STEP;
            steps += 1;
        }
        if steps == 8 {
            self.acc = 0.0;
        }
        if let Some((button, since)) = self.mouse_held {
            if since.elapsed().as_secs_f64() >= REPEAT_S {
                self.mouse_action(button);
                self.mouse_held = Some((button, Instant::now()));
            }
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
        let cam = self.session.player.camera();

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
        let engine = self.session.engine.clone();
        let uni = cam.uniform(aspect, engine.sun_at(cam.pos), &engine.planet, engine.water_alpha);
        let fs = gfx.gpu.render(&uni, &self.opaque, &self.water, Some(&view));
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
            preset: self.presets.get(self.preset).map_or("-".into(), |p| p.name.clone()),
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
        println!("app: WASD move, Space jump/up, Ctrl down, Shift sprint, mouse look (click to capture, Esc releases), LMB break, RMB place, 1-9 block, G walk/fly, scroll fly speed, P preset, R to ground, V vsync, F water, H hud, Q quit");
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
                    if event.state == ElementState::Pressed {
                        let first = self.keys.insert(code);
                        if first {
                            match code {
                                KeyCode::Escape => self.set_capture(false),
                                KeyCode::KeyQ => {
                                    self.save_on_exit();
                                    el.exit()
                                }
                                KeyCode::KeyP => self.apply_preset(self.preset + 1),
                                KeyCode::KeyV => {
                                    self.vsync = !self.vsync;
                                    self.configure_surface();
                                }
                                KeyCode::KeyH => self.show_stats = !self.show_stats,
                                KeyCode::KeyF => {
                                    let hide = !self.engine().hide_water;
                                    self.session.set_hide_water(hide);
                                    self.reset_streamer();
                                }
                                KeyCode::KeyG => self.session.toggle_mode(),
                                KeyCode::KeyR => {
                                    let dir = self.session.player.pos.normalize();
                                    self.session.set_mode(Mode::Walk);
                                    self.session.teleport(dir, None);
                                }
                                KeyCode::Digit1 => self.session.select(0),
                                KeyCode::Digit2 => self.session.select(1),
                                KeyCode::Digit3 => self.session.select(2),
                                KeyCode::Digit4 => self.session.select(3),
                                KeyCode::Digit5 => self.session.select(4),
                                KeyCode::Digit6 => self.session.select(5),
                                KeyCode::Digit7 => self.session.select(6),
                                KeyCode::Digit8 => self.session.select(7),
                                KeyCode::Digit9 => self.session.select(8),
                                _ => {}
                            }
                        }
                    } else {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button, .. } => {
                if !self.captured {
                    self.set_capture(true);
                } else {
                    self.mouse_action(button);
                    self.mouse_held = Some((button, Instant::now()));
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, .. } => self.mouse_held = None,
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                let p = &mut self.session.player;
                p.fly_speed = (p.fly_speed * 1.3f64.powf(steps)).clamp(1.0, 5000.0);
            }
            WindowEvent::Focused(false) => self.mouse_held = None,
            WindowEvent::Resized(_) => self.configure_surface(),
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.captured {
                let p = &mut self.session.player;
                p.heading += delta.0 * 0.0022;
                p.pitch = (p.pitch - delta.1 * 0.0022).clamp(-1.55, 1.55);
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
    let el = EventLoop::new().unwrap();
    let presets = session.engine.presets();
    let (screenshot, exit_after) = (opts.screenshot.clone(), opts.exit_after);
    let egui_ctx = egui::Context::default();
    egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
    let mut app = App {
        session,
        opts: Some(opts),
        gfx: None,
        streamer: None,
        egui_ctx,
        keys: HashSet::new(),
        last: Instant::now(),
        acc: 0.0,
        opaque: Vec::new(),
        water: Vec::new(),
        captured: false,
        vsync: true,
        show_stats: true,
        preset: 0,
        presets,
        frames: VecDeque::new(),
        cpu_ms: 0.0,
        last_tris: (0, 0, 0),
        started: Instant::now(),
        screenshot,
        exit_after,
        want_exit: false,
        mouse_held: None,
        save_note: String::new(),
    };
    el.run_app(&mut app).unwrap();
}

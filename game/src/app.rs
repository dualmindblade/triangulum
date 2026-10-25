//! The windowed app: free-fly camera, streaming, rendering, egui HUD.

use crate::camera::Camera;
use crate::engine::{Engine, Preset};
use crate::grid::BlockPos;
use crate::render::{DrawRecord, Renderer};
use crate::stream::Streamer;
use crate::ui::{EguiPaint, Hud, draw_hud};
use glam::DVec3;
use std::collections::{HashSet, VecDeque};
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
    pub start: Camera,
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

struct App {
    engine: Arc<Engine>,
    opts: Option<AppOptions>,
    gfx: Option<Gfx>,
    streamer: Option<Streamer>,
    egui_ctx: egui::Context,
    cam: Camera,
    keys: HashSet<KeyCode>,
    last: Instant,
    opaque: Vec<DrawRecord>,
    water: Vec<DrawRecord>,
    captured: bool,
    speed: f64,
    vsync: bool,
    show_hud: bool,
    hide_water: bool,
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
}

impl App {
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
        let p = &self.presets[self.preset];
        self.cam = self.engine.camera_at(p.dir, p.alt_agl, p.heading_deg, p.pitch_deg);
        println!("preset {}: {}", self.preset, p.name);
    }

    /// Block under the crosshair: march the view ray through loaded LOD-0
    /// chunks, falling back to the generator's column heights.
    fn pick(&self) -> Option<(BlockPos, String)> {
        let streamer = self.streamer.as_ref()?;
        let (fwd, _, _) = self.cam.vectors();
        let planet = &self.engine.planet;
        let mut t = 0.0;
        while t < 2000.0 {
            let p = self.cam.pos + fwd * t;
            let bp = planet.block_of_pos(p);
            let key = crate::grid::ChunkKey { face: bp.face, lod: 0, cx: bp.i / 32, cy: bp.j / 32, cz: bp.k / 32 };
            let hit = match streamer.chunks.get(&key) {
                Some(c) => {
                    let id = c.get((bp.i % 32) as usize, (bp.j % 32) as usize, (bp.k % 32) as usize);
                    if self.engine.registry.is_solid(id) || self.engine.registry.is_liquid(id) { Some(id) } else { None }
                }
                None => {
                    let dir = p.normalize();
                    let h = self.engine.generator.ground_height_m(dir);
                    let k_surface = planet.sea_level as f64 + h / planet.block_size;
                    if (bp.k as f64) < k_surface { Some(u16::MAX) } else { None }
                }
            };
            if let Some(id) = hit {
                let name = if id == u16::MAX { "(unloaded: column surface)".to_string() } else { self.engine.registry.def(id).name.clone() };
                return Some((bp, name));
            }
            t += (t * 0.01).max(0.5 * planet.block_size);
        }
        None
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.1);
        self.last = now;
        // Movement.
        let mut speed = self.speed;
        if self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight) {
            speed *= 4.0;
        }
        let (fwd, _, up) = self.cam.vectors();
        let right = self.cam.right();
        let mut mv = DVec3::ZERO;
        if self.keys.contains(&KeyCode::KeyW) { mv += fwd; }
        if self.keys.contains(&KeyCode::KeyS) { mv -= fwd; }
        if self.keys.contains(&KeyCode::KeyA) { mv -= right; }
        if self.keys.contains(&KeyCode::KeyD) { mv += right; }
        if self.keys.contains(&KeyCode::Space) { mv += up; }
        if self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight) { mv -= up; }
        if mv != DVec3::ZERO {
            self.cam.pos += mv.normalize() * speed * dt;
        }
        // Stay above the bottom of the world.
        let min_r = self.engine.planet.bottom_radius() + 2.0;
        if self.cam.pos.length() < min_r {
            self.cam.pos = self.cam.pos.normalize() * min_r;
        }

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
        let vp = self.cam.view_proj(aspect);
        streamer.update(self.cam.pos, vp, &mut gfx.gpu, &mut self.opaque, &mut self.water);
        let uni = self.cam.uniform(aspect, self.engine.sun_at(self.cam.pos), &self.engine.planet, self.engine.water_alpha);
        let fs = gfx.gpu.render(&uni, &self.opaque, &self.water, Some(&view));
        self.cpu_ms = t0.elapsed().as_secs_f64() * 1e3;
        self.last_tris = (fs.triangles, fs.water_triangles, (fs.draws + fs.water_draws) as usize);
        let gpu_ms = gfx.gpu.poll_gpu_ms();

        // HUD.
        if self.show_hud {
            let raw = gfx.egui_state.take_egui_input(&gfx.window);
            let hud = self.hud_info(gpu_ms);
            let full = self.egui_ctx.run_ui(raw, |ui| draw_hud(ui.ctx(), &hud));
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

    fn hud_info(&self, gpu_ms: f64) -> Hud {
        let now = Instant::now();
        let last_1s: Vec<f64> = self.frames.iter().filter(|(t, _)| now.duration_since(*t).as_secs_f64() <= 1.0).map(|(_, ms)| *ms).collect();
        let fps = if last_1s.is_empty() { 0.0 } else { 1000.0 / (last_1s.iter().sum::<f64>() / last_1s.len() as f64) };
        let mut all: Vec<f64> = self.frames.iter().map(|(_, ms)| *ms).collect();
        all.sort_by(|a, b| b.total_cmp(a));
        let n1 = (all.len() / 100).max(1).min(all.len().max(1));
        let low1 = if all.is_empty() { 0.0 } else { 1000.0 / (all[..n1].iter().sum::<f64>() / n1 as f64) };
        let streamer = self.streamer.as_ref().unwrap();
        let st = &streamer.stats;
        let (lat, lon, alt) = self.cam.lat_lon_alt(&self.engine.planet);
        let ground = self.engine.generator.ground_height_m(self.cam.pos.normalize());
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
            block: self.pick(),
            speed: self.speed,
            vsync: self.vsync,
            loaded: st.loaded,
            inflight: st.inflight,
            pending: st.pending,
            tris: self.last_tris.0,
            water_tris: self.last_tris.1,
            draws: self.last_tris.2,
            vram_mb: self.gfx.as_ref().map_or(0.0, |g| g.gpu.resident_bytes() as f64 / 1e6),
            preset: self.presets.get(self.preset).map_or("-".into(), |p| p.name.clone()),
            hide_water: self.hide_water,
            captured: self.captured,
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
        let gpu = Renderer::new(&instance, Some(&surface), opts.width, opts.height, self.engine.quad_mb, self.engine.slot_capacity, &self.engine.colours);
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
        self.streamer = Some(self.engine.streamer());
        self.cam = opts.start;
        self.vsync = opts.vsync;
        self.gfx = Some(Gfx { window, surface, config, gpu, egui_state, egui_paint });
        el.set_control_flow(ControlFlow::Poll);
        println!("app: WASD move, Space/Ctrl up/down, mouse look (click to capture, Esc releases), scroll speed, Shift boost, P preset, V vsync, F water, H hud, Q quit");
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(gfx) = self.gfx.as_mut() {
            let _ = gfx.egui_state.on_window_event(&gfx.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed {
                        let first = self.keys.insert(code);
                        if first {
                            match code {
                                KeyCode::Escape => self.set_capture(false),
                                KeyCode::KeyQ => el.exit(),
                                KeyCode::KeyP => self.apply_preset(self.preset + 1),
                                KeyCode::KeyV => {
                                    self.vsync = !self.vsync;
                                    self.configure_surface();
                                }
                                KeyCode::KeyH => self.show_hud = !self.show_hud,
                                KeyCode::KeyF => {
                                    self.hide_water = !self.hide_water;
                                    let engine = Arc::new(self.engine.with_flags(self.hide_water, self.engine.markers_on));
                                    self.engine = engine;
                                    if let (Some(s), Some(g)) = (&mut self.streamer, &mut self.gfx) {
                                        s.clear(&mut g.gpu);
                                    }
                                    self.streamer = Some(self.engine.streamer());
                                }
                                KeyCode::KeyR => {
                                    let dir = self.cam.pos.normalize();
                                    self.cam = self.engine.camera_at(dir, 1.7, self.cam.heading.to_degrees(), 0.0);
                                }
                                _ => {}
                            }
                        }
                    } else {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, .. } => {
                if !self.captured {
                    self.set_capture(true);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.speed = (self.speed * 1.3f64.powf(steps)).clamp(1.0, 5000.0);
            }
            WindowEvent::Resized(_) => self.configure_surface(),
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _id: winit::event::DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.captured {
                self.cam.heading += delta.0 * 0.0022;
                self.cam.pitch = (self.cam.pitch - delta.1 * 0.0022).clamp(-1.55, 1.55);
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.want_exit {
            el.exit();
            return;
        }
        if let Some(g) = &self.gfx {
            g.window.request_redraw();
        }
    }
}

pub fn run(engine: Arc<Engine>, opts: AppOptions) {
    let el = EventLoop::new().unwrap();
    let presets = engine.presets();
    let (screenshot, exit_after) = (opts.screenshot.clone(), opts.exit_after);
    let egui_ctx = egui::Context::default();
    egui_ctx.all_styles_mut(|s| s.animation_time = 0.0);
    let mut app = App {
        cam: opts.start,
        engine,
        opts: Some(opts),
        gfx: None,
        streamer: None,
        egui_ctx,
        keys: HashSet::new(),
        last: Instant::now(),
        opaque: Vec::new(),
        water: Vec::new(),
        captured: false,
        speed: 20.0,
        vsync: true,
        show_hud: true,
        hide_water: false,
        preset: 0,
        presets,
        frames: VecDeque::new(),
        cpu_ms: 0.0,
        last_tris: (0, 0, 0),
        started: Instant::now(),
        screenshot,
        exit_after,
        want_exit: false,
    };
    el.run_app(&mut app).unwrap();
}

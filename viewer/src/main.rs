//! Triangulum Phase-0 viewer: fly over Neisor from orbit.
//!
//!   cargo run --release                          interactive window
//!   cargo run --release -- --capture shot.png \
//!       --lat 15 --lon 40 --alt 12000            headless screenshot
//!
//! Controls: drag = orbit, scroll = altitude, Esc = quit.

use anyhow::Result;
use triangulum_viewer::camera::Camera;
use triangulum_viewer::planet::Planet;
use triangulum_viewer::renderer::Renderer;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

struct Args {
    capture: Option<String>,
    lat: f64,
    lon: f64,
    alt: f64,
    yaw: f64,   // degrees, 0 = north
    pitch: f64, // degrees, 0 = horizon (999 = auto by altitude)
    exaggeration: f64,
    size: (u32, u32),
    sun: Option<(f64, f64)>, // (lat, lon) degrees; None = sun follows camera
}

fn parse_args() -> Args {
    let mut a = Args {
        capture: None,
        lat: 10.0,
        lon: 30.0,
        alt: 20000.0,
        yaw: 0.0,
        pitch: 999.0,
        exaggeration: 10.0,
        size: (1600, 900),
        sun: None,
    };
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        let next = |i: usize| argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--capture" => {
                a.capture = Some(next(i));
                i += 1;
            }
            "--lat" => {
                a.lat = next(i).parse().unwrap_or(a.lat);
                i += 1;
            }
            "--lon" => {
                a.lon = next(i).parse().unwrap_or(a.lon);
                i += 1;
            }
            "--alt" => {
                a.alt = next(i).parse().unwrap_or(a.alt);
                i += 1;
            }
            "--exagg" => {
                a.exaggeration = next(i).parse().unwrap_or(a.exaggeration);
                i += 1;
            }
            "--yaw" => {
                a.yaw = next(i).parse().unwrap_or(a.yaw);
                i += 1;
            }
            "--pitch" => {
                a.pitch = next(i).parse().unwrap_or(a.pitch);
                i += 1;
            }
            "--sun-lat" => {
                let v: f64 = next(i).parse().unwrap_or(30.0);
                a.sun = Some((v, a.sun.map_or(30.0, |s| s.1)));
                i += 1;
            }
            "--sun-lon" => {
                let v: f64 = next(i).parse().unwrap_or(30.0);
                a.sun = Some((a.sun.map_or(30.0, |s| s.0), v));
                i += 1;
            }
            other => eprintln!("unknown arg: {other}"),
        }
        i += 1;
    }
    a
}

fn assets_dir() -> String {
    // work from repo root or from viewer/
    if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets".into()
    } else {
        "assets".into()
    }
}

fn main() -> Result<()> {
    let args = parse_args();
    let planet = Planet::load(&assets_dir())?;
    // default pitch: look at the planet from orbit, at the horizon when low
    let auto_pitch = if args.pitch > 360.0 {
        let t = (args.alt / 8000.0).clamp(0.0, 1.0);
        -12.0 - 73.0 * t
    } else {
        args.pitch.clamp(-86.0, 86.0)
    };
    let mut camera = Camera {
        lon: args.lon.to_radians(),
        lat: args.lat.to_radians(),
        altitude_km: args.alt,
        radius_km: planet.radius_km,
        ground_km: 0.0,
        yaw: args.yaw.to_radians(),
        pitch: auto_pitch.to_radians(),
    };
    camera.ground_km = triangulum_viewer::terrain::ground_height_km(
        &planet,
        camera.position().normalize(),
        args.exaggeration,
    );

    if let Some(path) = args.capture.clone() {
        return capture(planet, camera, args, &path);
    }

    let event_loop = EventLoop::new()?;
    let planet_seed = planet.seed;
    let mut app = App {
        planet,
        camera,
        args,
        gfx: None,
        dragging: false,
        last_cursor: (0.0, 0.0),
        keys: Default::default(),
        mode: Mode::Fly,
        last_frame: std::time::Instant::now(),
        vert_vel_mps: 0.0,
        grounded: false,
        mouse_locked: false,
        teleport: None,
        title_timer: 0.0,
        edits: load_edits(planet_seed),
        underwater: false,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

// ---------------------------------------------------------------- capture

fn capture(planet: Planet, camera: Camera, args: Args, path: &str) -> Result<()> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let mut renderer = Renderer::new(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        args.size,
        args.exaggeration,
    );
    renderer.sun_dir = args.sun.map(|(la, lo)| {
        let (la, lo) = (la.to_radians(), lo.to_radians());
        glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin())
    });
    // headless shots see the same edited world the game saves
    let edits = load_edits(planet.seed);
    renderer.underwater = triangulum_viewer::voxel::water_surface_km(
        &planet,
        &edits,
        camera.position().normalize(),
        args.exaggeration,
    )
    .is_some_and(|w| camera.ground_km + camera.altitude_km < w - 0.0003);
    let n = renderer.capture(&planet, &camera, &edits, path)?;
    println!(
        "captured {path} ({} tiles, lat {:.1} lon {:.1} alt {:.0} km)",
        n,
        camera.lat.to_degrees(),
        camera.lon.to_degrees(),
        camera.altitude_km
    );
    Ok(())
}

// ---------------------------------------------------------------- window app

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Fly,
    Walk,
}

struct App {
    planet: Planet,
    camera: Camera,
    args: Args,
    gfx: Option<Gfx>,
    dragging: bool,
    last_cursor: (f64, f64),
    keys: std::collections::HashSet<winit::keyboard::KeyCode>,
    mode: Mode,
    last_frame: std::time::Instant,
    vert_vel_mps: f64, // vertical velocity in walk mode (gravity, jumps, swim)
    grounded: bool,    // feet resting on a solid top last frame
    mouse_locked: bool, // pointer captured: raw-motion look, cursor hidden
    /// In-progress teleport entry ("lat lon [alt]"), typed into the title bar.
    teleport: Option<String>,
    title_timer: f64,
    edits: triangulum_viewer::voxel::Edits,
    underwater: bool, // eye below a water surface (walk mode wading)
}

/// Player eye height above the feet, km.
const EYE_KM: f64 = 0.0018;

/// Where in-game screenshots land (matches the interchange workflow).
fn interchange_dir() -> &'static str {
    if std::path::Path::new("viewer/interchange").exists() {
        "viewer/interchange"
    } else {
        "interchange"
    }
}

/// Player block edits persist here, keyed by planet seed.
fn edits_path(seed: i64) -> String {
    format!("{}/edits_seed{}.bin", assets_dir(), seed)
}

fn load_edits(seed: i64) -> triangulum_viewer::voxel::Edits {
    let mut out = triangulum_viewer::voxel::Edits::default();
    let Ok(raw) = std::fs::read(edits_path(seed)) else { return out };
    if raw.len() < 8 || &raw[0..4] != b"EDT1" {
        return out;
    }
    let n = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
    if raw.len() != 8 + n * 25 {
        return out;
    }
    for k in 0..n {
        let o = 8 + k * 25;
        let face = raw[o];
        let ci = u64::from_le_bytes(raw[o + 1..o + 9].try_into().unwrap());
        let cj = u64::from_le_bytes(raw[o + 9..o + 17].try_into().unwrap());
        let dh = i64::from_le_bytes(raw[o + 17..o + 25].try_into().unwrap());
        out.insert((face, ci, cj), dh);
    }
    out
}

fn save_edits(seed: i64, edits: &triangulum_viewer::voxel::Edits) {
    let mut buf = Vec::with_capacity(8 + edits.len() * 25);
    buf.extend_from_slice(b"EDT1");
    buf.extend_from_slice(&(edits.len() as u32).to_le_bytes());
    for (&(face, ci, cj), &dh) in edits {
        buf.push(face);
        buf.extend_from_slice(&ci.to_le_bytes());
        buf.extend_from_slice(&cj.to_le_bytes());
        buf.extend_from_slice(&dh.to_le_bytes());
    }
    if let Err(e) = std::fs::write(edits_path(seed), buf) {
        eprintln!("could not save edits: {e}");
    }
}

impl App {
    /// Break (dh = -1) or place (dh = +1) a block at the targeted column.
    /// Breaking removes the top block of the column you hit; placing is
    /// face-aware: aiming at the side of something builds on the column in
    /// front of it (the last air column the ray crossed), aiming down at a
    /// top face grows that column. Edits are per-column height deltas, so a
    /// placed block always lands on its column's top.
    fn edit_block(&mut self, dh: i64) {
        let eye = self.camera.position();
        let look = self.camera.look_dir();
        let reach_m = if self.mode == Mode::Walk { 8.0 } else { 60.0 };
        if let Some((hit, prev)) = triangulum_viewer::voxel::raycast_column(
            &self.planet,
            &self.edits,
            eye,
            look,
            reach_m,
            self.args.exaggeration,
        ) {
            let (face, ci, cj) = if dh > 0 { prev } else { hit };
            *self.edits.entry((face, ci, cj)).or_insert(0) += dh;
            let dirty = triangulum_viewer::voxel::chunks_touching_column(face, ci, cj);
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.invalidate_chunks(&dirty);
            }
            save_edits(self.planet.seed, &self.edits);
        }
    }

    /// P: capture the current view to interchange/ with the coordinates in
    /// the filename, so shared screenshots carry their own repro command.
    fn save_screenshot(&mut self) {
        let dir = interchange_dir();
        let _ = std::fs::create_dir_all(dir);
        let base = format!(
            "shot_lat{:.3}_lon{:.3}_alt{:.3}km_yaw{:.0}_pitch{:.0}",
            self.camera.lat.to_degrees(),
            self.camera.lon.to_degrees(),
            self.camera.altitude_km,
            self.camera.yaw.to_degrees(),
            self.camera.pitch.to_degrees(),
        );
        let mut path = format!("{dir}/{base}.png");
        let mut n = 2;
        while std::path::Path::new(&path).exists() {
            path = format!("{dir}/{base}_{n}.png");
            n += 1;
        }
        let Some(gfx) = self.gfx.as_mut() else { return };
        let msg = match gfx.renderer.capture(&self.planet, &self.camera, &self.edits, &path) {
            Ok(_) => format!("saved {path}"),
            Err(e) => format!("screenshot failed: {e}"),
        };
        println!("{msg}");
        gfx.window.set_title(&format!("Neisor — {msg}"));
        self.title_timer = -2.0; // let the message linger
    }

    /// Refresh the title-bar teleport prompt.
    fn show_teleport_prompt(&self) {
        if let (Some(input), Some(gfx)) = (&self.teleport, &self.gfx) {
            gfx.window.set_title(&format!(
                "Neisor — teleport> {input}_   (lat lon [alt km] — Enter go, Esc cancel)"
            ));
        }
    }

    /// Enter: parse "lat lon [alt]" and go (fly mode at the destination).
    fn teleport_go(&mut self) {
        let Some(input) = self.teleport.take() else { return };
        let nums: Vec<f64> = input
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect();
        if nums.len() < 2 || nums[0].abs() > 90.0 {
            if let Some(gfx) = &self.gfx {
                gfx.window.set_title("Neisor — teleport: need `lat lon [alt km]`");
                self.title_timer = -2.0;
            }
            return;
        }
        self.camera.lat = nums[0].to_radians();
        self.camera.lon = nums[1].to_radians();
        if let Some(&alt) = nums.get(2) {
            self.camera.altitude_km = alt.clamp(0.0025, 80000.0);
        } else {
            self.camera.altitude_km = self.camera.altitude_km.max(0.05);
        }
        self.mode = Mode::Fly;
        self.vert_vel_mps = 0.0;
        self.grounded = false;
        self.camera.ground_km = triangulum_viewer::terrain::ground_height_km(
            &self.planet,
            self.camera.position().normalize(),
            self.args.exaggeration,
        );
    }

    /// Capture or release the cursor for raw mouse-look.
    fn set_mouse_lock(&mut self, lock: bool) {
        let Some(gfx) = &self.gfx else { return };
        use winit::window::CursorGrabMode as G;
        if lock {
            let ok = gfx
                .window
                .set_cursor_grab(G::Locked)
                .or_else(|_| gfx.window.set_cursor_grab(G::Confined))
                .is_ok();
            if ok {
                gfx.window.set_cursor_visible(false);
                self.mouse_locked = true;
            }
        } else {
            let _ = gfx.window.set_cursor_grab(G::None);
            gfx.window.set_cursor_visible(true);
            self.mouse_locked = false;
        }
    }

    /// Keystrokes while the teleport prompt is open. Returns true if consumed.
    fn teleport_key(&mut self, key: &winit::keyboard::Key) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if self.teleport.is_none() {
            return false;
        }
        match key {
            Key::Named(NamedKey::Enter) => self.teleport_go(),
            Key::Named(NamedKey::Escape) => {
                self.teleport = None;
                self.title_timer = 1.0; // restore the HUD title promptly
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(t) = self.teleport.as_mut() {
                    t.pop();
                }
                self.show_teleport_prompt();
            }
            Key::Named(NamedKey::Space) => {
                if let Some(t) = self.teleport.as_mut() {
                    t.push(' ');
                }
                self.show_teleport_prompt();
            }
            Key::Character(s) => {
                if let Some(t) = self.teleport.as_mut() {
                    t.extend(s.chars().filter(|c| "0123456789.,+- ".contains(*c)));
                }
                self.show_teleport_prompt();
            }
            _ => {}
        }
        true
    }
}

impl App {
    /// Per-frame simulation: movement, ground following, jump physics.
    fn update(&mut self) {
        use winit::keyboard::KeyCode as K;
        let dt = {
            let now = std::time::Instant::now();
            let dt = now.duration_since(self.last_frame).as_secs_f64();
            self.last_frame = now;
            dt.min(0.1)
        };
        let fwd = (self.keys.contains(&K::KeyW) as i32 - self.keys.contains(&K::KeyS) as i32) as f64;
        let strafe = (self.keys.contains(&K::KeyD) as i32 - self.keys.contains(&K::KeyA) as i32) as f64;

        use triangulum_viewer::voxel::{ceiling_above_km, support_below_km, water_surface_km};
        let planet = &self.planet;
        let edits = &self.edits;
        let exagg = self.args.exaggeration;
        let voxels_live = self.camera.altitude_km
            < triangulum_viewer::renderer::VOXEL_MAX_ALT_KM;

        match self.mode {
            Mode::Fly => {
                self.underwater = false;
                if (fwd != 0.0 || strafe != 0.0) && self.teleport.is_none() {
                    // speed scales with altitude: cruise in orbit, glide low
                    let speed_kms = (self.camera.altitude_km * 0.5).clamp(0.02, 600.0);
                    let sprint = if self.keys.contains(&K::ShiftLeft) { 4.0 } else { 1.0 };
                    let h = self.camera.heading(strafe, fwd);
                    self.camera.translate(h, speed_kms * sprint * dt);
                }
                let dir2 = self.camera.position().normalize();
                if voxels_live {
                    // near the ground, absolute height is preserved when the
                    // ground re-samples, so a cave pit passing underneath no
                    // longer yanks the camera: descend deliberately and drop
                    // in. The reference is the voxel *support* under the
                    // camera (cave floors count) and roofs are solid.
                    let cur = self.camera.ground_km + self.camera.altitude_km;
                    let ground = support_below_km(planet, edits, dir2, cur - 1e-9, exagg);
                    let ceil = ceiling_above_km(planet, edits, dir2, ground + 1e-6, exagg);
                    let height = cur
                        .max(ground + 0.0025)
                        .min(ceil - 0.0008)
                        .max(ground + 0.0012);
                    self.camera.ground_km = ground;
                    self.camera.altitude_km = height - ground;
                } else {
                    // cruising: classic terrain-following at constant AGL
                    self.camera.ground_km = triangulum_viewer::terrain::ground_height_km(
                        planet, dir2, exagg,
                    );
                }
            }
            Mode::Walk => {
                let mut feet = self.camera.ground_km;
                // -- horizontal, with side collision and 1-block step-up
                if (fwd != 0.0 || strafe != 0.0) && self.teleport.is_none() {
                    let sprint = if self.keys.contains(&K::ShiftLeft) { 2.2 } else { 1.0 };
                    let h = self.camera.heading(strafe, fwd);
                    let saved = (self.camera.lat, self.camera.lon, self.camera.yaw);
                    self.camera.translate(h, 0.0043 * sprint * dt); // 4.3 m/s
                    let ndir = self.camera.position().normalize();
                    let block = triangulum_viewer::voxel::VOXEL_KM * exagg;
                    let step = if self.grounded { 1.05 * block } else { 0.05 * block };
                    let head = feet + EYE_KM + 0.0003;
                    // highest solid under the head in the target column: at or
                    // below the feet it's floor (walk on / fall past), within a
                    // step it's a stair, above that it's a wall
                    let s_head = support_below_km(planet, edits, ndir, head, exagg);
                    let new_feet = feet.max(s_head);
                    let headroom = ceiling_above_km(planet, edits, ndir, new_feet + 1e-6, exagg)
                        - new_feet
                        > EYE_KM + 0.0004;
                    let mut blocked = s_head > feet + step + 1e-9 || !headroom;
                    // body radius: the eye stays ~0.35 blocks away from any
                    // wall, so the near plane can never poke inside a block
                    // (walking face-first into a tree trunk showed its
                    // hollow interior). Probes ring the new position; walls
                    // above step height or tight ceilings reject the move.
                    if !blocked {
                        let r_km = 0.35 * block;
                        let pos = self.camera.position();
                        let (_, north, east) = self.camera.frame();
                        for k in 0..8 {
                            let a = k as f64 * std::f64::consts::FRAC_PI_4;
                            let pdir =
                                (pos + (north * a.cos() + east * a.sin()) * r_km).normalize();
                            let s = support_below_km(planet, edits, pdir, head, exagg);
                            if s > new_feet + step + 1e-9
                                || ceiling_above_km(planet, edits, pdir, new_feet + 1e-6, exagg)
                                    - new_feet
                                    <= EYE_KM + 0.0004
                            {
                                blocked = true;
                                break;
                            }
                        }
                    }
                    if blocked {
                        (self.camera.lat, self.camera.lon, self.camera.yaw) = saved;
                    } else {
                        feet = new_feet;
                        if s_head > self.camera.ground_km {
                            self.vert_vel_mps = self.vert_vel_mps.max(0.0);
                        }
                    }
                }
                let dir2 = self.camera.position().normalize();
                // -- vertical: gravity (or buoyancy), landing, head bump
                let water = water_surface_km(planet, edits, dir2, exagg);
                let in_water = water.is_some_and(|w| feet + 0.0009 < w);
                if in_water {
                    // sink slowly; hold Space to swim up
                    let target = if self.keys.contains(&K::Space) { 3.0 } else { -1.4 };
                    let blend = (6.0 * dt).min(1.0);
                    self.vert_vel_mps += (target - self.vert_vel_mps) * blend;
                } else {
                    self.vert_vel_mps = (self.vert_vel_mps - 9.81 * dt).max(-80.0);
                }
                let mut new_feet = feet + self.vert_vel_mps * dt / 1000.0;
                let support = support_below_km(planet, edits, dir2, feet + 1e-7, exagg);
                self.grounded = false;
                if new_feet <= support {
                    new_feet = support;
                    self.vert_vel_mps = 0.0;
                    self.grounded = true;
                } else if self.vert_vel_mps > 0.0 {
                    let ceil = ceiling_above_km(planet, edits, dir2, feet + EYE_KM, exagg);
                    if new_feet + EYE_KM + 0.0004 > ceil {
                        new_feet = (ceil - EYE_KM - 0.0004).max(support);
                        self.vert_vel_mps = 0.0;
                    }
                }
                self.camera.ground_km = new_feet;
                self.camera.altitude_km = EYE_KM;
                self.underwater =
                    water.is_some_and(|w| new_feet + EYE_KM < w - 0.0003);
            }
        }

        // window title as a tiny HUD, twice a second (the teleport prompt
        // owns the title while it's open)
        self.title_timer += dt;
        if self.title_timer > 0.5 && self.teleport.is_none() {
            self.title_timer = 0.0;
            if let Some(gfx) = &self.gfx {
                let mode = match self.mode {
                    Mode::Fly => "fly (click captures mouse, G walk, T teleport, P shot)",
                    Mode::Walk => "walk (F fly, space jump, T teleport, P shot)",
                };
                gfx.window.set_title(&format!(
                    "Neisor — {} | lat {:.3} lon {:.3} alt {:.3} km",
                    mode,
                    self.camera.lat.to_degrees(),
                    self.camera.lon.to_degrees(),
                    self.camera.altitude_km
                ));
            }
        }
    }
}

impl ApplicationHandler for App {
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        // raw mouse motion drives the view while the pointer is captured
        if let winit::event::DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if self.mouse_locked && self.teleport.is_none() {
                self.camera.yaw += dx * 0.0022;
                self.camera.pitch = (self.camera.pitch - dy * 0.0022).clamp(-1.50, 1.50);
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Neisor — triangulum viewer")
                        .with_inner_size(winit::dpi::LogicalSize::new(1600, 900)),
                )
                .expect("create window"),
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("device");
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface config");
        let caps = surface.get_capabilities(&adapter);
        if let Some(srgb) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        let format = config.format;
        surface.configure(&device, &config);
        let mut renderer = Renderer::new(
            device,
            queue,
            format,
            (config.width, config.height),
            self.args.exaggeration,
        );
        renderer.sun_dir = self.args.sun.map(|(la, lo)| {
            let (la, lo) = (la.to_radians(), lo.to_radians());
            glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin())
        });
        self.gfx = Some(Gfx { window, surface, config, renderer });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.gfx.is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{KeyCode as K, PhysicalKey};
                // an open teleport prompt owns the keyboard
                if event.state == ElementState::Pressed
                    && self.teleport_key(&event.logical_key)
                {
                    return;
                }
                if event.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
                    && event.state == ElementState::Pressed
                {
                    // Esc backs out one layer: mouse capture first, then quit
                    if self.mouse_locked {
                        self.set_mouse_lock(false);
                        return;
                    }
                    event_loop.exit();
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            self.keys.insert(code);
                            match code {
                                K::KeyG => {
                                    // walk starts wherever the camera is:
                                    // pressed in flight, you fall from there
                                    let feet = self.camera.ground_km
                                        + self.camera.altitude_km
                                        - EYE_KM;
                                    self.mode = Mode::Walk;
                                    self.camera.ground_km = feet;
                                    self.camera.altitude_km = EYE_KM;
                                    self.vert_vel_mps = 0.0;
                                    self.grounded = false;
                                }
                                K::KeyF => {
                                    self.mode = Mode::Fly;
                                    self.camera.altitude_km =
                                        self.camera.altitude_km.max(0.004);
                                }
                                K::Space => {
                                    if self.mode == Mode::Walk && self.grounded {
                                        self.vert_vel_mps = 5.2;
                                        self.grounded = false;
                                    }
                                }
                                K::KeyQ => self.edit_block(-1),
                                K::KeyE => self.edit_block(1),
                                K::KeyT => {
                                    self.teleport = Some(String::new());
                                    self.show_teleport_prompt();
                                }
                                K::KeyP => self.save_screenshot(),
                                _ => {}
                            }
                        }
                        ElementState::Released => {
                            self.keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::Resized(size) => {
                let gfx = self.gfx.as_mut().unwrap();
                gfx.config.width = size.width.max(1);
                gfx.config.height = size.height.max(1);
                gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                gfx.renderer.resize((gfx.config.width, gfx.config.height));
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                // first click captures the mouse for raw free-look (Esc
                // releases); drag-look remains as the uncaptured fallback
                if state == ElementState::Pressed && !self.mouse_locked {
                    self.set_mouse_lock(true);
                }
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (dx, dy) =
                    (position.x - self.last_cursor.0, position.y - self.last_cursor.1);
                self.last_cursor = (position.x, position.y);
                if self.dragging && !self.mouse_locked {
                    // free look: drag turns the head, not the globe.
                    // pitch stops short of vertical: at exactly nadir the
                    // view basis degenerates and the image flips
                    self.camera.yaw += dx * 0.0032;
                    self.camera.pitch = (self.camera.pitch - dy * 0.0032).clamp(-1.50, 1.50);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.mode == Mode::Fly {
                    let amount = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(p) => p.y / 60.0,
                    };
                    // floor at ~2.5 m: hover over the grass, sink into cave
                    // pits — the update pass keeps the camera out of solids
                    self.camera.altitude_km =
                        (self.camera.altitude_km * (1.0 - amount * 0.12)).clamp(0.0025, 80000.0);
                    // the descent cinematic: zooming eases the view from
                    // planet-gazing toward the horizon as altitude drops.
                    // The pull is a pure function of altitude (identical
                    // ascending and descending) and fades to nothing below
                    // FREE_LOOK_ALT_KM — near the ground the camera angle is
                    // entirely the player's, as it is in walk mode.
                    const FREE_LOOK_ALT_KM: f64 = 100.0;
                    let alt = self.camera.altitude_km;
                    if alt > FREE_LOOK_ALT_KM {
                        let t = (alt / 8000.0).clamp(0.0, 1.0);
                        let target = (-12.0 - 73.0 * t).to_radians();
                        let engage = ((alt / FREE_LOOK_ALT_KM).ln() / 8.0f64.ln())
                            .clamp(0.0, 1.0);
                        self.camera.pitch += (target - self.camera.pitch) * 0.35 * engage;
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.update();
                let gfx = self.gfx.as_mut().unwrap();
                use wgpu::CurrentSurfaceTexture as Cst;
                let frame = match gfx.surface.get_current_texture() {
                    Cst::Success(f) | Cst::Suboptimal(f) => f,
                    _ => {
                        gfx.surface.configure(&gfx.renderer.device, &gfx.config);
                        gfx.window.request_redraw();
                        return;
                    }
                };
                let view = frame.texture.create_view(&Default::default());
                gfx.renderer.underwater = self.underwater;
                gfx.renderer.draw(&view, &self.planet, &self.camera, &self.edits);
                gfx.renderer.queue.present(frame);
                gfx.window.request_redraw();
            }
            _ => {}
        }
    }
}

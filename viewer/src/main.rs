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
use triangulum_viewer::renderer::{Renderer, SunState};
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
    sun: Option<(f64, f64)>, // (lat, lon) degrees; None = day/night cycle
    /// Seconds per full day/night cycle (0 = no cycle, sun follows the
    /// camera — the legacy always-noon mode).
    day_len: f64,
    /// Voxel patch radius multiplier (chunks stream in asynchronously,
    /// so bigger discs cost memory and build throughput, not frame hitches).
    patch: f64,
    /// Opt in to the descent cinematic: above 100 km, scrolling altitude
    /// eases the view pitch toward the planet. Default OFF so scroll never
    /// touches pitch and the camera is entirely yours (request C-3).
    auto_tilt: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        capture: None,
        lat: 10.0,
        lon: 30.0,
        alt: 20000.0,
        yaw: 0.0,
        pitch: 999.0,
        exaggeration: 1.0,
        size: (1600, 900),
        sun: None,
        day_len: 1200.0,
        patch: 1.0,
        auto_tilt: false,
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
            "--day-len" => {
                a.day_len = next(i).parse().unwrap_or(a.day_len);
                i += 1;
            }
            "--patch" => {
                a.patch = next(i).parse::<f64>().unwrap_or(a.patch).clamp(0.3, 2.0);
                i += 1;
            }
            "--auto-tilt" => a.auto_tilt = true,
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
    let planet = Arc::new(Planet::load(&assets_dir())?);
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
        player: PlayerState::default(),
        last_frame: std::time::Instant::now(),
        mouse_locked: false,
        teleport: None,
        title_timer: 0.0,
        edits: load_edits(planet_seed),
        torches: load_torches(planet_seed),
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

// ---------------------------------------------------------------- capture

fn capture(planet: Arc<Planet>, camera: Camera, args: Args, path: &str) -> Result<()> {
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
    renderer.day_len_s = args.day_len;
    renderer.sun_ref_lon = args.lon.to_radians();
    renderer.patch_scale = args.patch;
    // headless shots see the same edited world the game saves
    let edits = load_edits(planet.seed);
    renderer.torches = load_torches(planet.seed);
    renderer.underwater = triangulum_viewer::voxel::water_surface_km(
        &planet,
        &edits,
        camera.position().normalize(),
        args.exaggeration,
    )
    .is_some_and(|w| camera.ground_km + camera.altitude_km < w - 0.0003);
    let (n, sun, sun_pinned, day_len_s) =
        capture_with_recorded_sun(&mut renderer, &planet, &camera, &edits, path)?;
    write_shot_sidecar(path, &planet, &camera, &args, "fly", sun, sun_pinned, day_len_s)?;
    println!(
        "captured {path} ({} tiles, lat {:.1} lon {:.1} alt {:.0} km)",
        n,
        camera.lat.to_degrees(),
        camera.lon.to_degrees(),
        camera.altitude_km
    );
    Ok(())
}

fn capture_with_recorded_sun(
    renderer: &mut Renderer,
    planet: &Arc<Planet>,
    camera: &Camera,
    edits: &triangulum_viewer::voxel::Edits,
    path: &str,
) -> Result<(usize, SunState, bool, f64)> {
    let sun_pinned = renderer.sun_dir.is_some();
    let day_len_s = renderer.day_len_s;
    let sun = renderer.sun_state(camera.position());
    let old_sun = renderer.sun_dir;
    renderer.sun_dir = Some(sun.dir);
    let result = renderer.capture(planet, camera, edits, path);
    renderer.sun_dir = old_sun;
    result.map(|n| (n, sun, sun_pinned, day_len_s))
}

fn world_source() -> String {
    let meta_path = format!("{}/meta.json", assets_dir());
    std::fs::read_to_string(&meta_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|meta| meta["source"].as_str().map(str::to_owned))
        .unwrap_or_else(|| assets_dir())
}

fn sun_lat_lon(sun: SunState) -> (f64, f64) {
    let dir = sun.dir.normalize();
    (dir.z.asin().to_degrees(), dir.y.atan2(dir.x).to_degrees())
}

fn write_shot_sidecar(
    path: &str,
    planet: &Arc<Planet>,
    camera: &Camera,
    args: &Args,
    mode: &str,
    sun: SunState,
    sun_pinned: bool,
    day_len_s: f64,
) -> Result<()> {
    let (sun_lat, sun_lon) = sun_lat_lon(sun);
    let js = serde_json::json!({
        "lat_deg": camera.lat.to_degrees(),
        "lon_deg": camera.lon.to_degrees(),
        "alt_km": camera.altitude_km,
        "yaw_deg": camera.yaw.to_degrees(),
        "pitch_deg": camera.pitch.to_degrees(),
        "sun_lat_deg": sun_lat,
        "sun_lon_deg": sun_lon,
        "sun_dir": {
            "x": sun.dir.x,
            "y": sun.dir.y,
            "z": sun.dir.z,
        },
        "sun_pinned": sun_pinned,
        "day_cycle_time_s": sun.day_time_s,
        "day_len_s": day_len_s,
        "exaggeration": args.exaggeration,
        "seed": planet.seed,
        "world": world_source(),
        "mode": mode,
    });
    let mut sidecar = std::path::PathBuf::from(path);
    sidecar.set_extension("json");
    std::fs::write(sidecar, serde_json::to_string_pretty(&js)?)?;
    Ok(())
}

// ---------------------------------------------------------------- window app

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

use triangulum_viewer::player::{Mode, PlayerState};

struct App {
    planet: Arc<Planet>,
    camera: Camera,
    args: Args,
    gfx: Option<Gfx>,
    dragging: bool,
    last_cursor: (f64, f64),
    keys: std::collections::HashSet<winit::keyboard::KeyCode>,
    player: PlayerState,
    last_frame: std::time::Instant,
    mouse_locked: bool, // pointer captured: raw-motion look, cursor hidden
    /// In-progress teleport entry ("lat lon [alt]"), typed into the title bar.
    teleport: Option<String>,
    title_timer: f64,
    edits: triangulum_viewer::voxel::Edits,
    torches: triangulum_viewer::voxel::Torches,
}

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

/// Player-placed torches persist here, keyed by planet seed.
fn torches_path(seed: i64) -> String {
    format!("{}/torches_seed{}.bin", assets_dir(), seed)
}

fn load_torches(seed: i64) -> triangulum_viewer::voxel::Torches {
    let mut out = triangulum_viewer::voxel::Torches::default();
    let Ok(raw) = std::fs::read(torches_path(seed)) else { return out };
    if raw.len() < 8 || &raw[0..4] != b"TRC1" {
        return out;
    }
    let n = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
    if raw.len() != 8 + n * 17 {
        return out;
    }
    for k in 0..n {
        let o = 8 + k * 17;
        let face = raw[o];
        let ci = u64::from_le_bytes(raw[o + 1..o + 9].try_into().unwrap());
        let cj = u64::from_le_bytes(raw[o + 9..o + 17].try_into().unwrap());
        out.insert((face, ci, cj));
    }
    out
}

fn save_torches(seed: i64, torches: &triangulum_viewer::voxel::Torches) {
    let mut buf = Vec::with_capacity(8 + torches.len() * 17);
    buf.extend_from_slice(b"TRC1");
    buf.extend_from_slice(&(torches.len() as u32).to_le_bytes());
    for &(face, ci, cj) in torches {
        buf.push(face);
        buf.extend_from_slice(&ci.to_le_bytes());
        buf.extend_from_slice(&cj.to_le_bytes());
    }
    if let Err(e) = std::fs::write(torches_path(seed), buf) {
        eprintln!("could not save torches: {e}");
    }
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
        if let Some(dirty) = triangulum_viewer::player::edit_block(
            &self.planet,
            &mut self.edits,
            &self.camera,
            self.player.mode,
            dh,
            self.args.exaggeration,
        ) {
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.invalidate_chunks(&dirty);
            }
            self.player.refresh_after_edit(
                &self.planet,
                &self.edits,
                &self.camera,
                self.args.exaggeration,
            );
            save_edits(self.planet.seed, &self.edits);
        }
    }

    /// R: toggle a torch on the walkable top of the targeted column.
    fn toggle_torch(&mut self) {
        if let Some(dirty) = triangulum_viewer::player::toggle_torch(
            &self.planet,
            &self.edits,
            &mut self.torches,
            &self.camera,
            self.player.mode,
            self.args.exaggeration,
        ) {
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.torches = self.torches.clone();
                gfx.renderer.invalidate_chunks(&dirty);
            }
            save_torches(self.planet.seed, &self.torches);
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
        let mode = if self.player.mode == Mode::Walk { "walk" } else { "fly" };
        let msg = match capture_with_recorded_sun(
            &mut gfx.renderer,
            &self.planet,
            &self.camera,
            &self.edits,
            &path,
        ) {
            Ok((_, sun, sun_pinned, day_len_s)) => match write_shot_sidecar(
                &path,
                &self.planet,
                &self.camera,
                &self.args,
                mode,
                sun,
                sun_pinned,
                day_len_s,
            ) {
                Ok(()) => format!("saved {path}"),
                Err(e) => format!("saved {path}; sidecar failed: {e}"),
            },
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
        self.player.teleport(
            &self.planet,
            &self.edits,
            &mut self.camera,
            nums[0],
            nums[1],
            nums.get(2).copied(),
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
    /// Per-frame simulation: resolve held keys into a player Input and run
    /// the shared player physics (player.rs — same code the play harness
    /// scripts drive).
    fn update(&mut self) {
        use winit::keyboard::KeyCode as K;
        let dt = {
            let now = std::time::Instant::now();
            let dt = now.duration_since(self.last_frame).as_secs_f64();
            self.last_frame = now;
            dt.min(0.1)
        };
        // an open teleport prompt owns the keyboard: no movement input
        let input = if self.teleport.is_some() {
            triangulum_viewer::player::Input::default()
        } else {
            triangulum_viewer::player::Input {
                fwd: (self.keys.contains(&K::KeyW) as i32
                    - self.keys.contains(&K::KeyS) as i32) as f64,
                strafe: (self.keys.contains(&K::KeyD) as i32
                    - self.keys.contains(&K::KeyA) as i32) as f64,
                sprint: self.keys.contains(&K::ShiftLeft),
                swim_up: self.keys.contains(&K::Space),
            }
        };
        self.player.update(
            &self.planet,
            &self.edits,
            &mut self.camera,
            &input,
            self.args.exaggeration,
            dt,
        );

        // window title as a tiny HUD, twice a second (the teleport prompt
        // owns the title while it's open)
        self.title_timer += dt;
        if self.title_timer > 0.5 && self.teleport.is_none() {
            self.title_timer = 0.0;
            if let Some(gfx) = &self.gfx {
                let mode = match self.player.mode {
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
        renderer.day_len_s = self.args.day_len;
        renderer.sun_ref_lon = self.args.lon.to_radians();
        renderer.patch_scale = self.args.patch;
        renderer.torches = self.torches.clone();
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
                                K::KeyG => self.player.set_walk(&mut self.camera),
                                K::KeyF => self.player.set_fly(&mut self.camera),
                                K::Space => self.player.jump(),
                                K::KeyQ => self.edit_block(-1),
                                K::KeyE => self.edit_block(1),
                                K::KeyR => self.toggle_torch(),
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
                if self.player.mode == Mode::Fly {
                    let amount = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y as f64,
                        MouseScrollDelta::PixelDelta(p) => p.y / 60.0,
                    };
                    // floor at ~2.5 m: hover over the grass, sink into cave
                    // pits — the update pass keeps the camera out of solids
                    self.camera.altitude_km =
                        (self.camera.altitude_km * (1.0 - amount * 0.12)).clamp(0.0025, 80000.0);
                    // the descent cinematic (opt-in, request C-3): with
                    // --auto-tilt, zooming eases the view from planet-gazing
                    // toward the horizon as altitude drops. The pull is a pure
                    // function of altitude (identical ascending and descending)
                    // and fades to nothing below FREE_LOOK_ALT_KM. Default OFF:
                    // scroll never touches pitch, so the camera is entirely the
                    // player's, as it is in walk mode.
                    const FREE_LOOK_ALT_KM: f64 = 100.0;
                    let alt = self.camera.altitude_km;
                    if self.args.auto_tilt && alt > FREE_LOOK_ALT_KM {
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
                gfx.renderer.underwater = self.player.underwater;
                gfx.renderer.draw(&view, &self.planet, &self.camera, &self.edits);
                gfx.renderer.queue.present(frame);
                gfx.window.request_redraw();
            }
            _ => {}
        }
    }
}

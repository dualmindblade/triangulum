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
    /// Living weather (WEATHER.md): "live" (default), "off", or
    /// "COVER,PRECIP" to pin the sky (each 0..1) for art shots.
    weather: String,
    /// --no-voxels: pure heightfield-mesh render (no chunk streaming, no
    /// hole). The eyeball twin of the sync-diff harness's `voxels off`.
    voxels: bool,
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
        weather: "live".into(),
        voxels: true,
    };
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        let next = |i: usize| argv.get(i + 1).cloned().unwrap_or_default();
        // "NaN"/"inf" parse as valid f64 and poison the camera (every
        // comparison goes false: culling dies, tile selection explodes,
        // torch sorting panics) — numeric args accept finite values only
        let numf = |i: usize, d: f64| {
            next(i).parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(d)
        };
        match argv[i].as_str() {
            "--capture" => {
                a.capture = Some(next(i));
                i += 1;
            }
            "--lat" => {
                a.lat = numf(i, a.lat).clamp(-90.0, 90.0);
                i += 1;
            }
            "--lon" => {
                a.lon = numf(i, a.lon);
                i += 1;
            }
            "--alt" => {
                a.alt = numf(i, a.alt).clamp(0.0025, 80000.0);
                i += 1;
            }
            "--exagg" => {
                a.exaggeration = numf(i, a.exaggeration);
                i += 1;
            }
            "--yaw" => {
                a.yaw = numf(i, a.yaw);
                i += 1;
            }
            "--pitch" => {
                a.pitch = numf(i, a.pitch);
                i += 1;
            }
            "--sun-lat" => {
                let v = numf(i, 30.0);
                a.sun = Some((v, a.sun.map_or(30.0, |s| s.1)));
                i += 1;
            }
            "--sun-lon" => {
                let v = numf(i, 30.0);
                a.sun = Some((a.sun.map_or(30.0, |s| s.0), v));
                i += 1;
            }
            "--day-len" => {
                a.day_len = numf(i, a.day_len);
                i += 1;
            }
            "--patch" => {
                a.patch = numf(i, a.patch).clamp(0.3, 2.0);
                i += 1;
            }
            "--auto-tilt" => a.auto_tilt = true,
            "--no-voxels" => a.voxels = false,
            "--weather" => {
                a.weather = next(i);
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

/// Wire the living weather into a renderer per the --weather arg
/// (WEATHER.md). Missing weather.bin degrades to a clear sky, loudly.
fn apply_weather(renderer: &mut Renderer, spec: &str) {
    match triangulum_viewer::weather::WeatherField::load(&assets_dir()) {
        Ok(f) => renderer.weather_field = Some(f),
        Err(e) => eprintln!("weather off ({e}) - run scripts/bake_weather.py"),
    }
    renderer.weather_tuning = triangulum_viewer::weather::WeatherTuning::load(&assets_dir());
    match spec {
        "off" => renderer.weather_on = false,
        "" | "live" => {}
        s => {
            if let Some((c, p)) = s.split_once(',')
                && let (Ok(c), Ok(p)) = (c.parse::<f32>(), p.parse::<f32>())
                && c.is_finite()
                && p.is_finite()
            {
                renderer.weather_pin = Some((c.clamp(0.0, 1.0), p.clamp(0.0, 1.0)));
            } else {
                eprintln!("--weather expects off | live | COVER,PRECIP");
            }
        }
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
        photo_map: triangulum_viewer::ui::PhotoMap::new(interchange_dir().into()),
        egui_ctx: egui::Context::default(),
        egui_state: None,
        egui_paint: None,
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
    renderer.voxels_on = args.voxels;
    // headless shots see the same edited world the game saves
    let edits = load_edits(planet.seed);
    renderer.set_torches(load_torches(planet.seed));
    renderer.refresh_world_snapshot(&edits);
    apply_weather(&mut renderer, &args.weather);
    let eye_km = camera.ground_km + camera.altitude_km;
    renderer.underwater = triangulum_viewer::voxel::water_surface_km(
        &planet,
        &edits,
        camera.position().normalize(),
        eye_km,
        args.exaggeration,
    )
    .is_some_and(|w| eye_km < w - 0.0003);
    let (n, sun, sun_pinned, day_len_s) =
        capture_with_recorded_sun(&mut renderer, &planet, &camera, &edits, path)?;
    write_shot_sidecar(
        path, &planet, &camera, &args, "fly", sun, sun_pinned, day_len_s, &renderer,
    )?;
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
    renderer: &Renderer,
) -> Result<()> {
    let (sun_lat, sun_lon) = sun_lat_lon(sun);
    // the weather this photo was taken in (WEATHER.md): weather is a pure
    // function of (seed, position, time), so these fields make a storm
    // shot exactly reproducible like the sun already is
    let wx = renderer.last_weather;
    let weather_js = serde_json::json!({
        "on": renderer.weather_on,
        "pinned": renderer.weather_pin.map(|(c, p)| vec![c, p]),
        "t_s": renderer.render_time_s(),
        "season_frac": triangulum_viewer::weather::season_frac(
            renderer.render_time_s(),
            renderer.day_len_s,
            &renderer.weather_tuning,
        ),
        "cloud_cover": wx.cloud_cover,
        "precip": wx.precip,
        "snow_frac": wx.snow_frac,
        "temp_c": wx.temp_c,
    });
    let js = serde_json::json!({
        "lat_deg": camera.lat.to_degrees(),
        "lon_deg": camera.lon.to_degrees(),
        "alt_km": camera.altitude_km,
        // absolute ground height under the camera: alt_km alone can't
        // reproduce a shot (photo-map restore otherwise re-derives ground
        // from the far terrain surface — wrong in caves and on ice)
        "ground_km": camera.ground_km,
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
        "weather": weather_js,
        // which build took this photo — the first triage question after a
        // day of rapid pushes (a long-lived session outlives many commits).
        // option_env: a build-script hiccup must never fail the build.
        "build": option_env!("TRI_BUILD").unwrap_or("unstamped"),
        // framerate at the moment of the shot (avg/p95 frame ms, avg draw
        // CPU ms over ~4 s) — "the framerate suffered HERE" becomes data
        "frame_ms": renderer.frame_stats().map(|(avg, p95, cost)| serde_json::json!({
            "avg": (avg * 10.0).round() / 10.0,
            "p95": (p95 * 10.0).round() / 10.0,
            "draw_cpu": (cost * 10.0).round() / 10.0,
        })),
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
    /// T opens the photo-map popup (ui.rs): teleport by map, photo, or
    /// typed coordinates; browse/delete the screenshot roll.
    photo_map: triangulum_viewer::ui::PhotoMap,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_paint: Option<triangulum_viewer::ui::EguiPaint>,
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

/// A structurally valid save file can still carry a corrupt record (face
/// byte >= 6 panics `face_dir`; an off-lattice column or absurd edit delta
/// poisons meshing) — every record is range-checked on load, bad ones are
/// dropped with a warning instead of taking the session down.
fn valid_column(face: u8, ci: u64, cj: u64) -> bool {
    let n = triangulum_viewer::voxel::COLUMNS_PER_FACE;
    (face as usize) < 6 && ci < n && cj < n
}

/// Largest per-column edit delta a save may carry: generous for gameplay
/// (a 4 km tower), small enough that height arithmetic can't wrap.
const MAX_EDIT_BLOCKS: i64 = 4096;

/// Write through a temp file + rename so a crash mid-save can't leave a
/// truncated file that silently drops the whole roll on the next load.
fn write_atomic(path: &str, buf: &[u8]) -> std::io::Result<()> {
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, buf)?;
    std::fs::rename(&tmp, path)
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
    let mut dropped = 0usize;
    for k in 0..n {
        let o = 8 + k * 17;
        let face = raw[o];
        let ci = u64::from_le_bytes(raw[o + 1..o + 9].try_into().unwrap());
        let cj = u64::from_le_bytes(raw[o + 9..o + 17].try_into().unwrap());
        if !valid_column(face, ci, cj) {
            dropped += 1;
            continue;
        }
        out.insert((face, ci, cj));
    }
    if dropped > 0 {
        eprintln!("torches: dropped {dropped} corrupt record(s)");
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
    if let Err(e) = write_atomic(&torches_path(seed), &buf) {
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
    let mut dropped = 0usize;
    for k in 0..n {
        let o = 8 + k * 25;
        let face = raw[o];
        let ci = u64::from_le_bytes(raw[o + 1..o + 9].try_into().unwrap());
        let cj = u64::from_le_bytes(raw[o + 9..o + 17].try_into().unwrap());
        let dh = i64::from_le_bytes(raw[o + 17..o + 25].try_into().unwrap());
        if dh == 0 {
            continue; // dig-then-refill leaves a harmless no-op entry
        }
        if !valid_column(face, ci, cj) || dh.abs() > MAX_EDIT_BLOCKS {
            dropped += 1;
            continue;
        }
        out.insert((face, ci, cj), dh);
    }
    if dropped > 0 {
        eprintln!("edits: dropped {dropped} corrupt record(s)");
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
    if let Err(e) = write_atomic(&edits_path(seed), &buf) {
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
                gfx.renderer.refresh_edits_snapshot(&self.edits);
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
                gfx.renderer.set_torches(self.torches.clone());
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
                &gfx.renderer,
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

    /// V key: the in-game sync-delta meter — scripts/sync_diff.py's twin.
    /// Renders the current pose twice (voxel patch on, then off), diffs the
    /// pair in memory, and saves vox/mesh/heatmap PNGs plus sidecars under
    /// interchange/sync/. Both frames share one recorded sun and weather
    /// instant, so they differ ONLY by the voxel patch. Rules of use per
    /// TRANSITIONS.md "The meter": instrument, not an optimization target.
    fn save_sync_delta(&mut self) {
        let dir = format!("{}/sync", interchange_dir());
        let _ = std::fs::create_dir_all(&dir);
        let base = format!(
            "sync_lat{:.3}_lon{:.3}_alt{:.3}km_yaw{:.0}_pitch{:.0}",
            self.camera.lat.to_degrees(),
            self.camera.lon.to_degrees(),
            self.camera.altitude_km,
            self.camera.yaw.to_degrees(),
            self.camera.pitch.to_degrees(),
        );
        let mode = if self.player.mode == Mode::Walk { "walk" } else { "fly" };
        let Some(gfx) = self.gfx.as_mut() else { return };
        let r = &mut gfx.renderer;
        let sun_pinned = r.sun_dir.is_some();
        let day_len_s = r.day_len_s;
        let sun = r.sun_state(self.camera.position());
        let old_sun = r.sun_dir;
        r.sun_dir = Some(sun.dir);
        let was_on = r.voxels_on;
        r.voxels_on = true;
        let vox = r.capture_rgba(&self.planet, &self.camera, &self.edits);
        r.voxels_on = false;
        let mesh = r.capture_rgba(&self.planet, &self.camera, &self.edits);
        r.voxels_on = was_on;
        r.sun_dir = old_sun;
        let msg = match (vox, mesh) {
            (Ok((vox, _)), Ok((mesh, _))) => {
                let (w, h) = r.size;
                // same statistic as scripts/sync_diff.py: worst-channel
                // delta per pixel, <=2/255 counts as identical (noise floor)
                let mut hist = [0u64; 256];
                let mut lum = 0.0f64;
                let mut heat = mesh.clone();
                for i in (0..vox.len().min(mesh.len())).step_by(4) {
                    let d = vox[i]
                        .abs_diff(mesh[i])
                        .max(vox[i + 1].abs_diff(mesh[i + 1]))
                        .max(vox[i + 2].abs_diff(mesh[i + 2]));
                    hist[d as usize] += 1;
                    if d > 2 {
                        lum += 0.2126 * (f64::from(vox[i]) - f64::from(mesh[i]))
                            + 0.7152 * (f64::from(vox[i + 1]) - f64::from(mesh[i + 1]))
                            + 0.0722 * (f64::from(vox[i + 2]) - f64::from(mesh[i + 2]));
                    }
                    // heatmap: dimmed mesh frame, divergence burned in red
                    heat[i] = (f64::from(mesh[i]) * 0.35)
                        .max((f64::from(d) * 3.0).min(255.0)) as u8;
                    heat[i + 1] = (f64::from(mesh[i + 1]) * 0.35) as u8;
                    heat[i + 2] = (f64::from(mesh[i + 2]) * 0.35) as u8;
                }
                let total: u64 = hist.iter().sum::<u64>().max(1);
                let div: u64 = hist[3..].iter().sum();
                let mean = hist
                    .iter()
                    .enumerate()
                    .skip(3)
                    .map(|(v, &c)| v as f64 * c as f64)
                    .sum::<f64>()
                    / div.max(1) as f64;
                // numpy-compatible p95 (linear interpolation between order
                // statistics, truncated like the Python meter): the old
                // nearest-rank histogram walk disagreed with sync_diff.py
                // by a bin and reported 3 on IDENTICAL frames (its first
                // test was 0 >= 0) - review #2 finding 14
                let p95 = if div == 0 {
                    0
                } else {
                    let pos = (div - 1) as f64 * 0.95;
                    let (lo_rank, frac) = (pos.floor() as u64, pos.fract());
                    let (mut acc, mut lo_val, mut hi_val) = (0u64, 0usize, 0usize);
                    for v in 3..256 {
                        let start = acc;
                        acc += hist[v];
                        if lo_val == 0 && start <= lo_rank && lo_rank < acc {
                            lo_val = v;
                        }
                        if start <= lo_rank + 1 && lo_rank + 1 < acc {
                            hi_val = v;
                            break;
                        }
                    }
                    if hi_val == 0 {
                        hi_val = lo_val;
                    }
                    (lo_val as f64 + frac * (hi_val as f64 - lo_val as f64)) as usize
                };
                let div_frac = div as f64 / total as f64;
                let signed_lum = lum / div.max(1) as f64;
                let vox_path = format!("{dir}/{base}_vox.png");
                let write = Renderer::write_png(&vox_path, w, h, &vox)
                    .and_then(|()| {
                        Renderer::write_png(&format!("{dir}/{base}_mesh.png"), w, h, &mesh)
                    })
                    .and_then(|()| {
                        Renderer::write_png(&format!("{dir}/{base}_diff.png"), w, h, &heat)
                    })
                    .and_then(|()| {
                        // the vox frame gets the standard repro sidecar; the
                        // metrics ride in their own _delta.json next to it
                        write_shot_sidecar(
                            &vox_path,
                            &self.planet,
                            &self.camera,
                            &self.args,
                            mode,
                            sun,
                            sun_pinned,
                            day_len_s,
                            r,
                        )
                    })
                    .and_then(|()| {
                        let js = serde_json::json!({
                            "divergent_frac": (div_frac * 10000.0).round() / 10000.0,
                            "mean_delta": (mean * 100.0).round() / 100.0,
                            "p95_delta": p95,
                            "signed_lum": (signed_lum * 100.0).round() / 100.0,
                            "noise_floor": 2,
                        });
                        std::fs::write(
                            format!("{dir}/{base}_delta.json"),
                            serde_json::to_string_pretty(&js)?,
                        )?;
                        Ok(())
                    });
                match write {
                    Ok(()) => format!(
                        "sync delta: div {:.1}% mean {:.1} p95 {} lum {:+.1} — {dir}/{base}_*",
                        div_frac * 100.0,
                        mean,
                        p95,
                        signed_lum
                    ),
                    Err(e) => format!("sync delta: saved frames failed at {e}"),
                }
            }
            (Err(e), _) | (_, Err(e)) => format!("sync delta failed: {e}"),
        };
        println!("{msg}");
        gfx.window.set_title(&format!("Neisor — {msg}"));
        self.title_timer = -4.0; // numbers are the point — let them linger
    }

    /// Commit a destination chosen in the photo map: position, then the
    /// photo's view if it carried one, then (opt-in) its time of day.
    fn apply_teleport(&mut self, act: triangulum_viewer::ui::TeleportAction) {
        // NaN passes an `abs() > 90` check (all NaN comparisons are false)
        // and would poison the camera — require finite, in-range values
        if !act.lat.is_finite() || !act.lon.is_finite() || act.lat.abs() > 90.0 {
            return;
        }
        self.player.teleport(
            &self.planet,
            &self.edits,
            &mut self.camera,
            act.lat,
            act.lon,
            act.alt_km.filter(|a| a.is_finite()),
            self.args.exaggeration,
        );
        // exact photo restore: put the camera back on the PHOTOGRAPHED
        // ground (a cave floor, an ice sheet — not the generic far-terrain
        // surface the teleport derives) and back in the photographed mode.
        // Seed-gated: a recorded ground height on a different world would
        // embed the camera in rock. Walk physics then re-settles feet on
        // the real support, so a stale-but-same-seed height self-corrects.
        if act.seed == Some(self.planet.seed)
            && let Some(g) = act.ground_km.filter(|v| v.is_finite())
        {
            self.camera.ground_km = g;
            if let Some(a) = act.alt_km.filter(|v| v.is_finite()) {
                self.camera.altitude_km = a;
            }
            if act.walk {
                self.player.set_walk(&mut self.camera);
            }
        }
        if let Some(y) = act.yaw_deg.filter(|v| v.is_finite()) {
            self.camera.yaw = y.to_radians();
        }
        if let Some(p) = act.pitch_deg.filter(|v| v.is_finite()) {
            self.camera.pitch = p.to_radians().clamp(-1.50, 1.50);
        }
        if let (Some(t), Some(gfx)) = (act.day_time_s.filter(|v| v.is_finite()), self.gfx.as_mut())
        {
            // the recorded seconds are a PHASE of the recorded day length;
            // replay that phase under the current cycle
            let t = match act.day_len_s.filter(|v| *v > 0.0) {
                Some(len) if self.args.day_len > 0.0 => {
                    t.rem_euclid(len) / len * self.args.day_len
                }
                _ => t,
            };
            gfx.renderer.set_day_time_s(t);
        }
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
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.renderer.advance_render_time_s(dt);
        }
        // an open photo map owns the keyboard: no movement input
        let input = if self.photo_map.open {
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
        if self.title_timer > 0.5 && !self.photo_map.open {
            self.title_timer = 0.0;
            if let Some(gfx) = &self.gfx {
                let mode = match self.player.mode {
                    Mode::Fly => "fly (click captures mouse, G walk, T teleport, P shot, V sync)",
                    Mode::Walk => "walk (F fly, space jump, T teleport, P shot, V sync)",
                };
                // objective framerate, not "feels smooth": avg cadence
                // (vsync-locked 60 Hz reads 16.7), p95 where hitches live
                let perf = match gfx.renderer.frame_stats() {
                    Some((avg, p95, _)) => {
                        format!(" | {:.0} fps ({avg:.1} ms, p95 {p95:.1})", 1000.0 / avg)
                    }
                    None => String::new(),
                };
                gfx.window.set_title(&format!(
                    "Neisor [{}] — {} | lat {:.3} lon {:.3} alt {:.3} km{}",
                    option_env!("TRI_BUILD").unwrap_or("unstamped"),
                    mode,
                    self.camera.lat.to_degrees(),
                    self.camera.lon.to_degrees(),
                    self.camera.altitude_km,
                    perf
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
            if self.mouse_locked && !self.photo_map.open {
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
        renderer.voxels_on = self.args.voxels;
        renderer.set_torches(self.torches.clone());
        renderer.refresh_world_snapshot(&self.edits);
        apply_weather(&mut renderer, &self.args.weather);
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        ));
        self.egui_paint =
            Some(triangulum_viewer::ui::EguiPaint::new(&renderer.device, format));
        self.gfx = Some(Gfx { window, surface, config, renderer });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.gfx.is_none() {
            return;
        }
        if matches!(&event, WindowEvent::Focused(false)) {
            self.keys.clear();
        }
        // an open photo map owns the pointer and keyboard: events go to egui,
        // not the game (Esc closes the popup; frame/window events pass on)
        if self.photo_map.open
            && !matches!(
                event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            if let WindowEvent::KeyboardInput { event: ke, .. } = &event
                && ke.state == ElementState::Pressed
                && ke.logical_key
                    == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
            {
                self.photo_map.open = false;
                self.title_timer = 1.0;
                return;
            }
            if let (Some(st), Some(gfx)) = (self.egui_state.as_mut(), self.gfx.as_ref()) {
                let _ = st.on_window_event(&gfx.window, &event);
                gfx.window.request_redraw();
            }
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{KeyCode as K, PhysicalKey};
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
                            if !event.repeat {
                                match code {
                                    K::KeyG => self.player.set_walk(&mut self.camera),
                                    K::KeyF => self.player.set_fly(&mut self.camera),
                                    K::Space => self.player.jump(),
                                    K::KeyQ => self.edit_block(-1),
                                    K::KeyE => self.edit_block(1),
                                    K::KeyR => self.toggle_torch(),
                                    K::KeyT => {
                                        // the photo map owns input while open, so
                                        // release the pointer and stop movement
                                        self.set_mouse_lock(false);
                                        self.keys.clear();
                                        self.photo_map.toggle();
                                    }
                                    K::KeyP => self.save_screenshot(),
                                    K::KeyV => self.save_sync_delta(),
                                    _ => {}
                                }
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
                // photo map: build this frame's UI before borrowing gfx for
                // the draw (split borrows via locals)
                let mut ui_frame = None;
                let mut action = None;
                if self.photo_map.open
                    && let (Some(st), Some(gfx)) =
                        (self.egui_state.as_mut(), self.gfx.as_ref())
                {
                    let raw = st.take_egui_input(&gfx.window);
                    let pm = &mut self.photo_map;
                    // hand the map the planet + the live weather so its layers
                    // (seasonal temp/precip, clouds-now) and the "you are here"
                    // marker read the same state the renderer is drawing.
                    let renderer = &gfx.renderer;
                    let env = triangulum_viewer::ui::MapEnv {
                        planet: &self.planet,
                        weather_field: renderer.weather_field.as_ref(),
                        weather_tuning: &renderer.weather_tuning,
                        render_time_s: renderer.render_time_s(),
                        day_len_s: renderer.day_len_s,
                        cur_lat: self.camera.lat.to_degrees(),
                        cur_lon: self.camera.lon.to_degrees(),
                    };
                    let full = self.egui_ctx.run_ui(raw, |ctx| {
                        action = pm.ui(ctx, &env);
                    });
                    st.handle_platform_output(&gfx.window, full.platform_output);
                    let prims = self
                        .egui_ctx
                        .tessellate(full.shapes, full.pixels_per_point);
                    ui_frame = Some((prims, full.textures_delta, full.pixels_per_point));
                }
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
                if let (Some((prims, deltas, ppp)), Some(paint)) =
                    (ui_frame, self.egui_paint.as_mut())
                {
                    paint.paint(
                        &gfx.renderer.device,
                        &gfx.renderer.queue,
                        &view,
                        (gfx.config.width, gfx.config.height),
                        ppp,
                        &prims,
                        &deltas,
                    );
                }
                gfx.renderer.queue.present(frame);
                gfx.window.request_redraw();
                if let Some(act) = action {
                    self.apply_teleport(act);
                }
            }
            _ => {}
        }
    }
}

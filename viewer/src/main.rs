//! Triangulum Phase-0 viewer: fly over Neisor from orbit.
//!
//!   cargo run --release                          interactive window
//!   cargo run --release -- --capture shot.png \
//!       --lat 15 --lon 40 --alt 12000            headless screenshot
//!
//! Controls: mouse = look, LMB/RMB = break/place, C = cycle body focus,
//! Q/E = freecam roll, scroll = Neisor fly altitude, Esc = release/quit.

use anyhow::Result;
use std::sync::Arc;
use triangulum_viewer::camera::{Camera, CameraMode, CameraRig};
use triangulum_viewer::planet::Planet;
use triangulum_viewer::renderer::{Renderer, SunState};
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
    day_len: Option<f64>,
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
    /// Seek the absolute game/weather/orbit clock. Photo restore may then
    /// apply its recorded daily rotation phase as a separate offset.
    weather_time: Option<f64>,
    /// --no-voxels: pure heightfield-mesh render (no chunk streaming, no
    /// hole). The eyeball twin of the sync-diff harness's `voxels off`.
    voxels: bool,
    /// Multiplayer display name and optional startup invite. These flags are
    /// accepted in every build; joining requires `--features multiplayer`.
    name: String,
    join: Option<String>,
    /// Headless network capture waits this long for a remote presence.
    multiplayer_wait_s: f64,
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
        day_len: None,
        patch: 1.0,
        auto_tilt: false,
        weather: "live".into(),
        weather_time: None,
        voxels: true,
        name: "Player".into(),
        join: None,
        multiplayer_wait_s: 8.0,
    };
    let argv: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < argv.len() {
        let next = |i: usize| argv.get(i + 1).cloned().unwrap_or_default();
        // "NaN"/"inf" parse as valid f64 and poison the camera (every
        // comparison goes false: culling dies, tile selection explodes,
        // torch sorting panics) — numeric args accept finite values only
        let numf = |i: usize, d: f64| {
            next(i)
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .unwrap_or(d)
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
                a.day_len = next(i)
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0);
                i += 1;
            }
            "--patch" => {
                a.patch = numf(i, a.patch).clamp(0.3, 2.0);
                i += 1;
            }
            "--size" => {
                if let Some((width, height)) = next(i).split_once('x')
                    && let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>())
                    && width > 0
                    && height > 0
                {
                    a.size = (width, height);
                }
                i += 1;
            }
            "--auto-tilt" => a.auto_tilt = true,
            "--no-voxels" => a.voxels = false,
            "--weather" => {
                a.weather = next(i);
                i += 1;
            }
            "--weather-time" => {
                a.weather_time = next(i)
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0);
                i += 1;
            }
            "--name" => {
                a.name = next(i);
                i += 1;
            }
            "--join" => {
                a.join = Some(next(i));
                i += 1;
            }
            "--multiplayer-wait" => {
                a.multiplayer_wait_s = numf(i, a.multiplayer_wait_s).clamp(0.5, 60.0);
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
fn apply_weather(renderer: &mut Renderer, planet: &Planet, spec: &str) {
    renderer.weather_field = planet.weather.clone();
    renderer.weather_tuning = triangulum_viewer::weather::WeatherTuning::load(&assets_dir());
    renderer.solar_tuning = triangulum_viewer::orbits::SolarTuning::load(&assets_dir());
    renderer.day_len_s = renderer.solar_tuning.day_length_s;
    match spec {
        "off" => {
            renderer.weather_on = false;
            renderer.weather_pin = None;
        }
        "" | "live" => {
            renderer.weather_on = true;
            renderer.weather_pin = None;
        }
        s => {
            if let Some((c, p)) = s.split_once(',')
                && let (Ok(c), Ok(p)) = (c.parse::<f32>(), p.parse::<f32>())
                && c.is_finite()
                && p.is_finite()
            {
                renderer.weather_on = true;
                renderer.weather_pin = Some((c.clamp(0.0, 1.0), p.clamp(0.0, 1.0)));
            } else {
                eprintln!("--weather expects off | live | COVER,PRECIP");
            }
        }
    }
}

fn main() -> Result<()> {
    let args = parse_args();
    #[cfg(not(feature = "multiplayer"))]
    if args.join.is_some() {
        anyhow::bail!("--join requires cargo build --features multiplayer");
    }
    let planet = Arc::new(Planet::load(&assets_dir())?);
    // default pitch: look at the planet from orbit, at the horizon when low
    let auto_pitch = if args.pitch > 360.0 {
        let t = (args.alt / 8000.0).clamp(0.0, 1.0);
        -12.0 - 73.0 * t
    } else {
        args.pitch.clamp(-86.0, 86.0)
    };
    let mut camera = Camera {
        body: triangulum_viewer::orbits::BodyId::Neisor,
        center_km: glam::DVec3::ZERO,
        lon: args.lon.to_radians(),
        lat: args.lat.to_radians(),
        altitude_km: args.alt,
        radius_km: planet.radius_km,
        ground_km: 0.0,
        yaw: args.yaw.to_radians(),
        pitch: auto_pitch.to_radians(),
        roll: 0.0,
    };
    camera.ground_km = triangulum_viewer::terrain::ground_height_km(
        &*planet,
        camera.position().normalize(),
        args.exaggeration,
    );

    if let Some(path) = args.capture.clone() {
        #[cfg(feature = "multiplayer")]
        if args.join.is_some() {
            return capture_multiplayer(planet, camera, args, &path);
        }
        return capture(planet, camera, args, &path);
    }

    let event_loop = EventLoop::new()?;
    let planet_seed = planet.seed;
    let mut body_edits = triangulum_viewer::voxel::BodyEdits::from_neisor(load_edits(planet_seed));
    *body_edits.for_body_mut(triangulum_viewer::orbits::BodyId::Moon) =
        load_moon_edits(planet_seed);
    let mut photo_map = triangulum_viewer::ui::PhotoMap::new(interchange_dir().into());
    photo_map.set_join_defaults(args.name.clone(), args.join.clone());
    #[cfg(feature = "multiplayer")]
    let multiplayer = MultiplayerState::new(
        triangulum_multiplayer::load_world_identity(
            std::path::Path::new(&assets_dir()),
            option_env!("TRI_BUILD").unwrap_or("unstamped"),
        )?,
    );
    let mut app = App {
        planet,
        camera,
        args,
        gfx: None,
        dragging: false,
        last_cursor: (0.0, 0.0),
        keys: Default::default(),
        player: PlayerState::default(),
        camera_rig: CameraRig::default(),
        last_frame: std::time::Instant::now(),
        mouse_locked: false,
        photo_map,
        egui_ctx: egui::Context::default(),
        egui_state: None,
        egui_paint: None,
        title_timer: 0.0,
        edits: body_edits,
        torches: load_torches(planet_seed),
        moon_body: None,
        #[cfg(feature = "multiplayer")]
        multiplayer,
    };
    #[cfg(feature = "multiplayer")]
    if let Some(invite) = app.args.join.clone() {
        app.begin_join(invite, app.args.name.clone());
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}

// ---------------------------------------------------------------- capture

fn capture(planet: Arc<Planet>, camera: Camera, args: Args, path: &str) -> Result<()> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let (device, queue) = pollster::block_on(
        adapter.request_device(&triangulum_viewer::renderer::viewer_device_descriptor(&adapter)),
    )?;
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
    renderer.sun_ref_lon = args.lon.to_radians();
    renderer.patch_scale = args.patch;
    renderer.voxels_on = args.voxels;
    // headless shots see the same edited world the game saves
    let edits = load_edits(planet.seed);
    renderer.set_torches(load_torches(planet.seed));
    renderer.refresh_world_snapshot(&edits);
    apply_weather(&mut renderer, &planet, &args.weather);
    if let Some(day_len_s) = args.day_len {
        renderer.day_len_s = day_len_s;
    }
    if let Some(t_s) = args.weather_time {
        renderer.set_weather_time_s(t_s);
    }
    let eye_km = camera.ground_km + camera.altitude_km;
    let seasonal_planet = triangulum_viewer::voxel::SeasonalPlanet::new(
        Arc::clone(&planet),
        renderer.structural_season(&planet),
    );
    renderer.underwater = triangulum_viewer::voxel::water_surface_km(
        &seasonal_planet,
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

#[cfg(feature = "multiplayer")]
fn capture_multiplayer(planet: Arc<Planet>, camera: Camera, args: Args, path: &str) -> Result<()> {
    let invite = args.join.clone().expect("multiplayer capture requires --join");
    let identity = triangulum_multiplayer::load_world_identity(
        std::path::Path::new(&assets_dir()),
        option_env!("TRI_BUILD").unwrap_or("unstamped"),
    )?;
    let client = triangulum_viewer::net::NetworkClient::spawn();
    client.connect(invite, args.name.clone(), identity.clone()).map_err(anyhow::Error::msg)?;
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs_f64(args.multiplayer_wait_s);
    let mut last_presence = started - std::time::Duration::from_secs(1);
    let mut remote = triangulum_viewer::net::RemotePlayers::default();
    let mut shared_edits = triangulum_viewer::voxel::BodyEdits::default();
    let mut clock_slew: Option<(triangulum_multiplayer::ClockSlew, std::time::Instant, f64)> = None;
    let mut connected = false;
    let mut last_edit_sequence = 0u64;
    loop {
        while let Some(event) = client.try_recv() {
            use triangulum_viewer::net::ClientEvent;
            match event {
                ClientEvent::Connecting(_) => {}
                ClientEvent::Refused { code, message } => {
                    anyhow::bail!("MULTIPLAYER REFUSED [{code}]: {message}");
                }
                ClientEvent::Disconnected(reason) => anyhow::bail!("multiplayer capture disconnected: {reason}"),
                ClientEvent::Message(message) => match message {
                    triangulum_multiplayer::Message::Welcome(welcome) => {
                        if welcome.protocol_version != triangulum_multiplayer::PROTOCOL_VERSION {
                            anyhow::bail!(
                                "WELCOME protocol mismatch: client={} server={}",
                                triangulum_multiplayer::PROTOCOL_VERSION,
                                welcome.protocol_version,
                            );
                        }
                        if let Some(mismatch) = welcome.identity.mismatch(&identity) {
                            anyhow::bail!("WELCOME identity mismatch: {mismatch}");
                        }
                        let mut expected = 1u64;
                        for record in welcome.edit_journal {
                            if record.sequence != expected { anyhow::bail!("journal sequence gap in welcome"); }
                            let body = viewer_body(record.edit.body);
                            shared_edits.for_body_mut(body).insert(
                                (record.edit.face, record.edit.ci, record.edit.cj),
                                record.edit.value,
                            );
                            last_edit_sequence = record.sequence;
                            expected += 1;
                        }
                        remote.reset(welcome.player_id, welcome.players);
                        clock_slew = Some((
                            triangulum_multiplayer::ClockSlew::new(0.0, &welcome.clock, 2.0),
                            std::time::Instant::now(),
                            welcome.clock.time_scale,
                        ));
                        connected = true;
                    }
                    triangulum_multiplayer::Message::PlayerJoined(player) => remote.join(player),
                    triangulum_multiplayer::Message::PlayerLeft { player_id } => remote.leave(player_id),
                    triangulum_multiplayer::Message::Presence { player_id, pose } => remote.presence(player_id, pose),
                    triangulum_multiplayer::Message::Edit(record) => {
                        let expected = last_edit_sequence.saturating_add(1);
                        if record.sequence != expected {
                            anyhow::bail!(
                                "live journal sequence gap: expected {expected}, received {}",
                                record.sequence,
                            );
                        }
                        let body = viewer_body(record.edit.body);
                        shared_edits.for_body_mut(body).insert(
                            (record.edit.face, record.edit.ci, record.edit.cj), record.edit.value,
                        );
                        last_edit_sequence = record.sequence;
                    }
                    triangulum_multiplayer::Message::ClockEvent(event) => {
                        let local = clock_slew.as_ref().map_or(0.0, |(slew, at, _)| slew.sample(at.elapsed().as_secs_f64()));
                        clock_slew = Some((
                            triangulum_multiplayer::ClockSlew::new(local, &event.state, 2.0),
                            std::time::Instant::now(), event.state.time_scale,
                        ));
                    }
                    triangulum_multiplayer::Message::Error { code, message } => eprintln!("SERVER ERROR [{code}]: {message}"),
                    _ => {}
                },
            }
        }
        if connected && last_presence.elapsed() >= std::time::Duration::from_millis(67) {
            last_presence = std::time::Instant::now();
            let pose = triangulum_multiplayer::BodyPose {
                body: protocol_body(camera.body),
                lat_deg: camera.lat.to_degrees(),
                lon_deg: camera.lon.to_degrees(),
                alt_km: camera.ground_km + camera.altitude_km,
                yaw_deg: camera.yaw.to_degrees(),
                pitch_deg: camera.pitch.to_degrees(),
                roll_deg: camera.roll.to_degrees(),
                mode: triangulum_multiplayer::PlayerMode::Fly,
            };
            let _ = client.send(triangulum_multiplayer::Message::PresenceUpdate(pose));
        }
        let has_remote_pose = remote.iter().any(|(_, player)| player.sample(std::time::Instant::now()).is_some());
        let slew_ready = clock_slew.as_ref().is_some_and(|(_, at, _)| at.elapsed() >= std::time::Duration::from_secs(2));
        if connected && has_remote_pose && slew_ready { break; }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for a second player's presence for multiplayer capture");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))?;
    let (device, queue) = pollster::block_on(
        adapter.request_device(&triangulum_viewer::renderer::viewer_device_descriptor(&adapter)),
    )?;
    let mut renderer = Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb, args.size, args.exaggeration);
    renderer.sun_dir = args.sun.map(|(la, lo)| {
        let (la, lo) = (la.to_radians(), lo.to_radians());
        glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin())
    });
    renderer.sun_ref_lon = args.lon.to_radians();
    renderer.patch_scale = args.patch;
    renderer.voxels_on = args.voxels;
    renderer.refresh_world_snapshot(shared_edits.for_body(camera.body));
    apply_weather(&mut renderer, &planet, &args.weather);
    if let Some(day_len_s) = args.day_len { renderer.day_len_s = day_len_s; }
    if let Some((slew, at, scale)) = &clock_slew {
        renderer.set_time_scale(*scale);
        renderer.set_weather_time_s(slew.sample(at.elapsed().as_secs_f64()));
    }
    let now = std::time::Instant::now();
    renderer.set_remote_avatars(remote.iter().filter_map(|(_, player)| {
        let pose = player.sample(now)?;
        Some(triangulum_viewer::renderer::RemoteAvatar {
            name: player.info.name.clone(), body: viewer_body(pose.body),
            lat_deg: pose.lat_deg, lon_deg: pose.lon_deg, alt_km: pose.alt_km,
            yaw_deg: pose.yaw_deg, tint: player.info.tint,
        })
    }).collect());
    let edits = shared_edits.for_body(camera.body);
    let (n, sun, sun_pinned, day_len_s) =
        capture_with_recorded_sun(&mut renderer, &planet, &camera, edits, path)?;
    write_shot_sidecar(path, &planet, &camera, &args, "multiplayer", sun, sun_pinned, day_len_s, &renderer)?;
    println!("captured multiplayer {path} ({n} tiles, remote avatar + name label visible)");
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
    let sun = renderer.sun_state(camera.position(), planet.radius_km);
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
    let weather_t_s = renderer.weather_time_s();
    let solar = renderer.solar_state(camera.position(), planet.radius_km);
    let solar_occlusion = triangulum_viewer::orbits::solar_occlusion_at(
        camera.position(),
        solar,
        &renderer.solar_tuning,
        planet.radius_km,
    );
    let lunar_shadow = triangulum_viewer::orbits::lunar_shadow_fraction(
        solar,
        &renderer.solar_tuning,
        planet.radius_km,
    );
    let moon_radius = renderer
        .solar_tuning
        .radius_km(triangulum_viewer::orbits::BodyId::Moon, planet.radius_km);
    let moon_relative = camera.position() - solar.moon_km;
    let moon_alt_nominal = (moon_relative.length() - moon_radius).abs();
    // Attribution follows the camera's BOUND BODY: altitude_km is already
    // body-relative (P3), so the old comparison tested the moon's altitude
    // against itself and lunar shots filed themselves under Neisor
    // (Andrew's misplaced-photos report). Freecam flybys within 50 km of
    // the lunar surface still count as moon shots.
    let on_moon = camera.body == triangulum_viewer::orbits::BodyId::Moon
        || moon_alt_nominal < 50.0;
    let body_direction = if on_moon {
        moon_relative.normalize_or_zero()
    } else {
        camera.position().normalize_or_zero()
    };
    let body_lat = body_direction.z.clamp(-1.0, 1.0).asin().to_degrees();
    let body_lon = body_direction.y.atan2(body_direction.x).to_degrees();
    let body_alt = if on_moon {
        let relief = triangulum_viewer::moon::MoonGenerator::new(planet.seed)
            .height_km(body_direction, moon_radius);
        moon_relative.length() - moon_radius - relief
    } else {
        camera.altitude_km
    };
    let (body_yaw, body_pitch, body_roll) = if on_moon {
        let east0 = glam::DVec3::Z.cross(body_direction);
        let east = if east0.length_squared() > 0.5 {
            east0.normalize()
        } else {
            glam::DVec3::X
        };
        let north = body_direction.cross(east).normalize();
        let (look, view_up, _) = camera.view_basis();
        let pitch = look.dot(body_direction).clamp(-1.0, 1.0).asin();
        let horizontal = (look - body_direction * look.dot(body_direction)).normalize_or_zero();
        let yaw = horizontal.dot(east).atan2(horizontal.dot(north));
        let mut base_right = look.cross(body_direction).normalize_or_zero();
        if base_right.length_squared() < 0.5 {
            base_right = east;
        }
        let base_up = base_right.cross(look).normalize();
        let target_up = (view_up - look * view_up.dot(look)).normalize_or_zero();
        let roll = if target_up.length_squared() > 0.5 {
            target_up.dot(base_right).atan2(target_up.dot(base_up))
        } else {
            0.0
        };
        (yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees())
    } else {
        (
            camera.yaw.to_degrees(),
            camera.pitch.to_degrees(),
            camera.roll.to_degrees(),
        )
    };
    let weather_js = serde_json::json!({
        "on": renderer.weather_on,
        "pinned": renderer.weather_pin.map(|(c, p)| vec![c, p]),
        "t_s": weather_t_s,
        "season_frac": triangulum_viewer::weather::season_frac(
            weather_t_s,
            renderer.effective_day_len_s(),
            &renderer.solar_tuning,
        ),
        "cloud_cover": wx.cloud_cover,
        "precip": wx.precip,
        "snow_frac": wx.snow_frac,
        "humidity": wx.humidity,
        "temp_c": wx.temp_c,
    });
    let js = serde_json::json!({
        "lat_deg": camera.lat.to_degrees(),
        "lon_deg": camera.lon.to_degrees(),
        "alt_km": camera.altitude_km,
        "body": if on_moon { "moon" } else { "neisor" },
        "body_lat_deg": body_lat,
        "body_lon_deg": body_lon,
        "body_alt_km": body_alt,
        "body_yaw_deg": body_yaw,
        "body_pitch_deg": body_pitch,
        "body_roll_deg": body_roll,
        // absolute ground height under the camera: alt_km alone can't
        // reproduce a shot (photo-map restore otherwise re-derives ground
        // from the far terrain surface — wrong in caves and on ice)
        "ground_km": camera.ground_km,
        "yaw_deg": camera.yaw.to_degrees(),
        "pitch_deg": camera.pitch.to_degrees(),
        "roll_deg": camera.roll.to_degrees(),
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
        "solar": {
            "t_s": weather_t_s,
            "season_frac": solar.season_frac,
            "sun_km": [solar.sun_km.x, solar.sun_km.y, solar.sun_km.z],
            "moon_km": [solar.moon_km.x, solar.moon_km.y, solar.moon_km.z],
            "solar_occlusion": solar_occlusion,
            "lunar_shadow": lunar_shadow,
        },
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

#[cfg(feature = "multiplayer")]
struct MultiplayerState {
    client: triangulum_viewer::net::NetworkClient,
    identity: triangulum_multiplayer::WorldIdentity,
    status: String,
    connected: bool,
    remote_players: triangulum_viewer::net::RemotePlayers,
    offline_edits: Option<triangulum_viewer::voxel::BodyEdits>,
    offline_clock: Option<(f64, f64)>,
    clock_slew: Option<(triangulum_multiplayer::ClockSlew, std::time::Instant)>,
    last_presence: std::time::Instant,
    last_edit_sequence: u64,
}

#[cfg(feature = "multiplayer")]
impl MultiplayerState {
    fn new(identity: triangulum_multiplayer::WorldIdentity) -> Self {
        Self {
            client: triangulum_viewer::net::NetworkClient::spawn(),
            identity,
            status: "Not connected".into(),
            connected: false,
            remote_players: Default::default(),
            offline_edits: None,
            offline_clock: None,
            clock_slew: None,
            last_presence: std::time::Instant::now() - std::time::Duration::from_secs(1),
            last_edit_sequence: 0,
        }
    }
}

#[cfg(feature = "multiplayer")]
fn protocol_body(body: triangulum_viewer::orbits::BodyId) -> triangulum_multiplayer::BodyId {
    match body {
        triangulum_viewer::orbits::BodyId::Neisor => triangulum_multiplayer::BodyId::Neisor,
        triangulum_viewer::orbits::BodyId::Moon => triangulum_multiplayer::BodyId::Moon,
        triangulum_viewer::orbits::BodyId::Sun => triangulum_multiplayer::BodyId::Sun,
    }
}

#[cfg(feature = "multiplayer")]
fn viewer_body(body: triangulum_multiplayer::BodyId) -> triangulum_viewer::orbits::BodyId {
    match body {
        triangulum_multiplayer::BodyId::Neisor => triangulum_viewer::orbits::BodyId::Neisor,
        triangulum_multiplayer::BodyId::Moon => triangulum_viewer::orbits::BodyId::Moon,
        triangulum_multiplayer::BodyId::Sun => triangulum_viewer::orbits::BodyId::Sun,
    }
}

struct App {
    planet: Arc<Planet>,
    camera: Camera,
    args: Args,
    gfx: Option<Gfx>,
    dragging: bool,
    last_cursor: (f64, f64),
    keys: std::collections::HashSet<winit::keyboard::KeyCode>,
    player: PlayerState,
    camera_rig: CameraRig,
    last_frame: std::time::Instant,
    mouse_locked: bool, // pointer captured: raw-motion look, cursor hidden
    /// T opens the photo-map popup (ui.rs): teleport by map, photo, or
    /// typed coordinates; browse/delete the screenshot roll.
    photo_map: triangulum_viewer::ui::PhotoMap,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_paint: Option<triangulum_viewer::ui::EguiPaint>,
    title_timer: f64,
    edits: triangulum_viewer::voxel::BodyEdits,
    torches: triangulum_viewer::voxel::Torches,
    moon_body: Option<triangulum_viewer::voxel::LunarBody>,
    #[cfg(feature = "multiplayer")]
    multiplayer: MultiplayerState,
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

fn moon_edits_path(seed: i64) -> String {
    format!(
        "{}/edits_moon_lattice{}_seed{}.bin",
        assets_dir(),
        triangulum_viewer::voxel::LUNAR_COLUMNS_PER_FACE,
        seed
    )
}

fn legacy_moon_edits_path(seed: i64) -> String {
    format!("{}/edits_moon_seed{}.bin", assets_dir(), seed)
}

/// Player-placed torches persist here, keyed by planet seed.
fn torches_path(seed: i64) -> String {
    format!("{}/torches_seed{}.bin", assets_dir(), seed)
}

/// A structurally valid save file can still carry a corrupt record (face
/// byte >= 6 panics `face_dir`; an off-lattice column or absurd edit delta
/// poisons meshing) — every record is range-checked on load, bad ones are
/// dropped with a warning instead of taking the session down.
fn valid_column(face: u8, ci: u64, cj: u64, n: u64) -> bool {
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
    let Ok(raw) = std::fs::read(torches_path(seed)) else {
        return out;
    };
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
        if !valid_column(face, ci, cj, triangulum_viewer::voxel::COLUMNS_PER_FACE) {
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
    load_edits_path(
        &edits_path(seed),
        triangulum_viewer::voxel::COLUMNS_PER_FACE,
    )
}

fn load_moon_edits(seed: i64) -> triangulum_viewer::voxel::Edits {
    let path = moon_edits_path(seed);
    if !std::path::Path::new(&path).exists()
        && std::path::Path::new(&legacy_moon_edits_path(seed)).exists()
    {
        eprintln!(
            "MOON EDIT RESET: ignoring legacy {}-column lattice edits; new lunar lattice has {} columns/face (Neisor edits are untouched)",
            triangulum_viewer::voxel::COLUMNS_PER_FACE,
            triangulum_viewer::voxel::LUNAR_COLUMNS_PER_FACE,
        );
    }
    load_edits_path(&path, triangulum_viewer::voxel::LUNAR_COLUMNS_PER_FACE)
}

fn load_edits_path(path: &str, columns_per_face: u64) -> triangulum_viewer::voxel::Edits {
    let mut out = triangulum_viewer::voxel::Edits::default();
    let Ok(raw) = std::fs::read(path) else {
        return out;
    };
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
        if !valid_column(face, ci, cj, columns_per_face) || dh.abs() > MAX_EDIT_BLOCKS {
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

fn save_edits(seed: i64, body: triangulum_viewer::orbits::BodyId, edits: &triangulum_viewer::voxel::Edits) {
    let mut buf = Vec::with_capacity(8 + edits.len() * 25);
    buf.extend_from_slice(b"EDT1");
    buf.extend_from_slice(&(edits.len() as u32).to_le_bytes());
    for (&(face, ci, cj), &dh) in edits {
        buf.push(face);
        buf.extend_from_slice(&ci.to_le_bytes());
        buf.extend_from_slice(&cj.to_le_bytes());
        buf.extend_from_slice(&dh.to_le_bytes());
    }
    let path = if body == triangulum_viewer::orbits::BodyId::Moon {
        moon_edits_path(seed)
    } else {
        edits_path(seed)
    };
    if let Err(e) = write_atomic(&path, &buf) {
        eprintln!("could not save edits: {e}");
    }
}

impl App {
    #[cfg(feature = "multiplayer")]
    fn begin_join(&mut self, invite: String, name: String) {
        if invite.trim().is_empty() {
            self.multiplayer.status = "Join URL is empty".into();
            return;
        }
        if self.multiplayer.connected {
            self.multiplayer.client.disconnect();
            self.restore_offline_world();
        }
        let name = triangulum_multiplayer::clean_player_name(&name);
        match self.multiplayer.client.connect(
            invite.clone(),
            name,
            self.multiplayer.identity.clone(),
        ) {
            Ok(()) => self.multiplayer.status = format!("Connecting to {invite} ..."),
            Err(error) => self.multiplayer.status = format!("Could not start join: {error}"),
        }
    }

    #[cfg(feature = "multiplayer")]
    fn restore_offline_world(&mut self) {
        if let Some(edits) = self.multiplayer.offline_edits.take() {
            self.edits = edits;
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.replace_world_snapshot(self.edits.for_body(self.camera.body));
            }
        }
        if let Some((absolute_time_s, time_scale)) = self.multiplayer.offline_clock.take()
            && let Some(gfx) = self.gfx.as_mut()
        {
            gfx.renderer.set_time_scale(time_scale);
            gfx.renderer.set_weather_time_s(absolute_time_s);
        }
        self.multiplayer.connected = false;
        self.multiplayer.remote_players.clear();
        self.multiplayer.clock_slew = None;
        self.multiplayer.last_edit_sequence = 0;
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.renderer.set_remote_avatars(Vec::new());
        }
    }

    #[cfg(feature = "multiplayer")]
    fn apply_network_edit(&mut self, record: triangulum_multiplayer::EditRecord) -> Result<(), String> {
        let expected = self.multiplayer.last_edit_sequence.saturating_add(1);
        if record.sequence != expected {
            return Err(format!("edit sequence mismatch: expected {expected}, received {}", record.sequence));
        }
        record.edit.validate()?;
        let body = viewer_body(record.edit.body);
        self.edits.for_body_mut(body).insert(
            (record.edit.face, record.edit.ci, record.edit.cj),
            record.edit.value,
        );
        if body == self.camera.body {
            let active = self.edits.for_body(body);
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.refresh_edits_snapshot(active);
                let dirty = triangulum_viewer::voxel::chunks_touching_column_body(
                    body, record.edit.face, record.edit.ci, record.edit.cj,
                );
                gfx.renderer.invalidate_chunks(&dirty);
            }
        }
        self.multiplayer.last_edit_sequence = record.sequence;
        Ok(())
    }

    #[cfg(feature = "multiplayer")]
    fn install_welcome(&mut self, welcome: triangulum_multiplayer::Welcome) -> Result<(), String> {
        if welcome.protocol_version != triangulum_multiplayer::PROTOCOL_VERSION {
            return Err(format!(
                "WELCOME protocol mismatch: client={} server={}",
                triangulum_multiplayer::PROTOCOL_VERSION,
                welcome.protocol_version
            ));
        }
        if let Some(mismatch) = welcome.identity.mismatch(&self.multiplayer.identity) {
            return Err(format!("WELCOME identity mismatch: {mismatch}"));
        }
        if self.multiplayer.offline_edits.is_none() {
            self.multiplayer.offline_edits = Some(self.edits.clone());
        }
        self.edits = triangulum_viewer::voxel::BodyEdits::default();
        self.multiplayer.last_edit_sequence = 0;
        for record in welcome.edit_journal {
            self.apply_network_edit(record)?;
        }
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.renderer.replace_world_snapshot(self.edits.for_body(self.camera.body));
            let local_time = gfx.renderer.weather_time_s();
            if self.multiplayer.offline_clock.is_none() {
                self.multiplayer.offline_clock = Some((local_time, gfx.renderer.time_scale()));
            }
            gfx.renderer.set_time_scale(welcome.clock.time_scale);
            self.multiplayer.clock_slew = Some((
                triangulum_multiplayer::ClockSlew::new(local_time, &welcome.clock, 2.0),
                std::time::Instant::now(),
            ));
        }
        let peers = welcome.players.len();
        self.multiplayer.remote_players.reset(welcome.player_id, welcome.players);
        self.multiplayer.connected = true;
        self.multiplayer.status = format!(
            "Connected as player {} · {} other player{} · server controls time (D-17)",
            welcome.player_id,
            peers,
            if peers == 1 { "" } else { "s" },
        );
        Ok(())
    }

    #[cfg(feature = "multiplayer")]
    fn drain_multiplayer(&mut self) {
        let mut events = Vec::new();
        while let Some(event) = self.multiplayer.client.try_recv() { events.push(event); }
        for event in events {
            use triangulum_viewer::net::ClientEvent;
            match event {
                ClientEvent::Connecting(invite) => {
                    self.multiplayer.status = format!("Connecting to {invite} ...");
                }
                ClientEvent::Refused { code, message } => {
                    let loud = format!("MULTIPLAYER REFUSED [{code}]: {message}");
                    eprintln!("{loud}");
                    self.multiplayer.status = loud;
                    self.restore_offline_world();
                }
                ClientEvent::Disconnected(reason) => {
                    let was_refused = self.multiplayer.status.starts_with("MULTIPLAYER REFUSED");
                    self.restore_offline_world();
                    if !was_refused {
                        self.multiplayer.status = format!("Disconnected: {reason}");
                        eprintln!("MULTIPLAYER DISCONNECTED: {reason}");
                    }
                }
                ClientEvent::Message(message) => {
                    use triangulum_multiplayer::Message;
                    match message {
                        Message::Welcome(welcome) => {
                            if let Err(error) = self.install_welcome(welcome) {
                                eprintln!("MULTIPLAYER REFUSED LOCALLY: {error}");
                                self.multiplayer.status = format!("MULTIPLAYER REFUSED LOCALLY: {error}");
                                self.multiplayer.client.disconnect();
                                self.restore_offline_world();
                            }
                        }
                        Message::Edit(record) => {
                            if let Err(error) = self.apply_network_edit(record) {
                                eprintln!("MULTIPLAYER DIVERGENCE: {error}");
                                self.multiplayer.status = format!("Disconnected: {error}");
                                self.multiplayer.client.disconnect();
                                self.restore_offline_world();
                            }
                        }
                        Message::Presence { player_id, pose } => {
                            self.multiplayer.remote_players.presence(player_id, pose);
                        }
                        Message::PlayerJoined(player) => {
                            self.multiplayer.status = format!("{} joined · server controls time (D-17)", player.name);
                            self.multiplayer.remote_players.join(player);
                        }
                        Message::PlayerLeft { player_id } => {
                            self.multiplayer.remote_players.leave(player_id);
                            self.multiplayer.status = format!("Player {player_id} left · server controls time (D-17)");
                        }
                        Message::ClockEvent(event) => {
                            if let Some(gfx) = self.gfx.as_mut() {
                                let local_time = gfx.renderer.weather_time_s();
                                gfx.renderer.set_time_scale(event.state.time_scale);
                                self.multiplayer.clock_slew = Some((
                                    triangulum_multiplayer::ClockSlew::new(local_time, &event.state, 2.0),
                                    std::time::Instant::now(),
                                ));
                            }
                            self.multiplayer.status = format!("Authoritative clock event {:?} · server controls time (D-17)", event.kind);
                        }
                        Message::Error { code, message } => {
                            eprintln!("MULTIPLAYER SERVER ERROR [{code}]: {message}");
                            self.multiplayer.status = format!("Server: {message}");
                        }
                        Message::Pong { .. } | Message::Ping { .. } => {}
                        Message::Hello(_) | Message::Refusal(_) | Message::EditRequest(_)
                        | Message::PresenceUpdate(_) | Message::ClockCommand(_) => {}
                    }
                }
            }
        }
    }

    #[cfg(feature = "multiplayer")]
    fn update_multiplayer(&mut self) {
        self.drain_multiplayer();
        if let Some((slew, started)) = &self.multiplayer.clock_slew
            && let Some(gfx) = self.gfx.as_mut()
        {
            let elapsed = started.elapsed().as_secs_f64();
            gfx.renderer.set_weather_time_s(slew.sample(elapsed));
            if slew.complete(elapsed) { self.multiplayer.clock_slew = None; }
        }
        if self.multiplayer.connected
            && self.multiplayer.last_presence.elapsed() >= std::time::Duration::from_millis(67)
        {
            self.multiplayer.last_presence = std::time::Instant::now();
            if self.camera.body == triangulum_viewer::orbits::BodyId::Sun {
                return;
            }
            let mode = match self.player.mode {
                Mode::Fly => triangulum_multiplayer::PlayerMode::Fly,
                Mode::Walk => triangulum_multiplayer::PlayerMode::Walk,
            };
            let pose = triangulum_multiplayer::BodyPose {
                body: protocol_body(self.camera.body),
                lat_deg: self.camera.lat.to_degrees(),
                lon_deg: self.camera.lon.to_degrees(),
                alt_km: self.camera.ground_km + self.camera.altitude_km,
                yaw_deg: self.camera.yaw.to_degrees(),
                pitch_deg: self.camera.pitch.to_degrees(),
                roll_deg: self.camera.roll.to_degrees(),
                mode,
            };
            let _ = self.multiplayer.client.send(triangulum_multiplayer::Message::PresenceUpdate(pose));
        }
    }

    #[cfg(feature = "multiplayer")]
    fn refresh_remote_avatars(&mut self) {
        let now = std::time::Instant::now();
        let avatars = if self.multiplayer.connected {
            self.multiplayer.remote_players.iter().filter_map(|(_, remote)| {
                let pose = remote.sample(now)?;
                Some(triangulum_viewer::renderer::RemoteAvatar {
                    name: remote.info.name.clone(),
                    body: viewer_body(pose.body),
                    lat_deg: pose.lat_deg,
                    lon_deg: pose.lon_deg,
                    alt_km: pose.alt_km,
                    yaw_deg: pose.yaw_deg,
                    tint: remote.info.tint,
                })
            }).collect()
        } else { Vec::new() };
        if let Some(gfx) = self.gfx.as_mut() { gfx.renderer.set_remote_avatars(avatars); }
    }

    /// Break (dh = -1) or place (dh = +1) a block at the targeted column.
    /// Breaking removes the top block of the column you hit; placing is
    /// face-aware: aiming at the side of something builds on the column in
    /// front of it (the last air column the ray crossed), aiming down at a
    /// top face grows that column. Edits are per-column height deltas, so a
    /// placed block always lands on its column's top.
    fn edit_block(&mut self, dh: i64) {
        let body_id = self.camera.body;
        let seasonal_planet = triangulum_viewer::voxel::SeasonalPlanet::new(
            Arc::clone(&self.planet),
            self.gfx.as_ref().map_or_else(
                triangulum_viewer::weather::StructuralSeason::annual,
                |gfx| gfx.renderer.structural_season(&self.planet),
            ),
        );
        let body: &dyn triangulum_viewer::voxel::VoxelBody = match body_id {
            triangulum_viewer::orbits::BodyId::Neisor => &seasonal_planet,
            triangulum_viewer::orbits::BodyId::Moon => {
                let Some(body) = self.moon_body.as_ref() else { return };
                body
            }
            triangulum_viewer::orbits::BodyId::Sun => return,
        };
        let edits = self.edits.for_body_mut(body_id);
        if let Some(outcome) = triangulum_viewer::player::edit_block_detailed(
            body,
            edits,
            &self.camera,
            self.player.mode,
            dh,
            self.args.exaggeration,
        ) {
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.refresh_edits_snapshot(edits);
                gfx.renderer.invalidate_chunks(&outcome.dirty);
            }
            self.player.refresh_after_edit(
                body,
                edits,
                &self.camera,
                self.args.exaggeration,
            );
            #[cfg(feature = "multiplayer")]
            if self.multiplayer.connected {
                let (face, ci, cj) = outcome.column;
                let request = triangulum_multiplayer::EditRequest {
                    body: protocol_body(body_id),
                    face,
                    ci,
                    cj,
                    value: outcome.value,
                };
                if let Err(error) = self.multiplayer.client.send(
                    triangulum_multiplayer::Message::EditRequest(request),
                ) {
                    self.multiplayer.status = format!("Could not send edit: {error}");
                }
            } else {
                save_edits(self.planet.seed, body_id, edits);
            }
            #[cfg(not(feature = "multiplayer"))]
            save_edits(self.planet.seed, body_id, edits);
        }
    }

    /// R: toggle a torch on the walkable top of the targeted column.
    fn toggle_torch(&mut self) {
        #[cfg(feature = "multiplayer")]
        if self.multiplayer.connected {
            self.multiplayer.status =
                "Torch placement is offline-only in MP1; shared column edits remain enabled".into();
            return;
        }
        if self.camera.body != triangulum_viewer::orbits::BodyId::Neisor {
            return;
        }
        let edits = self.edits.for_body(triangulum_viewer::orbits::BodyId::Neisor);
        let seasonal_planet = triangulum_viewer::voxel::SeasonalPlanet::new(
            Arc::clone(&self.planet),
            self.gfx.as_ref().map_or_else(
                triangulum_viewer::weather::StructuralSeason::annual,
                |gfx| gfx.renderer.structural_season(&self.planet),
            ),
        );
        if let Some(dirty) = triangulum_viewer::player::toggle_torch(
            &seasonal_planet,
            edits,
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
        let edits = self.edits.for_body(self.camera.body);
        let mode = if self.player.mode == Mode::Walk {
            "walk"
        } else {
            "fly"
        };
        let msg = match capture_with_recorded_sun(
            &mut gfx.renderer,
            &self.planet,
            &self.camera,
            edits,
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
        let mode = if self.player.mode == Mode::Walk {
            "walk"
        } else {
            "fly"
        };
        let edits = self.edits.for_body(self.camera.body);
        let Some(gfx) = self.gfx.as_mut() else { return };
        let r = &mut gfx.renderer;
        let sun_pinned = r.sun_dir.is_some();
        let day_len_s = r.day_len_s;
        let sun = r.sun_state(self.camera.position(), self.planet.radius_km);
        let old_sun = r.sun_dir;
        r.sun_dir = Some(sun.dir);
        let was_on = r.voxels_on;
        r.voxels_on = true;
        let vox = r.capture_rgba(&self.planet, &self.camera, edits);
        r.voxels_on = false;
        let mesh = r.capture_rgba(&self.planet, &self.camera, edits);
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
                    heat[i] =
                        (f64::from(mesh[i]) * 0.35).max((f64::from(d) * 3.0).min(255.0)) as u8;
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
    /// photo's view if it carried one, then (opt-in) its time of day and
    /// recorded weather coordinate.
    fn apply_teleport(&mut self, act: triangulum_viewer::ui::TeleportAction) {
        // NaN passes an `abs() > 90` check (all NaN comparisons are false)
        // and would poison the camera — require finite, in-range values
        if !act.lat.is_finite() || !act.lon.is_finite() || act.lat.abs() > 90.0 {
            return;
        }
        if act.body == triangulum_viewer::ui::MapBody::Neisor {
            self.focus_camera(triangulum_viewer::orbits::BodyId::Neisor);
            self.player.teleport(
                &self.planet,
                self.edits.for_body(triangulum_viewer::orbits::BodyId::Neisor),
                &mut self.camera,
                act.lat,
                act.lon,
                act.alt_km.filter(|a| a.is_finite()),
                self.args.exaggeration,
            );
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.refresh_edits_snapshot(
                    self.edits.for_body(triangulum_viewer::orbits::BodyId::Neisor),
                );
            }
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
                self.camera.pitch = p.to_radians().clamp(
                    -triangulum_viewer::camera::MAX_PITCH_RAD,
                    triangulum_viewer::camera::MAX_PITCH_RAD,
                );
            }
            if let Some(r) = act.roll_deg.filter(|v| v.is_finite()) {
                self.camera.roll = r.to_radians().rem_euclid(std::f64::consts::TAU);
            }
        } else {
            let Some(gfx) = self.gfx.as_ref() else { return };
            let has_photo_view = act.yaw_deg.is_some() || act.pitch_deg.is_some();
            let landing = act.walk || !has_photo_view;
            let solar = gfx
                .renderer
                .solar_state(self.camera.position(), self.planet.radius_km);
            let radius = gfx.renderer.solar_tuning.radius_km(
                triangulum_viewer::orbits::BodyId::Moon,
                self.planet.radius_km,
            );
            let (lat, lon) = (act.lat.to_radians(), act.lon.to_radians());
            let direction =
                glam::DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
            let Some(moon_body) = self.moon_body.as_ref() else { return };
            let moon_edits = self.edits.for_body(triangulum_viewer::orbits::BodyId::Moon);
            let (surface, altitude) = if landing {
                (
                    triangulum_viewer::voxel::surface_height_km(
                        moon_body,
                        moon_edits,
                        direction,
                        self.args.exaggeration,
                    ),
                    triangulum_viewer::player::EYE_KM,
                )
            } else {
                (
                    triangulum_viewer::voxel::VoxelBody::ground_height_km(
                        moon_body,
                        direction,
                        self.args.exaggeration,
                    ),
                    act.alt_km
                        .filter(|v| v.is_finite() && *v > 0.0)
                        .unwrap_or(25.0)
                        .max(0.05),
                )
            };
            let position = solar.moon_km + direction * (radius + surface + altitude);
            let east0 = glam::DVec3::Z.cross(direction);
            let east = if east0.length_squared() > 0.5 {
                east0.normalize()
            } else {
                glam::DVec3::X
            };
            let north = direction.cross(east).normalize();
            let yaw = act
                .yaw_deg
                .filter(|v| v.is_finite())
                .unwrap_or(0.0)
                .to_radians();
            let pitch = act
                .pitch_deg
                .filter(|v| v.is_finite())
                .unwrap_or(if landing { -8.0 } else { -86.0 })
                .to_radians()
                .clamp(-1.50, 1.50);
            let horizontal = north * yaw.cos() + east * yaw.sin();
            let look = (horizontal * pitch.cos() + direction * pitch.sin()).normalize();
            let mut right = look.cross(direction).normalize_or_zero();
            if right.length_squared() < 0.5 {
                right = east;
            }
            let mut view_up = right.cross(look).normalize();
            if let Some(roll) = act.roll_deg.filter(|v| v.is_finite()) {
                let right = look.cross(view_up).normalize();
                let (s, c) = roll.to_radians().sin_cos();
                view_up = (view_up * c + right * s).normalize();
            }
            // Photo views FOCUS the moon: the rig tracks a focused body by
            // translating its center each frame with the body-local pose and
            // look preserved exactly, so the photographed view survives and
            // the moon no longer orbits away from under the visitor
            // (Andrew's drift report). Bare map clicks from freecam remain
            // freecam.
            let focused = landing
                || has_photo_view
                || self.camera_rig.mode != CameraMode::Freecam;
            self.camera_rig.place_near_body(
                triangulum_viewer::orbits::BodyId::Moon,
                solar,
                radius,
                position,
                look,
                view_up,
                focused,
                &mut self.camera,
            );
            if landing {
                self.player.set_walk(&mut self.camera);
                self.player.refresh_after_edit(
                    moon_body,
                    moon_edits,
                    &self.camera,
                    self.args.exaggeration,
                );
            } else {
                self.player.set_fly(&mut self.camera);
            }
            if let Some(gfx) = self.gfx.as_mut() {
                gfx.renderer.refresh_edits_snapshot(moon_edits);
            }
        }
        #[cfg(feature = "multiplayer")]
        let local_time_authority = !self.multiplayer.connected;
        #[cfg(not(feature = "multiplayer"))]
        let local_time_authority = true;
        if let (Some(on), Some(gfx)) = (act.weather_on, self.gfx.as_mut()) {
            gfx.renderer.weather_on = on;
            gfx.renderer.weather_pin = if on {
                act.weather_pin
                    .map(|(c, p)| (c.clamp(0.0, 1.0), p.clamp(0.0, 1.0)))
            } else {
                None
            };
            if local_time_authority
                && on
                && let Some(t_s) = act.weather_time_s.filter(|v| v.is_finite() && *v >= 0.0)
            {
                gfx.renderer.set_weather_time_s(t_s);
            }
        }
        if local_time_authority
            && let (Some(t), Some(gfx)) =
                (act.day_time_s.filter(|v| v.is_finite()), self.gfx.as_mut())
        {
            // Apply this AFTER the absolute weather/orbit seek: the daily
            // offset is defined relative to that absolute coordinate.
            let t = match act.day_len_s.filter(|v| *v > 0.0) {
                Some(len) if gfx.renderer.day_len_s > 0.0 => {
                    t.rem_euclid(len) / len * gfx.renderer.day_len_s
                }
                _ => t,
            };
            gfx.renderer.set_day_time_s(t);
        }
    }

    /// Capture or release the cursor for raw mouse-look.
    fn set_mouse_lock(&mut self, lock: bool) {
        let Some(gfx) = self.gfx.as_mut() else { return };
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
    /// The body a bare mode switch (F/G) applies to: stay wherever the
    /// camera is focused; only unfocused freecam (or sun focus - no
    /// walkable ground) falls back to Neisor.
    fn mode_switch_body(&self) -> triangulum_viewer::orbits::BodyId {
        match self.camera_rig.focused_body() {
            Some(body) if body != triangulum_viewer::orbits::BodyId::Sun => body,
            _ => triangulum_viewer::orbits::BodyId::Neisor,
        }
    }

    fn focus_camera(&mut self, body: triangulum_viewer::orbits::BodyId) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let solar = gfx
            .renderer
            .solar_state(self.camera.position(), self.planet.radius_km);
        let tuning = gfx.renderer.solar_tuning.clone();
        self.camera_rig.focus(
            body,
            solar,
            &tuning,
            self.planet.radius_km,
            &mut self.camera,
        );
        gfx.renderer
            .refresh_edits_snapshot(self.edits.for_body(body));
        if body != triangulum_viewer::orbits::BodyId::Neisor {
            self.player.set_fly(&mut self.camera);
        }
    }

    fn cycle_camera_focus(&mut self) {
        let Some(gfx) = self.gfx.as_mut() else { return };
        let solar = gfx
            .renderer
            .solar_state(self.camera.position(), self.planet.radius_km);
        let tuning = gfx.renderer.solar_tuning.clone();
        self.camera_rig
            .cycle(solar, &tuning, self.planet.radius_km, &mut self.camera);
        if let Some(body) = self.camera_rig.focused_body()
            && body != triangulum_viewer::orbits::BodyId::Sun
        {
            gfx.renderer
                .refresh_edits_snapshot(self.edits.for_body(body));
        }
        if self.camera_rig.mode != CameraMode::Focused(triangulum_viewer::orbits::BodyId::Neisor) {
            self.player.set_fly(&mut self.camera);
        }
    }

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
        if let Some(gfx) = &self.gfx {
            let solar = gfx
                .renderer
                .solar_state(self.camera.position(), self.planet.radius_km);
            self.camera_rig.realign(solar, &mut self.camera);
        }
        // an open photo map owns the keyboard: no movement input
        let input = if self.photo_map.open {
            triangulum_viewer::player::Input::default()
        } else {
            triangulum_viewer::player::Input {
                fwd: (self.keys.contains(&K::KeyW) as i32 - self.keys.contains(&K::KeyS) as i32)
                    as f64,
                strafe: (self.keys.contains(&K::KeyD) as i32 - self.keys.contains(&K::KeyA) as i32)
                    as f64,
                sprint: self.keys.contains(&K::ShiftLeft),
                swim_up: self.keys.contains(&K::Space),
            }
        };
        match self.camera_rig.mode {
            CameraMode::Focused(body_id @ (triangulum_viewer::orbits::BodyId::Neisor
                | triangulum_viewer::orbits::BodyId::Moon)) => {
                let gravity_mps2 = self
                    .gfx
                    .as_ref()
                    .map(|gfx| gfx.renderer.solar_tuning.surface_gravity_mps2(body_id))
                    .unwrap_or(if body_id == triangulum_viewer::orbits::BodyId::Moon {
                        1.635
                    } else {
                        9.81
                    });
                let seasonal_planet = triangulum_viewer::voxel::SeasonalPlanet::new(
                    Arc::clone(&self.planet),
                    self.gfx.as_ref().map_or_else(
                        triangulum_viewer::weather::StructuralSeason::annual,
                        |gfx| gfx.renderer.structural_season(&self.planet),
                    ),
                );
                let body: &dyn triangulum_viewer::voxel::VoxelBody = match body_id {
                    triangulum_viewer::orbits::BodyId::Neisor => &seasonal_planet,
                    triangulum_viewer::orbits::BodyId::Moon => {
                        let Some(body) = self.moon_body.as_ref() else { return };
                        body
                    }
                    _ => unreachable!(),
                };
                self.player.update(
                    body,
                    self.edits.for_body(body_id),
                    gravity_mps2,
                    &mut self.camera,
                    &input,
                    self.args.exaggeration,
                    dt,
                );
            }
            CameraMode::Focused(_) => {}
            CameraMode::Freecam => {
                let vertical = (self.keys.contains(&K::Space) as i32
                    - self.keys.contains(&K::ControlLeft) as i32)
                    as f64;
                let roll = (self.keys.contains(&K::KeyE) as i32
                    - self.keys.contains(&K::KeyQ) as i32) as f64;
                self.camera.roll =
                    (self.camera.roll + roll * dt * 1.2).rem_euclid(std::f64::consts::TAU);
                let nav_altitude = self
                    .gfx
                    .as_ref()
                    .map_or(self.camera.altitude_km.abs(), |gfx| {
                        let solar = gfx
                            .renderer
                            .solar_state(self.camera.position(), self.planet.radius_km);
                        triangulum_viewer::camera::nearest_surface_altitude_km(
                            &self.camera,
                            solar,
                            &gfx.renderer.solar_tuning,
                            self.planet.radius_km,
                        )
                    });
                let speed =
                    (nav_altitude * 0.5).clamp(0.02, 1500.0) * if input.sprint { 4.0 } else { 1.0 };
                self.camera
                    .translate_free(input.strafe, vertical, input.fwd, speed * dt);
                self.player.underwater = false;
                self.player.grounded = false;
            }
        }

        #[cfg(feature = "multiplayer")]
        self.update_multiplayer();

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
                #[cfg(feature = "multiplayer")]
                let multiplayer = if self.multiplayer.connected {
                    " | MULTIPLAYER · server controls time (D-17)"
                } else {
                    ""
                };
                #[cfg(not(feature = "multiplayer"))]
                let multiplayer = "";
                gfx.window.set_title(&format!(
                    "Neisor [{}] — {} | lat {:.3} lon {:.3} alt {:.3} km{}{}",
                    option_env!("TRI_BUILD").unwrap_or("unstamped"),
                    mode,
                    self.camera.lat.to_degrees(),
                    self.camera.lon.to_degrees(),
                    self.camera.altitude_km,
                    perf,
                    multiplayer,
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
                self.camera.pitch = (self.camera.pitch - dy * 0.0022).clamp(
                    -triangulum_viewer::camera::MAX_PITCH_RAD,
                    triangulum_viewer::camera::MAX_PITCH_RAD,
                );
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
        let (device, queue) = pollster::block_on(adapter.request_device(
            &triangulum_viewer::renderer::viewer_device_descriptor(&adapter),
        ))
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
        renderer.sun_ref_lon = self.args.lon.to_radians();
        renderer.patch_scale = self.args.patch;
        renderer.voxels_on = self.args.voxels;
        renderer.set_torches(self.torches.clone());
        renderer.refresh_world_snapshot(
            self.edits.for_body(triangulum_viewer::orbits::BodyId::Neisor),
        );
        apply_weather(&mut renderer, &self.planet, &self.args.weather);
        if let Some(day_len_s) = self.args.day_len {
            renderer.day_len_s = day_len_s;
        }
        if let Some(t_s) = self.args.weather_time {
            renderer.set_weather_time_s(t_s);
        }
        self.moon_body = Some(triangulum_viewer::voxel::LunarBody::new(
            renderer.solar_tuning.radius_km(
                triangulum_viewer::orbits::BodyId::Moon,
                self.planet.radius_km,
            ),
            Arc::new(triangulum_viewer::moon::MoonGenerator::new(self.planet.seed)),
        ));
        self.egui_state = Some(egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        ));
        self.egui_paint = Some(triangulum_viewer::ui::EguiPaint::new(
            &renderer.device,
            format,
        ));
        self.gfx = Some(Gfx {
            window,
            surface,
            config,
            renderer,
        });
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
                && ke.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
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
                                    K::KeyG => {
                                        // walk/fly IN PLACE: focus_camera
                                        // re-places the camera at the orbit
                                        // view, so calling it while already
                                        // focused yanked moonwalkers to
                                        // space (Andrew, twice). Refocus
                                        // only from freecam/sun; otherwise
                                        // just switch the mode.
                                        let body = self.mode_switch_body();
                                        if self.camera_rig.focused_body() != Some(body) {
                                            self.focus_camera(body);
                                        }
                                        self.player.set_walk(&mut self.camera);
                                    }
                                    K::KeyF => {
                                        let body = self.mode_switch_body();
                                        if self.camera_rig.focused_body() != Some(body) {
                                            self.focus_camera(body);
                                        }
                                        self.player.set_fly(&mut self.camera);
                                    }
                                    K::KeyC => self.cycle_camera_focus(),
                                    // time fast-forward ladder (Austin):
                                    // [ slower, ] faster - the ONE clock
                                    // (sun, seasons, weather, orbits)
                                    K::BracketLeft | K::BracketRight => {
                                        const LADDER: [f64; 5] =
                                            [1.0, 10.0, 60.0, 600.0, 3600.0];
                                        #[cfg(feature = "multiplayer")]
                                        let time_enabled = !self.multiplayer.connected;
                                        #[cfg(not(feature = "multiplayer"))]
                                        let time_enabled = true;
                                        if time_enabled && let Some(gfx) = self.gfx.as_mut() {
                                            let cur = gfx.renderer.time_scale();
                                            let idx = LADDER
                                                .iter()
                                                .position(|s| (s - cur).abs() < 0.5)
                                                .unwrap_or(0);
                                            let next = if code == K::BracketRight {
                                                (idx + 1).min(LADDER.len() - 1)
                                            } else {
                                                idx.saturating_sub(1)
                                            };
                                            gfx.renderer.set_time_scale(LADDER[next]);
                                        }
                                    }
                                    K::Space => self.player.jump(),
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
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                // first click captures the mouse for raw free-look (Esc
                // releases); drag-look remains as the uncaptured fallback
                if state == ElementState::Pressed && !self.mouse_locked {
                    self.set_mouse_lock(true);
                } else if state == ElementState::Pressed
                    && matches!(
                        self.camera_rig.mode,
                        CameraMode::Focused(
                            triangulum_viewer::orbits::BodyId::Neisor
                                | triangulum_viewer::orbits::BodyId::Moon
                        )
                    )
                {
                    self.edit_block(-1);
                }
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                if matches!(
                    self.camera_rig.mode,
                    CameraMode::Focused(
                        triangulum_viewer::orbits::BodyId::Neisor
                            | triangulum_viewer::orbits::BodyId::Moon
                    )
                )
                {
                    self.edit_block(1);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (dx, dy) = (
                    position.x - self.last_cursor.0,
                    position.y - self.last_cursor.1,
                );
                self.last_cursor = (position.x, position.y);
                if self.dragging && !self.mouse_locked {
                    // free look: drag turns the head, not the globe.
                    // pitch stops short of vertical: at exactly nadir the
                    // view basis degenerates and the image flips
                    self.camera.yaw += dx * 0.0032;
                    self.camera.pitch = (self.camera.pitch - dy * 0.0032).clamp(
                        -triangulum_viewer::camera::MAX_PITCH_RAD,
                        triangulum_viewer::camera::MAX_PITCH_RAD,
                    );
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.player.mode == Mode::Fly
                    && matches!(
                        self.camera_rig.mode,
                        CameraMode::Focused(
                            triangulum_viewer::orbits::BodyId::Neisor
                                | triangulum_viewer::orbits::BodyId::Moon
                        )
                    )
                {
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
                        let engage = ((alt / FREE_LOOK_ALT_KM).ln() / 8.0f64.ln()).clamp(0.0, 1.0);
                        self.camera.pitch += (target - self.camera.pitch) * 0.35 * engage;
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.update();
                #[cfg(feature = "multiplayer")]
                self.refresh_remote_avatars();
                // photo map: build this frame's UI before borrowing gfx for
                // the draw (split borrows via locals)
                let mut ui_frame = None;
                let mut action = None;
                if self.photo_map.open
                    && let (Some(st), Some(gfx)) = (self.egui_state.as_mut(), self.gfx.as_ref())
                {
                    let raw = st.take_egui_input(&gfx.window);
                    let pm = &mut self.photo_map;
                    // hand the map the planet + the live weather so its layers
                    // (seasonal temp/precip, clouds-now) and the "you are here"
                    // marker read the same state the renderer is drawing.
                    let renderer = &gfx.renderer;
                    let solar = renderer.solar_state(self.camera.position(), self.planet.radius_km);
                    let moon_radius = renderer.solar_tuning.radius_km(
                        triangulum_viewer::orbits::BodyId::Moon,
                        self.planet.radius_km,
                    );
                    let moon_relative = self.camera.position() - solar.moon_km;
                    let moon_direction = moon_relative.normalize_or_zero();
                    let cur_moon_lat = moon_direction.z.clamp(-1.0, 1.0).asin().to_degrees();
                    let cur_moon_lon = moon_direction.y.atan2(moon_direction.x).to_degrees();
                    let cur_body = match self.camera_rig.mode {
                        CameraMode::Focused(triangulum_viewer::orbits::BodyId::Moon) => {
                            triangulum_viewer::ui::MapBody::Moon
                        }
                        CameraMode::Freecam
                            if (moon_relative.length() - moon_radius).abs()
                                < self.camera.altitude_km.abs() =>
                        {
                            triangulum_viewer::ui::MapBody::Moon
                        }
                        _ => triangulum_viewer::ui::MapBody::Neisor,
                    };
                    #[cfg(feature = "multiplayer")]
                    let (multiplayer_connected, multiplayer_status) =
                        (self.multiplayer.connected, self.multiplayer.status.as_str());
                    #[cfg(not(feature = "multiplayer"))]
                    let (multiplayer_connected, multiplayer_status) =
                        (false, "Multiplayer support is not compiled into this binary");
                    let env = triangulum_viewer::ui::MapEnv {
                        planet: &self.planet,
                        weather_field: renderer.weather_field.as_ref().map(|v| &**v),
                        synoptic_raster: Some(&renderer.synoptic_raster),
                        weather_tuning: &renderer.weather_tuning,
                        solar_tuning: &renderer.solar_tuning,
                        weather_time_s: renderer.weather_time_s(),
                        day_len_s: renderer.effective_day_len_s(),
                        weather_on: renderer.weather_on,
                        weather_pin: renderer.weather_pin,
                        cur_lat: self.camera.lat.to_degrees(),
                        cur_lon: self.camera.lon.to_degrees(),
                        cur_moon_lat,
                        cur_moon_lon,
                        cur_body,
                        time_scale: renderer.time_scale(),
                        multiplayer_available: cfg!(feature = "multiplayer"),
                        multiplayer_connected,
                        multiplayer_status,
                    };
                    let full = self.egui_ctx.run_ui(raw, |ctx| {
                        action = pm.ui(ctx, &env);
                    });
                    st.handle_platform_output(&gfx.window, full.platform_output);
                    let prims = self.egui_ctx.tessellate(full.shapes, full.pixels_per_point);
                    ui_frame = Some((prims, full.textures_delta, full.pixels_per_point));
                }
                #[cfg(feature = "multiplayer")]
                if self.photo_map.pending_disconnect {
                    self.photo_map.pending_disconnect = false;
                    self.multiplayer.client.disconnect();
                    self.restore_offline_world();
                    self.multiplayer.status = "Disconnected; single-player world restored".into();
                }
                #[cfg(feature = "multiplayer")]
                if let Some((invite, name)) = self.photo_map.pending_join.take() {
                    self.begin_join(invite, name);
                }
                #[cfg(not(feature = "multiplayer"))]
                {
                    self.photo_map.pending_disconnect = false;
                    self.photo_map.pending_join = None;
                }
                #[cfg(feature = "multiplayer")]
                let local_time_authority = !self.multiplayer.connected;
                #[cfg(not(feature = "multiplayer"))]
                let local_time_authority = true;
                if local_time_authority
                    && let Some(t_s) = self.photo_map.pending_time_travel.take()
                    && let Some(gfx) = self.gfx.as_mut()
                {
                    gfx.renderer.set_weather_time_s(t_s);
                    // the focused rig re-centers on its body every frame, so
                    // a timeskip keeps the local pose per the P1 spec; the
                    // seasonal chunk buckets refresh through streaming.
                }
                if local_time_authority
                    && let Some(s) = self.photo_map.pending_time_scale.take()
                    && let Some(gfx) = self.gfx.as_mut()
                {
                    gfx.renderer.set_time_scale(s);
                }
                if !local_time_authority {
                    self.photo_map.pending_time_travel = None;
                    self.photo_map.pending_time_scale = None;
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
                let edits = self.edits.for_body(self.camera.body);
                gfx.renderer.draw(&view, &self.planet, &self.camera, edits);
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

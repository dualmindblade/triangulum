#![recursion_limit = "256"]

//! Scripted play harness: drive the game's own player physics and renderer
//! from a plain-text script, headless. The bridge between "an AI can only
//! look at stills" and "a human plays around": scripts are reproducible
//! play sessions that leave behind frames + machine-readable state.
//!
//!   cargo run --release --example play -- SCRIPT.play [--out DIR]
//!       [--exagg N] [--patch N] [--size WxH]
//!
//! Output: a run directory (default interchange/runs/<script-stem>/) with
//! NAME.png frames, NAME.json state sidecars, and run.log (the transcript).
//!
//! Script commands (one per line, `#` comments):
//!   teleport LAT LON [ALT_KM]     absolute jump (fly mode), like the T key
//!   moonpose LAT LON ALT YAW PITCH body-local focused-moon flyby/orbit pose
//!   moonland LAT LON [YAW PITCH]  teleport to lunar columns in walk mode
//!   moonprobe LAT LON              select a body-local direction for moon
//!                                 surface-height/albedo assertions
//!   look YAW PITCH                absolute view angles, degrees
//!   turn DYAW DPITCH              relative view change, degrees
//!   mode walk|fly                 like G / F
//!   hold KEYS SECONDS             movement keys held for a duration at a
//!                                 fixed 60 Hz timestep; freecam also accepts
//!                                 q/e roll and space/ctrl vertical motion
//!   tap space|lmb|rmb|r           jump / break / place / torch
//!   focus neisor|moon|sun|free    switch the camera state machine
//!   roll DEGREES                  set freecam roll numerically
//!   wait SECONDS                  time passes with no input (gravity acts)
//!   shot NAME                     render a frame (waits for streaming) +
//!                                 write NAME.json state sidecar
//!   state NAME                    state sidecar only, no frame
//!   assert FIELD OP VALUE         check a state value; any failure makes the
//!                                 run exit non-zero (self-checking scripts).
//!                                 FIELD: grounded, underwater, mode, has_water,
//!                                 alt_km, radius_km, ground_km, support_below_km,
//!                                 water_surface_km, ceiling_above_km,
//!                                 vert_vel_mps, lat_deg, lon_deg, yaw_deg,
//!                                 pitch_deg, block_width_height_ratio.
//!                                 OP: == != < <= > >= or ~ (approx,
//!                                 optional 4th token = tolerance). VALUE: a
//!                                 number, true/false, walk/fly, or `none`.
//!   sun LAT LON                   pin the sun. WARNING: this is a GLOBAL sun
//!                                 direction — far-longitude teleports then
//!                                 render at NIGHT. OMIT sun for surveys: the
//!                                 default lights every location at local noon.
//!   weather off|live              disable/enable the deterministic field
//!   weather pin COVER PRECIP      pin visible intensity (both 0..1)
//!   weather time T_S              seek absolute weather/orbital game time
//!   weather season FRAC           make the next shot use this year phase
//!   probe LAT LON                 dump sampler + column truth at a point
//!                                 into the transcript (h, water, lake/pond
//!                                 levels, river, cave water) — the census
//!                                 probe without leaving the run
//!   log TEXT...                   annotate the transcript
//!
//! Navigation is deliberately absolute-first: scripts teleport to
//! coordinates and set exact view angles; relative movement exists to
//! exercise the physics, not to find places.

use std::io::Write as IoWrite;
use std::sync::Arc;

use triangulum_viewer::camera::{Camera, CameraMode, CameraRig};
use triangulum_viewer::planet::{Planet, face_from_dir};
use triangulum_viewer::player::{self, Input, Mode, PlayerState};
use triangulum_viewer::renderer::Renderer;
use triangulum_viewer::voxel::{
    Edits, LunarBody, SeasonalPlanet, Torches, VoxelBody, ceiling_above_km,
    support_below_km, surface_height_km, water_surface_km,
};

const DT: f64 = 1.0 / 60.0; // fixed timestep: scripts are deterministic

fn focused_voxel_body<'a>(
    camera: &Camera,
    planet: &'a SeasonalPlanet,
    moon: &'a LunarBody,
) -> Option<&'a dyn VoxelBody> {
    match camera.body {
        triangulum_viewer::orbits::BodyId::Neisor => Some(planet),
        triangulum_viewer::orbits::BodyId::Moon => Some(moon),
        triangulum_viewer::orbits::BodyId::Sun => None,
    }
}

fn focused_edits<'a>(camera: &Camera, neisor: &'a Edits, moon: &'a Edits) -> &'a Edits {
    match camera.body {
        triangulum_viewer::orbits::BodyId::Moon => moon,
        _ => neisor,
    }
}

fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let script_path = argv.get(1).cloned().unwrap_or_default();
    if script_path.is_empty() || script_path.starts_with("--") {
        anyhow::bail!("usage: play SCRIPT.play [--out DIR] [--exagg N] [--patch N] [--size WxH]");
    }
    let mut exagg = 1.0f64;
    let mut patch = 1.0f64;
    let mut size = (1280u32, 720u32);
    let mut out_dir: Option<String> = None;
    let mut i = 2;
    while i < argv.len() {
        let next = |i: usize| argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--out" => {
                out_dir = Some(next(i));
                i += 1;
            }
            "--exagg" => {
                exagg = next(i).parse().unwrap_or(exagg);
                i += 1;
            }
            "--patch" => {
                patch = next(i).parse::<f64>().unwrap_or(patch).clamp(0.3, 2.0);
                i += 1;
            }
            "--size" => {
                let s = next(i);
                if let Some((w, h)) = s.split_once('x') {
                    size = (w.parse().unwrap_or(size.0), h.parse().unwrap_or(size.1));
                }
                i += 1;
            }
            other => eprintln!("unknown arg: {other}"),
        }
        i += 1;
    }

    // strip a UTF-8 BOM if present: Windows editors and PowerShell love
    // to prepend one, and it otherwise glues itself to the first command
    let script = std::fs::read_to_string(&script_path)?
        .trim_start_matches('\u{feff}')
        .to_string();
    let stem = std::path::Path::new(&script_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("run")
        .to_string();
    let interchange = if std::path::Path::new("viewer/interchange").exists() {
        "viewer/interchange"
    } else {
        "interchange"
    };
    let dir = out_dir.unwrap_or_else(|| format!("{interchange}/runs/{stem}"));
    std::fs::create_dir_all(&dir)?;
    let mut logf = std::fs::File::create(format!("{dir}/run.log"))?;
    macro_rules! trace {
        ($($t:tt)*) => {{
            let line = format!($($t)*);
            println!("{line}");
            let _ = writeln!(logf, "{line}");
        }};
    }

    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Arc::new(Planet::load(assets)?);
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
        size,
        exagg,
    );
    renderer.patch_scale = patch;
    // living weather: deterministic here BY CONSTRUCTION — render time is
    // the fixed sim clock (F-20), so even `weather live` scripts reproduce
    // byte-identical frames. weather.bin missing just means clear skies.
    renderer.weather_field = planet.weather.clone();
    renderer.weather_tuning = triangulum_viewer::weather::WeatherTuning::load(assets);
    renderer.solar_tuning = triangulum_viewer::orbits::SolarTuning::load(assets);
    renderer.day_len_s = renderer.solar_tuning.day_length_s;
    // deterministic physical day/orbit by default; `sun` remains an explicit
    // art/repro pin that rotates the complete Sun-Neisor-moon frame.

    // scripts run in a CLEAN world (no saved player edits/torches), so the
    // same script always produces the same frames on the same planet
    let mut edits = Edits::default();
    let mut moon_edits = Edits::default();
    let mut torches = Torches::default();
    let moon_body = LunarBody::new(
        renderer
            .solar_tuning
            .radius_km(triangulum_viewer::orbits::BodyId::Moon, planet.radius_km),
        Arc::new(triangulum_viewer::moon::MoonGenerator::new(planet.seed)),
    );
    let mut ps = PlayerState::default();
    let mut camera_rig = CameraRig::default();
    let mut camera = Camera {
        body: triangulum_viewer::orbits::BodyId::Neisor,
        center_km: glam::DVec3::ZERO,
        lon: 30f64.to_radians(),
        lat: 10f64.to_radians(),
        altitude_km: 100.0,
        radius_km: planet.radius_km,
        ground_km: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
    };
    ps.teleport(&planet, &edits, &mut camera, 10.0, 30.0, Some(100.0), exagg);

    let write_state = |name: &str,
                       camera: &Camera,
                       camera_rig: &CameraRig,
                       renderer: &Renderer,
                       ps: &PlayerState,
                       edits: &Edits,
                       dir_path: &str|
     -> anyhow::Result<()> {
        let d = camera.local_direction();
        let seasonal = SeasonalPlanet::new(
            Arc::clone(&planet),
            renderer.structural_season(&planet),
        );
        let body = focused_voxel_body(camera, &seasonal, &moon_body).unwrap_or(&seasonal);
        let (face, u, v) = face_from_dir(d);
        let s = triangulum_viewer::terrain::sample_at_season(
            &planet,
            face,
            u,
            v,
            triangulum_viewer::terrain::VOXEL_OCTAVES,
            seasonal.season,
        );
        let feet = camera.ground_km;
        let eye = feet + camera.altitude_km;
        let support = support_below_km(body, edits, d, feet + 1e-7, exagg);
        let ceil = ceiling_above_km(body, edits, d, feet + player::EYE_KM, exagg);
        let water = water_surface_km(body, edits, d, eye, exagg);
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
        let focus_distance = camera_rig.focus_distance_km(solar, camera);
        let focus_alignment = camera_rig.focus_alignment(solar, camera);
        let camera_pos = camera.position();
        let moon_radius = renderer
            .solar_tuning
            .radius_km(triangulum_viewer::orbits::BodyId::Moon, planet.radius_km);
        let moon_relative = camera_pos - solar.moon_km;
        let on_moon = camera.body == triangulum_viewer::orbits::BodyId::Moon;
        let body_direction = if on_moon {
            moon_relative.normalize_or_zero()
        } else {
            camera_pos.normalize_or_zero()
        };
        let body_lat = body_direction.z.clamp(-1.0, 1.0).asin().to_degrees();
        let body_lon = body_direction.y.atan2(body_direction.x).to_degrees();
        let body_alt = if on_moon {
            moon_relative.length()
                - moon_radius
                - triangulum_viewer::moon::MoonGenerator::new(planet.seed)
                    .height_km(body_direction, moon_radius)
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
            "radius_km": camera.radius_km + camera.ground_km + camera.altitude_km,
            "yaw_deg": camera.yaw.to_degrees(),
            "pitch_deg": camera.pitch.to_degrees(),
            "roll_deg": camera.roll.to_degrees(),
            "focus_id": camera_rig.numeric_focus_id(),
            "camera_x_km": camera_pos.x,
            "camera_y_km": camera_pos.y,
            "camera_z_km": camera_pos.z,
            "focus_distance_km": if focus_distance.is_finite() { Some(focus_distance) } else { None },
            "focus_alignment": if focus_alignment.is_finite() { Some(focus_alignment) } else { None },
            "solar_time_s": renderer.weather_time_s(),
            "season_frac": solar.season_frac,
            "sun_x_km": solar.sun_km.x,
            "sun_y_km": solar.sun_km.y,
            "sun_z_km": solar.sun_km.z,
            "moon_x_km": solar.moon_km.x,
            "moon_y_km": solar.moon_km.y,
            "moon_z_km": solar.moon_km.z,
            "solar_occlusion": solar_occlusion,
            "lunar_shadow": lunar_shadow,
            "mode": if ps.mode == Mode::Walk { "walk" } else { "fly" },
            "grounded": ps.grounded,
            "underwater": ps.underwater,
            "vert_vel_mps": ps.vert_vel_mps,
            "ground_km": feet,
            "support_below_km": support,
            "ceiling_above_km": if ceil.is_finite() { Some(ceil) } else { None },
            "water_surface_km": water,
                "terrain": {
                "h_km": s.h_km,
                "water_km": if s.water_km.is_finite() { Some(s.water_km) } else { None },
                "sea": s.sea,
                "lake": s.lake,
                    "temp_c": s.temp_c,
                    "seasonal_temp_c": s.seasonal_temp_c,
                    "frozen": s.frozen,
                    "season_bucket": s.structural_season.bucket,
                "precip_mm": s.precip,
                "koppen": planet.koppen(face, u, v),
                "river_dist_km": if s.river_dist_km.is_finite() { Some(s.river_dist_km) } else { None },
            },
        });
        std::fs::write(
            format!("{dir_path}/{name}.json"),
            serde_json::to_string_pretty(&js)?,
        )?;
        Ok(())
    };

    // assertions that failed: a run with any failure exits non-zero, so a
    // .play script doubles as a self-checking regression/discovery test.
    let mut assert_fails: u32 = 0;
    let mut sim_ticks: u64 = 0;
    // trailer recording: glide/hold append settled frames here, numbered
    // across the whole script so ffmpeg assembles one continuous timeline
    let mut rec_frame: u64 = 0;
    // session replay cadence: sim ticks advanced per `pose` line (posehz N)
    let mut pose_ticks: u64 = 6;
    let mut moon_probe_dir = glam::DVec3::X;
    for (ln, raw) in script.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        let f = |k: usize| -> anyhow::Result<f64> {
            toks.get(k)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| anyhow::anyhow!("line {}: bad number in `{raw}`", ln + 1))
        };
        match toks[0].to_ascii_lowercase().as_str() {
            "teleport" => {
                renderer.set_render_time_s(sim_ticks as f64 * DT);
                let solar = renderer.solar_state(camera.position(), planet.radius_km);
                let tuning = renderer.solar_tuning.clone();
                camera_rig.focus(
                    triangulum_viewer::orbits::BodyId::Neisor,
                    solar,
                    &tuning,
                    planet.radius_km,
                    &mut camera,
                );
                ps.teleport(&planet, &edits, &mut camera, f(1)?, f(2)?, f(3).ok(), exagg);
                renderer.refresh_edits_snapshot(&edits);
                trace!(
                    "[{}] teleport -> lat {:.6} lon {:.6} alt {:.5} km",
                    ln + 1,
                    f(1)?,
                    f(2)?,
                    camera.altitude_km
                );
            }
            "moonland" | "moonteleport" | "teleport-to-moon-surface" => {
                let (lat_deg, lon_deg) = (f(1)?, f(2)?);
                anyhow::ensure!(
                    lat_deg.is_finite() && lon_deg.is_finite() && lat_deg.abs() <= 90.0,
                    "line {}: moonland needs finite LAT LON",
                    ln + 1
                );
                let yaw_deg = toks.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0f64);
                let pitch_deg = toks.get(4).and_then(|s| s.parse().ok()).unwrap_or(-8.0f64);
                renderer.set_render_time_s(sim_ticks as f64 * DT);
                let solar = renderer.solar_state(camera.position(), planet.radius_km);
                let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
                let direction =
                    glam::DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
                let surface = surface_height_km(&moon_body, &moon_edits, direction, exagg);
                let position = solar.moon_km
                    + direction * (moon_body.radius_km + surface + player::EYE_KM);
                let east0 = glam::DVec3::Z.cross(direction);
                let east = if east0.length_squared() > 0.5 {
                    east0.normalize()
                } else {
                    glam::DVec3::X
                };
                let north = direction.cross(east).normalize();
                let (yaw, pitch) = (
                    yaw_deg.to_radians(),
                    pitch_deg.to_radians().clamp(-1.50, 1.50),
                );
                let horizontal = north * yaw.cos() + east * yaw.sin();
                let look = (horizontal * pitch.cos() + direction * pitch.sin()).normalize();
                let mut right = look.cross(direction).normalize_or_zero();
                if right.length_squared() < 0.5 {
                    right = east;
                }
                let view_up = right.cross(look).normalize();
                camera_rig.place_near_body(
                    triangulum_viewer::orbits::BodyId::Moon,
                    solar,
                    moon_body.radius_km,
                    position,
                    look,
                    view_up,
                    true,
                    &mut camera,
                );
                ps.set_walk(&mut camera);
                ps.refresh_after_edit(&moon_body, &moon_edits, &camera, exagg);
                renderer.refresh_edits_snapshot(&moon_edits);
                trace!(
                    "[{}] moonland -> lat {:.6} lon {:.6} surface {:.3} km grounded {}",
                    ln + 1,
                    lat_deg,
                    lon_deg,
                    surface,
                    ps.grounded
                );
            }
            "moonpose" => {
                let (lat_deg, lon_deg, altitude) = (f(1)?, f(2)?, f(3)?);
                anyhow::ensure!(
                    lat_deg.is_finite()
                        && lon_deg.is_finite()
                        && altitude.is_finite()
                        && lat_deg.abs() <= 90.0
                        && altitude > 0.0,
                    "line {}: moonpose needs finite LAT LON ALT_KM (alt > 0)",
                    ln + 1
                );
                renderer.set_render_time_s(sim_ticks as f64 * DT);
                let solar = renderer.solar_state(camera.position(), planet.radius_km);
                let radius = renderer
                    .solar_tuning
                    .radius_km(triangulum_viewer::orbits::BodyId::Moon, planet.radius_km);
                let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
                let direction =
                    glam::DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
                let height = triangulum_viewer::moon::MoonGenerator::new(planet.seed)
                    .height_km(direction, radius);
                let position = solar.moon_km + direction * (radius + height + altitude);
                let east0 = glam::DVec3::Z.cross(direction);
                let east = if east0.length_squared() > 0.5 {
                    east0.normalize()
                } else {
                    glam::DVec3::X
                };
                let north = direction.cross(east).normalize();
                let (yaw, pitch) = (f(4)?.to_radians(), f(5)?.to_radians().clamp(-1.50, 1.50));
                let horizontal = north * yaw.cos() + east * yaw.sin();
                let look = (horizontal * pitch.cos() + direction * pitch.sin()).normalize();
                let mut right = look.cross(direction).normalize_or_zero();
                if right.length_squared() < 0.5 {
                    right = east;
                }
                let view_up = right.cross(look).normalize();
                camera_rig.place_near_body(
                    triangulum_viewer::orbits::BodyId::Moon,
                    solar,
                    radius,
                    position,
                    look,
                    view_up,
                    true,
                    &mut camera,
                );
                ps.set_fly(&mut camera);
                renderer.refresh_edits_snapshot(&moon_edits);
                trace!(
                    "[{}] moonpose -> lat {:.6} lon {:.6} alt {:.5} km yaw {:.0} pitch {:.0}",
                    ln + 1,
                    lat_deg,
                    lon_deg,
                    altitude,
                    f(4)?,
                    f(5)?
                );
            }
            "moonprobe" => {
                let (lat_deg, lon_deg) = (f(1)?, f(2)?);
                anyhow::ensure!(
                    lat_deg.is_finite() && lon_deg.is_finite() && lat_deg.abs() <= 90.0,
                    "line {}: moonprobe needs finite LAT LON",
                    ln + 1
                );
                let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
                let requested =
                    glam::DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
                let (face, ci, cj) =
                    triangulum_viewer::voxel::column_id_body(&moon_body, requested);
                moon_probe_dir = triangulum_viewer::voxel::dir_of_column_body(
                    &moon_body,
                    face as usize,
                    ci,
                    cj,
                );
                trace!(
                    "[{}] moonprobe {:.6} {:.6} -> face {} column {} {}",
                    ln + 1,
                    lat_deg,
                    lon_deg,
                    face,
                    ci,
                    cj,
                );
            }
            "look" => {
                camera.yaw = f(1)?.to_radians();
                camera.pitch = f(2)?.to_radians().clamp(-1.50, 1.50);
                trace!("[{}] look yaw {:.0} pitch {:.0}", ln + 1, f(1)?, f(2)?);
            }
            "turn" => {
                camera.yaw += f(1)?.to_radians();
                camera.pitch = (camera.pitch + f(2)?.to_radians()).clamp(-1.50, 1.50);
                trace!(
                    "[{}] turn -> yaw {:.0} pitch {:.0}",
                    ln + 1,
                    camera.yaw.to_degrees(),
                    camera.pitch.to_degrees()
                );
            }
            "focus" => {
                renderer.set_render_time_s(sim_ticks as f64 * DT);
                let solar = renderer.solar_state(camera.position(), planet.radius_km);
                let tuning = renderer.solar_tuning.clone();
                match toks.get(1).copied() {
                    Some("neisor") => camera_rig.focus(
                        triangulum_viewer::orbits::BodyId::Neisor,
                        solar,
                        &tuning,
                        planet.radius_km,
                        &mut camera,
                    ),
                    Some("moon") => {
                        camera_rig.focus(
                            triangulum_viewer::orbits::BodyId::Moon,
                            solar,
                            &tuning,
                            planet.radius_km,
                            &mut camera,
                        );
                        ps.set_fly(&mut camera);
                    }
                    Some("sun") => {
                        camera_rig.focus(
                            triangulum_viewer::orbits::BodyId::Sun,
                            solar,
                            &tuning,
                            planet.radius_km,
                            &mut camera,
                        );
                        ps.set_fly(&mut camera);
                    }
                    Some("free") | Some("freecam") => {
                        camera_rig.freecam(&camera);
                        ps.set_fly(&mut camera);
                    }
                    _ => anyhow::bail!("line {}: focus neisor|moon|sun|free", ln + 1),
                }
                renderer.refresh_edits_snapshot(focused_edits(&camera, &edits, &moon_edits));
                trace!(
                    "[{}] focus {} (id {:.0})",
                    ln + 1,
                    toks[1],
                    camera_rig.numeric_focus_id()
                );
            }
            "roll" => {
                anyhow::ensure!(
                    camera_rig.mode == CameraMode::Freecam,
                    "line {}: roll is freecam-only",
                    ln + 1
                );
                camera.roll = f(1)?.to_radians().rem_euclid(std::f64::consts::TAU);
                trace!("[{}] roll {:.3} deg", ln + 1, camera.roll.to_degrees());
            }
            "mode" => {
                anyhow::ensure!(
                    matches!(
                        camera_rig.mode,
                        CameraMode::Focused(
                            triangulum_viewer::orbits::BodyId::Neisor
                                | triangulum_viewer::orbits::BodyId::Moon
                        )
                    ),
                    "line {}: walk/fly mode needs focused neisor or moon",
                    ln + 1
                );
                match toks.get(1).copied() {
                    Some("walk") => ps.set_walk(&mut camera),
                    Some("fly") => ps.set_fly(&mut camera),
                    _ => anyhow::bail!("line {}: mode walk|fly", ln + 1),
                }
                // a mode switch is a pose change; keep underwater consistent
                // for an immediate shot with no update tick in between
                let seasonal = SeasonalPlanet::new(
                    Arc::clone(&planet),
                    renderer.structural_season(&planet),
                );
                let body = focused_voxel_body(&camera, &seasonal, &moon_body).unwrap();
                let active_edits = focused_edits(&camera, &edits, &moon_edits);
                ps.refresh_underwater(body, active_edits, &camera, exagg);
                renderer.refresh_edits_snapshot(active_edits);
                trace!("[{}] mode {}", ln + 1, toks[1]);
            }
            "hold" => {
                let keys = toks
                    .get(1)
                    .ok_or_else(|| anyhow::anyhow!("line {}: hold KEYS SECONDS", ln + 1))?
                    .to_ascii_lowercase();
                let secs = f(2)?;
                let mut input = Input::default();
                let mut roll_axis = 0.0f64;
                let mut free_down = false;
                for k in keys.split('+') {
                    match k {
                        "w" => input.fwd += 1.0,
                        "s" => input.fwd -= 1.0,
                        "d" => input.strafe += 1.0,
                        "a" => input.strafe -= 1.0,
                        "shift" => input.sprint = true,
                        "space" => input.swim_up = true,
                        "ctrl" => free_down = true,
                        "q" => roll_axis -= 1.0,
                        "e" => roll_axis += 1.0,
                        other => anyhow::bail!("line {}: unknown key `{other}`", ln + 1),
                    }
                }
                let steps = (secs / DT).round().max(1.0) as usize;
                for _ in 0..steps {
                    match camera_rig.mode {
                        CameraMode::Focused(
                            triangulum_viewer::orbits::BodyId::Neisor
                            | triangulum_viewer::orbits::BodyId::Moon,
                        ) => {
                            let seasonal = SeasonalPlanet::new(
                                Arc::clone(&planet),
                                renderer.structural_season(&planet),
                            );
                            let body = focused_voxel_body(&camera, &seasonal, &moon_body).unwrap();
                            let active_edits = focused_edits(&camera, &edits, &moon_edits);
                            let gravity = renderer
                                .solar_tuning
                                .surface_gravity_mps2(camera.body);
                            ps.update(
                                body,
                                active_edits,
                                gravity,
                                &mut camera,
                                &input,
                                exagg,
                                DT,
                            );
                        }
                        CameraMode::Focused(_) => {}
                        CameraMode::Freecam => {
                            camera.roll = (camera.roll + roll_axis * DT * 1.2)
                                .rem_euclid(std::f64::consts::TAU);
                            let vertical = input.swim_up as i32 as f64 - free_down as i32 as f64;
                            let solar = renderer.solar_state(camera.position(), planet.radius_km);
                            let nav_altitude =
                                triangulum_viewer::camera::nearest_surface_altitude_km(
                                    &camera,
                                    solar,
                                    &renderer.solar_tuning,
                                    planet.radius_km,
                                );
                            let speed = (nav_altitude * 0.5).clamp(0.02, 1500.0)
                                * if input.sprint { 4.0 } else { 1.0 };
                            camera.translate_free(input.strafe, vertical, input.fwd, speed * DT);
                            ps.underwater = false;
                            ps.grounded = false;
                        }
                    }
                    sim_ticks += 1;
                    renderer.set_render_time_s(sim_ticks as f64 * DT);
                    let solar = renderer.solar_state(camera.position(), planet.radius_km);
                    camera_rig.realign(solar, &mut camera);
                }
                trace!(
                    "[{}] hold {keys} {secs}s -> lat {:.6} lon {:.6} alt {:.5} grounded {}",
                    ln + 1,
                    camera.lat.to_degrees(),
                    camera.lon.to_degrees(),
                    camera.altitude_km,
                    ps.grounded
                );
            }
            "wait" => {
                let steps = (f(1)? / DT).round().max(1.0) as usize;
                let input = Input::default();
                for _ in 0..steps {
                    if matches!(
                        camera_rig.mode,
                        CameraMode::Focused(
                            triangulum_viewer::orbits::BodyId::Neisor
                                | triangulum_viewer::orbits::BodyId::Moon
                        )
                    ) {
                        let seasonal = SeasonalPlanet::new(
                            Arc::clone(&planet),
                            renderer.structural_season(&planet),
                        );
                        let body = focused_voxel_body(&camera, &seasonal, &moon_body).unwrap();
                        let active_edits = focused_edits(&camera, &edits, &moon_edits);
                        let gravity = renderer
                            .solar_tuning
                            .surface_gravity_mps2(camera.body);
                        ps.update(
                            body,
                            active_edits,
                            gravity,
                            &mut camera,
                            &input,
                            exagg,
                            DT,
                        );
                    }
                    sim_ticks += 1;
                    renderer.set_render_time_s(sim_ticks as f64 * DT);
                    let solar = renderer.solar_state(camera.position(), planet.radius_km);
                    camera_rig.realign(solar, &mut camera);
                }
                trace!(
                    "[{}] wait {}s -> alt {:.4} grounded {}",
                    ln + 1,
                    f(1)?,
                    camera.altitude_km,
                    ps.grounded
                );
            }
            // posehz N — sample rate of the following `pose` lines (in-game
            // recordings are written at 10 Hz); pose LAT LON ALT YAW PITCH —
            // one sample of a recorded session: set the exact pose, advance
            // the world clock by the sample interval. With TRI_POSE_RENDER=1
            // each pose also records a settled frame (glide-style) so any
            // recorded session renders straight to video.
            "posehz" => {
                let hz = f(1)?.clamp(1.0, 60.0);
                pose_ticks = (60.0 / hz).round().max(1.0) as u64;
            }
            "pose" => {
                ps.teleport(&planet, &edits, &mut camera, f(1)?, f(2)?, Some(f(3)?), exagg);
                camera.yaw = f(4)?.to_radians();
                camera.pitch = f(5)?.to_radians().clamp(-1.50, 1.50);
                sim_ticks += pose_ticks;
                renderer.set_render_time_s(sim_ticks as f64 * DT);
                let solar = renderer.solar_state(camera.position(), planet.radius_km);
                camera_rig.realign(solar, &mut camera);
                if std::env::var_os("TRI_POSE_RENDER").is_some() {
                    let frames_dir = format!("{dir}/frames");
                    std::fs::create_dir_all(&frames_dir)?;
                    renderer.underwater = ps.underwater;
                    let active_edits = focused_edits(&camera, &edits, &moon_edits);
                    renderer.refresh_edits_snapshot(active_edits);
                    renderer.capture(
                        &planet,
                        &camera,
                        active_edits,
                        &format!("{frames_dir}/f{rec_frame:06}.png"),
                    )?;
                    rec_frame += 1;
                }
            }
            // glide LAT LON ALT_KM YAW PITCH SECONDS — smooth eased camera
            // move from the current pose, recording every output frame (30
            // fps) as a SETTLED capture into <run>/frames/. dwell SECONDS
            // records without moving. Settled per-frame captures mean every
            // trailer frame is fully converged (no pops, no blur) and the
            // whole timeline is deterministic; world time advances at real
            // rate (2 sim ticks per output frame).
            "glide" | "dwell" => {
                const REC_FPS: f64 = 30.0;
                let holding = toks[0] == "dwell";
                let seconds = if holding { f(1)? } else { f(6)? };
                let frames = (seconds * REC_FPS).round().max(1.0) as u64;
                let start = (
                    camera.lat.to_degrees(),
                    camera.lon.to_degrees(),
                    camera.altitude_km,
                    camera.yaw.to_degrees(),
                    camera.pitch.to_degrees(),
                );
                let target = if holding {
                    start
                } else {
                    (f(1)?, f(2)?, f(3)?, f(4)?, f(5)?)
                };
                let wrap = |a: f64| (a + 540.0).rem_euclid(360.0) - 180.0;
                let frames_dir = format!("{dir}/frames");
                std::fs::create_dir_all(&frames_dir)?;
                for i in 0..frames {
                    let t = (i + 1) as f64 / frames as f64;
                    let e = t * t * (3.0 - 2.0 * t);
                    let lat = start.0 + (target.0 - start.0) * e;
                    let lon = start.1 + wrap(target.1 - start.1) * e;
                    // geometric altitude interpolation: descents from orbit
                    // spend their time near the ground, not in empty sky
                    let alt = if start.2 > 1e-6 && target.2 > 1e-6 {
                        start.2 * (target.2 / start.2).powf(e)
                    } else {
                        start.2 + (target.2 - start.2) * e
                    };
                    let yaw = start.3 + wrap(target.3 - start.3) * e;
                    let pitch = start.4 + (target.4 - start.4) * e;
                    ps.teleport(&planet, &edits, &mut camera, lat, lon, Some(alt), exagg);
                    camera.yaw = yaw.to_radians();
                    camera.pitch = pitch.to_radians().clamp(-1.50, 1.50);
                    sim_ticks += 1;
                    renderer.set_render_time_s(sim_ticks as f64 * DT);
                    let solar = renderer.solar_state(camera.position(), planet.radius_km);
                    camera_rig.realign(solar, &mut camera);
                    renderer.underwater = ps.underwater;
                    let active_edits = focused_edits(&camera, &edits, &moon_edits);
                    renderer.refresh_edits_snapshot(active_edits);
                    renderer.capture(
                        &planet,
                        &camera,
                        active_edits,
                        &format!("{frames_dir}/f{rec_frame:06}.png"),
                    )?;
                    rec_frame += 1;
                    if rec_frame.is_multiple_of(30) {
                        trace!(
                            "[{}] {} frame {rec_frame} ({:.0}s of video)",
                            ln + 1,
                            toks[0],
                            rec_frame as f64 / REC_FPS
                        );
                    }
                }
                trace!(
                    "[{}] {} done -> lat {:.4} lon {:.4} alt {:.4} ({} frames total)",
                    ln + 1,
                    toks[0],
                    camera.lat.to_degrees(),
                    camera.lon.to_degrees(),
                    camera.altitude_km,
                    rec_frame
                );
            }
            "tap" => {
                match toks.get(1).copied() {
                    Some("space") => ps.jump(),
                    Some("lmb") | Some("rmb") => {
                        let dh = if toks[1] == "rmb" { 1 } else { -1 };
                        let seasonal = SeasonalPlanet::new(
                            Arc::clone(&planet),
                            renderer.structural_season(&planet),
                        );
                        let body = focused_voxel_body(&camera, &seasonal, &moon_body)
                            .ok_or_else(|| anyhow::anyhow!(
                                "line {}: block edit needs neisor or moon focus",
                                ln + 1
                            ))?;
                        let active_edits = if camera.body
                            == triangulum_viewer::orbits::BodyId::Moon
                        {
                            &mut moon_edits
                        } else {
                            &mut edits
                        };
                        if let Some(dirty) =
                            player::edit_block(body, active_edits, &camera, ps.mode, dh, exagg)
                        {
                            renderer.refresh_edits_snapshot(active_edits);
                            renderer.invalidate_chunks(&dirty);
                            // the ground under the player may have moved
                            ps.refresh_after_edit(body, active_edits, &camera, exagg);
                        } else {
                            trace!("[{}] tap {} hit nothing in reach", ln + 1, toks[1]);
                        }
                    }
                    Some("r") => {
                        if camera.body != triangulum_viewer::orbits::BodyId::Neisor {
                            trace!("[{}] tap r ignored: moon has no torch/weather stack", ln + 1);
                            continue;
                        }
                        if let Some(dirty) = player::toggle_torch(
                            &*planet,
                            &edits,
                            &mut torches,
                            &camera,
                            ps.mode,
                            exagg,
                        ) {
                            renderer.set_torches(torches.clone());
                            renderer.invalidate_chunks(&dirty);
                        } else {
                            trace!("[{}] tap r hit nothing in reach", ln + 1);
                        }
                    }
                    _ => anyhow::bail!("line {}: tap space|lmb|rmb|r", ln + 1),
                }
                trace!("[{}] tap {}", ln + 1, toks[1]);
            }
            "sun" => {
                if toks.get(1).is_some_and(|s| {
                    s.eq_ignore_ascii_case("physical") || s.eq_ignore_ascii_case("live")
                }) {
                    renderer.sun_dir = None;
                    trace!("[{}] sun physical", ln + 1);
                    continue;
                }
                let (la, lo) = (f(1)?.to_radians(), f(2)?.to_radians());
                renderer.sun_dir = Some(glam::DVec3::new(
                    la.cos() * lo.cos(),
                    la.cos() * lo.sin(),
                    la.sin(),
                ));
                trace!("[{}] sun pinned at lat {} lon {}", ln + 1, f(1)?, f(2)?);
            }
            // probe LAT LON — the block-truth spot check, in-run: dumps the
            // full-octave sample plus the column's ground/water/cave-water
            // so a hunt can correlate what it SEES with what the world IS
            // without loading the census binary per question.
            // bench N — render N frames at the current pose (no capture
            // readback) and print avg/p95 draw wall time: the objective
            // framerate instrument for regression bisection.
            "bench" => {
                let n: usize = f(1)? as usize;
                renderer.set_render_time_s(sim_ticks as f64 * DT);
                let solar = renderer.solar_state(camera.position(), planet.radius_km);
                camera_rig.realign(solar, &mut camera);
                renderer.underwater = ps.underwater;
                let mut times = Vec::with_capacity(n);
                let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("bench"),
                    size: wgpu::Extent3d {
                        width: renderer.size.0,
                        height: renderer.size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: renderer.format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                });
                let view = tex.create_view(&Default::default());
                let active_edits = focused_edits(&camera, &edits, &moon_edits);
                renderer.refresh_edits_snapshot(active_edits);
                renderer.gpu_timer_reset();
                for _ in 0..n {
                    let t0 = std::time::Instant::now();
                    renderer.draw(&view, &planet, &camera, active_edits);
                    let _ = renderer.device.poll(wgpu::PollType::wait_indefinitely());
                    times.push(t0.elapsed().as_secs_f64() * 1000.0);
                }
                times.sort_by(|a, b| a.total_cmp(b));
                let avg: f64 = times.iter().sum::<f64>() / times.len().max(1) as f64;
                let p95 = times[(times.len() as f64 * 0.95) as usize % times.len()];
                trace!(
                    "[{}] bench {n}: avg {avg:.2} ms  p95 {p95:.2} ms  min {:.2}  max {:.2}",
                    ln + 1,
                    times[0],
                    times[times.len() - 1]
                );
                if let Some(summary) = renderer.gpu_timer_summary() {
                    trace!("[{}] {summary}", ln + 1);
                }
            }
            // stream N - live detail-streaming calibration (0 strict,
            // 1 balanced, 2 eager); mirrors the window app's F9 cycle.
            "stream" => {
                let level = f(1)? as u8;
                renderer.stream_level = level.min(2);
                trace!("[{}] stream level {}", ln + 1, renderer.stream_level);
            }
            "probe" => {
                let (pla, plo) = (f(1)?.to_radians(), f(2)?.to_radians());
                let pdir =
                    glam::DVec3::new(pla.cos() * plo.cos(), pla.cos() * plo.sin(), pla.sin());
                let (pf, pu, pv) = triangulum_viewer::planet::face_from_dir(pdir);
                let season = renderer.structural_season(&planet);
                let s = triangulum_viewer::terrain::sample_at_season(
                    &planet,
                    pf,
                    pu,
                    pv,
                    triangulum_viewer::terrain::VOXEL_OCTAVES,
                    season,
                );
                let nn = triangulum_viewer::voxel::COLUMNS_PER_FACE as f64;
                let ci = (((pu + 1.0) * 0.5) * nn).floor() as i64;
                let cj = (((pv + 1.0) * 0.5) * nn).floor() as i64;
                let seasonal = SeasonalPlanet::new(Arc::clone(&planet), season);
                let c = triangulum_viewer::voxel::col_ctx_ext_body(
                    &seasonal,
                    &edits,
                    pf,
                    ci,
                    cj,
                );
                let fmt_lvl = |x: f64| {
                    if x.is_finite() {
                        format!("{:.1}m", x * 1000.0)
                    } else {
                        "-".into()
                    }
                };
                trace!(
                    "[{}] probe {:.5} {:.5}: h={:.1}m water={} lake={} frozen={} annual={:.2}C seasonal={:.2}C lake_lvl={} pond_lvl={} riv hw={:.1}m wet={:.2} | col ground={} water={} cave_water={}",
                    ln + 1,
                    f(1)?,
                    f(2)?,
                    s.h_km * 1000.0,
                    fmt_lvl(s.water_km),
                    s.lake,
                    s.frozen,
                    s.temp_c,
                    s.seasonal_temp_c,
                    fmt_lvl(s.lake_level_km),
                    fmt_lvl(s.pond_level_km),
                    s.river_hw_km * 1000.0,
                    s.river_wet,
                    c.ground,
                    if c.water == i64::MIN {
                        "-".into()
                    } else {
                        c.water.to_string()
                    },
                    if c.cave_water == i64::MIN {
                        "-".into()
                    } else {
                        c.cave_water.to_string()
                    },
                );
            }
            // voxels on | off — master switch for the voxel near-field.
            // `off` renders the pure heightfield mesh (no chunks, no hole
            // cut), which is what the sync-diff harness diffs against the
            // normal frame to measure mesh<->voxel appearance divergence.
            "voxels" => match toks.get(1).copied() {
                Some("on") => {
                    renderer.voxels_on = true;
                    trace!("[{}] voxels on", ln + 1);
                }
                Some("off") => {
                    renderer.voxels_on = false;
                    trace!("[{}] voxels off", ln + 1);
                }
                _ => anyhow::bail!("line {}: voxels on|off", ln + 1),
            },
            // weather off | live | pin COVER PRECIP — pin or disable the
            // living weather (WEATHER.md), mirroring `sun`. Weather rides
            // the fixed sim clock here, so even `live` is byte-identical
            // across runs; scripts pin it when a scene needs an exact sky.
            "weather" => match toks.get(1).copied() {
                Some("off") => {
                    renderer.weather_on = false;
                    renderer.weather_pin = None;
                    trace!("[{}] weather off", ln + 1);
                }
                Some("live") => {
                    renderer.weather_on = true;
                    renderer.weather_pin = None;
                    trace!("[{}] weather live", ln + 1);
                }
                Some("pin") => {
                    let (c, p) = (f(2)?, f(3)?);
                    anyhow::ensure!(
                        c.is_finite() && p.is_finite(),
                        "line {}: weather pin values must be finite",
                        ln + 1
                    );
                    renderer.weather_on = true;
                    renderer.weather_pin =
                        Some((c.clamp(0.0, 1.0) as f32, p.clamp(0.0, 1.0) as f32));
                    trace!("[{}] weather pin cover {c:.2} precip {p:.2}", ln + 1);
                }
                // jump the year: season is Neisor's orbital mean-anomaly phase,
                // so a script can shoot the same forest in deep winter and
                // high summer without simulating months of ticks
                Some("season") => {
                    let requested = f(2)?;
                    anyhow::ensure!(
                        requested.is_finite(),
                        "line {}: weather season must be finite",
                        ln + 1
                    );
                    let frac = requested.rem_euclid(1.0);
                    renderer.set_render_time_s(sim_ticks as f64 * DT);
                    renderer.set_season_frac(frac);
                    let solar = renderer.solar_state(camera.position(), planet.radius_km);
                    camera_rig.realign(solar, &mut camera);
                    trace!("[{}] weather season {frac:.2}", ln + 1);
                }
                Some("time") => {
                    let t_s = f(2)?;
                    anyhow::ensure!(
                        t_s.is_finite() && t_s >= 0.0,
                        "line {}: weather time must be finite and >= 0",
                        ln + 1
                    );
                    // The renderer may lag sim_ticks until a shot. Align the
                    // base clock first, then seek by offset so subsequent
                    // waits advance from exactly this absolute weather time.
                    renderer.set_render_time_s(sim_ticks as f64 * DT);
                    renderer.set_weather_time_s(t_s);
                    let solar = renderer.solar_state(camera.position(), planet.radius_km);
                    camera_rig.realign(solar, &mut camera);
                    trace!("[{}] weather time {t_s:.6}", ln + 1);
                }
                _ => anyhow::bail!(
                    "line {}: weather off|live|pin COVER PRECIP|time T_S|season FRAC",
                    ln + 1
                ),
            },
            "shot" => {
                let name = toks.get(1).copied().unwrap_or("frame");
                renderer.set_render_time_s(sim_ticks as f64 * DT);
                let solar = renderer.solar_state(camera.position(), planet.radius_km);
                camera_rig.realign(solar, &mut camera);
                renderer.underwater = ps.underwater;
                let active_edits = focused_edits(&camera, &edits, &moon_edits);
                renderer.refresh_edits_snapshot(active_edits);
                let n = renderer.capture(
                    &planet,
                    &camera,
                    active_edits,
                    &format!("{dir}/{name}.png"),
                )?;
                write_state(name, &camera, &camera_rig, &renderer, &ps, active_edits, &dir)?;
                trace!(
                    "[{}] shot {name} ({n} draws) at lat {:.6} lon {:.6} alt {:.5} yaw {:.0} pitch {:.0}",
                    ln + 1,
                    camera.lat.to_degrees(),
                    camera.lon.to_degrees(),
                    camera.altitude_km,
                    camera.yaw.to_degrees(),
                    camera.pitch.to_degrees()
                );
            }
            "state" => {
                let name = toks.get(1).copied().unwrap_or("state");
                write_state(
                    name,
                    &camera,
                    &camera_rig,
                    &renderer,
                    &ps,
                    focused_edits(&camera, &edits, &moon_edits),
                    &dir,
                )?;
                trace!("[{}] state {name}", ln + 1);
            }
            "log" => {
                trace!("[{}] # {}", ln + 1, toks[1..].join(" "));
            }
            // assert FIELD OP VALUE  — check a state value; a failure is
            // recorded and makes the whole run exit non-zero. FIELD is any
            // state key (grounded, underwater, mode, alt_km, radius_km,
            // ground_km, support_below_km, water_surface_km, ceiling_above_km,
            // has_water, vert_vel_mps, lat_deg, lon_deg, yaw_deg, pitch_deg).
            // OP is one of
            // == != < <= > >= or ~ (approx; optional 4th token = tolerance).
            // VALUE is a number, true/false, a mode string, or `none`.
            "assert" => {
                enum V {
                    B(bool),
                    N(f64),
                    S(&'static str),
                    None,
                }
                let field = toks
                    .get(1)
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_default();
                // two forms: `assert FIELD VALUE` (implicit ==) and
                // `assert FIELD OP VALUE` (+ optional tolerance token for `~`)
                let (op, want) = if toks.len() == 3 {
                    ("==", toks[2])
                } else {
                    (
                        toks.get(2).copied().unwrap_or("=="),
                        toks.get(3).copied().unwrap_or(""),
                    )
                };
                if toks.len() > 5 {
                    anyhow::bail!("line {}: assert: too many tokens", ln + 1);
                }
                let dir = camera.local_direction();
                let feet = camera.ground_km;
                let eye = feet + camera.altitude_km;
                let camera_pos = camera.position();
                let solar = renderer.solar_state(camera_pos, planet.radius_km);
                let seasonal = SeasonalPlanet::new(
                    Arc::clone(&planet),
                    renderer.structural_season(&planet),
                );
                let body = focused_voxel_body(&camera, &seasonal, &moon_body)
                    .unwrap_or(&seasonal);
                let active_edits = focused_edits(&camera, &edits, &moon_edits);
                let (column_face, column_ci, column_cj) =
                    triangulum_viewer::voxel::column_id_body(body, dir);
                let column = triangulum_viewer::voxel::col_ctx_body(
                    body,
                    active_edits,
                    column_face as usize,
                    column_ci,
                    column_cj,
                );
                let broadleaf_tint = triangulum_viewer::weather::deciduous_tint(
                    triangulum_viewer::voxel::Mat::LeavesBroad.color([0.0; 3]),
                    column.seasonal_temp as f64,
                    column.seasonal_temp_trend as f64,
                    column.structural_season,
                );
                let edited_water_count = active_edits
                    .iter()
                    .filter(|(_, delta)| **delta > 0)
                    .filter(|((face, ci, cj), _)| {
                        triangulum_viewer::voxel::col_ctx_body(
                            body,
                            active_edits,
                            *face as usize,
                            *ci,
                            *cj,
                        )
                        .has_water()
                    })
                    .count();
                let actual = match field.as_str() {
                    "grounded" => V::B(ps.grounded),
                    "underwater" => V::B(ps.underwater),
                    "has_water" => {
                        V::B(water_surface_km(body, active_edits, dir, eye, exagg).is_some())
                    }
                    "body" => V::S(match camera.body {
                        triangulum_viewer::orbits::BodyId::Neisor => "neisor",
                        triangulum_viewer::orbits::BodyId::Moon => "moon",
                        triangulum_viewer::orbits::BodyId::Sun => "sun",
                    }),
                    "mode" => V::S(if ps.mode == Mode::Walk { "walk" } else { "fly" }),
                    "alt_km" => V::N(camera.altitude_km),
                    // absolute distance from the planet center (= radius_km +
                    // ground_km + altitude_km): the quantity the fly cruise
                    // elevation-lock (C-1) holds constant over mountains.
                    "radius_km" => V::N(camera.radius_km + camera.ground_km + camera.altitude_km),
                    "ground_km" => V::N(camera.ground_km),
                    "clearance_m" => V::N(
                        (feet - support_below_km(
                            body,
                            active_edits,
                            dir,
                            feet + 1e-7,
                            exagg,
                        )) * 1000.0,
                    ),
                    "vert_vel_mps" => V::N(ps.vert_vel_mps),
                    "gravity_mps2" => {
                        V::N(renderer.solar_tuning.surface_gravity_mps2(camera.body))
                    }
                    "lat_deg" => V::N(camera.lat.to_degrees()),
                    "lon_deg" => V::N(camera.lon.to_degrees()),
                    "yaw_deg" => V::N(camera.yaw.to_degrees()),
                    "pitch_deg" => V::N(camera.pitch.to_degrees()),
                    "roll_deg" => V::N(camera.roll.to_degrees()),
                    "focus_id" => V::N(camera_rig.numeric_focus_id()),
                    "camera_x_km" => V::N(camera_pos.x),
                    "camera_y_km" => V::N(camera_pos.y),
                    "camera_z_km" => V::N(camera_pos.z),
                    "solar_time_s" => V::N(renderer.weather_time_s()),
                    "season_frac" => V::N(solar.season_frac),
                    "season_bucket" => V::N(column.structural_season.bucket as f64),
                    "seasonal_temp_c" => V::N(column.seasonal_temp as f64),
                    "frozen" => V::B(column.frozen),
                    "broadleaf_r" => V::N(broadleaf_tint[0] as f64),
                    "broadleaf_g" => V::N(broadleaf_tint[1] as f64),
                    "broadleaf_b" => V::N(broadleaf_tint[2] as f64),
                    "sun_x_km" => V::N(solar.sun_km.x),
                    "sun_y_km" => V::N(solar.sun_km.y),
                    "sun_z_km" => V::N(solar.sun_km.z),
                    "moon_x_km" => V::N(solar.moon_km.x),
                    "moon_y_km" => V::N(solar.moon_km.y),
                    "moon_z_km" => V::N(solar.moon_km.z),
                    "solar_occlusion" => V::N(triangulum_viewer::orbits::solar_occlusion_at(
                        camera_pos,
                        solar,
                        &renderer.solar_tuning,
                        planet.radius_km,
                    )),
                    "lunar_shadow" => V::N(triangulum_viewer::orbits::lunar_shadow_fraction(
                        solar,
                        &renderer.solar_tuning,
                        planet.radius_km,
                    )),
                    "focus_distance_km" => {
                        let value = camera_rig.focus_distance_km(solar, &camera);
                        if value.is_finite() {
                            V::N(value)
                        } else {
                            V::None
                        }
                    }
                    "focus_alignment" => {
                        let value = camera_rig.focus_alignment(solar, &camera);
                        if value.is_finite() {
                            V::N(value)
                        } else {
                            V::None
                        }
                    }
                    "support_below_km" => {
                        V::N(support_below_km(body, active_edits, dir, feet + 1e-7, exagg))
                    }
                    // the generic solid surface (aiming/placing/torches);
                    // on a frozen sheet it must AGREE with the walkable
                    // support — regressing to the seabed re-opens the
                    // edit-through-ice family (Sol review, 2026-07-09)
                    "surface_height_km" => {
                        V::N(surface_height_km(body, active_edits, dir, exagg))
                    }
                    "column_edit_blocks" => {
                        V::N(
                            active_edits
                                .get(&(column_face, column_ci, column_cj))
                                .copied()
                                .unwrap_or(0) as f64,
                        )
                    }
                    "edit_count" => V::N(active_edits.len() as f64),
                    "edited_water_count" => V::N(edited_water_count as f64),
                    "edit_total_blocks" => {
                        V::N(active_edits.values().copied().sum::<i64>() as f64)
                    }
                    "column_ground_block" => V::N(column.ground as f64),
                    "column_natural_ground_block" => V::N(column.ground0 as f64),
                    "column_step_max" => {
                        let neighbors = [
                            (1i64, 0i64),
                            (-1, 0),
                            (0, 1),
                            (0, -1),
                        ];
                        let step = neighbors
                            .into_iter()
                            .map(|(di, dj)| {
                                let n = triangulum_viewer::voxel::col_ctx_ext_body(
                                    body,
                                    active_edits,
                                    column_face as usize,
                                    column_ci as i64 + di,
                                    column_cj as i64 + dj,
                                );
                                (column.ground0 - n.ground0).abs()
                            })
                            .max()
                            .unwrap_or(0);
                        V::N(step as f64)
                    }
                    "column_step_pos_i" | "column_step_neg_i" | "column_step_pos_j"
                    | "column_step_neg_j" => {
                        let (di, dj) = match field.as_str() {
                            "column_step_pos_i" => (1, 0),
                            "column_step_neg_i" => (-1, 0),
                            "column_step_pos_j" => (0, 1),
                            _ => (0, -1),
                        };
                        let neighbor = triangulum_viewer::voxel::col_ctx_ext_body(
                            body,
                            active_edits,
                            column_face as usize,
                            column_ci as i64 + di,
                            column_cj as i64 + dj,
                        );
                        V::N((neighbor.ground0 - column.ground0) as f64)
                    }
                    "neighbor_ceiling_km" => {
                        let nn = body.columns_per_face() as f64;
                        let mut nearest = f64::INFINITY;
                        for (di, dj) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
                            let (face, ci, cj) = triangulum_viewer::voxel::canonical_column_body(
                                body,
                                column_face as usize,
                                column_ci as i64 + di,
                                column_cj as i64 + dj,
                            );
                            let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
                            let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
                            let neighbor_dir = triangulum_viewer::planet::face_dir(
                                face as usize,
                                u,
                                v,
                            );
                            nearest = nearest.min(ceiling_above_km(
                                body,
                                active_edits,
                                neighbor_dir,
                                feet + player::EYE_KM,
                                exagg,
                            ));
                        }
                        if nearest.is_finite() { V::N(nearest) } else { V::None }
                    }
                    "column_albedo" => column
                        .lunar
                        .map(|l| V::N(l.albedo as f64))
                        .unwrap_or(V::None),
                    "block_width_height_ratio" => {
                        // Probe actual neighboring column-center spacing in
                        // body-local world space, then express it in Neisor
                        // block-width units. Radial shells are one metre on
                        // both bodies, so width / Neisor-width is the block
                        // width/height proportion contract Andrew sees.
                        let spacing_m = |probe_body: &dyn VoxelBody, probe_dir| {
                            let (f, ci, cj) =
                                triangulum_viewer::voxel::column_id_body(probe_body, probe_dir);
                            let center = triangulum_viewer::voxel::dir_of_column_body(
                                probe_body,
                                f as usize,
                                ci,
                                cj,
                            );
                            let mut sum = 0.0;
                            for (di, dj) in [(1i64, 0i64), (0, 1)] {
                                let (nf, ni, nj) =
                                    triangulum_viewer::voxel::canonical_column_body(
                                        probe_body,
                                        f as usize,
                                        ci as i64 + di,
                                        cj as i64 + dj,
                                    );
                                let neighbor = triangulum_viewer::voxel::dir_of_column_body(
                                    probe_body,
                                    nf as usize,
                                    ni,
                                    nj,
                                );
                                sum += (neighbor - center).length()
                                    * probe_body.radius_km()
                                    * 1000.0;
                            }
                            sum * 0.5
                        };
                        let body_width_m = spacing_m(body, dir);
                        let neisor_width_m = spacing_m(&seasonal, dir);
                        V::N(body_width_m / neisor_width_m)
                    }
                    "lunar_material" => column
                        .lunar
                        .map(|l| {
                            V::S(match l.material {
                                triangulum_viewer::moon::MoonMaterial::Highland => "highland",
                                triangulum_viewer::moon::MoonMaterial::Maria => "maria",
                                triangulum_viewer::moon::MoonMaterial::Ray => "ray",
                            })
                        })
                        .unwrap_or(V::None),
                    "moon_surface_height_km" => {
                        let radius = renderer
                            .solar_tuning
                            .radius_km(triangulum_viewer::orbits::BodyId::Moon, planet.radius_km);
                        V::N(
                            triangulum_viewer::moon::MoonGenerator::new(planet.seed)
                                .height_km(moon_probe_dir, radius),
                        )
                    }
                    "moon_surface_albedo" => V::N(
                        triangulum_viewer::moon::MoonGenerator::new(planet.seed)
                            .sample(moon_probe_dir)
                            .albedo,
                    ),
                    "water_surface_km" => {
                        match water_surface_km(body, active_edits, dir, eye, exagg) {
                            Some(w) => V::N(w),
                            None => V::None,
                        }
                    }
                    "ceiling_above_km" => {
                        let c = ceiling_above_km(
                            body,
                            active_edits,
                            dir,
                            feet + player::EYE_KM,
                            exagg,
                        );
                        if c.is_finite() { V::N(c) } else { V::None }
                    }
                    _ => anyhow::bail!("line {}: assert: unknown field `{field}`", ln + 1),
                };
                let (pass, shown) = match &actual {
                    V::B(b) => {
                        let w = match want.to_ascii_lowercase().as_str() {
                            "true" | "1" => true,
                            "false" | "0" => false,
                            _ => anyhow::bail!(
                                "line {}: assert bool: expected true/1/false/0, got `{want}`",
                                ln + 1
                            ),
                        };
                        let p = match op {
                            "==" => *b == w,
                            "!=" => *b != w,
                            _ => anyhow::bail!("line {}: assert bool needs == or !=", ln + 1),
                        };
                        (p, b.to_string())
                    }
                    V::S(s) => {
                        let p = match op {
                            "==" => s.eq_ignore_ascii_case(want),
                            "!=" => !s.eq_ignore_ascii_case(want),
                            _ => anyhow::bail!("line {}: assert string needs == or !=", ln + 1),
                        };
                        (p, s.to_string())
                    }
                    V::None => {
                        let is_none = want.eq_ignore_ascii_case("none");
                        let p = match op {
                            "==" => is_none,
                            "!=" => !is_none,
                            _ => false,
                        };
                        (p, "none".to_string())
                    }
                    V::N(n) => {
                        if want.eq_ignore_ascii_case("none") {
                            (op == "!=", format!("{n:.6}"))
                        } else {
                            let w: f64 = want.parse().map_err(|_| {
                                anyhow::anyhow!("line {}: assert: bad number `{want}`", ln + 1)
                            })?;
                            if !w.is_finite() {
                                anyhow::bail!(
                                    "line {}: assert: non-finite number `{want}`",
                                    ln + 1
                                );
                            }
                            let eq = (n - w).abs() <= 1e-6 + w.abs() * 1e-4;
                            let p = match op {
                                "==" => eq,
                                "~" => {
                                    let tol = if let Some(raw_tol) = toks.get(4) {
                                        let tol: f64 = raw_tol.parse().map_err(|_| {
                                            anyhow::anyhow!(
                                                "line {}: assert: bad tolerance `{raw_tol}`",
                                                ln + 1
                                            )
                                        })?;
                                        if !tol.is_finite() || tol < 0.0 {
                                            anyhow::bail!(
                                                "line {}: assert: bad tolerance `{raw_tol}`",
                                                ln + 1
                                            );
                                        }
                                        tol
                                    } else {
                                        0.01
                                    };
                                    (n - w).abs() <= tol
                                }
                                "!=" => !eq,
                                "<" => *n < w,
                                "<=" => *n <= w,
                                ">" => *n > w,
                                ">=" => *n >= w,
                                _ => anyhow::bail!("line {}: assert: bad op `{op}`", ln + 1),
                            };
                            (p, format!("{n:.6}"))
                        }
                    }
                };
                if pass {
                    trace!(
                        "[{}] assert OK: {field} {op} {want} (actual {shown})",
                        ln + 1
                    );
                } else {
                    assert_fails += 1;
                    trace!(
                        "[{}] ASSERT FAIL: {field} {op} {want} (actual {shown})",
                        ln + 1
                    );
                }
            }
            other => anyhow::bail!("line {}: unknown command `{other}`", ln + 1),
        }
    }
    if assert_fails > 0 {
        trace!("run FAILED: {assert_fails} assertion(s) did not hold -> {dir}");
        anyhow::bail!("{assert_fails} assertion(s) failed");
    }
    trace!("run complete -> {dir}");
    Ok(())
}

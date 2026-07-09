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
//!   look YAW PITCH                absolute view angles, degrees
//!   turn DYAW DPITCH              relative view change, degrees
//!   mode walk|fly                 like G / F
//!   hold KEYS SECONDS             movement keys held for a duration at a
//!                                 fixed 60 Hz timestep; KEYS is any of
//!                                 w,a,s,d,shift,space joined by '+'
//!   tap space|q|e|r               jump / break / place / torch
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
//!                                 pitch_deg. OP: == != < <= > >= or ~ (approx,
//!                                 optional 4th token = tolerance). VALUE: a
//!                                 number, true/false, walk/fly, or `none`.
//!   sun LAT LON                   pin the sun. WARNING: this is a GLOBAL sun
//!                                 direction — far-longitude teleports then
//!                                 render at NIGHT. OMIT sun for surveys: the
//!                                 default lights every location at local noon.
//!   log TEXT...                   annotate the transcript
//!
//! Navigation is deliberately absolute-first: scripts teleport to
//! coordinates and set exact view angles; relative movement exists to
//! exercise the physics, not to find places.

use std::io::Write as IoWrite;
use std::sync::Arc;

use triangulum_viewer::camera::Camera;
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::player::{self, Input, Mode, PlayerState};
use triangulum_viewer::renderer::Renderer;
use triangulum_viewer::voxel::{
    ceiling_above_km, support_below_km, water_surface_km, Edits, Torches,
};

const DT: f64 = 1.0 / 60.0; // fixed timestep: scripts are deterministic

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
    let mut renderer = Renderer::new(device, queue, wgpu::TextureFormat::Rgba8UnormSrgb, size, exagg);
    renderer.patch_scale = patch;
    // deterministic by default: no day cycle (always day where you stand);
    // the `sun` command pins an exact sun for lighting-specific scripts

    // scripts run in a CLEAN world (no saved player edits/torches), so the
    // same script always produces the same frames on the same planet
    let mut edits = Edits::default();
    let mut torches = Torches::default();
    let mut ps = PlayerState::default();
    let mut camera = Camera {
        lon: 30f64.to_radians(),
        lat: 10f64.to_radians(),
        altitude_km: 100.0,
        radius_km: planet.radius_km,
        ground_km: 0.0,
        yaw: 0.0,
        pitch: 0.0,
    };
    ps.teleport(&planet, &edits, &mut camera, 10.0, 30.0, Some(100.0), exagg);

    let write_state = |name: &str,
                       camera: &Camera,
                       ps: &PlayerState,
                       edits: &Edits,
                       dir_path: &str|
     -> anyhow::Result<()> {
        let d = camera.position().normalize();
        let (face, u, v) = face_from_dir(d);
        let s = triangulum_viewer::terrain::sample(
            &planet,
            face,
            u,
            v,
            triangulum_viewer::terrain::VOXEL_OCTAVES,
        );
        let feet = camera.ground_km;
        let eye = feet + camera.altitude_km;
        let support = support_below_km(&planet, edits, d, feet + 1e-7, exagg);
        let ceil = ceiling_above_km(&planet, edits, d, feet + player::EYE_KM, exagg);
        let water = water_surface_km(&planet, edits, d, eye, exagg);
        let js = serde_json::json!({
            "lat_deg": camera.lat.to_degrees(),
            "lon_deg": camera.lon.to_degrees(),
            "alt_km": camera.altitude_km,
            "radius_km": camera.radius_km + camera.ground_km + camera.altitude_km,
            "yaw_deg": camera.yaw.to_degrees(),
            "pitch_deg": camera.pitch.to_degrees(),
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
                ps.teleport(&planet, &edits, &mut camera, f(1)?, f(2)?, f(3).ok(), exagg);
                trace!("[{}] teleport -> lat {:.6} lon {:.6} alt {:.5} km",
                    ln + 1, f(1)?, f(2)?, camera.altitude_km);
            }
            "look" => {
                camera.yaw = f(1)?.to_radians();
                camera.pitch = f(2)?.to_radians().clamp(-1.50, 1.50);
                trace!("[{}] look yaw {:.0} pitch {:.0}", ln + 1, f(1)?, f(2)?);
            }
            "turn" => {
                camera.yaw += f(1)?.to_radians();
                camera.pitch = (camera.pitch + f(2)?.to_radians()).clamp(-1.50, 1.50);
                trace!("[{}] turn -> yaw {:.0} pitch {:.0}",
                    ln + 1, camera.yaw.to_degrees(), camera.pitch.to_degrees());
            }
            "mode" => {
                match toks.get(1).copied() {
                    Some("walk") => ps.set_walk(&mut camera),
                    Some("fly") => ps.set_fly(&mut camera),
                    _ => anyhow::bail!("line {}: mode walk|fly", ln + 1),
                }
                // a mode switch is a pose change; keep underwater consistent
                // for an immediate shot with no update tick in between
                ps.refresh_underwater(&planet, &edits, &camera, exagg);
                trace!("[{}] mode {}", ln + 1, toks[1]);
            }
            "hold" => {
                let keys = toks
                    .get(1)
                    .ok_or_else(|| anyhow::anyhow!("line {}: hold KEYS SECONDS", ln + 1))?
                    .to_ascii_lowercase();
                let secs = f(2)?;
                let mut input = Input::default();
                for k in keys.split('+') {
                    match k {
                        "w" => input.fwd += 1.0,
                        "s" => input.fwd -= 1.0,
                        "d" => input.strafe += 1.0,
                        "a" => input.strafe -= 1.0,
                        "shift" => input.sprint = true,
                        "space" => input.swim_up = true,
                        other => anyhow::bail!("line {}: unknown key `{other}`", ln + 1),
                    }
                }
                let steps = (secs / DT).round().max(1.0) as usize;
                for _ in 0..steps {
                    ps.update(&planet, &edits, &mut camera, &input, exagg, DT);
                }
                trace!("[{}] hold {keys} {secs}s -> lat {:.6} lon {:.6} alt {:.5} grounded {}",
                    ln + 1, camera.lat.to_degrees(), camera.lon.to_degrees(),
                    camera.altitude_km, ps.grounded);
            }
            "wait" => {
                let steps = (f(1)? / DT).round().max(1.0) as usize;
                let input = Input::default();
                for _ in 0..steps {
                    ps.update(&planet, &edits, &mut camera, &input, exagg, DT);
                }
                trace!("[{}] wait {}s -> alt {:.4} grounded {}",
                    ln + 1, f(1)?, camera.altitude_km, ps.grounded);
            }
            "tap" => {
                match toks.get(1).copied() {
                    Some("space") => ps.jump(),
                    Some("q") | Some("e") => {
                        let dh = if toks[1] == "e" { 1 } else { -1 };
                        if let Some(dirty) =
                            player::edit_block(&planet, &mut edits, &camera, ps.mode, dh, exagg)
                        {
                            renderer.invalidate_chunks(&dirty);
                            // the ground under the player may have moved
                            ps.refresh_after_edit(&planet, &edits, &camera, exagg);
                        } else {
                            trace!("[{}] tap {} hit nothing in reach", ln + 1, toks[1]);
                        }
                    }
                    Some("r") => {
                        if let Some(dirty) = player::toggle_torch(
                            &planet, &edits, &mut torches, &camera, ps.mode, exagg,
                        ) {
                            renderer.torches = torches.clone();
                            renderer.invalidate_chunks(&dirty);
                        } else {
                            trace!("[{}] tap r hit nothing in reach", ln + 1);
                        }
                    }
                    _ => anyhow::bail!("line {}: tap space|q|e|r", ln + 1),
                }
                trace!("[{}] tap {}", ln + 1, toks[1]);
            }
            "sun" => {
                let (la, lo) = (f(1)?.to_radians(), f(2)?.to_radians());
                renderer.sun_dir = Some(glam::DVec3::new(
                    la.cos() * lo.cos(),
                    la.cos() * lo.sin(),
                    la.sin(),
                ));
                trace!("[{}] sun pinned at lat {} lon {}", ln + 1, f(1)?, f(2)?);
            }
            "shot" => {
                let name = toks.get(1).copied().unwrap_or("frame");
                renderer.underwater = ps.underwater;
                let n = renderer.capture(&planet, &camera, &edits, &format!("{dir}/{name}.png"))?;
                write_state(name, &camera, &ps, &edits, &dir)?;
                trace!("[{}] shot {name} ({n} draws) at lat {:.6} lon {:.6} alt {:.5} yaw {:.0} pitch {:.0}",
                    ln + 1, camera.lat.to_degrees(), camera.lon.to_degrees(),
                    camera.altitude_km, camera.yaw.to_degrees(), camera.pitch.to_degrees());
            }
            "state" => {
                let name = toks.get(1).copied().unwrap_or("state");
                write_state(name, &camera, &ps, &edits, &dir)?;
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
                let field = toks.get(1).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
                // two forms: `assert FIELD VALUE` (implicit ==) and
                // `assert FIELD OP VALUE` (+ optional tolerance token for `~`)
                let (op, want) = if toks.len() == 3 {
                    ("==", toks[2])
                } else {
                    (toks.get(2).copied().unwrap_or("=="), toks.get(3).copied().unwrap_or(""))
                };
                let dir = camera.position().normalize();
                let feet = camera.ground_km;
                let eye = feet + camera.altitude_km;
                let actual = match field.as_str() {
                    "grounded" => V::B(ps.grounded),
                    "underwater" => V::B(ps.underwater),
                    "has_water" => {
                        V::B(water_surface_km(&planet, &edits, dir, eye, exagg).is_some())
                    }
                    "mode" => V::S(if ps.mode == Mode::Walk { "walk" } else { "fly" }),
                    "alt_km" => V::N(camera.altitude_km),
                    // absolute distance from the planet center (= radius_km +
                    // ground_km + altitude_km): the quantity the fly cruise
                    // elevation-lock (C-1) holds constant over mountains.
                    "radius_km" => V::N(camera.radius_km + camera.ground_km + camera.altitude_km),
                    "ground_km" => V::N(camera.ground_km),
                    "vert_vel_mps" => V::N(ps.vert_vel_mps),
                    "lat_deg" => V::N(camera.lat.to_degrees()),
                    "lon_deg" => V::N(camera.lon.to_degrees()),
                    "yaw_deg" => V::N(camera.yaw.to_degrees()),
                    "pitch_deg" => V::N(camera.pitch.to_degrees()),
                    "support_below_km" => {
                        V::N(support_below_km(&planet, &edits, dir, feet + 1e-7, exagg))
                    }
                    "water_surface_km" => match water_surface_km(&planet, &edits, dir, eye, exagg) {
                        Some(w) => V::N(w),
                        None => V::None,
                    },
                    "ceiling_above_km" => {
                        let c = ceiling_above_km(
                            &planet, &edits, dir, feet + player::EYE_KM, exagg,
                        );
                        if c.is_finite() { V::N(c) } else { V::None }
                    }
                    _ => anyhow::bail!("line {}: assert: unknown field `{field}`", ln + 1),
                };
                let (pass, shown) = match &actual {
                    V::B(b) => {
                        let w = want.eq_ignore_ascii_case("true") || want == "1";
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
                            let tol = toks.get(4).and_then(|s| s.parse().ok()).unwrap_or(0.01);
                            let p = match op {
                                "==" => (n - w).abs() <= 1e-6 + w.abs() * 1e-4,
                                "~" => (n - w).abs() <= tol,
                                "!=" => (n - w).abs() > 1e-9,
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
                    trace!("[{}] assert OK: {field} {op} {want} (actual {shown})", ln + 1);
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

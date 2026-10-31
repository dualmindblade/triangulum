//! Photos (bug round #11; `docs/feature-inventory.md` §17.5): the frame, its
//! JSON sidecar, the roll of photos in `game/interchange/`, and turning a photo
//! back into a repro. "No repro, no fix" starts here: a sidecar holds what it
//! takes to stand in the same place and see the same frame.
//!
//! One format, for every way of taking a shot (P, Shift+P, the look panel's
//! button, `--screenshot`, the harness's `photo`). `format` names it and
//! `version` counts it. Everything else is one JSON object, and a sidecar is
//! only ever changed by reading it, changing keys and writing it back
//! (`edit_sidecar`), so keys this build has never heard of survive: to add to
//! a photo (annotations, say), add a key. `scan` hands out the typed fields the
//! window needs; anything else is read from the file by whoever wants it.
//!
//! The keys: `format`, `version`, `png`, `taken_utc`, `kind` (settled, live or
//! panel), `settle` (settled, seconds, frames), `title`, `notes`, `pose`
//! (lat, lon, alt_m, ground_m, alt_agl_m, heading_deg, pitch_deg, mode, exact:
//! the player's whole state, as a replay writes it), `sky` (clock_seconds,
//! clock_exact, speed, local_hour, season, sun_dir, settings), `world` (seed,
//! block size, world height, sea level, columns, the terrain dials), `terrain`,
//! `generator_version`, `sim_rules`, `bake_sha256`, `edits` (hash, chunks,
//! sim_tick), `build` (git, engine), `view` (width, height, fov_deg, mesh_view,
//! hide_water, hud), `frame_ms`, `look`, `blocks`, `texture_size`, `state`.
//!
//! Writing a PNG takes a few hundred milliseconds, so frames are written on a
//! thread of their own (`ShotWriter`); only the pixels are read back on the
//! main thread. Photos are never destroyed: delete moves a photo and everything
//! named after it to `interchange/trash/`.

use crate::camera::Camera;
use crate::look::Look;
use crate::render::MeshView;
use crate::replay::{ChunkDigests, edit_state, pose_text};
use crate::session::Session;
use crate::sky::Settings;
use crate::world::Registry;
use crate::world::save::{self, Identity};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};

pub const FORMAT: &str = "triangulum-shot";
pub const VERSION: u32 = 1;

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Send photos somewhere else (`--interchange`; evidence runs and tests).
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// Where photos live: `game/interchange/`.
pub fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_else(|| PathBuf::from(format!("{}/interchange", env!("CARGO_MANIFEST_DIR"))))
}

/// The name of a photo, as the old viewer named them.
pub fn shot_stem(lat: f64, lon: f64, alt_m: f64, heading_deg: f64, pitch_deg: f64, live: bool) -> String {
    format!("shot_lat{lat:.3}_lon{lon:.3}_alt{:.3}km_yaw{heading_deg:.0}_pitch{pitch_deg:.0}{}", alt_m / 1000.0, if live { "_raw" } else { "" })
}

/// `dir/stem.png`, or `dir/stem_2.png` and so on: never over another photo.
pub fn free_path(dir: &Path, stem: &str) -> PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}.png") } else { format!("{stem}_{n}.png") };
        let p = dir.join(name);
        if !p.exists() && !p.with_extension("json").exists() {
            return p;
        }
        n += 1;
    }
}

/// How a shot was taken.
pub struct ShotInfo {
    /// `settled` (P), `live` (Shift+P), `panel` (the look panel's button).
    pub kind: &'static str,
    /// The world finished loading before the frame was taken.
    pub settled: bool,
    pub settle_s: f64,
    pub settle_frames: u32,
    pub width: u32,
    pub height: u32,
    /// The picture includes the HUD.
    pub hud: bool,
    pub mesh_view: MeshView,
    pub preview_world: bool,
    /// (average, 95th percentile) frame time in ms, when the shooter knows it.
    pub frame_ms: Option<(f64, f64)>,
}

/// What a shot's look amounts to: every look value and every block's colours.
pub fn look_fields(look: &Look, registry: &Registry, texture_size: u32) -> (Value, Value, u32) {
    let colours = registry.colours();
    let textures = registry.textures();
    let blocks: Map<String, Value> = registry
        .defs()
        .iter()
        .zip(&colours)
        .zip(&textures)
        .filter(|((d, _), _)| !d.name.starts_with("unused_") && d.name != "air")
        .map(|((d, c), tex)| {
            let [t, s, b] = c.faces().map(|f| f.map(|x| (x as f64 * 1e4).round() / 1e4));
            let mut v = json!({ "top": t, "side": s, "bottom": b });
            if tex.any() {
                let [tt, ts, tb] = tex.faces();
                v["textures"] = json!({ "top": tt, "side": ts, "bottom": tb });
            }
            (d.name.clone(), v)
        })
        .collect();
    // Through text, so f32 colours read 0.55 and not 0.550000011920929.
    let look: Value = serde_json::to_string(&look.tidy()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    (look, Value::Object(blocks), texture_size)
}

/// The sidecar of a shot of `session` through `cam`. Reads the session and
/// changes nothing in it.
#[allow(clippy::too_many_arguments)]
pub fn sidecar(session: &Session, cam: &Camera, settings: &Settings, look: &Look, digests: &mut ChunkDigests, png: &Path, info: &ShotInfo) -> Value {
    let engine = &session.engine;
    let planet = &engine.planet;
    let (lat, lon) = crate::grid::Planet::lat_lon_deg(cam.pos);
    let alt_m = cam.pos.length() - planet.radius;
    let ground_m = engine.ground(cam.pos.normalize());
    // The camera can be ahead of the player by mouse motion not yet stepped
    // (`Controls::camera`): the exact pose is the one the photo saw.
    let mut seen = session.player.clone();
    seen.heading = cam.heading;
    seen.pitch = cam.pitch;
    let state = settings.at(session.sky.seconds);
    let (edit_hash, chunks) = edit_state(&engine.edits, digests);
    let reg = &engine.registry;
    let (look_json, blocks, texture_size) = look_fields(look, reg, reg.texture_size());
    let mut v = json!({
        "format": FORMAT,
        "version": VERSION,
        "png": png.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
        "taken_utc": crate::replay::utc_stamp(),
        "kind": info.kind,
        "settle": { "settled": info.settled, "seconds": info.settle_s, "frames": info.settle_frames },
        "title": "",
        "notes": "",
        "pose": {
            "lat": lat, "lon": lon, "alt_m": alt_m,
            "ground_m": ground_m, "alt_agl_m": alt_m - ground_m,
            "heading_deg": cam.heading.to_degrees(), "pitch_deg": cam.pitch.to_degrees(),
            "mode": if session.player.mode == crate::player::Mode::Walk { "walk" } else { "fly" },
            "exact": pose_text(&seen),
        },
        "sky": {
            "clock_seconds": session.sky.seconds,
            "clock_exact": format!("{:?}", session.sky.seconds),
            "speed": session.sky.speed,
            "local_hour": crate::workshop::local_hour(settings, session.sky.seconds, cam.pos.y.atan2(cam.pos.x)),
            "season": state.season,
            "sun_dir": state.sun.normalize().to_array(),
            "settings": {
                "day_length_s": settings.day_length_s,
                "lunar_month_days": settings.lunar_month_days,
                "months_per_season": settings.months_per_season,
                "year_days": settings.year_days,
            },
        },
        "world": Identity::of(&engine.config),
        "terrain": {
            "roughness": engine.config.roughness,
            "landform": engine.config.landform,
            "vertical_scale": engine.vscale(),
            "preview_world": info.preview_world,
        },
        "generator_version": save::GENERATOR_VERSION,
        "sim_rules": crate::sim::RULES_VERSION,
        "bake_sha256": session.bake_sha256,
    });
    // The json! macro has a depth limit, and these read better apart.
    v["edits"] = json!({ "hash": edit_hash, "chunks": chunks, "sim_tick": session.sim.tick() });
    v["build"] = json!({ "git": env!("TRIANGULUM_BUILD"), "engine": format!("triangulum {}", env!("CARGO_PKG_VERSION")) });
    v["view"] = json!({ "width": info.width, "height": info.height, "fov_deg": cam.fov_y.to_degrees(), "mesh_view": info.mesh_view.name(), "hide_water": engine.hide_water, "hud": info.hud });
    if let Some((avg, p95)) = info.frame_ms {
        v["frame_ms"] = json!({ "avg": avg, "p95": p95 });
    }
    v["look"] = look_json;
    v["blocks"] = blocks;
    v["texture_size"] = json!(texture_size);
    v["state"] = session.state();
    v
}

/// A frame and its sidecar, ready to write.
pub struct Shot {
    pub png: PathBuf,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub sidecar: Value,
}

/// Write a shot. The PNG goes last and appears whole, so anything that sees
/// `shot_*.png` also sees its sidecar.
pub fn write_shot(s: &Shot) -> Result<(), String> {
    if let Some(dir) = s.png.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    write_json(&s.png.with_extension("json"), &s.sidecar)?;
    let part = s.png.with_extension("png.part");
    crate::render::write_png(&part.to_string_lossy(), s.width, s.height, &s.rgba);
    std::fs::rename(&part, &s.png).map_err(|e| format!("write {}: {e}", s.png.display()))
}

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    let part = path.with_extension("json.part");
    std::fs::write(&part, text).map_err(|e| format!("write {}: {e}", part.display()))?;
    std::fs::rename(&part, path).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Writes shots off the main thread.
pub struct ShotWriter {
    tx: mpsc::Sender<Result<PathBuf, String>>,
    rx: mpsc::Receiver<Result<PathBuf, String>>,
    busy: usize,
}

impl Default for ShotWriter {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, busy: 0 }
    }
}

impl ShotWriter {
    pub fn write(&mut self, shot: Shot) {
        self.busy += 1;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(write_shot(&shot).map(|()| shot.png));
        });
    }

    /// A shot that finished writing, if any.
    pub fn poll(&mut self) -> Option<Result<PathBuf, String>> {
        let r = self.rx.try_recv().ok();
        if r.is_some() {
            self.busy -= 1;
        }
        r
    }

    /// Wait for everything in hand (quitting).
    pub fn finish(&mut self) {
        while self.busy > 0 {
            if self.rx.recv().is_err() {
                return;
            }
            self.busy -= 1;
        }
    }
}

// ---------------------------------------------------------------- the roll

/// One photo with what the window needs of its sidecar.
#[derive(Clone, Debug)]
pub struct Photo {
    pub png: PathBuf,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub title: String,
    pub notes: String,
    /// The player's exact pose (`replay::pose_text`) and the world it is good
    /// in: a pose restored into another world could land inside a mountain.
    pub exact: Option<String>,
    pub world: Option<Identity>,
    pub bake_sha256: Option<String>,
    /// The sky clock when it was taken, and the day length it ran on.
    pub clock_seconds: Option<f64>,
    pub day_length_s: Option<f64>,
    pub local_hour: Option<f64>,
    pub live: bool,
}

impl Photo {
    /// The title, or failing that where it was taken.
    pub fn label(&self) -> String {
        if self.title.trim().is_empty() { format!("{:.3} {:.3}  alt {:.0} m", self.lat, self.lon, self.alt_m) } else { self.title.clone() }
    }
}

/// The pose in a file name: (lat, lon, alt km, yaw, pitch).
pub fn parse_name(name: &str) -> Option<(f64, f64, f64, f64, f64)> {
    let grab = |key: &str, until: &[char]| -> Option<f64> {
        let s = name.split(key).nth(1)?;
        let end = s.find(until).unwrap_or(s.len());
        s[..end].parse().ok()
    };
    Some((grab("_lat", &['_'])?, grab("_lon", &['_'])?, grab("_alt", &['k'])?, grab("_yaw", &['_'])?, grab("_pitch", &['_', '.'])?))
}

fn read_photo(png: PathBuf, name: String) -> Option<Photo> {
    let js: Option<Value> = std::fs::read_to_string(png.with_extension("json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let from_name = parse_name(&name);
    let f = |path: &[&str]| -> Option<f64> {
        let mut v = js.as_ref()?;
        for k in path {
            v = v.get(k)?;
        }
        v.as_f64().filter(|x| x.is_finite())
    };
    let s = |key: &str| js.as_ref().and_then(|j| j.get(key)).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let lat = f(&["pose", "lat"]).or(from_name.map(|n| n.0))?;
    let lon = f(&["pose", "lon"]).or(from_name.map(|n| n.1))?;
    Some(Photo {
        lat,
        lon,
        alt_m: f(&["pose", "alt_m"]).or(from_name.map(|n| n.2 * 1000.0)).unwrap_or(0.0),
        heading_deg: f(&["pose", "heading_deg"]).or(from_name.map(|n| n.3)).unwrap_or(0.0),
        pitch_deg: f(&["pose", "pitch_deg"]).or(from_name.map(|n| n.4)).unwrap_or(0.0),
        title: s("title"),
        notes: s("notes"),
        exact: js.as_ref().and_then(|j| j["pose"]["exact"].as_str()).map(str::to_string),
        world: js.as_ref().and_then(|j| serde_json::from_value(j["world"].clone()).ok()),
        bake_sha256: js.as_ref().and_then(|j| j["bake_sha256"].as_str()).map(str::to_string),
        clock_seconds: js.as_ref().and_then(|j| j["sky"]["clock_exact"].as_str()).and_then(|t| t.parse().ok()).or(f(&["sky", "clock_seconds"])),
        day_length_s: f(&["sky", "settings", "day_length_s"]),
        local_hour: f(&["sky", "local_hour"]),
        live: js.as_ref().and_then(|j| j["kind"].as_str()).is_some_and(|k| k != "settled") || name.contains("_raw"),
        png,
        name,
    })
}

/// Every photo in `dir`, top level only, newest first. A shot with no sidecar
/// still counts, by the pose in its name.
pub fn scan(dir: &Path) -> Vec<Photo> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(std::time::SystemTime, Photo)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with("shot_") || !name.ends_with(".png") {
                return None;
            }
            let when = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            Some((when, read_photo(e.path(), name)?))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Change a photo's sidecar: read it, let `f` change it, write it back, keeping
/// every key `f` did not touch. A shot with no sidecar gets one.
pub fn edit_sidecar(png: &Path, f: impl FnOnce(&mut Map<String, Value>)) -> Result<(), String> {
    let path = png.with_extension("json");
    let mut v: Value = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    let Some(obj) = v.as_object_mut() else { return Err(format!("{} is not a JSON object", path.display())) };
    f(obj);
    write_json(&path, &v)
}

/// Save a photo's title and notes in its sidecar.
pub fn save_notes(p: &Photo) -> Result<(), String> {
    edit_sidecar(&p.png, |m| {
        m.insert("title".into(), Value::String(p.title.clone()));
        m.insert("notes".into(), Value::String(p.notes.clone()));
    })
}

/// Everything that goes with a photo: files beside it named `stem.*`.
fn family(png: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (png.parent(), png.file_stem().map(|s| s.to_string_lossy().into_owned())) else { return vec![png.to_path_buf()] };
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![png.to_path_buf()] };
    let prefix = format!("{stem}.");
    let mut v: Vec<PathBuf> = rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(&prefix)).map(|e| e.path()).collect();
    // The picture moves first: if anything fails, it is what comes back.
    v.sort_by_key(|p| p.as_path() != png);
    v
}

/// Move a photo and its family to `dir/trash/`, as one: either they all move
/// or none does. Nothing is overwritten. Returns how many files moved.
pub fn trash(dir: &Path, png: &Path) -> Result<usize, String> {
    let bin = dir.join("trash");
    std::fs::create_dir_all(&bin).map_err(|e| format!("create {}: {e}", bin.display()))?;
    let files = family(png);
    let stem = png.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    // One free name for the whole family.
    let mut base = stem.clone();
    let mut n = 1;
    let taken = |base: &str| files.iter().any(|f| bin.join(moved_name(f, &stem, base)).exists());
    while taken(&base) {
        n += 1;
        base = format!("{stem}-{n}");
    }
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for f in &files {
        let to = bin.join(moved_name(f, &stem, &base));
        if let Err(e) = std::fs::rename(f, &to) {
            for (from, to) in done.iter().rev() {
                let _ = std::fs::rename(to, from);
            }
            return Err(format!("move {}: {e}", f.display()));
        }
        done.push((f.clone(), to));
    }
    Ok(done.len())
}

fn moved_name(f: &Path, stem: &str, base: &str) -> String {
    let name = f.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    format!("{base}{}", name.strip_prefix(stem).unwrap_or(&name))
}

// ---------------------------------------------------------------- repro

/// A repro of a photo, as written beside it.
#[derive(Debug)]
pub struct Repro {
    pub script: PathBuf,
    /// Run this from `game/` to take the same frame.
    pub command: String,
    /// Stand there by hand: lat, lon, altitude, look and time, no save.
    pub at_command: String,
}

/// The `--play` command line of a sidecar's world, size and sky.
fn world_args(js: &Value) -> Result<String, String> {
    let w = &js["world"];
    let num = |v: &Value, name: &str| v.as_f64().ok_or(format!("the sidecar has no {name}"));
    let mut a = vec![
        format!("--seed {}", w["seed"].as_u64().ok_or("the sidecar has no world seed")?),
        format!("--block-size {}", num(&w["block_size_m"], "block size")?),
        format!("--height {}", w["world_height"].as_u64().ok_or("the sidecar has no world height")?),
        format!("--sea-level {}", w["sea_level"].as_u64().ok_or("the sidecar has no sea level")?),
        format!("--n {}", w["columns_per_face"].as_u64().ok_or("the sidecar has no columns per face")?),
        format!("--roughness {}", num(&w["roughness"], "roughness")?),
        format!("--landform {}", num(&w["landform"], "landform")?),
    ];
    if let Some(v) = w["vertical_scale"].as_f64() {
        a.push(format!("--vscale {v}"));
    }
    if let (Some(x), Some(y)) = (js["view"]["width"].as_u64(), js["view"]["height"].as_u64()) {
        a.push(format!("--size {x}x{y}"));
    }
    let s = &js["sky"]["settings"];
    if let (Some(d), Some(l), Some(m), Some(y)) = (s["day_length_s"].as_f64(), s["lunar_month_days"].as_f64(), s["months_per_season"].as_u64(), s["year_days"].as_f64()) {
        a.push(format!("--day-length {d} --lunar-month-days {l} --months-per-season {m} --year-days {y}"));
    }
    Ok(a.join(" "))
}

/// The text of a `.play` script that takes the frame of a sidecar: the exact
/// pose and clock, the same view, then a settled clean frame compared with the
/// original (printed, not asserted: another machine's GPU rounds differently).
/// It fails loudly when the world's edits are not there to see.
pub fn repro_script(js: &Value, stem: &str, command: &str) -> Result<String, String> {
    let exact = js["pose"]["exact"].as_str().ok_or("the sidecar has no exact pose (pose.exact)")?;
    let clock = js["sky"]["clock_exact"].as_str().ok_or("the sidecar has no exact clock (sky.clock_exact)")?;
    let chunks = js["edits"]["chunks"].as_u64().unwrap_or(0);
    let mut t = String::new();
    t.push_str(&format!("# Repro of {stem}.png, taken {} by build {} (generator {}, simulation rules {}).\n", js["taken_utc"].as_str().unwrap_or("?"), js["build"]["git"].as_str().unwrap_or("?"), js["generator_version"], js["sim_rules"]));
    t.push_str("# Run it from game/:\n#\n");
    t.push_str(&format!("#   {command}\n#\n"));
    if chunks > 0 {
        t.push_str(&format!("# The world had {chunks} edited chunk(s), and a script starts from a fresh save, so\n# the edit_hash check below fails unless you add a COPY of that save:\n#   --save COPY --keep-save\n"));
    }
    if js["view"]["hud"].as_bool() == Some(true) {
        t.push_str("# The original shows the HUD; this frame does not, so they differ there.\n");
    }
    if js["kind"].as_str() != Some("settled") {
        t.push_str("# The original is a live frame, taken before the world finished loading;\n# this one is settled, so the distance can differ.\n");
    }
    t.push_str(&format!("pose {exact}\n"));
    t.push_str(&format!("clock {clock}\n"));
    if js["view"]["hide_water"].as_bool() == Some(true) {
        t.push_str("water off\n");
    }
    if let Some(m) = js["view"]["mesh_view"].as_str().filter(|m| *m != "normal") {
        t.push_str(&format!("meshview {m}\n"));
    }
    t.push_str("photo repro\n");
    t.push_str(&format!("compare repro {stem}\n"));
    if let Some(h) = js["edits"]["hash"].as_str() {
        t.push_str(&format!("assert edit_hash == {h}\n"));
    }
    if let Some(g) = js["generator_version"].as_u64() {
        t.push_str(&format!("assert generator_version == {g}\n"));
    }
    Ok(t)
}

/// Write the repro of the photo at `png` beside it: `stem.play`, and
/// `stem.look.json` with the look it was taken in.
pub fn repro(png: &Path) -> Result<Repro, String> {
    let side = png.with_extension("json");
    let text = std::fs::read_to_string(&side).map_err(|e| format!("read {}: {e}", side.display()))?;
    let js: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", side.display()))?;
    if js["format"].as_str() != Some(FORMAT) {
        return Err(format!("{} is not a {FORMAT} sidecar", side.display()));
    }
    let stem = png.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let script = png.with_extension("play");
    let shown = |p: &Path| p.display().to_string().replace('\\', "/");
    let mut command = format!("cargo run --release -- --play {} {}", shown(&script), world_args(&js)?);
    if !js["look"].is_null() {
        let look = png.with_extension("look.json");
        write_json(&look, &js["look"])?;
        command.push_str(&format!(" --look-file {}", shown(&look)));
    }
    let body = repro_script(&js, &stem, &command)?;
    std::fs::write(&script, body).map_err(|e| format!("write {}: {e}", script.display()))?;
    let p = &js["pose"];
    let hour = js["sky"]["local_hour"].as_f64().unwrap_or(12.0);
    let num = |k: &str| p[k].as_f64().unwrap_or(0.0);
    let at_command = format!("cargo run --release -- --at {:.6} {:.6} {:.2} --look {:.3} {:.3} --time {:.4} --{}", num("lat"), num("lon"), num("alt_m"), num("heading_deg"), num("pitch_deg"), hour, p["mode"].as_str().unwrap_or("fly"));
    Ok(Repro { script, command, at_command })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("triangulum-photos-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fake_shot(dir: &Path, stem: &str, side: Value) -> PathBuf {
        let png = dir.join(format!("{stem}.png"));
        write_shot(&Shot { png: png.clone(), width: 2, height: 2, rgba: vec![9; 16], sidecar: side }).unwrap();
        png
    }

    fn sample() -> Value {
        json!({
            "format": FORMAT, "version": VERSION, "kind": "settled", "taken_utc": "20261031-050000",
            "title": "", "notes": "",
            "pose": { "lat": 12.5, "lon": -40.25, "alt_m": 300.5, "heading_deg": 45.0, "pitch_deg": -20.0, "mode": "fly", "exact": "1.0,2.0,3.0,0.0,0.0,0.0,0.5,0.25,fly,0,0,0,20.0,0" },
            "sky": { "clock_seconds": 77.5, "clock_exact": "77.5", "local_hour": 9.5, "settings": { "day_length_s": 1440.0, "lunar_month_days": 7.0, "months_per_season": 3, "year_days": 84.0 } },
            "world": { "name": "Neisor", "seed": 42, "block_size_m": 1.0, "world_height": 2048, "sea_level": 1024, "columns_per_face": 200000, "vertical_scale": null, "roughness": 1.0, "landform": 1.0 },
            "generator_version": 3, "sim_rules": 2, "bake_sha256": "abc",
            "edits": { "hash": "00ff", "chunks": 0 },
            "build": { "git": "abc1234" },
            "view": { "width": 1920, "height": 1080, "mesh_view": "normal", "hide_water": false, "hud": false },
            "look": { "sky": {} },
            "state": {},
        })
    }

    #[test]
    fn a_name_holds_the_pose_and_never_replaces_a_photo() {
        let stem = shot_stem(4.99, -29.4031, 47.0, 37.2, -29.0, false);
        assert_eq!(stem, "shot_lat4.990_lon-29.403_alt0.047km_yaw37_pitch-29");
        assert_eq!(parse_name(&format!("{stem}.png")), Some((4.99, -29.403, 0.047, 37.0, -29.0)));
        assert_eq!(parse_name(&format!("{}_2.png", shot_stem(1.0, 2.0, 3000.0, 4.0, 5.0, true))), Some((1.0, 2.0, 3.0, 4.0, 5.0)));
        assert_eq!(parse_name("bugs-found-1.txt"), None);
        let dir = scratch("names");
        let a = free_path(&dir, "shot_x");
        assert_eq!(a.file_name().unwrap(), "shot_x.png");
        std::fs::write(&a, b"1").unwrap();
        let b = free_path(&dir, "shot_x");
        assert_eq!(b.file_name().unwrap(), "shot_x_2.png");
        // A sidecar left alone also holds its name.
        std::fs::write(dir.join("shot_y.json"), b"{}").unwrap();
        assert_eq!(free_path(&dir, "shot_y").file_name().unwrap(), "shot_y_2.png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Title and notes go into the sidecar and every other key stays, new
    /// keys a later build added included.
    #[test]
    fn notes_keep_every_other_key() {
        let dir = scratch("notes");
        let mut side = sample();
        side["annotations"] = json!([{ "at": [0.5, 0.5], "text": "here" }]);
        let png = fake_shot(&dir, "shot_lat12.500_lon-40.250_alt0.300km_yaw45_pitch-20", side);
        let mut p = scan(&dir).remove(0);
        assert_eq!((p.lat, p.lon, p.alt_m), (12.5, -40.25, 300.5));
        assert_eq!(p.clock_seconds, Some(77.5));
        assert_eq!(p.world.as_ref().map(|w| w.seed), Some(42));
        p.title = "The lake".into();
        p.notes = "Water sticks to the shore\nsecond line".into();
        save_notes(&p).unwrap();
        let back = scan(&dir).remove(0);
        assert_eq!(back.title, "The lake");
        assert_eq!(back.notes, "Water sticks to the shore\nsecond line");
        assert_eq!(back.label(), "The lake");
        let js: Value = serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
        assert_eq!(js["annotations"][0]["text"], "here");
        assert_eq!(js["pose"]["lat"], 12.5);
        assert_eq!(js["look"], json!({ "sky": {} }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A shot with no sidecar shows by its name; damaged JSON does too; files
    /// that aren't shots, and folders, are left alone.
    #[test]
    fn the_roll_reads_what_it_can_and_nothing_else() {
        let dir = scratch("roll");
        std::fs::write(dir.join("shot_lat1.000_lon2.000_alt0.500km_yaw3_pitch-4.png"), b"x").unwrap();
        std::fs::write(dir.join("shot_lat5.000_lon6.000_alt0.100km_yaw7_pitch8_raw.png"), b"x").unwrap();
        std::fs::write(dir.join("shot_lat5.000_lon6.000_alt0.100km_yaw7_pitch8_raw.json"), b"{ damaged").unwrap();
        std::fs::write(dir.join("shot_noname.png"), b"x").unwrap();
        std::fs::write(dir.join("bugs-found-1.txt"), b"Andrew's notes").unwrap();
        std::fs::create_dir_all(dir.join("trash")).unwrap();
        std::fs::write(dir.join("trash/shot_lat9.000_lon9.000_alt0.100km_yaw7_pitch8.png"), b"x").unwrap();
        let roll = scan(&dir);
        assert_eq!(roll.len(), 2, "{roll:?}");
        let a = roll.iter().find(|p| p.lat == 1.0).unwrap();
        assert_eq!((a.lon, a.alt_m, a.heading_deg, a.pitch_deg), (2.0, 500.0, 3.0, -4.0));
        assert!(!a.live && a.exact.is_none());
        assert!(roll.iter().find(|p| p.lat == 5.0).unwrap().live);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Delete moves the photo and everything named after it to trash/, never
    /// over anything already there, and leaves everything else alone.
    #[test]
    fn delete_moves_to_the_trash_and_destroys_nothing() {
        let dir = scratch("trash");
        std::fs::write(dir.join("bugs-found-1.txt"), b"keep").unwrap();
        let png = fake_shot(&dir, "shot_a", sample());
        std::fs::write(png.with_extension("play"), b"repro").unwrap();
        std::fs::write(dir.join("shot_ab.json"), b"another photo's").unwrap();
        assert_eq!(trash(&dir, &png).unwrap(), 3);
        assert!(!png.exists() && !png.with_extension("json").exists() && !png.with_extension("play").exists());
        assert!(dir.join("trash/shot_a.png").exists() && dir.join("trash/shot_a.json").exists() && dir.join("trash/shot_a.play").exists());
        assert!(dir.join("bugs-found-1.txt").exists() && dir.join("shot_ab.json").exists());
        // The same name again: the old one stays.
        std::fs::write(dir.join("trash/shot_a.play"), b"first").unwrap();
        let png = fake_shot(&dir, "shot_a", sample());
        assert_eq!(trash(&dir, &png).unwrap(), 2);
        assert_eq!(std::fs::read(dir.join("trash/shot_a.play")).unwrap(), b"first");
        assert!(dir.join("trash/shot_a-2.png").exists() && dir.join("trash/shot_a-2.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_repro_script_is_written_from_the_sidecar() {
        let dir = scratch("repro");
        let png = fake_shot(&dir, "shot_r", sample());
        let r = repro(&png).unwrap();
        let text = std::fs::read_to_string(&r.script).unwrap();
        assert!(text.contains("pose 1.0,2.0,3.0,0.0,0.0,0.0,0.5,0.25,fly,0,0,0,20.0,0\n"), "{text}");
        assert!(text.contains("clock 77.5\n") && text.contains("photo repro\n") && text.contains("compare repro shot_r\n"), "{text}");
        assert!(text.contains("assert edit_hash == 00ff\n") && text.contains("assert generator_version == 3\n"), "{text}");
        assert!(!text.contains("water off") && !text.contains("meshview"), "{text}");
        assert!(r.command.contains("--seed 42") && r.command.contains("--size 1920x1080") && r.command.contains("--look-file") && r.command.contains("--day-length 1440"), "{}", r.command);
        assert!(r.at_command.contains("--at 12.500000 -40.250000 300.50") && r.at_command.contains("--look 45.000 -20.000") && r.at_command.ends_with("--fly"), "{}", r.at_command);
        assert!(png.with_extension("look.json").exists());
        // What the shot showed comes through.
        let mut side = sample();
        side["view"]["hide_water"] = json!(true);
        side["view"]["mesh_view"] = json!("lod+edges");
        side["edits"]["chunks"] = json!(4);
        let t = repro_script(&side, "s", "cmd").unwrap();
        assert!(t.contains("water off\n") && t.contains("meshview lod+edges\n") && t.contains("4 edited chunk"), "{t}");
        // A sidecar that can't say where it was is refused, in words.
        let mut side = sample();
        side["pose"]["exact"] = Value::Null;
        assert!(repro_script(&side, "s", "cmd").unwrap_err().contains("exact pose"));
        let not_ours = fake_shot(&dir, "shot_old", json!({ "lat_deg": 1.0 }));
        assert!(repro(&not_ours).unwrap_err().contains(FORMAT));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

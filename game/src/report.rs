//! Bug reports (B6): the moments marked in a recording, reading them back, and
//! gathering everything into one folder to send.
//!
//! F9 grabs the frame as a `Moment`, the annotator opens on it, and when it is
//! saved the moment becomes a shot on disk and, while recording, a marker: a
//! `note marker {JSON}` event at that step, naming its shot. Markers change
//! nothing a replay does. Their shots live in `<recording>.markers/` beside the
//! recording, each with a whole sidecar, so a marker stands on its own: when a
//! later build refuses the recording, its shot, its text, its exact pose and
//! `--repro` all still work.
//!
//! `make` gathers a recording or a set of photos into
//! `interchange/reports/<when>-<title>/` with a `report.md` written for someone
//! with no other context. That folder is the thing to send.

use crate::annotate::{self, Annotator, Mark};
use crate::camera::Camera;
use crate::look::Look;
use crate::photos::{self, Photo, Shot, ShotInfo};
use crate::render::MeshView;
use crate::replay::{ChunkDigests, Header, MAGIC};
use crate::session::Session;
use crate::sky::Settings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A `--watch` line stays this long (steps of the replay).
pub const SHOW_STEPS: u64 = 240;

/// One marked moment, as a replay's `note marker` holds it. The step is the
/// line's.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    #[serde(skip)]
    pub step: u64,
    /// The shot's stem, in `<recording>.markers/`.
    pub shot: String,
    pub text: String,
    /// The pose the shot saw (`replay::pose_text`): the camera can be ahead of
    /// the step by mouse motion not yet stepped.
    #[serde(default)]
    pub pose: String,
    /// The sky clock, exactly.
    #[serde(default)]
    pub clock: String,
}

impl Marker {
    /// A note's text, if it is a marker.
    pub fn parse(note: &str, step: u64) -> Option<Marker> {
        let mut m: Marker = serde_json::from_str(note.strip_prefix("marker ")?).ok()?;
        m.step = step;
        Some(m)
    }

    pub fn seconds(&self) -> f64 {
        self.step as f64 / 60.0
    }

    /// `marker02_step001234`: the settled shot `--replay` takes of it.
    pub fn replay_shot(&self, n: usize) -> String {
        format!("marker{n:02}_step{:06}", self.step)
    }
}

/// A note this module wrote, not one a player should be warned about.
pub fn is_ours(note: &str) -> bool {
    note.starts_with("marker {") || note.starts_with("visual {")
}

/// A `note visual` line's list of what changed.
pub fn visual_change(note: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(note.strip_prefix("visual ")?).ok()?;
    Some(v["changed"].as_array()?.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
}

/// Where a recording's marker shots live.
pub fn markers_dir(recording: &Path) -> PathBuf {
    let stem = recording.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    recording.with_file_name(format!("{stem}.markers"))
}

/// What `--watch` shows as a replay passes a marker.
pub struct Ticker {
    markers: Vec<Marker>,
}

impl Ticker {
    pub fn new(markers: Vec<Marker>) -> Self {
        Self { markers }
    }

    /// The line for this step, if a marker has just passed.
    pub fn line(&self, step: u64) -> Option<String> {
        let i = self.markers.iter().rposition(|m| m.step <= step && step < m.step + SHOW_STEPS)?;
        let m = &self.markers[i];
        Some(format!("Marker {} of {}, step {} ({:.1} s): {}", i + 1, self.markers.len(), m.step, m.seconds(), if m.text.trim().is_empty() { "(no text)" } else { m.text.trim() }))
    }
}

// ---------------------------------------------------------------- a moment

/// F9: a frame grabbed, waiting for the annotator. No step passes while it
/// waits, so its step and its state are the moment's.
pub struct Moment {
    pub png: PathBuf,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub sidecar: Value,
    pub step: u64,
    /// The recording's file and this marker's number, while recording.
    pub recording: Option<(PathBuf, u64)>,
}

/// What it takes to grab a moment.
pub struct Grab<'a> {
    pub session: &'a Session,
    pub cam: &'a Camera,
    pub settings: &'a Settings,
    pub look: &'a Look,
    pub digests: &'a mut ChunkDigests,
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub mesh_view: MeshView,
    pub preview_world: bool,
    pub frame_ms: Option<(f64, f64)>,
}

impl Moment {
    /// A live frame and its sidecar. While recording its shot goes beside the
    /// recording, and otherwise it is a photo like any other.
    pub fn grab(g: Grab) -> Self {
        let session = g.session;
        let recording = session.recorder.as_ref().map(|r| (r.path().to_path_buf(), r.next_marker()));
        let step = session.recorder.as_ref().map_or(session.steps(), |r| r.steps(session));
        let (lat, lon) = crate::grid::Planet::lat_lon_deg(g.cam.pos);
        let alt = g.cam.pos.length() - session.engine.planet.radius;
        let png = match &recording {
            Some((rec, n)) => photos::free_path(&markers_dir(rec), &format!("marker{n:02}_step{step:06}")),
            None => photos::free_path(&photos::dir(), &photos::shot_stem(lat, lon, alt, g.cam.heading.to_degrees(), g.cam.pitch.to_degrees(), true)),
        };
        let info = ShotInfo { kind: "marker", settled: false, settle_s: 0.0, settle_frames: 0, width: g.width, height: g.height, hud: false, mesh_view: g.mesh_view, preview_world: g.preview_world, frame_ms: g.frame_ms };
        let sidecar = photos::sidecar(session, g.cam, g.settings, g.look, g.digests, &png, &info);
        Self { png, width: g.width, height: g.height, rgba: g.rgba, sidecar, step, recording }
    }

    /// The annotator for this moment.
    pub fn annotator(&self) -> Annotator {
        let context = match &self.recording {
            Some((_, n)) => format!("Marker {n}, step {} ({:.1} s into the recording). The game is stopped.", self.step, self.step as f64 / 60.0),
            None => "Not recording: this saves a marked photo. The game is stopped.".to_string(),
        };
        let mut a = Annotator::new("Mark this moment", context, "Save and back to the game", self.width, self.height);
        a.set_picture(annotate::colour_image(&self.rgba, self.width, self.height));
        a
    }

    /// The annotator was saved: the marker goes in the recording, and the shot
    /// comes back to be written (off this thread: `ShotWriter::write_marked`).
    pub fn commit(mut self, session: &mut Session, marks: Vec<Mark>, comment: String) -> (Shot, Vec<Mark>) {
        let side = self.sidecar.as_object_mut().expect("a sidecar is an object");
        side.insert("notes".into(), Value::String(comment.clone()));
        side.insert("annotations".into(), annotate::marks_json(&marks, self.width, self.height));
        if !marks.is_empty() {
            let flat = photos::annotated_of(&self.png);
            side.insert("annotated".into(), Value::String(flat.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())));
        }
        if let Some((rec, n)) = &self.recording {
            side.insert("marker".into(), json!({ "recording": rec.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()), "n": n, "step": self.step, "time_s": self.step as f64 / 60.0 }));
            let marker = Marker {
                step: self.step,
                shot: self.png.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned()),
                text: comment,
                pose: self.sidecar["pose"]["exact"].as_str().unwrap_or("").to_string(),
                clock: self.sidecar["sky"]["clock_exact"].as_str().unwrap_or("").to_string(),
            };
            if let Some(mut r) = session.recorder.take() {
                r.marker(session, &marker);
                session.recorder = Some(r);
            }
        }
        (Shot { png: self.png, width: self.width, height: self.height, rgba: self.rgba, sidecar: self.sidecar }, marks)
    }
}

// ------------------------------------------------------------ the recordings

/// A marker in the T window's list.
#[derive(Clone, Debug)]
pub struct MarkerRow {
    pub marker: Marker,
    pub png: PathBuf,
    /// The shot, when it is on disk: its notes are the marker's latest text.
    pub photo: Option<Photo>,
}

impl MarkerRow {
    pub fn text(&self) -> &str {
        match &self.photo {
            Some(p) if !p.notes.trim().is_empty() => &p.notes,
            _ => &self.marker.text,
        }
    }
}

/// One `.replay` file, as the T window shows it.
#[derive(Clone, Debug, Default)]
pub struct Recording {
    pub path: PathBuf,
    pub name: String,
    pub title: String,
    pub description: String,
    pub created_utc: String,
    pub build: String,
    pub steps: u64,
    pub ended: bool,
    pub markers: Vec<MarkerRow>,
    /// Why it could not be read.
    pub error: Option<String>,
}

impl Recording {
    pub fn label(&self) -> String {
        if self.title.trim().is_empty() { self.name.trim_end_matches(".replay").to_string() } else { self.title.clone() }
    }

    pub fn seconds(&self) -> f64 {
        self.steps as f64 / 60.0
    }
}

/// What a `.replay` holds, read quickly: the header, and from the events only
/// the markers, the last step and whether it ended. No hashing, and a damaged
/// tail is fine; this is for lists, not for playing.
pub fn peek(path: &Path) -> Recording {
    let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let mut r = Recording { path: path.to_path_buf(), name, ..Default::default() };
    match peek_file(path, &mut r) {
        Ok(()) => {}
        Err(e) => r.error = Some(e),
    }
    if let Ok(t) = std::fs::read_to_string(path.with_extension("json"))
        && let Ok(v) = serde_json::from_str::<Value>(&t)
    {
        r.title = v["title"].as_str().unwrap_or("").to_string();
        r.description = v["description"].as_str().unwrap_or("").to_string();
    }
    r
}

fn peek_file(path: &Path, r: &mut Recording) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read: {e}"))?;
    let mut at = 0;
    let mut line = || -> Result<String, String> {
        let end = bytes[at..].iter().position(|&b| b == b'\n').ok_or("cut off in its header")?;
        let l = String::from_utf8_lossy(&bytes[at..at + end]).trim_end_matches('\r').to_string();
        at += end + 1;
        Ok(l)
    };
    if !line()?.starts_with(MAGIC) {
        return Err("not a Triangulum replay".into());
    }
    let h: Header = serde_json::from_str(&line()?).map_err(|e| format!("bad header: {e}"))?;
    r.created_utc = h.created_utc.clone();
    r.build = h.build.clone();
    let start = at + h.files.iter().map(|f| f.bytes as usize).sum::<usize>();
    let text = String::from_utf8_lossy(bytes.get(start..).ok_or("cut off inside its snapshot")?);
    let dir = markers_dir(path);
    for l in text.lines() {
        let Some((step, body)) = l.strip_prefix('@').and_then(|l| l.split_once(' ')) else { continue };
        let Ok(step) = step.parse::<u64>() else { continue };
        r.steps = r.steps.max(step);
        if body.starts_with("end ") {
            r.ended = true;
        }
        if let Some(m) = body.strip_prefix("note ").and_then(|t| Marker::parse(t, step)) {
            let png = dir.join(format!("{}.png", m.shot));
            let photo = if png.exists() { photos::read_photo(png.clone(), format!("{}.png", m.shot)) } else { None };
            r.markers.push(MarkerRow { marker: m, png, photo });
        }
    }
    Ok(())
}

/// Every recording in `dir`, newest first.
pub fn scan_recordings(dir: &Path) -> Vec<Recording> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<(std::time::SystemTime, Recording)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "replay"))
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), peek(&e.path())))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
    found.into_iter().map(|(_, r)| r).collect()
}

/// Save a recording's title and description, beside it (`<stem>.json`).
pub fn save_recording_notes(r: &Recording) -> Result<(), String> {
    photos::edit_sidecar(&r.path, |m| {
        m.insert("format".into(), Value::String("triangulum-recording".into()));
        m.insert("title".into(), Value::String(r.title.clone()));
        m.insert("description".into(), Value::String(r.description.clone()));
    })
}

// ------------------------------------------------------------------ the report

/// What to gather.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub title: String,
    pub description: String,
    pub recording: Option<PathBuf>,
    pub photos: Vec<PathBuf>,
}

fn slug(text: &str) -> String {
    let s: String = text.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s: Vec<&str> = s.split('-').filter(|p| !p.is_empty()).collect();
    let mut out = s.join("-");
    out.truncate(40);
    out.trim_end_matches('-').to_string()
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("copy {} to {}: {e}", from.display(), to.display()))
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())
}

/// One thing the report lists.
struct Item {
    /// Path of the picture inside the report folder.
    png: String,
    /// Of the copy with the marks, when there is one.
    annotated: Option<String>,
    side: Value,
    text: String,
    step: Option<u64>,
}

/// A `utc_stamp` for reading: 2026-10-31 08:50:29.
fn readable_utc(stamp: &str) -> String {
    match (stamp.get(0..4), stamp.get(4..6), stamp.get(6..8), stamp.get(9..11), stamp.get(11..13), stamp.get(13..15)) {
        (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) => format!("{y}-{mo}-{d} {h}:{mi}:{s}"),
        _ => stamp.to_string(),
    }
}

fn first_line(text: &str, or: &str) -> String {
    let l = text.lines().find(|l| !l.trim().is_empty()).unwrap_or(or).trim();
    if l.chars().count() > 70 { format!("{}...", l.chars().take(67).collect::<String>()) } else { l.to_string() }
}

fn read_side(png: &Path) -> Value {
    std::fs::read_to_string(png.with_extension("json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

/// Gather a report into `dir/reports/<when>-<title>/` and return the folder. It
/// is built under another name and renamed when it's whole, so a half-made
/// report never looks like one. Nothing outside it changes.
pub fn make(dir: &Path, req: &Request) -> Result<PathBuf, String> {
    let reports = dir.join("reports");
    let when = crate::replay::utc_stamp();
    let title = if req.title.trim().is_empty() { "untitled".to_string() } else { req.title.trim().to_string() };
    let base = format!("{when}-{}", slug(&title));
    let mut folder = reports.join(&base);
    let mut n = 1;
    while folder.exists() {
        n += 1;
        folder = reports.join(format!("{base}-{n}"));
    }
    let name = name_of(&folder);
    let part = reports.join(format!("{name}.part"));
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| format!("create {}: {e}", part.display()))?;
    let done = gather(&part, &name, &title, req);
    if let Err(e) = done {
        let _ = std::fs::remove_dir_all(&part);
        return Err(e);
    }
    std::fs::rename(&part, &folder).map_err(|e| format!("finish {}: {e}", folder.display()))?;
    Ok(folder)
}

fn gather(part: &Path, name: &str, title: &str, req: &Request) -> Result<(), String> {
    let mut items = Vec::new();
    let mut rec_section = String::new();
    if let Some(rec) = &req.recording {
        let peeked = peek(rec);
        copy(rec, &part.join(name_of(rec)))?;
        if rec.with_extension("json").exists() {
            copy(&rec.with_extension("json"), &part.join(name_of(&rec.with_extension("json"))))?;
        }
        // The markers keep their folder and names, so `--replay` finds them
        // beside the copy of the file as it does beside the original.
        let marks = markers_dir(rec);
        let folder = name_of(&marks);
        if marks.is_dir() {
            std::fs::create_dir_all(part.join(&folder)).map_err(|e| format!("create {folder}: {e}"))?;
            for e in std::fs::read_dir(&marks).map_err(|e| format!("read {}: {e}", marks.display()))?.flatten() {
                let p = e.path();
                // Repros written beside a shot are made again from its sidecar.
                if p.is_file() && !matches!(p.extension().and_then(|x| x.to_str()), Some("play")) && !name_of(&p).ends_with(".look.json") {
                    copy(&p, &part.join(&folder).join(name_of(&p)))?;
                }
            }
        }
        for row in &peeked.markers {
            let side = read_side(&row.png);
            let flat = photos::annotated_of(&row.png);
            items.push(Item { png: format!("{folder}/{}.png", row.marker.shot), annotated: flat.exists().then(|| format!("{folder}/{}", name_of(&flat))), side, text: row.text().to_string(), step: Some(row.marker.step) });
        }
        rec_section = recording_section(rec, &peeked, name);
    }
    for png in &req.photos {
        copy(png, &part.join(name_of(png)))?;
        let side_path = png.with_extension("json");
        if side_path.exists() {
            copy(&side_path, &part.join(name_of(&side_path)))?;
        }
        let flat = photos::annotated_of(png);
        let annotated = if flat.exists() {
            copy(&flat, &part.join(name_of(&flat)))?;
            Some(name_of(&flat))
        } else {
            None
        };
        let side = read_side(png);
        let text = side["notes"].as_str().unwrap_or("").to_string();
        items.push(Item { png: name_of(png), annotated, side, text, step: None });
    }
    let md = report_md(name, title, &req.description, &rec_section, &items);
    std::fs::write(part.join("report.md"), md).map_err(|e| format!("write report.md: {e}"))
}

fn recording_section(rec: &Path, r: &Recording, name: &str) -> String {
    let file = name_of(rec);
    let mut t = String::from("## The recording\n\n");
    t.push_str(&format!("- **File:** `{file}`, {:.1} seconds of play ({} steps at 60 a second), recorded {}.\n", r.seconds(), r.steps, if r.created_utc.is_empty() { "at an unknown time".to_string() } else { format!("{} UTC", readable_utc(&r.created_utc)) }));
    if !r.build.is_empty() {
        t.push_str(&format!("- **Build:** {}.\n", r.build));
    }
    if !r.ended {
        t.push_str("- It has no end line: the game stopped before the recording did. It plays up to where it stops.\n");
    }
    t.push_str(&format!(
        "- **Watch it happen:** `cargo run --release -- --replay interchange/reports/{name}/{file} --watch` plays it in a window at real time, and each marker's text shows as it passes.\n- **Check it:** `cargo run --release -- --replay interchange/reports/{name}/{file}` plays it with no window, takes a settled shot at every marker, and writes `out/replay/{}/report.json`, which says separately whether the simulation reproduced and whether the look was the same.\n",
        rec.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned())
    ));
    t.push_str("- **If it will not play:** a recording only plays under the simulation rules and terrain generator it was made with, and a later build may refuse it. Nothing below depends on it: the pictures, the words and the poses stand on their own, and `--repro` rebuilds each frame. To play it anyway, check out the build above.\n\n");
    t
}

fn item_section(n: usize, it: &Item, name: &str) -> String {
    let side = &it.side;
    let heading = first_line(&it.text, if side["title"].as_str().is_some_and(|t| !t.trim().is_empty()) { side["title"].as_str().unwrap() } else { "No words, just the picture" });
    let mut t = format!("### {n}. {heading}\n\n");
    let shown = it.annotated.as_deref().unwrap_or(&it.png);
    t.push_str(&format!("![what was marked]({shown})\n\n"));
    if it.text.trim().is_empty() {
        t.push_str("- **What was written:** nothing; the picture is the message.\n");
    } else {
        t.push_str(&format!("- **What was written:** {}\n", it.text.trim().replace('\n', " / ")));
    }
    if let Some(title) = side["title"].as_str().filter(|t| !t.trim().is_empty()) {
        t.push_str(&format!("- **Title:** {title}\n"));
    }
    match it.step {
        Some(s) => t.push_str(&format!("- **When:** step {s}, which is {:.2} seconds into the recording", s as f64 / 60.0)),
        None => t.push_str("- **When:** a photo, not part of a recording"),
    }
    match side["taken_utc"].as_str() {
        Some(u) => t.push_str(&format!(" (taken {} UTC).\n", readable_utc(u))),
        None => t.push_str(".\n"),
    }
    match &it.annotated {
        Some(a) => t.push_str(&format!("- **Picture with the marks:** `{a}`. The clean frame is `{}`.\n", it.png)),
        None => t.push_str(&format!("- **Picture:** `{}` (no marks were drawn).\n", it.png)),
    }
    let marks = annotate::marks_of(side);
    if !marks.is_empty() {
        let (w, h) = (side["annotations"]["image"][0].as_u64().unwrap_or(0), side["annotations"]["image"][1].as_u64().unwrap_or(0));
        t.push_str(&format!("- **What the marks point at,** in pixels of the {w} x {h} picture, from its top left corner:\n"));
        for m in &marks {
            t.push_str(&format!("  - {}\n", m.describe()));
        }
    }
    let p = &side["pose"];
    if let (Some(lat), Some(lon), Some(alt)) = (p["lat"].as_f64(), p["lon"].as_f64(), p["alt_m"].as_f64()) {
        t.push_str(&format!(
            "- **Where:** lat {lat:.5}, lon {lon:.5}, {alt:.1} m above sea level ({:.1} m above the ground), facing {:.1} degrees and {:.1} up, {}; about {:.1} o'clock local time.\n",
            p["alt_agl_m"].as_f64().unwrap_or(0.0),
            p["heading_deg"].as_f64().unwrap_or(0.0),
            p["pitch_deg"].as_f64().unwrap_or(0.0),
            if p["mode"].as_str() == Some("walk") { "walking" } else { "flying" },
            side["sky"]["local_hour"].as_f64().unwrap_or(0.0)
        ));
    }
    if let (Some(exact), Some(clock)) = (p["exact"].as_str(), side["sky"]["clock_exact"].as_str()) {
        t.push_str(&format!("- **Exact teleport,** two lines for a `.play` script:\n\n  ```\n  pose {exact}\n  clock {clock}\n  ```\n\n"));
        t.push_str(&format!("- **The same frame again:** `cargo run --release -- --repro interchange/reports/{name}/{}` writes a script beside the picture and prints the command that runs it. It needs no recording.\n", it.png));
        t.push_str(&format!("- **Stand there by hand:** `{}`\n", photos::at_command(side)));
    } else {
        t.push_str("- **Exact teleport:** this picture has no sidecar with an exact pose, only what its name says.\n");
    }
    if let Some(n) = side["edits"]["chunks"].as_u64().filter(|n| *n > 0) {
        t.push_str(&format!("- **The world had changed:** {n} chunk(s) were edited at this moment. A repro starts from an empty save, so it shows the land without those edits and its edit check fails at the end; the frame is still written. To see the edits, play the recording or use a copy of the save.\n"));
    }
    if !side.is_null() && side["kind"].as_str() != Some("settled") {
        t.push_str("- **The frame is live:** it is what was on screen, so distant land may not have finished loading. A repro's frame is settled, so the distance can differ.\n");
    }
    if side.is_null() {
        t.push_str("- **Made on:** unknown; this picture has no sidecar.\n\n");
    } else {
        t.push_str(&format!("- **Made on:** build {}, terrain generator {}, simulation rules {}, world seed {}.\n\n", side["build"]["git"].as_str().unwrap_or("unknown"), side["generator_version"], side["sim_rules"], side["world"]["seed"]));
    }
    t
}

fn report_md(name: &str, title: &str, description: &str, recording: &str, items: &[Item]) -> String {
    let mut t = format!("# Bug report: {title}\n\n");
    t.push_str(&format!("Made {} UTC with Triangulum build {}. {} marked item(s).\n\n", readable_utc(&crate::replay::utc_stamp()), crate::visual::BUILD, items.len()));
    if !description.trim().is_empty() {
        t.push_str(description.trim());
        t.push_str("\n\n");
    }
    t.push_str(&format!(
        "## How to read this\n\nEach numbered item below is one thing somebody saw and marked: what they wrote, the picture with their marks on it, the exact moment and the exact place. Pictures are in this folder and open with anything.\n\nIf you are here to fix something: the game is at https://github.com/dualmindblade/triangulum, and commands run from its `game/` folder. They assume this folder is at `game/interchange/reports/{name}/`; if it is somewhere else, change the path. Add `--bake DIR` if your checkout has no `game/assets/world/`.\n\n"
    ));
    t.push_str(recording);
    t.push_str("## What was marked\n\n");
    if items.is_empty() {
        t.push_str("Nothing was marked: the recording is the whole report.\n");
    }
    for (i, it) in items.iter().enumerate() {
        t.push_str(&item_section(i + 1, it, name));
    }
    t
}

/// The text `--replay` prints about a recording's markers.
pub fn marker_list(markers: &[Marker]) -> String {
    let mut t = format!("{} marker{}:", markers.len(), if markers.len() == 1 { "" } else { "s" });
    for (i, m) in markers.iter().enumerate() {
        t.push_str(&format!("\n  {}. step {} ({:.1} s): {}", i + 1, m.step, m.seconds(), if m.text.trim().is_empty() { "(no text)".to_string() } else { format!("{:?}", m.text.trim()) }));
    }
    t
}

/// A marker with everything a reader needs and nothing that needs a replay to
/// have played: `report.json`'s `markers`, whether or not it did.
pub fn marker_json(m: &Marker, n: usize, recording: &Path) -> Value {
    let png = markers_dir(recording).join(format!("{}.png", m.shot));
    let flat = photos::annotated_of(&png);
    let side = read_side(&png);
    let text = side["notes"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or(&m.text);
    json!({
        "n": n,
        "step": m.step,
        "time_s": m.seconds(),
        "text": text,
        "shot": m.shot,
        "pose": m.pose,
        "clock": m.clock,
        "recorded_shot": png.exists().then(|| png.display().to_string()),
        "annotated": flat.exists().then(|| flat.display().to_string()),
        "marks": side["annotations"]["marks"],
    })
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;

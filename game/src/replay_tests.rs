//! M2c: replays. Recording and playing back give the same state; the same
//! inputs at different frame rates give the same world; a damaged replay
//! names its first divergence; a replay never touches the real save. The
//! sessions are driven through `Controls`, the windowed app's own input
//! path, with synthetic window events (keys, buttons, mouse motion, the
//! wheel) delivered between frames as winit delivers them.

use super::*;
use crate::controls::Controls;
use crate::engine::Engine;
use crate::grid::WorldConfig;
use crate::mesh::MeshOptions;
use crate::player::{Mode, Player};
use crate::sim::{Sim, SimConfig};
use crate::stream::LodParams;
use crate::world::Registry;
use std::sync::OnceLock;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

const BAKE: &str = "C:/code/triangulum/game/assets/world/seed42";

/// One engine for every test (the bake loads once); sessions get their own
/// edit stores.
fn template() -> (Arc<Engine>, String) {
    static T: OnceLock<(Arc<Engine>, String)> = OnceLock::new();
    T.get_or_init(|| {
        let root = env!("CARGO_MANIFEST_DIR");
        let mut config = WorldConfig::load(&format!("{root}/assets/world.json")).unwrap();
        config.bake = BAKE.into();
        let registry = Arc::new(Registry::load(&format!("{root}/assets/blocks.json")).unwrap());
        let bake: Arc<dyn crate::worldgen::bake::Bake> = Arc::new(crate::worldgen::bake::open(BAKE).expect("world bake"));
        let sha = bake.identity().expect("the bake has an identity").to_string();
        let engine = Engine::build(config, registry, bake, Arc::new(EditStore::new()), LodParams { factor: 5.0, max_inflight: 4 }, MeshOptions { split: true, ao: true }, 2, false, Vec::new(), false).unwrap();
        (Arc::new(engine), sha)
    })
    .clone()
}

/// A fresh session standing on the hills preset, with its own edit store.
pub(crate) fn fresh(save: Option<SaveDir>) -> Session {
    let (t, sha) = template();
    let engine = Arc::new(t.with_edits(Arc::new(EditStore::new())));
    let pr = engine.presets()[0].clone();
    let p = engine.planet;
    let player = Player::new(pr.dir * (p.radius + engine.ground(pr.dir) + 2.0), pr.heading_deg, pr.pitch_deg, Mode::Walk, p.block_size);
    let mut s = Session::new(engine, player, save, sha, Sim::new(SimConfig::default()));
    s.teleport(pr.dir, None);
    s.sky = Clock { seconds: 1000.0, speed: 1.0 };
    s
}

fn temp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("triangulum-replay-test-{tag}-{}.replay", std::process::id()))
}

/// A window event, as the app hands it to `Controls`.
#[derive(Clone, Copy, Debug)]
enum Ev {
    Key(KeyCode, bool),
    Button(MouseButton, bool),
    Motion(f64, f64),
    Scroll(f64),
}

/// A session's worth of input: walk, look around, dig (a held button
/// repeats), place sand that falls, pour water, fly, change the hotbar,
/// the fly speed, the sky speed and the water view. Every event is at an
/// even step, so a 30 fps window (two steps a frame) can deliver it at the
/// same step as a 144 fps one.
fn timeline() -> Vec<(u64, Ev)> {
    use Ev::*;
    let mut v = vec![(10, Key(KeyCode::KeyW, true))];
    for s in (12..150).step_by(4) {
        v.push((s, Motion(if s == 68 { 25.0 } else { 3.0 }, if s % 8 == 0 { 1.5 } else { -1.0 })));
    }
    v.extend([
        (60, Key(KeyCode::ShiftLeft, true)),
        (90, Key(KeyCode::ShiftLeft, false)),
        (150, Key(KeyCode::KeyW, false)),
        // Water from the bucket onto the ground a few metres ahead: it
        // spreads over the open ground.
        (150, Motion(0.0, 180.0)),
        (150, Key(KeyCode::Digit9, true)),
        (150, Key(KeyCode::Digit9, false)),
        (152, Button(MouseButton::Right, true)),
        (152, Button(MouseButton::Right, false)),
        // Look further down and dig with a held button (it repeats).
        (154, Key(KeyCode::Digit1, true)),
        (154, Key(KeyCode::Digit1, false)),
        (154, Motion(0.0, 120.0)),
        (154, Button(MouseButton::Left, true)),
        (214, Button(MouseButton::Left, false)),
        // A quick tap of Space: pressed and released between two steps.
        (216, Key(KeyCode::Space, true)),
        (216, Key(KeyCode::Space, false)),
        // Sand into the dig and on the ground further out.
        (220, Key(KeyCode::Digit4, true)),
        (220, Key(KeyCode::Digit4, false)),
        (222, Button(MouseButton::Right, true)),
        (224, Button(MouseButton::Right, false)),
        (230, Motion(0.0, -120.0)),
        (232, Button(MouseButton::Right, true)),
        (234, Button(MouseButton::Right, false)),
        // Time, view and fly mode.
        (250, Key(KeyCode::BracketRight, true)),
        (250, Key(KeyCode::BracketRight, false)),
        (260, Key(KeyCode::KeyF, true)),
        (262, Key(KeyCode::KeyF, false)),
        (280, Key(KeyCode::KeyF, true)),
        (280, Key(KeyCode::KeyF, false)),
        (300, Key(KeyCode::KeyG, true)),
        (300, Key(KeyCode::KeyG, false)),
        (302, Scroll(2.0)),
        (304, Key(KeyCode::Space, true)),
        (330, Key(KeyCode::Space, false)),
        (332, Key(KeyCode::KeyD, true)),
        (380, Key(KeyCode::KeyD, false)),
    ]);
    v.sort_by_key(|e| e.0);
    v
}

/// Run a timeline through `Controls` at a frame pacing: before each frame,
/// deliver the events due by now (as winit delivers events between
/// redraws), then run the frame. `dt(n)` is frame n's length.
fn drive(s: &mut Session, events: &[(u64, Ev)], end: u64, mut dt: impl FnMut(usize) -> f64) {
    let mut c = Controls::default();
    let mut next = 0;
    let mut frame = 0;
    while s.steps() < end {
        while next < events.len() && events[next].0 <= s.steps() {
            match events[next].1 {
                Ev::Key(k, down) => {
                    c.key(k, down);
                }
                Ev::Button(b, down) => c.button(b, down),
                Ev::Motion(x, y) => c.motion(x, y),
                Ev::Scroll(l) => c.scroll(l),
            }
            next += 1;
        }
        c.frame(s, dt(frame));
        frame += 1;
    }
}

/// Record a session driven at a pacing; returns the recording's summary
/// and the session's final state hash.
fn record(tag: &str, save: Option<SaveDir>, dt: impl FnMut(usize) -> f64) -> (Summary, String, Session) {
    let mut s = fresh(save);
    s.start_recording(&temp(tag), "test").unwrap();
    drive(&mut s, &timeline(), 400, dt);
    let summary = s.stop_recording().unwrap();
    let hash = s.sim_hash();
    (summary, hash, s)
}

fn play(rep: &Replay, tag: &str) -> (Report, Session) {
    let (t, sha) = template();
    let block = t.planet.block_size;
    let template_session = Session::new(t, Player::new(DVec3::X * 1e7, 0.0, 0.0, Mode::Fly, block), None, sha, Sim::new(SimConfig::default()));
    let scratch = scratch_dir(&format!("test-{tag}-{}", std::process::id()));
    let mut s = session_from(&template_session, rep, &scratch).unwrap();
    let report = Runner::new(rep, &s).run(&mut s);
    let _ = std::fs::remove_dir_all(&scratch);
    (report, s)
}

/// The event lines of a replay file (no header or snapshot).
fn events_of(path: &Path) -> Vec<String> {
    read(path).unwrap().lines.into_iter().map(|l| l.text).collect()
}

#[test]
fn record_then_replay_gives_the_same_state() {
    let (summary, hash, s) = record("roundtrip", None, |_| STEP);
    assert_eq!(summary.steps, 400);
    assert!(summary.checks >= 6, "a check a second and one at the end: {}", summary.checks);
    assert_eq!(summary.notes, 0, "everything went through the driver");
    // The session did something worth reproducing.
    let st = &s.sim.stats;
    assert!(s.engine.edits.len() > 0 && st.water_placed > 0 && st.water_moved > 0, "edits {} water placed {} moved {}", s.engine.edits.len(), st.water_placed, st.water_moved);
    assert_eq!(s.player.mode, Mode::Fly);
    let rep = read(&summary.path).unwrap();
    assert!(rep.ended && rep.warnings.is_empty(), "{:?}", rep.warnings);
    let (report, back) = play(&rep, "roundtrip");
    assert!(report.ok(), "{}", report.text());
    assert_eq!(report.matched, report.checks);
    assert_eq!(report.checks as u64, summary.checks);
    assert_eq!(report.steps, 400);
    let (a, b) = report.end_sha.clone().unwrap();
    assert_eq!(a, b);
    assert_eq!(a, hash, "the end line holds the session's hash");
    assert_eq!(pose_text(&back.player), pose_text(&s.player), "the player ends exactly where it did");
    assert_eq!(back.sky.seconds.to_bits(), s.sky.seconds.to_bits());
    let _ = std::fs::remove_file(&summary.path);
}

/// M2-hardening changed what the simulation does at the edge of the active
/// region, so a replay names the rules it was recorded under
/// (`sim::RULES_VERSION`) and one recorded under other rules (an M2c file
/// has none: version 1) is refused with the reason, not played into a
/// divergence.
#[test]
fn a_replay_recorded_under_other_simulation_rules_is_refused() {
    let mut s = fresh(None);
    let path = temp("rules");
    s.start_recording(&path, "rules").unwrap();
    drive(&mut s, &timeline()[..3], 30, |_| STEP);
    s.stop_recording().unwrap();
    let mut rep = read(&path).unwrap();
    assert_eq!(rep.header.sim_rules, crate::sim::RULES_VERSION);
    let (t, sha) = template();
    let block = t.planet.block_size;
    let template_session = Session::new(t, Player::new(DVec3::X * 1e7, 0.0, 0.0, Mode::Fly, block), None, sha, Sim::new(SimConfig::default()));
    assert!(compatibility(&template_session, &rep.header).is_ok());
    // An M2c file: no sim_rules in its header.
    let text = serde_json::to_string(&rep.header).unwrap().replace(&format!("\"sim_rules\":{},", crate::sim::RULES_VERSION), "");
    assert!(!text.contains("sim_rules"));
    rep.header = serde_json::from_str(&text).unwrap();
    assert_eq!(rep.header.sim_rules, 1);
    let err = compatibility(&template_session, &rep.header).unwrap_err();
    assert!(err.contains("simulation rules version 1 vs"), "{err}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_same_inputs_at_different_frame_rates_give_the_same_world() {
    // 144 fps: at most one step a frame. 30 fps: exactly two.
    let (fast, fast_hash, _) = record("fps144", None, |_| STEP * 60.0 / 144.0);
    let (slow, slow_hash, _) = record("fps30", None, |_| 2.0 * STEP);
    assert_eq!(fast_hash, slow_hash, "the same world");
    assert_eq!(events_of(&fast.path), events_of(&slow.path), "the same inputs at the same steps, the same checks");
    // A ragged frame rate (stalls up to 0.2 s): its events land where its
    // frames let them, and its replay still reproduces it exactly.
    let mut seed = 12345u64;
    let (ragged, ragged_hash, _) = record("ragged", None, move |_| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        0.002 + (seed >> 40) as f64 / (1u64 << 24) as f64 * 0.2
    });
    for (tag, s, h) in [("fps144", &fast, &fast_hash), ("fps30", &slow, &slow_hash), ("ragged", &ragged, &ragged_hash)] {
        let (report, _) = play(&read(&s.path).unwrap(), tag);
        assert!(report.ok(), "{tag}: {}", report.text());
        assert_eq!(report.end_sha.as_ref().unwrap().1, *h, "{tag}");
        let _ = std::fs::remove_file(&s.path);
    }
}

/// What recording costs the app: the snapshot at F10 and a check (once a
/// second) with many edited chunks, cold and with nothing changed since.
/// `cargo test --release -- --ignored replay_cost --nocapture`.
#[test]
#[ignore]
fn replay_cost() {
    let mut s = fresh(None);
    let p = s.engine.planet;
    let stone = s.engine.registry.id("stone").unwrap();
    let g = crate::player::Frame::at(&p, s.player.pos);
    let (ci, cj, ck) = g.feet_cell();
    let mut n = 0;
    for a in 0..20i64 {
        for b in 0..20i64 {
            if let Some(bp) = s.engine.world.resolve_block(g.face, ci + 32 * a, cj + 32 * b, ck + 40) {
                n += s.engine.world.set_block(bp, stone) as usize;
            }
        }
    }
    let t = std::time::Instant::now();
    s.start_recording(&temp("cost"), "cost").unwrap();
    let start_ms = t.elapsed().as_secs_f64() * 1e3;
    let mut d = ChunkDigests::default();
    let t = std::time::Instant::now();
    let _ = measure(&s, &mut d);
    let cold = t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    let _ = measure(&s, &mut d);
    let warm = t.elapsed().as_secs_f64() * 1e3;
    let summary = s.stop_recording().unwrap();
    println!("{n} edited chunks: start recording {start_ms:.1} ms ({:.0} KB file); a check {cold:.2} ms cold, {warm:.3} ms with nothing changed", summary.bytes as f64 / 1e3);
    let _ = std::fs::remove_file(&summary.path);
}

/// Review 2 #16: a key pressed and released between two frames (a quick
/// tap at 30 fps) used to be missed, since the held keys were sampled once
/// a frame; it now counts as held for one step at any frame rate.
#[test]
fn a_tap_shorter_than_a_frame_still_counts() {
    for (fps, dt) in [(30, 2.0 * STEP), (144, STEP * 60.0 / 144.0)] {
        let mut s = fresh(None);
        assert!(s.player.on_ground);
        let events = [(4, Ev::Key(KeyCode::Space, true)), (4, Ev::Key(KeyCode::Space, false))];
        drive(&mut s, &events, 12, |_| dt);
        assert!(!s.player.on_ground && s.player.vel.dot(s.player.pos.normalize()) > 0.0, "{fps} fps: the tap jumped");
    }
}

/// Write a copy of a replay with one event line changed.
fn corrupt(path: &Path, tag: &str, f: impl Fn(&str) -> Option<String>) -> PathBuf {
    let bytes = std::fs::read(path).unwrap();
    let rep = read(path).unwrap();
    let head: usize = {
        // The event section starts after the two header lines and the snapshot.
        let first = bytes.iter().position(|&b| b == b'\n').unwrap() + 1;
        let second = first + bytes[first..].iter().position(|&b| b == b'\n').unwrap() + 1;
        second + rep.header.files.iter().map(|f| f.bytes as usize).sum::<usize>()
    };
    let text = String::from_utf8(bytes[head..].to_vec()).unwrap();
    let mut changed = 0;
    let lines: Vec<String> = text
        .lines()
        .map(|l| match f(l) {
            Some(n) => {
                changed += 1;
                n
            }
            None => l.to_string(),
        })
        .collect();
    assert_eq!(changed, 1, "exactly one line corrupted");
    let mut out = bytes[..head].to_vec();
    out.extend_from_slice((lines.join("\n") + "\n").as_bytes());
    let p = temp(tag);
    std::fs::write(&p, out).unwrap();
    p
}

#[test]
fn a_corrupted_replay_reports_its_first_divergence() {
    let (summary, _, _) = record("corrupt-src", None, |_| STEP);
    // 1. One mouse movement a little off: the player turns more at step 68,
    // walks elsewhere, and the first check after it (step 120) says so.
    let p = corrupt(&summary.path, "corrupt-look", |l| (l == "@68 look 25.0 -1.0").then(|| "@68 look 26.0 -1.0".to_string()));
    let (report, _) = play(&read(&p).unwrap(), "corrupt-look");
    let d = report.first.clone().expect("a divergence");
    assert_eq!(d.step, 120, "{}", report.text());
    assert!(d.what.iter().any(|w| w.starts_with("player:") && w.contains("position")), "{:?}", d.what);
    assert!(report.later_mismatches > 0);
    let _ = std::fs::remove_file(&p);
    // 2. A held left button turned into a right one: blocks placed instead
    // of dug. The next check names the chunk.
    let p = corrupt(&summary.path, "corrupt-press", |l| (l == "@154 press break").then(|| "@154 press place".to_string()));
    let (report, _) = play(&read(&p).unwrap(), "corrupt-press");
    let d = report.first.clone().expect("a divergence");
    assert_eq!(d.step, 180, "{}", report.text());
    assert!(d.what.iter().any(|w| w.starts_with("world:") && w.contains(" differs")), "{:?}", d.what);
    let _ = std::fs::remove_file(&p);
    // 3. A damaged check (its world digest): reported at that check.
    let p = corrupt(&summary.path, "corrupt-check", |l| {
        if !l.starts_with("@240 check") {
            return None;
        }
        let i = l.find(" world=").unwrap() + 7;
        let flipped = if &l[i..i + 1] == "0" { "1" } else { "0" };
        Some(format!("{}{}{}", &l[..i], flipped, &l[i + 1..]))
    });
    let (report, _) = play(&read(&p).unwrap(), "corrupt-check");
    let d = report.first.clone().expect("a divergence");
    assert_eq!(d.step, 240);
    assert!(d.what.iter().any(|w| w.starts_with("world: digest")), "{:?}", d.what);
    assert_eq!(report.later_mismatches, 0, "only that check");
    let _ = std::fs::remove_file(&p);
    // 4. A damaged snapshot is refused before anything plays.
    let mut bytes = std::fs::read(&summary.path).unwrap();
    let first = bytes.iter().position(|&b| b == b'\n').unwrap() + 1;
    let second = first + bytes[first..].iter().position(|&b| b == b'\n').unwrap() + 1;
    bytes[second + 3] ^= 0xFF;
    let p = temp("corrupt-snapshot");
    std::fs::write(&p, &bytes).unwrap();
    assert!(read(&p).err().unwrap().contains("damaged"));
    let _ = std::fs::remove_file(&p);
    // 5. A recording cut off mid-line (a crash) plays to its last event.
    let bytes = std::fs::read(&summary.path).unwrap();
    let cut = bytes.len() - 40;
    let p = temp("corrupt-cut");
    std::fs::write(&p, &bytes[..cut]).unwrap();
    let rep = read(&p).unwrap();
    assert!(!rep.ended && rep.warnings.len() == 2, "{:?}", rep.warnings);
    let (report, _) = play(&rep, "corrupt-cut");
    assert!(report.ok() && report.end_sha.is_none(), "{}", report.text());
    let _ = std::fs::remove_file(&p);
    let _ = std::fs::remove_file(&summary.path);
}

/// Every file in a directory with its bytes, sorted.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let b = std::fs::read(&p).unwrap();
                out.push((p, b));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_replay_never_touches_the_real_save() {
    let real = std::env::temp_dir().join(format!("triangulum-replay-real-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&real);
    // A session on a real save: edits before the recording, saved by F10.
    let mut s = fresh(Some(SaveDir::new(&real)));
    s.player.pitch = -1.2;
    assert!(s.break_block().is_some());
    let path = temp("realsave");
    s.start_recording(&path, "test").unwrap();
    assert!(SaveDir::new(&real).exists(), "starting the recording saved first");
    drive(&mut s, &timeline(), 400, |_| STEP);
    s.stop_recording().unwrap();
    s.save_now().unwrap();
    let before = tree(&real);
    assert!(!before.is_empty());
    let rep = read(&path).unwrap();
    assert!(rep.header.save_ref.as_deref().is_some_and(|r| r.contains("triangulum-replay-real-save")), "the header names the save it came from");
    // The replay plays in its scratch save and leaves the real one exactly
    // as it was.
    let (report, back) = play(&rep, "realsave");
    assert!(report.ok(), "{}", report.text());
    let scratch = back.save.as_ref().unwrap().path.clone();
    assert!(scratch.components().any(|c| c.as_os_str() == "_replay"), "{}", scratch.display());
    assert_ne!(scratch, real);
    assert_eq!(tree(&real), before, "not one byte of the real save changed");
    // It refuses to play anywhere but a _replay scratch directory, and
    // never in the session's own save.
    let template_session = fresh(Some(SaveDir::new(&real)));
    assert!(session_from(&template_session, &rep, &real).err().unwrap().contains("_replay"));
    let own = scratch_dir("own");
    let template_own = fresh(Some(SaveDir::new(&own)));
    assert!(session_from(&template_own, &rep, &own).err().unwrap().contains("own save"));
    assert_eq!(tree(&real), before);
    let _ = std::fs::remove_dir_all(&real);
    let _ = std::fs::remove_file(&path);
}

/// Bug round #11: everything the T window does goes through recorded inputs
/// at a step boundary. A teleport (map point, typed coordinates, a photo with
/// its clock) and a place, chosen during a recording, land in the file as
/// `pose`, `sky` and `preset`, and the replay ends exactly where the session
/// did, with nothing the recorder could not capture.
#[test]
fn the_t_windows_teleports_and_places_replay_exactly() {
    use crate::menu::{Go, PhotoGo, TimeGo, actions};
    let settings = crate::sky::Settings::default();
    let mut s = fresh(None);
    let hills = s.player.pos.normalize();
    let (hills_lat, hills_lon) = crate::grid::Planet::lat_lon_deg(hills);
    let path = temp("window");
    s.start_recording(&path, "test").unwrap();
    let mut c = Controls::default();
    let run = |c: &mut Controls, s: &mut Session, n: usize| {
        for _ in 0..n {
            c.frame(s, STEP);
        }
    };
    c.key(KeyCode::KeyW, true);
    run(&mut c, &mut s, 20);
    // The window opens: the key lets go, and the walker stops being driven.
    c.release_all();
    run(&mut c, &mut s, 4);
    let photo = pose_text(&s.player);
    // Typed coordinates with an altitude.
    let go = Go::Point { lat: 12.5, lon: -40.25, alt_m: Some(900.0) };
    for a in actions(&go, &s, &settings) {
        c.queue(a);
    }
    run(&mut c, &mut s, 10);
    assert_eq!(s.player.mode, Mode::Fly);
    // A place.
    for a in actions(&Go::Place(1), &s, &settings) {
        c.queue(a);
    }
    run(&mut c, &mut s, 10);
    // A map point: on the ground, walking.
    let go = Go::Point { lat: hills_lat + 0.01, lon: hills_lon, alt_m: None };
    for a in actions(&go, &s, &settings) {
        c.queue(a);
    }
    run(&mut c, &mut s, 30);
    assert_eq!(s.player.mode, Mode::Walk);
    assert!(s.player.on_ground, "it stands on the ground");
    // A photo, exact, with its time of day.
    let go = Go::Photo(PhotoGo { exact: Some(photo.clone()), lat: hills_lat, lon: hills_lon, alt_m: 50.0, heading_deg: 0.0, pitch_deg: 0.0, time: Some(TimeGo::Clock(5000.0)) });
    for a in actions(&go, &s, &settings) {
        c.queue(a);
    }
    run(&mut c, &mut s, 30);
    let summary = s.stop_recording().unwrap();
    assert_eq!(summary.notes, 0, "the recorder missed nothing");
    let ev = events_of(&summary.path);
    assert!(ev.iter().filter(|l| l.contains(" pose ")).count() >= 3, "{ev:?}");
    assert!(ev.iter().any(|l| l.contains(" preset 1")), "{ev:?}");
    assert!(ev.iter().any(|l| l.contains(" sky 5000.0 ")), "{ev:?}");
    let (report, back) = play(&read(&summary.path).unwrap(), "window");
    assert!(report.ok(), "{}", report.text());
    assert_eq!(pose_text(&back.player), pose_text(&s.player), "the player ends exactly where it did");
    assert_eq!(back.sky.seconds.to_bits(), s.sky.seconds.to_bits());
    let _ = std::fs::remove_file(&summary.path);
}

/// The replay format did not change: `preset` still means what P meant, the
/// recordings Andrew made still parse, line for line, and a replay with a
/// `preset` in it plays. (Playing the five real ones is the `--replay` check
/// in the findings: a later mission's rule change may refuse them.)
#[test]
fn old_replays_still_parse_and_preset_still_plays() {
    let mut s = fresh(None);
    let path = temp("old-preset");
    s.start_recording(&path, "test").unwrap();
    s.step(&Input::default(), STEP);
    s.apply(Action::Preset(2)).unwrap();
    s.step(&Input::default(), STEP);
    let summary = s.stop_recording().unwrap();
    let rep = read(&summary.path).unwrap();
    assert!(rep.lines.iter().any(|l| l.text == "@1 preset 2"), "{:?}", events_of(&summary.path));
    let (report, back) = play(&rep, "old-preset");
    assert!(report.ok(), "{}", report.text());
    assert_eq!(pose_text(&back.player), pose_text(&s.player));
    let _ = std::fs::remove_file(&summary.path);
    let dir = Path::new("C:/code/triangulum/game/replays");
    let mut read_some = 0;
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.path().extension().is_some_and(|x| x == "replay") {
            let rep = read(&e.path()).unwrap_or_else(|err| panic!("{}: {err}", e.path().display()));
            assert!(!rep.lines.is_empty());
            read_some += 1;
        }
    }
    println!("{read_some} recordings parsed");
}

/// P takes a photo now: it queues nothing, so it moves nobody and no replay
/// sees it. The places moved to the T window.
#[test]
fn p_no_longer_moves_the_player() {
    // The same ten steps with and without P: the same player, the same file.
    let run = |events: &[(u64, Ev)], tag: &str| {
        let mut s = fresh(None);
        let path = temp(tag);
        s.start_recording(&path, "test").unwrap();
        drive(&mut s, events, 10, |_| STEP);
        let summary = s.stop_recording().unwrap();
        let lines = events_of(&summary.path);
        let _ = std::fs::remove_file(&summary.path);
        (pose_text(&s.player), lines)
    };
    let (with, lines) = run(&[(2, Ev::Key(KeyCode::KeyP, true)), (2, Ev::Key(KeyCode::KeyP, false))], "p-key");
    let (without, plain) = run(&[], "no-p-key");
    assert_eq!(with, without);
    assert_eq!(lines, plain);
    assert!(!lines.iter().any(|l| l.contains("preset")));
    let _ = std::fs::remove_file(&summary.path);
}

/// Taking a photo reads the session and changes nothing: the same session with
/// and without photos taken along the way gives the same world, the same file
/// of inputs and the same hash.
#[test]
fn a_photo_disturbs_nothing() {
    let info = || crate::photos::ShotInfo { kind: "settled", settled: true, settle_s: 0.0, settle_frames: 0, width: 4, height: 2, hud: false, mesh_view: Default::default(), preview_world: false, frame_ms: None };
    let (mut hashes, mut lines) = (Vec::new(), Vec::new());
    for shoot in [false, true] {
        let mut s = fresh(None);
        s.start_recording(&temp(if shoot { "photo-on" } else { "photo-off" }), "test").unwrap();
        let settings = crate::sky::Settings::default();
        let look = crate::look::Look::defaults(&settings, s.engine.planet.radius);
        let mut digests = ChunkDigests::default();
        for (n, chunk) in timeline().chunk_by(|a, b| a.0 == b.0).enumerate() {
            let end = chunk[0].0 + 1;
            drive(&mut s, chunk, end, |_| STEP);
            if shoot && n % 5 == 0 {
                let cam = s.player.camera();
                let v = crate::photos::sidecar(&s, &cam, &settings, &look, &mut digests, Path::new("shot_x.png"), &info());
                assert_eq!(v["format"], crate::photos::FORMAT);
                assert_eq!(v["pose"]["exact"], pose_text(&s.player));
                assert_eq!(v["world"]["seed"], s.engine.config.seed);
                assert_eq!(v["generator_version"], crate::world::save::GENERATOR_VERSION);
                assert_eq!(v["edits"]["chunks"], s.engine.edits.len());
                assert!(v["edits"]["hash"].as_str().is_some_and(|h| h.len() == 16));
                assert!(!v["build"]["git"].as_str().unwrap_or("").is_empty());
            }
        }
        let summary = s.stop_recording().unwrap();
        hashes.push(s.sim_hash());
        lines.push(events_of(&summary.path));
        let _ = std::fs::remove_file(&summary.path);
    }
    assert_eq!(hashes[0], hashes[1]);
    assert_eq!(lines[0], lines[1]);
}

/// Where a teleport lands (§15.7): it reads the session and leaves it as it
/// was; an altitude hovers, clamped to the ground and the ceiling; no altitude
/// stands on the ground, or hovers over water too deep to stand in.
#[test]
fn a_teleport_lands_where_it_should() {
    use crate::session::{TELEPORT_MAX_ALT_M, TELEPORT_MIN_AGL_M};
    let s = fresh(None);
    let before = pose_text(&s.player);
    let radius = s.engine.planet.radius;
    let land = s.player.pos.normalize();
    let ground = s.engine.ground(land);
    let high = s.teleport_pose(land, Some(ground + 500.0));
    assert_eq!(high.mode, Mode::Fly);
    assert!((high.pos.length() - radius - ground - 500.0).abs() < 1e-6);
    let low = s.teleport_pose(land, Some(ground - 900.0));
    assert!((low.pos.length() - radius - ground - TELEPORT_MIN_AGL_M).abs() < 1e-6, "never into the ground");
    let top = s.teleport_pose(land, Some(1e12));
    assert!((top.pos.length() - radius - TELEPORT_MAX_ALT_M).abs() < 1e-3);
    let stand = s.teleport_pose(land, None);
    assert_eq!(stand.mode, Mode::Walk);
    assert!(stand.on_ground && stand.vel == DVec3::ZERO);
    // The deepest sea: not a walk on the bottom.
    let sea = s.engine.places.trench.0;
    let over = s.teleport_pose(sea, None);
    assert_eq!(over.mode, Mode::Fly);
    assert!(over.pos.length() > radius, "above the surface");
    assert_eq!(pose_text(&s.player), before, "looking where to go moves nobody");
}

#[test]
fn harness_pokes_are_recorded_as_poses_and_replay_exactly() {
    // The harness moves the player directly (teleports, look commands);
    // the recorder notices and writes the exact pose, so they replay too.
    let mut s = fresh(None);
    let path = temp("pokes");
    s.start_recording(&path, "test").unwrap();
    let input = Input { forward: true, ..Default::default() };
    for _ in 0..30 {
        s.step(&input, STEP);
    }
    s.player.heading += 1.0;
    s.teleport(s.player.pos.normalize(), Some(80.0));
    s.sky.seconds += 500.0;
    for _ in 0..90 {
        s.step(&Input::default(), STEP);
    }
    let summary = s.stop_recording().unwrap();
    assert_eq!(summary.notes, 0);
    let ev = events_of(&summary.path);
    assert!(ev.iter().any(|l| l.starts_with("@30 pose ")), "{ev:?}");
    assert!(ev.iter().any(|l| l.starts_with("@30 sky ")), "{ev:?}");
    let (report, back) = play(&read(&summary.path).unwrap(), "pokes");
    assert!(report.ok(), "{}", report.text());
    assert_eq!(pose_text(&back.player), pose_text(&s.player));
    // A world change behind the driver's back is written down as a note
    // (the replay cannot reproduce it, and says where).
    let mut s = fresh(None);
    s.start_recording(&path, "test").unwrap();
    s.step(&Input::default(), STEP);
    s.player.pitch = -1.2;
    assert!(s.break_block().is_some());
    s.step(&Input::default(), STEP);
    let summary = s.stop_recording().unwrap();
    assert_eq!(summary.notes, 1);
    let (report, _) = play(&read(&summary.path).unwrap(), "pokes-note");
    assert!(report.notes.iter().any(|n| n.contains("outside the recorded inputs")), "{:?}", report.notes);
    assert!(!report.ok(), "the world differs at the end");
    let _ = std::fs::remove_file(&summary.path);
}

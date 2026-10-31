//! B6: F9 during a recording. A marker is a `note` at its step and nothing else:
//! the session goes on as if it had never been there. Every test goes through
//! `Controls` and `Gate` with synthetic window events, as `replay_tests.rs` does,
//! because that is the path the app's window events take.

use super::*;
use crate::annotate::{Gate, Ink, Mark};
use crate::report::{Grab, Moment};
use crate::sky::Settings;

/// What happens inside one open annotator.
#[derive(Clone, Default)]
struct Inside {
    /// Open at the first frame with this many steps done.
    at: u64,
    /// Frames drawn while it's open.
    frames: usize,
    /// Window events while it's open, as a hand would make them.
    events: Vec<Ev>,
    text: &'static str,
}

fn deliver(c: &mut Controls, gate: &mut Gate, ev: Ev) {
    match ev {
        Ev::Key(k, down) => {
            gate.key(c, k, down);
        }
        Ev::Button(b, down) => gate.button(c, b, down),
        Ev::Motion(x, y) => gate.motion(c, x, y),
        Ev::Scroll(l) => gate.scroll(c, l),
    }
}

/// `drive`, with F9 pressed at each `Inside`: grab, gate open, frames with no
/// steps, gate shut, marker written by the real `Moment`. The camera is the
/// one the app grabs, ahead of the step by any pending mouse motion.
fn drive_marked(s: &mut Session, events: &[(u64, Ev)], end: u64, marks: &[Inside], mut dt: impl FnMut(usize) -> f64) {
    let (mut c, mut gate) = (Controls::default(), Gate::default());
    let settings = Settings::default();
    let look = crate::look::Look::defaults(&settings, s.engine.planet.radius);
    let mut digests = ChunkDigests::default();
    let (mut next, mut frame, mut mark) = (0, 0, 0);
    while s.steps() < end {
        while next < events.len() && events[next].0 <= s.steps() {
            deliver(&mut c, &mut gate, events[next].1);
            next += 1;
        }
        if mark < marks.len() && marks[mark].at <= s.steps() {
            let m = &marks[mark];
            mark += 1;
            let before = s.steps();
            let cam = c.camera(&s.player);
            let moment = Moment::grab(Grab { session: s, cam: &cam, settings: &settings, look: &look, digests: &mut digests, rgba: vec![9; 16], width: 2, height: 2, mesh_view: Default::default(), preview_world: false, frame_ms: None });
            gate.open();
            for &ev in &m.events {
                deliver(&mut c, &mut gate, ev);
            }
            for _ in 0..m.frames {
                assert_eq!(gate.frame(&mut c, s, 0.1), 0, "no steps with the annotator open");
            }
            assert_eq!(s.steps(), before, "the game stood still");
            gate.close();
            let marks = vec![Mark::Box { colour: Ink::Red, from: [0.0, 0.0], to: [1.0, 1.0] }];
            let _ = moment.commit(s, marks, m.text.to_string());
        }
        gate.frame(&mut c, s, dt(frame));
        frame += 1;
    }
}

fn marked_session(tag: &str, events: &[(u64, Ev)], end: u64, marks: &[Inside], dt: impl FnMut(usize) -> f64) -> (Summary, Session) {
    let mut s = fresh(None);
    s.start_recording(&temp(tag), "test").unwrap();
    drive_marked(&mut s, events, end, marks, dt);
    let summary = s.stop_recording().unwrap();
    (summary, s)
}

fn lines_without_markers(path: &Path) -> Vec<String> {
    events_of(path).into_iter().filter(|l| !l.contains(" note marker ")).collect()
}

/// The whole of B6's promise: the timeline with three markers, one while W is
/// held, one while the left button is held and repeating, one in a quiet moment,
/// gives the same file of inputs and the same world as the timeline with none.
/// A held key and a held button carry on as if nothing had happened, and the
/// recording replays exactly, listing its markers.
#[test]
fn markers_change_nothing() {
    let (plain, plain_session) = marked_session("plain", &timeline(), 400, &[], |_| STEP);
    let plain_hash = plain_session.sim_hash_copy();
    let marks = [Inside { at: 40, frames: 300, text: "W held", ..Default::default() }, Inside { at: 170, frames: 5, text: "digging", ..Default::default() }, Inside { at: 240, frames: 2000, text: "quiet, for a long while", ..Default::default() }];
    let (summary, s) = marked_session("three", &timeline(), 400, &marks, |_| STEP);
    assert_eq!(s.sim_hash_copy(), plain_hash, "the same world");
    assert_eq!(pose_text(&s.player), pose_text(&plain_session.player));
    assert_eq!(lines_without_markers(&summary.path), lines_without_markers(&plain.path), "the same inputs at the same steps");
    assert_eq!(summary.markers, 3);
    assert_eq!(summary.notes, 0, "a marker isn't something the recorder could not capture");
    let rep = read(&summary.path).unwrap();
    let m = rep.markers();
    assert_eq!(m.iter().map(|m| (m.step, m.text.as_str())).collect::<Vec<_>>(), vec![(40, "W held"), (170, "digging"), (240, "quiet, for a long while")]);
    assert!(m.iter().all(|m| m.shot.starts_with("marker0") && !m.pose.is_empty() && !m.clock.is_empty()), "{m:?}");
    let (report, back) = play(&rep, "three");
    assert!(report.ok(), "{}", report.text());
    assert!(report.notes.is_empty(), "markers aren't warnings: {:?}", report.notes);
    assert_eq!(report.matched, report.checks);
    assert_eq!(pose_text(&back.player), pose_text(&s.player));
    let _ = std::fs::remove_file(&plain.path);
    let _ = std::fs::remove_file(&summary.path);
}

/// Everything a hand does in the annotator stays there: typing a sentence full
/// of game keys, clicking, dragging, the wheel, the mouse moving. (At a step
/// with nothing held: a held key can't be pressed again without letting go,
/// and letting go is the next test.)
#[test]
fn nothing_done_in_the_annotator_reaches_the_game() {
    use KeyCode::*;
    let mut typed = Vec::new();
    for k in [KeyW, KeyA, KeyS, KeyD, Space, KeyG, KeyF, KeyR, Digit1, Digit9, BracketLeft, BracketRight, ShiftLeft, ControlLeft, KeyP, KeyT, F10, F1, Escape, Enter] {
        typed.push(Ev::Key(k, true));
        typed.push(Ev::Key(k, false));
    }
    typed.extend([Ev::Button(MouseButton::Left, true), Ev::Motion(300.0, -40.0), Ev::Scroll(4.0), Ev::Button(MouseButton::Left, false), Ev::Button(MouseButton::Right, true), Ev::Button(MouseButton::Right, false)]);
    let (plain, a) = marked_session("hands-plain", &timeline(), 400, &[], |_| STEP);
    let (hands, b) = marked_session("hands", &timeline(), 400, &[Inside { at: 226, frames: 30, events: typed, text: "w a s d" }], |_| STEP);
    assert_eq!(lines_without_markers(&hands.path), lines_without_markers(&plain.path));
    assert_eq!(a.sim_hash_copy(), b.sim_hash_copy());
    let _ = std::fs::remove_file(&plain.path);
    let _ = std::fs::remove_file(&hands.path);
}

/// A key let go in the annotator is let go at that step: the same file, marker
/// aside, as letting go at that step with no annotator. The same for a button
/// held and repeating.
#[test]
fn what_is_let_go_in_there_is_let_go() {
    // W down at 10, up inside an annotator at 60.
    let walk = [(10, Ev::Key(KeyCode::KeyW, true))];
    let (inside, a) = marked_session("let-go", &walk, 120, &[Inside { at: 60, frames: 20, events: vec![Ev::Key(KeyCode::KeyW, false)], text: "" }], |_| STEP);
    let (outside, b) = marked_session("let-go-plain", &[(10, Ev::Key(KeyCode::KeyW, true)), (60, Ev::Key(KeyCode::KeyW, false))], 120, &[], |_| STEP);
    assert_eq!(lines_without_markers(&inside.path), lines_without_markers(&outside.path));
    assert_eq!(pose_text(&a.player), pose_text(&b.player));
    assert!(events_of(&inside.path).iter().any(|l| l == "@60 keys -"), "{:?}", events_of(&inside.path));
    // The left button down and repeating at 20, up inside at 50.
    let dig = [(0, Ev::Motion(0.0, 300.0)), (20, Ev::Button(MouseButton::Left, true))];
    let (inside, a) = marked_session("button", &dig, 150, &[Inside { at: 50, frames: 20, events: vec![Ev::Button(MouseButton::Left, false)], text: "" }], |_| STEP);
    let (outside, b) = marked_session("button-plain", &[(0, Ev::Motion(0.0, 300.0)), (20, Ev::Button(MouseButton::Left, true)), (50, Ev::Button(MouseButton::Left, false))], 150, &[], |_| STEP);
    assert_eq!(lines_without_markers(&inside.path), lines_without_markers(&outside.path));
    assert_eq!(a.sim_hash_copy(), b.sim_hash_copy());
    assert!(a.held().is_none(), "it isn't still digging");
    // A click inside isn't a release of something the game never saw go down.
    let (clicked, _) = marked_session("click", &[], 40, &[Inside { at: 20, frames: 5, events: vec![Ev::Button(MouseButton::Left, true), Ev::Button(MouseButton::Left, false)], text: "" }], |_| STEP);
    assert!(!events_of(&clicked.path).iter().any(|l| l.contains("release")), "{:?}", events_of(&clicked.path));
    for s in [inside, outside, clicked] {
        let _ = std::fs::remove_file(&s.path);
    }
}

/// However long the annotator was open, the steps come back with no burst to
/// catch up: the frame after it owes what a frame owes.
#[test]
fn the_time_in_there_is_not_owed_afterwards() {
    let mut s = fresh(None);
    let (mut c, mut g) = (Controls::default(), Gate::default());
    for _ in 0..10 {
        g.frame(&mut c, &mut s, STEP);
    }
    let before = s.steps();
    g.open();
    for _ in 0..1000 {
        assert_eq!(g.frame(&mut c, &mut s, 0.25), 0);
    }
    assert_eq!(s.steps(), before);
    g.close();
    assert_eq!(g.frame(&mut c, &mut s, STEP), 1);
    assert_eq!(s.steps(), before + 1);
}

/// A marker holds what the player saw: the camera can be ahead of the step by
/// mouse motion. The note comes before that motion in the file, the shot's pose
/// has it, and the player's own state doesn't until the step, as ever.
#[test]
fn a_marker_sees_what_the_player_saw() {
    let events = [(20, Ev::Motion(120.0, 40.0))];
    let (summary, s) = marked_session("ahead", &events, 40, &[Inside { at: 20, frames: 3, text: "looking right", ..Default::default() }], |_| STEP);
    let rep = read(&summary.path).unwrap();
    let m = &rep.markers()[0];
    let seen = parse_pose(&m.pose, s.engine.planet.block_size).unwrap();
    let ev = events_of(&summary.path);
    let note = ev.iter().position(|l| l.contains("note marker")).unwrap();
    let look = ev.iter().position(|l| l.starts_with("@20 look")).unwrap();
    assert!(note < look, "the marker comes first: {ev:?}");
    let (report, back) = play(&rep, "ahead");
    assert!(report.ok(), "{}", report.text());
    assert_eq!(back.player.heading.to_bits(), seen.heading.to_bits(), "the shot looked where the motion was going");
    let _ = std::fs::remove_file(&summary.path);
}

/// The same inputs at different frame rates still give the same file, markers
/// and all; a ragged rate with annotators open still replays exactly.
#[test]
fn markers_hold_at_any_frame_rate() {
    let marks = [Inside { at: 60, frames: 100, text: "one", ..Default::default() }, Inside { at: 200, frames: 3, text: "two", ..Default::default() }];
    let (fast, a) = marked_session("m144", &timeline(), 400, &marks, |_| STEP * 60.0 / 144.0);
    let (slow, b) = marked_session("m30", &timeline(), 400, &marks, |_| 2.0 * STEP);
    assert_eq!(a.sim_hash_copy(), b.sim_hash_copy());
    assert_eq!(lines_without_markers(&fast.path), lines_without_markers(&slow.path));
    let mut seed = 777u64;
    let (ragged, _) = marked_session("mragged", &timeline(), 400, &marks, move |_| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        0.002 + (seed >> 40) as f64 / (1u64 << 24) as f64 * 0.2
    });
    for (tag, s) in [("m144", &fast), ("m30", &slow), ("mragged", &ragged)] {
        let rep = read(&s.path).unwrap();
        assert_eq!(rep.markers().len(), 2, "{tag}");
        let (report, _) = play(&rep, tag);
        assert!(report.ok(), "{tag}: {}", report.text());
        let _ = std::fs::remove_file(&s.path);
    }
}

/// A marker is on disk as soon as it's saved: a crash a moment later keeps it.
/// And a comment with quotes, new lines and accents comes through on one line.
#[test]
fn a_marker_is_on_disk_at_once_and_its_words_survive() {
    let mut s = fresh(None);
    let path = temp("flush");
    s.start_recording(&path, "test").unwrap();
    drive_marked(&mut s, &[], 7, &[Inside { at: 5, frames: 1, text: "he said \"no\"\nand left, déjà vu", ..Default::default() }], |_| STEP);
    let rep = read(&path).unwrap();
    assert!(!rep.ended);
    assert_eq!(rep.markers()[0].step, 5);
    assert_eq!(rep.markers()[0].text, "he said \"no\"\nand left, déjà vu");
    s.stop_recording().unwrap();
    let _ = std::fs::remove_file(&path);
}

/// A look change during a recording is written down at its step, in words. A
/// replay reports it and goes on, exact as ever: it's a note, not a divergence.
#[test]
fn a_look_change_during_a_recording_is_written_down() {
    use crate::visual::{Reproduced, Visual, judge};
    let reg = crate::world::Registry::load(&format!("{}/assets/blocks.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let look = crate::look::Look::defaults(&Settings::default(), 90_000.0);
    let v = |l: &crate::look::Look| Arc::new(Visual::of(l, &reg, Default::default(), "normal", true, true, 2));
    let mut s = fresh(None);
    s.visual = Some(v(&look));
    let path = temp("visual");
    s.start_recording(&path, "test").unwrap();
    for _ in 0..30 {
        s.step(&Input::default(), STEP);
    }
    let mut changed = look.clone();
    changed.sky.fog_strength = 0.25;
    s.visual = Some(v(&changed));
    for _ in 0..30 {
        s.step(&Input::default(), STEP);
    }
    // The same look in a new value isn't a change.
    s.visual = Some(v(&changed));
    s.step(&Input::default(), STEP);
    let summary = s.stop_recording().unwrap();
    assert_eq!(summary.notes, 0);
    let rep = read(&summary.path).unwrap();
    assert_eq!(rep.header.visual.as_deref_look(), Some(0.7), "the header holds the look at the start");
    let changes = rep.visual_changes();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].0, 30);
    assert!(changes[0].1[0].contains("sky.fog_strength"), "{changes:?}");
    let (report, _) = play(&rep, "visual");
    assert!(report.ok() && report.notes.is_empty(), "{}", report.text());
    let v = judge(rep.header.visual.as_ref(), &rep.header.build, rep.visual_changes(), &Visual::of(&look, &reg, Default::default(), "normal", true, true, 2));
    assert_eq!(v.reproduced, Reproduced::Different, "it can't say the look was the same throughout");
    assert!(v.text().contains("at step 30"), "{}", v.text());
    let _ = std::fs::remove_file(&summary.path);
}

/// A file from before B6 has no build, no look and no markers, and reads as it
/// always did: an old header parses, and its look is unknown, not assumed.
#[test]
fn an_old_header_still_parses() {
    let mut s = fresh(None);
    let path = temp("old-header");
    s.start_recording(&path, "test").unwrap();
    s.step(&Input::default(), STEP);
    let summary = s.stop_recording().unwrap();
    let rep = read(&summary.path).unwrap();
    let mut v = serde_json::to_value(&rep.header).unwrap();
    let m = v.as_object_mut().unwrap();
    assert!(m.remove("build").is_some() && m.remove("visual").is_some());
    let old: Header = serde_json::from_value(v).unwrap();
    assert!(old.build.is_empty() && old.visual.is_none());
    assert!(rep.markers().is_empty() && rep.visual_changes().is_empty());
    let _ = std::fs::remove_file(&summary.path);
}

impl Session {
    /// The state hash, without touching the session the test still reads.
    fn sim_hash_copy(&self) -> String {
        let world = self.engine.world.clone();
        self.sim.state_hash(world.as_ref())
    }
}

trait LookOf {
    fn as_deref_look(&self) -> Option<f64>;
}

impl LookOf for Option<crate::visual::Visual> {
    fn as_deref_look(&self) -> Option<f64> {
        self.as_ref()?.look["sky"]["fog_strength"].as_f64()
    }
}

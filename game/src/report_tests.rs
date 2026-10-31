//! Reading a report back, and the folder that gets sent.

use super::*;
use crate::annotate::Ink;
use crate::player::{Input, STEP};
use crate::replay::tests::fresh;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("triangulum-report-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn picture() -> Vec<u8> {
    [70u8, 120, 90, 255].repeat(64 * 36)
}

/// F9 in a test: the real grab, annotator and commit, 64 x 36 pixels. The shot
/// and its marks are written as the app's writer thread does.
fn mark(s: &mut Session, text: &str, marks: Vec<Mark>) -> PathBuf {
    let settings = Settings::default();
    let look = Look::defaults(&settings, s.engine.planet.radius);
    let mut digests = ChunkDigests::default();
    let cam = s.player.camera();
    let moment = Moment::grab(Grab { session: s, cam: &cam, settings: &settings, look: &look, digests: &mut digests, rgba: picture(), width: 64, height: 36, mesh_view: Default::default(), preview_world: false, frame_ms: None });
    let (shot, marks) = moment.commit(s, marks, text.to_string());
    photos::write_marked(&shot, &marks).unwrap();
    shot.png
}

/// A recording with two marked moments, in `dir`.
fn recording(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let mut s = fresh(None);
    let path = dir.join("Neisor-20261031-120000.replay");
    s.start_recording(&path, "test").unwrap();
    for _ in 0..30 {
        s.step(&Input::default(), STEP);
    }
    let a = mark(&mut s, "the water climbs the wall", vec![Mark::Box { colour: Ink::Red, from: [10.0, 8.0], to: [30.0, 20.0] }]);
    for _ in 0..60 {
        s.step(&Input { forward: true, ..Default::default() }, STEP);
    }
    let b = mark(&mut s, "", Vec::new());
    for _ in 0..10 {
        s.step(&Input::default(), STEP);
    }
    s.stop_recording().unwrap();
    (path, a, b)
}

#[test]
fn a_note_is_a_marker_only_if_it_says_so() {
    let m = Marker { step: 0, shot: "marker01_step000005".into(), text: "a \"word\"\nand another".into(), pose: "1,2".into(), clock: "3.5".into() };
    let note = format!("marker {}", serde_json::to_string(&m).unwrap());
    assert!(!note.contains('\n'), "one line");
    assert!(is_ours(&note));
    let back = Marker::parse(&note, 5).unwrap();
    assert_eq!(back, Marker { step: 5, ..m });
    assert_eq!(back.replay_shot(2), "marker02_step000005");
    for plain in ["a step of 0.1 s, not 1/60: the replay steps 1/60 and will diverge", "the world changed outside the recorded inputs; the replay will diverge here", "marker", "visual"] {
        assert!(!is_ours(plain) && Marker::parse(plain, 1).is_none(), "{plain}");
    }
    assert_eq!(Marker::parse("marker {not json", 1), None);
    assert_eq!(visual_change("visual {\"changed\":[\"look sky.fog_strength\"]}"), Some(vec!["look sky.fog_strength".to_string()]));
}

/// `--watch`: the words show as the marker passes, for a few seconds of replay,
/// and the newest wins when two are close.
#[test]
fn a_markers_words_show_as_it_passes() {
    let at = |step, text: &str| Marker { step, text: text.into(), ..Default::default() };
    let t = Ticker::new(vec![at(100, "first"), at(200, ""), at(900, "last")]);
    assert_eq!(t.line(99), None);
    assert_eq!(t.line(100).as_deref(), Some("Marker 1 of 3, step 100 (1.7 s): first"));
    assert_eq!(t.line(199).as_deref(), Some("Marker 1 of 3, step 100 (1.7 s): first"));
    assert!(t.line(200).unwrap().contains("Marker 2 of 3") && t.line(200).unwrap().contains("(no text)"));
    assert_eq!(t.line(200 + SHOW_STEPS), None);
    assert!(t.line(900 + SHOW_STEPS - 1).unwrap().ends_with("last"));
    assert_eq!(t.line(900 + SHOW_STEPS), None);
    assert_eq!(Ticker::new(Vec::new()).line(0), None);
}

/// The T window's list reads a recording's markers without playing it, keeps a
/// title and description beside it, and takes the latest words from the shot.
#[test]
fn a_recording_lists_its_markers_and_keeps_its_notes() {
    let dir = scratch("list");
    let (path, a, b) = recording(&dir);
    assert_eq!(a.parent().unwrap(), dir.join("Neisor-20261031-120000.markers"), "shots beside the recording");
    assert_eq!(a.file_name().unwrap(), "marker01_step000030.png");
    assert_eq!(b.file_name().unwrap(), "marker02_step000090.png");
    let side: Value = serde_json::from_str(&std::fs::read_to_string(a.with_extension("json")).unwrap()).unwrap();
    assert_eq!(side["format"], photos::FORMAT);
    assert_eq!(side["notes"], "the water climbs the wall");
    assert_eq!(side["marker"]["step"], 30);
    assert_eq!(side["marker"]["n"], 1);
    assert_eq!(side["kind"], "marker");
    assert_eq!(annotate::marks_of(&side).len(), 1);
    assert!(a.with_file_name("marker01_step000030_annotated.png").exists());
    assert!(!b.with_file_name("marker02_step000090_annotated.png").exists(), "no marks, no copy");
    let mut r = peek(&path);
    assert_eq!(r.error, None);
    assert!(r.ended);
    assert_eq!(r.steps, 100);
    assert!((r.seconds() - 100.0 / 60.0).abs() < 1e-9);
    assert_eq!(r.build, crate::visual::BUILD);
    assert_eq!(r.markers.len(), 2);
    assert_eq!((r.markers[0].marker.step, r.markers[1].marker.step), (30, 90));
    assert_eq!(r.markers[0].text(), "the water climbs the wall");
    assert!(r.markers[0].photo.as_ref().is_some_and(|p| p.annotated.is_some()));
    assert_eq!(r.label(), "Neisor-20261031-120000");
    r.title = "The wall".into();
    r.description = "It climbs.\nThen it stops.".into();
    save_recording_notes(&r).unwrap();
    let back = peek(&path);
    assert_eq!((back.title.as_str(), back.description.as_str(), back.label().as_str()), ("The wall", "It climbs.\nThen it stops.", "The wall"));
    // Words changed on the shot later win over the ones in the file.
    let mut p = back.markers[0].photo.clone().unwrap();
    p.notes = "better words".into();
    photos::save_notes(&p).unwrap();
    assert_eq!(peek(&path).markers[0].text(), "better words");
    // A second recording, a damaged one and a stranger: the list carries on.
    std::fs::write(dir.join("junk.replay"), b"hello").unwrap();
    std::fs::write(dir.join("notes.txt"), b"hello").unwrap();
    let all = scan_recordings(&dir);
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(|r| r.error.is_some() && r.name == "junk.replay"));
    assert!(scan_recordings(&dir.join("nothing-here")).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A recording cut off by a crash lists what it has.
#[test]
fn a_cut_off_recording_still_lists_its_markers() {
    let dir = scratch("cut");
    let (path, _, _) = recording(&dir);
    let bytes = std::fs::read(&path).unwrap();
    let keep = bytes.windows(9).rposition(|w| w == b"@90 note ").unwrap();
    std::fs::write(&path, &bytes[..keep + 3]).unwrap();
    let r = peek(&path);
    assert_eq!(r.error, None);
    assert!(!r.ended);
    assert_eq!(r.markers.len(), 1, "{:?}", r.markers);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Make report: one folder, with the recording, every shot and sidecar, and a
/// `report.md` holding for each item what was written, the step and time, the
/// exact pose, the build, and the annotated picture's path. Nothing else moves.
#[test]
fn a_report_gathers_everything_into_one_folder() {
    let dir = scratch("make");
    let (path, a, _) = recording(&dir);
    let mut r = peek(&path);
    r.title = "The wall".into();
    save_recording_notes(&r).unwrap();
    let original = std::fs::read(&a).unwrap();
    let inter = dir.join("interchange");
    let req = Request { title: "Water climbs the wall!".into(), description: "Standing by the lake,\nit goes up.".into(), recording: Some(path.clone()), photos: Vec::new() };
    let folder = make(&inter, &req).unwrap();
    assert_eq!(folder.parent().unwrap(), inter.join("reports"));
    assert!(name_of(&folder).ends_with("-water-climbs-the-wall"), "{}", folder.display());
    assert!(!folder.with_file_name(format!("{}.part", name_of(&folder))).exists(), "nothing half made left");
    for f in ["report.md", "Neisor-20261031-120000.replay", "Neisor-20261031-120000.json", "Neisor-20261031-120000.markers/marker01_step000030.png", "Neisor-20261031-120000.markers/marker01_step000030.json", "Neisor-20261031-120000.markers/marker01_step000030_annotated.png", "Neisor-20261031-120000.markers/marker02_step000090.png"] {
        assert!(folder.join(f).exists(), "{f}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), std::fs::read(folder.join("Neisor-20261031-120000.replay")).unwrap());
    assert_eq!(std::fs::read(&a).unwrap(), original);
    let md = std::fs::read_to_string(folder.join("report.md")).unwrap();
    let side: Value = serde_json::from_str(&std::fs::read_to_string(a.with_extension("json")).unwrap()).unwrap();
    for want in [
        "# Bug report: Water climbs the wall!",
        "Standing by the lake,",
        "### 1. the water climbs the wall",
        "**What was written:** the water climbs the wall",
        "step 30, which is 0.50 seconds into the recording",
        "step 90, which is 1.50 seconds",
        "Neisor-20261031-120000.markers/marker01_step000030_annotated.png",
        "red box from (10, 8) to (30, 20)",
        "in pixels of the 64 x 36 picture",
        side["pose"]["exact"].as_str().unwrap(),
        side["sky"]["clock_exact"].as_str().unwrap(),
        crate::visual::BUILD,
        "--repro interchange/reports/",
        "--replay interchange/reports/",
        "--watch",
        "If it will not play",
        "### 2. No words, just the picture",
        "(no marks were drawn)",
    ] {
        assert!(md.contains(want), "{want:?} not in:\n{md}");
    }
    // Another report of the same thing in the same second is another folder.
    let again = make(&inter, &req).unwrap();
    assert_ne!(again, folder);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A set of photos: each with its marks and words, no recording section.
#[test]
fn a_report_of_photos() {
    let dir = scratch("photos");
    let (png, plain) = (dir.join("shot_a.png"), dir.join("shot_b.png"));
    let s = fresh(None);
    let settings = Settings::default();
    let look = Look::defaults(&settings, s.engine.planet.radius);
    let cam = s.player.camera();
    let info = || ShotInfo { kind: "settled", settled: true, settle_s: 0.0, settle_frames: 0, width: 64, height: 36, hud: false, mesh_view: Default::default(), preview_world: false, frame_ms: None };
    for p in [&png, &plain] {
        let side = photos::sidecar(&s, &cam, &settings, &look, &mut ChunkDigests::default(), p, &info());
        photos::write_shot(&Shot { png: p.clone(), width: 64, height: 36, rgba: picture(), sidecar: side }).unwrap();
    }
    annotate::save_to_photo(&png, &[Mark::Arrow { colour: Ink::Blue, from: [5.0, 5.0], to: [50.0, 30.0] }], "the ridge has a seam").unwrap();
    let folder = make(&dir.join("interchange"), &Request { title: String::new(), description: String::new(), recording: None, photos: vec![png.clone(), plain.clone()] }).unwrap();
    assert!(name_of(&folder).ends_with("-untitled"));
    for f in ["shot_a.png", "shot_a.json", "shot_a_annotated.png", "shot_b.png", "shot_b.json"] {
        assert!(folder.join(f).exists(), "{f}");
    }
    let md = std::fs::read_to_string(folder.join("report.md")).unwrap();
    for want in ["blue arrow from (5, 5) pointing at (50, 30)", "the ridge has a seam", "a photo, not part of a recording", "`shot_a_annotated.png`", "pose "] {
        assert!(md.contains(want), "{want:?} not in:\n{md}");
    }
    assert!(!md.contains("## The recording"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A shot from before sidecars, or with a broken one, still goes in the report
/// and says what it doesn't know.
#[test]
fn a_report_says_what_it_does_not_have() {
    let dir = scratch("bare");
    let png = dir.join("shot_lat1.000_lon2.000_alt0.500km_yaw3_pitch-4.png");
    crate::render::write_png(&png.to_string_lossy(), 2, 2, &[9; 16]);
    let folder = make(&dir, &Request { title: "Bare".into(), photos: vec![png], ..Default::default() }).unwrap();
    let md = std::fs::read_to_string(folder.join("report.md")).unwrap();
    assert!(md.contains("no sidecar with an exact pose"), "{md}");
    // Nothing to gather from is refused in words and leaves nothing behind.
    let err = make(&dir, &Request { title: "Gone".into(), recording: Some(dir.join("gone.replay")), ..Default::default() }).unwrap_err();
    assert!(err.contains("gone.replay"), "{err}");
    let left: Vec<String> = std::fs::read_dir(dir.join("reports")).unwrap().flatten().map(|e| name_of(&e.path())).collect();
    assert_eq!(left.len(), 1, "{left:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The point of it all: with the recording gone or refused, a report's shots,
/// words and poses stand on their own, and `--repro` works from the copy in the
/// report with nothing but its sidecar.
#[test]
fn a_report_stands_without_its_replay() {
    let dir = scratch("alone");
    let (path, _, _) = recording(&dir);
    let folder = make(&dir, &Request { title: "Alone".into(), recording: Some(path.clone()), ..Default::default() }).unwrap();
    let copy = folder.join("Neisor-20261031-120000.markers/marker01_step000030.png");
    // Nothing of the original left, and the replay itself gone from the report.
    std::fs::remove_file(folder.join("Neisor-20261031-120000.replay")).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir_all(markers_dir(&path)).unwrap();
    let side: Value = serde_json::from_str(&std::fs::read_to_string(copy.with_extension("json")).unwrap()).unwrap();
    let r = photos::repro(&copy).unwrap();
    let script = std::fs::read_to_string(&r.script).unwrap();
    assert!(script.contains(&format!("pose {}\n", side["pose"]["exact"].as_str().unwrap())), "{script}");
    assert!(script.contains(&format!("clock {}\n", side["sky"]["clock_exact"].as_str().unwrap())), "{script}");
    assert!(script.contains("photo repro\n") && script.contains("compare repro marker01_step000030\n"), "{script}");
    assert!(script.contains("live frame"), "it says what kind of frame the original was: {script}");
    assert!(r.command.contains("--seed 42") && r.command.contains("--look-file"), "{}", r.command);
    // `--replay`'s markers read from the file alone, when it is there to read.
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_title_makes_a_plain_folder_name() {
    assert_eq!(slug("Water climbs the wall!"), "water-climbs-the-wall");
    assert_eq!(slug("  ../..\\evil:name?  "), "evil-name");
    assert_eq!(slug("déjà vu"), "d-j-vu");
    assert_eq!(slug("!!!"), "");
    assert!(slug(&"long ".repeat(40)).len() <= 40);
    assert_eq!(readable_utc("20261031-085029"), "2026-10-31 08:50:29");
    assert_eq!(readable_utc("nonsense"), "nonsense");
}

/// `--replay`'s list, and what `report.json` says of a marker when nothing has
/// played: everything a reader needs.
#[test]
fn a_replays_markers_stand_on_their_own() {
    let dir = scratch("json");
    let (path, _, _) = recording(&dir);
    let rep = crate::replay::read(&path).unwrap();
    let m = rep.markers();
    let list = marker_list(&m);
    assert!(list.starts_with("2 markers:"), "{list}");
    assert!(list.contains("1. step 30 (0.5 s): \"the water climbs the wall\"") && list.contains("2. step 90 (1.5 s): (no text)"), "{list}");
    let j = marker_json(&m[0], 1, &path);
    assert_eq!((j["n"].as_u64(), j["step"].as_u64(), j["text"].as_str()), (Some(1), Some(30), Some("the water climbs the wall")));
    assert_eq!(j["pose"].as_str(), Some(m[0].pose.as_str()));
    assert!(j["recorded_shot"].as_str().is_some_and(|p| p.ends_with("marker01_step000030.png")));
    assert!(j["annotated"].as_str().is_some_and(|p| p.ends_with("marker01_step000030_annotated.png")));
    assert_eq!(j["marks"][0]["tool"], "box");
    let _ = std::fs::remove_dir_all(&dir);
}

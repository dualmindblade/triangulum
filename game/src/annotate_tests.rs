//! The annotator, by the pointer and key events a hand makes, through the real
//! panel with no window on the desktop; and the pixels it writes.

use super::*;
use crate::menu::Rig;
use std::path::PathBuf;

struct T {
    a: Annotator,
    rig: Rig,
    size: (u32, u32),
    out: Option<Outcome>,
}

impl T {
    fn new(scale: f32, size: (u32, u32), picture: (u32, u32)) -> Self {
        let mut a = Annotator::new("Mark this moment", "test", "Save", picture.0, picture.1);
        a.set_picture(colour_image(&vec![90u8; (picture.0 * picture.1 * 4) as usize], picture.0, picture.1));
        let mut t = Self { a, rig: Rig::new(egui::Context::default(), scale), size, out: None };
        t.passes(3);
        t
    }

    fn pass(&mut self) -> egui::FullOutput {
        let raw = self.rig.input(self.size.0, self.size.1);
        let (a, out) = (&mut self.a, &mut self.out);
        self.rig.ctx.run_ui(raw, |ui| {
            if let Some(o) = a.ui(ui.ctx()) {
                *out = Some(o);
            }
        })
    }

    fn passes(&mut self, n: usize) {
        for _ in 0..n {
            self.pass();
        }
    }

    fn at(&self, fx: f32, fy: f32) -> Pos2 {
        let r = self.a.spots["canvas"];
        r.min + vec2(fx * r.width(), fy * r.height())
    }

    fn stroke(&mut self, pts: &[(f32, f32)]) {
        let first = self.at(pts[0].0, pts[0].1);
        self.rig.move_to(first);
        self.pass();
        self.rig.button(true);
        self.pass();
        for p in &pts[1..] {
            let q = self.at(p.0, p.1);
            self.rig.move_to(q);
            self.pass();
        }
        self.rig.button(false);
        self.pass();
    }

    fn click(&mut self, spot: &str) {
        let c = self.a.spots.get(spot).unwrap_or_else(|| panic!("no spot {spot}: {:?}", self.a.spots.keys())).center();
        self.rig.move_to(c);
        self.passes(2);
        self.rig.button(true);
        self.pass();
        self.rig.button(false);
        self.passes(2);
    }
}

/// The feel of the pen: the mark exists, on the pressed pixel, in the very pass
/// that saw the press (no drag threshold, no dead zone), and every pointer
/// event between two frames is kept, so a fast curve isn't a chord.
#[test]
fn a_stroke_starts_on_the_pixel_pressed_and_keeps_every_point() {
    let mut t = T::new(1.0, (1920, 1080), (1920, 1080));
    let p = t.at(0.25, 0.5);
    t.rig.move_to(p);
    t.pass();
    assert!(t.a.drawing().is_none());
    t.rig.button(true);
    let full = t.pass();
    let Some(Mark::Pen { points, .. }) = t.a.drawing() else { panic!("the press made a stroke at once") };
    assert_eq!(points.len(), 1);
    assert!((points[0][0] - 480.0).abs() < 1.0 && (points[0][1] - 540.0).abs() < 1.0, "{points:?}");
    assert!(!full.shapes.is_empty());
    // Fifty pointer moves between two frames: all of them land.
    for i in 1..=50 {
        t.rig.move_to(p + vec2(i as f32 * 2.0, (i as f32 * 0.4).sin() * 30.0));
    }
    t.pass();
    let Some(Mark::Pen { points, .. }) = t.a.drawing() else { panic!() };
    assert_eq!(points.len(), 51, "every point of a fast stroke");
    t.rig.button(false);
    t.pass();
    assert_eq!(t.a.marks.len(), 1);
    assert!(t.a.drawing().is_none());
}

/// A mark lands on the same picture pixel wherever the picture is on screen, at
/// every window size and display scale. Dragging off the edge stays on it.
#[test]
fn marks_land_on_the_right_pixels_at_every_scale() {
    for (scale, size, picture) in [(1.0, (1920, 1080), (1920, 1080)), (1.5, (1920, 1080), (1920, 1080)), (1.0, (1280, 720), (1920, 1080)), (1.25, (1366, 768), (2560, 1440)), (1.5, (3840, 2160), (3840, 2160)), (1.0, (1024, 600), (640, 480))] {
        let mut t = T::new(scale, size, picture);
        t.a.tool = Tool::Box;
        t.stroke(&[(0.25, 0.5), (0.75, 0.9)]);
        let Mark::Box { from, to, .. } = &t.a.marks[0] else { panic!() };
        let (w, h) = (picture.0 as f64, picture.1 as f64);
        let near = |a: f64, b: f64| (a - b).abs() < 2.0;
        assert!(near(from[0], w * 0.25) && near(from[1], h * 0.5) && near(to[0], w * 0.75) && near(to[1], h * 0.9), "{scale} {size:?} {picture:?}: {from:?} {to:?}");
        // The picture, the comment and the way out are on screen.
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(size.0 as f32 / scale, size.1 as f32 / scale));
        for spot in ["canvas", "comment", "annot-save", "annot-cancel", "undo", "tool-pen", "ink-red"] {
            assert!(screen.contains_rect(t.a.spots[spot]), "{scale} {size:?}: {spot} {:?} on {screen:?}", t.a.spots[spot]);
        }
        let k = t.a.spots["canvas"].width() / t.a.spots["canvas"].height();
        assert!((k - picture.0 as f32 / picture.1 as f32).abs() < 0.01, "the picture keeps its shape");
        // Off the edge: on the last pixel, not outside the picture.
        t.a.tool = Tool::Arrow;
        let c = t.at(0.5, 0.5);
        t.rig.move_to(c);
        t.pass();
        t.rig.button(true);
        t.pass();
        t.rig.move_to(c + vec2(9000.0, 9000.0));
        t.pass();
        t.rig.button(false);
        t.pass();
        let Mark::Arrow { to, .. } = t.a.marks.last().unwrap() else { panic!() };
        assert_eq!(*to, [w, h], "{scale} {size:?}");
    }
}

#[test]
fn every_tool_and_every_colour_make_their_mark() {
    let mut t = T::new(1.0, (1920, 1080), (1920, 1080));
    for (i, tool) in ["tool-arrow", "tool-box", "tool-pen"].into_iter().enumerate() {
        let ink = Ink::ALL[i + 1];
        t.click(tool);
        t.click(&format!("ink-{}", ink.name()));
        t.stroke(&[(0.1, 0.1), (0.2, 0.2)]);
        t.click(&format!("ink-{}", Ink::Red.name()));
        assert_eq!(t.a.marks.len(), i + 1, "{tool}");
    }
    assert!(matches!(t.a.marks[0], Mark::Arrow { colour: Ink::Yellow, .. }));
    assert!(matches!(t.a.marks[1], Mark::Box { colour: Ink::Green, .. }));
    assert!(matches!(t.a.marks[2], Mark::Pen { colour: Ink::Blue, .. }));
    // A click with the box or the arrow is no mark; a dot with the pen is one.
    t.click("tool-box");
    t.stroke(&[(0.4, 0.4)]);
    t.click("tool-arrow");
    t.stroke(&[(0.4, 0.4)]);
    assert_eq!(t.a.marks.len(), 3);
    t.click("tool-pen");
    t.stroke(&[(0.4, 0.4)]);
    assert_eq!(t.a.marks.len(), 4);
    // Each says what it is, in pixels.
    let words: Vec<String> = t.a.marks.iter().map(Mark::describe).collect();
    assert!(words[1].contains("green box from (192, 108) to (384, 216)"), "{words:?}");
    assert!(words[0].contains("yellow arrow from (192, 108) pointing at (384, 216)"), "{words:?}");
}

/// Text: click and type where it goes. The click's release must not take the
/// keys away (it lands outside the field that just opened), Enter finishes it,
/// and the next keys are the comment's again.
#[test]
fn a_text_mark_is_typed_where_it_was_clicked() {
    let mut t = T::new(1.0, (1920, 1080), (1920, 1080));
    t.click("tool-text");
    let p = t.at(0.5, 0.25);
    t.rig.move_to(p);
    t.passes(2);
    t.rig.button(true);
    t.pass();
    t.rig.button(false);
    t.passes(3);
    t.rig.text("here it is");
    t.passes(2);
    assert!(t.a.comment.is_empty(), "not the comment's");
    t.rig.key(Key::Enter);
    t.passes(2);
    assert_eq!(t.out, None, "Enter finished the text, it didn't save");
    let Mark::Text { at, text, .. } = &t.a.marks[0] else { panic!("{:?}", t.a.marks) };
    assert_eq!(text, "here it is");
    assert!((at[0] - 960.0).abs() < 1.0 && (at[1] - 270.0).abs() < 1.0, "{at:?}");
    t.rig.text("back in the comment");
    t.passes(2);
    assert_eq!(t.a.comment, "back in the comment");
    // Clicking elsewhere finishes it too; empty text is no mark; Esc drops it.
    let q = t.at(0.2, 0.7);
    t.rig.move_to(q);
    t.passes(2);
    t.rig.button(true);
    t.pass();
    t.rig.button(false);
    t.passes(3);
    t.rig.text("second");
    t.passes(2);
    t.rig.move_to(t.at(0.8, 0.7));
    t.passes(2);
    t.rig.button(true);
    t.pass();
    t.rig.button(false);
    t.passes(3);
    assert_eq!(t.a.marks.len(), 2);
    t.rig.key(Key::Escape);
    t.passes(2);
    assert_eq!(t.a.marks.len(), 2, "Esc dropped the empty draft only");
    assert_eq!(t.out, None);
}

/// Typing goes in the comment after any stroke, with no click on the box.
/// Ctrl+Z takes marks away, newest first, and leaves the words alone.
#[test]
fn typing_just_works_and_undo_takes_marks() {
    let mut t = T::new(1.0, (1920, 1080), (1920, 1080));
    t.rig.text("before ");
    t.passes(2);
    t.stroke(&[(0.1, 0.1), (0.2, 0.3)]);
    t.click("tool-box");
    t.stroke(&[(0.3, 0.3), (0.5, 0.5)]);
    t.rig.text("after");
    t.passes(2);
    assert_eq!(t.a.comment, "before after");
    assert_eq!(t.a.marks.len(), 2);
    t.rig.key_with(Key::Z, Modifiers::COMMAND);
    t.passes(2);
    assert_eq!(t.a.marks.len(), 1);
    assert!(matches!(t.a.marks[0], Mark::Pen { .. }));
    assert_eq!(t.a.comment, "before after", "the words stay");
    t.click("undo");
    assert!(t.a.marks.is_empty());
}

#[test]
fn enter_saves_and_esc_asks_before_it_loses_anything() {
    // Nothing to lose: Esc is out at once.
    let mut t = T::new(1.0, (1280, 720), (1920, 1080));
    t.rig.key(Key::Escape);
    t.passes(2);
    assert_eq!(t.out, Some(Outcome::Cancel));
    // Marks to lose: the first Esc warns, the second means it.
    let mut t = T::new(1.0, (1280, 720), (1920, 1080));
    t.stroke(&[(0.1, 0.1), (0.2, 0.2)]);
    t.rig.key(Key::Escape);
    t.passes(2);
    assert_eq!(t.out, None);
    t.rig.key(Key::Escape);
    t.passes(2);
    assert_eq!(t.out, Some(Outcome::Cancel));
    // Words to lose, the same; drawing again takes the warning back.
    let mut t = T::new(1.0, (1280, 720), (1920, 1080));
    t.rig.text("x");
    t.passes(2);
    t.rig.key(Key::Escape);
    t.passes(2);
    t.stroke(&[(0.1, 0.1), (0.2, 0.2)]);
    t.rig.key(Key::Escape);
    t.passes(2);
    assert_eq!(t.out, None);
    // Enter saves; Shift+Enter is a new line.
    let mut t = T::new(1.0, (1280, 720), (1920, 1080));
    t.rig.text("first");
    t.passes(2);
    t.rig.key_with(Key::Enter, Modifiers::SHIFT);
    t.passes(2);
    t.rig.text("second");
    t.passes(2);
    assert_eq!(t.out, None);
    assert_eq!(t.a.comment, "first\nsecond");
    t.rig.key(Key::Enter);
    t.passes(2);
    assert_eq!(t.out, Some(Outcome::Save));
    assert_eq!(t.a.comment, "first\nsecond", "Enter is not in the comment");
    // The buttons do the same.
    let mut t = T::new(1.0, (1280, 720), (1920, 1080));
    t.click("annot-save");
    assert_eq!(t.out, Some(Outcome::Save));
    let mut t = T::new(1.0, (1280, 720), (1920, 1080));
    t.click("annot-cancel");
    assert_eq!(t.out, Some(Outcome::Cancel));
}

/// A marker pen keeps up: the panel's cost with a lot on the picture, an egui
/// pass and its tessellation, against 16 ms. Generous, since other work shares
/// the machine; the figure is printed for the findings.
#[test]
fn a_pass_stays_quick_with_a_lot_drawn() {
    let mut t = T::new(1.0, (1920, 1080), (1920, 1080));
    for s in 0..200 {
        let points = (0..300).map(|i| [100.0 + i as f64 * 5.0, 100.0 + s as f64 * 4.0 + ((i as f64) * 0.1).sin() * 20.0]).collect();
        t.a.marks.push(Mark::Pen { colour: Ink::Red, points });
    }
    t.passes(2);
    let mut worst = 0.0f64;
    let mut sum = 0.0;
    for _ in 0..20 {
        let raw = t.rig.input(1920, 1080);
        let ms = pass_ms(&mut t.a, &t.rig.ctx, raw);
        worst = worst.max(ms);
        sum += ms;
    }
    println!("annotator with 200 strokes of 300 points (60,000 points): {:.2} ms a pass on average, {worst:.2} ms at worst", sum / 20.0);
    assert!(worst < 50.0, "{worst:.1} ms");
    // Flattening runs off the main thread: this is how long the save takes there.
    let f = std::time::Instant::now();
    let flat = flatten(&grey(1920, 1080), 1920, 1080, &t.a.marks);
    println!("flattening the same on 1920 x 1080: {:.0} ms", f.elapsed().as_secs_f64() * 1e3);
    assert_ne!(flat, grey(1920, 1080));
}

fn grey(w: u32, h: u32) -> Vec<u8> {
    [90u8, 100, 110, 255].repeat((w * h) as usize)
}

fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

/// The flattened picture has the marks on the pixels they name, the same colour
/// as on screen, a dark edge, and nothing anywhere else.
#[test]
fn flattening_puts_marks_on_their_pixels_and_nothing_elsewhere() {
    let (w, h) = (640, 360);
    let rgba = grey(w, h);
    let before = rgba.clone();
    let marks = vec![Mark::Box { colour: Ink::Yellow, from: [100.0, 100.0], to: [300.0, 200.0] }];
    let flat = flatten(&rgba, w, h, &marks);
    assert_eq!(rgba, before, "the input is left alone");
    assert_eq!(flat.len(), rgba.len());
    assert_eq!(px(&flat, w, 200, 100), [255, 225, 30], "on the box's top edge, its colour");
    assert_eq!(px(&flat, w, 100, 150), [255, 225, 30], "on its left edge");
    assert_eq!(px(&flat, w, 200, 150), [90, 100, 110], "inside: the picture");
    assert_eq!(px(&flat, w, 20, 20), [90, 100, 110], "outside: the picture");
    let edge = px(&flat, w, 200, 98);
    assert!(edge[0] < 60 && edge[2] < 60, "a dark edge beside the line, {edge:?}");
    // Nothing further than the line and its edge changed.
    for y in 0..h {
        for x in 0..w {
            let far = !(90..=310).contains(&x) || !(90..=210).contains(&y) || ((110..=290).contains(&x) && (110..=190).contains(&y));
            if far {
                assert_eq!(px(&flat, w, x, y), [90, 100, 110], "{x} {y}");
            }
        }
    }
    // Every other kind lands too, and no marks is the picture unchanged.
    assert_eq!(flatten(&rgba, w, h, &[]), rgba);
    let flat = flatten(&rgba, w, h, &[Mark::Pen { colour: Ink::Red, points: vec![[10.0, 300.0], [200.0, 300.0]] }, Mark::Arrow { colour: Ink::Green, from: [400.0, 50.0], to: [600.0, 50.0] }]);
    assert_eq!(px(&flat, w, 100, 300), [255, 44, 44]);
    assert_eq!(px(&flat, w, 450, 50), [40, 235, 90]);
    assert_eq!(px(&flat, w, 595, 50), [40, 235, 90], "the head reaches the tip");
    // A mark off the picture is clipped, not a crash.
    let _ = flatten(&rgba, w, h, &[Mark::Pen { colour: Ink::Red, points: vec![[-500.0, -500.0], [9000.0, 9000.0]] }]);
}

/// Text comes through the same font atlas as the screen, legible, on a dark
/// plate, on any thread.
#[test]
fn flattened_text_is_there() {
    let (w, h) = (640, 360);
    let rgba = grey(w, h);
    let flat = std::thread::spawn(move || flatten(&rgba, w, h, &[Mark::Text { colour: Ink::White, at: [40.0, 40.0], text: "Andrew was here".to_string() }])).join().unwrap();
    let (mut bright, mut plate) = (0, 0);
    for y in 30..80 {
        for x in 30..320 {
            let p = px(&flat, w, x, y);
            bright += (p[0] > 200 && p[1] > 200 && p[2] > 200) as u32;
            plate += (p[0] < 40) as u32;
        }
    }
    assert!(bright > 150, "the letters: {bright} pixels");
    assert!(plate > 800, "the plate behind them: {plate} pixels");
    assert_eq!(px(&flat, w, 500, 300), [90, 100, 110]);
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("triangulum-annotate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Marks go in the sidecar as data and in a flattened copy, the PNG doesn't
/// change by a byte, and every other key survives. Taking the marks away takes
/// the copy away.
#[test]
fn a_photo_keeps_its_original_and_its_sidecar() {
    let dir = scratch("photo");
    let png = dir.join("shot_a.png");
    let side = json!({ "format": crate::photos::FORMAT, "title": "kept", "later_key": { "x": 1 } });
    crate::photos::write_shot(&crate::photos::Shot { png: png.clone(), width: 64, height: 36, rgba: grey(64, 36), sidecar: side }).unwrap();
    let original = std::fs::read(&png).unwrap();
    let marks = vec![Mark::Box { colour: Ink::Red, from: [10.0, 10.0], to: [40.0, 20.0] }, Mark::Text { colour: Ink::White, at: [5.0, 25.0], text: "a \"quoted\" word".to_string() }];
    save_to_photo(&png, &marks, "says what's wrong").unwrap();
    assert_eq!(std::fs::read(&png).unwrap(), original, "the original is not touched");
    let flat = crate::photos::annotated_of(&png);
    assert_eq!(flat.file_name().unwrap(), "shot_a_annotated.png");
    let (w, h, rgba) = crate::render::read_png(&flat.to_string_lossy()).unwrap();
    assert_eq!((w, h), (64, 36));
    assert_ne!(rgba, grey(64, 36), "the marks are on it");
    let js: Value = serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(marks_of(&js), marks, "the marks come back as data");
    assert_eq!(js["annotations"]["image"], json!([64, 36]));
    assert_eq!(js["annotated"], "shot_a_annotated.png");
    assert_eq!((js["notes"].as_str(), js["title"].as_str()), (Some("says what's wrong"), Some("kept")));
    assert_eq!(js["later_key"]["x"], 1, "a key this build doesn't know stays");
    // Annotated again, with fewer marks: the same copy, redrawn.
    save_to_photo(&png, &marks[..1], "fewer").unwrap();
    assert_eq!(std::fs::read(&png).unwrap(), original);
    let js: Value = serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!(marks_of(&js).len(), 1);
    save_to_photo(&png, &[], "none").unwrap();
    assert!(!flat.exists(), "no marks, no copy");
    let js: Value = serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert!(js.get("annotated").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A sidecar with no marks, or marks from a later build, reads as none.
#[test]
fn marks_this_build_cant_read_are_no_marks() {
    assert!(marks_of(&json!({})).is_empty());
    assert!(marks_of(&json!({ "annotations": [{ "at": [0.5, 0.5], "text": "here" }] })).is_empty());
    assert!(marks_of(&json!({ "annotations": { "marks": [{ "tool": "laser" }] } })).is_empty());
}

/// The gate, key by key. F9 is the case that bites: it goes down with the gate
/// shut and comes up with it open, and if the release were swallowed `Controls`
/// would hold it down forever and F9 would never fire again.
#[test]
fn a_release_always_gets_through_and_nothing_else_does() {
    let (mut c, mut g) = (Controls::default(), Gate::default());
    let mut s = crate::replay::tests::fresh(None);
    assert!(g.key(&mut c, KeyCode::F9, true));
    g.open();
    assert!(!g.key(&mut c, KeyCode::F9, false));
    g.close();
    assert!(g.key(&mut c, KeyCode::F9, true), "F9 fires again");
    assert!(!g.key(&mut c, KeyCode::F9, false));
    // A key held at F9 stays held, until it is let go in there.
    assert!(g.key(&mut c, KeyCode::KeyW, true));
    c.step(&mut s);
    g.open();
    assert!(c.input().forward);
    assert!(!g.key(&mut c, KeyCode::KeyW, false));
    assert!(!c.input().forward);
    // A key pressed in there is the annotator's, and its release is harmless.
    assert!(!g.key(&mut c, KeyCode::KeyA, true));
    assert!(!c.input().left);
    g.close();
    assert!(!c.input().left);
    assert!(!g.key(&mut c, KeyCode::KeyA, false));
    // A key pressed before and released after is still a clean press and release.
    assert!(g.key(&mut c, KeyCode::KeyD, true));
    c.step(&mut s);
    g.open();
    g.close();
    assert!(c.input().right);
    g.key(&mut c, KeyCode::KeyD, false);
    assert!(!c.input().right);
}

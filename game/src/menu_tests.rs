//! The T window, headless: synthetic pointer and key events through the real
//! window code (`Rig`), over the real world bake. What a mouse would do, with
//! no window on the desktop.

use super::*;
use crate::controls::{Controls, Route, route_key};
use crate::replay::tests::fresh;
use winit::keyboard::KeyCode;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("triangulum-menu-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Test {
    session: Session,
    menu: Menu,
    rig: Rig,
    presets: Vec<Preset>,
    size: (u32, u32),
    can_move: bool,
    went: Option<Go>,
    dir: PathBuf,
}

impl Test {
    fn new(tag: &str, scale: f32, size: (u32, u32)) -> Self {
        let dir = scratch(tag);
        let session = fresh(None);
        let presets = session.engine.presets();
        let mut menu = Menu::new(dir.clone());
        menu.show();
        Self { session, menu, rig: Rig::new(egui::Context::default(), scale), presets, size, can_move: true, went: None, dir }
    }

    /// One pass of the window: the input queued so far, then the window.
    fn pass(&mut self) -> egui::FullOutput {
        let raw = self.rig.input(self.size.0, self.size.1);
        let engine = self.session.engine.clone();
        let env = MenuEnv { engine: &engine, bake_sha256: &self.session.bake_sha256, presets: &self.presets, here: Planet::lat_lon_deg(self.session.player.pos), can_move: self.can_move, day_length_s: 1440.0 };
        let (menu, went) = (&mut self.menu, &mut self.went);
        self.rig.ctx.run_ui(raw, |ui| {
            if let Some(g) = menu.ui(ui.ctx(), &env) {
                *went = Some(g);
            }
        })
    }

    fn passes(&mut self, n: usize) {
        for _ in 0..n {
            self.pass();
        }
    }

    /// Passes until the threads are done and the screen has what they made.
    fn settle(&mut self) {
        let t = std::time::Instant::now();
        loop {
            self.pass();
            if !self.menu.busy() {
                self.passes(3);
                if !self.menu.busy() {
                    return;
                }
            }
            assert!(t.elapsed().as_secs() < 60, "the window never settled");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn at(&self, spot: &str, fx: f32, fy: f32) -> Pos2 {
        let r = self.menu.spots.get(spot).unwrap_or_else(|| panic!("no spot {spot}: {:?}", self.menu.spots.keys()));
        r.min + vec2(fx * r.width(), fy * r.height())
    }

    fn click_at(&mut self, p: Pos2) {
        self.rig.move_to(p);
        self.passes(2);
        self.rig.button(true);
        self.pass();
        self.rig.button(false);
        self.pass();
        self.passes(2);
    }

    fn click(&mut self, spot: &str) {
        self.click_at(self.at(spot, 0.5, 0.5));
    }

    fn type_into(&mut self, spot: &str, text: &str) {
        self.click(spot);
        self.rig.text(text);
        self.passes(2);
    }

    fn shot_at(&self, name: &str, lat: f64, lon: f64) -> PathBuf {
        let png = self.dir.join(format!("{name}.png"));
        let mut s = self.session_sidecar(&png);
        s["pose"]["lat"] = lat.into();
        s["pose"]["lon"] = lon.into();
        photos::write_shot(&photos::Shot { png: png.clone(), width: 4, height: 2, rgba: vec![120; 32], sidecar: s }).unwrap();
        png
    }

    fn session_sidecar(&self, png: &Path) -> serde_json::Value {
        let info = photos::ShotInfo { kind: "settled", settled: true, settle_s: 0.0, settle_frames: 0, width: 4, height: 2, hud: false, mesh_view: Default::default(), preview_world: false, frame_ms: None };
        let cam = self.session.player.camera();
        let settings = Settings::default();
        let look = crate::look::Look::defaults(&settings, self.session.engine.planet.radius);
        photos::sidecar(&self.session, &cam, &settings, &look, &mut Default::default(), png, &info)
    }
}

impl Drop for Test {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn typed_coordinates_are_strict() {
    assert_eq!(parse_coords("12.5 -40.2"), Ok((12.5, -40.2, None)));
    assert_eq!(parse_coords(" 12.5, -40.2, 300 "), Ok((12.5, -40.2, Some(300.0))));
    assert_eq!(parse_coords("0 190"), Ok((0.0, -170.0, None)));
    assert_eq!(parse_coords("0 180").unwrap().1, 180.0);
    for bad in ["", "12", "1 2 3 4", "NaN 2", "1 inf", "91 0", "x y", "1 2 z", "0 400"] {
        assert!(parse_coords(bad).is_err(), "{bad:?}");
    }
}

/// §17.1: with the window open nothing reaches the game. Keys stay out of
/// `Controls`, T shuts it unless a text field has the keys, and every release
/// still gets through so nothing sticks.
#[test]
fn an_open_window_owns_all_input() {
    let mut c = Controls::default();
    // Closed: game keys are the game's; T opens it, but not while typing in
    // the look panel, and an auto-repeat does nothing.
    assert_eq!(route_key(KeyCode::KeyW, true, false, false, false), Route::Game);
    assert_eq!(route_key(KeyCode::KeyT, true, false, false, false), Route::OpenMenu);
    assert_eq!(route_key(KeyCode::KeyT, true, true, false, false), Route::Window);
    assert_eq!(route_key(KeyCode::KeyT, true, false, false, true), Route::Game);
    // Open: nothing is the game's.
    for code in [KeyCode::KeyW, KeyCode::Space, KeyCode::KeyG, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::F1, KeyCode::F10, KeyCode::Digit1, KeyCode::BracketRight, KeyCode::KeyF, KeyCode::KeyM] {
        assert_eq!(route_key(code, true, false, true, false), Route::Window, "{code:?}");
    }
    assert_eq!(route_key(KeyCode::Escape, true, false, true, false), Route::MenuBack);
    assert_eq!(route_key(KeyCode::KeyT, true, false, true, false), Route::CloseMenu);
    assert_eq!(route_key(KeyCode::KeyT, true, false, true, true), Route::Window, "typing a T in the title");
    assert_eq!(route_key(KeyCode::KeyT, true, true, true, false), Route::Window, "a held T does not flicker it");
    // A key held when the window opened: opening lets go of it, and its
    // release passes through harmlessly.
    assert!(c.key(KeyCode::KeyW, true));
    assert!(c.input().forward);
    c.release_all();
    assert!(!c.input().forward, "opening the window stops the player");
    assert_eq!(route_key(KeyCode::KeyW, false, false, true, false), Route::Game);
    assert!(!c.key(KeyCode::KeyW, false));
}

/// §17.1: Esc backs out one layer at a time; the next Esc shuts the window.
#[test]
fn esc_backs_out_one_layer_at_a_time() {
    let mut t = Test::new("esc", 1.0, (1920, 1080));
    let _ = t.shot_at("shot_a", 10.0, 20.0);
    t.menu.rescan();
    t.settle();
    t.click("photo-0");
    assert_eq!(t.menu.selected(), Some(0));
    t.menu.lightbox = true;
    t.menu.confirm_delete = true;
    assert!(t.menu.escape(true), "a text field takes the first Esc, and nothing else changes");
    assert!(t.menu.lightbox_open() && t.menu.deleting());
    assert!(t.menu.escape(false));
    assert!(!t.menu.lightbox_open() && t.menu.deleting());
    assert!(t.menu.escape(false));
    assert!(!t.menu.deleting() && t.menu.selected().is_some());
    assert!(t.menu.escape(false));
    assert!(t.menu.selected().is_none());
    assert!(!t.menu.escape(false), "nothing left: the window shuts");
    // A target on the map is a layer too.
    t.click_at(t.at("map", 0.3, 0.3));
    assert!(t.menu.target().is_some());
    assert!(t.menu.escape(false));
    assert!(t.menu.target().is_none());
}

/// Zooming with the wheel keeps the ground under the pointer; dragging pans.
#[test]
fn wheel_and_drag_move_the_map_under_the_pointer() {
    let mut t = Test::new("view", 1.0, (1920, 1080));
    t.settle();
    let p = t.at("map", 0.7, 0.35);
    let rect = t.menu.spots["map"];
    let before = t.menu.view().bounds().unproject(rect, p);
    t.rig.move_to(p);
    t.passes(2);
    t.rig.wheel(6.0);
    t.passes(30);
    assert!(t.menu.view().zoom > 1.2, "zoom {}", t.menu.view().zoom);
    let after = t.menu.view().bounds().unproject(rect, p);
    assert!((after.0 - before.0).abs() < 1e-3 && (after.1 - before.1).abs() < 1e-3, "{before:?} {after:?}");
    let v = t.menu.view();
    t.rig.move_to(t.at("map", 0.5, 0.5));
    t.pass();
    t.rig.button(true);
    t.passes(2);
    t.rig.move_to(t.at("map", 0.4, 0.5));
    t.passes(2);
    t.rig.button(false);
    t.passes(2);
    assert!(t.menu.view().lon > v.lon, "dragging left moves the view east: {} {}", t.menu.view().lon, v.lon);
}

/// The map draws off the main thread: a pass of the window stays fast while a
/// build is running; moving on cancels the stale build, and its picture never
/// lands; shutting the window stops everything.
#[test]
fn a_build_never_holds_up_the_window_and_stale_builds_stop() {
    let mut t = Test::new("hitch", 1.0, (1920, 1080));
    let mut worst = 0.0f64;
    let mut started = false;
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_secs() < 30 {
        let p = std::time::Instant::now();
        t.pass();
        worst = worst.max(p.elapsed().as_secs_f64() * 1e3);
        started |= t.menu.builder.building().is_some();
        if started && t.menu.builder.building().is_none() {
            break;
        }
    }
    assert!(started, "a build ran");
    assert!(t.menu.picture.is_some(), "and landed");
    let full = t.menu.last_build_ms;
    assert!(worst < full.max(1.0), "a window pass took {worst:.1} ms against a {full:.1} ms build");
    println!("worst window pass {worst:.1} ms during a {full:.1} ms build");
    // Ten views in a row, each cancelling the last.
    let src = t.menu.source.clone().unwrap();
    let mut b = MapBuilder::default();
    let sig = t.menu.picture.as_ref().unwrap().1;
    for i in 0..10 {
        let mut s = sig;
        s.bounds.lon_left += i as f64;
        b.start(src.clone(), s, false);
    }
    let last = b.building().unwrap();
    let t1 = std::time::Instant::now();
    let built = loop {
        if let Some(x) = b.poll() {
            break x;
        }
        assert!(t1.elapsed().as_secs() < 30);
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(built.sig, last, "only the last view lands");
    while b.running.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        assert!(t1.elapsed().as_secs() < 30, "cancelled builds stopped");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // Shut: a build under way stops and its picture is dropped.
    b.start(src, sig, false);
    b.cancel();
    assert!(b.building().is_none());
    while b.running.load(std::sync::atomic::Ordering::SeqCst) > 0 {
        assert!(t1.elapsed().as_secs() < 30);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(b.poll().is_none());
}

/// With the window shut, drawing it costs a comparison: no threads, no work.
#[test]
fn a_shut_window_does_nothing() {
    let mut t = Test::new("shut", 1.0, (1920, 1080));
    t.settle();
    t.menu.hide();
    let before = t.menu.spots.len();
    t.menu.spots.clear();
    t.passes(3);
    assert!(t.menu.spots.is_empty() && before > 0, "nothing was laid out");
    assert!(!t.menu.busy());
}

/// A map click sets a target; Teleport goes there, on the ground. Typed
/// coordinates beat the target, and carry their altitude. Places go straight
/// to `Preset`.
#[test]
fn every_way_of_going_somewhere() {
    let mut t = Test::new("go", 1.0, (1920, 1080));
    t.settle();
    t.click_at(t.at("map", 0.25, 0.25));
    let (lat, lon) = t.menu.target().expect("a target");
    assert!((lat - 45.0).abs() < 2.0 && (lon + 90.0).abs() < 2.0, "{lat} {lon}");
    t.click("teleport");
    assert_eq!(t.went.take(), Some(Go::Point { lat, lon, alt_m: None }));
    assert!(!t.menu.open, "going shuts the window");

    t.menu.show();
    t.settle();
    t.type_into("coords", "12.5 -40.25 900");
    t.click("teleport");
    assert_eq!(t.went.take(), Some(Go::Point { lat: 12.5, lon: -40.25, alt_m: Some(900.0) }));

    t.menu.show();
    t.settle();
    t.type_into("coords", "no such place");
    assert_eq!(t.menu.destination(&MenuEnv { engine: &t.session.engine.clone(), bake_sha256: "", presets: &[], here: (0.0, 0.0), can_move: true, day_length_s: 1440.0 }), None);
    t.menu.coords.clear();
    t.passes(2);
    t.click("place-3");
    assert_eq!(t.went.take(), Some(Go::Place(3)));

    // Watching a replay the window is for looking only.
    t.menu.show();
    t.can_move = false;
    t.settle();
    t.click("place-3");
    assert_eq!(t.went.take(), None);
    t.type_into("coords", "1 2");
    t.click("teleport");
    assert_eq!(t.went.take(), None);
}

/// Every photo is a marker and a row; clicking either selects it; the notes
/// reach the sidecar through the text boxes; Teleport goes to the exact pose
/// when it is the same world and only near it when it isn't, and takes the
/// clock back only when asked.
#[test]
fn the_photo_roll_and_going_to_a_photo() {
    let mut t = Test::new("roll", 1.0, (1920, 1080));
    t.session.sky.seconds = 500.0;
    let png = t.shot_at("shot_here", 10.0, 20.0);
    let far = t.shot_at("shot_far", -30.0, -100.0);
    let other = t.session_sidecar(&far);
    let mut moved = other.clone();
    moved["world"]["seed"] = 7.into();
    moved["pose"]["lat"] = (-30.0).into();
    moved["pose"]["lon"] = (-100.0).into();
    std::fs::write(far.with_extension("json"), serde_json::to_string(&moved).unwrap()).unwrap();
    t.menu.rescan();
    t.settle();
    assert_eq!(t.menu.photos().len(), 2);
    let i = t.menu.photos().iter().position(|p| p.png == png).unwrap();
    // The marker: (10, 20) is at x = 200/360, y = 80/180 of the whole map.
    t.click_at(t.at("map", 200.0 / 360.0, 80.0 / 180.0));
    assert_eq!(t.menu.selected(), Some(i), "the marker selects its photo");
    t.settle();
    t.type_into("title", "The shore");
    t.type_into("notes", "Dark wedges by the water");
    t.click("save");
    let js: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(png.with_extension("json")).unwrap()).unwrap();
    assert_eq!((js["title"].as_str(), js["notes"].as_str()), (Some("The shore"), Some("Dark wedges by the water")));
    assert_eq!(js["format"], photos::FORMAT, "and the rest is still there");

    // The preview and the lightbox decode off this thread and arrive.
    let t0 = std::time::Instant::now();
    while t.menu.preview.is_none() {
        t.pass();
        assert!(t0.elapsed().as_secs() < 20, "the preview arrived");
    }
    t.passes(2);
    t.click("preview");
    assert!(t.menu.lightbox_open());
    while t.menu.full.is_none() {
        t.pass();
        assert!(t0.elapsed().as_secs() < 20, "the full picture arrived");
    }
    assert!(t.menu.escape(false));

    // The same world: exact, and the clock only when asked.
    t.click("teleport");
    let go = t.went.take().expect("gone");
    let Go::Photo(p) = &go else { panic!("{go:?}") };
    assert!(p.exact.is_some() && p.time.is_none(), "{p:?}");
    let settings = Settings::default();
    let acts = actions(&go, &t.session, &settings);
    assert_eq!(acts.len(), 1, "no clock unless asked");
    let Action::Pose(_) = &acts[0] else { panic!("{acts:?}") };

    t.menu.show();
    t.menu.restore_time = true;
    t.settle();
    let i = t.menu.photos().iter().position(|p| p.png == png).unwrap();
    t.click(&format!("photo-{i}"));
    t.click("teleport");
    let go = t.went.take().expect("gone");
    let Go::Photo(p) = &go else { panic!("{go:?}") };
    assert_eq!(p.time, Some(TimeGo::Clock(500.0)));
    t.session.sky.seconds = 9000.0;
    t.session.sky.speed = 10.0;
    let acts = actions(&go, &t.session, &settings);
    let Action::Sky(c) = &acts[1] else { panic!("{acts:?}") };
    assert_eq!((c.seconds, c.speed), (500.0, 10.0), "the clock goes back; its speed stays yours");

    // Another world: near it, by latitude and longitude, with no exact pose.
    t.menu.show();
    t.settle();
    let i = t.menu.photos().iter().position(|p| p.png == far).unwrap();
    t.click(&format!("photo-{i}"));
    t.click("teleport");
    let go = t.went.take().expect("gone");
    let Go::Photo(p) = &go else { panic!("{go:?}") };
    assert!(p.exact.is_none());
    let Action::Pose(pose) = &actions(&go, &t.session, &settings)[0] else { panic!() };
    let (lat, lon) = Planet::lat_lon_deg(pose.pos);
    assert!((lat + 30.0).abs() < 1e-6 && (lon + 100.0).abs() < 1e-6, "{lat} {lon}");
}

/// Delete asks first, then moves the ticked photos to trash/.
#[test]
fn delete_asks_then_moves_to_the_trash() {
    let mut t = Test::new("delete", 1.0, (1920, 1080));
    let keep = t.shot_at("shot_keep", 1.0, 1.0);
    let gone = t.shot_at("shot_gone", 2.0, 2.0);
    std::fs::write(t.dir.join("bugs-found-1.txt"), "Austin's notes").unwrap();
    t.menu.rescan();
    t.settle();
    let i = t.menu.photos().iter().position(|p| p.png == gone).unwrap();
    t.click(&format!("tick-{i}"));
    t.click("delete");
    assert!(t.menu.deleting());
    assert!(gone.exists(), "nothing moves before the answer");
    assert!(t.menu.escape(false), "Esc backs out of the question");
    assert!(gone.exists() && !t.menu.deleting());
    t.click("delete");
    t.passes(3);
    t.click("delete-yes");
    t.settle();
    assert!(!gone.exists() && t.dir.join("trash/shot_gone.png").exists() && t.dir.join("trash/shot_gone.json").exists());
    assert!(keep.exists() && t.dir.join("bugs-found-1.txt").exists());
    assert_eq!(t.menu.photos().len(), 1);
}

/// It adapts to the window: at 100% and 150%, big and small, the window fits
/// the screen, the map keeps its shape and the things to press are on screen.
#[test]
fn the_window_fits_at_every_size_and_scale() {
    for (size, scale) in [((1920, 1080), 1.0), ((1920, 1080), 1.5), ((1280, 720), 1.0), ((1280, 720), 1.5), ((2560, 1440), 1.5), ((3840, 2160), 1.5), ((1366, 768), 1.25), ((1024, 600), 1.0)] {
        let mut t = Test::new(&format!("fit{}-{}", size.0, (scale * 100.0) as u32), scale, size);
        let _ = t.shot_at("shot_a", 10.0, 20.0);
        t.menu.rescan();
        t.settle();
        t.click("photo-0");
        t.settle();
        let (w, h) = (size.0 as f32 / scale, size.1 as f32 / scale);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(w, h));
        let map = t.menu.spots["map"];
        assert!(screen.contains_rect(map), "{size:?} at {scale}: map {map:?} on {screen:?}");
        assert!((map.width() / map.height() - 2.0).abs() < 0.02, "{map:?}");
        assert!(map.width() > w * 0.35, "{size:?} at {scale}: the map is {} wide of {w}", map.width());
        // The way out is always on screen, however short the window.
        assert!(screen.contains_rect(t.menu.spots["coords"]), "{size:?} at {scale}");
        assert!(screen.contains_rect(t.menu.spots["teleport"]), "{size:?} at {scale}: {:?}", t.menu.spots["teleport"]);
    }
}

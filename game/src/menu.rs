//! The T window (bug round #11; `docs/feature-inventory.md` §17.3): the map,
//! the photo roll, teleport and the places, in one window.
//!
//! An open window owns all input (§17.1): nothing it sees reaches the game,
//! and Esc backs out of it one layer at a time (`Menu::escape`). The game goes
//! on running behind it, and nothing here touches it directly: the window
//! answers with a `Go`, which `actions` turns into recorded inputs (`pose`,
//! `sky`, `preset`), applied at the next step like any other input. So a
//! teleport during an F10 recording replays exactly.
//!
//! Nothing here runs with the window shut, and nothing slow runs on the main
//! thread with it open: the map builds on its own threads (`map.rs`), and the
//! roll is scanned and photos are decoded on threads of their own.
//!
//! To add to a photo (B6): the window shows a `Photo`, and whatever a sidecar
//! holds beyond that is read and written with `photos::edit_sidecar`. The
//! editor is `ui_photo_editor`; the lightbox is `ui_lightbox`.

use crate::engine::{Engine, Preset};
use crate::grid::Planet;
use crate::map::{self, Base, Built, MapBuilder, River, Sig, Source, View};
use crate::photos::{self, Photo};
use crate::session::{Action, Session};
use crate::sky::{Clock, Settings};
use crate::workshop::seek_hour;
use crate::world::save::Identity;
use egui::{Color32, Pos2, Rect, Stroke, Vec2, vec2};
use glam::DVec3;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

/// The photo preview on the map, and the lightbox: widest picture kept (pixels).
const PREVIEW_PX: usize = 560;
const LIGHTBOX_PX: usize = 2048;
/// Markers within this of the pointer (points) are hit.
const HIT: f32 = 10.0;
/// Height of the teleport controls at the foot of the side column (points).
const TELEPORT_H: f32 = 136.0;

/// Where the player asked to go.
#[derive(Clone, Debug, PartialEq)]
pub enum Go {
    /// A scenic place (`Engine::presets`).
    Place(usize),
    /// A point: with an altitude (m above sea level), hovering there; without
    /// one, on the ground.
    Point { lat: f64, lon: f64, alt_m: Option<f64> },
    Photo(PhotoGo),
}

/// Going to a photo.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoGo {
    /// The exact pose, when the photo was taken in this very world.
    pub exact: Option<String>,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: f64,
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub time: Option<TimeGo>,
}

/// Putting back a photo's time of day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeGo {
    /// The clock itself: sun, moon and season as they were.
    Clock(f64),
    /// A world with another day length: the same local hour.
    Hour(f64),
}

/// The inputs that do what `go` asks. They go in the queue like key presses,
/// so a recording holds them and a replay does the same.
pub fn actions(go: &Go, session: &Session, settings: &Settings) -> Vec<Action> {
    match go {
        Go::Place(i) => vec![Action::Preset(*i)],
        Go::Point { lat, lon, alt_m } => vec![Action::Pose(Box::new(session.teleport_pose(Planet::dir_of_lat_lon_deg(*lat, *lon), *alt_m)))],
        Go::Photo(p) => {
            let dir = Planet::dir_of_lat_lon_deg(p.lat, p.lon);
            let exact = p.exact.as_ref().and_then(|t| crate::replay::parse_pose(t, session.engine.planet.block_size).ok());
            let player = match exact {
                Some(mut e) => {
                    // Where it stood and looked; not what it was carrying.
                    e.vel = DVec3::ZERO;
                    e.selected = session.player.selected;
                    e.fly_speed = session.player.fly_speed;
                    e
                }
                None => {
                    let mut e = session.teleport_pose(dir, Some(p.alt_m));
                    e.heading = p.heading_deg.to_radians();
                    e.pitch = p.pitch_deg.to_radians().clamp(-crate::session::PITCH_MAX, crate::session::PITCH_MAX);
                    e
                }
            };
            let mut out = vec![Action::Pose(Box::new(player))];
            let lon = dir.y.atan2(dir.x);
            match p.time {
                Some(TimeGo::Clock(seconds)) => out.push(Action::Sky(Clock { seconds, speed: session.sky.speed })),
                Some(TimeGo::Hour(h)) => out.push(Action::Sky(Clock { seconds: seek_hour(settings, session.sky.seconds, lon, h), speed: session.sky.speed })),
                None => {}
            }
            out
        }
    }
}

/// Typed coordinates: `lat lon [alt]`, degrees and metres above sea level,
/// spaces or commas between them. Only finite numbers on the planet pass.
pub fn parse_coords(text: &str) -> Result<(f64, f64, Option<f64>), String> {
    let parts: Vec<&str> = text.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
    if !(2..=3).contains(&parts.len()) {
        return Err("type: lat lon [alt]".into());
    }
    let mut v = [0.0f64; 3];
    for (i, p) in parts.iter().enumerate() {
        v[i] = p.parse::<f64>().ok().filter(|x| x.is_finite()).ok_or(format!("{p:?} is not a number"))?;
    }
    if v[0].abs() > 90.0 {
        return Err("latitude runs from -90 to 90".into());
    }
    if v[1].abs() > 360.0 {
        return Err("longitude runs from -180 to 180".into());
    }
    // Only a longitude past the date line is wrapped: arithmetic would nudge
    // what was typed.
    let lon = if v[1].abs() > 180.0 { (v[1] + 180.0).rem_euclid(360.0) - 180.0 } else { v[1] };
    Ok((v[0], lon, (parts.len() == 3).then_some(v[2])))
}

/// What the window needs from the game this frame.
pub struct MenuEnv<'a> {
    pub engine: &'a Arc<Engine>,
    pub bake_sha256: &'a str,
    pub presets: &'a [Preset],
    /// Where the player is: lat, lon (degrees).
    pub here: (f64, f64),
    /// False while a replay is being watched: going anywhere would break it.
    pub can_move: bool,
    pub day_length_s: f64,
}

/// A decoded picture on its way from a thread.
struct Decoded {
    png: PathBuf,
    max_w: usize,
    image: Option<egui::ColorImage>,
}

struct Shown {
    png: PathBuf,
    tex: egui::TextureHandle,
}

pub struct Menu {
    pub open: bool,
    /// The mouse was captured when the window opened; closing gives it back.
    pub recapture: bool,
    dir: PathBuf,
    photos: Vec<Photo>,
    scanning: Option<mpsc::Receiver<Vec<Photo>>>,
    selected: Option<usize>,
    checked: BTreeSet<usize>,
    confirm_delete: bool,
    lightbox: bool,
    preview: Option<Shown>,
    full: Option<Shown>,
    asked: Vec<(PathBuf, usize)>,
    decoded_tx: mpsc::Sender<Decoded>,
    decoded_rx: mpsc::Receiver<Decoded>,
    /// The map.
    view: View,
    base: Base,
    relief: bool,
    show_rivers: bool,
    show_lakes: bool,
    show_markers: bool,
    source: Option<Arc<Source>>,
    builder: MapBuilder,
    picture: Option<(egui::TextureHandle, Sig, f64)>,
    rivers: Arc<Vec<River>>,
    rivers_coming: Option<mpsc::Receiver<Vec<River>>>,
    /// The last time a build landed (ms), shown while looking at speed.
    pub last_build_ms: f64,
    /// Teleport.
    target: Option<(f64, f64)>,
    coords: String,
    restore_time: bool,
    status: String,
    last_screen: Option<Vec2>,
    /// Passes left to hold the window in the centre: after the screen changes
    /// egui learns the window's new size a pass late.
    recentre: u32,
    /// A marker was clicked: show its row.
    scroll_to_selected: bool,
    /// Where things are on screen: the harness clicks them by name.
    pub spots: HashMap<String, Rect>,
}

impl Menu {
    pub fn new(dir: PathBuf) -> Self {
        let (decoded_tx, decoded_rx) = mpsc::channel();
        Self {
            open: false,
            recapture: false,
            dir,
            photos: Vec::new(),
            scanning: None,
            selected: None,
            checked: BTreeSet::new(),
            confirm_delete: false,
            lightbox: false,
            preview: None,
            full: None,
            asked: Vec::new(),
            decoded_tx,
            decoded_rx,
            view: View::default(),
            base: Base::Biomes,
            relief: true,
            show_rivers: true,
            show_lakes: true,
            show_markers: true,
            source: None,
            builder: MapBuilder::default(),
            picture: None,
            rivers: Arc::new(Vec::new()),
            rivers_coming: None,
            last_build_ms: 0.0,
            target: None,
            coords: String::new(),
            restore_time: false,
            status: String::new(),
            last_screen: None,
            recentre: 0,
            scroll_to_selected: false,
            spots: HashMap::new(),
        }
    }

    /// Open the window: a fresh look at the roll and the whole planet.
    pub fn show(&mut self) {
        self.open = true;
        self.selected = None;
        self.checked.clear();
        self.confirm_delete = false;
        self.lightbox = false;
        self.preview = None;
        self.full = None;
        self.target = None;
        self.coords.clear();
        self.view = View::default();
        self.status.clear();
        self.rescan();
    }

    /// Shut the window: anything still being built is dropped.
    pub fn hide(&mut self) {
        self.open = false;
        self.builder.cancel();
        self.scanning = None;
    }

    fn rescan(&mut self) {
        let (tx, rx) = mpsc::channel();
        let dir = self.dir.clone();
        std::thread::spawn(move || {
            let _ = tx.send(photos::scan(&dir));
        });
        self.scanning = Some(rx);
    }

    /// Is a map being built, or the roll being read?
    pub fn busy(&self) -> bool {
        self.builder.building().is_some() || self.scanning.is_some() || self.rivers_coming.is_some() || !self.asked.is_empty()
    }

    /// The photos on the roll.
    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn target(&self) -> Option<(f64, f64)> {
        self.target
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn lightbox_open(&self) -> bool {
        self.lightbox
    }

    pub fn deleting(&self) -> bool {
        self.confirm_delete
    }

    /// Esc, one layer at a time: a text field lets go of the keys, then the
    /// lightbox, the delete question, the selection. False: nothing left, so
    /// the window shuts.
    pub fn escape(&mut self, typing: bool) -> bool {
        if typing {
            return true; // egui takes the focus away
        }
        if self.lightbox {
            self.lightbox = false;
            self.full = None;
            return true;
        }
        if self.confirm_delete {
            self.confirm_delete = false;
            return true;
        }
        if self.selected.is_some() || self.target.is_some() {
            self.selected = None;
            self.preview = None;
            self.target = None;
            return true;
        }
        false
    }

    /// Pick up whatever the threads finished.
    fn collect(&mut self, ctx: &egui::Context) {
        if let Some(Ok(roll)) = self.scanning.as_ref().map(|r| r.try_recv()) {
            self.scanning = None;
            // Keep the selection on the same photo.
            let keep = self.selected.and_then(|i| self.photos.get(i)).map(|p| p.png.clone());
            self.photos = roll;
            self.selected = keep.and_then(|png| self.photos.iter().position(|p| p.png == png));
            self.checked.clear();
            if self.status.is_empty() {
                self.status = format!("{} photos", self.photos.len());
            }
        }
        if let Some(Ok(r)) = self.rivers_coming.as_ref().map(|r| r.try_recv()) {
            self.rivers_coming = None;
            self.rivers = Arc::new(r);
        }
        while let Ok(d) = self.decoded_rx.try_recv() {
            self.asked.retain(|a| !(a.0 == d.png && a.1 == d.max_w));
            let Some(image) = d.image else { continue };
            let tex = ctx.load_texture(if d.max_w == PREVIEW_PX { "photo-preview" } else { "photo-full" }, image, egui::TextureOptions::LINEAR);
            let shown = Shown { png: d.png, tex };
            if d.max_w == PREVIEW_PX {
                self.preview = Some(shown);
            } else {
                self.full = Some(shown);
            }
        }
        if let Some(b) = self.builder.poll() {
            self.install(ctx, b);
        }
    }

    fn install(&mut self, ctx: &egui::Context, b: Built) {
        if !b.rough {
            self.last_build_ms = b.ms;
        }
        let tex = ctx.load_texture("planet-map", b.image, egui::TextureOptions::LINEAR);
        self.picture = Some((tex, b.sig, b.interval_m));
    }

    /// Ask for a photo, decoded off this thread, if it isn't here or coming.
    fn want_picture(&mut self, png: &Path, max_w: usize) {
        let have = if max_w == PREVIEW_PX { &self.preview } else { &self.full };
        if have.as_ref().is_some_and(|s| s.png == png) || self.asked.iter().any(|a| a.0 == png && a.1 == max_w) {
            return;
        }
        self.asked.push((png.to_path_buf(), max_w));
        let (tx, path) = (self.decoded_tx.clone(), png.to_path_buf());
        std::thread::spawn(move || {
            let image = decode(&path, max_w);
            let _ = tx.send(Decoded { png: path, max_w, image });
        });
    }

    /// Where the Teleport button goes. A selected photo first, then a target
    /// on the map or typed.
    fn destination(&self, env: &MenuEnv) -> Option<Go> {
        if let Some(p) = self.selected.and_then(|i| self.photos.get(i)) {
            let here = Identity::of(&env.engine.config);
            let same = p.world.as_ref() == Some(&here) && p.bake_sha256.as_deref() == Some(env.bake_sha256);
            let time = self.restore_time.then(|| match (p.clock_seconds, p.day_length_s, p.local_hour) {
                (Some(c), Some(d), _) if d == env.day_length_s => Some(TimeGo::Clock(c)),
                (_, _, Some(h)) => Some(TimeGo::Hour(h)),
                _ => None,
            });
            return Some(Go::Photo(PhotoGo { exact: if same { p.exact.clone() } else { None }, lat: p.lat, lon: p.lon, alt_m: p.alt_m, heading_deg: p.heading_deg, pitch_deg: p.pitch_deg, time: time.flatten() }));
        }
        if let Ok((lat, lon, alt_m)) = parse_coords(&self.coords) {
            return Some(Go::Point { lat, lon, alt_m });
        }
        self.target.map(|(lat, lon)| Go::Point { lat, lon, alt_m: None })
    }

    fn spot(&mut self, name: &str, rect: Rect) {
        self.spots.insert(name.to_string(), rect);
    }

    /// Draw the window. Returns where to go when the player commits; the
    /// window shuts itself then.
    pub fn ui(&mut self, ctx: &egui::Context, env: &MenuEnv) -> Option<Go> {
        if !self.open {
            return None;
        }
        self.collect(ctx);
        let fresh = self.source.as_ref().is_none_or(|s| !s.current(env.engine));
        if fresh {
            self.source = Some(Arc::new(Source::new(env.engine)));
            self.picture = None;
            self.builder.cancel();
        }
        if self.rivers.is_empty() && self.rivers_coming.is_none() {
            let engine = env.engine.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(engine.bake.hydro().map_or(Vec::new(), map::rivers));
            });
            self.rivers_coming = Some(rx);
        }
        let screen = ctx.content_rect();
        // Fitted to the screen every frame and centred on any resize.
        let fit = vec2(screen.width() * 0.92, screen.height() * 0.90);
        if self.last_screen != Some(screen.size()) {
            self.recentre = 3;
        }
        self.last_screen = Some(screen.size());
        let mut window = egui::Window::new("Map, photos and teleport").collapsible(false).resizable(false).fixed_size(fit).pivot(egui::Align2::CENTER_CENTER).default_pos(screen.center());
        if self.recentre > 0 {
            self.recentre -= 1;
            window = window.current_pos(screen.center());
        }
        let mut go = None;
        window.show(ctx, |ui| {
            self.ui_top(ui);
            ui.separator();
            let height = ui.available_height();
            let side = (ui.available_width() * 0.30).clamp(270.0, 380.0);
            ui.horizontal_top(|ui| {
                let left = (ui.available_width() - side - 16.0).max(200.0);
                ui.allocate_ui_with_layout(vec2(left, height), egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_min_size(vec2(left, height));
                    self.ui_map(ui, env);
                });
                ui.separator();
                ui.allocate_ui_with_layout(vec2(side, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(side);
                    go = self.ui_side(ui, env, height);
                });
            });
        });
        self.ui_lightbox(ctx, screen);
        self.ui_confirm_delete(ctx);
        if go.is_some() {
            self.hide();
        }
        go
    }

    fn ui_top(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Base:");
            for (i, b) in Base::ALL.into_iter().enumerate() {
                let r = ui.radio_value(&mut self.base, b, b.label()).on_hover_text(b.hint());
                self.spots.insert(format!("base-{i}"), r.rect);
            }
            ui.separator();
            let r = ui.checkbox(&mut self.relief, "Relief").on_hover_text("Hills shaded from the upper left.");
            self.spots.insert("relief".to_string(), r.rect);
            let r = ui.checkbox(&mut self.show_rivers, "Rivers").on_hover_text("River courses from the bake, thicker by flow. Small ones appear as you zoom in.");
            self.spots.insert("rivers".to_string(), r.rect);
            let r = ui.checkbox(&mut self.show_lakes, "Lakes").on_hover_text("Lakes in blue, frozen ones pale.");
            self.spots.insert("lakes".to_string(), r.rect);
            let r = ui.checkbox(&mut self.show_markers, "Markers").on_hover_text("A marker for every photo.");
            self.spots.insert("markers".to_string(), r.rect);
            ui.separator();
            if ui.button("Reset view").clicked() {
                self.view = View::default();
            }
            ui.label(format!("{:.1}x", self.view.zoom));
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&self.status).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("Scroll zooms, drag pans, double-click resets, click sets a target. Esc backs out.").small().weak());
            });
        });
    }

    fn ui_map(&mut self, ui: &mut egui::Ui, env: &MenuEnv) {
        let avail = ui.available_size();
        let width = avail.x.min(avail.y * 2.0).max(200.0);
        let size = vec2(width, width * 0.5);
        ui.add_space(((avail.y - size.y) * 0.5).max(0.0));
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        self.spot("map", rect);

        // The view first, so this frame already shows where it went.
        let mut moving = false;
        let mut did_reset = false;
        if resp.double_clicked() {
            let hit = resp.interact_pointer_pos().and_then(|p| self.marker_at(rect, p));
            if hit.is_some() {
                self.selected = hit;
                self.lightbox = true;
            } else {
                self.view = View::default();
                did_reset = true;
            }
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            self.view.pan(d.x, d.y, rect.size());
            moving = true;
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y) as f64;
            if scroll != 0.0
                && let Some(cur) = resp.hover_pos()
            {
                self.view.zoom_at((scroll * 0.003).exp(), rect, cur);
                moving = true;
            }
        }

        // The picture for this view, if it isn't on its way: not while the
        // view is still moving (a new build each frame would never land).
        let bounds = self.view.bounds();
        let ppp = ui.ctx().pixels_per_point();
        let w = ((rect.width() * ppp).round() as usize).clamp(64, map::MAX_WIDTH);
        let sig = Sig { w, h: (w / 2).max(32), base: self.base, relief: self.relief, lakes: self.show_lakes, bounds };
        let have = self.picture.as_ref().is_some_and(|p| p.1 == sig);
        if !have && !moving && self.builder.building() != Some(sig)
            && let Some(src) = &self.source
        {
            let rough = self.picture.is_none();
            self.builder.start(src.clone(), sig, rough);
        }
        if self.builder.building().is_some() {
            ui.ctx().request_repaint();
        }

        let paint = ui.painter_at(rect);
        paint.rect_filled(rect, 0.0, Color32::from_rgb(8, 22, 42));
        if let Some((tex, built, _)) = &self.picture {
            paint.image(tex.id(), rect, bounds.uv_in(built.bounds), Color32::WHITE);
        }
        if self.show_rivers {
            map::paint_rivers(&paint, rect, &self.view, &self.rivers);
        }
        let in_rect = |p: Pos2| rect.expand(2.0).contains(p);
        if self.show_markers {
            for (i, p) in self.photos.iter().enumerate() {
                let pos = bounds.project(rect, p.lat, p.lon);
                if !in_rect(pos) {
                    continue;
                }
                let sel = self.selected == Some(i);
                let fill = if sel {
                    Color32::from_rgb(255, 230, 90)
                } else if self.checked.contains(&i) {
                    Color32::from_rgb(255, 120, 90)
                } else {
                    Color32::from_rgb(80, 200, 255)
                };
                paint.circle_filled(pos, if sel { 6.0 } else { 4.0 }, fill);
                paint.circle_stroke(pos, if sel { 8.0 } else { 5.5 }, Stroke::new(1.5, Color32::from_black_alpha(160)));
            }
        }
        // You are here.
        let here = bounds.project(rect, env.here.0, env.here.1);
        if in_rect(here) {
            paint.circle_filled(here, 4.0, Color32::from_rgb(120, 255, 140));
            paint.circle_stroke(here, 6.5, Stroke::new(2.0, Color32::from_rgb(20, 90, 30)));
        }
        // The free target (typed coordinates land here too).
        let typed = parse_coords(&self.coords).ok().map(|c| (c.0, c.1));
        if let Some((la, lo)) = typed.or(self.target) {
            let pos = bounds.project(rect, la, lo);
            if in_rect(pos) {
                let (s, st) = (7.0, Stroke::new(2.0, Color32::from_rgb(255, 90, 90)));
                paint.line_segment([pos - vec2(s, 0.0), pos + vec2(s, 0.0)], st);
                paint.line_segment([pos - vec2(0.0, s), pos + vec2(0.0, s)], st);
            }
        }
        // A click picks the nearest marker, or sets a target.
        if resp.clicked()
            && !did_reset
            && let Some(click) = resp.interact_pointer_pos()
        {
            match self.marker_at(rect, click) {
                Some(i) => {
                    self.selected = Some(i);
                    self.target = None;
                    self.coords.clear();
                    self.scroll_to_selected = true;
                }
                None => {
                    let (lat, lon) = bounds.unproject(rect, click);
                    self.target = Some((lat.clamp(-90.0, 90.0), lon.clamp(-180.0, 180.0)));
                    self.selected = None;
                    self.coords.clear();
                }
            }
        }
        // What is under the pointer.
        if let Some(hp) = resp.hover_pos() {
            let text = match self.marker_at(rect, hp) {
                Some(i) => self.photos[i].label(),
                None => {
                    let (lat, lon) = bounds.unproject(rect, hp);
                    format!("{lat:.3} {lon:.3}")
                }
            };
            let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(13.0), Color32::WHITE);
            let at = (hp + vec2(14.0, -8.0)).min(rect.right_bottom() - galley.size() - vec2(6.0, 6.0));
            ui.painter().rect_filled(Rect::from_min_size(at, galley.size()).expand(4.0), 4.0, Color32::from_black_alpha(200));
            ui.painter().galley(at, galley, Color32::WHITE);
        }
        self.ui_map_notes(&paint, rect);
        self.ui_preview(ui, rect);
    }

    /// The nearest photo marker within reach of a point.
    fn marker_at(&self, rect: Rect, p: Pos2) -> Option<usize> {
        if !self.show_markers {
            return None;
        }
        let b = self.view.bounds();
        self.photos.iter().enumerate().map(|(i, ph)| (i, b.project(rect, ph.lat, ph.lon).distance(p))).filter(|(_, d)| *d < HIT).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    }

    fn ui_map_notes(&self, paint: &egui::Painter, rect: Rect) {
        let mut text = String::new();
        if self.builder.building().is_some() {
            text = "Drawing the map...".into();
        } else if let Some((_, _, interval)) = &self.picture
            && *interval > 0.0
        {
            text = format!("Contours every {interval} m");
        }
        if text.is_empty() {
            return;
        }
        let g = paint.layout_no_wrap(text, egui::FontId::proportional(13.0), Color32::WHITE);
        let at = rect.left_top() + vec2(8.0, 8.0);
        paint.rect_filled(Rect::from_min_size(at, g.size()).expand(4.0), 4.0, Color32::from_black_alpha(170));
        paint.galley(at, g, Color32::WHITE);
    }

    /// The selected photo over the map's corner: painted, not laid out, so
    /// selecting one can't change the window's size.
    fn ui_preview(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let Some(png) = self.selected.and_then(|i| self.photos.get(i)).map(|p| p.png.clone()) else { return };
        self.want_picture(&png, PREVIEW_PX);
        let Some(shown) = self.preview.as_ref().filter(|s| s.png == png) else { return };
        let ts = shown.tex.size_vec2();
        let s = ((rect.width() * 0.30).clamp(140.0, 420.0) / ts.x).min((rect.height() * 0.42).clamp(90.0, 260.0) / ts.y).min(1.0);
        let pv = Rect::from_min_size(pos(rect.left() + 10.0, rect.bottom() - 10.0 - ts.y * s), ts * s);
        let paint = ui.painter_at(rect);
        paint.rect_filled(pv.expand(3.0), 4.0, Color32::from_black_alpha(190));
        paint.image(shown.tex.id(), pv, Rect::from_min_max(pos(0.0, 0.0), pos(1.0, 1.0)), Color32::WHITE);
        paint.rect_stroke(pv.expand(3.0), 4.0, Stroke::new(1.5, Color32::from_rgb(255, 230, 90)), egui::StrokeKind::Outside);
        self.spot("preview", pv);
        let r = ui.interact(pv, ui.id().with("photo-preview"), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Click for the full picture.");
        if r.clicked() {
            self.lightbox = true;
        }
    }

    fn ui_side(&mut self, ui: &mut egui::Ui, env: &MenuEnv, height: f32) -> Option<Go> {
        let mut go = None;
        // Places and photos scroll; teleport stays put at the bottom, so it is
        // there however short the window is.
        let list = (height - TELEPORT_H).max(60.0);
        egui::ScrollArea::vertical().id_salt("menu-side").auto_shrink([false, false]).max_height(list).show(ui, |ui| {
            ui.strong("Places");
            ui.horizontal_wrapped(|ui| {
                for (i, p) in env.presets.iter().enumerate() {
                    let (lat, lon) = Planet::lat_lon_deg(p.dir);
                    let r = ui.add_enabled(env.can_move, egui::Button::new(&p.name)).on_hover_text(format!("{lat:.3} {lon:.3}"));
                    self.spots.insert(format!("place-{i}"), r.rect);
                    if r.clicked() {
                        go = Some(Go::Place(i));
                    }
                }
            });
            ui.separator();
            self.ui_list(ui, list);
            if self.selected.is_some() {
                ui.separator();
                self.ui_photo_editor(ui);
            }
        });
        ui.separator();
        if let Some(g) = self.ui_teleport(ui, env) {
            go = Some(g);
        }
        go
    }

    fn ui_list(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.horizontal(|ui| {
            ui.strong(format!("Photos ({})", self.photos.len()));
            if ui.small_button("All").on_hover_text("Tick every photo.").clicked() {
                self.checked = (0..self.photos.len()).collect();
            }
            if ui.small_button("None").clicked() {
                self.checked.clear();
            }
            let n = self.checked.len();
            let r = ui.add_enabled(n > 0, egui::Button::new(format!("Delete {n}...")).small()).on_hover_text("Moves the ticked photos to interchange/trash/.");
            self.spots.insert("delete".to_string(), r.rect);
            if r.clicked() {
                self.confirm_delete = true;
            }
        });
        let row = 26.0;
        // Room for the places above and the editor below, when there is one.
        let others = if self.selected.is_some() { 310.0 } else { 120.0 };
        let list = (height - others).max(row * 3.0);
        let n = self.photos.len();
        if n == 0 {
            ui.label(egui::RichText::new("No photos yet. P takes one, Shift+P the live frame.").small().weak());
            return;
        }
        let scroll_to = self.selected.filter(|_| std::mem::take(&mut self.scroll_to_selected));
        egui::ScrollArea::vertical().id_salt("menu-photos").max_height(list).auto_shrink([false, true]).show_rows(ui, row, n, |ui, range| {
            for i in range {
                let sel = self.selected == Some(i);
                ui.horizontal(|ui| {
                    let mut ticked = self.checked.contains(&i);
                    let c = ui.checkbox(&mut ticked, "");
                    self.spots.insert(format!("tick-{i}"), c.rect);
                    if c.changed() {
                        if ticked {
                            self.checked.insert(i);
                        } else {
                            self.checked.remove(&i);
                        }
                    }
                    let p = &self.photos[i];
                    let r = ui.selectable_label(sel, p.label());
                    self.spots.insert(format!("photo-{i}"), r.rect);
                    if scroll_to == Some(i) {
                        r.scroll_to_me(Some(egui::Align::Center));
                    }
                    let mut hover = p.name.clone();
                    if !p.notes.trim().is_empty() {
                        hover.push_str("\n\n");
                        hover.push_str(&p.notes);
                    }
                    let r = r.on_hover_text(hover);
                    if r.clicked() || r.double_clicked() {
                        self.selected = Some(i);
                        self.target = None;
                        self.coords.clear();
                    }
                    if r.double_clicked() {
                        self.lightbox = true;
                    }
                });
            }
        });
    }

    /// Title and notes of the selected photo, kept in its sidecar.
    fn ui_photo_editor(&mut self, ui: &mut egui::Ui) {
        let Some(sel) = self.selected.filter(|&i| i < self.photos.len()) else { return };
        ui.label("Title");
        let t = ui.add(egui::TextEdit::singleline(&mut self.photos[sel].title).hint_text("A name for this shot").desired_width(f32::INFINITY));
        self.spots.insert("title".to_string(), t.rect);
        ui.label("Notes");
        let n = ui.add(egui::TextEdit::multiline(&mut self.photos[sel].notes).hint_text("Notes about this shot").desired_rows(2).desired_width(f32::INFINITY));
        self.spots.insert("notes".to_string(), n.rect);
        let mut save = t.lost_focus() || n.lost_focus();
        ui.horizontal(|ui| {
            let r = ui.button("Save");
            self.spots.insert("save".to_string(), r.rect);
            save |= r.clicked();
            let r = ui.button("Repro").on_hover_text("Write a script beside the photo that takes this frame again, for a bug report.");
            self.spots.insert("repro".to_string(), r.rect);
            if r.clicked() {
                self.status = match photos::repro(&self.photos[sel].png) {
                    Ok(r) => {
                        println!("repro of {}:\n  {}\n  {}", self.photos[sel].name, r.command, r.at_command);
                        format!("Repro written: {}", r.script.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()))
                    }
                    Err(e) => format!("No repro: {e}"),
                };
            }
        });
        if save {
            self.status = match photos::save_notes(&self.photos[sel]) {
                Ok(()) => format!("Saved the notes of {}", self.photos[sel].name),
                Err(e) => format!("Notes not saved: {e}"),
            };
        }
    }

    fn ui_teleport(&mut self, ui: &mut egui::Ui, env: &MenuEnv) -> Option<Go> {
        ui.horizontal(|ui| {
            ui.label("lat lon [alt m]");
            let r = ui.add(egui::TextEdit::singleline(&mut self.coords).hint_text("12.5 -40.2 300").desired_width(f32::INFINITY));
            self.spots.insert("coords".to_string(), r.rect);
        });
        if !self.coords.trim().is_empty()
            && let Err(e) = parse_coords(&self.coords)
        {
            ui.colored_label(Color32::from_rgb(255, 150, 120), e);
        }
        ui.checkbox(&mut self.restore_time, "Restore photo's time of day").on_hover_text("Teleporting to a photo also puts the sun, moon and season back as they were.");
        let dest = self.destination(env);
        let label = match &dest {
            Some(Go::Photo(p)) if p.exact.is_some() => "Teleport to photo",
            Some(Go::Photo(_)) => "Teleport near photo",
            Some(_) if !self.coords.trim().is_empty() => "Teleport to coordinates",
            Some(_) => "Teleport to target",
            None => "Teleport",
        };
        let r = ui.add_enabled(dest.is_some() && env.can_move, egui::Button::new(egui::RichText::new(label).strong()));
        let r = if !env.can_move {
            r.on_disabled_hover_text("Not while watching a replay.")
        } else if matches!(&dest, Some(Go::Photo(p)) if p.exact.is_none()) {
            r.on_hover_text("This photo was taken in another world, so you go to the same latitude, longitude and height, not the exact spot.")
        } else {
            r
        };
        self.spots.insert("teleport".to_string(), r.rect);
        r.clicked().then_some(dest).flatten()
    }

    fn ui_lightbox(&mut self, ctx: &egui::Context, screen: Rect) {
        if !self.lightbox {
            return;
        }
        let Some(p) = self.selected.and_then(|i| self.photos.get(i)).cloned() else {
            self.lightbox = false;
            return;
        };
        self.want_picture(&p.png, LIGHTBOX_PX);
        let full = &self.full;
        let modal = egui::Modal::new(egui::Id::new("photo-lightbox")).backdrop_color(Color32::from_black_alpha(170)).show(ctx, |ui| {
            let (max_w, max_h) = (screen.width() * 0.86, (screen.height() * 0.86 - 70.0).max(120.0));
            match full.as_ref().filter(|s| s.png == p.png) {
                Some(s) => {
                    let ts = s.tex.size_vec2();
                    let k = (max_w / ts.x).min(max_h / ts.y).min(2.0);
                    ui.set_max_width((ts.x * k).max(240.0));
                    ui.image((s.tex.id(), ts * k));
                }
                None => {
                    ui.set_max_width(320.0);
                    ui.label("Loading the picture...");
                }
            }
            ui.add_space(4.0);
            ui.strong(if p.title.trim().is_empty() { &p.name } else { &p.title });
            if !p.notes.trim().is_empty() {
                ui.label(egui::RichText::new(&p.notes).weak());
            }
            ui.label(egui::RichText::new("Click outside it or press Esc to close.").small().weak());
        });
        if modal.should_close() {
            self.lightbox = false;
            self.full = None;
        }
    }

    fn ui_confirm_delete(&mut self, ctx: &egui::Context) {
        if !self.confirm_delete {
            return;
        }
        let n = self.checked.len();
        let mut delete = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("photo-delete")).show(ctx, |ui| {
            ui.label(format!("Move {n} photo{} and everything saved with {} to interchange/trash/? Nothing is destroyed.", if n == 1 { "" } else { "s" }, if n == 1 { "it" } else { "them" }));
            ui.horizontal(|ui| {
                let r = ui.button("Move to trash");
                self.spots.insert("delete-yes".to_string(), r.rect);
                delete = r.clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if delete {
            self.delete_checked();
        }
        if delete || cancel || modal.should_close() {
            self.confirm_delete = false;
        }
    }

    fn delete_checked(&mut self) {
        let (mut files, mut failed) = (0, Vec::new());
        for &i in &self.checked {
            if let Some(p) = self.photos.get(i) {
                match photos::trash(&self.dir, &p.png) {
                    Ok(n) => files += n,
                    Err(e) => failed.push(e),
                }
            }
        }
        self.status = match failed.first() {
            None => format!("Moved {files} file(s) to interchange/trash/"),
            Some(e) => format!("Moved {files} file(s); {} failed: {e}", failed.len()),
        };
        self.checked.clear();
        self.selected = None;
        self.preview = None;
        self.full = None;
        self.lightbox = false;
        self.rescan();
    }
}

fn pos(x: f32, y: f32) -> Pos2 {
    egui::pos2(x, y)
}

/// A PNG as an egui picture, stepped down to at most `max_w` pixels wide.
fn decode(path: &Path, max_w: usize) -> Option<egui::ColorImage> {
    let (w, h, rgba) = crate::render::read_png(&path.to_string_lossy()).ok()?;
    let (w, h) = (w as usize, h as usize);
    let step = w.div_ceil(max_w).max(1);
    let (nw, nh) = ((w / step).max(1), (h / step).max(1));
    let mut px = Vec::with_capacity(nw * nh);
    for y in 0..nh {
        for x in 0..nw {
            let i = (y * step * w + x * step) * 4;
            px.push(Color32::from_rgb(rgba[i], rgba[i + 1], rgba[i + 2]));
        }
    }
    Some(egui::ColorImage { size: [nw, nh], source_size: vec2(nw as f32, nh as f32), pixels: px })
}

// ---------------------------------------------------------------- headless

/// Synthetic pointer and keyboard input for the window, without a real one:
/// the harness and the tests click and type through this, and egui can't tell
/// the difference. `scale` is the display scale (1.5 = 150%).
pub struct Rig {
    pub ctx: egui::Context,
    pub scale: f32,
    pub pointer: Pos2,
    events: Vec<egui::Event>,
    time: f64,
}

impl Rig {
    pub fn new(ctx: egui::Context, scale: f32) -> Self {
        ctx.all_styles_mut(|s| s.animation_time = 0.0);
        crate::ui::apply_panel_style(&ctx);
        Self { ctx, scale, pointer: Pos2::ZERO, events: Vec::new(), time: 0.0 }
    }

    /// The next pass's input, for a target of `w` x `h` pixels.
    pub fn input(&mut self, w: u32, h: u32) -> egui::RawInput {
        self.time += 1.0 / 60.0;
        let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w as f32 / self.scale, h as f32 / self.scale))), time: Some(self.time), events: std::mem::take(&mut self.events), ..Default::default() };
        if let Some(v) = raw.viewports.get_mut(&egui::ViewportId::ROOT) {
            v.native_pixels_per_point = Some(self.scale);
        }
        raw
    }

    pub fn move_to(&mut self, p: Pos2) {
        self.pointer = p;
        self.events.push(egui::Event::PointerMoved(p));
    }

    pub fn button(&mut self, pressed: bool) {
        self.events.push(egui::Event::PointerButton { pos: self.pointer, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
    }

    pub fn wheel(&mut self, lines: f32) {
        self.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, lines), phase: egui::TouchPhase::Move, modifiers: Default::default() });
    }

    pub fn text(&mut self, s: &str) {
        self.events.push(egui::Event::Text(s.to_string()));
    }

    pub fn key(&mut self, key: egui::Key) {
        self.events.push(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Default::default() });
        self.events.push(egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: Default::default() });
    }
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;

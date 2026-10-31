//! The planet map of the T window (bug round #11; `docs/feature-inventory.md`
//! §17.3): an equirectangular picture re-synthesised from the bake for exactly
//! the view on screen, so zooming in shows real detail and not stretched
//! pixels. The base layers come from the bake: Biomes (the colour of the block
//! the generator puts on top, from the same climate rules), Temp and Precip
//! (annual means; this engine has no seasonal climate yet), Elevation
//! (hypsometric tint with contour bands). Relief is a hillshade. Lakes come from
//! the generator's own columns; rivers are vector lines from `hydro.bin`,
//! drawn by the window so they stay sharp at any zoom.
//!
//! Everything here runs off the main thread. `MapBuilder` starts a build on
//! worker threads of its own (a quarter of the logical cores, at most four, so
//! the streamer's pool and the frame keep the rest) and cancels it the moment
//! the view moves on; the window keeps drawing the last picture, moved to the
//! live view, until the new one lands. Nothing runs while the window is shut.
//! Reading the bake and the generator is pure, so a map can't disturb the
//! game.

use crate::engine::Engine;
use crate::render::textures::to_srgb;
use crate::worldgen::Generator;
use crate::worldgen::bake::Bake;
use crate::worldgen::hydro::{HydroData, WaterKind};
use egui::{Color32, ColorImage};
use glam::DVec3;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

/// Deepest zoom (1 = the whole planet): 2 degrees of longitude across the map.
pub const MAX_ZOOM: f64 = 180.0;
/// The largest map picture: wider than this, the picture is scaled up.
pub const MAX_WIDTH: usize = 1600;

/// A lat/lon rectangle in degrees. `lat_top` is the larger latitude (north
/// up); longitude grows to the right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub lat_top: f64,
    pub lat_bot: f64,
    pub lon_left: f64,
    pub lon_right: f64,
}

impl Bounds {
    pub const WORLD: Bounds = Bounds { lat_top: 90.0, lat_bot: -90.0, lon_left: -180.0, lon_right: 180.0 };

    pub fn project(&self, rect: egui::Rect, lat: f64, lon: f64) -> egui::Pos2 {
        egui::pos2(rect.left() + ((lon - self.lon_left) / (self.lon_right - self.lon_left)) as f32 * rect.width(), rect.top() + ((self.lat_top - lat) / (self.lat_top - self.lat_bot)) as f32 * rect.height())
    }

    /// A point of `rect` as (lat, lon) degrees.
    pub fn unproject(&self, rect: egui::Rect, p: egui::Pos2) -> (f64, f64) {
        let fx = ((p.x - rect.left()) / rect.width()) as f64;
        let fy = ((p.y - rect.top()) / rect.height()) as f64;
        (self.lat_top + fy * (self.lat_bot - self.lat_top), self.lon_left + fx * (self.lon_right - self.lon_left))
    }

    /// Where this rectangle lies inside a picture made for `built`: a moved
    /// view shows the old pixels in the right place until the new ones land.
    pub fn uv_in(&self, built: Bounds) -> egui::Rect {
        let sx = |lon: f64| ((lon - built.lon_left) / (built.lon_right - built.lon_left)) as f32;
        let sy = |lat: f64| ((built.lat_top - lat) / (built.lat_top - built.lat_bot)) as f32;
        egui::Rect::from_min_max(egui::pos2(sx(self.lon_left), sy(self.lat_top)), egui::pos2(sx(self.lon_right), sy(self.lat_bot)))
    }
}

/// The pan and zoom of the map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f64,
    pub lat: f64,
    pub lon: f64,
}

impl Default for View {
    fn default() -> Self {
        Self { zoom: 1.0, lat: 0.0, lon: 0.0 }
    }
}

impl View {
    pub fn bounds(&self) -> Bounds {
        let (hl, ha) = (180.0 / self.zoom, 90.0 / self.zoom);
        Bounds { lat_top: self.lat + ha, lat_bot: self.lat - ha, lon_left: self.lon - hl, lon_right: self.lon + hl }
    }

    /// Keep the zoom in range and the framed rectangle on the planet.
    pub fn clamp(&mut self) {
        self.zoom = self.zoom.clamp(1.0, MAX_ZOOM);
        let (hl, ha) = (180.0 / self.zoom, 90.0 / self.zoom);
        self.lon = self.lon.clamp(-180.0 + hl, 180.0 - hl);
        self.lat = self.lat.clamp(-90.0 + ha, 90.0 - ha);
    }

    /// Drag the map by (dx, dy) of a `size` map: the ground follows the pointer.
    pub fn pan(&mut self, dx: f32, dy: f32, size: egui::Vec2) {
        let b = self.bounds();
        self.lon -= dx as f64 / size.x as f64 * (b.lon_right - b.lon_left);
        self.lat += dy as f64 / size.y as f64 * (b.lat_top - b.lat_bot);
        self.clamp();
    }

    /// Zoom by `factor`, keeping the ground under `cursor` under it.
    pub fn zoom_at(&mut self, factor: f64, rect: egui::Rect, cursor: egui::Pos2) {
        let (glat, glon) = self.bounds().unproject(rect, cursor);
        let fx = ((cursor.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
        let fy = ((cursor.y - rect.top()) / rect.height()).clamp(0.0, 1.0) as f64;
        self.zoom = (self.zoom * factor).clamp(1.0, MAX_ZOOM);
        self.lon = glon + 180.0 / self.zoom * (1.0 - 2.0 * fx);
        self.lat = glat + 90.0 / self.zoom * (2.0 * fy - 1.0);
        self.clamp();
    }
}

/// The colour field under everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Biomes,
    Temperature,
    Precipitation,
    Elevation,
}

impl Base {
    pub const ALL: [Base; 4] = [Base::Biomes, Base::Temperature, Base::Precipitation, Base::Elevation];

    pub fn label(self) -> &'static str {
        match self {
            Base::Biomes => "Biomes",
            Base::Temperature => "Temp",
            Base::Precipitation => "Precip",
            Base::Elevation => "Elevation",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Base::Biomes => "The block on top of the ground, by the climate, in its colour from the look panel.",
            Base::Temperature => "Mean temperature over the year (this engine has no seasons of climate yet).",
            Base::Precipitation => "Mean rain and snow over the year.",
            Base::Elevation => "Height: green lowlands to white peaks, with contour lines. Every 4th line is stronger, and the spacing follows the land in view as you zoom.",
        }
    }
}

/// What one picture of the map shows: a picture is good for one of these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sig {
    pub w: usize,
    pub h: usize,
    pub base: Base,
    pub relief: bool,
    pub lakes: bool,
    pub bounds: Bounds,
}

/// What a build reads: pure, shared and read-only.
pub struct Source {
    bake: Arc<dyn Bake>,
    generator: Arc<Generator>,
    /// Metres of game terrain per km of bake elevation.
    m_per_km: f64,
    radius_m: f64,
    block_m: f64,
    /// The tallest bake elevation (km): white on the Elevation tint.
    peak_km: f64,
    palette: Palette,
}

/// The surface blocks' top colours (linear), as the look stands.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Palette {
    grass: [f32; 3],
    dry: [f32; 3],
    sand: [f32; 3],
    snow: [f32; 3],
}

impl Source {
    pub fn new(engine: &Engine) -> Self {
        // The live colours, so the map follows the look panel.
        let top = |name: &str| engine.registry.id(name).map_or([0.5; 3], |id| engine.registry.colour(id));
        Self {
            bake: engine.bake.clone(),
            generator: engine.generator.clone(),
            m_per_km: 1000.0 * engine.vscale(),
            radius_m: engine.planet.radius,
            block_m: engine.planet.block_size,
            peak_km: engine.places.peak.1.max(0.1),
            palette: Palette { grass: top("grass"), dry: top("dry_grass"), sand: top("sand"), snow: top("snow") },
        }
    }

    /// Is this the world `engine` shows, in the colours it has now?
    pub fn current(&self, engine: &Engine) -> bool {
        Arc::ptr_eq(&self.generator, &engine.generator) && self.palette == Source::new(engine).palette
    }
}

// ---------------------------------------------------------------- colours
//
// The ramps are the old viewer's (ui.rs), authored in display sRGB and held
// as power-2.2 values while a pixel is worked out.

fn disp(r: f32, g: f32, b: f32) -> [f32; 3] {
    [r.powf(2.2), g.powf(2.2), b.powf(2.2)]
}

fn ramp(stops: &[(f32, [f32; 3])], x: f32) -> [f32; 3] {
    if x <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if x <= b.0 {
            let t = ((x - a.0) / (b.0 - a.0)).clamp(0.0, 1.0);
            return mix(a.1, b.1, t);
        }
    }
    stops[stops.len() - 1].1
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Cold blue to hot red, pale in the temperate middle.
fn temp_stops() -> [(f32, [f32; 3]); 5] {
    [(0.00, disp(0.13, 0.20, 0.62)), (0.30, disp(0.25, 0.55, 0.85)), (0.50, disp(0.80, 0.84, 0.58)), (0.72, disp(0.90, 0.58, 0.24)), (1.00, disp(0.72, 0.10, 0.08))]
}

/// Dry tan to wet blue-green.
fn precip_stops() -> [(f32, [f32; 3]); 4] {
    [(0.00, disp(0.78, 0.70, 0.47)), (0.32, disp(0.72, 0.72, 0.42)), (0.60, disp(0.34, 0.62, 0.45)), (1.00, disp(0.08, 0.45, 0.55))]
}

/// Lowland green, tan, brown, rock grey, white peaks.
fn hypso_stops() -> [(f32, [f32; 3]); 6] {
    [(0.00, disp(0.33, 0.51, 0.29)), (0.16, disp(0.56, 0.61, 0.33)), (0.36, disp(0.79, 0.69, 0.44)), (0.58, disp(0.62, 0.44, 0.28)), (0.80, disp(0.55, 0.48, 0.44)), (1.00, disp(0.96, 0.96, 0.97))]
}

/// The sea by depth, shelf teal to deep navy: one ramp for every base that
/// shows the sea, so changing layers never recolours the ocean.
fn sea_colour(elev_km: f32) -> [f32; 3] {
    let d = (-elev_km / 4.0).clamp(0.0, 1.0);
    [0.10 + (0.02 - 0.10) * d, 0.32 + (0.08 - 0.32) * d, 0.42 + (0.22 - 0.42) * d]
}

const LAKE: [f32; 3] = [0.025, 0.150, 0.550];
const LAKE_FROZEN: [f32; 3] = [0.620, 0.750, 0.840];

/// A block colour (linear) in the map's power-2.2 space.
fn block_colour(c: [f32; 3]) -> [f32; 3] {
    c.map(|x| (to_srgb(x) as f32 / 255.0).powf(2.2))
}

/// The ground's colour by the climate: `Generator::surface`'s thresholds,
/// softened over the dither those rules use, on the blocks' own colours.
fn biome_colour(temp: f32, precip: f32, p: &Palette) -> [f32; 3] {
    let wet = mix(p.dry, p.grass, smooth(260.0, 380.0, precip));
    let desert = smooth(165.0, 95.0, precip) * smooth(6.5, 9.5, temp);
    let ground = mix(wet, p.sand, desert);
    mix(ground, p.snow, smooth(-1.5, -4.5, temp))
}

/// Minor contour spacing (metres) for `range` metres of land in view: about
/// twelve bands, snapped to a 1-2-5 ladder so panning rarely changes it. 0 when
/// there is nothing to contour.
pub fn contour_interval_m(range: f64) -> f64 {
    if !range.is_finite() || range <= 1e-3 {
        return 0.0;
    }
    let raw = (range / 12.0).max(0.25);
    let mag = 10f64.powf(raw.log10().floor());
    let step = raw / mag;
    let nice = if step < 1.5 {
        1.0
    } else if step < 3.5 {
        2.0
    } else if step < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

/// Every Nth contour is an index line.
const CONTOUR_INDEX: i64 = 4;

// ---------------------------------------------------------------- synthesis

#[derive(Clone, Copy, Default)]
struct Cell {
    elev_km: f32,
    temp: f32,
    precip: f32,
    sea: bool,
    /// 0 no lake, 1 water, 2 frozen.
    lake: u8,
}

/// A finished picture.
pub struct Built {
    pub sig: Sig,
    pub image: ColorImage,
    /// The contour spacing it was drawn with (m; 0: none).
    pub interval_m: f64,
    pub ms: f64,
    /// A quick first look: the sharp picture is still coming.
    pub rough: bool,
}

fn sample(src: &Source, dir: DVec3, lakes: bool, voxel_m: f64) -> Cell {
    let s = src.bake.sample(dir);
    let sea = s.ocean >= 0.5 && s.elev_km <= 0.0;
    let mut cell = Cell { elev_km: s.elev_km as f32, temp: s.temp_c as f32, precip: s.precip_mm as f32, sea, lake: 0 };
    // Most of the planet has no lake near it. Where the bake says there
    // may be one, the generator's own column decides.
    if lakes && !sea && s.lake > 0.01 {
        let c = src.generator.column(dir, voxel_m);
        if c.wet() && c.water_kind == WaterKind::Lake {
            cell.lake = if c.temp_c < -4.0 { 2 } else { 1 };
        }
    }
    cell
}

/// Worker threads for one picture.
fn threads() -> usize {
    (std::thread::available_parallelism().map_or(4, |n| n.get()) / 4).clamp(1, 4)
}

/// Sample the view row by row on a few threads. None when cancelled.
fn sample_view(src: &Source, sig: &Sig, cancel: &AtomicBool) -> Option<Vec<Cell>> {
    let (w, h, b) = (sig.w, sig.h, sig.bounds);
    let px_m = (b.lon_right - b.lon_left).to_radians() * src.radius_m / w as f64;
    let voxel_m = px_m.max(src.block_m);
    let mut cells = vec![Cell::default(); w * h];
    let rows = Mutex::new(cells.chunks_mut(w).enumerate());
    let row = || {
        loop {
            let Some((y, line)) = rows.lock().unwrap().next() else { return };
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let lat = (b.lat_top + (b.lat_bot - b.lat_top) * (y as f64 + 0.5) / h as f64).to_radians();
            let (slat, clat) = lat.sin_cos();
            for (x, c) in line.iter_mut().enumerate() {
                let lon = (b.lon_left + (b.lon_right - b.lon_left) * (x as f64 + 0.5) / w as f64).to_radians();
                *c = sample(src, DVec3::new(clat * lon.cos(), clat * lon.sin(), slat), sig.lakes, voxel_m);
            }
        }
    };
    std::thread::scope(|s| {
        for _ in 1..threads() {
            s.spawn(row);
        }
        row();
    });
    (!cancel.load(Ordering::Relaxed)).then_some(cells)
}

/// The picture of a view. None when cancelled.
pub fn synth(src: &Source, sig: &Sig, cancel: &AtomicBool) -> Option<(ColorImage, f64)> {
    let cells = sample_view(src, sig, cancel)?;
    let (w, h, b) = (sig.w, sig.h, sig.bounds);
    let m = src.m_per_km as f32;
    // Contours: the land's own relief in view sets the spacing.
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for c in cells.iter().filter(|c| !c.sea) {
        let e = (c.elev_km * m) as f64;
        lo = lo.min(e);
        hi = hi.max(e);
    }
    let interval = if sig.base == Base::Elevation { contour_interval_m(hi - lo) } else { 0.0 };
    let band = |e_km: f32| ((e_km * m) as f64 / interval).floor() as i64;
    let (ts, ps, hs) = (temp_stops(), precip_stops(), hypso_stops());
    let palette = Palette { grass: block_colour(src.palette.grass), dry: block_colour(src.palette.dry), sand: block_colour(src.palette.sand), snow: block_colour(src.palette.snow) };
    // Relief: a hillshade from the upper left, by slope in true map scale,
    // steepened as a map is (the bake's slopes are gentle at this size).
    let dlat = ((b.lat_top - b.lat_bot) / h as f64).to_radians() * src.radius_m;
    let dlon = ((b.lon_right - b.lon_left) / w as f64).to_radians() * src.radius_m;
    let exaggerate = 26.0 / sig_zoom(&b).powf(0.25);
    let light = DVec3::new(-0.34, 0.43, 0.835).normalize();
    let mut px = vec![Color32::BLACK; w * h];
    for y in 0..h {
        if y % 64 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        let lat = (b.lat_top + (b.lat_bot - b.lat_top) * (y as f64 + 0.5) / h as f64).to_radians();
        let east_m = (dlon * lat.cos().abs().max(0.04) * 2.0).max(1e-6);
        let north_m = (dlat * 2.0).max(1e-6);
        let (ym, yp) = (y.saturating_sub(1), (y + 1).min(h - 1));
        for x in 0..w {
            let i = y * w + x;
            let c = cells[i];
            let mut colour = match sig.base {
                Base::Biomes if c.sea => sea_colour(c.elev_km),
                Base::Biomes => biome_colour(c.temp, c.precip, &palette),
                Base::Temperature => ramp(&ts, ((c.temp + 30.0) / 65.0).clamp(0.0, 1.0)),
                Base::Precipitation => ramp(&ps, (c.precip / 12.0 / 300.0).clamp(0.0, 1.0).powf(0.7)),
                Base::Elevation if c.sea => sea_colour(c.elev_km),
                Base::Elevation => {
                    let mut t = ramp(&hs, (c.elev_km as f64 / src.peak_km).clamp(0.0, 1.0) as f32);
                    if interval > 0.0 {
                        let here = band(c.elev_km);
                        // A pixel whose band differs from its right or lower
                        // neighbour's carries the line on its higher side.
                        let line = [(x + 1 < w).then(|| i + 1), (y + 1 < h).then(|| i + w)].into_iter().flatten().filter(|&n| !cells[n].sea).map(|n| band(cells[n].elev_km)).filter(|&n| n != here).map(|n| n.max(here)).max();
                        if let Some(l) = line {
                            t = scale(t, if l.rem_euclid(CONTOUR_INDEX) == 0 { 0.42 } else { 0.68 });
                        }
                    }
                    t
                }
            };
            if sig.relief && !c.sea {
                let (xm, xp) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let east = ((cells[y * w + xp].elev_km - cells[y * w + xm].elev_km) * m) as f64 / east_m;
                let north = ((cells[ym * w + x].elev_km - cells[yp * w + x].elev_km) * m) as f64 / north_m;
                let n = DVec3::new(-east * exaggerate, -north * exaggerate, 1.0).normalize();
                let hill = n.dot(light).clamp(0.0, 1.0);
                let shade = ((hill - light.z) * 1.7 + 1.0).clamp(0.45, 1.5) as f32;
                colour = scale(colour, shade);
            }
            colour = match c.lake {
                1 => LAKE,
                2 => LAKE_FROZEN,
                _ => colour,
            };
            let [r, g, bl] = colour.map(|v| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8);
            px[i] = Color32::from_rgb(r, g, bl);
        }
    }
    Some((ColorImage { size: [w, h], source_size: egui::vec2(w as f32, h as f32), pixels: px }, interval))
}

fn sig_zoom(b: &Bounds) -> f64 {
    (360.0 / (b.lon_right - b.lon_left)).max(1.0)
}

// ---------------------------------------------------------------- building

struct Job {
    sig: Sig,
    cancel: Arc<AtomicBool>,
}

/// Builds pictures off the main thread, one at a time: a new request cancels
/// the one running.
pub struct MapBuilder {
    job: Option<Job>,
    tx: mpsc::Sender<(u64, Built)>,
    rx: mpsc::Receiver<(u64, Built)>,
    id: u64,
    /// Worker threads alive, for the tests: a build ends when it is cancelled.
    pub running: Arc<std::sync::atomic::AtomicUsize>,
}

impl Default for MapBuilder {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { job: None, tx, rx, id: 0, running: Default::default() }
    }
}

impl MapBuilder {
    /// The picture being built, if any.
    pub fn building(&self) -> Option<Sig> {
        self.job.as_ref().map(|j| j.sig)
    }

    /// Start on `sig`, dropping whatever is running. With `rough`, a small
    /// picture comes first (the map has nothing to show yet).
    pub fn start(&mut self, src: Arc<Source>, sig: Sig, rough: bool) {
        self.cancel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.job = Some(Job { sig, cancel: cancel.clone() });
        self.id += 1;
        let (id, tx, running) = (self.id, self.tx.clone(), self.running.clone());
        running.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            let build = |sig: Sig, rough: bool| {
                let t0 = std::time::Instant::now();
                if let Some((image, interval_m)) = synth(&src, &sig, &cancel) {
                    let _ = tx.send((id, Built { sig, image, interval_m, ms: t0.elapsed().as_secs_f64() * 1e3, rough }));
                }
            };
            if rough {
                build(Sig { w: sig.w / 4, h: sig.h / 4, ..sig }, true);
            }
            build(sig, false);
            running.fetch_sub(1, Ordering::SeqCst);
        });
    }

    /// Stop the running build (the view moved on, or the window shut).
    pub fn cancel(&mut self) {
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The newest picture finished since the last call. A cancelled build's
    /// pictures never come back.
    pub fn poll(&mut self) -> Option<Built> {
        let mut last = None;
        while let Ok((id, built)) = self.rx.try_recv() {
            if id == self.id && self.job.is_some() {
                if !built.rough {
                    self.job = None;
                }
                last = Some(built);
            }
        }
        last
    }
}

// ---------------------------------------------------------------- rivers

/// One river link as a line on the map.
pub struct River {
    /// log10 of the discharge in m^3/s.
    pub flow: f32,
    /// (lat, lon) degrees, unbroken across the map (a link over the date
    /// line is split there).
    pub points: Vec<[f32; 2]>,
    /// lat min, lat max, lon min, lon max.
    bbox: [f32; 4],
}

/// Every river link, biggest first.
pub fn rivers(hydro: &HydroData) -> Vec<River> {
    let geo = |d: DVec3| {
        let (lat, lon) = crate::grid::Planet::lat_lon_deg(d);
        [lat as f32, lon as f32]
    };
    let mut out = Vec::with_capacity(hydro.segs.len());
    for seg in &hydro.segs {
        let (a, b) = (&hydro.nodes[seg.a as usize], &hydro.nodes[seg.b as usize]);
        let flow = a.flow.max(b.flow).max(1.0).log10() as f32;
        let mut line = vec![geo(a.pos)];
        let mut push = |line: &mut Vec<[f32; 2]>, p: [f32; 2]| {
            if (line[line.len() - 1][1] - p[1]).abs() > 90.0 {
                out.push(River::new(flow, std::mem::take(line)));
            }
            line.push(p);
        };
        for d in &seg.interior {
            push(&mut line, geo(*d));
        }
        push(&mut line, geo(b.pos));
        out.push(River::new(flow, line));
    }
    out.retain(|r| r.points.len() >= 2);
    out.sort_by(|a, b| b.flow.total_cmp(&a.flow));
    out
}

impl River {
    fn new(flow: f32, points: Vec<[f32; 2]>) -> Self {
        let mut bbox = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
        for p in &points {
            bbox = [bbox[0].min(p[0]), bbox[1].max(p[0]), bbox[2].min(p[1]), bbox[3].max(p[1])];
        }
        Self { flow, points, bbox }
    }

    fn visible(&self, b: &Bounds) -> bool {
        self.bbox[1] as f64 >= b.lat_bot && self.bbox[0] as f64 <= b.lat_top && self.bbox[3] as f64 >= b.lon_left && self.bbox[2] as f64 <= b.lon_right
    }
}

/// The smallest river shown at a zoom: the main network at the whole planet,
/// everything by about 4x (the viewer's rule).
pub fn river_gate(zoom: f64) -> f32 {
    (3.8 - 0.85 * zoom.log2()).clamp(2.0, 6.0) as f32
}

/// Draw the rivers in view, clipped to the painter's rectangle.
pub fn paint_rivers(paint: &egui::Painter, rect: egui::Rect, view: &View, rivers: &[River]) {
    let b = view.bounds();
    let gate = river_gate(view.zoom);
    for r in rivers {
        if r.flow < gate {
            break; // biggest first
        }
        if !r.visible(&b) {
            continue;
        }
        let width = (0.5 + (r.flow - 2.5)).clamp(0.6, 3.5);
        let bright = (0.30 + (r.flow - 2.0) * 0.26).clamp(0.4, 1.0);
        let colour = Color32::from_rgb((55.0 * bright) as u8, (118.0 * bright) as u8, (232.0 * bright) as u8);
        let points: Vec<egui::Pos2> = r.points.iter().map(|p| b.project(rect, p[0] as f64, p[1] as f64)).collect();
        paint.add(egui::Shape::line(points, egui::Stroke::new(width, colour)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(800.0, 400.0))
    }

    /// The ground under the cursor stays under it as the map zooms, and the
    /// view never leaves the planet.
    #[test]
    fn zoom_keeps_the_ground_under_the_cursor() {
        let mut v = View::default();
        let cursor = egui::pos2(640.0, 180.0);
        let (lat, lon) = v.bounds().unproject(rect(), cursor);
        for _ in 0..6 {
            v.zoom_at(1.8, rect(), cursor);
            let (la, lo) = v.bounds().unproject(rect(), cursor);
            assert!((la - lat).abs() < 1e-6 && (lo - lon).abs() < 1e-6, "zoom {}: {la} {lo} vs {lat} {lon}", v.zoom);
        }
        // Near the edge of the world the view stops at the edge.
        let mut v = View::default();
        v.zoom_at(3.0, rect(), egui::pos2(101.0, 51.0));
        v.pan(5000.0, 5000.0, rect().size());
        let b = v.bounds();
        assert!(b.lon_left >= -180.0 - 1e-9 && b.lat_top <= 90.0 + 1e-9, "{b:?}");
        // Zoomed all the way out, the whole planet.
        v.zoom_at(0.0001, rect(), egui::pos2(300.0, 300.0));
        assert_eq!(v.bounds(), Bounds::WORLD);
        assert!(MAX_ZOOM >= 180.0);
    }

    /// A moved view finds its window inside the old picture.
    #[test]
    fn a_moved_view_addresses_the_old_picture() {
        let whole = Bounds::WORLD;
        let half = Bounds { lat_top: 45.0, lat_bot: -45.0, lon_left: 0.0, lon_right: 180.0 };
        let uv = half.uv_in(whole);
        assert!((uv.min.x - 0.5).abs() < 1e-6 && (uv.max.x - 1.0).abs() < 1e-6);
        assert!((uv.min.y - 0.25).abs() < 1e-6 && (uv.max.y - 0.75).abs() < 1e-6);
    }

    #[test]
    fn contour_spacing_follows_the_relief() {
        assert_eq!(contour_interval_m(0.0), 0.0);
        assert_eq!(contour_interval_m(f64::NAN), 0.0);
        assert_eq!(contour_interval_m(120.0), 10.0);
        assert_eq!(contour_interval_m(600.0), 50.0);
        assert_eq!(contour_interval_m(240.0), 20.0);
        // Never finer than a quarter metre.
        assert_eq!(contour_interval_m(0.5), 0.2);
    }

    #[test]
    fn the_river_gate_opens_with_zoom() {
        assert!(river_gate(1.0) > river_gate(2.0));
        assert_eq!(river_gate(MAX_ZOOM), 2.0);
    }
}

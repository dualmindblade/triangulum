//! The photo map: an egui overlay opened with T. Every screenshot in
//! `interchange/` becomes a marker on a minimap of the planet (synthesized
//! from the baked rasters — no extra data files); markers and the photo
//! list select each other, a preview shows the shot, and "Teleport" commits
//! to the photo's exact pose — optionally restoring its time of day from
//! the JSON sidecar. Photos can be deleted (bulk, with confirmation); they
//! move to `interchange/trash/`, never straight to oblivion.
//!
//! egui paints through a small custom wgpu backend (`EguiPaint`) because
//! egui-wgpu pins a different wgpu major than the renderer; the paint side
//! of egui is just textured triangles and one texture manager, so we own
//! those ~200 lines and keep the renderer's wgpu.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::epaint::{ImageDelta, Primitive};
use egui::{ClippedPrimitive, Color32, TextureId, TexturesDelta};
use glam::DVec3;

use crate::planet::{Planet, face_from_dir, ground_tint, sea_from_fields};

/// Deepest zoom the pan/zoom view allows (1 = whole planet). At 180x the map
/// frames ~2 deg of longitude (~300 km on Neisor) across the widget — fine
/// enough to trace a creek; the base rasters are ~10 km/texel near a face
/// center, so terrain reads smooth-but-soft there while the vector rivers and
/// lakes (exact geometry) stay razor sharp.
const MAX_ZOOM: f64 = 180.0;

/// The photo map intentionally offers only landable/mappable bodies.  The Sun
/// remains a physical focus target, not a teleport-map surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapBody {
    Neisor,
    Moon,
}

// ------------------------------------------------------------- photo index

/// One screenshot with a known pose. `day_time_s` comes from the sidecar
/// (present on every shot taken after 2026-07-08); older filename-only
/// shots still get position/view from the name.
pub struct Photo {
    pub path: PathBuf,
    pub name: String,
    pub body: MapBody,
    pub lat: f64,
    pub lon: f64,
    pub alt_km: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
    pub day_time_s: Option<f64>,
    /// Recorded weather restore coordinate. `weather_on == None` marks a
    /// legacy/pre-weather sidecar; `Some(true)` plus no pin means live.
    pub weather_on: Option<bool>,
    pub weather_pin: Option<(f32, f32)>,
    pub weather_time_s: Option<f64>,
    // exact-restore state (newer sidecars only; older photos fall back to
    // the generic fly-mode teleport): the photographed ground height, mode,
    // day length, and seed let restore reproduce the shot instead of
    // hovering 2.5 m over the far terrain surface in fly mode with the sun
    // at the wrong phase
    pub ground_km: Option<f64>,
    pub mode: Option<String>,
    pub day_len_s: Option<f64>,
    pub seed: Option<i64>,
    /// Player-authored, editable in the photo window and stored back in the
    /// sidecar. Absent on legacy sidecars (and never present on filename-only
    /// shots) — read as empty strings, never an error.
    pub title: String,
    pub note: String,
}

fn sidecar_weather(js: &serde_json::Value) -> Option<(bool, Option<(f32, f32)>, Option<f64>)> {
    let weather = js.get("weather")?.as_object()?;
    let on = weather.get("on")?.as_bool()?;
    let pin = match weather.get("pinned") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => {
            let p = v.as_array()?;
            if p.len() != 2 {
                return None;
            }
            let (c, r) = (p[0].as_f64()?, p[1].as_f64()?);
            if !c.is_finite()
                || !r.is_finite()
                || !(0.0..=1.0).contains(&c)
                || !(0.0..=1.0).contains(&r)
            {
                return None;
            }
            Some((c as f32, r as f32))
        }
    };
    let time = match weather.get("t_s") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => {
            let t_s = v.as_f64()?;
            if !t_s.is_finite() || t_s < 0.0 {
                return None;
            }
            Some(t_s)
        }
    };
    Some((on, pin, time))
}

/// A free-text sidecar field (title / note): missing or non-string reads as
/// empty so pre-notes sidecars and filename-only shots keep working.
fn sidecar_str(js: &serde_json::Value, key: &str) -> String {
    js.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Set `title`/`note` on a sidecar value, preserving every other field
/// (read-modify-write). A non-object value (or an absent sidecar the caller
/// passes as `{}`) becomes a fresh object holding just these two keys.
fn apply_notes(js: &mut serde_json::Value, title: &str, note: &str) {
    if !js.is_object() {
        *js = serde_json::Value::Object(serde_json::Map::new());
    }
    if let Some(obj) = js.as_object_mut() {
        obj.insert(
            "title".to_string(),
            serde_json::Value::String(title.to_string()),
        );
        obj.insert(
            "note".to_string(),
            serde_json::Value::String(note.to_string()),
        );
    }
}

fn parse_filename(name: &str) -> Option<(f64, f64, f64, f64, f64)> {
    // shot_lat4.990_lon-29.403_alt0.047km_yaw37_pitch-29.png
    let grab = |key: &str, until: &str| -> Option<f64> {
        let s = name.split(key).nth(1)?;
        let s = s.split(until).next()?;
        s.parse().ok()
    };
    Some((
        grab("_lat", "_lon")?,
        grab("_lon", "_alt")?,
        grab("_alt", "km")?,
        grab("_yaw", "_pitch")?,
        grab("_pitch", ".png")?,
    ))
}

/// Scan the interchange dir (top level only — harness run output lives in
/// subdirectories and is not the player's photo roll).
pub fn scan_photos(interchange: &Path) -> Vec<Photo> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(interchange) else {
        return out;
    };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("shot_") || !name.ends_with(".png") {
            continue;
        }
        let sidecar = path.with_extension("json");
        // parse the sidecar once: pose may come from it (below) or from the
        // filename, but title/note always come from it when present.
        let js = std::fs::read_to_string(&sidecar)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
        let mut photo: Option<Photo> = None;
        if let Some(js) = &js {
            let f = |k: &str| js.get(k).and_then(|v| v.as_f64());
            let body = match js.get("body").and_then(|v| v.as_str()) {
                Some(s) if s.eq_ignore_ascii_case("moon") => MapBody::Moon,
                _ if f("focus_id") == Some(1.0) && f("body_lat_deg").is_some() => MapBody::Moon,
                _ => MapBody::Neisor,
            };
            let (lat, lon, alt) = match body {
                MapBody::Neisor => (f("lat_deg"), f("lon_deg"), f("alt_km")),
                MapBody::Moon => (
                    f("body_lat_deg").or_else(|| f("lat_deg")),
                    f("body_lon_deg").or_else(|| f("lon_deg")),
                    f("body_alt_km").or_else(|| f("alt_km")),
                ),
            };
            if let (Some(lat), Some(lon)) = (lat, lon) {
                let weather = sidecar_weather(js);
                let (yaw_deg, pitch_deg, roll_deg) = match body {
                    MapBody::Neisor => (f("yaw_deg"), f("pitch_deg"), f("roll_deg")),
                    MapBody::Moon => (
                        f("body_yaw_deg").or_else(|| f("yaw_deg")),
                        f("body_pitch_deg").or_else(|| f("pitch_deg")),
                        f("body_roll_deg").or_else(|| f("roll_deg")),
                    ),
                };
                photo = Some(Photo {
                    path: path.clone(),
                    name: name.clone(),
                    body,
                    lat,
                    lon,
                    alt_km: alt.unwrap_or(if body == MapBody::Moon { 25.0 } else { 0.3 }),
                    yaw_deg: yaw_deg.unwrap_or(0.0),
                    pitch_deg: pitch_deg.unwrap_or(-20.0),
                    roll_deg: roll_deg.unwrap_or(0.0),
                    day_time_s: f("day_cycle_time_s"),
                    weather_on: weather.map(|w| w.0),
                    weather_pin: weather.and_then(|w| w.1),
                    weather_time_s: weather.and_then(|w| w.2),
                    ground_km: f("ground_km"),
                    mode: js.get("mode").and_then(|v| v.as_str()).map(str::to_owned),
                    day_len_s: f("day_len_s").filter(|v| *v > 0.0),
                    seed: js.get("seed").and_then(|v| v.as_i64()),
                    title: String::new(),
                    note: String::new(),
                });
            }
        }
        if photo.is_none()
            && let Some((lat, lon, alt, yaw, pitch)) = parse_filename(&name)
        {
            photo = Some(Photo {
                path: path.clone(),
                name: name.clone(),
                body: MapBody::Neisor,
                lat,
                lon,
                alt_km: alt,
                yaw_deg: yaw,
                pitch_deg: pitch,
                roll_deg: 0.0,
                day_time_s: None,
                weather_on: None,
                weather_pin: None,
                weather_time_s: None,
                ground_km: None,
                mode: None,
                day_len_s: None,
                seed: None,
                title: String::new(),
                note: String::new(),
            });
        }
        // title/note ride the sidecar regardless of where the pose came from,
        // so a note saved onto an otherwise filename-only shot round-trips.
        if let (Some(p), Some(js)) = (photo.as_mut(), &js) {
            p.title = sidecar_str(js, "title");
            p.note = sidecar_str(js, "note");
        }
        if let Some(p) = photo {
            out.push(p);
        }
    }
    // newest first: the shot you just took is the one you want
    out.sort_by(|a, b| {
        let m = |p: &Photo| std::fs::metadata(&p.path).and_then(|m| m.modified()).ok();
        m(b).cmp(&m(a))
    });
    out
}

// ---------------------------------------------------------------- minimap
//
// The minimap is an equirectangular window on the planet, RE-SYNTHESIZED from
// the baked rasters (and the seasonal weather field) for whatever lat/lon
// rectangle the pan/zoom view currently frames — so zooming in shows real map
// detail, not upscaled pixels. Rivers and lakes are drawn as vector overlays
// straight from rivers.bin geometry (see `PhotoMap::ui`), staying crisp at any
// zoom; only the smooth color fields (biome / temperature / precipitation /
// cloud) are rasterized into the texture here.

/// Which full-coverage color field paints the base of the map.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BaseLayer {
    Biomes,
    Temperature,
    Precipitation,
    /// Hypsometric tint + contour isolines (Neisor only): terrain structure
    /// for locating alpine ranges, river valleys, and cliff bands.
    Elevation,
}

/// Everything the map needs from the app besides the photo roll: the planet
/// rasters + rivers, the loaded weather climatology and its tuning, the
/// current weather time (so temp/precip sample the CURRENT season and clouds
/// show the CURRENT synoptic field), and the player's position for the "you
/// are here" marker. All borrowed — the map owns none of it.
pub struct MapEnv<'a> {
    pub planet: &'a Planet,
    pub weather_field: Option<&'a crate::weather::WeatherField>,
    /// The exact quantized bytes bound to the cloud-deck shader. Clouds-now
    /// samples this instead of independently re-evaluating weather_at.
    pub synoptic_raster: Option<&'a crate::weather::SynopticRaster>,
    pub weather_tuning: &'a crate::weather::WeatherTuning,
    pub solar_tuning: &'a crate::orbits::SolarTuning,
    pub weather_time_s: f64,
    pub day_len_s: f64,
    pub weather_on: bool,
    pub weather_pin: Option<(f32, f32)>,
    pub cur_lat: f64,
    pub cur_lon: f64,
    pub cur_moon_lat: f64,
    pub cur_moon_lon: f64,
    pub cur_body: MapBody,
    pub time_scale: f64,
    pub multiplayer_available: bool,
    pub multiplayer_connected: bool,
    pub multiplayer_status: &'a str,
}

/// A lat/lon rectangle (degrees) the view frames. `lat_top` is the larger
/// latitude (north-up); longitude increases left→right.
#[derive(Clone, Copy, PartialEq)]
struct Bounds {
    lat_top: f64,
    lat_bot: f64,
    lon_left: f64,
    lon_right: f64,
}

impl Bounds {
    fn project(&self, rect: egui::Rect, lat: f64, lon: f64) -> egui::Pos2 {
        egui::pos2(
            rect.left()
                + ((lon - self.lon_left) / (self.lon_right - self.lon_left)) as f32 * rect.width(),
            rect.top()
                + ((self.lat_top - lat) / (self.lat_top - self.lat_bot)) as f32 * rect.height(),
        )
    }
    /// Screen point -> (lat, lon) degrees.
    fn unproject(&self, rect: egui::Rect, p: egui::Pos2) -> (f64, f64) {
        let fx = ((p.x - rect.left()) / rect.width()) as f64;
        let fy = ((p.y - rect.top()) / rect.height()) as f64;
        (
            self.lat_top + fy * (self.lat_bot - self.lat_top),
            self.lon_left + fx * (self.lon_right - self.lon_left),
        )
    }
    /// UV rect addressing THIS (live) window inside a texture synthesized for
    /// `synth` — a pan/zoom translates/scales the existing pixels for one
    /// frame (slippy-map feel) until the crisp re-synth for the new view lands.
    fn uv_in(&self, synth: Bounds) -> egui::Rect {
        let sx = |lon: f64| ((lon - synth.lon_left) / (synth.lon_right - synth.lon_left)) as f32;
        let sy = |lat: f64| ((synth.lat_top - lat) / (synth.lat_top - synth.lat_bot)) as f32;
        egui::Rect::from_min_max(
            egui::pos2(sx(self.lon_left), sy(self.lat_top)),
            egui::pos2(sx(self.lon_right), sy(self.lat_bot)),
        )
    }
}

/// The (resolution, layers, region) a synthesized base texture is valid for.
/// f64 equality is exact here — the view only moves on deterministic user
/// actions, never NaN.
#[derive(Clone, PartialEq)]
struct MapSig {
    w: usize,
    h: usize,
    body: MapBody,
    base: BaseLayer,
    relief: bool,
    clouds: bool,
    weather_on: bool,
    weather_pin: Option<(f32, f32)>,
    /// Live clouds use the renderer raster's two-second key; seasonal-only
    /// bases retain their cheaper 60-second refresh cadence.
    weather_time_bucket: Option<i64>,
    bounds: Bounds,
}

const MAP_SEASON_BUCKET_S: f64 = 60.0;

fn map_weather_time_bucket(
    base: BaseLayer,
    clouds: bool,
    weather_on: bool,
    weather_pin: Option<(f32, f32)>,
    weather_time_s: f64,
) -> Option<i64> {
    let moving_clouds = clouds && weather_on && weather_pin.is_none();
    // Biomes and Elevation are season-independent bases; only the seasonal
    // climate fields (and moving clouds) warrant periodic re-synthesis.
    let seasonal_base = matches!(base, BaseLayer::Temperature | BaseLayer::Precipitation);
    let time_sensitive = moving_clouds || seasonal_base;
    time_sensitive.then(|| {
        let t_s = if weather_time_s.is_finite() {
            weather_time_s
        } else {
            0.0
        };
        let bucket_s = if moving_clouds {
            crate::weather::SYNOPTIC_RASTER_INTERVAL_S
        } else {
            MAP_SEASON_BUCKET_S
        };
        (t_s / bucket_s).floor() as i64
    })
}

fn dir_to_geo(d: DVec3) -> (f64, f64) {
    (d.z.asin().to_degrees(), d.y.atan2(d.x).to_degrees())
}

/// Author a color in display sRGB; the map buffer works in the linear-ish
/// space `ground_tint` uses (encoded to sRGB once at the end), so convert.
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
            return [
                a.1[0] + (b.1[0] - a.1[0]) * t,
                a.1[1] + (b.1[1] - a.1[1]) * t,
                a.1[2] + (b.1[2] - a.1[2]) * t,
            ];
        }
    }
    stops[stops.len() - 1].1
}

// The ramp stops carry `disp` (a per-channel powf) — built ONCE per synth,
// not per pixel, or the temperature/precip bases spend their whole budget on
// pow. `synth_map` hoists these out of the pixel loop.

/// Cold blue → hot red; pale temperate mid.
fn temp_stops() -> [(f32, [f32; 3]); 5] {
    [
        (0.00, disp(0.13, 0.20, 0.62)),
        (0.30, disp(0.25, 0.55, 0.85)),
        (0.50, disp(0.80, 0.84, 0.58)),
        (0.72, disp(0.90, 0.58, 0.24)),
        (1.00, disp(0.72, 0.10, 0.08)),
    ]
}

/// Dry tan → wet blue-green.
fn precip_stops() -> [(f32, [f32; 3]); 4] {
    [
        (0.00, disp(0.78, 0.70, 0.47)),
        (0.32, disp(0.72, 0.72, 0.42)),
        (0.60, disp(0.34, 0.62, 0.45)),
        (1.00, disp(0.08, 0.45, 0.55)),
    ]
}

/// Span roughly -30..+35 C onto the cold→hot ramp.
fn temp_color(t_c: f64, stops: &[(f32, [f32; 3])]) -> [f32; 3] {
    let x = (((t_c + 30.0) / 65.0) as f32).clamp(0.0, 1.0);
    ramp(stops, x)
}

/// Precip is heavily skewed, so compress with a power before the ramp
/// (mm/month; the climatology's precip is per-month).
fn precip_color(mm: f64, stops: &[(f32, [f32; 3])]) -> [f32; 3] {
    let x = ((mm.max(0.0) / 300.0) as f32).clamp(0.0, 1.0).powf(0.7);
    ramp(stops, x)
}

/// Sea base color by depth: deep navy to shelf teal. One bathymetry ramp is
/// shared by the biome and elevation bases so switching layers never recolors
/// the ocean.
fn sea_depth_color(e_km: f64) -> [f32; 3] {
    let d = (-e_km / 4.0).clamp(0.0, 1.0) as f32;
    [
        0.10 + (0.02 - 0.10) * d,
        0.32 + (0.08 - 0.32) * d,
        0.42 + (0.22 - 0.42) * d,
    ]
}

/// Biome base color at a cell — the legacy koppen tint (ocean by depth, land
/// by class), optionally shaded by elevation + snow (the relief layer). With
/// `relief` on this reproduces the original `build_minimap` land/sea look.
fn biome_color(planet: &Planet, f: usize, u: f64, v: f64, relief: bool) -> [f32; 3] {
    let e = planet.elevation(f, u, v) as f64;
    let climate = planet.biome_climate(f, u, v);
    let k = climate.koppen;
    if climate.sea {
        sea_depth_color(e)
    } else {
        let g = ground_tint(k);
        if relief {
            let t = climate.temp_c;
            let l = (e / 4.5).clamp(0.0, 1.0) as f32;
            let snow = if t < -9.0 { 0.75f32 } else { 0.0 };
            let m = l.max(snow);
            [
                g[0] + (0.93 - g[0]) * m,
                g[1] + (0.90 - g[1]) * m,
                g[2] + (0.88 - g[2]) * m,
            ]
        } else {
            g
        }
    }
}

// ------------------------------------------------------- elevation base layer
//
// Hypsometric tint + per-pixel contour isolines, for reading terrain structure
// (alpine ranges, river valleys, cliff bands) straight off the teleport map.
// Everything below is a pure function of the baked rasters and the view
// bounds — no wall clock, no randomness — so the layer is deterministic and
// signature-cacheable like every other base.

/// Aim for about this many minor contour bands across the visible land relief.
const CONTOUR_TARGET_BANDS: f64 = 12.0;
/// Every Nth isoline is an index contour with a stronger stroke.
const CONTOUR_INDEX_EVERY: i64 = 4;

/// Minor contour interval (km) for a visible land elevation range: roughly
/// `CONTOUR_TARGET_BANDS` bands across the range, snapped DOWN onto a 1-2-5
/// ladder so panning at one zoom rarely churns the interval. Because the
/// range shrinks as the view zooms into flatter country, the interval
/// tightens with zoom automatically (≈4.5 km of relief in view → 500 m
/// lines; a lowland valley with 300 m of relief → 20 m lines). Returns 0.0
/// when there is nothing to contour (no land in view, flat, or non-finite).
fn contour_interval_km(land_range_km: f64) -> f64 {
    if !land_range_km.is_finite() || land_range_km <= 1e-6 {
        return 0.0;
    }
    // floor at 2 m: the base rasters are ~10 km/texel, so finer lines would
    // only trace bilinear-interpolation artifacts.
    let raw = (land_range_km / CONTOUR_TARGET_BANDS).max(0.002);
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

/// Lowland green → tan → brown → rock grey → white peaks, indexed by
/// elevation normalized to the planet's tallest terrain
/// (`ElevationField::hypso_max_km`). Built once per synth (see the note on
/// `disp` above `temp_stops`).
fn hypso_stops() -> [(f32, [f32; 3]); 6] {
    [
        (0.00, disp(0.33, 0.51, 0.29)),
        (0.16, disp(0.56, 0.61, 0.33)),
        (0.36, disp(0.79, 0.69, 0.44)),
        (0.58, disp(0.62, 0.44, 0.28)),
        (0.80, disp(0.55, 0.48, 0.44)),
        (1.00, disp(0.96, 0.96, 0.97)),
    ]
}

/// The elevation base's sampled window: per-pixel elevation, the physical
/// land/sea split, and the contour interval chosen from the land relief that
/// is actually in view.
struct ElevationField {
    w: usize,
    h: usize,
    elev_km: Vec<f32>,
    sea: Vec<bool>,
    /// Minor isoline spacing; 0.0 disables contours (all-sea or flat view).
    interval_km: f64,
    /// Where the land tint tops out (white): the PLANET's tallest raster
    /// texel, not the view's — so a mountain stays the same color at every
    /// pan/zoom and white always means "the highest terrain this seed has".
    hypso_max_km: f64,
}

impl ElevationField {
    fn sample(planet: &Planet, b: Bounds, w: usize, h: usize) -> Self {
        // One sequential pass over the six elevation rasters (~a few ms,
        // small next to the per-pixel bilinears below). The maximum texel is
        // always land — the sea floor is negative — and bilinear samples
        // can never exceed it.
        let hypso_max_km = planet
            .faces
            .iter()
            .flat_map(|f| f.elev_km.iter())
            .fold(f32::MIN, |a, &e| a.max(e))
            .max(1.0) as f64;
        let mut elev_km = vec![0.0f32; w * h];
        let mut sea = vec![false; w * h];
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for y in 0..h {
            let lat =
                (b.lat_top + (b.lat_bot - b.lat_top) * (y as f64 + 0.5) / h as f64).to_radians();
            let (slat, clat) = lat.sin_cos();
            for x in 0..w {
                let lon = (b.lon_left + (b.lon_right - b.lon_left) * (x as f64 + 0.5) / w as f64)
                    .to_radians();
                let (f, u, v) = face_from_dir(DVec3::new(clat * lon.cos(), clat * lon.sin(), slat));
                let e = planet.elevation(f, u, v);
                // Mirrors Planet::true_sea_at: positive elevation proves land
                // with one raster read; only shoreline/basin pixels consult
                // the ocean masks. Dry below-sea-level basins stay land.
                let is_sea = e <= 0.0
                    && sea_from_fields(
                        e as f64,
                        planet.water_frac(f, u, v) as f64,
                        planet.ocean(f, u, v) as f64,
                    );
                elev_km[y * w + x] = e;
                sea[y * w + x] = is_sea;
                if !is_sea {
                    lo = lo.min(e as f64);
                    hi = hi.max(e as f64);
                }
            }
        }
        let interval_km = contour_interval_km(hi - lo);
        Self {
            w,
            h,
            elev_km,
            sea,
            interval_km,
            hypso_max_km,
        }
    }

    /// Quantized contour band at a pixel index.
    fn level(&self, i: usize) -> i64 {
        (self.elev_km[i] as f64 / self.interval_km).floor() as i64
    }

    /// Base tint + contour stroke at a pixel, in the map's linear space.
    /// A pixel is on a contour when its band differs from the +x or +y
    /// neighbor's, so every isoline crossing gets a one-pixel stroke on its
    /// higher side; index contours (every `CONTOUR_INDEX_EVERY`th line)
    /// stroke darker. Sea pixels keep the shared bathymetry ramp untouched.
    fn color(&self, x: usize, y: usize, stops: &[(f32, [f32; 3])]) -> [f32; 3] {
        let i = y * self.w + x;
        let e = self.elev_km[i] as f64;
        if self.sea[i] {
            return sea_depth_color(e);
        }
        let mut c = ramp(stops, (e / self.hypso_max_km).clamp(0.0, 1.0) as f32);
        if self.interval_km > 0.0 {
            let lv = self.level(i);
            let mut crossing: Option<i64> = None;
            let neighbors = [
                (x + 1 < self.w).then(|| i + 1),
                (y + 1 < self.h).then(|| i + self.w),
            ];
            for ni in neighbors.into_iter().flatten() {
                if self.sea[ni] {
                    continue; // the coastline is already a color break
                }
                let nl = self.level(ni);
                if nl != lv {
                    let line = lv.max(nl);
                    crossing = Some(crossing.map_or(line, |best: i64| best.max(line)));
                }
            }
            if let Some(line) = crossing {
                let k = if line.rem_euclid(CONTOUR_INDEX_EVERY) == 0 {
                    0.42 // index contour: strong stroke
                } else {
                    0.68 // minor contour: subtle stroke
                };
                c = [c[0] * k, c[1] * k, c[2] * k];
            }
        }
        c
    }
}

/// Encode one linear-space map color to display sRGB (the inverse of `disp`).
fn srgb8(c: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (c[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
    )
}

/// Rasterize the elevation base alone (no clouds). `synth_map` delegates its
/// per-pixel colors here via `ElevationField::color`, so this headless twin
/// cannot drift from what the popup shows.
fn synth_elevation_map(planet: &Planet, b: Bounds, w: usize, h: usize) -> egui::ColorImage {
    let field = ElevationField::sample(planet, b, w, h);
    let stops = hypso_stops();
    let mut px = vec![Color32::BLACK; w * h];
    for y in 0..h {
        for x in 0..w {
            px[y * w + x] = srgb8(field.color(x, y, &stops));
        }
    }
    egui::ColorImage {
        size: [w, h],
        source_size: egui::Vec2::new(w as f32, h as f32),
        pixels: px,
    }
}

/// Headless evidence/export entry point for the teleport map's Elevation
/// base: hypsometric tint + contours for a lat/lon window, like
/// `weather_map_image` frames its window.
pub fn elevation_map_image(
    planet: &Planet,
    center_lat: f64,
    center_lon: f64,
    zoom: f64,
    width: usize,
    height: usize,
) -> egui::ColorImage {
    let zoom = zoom.clamp(1.0, MAX_ZOOM);
    let half_lon = 180.0 / zoom;
    let half_lat = 90.0 / zoom;
    let center_lon = center_lon.clamp(-180.0 + half_lon, 180.0 - half_lon);
    let center_lat = center_lat.clamp(-90.0 + half_lat, 90.0 - half_lat);
    let bounds = Bounds {
        lat_top: center_lat + half_lat,
        lat_bot: center_lat - half_lat,
        lon_left: center_lon - half_lon,
        lon_right: center_lon + half_lon,
    };
    synth_elevation_map(planet, bounds, width.max(64), height.max(32))
}

/// Rasterize the base color field for `b` at `w`x`h`. Rivers/lakes/markers are
/// NOT drawn here (they are vector overlays); this is the biome / temperature
/// / precipitation / elevation field, optionally relief-shaded, with the live
/// cloud field alpha-composited on top. Clouds sample the renderer's tiny
/// shared raster, so map synthesis no longer performs synoptic fbm once per
/// map pixel.
fn synth_map(
    env: &MapEnv,
    base: BaseLayer,
    relief: bool,
    clouds: bool,
    b: Bounds,
    w: usize,
    h: usize,
) -> egui::ColorImage {
    let planet = env.planet;
    let season = crate::weather::season_frac(env.weather_time_s, env.day_len_s, env.solar_tuning);
    let angle = std::f64::consts::TAU * season;
    let (sn1, cs1) = angle.sin_cos();
    let (sn2, cs2) = (2.0 * angle).sin_cos();
    // Legacy/no-frame fallback only: normal map synthesis reads the exact
    // renderer raster below. Resolve systems once if a caller has not drawn
    // a frame yet and therefore supplies no current raster.
    let fallback_cyclones = (clouds && env.synoptic_raster.is_none())
        .then(|| {
            env.weather_field.map(|wf| {
                crate::weather::cyclone_systems(
                    wf,
                    planet.seed,
                    planet.radius_km,
                    season,
                    env.weather_time_s,
                    env.weather_tuning,
                )
            })
        })
        .flatten();
    let (tstops, pstops) = (temp_stops(), precip_stops());
    // The elevation base needs neighbor comparisons for its contour strokes,
    // so it samples its window up front; the loop below only reads buffers.
    let elevation_field =
        (base == BaseLayer::Elevation).then(|| ElevationField::sample(planet, b, w, h));
    let hstops = hypso_stops();
    let mut px = vec![Color32::BLACK; w * h];
    for y in 0..h {
        let lat = (b.lat_top + (b.lat_bot - b.lat_top) * (y as f64 + 0.5) / h as f64).to_radians();
        let (slat, clat) = (lat.sin(), lat.cos());
        for x in 0..w {
            let lon = (b.lon_left + (b.lon_right - b.lon_left) * (x as f64 + 0.5) / w as f64)
                .to_radians();
            let dir = DVec3::new(clat * lon.cos(), clat * lon.sin(), slat);
            let (f, u, v) = face_from_dir(dir);
            let mut c = match base {
                BaseLayer::Biomes => biome_color(planet, f, u, v, relief),
                BaseLayer::Temperature => {
                    let t = match env.weather_field {
                        Some(wf) => wf.climate_sample(planet, f, u, v, cs1, sn1, cs2, sn2).0,
                        None => planet.temp(f, u, v) as f64,
                    };
                    temp_color(t, &tstops)
                }
                BaseLayer::Precipitation => {
                    let p = match env.weather_field {
                        Some(wf) => wf.climate_sample(planet, f, u, v, cs1, sn1, cs2, sn2).1,
                        None => planet.precip(f, u, v) as f64 / 12.0,
                    };
                    precip_color(p, &pstops)
                }
                BaseLayer::Elevation => elevation_field
                    .as_ref()
                    .expect("elevation field sampled above for this base")
                    .color(x, y, &hstops),
            };
            // relief on the weather bases: a gentle hypsometric brightening of
            // land only (biome relief is folded into biome_color above, and
            // the elevation base IS relief — double-shading would smear its
            // contours).
            if relief
                && matches!(base, BaseLayer::Temperature | BaseLayer::Precipitation)
                && planet.water_frac(f, u, v) < 0.5
            {
                let e = planet.elevation(f, u, v) as f64;
                let l = (e / 6.0).clamp(-0.25, 0.85) as f32;
                let m = 0.82 + 0.32 * l;
                c = [c[0] * m, c[1] * m, c[2] * m];
            }
            // clouds NOW: the live synoptic field, white wisps grading to grey
            // storm as cover rises, alpha-composited over the base.
            if clouds
                && env.weather_on
                && let Some(wf) = env.weather_field
            {
                let cover = if let Some((cover, _)) = env.weather_pin {
                    // Exact scalar pins mirror the shader's compatibility
                    // path; the uploaded raster remains spatially uniform.
                    cover
                } else if let Some(raster) = env.synoptic_raster.filter(|raster| {
                    matches!(
                        raster.source(),
                        crate::weather::SynopticRasterSource::Live { seed, .. }
                            if seed == planet.seed
                    )
                }) {
                    raster.sample(dir).0 as f32
                } else {
                    crate::weather::weather_at_with_cyclones(
                        wf,
                        planet,
                        dir,
                        env.weather_time_s,
                        env.day_len_s,
                        env.solar_tuning,
                        env.weather_tuning,
                        fallback_cyclones
                            .as_ref()
                            .expect("cloud fallback resolved cyclone bank"),
                    )
                    .cloud_cover as f32
                };
                if cover > 0.01 {
                    let a = if env.weather_pin.is_some() {
                        cover.powf(1.1) * 0.9
                    } else {
                        // Match the deck's presence gates: weak synoptic
                        // cover should leave a recognizable clear lane on
                        // the map, while storm regions remain emphatic.
                        let x = ((cover - 0.16) / 0.78).clamp(0.0, 1.0);
                        x * x * (3.0 - 2.0 * x) * 0.9
                    };
                    let cl = disp(
                        0.95 - 0.42 * cover,
                        0.95 - 0.40 * cover,
                        0.97 - 0.36 * cover,
                    );
                    c = [
                        c[0] * (1.0 - a) + cl[0] * a,
                        c[1] * (1.0 - a) + cl[1] * a,
                        c[2] * (1.0 - a) + cl[2] * a,
                    ];
                }
            }
            px[y * w + x] = srgb8(c);
        }
    }
    egui::ColorImage {
        size: [w, h],
        source_size: egui::Vec2::new(w as f32, h as f32),
        pixels: px,
    }
}

#[derive(Clone, Debug)]
struct WindStroke {
    points: Vec<DVec3>,
    mean_speed_mps: f64,
}

fn wind_hash01(seed: i64, x: usize, y: usize, lane: u64) -> f64 {
    let mut z = seed as u64
        ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)
        ^ lane.wrapping_mul(0x94D0_49BB_1331_11EB);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

/// Integrate deterministic short paths through the map's instantaneous
/// tangent field. Seeds are view-local (so both the whole globe and a zoomed
/// weather window retain useful density); every path sample is still a pure
/// function of seed, absolute weather time, and direction.
fn wind_streamlines(env: &MapEnv, bounds: Bounds, zoom: f64) -> Vec<WindStroke> {
    let Some(field) = env.weather_field else {
        return Vec::new();
    };
    let season = crate::weather::season_frac(env.weather_time_s, env.day_len_s, env.solar_tuning);
    let cyclones = if env.weather_on {
        crate::weather::cyclone_systems(
            field,
            env.planet.seed,
            env.planet.radius_km,
            season,
            env.weather_time_s,
            env.weather_tuning,
        )
    } else {
        crate::weather::CycloneSystems::default()
    };
    let target = env.weather_tuning.wind_map_density as usize;
    let aspect = ((bounds.lon_right - bounds.lon_left)
        / (bounds.lat_top - bounds.lat_bot).max(1e-6))
    .clamp(0.5, 4.0);
    let cols = ((target as f64 * aspect).sqrt().round() as usize).max(1);
    let rows = target.div_ceil(cols).max(1);
    let path_km = env.weather_tuning.wind_map_length_km / zoom.max(1.0).sqrt();
    const STEPS: usize = 14;
    let base_step = path_km / STEPS as f64 / env.planet.radius_km.max(1e-6);
    let mut strokes = Vec::with_capacity(target);
    for y in 0..rows {
        for x in 0..cols {
            if strokes.len() >= target {
                break;
            }
            let jx = wind_hash01(env.planet.seed, x, y, 1) - 0.5;
            let jy = wind_hash01(env.planet.seed, x, y, 2) - 0.5;
            let fx = (x as f64 + 0.5 + jx * 0.64) / cols as f64;
            let fy = (y as f64 + 0.5 + jy * 0.64) / rows as f64;
            let lon = bounds.lon_left + (bounds.lon_right - bounds.lon_left) * fx;
            let lat = (bounds.lat_top + (bounds.lat_bot - bounds.lat_top) * fy).clamp(-89.5, 89.5);
            let (slat, clat) = lat.to_radians().sin_cos();
            let lon_r = lon.to_radians();
            let mut dir = DVec3::new(clat * lon_r.cos(), clat * lon_r.sin(), slat);
            let mut points = Vec::with_capacity(STEPS + 1);
            let mut speed_sum = 0.0;
            points.push(dir);
            for _ in 0..STEPS {
                let wind = crate::weather::synoptic_wind_tangent_with_cyclones(
                    field,
                    dir,
                    env.planet.radius_km,
                    env.weather_tuning,
                    &cyclones,
                );
                let speed = wind.length();
                if !speed.is_finite() || speed < 0.05 {
                    break;
                }
                speed_sum += speed;
                let tangent = wind / speed;
                // Calm paths stay short; ordinary 10-20 m/s flow consumes
                // the tuned length; storm cores can extend modestly beyond.
                let strength = (0.38 + 0.62 * (speed / 18.0).clamp(0.0, 1.35)).min(1.22);
                let theta = base_step * strength;
                dir = (dir * theta.cos() + tangent * theta.sin()).normalize();
                points.push(dir);
            }
            if points.len() >= 3 {
                strokes.push(WindStroke {
                    mean_speed_mps: speed_sum / (points.len() - 1) as f64,
                    points,
                });
            }
        }
    }
    strokes
}

fn paint_wind_streamlines(
    paint: &egui::Painter,
    rect: egui::Rect,
    bounds: Bounds,
    strokes: &[WindStroke],
) {
    for stroke in strokes {
        let n = stroke.points.len().saturating_sub(1).max(1);
        let energy = (stroke.mean_speed_mps / 24.0).clamp(0.0, 1.0) as f32;
        for (index, pair) in stroke.points.windows(2).enumerate() {
            let (lat0, lon0) = dir_to_geo(pair[0]);
            let (lat1, lon1) = dir_to_geo(pair[1]);
            if (lon0 - lon1).abs() > 90.0 {
                continue;
            }
            let a = bounds.project(rect, lat0, lon0);
            let b = bounds.project(rect, lat1, lon1);
            let f = (index + 1) as f32 / n as f32;
            let alpha = (38.0 + 190.0 * f.powf(1.35)) as u8;
            let color = Color32::from_rgba_unmultiplied(
                (105.0 + 70.0 * energy) as u8,
                (195.0 + 45.0 * energy) as u8,
                255,
                alpha,
            );
            paint.line_segment([a, b], egui::Stroke::new(0.65 + 1.35 * f, color));
        }
        if let Some(&head) = stroke.points.last() {
            let (lat, lon) = dir_to_geo(head);
            let p = bounds.project(rect, lat, lon);
            paint.circle_filled(p, 1.35, Color32::from_rgba_unmultiplied(210, 245, 255, 230));
        }
    }
}

fn blend_map_pixel(image: &mut egui::ColorImage, x: i32, y: i32, color: Color32) {
    if x < 0 || y < 0 || x >= image.size[0] as i32 || y >= image.size[1] as i32 {
        return;
    }
    let index = y as usize * image.size[0] + x as usize;
    let dst = image.pixels[index].to_array();
    let src = color.to_array();
    let a = src[3] as f32 / 255.0;
    image.pixels[index] = Color32::from_rgb(
        (dst[0] as f32 * (1.0 - a) + src[0] as f32 * a).round() as u8,
        (dst[1] as f32 * (1.0 - a) + src[1] as f32 * a).round() as u8,
        (dst[2] as f32 * (1.0 - a) + src[2] as f32 * a).round() as u8,
    );
}

fn rasterize_map_line(
    image: &mut egui::ColorImage,
    a: (f64, f64),
    b: (f64, f64),
    radius: i32,
    color: Color32,
) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
    for step in 0..=steps {
        let t = step as f64 / steps as f64;
        let x = (a.0 + dx * t).round() as i32;
        let y = (a.1 + dy * t).round() as i32;
        for oy in -radius..=radius {
            for ox in -radius..=radius {
                if ox * ox + oy * oy <= radius * radius {
                    blend_map_pixel(image, x + ox, y + oy, color);
                }
            }
        }
    }
}

fn rasterize_wind_streamlines(
    image: &mut egui::ColorImage,
    bounds: Bounds,
    strokes: &[WindStroke],
) {
    let image_width = image.size[0] as f64;
    let image_height = image.size[1] as f64;
    let project = |dir: DVec3| {
        let (lat, lon) = dir_to_geo(dir);
        (
            (lon - bounds.lon_left) / (bounds.lon_right - bounds.lon_left) * image_width,
            (bounds.lat_top - lat) / (bounds.lat_top - bounds.lat_bot) * image_height,
        )
    };
    for stroke in strokes {
        let n = stroke.points.len().saturating_sub(1).max(1);
        let energy = (stroke.mean_speed_mps / 24.0).clamp(0.0, 1.0) as f32;
        for (index, pair) in stroke.points.windows(2).enumerate() {
            let (_, lon0) = dir_to_geo(pair[0]);
            let (_, lon1) = dir_to_geo(pair[1]);
            if (lon0 - lon1).abs() > 90.0 {
                continue;
            }
            let f = (index + 1) as f32 / n as f32;
            let color = Color32::from_rgba_unmultiplied(
                (105.0 + 70.0 * energy) as u8,
                (195.0 + 45.0 * energy) as u8,
                255,
                (38.0 + 190.0 * f.powf(1.35)) as u8,
            );
            rasterize_map_line(
                image,
                project(pair[0]),
                project(pair[1]),
                (f > 0.7) as i32,
                color,
            );
        }
        if let Some(&head) = stroke.points.last() {
            let p = project(head);
            rasterize_map_line(
                image,
                p,
                p,
                2,
                Color32::from_rgba_unmultiplied(210, 245, 255, 230),
            );
        }
    }
}

/// Headless twin of the Neisor teleport-map raster, used by visual gates and
/// evidence capture without opening a window. It shares `synth_map` and the
/// exact streamline geometry with the interactive toggle.
pub fn weather_map_image(
    env: &MapEnv,
    center_lat: f64,
    center_lon: f64,
    zoom: f64,
    width: usize,
    height: usize,
    show_clouds: bool,
    show_wind: bool,
) -> egui::ColorImage {
    let zoom = zoom.clamp(1.0, MAX_ZOOM);
    let half_lon = 180.0 / zoom;
    let half_lat = 90.0 / zoom;
    let center_lon = center_lon.clamp(-180.0 + half_lon, 180.0 - half_lon);
    let center_lat = center_lat.clamp(-90.0 + half_lat, 90.0 - half_lat);
    let bounds = Bounds {
        lat_top: center_lat + half_lat,
        lat_bot: center_lat - half_lat,
        lon_left: center_lon - half_lon,
        lon_right: center_lon + half_lon,
    };
    let mut image = synth_map(
        env,
        BaseLayer::Biomes,
        true,
        show_clouds,
        bounds,
        width.max(64),
        height.max(32),
    );
    if show_wind {
        let strokes = wind_streamlines(env, bounds, zoom);
        rasterize_wind_streamlines(&mut image, bounds, &strokes);
    }
    image
}

/// Equirectangular moon chart synthesized from the exact mesh law.  Albedo is
/// sampled once per pixel; relief uses finite differences over that same
/// height buffer, so map generation does not invent a second crater field.
fn synth_moon_map(seed: i64, b: Bounds, w: usize, h: usize) -> egui::ColorImage {
    let moon = crate::moon::MoonGenerator::new(seed);
    let mut height = vec![0.0f64; w * h];
    let mut albedo = vec![0.0f64; w * h];
    for y in 0..h {
        let lat_deg = b.lat_top + (b.lat_bot - b.lat_top) * (y as f64 + 0.5) / h as f64;
        let lat = lat_deg.to_radians();
        let (slat, clat) = lat.sin_cos();
        for x in 0..w {
            let lon = (b.lon_left + (b.lon_right - b.lon_left) * (x as f64 + 0.5) / w as f64)
                .to_radians();
            let s = moon.sample(DVec3::new(clat * lon.cos(), clat * lon.sin(), slat));
            height[y * w + x] = s.height_ratio;
            albedo[y * w + x] = s.albedo;
        }
    }

    let dlon = ((b.lon_right - b.lon_left).to_radians() / w as f64)
        .abs()
        .max(1e-9);
    let dlat = ((b.lat_top - b.lat_bot).to_radians() / h as f64)
        .abs()
        .max(1e-9);
    // Fixed northwest map light: readable relief, independent of orbital time.
    let light = DVec3::new(-0.34, 0.43, 0.835).normalize(); // east, north, up
    let mut px = vec![Color32::BLACK; w * h];
    for y in 0..h {
        let ym = y.saturating_sub(1);
        let yp = (y + 1).min(h - 1);
        let lat = (b.lat_top + (b.lat_bot - b.lat_top) * (y as f64 + 0.5) / h as f64).to_radians();
        let east_scale =
            (dlon * lat.cos().abs().max(0.04) * (if w > 1 { 2.0 } else { 1.0 })).max(1e-9);
        let north_scale = (dlat * (if h > 1 { 2.0 } else { 1.0 })).max(1e-9);
        for x in 0..w {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let east = (height[y * w + xp] - height[y * w + xm]) / east_scale;
            let north = (height[ym * w + x] - height[yp * w + x]) / north_scale;
            let normal = DVec3::new(-east, -north, 1.0).normalize();
            let hill = normal.dot(light).clamp(-0.2, 1.0);
            let shade = (0.68 + 0.39 * hill).clamp(0.52, 1.07);
            let v = (albedo[y * w + x] * shade).clamp(0.0, 1.0);
            let srgb = v.powf(1.0 / 2.2);
            px[y * w + x] = Color32::from_rgb(
                (srgb * 250.0) as u8,
                (srgb * 252.0) as u8,
                (srgb * 255.0) as u8,
            );
        }
    }
    egui::ColorImage {
        size: [w, h],
        source_size: egui::Vec2::new(w as f32, h as f32),
        pixels: px,
    }
}

/// Headless evidence/export entry point for the teleport map's moon tab.
/// This deliberately calls the exact popup synthesizer above: contact sheets
/// and tests therefore cannot drift into a third lunar map implementation.
pub fn full_moon_map(seed: i64, w: usize, h: usize) -> egui::ColorImage {
    synth_moon_map(
        seed,
        Bounds {
            lat_top: 90.0,
            lat_bot: -90.0,
            lon_left: -180.0,
            lon_right: 180.0,
        },
        w,
        h,
    )
}

// ------------------------------------------------------------- popup state

/// What the popup asks the app to do when the player commits.
pub struct TeleportAction {
    pub body: MapBody,
    pub lat: f64,
    pub lon: f64,
    pub alt_km: Option<f64>,
    pub yaw_deg: Option<f64>,
    pub pitch_deg: Option<f64>,
    pub roll_deg: Option<f64>,
    /// Some(seconds into the day cycle) when "restore time of day" is on
    /// and the photo recorded it.
    pub day_time_s: Option<f64>,
    /// Day length the photo was taken under: day_time_s is meaningful only
    /// as a PHASE of it (t=600 of a 1200 s day is noon, not dusk of a 600 s
    /// day) — the app rescales to the current cycle.
    pub day_len_s: Option<f64>,
    /// Weather restore is controlled by the same opt-in checkbox as time of
    /// day. None means a legacy sidecar or a non-photo destination; Some(false)
    /// explicitly restores weather-off. Some(true) + no pin restores live.
    pub weather_on: Option<bool>,
    pub weather_pin: Option<(f32, f32)>,
    pub weather_time_s: Option<f64>,
    /// Exact-restore state from the sidecar (photo destinations only): the
    /// photographed ground height and mode reproduce the shot instead of
    /// hovering in fly mode 2.5 m over the far terrain surface. The app
    /// honors these only when `seed` matches the loaded planet — restoring
    /// a recorded ground height onto a different world embeds the camera.
    pub ground_km: Option<f64>,
    pub walk: bool,
    pub seed: Option<i64>,
}

pub struct PhotoMap {
    pub open: bool,
    interchange: PathBuf,
    photos: Vec<Photo>,
    map_tex: Option<egui::TextureHandle>,
    /// The (res, layers, region) `map_tex` was synthesized for — a mismatch
    /// with the live view triggers a re-synth (except mid-drag, which
    /// slippy-pans the stale texture until the drag settles).
    map_built: Option<MapSig>,
    preview: Option<(usize, egui::TextureHandle)>,
    /// Full-resolution texture for the lightbox — separate from `preview`,
    /// which is downscaled to 560 px for the map-corner overlay.
    lightbox_tex: Option<(usize, egui::TextureHandle)>,
    /// The full-size photo lightbox (an `egui::Modal` over everything) is
    /// showing. Only meaningful while `selected` is Some.
    lightbox_open: bool,
    selected: Option<usize>,
    checked: HashSet<usize>,
    custom_dest: Option<(f64, f64)>,
    confirm_delete: bool,
    restore_time: bool,
    coord_input: String,
    scroll_to_selected: bool,
    status: String,
    body: MapBody,
    // ---- layer toggles (persist across opens) ----
    base_layer: BaseLayer,
    show_relief: bool,
    show_rivers: bool,
    show_lakes: bool,
    show_clouds: bool,
    show_wind: bool,
    show_markers: bool,
    // ---- pan/zoom view (equirectangular); zoom 1 = whole planet ----
    view_zoom: f64,
    view_center_lat: f64,
    view_center_lon: f64,
    /// Screen size the popup was last laid out for. egui remembers window
    /// positions, so without this a drag or an OS-window resize leaves the
    /// popup off-center forever ("back to the old version" - Austin,
    /// 2026-07-12); on any size change we force a re-center.
    last_screen: Option<egui::Vec2>,
    // ---- time travel (Austin, 2026-07-12: verify the seasons easily) ----
    travel_year: i64,
    travel_month: i64,
    travel_day: i64,
    travel_day_frac: f64,
    travel_seeded: bool,
    /// consumed by the app after ui(): absolute seconds to seek to
    pub pending_time_travel: Option<f64>,
    /// consumed by the app after ui(): new unified-clock rate
    pub pending_time_scale: Option<f64>,
    /// Join controls live in the photo-map window; the app consumes these
    /// requests after egui releases its borrows.
    pub pending_join: Option<(String, String)>,
    pub pending_disconnect: bool,
    join_url: String,
    join_name: String,
}

impl PhotoMap {
    pub fn new(interchange: PathBuf) -> Self {
        Self {
            open: false,
            interchange,
            photos: Vec::new(),
            map_tex: None,
            map_built: None,
            preview: None,
            lightbox_tex: None,
            lightbox_open: false,
            selected: None,
            checked: HashSet::new(),
            custom_dest: None,
            confirm_delete: false,
            restore_time: false,
            coord_input: String::new(),
            scroll_to_selected: false,
            status: String::new(),
            body: MapBody::Neisor,
            base_layer: BaseLayer::Biomes,
            show_relief: true,
            show_rivers: true,
            show_lakes: true,
            show_clouds: false,
            show_wind: false,
            show_markers: true,
            view_zoom: 1.0,
            view_center_lat: 0.0,
            view_center_lon: 0.0,
            last_screen: None,
            travel_year: 1,
            travel_month: 1,
            travel_day: 1,
            travel_day_frac: 0.5,
            travel_seeded: false,
            pending_time_travel: None,
            pending_time_scale: None,
            pending_join: None,
            pending_disconnect: false,
            join_url: String::new(),
            join_name: "Player".into(),
        }
    }

    pub fn set_join_defaults(&mut self, name: impl Into<String>, invite: Option<String>) {
        self.join_name = name.into();
        if let Some(invite) = invite {
            self.join_url = invite;
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.photos = scan_photos(&self.interchange);
            self.selected = None;
            self.preview = None;
            self.lightbox_tex = None;
            self.lightbox_open = false;
            self.checked.clear();
            self.custom_dest = None;
            self.confirm_delete = false;
            // fresh orientation each open: whole planet, and force a re-synth
            // so temperature/precip pick up the current season and clouds the
            // current synoptic field.
            self.reset_view();
            self.map_built = None;
            let count = self.photos.iter().filter(|p| p.body == self.body).count();
            self.status = format!(
                "{count} {} photos",
                if self.body == MapBody::Moon {
                    "moon"
                } else {
                    "Neisor"
                }
            );
        }
    }

    /// Esc backs out one layer inside the open map — lightbox, then delete
    /// confirmation, then the photo selection / map destination — so the
    /// window can always return to its plain state before the next Esc
    /// closes it. Returns true when the keypress was consumed here; false
    /// tells the caller to close the popup itself.
    pub fn handle_escape(&mut self) -> bool {
        if !self.open {
            return false;
        }
        if self.lightbox_open {
            self.lightbox_open = false;
            self.lightbox_tex = None;
            return true;
        }
        if self.confirm_delete {
            self.confirm_delete = false;
            return true;
        }
        if self.selected.is_some() || self.custom_dest.is_some() {
            self.selected = None;
            self.preview = None;
            self.custom_dest = None;
            return true;
        }
        false
    }

    fn reset_view(&mut self) {
        self.view_zoom = 1.0;
        self.view_center_lat = 0.0;
        self.view_center_lon = 0.0;
    }

    /// Keep the view within the planet: zoom in range, and the framed
    /// rectangle inside [-180,180]x[-90,90] (so zoom 1 centers the globe).
    fn clamp_view(&mut self) {
        self.view_zoom = self.view_zoom.clamp(1.0, MAX_ZOOM);
        let hl = 180.0 / self.view_zoom;
        let hla = 90.0 / self.view_zoom;
        self.view_center_lon = self.view_center_lon.clamp(-180.0 + hl, 180.0 - hl);
        self.view_center_lat = self.view_center_lat.clamp(-90.0 + hla, 90.0 - hla);
    }

    fn bounds(&self) -> Bounds {
        let hl = 180.0 / self.view_zoom;
        let hla = 90.0 / self.view_zoom;
        Bounds {
            lat_top: self.view_center_lat + hla,
            lat_bot: self.view_center_lat - hla,
            lon_left: self.view_center_lon - hl,
            lon_right: self.view_center_lon + hl,
        }
    }

    fn load_preview(&mut self, ctx: &egui::Context, idx: usize) {
        if self.preview.as_ref().is_some_and(|(i, _)| *i == idx) {
            return;
        }
        let Some(photo) = self.photos.get(idx) else {
            return;
        };
        // <=560 px wide is plenty for the small map-corner overlay
        let Some(img) = decode_photo_scaled(&photo.path, 560) else {
            return;
        };
        let tex = ctx.load_texture(format!("preview{idx}"), img, Default::default());
        self.preview = Some((idx, tex));
    }

    /// Near-full-resolution texture for the lightbox (capped so a giant
    /// screenshot can't blow the texture budget; 2048 wide fills any screen
    /// the game realistically runs on).
    fn load_lightbox(&mut self, ctx: &egui::Context, idx: usize) {
        if self.lightbox_tex.as_ref().is_some_and(|(i, _)| *i == idx) {
            return;
        }
        let Some(photo) = self.photos.get(idx) else {
            return;
        };
        let Some(img) = decode_photo_scaled(&photo.path, 2048) else {
            return;
        };
        let tex = ctx.load_texture(
            format!("photo-full{idx}"),
            img,
            egui::TextureOptions::LINEAR,
        );
        self.lightbox_tex = Some((idx, tex));
    }

    /// Persist the selected photo's title/note into its JSON sidecar,
    /// preserving every existing field. The in-memory `Photo` already holds
    /// the edited text (the edit boxes bind straight to it), so we only touch
    /// disk here — no re-scan, which would resort the roll under the player.
    fn save_notes(&mut self, idx: usize) {
        let Some(p) = self.photos.get(idx) else {
            return;
        };
        let sidecar = p.path.with_extension("json");
        let name = p.name.clone();
        let title = p.title.clone();
        let note = p.note.clone();
        let mut js = std::fs::read_to_string(&sidecar)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        apply_notes(&mut js, &title, &note);
        self.status = match serde_json::to_string_pretty(&js) {
            Ok(text) => match std::fs::write(&sidecar, text) {
                Ok(()) => format!("saved notes for {name}"),
                Err(e) => format!("note save failed: {e}"),
            },
            Err(e) => format!("note save failed: {e}"),
        };
    }

    fn trash_pair_paths(trash: &Path, png: &Path) -> Option<(PathBuf, PathBuf)> {
        let stem = png.file_stem()?.to_string_lossy();
        let ext = png.extension().and_then(|e| e.to_str()).unwrap_or("png");
        for suffix in 0usize.. {
            let base = if suffix == 0 {
                stem.to_string()
            } else {
                format!("{stem}-{suffix}")
            };
            let png_dest = trash.join(format!("{base}.{ext}"));
            let json_dest = trash.join(format!("{base}.json"));
            if !png_dest.exists() && !json_dest.exists() {
                return Some((png_dest, json_dest));
            }
        }
        None
    }

    fn delete_checked(&mut self) {
        let trash = self.interchange.join("trash");
        if let Err(e) = std::fs::create_dir_all(&trash) {
            self.status = format!("trash unavailable: {e}");
            return;
        }
        let mut moved_files = 0usize;
        let mut failed_files = 0usize;
        for &i in &self.checked {
            if let Some(p) = self.photos.get(i) {
                let Some((png_dest, json_dest)) = Self::trash_pair_paths(&trash, &p.path) else {
                    failed_files += 1;
                    continue;
                };
                if std::fs::rename(&p.path, &png_dest).is_err() {
                    failed_files += 1;
                    continue;
                }
                moved_files += 1;
                let sc = p.path.with_extension("json");
                if sc.exists() {
                    match std::fs::rename(&sc, &json_dest) {
                        Ok(()) => moved_files += 1,
                        Err(_) => {
                            failed_files += 1;
                            moved_files -= 1;
                            if std::fs::rename(&png_dest, &p.path).is_err() {
                                failed_files += 1;
                            }
                        }
                    }
                }
            }
        }
        self.status = if failed_files == 0 {
            format!("moved {moved_files} file(s) to interchange/trash/")
        } else {
            format!("moved {moved_files} file(s); {failed_files} file failure(s)")
        };
        self.checked.clear();
        self.selected = None;
        self.preview = None;
        self.lightbox_tex = None;
        self.lightbox_open = false;
        self.photos = scan_photos(&self.interchange);
    }

    /// Build the popup for this frame. Returns a teleport action when the
    /// player commits (the caller closes the popup by our `open` flag).
    pub fn ui(&mut self, ctx: &egui::Context, env: &MapEnv) -> Option<TeleportAction> {
        if !self.open {
            return None;
        }
        let planet = env.planet;
        let mut action: Option<TeleportAction> = None;
        let screen = ctx.content_rect();
        // Sized to the screen EVERY frame with a comfortable margin, and
        // centered by default: manual horizontal resizing fought the map's
        // fill-available-width layout and snapped back (Austin's report), so
        // the window is not user-resizable - it auto-fits on every window
        // resize instead. Dragging to reposition stays enabled.
        let fit = egui::vec2(screen.width() * 0.92, screen.height() * 0.90);
        let recenter = self.last_screen != Some(screen.size());
        self.last_screen = Some(screen.size());
        let mut window = egui::Window::new("Photo map — teleport")
            .collapsible(false)
            .resizable(false)
            .fixed_size(fit)
            .vscroll(true)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center());
        if recenter {
            // any app-window resize snaps the popup back to center (egui
            // otherwise keeps the remembered, possibly off-screen position)
            window = window.current_pos(screen.center());
        }
        window.show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("Esc backs out · scroll = zoom · drag = pan · double-click = reset · click = destination");
                    });
                });
                if env.multiplayer_available {
                    ui.group(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong("Multiplayer (MP1)");
                            ui.label("Name:");
                            ui.add_enabled(
                                !env.multiplayer_connected,
                                egui::TextEdit::singleline(&mut self.join_name).desired_width(110.0),
                            );
                            ui.label("Join server:");
                            ui.add_enabled(
                                !env.multiplayer_connected,
                                egui::TextEdit::singleline(&mut self.join_url)
                                    .desired_width(430.0)
                                    .hint_text("paste triangulum://host:port/#token or ws://..."),
                            );
                            if env.multiplayer_connected {
                                if ui.button("Disconnect").clicked() { self.pending_disconnect = true; }
                            } else if ui.button("Join").clicked() {
                                self.pending_join = Some((self.join_url.trim().to_string(), self.join_name.trim().to_string()));
                            }
                        });
                        if !env.multiplayer_status.is_empty() {
                            ui.label(env.multiplayer_status);
                        }
                    });
                }
                // ---- time: the one clock, readable and travelable ----
                // (Austin, 2026-07-12: verify seasons simply; stay in place,
                // coordinates optional - map clicks still teleport as ever.)
                {
                    let cal = env.solar_tuning.calendar(env.weather_time_s);
                    if !self.travel_seeded {
                        self.travel_year = cal.year;
                        self.travel_month = cal.month;
                        self.travel_day = cal.day;
                        self.travel_day_frac =
                            (cal.hour as f64 + cal.minute as f64 / 60.0) / 24.0;
                        self.travel_seeded = true;
                    }
                    let months_per_year = (env.solar_tuning.year_days()
                        / env.solar_tuning.lunar_days())
                    .round() as i64;
                    let days_per_month = env.solar_tuning.lunar_days().round() as i64;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "Now: year {} month {} day {}  {:02}:{:02} — season {:.3} — t = {:.1} s — speed {}x",
                            cal.year, cal.month, cal.day, cal.hour, cal.minute,
                            cal.season_frac, env.weather_time_s, env.time_scale,
                        ));
                    });
                    if env.multiplayer_connected {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 205, 90),
                            "Server operator controls time while connected (D-17); [ ], Travel, and Speed are disabled.",
                        );
                    }
                    ui.add_enabled_ui(!env.multiplayer_connected, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Travel to:");
                            ui.add(
                                egui::DragValue::new(&mut self.travel_year)
                                    .range(1..=9999)
                                    .prefix("year "),
                            );
                            ui.add(
                                egui::DragValue::new(&mut self.travel_month)
                                    .range(1..=months_per_year.max(1))
                                    .prefix("month "),
                            );
                            ui.add(
                                egui::DragValue::new(&mut self.travel_day)
                                    .range(1..=days_per_month.max(1))
                                    .prefix("day "),
                            );
                            ui.add(
                                egui::Slider::new(&mut self.travel_day_frac, 0.0..=1.0)
                                    .show_value(false)
                                    .text("time of day"),
                            );
                            if ui
                                .button("Travel (stay here)")
                                .on_hover_text(
                                    "seek the unified clock: sun, seasons, weather, and orbits all move; your position does not",
                                )
                                .clicked()
                            {
                                self.pending_time_travel =
                                    Some(env.solar_tuning.calendar_to_t_s(
                                        self.travel_year,
                                        self.travel_month,
                                        self.travel_day,
                                        self.travel_day_frac,
                                    ));
                            }
                            ui.separator();
                            ui.label("Speed:");
                            for scale in [1.0, 10.0, 60.0, 600.0, 3600.0] {
                                let active = (env.time_scale - scale).abs() < 0.5;
                                if ui.selectable_label(active, format!("{scale:.0}x")).clicked()
                                {
                                    self.pending_time_scale = Some(scale);
                                }
                            }
                        });
                    });
                    ui.separator();
                }
                // ---- body + layer controls ----
                let body_before = self.body;
                ui.horizontal_wrapped(|ui| {
                    ui.label("Body:");
                    ui.radio_value(&mut self.body, MapBody::Neisor, "Neisor");
                    ui.radio_value(&mut self.body, MapBody::Moon, "moon");
                    ui.separator();
                    ui.label("Base:");
                    ui.radio_value(&mut self.base_layer, BaseLayer::Biomes, "Biomes");
                    ui.radio_value(&mut self.base_layer, BaseLayer::Temperature, "Temp");
                    ui.radio_value(&mut self.base_layer, BaseLayer::Precipitation, "Precip");
                    ui.radio_value(&mut self.base_layer, BaseLayer::Elevation, "Elevation")
                        .on_hover_text(
                            "hypsometric tint (green lowlands → white peaks) with contour \
                             isolines; every 4th line is a stronger index contour, and the \
                             interval adapts to the relief in view as you zoom",
                        );
                    ui.separator();
                    ui.checkbox(&mut self.show_relief, "Relief")
                        .on_hover_text("elevation + snow shading of the base");
                    ui.checkbox(&mut self.show_rivers, "Rivers")
                        .on_hover_text("river courses from rivers.bin — thicker/brighter by flow; creeks appear as you zoom in");
                    ui.checkbox(&mut self.show_lakes, "Lakes")
                        .on_hover_text("liquid lakes blue, frozen ones pale");
                    let cloud_tip = if !env.weather_on {
                        "weather is off; this layer is intentionally empty"
                    } else if env.weather_pin.is_some() {
                        "the renderer's pinned cloud-cover field"
                    } else {
                        "the live synoptic cloud field at the current weather time"
                    };
                    ui.checkbox(&mut self.show_clouds, "Clouds now")
                        .on_hover_text(cloud_tip);
                    ui.add_enabled(
                        self.body == MapBody::Neisor,
                        egui::Checkbox::new(&mut self.show_wind, "Wind"),
                    )
                    .on_hover_text(
                        "Neisor only: short comet streamlines integrated through baked wind plus current synoptic storm drift",
                    );
                    ui.checkbox(&mut self.show_markers, "Markers")
                        .on_hover_text("photo markers");
                    ui.separator();
                    if ui.button("Reset view").clicked() {
                        self.reset_view();
                    }
                    ui.label(format!("{:.0}×", self.view_zoom));
                });
                if self.body != body_before {
                    self.reset_view();
                    self.map_built = None;
                    self.selected = None;
                    self.preview = None;
                    self.lightbox_tex = None;
                    self.lightbox_open = false;
                    self.custom_dest = None;
                    self.checked.clear();
                    let count = self.photos.iter().filter(|p| p.body == self.body).count();
                    self.status = format!(
                        "{count} {} photos",
                        if self.body == MapBody::Moon { "moon" } else { "Neisor" }
                    );
                }
                ui.separator();
                let list_w = 340.0;
                ui.horizontal_top(|ui| {
                    // ---------------- left: the map ----------------
                    ui.vertical(|ui| {
                        // The preview and notes no longer live under the map
                        // (overlay + right column now), so the 2:1 map claims
                        // the whole remaining region — as large as it fits.
                        // set_width pins BOTH min and max so nothing in this
                        // column can ever widen the window (see the old
                        // off-screen-inflation bug fixed here).
                        let region_w = (ui.available_width() - list_w - 24.0).max(320.0);
                        let region_h = (ui.available_height() - 4.0).max(160.0);
                        ui.set_width(region_w);
                        let map_w = region_w.min(region_h * 2.0);
                        let map_h = map_w * 0.5;
                        // center the fixed-aspect map inside its region
                        ui.add_space(((region_h - map_h) * 0.5).max(0.0));
                        let (rect, resp) = ui
                            .horizontal(|ui| {
                                ui.add_space(((region_w - map_w) * 0.5).max(0.0));
                                ui.allocate_exact_size(
                                    egui::vec2(map_w, map_h),
                                    egui::Sense::click_and_drag(),
                                )
                            })
                            .inner;

                        // --- interpret reset / pan / zoom BEFORE deriving the
                        // bounds, so this frame's texture and overlays already
                        // reflect the new view (no one-frame lag) ---
                        // `view_animating` means a pan or a (smoothed) zoom is
                        // in progress this frame: skip the expensive re-synth
                        // and let `uv_in` translate/scale the existing texture
                        // for a smooth feel; the crisp rebuild lands the frame
                        // it settles.
                        let mut did_reset = false;
                        let mut view_animating = false;
                        if resp.double_clicked() {
                            // on a marker: open the photo full-size (the first
                            // click of the pair already selected it); on open
                            // water/land: reset the view as before
                            let marker_hit = self.show_markers
                                && resp.interact_pointer_pos().is_some_and(|click| {
                                    let live = self.bounds();
                                    self.photos.iter().any(|p| {
                                        p.body == self.body
                                            && live.project(rect, p.lat, p.lon).distance(click)
                                                < 10.0
                                    })
                                });
                            if marker_hit {
                                self.lightbox_open = true;
                            } else {
                                self.reset_view();
                                did_reset = true;
                            }
                        }
                        if resp.dragged() {
                            let live = self.bounds();
                            let d = resp.drag_delta();
                            self.view_center_lon -= d.x as f64 / rect.width() as f64
                                * (live.lon_right - live.lon_left);
                            self.view_center_lat += d.y as f64 / rect.height() as f64
                                * (live.lat_top - live.lat_bot);
                            self.clamp_view();
                            view_animating = true;
                        }
                        if resp.hovered() {
                            let scroll = ui.input(|i| i.smooth_scroll_delta.y) as f64;
                            if scroll.abs() > 0.0
                                && let Some(cur) = resp.hover_pos()
                            {
                                // keep the geo point under the cursor fixed
                                let live = self.bounds();
                                let (glat, glon) = live.unproject(rect, cur);
                                let fx = ((cur.x - rect.left()) / rect.width()).clamp(0.0, 1.0)
                                    as f64;
                                let fy = ((cur.y - rect.top()) / rect.height()).clamp(0.0, 1.0)
                                    as f64;
                                self.view_zoom =
                                    (self.view_zoom * (scroll * 0.003).exp()).clamp(1.0, MAX_ZOOM);
                                let hl = 180.0 / self.view_zoom;
                                let hla = 90.0 / self.view_zoom;
                                self.view_center_lon = glon + hl * (1.0 - 2.0 * fx);
                                self.view_center_lat = glat + hla * (2.0 * fy - 1.0);
                                self.clamp_view();
                                view_animating = true;
                            }
                        }

                        let bounds = self.bounds();

                        // --- (re)synthesize the base texture when the view or
                        // the rasterized layers changed and we're not mid-drag
                        // (a drag slippy-pans the existing pixels; the crisp
                        // rebuild lands when it settles) ---
                        let ppp = ui.ctx().pixels_per_point();
                        // Resolution cap keeps every re-synth well under ~200 ms
                        // (measured; see the commit message). Cloud cover is now
                        // one cheap shared-raster lookup; temp/precip still carry
                        // two harmonic bilinears (~2x biomes), and biomes are
                        // cheapest — so the crisp cap tracks the base.
                        // Below the cap the map is native; above it LINEAR-
                        // upscales (soft, but clouds/temp are soft fields).
                        let (cap_w, cap_h) = if self.body == MapBody::Moon {
                            (640, 320)
                        } else if self.show_clouds {
                            (512, 256)
                        } else if matches!(
                            self.base_layer,
                            BaseLayer::Biomes | BaseLayer::Elevation
                        ) {
                            // elevation is the cheapest base (one raster
                            // bilinear on land, no biome warp), so it shares
                            // the crispest cap
                            (1280, 640)
                        } else {
                            (960, 480)
                        };
                        let tw = ((map_w * ppp).round() as usize).clamp(64, cap_w);
                        let th = ((map_h * ppp).round() as usize).clamp(32, cap_h);
                        let weather_time_bucket = (self.body == MapBody::Neisor)
                            .then(|| {
                                map_weather_time_bucket(
                                    self.base_layer,
                                    self.show_clouds,
                                    env.weather_on,
                                    env.weather_pin,
                                    env.weather_time_s,
                                )
                            })
                            .flatten();
                        let sig = MapSig {
                            w: tw,
                            h: th,
                            body: self.body,
                            base: self.base_layer,
                            relief: self.show_relief,
                            clouds: self.show_clouds,
                            weather_on: env.weather_on,
                            weather_pin: env.weather_pin,
                            weather_time_bucket,
                            bounds,
                        };
                        if self.map_tex.is_none()
                            || (self.map_built.as_ref() != Some(&sig) && !view_animating)
                        {
                            let t0 = std::time::Instant::now();
                            let img = match self.body {
                                MapBody::Neisor => synth_map(
                                    env,
                                    self.base_layer,
                                    self.show_relief,
                                    self.show_clouds,
                                    bounds,
                                    tw,
                                    th,
                                ),
                                MapBody::Moon => {
                                    synth_moon_map(env.planet.seed, bounds, tw, th)
                                }
                            };
                            self.map_tex = Some(ui.ctx().load_texture(
                                "planet-minimap",
                                img,
                                egui::TextureOptions::LINEAR,
                            ));
                            // profiling aid (house rule: keep map synth under
                            // ~200 ms even with the per-pixel cloud fbm).
                            eprintln!(
                                "map synth {tw}x{th} base={:?} relief={} clouds={}: {:.1} ms",
                                self.base_layer,
                                self.show_relief,
                                self.show_clouds,
                                t0.elapsed().as_secs_f64() * 1000.0
                            );
                            self.map_built = Some(sig);
                        }

                        // everything geographic is drawn through a painter
                        // clipped to the map, so an edge lake/river/marker
                        // can't bleed into the panel around it.
                        let paint = ui.painter_at(rect);

                        // paint the base texture, addressing the live window
                        // inside whatever region the texture was built for
                        if let (Some(tex), Some(built)) = (&self.map_tex, &self.map_built) {
                            paint.image(tex.id(), rect, bounds.uv_in(built.bounds), Color32::WHITE);
                        }

                        // ---- vector overlays (crisp geometry at any zoom) ----
                        let deg_per_km = 360.0 / (std::f64::consts::TAU * planet.radius_km);
                        let px_per_deg =
                            rect.width() as f64 / (bounds.lon_right - bounds.lon_left);

                        // lakes: liquid blue, frozen pale (skip dry rim rows)
                        if self.body == MapBody::Neisor && self.show_lakes {
                            for l in &planet.rivers.lakes {
                                if l.rim {
                                    continue;
                                }
                                let (la, lo) = dir_to_geo(l.center);
                                let pos = bounds.project(rect, la, lo);
                                let r_px = (l.radius_km as f64 * deg_per_km * px_per_deg)
                                    .max(1.5) as f32;
                                if pos.x + r_px < rect.left()
                                    || pos.x - r_px > rect.right()
                                    || pos.y + r_px < rect.top()
                                    || pos.y - r_px > rect.bottom()
                                {
                                    continue;
                                }
                                let (f, u, v) = face_from_dir(l.center);
                                let structural = if env.weather_on {
                                    crate::weather::StructuralSeason::quantized(
                                        crate::weather::season_frac(
                                            env.weather_time_s,
                                            env.day_len_s,
                                            env.solar_tuning,
                                        ),
                                        env.weather_tuning,
                                    )
                                } else {
                                    crate::weather::StructuralSeason::annual()
                                };
                                let frozen = planet.water_frozen(f, u, v, false, structural);
                                let fill = if frozen {
                                    Color32::from_rgb(206, 224, 236)
                                } else {
                                    Color32::from_rgb(42, 108, 196)
                                };
                                paint.circle_filled(pos, r_px, fill);
                            }
                        }

                        // rivers: polylines from rivers.bin. Big rivers pop;
                        // the flow gate drops as you zoom so creeks fade in.
                        if self.body == MapBody::Neisor && self.show_rivers {
                            // rivers.bin's flow_log floor is ~2.1 and its ceil
                            // ~4.8; at the whole planet show only the major
                            // network (~gate 3.8), dropping the gate ~0.85 per
                            // zoom-doubling so the full drainage (creeks and
                            // all) is visible by ~4x.
                            let min_flow =
                                (3.8 - 0.85 * self.view_zoom.log2()).clamp(2.0, 6.0) as f32;
                            let m = 4.0f32;
                            for s in &planet.rivers.segments {
                                if s.flow_log < min_flow {
                                    continue;
                                }
                                let (la, loa) = dir_to_geo(s.a);
                                let (lb, lob) = dir_to_geo(s.b);
                                // a segment straddling the ±180 seam (or a pole)
                                // projects to a spurious full-width streak — drop
                                if (loa - lob).abs() > 90.0 {
                                    continue;
                                }
                                let pa = bounds.project(rect, la, loa);
                                let pb = bounds.project(rect, lb, lob);
                                let (minx, maxx) = (pa.x.min(pb.x), pa.x.max(pb.x));
                                let (miny, maxy) = (pa.y.min(pb.y), pa.y.max(pb.y));
                                if maxx < rect.left() - m
                                    || minx > rect.right() + m
                                    || maxy < rect.top() - m
                                    || miny > rect.bottom() + m
                                {
                                    continue;
                                }
                                let width = (0.5 + (s.flow_log - 2.5) * 1.0).clamp(0.6, 3.5);
                                let bright = (0.30 + (s.flow_log - 2.0) * 0.26).clamp(0.4, 1.0);
                                let col = Color32::from_rgb(
                                    (55.0 * bright) as u8,
                                    (118.0 * bright) as u8,
                                    (232.0 * bright) as u8,
                                );
                                paint.line_segment([pa, pb], egui::Stroke::new(width, col));
                            }
                        }

                        if self.body == MapBody::Neisor && self.show_wind {
                            let strokes = wind_streamlines(env, bounds, self.view_zoom);
                            paint_wind_streamlines(&paint, rect, bounds, &strokes);
                        }

                        // markers, "you are here", custom destination (top)
                        let in_rect = |p: egui::Pos2| rect.expand(2.0).contains(p);
                        if self.show_markers {
                            for (i, p) in self.photos.iter().enumerate() {
                                if p.body != self.body {
                                    continue;
                                }
                                let pos = bounds.project(rect, p.lat, p.lon);
                                if !in_rect(pos) {
                                    continue;
                                }
                                let sel = self.selected == Some(i);
                                let checkedc = self.checked.contains(&i);
                                let fill = if sel {
                                    Color32::from_rgb(255, 230, 90)
                                } else if checkedc {
                                    Color32::from_rgb(255, 120, 90)
                                } else {
                                    Color32::from_rgb(80, 200, 255)
                                };
                                paint.circle_filled(pos, if sel { 6.0 } else { 4.0 }, fill);
                                paint.circle_stroke(
                                    pos,
                                    if sel { 8.0 } else { 5.5 },
                                    egui::Stroke::new(1.5, Color32::from_black_alpha(160)),
                                );
                            }
                        }
                        // current player position
                        if env.cur_body == self.body {
                            let (cur_lat, cur_lon) = match self.body {
                                MapBody::Neisor => (env.cur_lat, env.cur_lon),
                                MapBody::Moon => (env.cur_moon_lat, env.cur_moon_lon),
                            };
                            let pos = bounds.project(rect, cur_lat, cur_lon);
                            if in_rect(pos) {
                                paint.circle_filled(pos, 4.0, Color32::from_rgb(120, 255, 140));
                                paint.circle_stroke(
                                    pos,
                                    6.5,
                                    egui::Stroke::new(2.0, Color32::from_rgb(20, 90, 30)),
                                );
                            }
                        }
                        if let Some((la, lo)) = self.custom_dest {
                            let pos = bounds.project(rect, la, lo);
                            if in_rect(pos) {
                                let s = 7.0;
                                let st = egui::Stroke::new(2.0, Color32::from_rgb(255, 90, 90));
                                paint.line_segment(
                                    [pos - egui::vec2(s, 0.0), pos + egui::vec2(s, 0.0)],
                                    st,
                                );
                                paint.line_segment(
                                    [pos - egui::vec2(0.0, s), pos + egui::vec2(0.0, s)],
                                    st,
                                );
                            }
                        }
                        // clicks (a real click, not a drag/double-click): pick
                        // the nearest photo marker, else set a free destination
                        // — both through the LIVE view transform
                        if resp.clicked()
                            && !did_reset
                            && let Some(click) = resp.interact_pointer_pos()
                        {
                            let mut best: Option<(usize, f32)> = None;
                            if self.show_markers {
                                for (i, p) in self.photos.iter().enumerate() {
                                    if p.body != self.body {
                                        continue;
                                    }
                                    let d = bounds.project(rect, p.lat, p.lon).distance(click);
                                    if d < 10.0 && best.is_none_or(|(_, bd)| d < bd) {
                                        best = Some((i, d));
                                    }
                                }
                            }
                            match best {
                                Some((i, _)) => {
                                    self.selected = Some(i);
                                    self.custom_dest = None;
                                    self.scroll_to_selected = true;
                                }
                                None => {
                                    let (lat, lon) = bounds.unproject(rect, click);
                                    self.custom_dest =
                                        Some((lat.clamp(-90.0, 90.0), lon.clamp(-180.0, 180.0)));
                                    self.selected = None;
                                }
                            }
                        }
                        // hover label, painted beside the cursor
                        if self.show_markers
                            && let Some(hp) = resp.hover_pos()
                        {
                            for p in &self.photos {
                                if p.body != self.body {
                                    continue;
                                }
                                if bounds.project(rect, p.lat, p.lon).distance(hp) < 10.0 {
                                    let font = egui::FontId::proportional(12.0);
                                    let text = if p.title.trim().is_empty() {
                                        p.name.clone()
                                    } else {
                                        p.title.clone()
                                    };
                                    let galley = ui.painter().layout_no_wrap(
                                        text,
                                        font,
                                        Color32::WHITE,
                                    );
                                    let at = hp + egui::vec2(14.0, -8.0);
                                    let bg = egui::Rect::from_min_size(at, galley.size())
                                        .expand(4.0);
                                    ui.painter().rect_filled(
                                        bg,
                                        4.0,
                                        Color32::from_black_alpha(200),
                                    );
                                    ui.painter().galley(at, galley, Color32::WHITE);
                                    break;
                                }
                            }
                        }
                        // small preview of the selected photo, overlaid on the
                        // map's bottom-left corner. PAINTED, never laid out, so
                        // selecting a photo can't change the window size (the
                        // old below-map preview + unbounded-width note editor
                        // inflated the popup past the screen edges). Click it
                        // for the full-size lightbox; ✕ clears the selection.
                        if let Some(sel) = self.selected {
                            self.load_preview(ui.ctx(), sel);
                            if let Some((pi, tex)) = &self.preview
                                && *pi == sel
                            {
                                let ts = tex.size_vec2();
                                let pv_max_w = (map_w * 0.30).clamp(140.0, 420.0);
                                let pv_max_h = (map_h * 0.42).clamp(90.0, 260.0);
                                let s = (pv_max_w / ts.x).min(pv_max_h / ts.y).min(1.0);
                                let pv = egui::Rect::from_min_size(
                                    egui::pos2(
                                        rect.left() + 10.0,
                                        rect.bottom() - 10.0 - ts.y * s,
                                    ),
                                    ts * s,
                                );
                                paint.rect_filled(
                                    pv.expand(3.0),
                                    4.0,
                                    Color32::from_black_alpha(190),
                                );
                                paint.image(
                                    tex.id(),
                                    pv,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    Color32::WHITE,
                                );
                                paint.rect_stroke(
                                    pv.expand(3.0),
                                    4.0,
                                    egui::Stroke::new(1.5, Color32::from_rgb(255, 230, 90)),
                                    egui::StrokeKind::Outside,
                                );
                                let pv_resp = ui
                                    .interact(
                                        pv,
                                        ui.id().with("photo-preview"),
                                        egui::Sense::click(),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .on_hover_text("click to view full size");
                                if pv_resp.clicked() {
                                    self.lightbox_open = true;
                                }
                                // close affordance: selection is always clearable
                                let close = egui::Rect::from_center_size(
                                    pv.right_top() + egui::vec2(-9.0, 9.0),
                                    egui::vec2(18.0, 18.0),
                                );
                                if ui
                                    .put(close, egui::Button::new("✕").small())
                                    .on_hover_text("clear selection (Esc)")
                                    .clicked()
                                {
                                    self.selected = None;
                                    self.preview = None;
                                }
                            }
                        }
                    });
                    ui.separator();
                    // ---------------- right: the photo list ----------------
                    ui.vertical(|ui| {
                        ui.set_width(list_w);
                        let body_photo_indices: Vec<usize> = self
                            .photos
                            .iter()
                            .enumerate()
                            .filter_map(|(i, p)| (p.body == self.body).then_some(i))
                            .collect();
                        ui.horizontal(|ui| {
                            if ui.button("Select all").clicked() {
                                self.checked = body_photo_indices.iter().copied().collect();
                            }
                            if ui.button("Clear").clicked() {
                                self.checked.clear();
                            }
                            let n = self.checked.len();
                            ui.add_enabled_ui(n > 0, |ui| {
                                if ui
                                    .button(format!("Delete {n}…"))
                                    .on_hover_text("moves to interchange/trash/")
                                    .clicked()
                                {
                                    self.confirm_delete = true;
                                }
                            });
                        });
                        ui.separator();
                        let row_h = 34.0;
                        // reserve room under the list for the coord/teleport
                        // footer, plus the title/notes editor when a photo is
                        // selected (it lives in this fixed-width column so it
                        // can never widen the window)
                        let footer_h = if self.selected.is_some() { 286.0 } else { 96.0 };
                        egui::ScrollArea::vertical().max_height(
                            (ui.available_height() - footer_h).max(row_h * 3.0),
                        ).show_rows(
                            ui,
                            row_h,
                            body_photo_indices.len(),
                            |ui, range| {
                                for row in range {
                                    let i = body_photo_indices[row];
                                    let p = &self.photos[i];
                                    let sel = self.selected == Some(i);
                                    ui.horizontal(|ui| {
                                        let mut ck = self.checked.contains(&i);
                                        if ui.checkbox(&mut ck, "").changed() {
                                            if ck {
                                                self.checked.insert(i);
                                            } else {
                                                self.checked.remove(&i);
                                            }
                                        }
                                        let clock =
                                            if p.day_time_s.is_some() { "  ⏱" } else { "" };
                                        // the player's title when they gave one,
                                        // else the coordinates as before
                                        let label = if p.title.trim().is_empty() {
                                            format!(
                                                "{:.3} {:.3}  alt {:.0} m{}",
                                                p.lat,
                                                p.lon,
                                                p.alt_km * 1000.0,
                                                clock,
                                            )
                                        } else {
                                            format!("{}{}", p.title, clock)
                                        };
                                        let r = ui.selectable_label(sel, label);
                                        if sel && self.scroll_to_selected {
                                            r.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if r.double_clicked() {
                                            // the first click of the pair
                                            // already selected + previewed it
                                            self.selected = Some(i);
                                            self.custom_dest = None;
                                            self.lightbox_open = true;
                                        } else if r.clicked() {
                                            self.selected = Some(i);
                                            self.custom_dest = None;
                                        }
                                        // filename, plus the note when present
                                        let mut hover = p.name.clone();
                                        if !p.note.trim().is_empty() {
                                            hover.push_str("\n\n");
                                            hover.push_str(&p.note);
                                        }
                                        r.on_hover_text(hover);
                                    });
                                }
                                self.scroll_to_selected = false;
                            },
                        );
                        ui.separator();
                        // title + notes for the selected photo, editable and
                        // written straight into its sidecar. The edit boxes
                        // bind to the in-memory Photo; Save (or focus loss)
                        // commits it to disk. This column's set_width bounds
                        // the desired_width(INFINITY) edits at list_w.
                        if let Some(sel) = self.selected
                            && sel < self.photos.len()
                        {
                            ui.label("Title");
                            let title_resp = ui.add(
                                egui::TextEdit::singleline(&mut self.photos[sel].title)
                                    .hint_text("a name for this shot")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.label("Notes");
                            let note_resp = ui.add(
                                egui::TextEdit::multiline(&mut self.photos[sel].note)
                                    .hint_text("notes about this shot")
                                    .desired_rows(3)
                                    .desired_width(f32::INFINITY),
                            );
                            let save = ui.button("Save").clicked();
                            if save || title_resp.lost_focus() || note_resp.lost_focus() {
                                self.save_notes(sel);
                            }
                            ui.separator();
                        }
                        // manual coordinates, like the old title-bar prompt
                        ui.horizontal(|ui| {
                            ui.label("lat lon [alt km]:");
                            ui.text_edit_singleline(&mut self.coord_input);
                        });
                        ui.checkbox(
                            &mut self.restore_time,
                            "Restore photo's time of day",
                        )
                        .on_hover_text(
                            "teleporting to a photo also rewinds the day/night cycle \
                             and restores its recorded weather pin/off state or absolute \
                             weather time (sidecar shots only)",
                        );
                        let dest = self.destination();
                        ui.add_enabled_ui(dest.is_some(), |ui| {
                            let label = match (&self.selected, &self.custom_dest) {
                                (Some(_), _) => "Teleport to photo",
                                (None, Some(_)) => "Teleport to map point",
                                _ if !self.coord_input.trim().is_empty() => "Teleport to coordinates",
                                _ => "Teleport",
                            };
                            if ui.button(label).clicked() {
                                action = dest;
                            }
                        });
                    });
                });
            });
        // full-size photo lightbox: a true egui::Modal anchored to the SCREEN,
        // not the map window — the dimmed backdrop covers everything (map
        // window included), so it reads as a photo viewer rather than a
        // window stacked inside a window. Click outside (or Esc, routed via
        // handle_escape) dismisses it and returns to the map untouched.
        if self.lightbox_open {
            match self.selected {
                Some(sel) if sel < self.photos.len() => {
                    self.load_lightbox(ctx, sel);
                    let modal = egui::Modal::new(egui::Id::new("photo-lightbox"))
                        .backdrop_color(Color32::from_black_alpha(160))
                        .show(ctx, |ui| {
                            let max_w = screen.width() * 0.86;
                            let max_h = (screen.height() * 0.86 - 70.0).max(120.0);
                            if let Some((pi, tex)) = &self.lightbox_tex
                                && *pi == sel
                            {
                                let ts = tex.size_vec2();
                                let s = (max_w / ts.x).min(max_h / ts.y).min(2.0);
                                ui.set_max_width((ts.x * s).max(240.0));
                                ui.image((tex.id(), ts * s));
                            } else {
                                ui.set_max_width(320.0);
                                ui.label("loading full-size photo…");
                            }
                            ui.add_space(4.0);
                            let p = &self.photos[sel];
                            ui.strong(if p.title.trim().is_empty() {
                                &p.name
                            } else {
                                &p.title
                            });
                            if !p.note.trim().is_empty() {
                                ui.weak(&p.note);
                            }
                            ui.weak("click outside or press Esc to close");
                        });
                    if modal.should_close() {
                        self.lightbox_open = false;
                        self.lightbox_tex = None;
                    }
                }
                _ => {
                    self.lightbox_open = false;
                    self.lightbox_tex = None;
                }
            }
        }
        // delete confirmation modal
        if self.confirm_delete {
            let n = self.checked.len();
            egui::Window::new("Delete photos?")
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(screen.center())
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Move {n} photo{} (and sidecars) to interchange/trash/?",
                        if n == 1 { "" } else { "s" }
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            self.delete_checked();
                            self.confirm_delete = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.confirm_delete = false;
                        }
                    });
                });
        }
        if action.is_some() {
            self.open = false;
        }
        action
    }

    fn destination(&self) -> Option<TeleportAction> {
        if let Some(i) = self.selected {
            let p = self.photos.get(i)?;
            return Some(TeleportAction {
                body: p.body,
                lat: p.lat,
                lon: p.lon,
                alt_km: Some(p.alt_km.max(0.0025)),
                yaw_deg: Some(p.yaw_deg),
                pitch_deg: Some(p.pitch_deg),
                roll_deg: Some(p.roll_deg),
                day_time_s: if self.restore_time {
                    p.day_time_s
                } else {
                    None
                },
                day_len_s: if self.restore_time { p.day_len_s } else { None },
                weather_on: if self.restore_time {
                    p.weather_on
                } else {
                    None
                },
                weather_pin: if self.restore_time {
                    p.weather_pin
                } else {
                    None
                },
                weather_time_s: if self.restore_time {
                    p.weather_time_s
                } else {
                    None
                },
                ground_km: p.ground_km,
                walk: p.mode.as_deref() == Some("walk"),
                seed: p.seed,
            });
        }
        if let Some((lat, lon)) = self.custom_dest {
            return Some(TeleportAction {
                body: self.body,
                lat,
                lon,
                alt_km: None,
                yaw_deg: None,
                pitch_deg: None,
                roll_deg: None,
                day_time_s: None,
                day_len_s: None,
                weather_on: None,
                weather_pin: None,
                weather_time_s: None,
                ground_km: None,
                walk: false,
                seed: None,
            });
        }
        // manual "lat lon [alt]" text, the old prompt's grammar — strict:
        // exactly 2 or 3 tokens, every one a finite number ("NaN" parses as
        // a valid f64 and would poison the camera), latitude in range
        let toks: Vec<&str> = self.coord_input.split_whitespace().collect();
        if toks.len() == 2 || toks.len() == 3 {
            let parts: Vec<f64> = toks
                .iter()
                .filter_map(|t| t.parse().ok())
                .filter(|v: &f64| v.is_finite())
                .collect();
            if parts.len() == toks.len() && parts[0].abs() <= 90.0 {
                return Some(TeleportAction {
                    body: self.body,
                    lat: parts[0],
                    lon: parts[1],
                    alt_km: parts.get(2).copied().filter(|a| *a > 0.0),
                    yaw_deg: None,
                    pitch_deg: None,
                    roll_deg: None,
                    day_time_s: None,
                    day_len_s: None,
                    weather_on: None,
                    weather_pin: None,
                    weather_time_s: None,
                    ground_km: None,
                    walk: false,
                    seed: None,
                });
            }
        }
        None
    }
}

/// Read a photo PNG and decode it into an egui image, step-downsampled to at
/// most `max_w` pixels wide (shared by the small preview and the lightbox).
fn decode_photo_scaled(path: &Path, max_w: usize) -> Option<egui::ColorImage> {
    let raw = std::fs::read(path).ok()?;
    let (mut dims, mut px) = png_dims_and_rgba(&raw).ok()?;
    if dims[0] > max_w {
        let step = dims[0].div_ceil(max_w);
        let (nw, nh) = (dims[0] / step, dims[1] / step);
        let mut small = Vec::with_capacity(nw * nh);
        for y in 0..nh {
            for x in 0..nw {
                small.push(px[y * step * dims[0] + x * step]);
            }
        }
        dims = [nw, nh];
        px = small;
    }
    Some(egui::ColorImage {
        size: dims,
        source_size: egui::Vec2::new(dims[0] as f32, dims[1] as f32),
        pixels: px,
    })
}

/// Decode a PNG into egui Color32 pixels (RGB/RGBA/gray supported).
fn png_dims_and_rgba(raw: &[u8]) -> anyhow::Result<([usize; 2], Vec<Color32>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(raw));
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| anyhow::anyhow!("png output size overflow"))?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let mut px = Vec::with_capacity(w * h);
    match info.color_type {
        png::ColorType::Rgba => {
            for c in buf[..w * h * 4].chunks_exact(4) {
                px.push(Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
            }
        }
        png::ColorType::Rgb => {
            for c in buf[..w * h * 3].chunks_exact(3) {
                px.push(Color32::from_rgb(c[0], c[1], c[2]));
            }
        }
        png::ColorType::Grayscale => {
            for &g in &buf[..w * h] {
                px.push(Color32::from_gray(g));
            }
        }
        other => anyhow::bail!("unsupported png color type {other:?}"),
    }
    Ok(([w, h], px))
}

// ----------------------------------------------------- egui paint (wgpu 30)

/// Minimal egui renderer against the viewer's own wgpu version. egui's
/// output is clipped, textured, vertex-colored triangles in physical pixel
/// space; this uploads them and draws with premultiplied-alpha blending
/// onto the already-rendered frame.
pub struct EguiPaint {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_layout: wgpu::BindGroupLayout,
    textures: std::collections::HashMap<TextureId, (wgpu::Texture, wgpu::BindGroup)>,
    vbuf: Option<wgpu::Buffer>,
    ibuf: Option<wgpu::Buffer>,
    uniform: wgpu::Buffer,
    uniform_bind: wgpu::BindGroup,
}

const EGUI_SHADER: &str = r#"
struct Screen { size_px: vec2<f32>, _pad: vec2<f32> };
@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>, // sRGB 0-1, premultiplied coverage semantics
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c < vec3<f32>(0.04045));
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(
        in.pos.x / screen.size_px.x * 2.0 - 1.0,
        1.0 - in.pos.y / screen.size_px.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = in.uv;
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
    // egui vertex colors and textures are ALREADY premultiplied — do not
    // multiply rgb by alpha again (double-premultiply darkens translucent
    // panels and fringes text edges)
    return in.color * t;
}
"#;

impl EguiPaint {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("egui"),
            source: wgpu::ShaderSource::Wgsl(EGUI_SHADER.into()),
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("egui"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("egui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("egui"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("egui-screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("egui-screen"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            pipeline,
            sampler,
            bind_layout,
            textures: Default::default(),
            vbuf: None,
            ibuf: None,
            uniform,
            uniform_bind,
        }
    }

    fn apply_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: TextureId,
        delta: &ImageDelta,
    ) {
        let size = delta.image.size();
        let pixels: Vec<u8> = match &delta.image {
            egui::ImageData::Color(img) => img.pixels.iter().flat_map(|c| c.to_array()).collect(),
        };
        let whole = delta.pos.is_none();
        if whole {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("egui-tex"),
                size: wgpu::Extent3d {
                    width: size[0] as u32,
                    height: size[1] as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("egui-tex"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.textures.insert(id, (tex, bind));
        }
        if let Some((tex, _)) = self.textures.get(&id) {
            let (x, y) = delta.pos.map_or((0, 0), |p| (p[0] as u32, p[1] as u32));
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * size[0] as u32),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: size[0] as u32,
                    height: size[1] as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Paint one egui frame onto `view` (which already holds the scene).
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        size_px: (u32, u32),
        pixels_per_point: f32,
        primitives: &[ClippedPrimitive],
        deltas: &TexturesDelta,
    ) {
        for (id, delta) in &deltas.set {
            self.apply_texture(device, queue, *id, delta);
        }
        // flatten meshes into one vertex/index upload
        let mut verts: Vec<u8> = Vec::new();
        let mut idxs: Vec<u32> = Vec::new();
        let mut draws = Vec::new(); // (clip, tex, index range, base vertex)
        let mut vcount = 0u32;
        for cp in primitives {
            let Primitive::Mesh(mesh) = &cp.primitive else {
                continue;
            };
            let istart = idxs.len() as u32;
            idxs.extend(mesh.indices.iter().map(|&i| i + vcount));
            for v in &mesh.vertices {
                verts.extend_from_slice(&v.pos.x.to_le_bytes());
                verts.extend_from_slice(&v.pos.y.to_le_bytes());
                verts.extend_from_slice(&v.uv.x.to_le_bytes());
                verts.extend_from_slice(&v.uv.y.to_le_bytes());
                verts.extend_from_slice(&v.color.to_array());
            }
            vcount += mesh.vertices.len() as u32;
            draws.push((cp.clip_rect, mesh.texture_id, istart..idxs.len() as u32));
        }
        if draws.is_empty() {
            return;
        }
        let vbytes: &[u8] = &verts;
        let ibytes: &[u8] = bytemuck::cast_slice(&idxs);
        let need_v = vbytes.len() as u64;
        let need_i = ibytes.len() as u64;
        if self.vbuf.as_ref().is_none_or(|b| b.size() < need_v) {
            self.vbuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("egui-v"),
                size: need_v.next_power_of_two().max(1 << 16),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if self.ibuf.as_ref().is_none_or(|b| b.size() < need_i) {
            self.ibuf = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("egui-i"),
                size: need_i.next_power_of_two().max(1 << 16),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let (vbuf, ibuf) = (self.vbuf.as_ref().unwrap(), self.ibuf.as_ref().unwrap());
        queue.write_buffer(vbuf, 0, vbytes);
        queue.write_buffer(ibuf, 0, ibytes);
        let logical = [
            size_px.0 as f32 / pixels_per_point,
            size_px.1 as f32 / pixels_per_point,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&logical));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.uniform_bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            for (clip, tex_id, range) in draws {
                let Some((_, bind)) = self.textures.get(&tex_id) else {
                    continue;
                };
                // clip rect: logical points -> physical pixels, clamped
                let cx = (clip.min.x * pixels_per_point).max(0.0) as u32;
                let cy = (clip.min.y * pixels_per_point).max(0.0) as u32;
                let cx1 = ((clip.max.x * pixels_per_point) as u32).min(size_px.0);
                let cy1 = ((clip.max.y * pixels_per_point) as u32).min(size_px.1);
                if cx1 <= cx || cy1 <= cy {
                    continue;
                }
                pass.set_scissor_rect(cx, cy, cx1 - cx, cy1 - cy);
                pass.set_bind_group(1, bind, &[]);
                pass.draw_indexed(range, 0, 0..1);
            }
        }
        queue.submit([enc.finish()]);
        for id in &deltas.free {
            self.textures.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_weather_restore_is_opt_in_with_time_checkbox() {
        let parsed = sidecar_weather(&serde_json::json!({
            "weather": {"on": true, "pinned": [0.97, 0.9], "t_s": 24_000.0}
        }));
        assert_eq!(parsed, Some((true, Some((0.97, 0.9)), Some(24_000.0))));

        let mut map = PhotoMap::new(PathBuf::from("unused"));
        map.photos.push(Photo {
            path: PathBuf::from("shot.png"),
            name: "shot.png".into(),
            body: MapBody::Neisor,
            lat: 1.0,
            lon: 2.0,
            alt_km: 0.1,
            yaw_deg: 3.0,
            pitch_deg: -4.0,
            roll_deg: 5.0,
            day_time_s: Some(600.0),
            weather_on: Some(true),
            weather_pin: Some((0.97, 0.9)),
            weather_time_s: Some(24_000.0),
            ground_km: Some(0.2),
            mode: Some("fly".into()),
            day_len_s: Some(1200.0),
            seed: Some(42),
            title: String::new(),
            note: String::new(),
        });
        map.selected = Some(0);

        let normal = map.destination().unwrap();
        assert_eq!(normal.body, MapBody::Neisor);
        assert_eq!(normal.day_time_s, None);
        assert_eq!(normal.weather_on, None);
        assert_eq!(normal.weather_pin, None);
        assert_eq!(normal.weather_time_s, None);
        assert_eq!(normal.roll_deg, Some(5.0));

        map.restore_time = true;
        let restored = map.destination().unwrap();
        assert_eq!(restored.day_time_s, Some(600.0));
        assert_eq!(restored.weather_on, Some(true));
        assert_eq!(restored.weather_pin, Some((0.97, 0.9)));
        assert_eq!(restored.weather_time_s, Some(24_000.0));
    }

    #[test]
    fn escape_backs_out_lightbox_then_selection_then_window() {
        let mut map = PhotoMap::new(PathBuf::from("unused"));
        // closed map: Esc is not ours to consume
        assert!(!map.handle_escape());
        map.open = true;
        map.selected = Some(0);
        map.custom_dest = Some((1.0, 2.0));
        map.lightbox_open = true;
        // 1st Esc: the lightbox closes, the selection stays on the map
        assert!(map.handle_escape());
        assert!(!map.lightbox_open);
        assert_eq!(map.selected, Some(0));
        // 2nd Esc: selection and free destination clear — the window is
        // back in its plain state (the old bug: nothing ever cleared these)
        assert!(map.handle_escape());
        assert_eq!(map.selected, None);
        assert_eq!(map.custom_dest, None);
        // 3rd Esc: nothing left to pop — the caller closes the window
        assert!(!map.handle_escape());
    }

    #[test]
    fn moon_map_is_deterministic_and_uses_generated_range() {
        let bounds = Bounds {
            lat_top: 90.0,
            lat_bot: -90.0,
            lon_left: -180.0,
            lon_right: 180.0,
        };
        let a = synth_moon_map(42, bounds, 128, 64);
        let b = synth_moon_map(42, bounds, 128, 64);
        assert_eq!(a.pixels, b.pixels);
        let (mut lo, mut hi) = (255u8, 0u8);
        for pixel in &a.pixels {
            let [r, g, b, _] = pixel.to_array();
            let value = r.max(g).max(b);
            lo = lo.min(value);
            hi = hi.max(value);
        }
        assert!(lo < 170, "moon map lost dark maria: {lo}..{hi}");
        assert!(hi > 220, "moon map lost bright relief/rays: {lo}..{hi}");

        let mut map = PhotoMap::new(PathBuf::from("unused"));
        map.body = MapBody::Moon;
        map.custom_dest = Some((12.0, -34.0));
        assert_eq!(map.destination().unwrap().body, MapBody::Moon);
    }

    #[test]
    fn sidecar_notes_round_trip_preserves_other_fields() {
        // a representative sidecar with pose, repro, and weather fields
        let mut js = serde_json::json!({
            "lat_deg": 4.99,
            "lon_deg": -29.403,
            "alt_km": 0.047,
            "seed": 42,
            "mode": "walk",
            "weather": {"on": true, "pinned": [0.97, 0.9], "t_s": 24_000.0},
        });
        apply_notes(&mut js, "Cliffside creek", "north bank, dusk light");

        // the two new fields land as strings
        assert_eq!(sidecar_str(&js, "title"), "Cliffside creek");
        assert_eq!(sidecar_str(&js, "note"), "north bank, dusk light");
        // every prior field is byte-for-byte untouched
        assert_eq!(js.get("lat_deg").and_then(|v| v.as_f64()), Some(4.99));
        assert_eq!(js.get("lon_deg").and_then(|v| v.as_f64()), Some(-29.403));
        assert_eq!(js.get("alt_km").and_then(|v| v.as_f64()), Some(0.047));
        assert_eq!(js.get("seed").and_then(|v| v.as_i64()), Some(42));
        assert_eq!(js.get("mode").and_then(|v| v.as_str()), Some("walk"));
        assert_eq!(js["weather"]["pinned"][0].as_f64(), Some(0.97));
        assert_eq!(js["weather"]["t_s"].as_f64(), Some(24_000.0));

        // re-saving overwrites just the two keys (no dupes, nothing dropped);
        // 6 original keys + title + note = 8
        apply_notes(&mut js, "renamed", "");
        assert_eq!(sidecar_str(&js, "title"), "renamed");
        assert_eq!(sidecar_str(&js, "note"), "");
        assert_eq!(js.get("seed").and_then(|v| v.as_i64()), Some(42));
        assert_eq!(js.as_object().unwrap().len(), 8);
    }

    #[test]
    fn sidecar_notes_default_to_empty_and_survive_non_object() {
        // legacy sidecar without the keys reads as empty, never an error
        let legacy = serde_json::json!({"lat_deg": 1.0, "lon_deg": 2.0});
        assert_eq!(sidecar_str(&legacy, "title"), "");
        assert_eq!(sidecar_str(&legacy, "note"), "");

        // a corrupt / non-object sidecar becomes a fresh object with just the
        // notes rather than panicking or silently dropping the save
        let mut junk = serde_json::json!("not an object");
        apply_notes(&mut junk, "t", "n");
        assert_eq!(sidecar_str(&junk, "title"), "t");
        assert_eq!(sidecar_str(&junk, "note"), "n");
        assert_eq!(junk.as_object().unwrap().len(), 2);
    }

    #[test]
    fn clouds_now_bucket_tracks_only_moving_or_seasonal_fields() {
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Biomes, true, true, None, 1.9),
            Some(0)
        );
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Biomes, true, true, None, 2.0),
            Some(1)
        );
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Biomes, true, true, Some((0.5, 0.0)), 60.0),
            None
        );
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Biomes, true, false, None, 60.0),
            None
        );
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Temperature, false, false, None, 60.0),
            Some(1)
        );
        // the elevation base is season-independent: no periodic rebuild
        // unless live clouds are composited over it
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Elevation, false, true, None, 60.0),
            None
        );
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Elevation, true, true, None, 1.9),
            Some(0)
        );
    }

    #[test]
    fn contour_interval_snaps_nice_and_rejects_flat_views() {
        // nothing to contour: all-sea view (range is -inf), flat land, junk
        assert_eq!(contour_interval_km(f64::NEG_INFINITY), 0.0);
        assert_eq!(contour_interval_km(0.0), 0.0);
        assert_eq!(contour_interval_km(f64::NAN), 0.0);
        // planet-scale relief (~4.5 km in view) → 500 m minors / 2 km index
        assert!((contour_interval_km(4.5) - 0.5).abs() < 1e-12);
        // a lowland valley with 300 m of relief tightens to 20 m lines
        assert!((contour_interval_km(0.3) - 0.02).abs() < 1e-12);
        // micro-relief never collapses below the 2 m raster-noise floor
        assert!(contour_interval_km(1e-4) >= 0.002);
        // zooming out (larger range) never tightens the interval
        let mut last = 0.0f64;
        for range in [0.01, 0.1, 0.5, 1.0, 2.0, 4.0, 8.0] {
            let step = contour_interval_km(range);
            assert!(step >= last, "interval shrank at range {range}: {step} < {last}");
            last = step;
        }
    }

    #[test]
    fn elevation_layer_is_deterministic_with_adaptive_contours() {
        let planet = crate::planet::elevation_test_planet(42);
        let bounds = Bounds {
            lat_top: 45.0,
            lat_bot: -45.0,
            lon_left: -90.0,
            lon_right: 90.0,
        };
        // the fixture ramp is steep (7 km across each face), so sample at a
        // resolution where 0.2 km bands are several pixels wide and contour
        // strokes stay visibly sparse
        let (w, h) = (320usize, 160usize);
        // same bounds + layer → identical pixels (house determinism rule)
        let a = synth_elevation_map(&planet, bounds, w, h);
        let b = synth_elevation_map(&planet, bounds, w, h);
        assert_eq!(a.pixels, b.pixels);

        // the interactive path (synth_map with the Elevation base, relief
        // flag irrelevant, clouds off) must match the headless twin exactly
        let env = MapEnv {
            planet: &planet,
            weather_field: None,
            synoptic_raster: None,
            weather_tuning: &crate::weather::WeatherTuning::default(),
            solar_tuning: &crate::orbits::SolarTuning::default(),
            weather_time_s: 0.0,
            day_len_s: 1200.0,
            weather_on: false,
            weather_pin: None,
            cur_lat: 0.0,
            cur_lon: 0.0,
            cur_moon_lat: 0.0,
            cur_moon_lon: 0.0,
            cur_body: MapBody::Neisor,
            time_scale: 1.0,
            multiplayer_available: false,
            multiplayer_connected: false,
            multiplayer_status: "",
        };
        let via_map = synth_map(&env, BaseLayer::Elevation, true, false, bounds, w, h);
        assert_eq!(via_map.pixels, a.pixels);

        // the fixture's visible land runs 0..~3.9 km, so the adaptive
        // interval snaps to 200 m minors (index lines every 800 m)
        let field = ElevationField::sample(&planet, bounds, w, h);
        assert!(
            (field.interval_km - 0.2).abs() < 1e-9,
            "interval {} for the ~3.9 km fixture",
            field.interval_km
        );

        // classify pixels off the field, away from contour strokes: a pixel
        // is "plain" when its +x/+y neighbors share its band and sea-ness
        let idx = |x: usize, y: usize| y * w + x;
        let plain = |x: usize, y: usize| {
            [(x + 1, y), (x, y + 1)].iter().all(|&(nx, ny)| {
                nx >= w
                    || ny >= h
                    || (field.sea[idx(nx, ny)] == field.sea[idx(x, y)]
                        && field.level(idx(nx, ny)) == field.level(idx(x, y)))
            })
        };
        let (mut sea_px, mut low_px, mut high_px, mut strokes) = (None, None, None, 0usize);
        for y in 0..h {
            for x in 0..w {
                let i = idx(x, y);
                if field.sea[i] {
                    if plain(x, y) {
                        sea_px.get_or_insert(i);
                    }
                    continue;
                }
                if !plain(x, y) {
                    strokes += 1;
                    continue;
                }
                let e = field.elev_km[i];
                if (0.05..0.4).contains(&e) {
                    low_px.get_or_insert(i);
                }
                // track the tallest un-stroked land pixel as "the peak"
                if high_px.is_none_or(|j: usize| field.elev_km[j] < e) {
                    high_px = Some(i);
                }
            }
        }
        // hypsometric ordering: sea reads blue, lowland green, peaks pale
        let rgb = |i: usize| {
            let [r, g, b, _] = a.pixels[i].to_array();
            (r as i32, g as i32, b as i32)
        };
        let (sr, _sg, sb) = rgb(sea_px.expect("fixture shows sea"));
        assert!(sb > sr + 20, "sea should read blue: {:?}", rgb(sea_px.unwrap()));
        let (lr, lg, lb) = rgb(low_px.expect("fixture shows lowland"));
        assert!(lg > lr && lg > lb, "lowland should read green: {lr},{lg},{lb}");
        let (hr, hg, hb) = rgb(high_px.expect("fixture shows peaks"));
        assert!(
            hr + hg + hb > lr + lg + lb + 90,
            "peaks should read paler than lowlands: {hr},{hg},{hb} vs {lr},{lg},{lb}"
        );
        // contours exist and stay lines, not fills (the count includes the
        // coastline band `plain` also rejects, so the bound is generous)
        let land_total = field.sea.iter().filter(|s| !**s).count();
        assert!(strokes > 50, "expected contour strokes, got {strokes}");
        assert!(
            strokes < land_total / 3,
            "contours should be sparse strokes: {strokes} of {land_total} land px"
        );
        // and a stroked pixel is genuinely darker than its own plain tint
        let (cx, cy) = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .find(|&(x, y)| {
                let i = idx(x, y);
                !field.sea[i]
                    && !plain(x, y)
                    && x + 1 < w
                    && !field.sea[i + 1]
                    && field.level(i) != field.level(i + 1)
            })
            .expect("a horizontal band crossing exists");
        let stops = hypso_stops();
        let stroked = srgb8(field.color(cx, cy, &stops));
        let plain_tint = srgb8(ramp(
            &stops,
            (field.elev_km[idx(cx, cy)] as f64 / field.hypso_max_km).clamp(0.0, 1.0) as f32,
        ));
        let lum = |c: Color32| {
            let [r, g, b, _] = c.to_array();
            r as u32 + g as u32 + b as u32
        };
        assert!(
            lum(stroked) + 30 < lum(plain_tint),
            "contour stroke should darken the tint: {stroked:?} vs {plain_tint:?}"
        );
        assert_eq!(a.pixels[idx(cx, cy)], stroked, "image uses the stroked color");
    }

    /// Renders the Elevation base over the tallest terrain on the baked
    /// planet and saves evidence for human eyeballing. Run once with:
    /// `cargo test --release --lib elevation_map_sample_png -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn elevation_map_sample_png() {
        let assets = if std::path::Path::new("assets/meta.json").exists() {
            "assets"
        } else {
            "viewer/assets"
        };
        let planet = Planet::load(assets).expect("baked assets present");
        // center the window on the planet's highest raster texel — the most
        // mountainous view the map can offer
        let mut best = (0usize, 0usize, 0usize, f32::MIN);
        for (fi, face) in planet.faces.iter().enumerate() {
            for (i, &e) in face.elev_km.iter().enumerate() {
                if e > best.3 {
                    best = (fi, i % face.res, i / face.res, e);
                }
            }
        }
        let d = (planet.faces[best.0].res - 1) as f64;
        let u = -1.0 + 2.0 * best.1 as f64 / d;
        let v = -1.0 + 2.0 * best.2 as f64 / d;
        let (lat, lon) = dir_to_geo(crate::planet::face_dir(best.0, u, v));
        let zoom = 20.0;
        let t0 = std::time::Instant::now();
        let img = elevation_map_image(&planet, lat, lon, zoom, 1280, 640);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let dir = if std::path::Path::new("interchange").is_dir() {
            "interchange"
        } else {
            "viewer/interchange"
        };
        let save = |img: &egui::ColorImage, path: &str| {
            let mut bytes = Vec::with_capacity(img.pixels.len() * 4);
            for p in &img.pixels {
                bytes.extend_from_slice(&p.to_array());
            }
            crate::renderer::Renderer::write_png(
                path,
                img.size[0] as u32,
                img.size[1] as u32,
                &bytes,
            )
            .expect("png written");
        };
        let path = format!("{dir}/elevation-map-sample.png");
        save(&img, &path);
        eprintln!(
            "elevation map sample: peak {:.2} km at lat {lat:.3} lon {lon:.3}, zoom {zoom}, \
             synth {ms:.1} ms -> {path}",
            best.3
        );
        // the default-zoom view too, to eyeball the planet-scale interval
        let t0 = std::time::Instant::now();
        let overview = elevation_map_image(&planet, 0.0, 0.0, 1.0, 1280, 640);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let opath = format!("{dir}/elevation-map-overview.png");
        save(&overview, &opath);
        eprintln!("elevation map overview: zoom 1, synth {ms:.1} ms -> {opath}");
    }
}

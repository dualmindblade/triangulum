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

use crate::planet::{face_from_dir, ground_tint, Planet};

/// Deepest zoom the pan/zoom view allows (1 = whole planet). At 180x the map
/// frames ~2 deg of longitude (~300 km on Neisor) across the widget — fine
/// enough to trace a creek; the base rasters are ~10 km/texel near a face
/// center, so terrain reads smooth-but-soft there while the vector rivers and
/// lakes (exact geometry) stay razor sharp.
const MAX_ZOOM: f64 = 180.0;

// ------------------------------------------------------------- photo index

/// One screenshot with a known pose. `day_time_s` comes from the sidecar
/// (present on every shot taken after 2026-07-08); older filename-only
/// shots still get position/view from the name.
pub struct Photo {
    pub path: PathBuf,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub alt_km: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
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
            if !c.is_finite() || !r.is_finite() || !(0.0..=1.0).contains(&c)
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
        let mut photo: Option<Photo> = None;
        if let Ok(raw) = std::fs::read_to_string(&sidecar)
            && let Ok(js) = serde_json::from_str::<serde_json::Value>(&raw)
        {
            let f = |k: &str| js.get(k).and_then(|v| v.as_f64());
            if let (Some(lat), Some(lon)) = (f("lat_deg"), f("lon_deg")) {
                let weather = sidecar_weather(&js);
                photo = Some(Photo {
                    path: path.clone(),
                    name: name.clone(),
                    lat,
                    lon,
                    alt_km: f("alt_km").unwrap_or(0.3),
                    yaw_deg: f("yaw_deg").unwrap_or(0.0),
                    pitch_deg: f("pitch_deg").unwrap_or(-20.0),
                    day_time_s: f("day_cycle_time_s"),
                    weather_on: weather.map(|w| w.0),
                    weather_pin: weather.and_then(|w| w.1),
                    weather_time_s: weather.and_then(|w| w.2),
                    ground_km: f("ground_km"),
                    mode: js.get("mode").and_then(|v| v.as_str()).map(str::to_owned),
                    day_len_s: f("day_len_s").filter(|v| *v > 0.0),
                    seed: js.get("seed").and_then(|v| v.as_i64()),
                });
            }
        }
        if photo.is_none()
            && let Some((lat, lon, alt, yaw, pitch)) = parse_filename(&name)
        {
            photo = Some(Photo {
                path: path.clone(),
                name: name.clone(),
                lat,
                lon,
                alt_km: alt,
                yaw_deg: yaw,
                pitch_deg: pitch,
                day_time_s: None,
                weather_on: None,
                weather_pin: None,
                weather_time_s: None,
                ground_km: None,
                mode: None,
                day_len_s: None,
                seed: None,
            });
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
}

/// Everything the map needs from the app besides the photo roll: the planet
/// rasters + rivers, the loaded weather climatology and its tuning, the
/// current weather time (so temp/precip sample the CURRENT season and clouds
/// show the CURRENT synoptic field), and the player's position for the "you
/// are here" marker. All borrowed — the map owns none of it.
pub struct MapEnv<'a> {
    pub planet: &'a Planet,
    pub weather_field: Option<&'a crate::weather::WeatherField>,
    pub weather_tuning: &'a crate::weather::WeatherTuning,
    pub weather_time_s: f64,
    pub day_len_s: f64,
    pub weather_on: bool,
    pub weather_pin: Option<(f32, f32)>,
    pub cur_lat: f64,
    pub cur_lon: f64,
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
    base: BaseLayer,
    relief: bool,
    clouds: bool,
    weather_on: bool,
    weather_pin: Option<(f32, f32)>,
    /// 60-second buckets refresh live fronts at a useful visual cadence while
    /// avoiding a costly map synthesis every frame.
    weather_time_bucket: Option<i64>,
    bounds: Bounds,
}

const MAP_WEATHER_BUCKET_S: f64 = 60.0;

fn map_weather_time_bucket(
    base: BaseLayer,
    clouds: bool,
    weather_on: bool,
    weather_pin: Option<(f32, f32)>,
    weather_time_s: f64,
) -> Option<i64> {
    let time_sensitive = (clouds && weather_on && weather_pin.is_none())
        || base != BaseLayer::Biomes;
    time_sensitive.then(|| {
        let t_s = if weather_time_s.is_finite() { weather_time_s } else { 0.0 };
        (t_s / MAP_WEATHER_BUCKET_S).floor() as i64
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

/// Biome base color at a cell — the legacy koppen tint (ocean by depth, land
/// by class), optionally shaded by elevation + snow (the relief layer). With
/// `relief` on this reproduces the original `build_minimap` land/sea look.
fn biome_color(planet: &Planet, f: usize, u: f64, v: f64, relief: bool) -> [f32; 3] {
    let e = planet.elevation(f, u, v) as f64;
    let climate = planet.biome_climate(f, u, v);
    let k = climate.koppen;
    if k == 255 {
        // sea: deep navy to shelf teal (bathymetry, kept regardless of relief)
        let d = (-e / 4.0).clamp(0.0, 1.0) as f32;
        [
            0.10 + (0.02 - 0.10) * d,
            0.32 + (0.08 - 0.32) * d,
            0.42 + (0.22 - 0.42) * d,
        ]
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

/// Rasterize the base color field for `b` at `w`x`h`. Rivers/lakes/markers are
/// NOT drawn here (they are vector overlays); this is the biome / temperature
/// / precipitation field, optionally relief-shaded, with the live cloud field
/// alpha-composited on top. Cost is dominated by the cloud layer (per-pixel
/// synoptic fbm via `weather_at`) — timed at the call site.
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
    let season = crate::weather::season_frac(
        env.weather_time_s,
        env.day_len_s,
        env.weather_tuning,
    );
    let angle = std::f64::consts::TAU * season;
    let (sn1, cs1) = angle.sin_cos();
    let (sn2, cs2) = (2.0 * angle).sin_cos();
    let (tstops, pstops) = (temp_stops(), precip_stops());
    let mut px = vec![Color32::BLACK; w * h];
    for y in 0..h {
        let lat =
            (b.lat_top + (b.lat_bot - b.lat_top) * (y as f64 + 0.5) / h as f64).to_radians();
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
                        Some(wf) => wf.climate_sample(
                            planet, f, u, v, cs1, sn1, cs2, sn2,
                        ).0,
                        None => planet.temp(f, u, v) as f64,
                    };
                    temp_color(t, &tstops)
                }
                BaseLayer::Precipitation => {
                    let p = match env.weather_field {
                        Some(wf) => wf.climate_sample(
                            planet, f, u, v, cs1, sn1, cs2, sn2,
                        ).1,
                        None => planet.precip(f, u, v) as f64 / 12.0,
                    };
                    precip_color(p, &pstops)
                }
            };
            // relief on the weather bases: a gentle hypsometric brightening of
            // land only (biome relief is folded into biome_color above).
            if relief && base != BaseLayer::Biomes && planet.water_frac(f, u, v) < 0.5 {
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
                let cover = env.weather_pin.map_or_else(
                    || {
                        crate::weather::weather_at(
                            wf,
                            planet,
                            dir,
                            env.weather_time_s,
                            env.day_len_s,
                            env.weather_tuning,
                        )
                        .cloud_cover as f32
                    },
                    |(cover, _)| cover,
                );
                if cover > 0.01 {
                    let a = cover.powf(1.1) * 0.9;
                    let cl = disp(0.95 - 0.42 * cover, 0.95 - 0.40 * cover, 0.97 - 0.36 * cover);
                    c = [
                        c[0] * (1.0 - a) + cl[0] * a,
                        c[1] * (1.0 - a) + cl[1] * a,
                        c[2] * (1.0 - a) + cl[2] * a,
                    ];
                }
            }
            px[y * w + x] = Color32::from_rgb(
                (c[0].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
                (c[1].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
                (c[2].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8,
            );
        }
    }
    egui::ColorImage { size: [w, h], source_size: egui::Vec2::new(w as f32, h as f32), pixels: px }
}

// ------------------------------------------------------------- popup state

/// What the popup asks the app to do when the player commits.
pub struct TeleportAction {
    pub lat: f64,
    pub lon: f64,
    pub alt_km: Option<f64>,
    pub yaw_deg: Option<f64>,
    pub pitch_deg: Option<f64>,
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
    selected: Option<usize>,
    checked: HashSet<usize>,
    custom_dest: Option<(f64, f64)>,
    confirm_delete: bool,
    restore_time: bool,
    coord_input: String,
    scroll_to_selected: bool,
    status: String,
    // ---- layer toggles (persist across opens) ----
    base_layer: BaseLayer,
    show_relief: bool,
    show_rivers: bool,
    show_lakes: bool,
    show_clouds: bool,
    show_markers: bool,
    // ---- pan/zoom view (equirectangular); zoom 1 = whole planet ----
    view_zoom: f64,
    view_center_lat: f64,
    view_center_lon: f64,
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
            selected: None,
            checked: HashSet::new(),
            custom_dest: None,
            confirm_delete: false,
            restore_time: false,
            coord_input: String::new(),
            scroll_to_selected: false,
            status: String::new(),
            base_layer: BaseLayer::Biomes,
            show_relief: true,
            show_rivers: true,
            show_lakes: true,
            show_clouds: false,
            show_markers: true,
            view_zoom: 1.0,
            view_center_lat: 0.0,
            view_center_lon: 0.0,
        }
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.photos = scan_photos(&self.interchange);
            self.selected = None;
            self.preview = None;
            self.checked.clear();
            self.custom_dest = None;
            self.confirm_delete = false;
            // fresh orientation each open: whole planet, and force a re-synth
            // so temperature/precip pick up the current season and clouds the
            // current synoptic field.
            self.reset_view();
            self.map_built = None;
            self.status = format!("{} photos", self.photos.len());
        }
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
        let Some(photo) = self.photos.get(idx) else { return };
        let Ok(raw) = std::fs::read(&photo.path) else { return };
        let Ok(mut dec) = png_dims_and_rgba(&raw) else { return };
        // downscale to <=560 px wide for the preview texture
        let max_w = 560usize;
        if dec.0[0] > max_w {
            let step = dec.0[0].div_ceil(max_w);
            let (nw, nh) = (dec.0[0] / step, dec.0[1] / step);
            let mut small = Vec::with_capacity(nw * nh);
            for y in 0..nh {
                for x in 0..nw {
                    small.push(dec.1[y * step * dec.0[0] + x * step]);
                }
            }
            dec = ([nw, nh], small);
        }
        let img = egui::ColorImage {
            size: dec.0,
            source_size: egui::Vec2::new(dec.0[0] as f32, dec.0[1] as f32),
            pixels: dec.1,
        };
        let tex = ctx.load_texture(format!("preview{idx}"), img, Default::default());
        self.preview = Some((idx, tex));
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
        egui::Window::new("Photo map — teleport")
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(screen.width() * 0.86, screen.height() * 0.84))
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label("Esc closes · scroll = zoom · drag = pan · double-click = reset · click = destination");
                    });
                });
                // ---- layer controls ----
                ui.horizontal_wrapped(|ui| {
                    ui.label("Base:");
                    ui.radio_value(&mut self.base_layer, BaseLayer::Biomes, "Biomes");
                    ui.radio_value(&mut self.base_layer, BaseLayer::Temperature, "Temp");
                    ui.radio_value(&mut self.base_layer, BaseLayer::Precipitation, "Precip");
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
                    ui.checkbox(&mut self.show_markers, "Markers")
                        .on_hover_text("photo markers");
                    ui.separator();
                    if ui.button("Reset view").clicked() {
                        self.reset_view();
                    }
                    ui.label(format!("{:.0}×", self.view_zoom));
                });
                ui.separator();
                let list_w = 340.0;
                ui.horizontal_top(|ui| {
                    // ---------------- left: map + preview ----------------
                    ui.vertical(|ui| {
                        let avail = ui.available_width() - list_w;
                        let map_w = avail.max(320.0);
                        let map_h = map_w * 0.5;
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(map_w, map_h),
                            egui::Sense::click_and_drag(),
                        );

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
                            self.reset_view();
                            did_reset = true;
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
                        // resolution cap keeps every re-synth well under ~200 ms
                        // (measured; see the commit message). Cost per pixel:
                        // clouds carry a synoptic fbm (dear), the temp/precip
                        // climatology two harmonic bilinears (~2x biomes), and
                        // biomes are cheapest — so the crisp cap tracks the base.
                        // Below the cap the map is native; above it LINEAR-
                        // upscales (soft, but clouds/temp are soft fields).
                        let (cap_w, cap_h) = if self.show_clouds {
                            (512, 256)
                        } else if self.base_layer == BaseLayer::Biomes {
                            (1280, 640)
                        } else {
                            (960, 480)
                        };
                        let tw = ((map_w * ppp).round() as usize).clamp(64, cap_w);
                        let th = ((map_h * ppp).round() as usize).clamp(32, cap_h);
                        let weather_time_bucket = map_weather_time_bucket(
                            self.base_layer,
                            self.show_clouds,
                            env.weather_on,
                            env.weather_pin,
                            env.weather_time_s,
                        );
                        let sig = MapSig {
                            w: tw,
                            h: th,
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
                            let img = synth_map(
                                env,
                                self.base_layer,
                                self.show_relief,
                                self.show_clouds,
                                bounds,
                                tw,
                                th,
                            );
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
                        if self.show_lakes {
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
                                // -4 C, matching every WORLD consumer (walkable-ice class in
                                // terrain + voxel + physics): 0 C painted 1,700+ liquid
                                // lakes pale on the map (review #2 finding 11)
                                let frozen = planet.temp(f, u, v) < -4.0;
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
                        if self.show_rivers {
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

                        // markers, "you are here", custom destination (top)
                        let in_rect = |p: egui::Pos2| rect.expand(2.0).contains(p);
                        if self.show_markers {
                            for (i, p) in self.photos.iter().enumerate() {
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
                        {
                            let pos = bounds.project(rect, env.cur_lat, env.cur_lon);
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
                                if bounds.project(rect, p.lat, p.lon).distance(hp) < 10.0 {
                                    let font = egui::FontId::proportional(12.0);
                                    let galley = ui.painter().layout_no_wrap(
                                        p.name.clone(),
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
                        // preview of the selected photo
                        if let Some(sel) = self.selected {
                            self.load_preview(ui.ctx(), sel);
                            if let Some((pi, tex)) = &self.preview
                                && *pi == sel
                            {
                                let size = tex.size_vec2();
                                let scale = (map_w / size.x).min(220.0 / size.y).min(1.0);
                                ui.add_space(6.0);
                                ui.image((tex.id(), size * scale));
                            }
                        }
                    });
                    ui.separator();
                    // ---------------- right: the photo list ----------------
                    ui.vertical(|ui| {
                        ui.set_width(list_w);
                        ui.horizontal(|ui| {
                            if ui.button("Select all").clicked() {
                                self.checked = (0..self.photos.len()).collect();
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
                        egui::ScrollArea::vertical().max_height(
                            ui.available_height() - 96.0,
                        ).show_rows(
                            ui,
                            row_h,
                            self.photos.len(),
                            |ui, range| {
                                for i in range {
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
                                        let label = format!(
                                            "{:.3} {:.3}  alt {:.0} m{}",
                                            p.lat,
                                            p.lon,
                                            p.alt_km * 1000.0,
                                            if p.day_time_s.is_some() { "  ⏱" } else { "" },
                                        );
                                        let r = ui.selectable_label(sel, label);
                                        if sel && self.scroll_to_selected {
                                            r.scroll_to_me(Some(egui::Align::Center));
                                        }
                                        if r.clicked() {
                                            self.selected = Some(i);
                                            self.custom_dest = None;
                                        }
                                        r.on_hover_text(&p.name);
                                    });
                                }
                                self.scroll_to_selected = false;
                            },
                        );
                        ui.separator();
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
                lat: p.lat,
                lon: p.lon,
                alt_km: Some(p.alt_km.max(0.0025)),
                yaw_deg: Some(p.yaw_deg),
                pitch_deg: Some(p.pitch_deg),
                day_time_s: if self.restore_time { p.day_time_s } else { None },
                day_len_s: if self.restore_time { p.day_len_s } else { None },
                weather_on: if self.restore_time { p.weather_on } else { None },
                weather_pin: if self.restore_time { p.weather_pin } else { None },
                weather_time_s: if self.restore_time { p.weather_time_s } else { None },
                ground_km: p.ground_km,
                walk: p.mode.as_deref() == Some("walk"),
                seed: p.seed,
            });
        }
        if let Some((lat, lon)) = self.custom_dest {
            return Some(TeleportAction {
                lat,
                lon,
                alt_km: None,
                yaw_deg: None,
                pitch_deg: None,
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
                    lat: parts[0],
                    lon: parts[1],
                    alt_km: parts.get(2).copied().filter(|a| *a > 0.0),
                    yaw_deg: None,
                    pitch_deg: None,
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
            egui::ImageData::Color(img) => {
                img.pixels.iter().flat_map(|c| c.to_array()).collect()
            }
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
            let Primitive::Mesh(mesh) = &cp.primitive else { continue };
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
        let mut enc =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
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
                let Some((_, bind)) = self.textures.get(&tex_id) else { continue };
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
            lat: 1.0,
            lon: 2.0,
            alt_km: 0.1,
            yaw_deg: 3.0,
            pitch_deg: -4.0,
            day_time_s: Some(600.0),
            weather_on: Some(true),
            weather_pin: Some((0.97, 0.9)),
            weather_time_s: Some(24_000.0),
            ground_km: Some(0.2),
            mode: Some("fly".into()),
            day_len_s: Some(1200.0),
            seed: Some(42),
        });
        map.selected = Some(0);

        let normal = map.destination().unwrap();
        assert_eq!(normal.day_time_s, None);
        assert_eq!(normal.weather_on, None);
        assert_eq!(normal.weather_pin, None);
        assert_eq!(normal.weather_time_s, None);

        map.restore_time = true;
        let restored = map.destination().unwrap();
        assert_eq!(restored.day_time_s, Some(600.0));
        assert_eq!(restored.weather_on, Some(true));
        assert_eq!(restored.weather_pin, Some((0.97, 0.9)));
        assert_eq!(restored.weather_time_s, Some(24_000.0));
    }

    #[test]
    fn clouds_now_bucket_tracks_only_moving_or_seasonal_fields() {
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Biomes, true, true, None, 59.9),
            Some(0)
        );
        assert_eq!(
            map_weather_time_bucket(BaseLayer::Biomes, true, true, None, 60.0),
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
    }
}

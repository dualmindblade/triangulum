//! Andrew's look (M3a): every render dial of the look panel, in plain
//! units, read from `game/assets/look.json` at startup and written back by
//! the panel's "Save look". The renderer turns it into its sky uniform
//! (`render/sky.rs`); nothing here touches the world, the generator or a
//! save. Block colours live in `blocks.json` and terrain in `world.json`.
//!
//! Defaults are the M1 look: July's constants, with the optical lengths
//! scaled to the planet as `world.json`'s `sky` object says, so a missing
//! field (or "Reset" in the panel) means "as it was before the panel".

use crate::sky::Settings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

/// The comment written at the top of look.json.
pub const COMMENT: &str = "Andrew's look: the render dials of the look panel (F1 in the game). Edit it here or in the panel; the game reloads it as soon as it changes. Colours are linear RGB 0..1. Block colours are in blocks.json, terrain in world.json. Missing fields take the default (the M1 look).";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Look {
    #[serde(rename = "_comment", default)]
    pub comment: String,
    pub sky: SkyLook,
    pub light: LightLook,
    pub water: WaterLook,
    pub detail: DetailLook,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkyLook {
    /// Scale height of the air (m): the sky has faded to about a third of
    /// its brightness this high up, and to space a few times higher.
    pub atmosphere_height_m: f64,
    /// Fog per kilometre of view (things fade out over about 1 / this km).
    pub fog_density_per_km: f64,
    /// How much of a distant view the fog may cover, at most (0..1).
    pub fog_strength: f64,
    /// The sky at the horizon, and the fog's colour.
    pub horizon_colour: [f32; 3],
    /// The sky straight up.
    pub zenith_colour: [f32; 3],
    /// The glow around a low sun.
    pub sunset_colour: [f32; 3],
    pub sunset_strength: f64,
    /// How bright the empty sky stays at night (fraction of the day sky).
    pub night_sky: f64,
    pub star_brightness: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightLook {
    /// Direct sunlight on faces turned toward the sun.
    pub sun_strength: f64,
    /// Light from the blue sky on everything facing up, by day.
    pub sky_ambient: f64,
    /// Light everywhere, day and night, even in shadow.
    pub shadow_light: f64,
    /// Extra light at night only.
    pub night_light: f64,
    /// Moonlight (1 = July's).
    pub moonlight: f64,
    pub moon_colour: [f32; 3],
    /// Brightness of side faces and of undersides, relative to tops.
    pub side_shade: f64,
    pub underside_shade: f64,
    /// Darkening in corners and creases (0 = none).
    pub corner_shadows: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaterLook {
    /// Opacity looking straight down (0 clear .. 1 opaque).
    pub opacity: f64,
    /// Opacity looking along the surface.
    pub edge_opacity: f64,
    /// The sun's sparkle on the surface, and how tight it is.
    pub glint: f64,
    pub glint_sharpness: f64,
    /// The tint with your eyes under water, and how strong.
    pub underwater_colour: [f32; 3],
    pub underwater_strength: f64,
    /// Keep the far ocean (the opaque `deep_ocean` block seen from high
    /// up) in step with the water colour and opacity.
    pub far_ocean_follows_water: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailLook {
    /// LOD factor: a chunk splits when the camera is closer than this
    /// many chunk widths (higher = finer distant detail, more work).
    pub lod_factor: f64,
}

impl Look {
    /// The M1 look on a planet of `radius` metres with these sky settings.
    pub fn defaults(settings: &Settings, radius: f64) -> Look {
        Look {
            comment: COMMENT.into(),
            sky: SkyLook {
                atmosphere_height_m: settings.atmosphere_height_radii * radius,
                fog_density_per_km: settings.fog_per_radius / radius * 1000.0,
                fog_strength: 0.7,
                horizon_colour: [0.55, 0.70, 0.88],
                zenith_colour: [0.10, 0.28, 0.62],
                sunset_colour: [0.95, 0.55, 0.30],
                sunset_strength: 0.55,
                night_sky: 0.03,
                star_brightness: 1.0,
            },
            light: LightLook {
                sun_strength: 1.0,
                sky_ambient: 0.40,
                shadow_light: 0.10,
                night_light: 0.0,
                moonlight: 1.0,
                moon_colour: [0.40, 0.50, 0.72],
                side_shade: 0.90,
                underside_shade: 0.55,
                corner_shadows: 0.45,
            },
            water: WaterLook {
                opacity: 0.62,
                edge_opacity: 0.95,
                glint: 0.6,
                glint_sharpness: 60.0,
                underwater_colour: [0.02, 0.07, 0.16],
                underwater_strength: 0.6,
                far_ocean_follows_water: true,
            },
            detail: DetailLook { lod_factor: 5.0 },
        }
    }

    /// Parse a look file over `defaults`: fields the file leaves out keep
    /// their default; unknown fields and bad values are refused.
    pub fn parse(text: &str, defaults: &Look) -> Result<Look, String> {
        let file: Value = serde_json::from_str(text).map_err(|e| format!("parse: {e}"))?;
        let mut v = serde_json::to_value(defaults).map_err(|e| e.to_string())?;
        overlay(&mut v, file);
        let look: Look = serde_json::from_value(v).map_err(|e| e.to_string())?;
        look.validate()?;
        Ok(look)
    }

    pub fn load(path: &Path, defaults: &Look) -> Result<Look, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::parse(&text, defaults).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The file's text: the tidied look, pretty-printed, with the file's
    /// line ending (`nl`).
    pub fn to_text(&self, nl: &str) -> String {
        let mut t = self.tidy();
        t.comment = COMMENT.into();
        let text = collapse_number_lists(&serde_json::to_string_pretty(&t).expect("look serialises")) + "\n";
        if nl == "\n" { text } else { text.replace('\n', nl) }
    }

    /// Write the look to `path` (beside it first, then renamed over it),
    /// keeping the existing file's line ending.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let nl = match std::fs::read_to_string(path) {
            Ok(old) if !old.contains("\r\n") => "\n",
            Ok(_) => "\r\n",
            Err(_) => if cfg!(windows) { "\r\n" } else { "\n" },
        };
        let tmp = path.with_extension("json.saving");
        std::fs::write(&tmp, self.to_text(nl)).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
    }

    /// Values rounded for the file: four significant figures, colours to
    /// four decimals (the panel's sliders leave long tails).
    pub fn tidy(&self) -> Look {
        let n = |x: f64| round_sig(x, 4);
        let c = |v: [f32; 3]| v.map(|x| ((x as f64 * 1e4).round() / 1e4) as f32);
        let mut t = self.clone();
        let s = &mut t.sky;
        s.atmosphere_height_m = n(s.atmosphere_height_m);
        s.fog_density_per_km = n(s.fog_density_per_km);
        s.fog_strength = n(s.fog_strength);
        s.horizon_colour = c(s.horizon_colour);
        s.zenith_colour = c(s.zenith_colour);
        s.sunset_colour = c(s.sunset_colour);
        s.sunset_strength = n(s.sunset_strength);
        s.night_sky = n(s.night_sky);
        s.star_brightness = n(s.star_brightness);
        let l = &mut t.light;
        l.sun_strength = n(l.sun_strength);
        l.sky_ambient = n(l.sky_ambient);
        l.shadow_light = n(l.shadow_light);
        l.night_light = n(l.night_light);
        l.moonlight = n(l.moonlight);
        l.moon_colour = c(l.moon_colour);
        l.side_shade = n(l.side_shade);
        l.underside_shade = n(l.underside_shade);
        l.corner_shadows = n(l.corner_shadows);
        let w = &mut t.water;
        w.opacity = n(w.opacity);
        w.edge_opacity = n(w.edge_opacity);
        w.glint = n(w.glint);
        w.glint_sharpness = n(w.glint_sharpness);
        w.underwater_colour = c(w.underwater_colour);
        w.underwater_strength = n(w.underwater_strength);
        t.detail.lod_factor = n(t.detail.lod_factor);
        t
    }

    pub fn validate(&self) -> Result<(), String> {
        let s = &self.sky;
        let l = &self.light;
        let w = &self.water;
        let numbers = [
            ("sky.atmosphere_height_m", s.atmosphere_height_m),
            ("sky.fog_density_per_km", s.fog_density_per_km),
            ("sky.fog_strength", s.fog_strength),
            ("sky.sunset_strength", s.sunset_strength),
            ("sky.night_sky", s.night_sky),
            ("sky.star_brightness", s.star_brightness),
            ("light.sun_strength", l.sun_strength),
            ("light.sky_ambient", l.sky_ambient),
            ("light.shadow_light", l.shadow_light),
            ("light.night_light", l.night_light),
            ("light.moonlight", l.moonlight),
            ("light.side_shade", l.side_shade),
            ("light.underside_shade", l.underside_shade),
            ("light.corner_shadows", l.corner_shadows),
            ("water.opacity", w.opacity),
            ("water.edge_opacity", w.edge_opacity),
            ("water.glint", w.glint),
            ("water.glint_sharpness", w.glint_sharpness),
            ("water.underwater_strength", w.underwater_strength),
            ("detail.lod_factor", self.detail.lod_factor),
        ];
        for (name, v) in numbers {
            if !v.is_finite() || v < 0.0 {
                return Err(format!("{name} must be a number of at least 0, not {v}"));
            }
        }
        for (name, c) in [("sky.horizon_colour", s.horizon_colour), ("sky.zenith_colour", s.zenith_colour), ("sky.sunset_colour", s.sunset_colour), ("light.moon_colour", l.moon_colour), ("water.underwater_colour", w.underwater_colour)] {
            if c.iter().any(|x| !x.is_finite() || *x < 0.0) {
                return Err(format!("{name} must be three numbers of at least 0"));
            }
        }
        if s.atmosphere_height_m < 1.0 {
            return Err("sky.atmosphere_height_m must be at least 1 m".into());
        }
        if !(0.2..=100.0).contains(&self.detail.lod_factor) {
            return Err("detail.lod_factor must be between 0.2 and 100".into());
        }
        for (name, v) in [("sky.fog_strength", s.fog_strength), ("water.opacity", w.opacity), ("water.edge_opacity", w.edge_opacity), ("water.underwater_strength", w.underwater_strength)] {
            if v > 1.0 {
                return Err(format!("{name} must be between 0 and 1"));
            }
        }
        Ok(())
    }
}

fn round_sig(x: f64, digits: i32) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let scale = 10f64.powi(digits - 1 - x.abs().log10().floor() as i32);
    let r = (x * scale).round() / scale;
    // Through text, so the file shows 639.7 and not 639.7000000000001.
    format!("{r:.*}", (digits - 1 - x.abs().log10().floor() as i32).max(0) as usize).parse().unwrap_or(r)
}

/// Pretty JSON puts every number of a colour on its own line; put a list
/// of numbers on one line, `[0.55, 0.7, 0.88]`, so the file reads by eye.
fn collapse_number_lists(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        match tail.find(']') {
            Some(close) if tail[..close].chars().all(|c| c.is_ascii_digit() || " \n\r\t,.-+eE".contains(c)) => {
                let items: Vec<&str> = tail[..close].split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
                out.push('[');
                out.push_str(&items.join(", "));
                out.push(']');
                rest = &tail[close + 1..];
            }
            _ => {
                out.push('[');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Merge `top` into `base`: objects field by field, anything else replaced.
fn overlay(base: &mut Value, top: Value) {
    match (base, top) {
        (Value::Object(b), Value::Object(t)) => {
            for (k, v) in t {
                match b.get_mut(&k) {
                    Some(slot) => overlay(slot, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

// ------------------------------------------------------------ startup

/// What main hands the renderer and the workshop: the look to start with,
/// its defaults, and the files the panel saves to and watches.
#[derive(Clone, Debug)]
pub struct Startup {
    pub look: Look,
    pub defaults: Look,
    pub look_path: PathBuf,
    pub blocks_path: PathBuf,
    pub world_path: PathBuf,
    /// Open the look panel when the game starts (`--panel`).
    pub panel_open: bool,
}

static STARTUP: OnceLock<Startup> = OnceLock::new();

/// Installed once by main, like `sky::install`, so the renderer's
/// constructors (app, bench, harness, reels) all start from the same look.
pub fn install(startup: Startup) {
    STARTUP.set(startup).expect("look installed twice");
}

pub fn startup() -> Option<&'static Startup> {
    STARTUP.get()
}

/// The look file named on the command line, or the shipped one; missing
/// is fine (the defaults), broken is an error.
pub fn load_startup(path: &Path, defaults: &Look) -> Result<Look, String> {
    if path.exists() { Look::load(path, defaults) } else { Ok(defaults.clone()) }
}

// ------------------------------------------------------------ watching

/// A file the game reloads when it changes on disk: polled (a metadata
/// read every half second costs microseconds), so no watcher thread.
pub struct FileWatch {
    pub path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
}

impl FileWatch {
    pub fn new(path: PathBuf) -> Self {
        let stamp = Self::stamp_of(&path);
        Self { path, stamp }
    }
    fn stamp_of(path: &Path) -> Option<(SystemTime, u64)> {
        let m = std::fs::metadata(path).ok()?;
        Some((m.modified().ok()?, m.len()))
    }
    /// Has the file changed since the last call (or `mark`)? A file that
    /// disappears is not a change.
    pub fn changed(&mut self) -> bool {
        let now = Self::stamp_of(&self.path);
        if now == self.stamp {
            return false;
        }
        self.stamp = now;
        now.is_some()
    }
    /// The game wrote the file itself: take its new stamp as seen.
    pub fn mark(&mut self) {
        self.stamp = Self::stamp_of(&self.path);
    }
}

// ------------------------------------------------------------ world.json

/// The three terrain dials the panel exposes. They change what the
/// generator makes, so they are part of a save's identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Terrain {
    pub roughness: f64,
    pub landform: f64,
    /// Resolved: R / 6,371,000 when world.json says null.
    pub vscale: f64,
}

impl Terrain {
    pub fn same(&self, o: &Terrain) -> bool {
        (self.roughness - o.roughness).abs() < 1e-9 && (self.landform - o.landform).abs() < 1e-9 && (self.vscale - o.vscale).abs() < 1e-9
    }
}

/// Replace the value of `"key": value` in a JSON text, keeping everything
/// else (comments, order, spacing) as it is.
pub fn set_json_value(text: &str, key: &str, value: &str) -> Result<String, String> {
    let pat = format!("\"{key}\"");
    let start = text.find(&pat).ok_or_else(|| format!("no \"{key}\" in the file"))?;
    let after = start + pat.len();
    let colon = after + text[after..].find(':').ok_or_else(|| format!("no value after \"{key}\""))?;
    let vstart = colon + 1 + text[colon + 1..].len() - text[colon + 1..].trim_start().len();
    let vlen = text[vstart..].find([',', '}', '\r', '\n']).unwrap_or(text.len() - vstart);
    let vend = vstart + text[vstart..vstart + vlen].trim_end().len();
    Ok(format!("{}{}{}", &text[..vstart], value, &text[vend..]))
}

/// Write the terrain dials into world.json as the new default. `earth`
/// is the vertical scale that world.json spells `null` (Earth's slopes).
pub fn save_terrain_default(path: &Path, t: &Terrain, earth_vscale: f64) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    // serde_json spells 1.0 as "1.0", as the file does.
    let num = |x: f64, d: i32| serde_json::to_string(&round_sig(x, d)).unwrap_or_else(|_| x.to_string());
    let vs = if (t.vscale - earth_vscale).abs() < 1e-7 { "null".to_string() } else { num(t.vscale, 5) };
    let mut new = set_json_value(&text, "vertical_scale", &vs)?;
    new = set_json_value(&new, "roughness", &num(t.roughness, 4))?;
    new = set_json_value(&new, "landform", &num(t.landform, 4))?;
    let tmp = path.with_extension("json.saving");
    std::fs::write(&tmp, new).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 90_569.6;

    fn defaults() -> Look {
        Look::defaults(&Settings::default(), R)
    }

    /// The JSON round trip: the defaults written and read back are the
    /// tidied defaults; a written file reads back to itself; the shipped
    /// look.json is the M1 look; a partial file fills in the defaults; an
    /// unknown field and a bad value are refused with their names.
    #[test]
    fn look_json_round_trip() {
        let d = defaults();
        let text = d.to_text("\n");
        let back = Look::parse(&text, &d).unwrap();
        assert_eq!(back, d.tidy());
        assert_eq!(back.to_text("\n"), text, "a written file reads back to itself");
        assert_eq!(Look::parse(&d.to_text("\r\n"), &d).unwrap(), back, "CRLF files read the same");
        // The default atmosphere is July's 45 km scaled to Neisor: ~640 m.
        assert!((back.sky.atmosphere_height_m - 639.7).abs() < 0.05, "{}", back.sky.atmosphere_height_m);
        assert!((d.sky.atmosphere_height_m - Settings::default().atmosphere_height_radii * R).abs() < 1e-9);

        let shipped = Look::load(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/look.json")), &d).unwrap();
        assert_eq!(shipped, d.tidy(), "the shipped look.json is the M1 look");

        let partial = Look::parse(r#"{ "sky": { "atmosphere_height_m": 4000 }, "water": { "opacity": 0.4 } }"#, &d).unwrap();
        assert_eq!(partial.sky.atmosphere_height_m, 4000.0);
        assert_eq!(partial.water.opacity, 0.4);
        assert_eq!(partial.sky.zenith_colour, d.sky.zenith_colour);
        assert_eq!(partial.light, d.light);

        let e = Look::parse(r#"{ "sky": { "atmosphere_hieght_m": 4000 } }"#, &d).unwrap_err();
        assert!(e.contains("atmosphere_hieght_m"), "{e}");
        let e = Look::parse(r#"{ "water": { "opacity": 1.5 } }"#, &d).unwrap_err();
        assert!(e.contains("water.opacity"), "{e}");
        let e = Look::parse(r#"{ "light": { "sun_strength": -1 } }"#, &d).unwrap_err();
        assert!(e.contains("light.sun_strength"), "{e}");
    }

    #[test]
    fn tidy_rounds_for_the_file() {
        assert_eq!(round_sig(639.7146253, 4), 639.7);
        assert_eq!(round_sig(0.2462034, 4), 0.2462);
        assert_eq!(round_sig(0.62000001, 4), 0.62);
        assert_eq!(round_sig(60.0, 4), 60.0);
        assert_eq!(round_sig(0.0, 4), 0.0);
        let mut l = defaults();
        l.sky.horizon_colour = [0.123456, 0.5, 1.0];
        assert_eq!(l.tidy().sky.horizon_colour, [0.1235, 0.5, 1.0]);
    }

    #[test]
    fn world_json_terrain_edit_keeps_the_file() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/world.json")).unwrap();
        let edited = set_json_value(&text, "roughness", "1.5").unwrap();
        let edited = set_json_value(&edited, "vertical_scale", "0.125").unwrap();
        let edited = set_json_value(&edited, "landform", "0").unwrap();
        // Only those three values changed.
        let a: Value = serde_json::from_str(&text).unwrap();
        let mut b: Value = serde_json::from_str(&edited).unwrap();
        assert_eq!(b["roughness"], 1.5);
        assert_eq!(b["vertical_scale"], 0.125);
        assert_eq!(b["landform"], 0);
        b["roughness"] = a["roughness"].clone();
        b["vertical_scale"] = a["vertical_scale"].clone();
        b["landform"] = a["landform"].clone();
        assert_eq!(a, b);
        assert_eq!(text.lines().count(), edited.lines().count());
        // And back: null for Earth's slopes.
        let back = set_json_value(&edited, "vertical_scale", "null").unwrap();
        assert!(back.contains("\"vertical_scale\": null,"));
        assert!(set_json_value(&text, "no_such_key", "1").is_err());
    }

    #[test]
    fn file_watch_sees_changes_but_not_its_own_writes() {
        let dir = std::env::temp_dir().join(format!("triangulum-watch-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("a.json");
        std::fs::write(&p, "{}").unwrap();
        let mut w = FileWatch::new(p.clone());
        assert!(!w.changed());
        std::fs::write(&p, "{ \"x\": 1 }").unwrap();
        assert!(w.changed());
        assert!(!w.changed());
        std::fs::write(&p, "{ \"x\": 22 }").unwrap();
        w.mark();
        assert!(!w.changed(), "a write the game made itself is not a change");
        std::fs::remove_file(&p).unwrap();
        assert!(!w.changed(), "a deleted file is not a change");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

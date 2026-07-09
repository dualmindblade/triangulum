//! Living weather (WEATHER.md): Layer 1 climatology from weather.bin plus
//! the Layer 2 stateless synoptic field. Everything here is a PURE function
//! of (planet seed, direction, weather time) — no mutable state, no RNG
//! draws, no wall clock — so any (seed, position, time) reproduces the
//! identical weather: the play harness and photo sidecars stay exact.
//!
//! Layer 3 (what you SEE) lives in renderer.rs + shader.wgsl and reads one
//! `Weather` sample per frame at the camera; per-pixel detail is shader
//! noise driven by the same uniforms.

use glam::DVec3;

use crate::noise::fbm_band;
use crate::planet::Planet;

/// Every art-directable knob in one place (WEATHER.md: "if Andrew wants to
/// art-direct a value, it must be a knob"). Defaults in code; an optional
/// `viewer/assets/weather_tuning.json` overrides any subset by field name.
#[derive(Clone, serde::Deserialize)]
#[serde(default)]
pub struct WeatherTuning {
    /// Game days per year: seasons at a pace a session can feel.
    pub days_per_year: f64,
    /// Where in the year the world starts (0 = January).
    pub epoch_frac: f64,
    /// Synoptic anomaly amplitude added to the cloud climatology (0 =
    /// climate-mean skies forever, 1 = full storm/clear swings).
    pub storminess: f64,
    /// How many times faster than the real wind the synoptic pattern
    /// drifts (real fronts take days; we want minutes).
    pub synoptic_speed: f64,
    /// Base frequency of the big cloud systems (~800 km at 40).
    pub synoptic_freq: f64,
    /// Base frequency of the small cloud texture (~100 km at 320).
    pub meso_freq: f64,
    /// Cloud-cover contrast curve: cover = smoothstep(lo, hi, raw).
    pub cover_lo: f64,
    pub cover_hi: f64,
    /// Cover above which precipitation starts, and the intensity gamma.
    pub precip_threshold: f64,
    pub precip_gamma: f64,
    /// mm/month of climatological precip that counts as "fully wet".
    pub precip_wet_norm: f64,
    /// Rain/snow mix band (C): all snow below lo, all rain above hi.
    pub snow_lo_c: f64,
    pub snow_hi_c: f64,
    /// Fraction of direct sunlight that survives full overcast.
    pub overcast_sun_floor: f64,
    /// Max rain darkening of ground albedo (0..1).
    pub rain_darken: f64,
    /// Snow dusting: full white this many C below freezing at the pixel.
    pub dust_full_c: f64,
    /// Cloud shell altitude above the surface (km).
    pub shell_alt_km: f64,
    /// Camera altitude (km) by which the W1 cloud shell has faded out —
    /// W1 renders NO clouds from space (WEATHER.md hard constraint; the
    /// real orbital layer with a capped opacity is W3).
    pub shell_fade_km: f64,
    /// Max precipitation particles at full intensity.
    pub particles_max: u32,
}

impl Default for WeatherTuning {
    fn default() -> Self {
        Self {
            days_per_year: 20.0,
            epoch_frac: 0.45,
            storminess: 0.55,
            synoptic_speed: 150.0,
            synoptic_freq: 40.0,
            meso_freq: 320.0,
            cover_lo: 0.25,
            cover_hi: 0.85,
            precip_threshold: 0.55,
            precip_gamma: 1.6,
            precip_wet_norm: 220.0,
            snow_lo_c: -1.5,
            snow_hi_c: 1.5,
            overcast_sun_floor: 0.35,
            rain_darken: 0.30,
            dust_full_c: 3.0,
            shell_alt_km: 3.2,
            shell_fade_km: 15.0,
            particles_max: 9000,
        }
    }
}

impl WeatherTuning {
    /// Defaults overridden by assets/weather_tuning.json when present.
    pub fn load(assets_dir: &str) -> Self {
        let path = format!("{assets_dir}/weather_tuning.json");
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(t) => {
                    println!("weather tuning: {path}");
                    t
                }
                Err(e) => {
                    eprintln!("weather tuning ignored ({path}: {e})");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }
}

/// One weather sample: what the air is doing at a place and moment.
#[derive(Clone, Copy, Debug, Default)]
pub struct Weather {
    /// Seasonal 2 m air temperature (C) — annual-mean raster + harmonic.
    pub temp_c: f64,
    /// Cloud cover 0..1 after the contrast curve.
    pub cloud_cover: f64,
    /// Precipitation intensity 0..1 (drizzle -> downpour).
    pub precip: f64,
    /// 0 = all rain, 1 = all snow (mixed in between).
    pub snow_frac: f64,
    /// Climatological wind, m/s east/north.
    pub wind_e: f64,
    pub wind_n: f64,
    /// Raw synoptic anomaly (-1..1): sky mood beyond cover alone.
    pub storm: f64,
}

const LAYERS: usize = 10;
const L_TEMP_A: usize = 0;
const L_TEMP_B: usize = 1;
const L_PRC_MEAN: usize = 2;
const L_PRC_A: usize = 3;
const L_PRC_B: usize = 4;
const L_CLD_MEAN: usize = 5;
const L_CLD_A: usize = 6;
const L_CLD_B: usize = 7;
const L_WIND_E: usize = 8;
const L_WIND_N: usize = 9;

/// The baked climatology (WEATHER.md Layer 1): per-face harmonic rasters.
pub struct WeatherField {
    res: usize,
    /// [face][layer] -> res*res raster, v-major, edge-inclusive.
    faces: Vec<Vec<Vec<f32>>>,
}

impl WeatherField {
    pub fn load(assets_dir: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read(format!("{assets_dir}/weather.bin"))?;
        anyhow::ensure!(raw.len() >= 12 && &raw[0..4] == b"WEA1", "bad weather.bin header");
        let res = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let n_layers = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
        anyhow::ensure!(n_layers == LAYERS, "weather.bin has {n_layers} layers, expected {LAYERS}");
        anyhow::ensure!(
            raw.len() == 12 + 6 * n_layers * res * res * 4,
            "weather.bin size mismatch — rerun scripts/bake_weather.py"
        );
        let mut off = 12;
        let mut faces = Vec::with_capacity(6);
        for _ in 0..6 {
            let mut layers = Vec::with_capacity(LAYERS);
            for _ in 0..LAYERS {
                let n = res * res;
                let tex: Vec<f32> = raw[off..off + n * 4]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                off += n * 4;
                layers.push(tex);
            }
            faces.push(layers);
        }
        Ok(Self { res, faces })
    }

    /// Bilinear layer sample at (face, u, v) — mirrors planet.rs::bilinear.
    fn at(&self, face: usize, layer: usize, u: f64, v: f64) -> f64 {
        let data = &self.faces[face][layer];
        let res = self.res as f64;
        let x = ((u * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let y = ((v * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.res - 1), (y0 + 1).min(self.res - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let at = |xx: usize, yy: usize| data[yy * self.res + xx] as f64;
        let a = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let b = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        a * (1.0 - fy) + b * fy
    }
}

/// Season fraction [0,1) at weather time `t_s` (0 = January).
pub fn season_frac(t_s: f64, planet_day_len_s: f64, tuning: &WeatherTuning) -> f64 {
    let day = if planet_day_len_s > 0.0 { planet_day_len_s } else { 1200.0 };
    (tuning.epoch_frac + t_s / (day * tuning.days_per_year)).rem_euclid(1.0)
}

/// The full weather sample (Layers 1+2) at a direction and weather time.
/// Pure: same (planet, dir, t) — same weather, forever.
pub fn weather_at(
    field: &WeatherField,
    planet: &Planet,
    dir: DVec3,
    t_s: f64,
    day_len_s: f64,
    tuning: &WeatherTuning,
) -> Weather {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let t_yr = season_frac(t_s, day_len_s, tuning);
    let (cs, sn) = ((std::f64::consts::TAU * t_yr).cos(), (std::f64::consts::TAU * t_yr).sin());

    // Layer 1: climatology at this season
    let temp_c = planet.temp(face, u, v) as f64
        + field.at(face, L_TEMP_A, u, v) * cs
        + field.at(face, L_TEMP_B, u, v) * sn;
    let prc = (field.at(face, L_PRC_MEAN, u, v)
        + field.at(face, L_PRC_A, u, v) * cs
        + field.at(face, L_PRC_B, u, v) * sn)
        .max(0.0);
    let cld = (field.at(face, L_CLD_MEAN, u, v)
        + field.at(face, L_CLD_A, u, v) * cs
        + field.at(face, L_CLD_B, u, v) * sn)
        .clamp(0.0, 1.0);
    let wind_e = field.at(face, L_WIND_E, u, v);
    let wind_n = field.at(face, L_WIND_N, u, v);

    // Layer 2: the "sim" — noise advected along the climatological wind.
    // The domain slides, the function never changes: stateless, seekable.
    let east0 = DVec3::Z.cross(dir);
    let east = if east0.length_squared() < 1e-9 { DVec3::Y } else { east0.normalize() };
    let north = dir.cross(east);
    let drift = (east * wind_e + north * wind_n)
        * (tuning.synoptic_speed * t_s / (1000.0 * planet.radius_km));
    let adv = (dir - drift).normalize_or_zero();
    let seed = planet.seed;
    let synoptic = fbm_band(adv, 0, 3, tuning.synoptic_freq, seed.wrapping_add(80081));
    // the fine texture drifts a little faster (gust fronts outrun systems)
    let adv2 = (dir - drift * 1.6).normalize_or_zero();
    let meso = fbm_band(adv2, 0, 2, tuning.meso_freq, seed.wrapping_add(90091));

    let raw = cld + tuning.storminess * synoptic + 0.18 * meso;
    let cover = smooth01((raw - tuning.cover_lo) / (tuning.cover_hi - tuning.cover_lo));

    // precipitation: falls out of heavy cover, scaled by how wet this
    // climate is right now (a desert overcast passes dry)
    let wetness = (prc / tuning.precip_wet_norm).clamp(0.0, 1.5);
    let over = ((cover - tuning.precip_threshold) / (1.0 - tuning.precip_threshold)).max(0.0);
    let precip = (over.powf(tuning.precip_gamma) * wetness).clamp(0.0, 1.0);
    let snow_frac =
        1.0 - smooth01((temp_c - tuning.snow_lo_c) / (tuning.snow_hi_c - tuning.snow_lo_c));

    Weather {
        temp_c,
        cloud_cover: cover,
        precip,
        snow_frac,
        wind_e,
        wind_n,
        storm: synoptic,
    }
}

fn smooth01(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

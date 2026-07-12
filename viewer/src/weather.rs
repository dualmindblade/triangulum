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
#[derive(Clone, Debug, serde::Deserialize)]
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
    /// Signed local redistribution of rain between sub-raster troughs and
    /// peaks. 0 disables it; 0.18 means at most +18% in a deep crevice and
    /// -18% on a sharp local high. The global precipitation sample is never
    /// changed -- this is presentation-scale interpolation only (D-8).
    pub rain_crevice_bias: f64,
    /// Snow dusting: full white this many C below freezing at the pixel.
    pub dust_full_c: f64,
    /// Low storm-base cloud shell altitude above the surface (km).
    pub shell_alt_km: f64,
    /// Middle cumulus and high cirrus shell altitudes (km).
    pub cloud_mid_alt_km: f64,
    pub cloud_high_alt_km: f64,
    /// Camera altitude (km) at which below-deck rendering has handed fully
    /// to the capped orbital composite.
    pub shell_fade_km: f64,
    /// Number of presentation shells. One keeps the middle formation, two
    /// add cirrus, and three add the low storm base.
    pub cloud_layer_count: u32,
    /// Noise frequencies on the unit sphere (larger = smaller formations).
    pub cloud_low_scale: f64,
    pub cloud_mid_scale: f64,
    pub cloud_high_scale: f64,
    /// Per-layer opacity multipliers before the orbital hard cap.
    pub cloud_low_density: f64,
    pub cloud_mid_density: f64,
    pub cloud_high_density: f64,
    /// W3 hard cap: even stacked shells cannot hide more orbital ground.
    pub orbit_cloud_opacity_cap: f64,
    /// Max precipitation particles at full intensity.
    pub particles_max: u32,
}

/// A tuning file is art direction, not permission to ask the GPU to draw an
/// unbounded instance count. This is deliberately generous (11x the shipped
/// default) while still turning a typo such as 4 billion into a loud fallback.
pub const PARTICLES_MAX_CAP: u32 = 100_000;

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
            rain_crevice_bias: 0.18,
            dust_full_c: 3.0,
            shell_alt_km: 1.8,
            cloud_mid_alt_km: 3.8,
            cloud_high_alt_km: 8.2,
            shell_fade_km: 15.0,
            cloud_layer_count: 3,
            cloud_low_scale: 460.0,
            cloud_mid_scale: 900.0,
            cloud_high_scale: 620.0,
            cloud_low_density: 0.92,
            cloud_mid_density: 0.82,
            cloud_high_density: 0.32,
            orbit_cloud_opacity_cap: 0.55,
            particles_max: 9000,
        }
    }
}

impl WeatherTuning {
    /// Defaults overridden by assets/weather_tuning.json when present.
    pub fn load(assets_dir: &str) -> Self {
        let path = format!("{assets_dir}/weather_tuning.json");
        match std::fs::read_to_string(&path) {
            Ok(raw) => Self::from_json(&raw, &path),
            Err(_) => Self::default(),
        }
    }

    fn from_json(raw: &str, path: &str) -> Self {
        match serde_json::from_str::<Self>(raw) {
            Ok(t) => match t.validate() {
                Ok(()) => {
                    println!("weather tuning: {path}");
                    t
                }
                Err(e) => {
                    eprintln!("weather tuning ignored ({path}: invalid {e}); using defaults");
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!("weather tuning ignored ({path}: {e}); using defaults");
                Self::default()
            }
        }
    }

    /// Reject values that can divide by zero, create NaNs/infinities, invert
    /// a smoothstep band, or turn a typo into an absurd GPU draw. The whole
    /// override falls back together: partially applying a bad art-direction
    /// file is harder to diagnose than one loud, deterministic default.
    fn validate(&self) -> Result<(), String> {
        let finite = [
            ("days_per_year", self.days_per_year),
            ("epoch_frac", self.epoch_frac),
            ("storminess", self.storminess),
            ("synoptic_speed", self.synoptic_speed),
            ("synoptic_freq", self.synoptic_freq),
            ("meso_freq", self.meso_freq),
            ("cover_lo", self.cover_lo),
            ("cover_hi", self.cover_hi),
            ("precip_threshold", self.precip_threshold),
            ("precip_gamma", self.precip_gamma),
            ("precip_wet_norm", self.precip_wet_norm),
            ("snow_lo_c", self.snow_lo_c),
            ("snow_hi_c", self.snow_hi_c),
            ("overcast_sun_floor", self.overcast_sun_floor),
            ("rain_darken", self.rain_darken),
            ("rain_crevice_bias", self.rain_crevice_bias),
            ("dust_full_c", self.dust_full_c),
            ("shell_alt_km", self.shell_alt_km),
            ("cloud_mid_alt_km", self.cloud_mid_alt_km),
            ("cloud_high_alt_km", self.cloud_high_alt_km),
            ("shell_fade_km", self.shell_fade_km),
            ("cloud_low_scale", self.cloud_low_scale),
            ("cloud_mid_scale", self.cloud_mid_scale),
            ("cloud_high_scale", self.cloud_high_scale),
            ("cloud_low_density", self.cloud_low_density),
            ("cloud_mid_density", self.cloud_mid_density),
            ("cloud_high_density", self.cloud_high_density),
            ("orbit_cloud_opacity_cap", self.orbit_cloud_opacity_cap),
        ];
        if let Some((name, _)) = finite.into_iter().find(|(_, v)| !v.is_finite()) {
            return Err(format!("{name} must be finite"));
        }
        if self.days_per_year <= 0.0 {
            return Err("days_per_year must be > 0".into());
        }
        if self.storminess < 0.0 || self.synoptic_speed < 0.0 {
            return Err("storminess and synoptic_speed must be >= 0".into());
        }
        if self.synoptic_freq <= 0.0 || self.meso_freq <= 0.0 {
            return Err("synoptic_freq and meso_freq must be > 0".into());
        }
        if self.cover_lo >= self.cover_hi {
            return Err("cover_lo must be < cover_hi".into());
        }
        if !(0.0..1.0).contains(&self.precip_threshold) {
            return Err("precip_threshold must be in [0, 1)".into());
        }
        if self.precip_gamma <= 0.0 || self.precip_wet_norm <= 0.0 {
            return Err("precip_gamma and precip_wet_norm must be > 0".into());
        }
        if self.snow_lo_c >= self.snow_hi_c {
            return Err("snow_lo_c must be < snow_hi_c".into());
        }
        if !(0.0..=1.0).contains(&self.overcast_sun_floor)
            || !(0.0..=1.0).contains(&self.rain_darken)
            || !(0.0..=1.0).contains(&self.rain_crevice_bias)
        {
            return Err(
                "overcast_sun_floor, rain_darken, and rain_crevice_bias must be in [0, 1]"
                    .into(),
            );
        }
        if self.dust_full_c <= 0.0 {
            return Err("dust_full_c must be > 0".into());
        }
        if self.shell_alt_km < 0.0
            || self.cloud_mid_alt_km <= self.shell_alt_km
            || self.cloud_high_alt_km <= self.cloud_mid_alt_km
            || self.shell_fade_km <= self.cloud_high_alt_km
        {
            return Err(
                "cloud altitudes require 0 <= low < mid < high < shell_fade_km".into(),
            );
        }
        if !(1..=3).contains(&self.cloud_layer_count) {
            return Err("cloud_layer_count must be in 1..=3".into());
        }
        if self.cloud_low_scale <= 0.0
            || self.cloud_mid_scale <= 0.0
            || self.cloud_high_scale <= 0.0
        {
            return Err("cloud scales must be > 0".into());
        }
        if !(0.0..=1.0).contains(&self.cloud_low_density)
            || !(0.0..=1.0).contains(&self.cloud_mid_density)
            || !(0.0..=1.0).contains(&self.cloud_high_density)
            || !(0.0..=1.0).contains(&self.orbit_cloud_opacity_cap)
        {
            return Err("cloud densities and orbit opacity cap must be in [0, 1]".into());
        }
        if self.particles_max > PARTICLES_MAX_CAP {
            return Err(format!("particles_max must be <= {PARTICLES_MAX_CAP}"));
        }
        Ok(())
    }

    /// Set the epoch so the weather clock is at `target` on the very next
    /// sample. This is the semantic behind `weather season FRAC`: elapsed
    /// simulation/weather time must not be added on top of the requested
    /// phase.
    pub fn set_season_frac(&mut self, weather_t_s: f64, planet_day_len_s: f64, target: f64) {
        let day = effective_day_len(planet_day_len_s);
        let days = if self.days_per_year.is_finite() && self.days_per_year > 0.0 {
            self.days_per_year
        } else {
            Self::default().days_per_year
        };
        let weather_t_s = if weather_t_s.is_finite() { weather_t_s } else { 0.0 };
        let target = if target.is_finite() { target } else { 0.0 };
        let elapsed = weather_t_s / (day * days);
        self.epoch_frac = (target.rem_euclid(1.0) - elapsed).rem_euclid(1.0);
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

// WEA2 keeps temperature at k=1 and adds k=2 cosine/sine terms for the two
// fields whose real monthly series are commonly bimodal. Internally a legacy
// WEA1 file is expanded to this layout with those four terms zero-filled.
const LAYERS: usize = 14;
const WEA1_LAYERS: usize = 10;
const L_TEMP_A: usize = 0;
const L_TEMP_B: usize = 1;
const L_PRC_MEAN: usize = 2;
const L_PRC_A1: usize = 3;
const L_PRC_B1: usize = 4;
const L_PRC_A2: usize = 5;
const L_PRC_B2: usize = 6;
const L_CLD_MEAN: usize = 7;
const L_CLD_A1: usize = 8;
const L_CLD_B1: usize = 9;
const L_CLD_A2: usize = 10;
const L_CLD_B2: usize = 11;
const L_WIND_E: usize = 12;
const L_WIND_N: usize = 13;

const WEA1_TO_WEA2: [usize; WEA1_LAYERS] = [
    L_TEMP_A,
    L_TEMP_B,
    L_PRC_MEAN,
    L_PRC_A1,
    L_PRC_B1,
    L_CLD_MEAN,
    L_CLD_A1,
    L_CLD_B1,
    L_WIND_E,
    L_WIND_N,
];

/// The baked climatology (WEATHER.md Layer 1): per-face harmonic rasters.
pub struct WeatherField {
    res: usize,
    /// [face][layer] -> res*res raster, v-major, edge-inclusive.
    faces: Vec<Vec<Vec<f32>>>,
}

impl WeatherField {
    pub fn load(assets_dir: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read(format!("{assets_dir}/weather.bin"))?;
        Self::from_bytes(&raw)
    }

    fn from_bytes(raw: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(raw.len() >= 12, "weather.bin header is truncated");
        let (source_layers, legacy) = match &raw[0..4] {
            b"WEA2" => (LAYERS, false),
            b"WEA1" => (WEA1_LAYERS, true),
            magic => anyhow::bail!(
                "unsupported weather.bin magic {:?}; expected WEA2 (rerun python scripts/bake_weather.py)",
                String::from_utf8_lossy(magic)
            ),
        };
        let res = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let n_layers = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
        anyhow::ensure!(res > 0, "weather.bin resolution must be > 0");
        anyhow::ensure!(
            n_layers == source_layers,
            "weather.bin {:?} has {n_layers} layers, expected {source_layers}",
            String::from_utf8_lossy(&raw[0..4])
        );
        let expected_len = 6usize
            .checked_mul(n_layers)
            .and_then(|n| n.checked_mul(res))
            .and_then(|n| n.checked_mul(res))
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(12))
            .ok_or_else(|| anyhow::anyhow!("weather.bin dimensions overflow"))?;
        anyhow::ensure!(
            raw.len() == expected_len,
            "weather.bin size mismatch — rerun scripts/bake_weather.py"
        );
        let mut off = 12;
        let mut faces = Vec::with_capacity(6);
        for _ in 0..6 {
            let n = res * res;
            let mut layers = vec![vec![0.0; n]; LAYERS];
            for source_layer in 0..source_layers {
                let n = res * res;
                let tex: Vec<f32> = raw[off..off + n * 4]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                anyhow::ensure!(
                    tex.iter().all(|v| v.is_finite()),
                    "weather.bin contains a non-finite coefficient"
                );
                off += n * 4;
                let layer = if legacy {
                    WEA1_TO_WEA2[source_layer]
                } else {
                    source_layer
                };
                layers[layer] = tex;
            }
            faces.push(layers);
        }
        if legacy {
            eprintln!(
                "weather.bin legacy WEA1 loaded: precipitation/cloud k=2 terms are zero; \
                 rebake with `python scripts/bake_weather.py output/seed42_r8 256` for WEA2"
            );
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

    /// Climatology-only sample (Layer 1, no synoptic noise): seasonal 2 m air
    /// temperature (C) and precipitation (mm/month) at a face coordinate.
    /// The `cs1`/`sn1` and `cs2`/`sn2` pairs are the annual and semiannual
    /// season angles, precomputed so a whole-map sweep pays no per-pixel
    /// trig. This is the cheap path the photo map's temperature/precipitation
    /// layers use — the cloud layer wants the full `weather_at` (its synoptic
    /// field is the whole point).
    pub fn climate_sample(
        &self,
        planet: &Planet,
        face: usize,
        u: f64,
        v: f64,
        cs1: f64,
        sn1: f64,
        cs2: f64,
        sn2: f64,
    ) -> (f64, f64) {
        let temp_c = planet.temp(face, u, v) as f64
            + self.at(face, L_TEMP_A, u, v) * cs1
            + self.at(face, L_TEMP_B, u, v) * sn1;
        let precip = (self.at(face, L_PRC_MEAN, u, v)
            + self.at(face, L_PRC_A1, u, v) * cs1
            + self.at(face, L_PRC_B1, u, v) * sn1
            + self.at(face, L_PRC_A2, u, v) * cs2
            + self.at(face, L_PRC_B2, u, v) * sn2)
            .max(0.0);
        (temp_c, precip)
    }
}

fn effective_day_len(planet_day_len_s: f64) -> f64 {
    if planet_day_len_s.is_finite() && planet_day_len_s > 0.0 {
        planet_day_len_s
    } else {
        1200.0
    }
}

/// Season fraction [0,1) at weather time `t_s` (0 = January).
pub fn season_frac(t_s: f64, planet_day_len_s: f64, tuning: &WeatherTuning) -> f64 {
    let defaults = WeatherTuning::default();
    let day = effective_day_len(planet_day_len_s);
    let days_per_year = if tuning.days_per_year.is_finite() && tuning.days_per_year > 0.0 {
        tuning.days_per_year
    } else {
        defaults.days_per_year
    };
    let epoch = if tuning.epoch_frac.is_finite() {
        tuning.epoch_frac
    } else {
        defaults.epoch_frac
    };
    let t_s = if t_s.is_finite() { t_s } else { 0.0 };
    (epoch + t_s / (day * days_per_year)).rem_euclid(1.0)
}

/// Move a sampling direction upstream by a tangent arc vector. `arc.length()`
/// is an angle in radians on the unit sphere, not a chord/tangent
/// approximation. Rodrigues' axis-angle rotation therefore keeps moving (and
/// wrapping) for arbitrarily long weather clocks instead of asymptoting at
/// 90 degrees like `normalize(dir - arc)`.
fn advect_great_circle(dir: DVec3, arc: DVec3) -> DVec3 {
    let dir = dir.normalize_or_zero();
    if dir == DVec3::ZERO {
        return dir;
    }
    let tangent = arc - dir * arc.dot(dir);
    let theta = tangent.length();
    if !theta.is_finite() || theta <= 1e-15 {
        return dir;
    }
    // tangent x dir rotates dir toward -tangent: upstream, matching the old
    // small-angle `dir - drift` sign while remaining exact at large angles.
    let axis = tangent.cross(dir).normalize();
    let angle = theta.rem_euclid(std::f64::consts::TAU);
    let (sin, cos) = angle.sin_cos();
    (dir * cos + axis.cross(dir) * sin + axis * axis.dot(dir) * (1.0 - cos)).normalize()
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
    let angle = std::f64::consts::TAU * t_yr;
    let (sn1, cs1) = angle.sin_cos();
    let (sn2, cs2) = (2.0 * angle).sin_cos();

    // Layer 1: climatology at this season
    let temp_c = planet.temp(face, u, v) as f64
        + field.at(face, L_TEMP_A, u, v) * cs1
        + field.at(face, L_TEMP_B, u, v) * sn1;
    let prc = (field.at(face, L_PRC_MEAN, u, v)
        + field.at(face, L_PRC_A1, u, v) * cs1
        + field.at(face, L_PRC_B1, u, v) * sn1
        + field.at(face, L_PRC_A2, u, v) * cs2
        + field.at(face, L_PRC_B2, u, v) * sn2)
        .max(0.0);
    let cld = (field.at(face, L_CLD_MEAN, u, v)
        + field.at(face, L_CLD_A1, u, v) * cs1
        + field.at(face, L_CLD_B1, u, v) * sn1
        + field.at(face, L_CLD_A2, u, v) * cs2
        + field.at(face, L_CLD_B2, u, v) * sn2)
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
    let adv = advect_great_circle(dir, drift);
    let seed = planet.seed;
    let synoptic = fbm_band(adv, 0, 3, tuning.synoptic_freq, seed.wrapping_add(80081));
    // the fine texture drifts a little faster (gust fronts outrun systems)
    let adv2 = advect_great_circle(dir, drift * 1.6);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn weather_bytes(magic: &[u8; 4], layers: usize) -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend_from_slice(magic);
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&(layers as u32).to_le_bytes());
        for face in 0..6 {
            for layer in 0..layers {
                raw.extend_from_slice(&((face * 100 + layer) as f32).to_le_bytes());
            }
        }
        raw
    }

    #[test]
    fn wea1_loads_loudly_with_zero_semiannual_terms() {
        let field = WeatherField::from_bytes(&weather_bytes(b"WEA1", WEA1_LAYERS)).unwrap();
        assert_eq!(field.at(0, L_PRC_A1, 0.0, 0.0), 3.0);
        assert_eq!(field.at(0, L_PRC_A2, 0.0, 0.0), 0.0);
        assert_eq!(field.at(0, L_CLD_MEAN, 0.0, 0.0), 5.0);
        assert_eq!(field.at(0, L_CLD_A2, 0.0, 0.0), 0.0);
        assert_eq!(field.at(0, L_WIND_E, 0.0, 0.0), 8.0);
    }

    #[test]
    fn wea2_preserves_all_fourier_layers() {
        let field = WeatherField::from_bytes(&weather_bytes(b"WEA2", LAYERS)).unwrap();
        assert_eq!(field.at(0, L_PRC_A2, 0.0, 0.0), L_PRC_A2 as f64);
        assert_eq!(field.at(0, L_CLD_B2, 0.0, 0.0), L_CLD_B2 as f64);
        assert_eq!(field.at(5, L_WIND_N, 0.0, 0.0), 500.0 + L_WIND_N as f64);
    }

    #[test]
    fn invalid_tuning_override_falls_back_as_a_whole() {
        let invalid = [
            r#"{"days_per_year":0,"epoch_frac":0.12}"#,
            r#"{"cover_lo":0.8,"cover_hi":0.2,"epoch_frac":0.12}"#,
            r#"{"snow_lo_c":2,"snow_hi_c":-2,"epoch_frac":0.12}"#,
            r#"{"precip_threshold":1,"epoch_frac":0.12}"#,
            r#"{"particles_max":100001,"epoch_frac":0.12}"#,
            r#"{"rain_crevice_bias":1.01,"epoch_frac":0.12}"#,
            r#"{"cloud_layer_count":0,"epoch_frac":0.12}"#,
            r#"{"cloud_layer_count":4,"epoch_frac":0.12}"#,
            r#"{"cloud_mid_alt_km":1.0,"epoch_frac":0.12}"#,
            r#"{"cloud_high_alt_km":16.0,"epoch_frac":0.12}"#,
            r#"{"cloud_low_scale":0,"epoch_frac":0.12}"#,
            r#"{"cloud_mid_density":1.01,"epoch_frac":0.12}"#,
            r#"{"orbit_cloud_opacity_cap":1.01,"epoch_frac":0.12}"#,
            r#"{"days_per_year":1e999,"epoch_frac":0.12}"#,
        ];
        for raw in invalid {
            let got = WeatherTuning::from_json(raw, "test-weather-tuning.json");
            assert_eq!(got.epoch_frac, WeatherTuning::default().epoch_frac, "{raw}");
            assert!(got.validate().is_ok());
        }

        let mut non_finite = WeatherTuning::default();
        non_finite.cover_lo = f64::NAN;
        assert!(non_finite.validate().is_err());
    }

    #[test]
    fn cloud_v2_defaults_keep_three_ordered_layers_and_w3_cap() {
        let t = WeatherTuning::default();
        assert_eq!(t.cloud_layer_count, 3);
        assert!(t.shell_alt_km < t.cloud_mid_alt_km);
        assert!(t.cloud_mid_alt_km < t.cloud_high_alt_km);
        assert!(t.cloud_high_alt_km < t.shell_fade_km);
        assert_eq!(t.orbit_cloud_opacity_cap, 0.55);
        assert!(t.validate().is_ok());
    }

    #[test]
    fn tuning_load_rejects_degenerate_file() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "triangulum-weather-tuning-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("weather_tuning.json"),
            r#"{"days_per_year":0,"epoch_frac":0.12}"#,
        )
        .unwrap();
        let got = WeatherTuning::load(dir.to_str().unwrap());
        assert_eq!(got.days_per_year, WeatherTuning::default().days_per_year);
        assert_eq!(got.epoch_frac, WeatherTuning::default().epoch_frac);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn season_command_targets_next_shot_after_elapsed_time() {
        let mut tuning = WeatherTuning::default();
        tuning.set_season_frac(1200.0, 1200.0, 0.25);
        assert!((tuning.epoch_frac - 0.20).abs() < 1e-12);
        assert!((season_frac(1200.0, 1200.0, &tuning) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn great_circle_advection_tracks_unwrapped_arc_through_two_years() {
        let dir = DVec3::X;
        let intended_deg: [f64; 6] = [28.39, 56.78, 113.55, 189.25, 227.10, 378.50];
        for scale in [1.0f64, 1.6] {
            for intended in intended_deg {
                let angle = (intended * scale).to_radians();
                let got = advect_great_circle(dir, DVec3::Y * angle);
                let expected = DVec3::new(angle.cos(), -angle.sin(), 0.0);
                assert!(got.distance(expected) < 1e-12, "{intended} deg at {scale}x");

                // Recover the oriented phase and add back complete turns so
                // this is the same intended-vs-effective measurement used in
                // the W-8 evidence table, rather than acos' [0, pi] fold.
                let phase = (-got.y).atan2(got.x).rem_euclid(std::f64::consts::TAU);
                let effective = phase + (angle / std::f64::consts::TAU).floor()
                    * std::f64::consts::TAU;
                assert!((effective - angle).abs() < 1e-12);
                assert!((got.length() - 1.0).abs() < 1e-12);
            }
        }
    }
}

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
    /// Structural W4 freeze/thaw thresholds. Cooling water freezes below
    /// `freeze_c`; warming ice persists until `thaw_c`.
    pub freeze_c: f64,
    pub thaw_c: f64,
    /// Number of immutable structural snapshots per orbit. Geometry changes
    /// only when this bucket changes, never continuously within a frame.
    pub season_buckets: u32,
    /// Strength of the broadleaf autumn/dormancy tint curve.
    pub deciduous_tint_strength: f64,
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
    /// Maximum direct-light loss under an opaque daytime cloud stack.
    pub cloud_shadow_strength: f64,
    /// Additional subtle ground darkening under the capped orbital cloud
    /// composite. This is deliberately weaker than the ground projection.
    pub orbit_cloud_shadow_strength: f64,
    /// Valley/bank fog extinction multiplier and its ceiling above the
    /// camera-ground reference (km).
    pub fog_density: f64,
    pub fog_ceiling_km: f64,
    /// Half-width of the sunrise mist window in local solar hours.
    pub fog_dawn_window_h: f64,
    /// Great-circle distance of the eight storm-edge probes (km), maximum
    /// directional gloom, and the classic warm/green pre-storm cast amount.
    pub storm_edge_probe_km: f64,
    pub storm_edge_strength: f64,
    pub storm_green_cast: f64,
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
            freeze_c: -5.0,
            thaw_c: -2.0,
            season_buckets: 24,
            deciduous_tint_strength: 0.22,
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
            cloud_shadow_strength: 0.35,
            orbit_cloud_shadow_strength: 0.10,
            fog_density: 0.85,
            fog_ceiling_km: 0.24,
            fog_dawn_window_h: 2.0,
            storm_edge_probe_km: 650.0,
            storm_edge_strength: 0.32,
            storm_green_cast: 0.08,
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
            ("freeze_c", self.freeze_c),
            ("thaw_c", self.thaw_c),
            ("deciduous_tint_strength", self.deciduous_tint_strength),
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
            ("cloud_shadow_strength", self.cloud_shadow_strength),
            (
                "orbit_cloud_shadow_strength",
                self.orbit_cloud_shadow_strength,
            ),
            ("fog_density", self.fog_density),
            ("fog_ceiling_km", self.fog_ceiling_km),
            ("fog_dawn_window_h", self.fog_dawn_window_h),
            ("storm_edge_probe_km", self.storm_edge_probe_km),
            ("storm_edge_strength", self.storm_edge_strength),
            ("storm_green_cast", self.storm_green_cast),
        ];
        if let Some((name, _)) = finite.into_iter().find(|(_, v)| !v.is_finite()) {
            return Err(format!("{name} must be finite"));
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
        if self.freeze_c >= self.thaw_c {
            return Err("freeze_c must be < thaw_c".into());
        }
        if !(4..=96).contains(&self.season_buckets) {
            return Err("season_buckets must be in 4..=96".into());
        }
        if !(0.0..=1.0).contains(&self.deciduous_tint_strength) {
            return Err("deciduous_tint_strength must be in [0, 1]".into());
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
        if !(0.0..=1.0).contains(&self.cloud_shadow_strength)
            || !(0.0..=1.0).contains(&self.orbit_cloud_shadow_strength)
            || !(0.0..=1.0).contains(&self.storm_edge_strength)
            || !(0.0..=1.0).contains(&self.storm_green_cast)
        {
            return Err(
                "cloud/orbit shadow strength, storm-edge strength, and green cast must be in [0, 1]"
                    .into(),
            );
        }
        if self.fog_density < 0.0 || self.fog_ceiling_km <= 0.0 {
            return Err("fog_density must be >= 0 and fog_ceiling_km must be > 0".into());
        }
        if self.fog_dawn_window_h <= 0.0 || self.fog_dawn_window_h > 12.0 {
            return Err("fog_dawn_window_h must be in (0, 12]".into());
        }
        if self.storm_edge_probe_km <= 0.0 || self.storm_edge_probe_km > 3000.0 {
            return Err("storm_edge_probe_km must be in (0, 3000]".into());
        }
        if self.particles_max > PARTICLES_MAX_CAP {
            return Err(format!("particles_max must be <= {PARTICLES_MAX_CAP}"));
        }
        Ok(())
    }

}

/// The legacy structural epoch. Its bucket deliberately evaluates the annual
/// mean byte-for-byte so every pre-W4 baseline has an exact compatibility
/// point while all other buckets follow the baked seasonal climatology.
pub const CANONICAL_SEASON_FRAC: f64 = 0.45;

/// Immutable input shared by terrain sampling, meshing, physics, and edits.
/// `enabled == false` is the weather-off/legacy annual-mean law.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StructuralSeason {
    pub enabled: bool,
    pub bucket: u32,
    pub bucket_count: u32,
    pub season_frac: f64,
    pub sin_phase: f64,
    pub cos_phase: f64,
    pub freeze_c: f64,
    pub thaw_c: f64,
    pub deciduous_tint_strength: f64,
}

impl StructuralSeason {
    pub const ANNUAL_BUCKET: u32 = u32::MAX;

    pub fn annual() -> Self {
        Self {
            enabled: false,
            bucket: Self::ANNUAL_BUCKET,
            bucket_count: 1,
            season_frac: CANONICAL_SEASON_FRAC,
            sin_phase: 0.0,
            cos_phase: 0.0,
            freeze_c: -4.0,
            thaw_c: -4.0,
            deciduous_tint_strength: 0.0,
        }
    }

    pub fn quantized(season_frac: f64, tuning: &WeatherTuning) -> Self {
        let count = tuning.season_buckets.clamp(4, 96);
        let frac = if season_frac.is_finite() {
            season_frac.rem_euclid(1.0)
        } else {
            CANONICAL_SEASON_FRAC
        };
        let bucket = (frac * count as f64).floor() as u32 % count;
        let canonical_bucket = (CANONICAL_SEASON_FRAC * count as f64).floor() as u32;
        // Canonical compatibility is bucket-wide: a short wait in an old
        // suite cannot nudge its first rebuild away from the blessed state.
        let sampled = if bucket == canonical_bucket {
            CANONICAL_SEASON_FRAC
        } else {
            (bucket as f64 + 0.5) / count as f64
        };
        let (sin_phase, cos_phase) = (std::f64::consts::TAU * sampled).sin_cos();
        Self {
            enabled: true,
            bucket,
            bucket_count: count,
            season_frac: sampled,
            sin_phase,
            cos_phase,
            freeze_c: tuning.freeze_c,
            thaw_c: tuning.thaw_c,
            deciduous_tint_strength: tuning.deciduous_tint_strength,
        }
    }

    pub fn is_canonical(self) -> bool {
        self.enabled && self.season_frac.to_bits() == CANONICAL_SEASON_FRAC.to_bits()
    }
}

impl Default for StructuralSeason {
    fn default() -> Self {
        Self::annual()
    }
}

/// Subtle broadleaf-only color cycle. Temperature makes the phase local to
/// latitude/coast/elevation; the derivative distinguishes autumn cooling from
/// spring warming at the same temperature. Canonical and weather-off calls
/// return the input bit-for-bit.
pub fn deciduous_tint(
    base: [f32; 3],
    temp_c: f64,
    trend_c_per_orbit: f64,
    season: StructuralSeason,
) -> [f32; 3] {
    if !season.enabled || season.is_canonical() || season.deciduous_tint_strength <= 0.0 {
        return base;
    }
    let smooth = |lo: f64, hi: f64, value: f64| {
        let x = ((value - lo) / (hi - lo)).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let cooling = smooth(0.0, 8.0, -trend_c_per_orbit);
    let autumn = smooth(14.0, 4.0, temp_c) * smooth(-2.0, 8.0, temp_c) * cooling;
    let dormant = smooth(4.0, -6.0, temp_c);
    let target = if dormant > autumn {
        [0.12, 0.115, 0.055]
    } else {
        [0.34, 0.16, 0.025]
    };
    let amount = season.deciduous_tint_strength * autumn.max(dormant);
    std::array::from_fn(|i| {
        (f64::from(base[i]) + (f64::from(target[i]) - f64::from(base[i])) * amount) as f32
    })
}

/// Pure W4 inland-water class law, separated for monotonicity gates. The
/// comparator is stable in world space; only the warming/cooling branch can
/// select which edge of the hysteresis band is active.
pub fn hysteretic_frozen(
    temp_c: f64,
    trend_c_per_orbit: f64,
    comparator: f64,
    freeze_c: f64,
    thaw_c: f64,
) -> bool {
    let edge = if trend_c_per_orbit >= 0.0 { thaw_c } else { freeze_c };
    temp_c < edge + (comparator.clamp(0.0, 1.0) - 0.5)
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
    /// Deterministic near-surface humidity proxy. The bake has no humidity
    /// raster, so seasonal wetness supplies the slow term while cover and
    /// active precipitation supply the synoptic terms.
    pub humidity: f64,
    /// Climatological wind, m/s east/north.
    pub wind_e: f64,
    pub wind_n: f64,
    /// Raw synoptic anomaly (-1..1): sky mood beyond cover alone.
    pub storm: f64,
}

// WEA2 keeps temperature at k=1 and adds k=2 cosine/sine terms for the two
// fields whose real monthly series are commonly bimodal. Internally a legacy
// WEA1 file is expanded to this layout with those four terms zero-filled.
const LAYERS: usize = 26;
const WEA2_LAYERS: usize = 14;
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
const L_SEAICE_0: usize = 14;

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
    /// Temperature cosine/sine coefficients interleaved for the W4 hot path.
    temp_harmonics: Vec<Vec<[f32; 2]>>,
    has_seaice: bool,
}

impl WeatherField {
    pub fn load(assets_dir: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read(format!("{assets_dir}/weather.bin"))?;
        Self::from_bytes(&raw)
    }

    fn from_bytes(raw: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(raw.len() >= 12, "weather.bin header is truncated");
        let (source_layers, version) = match &raw[0..4] {
            b"WEA3" => (LAYERS, 3),
            b"WEA2" => (WEA2_LAYERS, 2),
            b"WEA1" => (WEA1_LAYERS, 1),
            magic => anyhow::bail!(
                "unsupported weather.bin magic {:?}; expected WEA3 (rerun python scripts/bake_weather.py)",
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
                let layer = if version == 1 {
                    WEA1_TO_WEA2[source_layer]
                } else {
                    source_layer
                };
                layers[layer] = tex;
            }
            faces.push(layers);
        }
        if version == 1 {
            eprintln!(
                "weather.bin legacy WEA1 loaded: precipitation/cloud k=2 terms are zero; \
                 rebake with `python scripts/bake_weather.py output/seed42_r8 256` for WEA2"
            );
        } else if version == 2 {
            eprintln!(
                "weather.bin legacy WEA2 loaded: seasonal sea ice falls back to temperature; \
                 rebake with `python scripts/bake_weather.py output/seed42_r8 256` for WEA3"
            );
        }
        let temp_harmonics = faces
            .iter()
            .map(|layers| {
                layers[L_TEMP_A]
                    .iter()
                    .copied()
                    .zip(layers[L_TEMP_B].iter().copied())
                    .map(|(a, b)| [a, b])
                    .collect()
            })
            .collect();
        Ok(Self { res, faces, temp_harmonics, has_seaice: version == 3 })
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

    /// Two co-located layers with one coordinate/bilinear setup. Structural
    /// temperature stores cosine/sine coefficients separately on disk, but
    /// they are always consumed as a pair on the sample path.
    #[inline(always)]
    fn pair_at(&self, face: usize, layers: [usize; 2], u: f64, v: f64) -> [f64; 2] {
        let res = self.res as f64;
        let x = ((u * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let y = ((v * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.res - 1), (y0 + 1).min(self.res - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let i00 = y0 * self.res + x0;
        let i10 = y0 * self.res + x1;
        let i01 = y1 * self.res + x0;
        let i11 = y1 * self.res + x1;
        let interpolate = |data: &[f32]| {
            let a = data[i00] as f64 * (1.0 - fx) + data[i10] as f64 * fx;
            let b = data[i01] as f64 * (1.0 - fx) + data[i11] as f64 * fx;
            a * (1.0 - fy) + b * fy
        };
        let face_layers = &self.faces[face];
        [interpolate(&face_layers[layers[0]]), interpolate(&face_layers[layers[1]])]
    }

    #[inline(always)]
    fn temp_pair_at(&self, face: usize, u: f64, v: f64) -> [f64; 2] {
        let res = self.res as f64;
        let x = ((u * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let y = ((v * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.res - 1), (y0 + 1).min(self.res - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let data = &self.temp_harmonics[face];
        let p00 = data[y0 * self.res + x0];
        let p10 = data[y0 * self.res + x1];
        let p01 = data[y1 * self.res + x0];
        let p11 = data[y1 * self.res + x1];
        let ix = 1.0 - fx;
        let iy = 1.0 - fy;
        let a0 = p00[0] as f64 * ix + p10[0] as f64 * fx;
        let b0 = p01[0] as f64 * ix + p11[0] as f64 * fx;
        let a1 = p00[1] as f64 * ix + p10[1] as f64 * fx;
        let b1 = p01[1] as f64 * ix + p11[1] as f64 * fx;
        [a0 * iy + b0 * fy, a1 * iy + b1 * fy]
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

    /// W4's one structural temperature field. The harmonic anomaly is
    /// follows the baked Fourier shape in every seasonal bucket. The single
    /// canonical compatibility bucket is an explicit annual-raster cut line.
    /// No synoptic/weather-presentation temperature participates.
    pub fn seasonal_temp_c(
        &self,
        planet: &Planet,
        face: usize,
        u: f64,
        v: f64,
        season_frac: f64,
    ) -> f64 {
        self.seasonal_temp_state(planet, face, u, v, season_frac).0
    }

    /// Temperature plus derivative from one pair of coefficient reads.
    pub fn seasonal_temp_state(
        &self,
        planet: &Planet,
        face: usize,
        u: f64,
        v: f64,
        season_frac: f64,
    ) -> (f64, f64) {
        let annual = planet.temp(face, u, v) as f64;
        self.seasonal_temp_state_from_annual(annual, face, u, v, season_frac)
    }

    pub(crate) fn seasonal_temp_state_from_annual(
        &self,
        annual: f64,
        face: usize,
        u: f64,
        v: f64,
        season_frac: f64,
    ) -> (f64, f64) {
        let angle = std::f64::consts::TAU * season_frac.rem_euclid(1.0);
        let (sn, cs) = angle.sin_cos();
        self.seasonal_temp_state_from_phase(
            annual,
            face,
            u,
            v,
            sn,
            cs,
            season_frac.to_bits() == CANONICAL_SEASON_FRAC.to_bits(),
        )
    }

    pub(crate) fn seasonal_temp_state_from_phase(
        &self,
        annual: f64,
        face: usize,
        u: f64,
        v: f64,
        sn: f64,
        cs: f64,
        canonical: bool,
    ) -> (f64, f64) {
        let [a, b] = self.temp_pair_at(face, u, v);
        let temp = if canonical { annual } else { annual + a * cs + b * sn };
        let trend = std::f64::consts::TAU * (-a * sn + b * cs);
        (temp, trend)
    }

    /// Sign of the local Fourier temperature derivative (C per orbit).
    /// Positive means the hysteresis loop is on its thawing branch.
    pub fn seasonal_temp_trend(
        &self,
        face: usize,
        u: f64,
        v: f64,
        season_frac: f64,
    ) -> f64 {
        // This public convenience path is kept for map/probe callers; hot
        // structural sampling uses `seasonal_temp_state` once.
        let a = self.at(face, L_TEMP_A, u, v);
        let b = self.at(face, L_TEMP_B, u, v);
        let angle = std::f64::consts::TAU * season_frac.rem_euclid(1.0);
        let (sn, cs) = angle.sin_cos();
        std::f64::consts::TAU * (-a * sn + b * cs)
    }

    /// Cyclic linear interpolation of the twelve baked monthly sea-ice
    /// rasters. Returns None for legacy WEA1/2 assets so callers can fall
    /// back loudly and deterministically during migration.
    pub fn sea_ice_fraction(
        &self,
        face: usize,
        u: f64,
        v: f64,
        season_frac: f64,
    ) -> Option<f64> {
        if !self.has_seaice {
            return None;
        }
        let x = season_frac.rem_euclid(1.0) * 12.0 - 0.5;
        let m0 = x.floor() as i32;
        let t = x - x.floor();
        let i0 = m0.rem_euclid(12) as usize;
        let i1 = (m0 + 1).rem_euclid(12) as usize;
        let [a, b] = self.pair_at(face, [L_SEAICE_0 + i0, L_SEAICE_0 + i1], u, v);
        Some((a * (1.0 - t) + b * t).clamp(0.0, 1.0))
    }
}

/// Weather's public season helper deliberately delegates to the orbital
/// clock. There is no weather-owned year length or epoch to drift from the
/// physical Sun-Neisor geometry.
pub fn season_frac(
    t_s: f64,
    planet_day_len_s: f64,
    solar_tuning: &crate::orbits::SolarTuning,
) -> f64 {
    solar_tuning.season_frac(t_s, planet_day_len_s)
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
    solar_tuning: &crate::orbits::SolarTuning,
    tuning: &WeatherTuning,
) -> Weather {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let t_yr = season_frac(t_s, day_len_s, solar_tuning);
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
    let humidity = (0.55 * wetness.min(1.0) + 0.35 * cover + 0.25 * precip).clamp(0.0, 1.0);

    Weather {
        temp_c,
        cloud_cover: cover,
        precip,
        snow_frac,
        humidity,
        wind_e,
        wind_n,
        storm: synoptic,
    }
}

/// Smooth mist window centered on local sunrise. Hours are solar-clock
/// hours on a normalized 24-hour day, independent of the accelerated render
/// day length. Polar day/night has no sunrise and therefore no dawn mist.
pub fn dawn_window(surface_dir: DVec3, sun_dir: DVec3, half_width_h: f64) -> f64 {
    if !surface_dir.is_finite()
        || !sun_dir.is_finite()
        || !half_width_h.is_finite()
        || half_width_h <= 0.0
    {
        return 0.0;
    }
    let surface = surface_dir.normalize_or_zero();
    let sun = sun_dir.normalize_or_zero();
    if surface == DVec3::ZERO || sun == DVec3::ZERO {
        return 0.0;
    }
    let latitude = surface.z.clamp(-1.0, 1.0).asin();
    let declination = sun.z.clamp(-1.0, 1.0).asin();
    let sunrise_cos = -latitude.tan() * declination.tan();
    if !(-1.0..=1.0).contains(&sunrise_cos) {
        return 0.0;
    }
    let longitude = surface.y.atan2(surface.x);
    let sun_longitude = sun.y.atan2(sun.x);
    let hour_angle = (sun_longitude - longitude + std::f64::consts::PI)
        .rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;
    // Neisor's fixed-frame Sun longitude decreases through the day, so the
    // positive horizon crossing is sunrise (the negative crossing is dusk).
    let sunrise_angle = sunrise_cos.acos();
    let delta = (hour_angle - sunrise_angle + std::f64::consts::PI)
        .rem_euclid(std::f64::consts::TAU)
        - std::f64::consts::PI;
    let width = std::f64::consts::TAU * (half_width_h.clamp(0.0, 12.0) / 24.0);
    smooth01(1.0 - delta.abs() / width.max(1e-9))
}

/// First-order directional fit for eight (or any symmetric set of) weather
/// probes. `base + dot(view_dir, gradient)` is the storm load seen toward a
/// horizon direction. Both outputs remain planet-frame/world-anchored.
#[derive(Clone, Copy, Debug, Default)]
pub struct StormEdge {
    pub base: f64,
    pub gradient: DVec3,
}

fn storm_edge_load(weather: Weather) -> f64 {
    let cover = smooth01((weather.cloud_cover - 0.48) / 0.52);
    (cover * (0.35 + 0.65 * weather.precip)).clamp(0.0, 1.0)
}

pub fn storm_edge_fit(center_dir: DVec3, probes: &[(DVec3, Weather)]) -> StormEdge {
    let center = center_dir.normalize_or_zero();
    if center == DVec3::ZERO || probes.is_empty() {
        return StormEdge::default();
    }
    let mut base = 0.0;
    let mut gradient = DVec3::ZERO;
    let mut count = 0.0;
    for &(probe_dir, weather) in probes {
        let probe = probe_dir.normalize_or_zero();
        let tangent = (probe - center * probe.dot(center)).normalize_or_zero();
        if tangent == DVec3::ZERO {
            continue;
        }
        let load = storm_edge_load(weather);
        base += load;
        gradient += tangent * load;
        count += 1.0;
    }
    if count == 0.0 {
        return StormEdge::default();
    }
    StormEdge {
        base: (base / count).clamp(0.0, 1.0),
        // A symmetric compass ring has E[t t^T] = I/2 in its tangent
        // plane, hence the factor two recovers the SH-1 coefficient.
        gradient: gradient * (2.0 / count),
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
        let field = WeatherField::from_bytes(&weather_bytes(b"WEA2", WEA2_LAYERS)).unwrap();
        assert_eq!(field.at(0, L_PRC_A2, 0.0, 0.0), L_PRC_A2 as f64);
        assert_eq!(field.at(0, L_CLD_B2, 0.0, 0.0), L_CLD_B2 as f64);
        assert_eq!(field.at(5, L_WIND_N, 0.0, 0.0), 500.0 + L_WIND_N as f64);
    }

    #[test]
    fn wea3_preserves_monthly_sea_ice() {
        let field = WeatherField::from_bytes(&weather_bytes(b"WEA3", LAYERS)).unwrap();
        assert_eq!(
            field.sea_ice_fraction(0, 0.0, 0.0, 0.5 / 12.0),
            Some(1.0)
        );
    }

    #[test]
    fn season_buckets_are_deterministic_and_canonical_is_exact() {
        let tuning = WeatherTuning::default();
        let a = StructuralSeason::quantized(0.950_000_000_1, &tuning);
        let b = StructuralSeason::quantized(0.950_000_000_1, &tuning);
        assert_eq!(a, b);
        assert_eq!(a.bucket, 22);
        let canonical = StructuralSeason::quantized(CANONICAL_SEASON_FRAC, &tuning);
        assert!(canonical.is_canonical());
        assert_eq!(canonical.bucket_count, 24);
    }

    #[test]
    fn hysteresis_is_monotone_on_each_branch() {
        let mut cooling = Vec::new();
        let mut warming = Vec::new();
        for step in 0..=160 {
            let temp = -10.0 + step as f64 * 0.05;
            cooling.push(hysteretic_frozen(temp, -1.0, 0.5, -5.0, -2.0));
            warming.push(hysteretic_frozen(temp, 1.0, 0.5, -5.0, -2.0));
        }
        assert!(cooling.windows(2).all(|w| !(!w[0] && w[1])));
        assert!(warming.windows(2).all(|w| !(!w[0] && w[1])));
        for (cold, warm) in cooling.into_iter().zip(warming) {
            assert!(!cold || warm, "cooling branch froze later than thaw branch");
        }
    }

    #[test]
    fn invalid_tuning_override_falls_back_as_a_whole() {
        let invalid = [
            r#"{"storminess":-1}"#,
            r#"{"cover_lo":0.8,"cover_hi":0.2}"#,
            r#"{"snow_lo_c":2,"snow_hi_c":-2}"#,
            r#"{"freeze_c":-1,"thaw_c":-2}"#,
            r#"{"season_buckets":2}"#,
            r#"{"deciduous_tint_strength":1.1}"#,
            r#"{"precip_threshold":1}"#,
            r#"{"particles_max":100001}"#,
            r#"{"rain_crevice_bias":1.01}"#,
            r#"{"cloud_layer_count":0}"#,
            r#"{"cloud_layer_count":4}"#,
            r#"{"cloud_mid_alt_km":1.0}"#,
            r#"{"cloud_high_alt_km":16.0}"#,
            r#"{"cloud_low_scale":0}"#,
            r#"{"cloud_mid_density":1.01}"#,
            r#"{"orbit_cloud_opacity_cap":1.01}"#,
            r#"{"cloud_shadow_strength":1.01}"#,
            r#"{"orbit_cloud_shadow_strength":-0.01}"#,
            r#"{"fog_density":-0.01}"#,
            r#"{"fog_ceiling_km":0}"#,
            r#"{"fog_dawn_window_h":12.01}"#,
            r#"{"storm_edge_probe_km":3001}"#,
            r#"{"storm_edge_strength":1.01}"#,
            r#"{"storm_green_cast":-0.01}"#,
            r#"{"storminess":1e999}"#,
        ];
        for raw in invalid {
            let got = WeatherTuning::from_json(raw, "test-weather-tuning.json");
            assert_eq!(got.storminess, WeatherTuning::default().storminess, "{raw}");
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
    fn w2_tuning_defaults_are_bounded_and_ordered() {
        let t = WeatherTuning::default();
        assert_eq!(t.cloud_shadow_strength, 0.35);
        assert!(t.orbit_cloud_shadow_strength < t.cloud_shadow_strength);
        assert!(t.fog_density > 0.0);
        assert!(t.fog_ceiling_km > 0.0);
        assert!((0.0..=12.0).contains(&t.fog_dawn_window_h));
        assert!((0.0..=1.0).contains(&t.storm_edge_strength));
        assert!((0.0..=1.0).contains(&t.storm_green_cast));
        assert!(t.validate().is_ok());
    }

    #[test]
    fn dawn_window_selects_sunrise_not_noon_or_sunset() {
        let site = DVec3::X;
        let sunrise = DVec3::Y;
        let noon = DVec3::X;
        let sunset = -DVec3::Y;
        assert_eq!(dawn_window(site, sunrise, 2.0), 1.0);
        assert_eq!(dawn_window(site, noon, 2.0), 0.0);
        assert_eq!(dawn_window(site, sunset, 2.0), 0.0);
        assert_eq!(dawn_window(DVec3::Z, DVec3::X, 2.0), 0.0);
    }

    #[test]
    fn storm_edge_fit_points_toward_the_storm() {
        let center = DVec3::Z;
        let clear = Weather::default();
        let storm = Weather {
            cloud_cover: 1.0,
            precip: 1.0,
            ..Weather::default()
        };
        let probes = [
            (DVec3::X, storm),
            (DVec3::Y, clear),
            (-DVec3::X, clear),
            (-DVec3::Y, clear),
        ];
        let edge = storm_edge_fit(center, &probes);
        assert!((edge.base - 0.25).abs() < 1e-12);
        assert!(edge.gradient.x > 0.49);
        assert!(edge.gradient.y.abs() < 1e-12);
        assert!(edge.gradient.z.abs() < 1e-12);
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
            r#"{"cover_lo":0.9,"cover_hi":0.1}"#,
        )
        .unwrap();
        let got = WeatherTuning::load(dir.to_str().unwrap());
        assert_eq!(got.cover_lo, WeatherTuning::default().cover_lo);
        assert_eq!(got.cover_hi, WeatherTuning::default().cover_hi);
        std::fs::remove_dir_all(dir).unwrap();
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

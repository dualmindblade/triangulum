//! Living weather (WEATHER.md): Layer 1 climatology from weather.bin plus
//! the Layer 2 stateless synoptic field. Everything here is a PURE function
//! of (planet seed, direction, weather time) — no mutable state, no RNG
//! draws, no wall clock — so any (seed, position, time) reproduces the
//! identical weather: the play harness and photo sidecars stay exact.
//!
//! Layer 3 (what you SEE) lives in renderer.rs + shader.wgsl. Camera-local
//! responses read one `Weather` sample per frame; deck shape reads the small
//! deterministic `SynopticRaster` below at each shell-hit direction, with
//! shader fabric supplying the sub-raster scales.

use glam::DVec3;
use rayon::prelude::*;

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
    /// Planetary cover wave: a continent-scale band added to the cloud
    /// climatology so global cover varies (large clear breaks, dense belts)
    /// instead of averaging out. ~hemisphere-scale features at 5.
    pub planetary_freq: f64,
    /// Amplitude of the planetary wave in `raw` cover units. 0 disables.
    pub planetary_strength: f64,
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
    /// W-MOTION pass 2: bounded analytic storm systems. Centers follow
    /// deterministic zonal tracks; the baked cloud/precip climatology picks
    /// their tracks and continuously scales their strength.
    pub cyclone_count: u32,
    pub cyclone_radius_km: f64,
    /// Accelerated visual angular speed at the cyclone core (degrees/s).
    pub cyclone_spin_deg_s: f64,
    /// Closed-form eastward center speed along the latitude track (km/s).
    pub cyclone_track_km_s: f64,
    pub cyclone_cover_boost: f64,
    pub cyclone_precip_boost: f64,
    /// Andrew's density wave (2026-07-13): spiral arms along which cover is
    /// denser, whose PATTERN rotates with storm age while the fabric stays
    /// put - cells seed and disperse as an arm sweeps over them. A rotating
    /// density mask is rigid in angle-space, so unlike fabric advection it
    /// accumulates zero shear and may rotate forever. count 0 disables.
    pub cyclone_arm_count: u32,
    pub cyclone_arm_strength: f64,
    /// Logarithmic-spiral pitch (radians of azimuth per ln-radius unit).
    pub cyclone_arm_twist: f64,
    /// Pattern angular speed (degrees/s of weather time).
    pub cyclone_arm_spin_deg_s: f64,
    /// Finite deterministic storm life (seconds of weather time). Each
    /// system index is reborn on a freshly seeded corridor every period with
    /// a smooth grow/decay envelope; rebirth phases are staggered per index.
    pub cyclone_life_s: f64,
    /// Total fabric wrap a vortex may accumulate at its core (degrees). The
    /// spiral tightens toward this asymptote (tanh) instead of winding
    /// without bound into thread-thin streaks.
    pub cyclone_max_wrap_deg: f64,
    /// Domain rotation about the spin axis (degrees/hour). The shear term is
    /// multiplied by cos^2(latitude), matching WEATHER.md's analytic law.
    pub differential_rotation_deg_h: f64,
    pub differential_rotation_shear_deg_h: f64,
    /// Period (s) of the bounded zonal-shear slosh. The cos^2(lat) shear
    /// phase is A*sin(2*pi*t/P) with A chosen so the t=0 rate matches
    /// differential_rotation_shear_deg_h; unlike shear*t it cannot stretch
    /// the fabric into east-west threads on long clocks.
    pub zonal_shear_period_s: f64,
    /// Parent-attached asymmetric fronts: a narrow leading ridge followed by
    /// a wider trailing smear, both limited along the front's length.
    pub front_strength: f64,
    pub front_leading_km: f64,
    pub front_trailing_km: f64,
    pub front_length_km: f64,
    /// Teleport-map wind visualization. `wind_map_density` is the target
    /// number of comet streamlines in the visible map window; length is the
    /// whole-planet (1x zoom) path length and scales down as the map zooms.
    pub wind_map_density: u32,
    pub wind_map_length_km: f64,
    /// Max precipitation particles at full intensity.
    pub particles_max: u32,
}

/// A tuning file is art direction, not permission to ask the GPU to draw an
/// unbounded instance count. This is deliberately generous (11x the shipped
/// default) while still turning a typo such as 4 billion into a loud fallback.
pub const PARTICLES_MAX_CAP: u32 = 100_000;
/// Fixed shader/uniform budget. `cyclone_count` selects a prefix at runtime;
/// unused lanes stay zero so tuning cannot turn into an unbounded fragment
/// loop.
pub const MAX_CYCLONES: usize = 4;

impl Default for WeatherTuning {
    fn default() -> Self {
        Self {
            storminess: 0.55,
            synoptic_speed: 150.0,
            synoptic_freq: 40.0,
            meso_freq: 320.0,
            planetary_freq: 5.0,
            planetary_strength: 0.32,
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
            cyclone_count: 2,
            cyclone_radius_km: 950.0,
            cyclone_spin_deg_s: 0.75,
            cyclone_track_km_s: 0.35,
            cyclone_cover_boost: 0.48,
            cyclone_precip_boost: 0.68,
            cyclone_arm_count: 2,
            cyclone_arm_strength: 0.55,
            cyclone_arm_twist: 3.0,
            cyclone_arm_spin_deg_s: 0.35,
            cyclone_life_s: 6300.0,
            // ~2 radians of core twist is the most this fabric tolerates:
            // the tangential stretch peaks near 2*wrap*rn^2*exp(-rn^2), so
            // 480 deg already shears fine cloud texture into the concentric
            // thread artifact of the "Cyclone Evolution" photos.
            cyclone_max_wrap_deg: 120.0,
            differential_rotation_deg_h: 18.0,
            differential_rotation_shear_deg_h: 24.0,
            zonal_shear_period_s: 14400.0,
            front_strength: 0.38,
            front_leading_km: 90.0,
            front_trailing_km: 360.0,
            front_length_km: 1800.0,
            wind_map_density: 180,
            wind_map_length_km: 1250.0,
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
            ("planetary_freq", self.planetary_freq),
            ("planetary_strength", self.planetary_strength),
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
            ("cyclone_radius_km", self.cyclone_radius_km),
            ("cyclone_spin_deg_s", self.cyclone_spin_deg_s),
            ("cyclone_track_km_s", self.cyclone_track_km_s),
            ("cyclone_cover_boost", self.cyclone_cover_boost),
            ("cyclone_precip_boost", self.cyclone_precip_boost),
            ("cyclone_life_s", self.cyclone_life_s),
            ("cyclone_max_wrap_deg", self.cyclone_max_wrap_deg),
            ("cyclone_arm_strength", self.cyclone_arm_strength),
            ("cyclone_arm_twist", self.cyclone_arm_twist),
            ("cyclone_arm_spin_deg_s", self.cyclone_arm_spin_deg_s),
            (
                "differential_rotation_deg_h",
                self.differential_rotation_deg_h,
            ),
            (
                "differential_rotation_shear_deg_h",
                self.differential_rotation_shear_deg_h,
            ),
            ("zonal_shear_period_s", self.zonal_shear_period_s),
            ("front_strength", self.front_strength),
            ("front_leading_km", self.front_leading_km),
            ("front_trailing_km", self.front_trailing_km),
            ("front_length_km", self.front_length_km),
            ("wind_map_length_km", self.wind_map_length_km),
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
        if self.planetary_freq <= 0.0 || self.planetary_freq > 64.0 {
            return Err("planetary_freq must be in (0, 64]".into());
        }
        if !(0.0..=1.5).contains(&self.planetary_strength) {
            return Err("planetary_strength must be in [0, 1.5]".into());
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
                "overcast_sun_floor, rain_darken, and rain_crevice_bias must be in [0, 1]".into(),
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
            return Err("cloud altitudes require 0 <= low < mid < high < shell_fade_km".into());
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
        if self.cyclone_count as usize > MAX_CYCLONES {
            return Err(format!("cyclone_count must be in 0..={MAX_CYCLONES}"));
        }
        if self.cyclone_radius_km <= 0.0 || self.cyclone_radius_km > 3000.0 {
            return Err("cyclone_radius_km must be in (0, 3000]".into());
        }
        if self.cyclone_spin_deg_s.abs() > 5.0 {
            return Err("cyclone_spin_deg_s magnitude must be <= 5".into());
        }
        if self.cyclone_track_km_s < 0.0 || self.cyclone_track_km_s > 5.0 {
            return Err("cyclone_track_km_s must be in [0, 5]".into());
        }
        if !(0.0..=1.5).contains(&self.cyclone_cover_boost)
            || !(0.0..=1.5).contains(&self.cyclone_precip_boost)
        {
            return Err("cyclone cover/precip boosts must be in [0, 1.5]".into());
        }
        if !(600.0..=604_800.0).contains(&self.cyclone_life_s) {
            return Err("cyclone_life_s must be in [600, 604800]".into());
        }
        if self.cyclone_arm_count > 8 {
            return Err("cyclone_arm_count must be in 0..=8".into());
        }
        if !(0.0..=1.5).contains(&self.cyclone_arm_strength) {
            return Err("cyclone_arm_strength must be in [0, 1.5]".into());
        }
        if self.cyclone_arm_twist.abs() > 12.0 {
            return Err("cyclone_arm_twist magnitude must be <= 12".into());
        }
        if self.cyclone_arm_spin_deg_s.abs() > 5.0 {
            return Err("cyclone_arm_spin_deg_s magnitude must be <= 5".into());
        }
        if self.cyclone_max_wrap_deg <= 0.0 || self.cyclone_max_wrap_deg > 3600.0 {
            return Err("cyclone_max_wrap_deg must be in (0, 3600]".into());
        }
        if self.differential_rotation_deg_h.abs() > 360.0
            || self.differential_rotation_shear_deg_h.abs() > 360.0
        {
            return Err("differential rotation rates must have magnitude <= 360 deg/h".into());
        }
        if !(60.0..=604_800.0).contains(&self.zonal_shear_period_s) {
            return Err("zonal_shear_period_s must be in [60, 604800]".into());
        }
        if !(0.0..=1.5).contains(&self.front_strength) {
            return Err("front_strength must be in [0, 1.5]".into());
        }
        if self.front_leading_km <= 0.0
            || self.front_trailing_km <= self.front_leading_km
            || self.front_length_km <= self.front_trailing_km
            || self.front_length_km > 6000.0
        {
            return Err("front scales require 0 < leading < trailing < length <= 6000 km".into());
        }
        if !(16..=1024).contains(&self.wind_map_density) {
            return Err("wind_map_density must be in 16..=1024".into());
        }
        if self.wind_map_length_km <= 0.0 || self.wind_map_length_km > 5000.0 {
            return Err("wind_map_length_km must be in (0, 5000]".into());
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
    let edge = if trend_c_per_orbit >= 0.0 {
        thaw_c
    } else {
        freeze_c
    };
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

/// The deck's spatial Layer-2 bridge. At 64 edge-inclusive texels per cube
/// face, one RGBA8 upload is 98,304 bytes. R/G carry cloud cover and
/// precipitation; B is reserved for a future spatial temperature channel and
/// A remains opaque. The byte layout is face-major, then v row, then u.
pub const SYNOPTIC_RASTER_RES: usize = 64;
pub const SYNOPTIC_RASTER_BYTES: usize = 6 * SYNOPTIC_RASTER_RES * SYNOPTIC_RASTER_RES * 4;
/// A deterministic time bucket avoids evaluating 24,576 weather points every
/// frame. It is a pure function of absolute weather time (never frame count or
/// accumulated state), so a capture starting cold chooses the same raster as
/// continuous playback. Two seconds is only ~3 km of default front advection.
pub const SYNOPTIC_RASTER_INTERVAL_S: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynopticRasterSource {
    Off,
    Live {
        seed: i64,
        weather_time_bits: u64,
    },
    Pinned {
        seed: i64,
        cover_bits: u32,
        precip_bits: u32,
    },
}

/// Quantize the rebuild clock without introducing any history. Explicit
/// seeks and replay captures therefore resolve the same source key directly.
pub fn synoptic_raster_time_s(weather_time_s: f64) -> f64 {
    let t = if weather_time_s.is_finite() {
        weather_time_s.max(0.0)
    } else {
        0.0
    };
    let q = (t / SYNOPTIC_RASTER_INTERVAL_S).floor() * SYNOPTIC_RASTER_INTERVAL_S;
    if q == 0.0 { 0.0 } else { q }
}

fn unorm8(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[derive(Clone, Debug)]
pub struct SynopticRaster {
    source: SynopticRasterSource,
    bytes: Vec<u8>,
}

impl Default for SynopticRaster {
    fn default() -> Self {
        Self::off()
    }
}

impl SynopticRaster {
    pub fn off() -> Self {
        Self::uniform_with_source(0.0, 0.0, SynopticRasterSource::Off)
    }

    pub fn pinned(seed: i64, cover: f32, precip: f32) -> Self {
        Self::uniform_with_source(
            cover as f64,
            precip as f64,
            SynopticRasterSource::Pinned {
                seed,
                cover_bits: cover.to_bits(),
                precip_bits: precip.to_bits(),
            },
        )
    }

    fn uniform_with_source(cover: f64, precip: f64, source: SynopticRasterSource) -> Self {
        let texel = [unorm8(cover), unorm8(precip), 0, 255];
        let mut bytes = vec![0; SYNOPTIC_RASTER_BYTES];
        bytes
            .par_chunks_mut(4)
            .for_each(|pixel| pixel.copy_from_slice(&texel));
        Self { source, bytes }
    }

    /// Bake the complete CPU weather field at cube-face texel nodes. This is
    /// a direct, stateless evaluation: the same assets, seed, tuning, and
    /// weather time always produce identical bytes, independent of thread
    /// scheduling or what raster was baked previously.
    pub fn bake_live(
        field: &WeatherField,
        planet: &Planet,
        weather_time_s: f64,
        day_len_s: f64,
        solar_tuning: &crate::orbits::SolarTuning,
        tuning: &WeatherTuning,
    ) -> Self {
        let time = if weather_time_s.is_finite() {
            weather_time_s.max(0.0)
        } else {
            0.0
        };
        let season = season_frac(time, day_len_s, solar_tuning);
        let cyclones = cyclone_systems(field, planet.seed, planet.radius_km, season, time, tuning);
        let mut bytes = vec![0; SYNOPTIC_RASTER_BYTES];
        bytes
            .par_chunks_mut(4)
            .enumerate()
            .for_each(|(index, pixel)| {
                let face_stride = SYNOPTIC_RASTER_RES * SYNOPTIC_RASTER_RES;
                let face = index / face_stride;
                let within = index % face_stride;
                let y = within / SYNOPTIC_RASTER_RES;
                let x = within % SYNOPTIC_RASTER_RES;
                let denom = (SYNOPTIC_RASTER_RES - 1) as f64;
                let u = -1.0 + 2.0 * x as f64 / denom;
                let v = -1.0 + 2.0 * y as f64 / denom;
                let dir = crate::planet::face_dir(face, u, v);
                let weather = weather_at_with_cyclones(
                    field,
                    planet,
                    dir,
                    time,
                    day_len_s,
                    solar_tuning,
                    tuning,
                    &cyclones,
                );
                pixel.copy_from_slice(&[
                    unorm8(weather.cloud_cover),
                    unorm8(weather.precip),
                    0,
                    255,
                ]);
            });
        Self {
            source: SynopticRasterSource::Live {
                seed: planet.seed,
                weather_time_bits: time.to_bits(),
            },
            bytes,
        }
    }

    pub fn source(&self) -> SynopticRasterSource {
        self.source
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn resolution(&self) -> usize {
        SYNOPTIC_RASTER_RES
    }

    pub fn sample(&self, dir: DVec3) -> (f64, f64) {
        let dir = dir.normalize_or_zero();
        if dir == DVec3::ZERO || !dir.is_finite() {
            return (0.0, 0.0);
        }
        let (face, u, v) = crate::planet::face_from_dir(dir);
        self.sample_face(face, u, v)
    }

    /// CPU twin of the shader's edge-inclusive, linearly filtered array
    /// lookup. Keeping this public lets the teleport map display the exact
    /// bytes the cloud deck sees rather than re-evaluating a parallel field.
    pub fn sample_face(&self, face: usize, u: f64, v: f64) -> (f64, f64) {
        let res = SYNOPTIC_RASTER_RES;
        let x = ((u * 0.5 + 0.5) * (res - 1) as f64).clamp(0.0, (res - 1) as f64);
        let y = ((v * 0.5 + 0.5) * (res - 1) as f64).clamp(0.0, (res - 1) as f64);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(res - 1);
        let y1 = (y0 + 1).min(res - 1);
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let at = |xx: usize, yy: usize, channel: usize| {
            let index = ((face * res * res + yy * res + xx) * 4) + channel;
            self.bytes[index] as f64 / 255.0
        };
        let bilinear = |channel| {
            let a = at(x0, y0, channel) * (1.0 - fx) + at(x1, y0, channel) * fx;
            let b = at(x0, y1, channel) * (1.0 - fx) + at(x1, y1, channel) * fx;
            a * (1.0 - fy) + b * fy
        };
        (bilinear(0), bilinear(1))
    }
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
    L_TEMP_A, L_TEMP_B, L_PRC_MEAN, L_PRC_A1, L_PRC_B1, L_CLD_MEAN, L_CLD_A1, L_CLD_B1, L_WIND_E,
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
        Ok(Self {
            res,
            faces,
            temp_harmonics,
            has_seaice: version == 3,
        })
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
        [
            interpolate(&face_layers[layers[0]]),
            interpolate(&face_layers[layers[1]]),
        ]
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
        let temp = if canonical {
            annual
        } else {
            annual + a * cs + b * sn
        };
        let trend = std::f64::consts::TAU * (-a * sn + b * cs);
        (temp, trend)
    }

    /// Sign of the local Fourier temperature derivative (C per orbit).
    /// Positive means the hysteresis loop is on its thawing branch.
    pub fn seasonal_temp_trend(&self, face: usize, u: f64, v: f64, season_frac: f64) -> f64 {
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
    pub fn sea_ice_fraction(&self, face: usize, u: f64, v: f64, season_frac: f64) -> Option<f64> {
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

    /// Annual baked wind at a world direction, in metres per second along
    /// the local east/north basis. The wind map integrates this same
    /// bilinear field and composes current analytic storm drift on top.
    pub fn wind_at(&self, dir: DVec3) -> (f64, f64) {
        let dir = dir.normalize_or_zero();
        if dir == DVec3::ZERO || !dir.is_finite() {
            return (0.0, 0.0);
        }
        let (face, u, v) = crate::planet::face_from_dir(dir);
        (self.at(face, L_WIND_E, u, v), self.at(face, L_WIND_N, u, v))
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

/// One closed-form W-MOTION storm system at the requested weather time.
/// `intensity` is the baked seasonal storm climatology at the moving center
/// times a deterministic finite-life envelope; there is no lifetime counter
/// or mutable storm state — age is a pure function of (t, seed, index).
#[derive(Clone, Copy, Debug, Default)]
pub struct CycloneSystem {
    pub center: DVec3,
    pub intensity: f64,
    /// Orientation of the attached front's leading-edge normal in the local
    /// east/north frame.
    pub front_angle: f64,
    /// Hemisphere-signed accumulated core rotation (radians), saturated at
    /// cyclone_max_wrap_deg so the fabric never winds into endless streaks.
    pub wrap_angle: f64,
    /// Hemisphere-signed spiral-arm pattern phase (radians). Unbounded in
    /// age is safe: a rotating density MASK is rigid in angle-space and
    /// accumulates no shear, and age resets each finite life.
    pub arm_phase: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct CycloneSystems {
    pub count: u32,
    pub systems: [CycloneSystem; MAX_CYCLONES],
}

impl Default for CycloneSystems {
    fn default() -> Self {
        Self {
            count: 0,
            systems: [CycloneSystem::default(); MAX_CYCLONES],
        }
    }
}

/// Stable 53-bit seed hash. Runtime weather never draws from an RNG; every
/// candidate track and front bearing is addressed directly by index/lane.
fn cyclone_hash01(seed: i64, index: u32, lane: u32) -> f64 {
    let mut z = (seed as u64)
        .wrapping_add(0x9E37_79B9_7F4A_7C15u64.wrapping_mul(index as u64 + 1))
        .wrapping_add(0xD1B5_4A32_D192_ED03u64.wrapping_mul(lane as u64 + 1));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

fn seeded_cyclone_track(seed: i64, index: u32, epoch_salt: u32, candidate: u32) -> (f64, f64) {
    // Alternate hemispheres so a small N does not accidentally put every
    // system in one storm belt. Candidate latitude/longitude remain seed-
    // addressed; the climatology chooses among them below. The epoch salt
    // re-rolls the corridors each finite life, so storms visit different
    // parts of their hemisphere over long clocks.
    let north = ((index as u64 + seed as u64) & 1) == 0;
    let sign = if north { 1.0 } else { -1.0 };
    let lane = epoch_salt.wrapping_mul(64).wrapping_add(candidate * 3);
    let latitude = sign * (20.0 + 32.0 * cyclone_hash01(seed, index, lane)).to_radians();
    let longitude = std::f64::consts::TAU * cyclone_hash01(seed, index, lane + 1)
        - std::f64::consts::PI;
    (latitude, longitude)
}

fn cyclone_track_center(
    latitude: f64,
    longitude0: f64,
    t_s: f64,
    radius_km: f64,
    tuning: &WeatherTuning,
) -> DVec3 {
    let cos_lat = latitude.cos();
    let longitude = (longitude0
        + tuning.cyclone_track_km_s * t_s / (radius_km * cos_lat.max(0.15)))
    .rem_euclid(std::f64::consts::TAU);
    DVec3::new(
        cos_lat * longitude.cos(),
        cos_lat * longitude.sin(),
        latitude.sin(),
    )
}

fn climate_cloud_precip_at(field: &WeatherField, dir: DVec3, season_frac: f64) -> (f64, f64) {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let angle = std::f64::consts::TAU * season_frac;
    let (sn1, cs1) = angle.sin_cos();
    let (sn2, cs2) = (2.0 * angle).sin_cos();
    let precip = (field.at(face, L_PRC_MEAN, u, v)
        + field.at(face, L_PRC_A1, u, v) * cs1
        + field.at(face, L_PRC_B1, u, v) * sn1
        + field.at(face, L_PRC_A2, u, v) * cs2
        + field.at(face, L_PRC_B2, u, v) * sn2)
        .max(0.0);
    let cloud = (field.at(face, L_CLD_MEAN, u, v)
        + field.at(face, L_CLD_A1, u, v) * cs1
        + field.at(face, L_CLD_B1, u, v) * sn1
        + field.at(face, L_CLD_A2, u, v) * cs2
        + field.at(face, L_CLD_B2, u, v) * sn2)
        .clamp(0.0, 1.0);
    (cloud, precip)
}

fn storm_climatology(cloud: f64, precip: f64, tuning: &WeatherTuning) -> f64 {
    let wet = (precip / tuning.precip_wet_norm).clamp(0.0, 1.0);
    smooth01((0.72 * cloud + 0.28 * wet - 0.18) / 0.62)
}

/// Resolve the fixed-size system bank for one weather instant. Eight seeded
/// candidate tracks per system are scored against the annual baked storm
/// field, so placement favors storm corridors without storing anything.
/// Current seasonal climatology then modulates presence along the zonal path.
pub fn cyclone_systems(
    field: &WeatherField,
    seed: i64,
    radius_km: f64,
    season_frac: f64,
    t_s: f64,
    tuning: &WeatherTuning,
) -> CycloneSystems {
    let mut out = CycloneSystems {
        count: tuning.cyclone_count.min(MAX_CYCLONES as u32),
        ..CycloneSystems::default()
    };
    let life = tuning.cyclone_life_s.max(1.0);
    for index in 0..out.count {
        // Finite deterministic life: the epoch clock is phase-staggered per
        // index so the bank never turns over all at once, and age is a pure
        // function of (t, seed, index) — a seek lands mid-life correctly.
        let phase = cyclone_hash01(seed, index, 11) * life;
        let cycle_t = t_s + phase;
        let epoch = (cycle_t / life).floor();
        let age_s = cycle_t - epoch * life;
        let epoch_salt = (epoch as i64).rem_euclid(1 << 24) as u32;
        // Grow in over the first ~18% of life, decay over the last ~22%.
        let age01 = age_s / life;
        let lifecycle = smooth01(age01 / 0.18) * (1.0 - smooth01((age01 - 0.78) / 0.22));

        let mut best = (f64::NEG_INFINITY, 0.0, 0.0);
        for candidate in 0..8 {
            let (latitude, longitude) =
                seeded_cyclone_track(seed, index, epoch_salt, candidate);
            let base = cyclone_track_center(latitude, longitude, 0.0, radius_km, tuning);
            let (face, u, v) = crate::planet::face_from_dir(base);
            let cloud = field.at(face, L_CLD_MEAN, u, v).clamp(0.0, 1.0);
            let precip = field.at(face, L_PRC_MEAN, u, v).max(0.0);
            // The tiny final term is only a deterministic tie breaker for a
            // flat/synthetic bake; it cannot outweigh climatology.
            let score = 0.72 * cloud
                + 0.28 * (precip / tuning.precip_wet_norm).clamp(0.0, 1.0)
                + cyclone_hash01(seed, index, epoch_salt.wrapping_mul(64).wrapping_add(candidate * 3 + 2)) * 1e-9;
            if score > best.0 {
                best = (score, latitude, longitude);
            }
        }
        // The track drifts by age, not absolute time: each rebirth starts at
        // the head of its corridor instead of inheriting an epoch of drift.
        let center = cyclone_track_center(best.1, best.2, age_s, radius_km, tuning);
        let (cloud, precip) = climate_cloud_precip_at(field, center, season_frac);
        let hemisphere = if center.z >= 0.0 { 1.0 } else { -1.0 };
        let max_wrap = tuning.cyclone_max_wrap_deg.to_radians();
        let raw_wrap = tuning.cyclone_spin_deg_s.to_radians() * age_s;
        out.systems[index as usize] = CycloneSystem {
            center,
            intensity: storm_climatology(cloud, precip, tuning) * lifecycle,
            front_angle: (cyclone_hash01(seed, index, epoch_salt.wrapping_mul(64).wrapping_add(33))
                - 0.5)
                * 1.8,
            wrap_angle: -hemisphere * max_wrap * (raw_wrap / max_wrap).tanh(),
            arm_phase: -hemisphere * tuning.cyclone_arm_spin_deg_s.to_radians() * age_s,
        };
    }
    out
}

fn rotate_axis(v: DVec3, axis: DVec3, angle: f64) -> DVec3 {
    let (sin, cos) = angle.sin_cos();
    (v * cos + axis.cross(v) * sin + axis * axis.dot(v) * (1.0 - cos)).normalize()
}

/// Bounded zonal-shear phase (radians). shear*t winds the fabric between
/// latitudes without limit — by t~100000 s the whole deck is east-west
/// threads. This slosh keeps the configured shear RATE at its zero
/// crossings while capping total relative displacement at
/// rate * period / (2*pi). Shared by the CPU domain and the GPU uniform so
/// both renderers stay one truth.
pub fn zonal_shear_phase(t_s: f64, tuning: &WeatherTuning) -> f64 {
    let period = tuning.zonal_shear_period_s.max(1.0);
    let rate = tuning.differential_rotation_shear_deg_h.to_radians() / 3600.0;
    let amplitude = rate * period / std::f64::consts::TAU;
    amplitude * (std::f64::consts::TAU * t_s / period).sin()
}

/// Inverse-map the procedural cloud domain into the co-rotating frame of
/// each nearby cyclone, then through the latitude-dependent zonal rotation.
/// This is O(1) in time: a seek evaluates the same closed form as playback.
fn structured_weather_domain(
    dir: DVec3,
    t_s: f64,
    radius_km: f64,
    tuning: &WeatherTuning,
    cyclones: &CycloneSystems,
) -> DVec3 {
    let mut mapped = dir.normalize_or_zero();
    let radius = tuning.cyclone_radius_km / radius_km;
    let radius2 = radius * radius;
    for system in cyclones.systems.iter().take(cyclones.count as usize) {
        if system.intensity <= 1e-6 {
            continue;
        }
        let chord2 = (2.0 * (1.0 - dir.dot(system.center))).max(0.0);
        if chord2 >= radius2 * 9.0 {
            continue;
        }
        let falloff = (-chord2 / radius2).exp() * system.intensity;
        // The bounded per-system wrap replaces rate*t: the spiral tightens
        // toward cyclone_max_wrap_deg and relaxes as the life envelope dies,
        // so long clocks never wind the fabric into thread-thin streaks.
        mapped = rotate_axis(mapped, system.center, system.wrap_angle * falloff);
    }
    let cos2_lat = (1.0 - dir.z * dir.z).max(0.0);
    let theta = tuning.differential_rotation_deg_h.to_radians() * (t_s / 3600.0)
        + zonal_shear_phase(t_s, tuning) * cos2_lat;
    rotate_axis(mapped, DVec3::Z, -theta)
}

#[derive(Clone, Copy, Debug, Default)]
struct StructuredLoads {
    cyclone_signed: f64,
    cyclone_positive: f64,
    front: f64,
}

/// Radial storm/eye profile plus an asymmetric finite front ridge. Chord and
/// tangent coordinates avoid acos in the per-sample path and match the GPU
/// approximation over the configured synoptic radii.
fn structured_loads(
    dir: DVec3,
    radius_km: f64,
    tuning: &WeatherTuning,
    cyclones: &CycloneSystems,
) -> StructuredLoads {
    let mut out = StructuredLoads::default();
    let cyclone_radius = tuning.cyclone_radius_km / radius_km;
    let radius2 = cyclone_radius * cyclone_radius;
    let leading = tuning.front_leading_km / radius_km;
    let trailing = tuning.front_trailing_km / radius_km;
    let length = tuning.front_length_km / radius_km;
    for system in cyclones.systems.iter().take(cyclones.count as usize) {
        if system.intensity <= 1e-6 {
            continue;
        }
        let east0 = DVec3::Z.cross(system.center);
        let east = if east0.length_squared() < 1e-12 {
            DVec3::Y
        } else {
            east0.normalize()
        };
        let north = system.center.cross(east);
        let chord2 = (2.0 * (1.0 - dir.dot(system.center))).max(0.0);
        if chord2 < radius2 * 9.0 {
            let rn2 = chord2 / radius2;
            let rn = rn2.sqrt();
            let envelope = (-rn2).exp();
            let eye = 1.0 - smooth01((rn - 0.04) / 0.14);
            let eyewall = 1.0 - smooth01((rn - 0.28).abs() / 0.16);
            let mut profile = system.intensity * (0.75 * envelope + 0.75 * eyewall - 1.40 * eye);
            // Andrew's spiral density wave: cover is denser along rotating
            // logarithmic arms (thinner between - signed cos). Only the
            // PATTERN moves; the fabric does not, so cloud cells seed and
            // disperse as an arm sweeps over them. Arms live outside the
            // eyewall and fade with the storm envelope.
            if tuning.cyclone_arm_count > 0 {
                let offset = dir - system.center * dir.dot(system.center);
                let azimuth = offset.dot(north).atan2(offset.dot(east));
                let hemisphere = if system.center.z >= 0.0 { 1.0 } else { -1.0 };
                let wave = (tuning.cyclone_arm_count as f64 * azimuth
                    + hemisphere * tuning.cyclone_arm_twist * rn.max(0.15).ln()
                    - system.arm_phase)
                    .cos();
                // signed sharpening: narrower, more legible arm crests
                let spiral = wave * wave.abs();
                let arm_env = envelope * smooth01((rn - 0.30) / 0.25);
                profile += system.intensity * tuning.cyclone_arm_strength * spiral * arm_env;
            }
            out.cyclone_signed += profile;
            out.cyclone_positive += profile.max(0.0);
        }

        let normal = east * system.front_angle.cos() + north * system.front_angle.sin();
        let along_axis = system.center.cross(normal).normalize();
        let cross = dir.dot(normal) - cyclone_radius * 0.58;
        let along = dir.dot(along_axis);
        if along.abs() < length * 1.8 && cross.abs() < trailing * 3.0 {
            let width = if cross >= 0.0 { leading } else { trailing };
            let cross_profile = 1.0 - smooth01(cross.abs() / (width * 1.5));
            let along_profile = 1.0 - smooth01((along.abs() / length - 0.65) / 0.65);
            out.front += system.intensity * cross_profile * along_profile;
        }
    }
    out.cyclone_signed = out.cyclone_signed.clamp(-1.0, 2.0);
    out.cyclone_positive = out.cyclone_positive.clamp(0.0, 2.0);
    out.front = out.front.clamp(0.0, 1.5);
    out
}

/// Tangent wind used by the teleport-map streamline layer. The climatology
/// remains the physical base; bounded visual drift from the same current
/// cyclone bank and differential rotation that move the cloud structures is
/// composed on top. This is a pure vector field, not particle accumulation.
pub fn synoptic_wind_tangent_with_cyclones(
    field: &WeatherField,
    dir: DVec3,
    radius_km: f64,
    tuning: &WeatherTuning,
    cyclones: &CycloneSystems,
) -> DVec3 {
    let dir = dir.normalize_or_zero();
    if dir == DVec3::ZERO || !dir.is_finite() {
        return DVec3::ZERO;
    }
    let east0 = DVec3::Z.cross(dir);
    let east = if east0.length_squared() < 1e-12 {
        DVec3::Y
    } else {
        east0.normalize()
    };
    let north = dir.cross(east);
    let (wind_e, wind_n) = field.wind_at(dir);
    let mut tangent = east * wind_e + north * wind_n;

    let cyclone_radius = (tuning.cyclone_radius_km / radius_km.max(1e-6)).max(1e-9);
    let radius2 = cyclone_radius * cyclone_radius;
    // The presentation spin is intentionally accelerated, so converting its
    // angular rate literally would yield kilometre-per-second arrows. Keep
    // its direction and relative tuning while bounding the map contribution
    // to a meteorological 0..36 m/s range.
    let spin_mps = (24.0 * (tuning.cyclone_spin_deg_s / 0.75)).clamp(-36.0, 36.0);
    for system in cyclones.systems.iter().take(cyclones.count as usize) {
        if system.intensity <= 1e-6 {
            continue;
        }
        let chord2 = (2.0 * (1.0 - dir.dot(system.center))).max(0.0);
        if chord2 >= radius2 * 9.0 {
            continue;
        }
        let falloff = (-chord2 / radius2).exp() * system.intensity;
        let around = system.center.cross(dir).normalize_or_zero();
        let hemisphere = if system.center.z >= 0.0 { 1.0 } else { -1.0 };
        tangent += around * (hemisphere * spin_mps * falloff);
        // A small convergent component makes paths curl into rain bands
        // instead of drawing perfect circles around every analytic center.
        let inward = (system.center - dir * system.center.dot(dir)).normalize_or_zero();
        tangent += inward * (4.0 * falloff);
    }

    let cos2_lat = (1.0 - dir.z * dir.z).max(0.0);
    let rotation_deg_h =
        tuning.differential_rotation_deg_h + tuning.differential_rotation_shear_deg_h * cos2_lat;
    tangent + east * (rotation_deg_h / 42.0 * 4.0).clamp(-8.0, 8.0)
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
    let t_yr = season_frac(t_s, day_len_s, solar_tuning);
    let cyclones = cyclone_systems(field, planet.seed, planet.radius_km, t_yr, t_s, tuning);
    weather_at_with_cyclones(
        field,
        planet,
        dir,
        t_s,
        day_len_s,
        solar_tuning,
        tuning,
        &cyclones,
    )
}

/// Hot path for a frame/map sweep: callers resolve the immutable cyclone
/// bank once and reuse it for the camera, deck mean, and W2 compass probes.
pub fn weather_at_with_cyclones(
    field: &WeatherField,
    planet: &Planet,
    dir: DVec3,
    t_s: f64,
    day_len_s: f64,
    solar_tuning: &crate::orbits::SolarTuning,
    tuning: &WeatherTuning,
    cyclones: &CycloneSystems,
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
    let east = if east0.length_squared() < 1e-9 {
        DVec3::Y
    } else {
        east0.normalize()
    };
    let north = dir.cross(east);
    let drift = (east * wind_e + north * wind_n)
        * (tuning.synoptic_speed * t_s / (1000.0 * planet.radius_km));
    let structured = structured_weather_domain(dir, t_s, planet.radius_km, tuning, cyclones);
    let adv = advect_great_circle(structured, drift);
    let seed = planet.seed;
    let synoptic = fbm_band(adv, 0, 3, tuning.synoptic_freq, seed.wrapping_add(80081));
    // the fine texture drifts a little faster (gust fronts outrun systems)
    let adv2 = advect_great_circle(dir, drift * 1.6);
    let meso = fbm_band(adv2, 0, 2, tuning.meso_freq, seed.wrapping_add(90091));
    // the planetary wave crawls: continent-scale breaks and dense belts
    // migrate slower than the synoptic systems embedded in them
    let adv3 = advect_great_circle(dir, drift * 0.55);
    let planetary = fbm_band(adv3, 0, 2, tuning.planetary_freq, seed.wrapping_add(70071));

    let structure = structured_loads(dir, planet.radius_km, tuning, cyclones);
    let raw = cld
        + tuning.planetary_strength * planetary
        + tuning.storminess * synoptic
        + 0.18 * meso
        + tuning.cyclone_cover_boost * structure.cyclone_signed
        + tuning.front_strength * structure.front;
    let cover = smooth01((raw - tuning.cover_lo) / (tuning.cover_hi - tuning.cover_lo));

    // precipitation: falls out of heavy cover, scaled by how wet this
    // climate is right now (a desert overcast passes dry)
    let wetness = (prc / tuning.precip_wet_norm).clamp(0.0, 1.5);
    let over = ((cover - tuning.precip_threshold) / (1.0 - tuning.precip_threshold)).max(0.0);
    let structured_precip = wetness.min(1.0)
        * (tuning.cyclone_precip_boost * structure.cyclone_positive
            + tuning.front_strength * 0.75 * structure.front);
    let precip = (over.powf(tuning.precip_gamma) * wetness + structured_precip).clamp(0.0, 1.0);
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
        storm: (synoptic + 0.55 * structure.cyclone_positive + 0.35 * structure.front)
            .clamp(-1.0, 1.0),
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

    fn synoptic_weather_bytes() -> Vec<u8> {
        const RES: usize = 4;
        let mut raw = Vec::new();
        raw.extend_from_slice(b"WEA3");
        raw.extend_from_slice(&(RES as u32).to_le_bytes());
        raw.extend_from_slice(&(LAYERS as u32).to_le_bytes());
        for face in 0..6 {
            for layer in 0..LAYERS {
                for y in 0..RES {
                    for x in 0..RES {
                        let spatial = face as f32 * 0.025 + x as f32 * 0.012 + y as f32 * 0.008;
                        let value = match layer {
                            L_PRC_MEAN => 105.0 + spatial * 80.0,
                            L_CLD_MEAN => 0.34 + spatial,
                            L_WIND_E => 4.0 + face as f32 * 0.6 + y as f32 * 0.2,
                            L_WIND_N => -2.0 + x as f32 * 0.7,
                            L_SEAICE_0..=25 => 0.0,
                            _ => 0.0,
                        };
                        raw.extend_from_slice(&value.to_le_bytes());
                    }
                }
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
        assert_eq!(field.sea_ice_fraction(0, 0.0, 0.0, 0.5 / 12.0), Some(1.0));
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
            r#"{"cyclone_count":5}"#,
            r#"{"cyclone_radius_km":0}"#,
            r#"{"cyclone_spin_deg_s":5.01}"#,
            r#"{"cyclone_track_km_s":-0.01}"#,
            r#"{"cyclone_cover_boost":1.51}"#,
            r#"{"cyclone_precip_boost":-0.01}"#,
            r#"{"differential_rotation_deg_h":361}"#,
            r#"{"differential_rotation_shear_deg_h":-361}"#,
            r#"{"front_strength":1.51}"#,
            r#"{"front_leading_km":0}"#,
            r#"{"front_leading_km":400,"front_trailing_km":300}"#,
            r#"{"front_trailing_km":400,"front_length_km":300}"#,
            r#"{"wind_map_density":15}"#,
            r#"{"wind_map_density":1025}"#,
            r#"{"wind_map_length_km":0}"#,
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
    fn wmotion2_defaults_are_bounded_and_asymmetric() {
        let t = WeatherTuning::default();
        assert!((t.cyclone_count as usize) <= MAX_CYCLONES);
        assert!(t.cyclone_radius_km > 0.0);
        assert!(t.cyclone_spin_deg_s.abs() > 0.0);
        assert!(t.cyclone_track_km_s > 0.0);
        assert!(t.front_leading_km < t.front_trailing_km);
        assert!(t.front_trailing_km < t.front_length_km);
        assert!((16..=1024).contains(&t.wind_map_density));
        assert!(t.wind_map_length_km > 0.0);
        assert!(t.validate().is_ok());
    }

    #[test]
    fn synoptic_raster_is_byte_deterministic_and_matches_weather_texels() {
        let field = WeatherField::from_bytes(&synoptic_weather_bytes()).unwrap();
        let planet = crate::planet::weather_test_planet(42);
        let tuning = WeatherTuning::default();
        let solar = crate::orbits::SolarTuning::default();
        let time = 1234.5;
        let a =
            SynopticRaster::bake_live(&field, &planet, time, solar.day_length_s, &solar, &tuning);
        let b =
            SynopticRaster::bake_live(&field, &planet, time, solar.day_length_s, &solar, &tuning);
        assert_eq!(a.bytes(), b.bytes());
        assert_eq!(a.bytes().len(), SYNOPTIC_RASTER_BYTES);
        assert_eq!(
            a.source(),
            SynopticRasterSource::Live {
                seed: 42,
                weather_time_bits: time.to_bits(),
            }
        );

        let tolerance = 0.5 / 255.0 + 1e-12;
        for face in 0..6 {
            for &(x, y) in &[(0usize, 0usize), (13, 29), (37, 51), (63, 63)] {
                let denom = (SYNOPTIC_RASTER_RES - 1) as f64;
                let u = -1.0 + 2.0 * x as f64 / denom;
                let v = -1.0 + 2.0 * y as f64 / denom;
                let dir = crate::planet::face_dir(face, u, v);
                let expected = weather_at(
                    &field,
                    &planet,
                    dir,
                    time,
                    solar.day_length_s,
                    &solar,
                    &tuning,
                );
                let got = a.sample_face(face, u, v);
                assert!(
                    (got.0 - expected.cloud_cover).abs() <= tolerance,
                    "face {face} ({x},{y}) cover {} != {}",
                    got.0,
                    expected.cloud_cover,
                );
                assert!(
                    (got.1 - expected.precip).abs() <= tolerance,
                    "face {face} ({x},{y}) precip {} != {}",
                    got.1,
                    expected.precip,
                );
            }
        }
    }

    #[test]
    fn pinned_synoptic_raster_is_uniform_and_time_independent() {
        let a = SynopticRaster::pinned(42, 0.65, 0.2);
        let b = SynopticRaster::pinned(42, 0.65, 0.2);
        assert_eq!(a.bytes(), b.bytes());
        for pixel in a.bytes().chunks_exact(4) {
            assert_eq!(pixel, &a.bytes()[0..4]);
        }
        let center = a.sample(DVec3::new(1.0, 0.2, -0.4));
        assert!((center.0 - unorm8(0.65) as f64 / 255.0).abs() < 1e-12);
        assert!((center.1 - unorm8(0.2) as f64 / 255.0).abs() < 1e-12);
    }

    #[test]
    fn map_wind_field_is_deterministic_finite_and_tangent() {
        let field = WeatherField::from_bytes(&synoptic_weather_bytes()).unwrap();
        let planet = crate::planet::weather_test_planet(42);
        let tuning = WeatherTuning::default();
        let systems = cyclone_systems(&field, 42, planet.radius_km, 0.45, 3500.0, &tuning);
        for dir in [
            DVec3::new(1.0, 0.2, 0.1).normalize(),
            DVec3::new(-0.3, 0.8, -0.4).normalize(),
            DVec3::new(0.01, 0.02, 1.0).normalize(),
        ] {
            let a = synoptic_wind_tangent_with_cyclones(
                &field,
                dir,
                planet.radius_km,
                &tuning,
                &systems,
            );
            let b = synoptic_wind_tangent_with_cyclones(
                &field,
                dir,
                planet.radius_km,
                &tuning,
                &systems,
            );
            assert_eq!(
                a.to_array().map(f64::to_bits),
                b.to_array().map(f64::to_bits)
            );
            assert!(a.is_finite());
            assert!(a.length() > 0.05);
            assert!(a.dot(dir).abs() < 1e-10, "wind left tangent plane: {a:?}");
        }
    }

    #[test]
    fn cyclone_centers_are_deterministic_at_two_times_and_cross_hour_smoothly() {
        let field = WeatherField::from_bytes(&weather_bytes(b"WEA3", LAYERS)).unwrap();
        let tuning = WeatherTuning::default();
        let sample = |time| cyclone_systems(&field, 42, 6371.0, 0.45, time, &tuning);
        let at_zero_a = sample(0.0);
        let at_zero_b = sample(0.0);
        let at_later_a = sample(1234.5);
        let at_later_b = sample(1234.5);
        for index in 0..tuning.cyclone_count as usize {
            assert_eq!(
                at_zero_a.systems[index].center.to_array().map(f64::to_bits),
                at_zero_b.systems[index].center.to_array().map(f64::to_bits),
            );
            assert_eq!(
                at_later_a.systems[index]
                    .center
                    .to_array()
                    .map(f64::to_bits),
                at_later_b.systems[index]
                    .center
                    .to_array()
                    .map(f64::to_bits),
            );
            assert!((at_zero_a.systems[index].center.length() - 1.0).abs() < 1e-12);
            assert!((at_later_a.systems[index].center.length() - 1.0).abs() < 1e-12);
            assert!(
                at_zero_a.systems[index]
                    .center
                    .distance(at_later_a.systems[index].center)
                    > 1e-6
            );
        }
        let before = sample(3599.999).systems[0].center;
        let after = sample(3600.001).systems[0].center;
        assert!(before.distance(after) < 1e-6, "hour boundary jumped");
        let probe = (before + DVec3::Y * 0.04).normalize();
        let systems_before = sample(3599.999);
        let systems_after = sample(3600.001);
        let domain_before =
            structured_weather_domain(probe, 3599.999, 6371.0, &tuning, &systems_before);
        let domain_after =
            structured_weather_domain(probe, 3600.001, 6371.0, &tuning, &systems_after);
        assert!(
            domain_before.distance(domain_after) < 1e-4,
            "structured domain reset at the hourly presentation wrap"
        );
        let loads_before = structured_loads(probe, 6371.0, &tuning, &systems_before);
        let loads_after = structured_loads(probe, 6371.0, &tuning, &systems_after);
        assert!((loads_before.cyclone_signed - loads_after.cyclone_signed).abs() < 1e-4);
        assert!((loads_before.front - loads_after.front).abs() < 1e-4);
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
                let effective =
                    phase + (angle / std::f64::consts::TAU).floor() * std::f64::consts::TAU;
                assert!((effective - angle).abs() < 1e-12);
                assert!((got.length() - 1.0).abs() < 1e-12);
            }
        }
    }
}

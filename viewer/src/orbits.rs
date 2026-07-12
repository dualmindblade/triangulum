//! Stateless solar-system frames for SOLAR.md P1.
//!
//! All orbital and body-frame math is f64 and is evaluated directly from an
//! absolute game time. There is no integration and no retained ephemeris
//! state: seeking to a time costs the same as advancing one frame. Rendering
//! converts the resulting body positions to camera-relative f32 only after
//! subtracting the f64 camera position in `renderer.rs`.

use glam::{DQuat, DVec3};

const TAU: f64 = std::f64::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyId {
    Sun,
    Neisor,
    Moon,
}

impl BodyId {
    pub const ALL: [Self; 3] = [Self::Sun, Self::Neisor, Self::Moon];

    pub const fn numeric_id(self) -> f64 {
        match self {
            Self::Neisor => 0.0,
            Self::Moon => 1.0,
            Self::Sun => 2.0,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeplerElements {
    /// Semi-major axis in Neisor radii. Conversion to km happens at the
    /// frame boundary so the same tuning file works with another bake scale.
    pub semi_major_axis_neisor_radii: f64,
    /// Elliptic eccentricity, 0 <= e < 1.
    pub eccentricity: f64,
    /// Classical orientation angles in the parent's inertial frame.
    pub inclination_deg: f64,
    pub longitude_ascending_node_deg: f64,
    pub argument_periapsis_deg: f64,
    /// Mean anomaly at absolute t=0. Neisor's value is also the one and only
    /// weather/season epoch phase.
    pub mean_anomaly_at_epoch_deg: f64,
    /// Orbital period measured in tunable game days.
    pub period_days: f64,
}

impl Default for KeplerElements {
    fn default() -> Self {
        Self {
            semi_major_axis_neisor_radii: 1.0,
            eccentricity: 0.0,
            inclination_deg: 0.0,
            longitude_ascending_node_deg: 0.0,
            argument_periapsis_deg: 0.0,
            mean_anomaly_at_epoch_deg: 0.0,
            period_days: 1.0,
        }
    }
}

impl KeplerElements {
    fn validate(&self, name: &str) -> Result<(), String> {
        let finite = [
            (
                "semi_major_axis_neisor_radii",
                self.semi_major_axis_neisor_radii,
            ),
            ("eccentricity", self.eccentricity),
            ("inclination_deg", self.inclination_deg),
            (
                "longitude_ascending_node_deg",
                self.longitude_ascending_node_deg,
            ),
            ("argument_periapsis_deg", self.argument_periapsis_deg),
            ("mean_anomaly_at_epoch_deg", self.mean_anomaly_at_epoch_deg),
            ("period_days", self.period_days),
        ];
        if let Some((field, _)) = finite.into_iter().find(|(_, value)| !value.is_finite()) {
            return Err(format!("{name}.{field} must be finite"));
        }
        if self.semi_major_axis_neisor_radii <= 0.0 {
            return Err(format!("{name}.semi_major_axis_neisor_radii must be > 0"));
        }
        if !(0.0..1.0).contains(&self.eccentricity) {
            return Err(format!("{name}.eccentricity must be in [0, 1)"));
        }
        if self.period_days <= 0.0 {
            return Err(format!("{name}.period_days must be > 0"));
        }
        Ok(())
    }

    pub fn mean_anomaly_rad(&self, absolute_t_s: f64, day_length_s: f64) -> f64 {
        let elapsed_days = finite_time(absolute_t_s) / effective_day_length(day_length_s);
        (self.mean_anomaly_at_epoch_deg.to_radians() + TAU * elapsed_days / self.period_days)
            .rem_euclid(TAU)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BodyTuning {
    pub parent: Option<BodyId>,
    pub radius_neisor: f64,
    pub orbit: Option<KeplerElements>,
}

impl Default for BodyTuning {
    fn default() -> Self {
        Self {
            parent: None,
            radius_neisor: 1.0,
            orbit: None,
        }
    }
}

/// Art-directable solar-system parameters. Defaults are duplicated in
/// `viewer/assets/solar_tuning.json`; a missing or invalid file falls back as
/// one unit, matching the established weather-tuning behavior.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SolarTuning {
    /// Seconds in one game day (Andrew D-5: 30 minutes by default).
    pub day_length_s: f64,
    /// Neisor's obliquity. The solar declination is obtained by rotating the
    /// actual Sun-Neisor vector through this angle, never from a second sine.
    pub axial_tilt_deg: f64,
    /// Prime-meridian angle at t=0. The renderer may add a session reference
    /// longitude, but it never changes the season/orbit phase.
    pub rotation_phase_deg: f64,
    pub sun: BodyTuning,
    pub neisor: BodyTuning,
    pub moon: BodyTuning,
    pub sun_tint: [f64; 3],
    pub sun_halo_strength: f64,
    pub moon_tint: [f64; 3],
    pub moon_copper_tint: [f64; 3],
}

impl Default for SolarTuning {
    fn default() -> Self {
        Self {
            day_length_s: 1800.0,
            axial_tilt_deg: 23.0,
            // Together with main.rs' default 30 deg reference longitude this
            // keeps t=0 close to the old mid-morning presentation.
            rotation_phase_deg: 15.8,
            sun: BodyTuning {
                parent: None,
                // Real Sun/Earth radius ratio, with a compressed orbital
                // scale chosen so the physical limb matches the old 0.51 deg
                // shader core from Neisor's surface.
                radius_neisor: 109.0,
                orbit: None,
            },
            neisor: BodyTuning {
                parent: Some(BodyId::Sun),
                radius_neisor: 1.0,
                orbit: Some(KeplerElements {
                    semi_major_axis_neisor_radii: 12_190.0,
                    eccentricity: 0.035,
                    inclination_deg: 0.0,
                    longitude_ascending_node_deg: 0.0,
                    argument_periapsis_deg: 42.92,
                    // 0.45 preserves the established starting weather season.
                    mean_anomaly_at_epoch_deg: 162.0,
                    period_days: 84.0,
                }),
            },
            moon: BodyTuning {
                parent: Some(BodyId::Neisor),
                radius_neisor: 0.27,
                orbit: Some(KeplerElements {
                    // Visually compressed: at mean distance the 0.27 R body
                    // subtends the old sky reel's ~0.021 rad radius.
                    semi_major_axis_neisor_radii: 13.86,
                    eccentricity: 0.08,
                    inclination_deg: 4.8,
                    longitude_ascending_node_deg: 225.0126009,
                    argument_periapsis_deg: -122.6728680,
                    mean_anomaly_at_epoch_deg: 90.0,
                    period_days: 7.0,
                }),
            },
            sun_tint: [1.0, 0.96, 0.86],
            sun_halo_strength: 0.50,
            moon_tint: [0.86, 0.89, 0.97],
            moon_copper_tint: [0.76, 0.25, 0.08],
        }
    }
}

impl SolarTuning {
    pub fn load(assets_dir: &str) -> Self {
        let path = format!("{assets_dir}/solar_tuning.json");
        match std::fs::read_to_string(&path) {
            Ok(raw) => Self::from_json(&raw, &path),
            Err(_) => Self::default(),
        }
    }

    fn from_json(raw: &str, path: &str) -> Self {
        match serde_json::from_str::<Self>(raw) {
            Ok(tuning) => match tuning.validate() {
                Ok(()) => {
                    println!("solar tuning: {path}");
                    tuning
                }
                Err(error) => {
                    eprintln!("solar tuning ignored ({path}: invalid {error}); using defaults");
                    Self::default()
                }
            },
            Err(error) => {
                eprintln!("solar tuning ignored ({path}: {error}); using defaults");
                Self::default()
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.day_length_s.is_finite() || self.day_length_s <= 0.0 {
            return Err("day_length_s must be finite and > 0".into());
        }
        if !self.axial_tilt_deg.is_finite() || self.axial_tilt_deg.abs() >= 90.0 {
            return Err("axial_tilt_deg must be finite and in (-90, 90)".into());
        }
        if !self.rotation_phase_deg.is_finite() {
            return Err("rotation_phase_deg must be finite".into());
        }
        if self.sun.parent.is_some() || self.sun.orbit.is_some() {
            return Err("sun must be the root (no parent or orbit)".into());
        }
        if self.neisor.parent != Some(BodyId::Sun) || self.neisor.orbit.is_none() {
            return Err("neisor must orbit sun".into());
        }
        if self.moon.parent != Some(BodyId::Neisor) || self.moon.orbit.is_none() {
            return Err("moon must orbit neisor".into());
        }
        for (name, body) in [
            ("sun", &self.sun),
            ("neisor", &self.neisor),
            ("moon", &self.moon),
        ] {
            if !body.radius_neisor.is_finite() || body.radius_neisor <= 0.0 {
                return Err(format!("{name}.radius_neisor must be finite and > 0"));
            }
            if let Some(orbit) = &body.orbit {
                orbit.validate(&format!("{name}.orbit"))?;
            }
        }
        let neisor_orbit = self.neisor.orbit.as_ref().unwrap();
        if neisor_orbit.semi_major_axis_neisor_radii * (1.0 - neisor_orbit.eccentricity)
            <= self.sun.radius_neisor + self.neisor.radius_neisor
        {
            return Err("neisor orbit intersects sun".into());
        }
        let moon_orbit = self.moon.orbit.as_ref().unwrap();
        if moon_orbit.semi_major_axis_neisor_radii * (1.0 - moon_orbit.eccentricity)
            <= self.neisor.radius_neisor + self.moon.radius_neisor
        {
            return Err("moon orbit intersects neisor".into());
        }
        for (name, color) in [
            ("sun_tint", self.sun_tint),
            ("moon_tint", self.moon_tint),
            ("moon_copper_tint", self.moon_copper_tint),
        ] {
            if color
                .into_iter()
                .any(|v| !v.is_finite() || !(0.0..=4.0).contains(&v))
            {
                return Err(format!("{name} components must be finite and in [0, 4]"));
            }
        }
        if !self.sun_halo_strength.is_finite() || !(0.0..=4.0).contains(&self.sun_halo_strength) {
            return Err("sun_halo_strength must be finite and in [0, 4]".into());
        }
        Ok(())
    }

    pub fn body(&self, id: BodyId) -> &BodyTuning {
        match id {
            BodyId::Sun => &self.sun,
            BodyId::Neisor => &self.neisor,
            BodyId::Moon => &self.moon,
        }
    }

    pub fn radius_km(&self, id: BodyId, neisor_radius_km: f64) -> f64 {
        self.body(id).radius_neisor * neisor_radius_km
    }

    pub fn year_days(&self) -> f64 {
        self.neisor.orbit.as_ref().unwrap().period_days
    }

    pub fn lunar_days(&self) -> f64 {
        self.moon.orbit.as_ref().unwrap().period_days
    }

    /// The one season phase: Neisor's mean anomaly. Weather Fourier terms,
    /// orbital position, and the geometry-derived declination all read this
    /// same value and absolute time.
    pub fn season_frac(&self, absolute_t_s: f64, day_length_s: f64) -> f64 {
        self.neisor
            .orbit
            .as_ref()
            .unwrap()
            .mean_anomaly_rad(absolute_t_s, day_length_s)
            / TAU
    }

    /// Retarget the epoch phase without introducing a second clock. This is
    /// the legacy `weather season FRAC` art command: changing Neisor's mean
    /// anomaly moves weather, declination, and every body together.
    pub fn set_season_frac(&mut self, absolute_t_s: f64, day_length_s: f64, target: f64) {
        let day = effective_day_length(day_length_s);
        let orbit = self.neisor.orbit.as_mut().unwrap();
        let elapsed = finite_time(absolute_t_s) / (day * orbit.period_days);
        orbit.mean_anomaly_at_epoch_deg =
            ((target.rem_euclid(1.0) - elapsed).rem_euclid(1.0) * 360.0).rem_euclid(360.0);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SolarState {
    /// Inertial hierarchy positions relative to the root Sun, km.
    pub inertial_sun_km: DVec3,
    pub inertial_neisor_km: DVec3,
    pub inertial_moon_km: DVec3,
    /// Positions in Neisor's rotating, tilted terrain frame, relative to
    /// Neisor's center. These are the positions consumed by camera/rendering.
    pub sun_km: DVec3,
    pub moon_km: DVec3,
    pub season_frac: f64,
    pub sun_declination_rad: f64,
}

impl SolarState {
    pub fn position_km(self, id: BodyId) -> DVec3 {
        match id {
            BodyId::Sun => self.sun_km,
            BodyId::Neisor => DVec3::ZERO,
            BodyId::Moon => self.moon_km,
        }
    }

    pub fn rotated_about_neisor(self, rotation: DQuat) -> Self {
        let sun_km = rotation * self.sun_km;
        let moon_km = rotation * self.moon_km;
        Self {
            sun_km,
            moon_km,
            sun_declination_rad: sun_km.normalize().z.asin(),
            ..self
        }
    }

    /// Explicit capture/local-noon override: rotate the complete visible
    /// frame about the observer, preserving all Sun-moon distances and angles
    /// as seen from that observer while placing the requested Sun direction
    /// exactly (important for legacy photo/sky-reel reproduction).
    pub fn rotated_about_observer(self, observer_km: DVec3, rotation: DQuat) -> Self {
        let sun_km = observer_km + rotation * (self.sun_km - observer_km);
        let moon_km = observer_km + rotation * (self.moon_km - observer_km);
        Self {
            sun_km,
            moon_km,
            sun_declination_rad: sun_km.normalize().z.asin(),
            ..self
        }
    }
}

/// Evaluate the complete hierarchy at an absolute orbit time and (possibly
/// phase-offset) rotation time. Photo restore only offsets rotation time; it
/// never forks the year/season/orbit coordinate.
pub fn state_at(
    tuning: &SolarTuning,
    absolute_t_s: f64,
    rotation_time_s: f64,
    neisor_radius_km: f64,
) -> SolarState {
    let day_length_s = effective_day_length(tuning.day_length_s);
    state_at_with_day_length(
        tuning,
        absolute_t_s,
        rotation_time_s,
        day_length_s,
        neisor_radius_km,
    )
}

pub fn state_at_with_day_length(
    tuning: &SolarTuning,
    absolute_t_s: f64,
    rotation_time_s: f64,
    day_length_s: f64,
    neisor_radius_km: f64,
) -> SolarState {
    let sun = DVec3::ZERO;
    let neisor = kepler_position_km(
        tuning.neisor.orbit.as_ref().unwrap(),
        absolute_t_s,
        day_length_s,
        neisor_radius_km,
    );
    let moon = neisor
        + kepler_position_km(
            tuning.moon.orbit.as_ref().unwrap(),
            absolute_t_s,
            day_length_s,
            neisor_radius_km,
        );

    // Ecliptic inertial -> equatorial inertial -> rotating planet-fixed.
    // z after the tilt is the physical solar declination; the daily z
    // rotation changes longitude only.
    let tilt = DQuat::from_rotation_x(tuning.axial_tilt_deg.to_radians());
    let spin_angle = tuning.rotation_phase_deg.to_radians()
        - TAU * finite_time(rotation_time_s) / effective_day_length(day_length_s);
    let inertial_to_fixed = DQuat::from_rotation_z(spin_angle) * tilt;
    let sun_km = inertial_to_fixed * (sun - neisor);
    let moon_km = inertial_to_fixed * (moon - neisor);
    SolarState {
        inertial_sun_km: sun,
        inertial_neisor_km: neisor,
        inertial_moon_km: moon,
        sun_km,
        moon_km,
        season_frac: tuning.season_frac(absolute_t_s, day_length_s),
        sun_declination_rad: sun_km.normalize().z.asin(),
    }
}

/// Classical elliptic Kepler orbit, evaluated in constant work. Eight Newton
/// steps give far more precision than rendering needs throughout the allowed
/// e<1 tuning range while keeping the cost strictly bounded.
pub fn kepler_position_km(
    elements: &KeplerElements,
    absolute_t_s: f64,
    day_length_s: f64,
    neisor_radius_km: f64,
) -> DVec3 {
    let mean = elements.mean_anomaly_rad(absolute_t_s, day_length_s);
    let e = elements.eccentricity;
    let mut eccentric = if e < 0.8 { mean } else { std::f64::consts::PI };
    for _ in 0..8 {
        let residual = eccentric - e * eccentric.sin() - mean;
        eccentric -= residual / (1.0 - e * eccentric.cos());
    }
    let a = elements.semi_major_axis_neisor_radii * neisor_radius_km;
    let local = DVec3::new(
        a * (eccentric.cos() - e),
        a * (1.0 - e * e).sqrt() * eccentric.sin(),
        0.0,
    );
    let orient = DQuat::from_rotation_z(elements.longitude_ascending_node_deg.to_radians())
        * DQuat::from_rotation_x(elements.inclination_deg.to_radians())
        * DQuat::from_rotation_z(elements.argument_periapsis_deg.to_radians());
    orient * local
}

/// Fraction of a circular source disc covered by a circular occulter. Radii
/// and separation are angular radians in their common tangent plane. This is
/// the exact two-circle intersection area used for the deterministic penumbra.
pub fn disc_overlap_fraction(source_radius: f64, occulter_radius: f64, separation: f64) -> f64 {
    let (r, q, d) = (source_radius.abs(), occulter_radius.abs(), separation.abs());
    if !r.is_finite() || !q.is_finite() || !d.is_finite() || r <= 0.0 || q <= 0.0 {
        return 0.0;
    }
    if d >= r + q {
        return 0.0;
    }
    if d <= (q - r).abs() {
        return if q >= r {
            1.0
        } else {
            (q * q / (r * r)).clamp(0.0, 1.0)
        };
    }
    let ar = ((d * d + r * r - q * q) / (2.0 * d * r))
        .clamp(-1.0, 1.0)
        .acos();
    let aq = ((d * d + q * q - r * r) / (2.0 * d * q))
        .clamp(-1.0, 1.0)
        .acos();
    let lens = r * r * ar + q * q * aq
        - 0.5
            * ((-d + r + q) * (d + r - q) * (d - r + q) * (d + r + q))
                .max(0.0)
                .sqrt();
    (lens / (std::f64::consts::PI * r * r)).clamp(0.0, 1.0)
}

fn angular_radius(radius_km: f64, vector_km: DVec3) -> f64 {
    (radius_km / vector_km.length().max(radius_km))
        .clamp(0.0, 1.0)
        .asin()
}

fn angular_separation(a: DVec3, b: DVec3) -> f64 {
    a.normalize_or_zero()
        .dot(b.normalize_or_zero())
        .clamp(-1.0, 1.0)
        .acos()
}

pub fn solar_occlusion_at(
    observer_km: DVec3,
    state: SolarState,
    tuning: &SolarTuning,
    neisor_radius_km: f64,
) -> f64 {
    let sun = state.sun_km - observer_km;
    let moon = state.moon_km - observer_km;
    disc_overlap_fraction(
        angular_radius(tuning.radius_km(BodyId::Sun, neisor_radius_km), sun),
        angular_radius(tuning.radius_km(BodyId::Moon, neisor_radius_km), moon),
        angular_separation(sun, moon),
    )
}

/// Conservative whole-planet gate for the expensive per-fragment penumbra
/// equation. If false, no observer on Neisor's surface can see the two discs
/// touch; the shader skips every asin/acos. True may include a little extra
/// work near contact but can never suppress a real surface eclipse.
pub fn solar_contact_possible(
    state: SolarState,
    tuning: &SolarTuning,
    neisor_radius_km: f64,
) -> bool {
    let sun = state.sun_km;
    let moon = state.moon_km;
    if moon.length() >= sun.length() {
        return false;
    }
    let source = angular_radius(tuning.radius_km(BodyId::Sun, neisor_radius_km), sun);
    let occulter = angular_radius(tuning.radius_km(BodyId::Moon, neisor_radius_km), moon);
    let surface_parallax = angular_radius(neisor_radius_km, moon);
    angular_separation(sun, moon) <= source + occulter + surface_parallax
}

/// Fraction of the solar disc hidden by Neisor as seen from the moon. This
/// drives the copper interpolation through partial contact and reaches 1 in
/// the umbra; it is the lunar counterpart of `solar_occlusion_at`.
pub fn lunar_shadow_fraction(
    state: SolarState,
    tuning: &SolarTuning,
    neisor_radius_km: f64,
) -> f64 {
    let from_moon_to_sun = state.sun_km - state.moon_km;
    let from_moon_to_neisor = -state.moon_km;
    disc_overlap_fraction(
        angular_radius(
            tuning.radius_km(BodyId::Sun, neisor_radius_km),
            from_moon_to_sun,
        ),
        angular_radius(neisor_radius_km, from_moon_to_neisor),
        angular_separation(from_moon_to_sun, from_moon_to_neisor),
    )
}

fn finite_time(t_s: f64) -> f64 {
    if t_s.is_finite() { t_s } else { 0.0 }
}

fn effective_day_length(day_length_s: f64) -> f64 {
    if day_length_s.is_finite() && day_length_s > 0.0 {
        day_length_s
    } else {
        SolarTuning::default().day_length_s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_andrew_d5_clock() {
        let t = SolarTuning::default();
        assert_eq!(t.day_length_s, 1800.0);
        assert_eq!(t.lunar_days(), 7.0);
        assert_eq!(t.year_days(), 84.0);
        assert_eq!(t.year_days() / t.lunar_days(), 12.0);
        assert!(t.validate().is_ok());
    }

    #[test]
    fn kepler_solution_has_small_residual_and_repeats() {
        let t = SolarTuning::default();
        let e = t.moon.orbit.as_ref().unwrap();
        let at = 12_345_678.25;
        let p0 = kepler_position_km(e, at, t.day_length_s, 8660.254037844386);
        let p1 = kepler_position_km(e, at, t.day_length_s, 8660.254037844386);
        assert_eq!(p0, p1);
        assert!(p0.is_finite());
        let r_neisor = p0.length() / 8660.254037844386;
        assert!(r_neisor >= 13.86 * (1.0 - 0.08) - 1e-10);
        assert!(r_neisor <= 13.86 * (1.0 + 0.08) + 1e-10);
    }

    #[test]
    fn hierarchy_and_season_are_absolute_time_functions() {
        let t = SolarTuning::default();
        let r = 8660.254037844386;
        let a = state_at(&t, 0.0, 0.0, r);
        let b = state_at(&t, 0.0, 0.0, r);
        assert_eq!(a.sun_km, b.sun_km);
        assert_eq!(a.moon_km, b.moon_km);
        assert!((a.season_frac - 0.45).abs() < 1e-12);
        assert!((a.inertial_moon_km - a.inertial_neisor_km).length() < 15.0 * r);
        assert!(a.sun_km.length() > 10_000.0 * r);
        assert!((a.sun_declination_rad.to_degrees() - 10.0).abs() < 0.3);
    }

    #[test]
    fn season_retarget_moves_the_orbit_clock_itself() {
        let mut t = SolarTuning::default();
        t.set_season_frac(1800.0, 1800.0, 0.25);
        assert!((t.season_frac(1800.0, 1800.0) - 0.25).abs() < 1e-12);
        let state = state_at_with_day_length(&t, 1800.0, 1800.0, 1800.0, 1.0);
        assert!((state.season_frac - 0.25).abs() < 1e-12);
    }

    #[test]
    fn overlap_is_proportional_and_smooth_at_contacts() {
        assert_eq!(disc_overlap_fraction(1.0, 0.5, 2.0), 0.0);
        assert_eq!(disc_overlap_fraction(1.0, 2.0, 0.1), 1.0);
        assert!((disc_overlap_fraction(1.0, 1.0, 1.0) - 0.3910022189557706).abs() < 1e-12);
        let just_out = disc_overlap_fraction(1.0, 0.5, 1.500_001);
        let just_in = disc_overlap_fraction(1.0, 0.5, 1.499_999);
        assert_eq!(just_out, 0.0);
        assert!(just_in > 0.0 && just_in < 1e-6);
    }

    #[test]
    fn whole_planet_contact_gate_rejects_quiet_frames_but_keeps_eclipse() {
        let tuning = SolarTuning::default();
        let radius = 8660.254037844386;
        let quiet = state_at(&tuning, 0.0, 0.0, radius);
        assert!(!solar_contact_possible(quiet, &tuning, radius));
        let contact = state_at(&tuning, 7621.82, 7621.82, radius);
        assert!(solar_contact_possible(contact, &tuning, radius));
    }

    #[test]
    fn default_eclipse_calendar_is_frequent_but_clustered_irregularly() {
        let tuning = SolarTuning::default();
        let radius = 8660.254037844386;
        let step_s = 10.0;
        let year_s = tuning.day_length_s * tuning.year_days();
        let mut samples = Vec::with_capacity((year_s / step_s) as usize + 1);
        let mut t_s = 0.0;
        while t_s <= year_s {
            let state = state_at(&tuning, t_s, t_s, radius);
            samples.push((
                solar_occlusion_at(DVec3::ZERO, state, &tuning, radius),
                lunar_shadow_fraction(state, &tuning, radius),
            ));
            t_s += step_s;
        }
        let peaks = |column: usize| {
            (1..samples.len() - 1)
                .filter(|&i| {
                    let get = |j: usize| if column == 0 { samples[j].0 } else { samples[j].1 };
                    get(i) > 0.001 && get(i) > get(i - 1) && get(i) >= get(i + 1)
                })
                .map(|i| i as f64 * step_s)
                .collect::<Vec<_>>()
        };
        let solar = peaks(0);
        let lunar = peaks(1);
        assert_eq!(solar.len(), 2, "{solar:?}");
        assert_eq!(lunar.len(), 9, "{lunar:?}");
        let longest_lunar_gap = lunar.windows(2).map(|w| w[1] - w[0]).fold(0.0f64, f64::max);
        assert!(
            longest_lunar_gap > tuning.day_length_s * tuning.lunar_days() * 2.5,
            "lunar contacts stopped clustering: {lunar:?}"
        );
    }

    #[test]
    fn invalid_json_falls_back_as_one_unit() {
        for raw in [
            r#"{"day_length_s":0}"#,
            r#"{"moon":{"parent":"sun"}}"#,
            r#"{"moon":{"orbit":{"eccentricity":1.0}}}"#,
            r#"{"sun_halo_strength":1e999}"#,
            r#"{"unknown":1}"#,
        ] {
            let got = SolarTuning::from_json(raw, "test-solar-tuning.json");
            assert_eq!(
                got.day_length_s,
                SolarTuning::default().day_length_s,
                "{raw}"
            );
            assert!(got.validate().is_ok());
        }
    }

    #[test]
    fn tuning_file_can_override_one_knob_like_weather_tuning() {
        let got = SolarTuning::from_json(r#"{"day_length_s":2400}"#, "partial.json");
        assert_eq!(got.day_length_s, 2400.0);
        assert_eq!(got.moon.radius_neisor, 0.27);
        assert_eq!(got.year_days(), 84.0);
        assert!(got.validate().is_ok());
    }
}

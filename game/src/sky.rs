//! July sky ephemeris, evaluated from one absolute clock (seconds).
//! Kepler elements and frame convention are ported from viewer/src/orbits.rs.
//! No visible moon or eclipse work belongs here yet; its orbit only lights nights.

use glam::{DQuat, DVec3};
use serde::Deserialize;
use std::sync::OnceLock;

const TAU: f64 = std::f64::consts::TAU;
pub const SPEEDS: [f64; 7] = [0.0, 0.1, 1.0, 10.0, 60.0, 600.0, 3600.0];

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub day_length_s: f64,
    pub lunar_month_days: f64,
    pub months_per_season: u32,
    pub year_days: f64,
    /// Optical lengths relative to planet radius: July's 45 / 6371 and 20 / 6371.
    pub atmosphere_height_radii: f64,
    pub limb_height_radii: f64,
    /// July's 0.0035/km, scaled by the Earth-to-Neisor radius ratio.
    pub fog_per_radius: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            day_length_s: 1440.0,
            lunar_month_days: 7.0,
            months_per_season: 3,
            year_days: 84.0,
            atmosphere_height_radii: 45.0 / 6371.0,
            limb_height_radii: 20.0 / 6371.0,
            fog_per_radius: 0.0035 * 6371.0,
        }
    }
}

impl Settings {
    pub fn load(path: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Document {
            #[serde(default)]
            sky: Settings,
        }
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        serde_json::from_str::<Document>(&raw)
            .map(|d| d.sky)
            .map_err(|e| format!("sky settings: {e}"))
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("day_length_s", self.day_length_s),
            ("lunar_month_days", self.lunar_month_days),
            ("year_days", self.year_days),
            ("atmosphere_height_radii", self.atmosphere_height_radii),
            ("limb_height_radii", self.limb_height_radii),
            ("fog_per_radius", self.fog_per_radius),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("sky.{name} must be finite and positive"));
            }
        }
        if self.months_per_season == 0 {
            return Err("sky.months_per_season must be positive".into());
        }
        // All three calendar knobs describe the same four-season year.
        if (self.year_days - 4.0 * self.months_per_season as f64 * self.lunar_month_days).abs()
            > 1e-8
        {
            return Err("sky.year_days must equal 4 * months_per_season * lunar_month_days".into());
        }
        Ok(())
    }

    pub fn at(&self, seconds: f64) -> Ephemeris {
        let days = seconds / self.day_length_s;
        let planet = orbit(days, self.year_days, 12190.0, 0.035, 0.0, 0.0, 42.92, 162.0);
        let moon = orbit(
            days,
            self.lunar_month_days,
            13.86,
            0.08,
            4.8,
            225.0126009,
            -122.6728680,
            90.0,
        );
        let spin = 15.8f64.to_radians() - TAU * days.rem_euclid(1.0);
        let inertial_to_fixed =
            DQuat::from_rotation_z(spin) * DQuat::from_rotation_x(23.0f64.to_radians());
        Ephemeris {
            sun: inertial_to_fixed * -planet,
            moon: inertial_to_fixed * moon,
            fixed_to_inertial: inertial_to_fixed.conjugate(),
            season: (0.45 + days / self.year_days).rem_euclid(1.0),
        }
    }

    /// Seek on the first rotation to the requested local solar hour. Only
    /// the absolute time changes: neither the orbit nor the sky is pinned to
    /// the camera, and teleporting cannot drag the sun along with the player.
    pub fn starting_seconds(&self, hour: f64, longitude: f64) -> f64 {
        let target = longitude + (12.0 - hour) * TAU / 24.0;
        let s = self.at(0.0).sun;
        let mut t = (s.y.atan2(s.x) - target).rem_euclid(TAU) / TAU * self.day_length_s;
        for _ in 0..5 {
            let s = self.at(t).sun;
            let error = (s.y.atan2(s.x) - target + std::f64::consts::PI).rem_euclid(TAU)
                - std::f64::consts::PI;
            t += error / TAU * self.day_length_s;
        }
        t
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ephemeris {
    /// Positions relative to Neisor's centre, in Neisor radii (f64 until upload).
    pub sun: DVec3,
    pub moon: DVec3,
    pub fixed_to_inertial: DQuat,
    /// The single season phase is the planet's mean anomaly, as in July.
    pub season: f64,
}

#[allow(clippy::too_many_arguments)]
fn orbit(
    days: f64,
    period: f64,
    a: f64,
    e: f64,
    inclination: f64,
    node: f64,
    periapsis: f64,
    epoch: f64,
) -> DVec3 {
    let mean = (epoch.to_radians() + TAU * (days / period).rem_euclid(1.0)).rem_euclid(TAU);
    let mut eccentric = mean;
    for _ in 0..8 {
        eccentric -= (eccentric - e * eccentric.sin() - mean) / (1.0 - e * eccentric.cos());
    }
    let local = DVec3::new(
        a * (eccentric.cos() - e),
        a * (1.0 - e * e).sqrt() * eccentric.sin(),
        0.0,
    );
    let orient = DQuat::from_rotation_z(node.to_radians())
        * DQuat::from_rotation_x(inclination.to_radians())
        * DQuat::from_rotation_z(periapsis.to_radians());
    orient * local
}

#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub seconds: f64,
    pub speed: f64,
}

impl Clock {
    /// Called by simulation steps, never by draw/settle/readback loops.
    pub fn advance(&mut self, dt: f64) {
        self.seconds += dt * self.speed;
    }

    pub fn change_speed(&mut self, faster: bool) {
        self.speed = if faster {
            SPEEDS
                .into_iter()
                .find(|&s| s > self.speed)
                .unwrap_or(*SPEEDS.last().unwrap())
        } else {
            SPEEDS
                .into_iter()
                .rev()
                .find(|&s| s < self.speed)
                .unwrap_or(0.0)
        };
    }
}

pub fn parse_hour(text: &str) -> Result<f64, String> {
    let hour = if let Some((h, m)) = text.split_once(':') {
        let h: u32 = h
            .parse()
            .map_err(|_| "--time needs HH:MM or decimal hours")?;
        let m: u32 = m
            .parse()
            .map_err(|_| "--time needs HH:MM or decimal hours")?;
        if h >= 24 || m >= 60 {
            return Err("--time must be between 00:00 and 23:59".into());
        }
        h as f64 + m as f64 / 60.0
    } else {
        text.parse::<f64>()
            .map_err(|_| "--time needs HH:MM or decimal hours")?
    };
    if !hour.is_finite() || !(0.0..24.0).contains(&hour) {
        return Err("--time must be in [0, 24) hours".into());
    }
    Ok(hour)
}

/// Startup-only bridge to existing app/bench/play renderer constructors.
/// Mutable time belongs to the renderer's Sky; this immutable configuration
/// is installed once by main, so worker/mesh/engine APIs stay unchanged.
#[derive(Clone, Debug)]
pub struct Startup {
    pub settings: Settings,
    pub radius: f64,
    pub clock: Clock,
}

static STARTUP: OnceLock<Startup> = OnceLock::new();

pub fn install(startup: Startup) {
    STARTUP.set(startup).expect("sky startup installed twice");
}

pub fn startup() -> Startup {
    STARTUP
        .get()
        .expect("main must configure sky before creating a renderer")
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_calendar_and_bad_parameters() {
        let mut s = Settings::default();
        assert_eq!(s.day_length_s, 1440.0);
        s.validate().unwrap();
        s.year_days = 85.0;
        assert!(s.validate().is_err());
        s.year_days = 84.0;
        s.day_length_s = f64::NAN;
        assert!(s.validate().is_err());
        for bad in ["24:00", "12:60", "NaN", "-1", "inf", "abc"] {
            assert!(parse_hour(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_hour("17:30").unwrap(), 17.5);
    }

    #[test]
    fn speed_change_has_no_jump_and_pause_does_not_advance() {
        let mut c = Clock {
            seconds: 987.0,
            speed: 1.0,
        };
        c.change_speed(true);
        assert_eq!(c.seconds, 987.0);
        c.advance(0.5);
        assert_eq!(c.seconds, 992.0);
        c.speed = 0.0;
        c.advance(1234.0);
        assert_eq!(c.seconds, 992.0);
        c.change_speed(true);
        assert_eq!(c.speed, 0.1);
    }

    #[test]
    fn seek_local_time_at_any_longitude_without_camera_following() {
        let s = Settings::default();
        for lon in [-3.0, -1.0, 0.0, 2.4, 3.0] {
            for hour in [0.0, 6.0, 12.0, 17.5, 23.99] {
                let sun = s.at(s.starting_seconds(hour, lon)).sun;
                let actual = (12.0 + (lon - sun.y.atan2(sun.x)) * 24.0 / TAU).rem_euclid(24.0);
                let error = (actual - hour + 12.0).rem_euclid(24.0) - 12.0;
                assert!(error.abs() < 1e-8, "{lon} {hour}: {actual}");
            }
        }
    }

    #[test]
    fn ephemeris_is_seekable_and_stars_rotate_with_inertial_frame() {
        let s = Settings::default();
        let t = 123.0;
        let a = s.at(t);
        let _ = s.at(1e8);
        let b = s.at(t);
        assert_eq!(a.sun, b.sun);
        assert_eq!(a.moon, b.moon);
        assert!((a.sun.length() - 12190.0).abs() < 12190.0 * 0.035);
        let quarter = s.at(t + 360.0);
        let inertial_ray = DVec3::X;
        let ray_a = a.fixed_to_inertial.conjugate() * inertial_ray;
        let ray_b = quarter.fixed_to_inertial.conjugate() * inertial_ray;
        assert!((a.fixed_to_inertial * ray_a - quarter.fixed_to_inertial * ray_b).length() < 1e-12);
        assert!((ray_a - ray_b).length() > 1.0);
    }
}

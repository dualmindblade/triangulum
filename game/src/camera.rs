//! Free-fly camera in planet-centred f64 coordinates; the GPU sees
//! camera-relative f32.
//!
//! The way a body faces is a vector, not a compass heading: `facing`, a unit
//! vector level with the ground. A heading is measured from north, and north
//! turns under you as you move, so holding a heading walks a rhumb line that
//! spirals into the pole, and at the pole it means nothing at all. A vector
//! moves with you (`carried`) and has no pole. A heading is still how people
//! and scripts say a direction, so `heading` and `facing_of` turn one into the
//! other at this one place.

use crate::grid::Planet;
use crate::render::CameraUniform;
use glam::{DVec3, Mat4, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: DVec3,
    /// The way it faces, level: a unit vector in the tangent plane at `pos`.
    pub facing: DVec3,
    /// Radians; positive looks up.
    pub pitch: f64,
    pub fov_y: f64,
}

/// `v` laid level at `up`, as a unit vector. If it stands straight up there is
/// no level part to keep, and it faces north.
pub fn level(up: DVec3, v: DVec3) -> DVec3 {
    let flat = v - up * v.dot(up);
    if flat.length_squared() > 1e-20 { flat.normalize() } else { Camera::local_frame(up).0 }
}

/// A unit vector at `pos` in the direction of a compass heading (radians, 0 =
/// north, positive = east).
pub fn facing_of(pos: DVec3, heading: f64) -> DVec3 {
    let (north, east, _) = Camera::local_frame(pos);
    north * heading.cos() + east * heading.sin()
}

/// The compass heading of a direction at `pos`, radians. At the exact pole,
/// where no north exists, it is measured from `local_frame`'s stand-in.
pub fn heading_of(pos: DVec3, facing: DVec3) -> f64 {
    let (north, east, _) = Camera::local_frame(pos);
    facing.dot(east).atan2(facing.dot(north))
}

/// Parallel transport: `v` carried along the great circle from unit `a` to
/// unit `b`, which is the rotation that takes `a` to `b` the short way. Plain
/// arithmetic, no trigonometry, so it does the same thing on every machine.
/// Exactly opposite points have no short way, and `v` stays as it is.
pub fn carried(a: DVec3, b: DVec3, v: DVec3) -> DVec3 {
    let d = 1.0 + a.dot(b);
    if d < 1e-12 {
        return v;
    }
    let s = a + b;
    v - s * (s.dot(v) / d) + b * (2.0 * a.dot(v))
}

/// `facing` turned to the right by `angle` radians about `up`.
pub fn turned(up: DVec3, facing: DVec3, angle: f64) -> DVec3 {
    let f = level(up, facing);
    (f * angle.cos() + f.cross(up) * angle.sin()).normalize()
}

impl Camera {
    pub fn new(pos: DVec3, heading: f64, pitch: f64, fov_y: f64) -> Self {
        Self { pos, facing: facing_of(pos, heading), pitch, fov_y }
    }

    /// The compass heading it faces, radians: 0 = local north (world +Z
    /// projected onto the tangent plane), positive = east.
    pub fn heading(&self) -> f64 {
        heading_of(self.pos, self.facing)
    }

    /// Local tangent frame: (north, east, up).
    pub fn local_frame(pos: DVec3) -> (DVec3, DVec3, DVec3) {
        let up = pos.normalize();
        let mut north = DVec3::Z - up * up.dot(DVec3::Z);
        if north.length_squared() < 1e-12 {
            north = DVec3::X - up * up.dot(DVec3::X);
        }
        let north = north.normalize();
        let east = north.cross(up).normalize();
        (north, east, up)
    }

    /// (forward, level forward, up)
    pub fn vectors(&self) -> (DVec3, DVec3, DVec3) {
        let up = self.pos.normalize();
        let fwd_h = level(up, self.facing);
        let fwd = fwd_h * self.pitch.cos() + up * self.pitch.sin();
        (fwd.normalize(), fwd_h, up)
    }

    pub fn right(&self) -> DVec3 {
        let (_, fwd_h, up) = self.vectors();
        fwd_h.cross(up).normalize()
    }

    #[allow(deprecated)]
    pub fn view_proj(&self, aspect: f64) -> Mat4 {
        let (fwd, _, up) = self.vectors();
        let view = Mat4::look_to_rh(Vec3::ZERO, fwd.as_vec3(), up.as_vec3());
        let proj = Mat4::perspective_infinite_reverse_rh(self.fov_y as f32, aspect as f32, 0.1);
        proj * view
    }

    /// Altitude above the sea-level sphere (m).
    pub fn altitude(&self, planet: &Planet) -> f64 {
        self.pos.length() - planet.radius
    }

    /// (lat deg, lon deg, altitude m above sea level).
    pub fn lat_lon_alt(&self, planet: &Planet) -> (f64, f64, f64) {
        let (lat, lon) = Planet::lat_lon_deg(self.pos);
        (lat, lon, self.altitude(planet))
    }

    pub fn uniform(&self, aspect: f64, sun_dir: DVec3, planet: &Planet, water_alpha: f32) -> CameraUniform {
        let vp = self.view_proj(aspect);
        let (_, _, up) = self.vectors();
        let alt = self.altitude(planet).max(0.0);
        // Fog range grows with altitude; sky darkens toward space.
        let fog_range = 30_000.0 + 3.0 * alt;
        let dark = ((alt - 20_000.0) / 130_000.0).clamp(0.0, 1.0);
        let dark = dark * dark * (3.0 - 2.0 * dark);
        CameraUniform {
            view_proj: vp.to_cols_array_2d(),
            inv_view_proj: vp.inverse().to_cols_array_2d(),
            sun_dir: [sun_dir.x as f32, sun_dir.y as f32, sun_dir.z as f32, 0.0],
            cam_up: [up.x as f32, up.y as f32, up.z as f32, alt as f32],
            fog: [(1.0 / fog_range) as f32, dark as f32, water_alpha, 0.0],
            sky_horizon: [0.70, 0.78, 0.88, 1.0],
            sky_zenith: [0.22, 0.42, 0.78, 1.0],
            light: [0.38, 0.75, 0.6, 0.0],
        }
    }
}

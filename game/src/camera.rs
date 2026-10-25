//! Free-fly camera in planet-centred f64 coordinates; the GPU sees
//! camera-relative f32.

use crate::grid::Planet;
use crate::render::CameraUniform;
use glam::{DVec3, Mat4, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub pos: DVec3,
    /// Radians; 0 = local north (world +Z projected onto the tangent plane), positive = east.
    pub heading: f64,
    /// Radians; positive looks up.
    pub pitch: f64,
    pub fov_y: f64,
}

impl Camera {
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
        let (north, east, up) = Self::local_frame(self.pos);
        let fwd_h = north * self.heading.cos() + east * self.heading.sin();
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

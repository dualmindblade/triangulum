//! Render inputs for the July atmosphere. No mesh/streamer state is involved.

use super::CameraUniform;
use crate::sky::{self, Clock, Settings};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SkyUniform {
    pub sun: [f32; 4],          // direction at camera, cosine of angular radius
    pub sun_position: [f32; 4], // centre-relative, in planet radii
    pub moon_position: [f32; 4],
    pub eye: [f32; 4],        // centre-relative in radii; w = radius in metres
    pub atmosphere: [f32; 4], // camera air density, inverse limb height, fog/m, underwater
    pub fixed_to_inertial: [[f32; 4]; 4],
}

pub struct Sky {
    pub settings: Settings,
    pub clock: Clock,
    radius: f64,
    observer: Option<(DVec3, bool)>,
}

impl Sky {
    pub fn new() -> Self {
        let initial = sky::startup();
        Self {
            settings: initial.settings,
            clock: initial.clock,
            radius: initial.radius,
            observer: None,
        }
    }

    /// The app supplies the exact eye, including negative sea-level altitude.
    /// Older headless callers can still supply their camera-only uniform.
    pub fn set_observer(&mut self, eye: DVec3, underwater: bool) {
        self.observer = Some((eye, underwater));
    }

    pub fn uniform(&mut self, camera: &CameraUniform) -> SkyUniform {
        let up = DVec3::new(
            camera.cam_up[0] as f64,
            camera.cam_up[1] as f64,
            camera.cam_up[2] as f64,
        )
        .normalize();
        let (eye_m, underwater) = self
            .observer
            .take()
            .unwrap_or((up * (self.radius + camera.cam_up[3] as f64), false));
        let eye = eye_m / self.radius;
        let state = self.settings.at(self.clock.seconds);
        let to_sun = state.sun - eye;
        let angular_radius = (109.0 / to_sun.length()).asin();
        SkyUniform {
            sun: to_sun
                .normalize()
                .as_vec3()
                .extend(angular_radius.cos() as f32)
                .to_array(),
            sun_position: state.sun.as_vec3().extend(0.0).to_array(),
            moon_position: state.moon.as_vec3().extend(0.0).to_array(),
            eye: eye.as_vec3().extend(self.radius as f32).to_array(),
            atmosphere: [
                (-(eye_m.length() - self.radius).max(0.0)
                    / (self.settings.atmosphere_height_radii * self.radius))
                    .exp() as f32,
                (1.0 / (self.settings.limb_height_radii * self.radius)) as f32,
                (self.settings.fog_per_radius / self.radius) as f32,
                if underwater { 1.0 } else { 0.0 },
            ],
            fixed_to_inertial: Mat4::from_quat(state.fixed_to_inertial.as_quat())
                .to_cols_array_2d(),
        }
    }
}

//! Render inputs for the July atmosphere and Andrew's look dials (M3a).
//! No mesh/streamer state is involved.

use super::CameraUniform;
use crate::look::Look;
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
    // The look (M3a); defaults are the constants these replaced.
    pub light: [f32; 4],      // sun strength, sky ambient, shadow light, night light
    pub shade: [f32; 4],      // side shade, underside shade, corner shadows, fog strength
    pub horizon: [f32; 4],    // rgb (also the fog), night sky
    pub zenith: [f32; 4],     // rgb, star brightness
    pub sunset: [f32; 4],     // rgb, sunset strength
    pub moon: [f32; 4],       // moonlight rgb (colour x strength), unused
    pub water: [f32; 4],      // opacity, edge opacity, glint, glint sharpness
    pub underwater: [f32; 4], // rgb, strength
    pub tex: [f32; 4],        // M3b: blocks per metre (textures tile once per block), unused x3
}

pub struct Sky {
    pub settings: Settings,
    pub clock: Clock,
    /// Andrew's look (the panel writes it; the uniform reads it each frame).
    pub look: Look,
    radius: f64,
    observer: Option<(DVec3, bool)>,
    /// Block size in metres (M3b: texture coordinates are in blocks).
    pub block_m: f64,
}

fn rgb(c: [f32; 3], w: f64) -> [f32; 4] {
    [c[0], c[1], c[2], w as f32]
}

impl Sky {
    pub fn new() -> Self {
        let initial = sky::startup();
        let look = crate::look::startup().map_or_else(|| Look::defaults(&initial.settings, initial.radius), |s| s.look.clone());
        let block_m = super::textures::installed().map_or(1.0, |t| t.block_m);
        Self { settings: initial.settings, clock: initial.clock, look, radius: initial.radius, observer: None, block_m }
    }

    /// The app supplies the exact eye, including negative sea-level altitude.
    /// Older headless callers can still supply their camera-only uniform.
    pub fn set_observer(&mut self, eye: DVec3, underwater: bool) {
        self.observer = Some((eye, underwater));
    }

    pub fn uniform(&mut self, camera: &CameraUniform) -> SkyUniform {
        let up = DVec3::new(camera.cam_up[0] as f64, camera.cam_up[1] as f64, camera.cam_up[2] as f64).normalize();
        let (eye_m, underwater) = self.observer.take().unwrap_or((up * (self.radius + camera.cam_up[3] as f64), false));
        let eye = eye_m / self.radius;
        let state = self.settings.at(self.clock.seconds);
        let to_sun = state.sun - eye;
        let angular_radius = (109.0 / to_sun.length()).asin();
        let look = &self.look;
        let atmosphere_m = look.sky.atmosphere_height_m;
        // The limb (the glow seen from space) keeps July's proportion to
        // the atmosphere: one dial raises both.
        let limb_m = atmosphere_m * self.settings.limb_height_radii / self.settings.atmosphere_height_radii;
        let (s, l, w) = (&look.sky, &look.light, &look.water);
        SkyUniform {
            sun: to_sun.normalize().as_vec3().extend(angular_radius.cos() as f32).to_array(),
            sun_position: state.sun.as_vec3().extend(0.0).to_array(),
            moon_position: state.moon.as_vec3().extend(0.0).to_array(),
            eye: eye.as_vec3().extend(self.radius as f32).to_array(),
            atmosphere: [
                (-(eye_m.length() - self.radius).max(0.0) / atmosphere_m).exp() as f32,
                (1.0 / limb_m) as f32,
                (s.fog_density_per_km / 1000.0) as f32,
                if underwater { 1.0 } else { 0.0 },
            ],
            fixed_to_inertial: Mat4::from_quat(state.fixed_to_inertial.as_quat()).to_cols_array_2d(),
            light: [l.sun_strength as f32, l.sky_ambient as f32, l.shadow_light as f32, l.night_light as f32],
            shade: [l.side_shade as f32, l.underside_shade as f32, l.corner_shadows as f32, s.fog_strength as f32],
            horizon: rgb(s.horizon_colour, s.night_sky),
            zenith: rgb(s.zenith_colour, s.star_brightness),
            sunset: rgb(s.sunset_colour, s.sunset_strength),
            moon: rgb(l.moon_colour.map(|c| c * l.moonlight as f32), 0.0),
            water: [w.opacity as f32, w.edge_opacity as f32, w.glint as f32, w.glint_sharpness as f32],
            underwater: rgb(w.underwater_colour, w.underwater_strength),
            tex: [(1.0 / self.block_m) as f32, 0.0, 0.0, 0.0],
        }
    }
}

//! Camera with free look. Position is kept in f64 kilometers (planet-scale
//! numbers overflow f32 precision badly); everything sent to the GPU is made
//! camera-relative first, so the GPU only ever sees small f32 values.
//!
//! Orientation is yaw/pitch in the local tangent frame: yaw 0 = north,
//! positive = east; pitch 0 = horizon, -90 deg = straight down.

use glam::{DMat4, DVec3};

pub struct Camera {
    pub lon: f64,           // radians
    pub lat: f64,
    pub altitude_km: f64,   // above the local surface (see ground_km)
    pub radius_km: f64,
    pub ground_km: f64,     // local surface height, updated per frame
    pub yaw: f64,           // radians
    pub pitch: f64,         // radians
}

impl Camera {
    pub fn position(&self) -> DVec3 {
        let r = self.radius_km + self.ground_km + self.altitude_km;
        DVec3::new(
            r * self.lat.cos() * self.lon.cos(),
            r * self.lat.cos() * self.lon.sin(),
            r * self.lat.sin(),
        )
    }

    /// Local tangent frame at the camera: (radial up, north, east).
    pub fn frame(&self) -> (DVec3, DVec3, DVec3) {
        let up = self.position().normalize();
        let east = DVec3::Z.cross(up).normalize_or_zero();
        let east = if east.length_squared() < 0.5 { DVec3::X } else { east };
        let north = up.cross(east).normalize();
        (up, north, east)
    }

    pub fn look_dir(&self) -> DVec3 {
        let (up, north, east) = self.frame();
        let horiz = north * self.yaw.cos() + east * self.yaw.sin();
        (horiz * self.pitch.cos() + up * self.pitch.sin()).normalize()
    }

    /// Heading on the ground plane (ignores pitch) — movement direction.
    pub fn heading(&self, strafe_right: f64, forward: f64) -> DVec3 {
        let (_, north, east) = self.frame();
        let fwd = north * self.yaw.cos() + east * self.yaw.sin();
        let right = north * (self.yaw + std::f64::consts::FRAC_PI_2).cos()
            + east * (self.yaw + std::f64::consts::FRAC_PI_2).sin();
        (fwd * forward + right * strafe_right).normalize_or_zero()
    }

    /// Move the camera over the sphere by `dist_km` along tangent `dir`,
    /// following a great circle. The view heading is parallel-transported:
    /// without this, "hold W" keeps a constant compass bearing, which on a
    /// sphere is a loxodrome — a spiral into the pole.
    pub fn translate(&mut self, dir: DVec3, dist_km: f64) {
        if dist_km <= 0.0 || dir.length_squared() < 1e-12 {
            return;
        }
        let r = self.radius_km + self.ground_km + self.altitude_km;
        let theta = dist_km / r;
        let pos = self.position().normalize();
        let t = (dir - pos * dir.dot(pos)).normalize_or_zero();

        // world-space forward before the move (what the player "means")
        let (_, north0, east0) = self.frame();
        let fwd0 = north0 * self.yaw.cos() + east0 * self.yaw.sin();

        let new = (pos * theta.cos() + t * theta.sin()).normalize();
        self.lat = new.z.clamp(-1.0, 1.0).asin();
        self.lon = new.y.atan2(new.x);

        // parallel transport fwd0 along the motion great circle: the
        // component along t rotates with the arc, the binormal is invariant
        let b = pos.cross(t);
        let a_t = fwd0.dot(t);
        let a_b = fwd0.dot(b);
        let t_new = t * theta.cos() - pos * theta.sin();
        let fwd1 = t_new * a_t + b * a_b;
        let (_, north1, east1) = self.frame();
        self.yaw = fwd1.dot(east1).atan2(fwd1.dot(north1));
    }

    /// View-projection with the camera at the origin (camera-relative space).
    /// Pitch is clamped short of vertical (main.rs), so radial up is always
    /// valid — switching up-vectors near nadir caused a visible view flip.
    pub fn view_proj(&self, aspect: f64) -> DMat4 {
        let look = self.look_dir();
        let (up_r, _, _) = self.frame();
        let view = DMat4::look_at_rh(DVec3::ZERO, look, up_r);
        // reversed-Z with an infinite far plane: f32 depth precision becomes
        // near-uniform over view distance, so the near plane can hug the eye
        // without far-field z-fighting. With the old 0.8 m floor the near
        // plane poked through walls and tree trunks in walk mode. At eye
        // height this gives ~14 cm — the screen-corner ray reaches 1.65x
        // that (~24 cm), safely inside the walker's 35 cm body radius.
        let near = (self.altitude_km * 0.08).clamp(0.0001, 50.0);
        let proj = DMat4::perspective_infinite_reverse_rh(65f64.to_radians(), aspect, near);
        proj * view
    }
}

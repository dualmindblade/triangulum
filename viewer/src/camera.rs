//! Camera with free look. Position is kept in f64 kilometers (planet-scale
//! numbers overflow f32 precision badly); everything sent to the GPU is made
//! camera-relative first, so the GPU only ever sees small f32 values.
//!
//! Orientation is yaw/pitch in the local tangent frame: yaw 0 = north,
//! positive = east; pitch 0 = horizon, -90 deg = straight down.

use glam::{DMat4, DVec3};

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub lon: f64,           // radians
    pub lat: f64,
    pub altitude_km: f64,   // above the local surface (see ground_km)
    pub radius_km: f64,
    pub ground_km: f64,     // local surface height, updated per frame
    pub yaw: f64,           // radians
    pub pitch: f64,         // radians
    /// View-axis rotation. Focused Neisor keeps this at zero; freecam owns it.
    pub roll: f64,
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

    /// World-space (look, view-up, view-right), including roll. All vectors
    /// are f64; only the final view-projection is narrowed for the GPU.
    pub fn view_basis(&self) -> (DVec3, DVec3, DVec3) {
        let look = self.look_dir();
        let (radial_up, _, east) = self.frame();
        let mut right = look.cross(radial_up).normalize_or_zero();
        if right.length_squared() < 0.5 {
            right = east;
        }
        let base_up = right.cross(look).normalize();
        let (sin_roll, cos_roll) = self.roll.sin_cos();
        let view_up = (base_up * cos_roll + right * sin_roll).normalize();
        let view_right = look.cross(view_up).normalize();
        (look, view_up, view_right)
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

    /// Set an arbitrary f64 position in the Neisor-centered render frame and
    /// preserve an arbitrary world-space view orientation. This is the bridge
    /// used by body focus realignment and freecam; walk/fly's spherical
    /// navigation continues to use `translate` unchanged.
    pub fn set_world_pose(&mut self, position_km: DVec3, look: DVec3, view_up: DVec3) {
        if !position_km.is_finite() || position_km.length_squared() < 1.0 {
            return;
        }
        let r = position_km.length();
        self.lat = (position_km.z / r).clamp(-1.0, 1.0).asin();
        self.lon = position_km.y.atan2(position_km.x);
        self.ground_km = 0.0;
        self.altitude_km = r - self.radius_km;
        self.set_world_orientation(look, view_up);
    }

    pub fn set_world_orientation(&mut self, look: DVec3, view_up: DVec3) {
        let look = look.normalize_or_zero();
        if look.length_squared() < 0.5 {
            return;
        }
        let (radial_up, north, east) = self.frame();
        self.pitch = look.dot(radial_up).clamp(-1.0, 1.0).asin();
        let horiz = (look - radial_up * look.dot(radial_up)).normalize_or_zero();
        if horiz.length_squared() > 0.5 {
            self.yaw = horiz.dot(east).atan2(horiz.dot(north));
        }
        let mut base_right = look.cross(radial_up).normalize_or_zero();
        if base_right.length_squared() < 0.5 {
            base_right = east;
        }
        let base_up = base_right.cross(look).normalize();
        let target_up = (view_up - look * view_up.dot(look)).normalize_or_zero();
        self.roll = if target_up.length_squared() > 0.5 {
            target_up.dot(base_right).atan2(target_up.dot(base_up))
        } else {
            0.0
        };
    }

    /// Local-axis 6DOF translation, preserving the world-space orientation as
    /// the Neisor-relative spherical coordinates change under it.
    pub fn translate_free(&mut self, strafe: f64, vertical: f64, forward: f64, dist_km: f64) {
        if dist_km <= 0.0 {
            return;
        }
        let (look, view_up, view_right) = self.view_basis();
        let direction = (view_right * strafe + view_up * vertical + look * forward)
            .normalize_or_zero();
        if direction.length_squared() < 0.5 {
            return;
        }
        self.set_world_pose(self.position() + direction * dist_km, look, view_up);
    }

    /// View-projection with the camera at the origin (camera-relative space).
    /// Pitch is clamped short of vertical (main.rs), so radial up is always
    /// valid — switching up-vectors near nadir caused a visible view flip.
    pub fn view_proj(&self, aspect: f64) -> DMat4 {
        let (look, view_up, _) = self.view_basis();
        let view = DMat4::look_at_rh(DVec3::ZERO, look, view_up);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    Focused(crate::orbits::BodyId),
    Freecam,
}

/// Focus/freecam state shared by the window app and play harness. A focused
/// body's camera offset is translated by the body's exact f64 displacement on
/// every clock jump, so timeskip never leaves the camera at stale coordinates.
pub struct CameraRig {
    pub mode: CameraMode,
    focus_offset_km: DVec3,
    neisor_camera: Option<Camera>,
}

impl Default for CameraRig {
    fn default() -> Self {
        Self {
            mode: CameraMode::Focused(crate::orbits::BodyId::Neisor),
            focus_offset_km: DVec3::ZERO,
            neisor_camera: None,
        }
    }
}

impl CameraRig {
    pub fn numeric_focus_id(&self) -> f64 {
        match self.mode {
            CameraMode::Focused(body) => body.numeric_id(),
            CameraMode::Freecam => -1.0,
        }
    }

    pub fn focused_body(&self) -> Option<crate::orbits::BodyId> {
        match self.mode {
            CameraMode::Focused(body) => Some(body),
            CameraMode::Freecam => None,
        }
    }

    pub fn focus(
        &mut self,
        target: crate::orbits::BodyId,
        solar: crate::orbits::SolarState,
        tuning: &crate::orbits::SolarTuning,
        neisor_radius_km: f64,
        camera: &mut Camera,
    ) {
        use crate::orbits::BodyId;
        if target == BodyId::Neisor {
            if let Some(saved) = self.neisor_camera.take() {
                *camera = saved;
            }
            camera.roll = 0.0;
            self.mode = CameraMode::Focused(BodyId::Neisor);
            self.focus_offset_km = camera.position();
            return;
        }
        if self.mode == CameraMode::Focused(BodyId::Neisor) {
            self.neisor_camera = Some(*camera);
        }
        let center = solar.position_km(target);
        let outward = center.normalize_or_zero();
        let outward = if outward.length_squared() > 0.5 { outward } else { DVec3::X };
        let radius = tuning.radius_km(target, neisor_radius_km);
        self.focus_offset_km = outward * radius * 3.0;
        let position = center + self.focus_offset_km;
        let look = -outward;
        let view_up = (DVec3::Z - look * DVec3::Z.dot(look)).normalize_or_zero();
        let view_up = if view_up.length_squared() > 0.5 { view_up } else { DVec3::Y };
        camera.set_world_pose(position, look, view_up);
        camera.roll = 0.0;
        self.mode = CameraMode::Focused(target);
    }

    pub fn freecam(&mut self, camera: &Camera) {
        if self.mode == CameraMode::Focused(crate::orbits::BodyId::Neisor) {
            self.neisor_camera = Some(*camera);
        }
        self.mode = CameraMode::Freecam;
    }

    pub fn cycle(
        &mut self,
        solar: crate::orbits::SolarState,
        tuning: &crate::orbits::SolarTuning,
        neisor_radius_km: f64,
        camera: &mut Camera,
    ) {
        use crate::orbits::BodyId;
        match self.mode {
            CameraMode::Focused(BodyId::Neisor) => {
                self.focus(BodyId::Moon, solar, tuning, neisor_radius_km, camera)
            }
            CameraMode::Focused(BodyId::Moon) => {
                self.focus(BodyId::Sun, solar, tuning, neisor_radius_km, camera)
            }
            CameraMode::Focused(BodyId::Sun) => self.freecam(camera),
            CameraMode::Freecam => {
                self.focus(BodyId::Neisor, solar, tuning, neisor_radius_km, camera)
            }
        }
    }

    pub fn realign(&mut self, solar: crate::orbits::SolarState, camera: &mut Camera) {
        let Some(body) = self.focused_body() else { return };
        if body == crate::orbits::BodyId::Neisor {
            return;
        }
        let center = solar.position_km(body);
        let position = center + self.focus_offset_km;
        let (_, old_up, _) = camera.view_basis();
        camera.set_world_pose(position, center - position, old_up);
    }

    pub fn focus_distance_km(
        &self,
        solar: crate::orbits::SolarState,
        camera: &Camera,
    ) -> f64 {
        self.focused_body()
            .map(|body| camera.position().distance(solar.position_km(body)))
            .unwrap_or(f64::NAN)
    }

    pub fn focus_alignment(
        &self,
        solar: crate::orbits::SolarState,
        camera: &Camera,
    ) -> f64 {
        self.focused_body()
            .map(|body| {
                camera
                    .look_dir()
                    .dot((solar.position_km(body) - camera.position()).normalize_or_zero())
            })
            .unwrap_or(f64::NAN)
    }
}

//! The player as library code: walk/fly physics, jumping, swimming, block
//! and torch edits. The window app (main.rs) and the scripted play harness
//! (examples/play.rs) both drive THIS simulation — there is exactly one
//! implementation of how the player moves, so what a script verifies is
//! what the game does.

use crate::camera::Camera;
use crate::planet::Planet;
use crate::voxel::{
    ceiling_above_km, chunks_touching_column, column_id, raycast_column, support_below_km,
    surface_height_km, water_surface_km, ChunkKey, Edits, Torches, VOXEL_KM,
};

/// Player eye height above the feet, km.
pub const EYE_KM: f64 = 0.0018;

/// Fly-speed ceiling (creative-director request C-2): speed grows with
/// altitude (x0.5) and stops increasing at this cap — reached around
/// 3,000 km up, roughly half a planet radius, which is where "pretty far
/// away" starts. At the cap a hemisphere crossing takes ~13 s (Shift for
/// 4x), while the planet's ~33 km/s equatorial rotation stays readable
/// whenever you pause.
const FAR_SPEED_CAP_KMS: f64 = 1500.0;

/// Minimum height the cruise elevation-lock keeps above the terrain when a
/// peak rises above the held radius (matches the voxel branch's floor, so the
/// handoff across the voxel boundary doesn't snap).
const CRUISE_FLOOR_KM: f64 = 0.0025;

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Mode {
    Fly,
    Walk,
}

/// One tick's worth of movement intent (already resolved from whatever
/// input source: held keys in the app, script commands in the harness).
#[derive(Clone, Copy, Default)]
pub struct Input {
    pub fwd: f64,      // +1 forward, -1 back
    pub strafe: f64,   // +1 right, -1 left
    pub sprint: bool,
    pub swim_up: bool, // Space held: swim up while in water
}

pub struct PlayerState {
    pub mode: Mode,
    pub vert_vel_mps: f64, // vertical velocity in walk mode (gravity, jumps, swim)
    pub grounded: bool,    // feet resting on a solid top last tick
    pub underwater: bool,  // eye below a water surface
    /// Cruise elevation-lock setpoint: the held `ground_km + altitude_km`
    /// (i.e. radius above the planet baseline) that fly cruising keeps
    /// constant above the voxel range. See the Fly branch of `update`.
    pub fly_elev_km: f64,
    /// True while the elevation-lock is riding terrain that poked above the
    /// held radius — freezes `fly_elev_km` so we settle back to it afterward.
    pub fly_riding: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            mode: Mode::Fly,
            vert_vel_mps: 0.0,
            grounded: false,
            underwater: false,
            fly_elev_km: 0.0,
            fly_riding: false,
        }
    }
}

impl PlayerState {
    /// Space in walk mode: jump if standing.
    pub fn jump(&mut self) {
        if self.mode == Mode::Walk && self.grounded {
            self.vert_vel_mps = 5.2;
            self.grounded = false;
        }
    }

    /// G: walk starts wherever the camera is — pressed in flight, you fall.
    pub fn set_walk(&mut self, camera: &mut Camera) {
        let feet = camera.ground_km + camera.altitude_km - EYE_KM;
        self.mode = Mode::Walk;
        camera.ground_km = feet;
        camera.altitude_km = EYE_KM;
        self.vert_vel_mps = 0.0;
        self.grounded = false;
        self.fly_riding = false;
    }

    /// F: back to fly mode.
    pub fn set_fly(&mut self, camera: &mut Camera) {
        self.mode = Mode::Fly;
        camera.altitude_km = camera.altitude_km.max(0.004);
        // re-baseline the elevation-lock on the next cruise tick
        self.fly_riding = false;
    }

    /// Jump to a lat/lon (degrees), fly mode, like the T key.
    pub fn teleport(
        &mut self,
        planet: &Planet,
        edits: &Edits,
        camera: &mut Camera,
        lat_deg: f64,
        lon_deg: f64,
        alt_km: Option<f64>,
        exaggeration: f64,
    ) {
        camera.lat = lat_deg.to_radians();
        camera.lon = lon_deg.to_radians();
        if let Some(alt) = alt_km {
            camera.altitude_km = alt.clamp(0.0025, 80000.0);
        } else {
            camera.altitude_km = camera.altitude_km.max(0.05);
        }
        self.mode = Mode::Fly;
        self.vert_vel_mps = 0.0;
        self.grounded = false;
        // a teleport is a deliberate elevation change: re-baseline the
        // cruise elevation-lock on the next tick rather than riding a stale
        // held radius from before the jump.
        self.fly_riding = false;
        camera.ground_km = crate::terrain::ground_height_km(
            planet,
            camera.position().normalize(),
            exaggeration,
        );
        // refresh the flag now: a teleport is a pose change with no update
        // tick before the next frame/shot, so a stale `underwater` would show.
        self.refresh_underwater(planet, edits, camera, exaggeration);
    }

    /// Recompute `underwater` from the eye position vs the local water
    /// surface. Must run after ANY pose/mode change (an update tick, a
    /// teleport, a mode switch) so the flag never goes stale: fly mode used to
    /// hard-code it false (so flying below water never tinted), and a teleport
    /// above water left a stale `true` until the next tick.
    pub fn refresh_underwater(
        &mut self,
        planet: &Planet,
        edits: &Edits,
        camera: &Camera,
        exaggeration: f64,
    ) {
        let dir = camera.position().normalize();
        let eye = camera.ground_km + camera.altitude_km;
        self.underwater = water_surface_km(planet, edits, dir, exaggeration)
            .is_some_and(|w| eye < w - 0.0003);
    }

    /// Recompute `grounded` after the world changed under the player (a block
    /// edit). Breaking the block underfoot drops the support a block down;
    /// without this the player reads `grounded=true` while hovering above the
    /// new support until the next tick. Walk mode only (fly never grounds).
    pub fn refresh_grounded(
        &mut self,
        planet: &Planet,
        edits: &Edits,
        camera: &Camera,
        exaggeration: f64,
    ) {
        if self.mode != Mode::Walk {
            return;
        }
        let dir = camera.position().normalize();
        let feet = camera.ground_km;
        let support = support_below_km(planet, edits, dir, feet + 1e-7, exaggeration);
        // grounded only while the feet are essentially resting on the support
        self.grounded = feet - support <= 1e-6;
    }

    /// Resync derived state after a block edit moved the world under the
    /// player — both the support (grounded) and the water column (underwater)
    /// can change. Paired so a caller can't refresh one and forget the other.
    pub fn refresh_after_edit(
        &mut self,
        planet: &Planet,
        edits: &Edits,
        camera: &Camera,
        exaggeration: f64,
    ) {
        self.refresh_grounded(planet, edits, camera, exaggeration);
        self.refresh_underwater(planet, edits, camera, exaggeration);
    }

    /// One simulation tick: movement, ground following, gravity, collision.
    pub fn update(
        &mut self,
        planet: &Planet,
        edits: &Edits,
        camera: &mut Camera,
        input: &Input,
        exaggeration: f64,
        dt: f64,
    ) {
        let exagg = exaggeration;
        let (fwd, strafe) = (input.fwd, input.strafe);
        let voxels_live = camera.altitude_km < crate::renderer::VOXEL_MAX_ALT_KM;

        match self.mode {
            Mode::Fly => {
                if fwd != 0.0 || strafe != 0.0 {
                    // Fly speed scales with altitude — glide low, cruise
                    // high, and the planet stays traversable from orbit —
                    // with a cap that engages only genuinely far out (C-2:
                    // "a limit that's pretty far away where velocity stops
                    // increasing"). The first C-2 cut made a radius term the
                    // MINIMUM everywhere, which silently slowed everything
                    // above ~40 km by 3-8x (Andrew felt it within a day).
                    // Past the cap your angular pace keeps falling, so the
                    // planet's rotation reads clearly when you drift or stop.
                    let speed_kms =
                        (camera.altitude_km * 0.5).clamp(0.02, FAR_SPEED_CAP_KMS);
                    let sprint = if input.sprint { 4.0 } else { 1.0 };
                    let h = camera.heading(strafe, fwd);
                    camera.translate(h, speed_kms * sprint * dt);
                }
                let dir2 = camera.position().normalize();
                if voxels_live {
                    // near the ground, absolute height is preserved when the
                    // ground re-samples, so a cave pit passing underneath no
                    // longer yanks the camera: descend deliberately and drop
                    // in. The reference is the voxel *support* under the
                    // camera (cave floors count) and roofs are solid.
                    let cur = camera.ground_km + camera.altitude_km;
                    let ground = support_below_km(planet, edits, dir2, cur - 1e-9, exagg);
                    let ceil = ceiling_above_km(planet, edits, dir2, ground + 1e-6, exagg);
                    let height = cur
                        .max(ground + 0.0025)
                        .min(ceil - 0.0008)
                        .max(ground + 0.0012);
                    camera.ground_km = ground;
                    camera.altitude_km = height - ground;
                    // re-baseline the cruise elevation-lock when we climb back
                    // out of the voxel range
                    self.fly_riding = false;
                } else {
                    // Cruising above the voxel range: LOCK TO ELEVATION (C-1).
                    // Hold a constant planet-center radius as WASD moves us,
                    // instead of the old constant height-above-ground (which
                    // bobbed the camera up and over every mountain). The held
                    // elevation is `fly_elev_km` (= ground_km + altitude_km =
                    // radius above the planet baseline). We re-baseline it to
                    // the player's current elevation every tick they're freely
                    // above the terrain — that tracks deliberate altitude
                    // changes (scroll, teleport) for free — but FREEZE it while
                    // riding terrain, so a peak poking above the held radius
                    // lifts us over it and then we settle back to the held
                    // radius on the far side (never underground: max of held vs
                    // terrain). Continuous with the voxel branch, which also
                    // preserves ground_km + altitude_km, so crossing the 2.5 km
                    // boundary doesn't snap.
                    let ground = crate::terrain::ground_height_km(planet, dir2, exagg);
                    let floor = ground + CRUISE_FLOOR_KM;
                    if !self.fly_riding {
                        self.fly_elev_km = camera.ground_km + camera.altitude_km;
                    }
                    let elevation = self.fly_elev_km.max(floor);
                    self.fly_riding = self.fly_elev_km < floor;
                    camera.ground_km = ground;
                    camera.altitude_km = elevation - ground;
                }
            }
            Mode::Walk => {
                self.fly_riding = false;
                let mut feet = camera.ground_km;
                // -- horizontal, with side collision and 1-block step-up
                if fwd != 0.0 || strafe != 0.0 {
                    let sprint = if input.sprint { 2.2 } else { 1.0 };
                    let h = camera.heading(strafe, fwd);
                    let saved = (camera.lat, camera.lon, camera.yaw);
                    camera.translate(h, 0.0043 * sprint * dt); // 4.3 m/s
                    let ndir = camera.position().normalize();
                    let block = VOXEL_KM * exagg;
                    let step = if self.grounded { 1.05 * block } else { 0.05 * block };
                    let head = feet + EYE_KM + 0.0003;
                    // highest solid under the head in the target column: at or
                    // below the feet it's floor (walk on / fall past), within a
                    // step it's a stair, above that it's a wall
                    let s_head = support_below_km(planet, edits, ndir, head, exagg);
                    let new_feet = feet.max(s_head);
                    let headroom = ceiling_above_km(planet, edits, ndir, new_feet + 1e-6, exagg)
                        - new_feet
                        > EYE_KM + 0.0004;
                    let mut blocked = s_head > feet + step + 1e-9 || !headroom;
                    // body radius: the eye stays ~0.35 blocks away from any
                    // wall, so the near plane can never poke inside a block
                    // (walking face-first into a tree trunk showed its
                    // hollow interior). Probes ring the new position — but a
                    // violated probe only blocks movement TOWARD it: placing
                    // blocks beside yourself puts a wall inside the ring
                    // instantly, and blocking every direction then deadlocks
                    // you forever. Directional blocking lets you escape and
                    // slide along walls while still keeping the standoff on
                    // approach.
                    if !blocked {
                        let r_km = 0.35 * block;
                        let pos = camera.position();
                        let (_, north, east) = camera.frame();
                        for k in 0..8 {
                            let a = k as f64 * std::f64::consts::FRAC_PI_4;
                            let probe = north * a.cos() + east * a.sin();
                            if h.dot(probe) <= 0.1 {
                                continue; // not moving toward this probe
                            }
                            let pdir = (pos + probe * r_km).normalize();
                            let s = support_below_km(planet, edits, pdir, head, exagg);
                            // headroom is measured above where the body would
                            // rest on THIS probe's support (its step-up level),
                            // not above the current low feet — otherwise a
                            // steppable 1-block ledge/rim reads as a zero-
                            // headroom wall and traps you (e.g. a dug hole).
                            let foot = new_feet.max(s);
                            if s > new_feet + step + 1e-9
                                || ceiling_above_km(planet, edits, pdir, foot + 1e-6, exagg)
                                    - foot
                                    <= EYE_KM + 0.0004
                            {
                                blocked = true;
                                break;
                            }
                        }
                    }
                    if blocked {
                        (camera.lat, camera.lon, camera.yaw) = saved;
                    } else {
                        feet = new_feet;
                        if s_head > camera.ground_km {
                            self.vert_vel_mps = self.vert_vel_mps.max(0.0);
                        }
                    }
                }
                let dir2 = camera.position().normalize();
                // -- vertical: gravity (or buoyancy), landing, head bump
                let water = water_surface_km(planet, edits, dir2, exagg);
                let in_water = water.is_some_and(|w| feet + 0.0009 < w);
                if in_water {
                    // sink slowly; hold Space to swim up
                    let target = if input.swim_up { 3.0 } else { -1.4 };
                    let blend = (6.0 * dt).min(1.0);
                    self.vert_vel_mps += (target - self.vert_vel_mps) * blend;
                } else {
                    self.vert_vel_mps = (self.vert_vel_mps - 9.81 * dt).max(-80.0);
                }
                let mut new_feet = feet + self.vert_vel_mps * dt / 1000.0;
                let support = support_below_km(planet, edits, dir2, feet + 1e-7, exagg);
                self.grounded = false;
                if new_feet <= support {
                    new_feet = support;
                    self.vert_vel_mps = 0.0;
                    self.grounded = true;
                } else if self.vert_vel_mps > 0.0 {
                    let ceil = ceiling_above_km(planet, edits, dir2, feet + EYE_KM, exagg);
                    if new_feet + EYE_KM + 0.0004 > ceil {
                        new_feet = (ceil - EYE_KM - 0.0004).max(support);
                        self.vert_vel_mps = 0.0;
                    }
                }
                camera.ground_km = new_feet;
                camera.altitude_km = EYE_KM;
            }
        }
        // one place decides `underwater`, for both modes, from the final pose
        self.refresh_underwater(planet, edits, camera, exagg);
    }
}

/// Break (dh = -1) or place (dh = +1) a block at the targeted column.
/// Breaking removes the top block of the column you hit; placing is
/// face-aware: aiming at the side of something builds on the column in
/// front of it. Returns the chunks whose meshes went stale, or None if
/// nothing was in reach.
pub fn edit_block(
    planet: &Planet,
    edits: &mut Edits,
    camera: &Camera,
    mode: Mode,
    dh: i64,
    exaggeration: f64,
) -> Option<Vec<ChunkKey>> {
    let reach_m = if mode == Mode::Walk { 8.0 } else { 60.0 };
    let (hit, prev) = raycast_column(
        planet,
        edits,
        camera.position(),
        camera.look_dir(),
        reach_m,
        exaggeration,
    )?;
    let (face, ci, cj) = if dh > 0 { prev } else { hit };
    // You can't place a block into your own body. Looking straight down while
    // walking, the placement column IS the column you stand in, and the new
    // block lands at your feet — without this it embeds the head in solid rock
    // and renders the block interior (a starfield void). Fly mode is noclip, so
    // this only guards the walking body.
    if dh > 0 && mode == Mode::Walk && (face, ci, cj) == column_id(camera.position().normalize()) {
        let surf_top = surface_height_km(planet, edits, camera.position().normalize(), exaggeration);
        let block = VOXEL_KM * exaggeration;
        let feet = camera.ground_km;
        let head = feet + EYE_KM;
        // reject when the new block [surf_top, surf_top+block] would overlap the
        // body [feet, head]; a pillar-jump (feet already above surf_top+block)
        // still places, so you can build up under yourself while airborne.
        if surf_top < head + 0.0004 && surf_top + block > feet + 1e-6 {
            return None;
        }
    }
    *edits.entry((face, ci, cj)).or_insert(0) += dh;
    Some(chunks_touching_column(face, ci, cj))
}

/// Toggle a torch on the walkable top of the targeted column. Returns the
/// stale chunks, or None if nothing was in reach.
pub fn toggle_torch(
    planet: &Planet,
    edits: &Edits,
    torches: &mut Torches,
    camera: &Camera,
    mode: Mode,
    exaggeration: f64,
) -> Option<Vec<ChunkKey>> {
    let reach_m = if mode == Mode::Walk { 8.0 } else { 60.0 };
    let ((face, ci, cj), _) = raycast_column(
        planet,
        edits,
        camera.position(),
        camera.look_dir(),
        reach_m,
        exaggeration,
    )?;
    if !torches.remove(&(face, ci, cj)) {
        torches.insert((face, ci, cj));
    }
    Some(chunks_touching_column(face, ci, cj))
}

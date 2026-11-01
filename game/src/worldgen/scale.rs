//! The local/global split (DESIGN.md, "Local and global"; round 2 #2a).
//!
//! Terrain has two kinds of thing in it, and this is the one place in the
//! code where they are told apart:
//!
//! - **Global** features belong to the planet. They are positions on the
//!   sphere and they stretch with its radius: the bake's rasters (continents,
//!   ocean basins, climate, where the mountain belts run), the river network
//!   and the lakes. Their *heights* are the bake's Earth-scale elevations times
//!   the vertical scale: `Scale::global_m`. Every height that comes from the
//!   bake goes through it, so `grep global_m` finds every global height.
//! - **Local** features belong to the ground under your feet and are sized in
//!   metres, the same on any planet: peaks, ridges, valleys, hills, gullies,
//!   roughness, river channels and banks, beaches, lake shores, strata. They
//!   are evaluated in metres along the surface (`Scale::chart_m`) and their
//!   amplitudes are metre constants in `landform.rs`, `hydro.rs` and
//!   `coast.rs` that never multiply by the vertical scale.
//!
//! The planet's radius sets the vertical scale (R / Earth's R when the
//! world gives none), so changing the radius stretches the global list and
//! leaves the local one alone (`relief_tests::local_features_keep_their_metres`).
//!
//! The world is finite: nothing stands above `ceiling_m` (half the world
//! height, where sea level sits in the middle). Global and local heights
//! are summed and then eased toward it (`soft_ceiling`), so a range that
//! would overtop the world rounds off under it instead of flat-topping.

/// Where the easing toward the ceiling begins, as a share of it.
const CEILING_KNEE: f64 = 0.72;
/// The height the easing approaches, as a share of the ceiling (a few
/// blocks of air stay above the highest block).
const CEILING_TOP: f64 = 0.985;
/// How high the local relief's peaks stand, as a share of its relief scale
/// (`landform.rs`: about 1.25), for `headroom`.
const PEAK_SHARE: f64 = 1.25;

#[derive(Clone, Copy, Debug)]
pub struct Scale {
    /// Metres of terrain per metre of bake elevation (R / Earth's R by default).
    pub vscale: f64,
    /// Sea-level radius (m).
    pub radius: f64,
    /// Block size (m).
    pub block_m: f64,
    /// Highest ground the world can hold (m above sea): half the world
    /// height, or the room above sea level if that is less.
    pub ceiling_m: f64,
}

impl Scale {
    pub fn new(vscale: f64, radius: f64, block_m: f64, height_blocks: u32, sea_level_blocks: u32) -> Self {
        let room = (height_blocks as f64 - sea_level_blocks as f64) * block_m;
        let half = 0.5 * height_blocks as f64 * block_m;
        Self { vscale, radius, block_m, ceiling_m: room.min(half) }
    }

    /// **Global:** a bake elevation (km, Earth scale) as a height on this
    /// planet (m above sea). Stretches with the radius.
    #[inline]
    pub fn global_m(&self, elev_km: f64) -> f64 {
        elev_km * 1000.0 * self.vscale
    }

    /// **Global:** a bake gradient (km per radian of a chart axis) as rise
    /// over run on this planet.
    #[inline]
    pub fn global_slope(&self, km_per_rad: f64) -> f64 {
        km_per_rad * 1000.0 * self.vscale / self.radius
    }

    /// **Local:** a chart angle (radians) as metres along the surface. Local
    /// features are evaluated in these metres, so they keep their size when
    /// the radius changes.
    #[inline]
    pub fn chart_m(&self, angle: f64) -> f64 {
        angle * self.radius
    }

    /// How much of the local relief stands where the global surface is
    /// `base_m`: all of it where its peaks (about `PEAK_SHARE` x the relief
    /// scale) fit between the global surface and the ceiling's top, less
    /// where they would not, none where the global surface reaches the top
    /// itself. On the 90 km planet this is 1 everywhere; on an Earth
    /// analogue the bake's own heights take most of the room on the belts,
    /// and without this the easing below squashed whole ranges into a flat
    /// cap at the top. Smooth in the base, so LODs agree, and never above 1,
    /// so every bound on the relief still holds.
    #[inline]
    pub fn headroom(&self, base_m: f64, relief_m: f64) -> f64 {
        let room = CEILING_TOP * self.ceiling_m - base_m;
        (room / (PEAK_SHARE * relief_m + 1.0)).clamp(0.0, 1.0)
    }

    /// The world's ceiling: heights above the knee ease toward `CEILING_TOP`
    /// of the ceiling and never reach it. Monotone, continuous in slope, and
    /// never steeper than 1, so bounds and band limits pass through it.
    #[inline]
    pub fn soft_ceiling(&self, h: f64) -> f64 {
        let knee = CEILING_KNEE * self.ceiling_m;
        if h <= knee {
            return h;
        }
        let span = (CEILING_TOP - CEILING_KNEE) * self.ceiling_m;
        knee + span * ((h - knee) / span).tanh()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_is_half_the_world_height_and_the_easing_is_gentle() {
        let s = Scale::new(1.0 / 70.0, 90_569.0, 1.0, 2048, 512);
        assert_eq!(s.ceiling_m, 1024.0, "sea at 512: the design ceiling is still half the height");
        let t1 = Scale::new(1.0 / 70.0, 90_569.0, 1.0, 2048, 1024);
        assert_eq!(t1.ceiling_m, 1024.0);
        let half = Scale::new(0.05, 90_569.0, 0.5, 4096, 1024);
        assert_eq!(half.ceiling_m, 1024.0, "half-metre blocks: 4096 blocks are 2048 m, half of it is 1024 m");
        let mut prev = -10.0;
        let mut x = -10.0;
        while x < 3000.0 {
            let y = s.soft_ceiling(x);
            assert!(y >= prev - 1e-9, "monotone at {x}");
            assert!(y - prev <= 0.5 + 1e-9, "slope under 1 at {x}: {}", y - prev);
            assert!(y < CEILING_TOP * s.ceiling_m, "under the ceiling at {x}: {y}");
            if x <= CEILING_KNEE * s.ceiling_m {
                assert_eq!(y, x, "unchanged below the knee");
            }
            prev = y;
            x += 0.5;
        }
        assert!(s.soft_ceiling(900.0) > 850.0, "a 900 m peak keeps most of its height: {}", s.soft_ceiling(900.0));
        assert!(s.soft_ceiling(5000.0) > 0.97 * s.ceiling_m, "a range far over the top rounds off just under the ceiling");
    }
}

//! Geological strata as real blocks.
//!
//! Under the soil the rock is a stack of beds. A bed boundary is a level
//! surface of `z_world + fold(x, y)`: the fold is low-frequency gradient
//! noise (5-30 m on a 1-3 km wavelength) so beds dip visibly across a
//! landscape at this scale; a small block-scale warp keeps boundaries from
//! reading as perfectly flat. Bed thickness is 2-6 blocks, hashed per bed,
//! and each bed's rock is hashed from its index, so every cliff and cut
//! shows the same beds at the same elevations. Edges are hard; the palette
//! is the old viewer's shader tints at full strength on stone (the approved
//! look, per the feature inventory). Nothing here scales with the terrain's
//! vertical scale.
//!
//! Hook: `GeoHint` carries the geology fields the new bake will add
//! (craton, volcanic, hardness). They are zero today; `rock_at` already
//! routes through them so basalt, granite basement and caprock can be
//! switched in without touching callers.

use super::noise::{gradient_noise, hash01};
use crate::world::BlockId;
use glam::DVec3;

/// Geology fields from the bake (all 0..1; zero until the new bake lands).
#[derive(Clone, Copy, Debug, Default)]
pub struct GeoHint {
    pub craton: f32,
    pub volcanic: f32,
    pub hardness: f32,
}

#[derive(Clone, Debug)]
pub struct Strata {
    /// Sedimentary beds a layer can be, cycled by hash.
    pub beds: Vec<BlockId>,
    /// Future: volcanic cover and basement (used when the hints say so).
    pub basalt: Option<BlockId>,
    pub basement: Option<BlockId>,
    /// Block size in metres (bed thicknesses are in blocks).
    pub block_m: f64,
    /// Two beds share every `pair_blocks` of height; the split between
    /// them is hashed, so each bed is 2..6 blocks when pair_blocks = 8.
    pub pair_blocks: f64,
    /// Fold: amplitude (m) and wavelength (m), two octaves.
    pub fold_amp_m: f64,
    pub fold_wl_m: f64,
    /// Block-scale undulation of the boundaries: amplitude (m), wavelength (m).
    pub warp_amp_m: f64,
    pub warp_wl_m: f64,
    pub radius_m: f64,
    pub seed: u64,
}

impl Strata {
    pub fn new(beds: Vec<BlockId>, block_m: f64, radius_m: f64, seed: u64) -> Self {
        Self {
            beds,
            basalt: None,
            basement: None,
            block_m,
            pair_blocks: 8.0,
            fold_amp_m: 18.0,
            fold_wl_m: 2000.0,
            warp_amp_m: 0.6,
            warp_wl_m: 90.0,
            radius_m,
            seed: seed ^ 0xDED7_DCEC_5EED_0001,
        }
    }

    /// Fold plus warp displacement (m) for a column direction: constant down
    /// a column, so it is computed once per column and the per-voxel bed
    /// lookup needs no noise at all.
    #[inline]
    pub fn column_offset_m(&self, dir: DVec3) -> f64 {
        let p = dir * self.radius_m;
        self.fold_amp_m * gradient_noise(p / self.fold_wl_m, self.seed)
            + self.fold_amp_m * 0.35 * gradient_noise(p / (self.fold_wl_m * 0.31) + DVec3::splat(7.3), self.seed.wrapping_add(1))
            + self.warp_amp_m * gradient_noise(p / self.warp_wl_m + DVec3::splat(2.9), self.seed.wrapping_add(2))
    }

    /// Rock block at elevation `elev_m` (above sea level) in a column with
    /// offset `col_off` from `column_offset_m`.
    #[inline]
    pub fn rock_at(&self, elev_m: f64, col_off: f64, geo: GeoHint) -> BlockId {
        if let Some(b) = self.basalt
            && geo.volcanic > 0.5
        {
            return b;
        }
        let f = (elev_m + col_off) / self.block_m; // in blocks
        let pair = (f / self.pair_blocks).floor();
        let local = f - pair * self.pair_blocks;
        let split = 2.0 + hash01(pair as i64, 3, -11, self.seed) * (self.pair_blocks - 4.0);
        let bed = pair as i64 * 2 + (local >= split) as i64;
        let pick = (hash01(bed, 17, -31, self.seed) * self.beds.len() as f64) as usize;
        self.beds[pick.min(self.beds.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beds_are_2_to_6_blocks_and_fold_gently() {
        let s = Strata::new(vec![10, 11, 12, 13, 14], 1.0, 90_569.0, 42);
        let dir = DVec3::new(0.3, 0.5, 0.81).normalize();
        let off = s.column_offset_m(dir);
        assert!(off.abs() <= 18.0 * 1.35 + 0.6 + 1e-9);
        // Bed boundaries down one column: every run of equal rock is a whole
        // number of beds, so runs are at least 2 blocks (a bed) long.
        let mut runs = Vec::new();
        let mut last = s.rock_at(0.0, off, GeoHint::default());
        let mut len = 1;
        for z in 1..400 {
            let r = s.rock_at(-(z as f64), off, GeoHint::default());
            if r == last {
                len += 1;
            } else {
                runs.push(len);
                last = r;
                len = 1;
            }
        }
        assert!(runs.len() > 30, "{} runs", runs.len());
        let avg = runs.iter().sum::<i32>() as f64 / runs.len() as f64;
        assert!((3.0..=8.0).contains(&avg), "average run {avg}");
        // Beds split a pair of 8 blocks into 2..6 and 2..6.
        for pair in -20..20 {
            let base = pair as f64 * 8.0;
            let mut t = 0;
            while t < 8 && s.rock_at(base + t as f64 + 0.5, 0.0, GeoHint::default()) == s.rock_at(base + 0.5, 0.0, GeoHint::default()) {
                t += 1;
            }
            // either the first bed is 2..6 blocks or the two beds happen to share a rock
            assert!((2..=6).contains(&t) || t == 8, "first bed in pair {pair} is {t} blocks");
        }
    }
}

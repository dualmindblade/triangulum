//! Landforms v3: the local relief, in metres, standing on the bake's global
//! surface (`scale.rs` has the split).
//!
//! The bake knows where ranges, plateaus, basins and rivers are; it does
//! not know their ridges and valleys, and at the planet's vertical scale its
//! own relief is a few metres per kilometre. This module adds the relief
//! people see, as a height displacement `dh` (metres) on top of
//! `bake elevation x vertical scale`, sized the way real hills and mountains
//! are and the same on any planet (T2, round 2 #1):
//!
//! 1. **Massif** (peaks, ridges and spurs): octaves of 2D gradient noise
//!    from 8 km down to 64 m, domain-warped, folded into a ridge network
//!    (1 - |n|) where the terrain is rugged and left rounded where it is
//!    gentle, with a 1/f spectrum below the dominant wavelength and the
//!    finer octaves weaker in the valleys of the coarser ones (a
//!    multifractal: craggy crests, smooth valley floors). The relief in
//!    metres comes from the bake's global fields: hills everywhere from its
//!    local-relief field, mountains on its belts (active uplift, rough
//!    highlands), so a belt reads as mountains, a plain stays a plain and
//!    hills roll. **The bake's surface is the floor:** the massif and the
//!    gullies are summed and eased above zero (`FLOOR_V`), so valley floors
//!    lie on the bake and peaks stand above it, never below. The planet's
//!    ceiling is applied by the generator (`Scale::soft_ceiling`).
//! 2. **Gullies** (drainage): slope-aligned stripe noise (see
//!    `noise::gully2d`) whose crests and troughs run downhill along the
//!    slope of the bake and the massif; where neighbouring cells' stripes
//!    disagree they merge and split, which reads as branching. Depth grows
//!    with the relief and the slope and fades where the slope vanishes.
//!    Letting each finer octave also follow the coarser gullies' walls
//!    (`GULLY_BRANCH`) gives a more dendritic pattern but spins the finer
//!    stripes' phase within metres; it is off (see the constant).
//! 3. **Mesas and escarpments**: where rock is hard and tectonically quiet,
//!    the surface is terraced at levels that follow the strata, giving flat
//!    treads and steep risers (mean-preserving).
//! 4. **Fine roughness** (the roughness dial): a little 2D fBm from ~24 m
//!    down, stronger on steep ground.
//!
//! Scales are chosen like real hillslopes: the dominant wavelength grows
//! with the relief so slopes approach a threshold (~0.75) in big mountains
//! and stay gentle in lowlands: 850 m peaks stand about 3 km apart with
//! kilometre valleys, 30 m hills roll at 500 m. None of it changes with the
//! vertical scale or the radius: only *where* it is (the belts) does.
//!
//! Purity and LOD: everything is a function of (seed, direction) and the
//! bake. Each octave is faded out when its wavelength is under ~4 voxels,
//! and finer octaves are steered by the *unfaded* gradient of coarser ones,
//! so an octave's value never depends on the LOD, only its weight does
//! (the multifractal weight reads only octaves that are fully visible
//! wherever the octave it weights is visible at all). The massif's floor
//! and the fine terms are per column; the hills average to the bake plus
//! about a third of their relief, the mountains stand on it.
//!
//! Domain: each cube face has its own equal-angle chart (u, v) =
//! (atan a, atan b) in metres (R u, R v); it extends smoothly past the
//! face, and within `CHART_BLEND` of an edge the neighbouring charts are
//! evaluated too and cross-faded (variance-preserving), so no seam shows.

use super::bake::{BakeSample, chart_uv};
use super::noise::{gully2d, noise2d};
use super::scale::Scale;
use cubesphere::FACES;
use glam::DVec3;

/// Half-width of the chart cross-fade at cube edges, in direction-component
/// units (|d_a| - |d_b|); 0.04 is about +-2.6 km on Neisor.
const CHART_BLEND: f64 = 0.04;

// --- Local relief (metres, the same on any planet; Andrew's dials). The
// *where* comes from the bake's Earth-scale fields, which stretch with the
// planet; the metres do not.
/// Hills: the gentlest plain still rolls this much, and the bake's
/// local-relief field (km, Earth scale) adds up to `HILL_ROUGH_M` by
/// `HILL_ROUGH_KM`.
const HILL_FLOOR_M: f64 = 10.0;
const HILL_ROUGH_M: f64 = 60.0;
const HILL_ROUGH_KM: f64 = 0.25;
/// Hills vary in size from place to place: the relief is modulated by
/// +-`HILL_VARY` on `HILL_VARY_WL_M`.
const HILL_VARY: f64 = 0.55;
const HILL_VARY_WL_M: f64 = 7000.0;
/// Mountains: the relief scale of the strongest belt (peaks reach about
/// 1.5x it above the bake's surface, see `FLOOR_V`), and the belt weight
/// from active uplift (0..1.2) and the bake's local relief (km).
const MOUNT_M: f64 = 640.0;
/// Mesa country raises its hills by this share (tablelands), and no
/// terrace step is taller than STEP_MAX_M (a riser is a cliff).
const MESA_RAISE: f64 = 0.8;
const STEP_MAX_M: f64 = 40.0;
const MOUNT_UPLIFT_LO: f64 = 0.05;
const MOUNT_UPLIFT_HI: f64 = 0.6;
const MOUNT_ROUGH_LO_KM: f64 = 0.15;
const MOUNT_ROUGH_HI_KM: f64 = 0.9;
/// The floor: the massif and gullies, in units of the relief, are eased
/// above `FLOOR_V` (a smooth maximum of width `FLOOR_K`) and shifted so the
/// floor is the bake's surface. About a tenth of the land lies on it.
const FLOOR_V: f64 = -0.5;
const FLOOR_K: f64 = 0.15;
/// Spectrum below the dominant wavelength: octave amplitude falls as
/// (wavelength / dominant)^H.
const SPECTRUM_H: f64 = 1.3;
/// Octaves at or above this fraction of the dominant wavelength are the
/// ridge network (folded); finer ones are rounded bumps on its flanks, so
/// a range has a few long crests, not ridges at every scale.
const RIDGED_DOWN_TO: f64 = 0.35;
/// Largest gully cell (m): gullies are hillslope drainage (rills and
/// gullies), not valleys; the valleys between the ridges come from the
/// river network's ramp (`hydro.rs`) and the fold itself.
const GULLY_MAX_CELL_M: f64 = 128.0;
/// Multifractal: a finer octave's weight in the valleys of the coarser ones
/// (1 on their crests).
const MF_LO: f64 = 0.25;
/// Threshold hillslope and the relief at which slopes reach half of it
/// (the ruggedness: how much of the massif folds into crests).
const S_MAX: f64 = 0.75;
const RELIEF_HALF_M: f64 = 100.0;
/// Dominant (ridge-to-ridge) wavelength: LAM_PER_RELIEF x relief + LAM_BASE_M,
/// within the bounds (m). 630 m of relief gives 1.8 km: flanks near 0.5.
const LAM_PER_RELIEF: f64 = 2.5;
const LAM_BASE_M: f64 = 250.0;
const LAM_LO: f64 = 200.0;
const LAM_HI: f64 = 4096.0;
/// Domain warp of the massif: amplitude as a share of the dominant
/// wavelength (bends the crests without breaking them into swirls), on two
/// fixed wavelengths (`WARP_WL_M` and a quarter of it).
const WARP_AMP: f64 = 0.25;
const WARP_WL_M: f64 = 4096.0;
/// Massif octaves: 8192 m down to 64 m.
const MASSIF_TOP: f64 = 8192.0;
const MASSIF_OCTAVES: u32 = 8;
/// Gully octaves: 2048 m down to 16 m.
const GULLY_TOP: f64 = 2048.0;
const GULLY_OCTAVES: u32 = 8;
/// Gully depth = GULLY_BETA x relief x (cell / dominant wavelength)^0.6
/// (drainage even on gentle ground) + GULLY_ALPHA x cell x slope (deeper
/// on steep ground; slope capped at GULLY_SLOPE_CAP).
const GULLY_BETA_LOW: f64 = 0.4;
const GULLY_BETA_RUGGED: f64 = 0.12;
const GULLY_ALPHA: f64 = 0.05;
const GULLY_SLOPE_CAP: f64 = 0.8;
/// Fine roughness octaves (m) and base amplitude (m).
const FINE_TOP: f64 = 24.0;
const FINE_OCTAVES: u32 = 3;
const FINE_AMP_M: f64 = 0.35;
const FINE_SLOPE_AMP_M: f64 = 1.0;
/// Ridge crest rounding (in noise units) and the folded noise's mean.
const RIDGE_EPS: f64 = 0.06;
/// Crest rounding of the fold that steers the gullies, and the share of each
/// gully octave's slope that steers the finer ones. At 1 the finer gullies
/// run down the coarser ones' walls (truly dendritic), but those walls curve
/// so fast that the finer stripes' phase spins within metres: needle-like
/// spikes, and a surface the chunk grid cannot interpolate (worst 4.8 m at
/// 4 m voxels at 1/8 with 0.5). Off until the stripes are steered at their
/// cell points instead of at the sample (see the mission notes).
const STEER_EPS: f64 = 0.25;
const GULLY_BRANCH: f64 = 0.0;

/// Which octaves a chart evaluation covers (see `Landform::grid_cut`).
#[derive(Clone, Copy, Debug)]
enum Part {
    /// Everything: single columns, bounds, chunks near cube edges.
    All,
    /// A chunk's grid node: gully octaves with cells >= cut (the massif only steers them).
    Coarse { cut: f64 },
    /// A chunk's column: the coarse gullies interpolated (height, gradient),
    /// plus the massif, the gullies under the cut and the fine roughness.
    Fine { cut: f64, h: f64, gx: f64, gy: f64 },
}

/// Per-column parameters derived from the bake.
#[derive(Clone, Copy, Debug, Default)]
pub struct Local {
    /// Landform relief scale (m): peaks stand about 1.5x it above the floor.
    pub relief: f64,
    /// 0..1: how much this column is on a mountain belt (global: where).
    pub mountain: f64,
    /// Dominant (ridge-to-ridge) wavelength, m.
    pub lam_d: f64,
    /// 0 rounded hills .. 1 sharp ridge network.
    pub rugged: f64,
    /// 0..1: hard, quiet rock that terraces into mesas and escarpments.
    pub mesa: f64,
    /// Terrace step (m).
    pub step_m: f64,
    /// Fade toward coasts and on the sea floor.
    pub fac: f64,
    /// Deepest a river valley (and its channel) can cut here (m), 0 far
    /// from big rivers; for bounds.
    pub valley_m: f64,
}

/// What the landform adds to one column.
#[derive(Clone, Copy, Debug, Default)]
pub struct LandformSample {
    /// Height added to the bake surface (m), mesas included.
    pub dh: f64,
    /// Slope of the landform's massif and gullies (rise over run, unfaded).
    pub slope: f64,
    /// 0..1: how much this column is a river channel bed.
    pub river: f64,
    pub local: Local,
}

#[derive(Clone, Debug)]
pub struct Landform {
    pub seed: u64,
    pub radius: f64,
    /// The local/global split: global heights go through `scale.global_m`.
    pub scale: Scale,
    /// Andrew's landform dial: scales the relief (peak height, hills), the
    /// gullies and the mesas.
    pub landform: f64,
    /// Andrew's roughness dial: scales the fine roughness.
    pub roughness: f64,
    /// Carve valleys along the discharge field's ridges (`valley`). Off when
    /// the bake has a river network: `hydro.rs` carves real courses instead.
    pub valleys: bool,
}

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Octave fade for voxels of `voxel_m`: full at 8 voxels per wavelength,
/// gone at 4 (each LOD step fades one octave of a factor-2 stack).
#[inline]
fn band(wl: f64, voxel_m: f64) -> f64 {
    (wl / (4.0 * voxel_m) - 1.0).clamp(0.0, 1.0)
}

/// Per-octave rotation (golden angle steps) and offset, so the octaves'
/// lattices never line up.
#[inline]
fn octave_frame(k: u32, salt: u64) -> (f64, f64, f64, f64) {
    let a = (k as f64 + 1.0) * 2.399_963_229_728_653 + salt as f64 * 0.731;
    let (s, c) = a.sin_cos();
    (c, s, 17.31 * (k as f64 + 1.0) + salt as f64 * 3.7, -9.77 * (k as f64 + 1.0) - salt as f64 * 1.3)
}

impl Landform {
    /// Per-column parameters from the bake sample.
    pub fn local(&self, s: &BakeSample) -> Local {
        let elev_m = self.scale.global_m(s.elev_km.max(0.0));
        let erod = if s.erodibility > 0.0 { s.erodibility } else { 1.0 };
        // Mesa country: hard flat-lying rock (erodibility under ~0.8), no
        // active uplift, dry, raised above the lowlands, on a gentle
        // regional slope (about 2% of the land on seed 42).
        let bake_slope = self.scale.global_slope((s.elev_du_km * s.elev_du_km + s.elev_dv_km * s.elev_dv_km).sqrt());
        // Where the belts are (global): active uplift, or a rough highland;
        // a soft OR of the two, with a core (the power) so a belt has a
        // high heart and lower flanks.
        let m_up = smoothstep(MOUNT_UPLIFT_LO, MOUNT_UPLIFT_HI, s.uplift);
        let m_rough = smoothstep(MOUNT_ROUGH_LO_KM, MOUNT_ROUGH_HI_KM, s.rough_km);
        let mountain = 1.0 - (1.0 - m_up) * (1.0 - m_rough);
        // Mesa country is tableland, not a range: it keeps out of the belts
        // (a 700 m mountain cut into 60 m steps is a wedding cake).
        let mesa = if s.erodibility > 0.0 {
            smoothstep(0.84, 0.74, erod) * (1.0 - smoothstep(0.05, 0.25, s.uplift)) * smoothstep(700.0, 350.0, s.precip_mm) * smoothstep(0.2, 0.6, s.elev_km) * (1.0 - smoothstep(0.05, 0.15, bake_slope)) * (1.0 - smoothstep(0.2, 0.5, mountain))
        } else {
            0.0
        };
        // The relief in metres (local): hills everywhere, mountains on the belts.
        let hill_m = HILL_FLOOR_M + HILL_ROUGH_M * smoothstep(0.0, HILL_ROUGH_KM, s.rough_km);
        let mount_m = MOUNT_M * mountain * mountain.sqrt();
        // Mesa country raises the hills (its tablelands); mountains stand as they are.
        let relief0 = hill_m * (1.0 + MESA_RAISE * mesa) + mount_m;
        let s_max = S_MAX * (1.25 - 0.35 * erod).clamp(0.6, 1.1);
        // Ridge-to-ridge wavelength: real mountain flanks (about 0.5) and
        // gentle hills (about 0.1 on 300 m), harder rock a little steeper.
        let lam_d = ((LAM_PER_RELIEF * relief0 + LAM_BASE_M) * (S_MAX / s_max)).clamp(LAM_LO, LAM_HI);
        let s_t = s_max * relief0 / (relief0 + RELIEF_HALF_M);
        // Hills under about 150 m of relief stay rounded; mountains fold
        // into crests. Tablelands are rounded underneath; the terraces make
        // the edges.
        let rugged = smoothstep(0.25, 0.55, s_t) * (1.0 - mesa);
        let step_m = (0.45 * relief0).clamp(3.0, STEP_MAX_M);
        // Coasts: weaker at the shoreline (beaches, low coastal plains), and
        // a third on the sea floor.
        // Continuous across the shoreline: 0.3 on the sea floor and at the
        // water's edge, full strength a few metres up and away from the sea.
        let fac = 0.3 + 0.7 * smoothstep(0.0, 3.0, elev_m) * smoothstep(0.5, 0.05, s.ocean);
        // The discharge on a river's axis can exceed the local value by the
        // ridge's own height; half a decade covers the bake's ridges.
        let valley_m = if self.landform > 0.0 && self.valleys && s.flow_smooth > VALLEY_FLOW_LO - 0.5 { 2.0 + 0.3 * relief0 * self.landform + CHANNEL_DEPTH_M } else { 0.0 };
        Local { relief: relief0 * self.landform, mountain, lam_d, rugged, mesa, step_m, fac, valley_m }
    }

    /// The landform displacement at a direction, band-limited for voxels of
    /// `voxel_m`. `s` is the bake sampled at `dir`; `strata_off` the
    /// strata column offset (terrace levels follow the beds).
    pub fn sample(&self, dir: DVec3, s: &BakeSample, voxel_m: f64, strata_off: f64) -> LandformSample {
        self.sample_with(dir, s, voxel_m, strata_off, None)
    }

    /// `sample`, optionally with the coarse part already known (interpolated
    /// from a chunk's grid nodes: (cut, height, gradient x, gradient y), in
    /// the chart of the bake's face; only valid away from cube edges).
    pub fn sample_with(&self, dir: DVec3, s: &BakeSample, voxel_m: f64, strata_off: f64, coarse: Option<(f64, f64, f64, f64)>) -> LandformSample {
        let local = self.local(s);
        let (mut dh, slope) = if let Some((cut, h, gx, gy)) = coarse {
            let g0 = self.bake_gradient_in_chart(s.face, dir, s);
            let (h, sl, _, _) = self.chart(s.face, dir, g0, &local, voxel_m, Part::Fine { cut, h, gx, gy });
            (h, sl)
        } else {
            // Chart weights: one per axis, cross-faded near cube edges.
            let a = [dir.x.abs(), dir.y.abs(), dir.z.abs()];
            let comps = [dir.x, dir.y, dir.z];
            let mut acc_h = 0.0;
            let mut acc_s = 0.0;
            let mut acc_w = 0.0;
            let mut acc_w2 = 0.0;
            for k in 0..3 {
                let others = a[(k + 1) % 3].max(a[(k + 2) % 3]);
                let w = smoothstep(-CHART_BLEND, CHART_BLEND, a[k] - others);
                if w <= 0.0 {
                    continue;
                }
                let face = (2 * k + (comps[k] < 0.0) as usize) as u8;
                let g0 = self.bake_gradient_in_chart(face, dir, s);
                let (h, sl, _, _) = self.chart(face, dir, g0, &local, voxel_m, Part::All);
                acc_h += w * h;
                acc_s += w * sl;
                acc_w += w;
                acc_w2 += w * w;
            }
            // Variance-preserving cross-fade of independent fields.
            (acc_h / acc_w2.sqrt().max(1e-12), acc_s / acc_w)
        };
        // The floor: the massif and gullies (in relief units) eased above
        // FLOOR_V and shifted so the bake's surface is the floor. Applied
        // after the cross-fade, which wants zero-mean fields. The slope
        // follows the easing's own slope (0 on the floor, 1 above it).
        let mut slope = slope;
        if local.relief > 0.0 {
            let v = dh / local.relief;
            let (f, df) = floor_ease(v);
            dh = local.relief * f;
            slope *= df;
        }
        dh *= local.fac;
        // Mesas: terrace the total surface at levels that follow the strata.
        if local.mesa > 0.0 && self.landform > 0.0 {
            let base = self.scale.global_m(s.elev_km);
            let fade = local.mesa * (local.step_m / (2.0 * voxel_m) - 1.0).clamp(0.0, 1.0);
            if fade > 0.0 {
                let h = base + dh + strata_off;
                dh += fade * (terrace(h, local.step_m, 0.22) - h);
            }
        }
        // River valleys along the ridges of planetgen's discharge.
        let mut river = 0.0;
        if self.landform > 0.0 && self.valleys {
            let base = self.scale.global_m(s.elev_km);
            let (h, r) = self.valley(s, &local, base + dh, base);
            dh = h - base;
            river = r;
        }
        LandformSample { dh, slope, river, local }
    }

    /// River valleys. planetgen's discharge (`flow_log10`, smoothed) peaks
    /// along its rivers: a river course is a ridge of that field, even where
    /// the bake's elevation, smoothed over ~1 km here, shows no trough. Where
    /// the ridge is clear (strongly concave across, weakly along), the
    /// distance to its axis is a Newton step along the cross-ridge
    /// eigenvector, and a flat-floored valley is carved below the bake
    /// surface at the axis: floor width and depth grow with the discharge,
    /// walls rise at a slope set by the ruggedness, and a soft minimum
    /// blends them into the hills. A shallow channel runs down the middle.
    /// Takes the full surface `h` and the bake surface `base` (m); returns
    /// the new surface and the channel weight (0..1).
    fn valley(&self, s: &BakeSample, l: &Local, h: f64, base: f64) -> (f64, f64) {
        let r = self.radius;
        // Flow derivatives per metre along the chart axes.
        let (fx, fy) = (s.flow_du / r, s.flow_dv / r);
        let (fxx, fyy, fxy) = (s.flow_uu / (r * r), s.flow_vv / (r * r), s.flow_uv / (r * r));
        let half_tr = 0.5 * (fxx + fyy);
        let disc = (0.25 * (fxx - fyy) * (fxx - fyy) + fxy * fxy).sqrt();
        let (m1, m2) = (half_tr - disc, half_tr + disc); // m1 most negative
        if m1 >= -VALLEY_KAPPA {
            return (h, 0.0);
        }
        // Cross-ridge direction: eigenvector of m1.
        let (ax, ay) = (fxy, m1 - fxx);
        let (bx, by) = (m1 - fyy, fxy);
        let (nx, ny) = if ax * ax + ay * ay >= bx * bx + by * by { (ax, ay) } else { (bx, by) };
        let nl = (nx * nx + ny * ny).sqrt();
        if nl < 1e-30 {
            return (h, 0.0);
        }
        let (nx, ny) = (nx / nl, ny / nl);
        let fn_ = fx * nx + fy * ny;
        let step = fn_ / -m1; // signed metres to the ridge axis along n
        let d = step.abs();
        let f_axis = s.flow_smooth + 0.5 * fn_ * fn_ / -m1;
        let wf = smoothstep(VALLEY_FLOW_LO, VALLEY_FLOW_HI, f_axis);
        let aniso = (-m1 - m2.max(0.0)) / -m1;
        let w = wf * smoothstep(VALLEY_KAPPA, 2.0 * VALLEY_KAPPA, -m1) * smoothstep(0.3, 0.7, aniso) * smoothstep(2.0, 6.0, s.edge_texels) * smoothstep(0.6, 0.3, s.ocean);
        if w <= 0.0 {
            return (h, 0.0);
        }
        // Bake surface at the axis (first order along the step).
        let k1 = self.scale.global_slope(1.0);
        let axis = base + (s.smooth_du_km * k1 * nx + s.smooth_dv_km * k1 * ny) * step;
        let half_floor = VALLEY_FLOOR_M + VALLEY_FLOOR_PER_FLOW_M * (f_axis - VALLEY_FLOW_LO).clamp(0.0, 2.0);
        let depth = (2.0 + 0.3 * l.relief) * wf;
        let wall = 0.3 + 0.4 * l.rugged;
        let k = 0.25 * depth + 0.5;
        // Softplus keeps the floor-to-wall corner smooth.
        let x = d - half_floor;
        let rise = if x > 8.0 * k { x } else { k * (x / k).exp().ln_1p() };
        // Channel: a rounded trough down the middle of the floor.
        let cw = CHANNEL_HALF_M + CHANNEL_PER_FLOW_M * (f_axis - VALLEY_FLOW_LO).clamp(0.0, 2.0);
        let c = (1.0 - (d / cw) * (d / cw)).max(0.0);
        let channel = c * c;
        let target = axis - depth + wall * rise - CHANNEL_DEPTH_M * wf * channel;
        let out = h + w * (smin(h, target, k) - h);
        (out, w * smoothstep(0.15, 0.4, channel) * if out < h - 0.5 { 1.0 } else { 0.0 })
    }

    /// Bake elevation gradient (m per m, at the vertical scale) in the
    /// equal-angle chart of `face`, from the sample's own chart.
    fn bake_gradient_in_chart(&self, face: u8, dir: DVec3, s: &BakeSample) -> (f64, f64) {
        let (gu, gv) = (self.scale.global_slope(s.elev_du_km), self.scale.global_slope(s.elev_dv_km));
        if face == s.face {
            return (gu, gv);
        }
        // Chain rule through the chart change, Jacobian by central differences.
        let (u, v) = chart_uv(s.face, dir);
        let e = 1e-5;
        let to = |du: f64, dv: f64| {
            let (axis, right, up) = FACES[s.face as usize];
            chart_uv(face, (axis + (u + du).tan() * right + (v + dv).tan() * up).normalize())
        };
        let (pu, mu, pv, mv) = (to(e, 0.0), to(-e, 0.0), to(0.0, e), to(0.0, -e));
        // J[i][j] = d(chart_b_i) / d(chart_a_j)
        let j00 = (pu.0 - mu.0) / (2.0 * e);
        let j10 = (pu.1 - mu.1) / (2.0 * e);
        let j01 = (pv.0 - mv.0) / (2.0 * e);
        let j11 = (pv.1 - mv.1) / (2.0 * e);
        // grad_a = J^T grad_b  =>  grad_b = J^-T grad_a
        let det = j00 * j11 - j01 * j10;
        if det.abs() < 1e-12 {
            return (gu, gv);
        }
        ((j11 * gu - j10 * gv) / det, (-j01 * gu + j00 * gv) / det)
    }

    /// Massif + gullies + fine roughness in one chart, or the part of it
    /// that `part` selects. Returns (height, unfaded slope of massif and
    /// gullies, and in `Part::Coarse` the unfaded gradient its octaves add).
    fn chart(&self, face: u8, dir: DVec3, g0: (f64, f64), l: &Local, voxel_m: f64, part: Part) -> (f64, f64, f64, f64) {
        let (u, v) = chart_uv(face, dir);
        let (x, y) = (u * self.radius, v * self.radius);
        let seed = self.seed ^ (face as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        // Which octaves this call adds to the height.
        let (cut, coarse_only, fine_only) = match part {
            Part::All => (0.0, false, false),
            Part::Coarse { cut } => (cut, true, false),
            Part::Fine { cut, .. } => (cut, false, true),
        };
        let adds = |wl: f64| if coarse_only { wl >= cut } else if fine_only { wl < cut } else { true };
        let (mut h, mut gcx, mut gcy) = match part {
            Part::Fine { h, gx, gy, .. } => (h, gx, gy),
            _ => (0.0, 0.0, 0.0),
        };
        // Unfaded gradient of everything so far (steers the gullies).
        let (mut gx, mut gy) = (g0.0 + gcx, g0.1 + gcy);

        if l.relief > 0.0 {
            // --- Massif: warped ridge network around the dominant wavelength.
            // The massif is always per column (its ridge crests are too sharp to
            // interpolate); at a grid node it only steers the coarse gullies.
            // The warp's wavelengths are fixed: a wavelength that followed the
            // relief would sweep the warp through whole noise cells wherever
            // the relief changes (x / wl with x tens of kilometres), which
            // drew stripes along every belt edge. Only its amplitude follows.
            let aw = WARP_AMP * l.lam_d;
            let (mut wx, mut wy, mut wxx, mut wxy, mut wyx, mut wyy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            for (lw, share, sx, sy) in [(WARP_WL_M, 0.7, 0xA1u64, 0xA2u64), (0.25 * WARP_WL_M, 0.3, 0xA4, 0xA5)] {
                let (nx, nxx, nxy) = noise2d(x / lw + 5.2, y / lw - 1.3, seed ^ sx);
                let (ny, nyx, nyy) = noise2d(x / lw - 7.7, y / lw + 3.1, seed ^ sy);
                wx += share * nx;
                wy += share * ny;
                wxx += share * nxx / lw;
                wxy += share * nxy / lw;
                wyx += share * nyx / lw;
                wyy += share * nyy / lw;
            }
            let (xw, yw) = (x + aw * wx, y + aw * wy);
            // Jacobian of the warp, d(xw, yw)/d(x, y).
            let (j00, j01, j10, j11) = (1.0 + aw * wxx, aw * wxy, aw * wyx, 1.0 + aw * wyy);
            // Hills vary in size from place to place (mountains less so).
            let (vn, _, _) = noise2d(x / HILL_VARY_WL_M + 2.7, y / HILL_VARY_WL_M - 4.1, seed ^ 0xA3);
            let vary = 1.0 + HILL_VARY * (1.0 - l.mountain) * vn / NOISE_SD * 0.5;
            let relief = l.relief * vary.max(0.3);
            let mut wl = MASSIF_TOP;
            // The running massif value in relief units, from the octaves
            // above this one (all fully visible wherever this one shows).
            let mut acc = 0.0;
            for k in 0..MASSIF_OCTAVES {
                let wgt = massif_weight(wl, l.lam_d);
                let fade = band(wl, voxel_m);
                if fade <= 0.0 {
                    break;
                }
                if wgt > 0.0 {
                    let (c, sn, ox, oy) = octave_frame(k, 1);
                    let (qx, qy) = ((c * xw - sn * yw) / wl + ox, (sn * xw + c * yw) / wl + oy);
                    let (n, nqx, nqy) = noise2d(qx, qy, seed.wrapping_add(k as u64 * 0x51));
                    // d n / d(xw, yw), then through the warp to d n / d(x, y).
                    let (dxw, dyw) = ((c * nqx + sn * nqy) / wl, (-sn * nqx + c * nqy) / wl);
                    let (dnx, dny) = (dxw * j00 + dyw * j10, dxw * j01 + dyw * j11);
                    // Ridged fold: crest where n = 0, rounded by RIDGE_EPS.
                    let sa = (n * n + RIDGE_EPS * RIDGE_EPS).sqrt();
                    let ridged = (RIDGE_MEAN - sa) / RIDGE_SD;
                    // The gullies are steered by a blunter fold (STEER_EPS): a knife-edge
                    // crest would spin their direction within metres. Only the
                    // long octaves fold into crests; the finer ones are rounded.
                    let rugged = l.rugged * smoothstep(0.5 * RIDGED_DOWN_TO * l.lam_d, RIDGED_DOWN_TO * l.lam_d, wl);
                    let val = rugged * ridged + (1.0 - rugged) * n / NOISE_SD;
                    let dval = rugged * (-n / (n * n + STEER_EPS * STEER_EPS).sqrt() / RIDGE_SD) + (1.0 - rugged) / NOISE_SD;
                    // Multifractal: weaker in the valleys of the coarser octaves.
                    let mf = if k == 0 { 1.0 } else { MF_LO + (1.0 - MF_LO) * smoothstep(-0.6, 0.4, acc) };
                    let amp = 0.25 * relief * wgt * mf;
                    gx += amp * dval * dnx;
                    gy += amp * dval * dny;
                    if !coarse_only {
                        h += amp * val * fade;
                    }
                    acc += 0.25 * wgt * mf * val * fade;
                }
                wl *= 0.5;
            }

            // --- Gullies: downhill stripes, each octave steered by the ones above.
            let lv = Local { relief, ..*l };
            let mut c = GULLY_TOP;
            for k in 0..GULLY_OCTAVES {
                let fade = band(c, voxel_m);
                if fade <= 0.0 {
                    break;
                }
                // Octaves from just under the dominant wavelength down.
                let wgt = gully_weight(&lv, c);
                let s = (gx * gx + gy * gy).sqrt();
                if wgt > 0.0 && s > 1e-9 && adds(c) {
                    // Across the slope, unit length on real slopes but
                    // shrinking to zero where the slope vanishes: the stripe
                    // phase is measured up to 1.25 cells from a cell point,
                    // so a spinning direction at a hilltop or saddle would
                    // otherwise sweep it through whole cycles in metres.
                    let s0 = 0.5 * lv.relief / lv.lam_d + 1e-6;
                    let sn = (s * s + s0 * s0).sqrt();
                    let (tx, ty) = (-gy / sn, gx / sn);
                    let (cr, sr, ox, oy) = octave_frame(k, 2);
                    let (qx, qy) = ((cr * x - sr * y) / c + ox, (sr * x + cr * y) / c + oy);
                    let (qtx, qty) = (cr * tx - sr * ty, sr * tx + cr * ty);
                    let (gv, gqx, gqy) = gully2d(qx, qy, qtx, qty, seed.wrapping_add(0x6011 + k as u64 * 0x77));
                    // Valley profile: v = 1 on a gully floor, 0 on a spur crest.
                    // Gentle ground keeps the rounded cosine; rugged ground
                    // uses v^2 (narrow floors, broad spurs). Both zero mean.
                    let v = 0.5 * (1.0 - gv);
                    let val = (1.0 - l.rugged) * (gv - GULLY_MEAN) - l.rugged * (v * v - GULLY_V2_MEAN) / (1.0 - GULLY_V2_MEAN);
                    let dv = (1.0 - l.rugged) + l.rugged * v / (1.0 - GULLY_V2_MEAN); // d val / d gv
                    let (dgx, dgy) = (dv * (cr * gqx + sr * gqy) / c, dv * (-sr * gqx + cr * gqy) / c);
                    let depth = wgt * gully_depth(&lv, c, s);
                    h += depth * val * fade;
                    gx += GULLY_BRANCH * depth * dgx;
                    gy += GULLY_BRANCH * depth * dgy;
                    gcx += GULLY_BRANCH * depth * dgx;
                    gcy += GULLY_BRANCH * depth * dgy;
                }
                c *= 0.5;
            }
        }
        if coarse_only {
            return (h, 0.0, gcx, gcy);
        }
        let (lx, ly) = (gx - g0.0, gy - g0.1);
        let slope = if l.relief > 0.0 { (lx * lx + ly * ly).sqrt() } else { 0.0 };

        // --- Fine roughness (roughness dial), stronger on steep ground.
        if self.roughness > 0.0 {
            let amp0 = self.roughness * (FINE_AMP_M + FINE_SLOPE_AMP_M * slope.min(1.0));
            let mut wl = FINE_TOP;
            let mut amp = amp0;
            for k in 0..FINE_OCTAVES {
                let fade = band(wl, voxel_m);
                if fade <= 0.0 {
                    break;
                }
                let (c, sn, ox, oy) = octave_frame(k, 3);
                let (n, _, _) = noise2d((c * x - sn * y) / wl + ox, (sn * x + c * y) / wl + oy, seed.wrapping_add(0xF1 + k as u64));
                h += amp * n / NOISE_SD * 0.5 * fade;
                wl *= 0.5;
                amp *= 0.5;
            }
        }
        (h, slope, 0.0, 0.0)
    }

    /// The octave cut for chunk-grid evaluation at a voxel size: octaves at
    /// or above it are sampled every 4 voxels and interpolated (8 or more
    /// samples per wavelength), the rest per column. None when no octave is
    /// that long.
    pub fn grid_cut(voxel_m: f64) -> Option<f64> {
        // 16 samples per cell: 8 left a 1.4 m worst case between the two
        // paths at 1/8 (the stripes' direction turns between nodes).
        let cut = 64.0 * voxel_m;
        (cut <= MASSIF_TOP.max(GULLY_TOP)).then_some(cut)
    }

    /// The coarse part (height, unfaded gradient) at a grid node in the
    /// chart of the bake's face, or None near a cube edge (where charts
    /// cross-fade and the chunk takes the direct path).
    pub fn coarse_node(&self, dir: DVec3, s: &BakeSample, voxel_m: f64, cut: f64) -> Option<(f64, f64, f64)> {
        let a = dir.abs();
        let comps = [a.x, a.y, a.z];
        let k = (s.face / 2) as usize;
        if comps[k] - comps[(k + 1) % 3].max(comps[(k + 2) % 3]) < CHART_BLEND {
            return None;
        }
        let local = self.local(s);
        let g0 = self.bake_gradient_in_chart(s.face, dir, s);
        let (h, _, gx, gy) = self.chart(s.face, dir, g0, &local, voxel_m, Part::Coarse { cut });
        Some((h, gx, gy))
    }

    /// Upper bound (m) on |dh| from every octave with a wavelength (or
    /// gully cell) under `wl_max` at a column with parameters `l`, plus the
    /// whole fine roughness and the terrace displacement. Massif values are
    /// bounded by MASSIF_MAX_ABS, gully values by 1, fine noise by 1 / NOISE_SD;
    /// the chart cross-fade can lift a sum by sqrt 2, the hill variation by
    /// 1 + HILL_VARY; the floor's easing never steepens anything.
    pub fn sub_amp_bound_m(&self, l: &Local, wl_max: f64) -> f64 {
        let mut b = 0.0;
        if l.relief > 0.0 {
            let lv = Local { relief: l.relief * (1.0 + HILL_VARY), ..*l };
            let mut wl = MASSIF_TOP;
            for _ in 0..MASSIF_OCTAVES {
                let wgt = massif_weight(wl, l.lam_d);
                if wl < wl_max && wgt > 0.0 {
                    b += 0.25 * lv.relief * wgt * MASSIF_MAX_ABS;
                }
                wl *= 0.5;
            }
            let mut c = GULLY_TOP;
            for _ in 0..GULLY_OCTAVES {
                if c < wl_max {
                    b += gully_weight(&lv, c) * gully_depth(&lv, c, GULLY_SLOPE_CAP);
                }
                c *= 0.5;
            }
        }
        let slope_max = 1.0;
        b += self.roughness * (FINE_AMP_M + FINE_SLOPE_AMP_M * slope_max) * 1.75 / NOISE_SD * 0.5;
        b * l.fac * 2f64.sqrt() + if l.mesa > 0.0 { l.step_m } else { 0.0 } + l.valley_m
    }

    /// Upper bound (m) on |dh| anywhere for a relief scale (for the horizon
    /// occluder and other global margins).
    pub fn bound_m(&self, relief: f64) -> f64 {
        let relief = relief * (1.0 + HILL_VARY);
        let l = Local { relief, mountain: 1.0, lam_d: LAM_LO, rugged: 1.0, mesa: 1.0, step_m: (0.45 * relief / self.landform.max(1e-9)).clamp(3.0, STEP_MAX_M), fac: 1.0, valley_m: 2.0 + 0.3 * relief + CHANNEL_DEPTH_M };
        // Every massif octave at its largest weight (conservative), all gullies.
        let massif = 0.25 * relief * MASSIF_MAX_ABS * 3.0;
        let gully: f64 = (0..GULLY_OCTAVES).map(|k| GULLY_BETA_LOW * relief + GULLY_ALPHA * GULLY_TOP * 0.5f64.powi(k as i32) * GULLY_SLOPE_CAP).sum();
        let fine = self.roughness * (FINE_AMP_M + FINE_SLOPE_AMP_M) * 1.75 / NOISE_SD * 0.5;
        // The floor lifts everything by up to -FLOOR_V (plus the easing's width).
        (massif + gully + fine) * 2f64.sqrt() + (FLOOR_K - FLOOR_V) * relief + l.step_m + l.valley_m
    }

    /// The largest relief scale (m) any column can have where the bake's
    /// local relief is at most `rough_km` (the belt weight at its worst),
    /// for bounds.
    pub fn relief_bound_m(&self, rough_km: f64, _elev_km: f64) -> f64 {
        (HILL_FLOOR_M + HILL_ROUGH_M * smoothstep(0.0, HILL_ROUGH_KM, rough_km.max(0.0)) + MOUNT_M) * 2.5 * self.landform
    }

    /// How much of this column's relief is a mountain belt's (0..1), for
    /// the valley widths of `hydro.rs`.
    pub fn mountain(&self, s: &BakeSample) -> f64 {
        self.local(s).mountain
    }
}

/// Spectrum of the massif: full at the dominant wavelength, 1/f^H below it,
/// a soft cutoff above it (nothing from three times it up).
#[inline]
fn massif_weight(wl: f64, lam_d: f64) -> f64 {
    let r = wl / lam_d;
    if r > 1.0 { smoothstep(3.0, 1.0, r) } else { r.powf(SPECTRUM_H) }
}

/// The floor: a smooth maximum of `v` (the massif and gullies in relief
/// units) with `FLOOR_V`, shifted so the floor is 0. Returns the value and
/// its slope d/dv (0 on the floor, 1 well above it, never more).
#[inline]
fn floor_ease(v: f64) -> (f64, f64) {
    let k = FLOOR_K;
    let x = v - FLOOR_V;
    if x >= k {
        return (x, 1.0);
    }
    if x <= -k {
        return (0.0, 0.0);
    }
    // Quadratic blend between the two lines over [-k, k]: C1, (x + k)^2 / 4k.
    let t = (x + k) / (2.0 * k);
    (k * t * t, t)
}

/// River valleys: discharge range (log10 m^3/s) over which carving fades in,
/// the flow ridge's curvature threshold (decades per m^2), floor half-width
/// (m) and its growth per decade of discharge.
const VALLEY_FLOW_LO: f64 = 1.6;
const VALLEY_FLOW_HI: f64 = 2.6;
const VALLEY_KAPPA: f64 = 1.0e-6;
const VALLEY_FLOOR_M: f64 = 20.0;
const VALLEY_FLOOR_PER_FLOW_M: f64 = 50.0;
/// The channel down the valley floor: half-width (m), growth per decade of
/// discharge, depth (m).
const CHANNEL_HALF_M: f64 = 4.0;
const CHANNEL_PER_FLOW_M: f64 = 5.0;
const CHANNEL_DEPTH_M: f64 = 1.6;

/// Polynomial smooth minimum (blend width k).
#[inline]
fn smin(a: f64, b: f64, k: f64) -> f64 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

/// Gully octaves run from just under the dominant wavelength (at most
/// `GULLY_MAX_CELL_M`) down three octaves.
#[inline]
fn gully_weight(l: &Local, c: f64) -> f64 {
    let top = (0.4 * l.lam_d).min(GULLY_MAX_CELL_M);
    smoothstep(2.0 * top, top, c) * smoothstep(top / 16.0, top / 8.0, c)
}

/// Gully depth (m) for cell size `c` on ground of slope `s`.
#[inline]
fn gully_depth(l: &Local, c: f64, s: f64) -> f64 {
    let beta = GULLY_BETA_LOW + (GULLY_BETA_RUGGED - GULLY_BETA_LOW) * l.rugged;
    // Gullies need a slope to drain: where the steering gradient vanishes
    // (hilltops, pits, saddles) their direction spins, so they fade out
    // there instead of forming pinwheels. The reference slope is a fraction
    // of the terrain's own relief per wavelength.
    let s_ref = 0.25 * l.relief / l.lam_d;
    let sf = s / (s + s_ref + 1e-9);
    (beta * l.relief * (c / l.lam_d).powf(0.6) * sf + GULLY_ALPHA * c * s.min(GULLY_SLOPE_CAP)) * (l.relief / (l.relief + 2.0))
}

/// Largest |value| a massif octave can take: the ridged fold reaches
/// (RIDGE_MEAN - 1) / RIDGE_SD at |n| = 1, the rounded noise 1 / NOISE_SD.
const MASSIF_MAX_ABS: f64 = 4.4;

/// Standard deviation of `noise2d` and the mean / standard deviation of
/// its ridged fold sqrt(n^2 + eps^2), measured (see the test below), so
/// that rounded and ridged octaves have zero mean and unit spread.
const NOISE_SD: f64 = 0.297;
const RIDGE_MEAN: f64 = 0.258;
const RIDGE_SD: f64 = 0.173;
/// Mean of `gully2d` and of v^2 = ((1 - g) / 2)^2 (measured, see the test).
const GULLY_MEAN: f64 = -0.0258;
const GULLY_V2_MEAN: f64 = 0.3134;

/// Mean-preserving terrace of height `h` with steps `step` (m): flat treads
/// and a riser over the top `w` fraction of each step. Over a uniform
/// spread of heights the terraced surface has the same mean.
#[inline]
pub fn terrace(h: f64, step: f64, w: f64) -> f64 {
    let t = h / step;
    let f = t.floor();
    let x = t - f;
    step * (f + smoothstep(1.0 - w, 1.0, x) + 0.5 - 0.5 * w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_constants_match_measured_moments() {
        let n = 400_000;
        let (mut s2, mut ra, mut ra2) = (0.0, 0.0, 0.0);
        for i in 0..n {
            let (x, y) = ((i % 631) as f64 * 0.3711 + 0.13, (i / 631) as f64 * 0.4123 + 0.07);
            let (v, _, _) = noise2d(x, y, 77);
            s2 += v * v;
            let r = (v * v + RIDGE_EPS * RIDGE_EPS).sqrt();
            ra += r;
            ra2 += r * r;
        }
        let nf = n as f64;
        let sd = (s2 / nf).sqrt();
        let rm = ra / nf;
        let rsd = (ra2 / nf - rm * rm).sqrt();
        assert!((sd - NOISE_SD).abs() < 0.01, "noise sd {sd}");
        assert!((rm - RIDGE_MEAN).abs() < 0.01, "ridge mean {rm}");
        assert!((rsd - RIDGE_SD).abs() < 0.01, "ridge sd {rsd}");
    }

    /// On a plain slope (no massif: relief set so the massif weights are all
    /// zero is not possible, so compare the gully part alone through the
    /// coarse path), the gullies' crests run downhill: their height varies
    /// across the slope, hardly along it.
    #[test]
    fn gully_stripes_run_downhill_on_a_plain_slope() {
        let lf = Landform { seed: 7, radius: 90_569.2, scale: Scale::new(0.125, 90_569.2, 1.0, 4096, 1536), landform: 1.0, roughness: 0.0, valleys: true };
        let l = Local { relief: 60.0, mountain: 0.0, lam_d: 900.0, rugged: 0.0, mesa: 0.0, step_m: 10.0, fac: 1.0, valley_m: 0.0 };
        let face = 2u8;
        for &(gx0, gy0) in &[(0.3f64, 0.0f64), (0.0, 0.3), (0.21, -0.21)] {
            let g_len = (gx0 * gx0 + gy0 * gy0).sqrt();
            let (dx, dy) = (gx0 / g_len, gy0 / g_len); // downhill (up the gradient, same line)
            let (sx, sy) = (-dy, dx); // across
            let mut along = 0.0;
            let mut across = 0.0;
            for k in 0..400 {
                let (u0, v0) = (0.1 + (k % 20) as f64 * 3e-3, -0.05 + (k / 20) as f64 * 3e-3);
                let at = |du: f64, dv: f64| {
                    let dir = crate::worldgen::inspect::chart_dir(face, u0 + du / lf.radius, v0 + dv / lf.radius);
                    // Coarse part only: gullies with cells >= 32 m, steered by g0 (+ the massif's gradient).
                    lf.chart(face, dir, (gx0, gy0), &l, 1.0, Part::Coarse { cut: 32.0 }).0
                };
                for t in 0..10 {
                    let o = t as f64 * 6.0;
                    along += (at(dx * (o + 6.0), dy * (o + 6.0)) - at(dx * o, dy * o)).abs();
                    across += (at(sx * (o + 6.0), sy * (o + 6.0)) - at(sx * o, sy * o)).abs();
                }
            }
            println!("slope ({gx0}, {gy0}): gully change along {along:.1}, across {across:.1}");
            assert!(across > 2.0 * along, "slope ({gx0}, {gy0}): along {along:.1} across {across:.1}");
        }
    }

    #[test]
    fn gully_moments_match_constants() {
        let (mut s1, mut s2, mut sv2, n) = (0.0, 0.0, 0.0, 400_000);
        for i in 0..n {
            let (x, y) = ((i % 641) as f64 * 0.3711 + 0.13, (i / 641) as f64 * 0.4123 + 0.07);
            let a = (i as f64 * 0.618_034).fract() * std::f64::consts::TAU;
            let (g, _, _) = gully2d(x, y, a.cos(), a.sin(), 3);
            s1 += g;
            s2 += g * g;
            let v = 0.5 * (1.0 - g);
            sv2 += v * v;
        }
        let nf = n as f64;
        println!("gully mean {:.4} sd {:.4} E[v^2] {:.4}", s1 / nf, (s2 / nf - (s1 / nf).powi(2)).sqrt(), sv2 / nf);
        assert!((s1 / nf - GULLY_MEAN).abs() < 0.01, "gully mean {}", s1 / nf);
        assert!((sv2 / nf - GULLY_V2_MEAN).abs() < 0.01, "E[v^2] {}", sv2 / nf);
    }

    /// The floor's easing is continuous in value and slope, never steeper
    /// than 1, zero on the floor and the identity above it.
    #[test]
    fn floor_ease_is_c1_and_never_steepens() {
        let (mut prev, mut x) = (floor_ease(-3.0), -3.0);
        while x < 3.0 {
            x += 1e-3;
            let (f, df) = floor_ease(x);
            assert!(f >= 0.0 && f >= prev.0 - 1e-12, "monotone and non-negative at {x}");
            assert!((f - prev.0) <= 1e-3 + 1e-9, "slope under 1 at {x}");
            assert!((df - prev.1).abs() < 0.02, "slope continuous at {x}");
            let fd = (f - prev.0) / 1e-3;
            assert!((fd - 0.5 * (df + prev.1)).abs() < 0.02, "slope matches at {x}: {fd} vs {df}");
            prev = (f, df);
        }
        assert_eq!(floor_ease(FLOOR_V - FLOOR_K), (0.0, 0.0));
        assert_eq!(floor_ease(FLOOR_V + 1.0), (1.0, 1.0));
    }

    /// The massif's spectrum peaks at the dominant wavelength, falls as a
    /// power below it and is gone three times above it.
    #[test]
    fn massif_spectrum_is_a_band_around_the_dominant_wavelength() {
        assert_eq!(massif_weight(1000.0, 1000.0), 1.0);
        assert_eq!(massif_weight(3000.0, 1000.0), 0.0);
        assert!((massif_weight(500.0, 1000.0) - 0.5f64.powf(SPECTRUM_H)).abs() < 1e-12);
        assert!(massif_weight(1500.0, 1000.0) > 0.5 && massif_weight(1500.0, 1000.0) < 1.0);
    }

    #[test]
    fn terrace_preserves_the_mean_and_is_continuous() {
        let step = 7.0;
        let n = 100_000;
        let mut sum = 0.0;
        for i in 0..n {
            let h = -49.0 + 98.0 * i as f64 / n as f64; // 14 whole steps
            sum += terrace(h, step, 0.22) - h;
        }
        assert!((sum / n as f64).abs() < 0.01, "terrace bias {}", sum / n as f64);
        for i in -30..30 {
            let edge = i as f64 * step;
            assert!((terrace(edge - 1e-9, step, 0.22) - terrace(edge + 1e-9, step, 0.22)).abs() < 1e-6);
        }
    }
}

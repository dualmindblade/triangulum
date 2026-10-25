//! Landforms v2: the relief between the bake's scale (~1 km here) and the
//! metre scale, invented by the engine but driven by the bake's geology.
//!
//! The bake knows where ranges, plateaus, basins and rivers are; it does
//! not know their ridges and valleys. This module adds them as a height
//! displacement `dh` (metres) on top of `bake elevation x vertical scale`:
//!
//! 1. **Massif** (ridges and spurs): a few octaves of 2D gradient noise
//!    around a dominant wavelength, domain-warped, folded into a ridge
//!    network (1 - |n|) where the terrain is rugged and left rounded where
//!    it is gentle. Zero mean.
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
//! Scales are chosen like real hillslopes: relief R comes from the bake
//! (roughness, uplift times elevation, plus a floor so plains roll), and
//! the dominant wavelength grows with relief so slopes approach a
//! threshold (~0.75) in big mountains and stay gentle in lowlands. That
//! keeps the landforms believable at any vertical scale: at 1/70 a range
//! is rugged hills, at 1/8 it is mountains with kilometre valleys.
//!
//! Purity and LOD: everything is a function of (seed, direction) and the
//! bake. Each octave is faded out when its wavelength is under ~4 voxels,
//! and finer octaves are steered by the *unfaded* gradient of coarser ones,
//! so an octave's value never depends on the LOD, only its weight does.
//! Every term has zero mean, so averages over kilometres match the bake.
//!
//! Domain: each cube face has its own equal-angle chart (u, v) =
//! (atan a, atan b) in metres (R u, R v); it extends smoothly past the
//! face, and within `CHART_BLEND` of an edge the neighbouring charts are
//! evaluated too and cross-faded (variance-preserving), so no seam shows.

use super::bake::{BakeSample, chart_uv};
use super::noise::{gully2d, noise2d};
use cubesphere::FACES;
use glam::DVec3;

/// Half-width of the chart cross-fade at cube edges, in direction-component
/// units (|d_a| - |d_b|); 0.04 is about +-2.6 km on Neisor.
const CHART_BLEND: f64 = 0.04;

/// Relief: metres of landform per metre of bake roughness (both at the
/// vertical scale), per metre of uplifted elevation, and the floor.
const C_ROUGH: f64 = 0.5;
const C_UPLIFT: f64 = 0.3;
const RELIEF_FLOOR_M: f64 = 8.0;
/// Threshold hillslope and the relief at which slopes reach half of it.
const S_MAX: f64 = 0.75;
const RELIEF_HALF_M: f64 = 100.0;
/// Dominant wavelength bounds (m).
const LAM_LO: f64 = 200.0;
const LAM_HI: f64 = 2600.0;
/// Massif octaves: 4096 m down to 64 m.
const MASSIF_TOP: f64 = 4096.0;
const MASSIF_OCTAVES: u32 = 7;
const MASSIF_TOP_LOG2: f64 = 12.0;
/// Gully octaves: 1024 m down to 16 m.
const GULLY_TOP: f64 = 1024.0;
const GULLY_OCTAVES: u32 = 7;
/// Gully depth = GULLY_BETA x relief x (cell / dominant wavelength)^0.6
/// (drainage even on gentle ground) + GULLY_ALPHA x cell x slope (deeper
/// on steep ground; slope capped at GULLY_SLOPE_CAP).
const GULLY_BETA_LOW: f64 = 0.6;
const GULLY_BETA_RUGGED: f64 = 0.25;
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
    /// Landform relief scale (m, about peak to trough of the massif).
    pub relief: f64,
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
    pub vscale: f64,
    /// Andrew's landform dial: scales the massif, gullies and mesas.
    pub landform: f64,
    /// Andrew's roughness dial: scales the fine roughness.
    pub roughness: f64,
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
        let vs = self.vscale;
        let rough_m = s.rough_km.max(0.0) * 1000.0 * vs;
        let elev_m = s.elev_km.max(0.0) * 1000.0 * vs;
        let up = (s.uplift / 0.8).clamp(0.0, 1.0);
        let erod = if s.erodibility > 0.0 { s.erodibility } else { 1.0 };
        // Mesa country: hard flat-lying rock (erodibility under ~0.8), no
        // active uplift, dry, raised above the lowlands, on a gentle
        // regional slope (about 2% of the land on seed 42).
        let bake_slope = (s.elev_du_km * s.elev_du_km + s.elev_dv_km * s.elev_dv_km).sqrt() * 1000.0 * vs / self.radius;
        let mesa = if s.erodibility > 0.0 {
            smoothstep(0.84, 0.74, erod) * (1.0 - smoothstep(0.05, 0.25, s.uplift)) * smoothstep(700.0, 350.0, s.precip_mm) * smoothstep(0.2, 0.6, s.elev_km) * (1.0 - smoothstep(0.05, 0.15, bake_slope))
        } else {
            0.0
        };
        let relief0 = (C_ROUGH * rough_m + C_UPLIFT * up * elev_m + RELIEF_FLOOR_M) * (1.0 + 1.5 * mesa);
        let s_max = S_MAX * (1.25 - 0.35 * erod).clamp(0.6, 1.1);
        let lam_d = (std::f64::consts::PI * (relief0 + RELIEF_HALF_M) / s_max).clamp(LAM_LO, LAM_HI);
        let s_t = s_max * relief0 / (relief0 + RELIEF_HALF_M);
        // Tablelands are rounded underneath; the terraces make the edges.
        let rugged = smoothstep(0.12, 0.45, s_t) * (1.0 - mesa);
        let step_m = (0.45 * relief0).clamp(3.0, 60.0);
        // Coasts: weaker at the shoreline (beaches, low coastal plains), and
        // a third on the sea floor.
        let fac = if s.ocean > 0.5 && s.elev_km < 0.0 {
            0.35
        } else {
            0.3 + 0.7 * smoothstep(0.0, 3.0, elev_m) * smoothstep(0.5, 0.05, s.ocean)
        };
        // The discharge on a river's axis can exceed the local value by the
        // ridge's own height; half a decade covers the bake's ridges.
        let valley_m = if self.landform > 0.0 && s.flow_smooth > VALLEY_FLOW_LO - 0.5 { 2.0 + 0.3 * relief0 * self.landform + CHANNEL_DEPTH_M } else { 0.0 };
        Local { relief: relief0 * self.landform, lam_d, rugged, mesa, step_m, fac, valley_m }
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
        dh *= local.fac;
        // Mesas: terrace the total surface at levels that follow the strata.
        if local.mesa > 0.0 && self.landform > 0.0 {
            let base = s.elev_km * 1000.0 * self.vscale;
            let fade = local.mesa * (local.step_m / (2.0 * voxel_m) - 1.0).clamp(0.0, 1.0);
            if fade > 0.0 {
                let h = base + dh + strata_off;
                dh += fade * (terrace(h, local.step_m, 0.22) - h);
            }
        }
        // River valleys along the ridges of planetgen's discharge.
        let mut river = 0.0;
        if self.landform > 0.0 {
            let base = s.elev_km * 1000.0 * self.vscale;
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
        let k1 = 1000.0 * self.vscale / r;
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
        let k = 1000.0 * self.vscale / self.radius;
        let (gu, gv) = (s.elev_du_km * k, s.elev_dv_km * k);
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
            let lw = 2.0 * l.lam_d;
            let aw = 0.35 * l.lam_d;
            let (wx, wxx, wxy) = noise2d(x / lw + 5.2, y / lw - 1.3, seed ^ 0xA1);
            let (wy, wyx, wyy) = noise2d(x / lw - 7.7, y / lw + 3.1, seed ^ 0xA2);
            let (xw, yw) = (x + aw * wx, y + aw * wy);
            // Jacobian of the warp, d(xw, yw)/d(x, y).
            let (j00, j01, j10, j11) = (1.0 + aw / lw * wxx, aw / lw * wxy, aw / lw * wyx, 1.0 + aw / lw * wyy);
            let ld = l.lam_d.log2();
            let mut wl = MASSIF_TOP;
            for k in 0..MASSIF_OCTAVES {
                // Triangle in log-wavelength centred half an octave above lam_d.
                let wgt = 1.0 - (((MASSIF_TOP_LOG2 - k as f64) - ld - 0.5).abs() / 1.5);
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
                    // crest would spin their direction within metres.
                    let val = l.rugged * ridged + (1.0 - l.rugged) * n / NOISE_SD;
                    let dval = l.rugged * (-n / (n * n + STEER_EPS * STEER_EPS).sqrt() / RIDGE_SD) + (1.0 - l.rugged) / NOISE_SD;
                    let amp = 0.25 * l.relief * wgt;
                    gx += amp * dval * dnx;
                    gy += amp * dval * dny;
                    if !coarse_only {
                        h += amp * val * fade;
                    }
                }
                wl *= 0.5;
            }

            // --- Gullies: downhill stripes, each octave steered by the ones above.
            let mut c = GULLY_TOP;
            for k in 0..GULLY_OCTAVES {
                let fade = band(c, voxel_m);
                if fade <= 0.0 {
                    break;
                }
                // Octaves from just under the dominant wavelength down.
                let wgt = gully_weight(l, c);
                let s = (gx * gx + gy * gy).sqrt();
                if wgt > 0.0 && s > 1e-9 && adds(c) {
                    // Across the slope, unit length on real slopes but
                    // shrinking to zero where the slope vanishes: the stripe
                    // phase is measured up to 1.25 cells from a cell point,
                    // so a spinning direction at a hilltop or saddle would
                    // otherwise sweep it through whole cycles in metres.
                    let s0 = 0.5 * l.relief / l.lam_d + 1e-6;
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
                    let depth = wgt * gully_depth(l, c, s);
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
        let cut = 32.0 * voxel_m;
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
    /// the chart cross-fade can lift a sum by sqrt 2.
    pub fn sub_amp_bound_m(&self, l: &Local, wl_max: f64) -> f64 {
        let mut b = 0.0;
        if l.relief > 0.0 {
            let ld = l.lam_d.log2();
            let mut wl = MASSIF_TOP;
            for _ in 0..MASSIF_OCTAVES {
                let wgt = 1.0 - ((wl.log2() - ld - 0.5).abs() / 1.5);
                if wl < wl_max && wgt > 0.0 {
                    b += 0.25 * l.relief * wgt * MASSIF_MAX_ABS;
                }
                wl *= 0.5;
            }
            let mut c = GULLY_TOP;
            for _ in 0..GULLY_OCTAVES {
                if c < wl_max {
                    b += gully_weight(l, c) * gully_depth(l, c, GULLY_SLOPE_CAP);
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
        let l = Local { relief, lam_d: LAM_LO, rugged: 1.0, mesa: 1.0, step_m: (0.45 * relief / self.landform.max(1e-9)).clamp(3.0, 60.0), fac: 1.0, valley_m: 2.0 + 0.3 * relief + CHANNEL_DEPTH_M };
        // Every massif octave at full weight (conservative), all gullies.
        let massif = 0.25 * relief * MASSIF_MAX_ABS * 2.5;
        let gully: f64 = (0..GULLY_OCTAVES).map(|k| GULLY_BETA_LOW * relief + GULLY_ALPHA * GULLY_TOP * 0.5f64.powi(k as i32) * GULLY_SLOPE_CAP).sum();
        let fine = self.roughness * (FINE_AMP_M + FINE_SLOPE_AMP_M) * 1.75 / NOISE_SD * 0.5;
        (massif + gully + fine) * 2f64.sqrt() + l.step_m + l.valley_m
    }

    /// The relief scale (m) for given bake roughness / elevation extremes
    /// with the worst uplift, for bounds.
    pub fn relief_bound_m(&self, rough_km: f64, elev_km: f64) -> f64 {
        (C_ROUGH * rough_km.max(0.0) * 1000.0 * self.vscale + C_UPLIFT * elev_km.max(0.0) * 1000.0 * self.vscale + RELIEF_FLOOR_M) * 2.5 * self.landform
    }
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

/// Gully octaves run from just under the dominant wavelength down four
/// octaves.
#[inline]
fn gully_weight(l: &Local, c: f64) -> f64 {
    smoothstep(0.8 * l.lam_d, 0.4 * l.lam_d, c) * smoothstep(l.lam_d / 24.0, l.lam_d / 12.0, c)
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
        let lf = Landform { seed: 7, radius: 90_569.2, vscale: 0.125, landform: 1.0, roughness: 0.0 };
        let l = Local { relief: 60.0, lam_d: 900.0, rugged: 0.0, mesa: 0.0, step_m: 10.0, fac: 1.0, valley_m: 0.0 };
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

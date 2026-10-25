//! Terrain inspection: the places the landform evidence is shot at, plan
//! view hillshade maps of the generator's heights, and the numeric checks
//! (slope distribution, large-scale fidelity to the bake, aspect anisotropy
//! as a lattice-artifact detector, LOD consistency).
//!
//!   --terrain-map  --out DIR [--at LAT LON 0] [--map-size M] [--map-px N]
//!   --terrain-stats --out DIR
//!
//! Everything here reads the generator through `Generator::column`, so it
//! measures exactly what chunks are built from.

use super::Generator;
use super::bake::{Bake, chart_uv};
use super::noise::hash01;
use cubesphere::FACES;
use glam::DVec3;
use std::fmt::Write as _;

/// The places the landform renders and maps look at, found from the bake
/// alone so the same places come out whatever the generator does.
#[derive(Clone, Copy, Debug)]
pub struct LandPlaces {
    /// Highest land texel (the highest range).
    pub peak: DVec3,
    /// Steepest coastal texel (the M1a coast, `engine::find_places`).
    pub coast: DVec3,
    /// Wettest valley: most river discharge on temperate land with real
    /// relief and a concave (valley-shaped) bake surface.
    pub river: DVec3,
    /// Flat, temperate, green land away from rivers and coasts.
    pub plain: DVec3,
}

/// Unit direction of equal-angle chart point (u, v) on `face`.
#[inline]
pub fn chart_dir(face: u8, u: f64, v: f64) -> DVec3 {
    let (axis, right, up) = FACES[face as usize];
    (axis + u.tan() * right + v.tan() * up).normalize()
}

pub fn find_land_places(bake: &dyn Bake) -> LandPlaces {
    // Same definitions as engine::find_places for the peak and the coast.
    let mut peak = (DVec3::Z, f64::MIN);
    let mut coast = (DVec3::Z, f64::MIN);
    bake.for_each_texel(&mut |dir, elev, ocean, grad| {
        let e = elev as f64;
        if !ocean && e > peak.1 {
            peak = (dir, e);
        }
        if e.abs() < 0.3 && (grad as f64) > coast.1 {
            coast = (dir, grad as f64);
        }
    });
    // River valley and plain: scan the equal-angle texel centres of every face.
    let res = 1024usize;
    let step = std::f64::consts::FRAC_PI_2 / (res - 1) as f64;
    let mut river = (DVec3::Z, f64::MIN);
    let mut plain = (DVec3::Z, f64::MIN);
    for f in 0..6u8 {
        for row in (8..res - 8).step_by(2) {
            for col in (8..res - 8).step_by(2) {
                let (u, v) = (-std::f64::consts::FRAC_PI_4 + col as f64 * step, -std::f64::consts::FRAC_PI_4 + row as f64 * step);
                let s = bake.sample(chart_dir(f, u, v));
                if s.ocean > 0.01 || s.lake > 0.01 {
                    continue;
                }
                if s.elev_km > 0.4 && s.rough_km > 0.15 && s.temp_c > 0.0 && s.temp_c < 20.0 && s.flow_log10 > river.1 {
                    // Concave across 4 texels: the mean of the four neighbours sits above the centre.
                    let k = 4.0 * step;
                    let lap = [(k, 0.0), (-k, 0.0), (0.0, k), (0.0, -k)].iter().map(|(du, dv)| bake.sample(chart_dir(f, u + du, v + dv)).elev_km).sum::<f64>() - 4.0 * s.elev_km;
                    if lap > 0.05 {
                        river = (chart_dir(f, u, v), s.flow_log10);
                    }
                }
                if s.elev_km > 0.15 && s.elev_km < 0.6 && s.rough_km < 0.03 && s.temp_c > 8.0 && s.temp_c < 18.0 && s.precip_mm > 500.0 && s.precip_mm < 1500.0 && s.flow_log10 < 1.5 {
                    let score = -s.rough_km;
                    // No sea or lake within ~10 texels (~1.4 km here, ~100 km on Earth).
                    let k = 10.0 * step;
                    let dry = [(k, 0.0), (-k, 0.0), (0.0, k), (0.0, -k), (k, k), (-k, -k), (k, -k), (-k, k)].iter().all(|(du, dv)| {
                        let n = bake.sample(chart_dir(f, u + du, v + dv));
                        n.ocean < 0.01 && n.lake < 0.01
                    });
                    if score > plain.1 && dry {
                        plain = (chart_dir(f, u, v), score);
                    }
                }
            }
        }
    }
    LandPlaces { peak: peak.0, coast: coast.0, river: river.0, plain: plain.0 }
}

/// Bake-only ground height (m above sea, water = 0) at the generator's
/// vertical scale: camera placement that does not depend on the relief.
pub fn bake_ground_m(g: &Generator, dir: DVec3) -> f64 {
    (g.bake.sample(dir).elev_km * 1000.0 * g.vscale).max(0.0)
}

/// Heights of a px x px grid of columns in the equal-angle chart of the
/// face under `centre`, `spacing` metres apart, band-limited for voxels of
/// `voxel_m`. Returns (heights, ocean flags, bake heights).
pub fn sample_grid(g: &Generator, centre: DVec3, px: usize, spacing: f64, voxel_m: f64) -> (Vec<f32>, Vec<bool>, Vec<f32>) {
    let face = g.bake.sample(centre).face;
    let (u0, v0) = chart_uv(face, centre);
    let r = g.planet.radius;
    let half = (px as f64 - 1.0) * 0.5;
    let rows: Vec<(Vec<f32>, Vec<bool>, Vec<f32>)> = {
        use rayon::prelude::*;
        (0..px)
            .into_par_iter()
            .map(|y| {
                let mut hs = Vec::with_capacity(px);
                let mut oc = Vec::with_capacity(px);
                let mut bk = Vec::with_capacity(px);
                for x in 0..px {
                    // Row 0 is the top of the image (+v).
                    let u = u0 + (x as f64 - half) * spacing / r;
                    let v = v0 + (half - y as f64) * spacing / r;
                    let dir = chart_dir(face, u, v);
                    let c = g.column(dir, voxel_m);
                    hs.push(c.height_m);
                    oc.push(c.ocean);
                    bk.push((g.bake.sample(dir).elev_km * 1000.0 * g.vscale) as f32);
                }
                (hs, oc, bk)
            })
            .collect()
    };
    let mut h = Vec::with_capacity(px * px);
    let mut o = Vec::with_capacity(px * px);
    let mut b = Vec::with_capacity(px * px);
    for (hs, oc, bk) in rows {
        h.extend(hs);
        o.extend(oc);
        b.extend(bk);
    }
    (h, o, b)
}

/// Plan-view hillshade (sun from the north-west at 45 degrees, true slopes)
/// tinted by height within the map, sea in blue.
pub fn write_hillshade(path: &str, h: &[f32], ocean: &[bool], px: usize, spacing: f64, caption_px_rows: usize) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (i, &v) in h.iter().enumerate() {
        if !ocean[i] {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if lo > hi {
        lo = -1.0;
        hi = 1.0;
    }
    // Sun from the north-west, 35 degrees up; shading stretched around the
    // flat-ground value so gentle (true-scale) slopes still read.
    let light = DVec3::new(-1.0, 1.0, 1.0).normalize();
    let flat = light.z;
    let mut rgba = vec![0u8; px * (px + caption_px_rows) * 4];
    for y in 0..px {
        for x in 0..px {
            let at = |xx: usize, yy: usize| h[yy.min(px - 1) * px + xx.min(px - 1)] as f64;
            let dx = (at(x + 1, y) - at(x.saturating_sub(1), y)) / (spacing * if x == 0 || x == px - 1 { 1.0 } else { 2.0 });
            let dy = (at(x, y.saturating_sub(1)) - at(x, y + 1)) / (spacing * if y == 0 || y == px - 1 { 1.0 } else { 2.0 });
            let n = DVec3::new(-dx, -dy, 1.0).normalize();
            let shade = (0.62 + 2.2 * (n.dot(light) - flat)).clamp(0.0, 1.25);
            let i = y * px + x;
            let t = ((h[i] - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0) as f64;
            // Low green-olive to high tan-grey.
            let base = if ocean[i] && h[i] < 0.0 {
                [0.30, 0.45, 0.70]
            } else {
                [0.42 + 0.40 * t, 0.55 + 0.25 * t, 0.36 + 0.42 * t]
            };
            let o = i * 4;
            for c in 0..3 {
                rgba[o + c] = ((base[c] * (0.15 + 0.85 * shade)).clamp(0.0, 1.0).powf(1.0 / 1.6) * 255.0) as u8;
            }
            rgba[o + 3] = 255;
        }
    }
    for p in rgba[px * px * 4..].chunks_exact_mut(4) {
        p.copy_from_slice(&[20, 20, 20, 255]);
    }
    write_png(path, px as u32, (px + caption_px_rows) as u32, &rgba);
}

pub fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(rgba).expect("png data");
}

/// Plan-view maps of the landform places at two zooms, written as PNGs
/// (`map_<place>_<size>.png`), plus a line per map with its height range.
pub fn terrain_maps(g: &Generator, places: &[(&str, DVec3)], out: &str, px: usize) -> String {
    let mut log = String::new();
    for &(name, dir) in places {
        for &(size, label) in &[(2_000.0, "2km"), (12_000.0, "12km")] {
            let spacing = size / px as f64;
            let (h, o, b) = sample_grid(g, dir, px, spacing, spacing.max(g.planet.block_size));
            let path = format!("{out}/map_{name}_{label}.png");
            write_hillshade(&path, &h, &o, px, spacing, 0);
            let (lo, hi) = h.iter().fold((f32::MAX, f32::MIN), |(a, z), &v| (a.min(v), z.max(v)));
            let (blo, bhi) = b.iter().fold((f32::MAX, f32::MIN), |(a, z), &v| (a.min(v), z.max(v)));
            let (lat, lon) = crate::grid::Planet::lat_lon_deg(dir);
            let line = format!("{name} {label}: lat {lat:.4} lon {lon:.4}, {px}x{px} at {spacing:.1} m: height {lo:.1}..{hi:.1} m (bake {blo:.1}..{bhi:.1} m) -> {path}\n");
            print!("{line}");
            log.push_str(&line);
        }
    }
    log
}

fn pctl(v: &mut [f64], ps: &[f64]) -> Vec<f64> {
    v.sort_by(|a, b| a.total_cmp(b));
    ps.iter().map(|p| if v.is_empty() { f64::NAN } else { v[(((v.len() - 1) as f64) * p).round() as usize] }).collect()
}

/// Random land directions (bake ocean fraction < 0.5), deterministic.
fn land_dirs(g: &Generator, n: usize, salt: u64) -> Vec<DVec3> {
    let mut out = Vec::new();
    let mut k = 0i64;
    while out.len() < n {
        let z = hash01(k, 1, 7, salt) * 2.0 - 1.0;
        let a = hash01(k, 2, 7, salt) * std::f64::consts::TAU;
        let s = (1.0 - z * z).sqrt();
        let d = DVec3::new(s * a.cos(), s * a.sin(), z);
        if g.bake.sample(d).ocean < 0.5 {
            out.push(d);
        }
        k += 1;
    }
    out
}

/// Slope statistics over patches of columns around the given places and
/// over random land: |grad h| (rise over run) at 1 m and 16 m spacing.
pub fn slope_report(g: &Generator, places: &[(&str, DVec3)]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "| where | spacing | slope p10 | p50 | p90 | p99 | max | frac < 0.02 | frac > 0.7 | frac > 1.0 |");
    let _ = writeln!(s, "|---|---|---|---|---|---|---|---|---|---|");
    let mut all: Vec<(String, DVec3)> = places.iter().map(|(n, d)| (n.to_string(), *d)).collect();
    for (i, d) in land_dirs(g, 60, 0x51_0E).into_iter().enumerate() {
        all.push((format!("land{i:02}"), d));
    }
    let mut pooled: [Vec<f64>; 2] = [Vec::new(), Vec::new()];
    for (name, dir) in &all {
        for (si, &spacing) in [1.0f64, 16.0].iter().enumerate() {
            let px = 96;
            let (h, o, _) = sample_grid(g, *dir, px, spacing, spacing.max(g.planet.block_size).min(spacing));
            let mut sl = Vec::new();
            for y in 1..px - 1 {
                for x in 1..px - 1 {
                    if o[y * px + x] {
                        continue;
                    }
                    let dx = (h[y * px + x + 1] - h[y * px + x - 1]) as f64 / (2.0 * spacing);
                    let dy = (h[(y + 1) * px + x] - h[(y - 1) * px + x]) as f64 / (2.0 * spacing);
                    sl.push((dx * dx + dy * dy).sqrt());
                }
            }
            if sl.is_empty() {
                continue;
            }
            if name.starts_with("land") {
                pooled[si].extend(sl.iter().copied());
            } else {
                let n = sl.len() as f64;
                let f = |t: f64, above: bool| sl.iter().filter(|&&v| if above { v > t } else { v < t }).count() as f64 / n;
                let (a, b, c) = (f(0.02, false), f(0.7, true), f(1.0, true));
                let p = pctl(&mut sl, &[0.1, 0.5, 0.9, 0.99, 1.0]);
                let _ = writeln!(s, "| {name} | {spacing} m | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {a:.2} | {b:.3} | {c:.3} |", p[0], p[1], p[2], p[3], p[4]);
            }
        }
    }
    for (si, spacing) in [1.0, 16.0].iter().enumerate() {
        let sl = &mut pooled[si];
        let n = sl.len() as f64;
        let f = |sl: &[f64], t: f64, above: bool| sl.iter().filter(|&&v| if above { v > t } else { v < t }).count() as f64 / n;
        let (a, b, c) = (f(sl, 0.02, false), f(sl, 0.7, true), f(sl, 1.0, true));
        let p = pctl(sl, &[0.1, 0.5, 0.9, 0.99, 1.0]);
        let _ = writeln!(s, "| 60 random land patches | {spacing} m | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {a:.2} | {b:.3} | {c:.3} |", p[0], p[1], p[2], p[3], p[4]);
    }
    s
}

/// Large-scale fidelity: over random land windows of 10 and 20 km, the mean
/// of (surface - bake surface), sampled on a 48 x 48 grid of full-detail
/// columns, against the spread of the bake inside the window. Also the same
/// window means for a coarse LOD (64 m voxels) against full detail.
pub fn fidelity_report(g: &Generator) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "| window | windows | mean(h - bake) RMS m | p95 abs m | max abs m | bake std in window, median m | RMS / bake std | detail std, median m | LOD6 vs LOD0 window mean, max abs m |");
    let _ = writeln!(s, "|---|---|---|---|---|---|---|---|---|");
    for &w in &[10_000.0, 20_000.0] {
        let dirs = land_dirs(g, 120, 0xF1DE ^ w as u64);
        let rows: Vec<(f64, f64, f64, f64)> = {
            use rayon::prelude::*;
            dirs.par_iter()
                .map(|&d| {
                    let px = 48;
                    let spacing = w / px as f64;
                    let (h, _, b) = sample_grid(g, d, px, spacing, g.planet.block_size);
                    let (h6, _, _) = sample_grid(g, d, px, spacing, 64.0 * g.planet.block_size);
                    let n = h.len() as f64;
                    let diff: f64 = h.iter().zip(&b).map(|(a, c)| (*a - *c) as f64).sum::<f64>() / n;
                    let bm = b.iter().map(|&v| v as f64).sum::<f64>() / n;
                    let bstd = (b.iter().map(|&v| (v as f64 - bm).powi(2)).sum::<f64>() / n).sqrt();
                    let dm = diff;
                    let dstd = (h.iter().zip(&b).map(|(a, c)| ((*a - *c) as f64 - dm).powi(2)).sum::<f64>() / n).sqrt();
                    let lod = (h6.iter().sum::<f32>() as f64 - h.iter().sum::<f32>() as f64) / n;
                    (diff, bstd, dstd, lod)
                })
                .collect()
        };
        let n = rows.len() as f64;
        let rms = (rows.iter().map(|r| r.0 * r.0).sum::<f64>() / n).sqrt();
        let mut abs: Vec<f64> = rows.iter().map(|r| r.0.abs()).collect();
        let p = pctl(&mut abs, &[0.95, 1.0]);
        let mut bstd: Vec<f64> = rows.iter().map(|r| r.1).collect();
        let bmed = pctl(&mut bstd, &[0.5])[0];
        let mut dstd: Vec<f64> = rows.iter().map(|r| r.2).collect();
        let dmed = pctl(&mut dstd, &[0.5])[0];
        let lmax = rows.iter().map(|r| r.3.abs()).fold(0.0, f64::max);
        let _ = writeln!(s, "| {} km | {} | {rms:.2} | {:.2} | {:.2} | {bmed:.1} | {:.3} | {dmed:.2} | {lmax:.2} |", w / 1000.0, rows.len(), p[0], p[1], rms / bmed.max(1e-6), );
    }
    s
}

/// Lattice-artifact detector: the direction of the engine's detail
/// gradient (surface minus bake) in the chart frame, histogrammed modulo
/// 90 degrees in 5 degree bins, weighted by gradient magnitude, pooled over
/// random land maps. Noise built on an axis-aligned lattice piles up at 0
/// and 45 degrees; isotropic detail is flat (max/min near 1).
pub fn anisotropy_report(g: &Generator) -> String {
    let mut bins = [0.0f64; 18];
    for d in land_dirs(g, 40, 0xA5_1507) {
        for &spacing in &[2.0, 16.0] {
            let px = 128;
            let (h, o, b) = sample_grid(g, d, px, spacing, spacing.max(g.planet.block_size).min(spacing));
            for y in 1..px - 1 {
                for x in 1..px - 1 {
                    if o[y * px + x] {
                        continue;
                    }
                    let e = |xx: usize, yy: usize| (h[yy * px + xx] - b[yy * px + xx]) as f64;
                    let dx = e(x + 1, y) - e(x - 1, y);
                    let dy = e(x, y - 1) - e(x, y + 1);
                    let m = (dx * dx + dy * dy).sqrt();
                    if m < 1e-9 {
                        continue;
                    }
                    let a = dy.atan2(dx).to_degrees().rem_euclid(90.0);
                    bins[((a / 5.0) as usize).min(17)] += m;
                }
            }
        }
    }
    let mean = bins.iter().sum::<f64>() / 18.0;
    let (mn, mx) = bins.iter().fold((f64::MAX, f64::MIN), |(a, z), &v| (a.min(v), z.max(v)));
    let mut s = String::new();
    let _ = writeln!(s, "aspect of the detail gradient mod 90 deg (5 deg bins, / mean): {}", bins.iter().map(|v| format!("{:.2}", v / mean)).collect::<Vec<_>>().join(" "));
    let _ = writeln!(s, "max/min bin ratio {:.3}; 0 deg bin {:.3}, 45 deg bin {:.3}", mx / mn.max(1e-12), bins[0] / mean, bins[9] / mean);
    s
}

/// LOD consistency: the column height at a coarse voxel's centre against the
/// mean of the full-detail columns the voxel covers, over random land.
pub fn lod_report(g: &Generator) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "| LOD | voxel m | mean abs(coarse - mean of fine) m | in voxels | p99 in voxels | max in voxels |");
    let _ = writeln!(s, "|---|---|---|---|---|---|");
    let dirs = land_dirs(g, 300, 0x10D);
    let planet = g.planet;
    for lod in 1..=7u8 {
        let vm = planet.voxel_m(lod);
        let mut errs: Vec<f64> = {
            use rayon::prelude::*;
            dirs.par_iter()
                .map(|&d| {
                    let (f, i, j) = planet.lattice_of_dir(d);
                    let vb = (1u64 << lod) as f64;
                    let (ci, cj) = ((i / vb).floor() * vb, (j / vb).floor() * vb);
                    let coarse = g.column(planet.dir(f, ci + 0.5 * vb, cj + 0.5 * vb), vm).height_m as f64;
                    let n = (1u64 << lod).min(16) as usize;
                    let step = vb / n as f64;
                    let mut sum = 0.0;
                    for a in 0..n {
                        for b in 0..n {
                            sum += g.column(planet.dir(f, ci + (a as f64 + 0.5) * step, cj + (b as f64 + 0.5) * step), planet.block_size).height_m as f64;
                        }
                    }
                    (coarse - sum / (n * n) as f64).abs()
                })
                .collect()
        };
        let mean = errs.iter().sum::<f64>() / errs.len() as f64;
        let p = pctl(&mut errs, &[0.99, 1.0]);
        let _ = writeln!(s, "| {lod} | {vm} | {mean:.3} | {:.3} | {:.3} | {:.3} |", mean / vm, p[0] / vm, p[1] / vm);
    }
    s
}

//! T2: local relief (mountains in metres on the planet's belts) and the rule
//! that land falls towards water. Surveys (ignored) and the gates.

use super::hydro_tests::{hydro, shore_point, world_gen};
use super::tests::dirs_pub;
use super::*;
use rayon::prelude::*;

fn pctl(v: &mut [f64], ps: &[f64]) -> Vec<f64> {
    v.sort_by(|a, b| a.total_cmp(b));
    ps.iter().map(|p| if v.is_empty() { f64::NAN } else { v[(((v.len() - 1) as f64) * p).round() as usize] }).collect()
}

fn fmt_q(name: &str, v: &mut [f64]) -> String {
    let q = pctl(v, &[0.0, 0.1, 0.5, 0.9, 0.99, 1.0]);
    format!("{name:12} min {:9.3} p10 {:9.3} p50 {:9.3} p90 {:9.3} p99 {:9.3} max {:9.3}", q[0], q[1], q[2], q[3], q[4], q[5])
}

/// Not a check: quantiles of the bake's fields over land, of the landform's
/// relief and displacement, and of the surface height, so the local relief
/// model can be set from the real distributions. Run with --ignored --nocapture.
#[test]
#[ignore]
fn relief_survey() {
    let g = world_gen();
    let land: Vec<DVec3> = dirs_pub(60_000, 0x7E11).into_iter().filter(|d| g.bake.sample(*d).ocean < 0.5).collect();
    println!("{} land samples", land.len());
    let rows: Vec<(f64, f64, f64, f64, f64, f64, f64, f64, f64)> = land
        .par_iter()
        .map(|&d| {
            let s = g.bake.sample(d);
            let l = g.lf.local(&s);
            let lf = g.lf.sample(d, &s, 1.0, 0.0);
            let c = g.column(d, 1.0);
            (s.uplift, s.rough_km, s.elev_km, s.erodibility, s.soil_km, l.relief, lf.dh, c.height_m as f64, s.temp_c)
        })
        .collect();
    let col = |f: &dyn Fn(&(f64, f64, f64, f64, f64, f64, f64, f64, f64)) -> f64| rows.iter().map(f).collect::<Vec<f64>>();
    println!("{}", fmt_q("uplift", &mut col(&|r| r.0)));
    println!("{}", fmt_q("rough_km", &mut col(&|r| r.1)));
    println!("{}", fmt_q("elev_km", &mut col(&|r| r.2)));
    println!("{}", fmt_q("erodibility", &mut col(&|r| r.3)));
    println!("{}", fmt_q("soil_km", &mut col(&|r| r.4)));
    println!("{}", fmt_q("relief", &mut col(&|r| r.5)));
    println!("{}", fmt_q("dh", &mut col(&|r| r.6)));
    println!("{}", fmt_q("height_m", &mut col(&|r| r.7)));
    println!("{}", fmt_q("temp_c", &mut col(&|r| r.8)));
    // Joint: where is uplift high, and what is rough_km there?
    for (lo, hi) in [(0.0, 0.1), (0.1, 0.3), (0.3, 0.6), (0.6, 2.0)] {
        let mut r: Vec<f64> = rows.iter().filter(|r| r.0 >= lo && r.0 < hi).map(|r| r.1).collect();
        let mut e: Vec<f64> = rows.iter().filter(|r| r.0 >= lo && r.0 < hi).map(|r| r.2).collect();
        let n = r.len();
        if n > 0 {
            println!("uplift [{lo}, {hi}): {n} ({:.1}%)  {}", 100.0 * n as f64 / rows.len() as f64, fmt_q("rough_km", &mut r));
            println!("{:>40}{}", "", fmt_q("elev_km", &mut e));
        }
    }
    for (lo, hi) in [(0.0, 0.1), (0.1, 0.3), (0.3, 0.6), (0.6, 1.0), (1.0, 9.0)] {
        let mut e: Vec<f64> = rows.iter().filter(|r| r.1 >= lo && r.1 < hi).map(|r| r.2).collect();
        let mut u: Vec<f64> = rows.iter().filter(|r| r.1 >= lo && r.1 < hi).map(|r| r.0).collect();
        let n = e.len();
        if n > 0 {
            println!("rough_km [{lo}, {hi}): {n} ({:.1}%)  {}", 100.0 * n as f64 / rows.len() as f64, fmt_q("elev_km", &mut e));
            println!("{:>40}{}", "", fmt_q("uplift", &mut u));
        }
    }
    // The highest columns found, and where.
    let mut by_h: Vec<(f64, DVec3)> = land.iter().zip(&rows).map(|(d, r)| (r.7, *d)).collect();
    by_h.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (h, d) in by_h.iter().take(5) {
        let (lat, lon) = Planet::lat_lon_deg(*d);
        let s = g.bake.sample(*d);
        println!("high: {h:.1} m at lat {lat:.3} lon {lon:.3} (elev {:.2} km, rough {:.2}, uplift {:.2})", s.elev_km, s.rough_km, s.uplift);
    }
}

/// The local/global split (#2a): on a planet twice the size (N 262,144,
/// R 181 km, vertical scale 1/35), the global surface is exactly twice as
/// tall, the belts are in the same places on the sphere (the same belt
/// weight at the same direction), the local relief is the same in metres,
/// the valleys the same width, and the peaks the same height (the same
/// quantiles of the relief over a belt, to a tenth).
#[test]
fn local_features_keep_their_metres() {
    let small = world_gen();
    let big = Generator::new(Planet::new(1.0, 2048, 1024, 262_144), small.bake.clone(), 42, Planet::new(1.0, 2048, 1024, 262_144).radius / 6_371_000.0, 1.0, 1.0, super::hydro_tests::registry()).unwrap();
    assert!((big.planet.radius / small.planet.radius - 2.0).abs() < 1e-9);
    let ratio = big.vscale / small.vscale;
    assert!((ratio - 2.0).abs() < 1e-9, "the vertical scale follows the radius: {ratio}");
    let land: Vec<DVec3> = dirs_pub(6000, 0x5CA1E).into_iter().filter(|d| small.bake.sample(*d).ocean < 0.5).collect();
    let mut belt: Vec<DVec3> = Vec::new();
    for &d in &land {
        let s = small.bake.sample(d);
        let (a, b) = (small.lf.local(&s), big.lf.local(&s));
        // Global: twice the height. Local: the same metres, the same belt.
        assert!((big.scale.global_m(s.elev_km) - 2.0 * small.scale.global_m(s.elev_km)).abs() < 1e-9);
        assert_eq!(a.mountain, b.mountain, "the belt weight is a place on the sphere");
        assert_eq!(a.relief, b.relief, "the relief is metres");
        assert_eq!(a.lam_d, b.lam_d, "the ridge spacing is metres");
        assert_eq!(hydro::valley_half_width(a.relief, a.mountain), hydro::valley_half_width(b.relief, b.mountain));
        if a.mountain > 0.8 {
            belt.push(d);
        }
    }
    assert!(belt.len() > 100, "too few belt samples ({})", belt.len());
    // The peaks: quantiles of the relief standing on the global surface over
    // the belt, on both planets. The noise lies differently on each (it is
    // laid out in metres on a bigger sphere), so the statistics agree, not
    // the columns.
    let dh = |g: &Generator| -> Vec<f64> {
        belt.par_iter()
            .map(|&d| {
                let s = g.bake.sample(d);
                g.lf.sample(d, &s, 1.0, 0.0).dh
            })
            .collect()
    };
    let (mut ds, mut db) = (dh(small), dh(&big));
    let qs = pctl(&mut ds, &[0.5, 0.9, 0.99]);
    let qb = pctl(&mut db, &[0.5, 0.9, 0.99]);
    println!("relief over {} belt samples: small p50 {:.0} p90 {:.0} p99 {:.0} m; big p50 {:.0} p90 {:.0} p99 {:.0} m", belt.len(), qs[0], qs[1], qs[2], qb[0], qb[1], qb[2]);
    for (a, b) in qs.iter().zip(&qb) {
        assert!((a - b).abs() < 0.1 * a.max(*b) + 5.0, "relief quantile {a:.0} m against {b:.0} m on the bigger planet");
    }
    // And the lakes and rivers: the same channel widths in metres (local),
    // levels twice as high where the ground is (global).
    let (hs, hb) = (small.hydro.as_ref().unwrap(), big.hydro.as_ref().unwrap());
    assert_eq!(hs.segs[100].hw, hb.segs[100].hw, "channel widths are metres");
    let (ls, lb) = (hs.lakes[by_level(hs)].w, hb.lakes[by_level(hs)].w);
    assert!(lb > 1.5 * ls && lb < 2.5 * ls, "the highest lake stands about twice as high on the bigger planet: {ls:.1} m, {lb:.1} m");
}

/// Index of the highest lake.
fn by_level(h: &hydro::Hydro) -> usize {
    (0..h.lakes.len()).max_by(|a, b| h.lakes[*a].w.total_cmp(&h.lakes[*b].w)).unwrap()
}

/// A plain hillshade (Lambert, sun from the north-west 40 degrees up, no
/// stretch) tinted by height, with contour lines every `contour_m`, so
/// mountains can be judged by eye.
fn write_relief_map(path: &str, h: &[f32], ocean: &[bool], px: usize, spacing: f64, contour_m: f32) {
    let light = DVec3::new(-1.0, 1.0, 0.84).normalize();
    let (lo, hi) = h.iter().zip(ocean).filter(|(_, o)| !**o).fold((f32::MAX, f32::MIN), |(a, z), (v, _)| (a.min(*v), z.max(*v)));
    let mut rgba = vec![0u8; px * px * 4];
    for y in 0..px {
        for x in 0..px {
            let at = |xx: usize, yy: usize| h[yy.min(px - 1) * px + xx.min(px - 1)] as f64;
            let dx = (at(x + 1, y) - at(x.saturating_sub(1), y)) / (spacing * if x == 0 || x == px - 1 { 1.0 } else { 2.0 });
            let dy = (at(x, y.saturating_sub(1)) - at(x, y + 1)) / (spacing * if y == 0 || y == px - 1 { 1.0 } else { 2.0 });
            let n = DVec3::new(-dx, -dy, 1.0).normalize();
            let shade = (0.15 + 0.85 * n.dot(light).max(0.0)) as f32;
            let i = y * px + x;
            let t = ((h[i] - lo) / (hi - lo).max(1e-3)).clamp(0.0, 1.0);
            let mut base = if ocean[i] && h[i] < 0.0 { [0.30, 0.45, 0.70] } else { [0.45 + 0.45 * t, 0.58 + 0.22 * t, 0.36 + 0.5 * t] };
            // A contour where the band index changes against a neighbour.
            let band = |v: f32| (v / contour_m).floor();
            let edge = band(h[i]) != band(at(x + 1, y) as f32) || band(h[i]) != band(at(x, y + 1) as f32);
            if edge && !ocean[i] {
                let strong = (band(h[i]) as i32).rem_euclid(5) == 0 || (band(at(x + 1, y) as f32) as i32).rem_euclid(5) == 0;
                let k = if strong { 0.35 } else { 0.65 };
                base = [base[0] * k, base[1] * k, base[2] * k];
            }
            let o = i * 4;
            for c in 0..3 {
                rgba[o + c] = ((base[c] * shade).clamp(0.0, 1.0).powf(1.0 / 1.8) * 255.0) as u8;
            }
            rgba[o + 3] = 255;
        }
    }
    inspect::write_png(path, px as u32, px as u32, &rgba);
}

/// Not a check: relief maps for the eye at the highest belt, the river
/// place, the plain and a lowland hills site, at 12 km, 4 km and 1.5 km.
/// Run with --ignored --nocapture; writes out/relief-maps/<label>/.
#[test]
#[ignore]
fn relief_maps() {
    let g: &Generator = match std::env::var("RELIEF_SIZE").ok().and_then(|s| s.parse::<u32>().ok()) {
        Some(units) => Box::leak(Box::new(super::tests::planet_size_generator(units))),
        None => world_gen(),
    };
    let label = std::env::var("RELIEF_LABEL").unwrap_or_else(|_| "now".into());
    let dir = format!("{}/out/relief-maps/{label}", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let places = inspect::find_land_places(g.bake.as_ref());
    // The strongest belt with real relief: the highest column of the survey.
    let belt = Planet::dir_of_lat_lon_deg(65.912, 134.841);
    let rough = Planet::dir_of_lat_lon_deg(39.907, -86.310);
    let sites: Vec<(&str, DVec3)> = vec![("belt", belt), ("rough", rough), ("peak", places.peak), ("river", places.river), ("plain", places.plain)];
    let px = 512;
    for (name, d) in sites {
        for (size, contour) in [(12_000.0, 100.0), (4_000.0, 50.0), (1_500.0, 20.0)] {
            let spacing = size / px as f64;
            let (h, _, b) = inspect::sample_grid(g, d, px, spacing, spacing.max(1.0));
            // Water (sea, lakes, rivers) in blue: every wet column, so the
            // valleys can be judged against the rivers in them.
            let face = g.bake.sample(d).face;
            let (u0, v0) = bake::chart_uv(face, d);
            let half = (px as f64 - 1.0) * 0.5;
            let o: Vec<bool> = (0..px * px)
                .into_par_iter()
                .map(|i| {
                    let (x, y) = (i % px, i / px);
                    let dir = inspect::chart_dir(face, u0 + (x as f64 - half) * spacing / g.planet.radius, v0 + (half - y as f64) * spacing / g.planet.radius);
                    g.column(dir, spacing.max(1.0)).wet()
                })
                .collect();
            let h: Vec<f32> = h.iter().zip(&o).map(|(v, w)| if *w { -1.0 } else { *v }).collect();
            let path = format!("{dir}/{name}_{}m.png", size as i64);
            write_relief_map(&path, &h, &o, px, spacing, contour);
            let (lo, hi) = h.iter().fold((f32::MAX, f32::MIN), |(a, z), &v| (a.min(v), z.max(v)));
            let (blo, bhi) = b.iter().fold((f32::MAX, f32::MIN), |(a, z), &v| (a.min(v), z.max(v)));
            let (lat, lon) = Planet::lat_lon_deg(d);
            println!("{name} {size:.0} m (lat {lat:.3} lon {lon:.3}): height {lo:.0}..{hi:.0} m, bake {blo:.0}..{bhi:.0} m -> {path}");
        }
    }
}

/// Not a check: find the evidence poses for Andrew (standing in a valley
/// looking up at a peak, a range from 2 km, a river gorge from the ground, a
/// lowland river from its bank, a lake shore from the ground, a mountain
/// belt from orbit) and write one `.play` per site to out/relief-shots/,
/// with sites.txt (site, lat, lon) so each run can start there in daylight.
#[test]
#[ignore]
fn relief_places() {
    // RELIEF_SIZE=U poses on a planet of U half-heights (T1's slider; the
    // shots then need `--planet-size U`).
    let g: &Generator = match std::env::var("RELIEF_SIZE").ok().and_then(|s| s.parse::<u32>().ok()) {
        Some(units) => Box::leak(Box::new(super::tests::planet_size_generator(units))),
        None => world_gen(),
    };
    let h: &hydro::Hydro = g.hydro.as_deref().unwrap();
    let p = &g.planet;
    let r = p.radius;
    let step = |at: DVec3, v: DVec3, d: f64| (at * r + v * d).normalize();
    let ground = |d: DVec3| g.ground_height_m(d);
    let dry_ground = |d: DVec3| g.column(d, p.block_size).height_m as f64;
    // Look from `eye` (alt m above sea) at `target` (alt m above sea).
    let look_at = |eye: DVec3, eye_alt: f64, target: DVec3, target_alt: f64| {
        let pe = eye * (r + eye_alt);
        let pt = target * (r + target_alt);
        let v = pt - pe;
        let (north, east, up) = crate::camera::Camera::local_frame(eye);
        let horiz = (v.dot(north).powi(2) + v.dot(east).powi(2)).sqrt();
        (v.dot(east).atan2(v.dot(north)).to_degrees(), v.dot(up).atan2(horiz).to_degrees())
    };
    let mut scripts: Vec<(String, String, DVec3)> = Vec::new();
    let mut add = |site: &str, eye: DVec3, alt: f64, head: f64, pitch: f64, note: &str| {
        let (lat, lon) = Planet::lat_lon_deg(eye);
        let text = format!("# T2 evidence: {note}\n# written by `cargo test --release relief_places -- --ignored`\nmode fly\nat {lat:.6} {lon:.6} {alt:.2}\nlook {head:.2} {pitch:.1}\nphoto {site}\n");
        println!("{site}: lat {lat:.5} lon {lon:.5} alt {alt:.1} heading {head:.1} pitch {pitch:.1} ({note})");
        scripts.push((site.to_string(), text, eye));
    };
    // 1-2. The belt: the highest column in a 10 km window, and the lowest
    // within 1.5 km of it (a valley floor).
    let belt = Planet::dir_of_lat_lon_deg(65.912, 134.841);
    let face = g.bake.sample(belt).face;
    let (u0, v0) = bake::chart_uv(face, belt);
    let mut peak = (belt, f64::MIN);
    for a in -125..=125 {
        for b in -125..=125 {
            let d = inspect::chart_dir(face, u0 + a as f64 * 40.0 / r, v0 + b as f64 * 40.0 / r);
            let hh = dry_ground(d);
            if hh > peak.1 {
                peak = (d, hh);
            }
        }
    }
    let (pu, pv) = bake::chart_uv(face, peak.0);
    let mut valley = (peak.0, f64::MAX);
    for a in -75..=75 {
        for b in -75..=75 {
            let d = inspect::chart_dir(face, pu + a as f64 * 20.0 / r, pv + b as f64 * 20.0 / r);
            let dist = ((a * a + b * b) as f64).sqrt() * 20.0;
            if dist < 300.0 || dist > 1500.0 {
                continue;
            }
            let c = g.column(d, p.block_size);
            if !c.wet() && (c.height_m as f64) < valley.1 {
                valley = (d, c.height_m as f64);
            }
        }
    }
    let (hd, pt) = look_at(valley.0, valley.1 + 1.7, peak.0, peak.1);
    add("valley_peak", valley.0, valley.1 + 1.7, hd, pt, &format!("standing on a valley floor at {:.0} m looking up at a {:.0} m peak {:.0} m away", valley.1, peak.1, (valley.0 - peak.0).length() * r));
    let away = {
        let v = valley.0 - peak.0;
        (v - peak.0 * v.dot(peak.0)).normalize()
    };
    let eye = step(peak.0, away, 2000.0);
    let alt = ground(eye).max(valley.1) + 120.0;
    let (hd, pt) = look_at(eye, alt, peak.0, peak.1);
    add("range_2km", eye, alt, hd, pt, &format!("the range from 2 km, {:.0} m above the ground", alt - ground(eye)));
    // 6. The belt from orbit: 30 km up, 15 km back, looking down across it.
    let orbit = step(belt, -away, 15_000.0);
    let (hd, pt) = look_at(orbit, 30_000.0, belt, 400.0);
    add("belt_orbit", orbit, 30_000.0, hd, pt, "the mountain belt from 30 km up");
    // 3. A river gorge: the mountain link whose channel lies deepest below
    // the ground 30 m beside it, mid-link.
    let ns = h.segs.len() as u32;
    let tangent = |k: u32, s: f64| {
        let seg = &h.segs[k as usize];
        let a = seg.point((s - 3.0).max(0.0));
        let b = seg.point((s + 3.0).min(seg.len));
        let t = b - a;
        let at = seg.point(s);
        (t - at * t.dot(at)).normalize()
    };
    let mut gorge = (0u32, 0.0f64, f64::MIN);
    for k in 0..ns {
        let s = &h.segs[k as usize];
        if s.valley_w < 700.0 || s.lake_a >= 0 || s.lake_b >= 0 {
            continue;
        }
        for t in [0.3, 0.5, 0.7] {
            let x = t * s.len;
            let at = s.point(x);
            let side = tangent(k, x).cross(at).normalize();
            let c = g.column(at, p.block_size);
            if !c.wet() {
                continue;
            }
            let rim = dry_ground(step(at, side, 30.0)).min(dry_ground(step(at, -side, 30.0)));
            let depth = rim - c.water_m;
            if depth > gorge.2 {
                gorge = (k, x, depth);
            }
        }
    }
    let (k, x, depth) = gorge;
    let s = &h.segs[k as usize];
    let at = s.point(x);
    let v = tangent(k, x);
    let side = v.cross(at).normalize();
    // Stand on the bank 50 m downstream, looking upstream along the gorge.
    let mut eye = step(step(at, v, 50.0), side, s.hw + 2.0);
    for t in 0..10 {
        if !g.column(eye, p.block_size).wet() {
            break;
        }
        eye = step(step(at, v, 50.0), side, s.hw + 2.0 + 2.0 * (t + 1) as f64);
    }
    let (hd, pt) = look_at(eye, ground(eye) + 1.7, at, g.column(at, p.block_size).water_m + 3.0);
    add("gorge_ground", eye, ground(eye) + 1.7, hd, pt.max(2.0), &format!("link {k}: a river {depth:.0} m below the ground 30 m aside, from its bank 50 m downstream, looking upstream"));
    let rim = step(step(at, v, 30.0), side, 90.0);
    let rim_alt = ground(rim).max(ground(at) + 10.0) + 12.0;
    let (hd, pt) = look_at(rim, rim_alt, step(at, v, -60.0), g.column(at, p.block_size).water_m);
    add("gorge_rim", rim, rim_alt, hd, pt, "the same gorge from above its rim, 90 m aside, looking up the river");
    // 4. A lowland river from its bank: the largest river away from lakes
    // and the sea with a smooth profile, mid-link.
    let mut by_flow: Vec<u32> = (0..ns)
        .filter(|&k| {
            let s = &h.segs[k as usize];
            s.lake_a < 0 && s.lake_b < 0 && h.node_kind[s.b as usize] == 0 && h.node_kind[s.a as usize] == 0 && h.profile(k, p.block_size).steps == 0 && s.valley_w < 300.0
        })
        .collect();
    by_flow.sort_by(|a, b| h.segs[*b as usize].flow.total_cmp(&h.segs[*a as usize].flow));
    let k = by_flow[0];
    let s = &h.segs[k as usize];
    let x = 0.5 * s.len;
    let at = s.point(x);
    let v = tangent(k, x);
    let side = v.cross(at).normalize();
    let mut eye = step(step(at, v, -40.0), side, s.hw + 3.0);
    for t in 0..10 {
        if !g.column(eye, p.block_size).wet() {
            break;
        }
        eye = step(step(at, v, -40.0), side, s.hw + 3.0 + 3.0 * (t + 1) as f64);
    }
    let (hd, pt) = look_at(eye, ground(eye) + 1.7, step(at, v, 120.0), g.column(at, p.block_size).water_m);
    add("river_bank", eye, ground(eye) + 1.7, hd, pt.max(-6.0), &format!("link {k}, {:.0} m3/s in lowland hills, from its bank looking downstream", s.flow));
    // 5. A lake shore from the ground: the largest lake, looking across.
    let mut by_size: Vec<(usize, i32)> = (0..h.lakes.len() as i32).map(|k| (h.field.iter().filter(|c| c.lake == k).count(), k)).collect();
    by_size.sort_by(|a, b| b.cmp(a));
    let lake = by_size[0].1;
    let shore = shore_point(lake, 1).or_else(|| shore_point(lake, 0)).unwrap();
    let inside = h.field.iter().filter(|c| c.lake == lake).min_by(|a, b| (a.pos - shore).length().total_cmp(&(b.pos - shore).length())).unwrap().pos;
    let v = {
        let t = inside - shore;
        (t - shore * t.dot(shore)).normalize()
    };
    let mut eye = step(shore, v, -25.0);
    for t in 0..20 {
        if !g.column(eye, p.block_size).wet() {
            break;
        }
        eye = step(shore, v, -25.0 - 4.0 * (t + 1) as f64);
    }
    let (hd, _) = look_at(eye, ground(eye) + 1.7, inside, h.lakes[lake as usize].w);
    add("lake_shore", eye, ground(eye) + 1.7, hd, -4.0, &format!("lake {lake} (level {:.2} m, lowered {:?}), from its shore", h.lakes[lake as usize].w, h.lakes_lowered));
    // 5b. The lake lowered furthest below the bake's level: its basin from
    // the shore and from 300 m.
    let raw = g.bake.hydro().unwrap();
    let (bowl, drop) = (0..h.lakes.len())
        .map(|k| (k, g.scale.soft_ceiling(g.scale.global_m(raw.lakes[k].level_km)) - h.lakes[k].w))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    if let Some(shore) = shore_point(bowl as i32, 0).or_else(|| shore_point(bowl as i32, 1)) {
        let inside = h.field.iter().filter(|c| c.lake == bowl as i32).min_by(|a, b| (a.pos - shore).length().total_cmp(&(b.pos - shore).length())).unwrap().pos;
        let v = {
            let t = inside - shore;
            (t - shore * t.dot(shore)).normalize()
        };
        let mut eye = step(shore, v, -30.0);
        for t in 0..20 {
            if !g.column(eye, p.block_size).wet() {
                break;
            }
            eye = step(shore, v, -30.0 - 4.0 * (t + 1) as f64);
        }
        let (hd, pt) = look_at(eye, ground(eye) + 1.7, inside, h.lakes[bowl].w);
        add("lake_bowl", eye, ground(eye) + 1.7, hd, pt.max(-12.0), &format!("lake {bowl}, lowered {drop:.1} m below the bake's level to the river reaching it, from its shore"));
        let eye3 = step(shore, v, -300.0);
        let (hd, pt) = look_at(eye3, ground(eye3).max(h.lakes[bowl].w) + 250.0, inside, h.lakes[bowl].w);
        add("lake_bowl_300m", eye3, ground(eye3).max(h.lakes[bowl].w) + 250.0, hd, pt, "the same lake from 250 m up, 300 m back");
    }
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/out/relief-shots");
    std::fs::create_dir_all(dir).unwrap();
    let mut list = String::new();
    for (site, text, eye) in scripts {
        std::fs::write(format!("{dir}/{site}.play"), text).unwrap();
        let (lat, lon) = Planet::lat_lon_deg(eye);
        list += &format!("{site} {lat:.6} {lon:.6}\n");
    }
    std::fs::write(format!("{dir}/sites.txt"), &list).unwrap();
    println!("wrote {dir}/*.play and sites.txt");
}

/// Not a check: the steepest steps between neighbouring columns over random
/// land patches (96 x 96 at 1 m), with where they are and what is there, to
/// find cliffs that should not exist.
#[test]
#[ignore]
fn relief_cliffs() {
    let g = world_gen();
    let land: Vec<DVec3> = dirs_pub(3000, 0xC11F).into_iter().filter(|d| g.bake.sample(*d).ocean < 0.5).take(300).collect();
    let mut worst: Vec<(f64, String)> = land
        .par_iter()
        .map(|&d| {
            let (h, _, _) = inspect::sample_grid(g, d, 96, 1.0, 1.0);
            let face = g.bake.sample(d).face;
            let (u0, v0) = bake::chart_uv(face, d);
            let r = g.planet.radius;
            let mut best = (0.0f64, 0usize, 0usize);
            for y in 0..96 {
                for x in 0..95 {
                    let s = (h[y * 96 + x + 1] - h[y * 96 + x]).abs() as f64;
                    if s > best.0 {
                        best = (s, x, y);
                    }
                }
            }
            let (s, x, y) = best;
            let half = 47.5;
            let at = inspect::chart_dir(face, u0 + (x as f64 - half) * 1.0 / r, v0 + (half - y as f64) * 1.0 / r);
            let (lat, lon) = Planet::lat_lon_deg(at);
            let c = g.column(at, 1.0);
            let sm = g.bake.sample(at);
            let l = g.lf.local(&sm);
            let cands = g.hydro_cands(at, 0.0).unwrap();
            let mut rivers = Vec::new();
            let hy = hydro();
            hy.rivers_at(at, &cands, 1.0, &mut rivers);
            let lake = hy.lake_at(at, &cands.cells, 1.0);
            let near_r: Vec<String> = rivers.iter().map(|rv| format!("link {} d {:.0} vw {:.0}", rv.seg, rv.d, hy.segs[rv.seg as usize].valley_w)).collect();
            (s, format!("step {s:.1} m at lat {lat:.5} lon {lon:.5}: h {:.1} W {:.1} {:?} relief {:.0} mesa {:.2} mountain {:.2}; rivers [{}]; lake fields {:?}", c.height_m, c.water_m, c.water_kind, l.relief, l.mesa, l.mountain, near_r.join(", "), lake.iter().map(|f| (f.lake, (f.field * 100.0).round() / 100.0)).collect::<Vec<_>>()))
        })
        .collect();
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, info) in worst.iter().take(8) {
        println!("{info}");
    }
}

/// Not a check: do the sim tests' hand-built scenes at the hills preset meet
/// the ground? `a_full_basin_rests_until_touched...` builds a 5 x 5 lake
/// 10 layers over the ground and a 9 x 7 basin beside it from 6 layers up;
/// `a_steady_throughflow...` uses the cell at the +i,+j corner of the chunk
/// holding ground + 12, 10 layers over that chunk's floor. Prints the
/// highest natural solid layer under each footprint against the scene's
/// floor, and what fills the throughflow cell.
#[test]
#[ignore]
fn relief_sim_scenes() {
    use crate::grid::BlockPos;
    let w = crate::world::tests::test_world();
    let g = crate::world::tests::hills_ground(&w);
    let reg = &w.registry;
    let top_solid = |i: u32, j: u32| -> u32 {
        (g.k.saturating_sub(40)..g.k + 80).rev().find(|&k| reg.is_solid(w.block_at(BlockPos { i, j, k, ..g }))).unwrap_or(0)
    };
    let footprint = |name: &str, i0: u32, j0: u32, ni: u32, nj: u32, floor_k: u32| {
        let (mut hi, mut at) = (0u32, (0u32, 0u32));
        for i in i0..i0 + ni {
            for j in j0..j0 + nj {
                let t = top_solid(i, j);
                if t > hi {
                    hi = t;
                    at = (i, j);
                }
            }
        }
        println!("{name}: floor plate at layer {floor_k} (ground at the site {}); highest natural solid under the footprint at layer {hi} ({:+} against the plate) at ({}, {})", g.k, hi as i64 - floor_k as i64, at.0, at.1);
    };
    let o = BlockPos { i: g.i - 3, j: g.j - 3, k: g.k + 10, ..g };
    let ob = BlockPos { i: o.i + 6, j: o.j - 2, k: o.k - 4, ..o };
    footprint("basin scene: the 9 x 7 basin", ob.i, ob.j, 11, 9, ob.k);
    footprint("basin scene: the 5 x 5 lake", o.i, o.j, 7, 7, o.k);
    let ck = crate::world::World::chunk_of(BlockPos { k: g.k + 12, ..g }).0;
    let (i0, j0, k0) = ck.origin_blocks();
    let (i0, j0, k0) = (i0 as u32, j0 as u32, k0 as u32);
    let c = BlockPos { face: ck.face, i: i0 + 31, j: j0 + 31, k: k0 + 10 };
    let name = |bp: BlockPos| reg.def(w.block_at(bp)).name.clone();
    println!(
        "throughflow scene: cell ({}, {}, {}) holds {}; below {}, -i {}, -j {}, +i {}, +j {}; the highest natural solid in that column is layer {}",
        c.i, c.j, c.k, name(c), name(BlockPos { k: c.k - 1, ..c }), name(BlockPos { i: c.i - 1, ..c }), name(BlockPos { j: c.j - 1, ..c }), name(BlockPos { i: c.i + 1, ..c }), name(BlockPos { j: c.j + 1, ..c }), top_solid(c.i, c.j)
    );
    // Generated water near the scenes: any natural water or ice cell in the
    // 3 x 3 chunks around the throughflow chunk at its layers, and around
    // the basin's chunk.
    let generator = &w.generator;
    let p = &w.planet;
    for (what, ci, cj, k_lo, k_hi) in [("throughflow chunk and its 8 neighbours", (i0 / 32) as i64, (j0 / 32) as i64, k0 as i64, k0 as i64 + 31), ("basin chunk and its 8 neighbours", (g.i / 32) as i64, (g.j / 32) as i64, g.k as i64 + 4, g.k as i64 + 20)] {
        let (mut wet, mut highest) = (0usize, f64::MIN);
        let mut example = None;
        for dx in -1..=1i64 {
            for dy in -1..=1i64 {
                for x in 0..32i64 {
                    for y in 0..32i64 {
                        let (i, j) = ((ci + dx) * 32 + x, (cj + dy) * 32 + y);
                        let col = generator.column(p.dir(g.face, i as f64 + 0.5, j as f64 + 0.5), p.block_size);
                        if col.water_m.is_finite() {
                            let top_layer = col.water_m / p.block_size + p.sea_level as f64;
                            if top_layer >= k_lo as f64 - 1.0 && col.water_m > col.height_m as f64 - 0.5 {
                                wet += 1;
                                if col.water_m > highest {
                                    highest = col.water_m;
                                    example = Some((i, j, col.water_kind, col.height_m));
                                }
                            }
                        }
                    }
                }
            }
        }
        println!("{what} (layers {k_lo}..{k_hi}): {wet} generated water columns with a surface at or above the scene's floor; highest {highest:.2} m above sea, e.g. {example:?}");
    }
}

/// Not a check: a lake for `mesh::tests::a_resting_lake_does_not_dip_at_chunk_borders`,
/// which wants a lake at rest with a partial top level (a surface off the
/// block grid) and sand on its shore, in a 5 x 5 block of chunks. Prints the
/// biggest lakes whose surface sits 0.3-0.7 of a block up, with the chunk
/// window around a shore point and the level the test should look for.
#[test]
#[ignore]
fn relief_lake_for_mesh() {
    let g = world_gen();
    let h = hydro();
    let p = &g.planet;
    let mut cands: Vec<(usize, i32, f64)> = (0..h.lakes.len() as i32)
        .map(|k| (h.field.iter().filter(|c| c.lake == k).count(), k, h.lakes[k as usize].w))
        .filter(|(_, _, w)| {
            let frac = (w / p.block_size).fract();
            (0.3..0.7).contains(&frac)
        })
        .collect();
    cands.sort_by(|a, b| b.0.cmp(&a.0));
    for (cells, k, w) in cands.iter().take(6) {
        let Some(shore) = shore_point(*k, 0) else { continue };
        let (f, i, j) = p.lattice_of_dir(shore);
        let (cx, cy) = ((i / 32.0).floor() as u32, (j / 32.0).floor() as u32);
        let cz = ((w / p.block_size) + p.sea_level as f64).floor() as u32 / 32;
        let level = ((w / p.block_size).fract() * 255.0).round() as u32;
        let (lat, lon) = Planet::lat_lon_deg(shore);
        println!("lake {k}: {cells} field cells, level {w:.3} m (partial level {level} in the cell at layer {}), shore at lat {lat:.4} lon {lon:.4}: face {f}, chunks cx {}..{} cy {}..{} cz {cz}", (w / p.block_size).floor() as i64 + p.sea_level as i64, cx - 2, cx + 2, cy - 2, cy + 2);
    }
}

/// Not a check: the local parameters and a 600 m transect (every 10 m, along
/// +u of the chart) at RELIEF_AT="lat lon" (or the belt's striped flank).
#[test]
#[ignore]
fn relief_probe() {
    let g = world_gen();
    let (lat, lon) = std::env::var("RELIEF_AT")
        .ok()
        .and_then(|s| {
            let v: Vec<f64> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            (v.len() == 2).then(|| (v[0], v[1]))
        })
        .unwrap_or((65.912, 134.841));
    let centre = Planet::dir_of_lat_lon_deg(lat, lon);
    let face = g.bake.sample(centre).face;
    let (u0, v0) = bake::chart_uv(face, centre);
    let r = g.planet.radius;
    let (dx, dy): (f64, f64) = std::env::var("RELIEF_OFF").ok().and_then(|s| {
        let v: Vec<f64> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        (v.len() == 2).then(|| (v[0], v[1]))
    }).unwrap_or((0.0, 0.0));
    let step: f64 = std::env::var("RELIEF_STEP").ok().and_then(|s| s.parse().ok()).unwrap_or(10.0);
    for k in 0..60 {
        let d = inspect::chart_dir(face, u0 + (dx + step * k as f64) / r, v0 + dy / r);
        let s = g.bake.sample(d);
        let l = g.lf.local(&s);
        let lf = g.lf.sample(d, &s, 1.0, 0.0);
        let c = g.column(d, 1.0);
        println!(
            "{:6.1} m: h {:7.2} dh {:7.2} slope {:.2} | relief {:.0} mountain {:.2} mesa {:.2} rugged {:.2} lam {:.0} step {:.1} strata {:.1} | uplift {:.2} rough {:.3} erod {:.2} elev {:.2} km temp {:.1} ({:.1} here) W {:.1} {:?}",
            step * k as f64, c.height_m, lf.dh, lf.slope, l.relief, l.mountain, l.mesa, l.rugged, l.lam_d, l.step_m, c.strata_off, s.uplift, s.rough_km, s.erodibility, s.elev_km, s.temp_c, c.temp_c, c.water_m, c.water_kind
        );
    }
}

/// A transect from a bank outward: ground heights (m) at 1 m steps, stopping
/// at the first wet column. `at` is the bank point (unit), `n` the outward
/// unit tangent.
fn transect(g: &Generator, at: DVec3, n: DVec3, len: usize) -> Vec<f64> {
    let r = g.planet.radius;
    let mut out = Vec::with_capacity(len);
    for x in 1..=len {
        let d = (at * r + n * x as f64).normalize();
        let c = g.column(d, g.planet.block_size);
        if c.wet() {
            break;
        }
        out.push(c.height_m as f64);
    }
    out
}

/// How far the ground at the bank stands above the lowest ground within the
/// transect (m): a levee reads as a positive number; land that falls towards
/// the water gives zero.
fn levee_m(t: &[f64]) -> Option<f64> {
    if t.len() < 20 {
        return None;
    }
    let near = t[0];
    let low = t.iter().copied().fold(f64::MAX, f64::min);
    Some(near - low)
}

struct Transects {
    n: usize,
    levees: Vec<f64>,
    rising: usize,
}

/// River cross-sections: 2,000 seeded (link, arc length) sites across the
/// planet, both banks, from the channel edge out to 60 m.
fn river_transects(reach: usize) -> Transects {
    let g = world_gen();
    let h = hydro();
    let ns = h.segs.len() as u32;
    let sites: Vec<(u32, f64)> = (0..2000i64).map(|t| ((noise::hash01(t, 2, 6, 0x7E12) * ns as f64) as u32 % ns, noise::hash01(t, 3, 6, 0x7E12))).collect();
    let rows: Vec<Vec<Option<(f64, String)>>> = sites
        .par_iter()
        .map(|&(k, u)| {
            let s = &h.segs[k as usize];
            let x = (0.1 + 0.8 * u) * s.len;
            let at = s.point(x);
            let a = s.point((x - 3.0).max(0.0));
            let b = s.point((x + 3.0).min(s.len));
            let t = b - a;
            let t = (t - at * t.dot(at)).normalize_or_zero();
            if t == DVec3::ZERO {
                return vec![None, None];
            }
            let side = t.cross(at).normalize();
            let r = g.planet.radius;
            [side, -side]
                .iter()
                .map(|&n| {
                    // The bank: the channel's edge at this LOD plus one block.
                    let bank = (at * r + n * (s.hw.max(2.0 * g.planet.block_size) + g.planet.block_size)).normalize();
                    let t = transect(g, bank, n, reach);
                    let lev = levee_m(&t)?;
                    let (lat, lon) = Planet::lat_lon_deg(bank);
                    let c = g.column(at, g.planet.block_size);
                    let base = g.scale.global_m(g.bake.sample(bank).elev_km);
                    let prof: Vec<String> = t.iter().step_by(6).map(|v| format!("{v:.1}")).collect();
                    Some((lev, format!("link {k} s {x:.0} at lat {lat:.4} lon {lon:.4}: W {:.2} bed {:.2} base {base:.2}; every 6 m: {}", c.water_m, c.height_m, prof.join(" "))))
                })
                .collect()
        })
        .collect();
    let mut all: Vec<&(f64, String)> = rows.iter().flatten().flatten().collect();
    all.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (lev, info) in all.iter().take(6) {
        println!("  levee {lev:.2} m: {info}");
    }
    let levees: Vec<f64> = all.iter().map(|e| e.0).collect();
    let rising = levees.iter().filter(|&&v| v > 0.5).count();
    Transects { n: levees.len(), levees, rising }
}

/// Lake-shore transects: two shore points on every lake, outward to 60 m.
fn lake_transects(reach: usize) -> Transects {
    let g = world_gen();
    let h = hydro();
    let pts: Vec<(DVec3, DVec3)> = (0..h.lakes.len() as i32)
        .into_par_iter()
        .flat_map_iter(|k| {
            (0..2).filter_map(move |nth| {
                let p = shore_point(k, nth)?;
                // Outward: away from the nearest cell of this lake.
                let inside = h.field.iter().filter(|c| c.lake == k).min_by(|a, b| (a.pos - p).length().total_cmp(&(b.pos - p).length()))?.pos;
                let v = p - inside;
                let n = (v - p * v.dot(p)).normalize_or_zero();
                (n != DVec3::ZERO).then_some((p, n))
            })
        })
        .collect();
    let levees: Vec<f64> = pts
        .par_iter()
        .filter_map(|&(p, n)| {
            let r = g.planet.radius;
            // Step out past the shore's own wet columns first.
            let mut start = p;
            for x in 0..12 {
                start = (p * r + n * x as f64).normalize();
                if !g.column(start, g.planet.block_size).wet() {
                    break;
                }
            }
            levee_m(&transect(g, start, n, reach))
        })
        .collect();
    let rising = levees.iter().filter(|&&v| v > 0.5).count();
    Transects { n: levees.len(), levees, rising }
}

fn report(name: &str, t: &mut Transects) -> f64 {
    let q = pctl(&mut t.levees, &[0.5, 0.9, 0.99, 1.0]);
    let share = t.rising as f64 / t.n.max(1) as f64;
    println!("{name}: {} transects; ground rising towards the water (over 0.5 m within 60 m of the bank) in {} ({:.1}%); levee height p50 {:.2} p90 {:.2} p99 {:.2} max {:.2} m", t.n, t.rising, 100.0 * share, q[0], q[1], q[2], q[3]);
    share
}

/// #6 (Andrew): land falls towards water. Of the river cross-sections and
/// lake shores sampled across the planet, only a small share may have the
/// ground rising towards the channel within 60 m of the bank, and any rise
/// must be small. On the baseline this fails (about half of all banks).
#[test]
fn land_falls_towards_water() {
    let t0 = std::time::Instant::now();
    let mut rivers = river_transects(60);
    let mut lakes = lake_transects(60);
    let (r, l) = (report("rivers", &mut rivers), report("lake shores", &mut lakes));
    println!("({:.1} s)", t0.elapsed().as_secs_f64());
    assert!(rivers.n > 1500 && lakes.n > 500, "too few transects ({} rivers, {} lakes)", rivers.n, lakes.n);
    assert!(r < 0.05, "rivers: ground rises towards the channel on {:.1}% of banks", 100.0 * r);
    assert!(l < 0.05, "lake shores: ground rises towards the water on {:.1}% of shores", 100.0 * l);
}

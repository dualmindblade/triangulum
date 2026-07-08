//! Water-discontinuity census: sweep the planet's water features analytically
//! (no rendering) and report every place the generated water surface is
//! physically impossible — the whole "wall of water" bug family the humans
//! have been finding by eye, found by exhaustion instead.
//!
//!   cargo run --release --example census                  # planet-wide
//!   cargo run --release --example census -- --at LAT LON --radius KM
//!                                                         # one site, dense
//!   census -- [--out FILE] [--quick N] (sample every Nth feature)
//!
//! Method: walk every river segment (cross-channel transects) and every lake
//! cell (radial spokes over the shore/flood annulus), sampling
//! terrain::sample at the voxel octave budget. Any adjacent pair that
//! disagrees about water is BISECTED until the disagreement either dies (it
//! was a smooth ramp / a real levee — not a bug) or survives at <=12 m
//! separation (a genuine discontinuity a player can stand next to). Classes:
//!
//!   WALL     water surface >2 m above adjacent dry ground
//!   JUMP     two water surfaces (neither sea) differing >2 m
//!   SEAJUMP  water surface differing from adjacent sea surface by >2 m
//!
//! By construction of the drainage export, levels along one course are
//! continuous — so JUMP only fires where independent reaches/levels collide
//! (nearest-segment Voronoi switches, lake-vs-river seams): exactly the bugs.
//! Findings are deduped into ~1.5 km site groups, sorted by magnitude, and
//! printed as teleport commands.

use std::fmt::Write as FmtWrite;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::DVec3;
use rayon::prelude::*;
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::terrain::{sample, Sample, VOXEL_OCTAVES};

const WALL_KM: f64 = 0.002; // 2 m
const JUMP_KM: f64 = 0.002;
const MIN_SEP_KM: f64 = 0.012; // bisect until this close
const GROUP_KM: f64 = 1.5;

static SAMPLES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Wall,
    Jump,
    SeaJump,
}

#[derive(Clone, Copy)]
struct Side {
    h_km: f64,
    water_km: f64, // NEG_INFINITY = dry
    sea: bool,
    lake: bool,
    river: bool,
}

#[derive(Clone)]
struct Finding {
    class: Class,
    mid: DVec3,
    mag_km: f64,
    a: Side,
    b: Side,
}

fn side(s: &Sample) -> Side {
    Side {
        h_km: s.h_km,
        water_km: if s.has_water() { s.water_km } else { f64::NEG_INFINITY },
        sea: s.sea,
        lake: s.lake,
        river: s.river_dist_km.is_finite() && s.river_dist_km < s.river_hw_km,
    }
}

fn probe(planet: &Planet, dir: DVec3) -> Side {
    SAMPLES.fetch_add(1, Ordering::Relaxed);
    let (f, u, v) = face_from_dir(dir);
    side(&sample(planet, f, u, v, VOXEL_OCTAVES))
}

/// The discontinuity class of an adjacent pair, if any.
fn classify(a: &Side, b: &Side) -> Option<(Class, f64)> {
    let (wa, wb) = (a.water_km, b.water_km);
    match (wa.is_finite(), wb.is_finite()) {
        (true, true) => {
            let d = (wa - wb).abs();
            if d > JUMP_KM {
                let c = if a.sea != b.sea { Class::SeaJump } else { Class::Jump };
                Some((c, d))
            } else {
                None
            }
        }
        (true, false) if wa - b.h_km > WALL_KM => Some((Class::Wall, wa - b.h_km)),
        (false, true) if wb - a.h_km > WALL_KM => Some((Class::Wall, wb - a.h_km)),
        _ => None,
    }
}

/// Bisect a flagged pair down to MIN_SEP_KM. Returns None when the
/// discontinuity dissolves under refinement (smooth ramp, or a dry levee
/// higher than both waters — physically fine, not a bug).
fn refine(planet: &Planet, mut pa: DVec3, mut a: Side, mut pb: DVec3, mut b: Side, r_km: f64) -> Option<Finding> {
    classify(&a, &b)?;
    while (pa - pb).length() * r_km > MIN_SEP_KM {
        let pm = (pa + pb).normalize();
        let m = probe(planet, pm);
        if classify(&a, &m).is_some() {
            pb = pm;
            b = m;
        } else if classify(&m, &b).is_some() {
            pa = pm;
            a = m;
        } else {
            return None;
        }
    }
    let (class, mag_km) = classify(&a, &b)?;
    Some(Finding { class, mid: (pa + pb).normalize(), mag_km, a, b })
}

/// Scan an ordered polyline of directions: classify consecutive pairs,
/// refine candidates (capped so a kilometre-long wall doesn't refine at
/// every step).
fn scan_line(planet: &Planet, pts: &[DVec3], r_km: f64, cap: usize, out: &mut Vec<Finding>) {
    let sides: Vec<Side> = pts.iter().map(|&p| probe(planet, p)).collect();
    let mut hits = 0usize;
    for i in 1..pts.len() {
        if classify(&sides[i - 1], &sides[i]).is_some() {
            if hits < cap
                && let Some(f) = refine(planet, pts[i - 1], sides[i - 1], pts[i], sides[i], r_km)
            {
                out.push(f);
            }
            hits += 1;
        }
    }
}

fn lat_lon(d: DVec3) -> (f64, f64) {
    (d.z.asin().to_degrees(), d.y.atan2(d.x).to_degrees())
}

fn describe(s: &Side) -> String {
    let kind = if s.sea {
        "sea"
    } else if s.lake {
        "lake"
    } else if s.river {
        "river"
    } else if s.water_km.is_finite() {
        "pond"
    } else {
        "dry"
    };
    if s.water_km.is_finite() {
        format!("{kind}@{:.1}m", s.water_km * 1000.0)
    } else {
        format!("{kind}@{:.1}m", s.h_km * 1000.0)
    }
}

struct Group {
    center: DVec3,
    mag_km: f64,
    n: usize,
    wall: usize,
    jump: usize,
    seajump: usize,
    best: Finding,
}

fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let mut out_path = String::new();
    let mut quick = 1usize;
    let mut at: Option<(f64, f64)> = None;
    let mut probe_at: Option<(f64, f64)> = None;
    let mut radius = 3.0f64;
    let mut i = 1;
    while i < argv.len() {
        let next = |i: usize| argv.get(i + 1).cloned().unwrap_or_default();
        match argv[i].as_str() {
            "--out" => {
                out_path = next(i);
                i += 1;
            }
            "--quick" => {
                quick = next(i).parse().unwrap_or(1).max(1);
                i += 1;
            }
            "--at" => {
                let lat: f64 = next(i).parse()?;
                let lon: f64 = argv.get(i + 2).cloned().unwrap_or_default().parse()?;
                at = Some((lat, lon));
                i += 2;
            }
            "--probe" => {
                let lat: f64 = next(i).parse()?;
                let lon: f64 = argv.get(i + 2).cloned().unwrap_or_default().parse()?;
                probe_at = Some((lat, lon));
                i += 2;
            }
            "--radius" => {
                radius = next(i).parse().unwrap_or(3.0);
                i += 1;
            }
            other => eprintln!("unknown arg: {other}"),
        }
        i += 1;
    }

    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let interchange = if std::path::Path::new("viewer/interchange").exists() {
        "viewer/interchange"
    } else {
        "interchange"
    };
    if out_path.is_empty() {
        out_path = format!("{interchange}/census.md");
    }
    let planet = Planet::load(assets)?;
    let r_km = planet.radius_km;
    let t0 = std::time::Instant::now();

    if let Some((lat, lon)) = probe_at {
        // full Sample dump at a point and its 8 neighbours (25 m ring) —
        // the triage tool for census findings
        let (la, lo) = (lat.to_radians(), lon.to_radians());
        let center = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
        let e = if center.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
        let t1 = (e - center * e.dot(center)).normalize();
        let t2 = center.cross(t1);
        for (dy, dx) in [(0i32, 0i32), (0, 1), (0, -1), (1, 0), (-1, 0), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let p = (center + (t1 * (dx as f64 * 0.025) + t2 * (dy as f64 * 0.025)) / r_km).normalize();
            let (f, u, v) = face_from_dir(p);
            let s = sample(&planet, f, u, v, VOXEL_OCTAVES);
            let (plat, plon) = lat_lon(p);
            println!(
                "({dx:+},{dy:+}) lat {plat:.4} lon {plon:.4}  h={:.1}m e_raw={:.1}m water={} sea={} lake={} riv_d={} wet={:.2} ofrac={:.2} wmask={:.2} rough={:.2}",
                s.h_km * 1000.0,
                s.e_raw * 1000.0,
                if s.has_water() { format!("{:.1}m", s.water_km * 1000.0) } else { "-".into() },
                s.sea,
                s.lake,
                if s.river_dist_km.is_finite() { format!("{:.2}km", s.river_dist_km) } else { "-".into() },
                s.wet_soft,
                planet.ocean(f, u, v),
                planet.water_frac(f, u, v),
                s.rough,
            );
        }
        return Ok(());
    }

    let mut findings: Vec<Finding> = Vec::new();

    if let Some((lat, lon)) = at {
        // ---- dense single-site grid: rows scanned in both axes ----
        let (la, lo) = (lat.to_radians(), lon.to_radians());
        let center = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
        let e = if center.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
        let t1 = (e - center * e.dot(center)).normalize();
        let t2 = center.cross(t1);
        let step = (radius / 200.0).max(0.02);
        let n = (radius / step).ceil() as i64;
        eprintln!("site census: {radius} km radius, {step:.3} km grid ({} pts)", (2 * n + 1) * (2 * n + 1));
        let rows: Vec<i64> = (-n..=n).collect();
        let mut per_row: Vec<Vec<Finding>> = rows
            .par_iter()
            .map(|&iy| {
                let mut out = Vec::new();
                // horizontal scan line
                let pts: Vec<DVec3> = (-n..=n)
                    .map(|ix| {
                        (center + (t1 * (ix as f64 * step) + t2 * (iy as f64 * step)) / r_km).normalize()
                    })
                    .collect();
                scan_line(&planet, &pts, r_km, usize::MAX, &mut out);
                // vertical scan line (same index used as column)
                let pts: Vec<DVec3> = (-n..=n)
                    .map(|ix| {
                        (center + (t1 * (iy as f64 * step) + t2 * (ix as f64 * step)) / r_km).normalize()
                    })
                    .collect();
                scan_line(&planet, &pts, r_km, usize::MAX, &mut out);
                out
            })
            .collect();
        for v in &mut per_row {
            findings.append(v);
        }
    } else {
        // ---- planet-wide: river transects + lake spokes ----
        let segs = &planet.rivers.segments;
        eprintln!("census: {} river segments, {} lake cells", segs.len(), planet.rivers.lakes.len());
        let offsets: [f64; 13] = [-3.2, -1.6, -0.8, -0.4, -0.2, -0.1, 0.0, 0.1, 0.2, 0.4, 0.8, 1.6, 3.2];
        let mut per_seg: Vec<Vec<Finding>> = (0..segs.len())
            .into_par_iter()
            .filter(|si| si % quick == 0) // NOT step_by: that serialized the pool
            .map(|si| {
                let s = &segs[si];
                let mut out = Vec::new();
                let len_km = (s.a - s.b).length() * r_km;
                let steps = ((len_km / 0.15).ceil() as usize).clamp(2, 400);
                for k in 0..=steps {
                    let t = k as f64 / steps as f64;
                    let p = (s.a + (s.b - s.a) * t).normalize();
                    let along = (s.b - s.a).normalize_or_zero();
                    let across = p.cross(along).normalize_or_zero();
                    if across.length_squared() < 0.5 {
                        continue;
                    }
                    let pts: Vec<DVec3> =
                        offsets.iter().map(|&o| (p + across * (o / r_km)).normalize()).collect();
                    scan_line(&planet, &pts, r_km, 6, &mut out);
                }
                if si % 2000 == 0 {
                    eprintln!("  seg {si}... ({} samples)", SAMPLES.load(Ordering::Relaxed));
                }
                out
            })
            .collect();
        for v in &mut per_seg {
            findings.append(v);
        }

        let lakes = &planet.rivers.lakes;
        let mut per_lake: Vec<Vec<Finding>> = (0..lakes.len())
            .into_par_iter()
            .filter(|li| li % quick == 0)
            .map(|li| {
                let l = &lakes[li];
                let mut out = Vec::new();
                if l.rim {
                    return out; // rims don't flood; their territory is covered by lake-cell annuli
                }
                let r = l.radius_km as f64;
                let e = if l.center.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
                let t1 = (e - l.center * e.dot(l.center)).normalize();
                let t2 = l.center.cross(t1);
                let (r0, r1) = (0.6 * r, 3.4 * r);
                let rstep = ((r1 - r0) / 120.0).max(0.15);
                let nr = ((r1 - r0) / rstep).ceil() as usize;
                for sp in 0..36 {
                    let ang = sp as f64 / 36.0 * std::f64::consts::TAU;
                    let radial = t1 * ang.cos() + t2 * ang.sin();
                    let pts: Vec<DVec3> = (0..=nr)
                        .map(|k| {
                            let d = r0 + k as f64 * rstep;
                            (l.center + radial * (d / r_km)).normalize()
                        })
                        .collect();
                    scan_line(&planet, &pts, r_km, 4, &mut out);
                }
                out
            })
            .collect();
        for v in &mut per_lake {
            findings.append(v);
        }
    }

    // ---- group into sites ----
    findings.sort_by(|x, y| y.mag_km.total_cmp(&x.mag_km));
    let mut groups: Vec<Group> = Vec::new();
    for f in &findings {
        let g = groups
            .iter_mut()
            .find(|g| (g.center - f.mid).length() * r_km < GROUP_KM);
        match g {
            Some(g) => {
                g.n += 1;
                match f.class {
                    Class::Wall => g.wall += 1,
                    Class::Jump => g.jump += 1,
                    Class::SeaJump => g.seajump += 1,
                }
            }
            None => {
                let mut g = Group {
                    center: f.mid,
                    mag_km: f.mag_km,
                    n: 1,
                    wall: 0,
                    jump: 0,
                    seajump: 0,
                    best: f.clone(),
                };
                match f.class {
                    Class::Wall => g.wall += 1,
                    Class::Jump => g.jump += 1,
                    Class::SeaJump => g.seajump += 1,
                }
                groups.push(g);
            }
        }
    }

    let mut report = String::new();
    let _ = writeln!(
        report,
        "# Water-discontinuity census\n\n{} raw findings, {} sites; {} samples in {:.0}s\n",
        findings.len(),
        groups.len(),
        SAMPLES.load(Ordering::Relaxed),
        t0.elapsed().as_secs_f64()
    );
    let (mut nw, mut nj, mut ns) = (0usize, 0usize, 0usize);
    for g in &groups {
        nw += g.wall;
        nj += g.jump;
        ns += g.seajump;
    }
    let _ = writeln!(report, "class totals: WALL {nw}  JUMP {nj}  SEAJUMP {ns}\n");
    for g in &groups {
        let (lat, lon) = lat_lon(g.center);
        let cls = [
            (g.wall, "WALL"),
            (g.jump, "JUMP"),
            (g.seajump, "SEAJUMP"),
        ]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, c)| format!("{c}:{n}"))
        .collect::<Vec<_>>()
        .join(",");
        let _ = writeln!(
            report,
            "teleport {lat:.3} {lon:.3}   # d={:.1}m n={} [{}] {} | {}",
            g.mag_km * 1000.0,
            g.n,
            cls,
            describe(&g.best.a),
            describe(&g.best.b),
        );
    }
    std::fs::write(&out_path, &report)?;
    // stdout: summary + top sites
    for line in report.lines().take(46) {
        println!("{line}");
    }
    if groups.len() > 40 {
        println!("... full list in {out_path}");
    }
    Ok(())
}

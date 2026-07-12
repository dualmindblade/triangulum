//! Block-truth water census: walks the CANONICAL COLUMN lattice in a disc
//! and reports block-scale water discontinuities the sample-level census
//! cannot see (closes BUGS.md T-1). Every column's open-air water and
//! cave-water pool is compared against its 4-neighborhood:
//!   LIP   open-air water (2+ deep) standing above a neighbor's dry ground
//!   EDGE  1-deep water standing above a dry neighbor - the shallow ring
//!         Austin reported at Difficulty Lake (analog-clearance dead band;
//!         includes creek films, which are 1-deep by construction)
//!   SHOAL equal-block liquid-lake tie raised to dry ground at water level
//!   CAVEP cave pool breaching the surface (pit open to sky, water below)
//! Prints class totals, worst sites as teleport commands, and per-class
//! coordinates for the play harness to re-shoot.
//!
//!   cargo run --release --example colcensus -- [--body neisor|moon] LAT LON [RADIUS_KM]
use std::sync::Arc;
use glam::DVec3;
use rayon::prelude::*;
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::voxel::{
    col_ctx_ext_body, ColCtx, Edits, LunarBody, VoxelBody, COLUMNS_PER_FACE,
};

struct Hit {
    class: &'static str,
    di: i64,
    dj: i64,
    drop: i64,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (body_name, first) = if args.get(1).is_some_and(|s| s == "--body") {
        (args.get(2).map(String::as_str).unwrap_or(""), 3usize)
    } else {
        ("neisor", 1usize)
    };
    let lat: f64 = args[first].parse()?;
    let lon: f64 = args[first + 1].parse()?;
    let radius_km: f64 = args
        .get(first + 2)
        .map(|s| s.parse().unwrap_or(0.25))
        .unwrap_or(0.25);
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets)?;
    let solar = triangulum_viewer::orbits::SolarTuning::load(assets);
    let moon = LunarBody::new(
        solar.radius_km(triangulum_viewer::orbits::BodyId::Moon, planet.radius_km),
        Arc::new(triangulum_viewer::moon::MoonGenerator::new(planet.seed)),
    );
    let body: &dyn VoxelBody = match body_name.to_ascii_lowercase().as_str() {
        "neisor" => &planet,
        "moon" => &moon,
        _ => anyhow::bail!("--body must be neisor or moon"),
    };
    let edits = Edits::default();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let dir = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let (face, u, v) = face_from_dir(dir);
    let nn = COLUMNS_PER_FACE as f64;
    let ci = (((u + 1.0) * 0.5) * nn).floor() as i64;
    let cj = (((v + 1.0) * 0.5) * nn).floor() as i64;
    // gnomonic columns are ~1.7 x 1.0 m at face centers; use the safe bound
    let half = (radius_km * 1000.0).ceil() as i64;
    eprintln!(
        "column census: face {face} center ({ci},{cj}), +-{half} cols (~{radius_km} km), {} columns",
        (2 * half + 1) * (2 * half + 1)
    );

    let ctx = |di: i64, dj: i64| -> ColCtx {
        col_ctx_ext_body(body, &edits, face, ci + di, cj + dj)
    };
    // a real great-circle disc, not the index square: gnomonic columns are
    // anisotropic (~1.7 x 1.0 m), so the [-half,half]^2 lattice both
    // over-reaches on one axis and would attribute corner findings to a
    // radius the user never asked for (review #2 finding 5 measured the
    // square at 3.5x the disc's column count)
    let radius_km_2 = radius_km * radius_km;
    let center_dir = triangulum_viewer::planet::face_dir(
        face,
        -1.0 + 2.0 * (ci as f64 + 0.5) / nn,
        -1.0 + 2.0 * (cj as f64 + 0.5) / nn,
    );
    let in_disc = |di: i64, dj: i64| -> bool {
        let u = -1.0 + 2.0 * ((ci + di) as f64 + 0.5) / nn;
        let v = -1.0 + 2.0 * ((cj + dj) as f64 + 0.5) / nn;
        let d = triangulum_viewer::planet::face_dir(face, u, v);
        (d - center_dir).length_squared() * body.radius_km() * body.radius_km() <= radius_km_2
    };
    let rows: Vec<Vec<Hit>> = (-half..=half)
        .into_par_iter()
        .map(|dj| {
            let mut out = Vec::new();
            for di in -half..=half {
                if !in_disc(di, dj) {
                    continue;
                }
                let c = ctx(di, dj);
                // neighbor ground tops (walk surface, ignoring trees)
                let n = [ctx(di + 1, dj), ctx(di - 1, dj), ctx(di, dj + 1), ctx(di, dj - 1)];
                if c.lake_shoal {
                    out.push(Hit { class: "SHOAL", di, dj, drop: 0 });
                }
                if c.has_water() {
                    let wtop = c.water;
                    let film = wtop == c.ground + 1;
                    // a healthy shore's dry bank stands AT or ABOVE the
                    // water top; any dry neighbor below it means standing
                    // water edge - Austin's Difficulty Lake ring is the
                    // drop=1 case (the analog-clearance dead band)
                    for nb in &n {
                        if !nb.has_water() && wtop - nb.ground >= 1 {
                            out.push(Hit {
                                class: if film { "EDGE" } else { "LIP" },
                                di,
                                dj,
                                drop: wtop - nb.ground,
                            });
                            break;
                        }
                    }
                } else if c.cave_water != i64::MIN && !c.filled(c.ground) {
                    // cave pool with the surface block carved away: a
                    // breach pit. Not a defect by itself (karst is canon) -
                    // counted so hunts can find dense fields and verify the
                    // mesh hint against them.
                    out.push(Hit { class: "CAVEP", di, dj, drop: c.ground - c.cave_water });
                }
            }
            out
        })
        .collect();

    let mut all: Vec<Hit> = rows.into_iter().flatten().collect();
    if body.body_id() == triangulum_viewer::orbits::BodyId::Moon {
        let center = ctx(0, 0);
        let material = center
            .lunar
            .map(|l| format!("{:?}", l.material))
            .unwrap_or_else(|| "none".into());
        let max_step = [ctx(1, 0), ctx(-1, 0), ctx(0, 1), ctx(0, -1)]
            .into_iter()
            .map(|n| (center.ground0 - n.ground0).abs())
            .max()
            .unwrap_or(0);
        println!(
            "moon center: ground={} material={} albedo={:.4} max-neighbor-step={} blocks",
            center.ground0,
            material,
            center.lunar.map_or(0.0, |l| l.albedo),
            max_step
        );
    }
    for class in ["LIP", "EDGE", "SHOAL", "CAVEP"] {
        let mut hits: Vec<&Hit> = all.iter().filter(|h| h.class == class).collect();
        hits.sort_by_key(|h| -h.drop);
        println!("{class}: {} columns", hits.len());
        for h in hits.iter().take(5) {
            // column center back to lat/lon for teleport
            let uu = -1.0 + 2.0 * ((ci + h.di) as f64 + 0.5) / nn;
            let vv = -1.0 + 2.0 * ((cj + h.dj) as f64 + 0.5) / nn;
            let d = triangulum_viewer::planet::face_dir(face, uu, vv);
            let hlat = d.z.asin().to_degrees();
            let hlon = d.y.atan2(d.x).to_degrees();
            let command = if body.body_id() == triangulum_viewer::orbits::BodyId::Moon {
                "moonland"
            } else {
                "teleport"
            };
            println!("  drop {} blocks  {command} {hlat:.5} {hlon:.5} 0.02", h.drop);
        }
    }
    all.retain(|h| h.class == "LIP");
    if !all.is_empty() {
        std::process::exit(2); // LIPs are defects; FILM/CAVEP informational
    }
    Ok(())
}

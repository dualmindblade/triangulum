//! Find flooded-cave sites (BUGS.md W-6). Scans a lat/lon window and reports
//! columns whose caves pass below the local water table (cave_water set),
//! classifying each as an open water-filled PIT (surface breached, swimmable
//! from above) or a BURIED flooded run. Usage:
//!   cargo run --release --example caveprobe -- LAT LON [HALF]
use triangulum_viewer::planet::{face_dir, face_from_dir, Planet};
use triangulum_viewer::voxel::{canonical_column, col_ctx_ext, ColCtx, COLUMNS_PER_FACE};

const CAVE_DEPTH: i64 = 26;

fn latlon(face: usize, ci: u64, cj: u64) -> (f64, f64) {
    let nn = COLUMNS_PER_FACE as f64;
    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
    let d = face_dir(face, u, v).normalize();
    (d.z.asin().to_degrees(), d.y.atan2(d.x).to_degrees())
}

fn flooded_cells(c: &ColCtx) -> i64 {
    if c.cave_water == i64::MIN {
        return 0;
    }
    let mut n = 0;
    for z in (c.ground0 - CAVE_DEPTH)..=c.cave_water {
        if c.cave_flooded(z) {
            n += 1;
        }
    }
    n
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lat: f64 = args[1].parse().unwrap();
    let lon: f64 = args[2].parse().unwrap();
    let half: i64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(60);
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets).unwrap();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let dir = glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let (face, u, v) = face_from_dir(dir);
    let nn = COLUMNS_PER_FACE as f64;
    let ci0 = ((u + 1.0) * 0.5 * nn) as i64;
    let cj0 = ((v + 1.0) * 0.5 * nn) as i64;
    let edits = Default::default();

    // center column report
    let (center_face, center_ci, center_cj) = canonical_column(face, ci0, cj0);
    let cc = col_ctx_ext(&planet, &edits, face, ci0, cj0);
    println!(
        "CENTER {lat:.4} {lon:.4}: face={} ci={} cj={} ground0={} ground={} top_solid={} water={} cave_water={} cave_bits={:026b}",
        center_face, center_ci, center_cj,
        cc.ground0, cc.ground, cc.top_solid(),
        if cc.water == i64::MIN { "dry".into() } else { cc.water.to_string() },
        if cc.cave_water == i64::MIN { "none".into() } else { cc.cave_water.to_string() },
        cc.cave_bits,
    );

    let mut pits = 0;
    let mut buried = 0;
    // swimmable pits: open to sky (top_solid < ground0), by water depth
    let mut cands: Vec<(i64, u8, u64, u64, ColCtx)> = Vec::new();
    for dj in -half..=half {
        for di in -half..=half {
            let i_ext = ci0 + di;
            let j_ext = cj0 + dj;
            let (canon_face, ci, cj) = canonical_column(face, i_ext, j_ext);
            let c = col_ctx_ext(&planet, &edits, face, i_ext, j_ext);
            if c.cave_water == i64::MIN {
                continue;
            }
            let open = c.top_solid() < c.ground0; // surface breached -> pit
            if open {
                pits += 1;
                let depth = c.cave_water - c.top_solid(); // water column blocks
                if flooded_cells(&c) >= 2 {
                    cands.push((depth, canon_face, ci, cj, c));
                }
            } else {
                buried += 1;
            }
        }
    }
    println!("window +-{half}: {pits} open pits, {buried} buried flooded columns");
    cands.sort_by_key(|(d, ..)| -d);
    for (depth, canon_face, ci, cj, c) in cands.iter().take(8) {
        let (blat, blon) = latlon(*canon_face as usize, *ci, *cj);
        println!(
            "PIT {blat:.5} {blon:.5}: face={canon_face} ci={ci} cj={cj} water_depth={depth}  top_solid={} cave_water={} ground0={} flooded={}",
            c.top_solid(), c.cave_water, c.ground0, flooded_cells(c),
        );
    }
    // buried dig target: SOLID surface (top_solid==ground0) with the flooded
    // water table only a few blocks down, so a short dig shaft reaches water
    let mut dig: Option<(i64, u8, u64, u64, ColCtx)> = None;
    for dj in -half..=half {
        for di in -half..=half {
            let i_ext = ci0 + di;
            let j_ext = cj0 + dj;
            let (canon_face, ci, cj) = canonical_column(face, i_ext, j_ext);
            let c = col_ctx_ext(&planet, &edits, face, i_ext, j_ext);
            if c.cave_water == i64::MIN || c.top_solid() != c.ground0 {
                continue;
            }
            let gap = c.ground0 - c.cave_water; // rock thickness above the table
            if gap >= 1 && gap <= 8 && flooded_cells(&c) >= 3 && c.cave_flooded(c.cave_water) {
                if dig.as_ref().map(|(g, ..)| gap < *g).unwrap_or(true) {
                    dig = Some((gap, canon_face, ci, cj, c));
                }
            }
        }
    }
    if let Some((gap, canon_face, ci, cj, c)) = &dig {
        let (dlat, dlon) = latlon(*canon_face as usize, *ci, *cj);
        println!(
            "\nBURIED DIG TARGET {dlat:.6} {dlon:.6}: face={canon_face} ci={ci} cj={cj} rock_cap={gap} blocks  ground0={} cave_water={} flooded={}",
            c.ground0, c.cave_water, flooded_cells(c),
        );
    }

    if let Some((_, canon_face, ci, cj, c)) = cands.first() {
        let (blat, blon) = latlon(*canon_face as usize, *ci, *cj);
        println!("\nBEST PIT PROFILE {blat:.6} {blon:.6}: face={canon_face} ci={ci} cj={cj}");
        for z in ((c.ground0 - CAVE_DEPTH)..=(c.ground0 + 1)).rev() {
            let kind = if c.filled(z) {
                "ROCK"
            } else if c.cave_flooded(z) {
                "WATER"
            } else {
                "air"
            };
            let mark = if z == c.top_solid() { " <- top_solid" }
                else if z == c.cave_water { " <- water table" } else { "" };
            println!("  z={z:>4} {kind}{mark}");
        }
    }
}

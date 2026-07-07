//! Probe generation context around a lat/lon: what koppen/flora decisions
//! are made there? Usage: cargo run --release --example probe -- LAT LON N
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::voxel::{col_ctx, COLUMNS_PER_FACE};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lat: f64 = args[1].parse().unwrap();
    let lon: f64 = args[2].parse().unwrap();
    let n: i64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(48);
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
    // scan anchors: report every tree and how many of its cells are buried
    // below the terrain of the cell's own column
    for dj in 0..n {
        for di in 0..n {
            let (ci, cj) = ((ci0 + di) as u64, (cj0 + dj) as u64);
            let c = col_ctx(&planet, &edits, face, ci, cj);
            if let Some((kind, trunk)) = triangulum_viewer::voxel::tree_at(
                &c,
                face as u8,
                ci,
                cj,
                planet.seed,
            ) {
                let rnd = 0u64; // shape detail irrelevant for burial stats
                let cells = triangulum_viewer::voxel::tree_cells(kind, trunk, rnd);
                let mut buried = 0;
                let total = cells.len();
                for &(dx, dy, dz, _) in &cells {
                    let cc = col_ctx(
                        &planet,
                        &edits,
                        face,
                        (ci as i64 + dx) as u64,
                        (cj as i64 + dy) as u64,
                    );
                    if c.ground + dz <= cc.ground {
                        buried += 1;
                    }
                }
                println!(
                    "tree {kind:?} trunk {trunk} at +({di},{dj}) anchor_ground {} buried {buried}/{total}",
                    c.ground
                );
            }
        }
    }
}

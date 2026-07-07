//! Print the voxel column (face, ci, cj) under a lat/lon, plus its ground —
//! for crafting test edits and aiming captures at exact columns.
//! Usage: cargo run --release --example colof -- LAT LON
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::voxel::{col_ctx, COLUMNS_PER_FACE};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let lat: f64 = a[1].parse().unwrap();
    let lon: f64 = a[2].parse().unwrap();
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets).unwrap();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let d = glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let (f, u, v) = face_from_dir(d);
    let n = COLUMNS_PER_FACE as f64;
    let ci = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let cj = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let c = col_ctx(&planet, &Default::default(), f, ci, cj);
    println!(
        "face {f} ci {ci} cj {cj} ground {} water {} koppen {}",
        c.ground,
        if c.water == i64::MIN { "dry".into() } else { c.water.to_string() },
        c.koppen
    );
}

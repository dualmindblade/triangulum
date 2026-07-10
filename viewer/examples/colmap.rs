//! Dump a col_ctx water/ground map around a lat/lon — ASCII art of the
//! voxel truth grid, for comparing against what chunks render.
//!   cargo run --release --example colmap -- LAT LON [HALF_COLS]
use glam::DVec3;
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::voxel::{col_ctx_ext, Edits};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let lat: f64 = args[1].parse()?;
    let lon: f64 = args[2].parse()?;
    let half: i64 = args.get(3).map(|s| s.parse().unwrap_or(60)).unwrap_or(60);
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets)?;
    let edits = Edits::default();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let dir = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let (face, u, v) = face_from_dir(dir);
    let nn = triangulum_viewer::voxel::COLUMNS_PER_FACE as f64;
    let ci = (((u + 1.0) * 0.5) * nn).floor() as i64;
    let cj = (((v + 1.0) * 0.5) * nn).floor() as i64;
    println!("center face {face} ci {ci} cj {cj} ({half} cols half-width, ~1.7 m/col)");
    // j increases = +v; print top row = +v side so orientation is stated
    for dj in (-half..=half).rev() {
        let mut row = String::new();
        for di in -half..=half {
            let c = col_ctx_ext(&planet, &edits, face, ci + di, cj + dj, );
            row.push(if c.has_water() {
                '~'
            } else if c.cave_water != i64::MIN {
                'C'
            } else if di == 0 && dj == 0 {
                '@'
            } else {
                '.'
            });
        }
        println!("{row}");
    }
    Ok(())
}

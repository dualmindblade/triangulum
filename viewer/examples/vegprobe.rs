//! Vegetation/beach agreement probe over the canonical column lattice.
//!
//! Reports the obsolete raw-height tree boundary, trees whose anchors use the
//! same coastal-sand decision as voxel materials, and the E-2 center-vs-full
//! tree eligibility mismatch. This is a diagnostic, not a render path.
//!
//!   cargo run --release --example vegprobe -- LAT LON [RADIUS_KM] [STRIDE]

use std::collections::BTreeMap;

use glam::DVec3;
use triangulum_viewer::planet::{face_from_dir, MainBlock, Planet};
use triangulum_viewer::voxel::{
    canonical_column, coastal_beach_at, col_ctx, tree_at, tree_here, Edits, COLUMNS_PER_FACE,
};

#[derive(Default)]
struct Band {
    columns: usize,
    coast_sand: usize,
    trees: usize,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let lat: f64 = args[1].parse()?;
    let lon: f64 = args[2].parse()?;
    let radius_km: f64 = args.get(3).map(|s| s.parse().unwrap_or(0.5)).unwrap_or(0.5);
    let stride: i64 = args
        .get(4)
        .map(|s| s.parse().unwrap_or(4))
        .unwrap_or(4)
        .max(1);
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets)?;
    let edits = Edits::default();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let center = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let (face, u, v) = face_from_dir(center);
    let n = COLUMNS_PER_FACE as f64;
    let ci0 = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as i64;
    let cj0 = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as i64;
    // Safe face-center bound; the great-circle test below owns the real disc.
    let half = (radius_km * 1000.0).ceil() as i64;
    let mut total = 0usize;
    let mut climate_blocks = [0usize; 3];
    let mut coast_sand = 0usize;
    let mut tree_lottery = 0usize;
    let mut full_trees = 0usize;
    let mut e2_mismatch = 0usize;
    let mut trees_on_coast_sand = 0usize;
    let mut raw_low_grass = 0usize;
    let mut raw_low_grass_trees = 0usize;
    let mut koppen = BTreeMap::<u8, (usize, usize)>::new();
    let mut bands = BTreeMap::<i64, Band>::new();

    for dj in (-half..=half).step_by(stride as usize) {
        for di in (-half..=half).step_by(stride as usize) {
            let (cf, ci, cj) = canonical_column(face, ci0 + di, cj0 + dj);
            let uu = -1.0 + 2.0 * (ci as f64 + 0.5) / n;
            let vv = -1.0 + 2.0 * (cj as f64 + 0.5) / n;
            let dir = triangulum_viewer::planet::face_dir(cf as usize, uu, vv);
            if (dir - center).length_squared() * planet.radius_km * planet.radius_km
                > radius_km * radius_km
            {
                continue;
            }
            total += 1;
            let c = col_ctx(&planet, &edits, cf as usize, ci, cj);
            let climate = c.climate;
            climate_blocks[match climate.main_block {
                MainBlock::Grass => 0,
                MainBlock::Sand => 1,
                MainBlock::Snow => 2,
            }] += 1;
            let coastal = coastal_beach_at(&c, cf, ci, cj, planet.seed);
            coast_sand += coastal as usize;
            let tree = tree_at(&c, cf, ci, cj, planet.seed);
            tree_lottery += tree.is_some() as usize;
            trees_on_coast_sand += (coastal && tree.is_some()) as usize;
            if tree.is_some() {
                let full = tree_here(&planet, &edits, cf as usize, ci, cj).is_some();
                full_trees += full as usize;
                e2_mismatch += (!full) as usize;
            }
            let low_grass = c.e_raw < 0.010 && !coastal && climate.main_block == MainBlock::Grass;
            raw_low_grass += low_grass as usize;
            raw_low_grass_trees += (low_grass && tree.is_some()) as usize;
            let k = koppen.entry(c.koppen).or_default();
            k.0 += 1;
            k.1 += tree.is_some() as usize;
            let band = bands.entry(c.ground0).or_default();
            band.columns += 1;
            band.coast_sand += coastal as usize;
            band.trees += tree.is_some() as usize;
        }
    }

    println!("pose {lat:.6} {lon:.6}, radius {radius_km:.3} km, stride {stride}");
    println!("columns {total}");
    println!(
        "climate main blocks: grass {} sand {} snow {}",
        climate_blocks[0], climate_blocks[1], climate_blocks[2]
    );
    println!("coastal sand {coast_sand}");
    println!(
        "tree_at {tree_lottery}; tree_here {full_trees}; E-2 center-only mismatches {e2_mismatch}"
    );
    println!("trees on coastal sand {trees_on_coast_sand}");
    println!("raw<10m grass/non-sand {raw_low_grass}; trees there {raw_low_grass_trees}");
    println!("koppen columns/trees: {koppen:?}");
    println!("ground bands (z: columns/sand/trees):");
    for (z, b) in bands {
        if b.coast_sand > 0 || b.trees > 0 {
            println!("  {z}: {}/{}/{}", b.columns, b.coast_sand, b.trees);
        }
    }
    Ok(())
}

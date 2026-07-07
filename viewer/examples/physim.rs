//! Sanity-sim for the walk/fly collision queries: drop onto the terrain,
//! report support/ceiling along a transect, find cave pits and roofs.
//! Usage: cargo run --release --example physim -- LAT LON [N]
use triangulum_viewer::planet::{face_dir, face_from_dir, Planet};
use triangulum_viewer::voxel::{
    ceiling_above_km, col_ctx, support_below_km, surface_height_km, tree_here, TreeKind,
    COLUMNS_PER_FACE,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lat: f64 = args[1].parse().unwrap();
    let lon: f64 = args[2].parse().unwrap();
    let n: i64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(60);
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets).unwrap();
    let edits = Default::default();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let start = glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let (face, u, v) = face_from_dir(start);
    let nn = COLUMNS_PER_FACE as f64;
    let ci0 = ((u + 1.0) * 0.5 * nn) as i64;
    let cj0 = ((v + 1.0) * 0.5 * nn) as i64;

    let mut pits = 0;
    let mut roofs = 0;
    let mut trunks = 0;
    for d in 0..n {
        let (ci, cj) = ((ci0 + d) as u64, cj0 as u64);
        let uu = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
        let vv = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
        let dir = face_dir(face, uu, vv);
        let c = col_ctx(&planet, &edits, face, ci, cj);
        let surf = surface_height_km(&planet, &edits, dir, 1.0);
        // walker feet 10 m above ground: lands on the trunk top if a tree
        // stands here (trunks are solid), else on the walkable surface
        let trunk = tree_here(&planet, &edits, face, ci, cj)
            .filter(|(k, _)| *k != TreeKind::Shrub)
            .map(|(_, t)| t)
            .unwrap_or(0);
        trunks += (trunk > 0) as i32;
        let expect = surf + trunk as f64 * 0.001;
        let support = support_below_km(&planet, &edits, dir, expect + 0.010, 1.0);
        let ceil = ceiling_above_km(&planet, &edits, dir, support + 1e-6, 1.0);
        assert!(
            (support - expect).abs() < 1e-9,
            "support from above must equal surface+trunk (col {d}: {support} vs {expect})"
        );
        if c.top_solid() < c.ground {
            pits += 1;
            // inside the pit: support below the breach must be the cave floor,
            // and if the tube continues there may be a roof overhead
            let deep = support_below_km(&planet, &edits, dir, surf - 0.0005, 1.0);
            assert!(deep <= surf + 1e-9);
        }
        if ceil.is_finite() {
            roofs += 1;
            assert!(ceil > support, "roof must clear the floor (col {d})");
            println!(
                "col +{d}: floor {:.1} m, roof {:.1} m, headroom {:.1} m (ground {} top {})",
                support * 1000.0,
                ceil * 1000.0,
                (ceil - support) * 1000.0,
                c.ground,
                c.top_solid()
            );
        }
    }
    println!(
        "{n} columns: {pits} cave-breached, {roofs} roofed, {trunks} solid trunks — queries consistent"
    );
}

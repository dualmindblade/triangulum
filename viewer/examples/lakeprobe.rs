//! Dump the raw LakeHit at a point and its 25 m ring — triage for shore-apron
//! and flood-eligibility questions (which lake claims a column, and how far
//! past which frontier it sits). Usage: lakeprobe LAT LON
use triangulum_viewer::planet::{face_from_dir, Planet};

fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let lat: f64 = argv[1].parse()?;
    let lon: f64 = argv[2].parse()?;
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets)?;
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let center = glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let e = if center.z.abs() < 0.9 { glam::DVec3::Z } else { glam::DVec3::X };
    let t1 = (e - center * e.dot(center)).normalize();
    let t2 = center.cross(t1);
    let r_km = planet.radius_km;
    for (dy, dx) in [(0i32, 0i32), (0, 1), (0, -1), (1, 0), (-1, 0)] {
        let p = (center + (t1 * (dx as f64 * 0.025) + t2 * (dy as f64 * 0.025)) / r_km)
            .normalize();
        let (f, u, v) = face_from_dir(p);
        match planet.rivers.lake_at(f, u, v, p) {
            Some(h) => println!(
                "({dx:+},{dy:+}) level={:.1}m d_lake={:.2}km r={:.2}km voronoi={} past_b={:.3}km apron_past={:.3}km dam={}",
                h.level_km * 1000.0,
                h.d_lake_km,
                h.radius_km,
                h.in_lake_voronoi,
                h.past_boundary_km,
                h.apron_past_km,
                h.rim_is_dam,
            ),
            None => println!("({dx:+},{dy:+}) no lake hit"),
        }
    }
    Ok(())
}

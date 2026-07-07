//! ASCII map of the sea classification around a lat/lon.
//!   '#' sea-classified   'k' koppen says ocean but NOT sea-classified
//!   '-' land with e_raw <= 0 (undershoot, dry)   '.' plain land
//! Usage: cargo run --release --example seamap -- LAT LON [SPAN_KM] [N]
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::terrain::sample;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lat: f64 = args[1].parse().unwrap();
    let lon: f64 = args[2].parse().unwrap();
    let span_km: f64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(50.0);
    let n: i64 = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(40);
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets).unwrap();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let dir = glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let east = glam::DVec3::Z.cross(dir).normalize();
    let north = dir.cross(east).normalize();
    println!("rows: north {} km (top) .. south; cols: west..east, {} km wide", span_km, span_km);
    for j in (-n..=n).rev() {
        let mut row = String::new();
        for i in -n..=n {
            let (dx, dy) = (
                span_km * i as f64 / n as f64 / 2.0,
                span_km * j as f64 / n as f64 / 2.0,
            );
            let p = (dir * planet.radius_km + east * dx + north * dy).normalize();
            let (face, u, v) = face_from_dir(p);
            let s = sample(&planet, face, u, v, 6);
            let kop = planet.koppen(face, u, v);
            row.push(if s.sea {
                '#'
            } else if kop == 255 {
                'k'
            } else if s.e_raw <= 0.0 {
                '-'
            } else {
                '.'
            });
        }
        println!("{row}");
    }
}

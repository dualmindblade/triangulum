//! Probe the baked rasters and the generator around a lat/lon: elevation,
//! koppen (255 = ocean), roughness, flow, and sample() water/terrain levels
//! at several octave depths. For chasing classification and hydrology bugs.
//! Usage: cargo run --release --example mapprobe -- LAT LON [SPAN_KM] [N]
use triangulum_viewer::planet::{face_from_dir, Planet};
use triangulum_viewer::terrain::sample;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lat: f64 = args[1].parse().unwrap();
    let lon: f64 = args[2].parse().unwrap();
    let span_km: f64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(30.0);
    let n: i64 = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(12);
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets).unwrap();
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let dir = glam::DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    // local tangent frame: scan a span_km x span_km box around the center
    let east = glam::DVec3::Z.cross(dir).normalize();
    let north = dir.cross(east).normalize();
    println!("center face/u/v = {:?}", face_from_dir(dir));
    println!(
        "{:>8} {:>8} | {:>8} {:>4} {:>5} {:>6} {:>5} | {:>8} {:>8} {:>8} {:>8} {:>4}",
        "dE km", "dN km", "e_raw m", "kop", "ofrac", "rough", "flow", "h4 m", "h6 m", "h12 m", "water m", "sea"
    );
    for j in -n..=n {
        for i in -n..=n {
            let (dx, dy) = (
                span_km * i as f64 / n as f64 / 2.0,
                span_km * j as f64 / n as f64 / 2.0,
            );
            let p = (dir * planet.radius_km + east * dx + north * dy).normalize();
            let (face, u, v) = face_from_dir(p);
            let e = planet.elevation(face, u, v) as f64;
            let k = planet.koppen(face, u, v);
            let r = planet.rough(face, u, v);
            let f = planet.flow(face, u, v);
            let s4 = sample(&planet, face, u, v, 4);
            let s6 = sample(&planet, face, u, v, 6);
            let s12 = sample(&planet, face, u, v, 12);
            let w = if s12.water_km > s12.h_km {
                format!("{:8.1}", s12.water_km * 1000.0)
            } else {
                "     dry".into()
            };
            println!(
                "{:8.2} {:8.2} | {:8.1} {:>4} {:5.2} {:6.2} {:5.2} | {:8.1} {:8.1} {:8.1} {} {:>4}",
                dx,
                dy,
                e * 1000.0,
                k,
                planet.water_frac(face, u, v),
                r,
                f,
                s4.h_km * 1000.0,
                s6.h_km * 1000.0,
                s12.h_km * 1000.0,
                w,
                if s12.sea { "SEA" } else { "-" }
            );
        }
        println!();
    }
}

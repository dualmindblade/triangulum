//! Headless lunar chunk-build microbenchmark used by the lattice mission.

use std::sync::Arc;
use std::time::Instant;

use glam::DVec3;
use triangulum_viewer::moon::MoonGenerator;
use triangulum_viewer::orbits::BodyId;
use triangulum_viewer::planet::{Planet, face_from_dir};
use triangulum_viewer::voxel::{
    CHUNK, ChunkKey, Edits, LunarBody, Torches, VoxelBody, build_chunk, column_of_body,
};

fn main() -> anyhow::Result<()> {
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets)?;
    let radius_km = planet.radius_km * 0.27;
    let moon = LunarBody::new(radius_km, Arc::new(MoonGenerator::new(planet.seed)));
    let (lat, lon) = (-36.2094f64.to_radians(), -1.0f64.to_radians());
    let dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
    let (face, u, v) = face_from_dir(dir);
    let (ci, cj) = column_of_body(&moon, u, v);
    let key = ChunkKey {
        body: BodyId::Moon,
        face: face as u8,
        cx: ci / CHUNK,
        cy: cj / CHUNK,
    };
    let edits = Edits::default();
    let torches = Torches::default();

    // One warm build removes allocator/page-fault noise from the timed patch.
    let warm = build_chunk(&moon, &edits, &torches, key, 1.0);
    std::hint::black_box(warm);
    let mut samples = Vec::with_capacity(5);
    for _ in 0..5 {
        let start = Instant::now();
        let mesh = build_chunk(&moon, &edits, &torches, key, 1.0);
        std::hint::black_box(mesh);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let avg = samples.iter().sum::<f64>() / samples.len() as f64;
    let p95 = samples[((samples.len() as f64 * 0.95) as usize).min(samples.len() - 1)];
    println!(
        "moon chunk {:?} ({} columns/face): avg {avg:.2} ms p95 {p95:.2} ms min {:.2} max {:.2}",
        key,
        moon.columns_per_face(),
        samples[0],
        samples[samples.len() - 1]
    );
    Ok(())
}

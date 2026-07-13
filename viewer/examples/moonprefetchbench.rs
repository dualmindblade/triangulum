//! Scalar-vs-prefetched lunar sampler microbenchmark for tile footprints.

use std::hint::black_box;
use std::time::Instant;

use triangulum_viewer::moon::MoonGenerator;
use triangulum_viewer::planet::face_dir;

fn directions(size: f64) -> Vec<glam::DVec3> {
    let mut out = Vec::with_capacity(35 * 35);
    for j in 0..35 {
        for i in 0..35 {
            let u = -0.5 * size + size * i as f64 / 34.0;
            let v = -0.5 * size + size * j as f64 / 34.0;
            out.push(face_dir(0, u, v));
        }
    }
    out
}

fn main() {
    let moon = MoonGenerator::new(42);
    for size in [2.0, 0.125, 0.0078125] {
        let dirs = directions(size);
        let start = Instant::now();
        let scalar: Vec<_> = dirs.iter().map(|&d| moon.sample(d)).collect();
        let scalar_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let batch = moon.sample_batch(&dirs);
        let batch_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(scalar, batch);
        black_box(batch);
        println!("tile uv size {size}: scalar {scalar_ms:.3} ms batch {batch_ms:.3} ms");
    }
}

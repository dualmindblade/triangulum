//! Reproducible wall-clock probe for the mesh tile builder.
//!
//! Uses the sync-diff `lake_shore` camera and the renderer's selection
//! settings, then builds that fixed key set with the same Rayon parallelism
//! as `Renderer::draw`. Asset loading, key selection, and one warm-up pass are
//! outside the timed region.

use glam::DVec3;
use rayon::prelude::*;
use std::time::{Duration, Instant};
use triangulum_viewer::planet::{Planet, face_from_dir};
use triangulum_viewer::terrain::{TileKey, VOXEL_OCTAVES, build_tile, sample, select_tiles};

const ERR_TARGET: f64 = 0.35;
const VOXEL_MAX_ALT_KM: f64 = 2.5;
const LAT_DEG: f64 = 13.346;
const LON_DEG: f64 = -4.806;
const ALT_KM: f64 = 0.443;

fn camera_dir() -> DVec3 {
    let lat = LAT_DEG.to_radians();
    let lon = LON_DEG.to_radians();
    DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
}

fn add_bits(sum: u64, values: impl IntoIterator<Item = f32>) -> u64 {
    values
        .into_iter()
        .fold(sum, |sum, value| sum.wrapping_add(value.to_bits() as u64))
}

fn build_set(planet: &Planet, keys: &[TileKey]) -> (usize, u64, u64, u64) {
    keys.par_iter()
        .map(|&key| {
            let mesh = build_tile(planet, key, 1.0);
            let (geometry_bits, other_bits, shore_bits) = mesh.vertices.iter().fold(
                (0u64, 0u64, 0u64),
                |(geometry, other, shore), vertex| {
                    let geometry = add_bits(
                        geometry,
                        vertex
                            .pos
                            .into_iter()
                            .chain(vertex.normal)
                            .chain([vertex.morph_dh]),
                    );
                    let other = add_bits(
                        other,
                        vertex
                            .color
                            .into_iter()
                            .chain(vertex.far_color_delta.map(f32::from))
                            .chain(vertex.water)
                            .chain([vertex.morph_wet, vertex.wflag]),
                    );
                    (
                        geometry,
                        other,
                        shore.wrapping_add(vertex.shore.to_bits() as u64),
                    )
                },
            );
            (mesh.vertices.len(), geometry_bits, other_bits, shore_bits)
        })
        .reduce(
            || (0, 0, 0, 0),
            |a, b| {
                (
                    a.0 + b.0,
                    a.1.wrapping_add(b.1),
                    a.2.wrapping_add(b.2),
                    a.3.wrapping_add(b.3),
                )
            },
        )
}

fn main() -> anyhow::Result<()> {
    let assets = if std::path::Path::new("assets/meta.json").exists() {
        "assets"
    } else {
        "viewer/assets"
    };
    let planet = Planet::load(assets)?;
    let dir = camera_dir();
    let (face, u, v) = face_from_dir(dir);
    let ground_km = sample(&planet, face, u, v, VOXEL_OCTAVES).render_h_km();
    let cam_pos = dir * (planet.radius_km + ground_km + ALT_KM);
    let voxel_radius_km = (200.0 + (VOXEL_MAX_ALT_KM - ALT_KM).max(0.0) * 120.0) / 1000.0;
    let keys = select_tiles(
        cam_pos,
        planet.radius_km,
        ERR_TARGET,
        Some((dir, voxel_radius_km + 0.2)),
    );
    let deep = keys.iter().filter(|key| key.deep).count();
    println!(
        "lake_shore fixed set: {} tiles ({} deep, {} spacing-capped)",
        keys.len(),
        deep,
        keys.len() - deep
    );

    let warm = build_set(&planet, &keys);
    let mut elapsed = Vec::new();
    let mut check = warm;
    for run in 1..=5 {
        let start = Instant::now();
        check = build_set(&planet, &keys);
        let dt = start.elapsed();
        println!("run {run}: {:.3} s", dt.as_secs_f64());
        elapsed.push(dt);
    }
    elapsed.sort_unstable();
    let median: Duration = elapsed[elapsed.len() / 2];
    println!(
        "median: {:.3} s; checksums: vertices={} geometry={} other={} shore={}",
        median.as_secs_f64(),
        check.0,
        check.1,
        check.2,
        check.3
    );
    Ok(())
}

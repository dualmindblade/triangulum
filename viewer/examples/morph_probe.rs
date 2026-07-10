//! Measure the fully-morphed child mesh against the parent triangle surface.

use glam::DVec3;
use triangulum_viewer::planet::{Planet, face_from_dir};
use triangulum_viewer::terrain::{TILE_QUADS, TileKey, TileMesh, build_tile};

fn world(mesh: &TileMesh, i: usize, j: usize) -> DVec3 {
    let v = mesh.vertices[j * (TILE_QUADS + 1) + i];
    mesh.origin_km + DVec3::from_array(v.pos.map(f64::from))
}

fn parent_surface(parent: &TileMesh, child: TileKey, i: usize, j: usize) -> DVec3 {
    let base_i = (child.ix as usize & 1) * (TILE_QUADS / 2);
    let base_j = (child.iy as usize & 1) * (TILE_QUADS / 2);
    let pi = base_i + i / 2;
    let pj = base_j + j / 2;
    let x = (i & 1) as f64 * 0.5;
    let y = (j & 1) as f64 * 0.5;
    if x + y <= 1.0 {
        world(parent, pi, pj) * (1.0 - x - y)
            + world(parent, pi + 1, pj) * x
            + world(parent, pi, pj + 1) * y
    } else {
        world(parent, pi + 1, pj) * (1.0 - y)
            + world(parent, pi + 1, pj + 1) * (x + y - 1.0)
            + world(parent, pi, pj + 1) * (1.0 - x)
    }
}

fn parent_wet(parent: &TileMesh, child: TileKey, i: usize, j: usize) -> f64 {
    let base_i = (child.ix as usize & 1) * (TILE_QUADS / 2);
    let base_j = (child.iy as usize & 1) * (TILE_QUADS / 2);
    let pi = base_i + i / 2;
    let pj = base_j + j / 2;
    let x = (i & 1) as f64 * 0.5;
    let y = (j & 1) as f64 * 0.5;
    let wet =
        |ii: usize, jj: usize| f64::from(parent.vertices[jj * (TILE_QUADS + 1) + ii].water[3]);
    if x + y <= 1.0 {
        wet(pi, pj) * (1.0 - x - y) + wet(pi + 1, pj) * x + wet(pi, pj + 1) * y
    } else {
        wet(pi + 1, pj) * (1.0 - y)
            + wet(pi + 1, pj + 1) * (x + y - 1.0)
            + wet(pi, pj + 1) * (1.0 - x)
    }
}

fn probe(planet: &Planet, key: TileKey) {
    let child = build_tile(planet, key, 1.0);
    let parent_key = TileKey {
        face: key.face,
        level: key.level - 1,
        ix: key.ix / 2,
        iy: key.iy / 2,
        deep: false,
    };
    let parent = build_tile(planet, parent_key, 1.0);
    let mut max_m = 0.0f64;
    let mut exact_max_m = 0.0f64;
    let mut wet_max = 0.0f64;
    let mut at = (0, 0);
    for j in 0..=TILE_QUADS {
        for i in 0..=TILE_QUADS {
            let v = child.vertices[j * (TILE_QUADS + 1) + i];
            let p = world(&child, i, j);
            let morphed = p + p.normalize() * f64::from(v.morph_dh);
            let target = parent_surface(&parent, key, i, j);
            let residual_m = morphed.distance(target) * 1000.0;
            if residual_m > max_m {
                max_m = residual_m;
                at = (i, j);
            }
            let target_local = target - child.origin_km;
            let delta = (target_local - DVec3::from_array(v.pos.map(f64::from))).as_vec3();
            let exact_local = DVec3::new(
                f64::from(v.pos[0] + delta.x),
                f64::from(v.pos[1] + delta.y),
                f64::from(v.pos[2] + delta.z),
            );
            exact_max_m =
                exact_max_m.max((child.origin_km + exact_local).distance(target) * 1000.0);
            wet_max = wet_max.max((f64::from(v.morph_wet) - parent_wet(&parent, key, i, j)).abs());
        }
    }
    println!(
        "{key:?}: max residual {max_m:.6} m at {at:?}; exact-vec3 simulation {exact_max_m:.6} m; wet residual {wet_max:.6}"
    );
}

fn key_at(lat: f64, lon: f64, level: u8) -> TileKey {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    let dir = DVec3::new(la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin());
    let (face, u, v) = face_from_dir(dir);
    let side = (1u32 << level) as f64;
    TileKey {
        face: face as u8,
        level,
        ix: (((u + 1.0) * 0.5 * side).floor() as u16).min((side - 1.0) as u16),
        iy: (((v + 1.0) * 0.5 * side).floor() as u16).min((side - 1.0) as u16),
        deep: false,
    }
}

fn main() {
    let assets = if std::path::Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets"
    } else {
        "assets"
    };
    let planet = Planet::load(assets).unwrap();
    probe(
        &planet,
        TileKey {
            face: 4,
            level: 9,
            ix: 339,
            iy: 308,
            deep: false,
        },
    );
    probe(&planet, key_at(4.990, -29.403, 9));
}

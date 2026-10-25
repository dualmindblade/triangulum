use super::*;
use std::sync::OnceLock;

const BAKE_DIR: &str = "C:/code/triangulum/game/assets/world/seed42";

fn shared_bake() -> Arc<dyn Bake> {
    static BAKE: OnceLock<Arc<bake::RasterBake>> = OnceLock::new();
    BAKE.get_or_init(|| Arc::new(bake::open(BAKE_DIR).expect("world bake"))).clone()
}

fn registry() -> Registry {
    Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap()
}

/// A generator at vertical scale `vscale` (None = Earth slopes) with world
/// height `h` and sea level `sea`, 1 m blocks.
fn generator_with(vscale: Option<f64>, h: u32, sea: u32) -> Generator {
    let planet = Planet::new(1.0, h, sea, 131072);
    let vs = vscale.unwrap_or(planet.radius / 6_371_000.0);
    Generator::new(planet, shared_bake(), 42, vs, 1.0, 1.0, &registry()).unwrap()
}

pub fn test_generator() -> Generator {
    generator_with(None, 2048, 512)
}

fn key_at(g: &Generator, dir: DVec3, lod: u8, height_m: f64) -> ChunkKey {
    let p = &g.planet;
    let (f, i, j) = p.lattice_of_dir(dir);
    let e = (CHUNK as f64) * (1u64 << lod) as f64;
    let k = p.sea_level as f64 + height_m / p.block_size;
    ChunkKey::neisor(f, lod, (i / e) as u32, (j / e) as u32, (k / e).max(0.0) as u32)
}

/// `dirs` for sibling test modules.
pub fn dirs_pub(n: usize, salt: u64) -> Vec<DVec3> {
    dirs(n, salt)
}

/// Deterministic pseudo-random unit directions.
fn dirs(n: usize, salt: u64) -> Vec<DVec3> {
    (0..n as i64)
        .map(|k| {
            let z = noise::hash01(k, 3, 9, salt) * 2.0 - 1.0;
            let a = noise::hash01(k, 4, 9, salt) * std::f64::consts::TAU;
            let s = (1.0 - z * z).sqrt();
            DVec3::new(s * a.cos(), s * a.sin(), z)
        })
        .collect()
}

/// Places that stress the landform: the highest range, the steepest coast,
/// the hardest quiet rock (mesas), plus random land.
fn stress_dirs(g: &Generator) -> Vec<DVec3> {
    let places = inspect::find_land_places(g.bake.as_ref());
    let mut v = vec![places.peak, places.coast, places.river, places.plain];
    let mut mesa = (DVec3::Z, f64::MAX);
    for d in dirs(4000, 77) {
        let s = g.bake.sample(d);
        if s.ocean < 0.5 && s.uplift < 0.05 && s.erodibility > 0.0 && s.erodibility < mesa.1 {
            mesa = (d, s.erodibility);
        }
    }
    v.push(mesa.0);
    v.extend(dirs(40, 5).into_iter().filter(|d| g.bake.sample(*d).ocean < 0.5).take(20));
    v
}

/// Not a check: per-part costs of a column and a chunk (run with --ignored --nocapture in release).
#[test]
#[ignore]
fn profile_column_parts() {
    use std::hint::black_box;
    use std::time::Instant;
    let g = test_generator();
    let hills = crate::engine::gnomonic_dir(crate::engine::HILLS.0, crate::engine::HILLS.1, crate::engine::HILLS.2);
    let p = g.planet;
    let (f, i0, j0) = p.lattice_of_dir(hills);
    let dirs: Vec<DVec3> = (0..200_000).map(|k| p.dir(f, i0 + (k % 447) as f64 * 1.37, j0 + (k / 447) as f64 * 1.37)).collect();
    let t = Instant::now();
    for d in &dirs {
        black_box(g.bake.sample(*d));
    }
    let bake_ns = t.elapsed().as_nanos() as f64 / dirs.len() as f64;
    let samples: Vec<_> = dirs.iter().map(|d| g.bake.sample(*d)).collect();
    let t = Instant::now();
    for (d, s) in dirs.iter().zip(&samples) {
        black_box(g.lf.sample(*d, s, 1.0, 0.0));
    }
    let lf_ns = t.elapsed().as_nanos() as f64 / dirs.len() as f64;
    let t = Instant::now();
    for d in &dirs {
        black_box(g.strata.column_offset_m(*d));
    }
    let strata_ns = t.elapsed().as_nanos() as f64 / dirs.len() as f64;
    let t = Instant::now();
    for d in &dirs {
        black_box(g.column(*d, 1.0));
    }
    let col_ns = t.elapsed().as_nanos() as f64 / dirs.len() as f64;
    let t = Instant::now();
    for (k, _) in dirs.iter().enumerate().take(200) {
        black_box(p.dir(f, i0 + k as f64, j0));
    }
    let key = key_at(&g, hills, 0, g.ground_height_m(hills));
    let t2 = Instant::now();
    for _ in 0..20 {
        black_box(g.chunk_columns(key));
    }
    let cols_us = t2.elapsed().as_micros() as f64 / 20.0;
    let t3 = Instant::now();
    for _ in 0..20 {
        black_box(g.generate(key));
    }
    let gen_us = t3.elapsed().as_micros() as f64 / 20.0;
    let _ = t;
    for lod in [1u8, 2, 3, 4, 5] {
        let k = key_at(&g, hills, lod, g.ground_height_m(hills));
        let t5 = Instant::now();
        for _ in 0..10 {
            black_box(g.chunk_columns(k));
        }
        let c_us = t5.elapsed().as_micros() as f64 / 10.0;
        let t6 = Instant::now();
        for _ in 0..10 {
            black_box(g.generate(k));
        }
        println!("LOD {lod}: chunk columns {c_us:.0} us, generate {:.0} us", t6.elapsed().as_micros() as f64 / 10.0);
    }
    let t4 = Instant::now();
    for k in 0..200u32 {
        black_box(g.footprint_height_range_m(crate::grid::FootprintKey { body: crate::grid::BODY_NEISOR, face: key.face, lod: (k % 6) as u8, cx: (key.cx >> (k % 6)) + k / 6, cy: key.cy >> (k % 6) }));
    }
    println!("footprint_height_range_m {:.0} us", t4.elapsed().as_micros() as f64 / 200.0);
    println!("bake.sample {bake_ns:.0} ns, landform {lf_ns:.0} ns, strata offset {strata_ns:.0} ns, column {col_ns:.0} ns; chunk columns {cols_us:.0} us, generate {gen_us:.0} us");
}

/// Chunks evaluate the landform's long gully octaves on a grid and
/// interpolate; the result must match the direct per-column evaluation
/// closely at every LOD that uses the grid, at 1/70 and 1/8. Mesa columns
/// are left out of the worst case: their terraces are near-steps of the
/// height, so a centimetre of interpolation can move a riser by a column.
#[test]
fn chunk_grid_matches_direct_columns() {
    for vs in [None, Some(0.125)] {
        let g = generator_with(vs, 4096, 1536);
        let p = g.planet;
        let mut used_grid = 0;
        let places = stress_dirs(&g);
        for lod in 0..=5u8 {
            let vm = p.voxel_m(lod);
            let mut errs = Vec::new();
            let mut worst_plain: f64 = 0.0;
            for d in places.iter().take(14) {
                let key = key_at(&g, *d, lod, 0.0);
                let fast = g.chunk_columns(key);
                let vb = key.voxel_blocks() as f64;
                let (i0, j0, _) = key.origin_blocks();
                let mut any = false;
                for x in -1..=CHUNK as i32 {
                    for y in -1..=CHUNK as i32 {
                        let dir = p.dir(key.face, i0 as f64 + (x as f64 + 0.5) * vb, j0 as f64 + (y as f64 + 0.5) * vb);
                        let c = g.column(dir, vm);
                        let e = (fast[((x + 1) as usize) * PAD + (y + 1) as usize].height_m - c.height_m).abs() as f64;
                        any |= e > 0.0;
                        errs.push(e);
                        if g.lf.local(&g.bake.sample(dir)).mesa <= 0.0 {
                            worst_plain = worst_plain.max(e);
                        }
                    }
                }
                used_grid += any as usize;
            }
            errs.sort_by(|a, b| a.total_cmp(b));
            let mean = errs.iter().sum::<f64>() / errs.len() as f64;
            let p99 = errs[(errs.len() - 1) * 99 / 100];
            println!("vscale {vs:?} LOD {lod} ({vm} m voxels): grid vs direct mean {mean:.4} m, p99 {p99:.3} m, worst off mesas {worst_plain:.3} m, worst {:.3} m", errs[errs.len() - 1]);
            assert!(mean < 0.05 * vm, "vscale {vs:?} LOD {lod}: mean {mean:.4} m");
            assert!(p99 < 0.5 * vm, "vscale {vs:?} LOD {lod}: p99 {p99:.3} m");
            assert!(worst_plain < 1.0 * vm, "vscale {vs:?} LOD {lod}: worst off mesas {worst_plain:.3} m");
        }
        assert!(used_grid > 20, "the grid path should be taken ({used_grid})");
    }
}

/// No seam at cube edges (the landform's charts cross-fade there, and the
/// bake's per-face rasters meet): walking across a face edge at 1 m steps,
/// the step between the two columns that straddle the edge is no larger
/// than the steps beside it on the same walk. Every side of every face (so
/// each of the 12 edges from both faces), 4 walks per side, at 1/8 where
/// relief is largest; each walk is checked to really change face.
#[test]
fn no_seam_across_cube_edges() {
    let g = generator_with(Some(0.125), 4096, 1536);
    let p = g.planet;
    let n = p.n as f64;
    let mut worst_ratio: f64 = 0.0;
    for face in 0..6u8 {
        for side in 0..4u8 {
            for &tf in &[0.13, 0.37, 0.61, 0.83] {
                let t = (n * tf).floor() + 0.5;
                // Column k of the walk: 60 columns inside the face, then 60 past the side.
                let at = |k: i32| -> (f64, f64) {
                    let d = k as f64 - 59.5; // < 0 inside, > 0 past the edge
                    match side {
                        0 => (-d, t),
                        1 => (n + d, t),
                        2 => (t, -d),
                        _ => (t, n + d),
                    }
                };
                let (i_in, j_in) = at(59);
                let (i_out, j_out) = at(60);
                assert_eq!(p.wrap(face, i_in, j_in).0, face);
                assert_ne!(p.wrap(face, i_out, j_out).0, face, "the walk must cross onto the neighbour");
                let h: Vec<f64> = (0..120)
                    .map(|k| {
                        let (i, j) = at(k);
                        g.column(p.dir(face, i, j), 1.0).height_m as f64
                    })
                    .collect();
                let steps: Vec<f64> = h.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
                let at_edge = steps[59];
                let beside = steps.iter().enumerate().filter(|(k, _)| (*k as i32 - 59).abs() > 2).map(|(_, v)| *v).fold(0.0, f64::max);
                worst_ratio = worst_ratio.max(at_edge / (beside + 0.25));
                assert!(at_edge <= 1.5 * beside + 0.25, "seam on face {face} side {side} at {t}: {at_edge:.2} m step at the edge, at most {beside:.2} m beside it");
            }
        }
    }
    println!("cube edges (96 walks): worst (step at the edge) / (largest step beside it + 0.25 m) = {worst_ratio:.2}");
}

#[test]
fn determinism_across_runs_and_thread_counts() {
    let g = test_generator();
    let keys = [
        ChunkKey::neisor(2, 0, 3295, 1813, 16),
        ChunkKey::neisor(2, 3, 400, 220, 2),
        ChunkKey::neisor(0, 9, 3, 5, 0),
    ];
    let once: Vec<Vec<u8>> = keys.iter().map(|k| g.generate(*k).chunk.to_bytes()).collect();
    for threads in [1usize, 8] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let again: Vec<Vec<u8>> = pool.install(|| {
            use rayon::prelude::*;
            keys.par_iter().map(|k| g.generate(*k).chunk.to_bytes()).collect()
        });
        assert_eq!(once, again, "chunk bytes differ with {threads} threads");
    }
    let ids_a = g.generate(keys[0]).ids;
    let ids_b = g.generate(keys[0]).ids;
    assert_eq!(ids_a, ids_b);
}

#[test]
fn surface_chunk_has_air_above_and_rock_below() {
    let g = test_generator();
    let dir = cubesphere::face_to_dir(g.planet.proj, 2, 0.5, -0.1);
    let (f, i, j) = g.planet.lattice_of_dir(dir);
    let h = g.ground_height_m(dir);
    let k = g.planet.sea_level as f64 + h;
    let key = ChunkKey::neisor(f, 0, i as u32 / 32, j as u32 / 32, (k as u32) / 32);
    let generated = g.generate(key);
    let top = generated.ids[pidx(0, 0, 32)];
    let bottom = generated.ids[pidx(0, 0, -1)];
    assert_eq!(top, g.blocks.air);
    assert!(bottom != g.blocks.air && bottom != g.blocks.water, "bottom {bottom}");
    assert!(!generated.chunk.is_uniform());
}

/// The height function is a pure function of (seed, direction, voxel size):
/// bit-identical across calls and across generators built separately, and
/// it changes with the seed.
#[test]
fn landform_height_is_deterministic() {
    let a = test_generator();
    let b = test_generator();
    let planet = Planet::new(1.0, 2048, 512, 131072);
    let other = Generator::new(planet, shared_bake(), 43, a.vscale, 1.0, 1.0, &registry()).unwrap();
    let mut differs = 0;
    for d in stress_dirs(&a) {
        for v in [1.0, 4.0, 64.0] {
            let h1 = a.column(d, v).height_m;
            assert_eq!(h1.to_bits(), a.column(d, v).height_m.to_bits());
            assert_eq!(h1.to_bits(), b.column(d, v).height_m.to_bits());
            if other.column(d, v).height_m != h1 {
                differs += 1;
            }
        }
    }
    assert!(differs > 50, "the seed must matter ({differs} of 75 differ)");
}

/// LOD consistency: a coarse voxel's column height agrees with the mean of
/// the full-detail columns it covers (no bias, small scatter), and adjacent
/// LODs differ little at the same point (no popping). Checked at 1/70 and
/// 1/8 on mountains, coasts, mesas and random land.
#[test]
fn landform_height_is_lod_consistent() {
    for vs in [None, Some(0.125)] {
        let g = generator_with(vs, 4096, 1536);
        let p = g.planet;
        let pts = stress_dirs(&g);
        for lod in 1..=6u8 {
            let vb = (1u64 << lod) as f64;
            let vm = p.voxel_m(lod);
            let (mut sum, mut sum_abs, mut worst, mut pop) = (0.0, 0.0, 0.0f64, 0.0f64);
            for d in &pts {
                let (f, i, j) = p.lattice_of_dir(*d);
                let (ci, cj) = ((i / vb).floor() * vb, (j / vb).floor() * vb);
                let centre = p.dir(f, ci + 0.5 * vb, cj + 0.5 * vb);
                let coarse = g.column(centre, vm).height_m as f64;
                let n = (1u64 << lod).min(8) as usize;
                let st = vb / n as f64;
                let mut fine = 0.0;
                for a in 0..n {
                    for b in 0..n {
                        fine += g.column(p.dir(f, ci + (a as f64 + 0.5) * st, cj + (b as f64 + 0.5) * st), p.block_size).height_m as f64;
                    }
                }
                let e = coarse - fine / (n * n) as f64;
                sum += e;
                sum_abs += e.abs();
                worst = worst.max(e.abs());
                // Popping: the same point one LOD finer.
                pop = pop.max((coarse - g.column(centre, vm * 0.5).height_m as f64).abs());
            }
            let n = pts.len() as f64;
            let (bias, mean_abs) = (sum / n / vm, sum_abs / n / vm);
            assert!(bias.abs() < 0.25, "vscale {vs:?} LOD {lod}: bias {bias:.3} voxels");
            assert!(mean_abs < 0.5, "vscale {vs:?} LOD {lod}: mean |coarse - fine mean| {mean_abs:.3} voxels");
            assert!(worst < 2.5 * vm, "vscale {vs:?} LOD {lod}: worst {worst:.2} m against {vm} m voxels");
            assert!(pop < 2.0 * vm, "vscale {vs:?} LOD {lod}: popping {pop:.2} m against {vm} m voxels");
        }
    }
}

/// Large-scale fidelity: over 12 km windows the landform averages out to
/// the bake (mean offset small against the bake's own relief).
#[test]
fn landform_averages_out_over_kilometres() {
    let g = generator_with(Some(0.125), 4096, 1536);
    let mut worst: f64 = 0.0;
    for (k, d) in stress_dirs(&g).into_iter().take(12).enumerate() {
        let (h, _, b) = inspect::sample_grid(&g, d, 40, 300.0, 1.0);
        let n = h.len() as f64;
        let diff = h.iter().zip(&b).map(|(a, c)| (*a - *c) as f64).sum::<f64>() / n;
        println!("place {k}: 12 km window mean offset {diff:.2} m");
        worst = worst.max(diff.abs());
    }
    assert!(worst < 20.0, "12 km window mean offset {worst:.1} m at 1/8");
}

/// Coordinator finding 1: at coarse LODs a voxel's centre can sit below the
/// sea floor; the ocean must still be there. Every ocean column of an
/// L5..L9 chunk at sea level has water (or ice) in the voxel holding sea
/// level; opaque deep ocean where that voxel is the world's bottom voxel
/// (L9 here, review 2 #12).
#[test]
fn coarse_lods_keep_the_ocean() {
    let g = test_generator();
    let p = g.planet;
    // Shallow and deep ocean: the steepest coast's offshore side, and random sea.
    let mut seas: Vec<DVec3> = dirs(400, 11).into_iter().filter(|d| g.bake.sample(*d).ocean > 0.99).take(6).collect();
    seas.push(crate::engine::find_places(g.bake.as_ref()).trench.0);
    let mut checked = 0;
    for d in seas {
        for lod in 5..=p.max_lod {
            let key = key_at(&g, d, lod, -0.5);
            let out = g.generate(key);
            let cols = g.chunk_columns(key);
            let vb = key.voxel_blocks() as i64;
            let k0 = key.origin_blocks().2 as i64;
            // The voxel holding layer sea_level - 1.
            let z = ((p.sea_level as i64 - 1 - k0).div_euclid(vb)) as i32;
            assert!((0..CHUNK as i32).contains(&z));
            let sea = if k0 + z as i64 * vb == 0 { g.blocks.deep_ocean } else { g.blocks.water };
            assert_eq!(sea == g.blocks.deep_ocean, lod == p.max_lod, "only the L9 sea voxel is the bottom voxel");
            for x in 0..CHUNK as i32 {
                for y in 0..CHUNK as i32 {
                    if cols[((x + 1) as usize) * PAD + (y + 1) as usize].ocean {
                        let id = out.ids[pidx(x, y, z)];
                        assert!(id == sea || id == g.blocks.ice, "LOD {lod}: ocean column ({x},{y}) has block {id} at sea level");
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 10_000, "only {checked} ocean columns checked");
}

/// Coordinator finding 2: a chunk classified Enclosed still generates its
/// full contents (strata, bedrock), equal to the per-block result computed
/// one voxel at a time, and markers elsewhere do not change it.
#[test]
fn enclosed_chunks_hold_the_full_per_block_result() {
    let mut g = test_generator();
    let hills = crate::engine::gnomonic_dir(crate::engine::HILLS.0, crate::engine::HILLS.1, crate::engine::HILLS.2);
    for (lod, depth) in [(0u8, 120.0), (2, 400.0)] {
        let key = key_at(&g, hills, lod, g.ground_height_m(hills) - depth);
        assert_eq!(g.classify(key), ChunkClass::Enclosed, "LOD {lod} chunk {depth} m down");
        let out = g.generate(key);
        assert!(!out.chunk.is_uniform(), "an enclosed chunk shows strata, not uniform stone");
        let p = g.planet;
        let vb = key.voxel_blocks() as f64;
        let (_i0, _j0, k0) = key.origin_blocks();
        let cols = g.chunk_columns(key);
        let sea = p.sea_level as f64 * p.block_size;
        for x in 0..CHUNK as i32 {
            for y in 0..CHUNK as i32 {
                // The chunk's own column samples (its strata offset is interpolated
                // from the chunk grid), each voxel worked out on its own.
                let c = cols[((x + 1) as usize) * PAD + (y + 1) as usize];
                for z in 0..CHUNK as i32 {
                    let k_lo = k0 as f64 + z as f64 * vb;
                    let centre = (k_lo + 0.5 * vb) * p.block_size - sea;
                    assert!(c.height_m as f64 - centre > p.voxel_m(lod) + 4.0, "voxel within the soil");
                    let want = if k_lo == 0.0 && lod == 0 { g.blocks.bedrock } else { g.strata.rock_at(centre, c.strata_off as f64, c.geo) };
                    assert_eq!(out.ids[pidx(x, y, z)], want, "LOD {lod} voxel ({x},{y},{z})");
                }
            }
        }
        // A marker far away must not change this chunk.
        let before = out.chunk.to_bytes();
        g.markers = vec![Marker { dir: g.planet.dir(key.face, 1000.5, 1000.5), height_blocks: 100, width_blocks: 4, block: g.blocks.marker_red, below: false }];
        assert_eq!(g.generate(key).chunk.to_bytes(), before);
        g.markers.clear();
    }
}

/// Coordinator finding 3: layer 0 is bedrock even where the terrain lies
/// below the world bottom (1/8 at H 2048, sea 1024: the trench is ~-1377 m
/// against a -1024 m floor), and coarse bottom voxels stay solid.
#[test]
fn bedrock_floor_at_layer_zero() {
    let g = generator_with(Some(0.125), 2048, 1024);
    let trench = crate::engine::find_places(g.bake.as_ref()).trench.0;
    assert!(g.column(trench, 1.0).height_m < -1024.0, "the trench should lie below the world bottom here");
    for lod in [0u8, 1, 3, 6] {
        let key = key_at(&g, trench, lod, -1024.0);
        assert_eq!(key.cz, 0);
        let out = g.generate(key);
        for x in 0..CHUNK as i32 {
            for y in 0..CHUNK as i32 {
                let id = out.ids[pidx(x, y, 0)];
                if lod == 0 {
                    assert_eq!(id, g.blocks.bedrock, "LOD 0 layer 0 at ({x},{y})");
                } else {
                    assert!(id != g.blocks.air && id != g.blocks.water, "LOD {lod} bottom voxel at ({x},{y}) is {id}");
                }
            }
        }
        if lod == 0 {
            // Above the floor the trench is water.
            assert_eq!(out.ids[pidx(5, 5, 1)], g.blocks.water);
        }
    }
    // Ordinary land: layer 0 is bedrock too.
    let hills = crate::engine::gnomonic_dir(crate::engine::HILLS.0, crate::engine::HILLS.1, crate::engine::HILLS.2);
    let out = g.generate(key_at(&g, hills, 0, -1024.0));
    assert!((0..CHUNK as i32).all(|x| out.ids[pidx(x, 7, 0)] == g.blocks.bedrock));
}

/// Coordinator finding 4: the footprint bounds contain every column a chunk
/// samples (border included), water surface included, at every LOD, at
/// face edges and corners, at 1/70 and at 1/8 (mountains, steep coast,
/// mesas); and classification agrees with the generated contents.
#[test]
fn footprint_bounds_contain_every_sampled_column() {
    for vs in [None, Some(0.125)] {
        let g = generator_with(vs, 4096, 1536);
        let p = g.planet;
        let mut keys = Vec::new();
        for (n, d) in stress_dirs(&g).into_iter().enumerate() {
            for lod in [0u8, 1, 2, 4, 6, 8, 9] {
                keys.push(key_at(&g, d, lod, g.ground_height_m(d) * if n % 2 == 0 { 1.0 } else { 0.0 }));
            }
        }
        // 300 random footprints anywhere (land and sea), at random LODs.
        for (k, d) in dirs(300, 0xB0D5 + vs.is_some() as u64).into_iter().enumerate() {
            keys.push(key_at(&g, d, (k % 10) as u8, 0.0));
        }
        // Face edges and corners.
        for face in 0..6u8 {
            for lod in [0u8, 3, 7, 9] {
                let last = p.chunks_per_edge(lod) - 1;
                for (cx, cy) in [(0, last / 2), (last, last / 3), (0, 0), (last, last)] {
                    keys.push(ChunkKey::neisor(face, lod, cx, cy, 0));
                }
            }
        }
        let mut worst_excess: f64 = f64::MIN;
        for key in keys {
            let (hmin, hmax) = g.footprint_height_range_m(key.footprint());
            let cols = g.chunk_columns(key);
            for c in &cols {
                let h = c.height_m as f64;
                let top = if c.ocean { h.max(0.0) } else { h };
                assert!(h >= hmin && top <= hmax, "vscale {vs:?} {key:?}: column {h:.2} (top {top:.2}) outside [{hmin:.2}, {hmax:.2}]");
                // The sea surface (0) and the land floor are exact bounds; measure the
                // landform's margin on the other columns.
                if !c.ocean && h > 2.0 {
                    worst_excess = worst_excess.max((hmin - h) / (hmax - hmin).max(1.0)).max((h - hmax) / (hmax - hmin).max(1.0));
                }
            }
            // Classification against the contents, at a few heights through the band.
            let e = key.extent_blocks() as f64 * p.block_size;
            let sea = p.sea_level as f64 * p.block_size;
            let cz_lo = ((hmin + sea) / e).floor().max(1.0) - 1.0;
            for cz in [cz_lo, cz_lo + 1.0, ((hmax + sea) / e).floor() + 1.0] {
                if cz < 0.0 || cz >= p.chunks_per_column(key.lod) as f64 {
                    continue;
                }
                let k = ChunkKey { cz: cz as u32, ..key };
                let class = g.classify(k);
                if class == ChunkClass::Mixed {
                    continue;
                }
                let ids = g.generate(k).ids;
                match class {
                    ChunkClass::Air => assert!(ids.iter().all(|&b| b == g.blocks.air), "{k:?} classified Air holds blocks"),
                    ChunkClass::Enclosed => assert!(ids.iter().all(|&b| b != g.blocks.air && b != g.blocks.water), "{k:?} classified Enclosed holds air or water"),
                    ChunkClass::Mixed => {}
                }
            }
        }
        println!("vscale {vs:?}: worst approach to a bound {worst_excess:.3} of the band (negative = inside)");
    }
}

/// Upward-facing opaque area of a meshed chunk, in voxel faces.
fn opaque_up_area(m: &crate::mesh::MeshData) -> u32 {
    m.opaque
        .iter()
        .filter(|q| q[0] & 3 == 2 && (q[0] >> 2) & 1 == 1)
        .map(|q| (((q[0] >> 18) & 31) + 1) * (((q[0] >> 23) & 31) + 1))
        .sum()
}

/// Review 2 #12: the planet is opaque at every LOD. Under the voxel holding
/// sea level in every ocean column there is a solid voxel inside the world
/// (at the coarsest LOD the sea voxel is the world's bottom voxel and is
/// itself opaque deep ocean), and the coarsest chunk, which holds the whole
/// world height, meshes to exactly one upward opaque face per column, so no
/// sky can show through the sea from orbit, plus a side wall wherever deep
/// ocean meets the chunk's border (closing the view under it from a finer
/// LOD's translucent water next door). At 1/70 (H 2048, sea 512: the
/// coarsest LOD is L9) and 1/8 (H 2048, sea 1024: L10), open sea and the
/// deepest trench (below the world floor at 1/8).
#[test]
fn every_ocean_column_is_opaque_underneath() {
    let reg = registry();
    for (vs, h, sea) in [(None, 2048, 512), (Some(0.125), 2048, 1024)] {
        let g = generator_with(vs, h, sea);
        let p = g.planet;
        let mut seas: Vec<DVec3> = dirs(400, 11).into_iter().filter(|d| g.bake.sample(*d).ocean > 0.99).take(4).collect();
        seas.push(crate::engine::find_places(g.bake.as_ref()).trench.0);
        let mut checked = 0usize;
        let mut walls_total = 0u32;
        for d in &seas {
            for lod in 0..=p.max_lod {
                let vb = 1i64 << lod;
                let top = key_at(&g, *d, lod, -0.5);
                let cols = g.chunk_columns(top);
                // The column's chunks from the sea-level one down, generated as needed.
                let mut stack: Vec<Generated> = vec![g.generate(top)];
                let sea_voxel = (p.sea_level as i64 - 1).div_euclid(vb) * vb;
                for x in 0..CHUNK as i32 {
                    for y in 0..CHUNK as i32 {
                        if !cols[((x + 1) as usize) * PAD + (y + 1) as usize].ocean {
                            continue;
                        }
                        let mut k = sea_voxel;
                        let solid_at = loop {
                            if k < 0 {
                                break None;
                            }
                            let n = (top.cz as i64 - k / (CHUNK as i64 * vb)) as usize;
                            while stack.len() <= n {
                                stack.push(g.generate(ChunkKey { cz: top.cz - stack.len() as u32, ..top }));
                            }
                            let c = &stack[n];
                            let z = ((k - c.key.origin_blocks().2 as i64) / vb) as i32;
                            if reg.is_solid(c.ids[pidx(x, y, z)]) {
                                break Some(k);
                            }
                            k -= vb;
                        };
                        assert!(solid_at.is_some(), "vscale {vs:?} LOD {lod}: ocean column ({x},{y}) of {top:?} has nothing opaque under the sea inside the world");
                        checked += 1;
                    }
                }
                if lod == p.max_lod {
                    assert_eq!(top.cz, 0, "the coarsest chunk holds the whole column");
                    let m = crate::mesh::mesh_chunk(&stack[0], &reg, &p, crate::mesh::MeshOptions::default());
                    assert_eq!(opaque_up_area(&m), (CHUNK * CHUNK) as u32, "vscale {vs:?} LOD {lod}: every column of {top:?} needs one upward opaque face");
                    // The deep ocean's skirt: one side wall per edge voxel facing
                    // a sea column of the border (which holds plain water).
                    let ids = &stack[0].ids;
                    let mut edges = 0u32;
                    for x in 0..CHUNK as i32 {
                        for y in 0..CHUNK as i32 {
                            if ids[pidx(x, y, 0)] != g.blocks.deep_ocean {
                                continue;
                            }
                            for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                                let outside = nx < 0 || ny < 0 || nx >= CHUNK as i32 || ny >= CHUNK as i32;
                                if outside {
                                    assert_ne!(ids[pidx(nx, ny, 0)], g.blocks.deep_ocean, "border columns hold no deep ocean");
                                    edges += (ids[pidx(nx, ny, 0)] == g.blocks.water) as u32;
                                }
                            }
                        }
                    }
                    let walls: u32 = m
                        .opaque
                        .iter()
                        .filter(|q| q[0] & 3 != 2 && q[1] & 0xFFFF == g.blocks.deep_ocean as u32)
                        .map(|q| (((q[0] >> 18) & 31) + 1) * (((q[0] >> 23) & 31) + 1))
                        .sum();
                    assert_eq!(walls, edges, "vscale {vs:?}: deep-ocean side walls along the edges of {top:?}");
                    walls_total += walls;
                }
            }
        }
        assert!(checked > 20_000, "only {checked} ocean columns checked");
        assert!(walls_total > 100, "vscale {vs:?}: only {walls_total} deep-ocean edge walls");
    }
}

/// Review 2 #13: the world ceiling is an air boundary. H 512, sea 256 (the
/// ceiling is 256 m above sea level), 1/8 so the highest range stands far
/// above it: the L0 chunk at cz 15 (layers 480..512, padded 479..513) under
/// that range has its top exposed, because layer 512 is air. It must not be
/// classified Enclosed (the streamer would never draw it and the range
/// would vanish as it refines), and the footprint bounds keep the clipped
/// surface. Then, for the top chunk at every LOD over high ground:
/// classification agrees with the generated contents.
#[test]
fn world_ceiling_exposes_clipped_terrain() {
    let reg = registry();
    let g = generator_with(Some(0.125), 512, 256);
    let p = g.planet;
    let ceiling = (p.height - p.sea_level) as f64 * p.block_size;
    let peak = inspect::find_land_places(g.bake.as_ref()).peak;
    assert!(g.column(peak, p.block_size).height_m as f64 > 600.0, "the range should stand above 600 m at 1/8");
    let key = key_at(&g, peak, 0, ceiling - 1.0);
    assert_eq!(key.cz, 15);
    // The review's numbers: a footprint minimum of 600 m above sea level.
    assert_ne!(g.classify_in(key, (600.0, 700.0)), ChunkClass::Enclosed, "a chunk reaching the ceiling is never enclosed");
    // The real bounds and contents.
    let (hmin, hmax) = g.footprint_height_range_m(key.footprint());
    assert!(hmin <= ceiling && hmin <= hmax, "bounds [{hmin:.1}, {hmax:.1}] must keep the surface clipped at {ceiling} m");
    assert_eq!(g.classify(key), ChunkClass::Mixed);
    let out = g.generate(key);
    for x in 0..CHUNK as i32 {
        for y in 0..CHUNK as i32 {
            assert!(reg.is_solid(out.ids[pidx(x, y, 31)]), "layer 511 at ({x},{y}) is solid under the range");
            assert_eq!(out.ids[pidx(x, y, 32)], g.blocks.air, "layer 512 is above the world");
        }
    }
    let m = crate::mesh::mesh_chunk(&out, &reg, &p, crate::mesh::MeshOptions::default());
    assert_eq!(opaque_up_area(&m), (CHUNK * CHUNK) as u32, "the clipped top shows on every column");
    // The chunk under it is still enclosed: only the top one is exposed.
    let below = ChunkKey { cz: 14, ..key };
    assert_eq!(g.classify(below), ChunkClass::Enclosed);
    assert!(crate::mesh::mesh_chunk(&g.generate(below), &reg, &p, crate::mesh::MeshOptions::default()).opaque.is_empty());
    // Every LOD, the top chunk over high ground: Enclosed only if every
    // padded voxel is solid, Air only if every one is air.
    let mut exposed = 0;
    for d in stress_dirs(&g).into_iter().take(8) {
        for lod in 0..=p.max_lod {
            let top = ChunkKey { cz: p.chunks_per_column(lod) - 1, ..key_at(&g, d, lod, 0.0) };
            let class = g.classify(top);
            let ids = g.generate(top).ids;
            assert!(class != ChunkClass::Enclosed || ids.iter().all(|&b| reg.is_solid(b)), "{top:?} classified Enclosed holds non-solid voxels");
            assert!(class != ChunkClass::Air || ids.iter().all(|&b| b == g.blocks.air), "{top:?} classified Air holds blocks");
            exposed += (g.column(d, p.voxel_m(lod)).height_m as f64 > ceiling) as usize;
        }
    }
    assert!(exposed > 5, "too few top chunks under ground above the ceiling ({exposed})");
}

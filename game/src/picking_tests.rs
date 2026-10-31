//! B3: an independent, fine world-ray oracle. No Frame, wrapping or DDA in
//! the reference: each sample is inverted from its actual world position.
use super::*;
use std::collections::HashMap;

#[path = "picking_legacy_test.rs"]
mod legacy;

const STONE: BlockId = 1;
const WATER: BlockId = 8;

struct Scene {
    planet: Planet,
    cells: HashMap<BlockPos, BlockId>,
}

impl Voxels for Scene {
    fn block(&self, face: u8, i: i64, j: i64, k: i64) -> BlockId {
        if k < 0 || k >= self.planet.height as i64 { return 0; }
        let (face, i, j) = self.planet.wrap_cell(face, i, j);
        self.cells.get(&BlockPos { face, i, j, k: k as u32 }).copied().unwrap_or(0)
    }
    fn is_solid(&self, id: BlockId) -> bool { id == STONE }
    fn is_liquid(&self, id: BlockId) -> bool { id == WATER }
}

fn cell_at(p: &Planet, pos: DVec3) -> BlockPos {
    let r = pos.length();
    let (face, i, j) = p.lattice_of_dir(pos / r);
    BlockPos { face, i: (i.floor() as u32).min(p.n - 1), j: (j.floor() as u32).min(p.n - 1), k: ((r - p.bottom_radius()) / p.block_size).floor() as u32 }
}

fn reference(w: &Scene, eye: DVec3, dir: DVec3, reach: f64, step: f64) -> [Option<(BlockPos, f64)>; 2] {
    let initial = cell_at(&w.planet, eye);
    let mut hits = [None; 2];
    let count = (reach / step).ceil() as usize;
    for s in 0..=count {
        let t = (s as f64 * step).min(reach);
        let bp = cell_at(&w.planet, eye + dir * t);
        let id = w.cells.get(&bp).copied().unwrap_or(0);
        for (liquids, hit) in hits.iter_mut().enumerate() {
            if hit.is_none() && (id == STONE || (liquids == 1 && bp != initial && id == WATER)) {
                *hit = Some((bp, t));
            }
        }
        if hits.iter().all(Option::is_some) { break; }
    }
    hits
}

struct Rng(u64);
impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn signed(&mut self) -> f64 { self.unit() * 2.0 - 1.0 }
}

fn sites() -> Vec<(&'static str, DVec3)> {
    let mut sites = Vec::new();
    for x in [-1.0, 1.0] {
        for y in [-1.0, 1.0] {
            for z in [-1.0, 1.0] { sites.push(("corner", DVec3::new(x, y, z).normalize())); }
        }
    }
    for axis in 0..3 {
        for a in [-1.0, 1.0] {
            for b in [-1.0, 1.0] {
                let mut v = DVec3::ZERO;
                v[(axis + 1) % 3] = a;
                v[(axis + 2) % 3] = b;
                sites.push(("edge", v.normalize()));
            }
        }
    }
    for (axis, right, up) in cubesphere::FACES { sites.push(("ground", (axis + right * 0.31 + up * 0.19).normalize())); }
    sites
}

fn cases(p: Planet, per_site: usize) -> Vec<(&'static str, Scene, DVec3, DVec3)> {
    let mut rng = Rng(0xB3C0_2026_1031);
    let mut out = Vec::new();
    for (kind, radial) in sites() {
        for _ in 0..per_site {
            // Cover the LENGTH of each edge as well as its symmetric
            // midpoint: the skew grows toward the corners.
            let radial = if kind == "edge" {
                let mut v = radial * 2.0f64.sqrt();
                let axis = (0..3).find(|&a| v[a] == 0.0).unwrap();
                v[axis] = rng.signed() * 0.9;
                v.normalize()
            } else { radial };
            let (north, east, _) = Camera::local_frame(radial);
            let centre = radial * p.radius_at(600.4);
            let eye = centre + north * (rng.signed() * 2.5) + east * (rng.signed() * 2.5) + radial * rng.signed();
            let angle = rng.unit() * std::f64::consts::TAU;
            let dir = (north * angle.cos() + east * angle.sin() + radial * (rng.signed() * 1.3)).normalize();
            let mut w = Scene { planet: p, cells: HashMap::new() };
            // Actual edited cells on AND beside the ray; the old extrapolated
            // ray can both miss a real target and invent an off-ray hit.
            for s in 0..12 {
                let point = eye + dir * (s as f64 * 0.6) + north * rng.signed() + east * rng.signed() + radial * rng.signed();
                w.cells.insert(cell_at(&p, point), if rng.unit() < 0.4 { WATER } else { STONE });
            }
            w.cells.insert(cell_at(&p, eye + dir * (0.3 + rng.unit() * 6.3)), STONE);
            out.push((kind, w, eye, dir));
        }
    }
    out
}

#[test]
fn b3_world_ray_reference() {
    let mut totals = [[0usize; 3]; 3]; // comparisons, raw mismatches, unresolved
    let mut wrong_blocks = [0usize; 3];
    let mut wrong_refined = [0usize; 3];
    let mut examples = 0;
    let before = std::env::var_os("B3_BEFORE").is_some();
    let mut other_face = 0;
    let mut three_faces = 0;
    for size in [1.0, 0.5] {
        let p = Planet::new(size, 2048, 512, (131072.0 / size) as u32);
        for (kind, w, eye, dir) in cases(p, 128) {
            let faces: std::collections::BTreeSet<_> = (0..=60).map(|s| cell_at(&p, eye + dir * (s as f64 * 0.1)).face).collect();
            three_faces += usize::from(faces.len() >= 3);
            let group = match kind { "corner" => 0, "edge" => 1, _ => 2 };
            let step = size * 0.001; // 1 mm / 0.5 mm, independent world samples
            let refs = reference(&w, eye, dir, REACH_M, step);
            for liquids in [false, true] {
                totals[group][0] += 1;
                let hit = if before { legacy::legacy_pick(&w, &p, eye, dir, REACH_M, liquids) }
                    else { pick(&w, &p, eye, dir, REACH_M, liquids) };
                let expected = refs[liquids as usize];
                wrong_blocks[group] += usize::from(hit.map(|h| h.block) != expected.map(|h| h.0));
                if !before {
                    if let Some(h) = hit {
                        assert_eq!(h.face, h.block.face);
                        assert_eq!(h.cell, (h.block.i as i64, h.block.j as i64, h.block.k as i64));
                        other_face += usize::from(h.face != cell_at(&p, eye).face);
                        if h.t > 1e-5 {
                            assert_eq!(cell_at(&p, eye + dir * (h.t + 1e-7)), h.block, "hit must be on the world ray");
                            let behind = cell_at(&p, eye + dir * (h.t - 1e-7));
                            let c = h.place_cell();
                            let (face, i, j) = p.wrap_cell(h.face, c.0, c.1);
                            assert_eq!(BlockPos { face, i, j, k: c.2 as u32 }, behind, "normal must name the real cell before entry: {h:?}");
                        }
                    }
                }
                if hit.map(|h| h.block) == expected.map(|h| h.0) {
                    if let (Some(h), Some((_, t))) = (hit, expected) {
                        // The first reference sample trails entry by <= step.
                        if (h.t - t).abs() <= step + 1e-7 { continue; }
                    } else { continue; }
                }
                totals[group][1] += 1;
                if before && hit.map(|h| h.block) == expected.map(|h| h.0) {
                    totals[group][2] += 1;
                    continue;
                }
                // A fine march can skip a sliver visited for < step. Re-run
                // at 1/32 spacing, and never excuse an off-ray cell.
                let fine = reference(&w, eye, dir, REACH_M, step / 32.0)[liquids as usize];
                wrong_refined[group] += usize::from(hit.map(|h| h.block) != fine.map(|h| h.0));
                let matches = hit.map(|h| h.block) == fine.map(|h| h.0)
                    && match (hit, fine) { (Some(h), Some((_, t))) => (h.t - t).abs() <= (if before { step } else { step / 32.0 }) + 1e-7, _ => true };
                if matches && !before {
                    if let (Some(h), Some((_, first))) = (hit, fine) {
                        let mut lo = first;
                        let mut hi = (h.t / step).ceil() * step;
                        assert_ne!(cell_at(&p, eye + dir * hi), h.block);
                        for _ in 0..32 {
                            let mid = (lo + hi) * 0.5;
                            if cell_at(&p, eye + dir * mid) == h.block { lo = mid; } else { hi = mid; }
                        }
                        println!("B3 oracle sliver {kind} size={size}: {:.6} mm ray interval, reference step {:.3} mm, refined step {:.6} mm", (hi - h.t) * 1000.0, step * 1000.0, step * 1000.0 / 32.0);
                    }
                }
                if !matches {
                    totals[group][2] += 1;
                    if examples < 8 {
                        println!("B3 mismatch {kind} size={size} liquids={liquids}: eye={eye:?} dir={dir:?} got={hit:?} reference={fine:?}");
                        examples += 1;
                    }
                }
            }
        }
    }
    println!("B3 rays traversing three faces: {three_faces}; hits on a different face from eye: {other_face}");
    for (((name, [total, raw, unresolved]), wrong), refined) in ["corners", "edges", "ground"].into_iter().zip(totals).zip(wrong_blocks).zip(wrong_refined) {
        println!("B3 {name}: {total} comparisons, {wrong} raw wrong blocks ({:.4}%), {refined} after 32x reference refinement; {raw} raw including distance > reference step, {unresolved} unresolved", wrong as f64 / total as f64 * 100.0);
    }
    assert_eq!(totals.iter().map(|v| v[2]).sum::<usize>(), 0, "true world ray disagrees with pick");
}

#[test]
fn b3_body_overlap_at_every_edge_and_corner() {
    let p = Planet::new(1.0, 2048, 512, 131072);
    let mut neighbours = 0;
    for (_, radial) in sites() {
        let (north, east, _) = Camera::local_frame(radial);
        let pos = radial * p.radius_at(600.2) + north * 0.04 + east * 0.03;
        let player = Player::new(pos, 0.0, 0.0, Mode::Walk, 1.0);
        let frame = Frame::at(&p, pos);
        // Independently sample points strictly INSIDE the body. Every cell
        // containing one must be refused, including the other two faces.
        let mut occupied = std::collections::HashSet::new();
        for x in [-0.27, -0.13, 0.0, 0.13, 0.27] {
            for y in [-0.27, -0.13, 0.0, 0.13, 0.27] {
                for z in [0.03, 0.7, 1.77] {
                    occupied.insert(cell_at(&p, pos + frame.ei * x + frame.ej * y + frame.ek * z));
                }
            }
        }
        for bp in occupied {
            neighbours += usize::from(bp.face != frame.face);
            let cell = (bp.i as i64, bp.j as i64, bp.k as i64);
            let overlaps = if std::env::var_os("B3_BEFORE").is_some() { legacy::overlaps(&player, &p, bp.face, cell) }
                else { player.overlaps_cell(&p, bp.face, cell) };
            assert!(overlaps, "body overlaps {bp:?} from face {}", frame.face);
        }
        for offset in [north * 4.0, -north * 4.0, east * 4.0, -east * 4.0, radial * 4.0] {
            let bp = cell_at(&p, pos + offset);
            assert!(!player.overlaps_cell(&p, bp.face, (bp.i as i64, bp.j as i64, bp.k as i64)), "distant cell {bp:?} must stay placeable");
        }
    }
    assert!(neighbours >= 40);
    println!("B3 body: {neighbours} overlapping cells on other faces refused");
}

#[test]
fn b3_bucket_skips_own_water_cell_and_hits_across_seam() {
    let p = Planet::new(1.0, 2048, 512, 131072);
    for (kind, radial) in sites() {
        if kind == "ground" { continue; }
        let (north, east, _) = Camera::local_frame(radial);
        let eye = radial * p.radius_at(600.5) - east * 1.4 - north * 0.7;
        let dir = (east + north * 0.45).normalize();
        let own = cell_at(&p, eye);
        let water = cell_at(&p, eye + dir * 3.1);
        let solid = cell_at(&p, eye + dir * 5.1);
        let w = Scene { planet: p, cells: HashMap::from([(own, WATER), (water, WATER), (solid, STONE)]) };
        assert_eq!(pick(&w, &p, eye, dir, 6.0, true).unwrap().block, water);
        assert_eq!(pick(&w, &p, eye, dir, 6.0, false).unwrap().block, solid);
        let hit = pick(&w, &p, eye, dir, 6.0, true).unwrap();
        assert!(pick(&w, &p, eye, dir, hit.t - 1e-6, true).is_none());
        assert_eq!(pick(&w, &p, eye, dir, hit.t + 1e-6, true).unwrap().block, water);
    }
}

#[test]
fn b3_radial_graze_enters_layer_between_equal_endpoint_layers() {
    let p = Planet::new(1.0, 2048, 512, 131072);
    let radial = DVec3::new(1.0, 0.3, 0.2).normalize();
    let (north, _, _) = Camera::local_frame(radial);
    let centre = radial * (p.radius_at(601.0) - 0.00001);
    let eye = centre - north * 3.0;
    let target = cell_at(&p, centre);
    let w = Scene { planet: p, cells: HashMap::from([(target, STONE)]) };
    assert_eq!(cell_at(&p, eye).k, 601);
    assert_eq!(cell_at(&p, eye + north * 6.0).k, 601);
    let hit = pick(&w, &p, eye, north, 6.0, false).expect("grazing entry");
    let expected = reference(&w, eye, north, 6.0, 0.0001)[0].unwrap();
    assert_eq!(hit.block, expected.0);
    assert!((hit.t - expected.1).abs() < 0.0001);
}

#[test]
fn b3_inclusive_reach_and_exact_seam_rays() {
    let p = Planet::new(1.0, 2048, 512, 131072);
    for (_, radial) in sites() {
        // Exactly on a cube edge/corner, radial direction, no arbitrary
        // epsilon that selects a different cube face at each step.
        let eye = radial * p.radius_at(603.5);
        let target = cell_at(&p, radial * p.radius_at(600.5));
        let w = Scene { planet: p, cells: HashMap::from([(target, STONE)]) };
        assert!(pick(&w, &p, eye, -radial, 2.5 - 1e-6, false).is_none());
        let hit = pick(&w, &p, eye, -radial, 2.5, false).expect("inclusive reach");
        assert_eq!(hit.block, target);
        assert_eq!(hit.normal, IVec3::Z);
        assert!((hit.t - 2.5).abs() < 1e-7);
    }
}

#[test]
fn b3_lateral_graze_turns_inside_a_face() {
    let p = Planet::new(1.0, 2048, 512, 131072);
    for face in 0..6 {
        let base = p.pos(face, 30000.0, 50000.5, 600.5);
        let f = Frame::at(&p, base);
        let left = p.lattice_of_pos(base - f.ej * 3.0).1;
        let right = p.lattice_of_pos(base + f.ej * 3.0).1;
        let bend = (left + right) * 0.5 - f.i;
        assert!(bend.abs() > 1e-8);
        let centre = base - f.to_world(DVec3::new(bend * 0.5, 0.0, 0.0));
        let eye = centre - f.ej * 3.0;
        let target = cell_at(&p, centre);
        assert_ne!(cell_at(&p, eye).i, target.i);
        assert_ne!(cell_at(&p, eye + f.ej * 6.0).i, target.i);
        let w = Scene { planet: p, cells: HashMap::from([(target, STONE)]) };
        let hit = pick(&w, &p, eye, f.ej, 6.0, false).expect("lateral tangent entry");
        let expected = reference(&w, eye, f.ej, 6.0, 0.0001)[0].unwrap();
        assert_eq!(hit.block, expected.0);
        assert!((hit.t - expected.1).abs() < 0.0001);
    }
}

#[test]
fn b3_outline_uses_the_mesh_grid_on_all_faces() {
    use crate::grid::ChunkKey;
    use crate::world::{Chunk, Registry};
    use crate::worldgen::{Generated, PAD_VOXELS};
    let p = Planet::new(1.0, 2048, 512, 131072);
    let reg = Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap();
    let mut compared = 0;
    for face in 0..6 {
        for i in [0, 1, p.n / 2 + 3, p.n - 2, p.n - 1] {
            for j in [0, 1, p.n / 2 + 5, p.n - 2, p.n - 1] {
                let block = BlockPos { face, i, j, k: 601 };
                let key = ChunkKey::neisor(face, 0, i / 32, j / 32, block.k / 32);
                let g = Generated { key, ids: vec![0; PAD_VOXELS], levels: None, chunk: Chunk::uniform(0), gen_us: 0 };
                let mesh = crate::mesh::mesh_chunk(&g, &reg, &p, crate::mesh::MeshOptions::default());
                for (c, outlined) in crate::ui::outline_corners(&p, block).iter().enumerate() {
                    let lp = glam::Vec3::new((i % 32 + (c & 1) as u32) as f32,
                        (j % 32 + ((c >> 1) & 1) as u32) as f32,
                        (block.k % 32 + ((c >> 2) & 1) as u32) as f32);
                    // Evaluate the real mesher's uploaded grid with the LOD-0
                    // shader equation (not planet.pos / a wrapped box).
                    let grid = lp.truncate() * 0.125;
                    let gx = (grid.x as usize).min(3);
                    let gy = (grid.y as usize).min(3);
                    let (fx, fy) = (grid.x - gx as f32, grid.y - gy as f32);
                    let sample = |base: usize, dx: usize, dy: usize| {
                        let off = (base + (gy + dy) * 5 + gx + dx) * 4;
                        glam::Vec3::from_slice(&mesh.grid[off..off + 3])
                    };
                    let blend = |base| sample(base, 0, 0).lerp(sample(base, 1, 0), fx)
                        .lerp(sample(base, 0, 1).lerp(sample(base, 1, 1), fx), fy);
                    let rendered = mesh.origin + (blend(0) + blend(25).normalize() * lp.z).as_dvec3();
                    assert!(outlined.distance(rendered) < 1e-9, "{block:?} corner {c}");
                    compared += 1;
                }
            }
        }
    }
    println!("B3 outline: {compared} vertices agree with the mesher's uploaded curvature grid");
}

#[test]
fn b3_outline_does_not_fold_a_box_over_three_faces() {
    let p = Planet::new(1.0, 2048, 512, 131072);
    let n = p.n;
    let (face, i, j) = p.wrap_cell(2, n as i64, n as i64);
    let bp = BlockPos { face, i, j, k: 523 };
    let actual = if std::env::var_os("B3_BEFORE").is_some() {
        std::array::from_fn(|c| p.pos(2, (n + (c & 1) as u32) as f64,
            (n + ((c >> 1) & 1) as u32) as f64, (523 + ((c >> 2) & 1)) as f64))
    } else { crate::ui::outline_corners(&p, bp) };
    let target: [DVec3; 8] = std::array::from_fn(|c| p.pos(bp.face,
        (bp.i + (c & 1) as u32) as f64, (bp.j + ((c >> 1) & 1) as u32) as f64,
        (bp.k + ((c >> 2) & 1) as u32) as f64));
    // Corner order can rotate across faces. Compare the unordered geometry;
    // allow the sub-mm LOD-0 grid interpolation, but no displaced vertices.
    for corner in actual {
        let error = target.iter().map(|v| v.distance(corner)).fold(f64::INFINITY, f64::min);
        assert!(error < 0.001, "outline corner is {error} m from the selected block");
    }
}

#[test]
#[ignore = "provisional timing; run explicitly with --ignored --nocapture"]
fn b3_pick_cost() {
    let p = Planet::new(1.0, 2048, 512, 131072);
    let scenes = cases(p, 16);
    let empty = Scene { planet: p, cells: HashMap::new() };
    for misses in [false, true] {
        for (name, picker) in [("before", legacy::legacy_pick as fn(&dyn Voxels, &Planet, DVec3, DVec3, f64, bool) -> Option<Hit>), ("after", pick)] {
            let mut times = Vec::new();
            for _ in 0..100 {
                for (_, w, eye, dir) in &scenes {
                    for liquids in [false, true] {
                        let start = std::time::Instant::now();
                        std::hint::black_box(picker(std::hint::black_box(if misses { &empty } else { w }), &p, *eye, *dir, REACH_M, liquids));
                        times.push(start.elapsed().as_secs_f64() * 1e6);
                    }
                }
            }
            times.sort_unstable_by(f64::total_cmp);
            println!("B3 provisional {name}, full-reach misses={misses}: {} picks, mean {:.3} us, p50 {:.3}, p95 {:.3}, p99 {:.3}, max {:.3}", times.len(), times.iter().sum::<f64>() / times.len() as f64, times[times.len()/2], times[times.len()*95/100], times[times.len()*99/100], times[times.len()-1]);
        }
    }
}

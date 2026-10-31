//! B7: the sloped powder surface. Neighbouring tops meet exactly, across
//! chunks and cube faces, the cube mesh leaves no face poking through, and with
//! the switch off none of it costs anything.

use super::super::tests::{hills_ground, test_world};
use super::super::{BlockId, World};
use super::*;
use crate::grid::BlockPos;
use crate::mesh::{MeshData, MeshOptions, mesh_chunk_levels, mesh_chunk_slopes};
use std::collections::{BTreeMap, BTreeSet};

/// A stone plate in the sky and, on it, a cone of sand set straight into the
/// world: `h0` units high at (0, 0), falling `fall` units per cell.
fn cone(w: &World, c: BlockPos, h0: i64, fall: i64, r: i64) {
    let (stone, sand) = (w.generator.blocks.stone, w.generator.blocks.sand);
    w.edits.set_slopes(true);
    let at = |di: i64, dj: i64, k: u32| w.resolve_block(c.face, c.i as i64 + di, c.j as i64 + dj, k as i64).unwrap();
    let mut writes: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
    for di in -r..=r {
        for dj in -r..=r {
            writes.push((at(di, dj, c.k - 1), stone, None));
            let h = h0 - fall * ((di * di + dj * dj) as f64).sqrt().round() as i64;
            let mut left = h.max(0);
            let mut k = c.k;
            while left > 0 {
                let u = left.min(255);
                writes.push((at(di, dj, k), sand, Some(if u == 255 { 0 } else { u as u8 })));
                left -= u;
                k += 1;
            }
        }
    }
    w.commit_levels(&writes);
}

fn mesh_of(w: &World, key: ChunkKey) -> MeshData {
    let made = w.generate(key);
    let levels = w.padded_levels(key, &made);
    let powder = w.padded_powder(key, &made);
    mesh_chunk_slopes(&made, levels.as_ref(), powder.as_ref(), &w.registry, &w.planet, MeshOptions::default())
}

/// The heights a slope quad's vertices stand at, decoded as the shader does.
fn heights(q: &SlopeQuad) -> [i32; 4] {
    let h = |word: u32, shift: u32| ((word >> shift) & 0xFFFF) as u16 as i16 as i32;
    [h(q[2], 0), h(q[2], 16), h(q[3], 0), h(q[3], 16)]
}

/// Every lattice corner a mesh's tops stand on, by its world position in
/// centimetres (so two faces' lattices agree), with the height there.
fn top_corners(w: &World, m: &MeshData, out: &mut BTreeMap<(i64, i64, i64), Vec<i32>>) {
    let (i0, j0, k0) = m.key.origin_blocks();
    for q in &m.slopes {
        let (axis, sign, slice, u, v, part) = (q[0] & 3, (q[0] >> 2) & 1, (q[0] >> 3) & 31, (q[0] >> 8) & 31, (q[0] >> 13) & 31, (q[0] >> 18) & 31);
        if axis != 2 || sign != 1 {
            continue;
        }
        let _ = slice;
        let h = heights(q);
        // Vertices 0, 2 and 3: the fan's corners (emit_slope's order).
        let corners = if part == 0 { [(1, 0), (0, 0), (0, 1)] } else { [(0, 1), (1, 1), (1, 0)] };
        for (n, (cu, cv)) in [0usize, 2, 3].into_iter().zip(corners) {
            let dir = w.planet.dir(m.key.face, (i0 + u as u64 + cu) as f64, (j0 + v as u64 + cv) as f64);
            let at = dir * 1e4 * w.planet.radius;
            out.entry((at.x.round() as i64 / 100, at.y.round() as i64 / 100, at.z.round() as i64 / 100)).or_default().push(h[n] + 255 * k0 as i32);
        }
    }
}

/// Every chunk the pile lies in or touches.
fn chunks_of(w: &World, c: BlockPos, r: i64) -> BTreeSet<ChunkKey> {
    let mut keys = BTreeSet::new();
    for di in -r..=r {
        for dj in -r..=r {
            for dk in 0..8 {
                keys.insert(World::chunk_of(w.resolve_block(c.face, c.i as i64 + di, c.j as i64 + dj, c.k as i64 + dk).unwrap()).0);
            }
        }
    }
    keys
}

fn assert_corners_meet(w: &World, c: BlockPos, r: i64) -> usize {
    let mut corners = BTreeMap::new();
    let keys = chunks_of(w, c, r);
    for key in &keys {
        top_corners(w, &mesh_of(w, *key), &mut corners);
    }
    assert!(!corners.is_empty());
    let mut shared = 0;
    for (at, hs) in &corners {
        let first = hs[0];
        assert!(hs.iter().all(|h| *h == first), "corner {at:?} has heights {hs:?}");
        shared += (hs.len() > 1) as usize;
    }
    println!("{} chunks, {} corners, {shared} shared, all equal", keys.len(), corners.len());
    shared
}

#[test]
fn tops_meet_exactly_inside_a_chunk_and_across_chunks() {
    let w = test_world();
    let g = hills_ground(&w);
    assert!(assert_corners_meet(&w, { let c = BlockPos { k: g.k + 24, ..g }; cone(&w, c, 700, 150, 6); c }, 6) > 20);
    let w = test_world();
    let c = BlockPos { face: 2, i: 32 * 3000, j: 32 * 2000, k: 32 * 47 + 1 };
    cone(&w, c, 800, 170, 6);
    assert!(assert_corners_meet(&w, c, 6) > 20);
}

#[test]
fn tops_meet_exactly_across_a_cube_edge_and_at_a_cube_corner() {
    let w = test_world();
    let n = w.planet.n;
    let c = BlockPos { face: 0, i: n - 1, j: n / 2, k: 1500 };
    cone(&w, c, 700, 150, 6);
    let keys = chunks_of(&w, c, 6);
    assert!(keys.iter().map(|k| k.face).collect::<BTreeSet<_>>().len() == 2, "on two faces");
    assert_corners_meet(&w, c, 6);
    let w = test_world();
    let c = BlockPos { face: 0, i: n - 1, j: n - 1, k: 1500 };
    cone(&w, c, 700, 150, 5);
    assert_corners_meet(&w, c, 5);
}

/// No hole: a slope column beside anything lower has a skirt reaching from its
/// surface to where its cubes begin, and beside another slope the edge is the
/// same two corners.
#[test]
fn every_edge_of_a_slope_is_closed() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    cone(&w, c, 700, 150, 6);
    let (mut shared, mut skirted) = (0, 0);
    let slope_of = |di: i64, dj: i64| -> Option<Slope> {
        let (ci, cj) = (c.i as i64 + di, c.j as i64 + dj);
        (c.k as i64..c.k as i64 + 6).rev().find_map(|k| w.slope_column(c.face, ci, cj, k))
    };
    for di in -7i64..=7 {
        for dj in -7i64..=7 {
            let Some(s) = slope_of(di, dj) else { continue };
            for (dx, dy, a, b, na, nb) in [(1i64, 0i64, 1, 3, 0, 2), (0, 1, 2, 3, 0, 1)] {
                match slope_of(di + dx, dj + dy) {
                    Some(n) => {
                        assert_eq!((s.corners[a], s.corners[b]), (n.corners[na], n.corners[nb]), "the edge beside ({di}, {dj})");
                        shared += 1;
                    }
                    None => skirted += 1,
                }
            }
            let (ci, cj) = (c.i as i64 + di, c.j as i64 + dj);
            let (x, y) = (ci as u32 % 32, cj as u32 % 32);
            let key = World::chunk_of(BlockPos { i: ci as u32, j: cj as u32, k: s.top as u32, ..c }).0;
            let m = mesh_of(&w, key);
            let lo = |axis: u32, sign: u32, slice: u32, u: u32| axis | (sign << 2) | (slice << 3) | (u << 8);
            // At the pile's outer edge the skirts must be there: a corner on
            // the ground needs none, one above it does.
            for (axis, sign, slice, u, nx, ny, e0, e1) in [(0u32, 0u32, x, y, -1i64, 0i64, 0usize, 2usize), (0, 1, x, y, 1, 0, 1, 3), (1, 0, y, x, 0, -1, 0, 1), (1, 1, y, x, 0, 1, 2, 3)] {
                if slope_of(di + nx, dj + ny).is_some() {
                    continue;
                }
                let bottom = 255 * s.floor as i32;
                if s.corners[e0].max(s.corners[e1]) <= bottom {
                    continue;
                }
                let k0 = key.origin_blocks().2 as i32;
                let found = m.slopes.iter().any(|q| q[0] == lo(axis, sign, slice, u) && heights(q) == [bottom - 255 * k0, bottom - 255 * k0, s.corners[e0] - 255 * k0, s.corners[e1] - 255 * k0]);
                assert!(found, "no skirt on side {axis}/{sign} of ({di}, {dj}): {s:?}");
            }
        }
    }
    println!("{shared} shared edges, {skirted} against something else");
    assert!(shared > 40 && skirted > 8);
}

/// A slope hides the cubes it cuts through: no cube face stands in a cell it
/// replaces, and every cell under `floor` keeps its cube.
#[test]
fn the_cube_mesh_leaves_the_cells_a_slope_replaces() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    cone(&w, c, 900, 200, 6);
    let key = World::chunk_of(c).0;
    let made = w.generate(key);
    let powder = w.padded_powder(key, &made).expect("a slope here");
    let soft: BTreeSet<(i8, i8, i8)> = powder.soft.iter().copied().collect();
    assert!(!soft.is_empty());
    let m = mesh_of(&w, key);
    for q in &m.opaque {
        let lo = q[0];
        let (axis, sign, slice, u, v, ww, hh) = (lo & 3, (lo >> 2) & 1, (lo >> 3) & 31, (lo >> 8) & 31, (lo >> 13) & 31, ((lo >> 18) & 31) + 1, ((lo >> 23) & 31) + 1);
        for a in u..u + ww {
            for b in v..v + hh {
                let cell = match axis {
                    0 => (slice as i8, a as i8, b as i8),
                    1 => (a as i8, slice as i8, b as i8),
                    _ => (a as i8, b as i8, slice as i8),
                };
                assert!(!soft.contains(&cell), "a cube face of {cell:?}, axis {axis} sign {sign}, stands in a slope");
            }
        }
    }
    // The whole cells a low corner goes under are in the set, not just the tops.
    let mut below_tops = 0;
    for &(x, y, z) in &soft {
        if soft.contains(&(x, y, z + 1)) {
            below_tops += 1;
        }
    }
    assert!(below_tops > 0, "a slope at 200 per cell cuts into whole cells");
}

/// A lone partial cell shows: a tent, standing at its own height at the
/// middle. It has no flat top at ground level, and a one-wide strip neither.
#[test]
fn a_lone_cell_stands_at_its_own_height() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    let (stone, sand) = (w.generator.blocks.stone, w.generator.blocks.sand);
    w.edits.set_slopes(true);
    let mut writes: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
    for di in -3i32..=3 {
        for dj in -3i32..=3 {
            writes.push((BlockPos { i: (c.i as i32 + di) as u32, j: (c.j as i32 + dj) as u32, k: c.k - 1, ..c }, stone, None));
        }
    }
    writes.push((c, sand, Some(150)));
    w.commit_levels(&writes);
    let s = w.slope_column(c.face, c.i as i64, c.j as i64, c.k as i64).unwrap();
    let base = 255 * c.k as i32;
    assert_eq!(s.centre, base + 150);
    assert_eq!(s.corners, [base; 4], "the corners are on the plate");
    assert!((s.height_at(0.5, 0.5) - c.k as f64 - 150.0 / 255.0).abs() < 1e-9);
    assert!((s.height_at(0.0, 0.0) - c.k as f64).abs() < 1e-9);
    assert!((s.height_at(0.25, 0.5) - c.k as f64 - 75.0 / 255.0).abs() < 1e-9, "half way down the tent");
}

/// `height_at` agrees with the triangles: at the corners, the middle, and
/// halfway along an edge, on an uneven cell.
#[test]
fn height_at_is_the_drawn_surface() {
    let s = Slope { id: 4, top: 10, floor: 9, corners: [2550, 2600, 2500, 2700], centre: 2650, slopes: [(0, 0); 5], open: [false; 4] };
    let at = |u, v| s.height_at(u, v) * 255.0;
    for (u, v, h) in [(0.0, 0.0, 2550.0), (1.0, 0.0, 2600.0), (0.0, 1.0, 2500.0), (1.0, 1.0, 2700.0), (0.5, 0.5, 2650.0), (0.5, 0.0, 2575.0), (0.0, 0.5, 2525.0), (1.0, 0.5, 2650.0), (0.5, 1.0, 2600.0)] {
        assert!((at(u, v) - h).abs() < 1e-6, "({u}, {v}): {} against {h}", at(u, v));
    }
    // Continuous across the four triangles: either side of a diagonal.
    assert!((at(0.3, 0.2999999) - at(0.2999999, 0.3)).abs() < 1e-3);
    assert!((at(0.7, 0.3000001) - at(0.6999999, 0.3)).abs() < 1e-3);
}

/// Off, it is the old mesh to the byte, and nothing is asked of the world.
#[test]
fn with_the_switch_off_there_is_no_slope() {
    let w = test_world();
    let g = hills_ground(&w);
    let c = BlockPos { k: g.k + 24, ..g };
    cone(&w, c, 700, 150, 5);
    w.edits.set_slopes(false);
    let key = World::chunk_of(c).0;
    let made = w.generate(key);
    assert!(w.padded_powder(key, &made).is_none());
    assert!(w.slope_at(c.face, c.i as i64, c.j as i64, c.k as i64).is_none());
    let levels = w.padded_levels(key, &made);
    let a = mesh_chunk_levels(&made, levels.as_ref(), &w.registry, &w.planet, MeshOptions::default());
    let b = mesh_chunk_slopes(&made, levels.as_ref(), None, &w.registry, &w.planet, MeshOptions::default());
    assert_eq!((a.opaque, a.water), (b.opaque, b.water));
    assert!(a.slopes.is_empty());
}

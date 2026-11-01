//! C2, Andrew's #5: what the renderer is told about a block that slid or
//! landed (`Sim::view`), and that none of it is state: the trail is not in
//! the hash or a replay's digest, a save carries it so a reload mid-slide
//! draws on, and an older save reads without it.

use super::tests::{ids, site};
use super::*;
use crate::world::tests::test_world;

/// A stone plate at `k`, `half` cells each way from (i, j).
fn plate(w: &World, o: BlockPos, half: u32) {
    let (_, stone, ..) = ids(w);
    let side = 2 * half + 1;
    let writes: Vec<(BlockPos, BlockId)> = (0..side).flat_map(|dx| (0..side).map(move |dy| (BlockPos { i: o.i + dx - half, j: o.j + dy - half, ..o }, stone))).collect();
    w.commit(&writes);
}

/// A sand block on the +i edge of a plate in the sky: it slides off into
/// the column beyond and falls to the ground. Returns the world, the sim
/// after the tick it slid in, the focus, the cell it left and its column.
fn slid() -> (World, Sim, DVec3, BlockPos, BlockPos) {
    let w = test_world();
    let (g, focus) = site(&w);
    let (.., sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 14, ..g };
    plate(&w, o, 3);
    let p = BlockPos { i: o.i + 3, k: o.k + 1, ..o };
    assert!(w.set_block(p, sand));
    let n = BlockPos { i: p.i + 1, ..p };
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!((sim.stats.slides, sim.falling().len()), (1, 1), "the block slid in the first tick");
    (w, sim, focus, p, n)
}

#[test]
fn a_slide_is_drawn_from_the_cell_it_left_and_then_on_its_way() {
    let (w, mut sim, focus, p, n) = slid();
    let f = sim.falling()[0];
    assert_eq!((f.face, f.i, f.j), (n.face, n.i, n.j), "the rule moved it into the next column");
    assert_eq!(f.from.map(|t| (t.cell, t.age)), Some((p, 1)), "and it remembers the cell it left");
    let rev = w.edits.revision(World::chunk_of(p).0).unwrap();
    assert!(rev > 0);
    // The frame right after the tick (phase 0): at the cell it left, at
    // the height it left from, nothing of its first tick's fall shown yet.
    let v = sim.view();
    assert_eq!(v.len(), 1);
    let d = v[0];
    assert_eq!((d.face, d.i, d.j, d.ghost), (n.face, n.i, n.j, false));
    assert_eq!(d.wrote, Some((p, rev)));
    assert_eq!(d.since, 0.0);
    assert_eq!(d.z, p.k as f64);
    assert_eq!(d.slid_from(), Some(p));
    assert_eq!(d.slide_progress(), 0.0);
    // Half a tick on: part of the way over and part of the tick's fall.
    sim.set_phase(0.5);
    let d = sim.view()[0];
    assert_eq!(d.since, 0.5);
    assert!((d.z - (p.k as f64 - 0.03125)).abs() < 1e-9, "{}", d.z);
    assert!(d.slide_progress() > 0.2 && d.slide_progress() < 0.3, "{}", d.slide_progress());
    // The next tick: a whole tick along, and the fall's first tick drawn in full.
    sim.step(&w, focus);
    sim.set_phase(0.0);
    let d = sim.view()[0];
    assert_eq!(d.since, 1.0);
    assert!((d.z - (p.k as f64 - 0.0625)).abs() < 1e-9, "{}", d.z);
    for _ in 0..3 {
        sim.step(&w, focus);
    }
    let d = sim.view()[0];
    assert_eq!(d.since, 4.0);
    assert_eq!(d.slide_progress(), 1.0, "in its own column after SLIDE_TICKS");
    assert_eq!(d.slid_from(), Some(p), "the renderer still knows where it came from (its mesh may be late)");
    let json = sim.blocks_json();
    assert_eq!(json[0]["from"], serde_json::json!([p.face, p.i, p.j, p.k, 5]));
}

#[test]
fn the_trail_is_look_only_and_not_in_the_hash_or_the_digest() {
    let (w, mut sim, _, p, _) = slid();
    let hash = sim.state_hash(&w);
    let digest = sim.state_digest(&w);
    assert_eq!(sim.falling[0].from.map(|t| t.cell), Some(p));
    sim.falling[0].from = None;
    sim.falling[0].z_prev = 0.0;
    assert_eq!(sim.state_hash(&w), hash, "the trail decides nothing");
    assert_eq!(sim.state_digest(&w), digest);
    assert_eq!(sim.view()[0].wrote, None);
    assert_eq!(sim.view()[0].since, SLIDE_TICKS, "no trail: drawn in its own column at once");
}

#[test]
fn a_snapshot_keeps_the_trail_and_a_version_4_file_reads_without_it() {
    let cell = BlockPos { face: 3, i: 70, j: 80, k: 90 };
    let f = Falling { from: Some(Trail { cell, rev: 7, age: 2 }), ..Falling::new(4, 3, 71, 80, 89.75, -2.5) };
    let snap = Snapshot { tick: 5, falling: vec![f, Falling::new(5, 3, 71, 80, 95.0, 0.0)], ..Default::default() };
    let bytes = snap.to_bytes();
    let read = Snapshot::from_bytes(&bytes).unwrap();
    assert_eq!(read.falling[0].from, Some(Trail { cell, rev: 0, age: 2 }), "the cell and the age; the revision is not kept");
    assert_eq!(read.falling[1].from, None);
    assert_eq!(read.falling[0].z_prev, 89.75, "where it was is where it is");
    // The same state as a version-4 file: no trailer, no trail.
    let mut v4 = bytes.clone();
    v4.truncate(v4.len() - (1 + 1 + 12 + 4) - 1);
    v4[4..8].copy_from_slice(&4u32.to_le_bytes());
    let read = Snapshot::from_bytes(&v4).unwrap();
    assert_eq!(read.falling[0].from, None);
    assert_eq!(read.falling.len(), 2);
    assert!(Snapshot::from_bytes(&bytes[..bytes.len() - 1]).unwrap_err().contains("truncated"));
}

#[test]
fn a_reload_mid_slide_draws_on() {
    let (w, mut sim, focus, p, n) = slid();
    sim.step(&w, focus);
    let snap = Snapshot::from_bytes(&sim.snapshot(&w).to_bytes()).unwrap();
    let mut back = Sim::from_snapshot(SimConfig::default(), snap);
    back.step(&w, focus);
    assert_eq!(back.state_hash(&w), { sim.step(&w, focus); sim.state_hash(&w) }, "the reload runs on to the same state");
    let d = back.view()[0];
    assert_eq!((d.face, d.i, d.j), (n.face, n.i, n.j));
    assert_eq!(d.slid_from(), Some(p), "still on its way from the cell it left");
    assert_eq!(d.since, 2.0);
    assert_eq!(d.wrote.map(|(_, rev)| rev), Some(0), "no revision: the reload meshes everything afresh, so the mesh counts as current");
}

#[test]
fn a_landed_block_is_drawn_through_its_last_tick_then_at_its_cell_until_its_mesh_could_be_late() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (.., sand, _) = ids(&w);
    let start = BlockPos { k: g.k + 6, ..g };
    assert!(w.set_block(start, sand));
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.falling().len(), 1);
    assert_eq!(sim.view()[0].wrote.map(|(c, _)| c), Some(start), "a fall's trail is its own cell");
    assert_eq!(sim.view()[0].slid_from(), None);
    let mut before = sim.falling()[0].z;
    while sim.stats.landings == 0 {
        before = sim.falling()[0].z;
        sim.step(&w, focus);
    }
    let landed = BlockPos { k: g.k + 1, ..g };
    assert_eq!(w.block_at(landed), sand);
    assert!(sim.falling().is_empty());
    let rev = w.edits.revision(World::chunk_of(landed).0).unwrap();
    let v = sim.view();
    assert_eq!(v.len(), 1);
    let d = v[0];
    assert!(d.ghost);
    assert_eq!(d.wrote, Some((landed, rev)));
    assert_eq!(d.since, 0.0);
    assert_eq!(d.z, before, "at phase 0 the landing tick's fall is still to be drawn");
    sim.set_phase(1.0);
    assert_eq!(sim.view()[0].z, landed.k as f64);
    sim.step(&w, focus);
    sim.set_phase(0.0);
    let d = sim.view()[0];
    assert_eq!((d.z, d.since), (landed.k as f64, 1.0));
    for _ in 0..GHOST_TICKS_MAX - 2 {
        sim.step(&w, focus);
    }
    assert_eq!(sim.view().len(), 1, "offered for as long as a mesh could be late");
    sim.step(&w, focus);
    assert!(sim.view().is_empty(), "and then no more");
}

#[test]
fn a_slide_over_a_cube_edge_names_the_cell_on_the_other_face_and_its_way_is_a_block_long() {
    let w = test_world();
    let (_, stone, sand, _) = ids(&w);
    let n = w.planet.n;
    let k = 1500u32;
    let p = BlockPos { face: 0, i: n - 1, j: n / 2, k };
    let across = lateral(&w, p).into_iter().flatten().find(|q| q.face != 0).expect("a neighbour over the edge");
    // A plate under the block and its three same-face neighbours; over the
    // edge the plate is one layer lower, so the way over is open.
    let mut writes = Vec::new();
    for (di, dj) in [(0i64, 0i64), (-1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1)] {
        writes.push((w.resolve_block(0, p.i as i64 + di, p.j as i64 + dj, k as i64 - 1).unwrap(), stone));
    }
    // A plate under the landing cell and round it, so it stays where it lands.
    for di in -1..=1i64 {
        for dj in -1..=1i64 {
            writes.push((w.resolve_block(across.face, across.i as i64 + di, across.j as i64 + dj, k as i64 - 2).unwrap(), stone));
        }
    }
    writes.push((p, sand));
    w.commit(&writes);
    let focus = w.planet.pos(0, p.i as f64, p.j as f64, k as f64);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.stats.slides, 1);
    let d = sim.view()[0];
    assert_eq!((d.face, d.i, d.j), (across.face, across.i, across.j), "it slid onto the other face");
    assert_ne!(d.face, 0);
    assert_eq!(d.slid_from(), Some(p));
    let way = w.planet.pos(p.face, p.i as f64 + 0.5, p.j as f64 + 0.5, k as f64) - w.planet.pos(d.face, d.i as f64 + 0.5, d.j as f64 + 0.5, k as f64);
    // Cells are a block wide only nominally: across a cube edge the two
    // centres are further apart than on the flat of a face. The way is
    // tangent to the planet and about a block long.
    let up = w.planet.pos(d.face, d.i as f64 + 0.5, d.j as f64 + 0.5, k as f64).normalize();
    assert!(way.normalize().dot(up).abs() < 0.01, "the way is level");
    assert!(way.length() > 0.8 && way.length() < 1.4, "the way from cell centre to cell centre over the edge is about one block: {} m", way.length());
    for _ in 0..30 {
        sim.step(&w, focus);
    }
    assert_eq!(sim.stats.landings, 1);
    assert_eq!(w.block_at(BlockPos { k: k - 1, ..across }), sand);
}

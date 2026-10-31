//! B4, Andrew's #7: powder and water slide sideways and down when there is a
//! lower open space directly beside them.
//!
//! Powder: a block at rest with a free cell beside it and a free cell below
//! that one moves over and falls. It settles into stable piles, crosses
//! chunk borders, cube edges and cube corners, waits at the edge of the
//! active region, and its choice of direction is a pure function of the
//! position and the tick. Water: the level-1 cell at a ledge, which the 2-level
//! rule could never move, goes over, and the potential strictly falls on
//! every tick that moves anything.

use super::tests::{copy_world, ids, site};
use super::water::{FULL, water_potential, water_total};
use super::*;
use crate::world::tests::test_world;
use std::collections::BTreeMap;

/// A rectangular stone plate at layer `o.k`.
fn plate(w: &World, o: BlockPos, nx: u32, ny: u32) {
    let (_, stone, ..) = ids(w);
    let writes: Vec<(BlockPos, BlockId)> = (0..nx).flat_map(|dx| (0..ny).map(move |dy| (BlockPos { i: o.i + dx, j: o.j + dy, ..o }, stone))).collect();
    w.commit(&writes);
}

/// Every powder cell in the edit store.
fn powder_cells(w: &World) -> Vec<BlockPos> {
    let mut out = Vec::new();
    for c in w.edits.snapshot_full() {
        let (i0, j0, k0) = c.key.origin_blocks();
        for x in 0..CHUNK {
            for y in 0..CHUNK {
                for z in 0..CHUNK {
                    if w.registry.has_gravity(c.ids[crate::world::chunk::index(x, y, z)]) {
                        out.push(BlockPos { face: c.key.face, i: i0 as u32 + x as u32, j: j0 as u32 + y as u32, k: k0 as u32 + z as u32 });
                    }
                }
            }
        }
    }
    out
}

/// The cell one layer down.
fn under(p: BlockPos) -> BlockPos {
    BlockPos { k: p.k - 1, ..p }
}

/// A powder cell nothing moves: supported, and no free cell beside it with a
/// free cell below that one (inside the active region).
fn assert_powder_at_rest(w: &World, sim: &Sim) {
    let reg = &w.registry;
    for p in powder_cells(w) {
        assert!(p.k == 0 || !reg.is_replaceable(w.block_at(under(p))), "{p:?} hangs in the air");
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let Some(n) = w.resolve_block(p.face, p.i as i64 + di, p.j as i64 + dj, p.k as i64) else { continue };
            if !sim.is_active(World::chunk_of(n).0) || n.k == 0 || !sim.is_active(World::chunk_of(under(n)).0) {
                continue;
            }
            assert!(!(reg.is_replaceable(w.block_at(n)) && reg.is_replaceable(w.block_at(under(n)))), "{p:?} could still slide into {n:?}");
        }
    }
}

/// Tick until nothing is queued or in flight; the ticks taken.
fn run_until_still(w: &World, sim: &mut Sim, focus: DVec3, max_ticks: usize) -> usize {
    for t in 1..=max_ticks {
        sim.step(w, focus);
        if sim.queued() == 0 && sim.falling().is_empty() {
            return t;
        }
    }
    panic!("not still after {max_ticks} ticks: {} queued, {} falling", sim.queued(), sim.falling().len());
}

/// Nothing happens for `ticks` ticks: no update runs, no chunk changes.
fn assert_stays_still(w: &World, sim: &mut Sim, focus: DVec3, ticks: usize) {
    let generation = w.edits.generation();
    let processed = sim.stats.processed;
    for _ in 0..ticks {
        sim.step(w, focus);
    }
    assert_eq!((sim.stats.processed - processed, w.edits.generation() - generation, sim.queued()), (0, 0, 0), "no jitter");
}

#[test]
fn sand_beside_a_drop_moves_over_and_falls() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, _, sand, gravel) = ids(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    plate(&w, o, 3, 3);
    // On the middle of the plate's +i side: the only drop is along +i.
    let edge = BlockPos { i: o.i + 2, j: o.j + 1, k: o.k + 1, ..o };
    assert!(w.set_block(edge, sand));
    // And in the plate's middle, where nothing is free below a neighbour.
    let centre = BlockPos { i: o.i + 1, j: o.j + 1, k: o.k + 1, ..o };
    assert!(w.set_block(centre, gravel));
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(w.block_at(edge), air, "it left its cell");
    assert_eq!(sim.stats.slides, 1);
    assert_eq!(sim.stats.falls, 0, "a slide is not a fall from its own column");
    assert_eq!(sim.falling().len(), 1);
    let f = sim.falling()[0];
    assert_eq!((f.i, f.j), (edge.i + 1, edge.j), "the next column along +i");
    assert!(f.z <= edge.k as f64 && f.z > edge.k as f64 - 1.0, "starting from its own height: {}", f.z);
    // It falls past the plate's layer to the ground and stays there.
    let ticks = run_until_still(&w, &mut sim, focus, 200);
    assert_eq!(sim.stats.landings, 1);
    let rest = (1..=20).map(|d| BlockPos { i: edge.i + 1, k: edge.k - d, ..edge }).find(|&b| w.block_at(b) == sand).expect("landed in the next column");
    assert!(rest.k < o.k, "below the plate: layer {} under {}", rest.k, o.k);
    println!("slid and landed {} layers down in {ticks} ticks", edge.k - rest.k);
    assert_eq!(w.block_at(centre), gravel, "the block in the middle has no drop beside it");
    assert_stays_still(&w, &mut sim, focus, 30);
}

/// A block has to have the free cell beside it AND the free cell below that
/// one: a step one block high is a stable slope.
#[test]
fn a_step_one_block_high_is_stable_and_a_step_two_high_is_not() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, _, sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    plate(&w, o, 7, 3);
    let cell = |dx: u32, dz: u32| BlockPos { i: o.i + dx, j: o.j + 1, k: o.k + dz, ..o };
    // A staircase: one block high over the plate, two, three.
    for (dx, height) in [(1u32, 1u32), (2, 2), (3, 3)] {
        for dz in 1..=height {
            assert!(w.set_block(cell(dx, dz), sand));
        }
    }
    // Walls along j so the stair can only move along i.
    plate(&w, BlockPos { k: o.k + 1, ..o }, 7, 1);
    plate(&w, BlockPos { j: o.j + 2, k: o.k + 1, ..o }, 7, 1);
    plate(&w, BlockPos { k: o.k + 2, ..o }, 7, 1);
    plate(&w, BlockPos { j: o.j + 2, k: o.k + 2, ..o }, 7, 1);
    plate(&w, BlockPos { k: o.k + 3, ..o }, 7, 1);
    plate(&w, BlockPos { j: o.j + 2, k: o.k + 3, ..o }, 7, 1);
    let mut sim = Sim::new(SimConfig::default());
    run_until_still(&w, &mut sim, focus, 100);
    // The three-high column stands three above its +i neighbour, so it has to
    // lose something; how much depends on which of its top two wins the free
    // cell. The one-high step (columns 1 and 2) never moves.
    assert!((1..=3).contains(&sim.stats.slides), "{}", sim.stats.slides);
    assert_eq!(w.block_at(cell(1, 1)), sand);
    assert_eq!(w.block_at(cell(2, 2)), sand);
    assert_powder_at_rest(&w, &sim);
    assert_stays_still(&w, &mut sim, focus, 20);
}

/// The direction: over many blocks with all four ways open, each way is
/// taken about equally often, so the rule carries no lattice bias.
#[test]
fn the_direction_of_a_slide_has_no_lattice_bias() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, stone, sand, _) = ids(&w);
    let mut writes = Vec::new();
    let mut tops = Vec::new();
    // 400 one-cell stone pillars, four apart, each with a sand block on top.
    for a in 0..20u32 {
        for b in 0..20u32 {
            let p = BlockPos { i: g.i - 40 + 4 * a, j: g.j - 40 + 4 * b, k: g.k + 24, ..g };
            writes.push((p, stone));
            let top = BlockPos { k: p.k + 1, ..p };
            writes.push((top, sand));
            tops.push(top);
        }
    }
    w.commit(&writes);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.stats.slides, 400);
    let mut ways: BTreeMap<(i64, i64), u32> = BTreeMap::new();
    for f in sim.falling() {
        let t = tops.iter().find(|t| (f.i as i64 - t.i as i64).abs() + (f.j as i64 - t.j as i64).abs() == 1).expect("beside its pillar");
        *ways.entry((f.i as i64 - t.i as i64, f.j as i64 - t.j as i64)).or_default() += 1;
    }
    println!("directions taken by 400 blocks: {ways:?}");
    assert_eq!(ways.len(), 4);
    assert!(ways.values().all(|&n| (70..=130).contains(&n)), "{ways:?}");
}

/// The direction does not depend on the order the blocks were placed or
/// notified in, or on which block's turn came first: every slide of a tick
/// is decided against the same world. Two heaps, the same cells placed in
/// opposite orders, come out the same; and two blocks facing one pit from
/// opposite sides both go.
#[test]
fn slides_do_not_depend_on_the_order_blocks_were_placed_or_ran() {
    let heap = |w: &World, o: BlockPos, reverse: bool| {
        let (_, _, sand, _) = ids(w);
        plate(w, o, 11, 11);
        let mut cells: Vec<BlockPos> = (3..8u32).flat_map(|dx| (3..8u32).flat_map(move |dy| (1..=4u32).map(move |dz| BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }))).collect();
        if reverse {
            cells.reverse();
        }
        for c in cells {
            assert!(w.set_block(c, sand));
        }
    };
    let (a, b) = (test_world(), test_world());
    let (g, focus) = site(&a);
    let o = BlockPos { k: g.k + 12, ..g };
    heap(&a, o, false);
    heap(&b, o, true);
    let (mut sa, mut sb) = (Sim::new(SimConfig::default()), Sim::new(SimConfig::default()));
    for _ in 0..200 {
        sa.step(&a, focus);
        sb.step(&b, focus);
        assert_eq!(sa.state_hash(&a), sb.state_hash(&b), "tick {}", sa.tick());
    }
    assert!(sa.stats.slides > 10, "{} slides", sa.stats.slides);
    // One pit one block wide between two sand blocks: decided against the
    // same world, both go in the same tick, whichever ran first.
    let w = test_world();
    let (_, _, sand, _) = ids(&w);
    let row = BlockPos { i: g.i - 4, j: g.j, k: g.k + 12, ..g };
    plate(&w, row, 3, 1);
    plate(&w, BlockPos { i: row.i + 4, ..row }, 3, 1);
    plate(&w, BlockPos { j: row.j - 1, ..row }, 7, 1);
    plate(&w, BlockPos { j: row.j + 1, ..row }, 7, 1);
    let left = BlockPos { i: row.i + 2, k: row.k + 1, ..row };
    let right = BlockPos { i: row.i + 4, k: row.k + 1, ..row };
    w.commit(&[(left, sand), (right, sand)]);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.stats.slides, 2, "both slid in one tick");
    assert_eq!(sim.falling().iter().filter(|f| f.i == row.i + 3).count(), 2, "into the same column");
    run_until_still(&w, &mut sim, focus, 400);
    assert_eq!(sim.stats.landings, sim.stats.slides, "and every slide ended, on the sloping ground below");
    assert_eq!(sim.stats.vanished, 0);
    // A pit one cell deep takes one of them. The other sees it filled and
    // stays: without a cell of its own, the second would stack above where
    // it started. Which one is `choice`'s, not the order they ran in.
    let w = test_world();
    let row = BlockPos { i: g.i - 4, j: g.j, k: g.k + 12, ..g };
    plate(&w, BlockPos { k: row.k - 1, ..row }, 7, 1);
    plate(&w, row, 3, 1);
    plate(&w, BlockPos { i: row.i + 4, ..row }, 3, 1);
    plate(&w, BlockPos { j: row.j - 1, ..row }, 7, 1);
    plate(&w, BlockPos { j: row.j + 1, ..row }, 7, 1);
    let air = ids(&w).0;
    assert!(w.set_block(BlockPos { i: row.i + 3, ..row }, air) || w.block_at(BlockPos { i: row.i + 3, ..row }) == air);
    w.commit(&[(left, sand), (right, sand)]);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.stats.slides, 1, "one pit cell, one block");
    run_until_still(&w, &mut sim, focus, 40);
    assert_eq!(sim.stats.slides, 1, "the other has no way down any more");
    assert_eq!(w.block_at(BlockPos { i: row.i + 3, ..row }), sand, "the pit is full");
    assert_eq!((w.block_at(left) == sand) as u32 + (w.block_at(right) == sand) as u32, 1);
    assert_powder_at_rest(&w, &sim);
}

/// Four blocks round one pit: the sum of their layers falls.
#[test]
fn four_blocks_round_a_pit_cannot_stack_above_where_they_started() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, _, sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    plate(&w, o, 5, 5);
    plate(&w, BlockPos { k: o.k + 1, ..o }, 5, 5);
    let pit = BlockPos { i: o.i + 2, j: o.j + 2, k: o.k + 1, ..o };
    assert!(w.set_block(pit, air));
    let round: Vec<BlockPos> = [(1, 2), (3, 2), (2, 1), (2, 3)].map(|(dx, dy)| BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + 2, ..o }).to_vec();
    for c in &round {
        assert!(w.set_block(*c, sand));
    }
    let mut sim = Sim::new(SimConfig::default());
    run_until_still(&w, &mut sim, focus, 100);
    assert_eq!(sim.stats.slides, 1);
    assert_eq!(w.block_at(pit), sand);
    assert_eq!(round.iter().filter(|c| w.block_at(**c) == sand).count(), 3);
    assert_powder_at_rest(&w, &sim);
}

/// Pour `count` blocks one at a time onto the middle of a plate.
fn pour_a_pile(count: u32) -> (World, BlockPos, Sim, DVec3) {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, _, sand, _) = ids(&w);
    let side = 61;
    let o = BlockPos { i: g.i - side / 2, j: g.j - side / 2, k: g.k + 16, ..g };
    plate(&w, o, side, side);
    let spout = BlockPos { i: g.i, j: g.j, k: o.k + 24, ..g };
    let mut sim = Sim::new(SimConfig::default());
    let mut placed = 0;
    for t in 0..60_000u32 {
        if placed < count && t % 2 == 0 {
            assert!(w.set_block(spout, sand));
            placed += 1;
        }
        sim.step(&w, focus);
        if placed == count && sim.queued() == 0 && sim.falling().is_empty() {
            return (w, o, sim, focus);
        }
    }
    panic!("the pile never settled: {} queued, {} falling", sim.queued(), sim.falling().len());
}

/// Height above the plate of every column of the pile.
fn pile_heights(w: &World, sim: &Sim, o: BlockPos) -> BTreeMap<(i64, i64), i64> {
    let mut h = BTreeMap::new();
    let _ = sim;
    for p in powder_cells(w) {
        if p.k > o.k {
            let e = h.entry((p.i as i64, p.j as i64)).or_insert(0);
            *e = (*e).max((p.k - o.k) as i64);
        }
    }
    h
}

/// Andrew's "settle into stable piles": a pile poured at one point comes to
/// rest, nothing moves after, no neighbouring columns differ by more than one
/// block, and the pile is as wide as that slope says. The slope is printed.
#[test]
fn a_poured_pile_settles_at_a_slope_of_one_block_per_block() {
    let count = 400;
    let (w, o, mut sim, focus) = pour_a_pile(count);
    assert_eq!(sim.stats.vanished, 0);
    assert_eq!(powder_cells(&w).len(), count as usize, "every block is still there");
    assert_powder_at_rest(&w, &sim);
    assert_stays_still(&w, &mut sim, focus, 40);
    let h = pile_heights(&w, &sim, o);
    let mut steepest = 0;
    for (&(i, j), &height) in &h {
        for (di, dj) in [(1, 0), (0, 1)] {
            let other = h.get(&(i + di, j + dj)).copied().unwrap_or(0);
            steepest = steepest.max((height - other).abs());
        }
    }
    assert_eq!(steepest, 1, "neighbouring columns differ by one block at most");
    let top = *h.values().max().unwrap();
    let (cx, cy) = h.iter().find(|(_, v)| **v == top).map(|(k, _)| *k).unwrap();
    let along = |dx: i64, dy: i64| -> Vec<i64> { (0..=top + 8).map(|d| h.get(&(cx + dx * d, cy + dy * d)).copied().unwrap_or(0)).collect() };
    println!("a pile of {count}: {} columns, height {top}, profile along +i {:?}, along the diagonal {:?}; {} slides in {} ticks", h.len(), along(1, 0), along(1, 1), sim.stats.slides, sim.tick());
    // The widest a slope of one can be: a diamond of height H holds 2 H^3 / 3 blocks.
    assert!(top >= 6 && top <= 14, "{top}");
}

/// Blocks dropped at one point on a plate that spans `centre`'s neighbours,
/// found by walking the lattice, so it works wherever the lattice is odd.
fn pile_across(w: &World, centre: BlockPos, radius: u32, count: u32) -> (Sim, usize) {
    let (_, stone, sand, _) = ids(w);
    let mut seen = vec![centre];
    let mut at = 0;
    for _ in 0..radius {
        let end = seen.len();
        for c in at..end {
            let p: BlockPos = seen[c];
            for n in w.neighbours(p) {
                if n.k == p.k && !seen.contains(&n) {
                    seen.push(n);
                }
            }
        }
        at = end;
    }
    let plate: Vec<(BlockPos, BlockId)> = seen.iter().map(|p| (*p, stone)).collect();
    w.commit(&plate);
    let focus = w.planet.pos(centre.face, centre.i as f64 + 0.5, centre.j as f64 + 0.5, centre.k as f64 + 2.0);
    let spout = BlockPos { k: centre.k + 8, ..centre };
    let mut sim = Sim::new(SimConfig::default());
    let mut placed = 0;
    for t in 0..20_000u32 {
        // Slowly: a block every half second. Ten a second outrun the slide,
        // which takes a column one block at a time, and stack up to the spout.
        if placed < count && t % 10 == 0 {
            assert!(w.set_block(spout, sand));
            placed += 1;
        }
        sim.step(w, focus);
        if placed == count && sim.queued() == 0 && sim.falling().is_empty() {
            assert_eq!(sim.stats.vanished, 0);
            assert_eq!(powder_cells(w).len(), count as usize, "every block is still there");
            assert_stays_still(w, &mut sim, focus, 20);
            return (sim, seen.len());
        }
    }
    panic!("the pile never settled");
}

/// Layers high in the air: plates there sit clear of any terrain.
const SKY: u32 = 1500;

#[test]
fn a_pile_forms_across_a_chunk_corner() {
    let w = test_world();
    // Where four chunks meet.
    let c = BlockPos { face: 2, i: 32 * 3000, j: 32 * 2000, k: 32 * 47 };
    let (sim, _) = pile_across(&w, c, 7, 60);
    assert_powder_at_rest(&w, &sim);
    let chunks: std::collections::BTreeSet<ChunkKey> = powder_cells(&w).iter().map(|p| World::chunk_of(*p).0).collect();
    assert!(chunks.len() >= 4, "the pile lies in four chunks and the layer's: {} chunks", chunks.len());
}

#[test]
fn a_pile_forms_across_a_cube_edge() {
    let w = test_world();
    let n = w.planet.n;
    let c = BlockPos { face: 0, i: n - 1, j: n / 2, k: SKY };
    let (sim, _) = pile_across(&w, c, 7, 60);
    assert_powder_at_rest(&w, &sim);
    let faces: std::collections::BTreeSet<u8> = powder_cells(&w).iter().map(|p| p.face).collect();
    assert_eq!(faces.len(), 2, "the pile lies on both faces: {faces:?}");
}

#[test]
fn a_pile_forms_on_a_cube_corner() {
    let w = test_world();
    let n = w.planet.n;
    let c = BlockPos { face: 0, i: n - 1, j: n - 1, k: SKY };
    let (sim, _) = pile_across(&w, c, 7, 60);
    assert_powder_at_rest(&w, &sim);
    let faces: std::collections::BTreeSet<u8> = powder_cells(&w).iter().map(|p| p.face).collect();
    assert_eq!(faces.len(), 3, "the pile lies on all three faces: {faces:?}");
}

/// The site chunk in the air and a one-metre region holding only it.
fn one_chunk(w: &World) -> (ChunkKey, (u32, u32, u32), DVec3, DVec3) {
    let (g, _) = site(w);
    // High enough that the chunk and the one below it are all air.
    let ck = World::chunk_of(BlockPos { k: g.k + 96, ..g }).0;
    let (i0, j0, k0) = ck.origin_blocks();
    let (i0, j0, k0) = (i0 as u32, j0 as u32, k0 as u32);
    let near = w.planet.pos(ck.face, i0 as f64 + 1.0, j0 as f64 + 16.0, k0 as f64 + 16.0);
    let both = w.planet.pos(ck.face, i0 as f64 + 32.0, j0 as f64 + 16.0, k0 as f64 + 16.0);
    (ck, (i0, j0, k0), near, both)
}

/// As a falling block does: a block whose slide would take it into a chunk
/// outside the active region stays exactly where it is, with its update
/// kept for when that chunk joins; it is saved, and it runs when the region
/// arrives. The same when only the cell below the free cell is outside.
#[test]
fn a_slide_waits_at_the_edge_of_the_active_region() {
    let w = test_world();
    let (ck, (i0, j0, k0), near, both) = one_chunk(&w);
    let (_, _, sand, _) = ids(&w);
    let next = ChunkKey { cx: ck.cx + 1, ..ck };
    // A plate up to the chunk's +i side, the drop in the next chunk.
    let o = BlockPos { face: ck.face, i: i0 + 27, j: j0 + 10, k: k0 + 8 };
    plate(&w, o, 5, 3);
    let edge = BlockPos { i: i0 + 31, j: j0 + 11, k: k0 + 9, ..o };
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..SimConfig::default() });
    sim.step(&w, near);
    assert!(sim.is_active(ck) && !sim.is_active(next));
    assert!(w.set_block(edge, sand));
    for _ in 0..20 {
        sim.step(&w, near);
    }
    assert_eq!(w.block_at(edge), sand, "it waits on the edge");
    assert_eq!(sim.stats.slides, 0);
    assert!(sim.falling().is_empty());
    assert_eq!(sim.queued(), 0, "waiting costs nothing");
    assert!(sim.frozen_count() > 0, "its update is kept");
    // It survives a save, and goes when the region reaches it.
    let w2 = copy_world(&w);
    let mut back = Sim::from_snapshot(sim.config, Snapshot::from_bytes(&sim.snapshot(&w).to_bytes()).unwrap());
    assert_eq!(back.state_hash(&w2), sim.state_hash(&w));
    back.step(&w2, both);
    assert!(back.is_active(next));
    assert_eq!(back.stats.slides, 1, "the region arrived and it went");
    assert_eq!(w2.block_at(edge), w2.generator.blocks.air);
    assert_eq!((back.falling()[0].i, back.falling()[0].j), (i0 + 32, j0 + 11), "over the old edge");

    // Only the cell under the free cell is outside: the plate's edge sits on
    // the chunk's bottom layer, so the free cell is in and its floor is not.
    let w = test_world();
    let (ck, (i0, j0, k0), _, _) = one_chunk(&w);
    let lower = ChunkKey { cz: ck.cz - 1, ..ck };
    let o = BlockPos { face: ck.face, i: i0 + 10, j: j0 + 10, k: k0 - 1 };
    plate(&w, o, 3, 1);
    let edge = BlockPos { i: i0 + 12, j: j0 + 10, k: k0, ..o };
    let (above, _) = (w.planet.pos(ck.face, i0 as f64 + 16.0, j0 as f64 + 16.0, k0 as f64 + 31.0), 0);
    let down = w.planet.pos(ck.face, i0 as f64 + 16.0, j0 as f64 + 16.0, k0 as f64 - 0.5);
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..SimConfig::default() });
    sim.step(&w, above);
    assert!(sim.is_active(ck) && !sim.is_active(lower));
    assert!(w.set_block(edge, sand));
    for _ in 0..20 {
        sim.step(&w, above);
    }
    assert_eq!(w.block_at(edge), sand, "the floor of the free cell is outside the region: it waits");
    assert_eq!(sim.stats.slides, 0);
    assert!(sim.frozen_count() > 0);
    sim.step(&w, down);
    assert!(sim.is_active(lower));
    assert_eq!(sim.stats.slides, 1, "and goes when the region reaches down");
}

/// A cut in a heap, saved in the middle of the collapse: the reload runs on
/// to exactly the state the uninterrupted run reaches.
#[test]
fn a_collapse_survives_a_save_and_reload_in_the_middle() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, stone, sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    plate(&w, o, 13, 13);
    // A 5 x 5 x 5 heap held in by a stone wall on its +i side.
    let cell = |dx: u32, dy: u32, dz: u32| BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o };
    let mut writes = Vec::new();
    for dy in 3..8 {
        for dz in 1..=5 {
            writes.push((cell(8, dy, dz), stone));
            writes.push((cell(2, dy, dz), stone));
            for dx in 3..8 {
                writes.push((cell(dx, dy, dz), sand));
            }
        }
        for dz in 1..=5 {
            for dx in 2..=8 {
                writes.push((cell(dx, 2, dz), stone));
                writes.push((cell(dx, 8, dz), stone));
            }
        }
    }
    w.commit(&writes);
    let mut sim = Sim::new(SimConfig::default());
    run_until_still(&w, &mut sim, focus, 30);
    assert_eq!(sim.stats.slides, 0, "boxed in");
    // Open the +i wall.
    let open: Vec<(BlockPos, BlockId)> = (3..8).flat_map(|dy| (1..=5).map(move |dz| (cell(8, dy, dz), air))).collect();
    w.commit(&open);
    for _ in 0..6 {
        sim.step(&w, focus);
    }
    assert!(!sim.falling().is_empty() && sim.stats.slides > 0, "mid-collapse");
    let w2 = copy_world(&w);
    let mut back = Sim::from_snapshot(sim.config, Snapshot::from_bytes(&sim.snapshot(&w).to_bytes()).unwrap());
    assert_eq!(back.state_hash(&w2), sim.state_hash(&w), "the reloaded state is the saved state");
    let ticks = run_until_still(&w, &mut sim, focus, 2000);
    for _ in 0..ticks {
        back.step(&w2, focus);
    }
    assert_eq!(back.state_hash(&w2), sim.state_hash(&w), "and it runs on to the same place");
    assert_powder_at_rest(&w, &sim);
    assert_eq!(powder_cells(&w).len(), 125);
    println!("the 125-block collapse: {} slides in {ticks} ticks", sim.stats.slides);
}

/// The pile's potential: the sum of every block's layer (in flight, the
/// layer it would be in at the top of its cell) never rises, tick by tick,
/// so a heap can only ever go down: no cycle, no jitter. A block that slid
/// is at least one layer lower when it lands.
#[test]
fn every_slide_ends_lower_so_a_heap_cannot_cycle() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, _, sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    plate(&w, o, 21, 21);
    let cells: Vec<BlockPos> = (8..13u32).flat_map(|dx| (8..13u32).flat_map(move |dy| (1..=8u32).map(move |dz| BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }))).collect();
    for c in &cells {
        assert!(w.set_block(*c, sand));
    }
    let height = |w: &World, sim: &Sim| -> i64 { powder_cells(w).iter().map(|p| p.k as i64).sum::<i64>() + sim.falling().iter().map(|f| f.z.ceil() as i64).sum::<i64>() };
    let mut sim = Sim::new(SimConfig::default());
    let mut h = height(&w, &sim);
    for _ in 0..3000 {
        sim.step(&w, focus);
        let now = height(&w, &sim);
        assert!(now <= h, "tick {}: the blocks' total height rose from {h} to {now}", sim.tick());
        h = now;
        if sim.queued() == 0 && sim.falling().is_empty() {
            break;
        }
    }
    assert_eq!(sim.queued(), 0);
    assert_powder_at_rest(&w, &sim);
    assert_eq!(powder_cells(&w).len() + sim.falling().len(), cells.len());
    println!("a tower of {} blocks spread with {} slides in {} ticks", cells.len(), sim.stats.slides, sim.tick());
}

/// A two-step stair inside walls: an upper shelf at `dz = 1` from x = 1 to
/// `shelf`, a lower floor from `shelf + 1` to `nx`. Cells are read with `cell`.
fn stair(w: &World, o: BlockPos, nx: u32, ny: u32, shelf: u32) {
    let (air, stone, ..) = ids(w);
    let mut writes = Vec::new();
    for dx in 0..nx + 2 {
        for dy in 0..ny + 2 {
            writes.push((BlockPos { i: o.i + dx, j: o.j + dy, ..o }, stone));
            let wall = dx == 0 || dy == 0 || dx == nx + 1 || dy == ny + 1;
            let on_shelf = (1..=shelf).contains(&dx);
            writes.push((BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + 1, ..o }, if wall || on_shelf { stone } else { air }));
            for dz in 2..=4 {
                writes.push((BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }, if wall { stone } else { air }));
            }
        }
    }
    w.commit(&writes);
}

/// Tick until settled, with the potential strictly falling on every tick
/// that moved a unit (a closed body of water); the ticks taken.
fn settle_water(w: &World, sim: &mut Sim, focus: DVec3, max_ticks: usize) -> usize {
    for t in 1..=max_ticks {
        let before = water_potential(&w.edits);
        let moved = sim.stats.water_moved;
        sim.step(w, focus);
        if sim.stats.water_moved > moved {
            let after = water_potential(&w.edits);
            assert!(after < before, "tick {}: potential {before} -> {after} while {} units moved", sim.tick(), sim.stats.water_moved - moved);
        }
        if sim.queued() == 0 && sim.falling().is_empty() {
            return t;
        }
    }
    panic!("not settled after {max_ticks} ticks: {} queued", sim.queued());
}

/// The cell at the ledge, level 1, that could not move at all under the
/// 2-level rule: it is 1 above the empty cell beside it.
#[test]
fn the_level_one_cell_at_a_ledge_goes_over_it() {
    let w = test_world();
    let (g, focus) = site(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    stair(&w, o, 7, 1, 3);
    let ledge = BlockPos { i: o.i + 3, j: o.j + 1, k: o.k + 2, ..o };
    let beside = BlockPos { i: o.i + 4, ..ledge };
    let lower = under(beside);
    assert!(w.set_water(ledge, w.water_id(), 1));
    let mut sim = Sim::new(SimConfig::default());
    let phi = water_potential(&w.edits);
    sim.step(&w, focus);
    assert_eq!(w.water_level_at(ledge), 0, "it went");
    assert_eq!(w.water_level_at(beside), 0, "through the free cell, down in one move");
    assert_eq!(w.water_level_at(lower), 1, "and landed in the cell below it");
    assert_eq!(water_total(&w.edits), 1);
    assert!(water_potential(&w.edits) < phi, "the potential fell, exactly as for a fall");
    assert_eq!(phi - water_potential(&w.edits), 1024);
    settle_water(&w, &mut sim, focus, 10);
    assert_stays_still(&w, &mut sim, focus, 20);
}

/// The ledge cell, whatever its level, gets to the lower space, and when the
/// water on a shelf has all gone over, nothing is left at the ledge.
#[test]
fn water_on_a_shelf_runs_over_the_ledge_and_the_potential_strictly_falls() {
    for start in [1u8, 2, 3, 7, 40, 255] {
        let w = test_world();
        let (g, focus) = site(&w);
        let o = BlockPos { k: g.k + 12, ..g };
        stair(&w, o, 9, 1, 3);
        let shelf = |dx: u32| BlockPos { i: o.i + dx, j: o.j + 1, k: o.k + 2, ..o };
        assert!(w.set_water(shelf(3), w.water_id(), start));
        let mut sim = Sim::new(SimConfig::default());
        let ticks = settle_water(&w, &mut sim, focus, 2000);
        assert_eq!(water_total(&w.edits), start as u64, "conserved");
        assert_eq!(w.water_level_at(shelf(3)), 0, "starting at {start}: the ledge cell ran dry");
        println!("level {start} at the ledge: settled in {ticks} ticks, shelf left {:?}", [w.water_level_at(shelf(1)), w.water_level_at(shelf(2)), w.water_level_at(shelf(3))]);
        assert_stays_still(&w, &mut sim, focus, 20);
    }
}

/// The free cell's floor may hold water: a shelf beside a pool with room in
/// its top cell drains into the pool, and stops when the pool's cell is full.
#[test]
fn a_ledge_over_a_pool_with_room_drains_into_it_until_it_is_full() {
    let w = test_world();
    let (g, focus) = site(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    stair(&w, o, 5, 1, 2);
    let water = w.water_id();
    let shelf = BlockPos { i: o.i + 2, j: o.j + 1, k: o.k + 2, ..o };
    let beside = BlockPos { i: o.i + 3, ..shelf };
    let pool = under(beside);
    // The pool's top cell one unit short of full, level across its floor.
    let pool_row: Vec<(BlockPos, BlockId, Option<u8>)> = (3..=5).map(|dx| (BlockPos { i: o.i + dx, ..pool }, water, Some(FULL))).collect();
    w.commit_levels(&pool_row);
    w.commit_levels(&[(pool, water, Some(FULL - 1))]);
    assert!(w.set_water(shelf, water, 1));
    let mut sim = Sim::new(SimConfig::default());
    settle_water(&w, &mut sim, focus, 50);
    assert_eq!(w.water_level_at(shelf), 0, "the room took it");
    assert_eq!(water_total(&w.edits), 3 * 255);
    // Full now: a second unit on the ledge has nowhere lower to go.
    assert!(w.set_water(shelf, water, 1));
    settle_water(&w, &mut sim, focus, 50);
    assert_eq!(w.water_level_at(shelf), 1, "a full pool is no lower open space");
}

/// A ledge that opens later: digging the cell below the free cell is not a
/// neighbour of the waiting block or water, so the free cell passes the news
/// on.
#[test]
fn a_ledge_opened_below_the_free_cell_wakes_what_rests_beside_it() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, _, sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    plate(&w, o, 5, 3);
    let wet = BlockPos { i: o.i + 1, j: o.j + 1, k: o.k + 1, ..o };
    let dry = BlockPos { i: o.i + 3, j: o.j + 1, k: o.k + 1, ..o };
    let walls: Vec<(BlockPos, BlockId)> = [(0, 1), (4, 1), (1, 0), (1, 2), (3, 0), (3, 2)].map(|(dx, dy)| (BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + 1, ..o }, ids(&w).1)).to_vec();
    w.commit(&walls);
    assert!(w.set_water(wet, w.water_id(), 1));
    assert!(w.set_block(dry, sand));
    let mut sim = Sim::new(SimConfig::default());
    run_until_still(&w, &mut sim, focus, 20);
    assert_eq!((w.water_level_at(wet), w.block_at(dry)), (1, sand), "both at rest");
    // Dig out the plate under the cell between them.
    let hole = BlockPos { i: o.i + 2, j: o.j + 1, ..o };
    assert!(w.set_block(hole, air));
    run_until_still(&w, &mut sim, focus, 60);
    assert_eq!(w.water_level_at(wet), 0, "the water went");
    assert_eq!(w.block_at(dry), air, "and so did the sand");
    assert_eq!(sim.stats.slides, 1);
}

/// With the slide off the old rules are back, exactly (`--no-slide`, and
/// what the scripts about falling straight down run under).
#[test]
fn with_the_slide_off_nothing_slides() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, _, sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 12, ..g };
    stair(&w, o, 7, 1, 3);
    let ledge = BlockPos { i: o.i + 3, j: o.j + 1, k: o.k + 2, ..o };
    assert!(w.set_water(ledge, w.water_id(), 1));
    let edge = BlockPos { j: o.j + 3, ..ledge };
    let _ = (edge, sand);
    let mut sim = Sim::new(SimConfig { flow: Flow::before(), ..SimConfig::default() });
    run_until_still(&w, &mut sim, focus, 20);
    assert_eq!(w.water_level_at(ledge), 1, "stuck, as it was");
    assert_eq!(sim.stats.water_moved, 0);
}

/// How far one collapse goes. A settled pile stands at exactly the slope the
/// rule allows, so this is the worst shape there is for a cascade: take one
/// block out of its foot, again and again, and count the blocks that move.
/// `cargo test --release one_block_from_a_pile -- --nocapture` prints them.
#[test]
fn one_block_from_a_pile_moves_a_few_blocks_and_never_the_whole_pile() {
    let count = 300;
    let (w, o, mut sim, focus) = pour_a_pile(count);
    let air = ids(&w).0;
    let g = BlockPos { k: o.k + 1, ..o };
    let foot: Vec<(i64, i64)> = pile_heights(&w, &sim, o).iter().filter(|(_, h)| **h == 2).map(|(c, _)| *c).collect();
    assert!(foot.len() >= 8, "{} columns of height 2", foot.len());
    let mut moved = Vec::new();
    for &(i, j) in foot.iter().step_by(foot.len() / 8).take(8) {
        let before = sim.stats.slides + sim.stats.falls;
        assert!(w.set_block(BlockPos { i: i as u32, j: j as u32, ..g }, air));
        let ticks = run_until_still(&w, &mut sim, focus, 1000);
        moved.push((sim.stats.slides + sim.stats.falls - before, ticks));
        assert_powder_at_rest(&w, &sim);
    }
    // The worst single cut: the block under the middle of the pile.
    let h = pile_heights(&w, &sim, o);
    let (&(i, j), &top) = h.iter().max_by_key(|(c, h)| (**h, **c)).unwrap();
    let before = sim.stats.slides + sim.stats.falls;
    assert!(w.set_block(BlockPos { i: i as u32, j: j as u32, ..g }, air));
    let ticks = run_until_still(&w, &mut sim, focus, 2000);
    let middle = sim.stats.slides + sim.stats.falls - before;
    println!("{count} blocks, {} columns: blocks moved and ticks after a cut at the foot {moved:?}; under the {top}-high middle: {middle} moved in {ticks} ticks", h.len());
    assert!(moved.iter().all(|m| m.0 < count as u64 / 4), "{moved:?}");
    assert_powder_at_rest(&w, &sim);
}

/// Not part of `cargo test`: the tick cost of a big collapse, a block of
/// 10,800 sand set down on a table with nothing holding its sides. Run alone:
/// `cargo test --release -- --ignored stress_sand_collapse --nocapture`.
#[test]
#[ignore]
fn stress_sand_collapse() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, _, sand, _) = ids(&w);
    let side = 91;
    let o = BlockPos { i: g.i - side / 2, j: g.j - side / 2, k: g.k + 30, ..g };
    plate(&w, o, side, side);
    let t0 = Instant::now();
    let cells: Vec<(BlockPos, BlockId)> = (30..60u32).flat_map(|dx| (30..60u32).flat_map(move |dy| (1..=12u32).map(move |dz| (BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }, sand)))).collect();
    assert_eq!(w.commit(&cells), 10_800);
    println!("placed 10,800 sand blocks in {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);
    let mut sim = Sim::new(SimConfig::default());
    let (mut worst, mut sum, mut ticks, mut peak) = (0u32, 0u64, 0u64, 0usize);
    for _ in 0..20_000 {
        sim.step(&w, focus);
        ticks += 1;
        if ticks > 1 {
            worst = worst.max(sim.stats.last_us);
            sum += sim.stats.last_us as u64;
        }
        peak = peak.max(sim.falling().len());
        if ticks % 100 == 0 || ticks <= 3 {
            println!("tick {ticks:5}: {:8.3} ms  in flight {:5}  queued {:5}  slides {:7}", sim.stats.last_us as f64 / 1e3, sim.falling().len(), sim.queued(), sim.stats.slides);
        }
        if sim.queued() == 0 && sim.falling().is_empty() {
            break;
        }
    }
    println!("settled in {ticks} ticks ({:.1} s): {} slides, {} falls, {} landings, {} vanished, peak {peak} in flight, worst tick after the first {:.3} ms, average {:.3} ms", ticks as f64 / 20.0, sim.stats.slides, sim.stats.falls, sim.stats.landings, sim.stats.vanished, worst as f64 / 1e3, sum as f64 / ticks as f64 / 1e3);
    assert_eq!(sim.stats.vanished, 0);
    assert_eq!(powder_cells(&w).len(), 10_800);
    assert_powder_at_rest(&w, &sim);
}

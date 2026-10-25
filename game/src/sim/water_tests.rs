//! M2b: Andrew's water rule. Settling with a strictly falling potential,
//! conservation, the boundary of the active region, a lake drained by a
//! channel, displacement, and "the simulation runs only where something
//! changed".

use super::tests::{ids, site};
use super::water::{Boundary, FULL, water_potential, water_total};
use super::*;
use crate::world::tests::test_world;

/// A stone basin floating in the air above the site: a floor plate at layer
/// `o.k` over (o.i .. o.i + n + 1, o.j .. o.j + n + 1), walls `depth` high
/// around an n x n interior of air. Interior cells are
/// (o.i + 1 ..= o.i + n, o.j + 1 ..= o.j + n, o.k + 1 ..= o.k + depth).
fn build_basin(w: &World, o: BlockPos, n: u32, depth: u32) {
    build_box(w, o, n, n, depth);
}

/// `build_basin` with an nx x ny interior.
fn build_box(w: &World, o: BlockPos, nx: u32, ny: u32, depth: u32) {
    let (air, stone, ..) = ids(w);
    let mut writes = Vec::new();
    for dx in 0..nx + 2 {
        for dy in 0..ny + 2 {
            writes.push((BlockPos { i: o.i + dx, j: o.j + dy, ..o }, stone));
            let wall = dx == 0 || dy == 0 || dx == nx + 1 || dy == ny + 1;
            for dz in 1..=depth {
                writes.push((BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }, if wall { stone } else { air }));
            }
        }
    }
    w.commit(&writes);
}

/// Tick until settled, recording the cost; prints the worst and the average
/// tick while water moved, and returns (ticks, worst us, avg us while moving).
fn time_until_settled(w: &World, sim: &mut Sim, focus: DVec3, label: &str, max_ticks: usize) -> (usize, u32, f64) {
    let mut costs: Vec<(u64, u32, u32, u64, u32, u32, u32)> = Vec::new();
    let mut ticks = 0;
    for _ in 0..max_ticks {
        let moved = sim.stats.water_moved;
        sim.step(w, focus);
        ticks += 1;
        let s = &sim.stats;
        costs.push((sim.tick(), s.last_us, s.water_updates_last, s.water_moved - moved, s.water_rule_us, s.water_commit_us, s.absorb_us));
        if sim.queued() == 0 && sim.falling().is_empty() {
            break;
        }
    }
    let worst = costs.iter().map(|c| c.1).max().unwrap_or(0);
    let moving: Vec<&(u64, u32, u32, u64, u32, u32, u32)> = costs.iter().filter(|c| c.3 > 0).collect();
    let avg = moving.iter().map(|c| c.1 as f64).sum::<f64>() / moving.len().max(1) as f64;
    let avg_of = |f: &dyn Fn(&(u64, u32, u32, u64, u32, u32, u32)) -> u32| moving.iter().map(|c| f(c) as f64).sum::<f64>() / moving.len().max(1) as f64 / 1e3;
    println!(
        "{label}: {ticks} ticks ({} with water moving), worst tick {:.3} ms, average moving tick {:.3} ms (rule {:.3}, commit {:.3}, absorb {:.3}), {} units moved, queued at the end {}",
        moving.len(),
        worst as f64 / 1e3,
        avg / 1e3,
        avg_of(&|c| c.4),
        avg_of(&|c| c.5),
        avg_of(&|c| c.6),
        sim.stats.water_moved,
        sim.queued()
    );
    for c in costs.iter().filter(|c| c.1 >= worst / 2 || c.0 % 25 == 0).take(40) {
        println!("  tick {:4}: {:8.3} ms (rule {:7.3} commit {:7.3} absorb {:7.3})  {:6} water updates  {:8} units moved", c.0, c.1 as f64 / 1e3, c.4 as f64 / 1e3, c.5 as f64 / 1e3, c.6 as f64 / 1e3, c.2, c.3);
    }
    (ticks, worst, avg)
}

/// Fill one interior layer of a basin to the brim.
fn fill_layer(w: &World, o: BlockPos, n: u32, dz: u32) {
    let water = w.water_id();
    let writes: Vec<(BlockPos, BlockId, Option<u8>)> = (1..=n).flat_map(|dx| (1..=n).map(move |dy| (BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }, water, Some(FULL)))).collect();
    assert_eq!(w.commit_levels(&writes), (n * n) as usize);
}

/// Every water cell in the edit store with its level.
fn water_cells(w: &World) -> Vec<(BlockPos, u8)> {
    let mut out = Vec::new();
    for c in w.edits.snapshot_full() {
        let Some(levels) = &c.water else { continue };
        let (i0, j0, k0) = c.key.origin_blocks();
        for x in 0..CHUNK {
            for y in 0..CHUNK {
                for z in 0..CHUNK {
                    let l = levels[crate::world::chunk::index(x, y, z)];
                    if l > 0 {
                        out.push((BlockPos { face: c.key.face, i: i0 as u32 + x as u32, j: j0 as u32 + y as u32, k: k0 as u32 + z as u32 }, l));
                    }
                }
            }
        }
    }
    out
}

/// Water in the edit store minus the generator's water in the same chunks:
/// what the rule, the player and the boundary changed, in units.
fn water_delta(w: &World) -> i64 {
    let mut d = 0i64;
    for c in w.edits.snapshot_full() {
        let now: i64 = c.water.as_ref().map_or(0, |l| l.iter().map(|&x| x as i64).sum());
        let natural: i64 = w.natural(c.key).water_dense().map_or(0, |l| l.iter().map(|&x| x as i64).sum());
        d += now - natural;
    }
    d
}

/// Andrew's resting conditions over every water cell inside the active
/// region: nothing replaceable and unfull below, and no lateral replaceable
/// neighbour (in the region) more than one level lower.
fn assert_at_rest(w: &World, sim: &Sim) {
    let reg = &w.registry;
    for (p, l) in water_cells(w) {
        if !sim.is_active(World::chunk_of(p).0) {
            continue;
        }
        if p.k > 0 {
            let below = BlockPos { k: p.k - 1, ..p };
            if sim.is_active(World::chunk_of(below).0) {
                let (bid, bl) = w.block_and_level_at(below);
                assert!(!reg.is_replaceable(bid) || bl == FULL, "{p:?} at {l} rests on {bid} at {bl}");
            }
        }
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            let Some(n) = w.resolve_block(p.face, p.i as i64 + di, p.j as i64 + dj, p.k as i64) else { continue };
            if !sim.is_active(World::chunk_of(n).0) {
                continue;
            }
            let (nid, nl) = w.block_and_level_at(n);
            if reg.is_replaceable(nid) {
                assert!(l as i32 - nl as i32 <= 1, "{p:?} at {l} leans {} over {n:?} at {nl}", l as i32 - nl as i32);
            }
        }
    }
}

/// Tick until nothing is queued and nothing falls, checking the potential
/// strictly falls on every tick in which the rule moved water (a closed
/// body of water: no boundary flows). Returns the ticks taken.
fn settle(w: &World, sim: &mut Sim, focus: DVec3, max_ticks: usize) -> usize {
    let mut moving_ticks = 0;
    for t in 1..=max_ticks {
        let before = water_potential(&w.edits);
        let moved = sim.stats.water_moved;
        sim.step(w, focus);
        assert_eq!(sim.stats.water_in + sim.stats.water_out, 0, "a closed body");
        if sim.stats.water_moved > moved {
            moving_ticks += 1;
            let after = water_potential(&w.edits);
            assert!(after < before, "tick {}: potential {before} -> {after} while {} units moved", sim.tick(), sim.stats.water_moved - moved);
        }
        if sim.queued() == 0 && sim.falling().is_empty() {
            assert!(moving_ticks > 0, "something moved");
            return t;
        }
    }
    panic!("not settled after {max_ticks} ticks: {} queued, {} moved", sim.queued(), sim.stats.water_moved);
}

#[test]
fn one_cell_spreads_evenly_then_the_basin_levels_and_rests() {
    let w = test_world();
    let (g, focus) = site(&w);
    let o = BlockPos { i: g.i - 3, j: g.j - 3, k: g.k + 10, ..g };
    build_basin(&w, o, 5, 2);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.queued(), 0, "stone and air settle in one tick");
    // One full cell on the floor's centre.
    let centre = BlockPos { i: o.i + 3, j: o.j + 3, k: o.k + 1, ..o };
    assert!(w.set_water(centre, w.water_id(), FULL));
    let phi0 = water_potential(&w.edits);
    assert_eq!(phi0, (1024 * centre.k as u64 + 255) * 255);
    sim.step(&w, focus);
    // Tick 1: the cell and its four neighbours at 51 each, exactly.
    let lv = |di: i64, dj: i64| w.water_level_at(BlockPos { i: (centre.i as i64 + di) as u32, j: (centre.j as i64 + dj) as u32, ..centre });
    assert_eq!([lv(0, 0), lv(-1, 0), lv(1, 0), lv(0, -1), lv(0, 1)], [51; 5]);
    assert_eq!(lv(1, 1), 0, "diagonals wait a tick");
    assert_eq!(sim.stats.water_moved, 204);
    assert!(water_potential(&w.edits) < phi0);
    let ticks = settle(&w, &mut sim, focus, 200);
    println!("single cell settled in {} ticks", ticks + 1);
    assert_eq!(water_total(&w.edits), 255, "conserved");
    assert_eq!(sim.stats.water_balance(), 0);
    let cells = water_cells(&w);
    assert_eq!(cells.len(), 25, "every floor cell holds water: {cells:?}");
    assert!(cells.iter().all(|(_, l)| (6..=15).contains(l)), "{cells:?}");
    assert_at_rest(&w, &sim);
    // Resting water costs nothing: no updates, no changes to the world (the
    // state hash moves with the tick counter, so the edit store's generation
    // and the processed count are the checks).
    let generation = w.edits.generation();
    let processed = sim.stats.processed;
    for _ in 0..20 {
        sim.step(&w, focus);
        assert_eq!(sim.stats.water_updates_last, 0);
    }
    assert_eq!(sim.stats.processed, processed);
    assert_eq!(w.edits.generation(), generation);
    assert_eq!(sim.queued(), 0);
    assert!(sim.stats.last_us < 500, "an idle tick is microseconds: {} us", sim.stats.last_us);
}

#[test]
fn a_pour_falls_as_a_column_then_spreads_and_the_potential_strictly_falls() {
    let w = test_world();
    let (g, focus) = site(&w);
    let o = BlockPos { i: g.i - 4, j: g.j - 4, k: g.k + 8, ..g };
    build_basin(&w, o, 7, 6);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    // A column of four full cells in the air, two cells above the floor.
    let x = BlockPos { i: o.i + 4, j: o.j + 4, k: o.k + 3, ..o };
    for dz in 0..4 {
        assert!(w.set_water(BlockPos { k: x.k + dz, ..x }, w.water_id(), FULL));
    }
    sim.step(&w, focus);
    // The column fell one cell as a unit: nothing spread.
    for dz in 0..4 {
        assert_eq!(w.water_level_at(BlockPos { k: x.k - 1 + dz, ..x }), FULL, "layer {dz} moved down");
    }
    assert_eq!(w.water_level_at(BlockPos { k: x.k + 3, ..x }), 0);
    assert_eq!(water_cells(&w).len(), 4, "no sideways spread while falling");
    sim.step(&w, focus);
    // Tick 2: another cell down as a unit; the bottom reaches the floor and,
    // with nothing below it to fall into, spreads in the same tick (down
    // first, then sideways); the cells above it still have room below and wait.
    assert_eq!(w.water_level_at(BlockPos { k: o.k + 1, ..x }), 51);
    for dz in 1..4 {
        assert_eq!(w.water_level_at(BlockPos { k: o.k + 1 + dz, ..x }), FULL, "layer {dz} waits to fall");
    }
    assert_eq!(water_cells(&w).len(), 8, "the bottom cell and its four neighbours, three full cells above");
    let ticks = settle(&w, &mut sim, focus, 400);
    println!("four-cell pour settled in {} ticks, {} units moved", ticks + 2, sim.stats.water_moved);
    assert_eq!(water_total(&w.edits), 4 * 255, "conserved");
    assert_at_rest(&w, &sim);
    // 1020 units over a 7 x 7 floor: one layer, 20 or 21 per cell, within a lean of one.
    let cells = water_cells(&w);
    assert_eq!(cells.len(), 49, "{}", cells.len());
    assert!(cells.iter().all(|(p, _)| p.k == o.k + 1));
    let (lo, hi) = cells.iter().fold((255u8, 0u8), |(lo, hi), (_, l)| (lo.min(*l), hi.max(*l)));
    assert!(hi - lo <= 7, "levels {lo}..{hi}: a tilt steeper than the basin is wide");
}

#[test]
fn a_full_basin_rests_until_touched_and_a_lake_drains_through_a_channel_to_its_sill() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, ..) = ids(&w);
    // A 5 x 5 lake, three layers deep, floating ten layers up, and beside it
    // along +i a wider, deeper basin whose rim is level with the lake's: the
    // channel through the shared wall drains the lake into it.
    let o = BlockPos { i: g.i - 3, j: g.j - 3, k: g.k + 10, ..g };
    let ob = BlockPos { i: o.i + 6, j: o.j - 2, k: o.k - 4, ..o };
    build_basin(&w, ob, 9, 7);
    build_basin(&w, o, 5, 3);
    for dz in 1..=3 {
        fill_layer(&w, o, 5, dz);
    }
    let total = 75 * 255;
    assert_eq!(water_total(&w.edits), total);
    let mut sim = Sim::new(SimConfig::default());
    // Generated-at-rest: every placed cell is notified once, nothing moves, then silence.
    sim.step(&w, focus);
    assert_eq!(sim.stats.water_moved, 0, "a brimful lake does not move");
    assert_eq!(sim.queued(), 0);
    let generation = w.edits.generation();
    let processed = sim.stats.processed;
    for _ in 0..10 {
        sim.step(&w, focus);
    }
    assert_eq!((w.edits.generation(), sim.stats.processed, sim.queued()), (generation, processed, 0), "silence");
    assert_eq!(sim.stats.water_updates_last, 0);
    // Open the channel: the wall cell on the +i side at the surface layer.
    let sill = BlockPos { i: o.i + 6, j: o.j + 3, k: o.k + 3, ..o };
    assert!(w.set_block(sill, air));
    let ticks = settle(&w, &mut sim, focus, 5000);
    println!("lake drained through the channel and settled in {ticks} ticks, {} units moved", sim.stats.water_moved);
    assert_eq!(water_total(&w.edits), total, "conserved");
    // Below the sill the lake is untouched; the surface layer drained to a
    // film that leans one level per cell up from the sill (Andrew's resting
    // lean: the rule leaves the steepest slope it allows).
    let mut surface = Vec::new();
    for dx in 1..=5 {
        for dy in 1..=5 {
            for dz in 1..=2 {
                assert_eq!(w.water_level_at(BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }), FULL, "layer {dz} stays full");
            }
            let top = w.water_level_at(BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + 3, ..o });
            let dist = (6 - dx) + (3i32 - dy as i32).unsigned_abs() as u32;
            assert!(top as u32 <= dist + 1, "surface cell ({dx}, {dy}), {dist} from the sill, drained to {top}");
            surface.push(top);
        }
    }
    println!("lake surface after draining: {surface:?}");
    assert!(w.water_level_at(sill) <= 1, "the sill cell holds a film at most: {}", w.water_level_at(sill));
    // What left lies on the lower basin's floor, one layer, within a lean of one.
    let lower: Vec<(BlockPos, u8)> = water_cells(&w).into_iter().filter(|(p, _)| p.i > o.i + 6).collect();
    let outside: u64 = lower.iter().map(|(_, l)| *l as u64).sum();
    let surface_left: u64 = water_cells(&w).iter().filter(|(p, _)| p.k == o.k + 3 && p.i <= o.i + 6).map(|(_, l)| *l as u64).sum();
    assert_eq!(outside + surface_left + 50 * 255, total);
    assert!(outside >= 24 * 255, "most of the surface layer left: {outside} units outside, {surface_left} on the surface");
    assert!(lower.iter().all(|(p, _)| p.k == ob.k + 1), "on the lower floor");
    assert_eq!(lower.len(), 81, "spread over the whole lower floor");
    assert_at_rest(&w, &sim);
}

#[test]
fn a_falling_block_displaces_water_upward_and_a_sealed_pool_loses_it() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, stone, sand, _) = ids(&w);
    let o = BlockPos { i: g.i - 2, j: g.j - 2, k: g.k + 10, ..g };
    build_basin(&w, o, 3, 3);
    fill_layer(&w, o, 3, 1);
    fill_layer(&w, o, 3, 2);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.stats.water_moved, 0);
    // Sand dropped over the centre falls through the water to the floor.
    let centre = BlockPos { i: o.i + 2, j: o.j + 2, k: o.k + 6, ..o };
    assert!(w.set_block(centre, sand));
    let mut landed = false;
    for _ in 0..40 {
        sim.step(&w, focus);
        if sim.stats.landings == 1 {
            landed = true;
            break;
        }
    }
    assert!(landed);
    assert_eq!(w.block_at(BlockPos { k: o.k + 1, ..centre }), sand, "on the floor");
    assert_eq!(sim.stats.water_displaced, 255);
    assert_eq!(sim.stats.water_lost, 0);
    assert_eq!(water_total(&w.edits), 18 * 255, "the water is all still there");
    settle(&w, &mut sim, focus, 200);
    assert_eq!(water_total(&w.edits), 18 * 255);
    assert_eq!(sim.stats.water_balance(), 0);
    // The displaced cell spread over the third layer.
    let third: u64 = (1..=3).flat_map(|dx| (1..=3).map(move |dy| (dx, dy))).map(|(dx, dy)| w.water_level_at(BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + 3, ..o }) as u64).sum();
    assert_eq!(third, 255);
    assert_at_rest(&w, &sim);
    // A sealed pool: a stone lid, then a block placed into the water has nowhere to send it.
    let o2 = BlockPos { i: g.i + 6, j: g.j - 2, k: g.k + 10, ..g };
    build_basin(&w, o2, 1, 1);
    fill_layer(&w, o2, 1, 1);
    let cell = BlockPos { i: o2.i + 1, j: o2.j + 1, k: o2.k + 1, ..o2 };
    assert!(w.set_block(BlockPos { k: cell.k + 1, ..cell }, stone));
    let (writes, lost) = w.displace_writes(cell);
    assert!(writes.is_empty());
    assert_eq!(lost, 255, "no room above: lost, and reported");
    // An open pool: the same placement pushes the water past the full second
    // layer into the third (about 28 per cell after the spread) and what does
    // not fit there into the air above the basin, nothing lost.
    let target = BlockPos { i: o.i + 1, j: o.j + 1, k: o.k + 1, ..o };
    let third_before = w.water_level_at(BlockPos { k: o.k + 3, ..target }) as u32;
    let (writes, lost) = w.displace_writes(target);
    assert_eq!(lost, 0);
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!((writes[0].0.k, writes[0].2), (o.k + 3, Some(FULL)));
    assert_eq!((writes[1].0.k, writes[1].2), (o.k + 4, Some((255 - (255 - third_before)) as u8)));
}

/// A sea shore on face 1: an ocean column and the land column beside it
/// along +i, both at the sea's surface layer, with the land solid there and
/// the sea surface cell water. Rows of j are tried until one has a shore
/// inside the face with solid land at the surface layer.
fn find_shore(w: &World) -> (BlockPos, BlockPos) {
    let p = w.planet;
    let k = p.sea_level - 1;
    for j in (50000..p.n).step_by(3001) {
        let ocean = |i: u32| w.generator.column(p.dir(1, i as f64 + 0.5, j as f64 + 0.5), p.block_size).ocean;
        let Some(sea_i) = (1000..p.n).step_by(2048).find(|&i| ocean(i)) else { continue };
        let mut i = sea_i;
        while i + 1 < p.n && ocean(i) {
            i += 1;
        }
        if ocean(i) {
            continue; // the sea runs to the face edge on this row
        }
        let land = BlockPos { face: 1, i, j, k };
        let sea = BlockPos { face: 1, i: i - 1, j, k };
        if w.block_at(sea) == w.water_id() && w.registry.is_solid(w.block_at(land)) && w.registry.is_solid(w.block_at(BlockPos { k: k - 1, ..land })) {
            return (sea, land);
        }
    }
    panic!("no shore found on face 1");
}

/// Review 3 #6: pouring into a full sea cell changes nothing, so it must
/// leave nothing behind: no edited chunk (a clean one would be in the hash
/// but never saved, so a reload changed the hash), and a chunk dug and put
/// back is the natural chunk as far as the hash goes.
#[test]
fn a_write_that_changes_nothing_leaves_no_chunk_and_no_trace_in_the_hash() {
    let w = test_world();
    let (sea, land) = find_shore(&w);
    let sim = Sim::new(SimConfig::default());
    let before = sim.state_hash(&w);
    assert_eq!(w.block_and_level_at(sea), (w.water_id(), FULL));
    assert!(!w.set_water(sea, w.water_id(), FULL), "the cell is already full");
    assert!(!w.set_block(BlockPos { k: sea.k + 30, ..sea }, w.generator.blocks.air), "the air over the sea is air");
    assert_eq!(w.edits.len(), 0, "a write that changed nothing leaves no edited chunk behind");
    assert_eq!(w.edits.dirty_count(), 0);
    assert_eq!(sim.state_hash(&w), before, "and nothing in the hash");
    // Dig a block of the shore and put it back: the chunk is edited (and
    // saved), its content is the generator's, and so is its hash.
    let stone = w.block_at(land);
    assert!(w.set_block(land, w.generator.blocks.air));
    assert!(w.set_block(land, stone));
    assert_eq!(w.edits.len(), 1);
    let _ = w.edits.take_changed();
    assert_eq!(sim.state_hash(&w), before, "the hash is of content, not of history");
}

#[test]
fn a_pit_dug_at_the_shore_fills_from_the_sea_and_the_sea_stays_full() {
    let w = test_world();
    let (sea, land) = find_shore(&w);
    println!("shore: sea {sea:?}, land {land:?} (the m2b-water.play site)");
    let (air, ..) = ids(&w);
    let p = w.planet;
    let focus = p.pos(land.face, land.i as f64 + 0.5, land.j as f64 + 0.5, land.k as f64 + 2.0);
    let mut sim = Sim::new(SimConfig::default());
    // An untouched sea costs nothing.
    for _ in 0..5 {
        sim.step(&w, focus);
    }
    assert_eq!(sim.stats.processed, 0);
    // A 3 x 3 pit, two deep, in the land right beside the sea surface cell.
    let mut writes = Vec::new();
    for di in 0..3 {
        for dj in -1..=1i64 {
            for dk in 0..2 {
                writes.push((BlockPos { i: land.i + di, j: (land.j as i64 + dj) as u32, k: land.k - dk, ..land }, air));
            }
        }
    }
    let dug = w.commit(&writes);
    assert!(dug >= 15, "{dug} cells dug (some may have been air)");
    // The sea is a natural body that becomes edited as it moves, so the
    // potential over edited chunks jumps when a chunk joins; the strict-fall
    // check is the closed-basin tests' job. Here: settles, and accounts.
    let mut t = 0;
    let mut worst_us = 0;
    loop {
        sim.step(&w, focus);
        t += 1;
        worst_us = worst_us.max(sim.stats.last_us);
        if sim.queued() == 0 || t > 2000 {
            break;
        }
    }
    println!("shore pit: settled in {t} ticks, {} units moved, in {} out {}, worst tick {:.2} ms", sim.stats.water_moved, sim.stats.water_in, sim.stats.water_out, worst_us as f64 / 1e3);
    assert!(t < 2000, "settled");
    // Accounting: the water inside the region changed by exactly what crossed the boundary.
    assert_eq!(water_delta(&w), sim.stats.water_in as i64 - sim.stats.water_out as i64);
    assert_eq!(sim.stats.water_out, 0, "the ocean has nowhere to drain to");
    // The pit holds water: its floor cells are full, its surface cells nearly so.
    for di in 0..3 {
        assert_eq!(w.water_level_at(BlockPos { i: land.i + di, k: land.k - 1, ..land }), FULL, "pit floor {di}");
        assert!(w.water_level_at(BlockPos { i: land.i + di, k: land.k, ..land }) >= 200, "pit surface {di}");
    }
    // The sea beside the pit dipped (a dish a few levels deep, within a lean
    // of one per cell); far away it is full.
    let mut dish = Vec::new();
    for d in 1..=60u32 {
        let l = w.water_level_at(BlockPos { i: sea.i - d, ..sea });
        dish.push(l);
        assert!(l >= FULL - 30, "sea {d} m out at {l}");
        if d >= 40 {
            assert_eq!(l, FULL, "sea {d} m out stays full");
        }
    }
    println!("sea levels out from the pit: {dish:?}");
    assert_at_rest(&w, &sim);
}

#[test]
fn boundary_cells_read_as_reservoirs_walls_or_drains_and_are_never_written() {
    let w = test_world();
    let (sea, land) = find_shore(&w);
    let (air, ..) = ids(&w);
    let p = w.planet;
    let mut sim = Sim::new(SimConfig::default());
    let focus = p.pos(land.face, land.i as f64 + 0.5, land.j as f64 + 0.5, land.k as f64 + 2.0);
    sim.step(&w, focus);
    assert_eq!(sim.boundary_cell(&w, land), Boundary::Wall, "rock");
    assert_eq!(sim.boundary_cell(&w, BlockPos { k: land.k + 3, ..land }), Boundary::Wall, "air");
    assert_eq!(sim.boundary_cell(&w, sea), Boundary::Water { level: FULL, drains: false }, "generated sea: a reservoir that does not drain");
    let drained = BlockPos { i: sea.i - 3, ..sea };
    assert!(w.set_water(drained, w.water_id(), 100));
    assert_eq!(sim.boundary_cell(&w, drained), Boundary::Water { level: 100, drains: false }, "a drained cell supplies at its own level and absorbs nothing");
    let removed = BlockPos { i: sea.i - 4, ..sea };
    assert!(w.set_block(removed, air));
    assert_eq!(sim.boundary_cell(&w, removed), Boundary::Wall, "water the player removed is a frozen hole, not a sink");

    // A tiny region: one chunk. Everything beyond it is injected: first a
    // river cell at level 10 that drains, then a reservoir at 200, then a wall.
    let (g, _) = site(&w);
    let ck = World::chunk_of(BlockPos { k: g.k + 12, ..g }).0;
    let (i0, j0, k0) = ck.origin_blocks();
    let (i0, j0, k0) = (i0 as u32, j0 as u32, k0 as u32);
    // The focus near the chunk's -i side: the chunk is active, its +i neighbour is not.
    let centre = p.pos(ck.face, i0 as f64 + 1.0, j0 as f64 + 16.0, k0 as f64 + 16.0);
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..SimConfig::default() });
    sim.step(&w, centre);
    assert!(sim.is_active(ck) && !sim.is_active(ChunkKey { cx: ck.cx + 1, ..ck }), "{} chunks active", sim.active_chunks());
    // A 3 x 3 basin against the chunk's +i side: its wall there is the next chunk's first column.
    let o = BlockPos { face: ck.face, i: i0 + 28, j: j0 + 10, k: k0 + 4 };
    build_basin(&w, o, 3, 2);
    fill_layer(&w, o, 3, 1);
    fill_layer(&w, o, 3, 2);
    let edge_i = i0 + 32; // the first column outside the active chunk
    assert_eq!(o.i + 4, edge_i, "the +i wall is the inactive column");
    // The injected river runs along the pool's bottom layer only (a river
    // cell has air above it, not another river cell).
    let outside: Vec<BlockPos> = (1..=3).flat_map(|dy| (1..=2).map(move |dz| BlockPos { i: edge_i, j: o.j + dy, k: o.k + dz, ..o })).collect();
    let before = water_delta(&w);
    let outside_before: Vec<(BlockId, u8)> = outside.iter().map(|&b| w.block_and_level_at(b)).collect();
    let is_outside = move |bp: BlockPos| bp.i == edge_i && (o.j + 1..=o.j + 3).contains(&bp.j) && bp.k == o.k + 1;
    sim.boundary_override = Some(Box::new(move |bp| is_outside(bp).then_some(Boundary::Water { level: 10, drains: true })));
    // Touch the water beside the edge so it is scheduled.
    assert!(w.set_water(BlockPos { i: o.i + 3, j: o.j + 1, k: o.k + 2, ..o }, w.water_id(), 254));
    let mut drain_ticks = 0;
    for _ in 0..3000 {
        sim.step(&w, centre);
        drain_ticks += 1;
        if sim.queued() == 0 {
            break;
        }
    }
    println!("pool drained into the river cells in {drain_ticks} ticks");
    for dz in (1..=2).rev() {
        for dy in 1..=3 {
            let row: Vec<String> = (1..=3).map(|dx| format!("{:4}", w.water_level_at(BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }))).collect();
            println!("  layer {dz} row {dy}: {}", row.join(""));
        }
    }
    println!("  out {}, moved {}, queued {:?}", sim.stats.water_out, sim.stats.water_moved, sim.queue.iter().take(8).map(|(d, k)| (d - sim.tick(), k.pos().i as i64 - o.i as i64, k.pos().j as i64 - o.j as i64, k.pos().k as i64 - o.k as i64)).collect::<Vec<_>>());
    assert_eq!(sim.queued(), 0, "settled");
    assert!(sim.stats.water_out > 0, "water left into the river cells: {}", sim.stats.water_out);
    assert_eq!(sim.stats.water_in, 0);
    assert_eq!(water_delta(&w) - before, -(sim.stats.water_out as i64) - 1, "the pool lost exactly what left (and the one unit the touch removed)");
    for (b, was) in outside.iter().zip(&outside_before) {
        assert_eq!(w.block_and_level_at(*b), *was, "the inactive cell {b:?} is never written");
    }
    // The pool emptied down toward the river's level: its cells beside the edge are near 10, the rest at most a lean above.
    for dy in 1..=3 {
        for dz in 1..=2 {
            let l = w.water_level_at(BlockPos { i: o.i + 3, j: o.j + dy, k: o.k + dz, ..o });
            assert!(l <= 11, "cell beside the drain at {l}");
        }
    }
    let after_drain = water_total(&w.edits) as i64;
    // Now a reservoir at 200: the pool fills back from the edge, up to 199.
    sim.boundary_override = Some(Box::new(move |bp| is_outside(bp).then_some(Boundary::Water { level: 200, drains: false })));
    let touch = BlockPos { i: o.i + 3, j: o.j + 1, k: o.k + 1, ..o };
    let touched = w.water_level_at(touch) as i64 - 5;
    assert!(w.set_water(touch, w.water_id(), touched as u8));
    let out_before = sim.stats.water_out;
    let in_before = sim.stats.water_in;
    let mut fill_ticks = 0;
    for _ in 0..3000 {
        sim.step(&w, centre);
        fill_ticks += 1;
        if sim.queued() == 0 {
            break;
        }
    }
    println!("pool refilled from the reservoir in {fill_ticks} ticks");
    assert_eq!(sim.queued(), 0, "settled");
    assert_eq!(sim.stats.water_out, out_before, "a reservoir absorbs nothing");
    assert!(sim.stats.water_in > in_before);
    assert_eq!(water_total(&w.edits) as i64, after_drain + (sim.stats.water_in - in_before) as i64 - 5);
    for dy in 1..=3 {
        let l = w.water_level_at(BlockPos { i: o.i + 3, j: o.j + dy, k: o.k + 1, ..o });
        assert!((198..=199).contains(&l), "cell beside the reservoir at {l}");
        let above = w.water_level_at(BlockPos { i: o.i + 3, j: o.j + dy, k: o.k + 2, ..o });
        assert!(above <= 1, "nothing fills the layer above a reservoir below full: {above}");
    }
    // A wall: nothing happens at the edge however the pool is disturbed.
    sim.boundary_override = Some(Box::new(move |bp| is_outside(bp).then_some(Boundary::Wall)));
    let total = water_total(&w.edits);
    let (in0, out0) = (sim.stats.water_in, sim.stats.water_out);
    assert!(w.set_water(BlockPos { i: o.i + 3, j: o.j + 2, k: o.k + 1, ..o }, w.water_id(), 100));
    for _ in 0..200 {
        sim.step(&w, centre);
    }
    assert_eq!((sim.stats.water_in, sim.stats.water_out), (in0, out0));
    assert!(water_total(&w.edits) < total, "the touch removed water and nothing replaced it");
    for (b, was) in outside.iter().zip(&outside_before) {
        assert_eq!(w.block_and_level_at(*b), *was);
    }
}

/// Not part of `cargo test`: the tick cost of a large pour (a 20 x 20 x 5
/// block of water, 2,000 cells, released five layers above a basin floor)
/// and a dam break (a 50 x 20 x 10 lake, 10,000 cells, whose wall into an
/// equal empty compartment is removed at once).
/// `cargo test --release -- --ignored water_stress --nocapture`.
#[test]
#[ignore]
fn water_stress_large_pour_and_dam_break() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, stone, ..) = ids(&w);
    // The pour.
    let o = BlockPos { i: g.i - 20, j: g.j - 20, k: g.k + 8, ..g };
    build_basin(&w, o, 40, 12);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    let water = w.water_id();
    let mut writes = Vec::new();
    for dx in 11..=30 {
        for dy in 11..=30 {
            for dz in 6..=10 {
                writes.push((BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }, water, Some(FULL)));
            }
        }
    }
    assert_eq!(w.commit_levels(&writes), 2000);
    let total = 2000 * 255;
    let (ticks, worst, _) = time_until_settled(&w, &mut sim, focus, "pour of 2,000 cells", 2000);
    assert!(ticks < 2000, "settled");
    assert_eq!(water_total(&w.edits), total, "conserved");
    assert_at_rest(&w, &sim);
    // 510,000 units over a 40 x 40 floor: 318.75 per column, so one full layer and a second at 63 or 64.
    let second: Vec<u8> = (1..=40).flat_map(|dx| (1..=40).map(move |dy| (dx, dy))).map(|(dx, dy)| w.water_level_at(BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + 2, ..o })).collect();
    let (lo, hi) = second.iter().fold((255u8, 0u8), |(lo, hi), &l| (lo.min(l), hi.max(l)));
    println!("pour: second layer levels {lo}..{hi}, worst tick {:.3} ms", worst as f64 / 1e3);

    // The dam break: two compartments of 50 x 20 x 10, the left one full.
    let w = test_world();
    let o = BlockPos { i: g.i - 50, j: g.j - 10, k: g.k + 8, ..g };
    build_box(&w, o, 100, 20, 10);
    let mut writes = Vec::new();
    for dy in 1..=20 {
        for dz in 1..=10 {
            writes.push((BlockPos { i: o.i + 51, j: o.j + dy, k: o.k + dz, ..o }, stone));
        }
    }
    w.commit(&writes);
    let mut writes = Vec::new();
    for dx in 1..=50 {
        for dy in 1..=20 {
            for dz in 1..=10 {
                writes.push((BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }, water, Some(FULL)));
            }
        }
    }
    assert_eq!(w.commit_levels(&writes), 10_000);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    assert_eq!(sim.stats.water_moved, 0, "the lake rests behind its dam");
    let first_tick_us = sim.stats.last_us;
    println!("dam break: the lake's 10,000 cells notified once: {:.3} ms, then at rest", first_tick_us as f64 / 1e3);
    let mut writes = Vec::new();
    for dy in 1..=20 {
        for dz in 1..=10 {
            writes.push((BlockPos { i: o.i + 51, j: o.j + dy, k: o.k + dz, ..o }, air));
        }
    }
    assert_eq!(w.commit(&writes), 200);
    let total = 10_000 * 255;
    let max_ticks: usize = std::env::var("WATER_STRESS_TICKS").ok().and_then(|s| s.parse().ok()).unwrap_or(4000);
    let (ticks, worst, _) = time_until_settled(&w, &mut sim, focus, "dam break of 10,000 cells", max_ticks);
    if ticks >= max_ticks {
        println!("dam break: NOT settled within {max_ticks} ticks; {} queued", sim.queued());
        return;
    }
    assert_eq!(water_total(&w.edits), total, "conserved");
    assert_at_rest(&w, &sim);
    // 10,000 cells over 101 x 20 columns: four full layers and most of a fifth.
    for dx in [1u32, 50, 51, 52, 100] {
        let col: Vec<u8> = (1..=10).map(|dz| w.water_level_at(BlockPos { i: o.i + dx, j: o.j + 10, k: o.k + dz, ..o })).collect();
        println!("dam break: column {dx} levels {col:?}");
        assert!(col[..4].iter().all(|&l| l == FULL), "four full layers at column {dx}: {col:?}");
    }
    println!("dam break: worst tick {:.3} ms", worst as f64 / 1e3);
}

#[test]
fn water_waits_for_its_period_and_the_period_is_in_the_hash() {
    let w = test_world();
    let (g, focus) = site(&w);
    let o = BlockPos { i: g.i - 3, j: g.j - 3, k: g.k + 10, ..g };
    build_basin(&w, o, 5, 2);
    let centre = BlockPos { i: o.i + 3, j: o.j + 3, k: o.k + 1, ..o };
    let mut sim = Sim::new(SimConfig { water_period: 4, ..SimConfig::default() });
    sim.step(&w, focus);
    assert!(w.set_water(centre, w.water_id(), FULL));
    // Placed before tick 2: it waits for tick 4.
    sim.step(&w, focus);
    assert_eq!(sim.tick(), 2);
    assert_eq!(w.water_level_at(centre), FULL);
    assert_eq!(sim.stats.water_moved, 0);
    assert!(sim.queued() > 0, "requeued for the next water step");
    sim.step(&w, focus);
    assert_eq!(sim.stats.water_moved, 0);
    sim.step(&w, focus);
    assert_eq!(sim.tick(), 4);
    assert_eq!(w.water_level_at(centre), 51, "moved on tick 4");
    for _ in 0..3 {
        sim.step(&w, focus);
        assert_eq!(w.water_level_at(centre), 51, "and not between");
    }
    sim.step(&w, focus);
    assert_ne!(w.water_level_at(centre), 51, "tick 8 moves again");
    // The period is in the hash (review 3 #4: this test never checked it).
    let mut other = Sim::from_snapshot(SimConfig { water_period: 1, ..sim.config }, sim.snapshot(&w));
    assert_ne!(other.state_hash(&w), sim.state_hash(&w), "the same state under another period hashes differently");
    other.config.water_period = 4;
    assert_eq!(other.state_hash(&w), sim.state_hash(&w));
}

/// The mesher's sloped surfaces (mesh.rs `Surface`): corners shared between
/// neighbouring water cells carry one height, side faces against air meet
/// the top face at the same two corners, and the corner formula is the
/// documented one.
#[test]
fn sloped_water_surfaces_share_corners_and_are_watertight() {
    use crate::mesh::{MeshOptions, mesh_chunk_levels};
    use std::collections::HashMap;
    let w = test_world();
    let (g, focus) = site(&w);
    // A basin inside one chunk: its interior sits in the chunk's middle.
    let ck = World::chunk_of(BlockPos { k: g.k + 12, ..g }).0;
    let (i0, j0, k0) = ck.origin_blocks();
    let o = BlockPos { face: ck.face, i: i0 as u32 + 8, j: j0 as u32 + 8, k: k0 as u32 + 6 };
    build_basin(&w, o, 9, 4);
    let mut sim = Sim::new(SimConfig::default());
    sim.step(&w, focus);
    let centre = BlockPos { i: o.i + 5, j: o.j + 5, k: o.k + 1, ..o };
    assert!(w.set_water(centre, w.water_id(), FULL));
    sim.step(&w, focus); // the plus shape of 51s
    let made = w.generate(ck);
    let levels = w.padded_levels(ck, &made).expect("partial water gives the mesher levels");
    let m = mesh_chunk_levels(&made, Some(&levels), &w.registry, &w.planet, MeshOptions::default());
    assert!(!m.water.is_empty());
    let (cx, cy, cz) = (centre.i - i0 as u32, centre.j - j0 as u32, centre.k - k0 as u32);
    // Decode: lattice corner (x, y) at layer z -> height, from every face.
    let mut corners: HashMap<(u32, u32, u32), u8> = HashMap::new();
    let mut tops = 0;
    let mut sides = 0;
    let mut note = |key: (u32, u32, u32), h: u8, what: &str| {
        if let Some(prev) = corners.insert(key, h) {
            assert_eq!(prev, h, "corner {key:?} from a {what} face disagrees: {prev} vs {h}");
        }
    };
    for q in &m.water {
        let lo = q[0];
        let (axis, sign, slice, u0, v0, qw, qh) = (lo & 3, (lo >> 2) & 1, (lo >> 3) & 31, (lo >> 8) & 31, (lo >> 13) & 31, ((lo >> 18) & 31) + 1, ((lo >> 23) & 31) + 1);
        let h = |c: u32| ((q[2] >> (8 * c)) & 0xFF) as u8;
        match axis {
            2 if sign == 1 => {
                tops += 1;
                for cu in 0..2 {
                    for cv in 0..2 {
                        note((u0 + cu * qw, v0 + cv * qh, slice), h(cu + 2 * cv), "top");
                    }
                }
            }
            2 => assert_eq!(q[2], crate::mesh::WATER_FLAT, "bottom faces are flat"),
            _ => {
                if qh != 1 {
                    assert_eq!(q[2], crate::mesh::WATER_FLAT, "a merged side column is full water");
                    continue;
                }
                sides += 1;
                assert_eq!(h(0), 255);
                assert_eq!(h(1), 255);
                for cu in 0..2 {
                    let (x, y) = if axis == 0 { (slice + sign, u0 + cu * qw) } else { (u0 + cu * qw, slice + sign) };
                    note((x, y, v0), h(2 + cu), "side");
                }
            }
        }
    }
    assert!(tops >= 5 && sides >= 8, "{tops} top faces, {sides} side faces");
    // The formula at two corners: the centre's +i+j corner averages three 51s and one air cell;
    // the +i cell's outer -j corner averages one 51 and three air cells.
    assert_eq!(corners[&(cx + 1, cy + 1, cz)], ((3 * 51 + 2) / 4) as u8, "{corners:?}");
    assert_eq!(corners[&(cx + 2, cy, cz)], ((51 + 2) / 4) as u8);
    // Full water meshes as it always did: no levels array, flat heights.
    let far = ChunkKey { cz: ck.cz + 20, ..ck };
    assert!(w.padded_levels(far, &w.generate(far)).is_none());
}

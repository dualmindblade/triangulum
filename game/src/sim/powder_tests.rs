//! B7, Andrew's #8: powder that holds a level and rests at an angle
//! (`--powder levels`, powder.rs).

use super::powder::{WHOLE, powder_potential, powder_total, units};
use super::tests::{copy_world, ids, site};
use super::*;
use crate::world::tests::test_world;
use std::collections::BTreeMap;

fn levels(deg: f64) -> SimConfig {
    SimConfig { flow: Flow { powder: Powder::Levels(Repose::of_degrees(deg).unwrap()), ..Flow::default() }, ..SimConfig::default() }
}

/// Layers high in the air: plates there sit clear of any terrain.
const SKY: u32 = 1500;

fn plate(w: &World, o: BlockPos, nx: u32, ny: u32) {
    let (_, stone, ..) = ids(w);
    let writes: Vec<(BlockPos, BlockId)> = (0..nx).flat_map(|dx| (0..ny).map(move |dy| (BlockPos { i: o.i + dx, j: o.j + dy, ..o }, stone))).collect();
    w.commit(&writes);
}

/// A site in the sky: a stone plate `size` wide centred at (i, j), at the
/// hills' face, and a focus standing beside it.
fn table(w: &World, size: u32) -> (BlockPos, DVec3) {
    let (g, _) = site(w);
    let o = BlockPos { i: g.i - size / 2, j: g.j - size / 2, k: g.k + 24, ..g };
    plate(w, o, size, size);
    let c = BlockPos { i: g.i, j: g.j, k: o.k + 1, ..g };
    (c, w.planet.pos(c.face, c.i as f64 + 0.5, c.j as f64 + 0.5, c.k as f64))
}

/// The top of the powder in a column, in units over layer `base`'s floor.
fn top_of(w: &World, bp: BlockPos, base: u32, reach: u32) -> Option<i64> {
    (base..base + reach).rev().find_map(|k| {
        let (id, level) = w.block_and_level_at(BlockPos { k, ..bp });
        w.registry.has_gravity(id).then(|| 255 * (k - base) as i64 + units(level) as i64)
    })
}

/// Tick until nothing is queued or in flight; each tick that moved powder
/// must lower the potential, which counts a block in flight by where it is.
fn settle(w: &World, sim: &mut Sim, focus: DVec3, max: usize) -> usize {
    let reg = w.registry.clone();
    let phi = |sim: &Sim| powder_potential(&w.edits, &reg) as f64 + sim.falling().iter().map(|f| (510.0 * f.z + 255.0) * 255.0).sum::<f64>();
    let total = |sim: &Sim| powder_total(&w.edits, &reg) + 255 * sim.falling().len() as u64;
    let start = total(sim);
    let mut last = phi(sim);
    for t in 1..=max {
        let moved = sim.stats.powder_moved;
        sim.step(w, focus);
        let now = phi(sim);
        if sim.stats.powder_moved != moved {
            assert!(now < last, "tick {t} moved powder and the potential went {last} to {now}");
        } else {
            assert!(now <= last, "tick {t}: the potential rose, {last} to {now}");
        }
        last = now;
        assert_eq!(total(sim), start, "tick {t}: powder appeared or vanished");
        if sim.queued() == 0 && sim.falling().is_empty() {
            return t;
        }
    }
    panic!("not still after {max} ticks: {} queued, {} falling, {} moves", sim.queued(), sim.falling().len(), sim.stats.powder_moves);
}

fn assert_stays_still(w: &World, sim: &mut Sim, focus: DVec3, ticks: usize) {
    let generation = w.edits.generation();
    let processed = sim.stats.processed;
    for _ in 0..ticks {
        sim.step(w, focus);
    }
    assert_eq!((sim.stats.processed - processed, w.edits.generation() - generation, sim.queued()), (0, 0, 0), "no jitter");
}

/// The height of every powder column over (c.i, c.j) +- `r`, and the steepest
/// neighbouring differences along an axis and a diagonal, over the columns
/// that hold powder.
fn profile(w: &World, c: BlockPos, r: i64, reach: u32) -> (BTreeMap<(i64, i64), i64>, i64, i64) {
    let mut h = BTreeMap::new();
    for di in -r..=r {
        for dj in -r..=r {
            if let Some(t) = top_of(w, BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, ..c }, c.k, reach) {
                h.insert((di, dj), t);
            }
        }
    }
    let (mut straight, mut diagonal) = (0, 0);
    for (&(a, b), &t) in &h {
        for (d, ways) in [(0, [(1, 0), (0, 1)]), (1, [(1, 1), (1, -1)])] {
            for (x, y) in ways {
                if let Some(&n) = h.get(&(a + x, b + y)) {
                    let step = (t - n).abs();
                    if d == 0 {
                        straight = straight.max(step);
                    } else {
                        diagonal = diagonal.max(step);
                    }
                }
            }
        }
    }
    (h, straight, diagonal)
}

/// The default game's settings text is exactly what it was: it goes into the
/// state hash, saves and replays.
#[test]
fn the_default_flow_text_is_what_it_was() {
    assert_eq!(Flow::default().text(), "slide=on film=seek+dry reach=8 dry=200");
    assert_eq!(Flow::before().text(), "slide=off film=keep reach=8 dry=200");
    assert_eq!(Flow::default().powder, Powder::Blocks);
    assert_eq!(SimConfig::default().flow.powder, Powder::Blocks);
}

#[test]
fn the_settings_round_trip_and_the_angle_has_limits() {
    let f = levels(32.5).flow;
    assert_eq!(f.text(), "slide=on film=seek+dry reach=8 dry=200 powder=levels angle=32.5");
    let words: Vec<&str> = "slide=on film=seek+dry reach=8 dry=200 powder=levels angle=32.5".split_whitespace().collect();
    let text = f.text();
    assert_eq!(Flow::parse(&text.split_whitespace().collect::<Vec<_>>()).unwrap(), f);
    assert_eq!(Flow::parse(&words).unwrap(), f);
    assert_eq!(Flow::parse(&["powder=levels"]).unwrap().powder, Powder::Levels(Repose::default()));
    let r = Repose::of_degrees(45.0).unwrap();
    assert_eq!((r.straight, r.diagonal), (255, 361));
    assert_eq!(Repose::of_degrees(35.0).unwrap().straight, 179);
    for bad in [0.0, 0.5, 70.5, 90.0, f64::NAN, -10.0] {
        assert!(Repose::of_degrees(bad).is_err(), "{bad}");
    }
    assert!(Flow::parse(&["angle=91"]).is_err());
    assert!(Repose::of_degrees(Repose::MIN_DEG).unwrap().straight >= 1, "the proof needs S >= 1");
}

/// The rule, at each angle: a tower comes to rest with no neighbour past the
/// slope, close to it, and with every unit of powder still there.
#[test]
fn a_tower_comes_to_rest_at_its_angle() {
    for deg in [20.0, 35.0, 45.0, 60.0] {
        let w = test_world();
        let (_, _, sand, _) = ids(&w);
        let (c, focus) = table(&w, 41);
        let writes: Vec<(BlockPos, BlockId)> = (0..12).map(|d| (BlockPos { k: c.k + d, ..c }, sand)).collect();
        w.commit(&writes);
        let mut sim = Sim::new(levels(deg));
        let t = settle(&w, &mut sim, focus, 3000);
        let Powder::Levels(r) = sim.config.flow.powder else { unreachable!() };
        let (h, straight, diagonal) = profile(&w, c, 20, 16);
        assert_eq!(powder_total(&w.edits, &w.registry), 12 * 255, "{deg}: every unit is there");
        let toe = 2 * super::powder::LEAST_SPREAD as i64;
        assert!(straight <= r.straight as i64 + toe && diagonal <= r.diagonal as i64 + toe, "{deg}: past the slope: {straight} > {} or {diagonal} > {}", r.straight, r.diagonal);
        assert!(straight * 10 >= r.straight as i64 * 8, "{deg}: a pile stands near the limit: {straight} of {}", r.straight);
        let top = h.values().max().unwrap();
        println!("{deg} degrees: 12 blocks settle in {t} ticks, {} moves, {} columns, {:.2} high, steepest {straight} of {} (axis), {diagonal} of {} (diagonal)", sim.stats.powder_moves, h.len(), *top as f64 / 255.0, r.straight, r.diagonal);
        assert_stays_still(&w, &mut sim, focus, 30);
    }
}

/// A pile is a slope, not a stair: its columns take many heights, and it is
/// about as wide along a diagonal as along an axis.
#[test]
fn a_pile_is_close_to_round_and_has_no_preferred_way() {
    let w = test_world();
    let (_, _, sand, _) = ids(&w);
    let (c, focus) = table(&w, 61);
    let writes: Vec<(BlockPos, BlockId)> = (0..60).map(|d| (BlockPos { k: c.k + d, ..c }, sand)).collect();
    w.commit(&writes);
    let mut sim = Sim::new(levels(35.0));
    let t = settle(&w, &mut sim, focus, 8000);
    let (h, _, _) = profile(&w, c, 30, 64);
    let reach = |x: i64, y: i64| (1..30).take_while(|&d| h.contains_key(&(x * d, y * d))).count() as i64 + 1;
    let along = [reach(1, 0), reach(-1, 0), reach(0, 1), reach(0, -1)];
    let across = [reach(1, 1), reach(1, -1), reach(-1, 1), reach(-1, -1)];
    println!("60 blocks: {t} ticks, {} columns; reach along the axes {along:?}, along the diagonals {across:?} (x 1.41)", h.len());
    let (lo, hi) = (*along.iter().min().unwrap(), *along.iter().max().unwrap());
    assert!(hi - lo <= 2, "the four axes agree: {along:?}");
    let diag = across.iter().sum::<i64>() as f64 * std::f64::consts::SQRT_2 / 4.0;
    let axis = along.iter().sum::<i64>() as f64 / 4.0;
    assert!((diag / axis - 1.0).abs() < 0.2, "round to 20%: {diag} against {axis}");
    let heights: std::collections::BTreeSet<i64> = h.values().map(|v| v % 255).collect();
    assert!(heights.len() > 10, "a slope has many levels, a stair one: {}", heights.len());
}

/// A cell holds what it was given, and moves down by levels when nothing
/// holds it up: a partial cell in the air falls a cell a tick and lands whole.
#[test]
fn a_partial_cell_falls_by_levels() {
    let w = test_world();
    let (c, focus) = table(&w, 9);
    let (air, _, sand, _) = ids(&w);
    let high = BlockPos { k: c.k + 6, ..c };
    assert!(w.commit_levels(&[(high, sand, Some(100))]) == 1);
    let mut sim = Sim::new(levels(35.0));
    sim.step(&w, focus);
    assert_eq!(w.block_and_level_at(high), (air, 0), "it left its cell");
    assert_eq!(w.block_and_level_at(BlockPos { k: high.k - 2, ..high }), (sand, 100), "two cells down, in one tick");
    assert!(sim.falling().is_empty(), "a partial cell is not a falling block");
    settle(&w, &mut sim, focus, 100);
    assert_eq!(powder_total(&w.edits, &w.registry), 100);
    assert!(w.block_at(c) == sand, "it reached the plate");
}

/// A whole block on a partial cell: it fills it from above, and what is left
/// stays where it landed.
#[test]
fn a_whole_block_landing_on_a_partial_cell_fills_it() {
    let w = test_world();
    let (c, focus) = table(&w, 21);
    let (air, stone, sand, _) = ids(&w);
    // Walled in on all eight sides, one wide, so only the vertical can happen.
    for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
        let wall: Vec<(BlockPos, BlockId)> = (0..4).map(|d| (BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, k: c.k + d, ..c }, stone)).collect();
        w.commit(&wall);
    }
    assert!(w.commit_levels(&[(c, sand, Some(120))]) == 1);
    let mut sim = Sim::new(levels(35.0));
    sim.step(&w, focus);
    let drop = BlockPos { k: c.k + 3, ..c };
    assert!(w.set_block(drop, sand));
    settle(&w, &mut sim, focus, 60);
    assert_eq!(sim.stats.landings, 1, "the whole block fell as a block");
    assert_eq!(w.block_and_level_at(c), (sand, 0), "the partial cell is full");
    assert_eq!(w.block_and_level_at(BlockPos { k: c.k + 1, ..c }), (sand, 120), "and the rest stays on it");
    assert_eq!(w.block_at(drop), air);
}

/// Water and powder: falling powder pushes a cell's water up, none is lost,
/// and partial powder is a floor and a wall to water.
#[test]
fn powder_meeting_water_pushes_it_up_and_none_is_lost() {
    let w = test_world();
    let (c, focus) = table(&w, 15);
    let (_, stone, sand, _) = ids(&w);
    let water = w.water_id();
    // A basin three wide, two deep, full of water.
    for di in -2i64..=2 {
        for dj in -2i64..=2 {
            let at = |d: u32| BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, k: c.k + d, ..c };
            if di.abs() == 2 || dj.abs() == 2 {
                w.commit(&[(at(0), stone), (at(1), stone), (at(2), stone), (at(3), stone)]);
            } else {
                w.commit_levels(&[(at(0), water, Some(255)), (at(1), water, Some(255))]);
            }
        }
    }
    let wet_units = |w: &World| -> u64 {
        let mut t = 0;
        for d in w.edits.snapshot_full() {
            if let Some(l) = &d.water {
                t += l.iter().zip(&d.ids).filter(|(_, id)| **id == water).map(|(l, _)| *l as u64).sum::<u64>();
            }
        }
        t
    };
    let before = wet_units(&w);
    assert_eq!(before, 18 * 255);
    let mut sim = Sim::new(levels(35.0));
    // A partial cell over the middle of the pool, and a block.
    let over = BlockPos { k: c.k + 3, ..c };
    w.commit_levels(&[(over, sand, Some(90))]);
    w.set_block(BlockPos { k: c.k + 5, ..c }, sand);
    for _ in 0..200 {
        sim.step(&w, focus);
        assert_eq!(wet_units(&w) + sim.stats.water_lost, before, "water is only ever moved");
    }
    assert_eq!(sim.stats.water_lost, 0);
    assert!(sim.stats.water_displaced > 0, "the powder took cells the water held");
    assert_eq!(powder_total(&w.edits, &w.registry) + 255 * sim.falling().len() as u64, 90 + 255);
    assert!(w.registry.has_gravity(w.block_at(c)), "powder reached the bottom");
    // Water rests on partial powder and does not go through it.
    let mut wet_over_powder = 0;
    for di in -1i64..=1 {
        for dj in -1i64..=1 {
            let at = BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, ..c };
            let (id, level) = w.block_and_level_at(at);
            if id == sand && level != 0 {
                wet_over_powder += (w.water_level_at(BlockPos { k: at.k + 1, ..at }) > 0) as u32;
            }
        }
    }
    assert!(wet_over_powder > 0, "the pool stands on the powder on its bed");
}

/// A cell holds one id, so two powders do not share one.
#[test]
fn two_powders_do_not_mix() {
    let w = test_world();
    let (c, focus) = table(&w, 21);
    let (_, _, sand, gravel) = ids(&w);
    let beside = BlockPos { i: c.i + 1, ..c };
    w.commit_levels(&[(c, sand, Some(200)), (beside, gravel, Some(10))]);
    let mut sim = Sim::new(levels(5.0));
    settle(&w, &mut sim, focus, 2000);
    let (_, bl) = w.block_and_level_at(beside);
    assert_eq!(w.block_at(beside), gravel);
    assert!(bl != 0, "gravel kept its id and took no sand");
    assert_eq!(powder_total(&w.edits, &w.registry), 210);
}

/// Andrew's "pour on one spot": a block every half second builds a pile whose
/// top is at the spout's column, at the slope all the way up.
#[test]
fn pouring_on_one_spot_builds_a_pile_at_the_angle() {
    let w = test_world();
    let (c, focus) = table(&w, 41);
    let (_, _, sand, _) = ids(&w);
    let spout = BlockPos { k: c.k + 14, ..c };
    let mut sim = Sim::new(levels(35.0));
    let mut placed = 0;
    let mut t = 0;
    loop {
        if placed < 40 && t % 10 == 0 && w.block_at(spout) != sand {
            assert!(w.set_block(spout, sand));
            placed += 1;
        }
        sim.step(&w, focus);
        t += 1;
        if placed == 40 && sim.queued() == 0 && sim.falling().is_empty() {
            break;
        }
        assert!(t < 4000, "never settled");
    }
    let Powder::Levels(r) = sim.config.flow.powder else { unreachable!() };
    let (h, straight, diagonal) = profile(&w, c, 20, 16);
    assert_eq!(powder_total(&w.edits, &w.registry), 40 * 255);
    let toe = 2 * super::powder::LEAST_SPREAD as i64;
    assert!(straight <= r.straight as i64 + toe && diagonal <= r.diagonal as i64 + toe, "{straight} {diagonal}");
    println!("40 poured: {t} ticks, {} columns, {:.2} high", h.len(), *h.values().max().unwrap() as f64 / 255.0);
    let centre = h[&(0, 0)];
    assert_eq!(centre, *h.values().max().unwrap(), "the top of the pile is under the spout");
}

/// Dig at the foot of a settled pile: its slope follows, including the
/// diagonal neighbours and the top of the pile, none of which heard the dig.
#[test]
fn digging_at_the_foot_brings_the_slope_down() {
    let w = test_world();
    let (c, focus) = table(&w, 41);
    let (air, _, sand, _) = ids(&w);
    let writes: Vec<(BlockPos, BlockId)> = (0..60).map(|d| (BlockPos { k: c.k + d, ..c }, sand)).collect();
    w.commit(&writes);
    let mut sim = Sim::new(levels(35.0));
    settle(&w, &mut sim, focus, 6000);
    let (h, ..) = profile(&w, c, 20, 20);
    // Dig the column two cells out from the top, down to the plate.
    let at = BlockPos { i: c.i + 2, ..c };
    let writes: Vec<(BlockPos, BlockId)> = (0..8).map(|d| (BlockPos { k: c.k + d, ..at }, air)).collect();
    w.commit(&writes);
    let moved = sim.stats.powder_moves;
    settle(&w, &mut sim, focus, 2000);
    let (after, straight, _) = profile(&w, c, 20, 20);
    let Powder::Levels(r) = sim.config.flow.powder else { unreachable!() };
    assert!(sim.stats.powder_moves > moved, "it moved");
    assert!(straight <= r.straight as i64 + 2 * super::powder::LEAST_SPREAD as i64, "back at rest within the slope: {straight}");
    assert!(after[&(0, 0)] < h[&(0, 0)], "the top came down: {} from {}", after[&(0, 0)], h[&(0, 0)]);
    assert!(after.contains_key(&(2, 0)), "the hole filled");
    assert_stays_still(&w, &mut sim, focus, 20);
}

/// Over `plate` and two runs in opposite orders of placing: the same world.
#[test]
fn the_same_inputs_give_the_same_world_whatever_order_they_came_in() {
    let run = |reverse: bool| -> String {
        let w = test_world();
        let (c, focus) = table(&w, 31);
        let (_, _, sand, gravel) = ids(&w);
        let mut writes: Vec<(BlockPos, BlockId)> = Vec::new();
        for d in 0..6 {
            for (di, dj, id) in [(0i64, 0i64, sand), (3, 1, gravel), (-2, 4, sand), (1, -3, gravel)] {
                writes.push((BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, k: c.k + d, ..c }, id));
            }
        }
        if reverse {
            writes.reverse();
        }
        for chunk in writes.chunks(3) {
            w.commit(chunk);
        }
        let mut sim = Sim::new(levels(30.0));
        settle(&w, &mut sim, focus, 3000);
        sim.state_hash(&w)
    };
    assert_eq!(run(false), run(true));
}

/// The rule runs on one thread, and what it reads through the generator
/// comes out the same from a pool of any size.
#[test]
fn the_hash_is_the_same_across_rayon_pools_of_one_and_eight_threads() {
    let run = || {
        let w = test_world();
        let (c, focus) = table(&w, 31);
        let (_, _, sand, _) = ids(&w);
        let writes: Vec<(BlockPos, BlockId)> = (0..16).map(|d| (BlockPos { k: c.k + d, ..c }, sand)).collect();
        w.commit(&writes);
        let mut sim = Sim::new(levels(35.0));
        settle(&w, &mut sim, focus, 1000);
        sim.state_hash(&w)
    };
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(run);
    let eight = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap().install(run);
    assert_eq!(one, eight);
}

/// Saved and restored in the middle of a collapse, it carries on exactly:
/// the level byte goes wherever the water layer goes.
#[test]
fn a_restored_state_runs_on_to_the_same_world() {
    let w = test_world();
    let (c, focus) = table(&w, 31);
    let (_, _, sand, _) = ids(&w);
    let writes: Vec<(BlockPos, BlockId)> = (0..30).map(|d| (BlockPos { k: c.k + d, ..c }, sand)).collect();
    w.commit(&writes);
    let mut sim = Sim::new(levels(35.0));
    for _ in 0..24 {
        sim.step(&w, focus);
    }
    assert!(sim.stats.powder_moves > 0);
    let snap = Snapshot::from_bytes(&sim.snapshot(&w).to_bytes()).unwrap();
    assert_eq!(snap.config.unwrap().flow, sim.config.flow, "the angle is in the state");
    let w2 = copy_world(&w);
    let mut sim2 = Sim::from_snapshot(sim.config, snap);
    assert_eq!(sim2.state_hash(&w2), sim.state_hash(&w));
    for t in 0..200 {
        sim.step(&w, focus);
        sim2.step(&w2, focus);
        assert_eq!(sim2.state_digest(&w2), sim.state_digest(&w), "tick {t}");
    }
    assert_eq!(sim2.state_hash(&w2), sim.state_hash(&w));
}

/// Blocks dropped at one point on a plate that spans `centre`'s neighbours.
fn pile_across(w: &World, centre: BlockPos, radius: u32, count: u32) -> Sim {
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
    let mut sim = Sim::new(levels(35.0));
    let mut placed = 0;
    for t in 0..20_000u32 {
        if placed < count && t % 10 == 0 && w.block_at(spout) != sand {
            assert!(w.set_block(spout, sand));
            placed += 1;
        }
        sim.step(w, focus);
        if placed == count && sim.queued() == 0 && sim.falling().is_empty() {
            assert_eq!(powder_total(&w.edits, &w.registry), count as u64 * 255, "every unit is still there");
            assert_stays_still(w, &mut sim, focus, 20);
            return sim;
        }
    }
    panic!("the pile never settled");
}

/// Where a powder cell is, by chunk and by face.
fn powder_where(w: &World) -> (std::collections::BTreeSet<ChunkKey>, std::collections::BTreeSet<u8>) {
    let mut chunks = std::collections::BTreeSet::new();
    let mut faces = std::collections::BTreeSet::new();
    for d in w.edits.snapshot_full() {
        if d.ids.iter().any(|&id| w.registry.has_gravity(id)) {
            chunks.insert(d.key);
            faces.insert(d.key.face);
        }
    }
    (chunks, faces)
}

#[test]
fn a_pile_forms_across_a_chunk_corner() {
    let w = test_world();
    let c = BlockPos { face: 2, i: 32 * 3000, j: 32 * 2000, k: 32 * 47 };
    pile_across(&w, c, 7, 30);
    let (chunks, _) = powder_where(&w);
    assert!(chunks.len() >= 4, "{} chunks", chunks.len());
}

#[test]
fn a_pile_forms_across_a_cube_edge() {
    let w = test_world();
    let n = w.planet.n;
    let c = BlockPos { face: 0, i: n - 1, j: n / 2, k: SKY };
    pile_across(&w, c, 7, 30);
    assert_eq!(powder_where(&w).1.len(), 2, "both faces");
}

#[test]
fn a_pile_forms_on_a_cube_corner() {
    let w = test_world();
    let n = w.planet.n;
    let c = BlockPos { face: 0, i: n - 1, j: n - 1, k: SKY };
    pile_across(&w, c, 7, 30);
    assert_eq!(powder_where(&w).1.len(), 3, "all three faces");
}

/// A way out through a chunk outside the region waits there and goes when
/// the region arrives, as the block slide's does.
#[test]
fn a_move_waits_at_the_edge_of_the_active_region() {
    let w = test_world();
    let (g, _) = site(&w);
    let ck = World::chunk_of(BlockPos { k: g.k + 96, ..g }).0;
    let (i0, j0, k0) = ck.origin_blocks();
    let (i0, j0, k0) = (i0 as u32, j0 as u32, k0 as u32);
    let near = w.planet.pos(ck.face, i0 as f64 + 1.0, j0 as f64 + 16.0, k0 as f64 + 16.0);
    let both = w.planet.pos(ck.face, i0 as f64 + 32.0, j0 as f64 + 16.0, k0 as f64 + 16.0);
    let (_, _, sand, _) = ids(&w);
    let o = BlockPos { face: ck.face, i: i0 + 26, j: j0 + 10, k: k0 + 8 };
    plate(&w, o, 12, 6);
    // Against the chunk's +i side, and two blocks high.
    let at = |d: u32| BlockPos { i: i0 + 31, j: j0 + 12, k: k0 + 9 + d, ..o };
    w.commit(&[(at(0), sand), (at(1), sand), (at(2), sand)]);
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..levels(35.0) });
    for _ in 0..40 {
        sim.step(&w, near);
    }
    let next = ChunkKey { cx: ck.cx + 1, ..ck };
    assert!(!sim.is_active(next), "the next chunk is outside");
    let total = powder_total(&w.edits, &w.registry);
    let inside = sim.frozen_count();
    assert!(inside > 0, "it waits, saved");
    assert_eq!(total, 3 * 255);
    assert!(top_of(&w, BlockPos { i: i0 + 32, j: j0 + 12, ..o }, k0 + 9, 6).is_none(), "nothing crossed");
    sim.step(&w, both);
    let mut sim2 = sim;
    for _ in 0..200 {
        sim2.step(&w, both);
    }
    assert!(top_of(&w, BlockPos { i: i0 + 32, j: j0 + 12, ..o }, k0 + 9, 6).is_some(), "it went when the region arrived");
    assert_eq!(powder_total(&w.edits, &w.registry), total);
}

/// A generated slope holds at the block rule's limit, whatever the angle: one
/// dig on a beach moves what it undercuts, not the beach. Everything the dig
/// disturbed still follows the angle.
#[test]
fn digging_a_real_beach_does_not_set_it_flowing() {
    let w = test_world();
    let (air, _, sand, _) = ids(&w);
    let c = BlockPos { face: 1, i: 1998, j: 59004, k: 512 };
    assert_eq!(w.block_at(c), sand, "the beach of b4-slide.play");
    let focus = w.planet.pos(c.face, c.i as f64 + 0.5, c.j as f64 + 0.5, c.k as f64 + 2.0);
    let mut sim = Sim::new(levels(20.0));
    w.set_block(c, air);
    w.set_block(BlockPos { k: c.k - 1, ..c }, air);
    w.set_block(BlockPos { k: c.k - 2, ..c }, air);
    w.set_block(BlockPos { k: c.k - 3, ..c }, air);
    let t = settle(&w, &mut sim, focus, 2000);
    println!("a four-deep shaft on the beach at 20 degrees: still in {t} ticks, {} moves, {} units", sim.stats.powder_moves, sim.stats.powder_moved);
    assert!(sim.stats.powder_moves < 2000, "it stopped soon: {} moves", sim.stats.powder_moves);
    assert_stays_still(&w, &mut sim, focus, 20);
}

/// A settled pile costs nothing: nothing queued, nothing frozen, no update.
#[test]
fn a_settled_pile_costs_nothing() {
    let w = test_world();
    let (c, focus) = table(&w, 41);
    let (_, _, sand, _) = ids(&w);
    let writes: Vec<(BlockPos, BlockId)> = (0..12).map(|d| (BlockPos { k: c.k + d, ..c }, sand)).collect();
    w.commit(&writes);
    let mut sim = Sim::new(levels(35.0));
    settle(&w, &mut sim, focus, 2000);
    assert_eq!(sim.frozen_count(), 0);
    assert_stays_still(&w, &mut sim, focus, 100);
}

/// `units`: the stored level of a powder cell.
#[test]
fn a_stored_zero_is_a_whole_block() {
    assert_eq!(units(0), WHOLE);
    assert_eq!(units(1), 1);
    assert_eq!(units(254), 254);
}

/// Random stairs of powder on a plate, several seeds and angles: the
/// potential falls on every tick that moves, every unit stays, each comes to
/// rest, and 50 more ticks do nothing.
#[test]
fn random_heaps_always_come_to_rest() {
    for (seed, deg) in [(1u64, 12.0), (2, 25.0), (3, 35.0), (4, 45.0), (5, 60.0), (6, 69.0)] {
        let w = test_world();
        let (c, focus) = table(&w, 41);
        let (_, _, sand, gravel) = ids(&w);
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut writes: Vec<(BlockPos, BlockId, Option<u8>)> = Vec::new();
        for di in -8i64..=8 {
            for dj in -8i64..=8 {
                let h = next() % 7;
                let id = if next() % 4 == 0 { gravel } else { sand };
                for d in 0..h as u32 {
                    writes.push((BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, k: c.k + d, ..c }, id, None));
                }
                let top = (next() % 256) as u8;
                writes.push((BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, k: c.k + h as u32, ..c }, id, Some(top)));
            }
        }
        w.commit_levels(&writes);
        let mut sim = Sim::new(levels(deg));
        let t = settle(&w, &mut sim, focus, 6000);
        println!("seed {seed} at {deg} degrees: {t} ticks, {} moves", sim.stats.powder_moves);
        assert_stays_still(&w, &mut sim, focus, 50);
    }
}

/// Through the real save directory, in the middle of a collapse: every level
/// comes back, and the run carries on to the same world.
#[test]
fn a_save_in_the_middle_of_a_collapse_reloads_exactly() {
    use crate::session::Session;
    use crate::world::save::SaveDir;
    let w = test_world();
    let (c, focus) = table(&w, 31);
    let (_, _, sand, _) = ids(&w);
    let writes: Vec<(BlockPos, BlockId)> = (0..30).map(|d| (BlockPos { k: c.k + d, ..c }, sand)).collect();
    w.commit(&writes);
    let mut sim = Sim::new(levels(35.0));
    for _ in 0..24 {
        sim.step(&w, focus);
    }
    let dir = SaveDir::new(std::env::temp_dir().join(format!("triangulum-powder-reload-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.path);
    let cfg = crate::grid::WorldConfig { name: "test".into(), seed: 42, block_size_m: 1.0, world_height: 2048, sea_level: 512, columns_per_face: 131072, bake: String::new(), vertical_scale: None, roughness: 1.0, landform: 1.0 };
    let bake = "ab".repeat(32);
    let snap = sim.snapshot(&w);
    dir.flush(&w.edits, &cfg, &w.registry, &bake, Some(&snap.to_bytes())).unwrap();
    let edits = std::sync::Arc::new(crate::world::EditStore::new());
    dir.load_into(&cfg, &w.registry, &bake, &edits).unwrap();
    let w2 = World::new(w.generator.clone(), w.registry.clone(), edits);
    let mut sim2 = Session::load_sim(Some(&dir), sim.config, &w.registry).unwrap();
    assert_eq!(sim2.state_hash(&w2), sim.state_hash(&w), "the reloaded state is the saved state");
    assert_eq!(powder_total(&w2.edits, &w2.registry), powder_total(&w.edits, &w.registry));
    for _ in 0..300 {
        sim.step(&w, focus);
        sim2.step(&w2, focus);
    }
    assert_eq!(sim2.state_hash(&w2), sim.state_hash(&w));
    let _ = std::fs::remove_dir_all(&dir.path);
}

/// Partial powder keeps to a save of its own: a levels game will not open a
/// normal save, a normal game will not open a levels save, and neither
/// changes a byte of what it refused. To the normal game partial powder would
/// be whole blocks, and sand would multiply.
#[test]
fn a_levels_save_and_a_normal_save_refuse_each_other() {
    use crate::session::Session;
    use crate::world::save::SaveDir;
    let w = test_world();
    let (c, _) = table(&w, 9);
    let (_, _, sand, _) = ids(&w);
    w.commit_levels(&[(c, sand, Some(77))]);
    let cfg = crate::grid::WorldConfig { name: "test".into(), seed: 42, block_size_m: 1.0, world_height: 2048, sea_level: 512, columns_per_face: 131072, bake: String::new(), vertical_scale: None, roughness: 1.0, landform: 1.0 };
    let bake = "ab".repeat(32);
    let id = std::process::id();
    let snapshot_of = |dir: &SaveDir| -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.path.clone()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push((p.display().to_string(), std::fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    };
    // A save made by a levels game.
    let scratch = SaveDir::new(std::env::temp_dir().join(format!("triangulum-powder-scratch-{id}")));
    let _ = std::fs::remove_dir_all(&scratch.path);
    let mut sim = Sim::new(levels(40.0));
    let bytes = sim.snapshot(&w).to_bytes();
    scratch.flush(&w.edits, &cfg, &w.registry, &bake, Some(&bytes)).unwrap();
    let before = snapshot_of(&scratch);
    let e = Session::load_sim(Some(&scratch), SimConfig::default(), &w.registry).map(|_| ()).unwrap_err();
    assert!(e.contains("--powder levels"), "{e}");
    assert_eq!(snapshot_of(&scratch), before, "refused, and not a byte changed");
    assert!(Session::load_sim(Some(&scratch), levels(30.0), &w.registry).is_ok(), "a levels game opens it, at any angle");
    // A normal save, with its simulation state and without.
    let real = SaveDir::new(std::env::temp_dir().join(format!("triangulum-powder-real-{id}")));
    let _ = std::fs::remove_dir_all(&real.path);
    let mut plain = Sim::new(SimConfig::default());
    let w2 = test_world();
    let (_, stone, ..) = ids(&w2);
    w2.set_block(c, stone);
    let bytes = plain.snapshot(&w2).to_bytes();
    real.flush(&w2.edits, &cfg, &w2.registry, &bake, Some(&bytes)).unwrap();
    let before = snapshot_of(&real);
    let e = Session::load_sim(Some(&real), levels(35.0), &w.registry).map(|_| ()).unwrap_err();
    assert!(e.contains("not a --powder levels save"), "{e}");
    assert_eq!(snapshot_of(&real), before);
    assert!(Session::load_sim(Some(&real), SimConfig::default(), &w.registry).is_ok(), "the normal game opens it as it always did");
    let no_state = SaveDir::new(std::env::temp_dir().join(format!("triangulum-powder-nostate-{id}")));
    let _ = std::fs::remove_dir_all(&no_state.path);
    w2.set_block(BlockPos { k: c.k + 1, ..c }, stone);
    no_state.flush(&w2.edits, &cfg, &w2.registry, &bake, None).unwrap();
    assert!(Session::load_sim(Some(&no_state), levels(35.0), &w.registry).is_err(), "a save with no state is not a levels save either");
    assert!(Session::load_sim(Some(&SaveDir::new(std::env::temp_dir().join(format!("triangulum-powder-absent-{id}")))), levels(35.0), &w.registry).is_ok(), "nothing there yet: a new scratch save");
    for d in [&scratch, &real, &no_state] {
        let _ = std::fs::remove_dir_all(&d.path);
    }
}

/// What a collapse costs under this rule (provisional: timings need the
/// machine to themselves), and what meshing a chunk of slope costs.
#[test]
#[ignore]
fn stress_powder_levels_collapse() {
    for deg in [25.0, 35.0, 45.0] {
        let w = test_world();
        let (c, focus) = table(&w, 101);
        let (_, _, sand, _) = ids(&w);
        let mut writes: Vec<(BlockPos, BlockId)> = Vec::new();
        for di in -15i64..15 {
            for dj in -15i64..15 {
                for d in 0..12 {
                    writes.push((BlockPos { i: (c.i as i64 + di) as u32, j: (c.j as i64 + dj) as u32, k: c.k + d, ..c }, sand));
                }
            }
        }
        w.commit(&writes);
        let mut sim = Sim::new(levels(deg));
        let (mut worst, mut sum, mut ticks, mut quiet) = (0u32, 0u64, 0u64, 0u64);
        let mut last = 0;
        while sim.queued() > 0 || sim.tick() == 0 || !sim.falling().is_empty() {
            sim.step(&w, focus);
            worst = worst.max(sim.stats.last_us);
            sum += sim.stats.last_us as u64;
            ticks += 1;
            // The tick after which nothing moved more than a few units.
            if sim.stats.powder_moved - last > 64 {
                quiet = ticks;
            }
            last = sim.stats.powder_moved;
            assert!(ticks < 12000, "never settled");
        }
        assert_eq!(powder_total(&w.edits, &w.registry), 30 * 30 * 12 * 255);
        println!("30 x 30 x 12 block with nothing round it, at {deg} degrees: {ticks} ticks, the last visible move at tick {quiet}, {} moves, average tick {:.2} ms, worst {:.2} ms", sim.stats.powder_moves, sum as f64 / ticks as f64 / 1e3, worst as f64 / 1e3);
        let key = World::chunk_of(BlockPos { k: c.k + 4, ..c }).0;
        let made = w.generate(key);
        let t = std::time::Instant::now();
        let p = w.padded_powder(key, &made).expect("a slope here");
        println!("the slope of one chunk: {:.2} ms, {} quads, {} cells replaced", t.elapsed().as_secs_f64() * 1e3, p.quads.len(), p.soft.len());
    }
}

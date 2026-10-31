//! B4, Andrew's #9: the level-1 film. Every rule on the three scenes he asked
//! about (a flat plateau with nowhere to go, a lake whose level has dropped,
//! the sheet one poured block makes), the potential on every move, the
//! accounting of water that leaves the world, and what must not regress: a
//! film at rest costs nothing, nothing keeps a chunk awake for ever, a lake's
//! edge and a fed trickle stay, an old save wakes nothing.

use super::tests::{copy_world, ids, site};
use super::water::{Boundary, FULL, PUDDLE_CELLS, film_potential, water_potential, water_total};
use super::*;
use crate::world::tests::test_world;

fn with(film: Film) -> SimConfig {
    SimConfig { flow: Flow { film, film_dry_ticks: 40, ..Flow::default() }, ..SimConfig::default() }
}

/// Water cells of one layer, as (cells, cells at level 1, units).
fn layer(w: &World, k: u32) -> (usize, usize, u64) {
    let (mut wet, mut film, mut units) = (0, 0, 0);
    for c in w.edits.snapshot_full() {
        let Some(levels) = &c.water else { continue };
        let k0 = c.key.origin_blocks().2 as u32;
        if k < k0 || k >= k0 + CHUNK as u32 {
            continue;
        }
        for x in 0..CHUNK {
            for y in 0..CHUNK {
                let l = levels[crate::world::chunk::index(x, y, (k - k0) as usize)];
                wet += (l > 0) as usize;
                film += (l == 1) as usize;
                units += l as u64;
            }
        }
    }
    (wet, film, units)
}

/// The potential that strictly falls under `flow`.
fn potential(w: &World, sim: &Sim) -> u64 {
    if sim.config.flow.film.merges() { film_potential(&w.edits) } else { water_potential(&w.edits) }
}

/// Tick until nothing is queued, in flight or waiting to dry, checking on
/// every tick that the potential strictly fell if anything moved and that
/// every unit is in the world or counted as dried. The ticks taken.
fn settle(w: &World, sim: &mut Sim, focus: DVec3, max_ticks: usize) -> usize {
    let start = water_total(&w.edits) + sim.stats.water_dried;
    for t in 1..=max_ticks {
        let before = potential(w, sim);
        let (moved, dried) = (sim.stats.water_moved, sim.stats.water_dried);
        sim.step(w, focus);
        if sim.stats.water_moved > moved || sim.stats.water_dried > dried {
            let after = potential(w, sim);
            assert!(after < before, "{} tick {}: potential {before} -> {after}, {} units moved, {} dried", sim.config.flow.film.name(), sim.tick(), sim.stats.water_moved - moved, sim.stats.water_dried - dried);
        }
        assert_eq!(water_total(&w.edits) + sim.stats.water_dried, start, "{} tick {}: every unit accounted for", sim.config.flow.film.name(), sim.tick());
        if sim.queued() == 0 && sim.falling().is_empty() && sim.drying_count() == 0 {
            return t;
        }
    }
    panic!("{}: not settled after {max_ticks} ticks: {} queued, {} drying", sim.config.flow.film.name(), sim.queued(), sim.drying_count());
}

/// After settling nothing runs, nothing changes, and it stays that way.
fn assert_quiet(w: &World, sim: &mut Sim, focus: DVec3) {
    let (generation, processed, dried) = (w.edits.generation(), sim.stats.processed, sim.stats.water_dried);
    for _ in 0..100 {
        sim.step(w, focus);
        assert_eq!(sim.stats.water_updates_last, 0);
    }
    assert_eq!((w.edits.generation() - generation, sim.stats.processed - processed, sim.stats.water_dried - dried), (0, 0, 0), "{}: settled water costs nothing", sim.config.flow.film.name());
}

/// A stone table at layer `o.k`, `half` cells each way from (o.i, o.j).
fn table(w: &World, o: BlockPos, half: u32) {
    let (_, stone, ..) = ids(w);
    let side = 2 * half + 1;
    let writes: Vec<(BlockPos, BlockId)> = (0..side).flat_map(|dx| (0..side).map(move |dy| (BlockPos { i: o.i + dx - half, j: o.j + dy - half, ..o }, stone))).collect();
    w.commit(&writes);
}

/// A flat film with nowhere to go: 25 cells at level 1 on a table whose edge
/// is 18 cells away, well past `film_reach`.
fn plateau(film: Film) -> (World, Sim, DVec3, u32) {
    let w = test_world();
    let (g, focus) = site(&w);
    let o = BlockPos { k: g.k + 14, ..g };
    table(&w, o, 20);
    let writes: Vec<(BlockPos, BlockId, Option<u8>)> = (0..5).flat_map(|a| (0..5).map(move |b| (a, b))).map(|(a, b)| (BlockPos { i: o.i + a - 2, j: o.j + b - 2, k: o.k + 1, ..o }, w.water_id(), Some(1))).collect();
    w.commit_levels(&writes);
    (w, Sim::new(with(film)), focus, o.k + 1)
}

/// The one block a player pours: a full cell on the middle of a wide table.
fn poured(film: Film) -> (World, Sim, DVec3, u32) {
    let w = test_world();
    let (g, focus) = site(&w);
    let o = BlockPos { k: g.k + 14, ..g };
    table(&w, o, 24);
    assert!(w.set_water(BlockPos { k: o.k + 1, ..o }, w.water_id(), FULL));
    (w, Sim::new(with(film)), focus, o.k + 1)
}

/// A lake whose level has dropped: a 3 x 3 pool under a terrace six cells
/// wide, all full, over a sump. Pull the plug under the pool and run it dry.
/// The terrace is the layer returned.
fn drained_lake(film: Film) -> (World, Sim, DVec3, u32) {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, stone, ..) = ids(&w);
    let water = w.water_id();
    let o = BlockPos { k: g.k + 20, ..g };
    let at = |dx: i32, dy: i32, dz: i32| BlockPos { i: (o.i as i32 + dx) as u32, j: (o.j as i32 + dy) as u32, k: (o.k as i32 + dz) as u32, ..o };
    let mut writes = Vec::new();
    for dx in -9..=9i32 {
        for dy in -9..=9i32 {
            let r = dx.abs().max(dy.abs());
            for dz in -7..=3i32 {
                let id = match dz {
                    -6..=-2 if r <= 6 => air,
                    0 if r <= 1 => water,
                    1 | 2 if r <= 7 => {
                        if dz == 1 { water } else { air }
                    }
                    _ => stone,
                };
                writes.push((at(dx, dy, dz), id, (id == water).then_some(FULL)));
            }
        }
    }
    w.commit_levels(&writes);
    let mut sim = Sim::new(with(film));
    sim.step(&w, focus);
    assert_eq!(sim.stats.water_moved, 0, "the lake rests until its plug goes");
    assert!(w.set_block(at(0, 0, -1), air));
    (w, sim, focus, o.k + 1)
}

type Scene = fn(Film) -> (World, Sim, DVec3, u32);

/// Every rule on every scene: the table the findings quote.
/// `cargo test --release film_rules_on_the_three_scenes -- --nocapture`.
#[test]
fn film_rules_on_the_three_scenes() {
    println!("{:9} {:10} {:>6} {:>6} {:>7} {:>6} {:>6}", "rule", "scene", "ticks", "wet", "level1", "units", "dried");
    for film in Film::ALL {
        for (scene, make, max) in [("plateau", plateau as Scene, 2000), ("lake", drained_lake as Scene, 9000), ("poured", poured as Scene, 20000)] {
            let (w, mut sim, focus, k) = make(film);
            let ticks = settle(&w, &mut sim, focus, max);
            let (wet, level1, units) = layer(&w, k);
            println!("{:9} {:10} {ticks:6} {wet:6} {level1:7} {units:6} {:6}", film.name(), scene, sim.stats.water_dried);
            assert_quiet(&w, &mut sim, focus);
            match (scene, film) {
                ("plateau", Film::Keep | Film::Seek) => assert_eq!((wet, level1, sim.stats.water_dried), (25, 25, 0), "{}: nowhere to go, nothing to take it", film.name()),
                ("plateau", Film::Dry | Film::SeekDry) => assert_eq!((wet, sim.stats.water_dried), (0, 25), "{}", film.name()),
                ("plateau", Film::Merge) => {
                    assert_eq!(units, 25, "merging keeps the water");
                    assert!(wet < 25, "it has beaded up: {wet} cells");
                }
                ("lake", Film::Keep) => assert!(level1 > 0, "the slide takes the wedge down to a film and stops"),
                ("lake", Film::Seek) => assert!(level1 > 0, "lines from the terrace's corners meet no drop"),
                ("lake", Film::Dry | Film::SeekDry) => assert_eq!(wet, 0, "{}: the bed is dry", film.name()),
                ("poured", Film::Keep | Film::Seek) => assert_eq!(units, 255, "{}: a puddle with nowhere to go stays", film.name()),
                ("poured", Film::Dry | Film::SeekDry) => assert_eq!((units, sim.stats.water_dried), (0, 255), "{}: a puddle dries away", film.name()),
                ("poured", Film::Merge) => {
                    assert_eq!(units, 255);
                    assert_eq!(level1, 0, "no edge thinner than 2");
                }
                _ => {}
            }
        }
    }
}

/// A film runs to a drop on a straight dry line, within its reach only, by
/// the same move as the slide: straight into the cell below the drop.
#[test]
fn seek_runs_along_a_dry_line_as_far_as_its_reach() {
    for (gap, reach, goes) in [(1u32, 1u32, true), (2, 1, false), (8, 8, true), (9, 8, false), (4, 8, true)] {
        let w = test_world();
        let (g, focus) = site(&w);
        let (_, stone, ..) = ids(&w);
        let o = BlockPos { k: g.k + 14, ..g };
        // A strip along +i with a drop `gap` cells from the film.
        let strip: Vec<(BlockPos, BlockId)> = (0..gap).flat_map(|dx| (0..3).map(move |dy| (BlockPos { i: o.i + dx, j: o.j + dy, ..o }, stone))).collect();
        w.commit(&strip);
        let walls: Vec<(BlockPos, BlockId)> = (0..gap + 1).flat_map(|dx| [0, 2].map(|dy| (BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + 1, ..o }, stone))).collect();
        w.commit(&walls);
        let film = BlockPos { j: o.j + 1, k: o.k + 1, ..o };
        assert!(w.set_block(BlockPos { i: film.i - 1, ..film }, stone), "the way behind it is closed");
        assert!(w.set_water(film, w.water_id(), 1));
        let mut sim = Sim::new(SimConfig { flow: Flow { film: Film::Seek, film_reach: reach, ..Flow::default() }, ..SimConfig::default() });
        let phi = water_potential(&w.edits);
        sim.step(&w, focus);
        assert_eq!(w.water_level_at(film) == 0, goes, "gap {gap} reach {reach}");
        if goes {
            assert_eq!(w.water_level_at(BlockPos { i: film.i + gap, k: film.k - 1, ..film }), 1, "below the drop");
            assert_eq!(phi - water_potential(&w.edits), 1024, "exactly a fall of one unit");
        }
    }
}

/// The line crosses a cube edge: over it, "straight on" is another axis of
/// another face, and the film still finds the drop.
#[test]
fn seek_runs_across_a_cube_edge() {
    let w = test_world();
    let n = w.planet.n;
    let (_, stone, ..) = ids(&w);
    let k = 1500;
    let film = BlockPos { face: 0, i: n - 2, j: n / 2, k };
    let a = BlockPos { i: n - 1, ..film };
    let beyond = lateral(&w, a)[1].expect("over the edge");
    assert_ne!(beyond.face, 0);
    let floor = |p: BlockPos| (BlockPos { k: k - 1, ..p }, stone);
    // A floor under the film, its neighbour on this face and the first cell
    // over the edge, and walls so the only line is the one across.
    let mut writes = vec![floor(film), floor(a), floor(beyond)];
    for c in [film, a, beyond] {
        for s in lateral(&w, c).into_iter().flatten() {
            if ![film, a, beyond].contains(&s) && !(c == beyond) {
                writes.push((s, stone));
            }
        }
    }
    w.commit(&writes);
    // Walls around the far cell except the way on: its neighbour opposite `a`.
    let around = lateral(&w, beyond);
    let came = around.iter().position(|c| *c == Some(a)).unwrap();
    let on = around[came ^ 1].unwrap();
    for (way, s) in around.iter().enumerate() {
        if way != came && way != (came ^ 1) {
            w.commit(&[(s.unwrap(), stone)]);
        }
    }
    assert!(w.set_water(film, w.water_id(), 1));
    let focus = w.planet.pos(0, film.i as f64 + 0.5, film.j as f64 + 0.5, k as f64 + 1.0);
    let mut sim = Sim::new(SimConfig { flow: Flow { film: Film::Seek, ..Flow::default() }, ..SimConfig::default() });
    sim.step(&w, focus);
    assert_eq!(w.water_level_at(film), 0, "it found the drop on the other face");
    assert_eq!(w.water_level_at(BlockPos { k: k - 1, ..on }), 1, "and went straight into the cell below it");
}

/// A film can only ever take what a puddle holds. Fed from generated water at
/// the region's edge, the edge of the film keeps its place: if it dried, the
/// boundary would refill it for ever and the chunk would never sleep.
#[test]
fn a_film_fed_from_the_boundary_comes_to_rest_and_stays() {
    let w = test_world();
    let (g, _) = site(&w);
    let (_, stone, ..) = ids(&w);
    let ck = World::chunk_of(BlockPos { k: g.k + 96, ..g }).0;
    let (i0, j0, k0) = ck.origin_blocks();
    let (i0, j0, k0) = (i0 as u32, j0 as u32, k0 as u32);
    let near = w.planet.pos(ck.face, i0 as f64 + 1.0, j0 as f64 + 16.0, k0 as f64 + 16.0);
    // A strip one cell wide from the chunk's +i side back along -i.
    let row = |dx: u32| BlockPos { face: ck.face, i: i0 + 31 - dx, j: j0 + 16, k: k0 + 10 };
    let mut writes = Vec::new();
    for dx in 0..16 {
        writes.push((BlockPos { k: row(dx).k - 1, ..row(dx) }, stone));
        writes.push((BlockPos { j: row(dx).j - 1, ..row(dx) }, stone));
        writes.push((BlockPos { j: row(dx).j + 1, ..row(dx) }, stone));
    }
    writes.push((row(16), stone));
    w.commit(&writes);
    let source = BlockPos { i: i0 + 32, ..row(0) };
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, flow: Flow { film_dry_ticks: 20, ..Flow::default() }, ..SimConfig::default() });
    sim.boundary_override = Some(Box::new(move |bp| (bp == source).then_some(Boundary::Water { level: 6, drains: false })));
    sim.step(&w, near);
    assert!(sim.is_active(ck) && !sim.is_active(World::chunk_of(source).0));
    assert!(w.set_water(row(0), w.water_id(), 1));
    for _ in 0..600 {
        sim.step(&w, near);
    }
    assert_eq!((sim.queued(), sim.drying_count()), (0, 0), "at rest: {:?}", (0..8).map(|d| w.water_level_at(row(d))).collect::<Vec<_>>());
    let (inflow, dried) = (sim.stats.water_in, sim.stats.water_dried);
    assert!(inflow > 0, "fed from the boundary");
    let levels: Vec<u8> = (0..8).map(|d| w.water_level_at(row(d))).collect();
    println!("the fed film at rest: {levels:?}, {inflow} in, {dried} dried");
    assert!(levels.contains(&1), "it keeps its edge: {levels:?}");
    assert!(dried <= 2, "only a flat tip, if any: the edge leans on fed water: {levels:?}, {dried} dried");
    let generation = w.edits.generation();
    for _ in 0..600 {
        sim.step(&w, near);
    }
    assert_eq!((sim.stats.water_in, sim.stats.water_dried, w.edits.generation()), (inflow, dried, generation), "and stays asleep");
}

/// The thin edge of a body too big to be a puddle stays: drying never eats
/// a pond from its shallows. A smaller one dries.
#[test]
fn the_edge_of_a_large_body_stays_and_the_edge_of_a_puddle_goes() {
    for (half, stays) in [(4u32, false), (12, true)] {
        let w = test_world();
        let (g, focus) = site(&w);
        let o = BlockPos { k: g.k + 14, ..g };
        table(&w, o, half + 6);
        let water = w.water_id();
        let side = 2 * half + 1;
        let mut writes = Vec::new();
        for dx in 0..side + 2 {
            for dy in 0..side + 2 {
                let rim = dx == 0 || dy == 0 || dx == side + 1 || dy == side + 1;
                writes.push((BlockPos { i: o.i + dx - half - 1, j: o.j + dy - half - 1, k: o.k + 1, ..o }, water, Some(if rim { 1 } else { 2 })));
            }
        }
        w.commit_levels(&writes);
        let thick = (side * side) as usize;
        assert_eq!(thick > PUDDLE_CELLS, stays);
        let mut sim = Sim::new(with(Film::Dry));
        settle(&w, &mut sim, focus, 6000);
        let (wet, ..) = layer(&w, o.k + 1);
        println!("{thick} cells of level 2 with a level-1 edge: {wet} cells left, {} units dried", sim.stats.water_dried);
        if stays {
            assert_eq!(sim.stats.water_dried, 0, "a body this big keeps its edge");
        } else {
            assert_eq!(wet, 0, "a puddle dries away");
        }
    }
}

/// The potential on random stairs under every rule: whatever water starts
/// where, every tick that moves anything lowers it, every unit is accounted
/// for, and it comes to rest and stays. (`settle` is the check.)
#[test]
fn the_potential_falls_on_every_tick_under_every_rule_on_random_stairs() {
    for film in Film::ALL {
        for seed in 1..=4u64 {
            let w = test_world();
            let (g, focus) = site(&w);
            let (_, stone, ..) = ids(&w);
            let water = w.water_id();
            let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xABCD;
            let mut next = move |n: u64| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x % n
            };
            let o = BlockPos { i: g.i - 8, j: g.j - 8, k: g.k + 14, ..g };
            let mut writes = Vec::new();
            for dx in 0..16u32 {
                for dy in 0..16u32 {
                    let wall = dx == 0 || dy == 0 || dx == 15 || dy == 15;
                    let top = if wall { 7 } else { (dx / 3 + next(3) as u32).min(5) };
                    for dz in 0..=top {
                        writes.push((BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + dz, ..o }, stone, None));
                    }
                    if !wall && next(3) > 0 {
                        writes.push((BlockPos { i: o.i + dx, j: o.j + dy, k: o.k + top + 1, ..o }, water, Some([1u8, 1, 2, 3, 9, 40, 255][next(7) as usize])));
                    }
                }
            }
            w.commit_levels(&writes);
            let mut sim = Sim::new(with(film));
            let ticks = settle(&w, &mut sim, focus, 12_000);
            println!("{} seed {seed}: settled in {ticks} ticks, {} units moved, {} dried", film.name(), sim.stats.water_moved, sim.stats.water_dried);
            assert_quiet(&w, &mut sim, focus);
        }
    }
}

/// Drying is state like any other: a film waiting to dry survives a save and
/// reload to the same future.
#[test]
fn a_film_waiting_to_dry_survives_a_save_and_reload() {
    let (w, mut sim, focus, _) = poured(Film::SeekDry);
    for _ in 0..90 {
        sim.step(&w, focus);
    }
    assert!(sim.drying_count() > 0, "cells are waiting");
    let w2 = copy_world(&w);
    let mut back = Sim::from_snapshot(sim.config, Snapshot::from_bytes(&sim.snapshot(&w).to_bytes()).unwrap());
    assert_eq!(back.drying_count(), sim.drying_count());
    assert_eq!(back.state_hash(&w2), sim.state_hash(&w), "the reloaded state is the saved state");
    let dried_before = sim.stats.water_dried;
    for _ in 0..200 {
        sim.step(&w, focus);
        back.step(&w2, focus);
    }
    assert!(sim.stats.water_dried > dried_before, "the wait ended in the run");
    assert_eq!(back.stats.water_dried, sim.stats.water_dried - dried_before);
    assert_eq!(back.state_hash(&w2), sim.state_hash(&w), "and runs on to the same place");
}

/// A cell whose chunk left the region when its time came waits with the
/// frozen updates, and dries after the region comes back.
#[test]
fn a_film_outside_the_region_does_not_dry_until_the_region_returns() {
    let (w, _, _, k) = plateau(Film::Dry);
    let (g, _) = site(&w);
    let focus = w.planet.pos(g.face, g.i as f64 + 0.5, g.j as f64 + 0.5, k as f64 + 1.0);
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..with(Film::Dry) });
    sim.step(&w, focus);
    for _ in 0..3 {
        sim.step(&w, focus);
    }
    assert!(sim.drying_count() > 0);
    let away = focus + focus.normalize().cross(DVec3::Z) * 400.0;
    for _ in 0..300 {
        sim.step(&w, away);
    }
    assert_eq!(sim.stats.water_dried, 0, "nothing outside the region changes");
    assert_eq!(layer(&w, k).0, 25);
    assert_eq!(sim.drying_count(), 0, "they went to wait with the frozen updates");
    assert!(sim.frozen_count() > 0);
    for _ in 0..300 {
        sim.step(&w, focus);
    }
    assert_eq!(sim.stats.water_dried, 25, "back in the region, it dried");
}

/// A save from the build before B4 holds water leaning at rest, films and
/// sand standing in towers, none of it scheduled. Under the new rules none
/// of it wakes: only what a change reaches moves, as with a generated river.
#[test]
fn a_save_from_before_loads_and_nothing_wakes_until_it_is_touched() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, _, sand, _) = ids(&w);
    let o = BlockPos { k: g.k + 14, ..g };
    table(&w, o, 6);
    // A sand tower no slide would let stand, and a film no rule would leave.
    let tower: Vec<(BlockPos, BlockId)> = (1..=6).map(|dz| (BlockPos { k: o.k + dz, ..o }, sand)).collect();
    w.commit(&tower);
    let film: Vec<(BlockPos, BlockId, Option<u8>)> = (2..6).map(|d| (BlockPos { i: o.i + d, k: o.k + 1, ..o }, w.water_id(), Some(1))).collect();
    w.commit_levels(&film);
    // As it stood at the end of the old build's session: every change absorbed, the queue empty.
    let _ = w.edits.take_changed();
    let old = Snapshot { tick: 5000, focus: Some(focus), config: Some(SimConfig { flow: Flow::before(), ..SimConfig::default() }), ..Default::default() };
    let mut bytes = old.to_bytes();
    let text = Flow::before().text();
    let at = bytes.len() - 3 * 8 - text.len() - 4;
    bytes.drain(at..at + 4 + text.len());
    bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
    let mut sim = Sim::from_snapshot(SimConfig::default(), Snapshot::from_bytes(&bytes).unwrap());
    let (generation, processed) = (w.edits.generation(), sim.stats.processed);
    for _ in 0..200 {
        sim.step(&w, focus);
    }
    assert_eq!((sim.stats.processed - processed, w.edits.generation() - generation), (0, 0), "nothing wakes");
    assert_eq!((sim.stats.slides, sim.stats.water_moved, sim.drying_count()), (0, 0, 0));
    // Touch it and only that part moves: the tower goes, the film beside it wakes.
    assert!(w.set_block(BlockPos { k: o.k + 7, ..o }, sand));
    assert!(w.set_block(BlockPos { k: o.k + 7, ..o }, air));
    for _ in 0..60 {
        sim.step(&w, focus);
    }
    assert!(sim.stats.slides > 0, "what a change reaches moves under the new rules");
}

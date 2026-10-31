use super::*;
use crate::grid::Planet;
use crate::world::EditStore;
use crate::world::save::SaveDir;
use crate::world::tests::{hills_ground, test_world};
use std::sync::Arc;

/// The rule before B4: blocks fall straight down their own column and
/// nothing slides. The tests of falling itself run under it, and so do the
/// ones that need a block to stay where it lands (a tower, a block on a
/// one-cell shelf): under the slide neither can stand.
fn straight_down() -> SimConfig {
    SimConfig { flow: Flow { slide: false }, ..SimConfig::default() }
}

#[test]
fn tick_is_three_player_steps() {
    assert!((STEPS_PER_TICK as f64 * crate::player::STEP - TICK_S).abs() < 1e-12);
}

#[test]
fn pos_key_round_trips_and_orders_canonically() {
    let ps = [
        BlockPos { face: 0, i: 0, j: 0, k: 0 },
        BlockPos { face: 0, i: 0, j: 0, k: 1 },
        BlockPos { face: 0, i: 0, j: 1, k: 0 },
        BlockPos { face: 0, i: 1, j: 0, k: 0 },
        BlockPos { face: 1, i: 0, j: 0, k: 0 },
        BlockPos { face: 5, i: PosKey::MAX_N - 1, j: PosKey::MAX_N - 1, k: PosKey::MAX_H - 1 },
    ];
    for w in ps.windows(2) {
        assert!(PosKey::of(w[0]) < PosKey::of(w[1]), "{:?} < {:?}", w[0], w[1]);
    }
    for p in ps {
        assert_eq!(PosKey::of(p).pos(), p);
        assert_eq!(PosKey::of(p).column(), PosKey::of(BlockPos { k: 0, ..p }));
    }
}

#[test]
fn snapshot_bytes_round_trip_and_reject_damage() {
    let snap = Snapshot {
        tick: 77,
        queue: vec![(78, PosKey(5)), (78, PosKey(9)), (80, PosKey(1))],
        frozen: vec![(ChunkKey::neisor(3, 0, 10, 11, 12), vec![(70, PosKey(123)), (71, PosKey(124))]), (ChunkKey::neisor(4, 0, 0, 0, 0), vec![])],
        falling: vec![Falling { id: 4, face: 2, i: 100, j: 200, z: 612.25, vel: -3.5 }, Falling { id: 5, face: 2, i: 100, j: 200, z: 615.0, vel: 0.0 }],
        config: Some(SimConfig { radius_m: 37.5, enabled: false, water_period: 3, flow: Flow { slide: false } }),
        focus: Some(DVec3::new(1.0e6, -2.5e6, 3.25e5)),
        rules: Some([7; 32]),
    };
    let bytes = snap.to_bytes();
    assert_eq!(Snapshot::from_bytes(&bytes).unwrap(), snap);
    // Version 2 adds one byte saying which parts follow.
    assert_eq!(Snapshot::default().to_bytes().len(), 4 + 4 + 8 + 8 * 3 + 1);
    // A version-1 file (no settings, focus or rules) still reads.
    let mut v1 = Snapshot { config: None, focus: None, rules: None, ..snap.clone() }.to_bytes();
    v1.pop();
    v1[4..8].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(Snapshot::from_bytes(&v1).unwrap(), Snapshot { config: None, focus: None, rules: None, ..snap.clone() });
    // A version-2 file (the build before B4): settings without flow settings.
    // It reads, and takes this build's flow settings.
    let no_focus = Snapshot { focus: None, rules: None, ..snap.clone() };
    let mut v2 = no_focus.to_bytes();
    let text = no_focus.config.unwrap().flow.text();
    let flow_at = v2.len() - text.len() - 4;
    assert_eq!(&v2[flow_at + 4..], text.as_bytes());
    v2.truncate(flow_at);
    v2[4..8].copy_from_slice(&2u32.to_le_bytes());
    let read = Snapshot::from_bytes(&v2).unwrap();
    assert_eq!(read.config.unwrap().flow, Flow::default());
    assert_eq!(read.config.unwrap().water_period, 3);
    assert!(Snapshot::from_bytes(&bytes[..bytes.len() - 3]).is_err(), "truncated");
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Snapshot::from_bytes(&extra).unwrap_err().contains("trailing"));
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert!(Snapshot::from_bytes(&bad).unwrap_err().contains("not a simulation state"));
    let mut sim = Sim::new(SimConfig::default());
    sim.restore(snap.clone());
    assert_eq!(sim.tick(), 77);
    assert_eq!(sim.queued(), 3);
    assert_eq!(sim.frozen_count(), 2);
    assert_eq!(sim.falling().len(), 2);
    assert!(sim.has_state() && !sim.is_dirty());
}

#[test]
fn neighbours_cross_chunk_borders_and_face_edges_but_not_the_world_top_or_floor() {
    let w = test_world();
    let p = w.planet;
    let n = p.n;
    // Interior: six, two of them in the next chunk along i (block 31 -> 32).
    let bp = BlockPos { face: 2, i: 31, j: 500, k: 600 };
    let ns = w.neighbours(bp);
    assert_eq!(ns.len(), 6);
    assert!(ns.contains(&BlockPos { i: 32, ..bp }) && ns.contains(&BlockPos { i: 30, ..bp }));
    assert!(ns.iter().all(|q| q.face == 2));
    // Last column of face 0: the +i neighbour is on face 2's edge, and its own
    // neighbours include the original (the step is its own inverse).
    let edge = BlockPos { face: 0, i: n - 1, j: 1000, k: 600 };
    let ns = w.neighbours(edge);
    assert_eq!(ns.len(), 6);
    let across: Vec<&BlockPos> = ns.iter().filter(|q| q.face != 0).collect();
    assert_eq!(across.len(), 1);
    assert_eq!(across[0].face, p.edges[0][1].face);
    assert!(w.neighbours(*across[0]).contains(&edge), "{:?} -> {:?}", across[0], w.neighbours(*across[0]));
    // Floor and top: five.
    assert_eq!(w.neighbours(BlockPos { k: 0, ..bp }).len(), 5);
    assert_eq!(w.neighbours(BlockPos { k: p.height - 1, ..bp }).len(), 5);
    // The corner block of a face: both across-edge neighbours are on other faces.
    let corner = BlockPos { face: 0, i: n - 1, j: n - 1, k: 600 };
    let ns = w.neighbours(corner);
    assert_eq!(ns.len(), 6);
    assert_eq!(ns.iter().filter(|q| q.face != 0).count(), 2);
}

/// The hills site: the ground block and a focus position standing on it.
pub(super) fn site(w: &World) -> (BlockPos, DVec3) {
    let g = hills_ground(w);
    let focus = w.planet.pos(g.face, g.i as f64 + 0.5, g.j as f64 + 0.5, g.k as f64 + 1.0);
    // The column above is air for a good way up (no trees yet).
    for dk in 1..=40 {
        assert_eq!(w.block_at(BlockPos { k: g.k + dk, ..g }), w.generator.blocks.air, "air above the ground at +{dk}");
    }
    (g, focus)
}

pub(super) fn ids(w: &World) -> (BlockId, BlockId, BlockId, BlockId) {
    let b = &w.generator.blocks;
    (b.air, b.stone, b.sand, b.gravel)
}

#[test]
fn sand_collapses_in_a_chain_and_a_column_drains_in_order() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, stone, sand, gravel) = ids(&w);
    let col = |k: u32| BlockPos { k, ..g };
    // Two stone, four sand, one gravel on the ground.
    let k0 = g.k + 1;
    w.set_block(col(k0), stone);
    w.set_block(col(k0 + 1), stone);
    for dk in 2..6 {
        w.set_block(col(k0 + dk), sand);
    }
    w.set_block(col(k0 + 6), gravel);
    let mut sim = Sim::new(straight_down());
    sim.step(&w, focus);
    assert_eq!(sim.falling().len(), 0, "a supported column does not move");
    assert_eq!(sim.queued(), 0, "every placement was processed in one tick");
    assert!(sim.active_chunks() > 0);
    // Dig the upper stone: the sand above it is notified and falls, one layer per tick.
    assert!(w.set_block(col(k0 + 1), air));
    let mut in_flight = Vec::new();
    for _ in 0..60 {
        sim.step(&w, focus);
        in_flight.push(sim.falling().len());
        if sim.stats.landings == 5 && sim.falling().is_empty() {
            break;
        }
    }
    assert_eq!(&in_flight[..5], &[1, 2, 3, 4, 5], "the chain starts one block per tick: {in_flight:?}");
    assert!(sim.falling().is_empty(), "everything landed: {in_flight:?}");
    assert_eq!(sim.stats.falls, 5);
    assert_eq!(sim.stats.landings, 5);
    assert_eq!(sim.stats.vanished, 0);
    // The column drained one layer: stone, sand x4, gravel, air.
    assert_eq!(w.block_at(col(k0)), stone);
    for dk in 1..5 {
        assert_eq!(w.block_at(col(k0 + dk)), sand, "sand at +{dk}");
    }
    assert_eq!(w.block_at(col(k0 + 5)), gravel);
    assert_eq!(w.block_at(col(k0 + 6)), air);
    // At rest nothing is scheduled and the ghosts fade.
    for _ in 0..GHOST_TICKS + 1 {
        sim.step(&w, focus);
    }
    assert_eq!(sim.queued(), 0);
    assert!(sim.view().is_empty());
    // The fall of one layer takes sqrt(2 / 25) s = 5.7 ticks: the count first drops in the sixth tick.
    let first_landing = in_flight.windows(2).position(|w| w[1] < w[0]).map(|i| i + 1).unwrap();
    assert!((5..=7).contains(&first_landing), "first landing in tick {first_landing}: {in_flight:?}");
}

#[test]
fn a_block_built_into_a_falling_blocks_path_makes_it_settle_above_it() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (air, stone, sand, _) = ids(&w);
    let col = |k: u32| BlockPos { k, ..g };
    // Sand 20 layers up on a floating stone; the stone is dug away.
    let k_top = g.k + 21;
    w.set_block(col(k_top - 1), stone);
    w.set_block(col(k_top), sand);
    let mut sim = Sim::new(straight_down());
    sim.step(&w, focus);
    w.set_block(col(k_top - 1), air);
    sim.step(&w, focus);
    assert_eq!(sim.falling().len(), 1);
    // Let it fall a few layers, then wall off the cell it is in.
    for _ in 0..8 {
        sim.step(&w, focus);
    }
    let f = sim.falling()[0];
    assert!(f.z < (k_top - 1) as f64 && f.z > (g.k + 2) as f64, "mid-fall at z {}", f.z);
    assert!(f.vel < 0.0);
    let cell = f.cell() as u32;
    w.set_block(col(cell), stone);
    sim.step(&w, focus);
    assert!(sim.falling().is_empty());
    assert_eq!(w.block_at(col(cell)), stone);
    assert_eq!(w.block_at(col(cell + 1)), sand, "settled on the first free cell above");
    assert_eq!(sim.stats.vanished, 0);
    // A second one, left alone, lands on the ground.
    w.set_block(col(k_top), sand);
    for _ in 0..80 {
        sim.step(&w, focus);
    }
    assert!(sim.falling().is_empty());
    assert_eq!(w.block_at(col(cell + 2)), sand, "the second sand lands on the first");
}

#[test]
fn updates_outside_the_active_region_wait_and_persist_and_run_when_it_arrives() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, _, sand, _) = ids(&w);
    let p = w.planet;
    // A sand block in the air 400 m along +i: its own chunk is far outside a 50 m region.
    let far = BlockPos { i: g.i + 400, ..g };
    let far_k = {
        let dir = p.dir(far.face, far.i as f64 + 0.5, far.j as f64 + 0.5);
        let mut k = (p.sea_level as f64 + w.generator.ground_height_m(dir) / p.block_size) as u32 + 4;
        while !w.registry.is_solid(w.block_at(BlockPos { k, ..far })) {
            k -= 1;
        }
        k + 5
    };
    let far = BlockPos { k: far_k, ..far };
    w.set_block(far, sand);
    let mut sim = Sim::new(SimConfig { radius_m: 50.0, ..straight_down() });
    sim.step(&w, focus);
    assert_eq!(sim.falling().len(), 0);
    assert_eq!(sim.queued(), 0);
    assert_eq!(sim.frozen_count(), 7, "the block and its six neighbours wait: {}", sim.frozen_count());
    for _ in 0..10 {
        sim.step(&w, focus);
    }
    assert_eq!(w.block_at(far), sand, "frozen: still in the air");
    // Through a save and a reload the frozen updates survive.
    let snap = sim.snapshot(&w);
    assert_eq!(snap.frozen.iter().map(|(_, v)| v.len()).sum::<usize>(), 7);
    let mut sim2 = Sim::from_snapshot(sim.config, Snapshot::from_bytes(&snap.to_bytes()).unwrap());
    assert_eq!(sim2.frozen_count(), 7);
    // Walk over: the region now covers the block, the updates wake, the sand falls.
    let far_focus = p.pos(far.face, far.i as f64 + 0.5, far.j as f64 + 0.5, far_k as f64 - 4.0);
    sim2.step(&w, far_focus);
    assert_eq!(sim2.frozen_count(), 0);
    assert_eq!(sim2.falling().len(), 1, "woken and falling");
    for _ in 0..40 {
        sim2.step(&w, far_focus);
    }
    assert!(sim2.falling().is_empty());
    assert_eq!(w.block_at(BlockPos { k: far_k - 4, ..far }), sand, "landed on the ground four layers down");
    assert!(w.registry.is_solid(w.block_at(BlockPos { k: far_k - 5, ..far })) && !w.registry.has_gravity(w.block_at(BlockPos { k: far_k - 5, ..far })));
    // Meanwhile a block falling in a chunk that goes inactive hangs where it is.
    let mut sim3 = Sim::new(SimConfig { radius_m: 50.0, ..straight_down() });
    w.set_block(far, sand);
    sim3.step(&w, far_focus);
    sim3.step(&w, far_focus);
    assert_eq!(sim3.falling().len(), 1);
    let z_before = sim3.falling()[0].z;
    for _ in 0..10 {
        sim3.step(&w, focus); // the player is back at the site, 400 m away
    }
    assert_eq!(sim3.falling().len(), 1);
    assert_eq!(sim3.falling()[0].z, z_before, "frozen mid-air");
}

/// Review 3 #7: a falling block never changes an inactive chunk. Its sweep
/// stops at the floor of the active region and it waits there, unchanged,
/// until the region reaches further; the same for a block that has to
/// settle above a cell built into it and for the water a landing displaces.
#[test]
fn a_falling_block_waits_at_the_edge_of_the_active_region() {
    let w = test_world();
    let (g, _) = site(&w);
    let p = w.planet;
    let (air, stone, sand, _) = ids(&w);
    let col = |k: u32| BlockPos { k, ..g };
    // kb: the first layer of a chunk in the air above the site.
    let kb = ((g.k + 8) / 32 + 1) * 32;
    let upper = World::chunk_of(col(kb)).0;
    let lower = World::chunk_of(col(kb - 1)).0;
    assert!(w.set_block(col(kb - 3), stone), "a shelf in the chunk below");
    let at = |k: f64| p.pos(g.face, g.i as f64 + 0.5, g.j as f64 + 0.5, k);
    let focus = at((kb + 31) as f64);
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..straight_down() });
    sim.step(&w, focus);
    assert!(sim.is_active(upper) && !sim.is_active(lower), "only the upper chunk is active");
    // A sand block at terminal speed a tenth of a layer above the edge: one
    // tick carries it 3.5 layers, past the edge.
    let start = Falling { id: sand, face: g.face, i: g.i, j: g.j, z: kb as f64 + 0.1, vel: -TERMINAL_M_S / p.block_size };
    sim.restore(Snapshot { tick: sim.tick(), falling: vec![start], focus: Some(focus), ..Default::default() });
    sim.step(&w, focus);
    assert_eq!(w.block_at(col(kb - 2)), air, "nothing landed in the inactive chunk");
    assert_eq!(sim.stats.landings, 0);
    assert_eq!(sim.falling().len(), 1, "it waits at the edge");
    let held = sim.falling()[0];
    assert_eq!(held.z, kb as f64, "at the floor of the region");
    for _ in 0..5 {
        sim.step(&w, focus);
    }
    assert_eq!(sim.falling(), &[held], "unchanged while it waits");
    // The region reaches down: it lands on the shelf.
    for _ in 0..20 {
        sim.step(&w, at(kb as f64 - 0.5));
    }
    assert!(sim.falling().is_empty());
    assert_eq!(w.block_at(col(kb - 2)), sand, "landed on the shelf");

    // A block in a cell something was built into settles in the first free
    // cell above, but not above the region's top.
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..straight_down() });
    let mid = at((kb + 16) as f64);
    sim.step(&w, mid);
    let above = World::chunk_of(col(kb + 32)).0;
    assert!(sim.is_active(upper) && !sim.is_active(above));
    assert!(w.set_block(col(kb + 31), stone));
    let inside = Falling { id: sand, face: g.face, i: g.i, j: g.j, z: (kb + 31) as f64 + 0.5, vel: -1.0 };
    sim.restore(Snapshot { tick: sim.tick(), falling: vec![inside], focus: Some(mid), ..Default::default() });
    sim.step(&w, mid);
    assert_eq!(w.block_at(col(kb + 32)), air, "the free cell above is outside the region: not written");
    assert_eq!(sim.falling(), &[inside], "it waits");
    sim.step(&w, at((kb + 32) as f64));
    assert!(sim.is_active(above));
    assert!(sim.falling().is_empty());
    assert_eq!(w.block_at(col(kb + 32)), sand, "settled once the cell above was in the region");
}

/// Review 3 #7, the displacement: a landing in water pushes the water up
/// its column; when the column runs out of the active region, the landing
/// waits instead of writing the inactive chunk above.
#[test]
fn a_landing_whose_displaced_water_would_leave_the_region_waits() {
    let w = test_world();
    let (g, _) = site(&w);
    let p = w.planet;
    let (air, stone, sand, _) = ids(&w);
    let water = w.water_id();
    let kb = ((g.k + 8) / 32 + 1) * 32;
    let col = |k: u32| BlockPos { k, ..g };
    // A one-cell shaft of stone from kb + 19 to kb + 41, full of water from
    // kb + 20 to kb + 40: the column crosses into the chunk above.
    let mut writes = vec![(col(kb + 19), stone, None)];
    for k in kb + 20..=kb + 41 {
        for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
            writes.push((BlockPos { i: (g.i as i64 + di) as u32, j: (g.j as i64 + dj) as u32, k, ..g }, stone, None));
        }
        if k <= kb + 40 {
            writes.push((col(k), water, Some(255)));
        }
    }
    w.commit_levels(&writes);
    let focus = p.pos(g.face, g.i as f64 + 0.5, g.j as f64 + 0.5, (kb + 16) as f64);
    let mut sim = Sim::new(SimConfig { radius_m: 1.0, ..SimConfig::default() });
    for _ in 0..3 {
        sim.step(&w, focus);
    }
    let upper = World::chunk_of(col(kb)).0;
    let above = World::chunk_of(col(kb + 32)).0;
    assert!(sim.is_active(upper) && !sim.is_active(above));
    assert_eq!(w.water_level_at(col(kb + 41)), 0);
    let f = Falling { id: sand, face: g.face, i: g.i, j: g.j, z: kb as f64 + 21.5, vel: 0.0 };
    sim.restore(Snapshot { tick: sim.tick(), falling: vec![f], focus: Some(focus), ..Default::default() });
    for _ in 0..30 {
        sim.step(&w, focus);
    }
    assert_eq!(w.block_and_level_at(col(kb + 41)), (air, 0), "no water pushed into the inactive chunk");
    assert_eq!(w.block_at(col(kb + 20)), water, "not landed: it waits above the water");
    assert_eq!(sim.falling().len(), 1);
    assert_eq!(sim.stats.water_lost, 0);
    // The region grows up: the landing goes through, and the water rises into the free cell.
    let up = p.pos(g.face, g.i as f64 + 0.5, g.j as f64 + 0.5, (kb + 32) as f64);
    for _ in 0..30 {
        sim.step(&w, up);
    }
    assert!(sim.is_active(above));
    assert!(sim.falling().is_empty());
    assert_eq!(w.block_at(col(kb + 20)), sand);
    assert_eq!(w.water_level_at(col(kb + 41)), 255, "the displaced cell");
    assert_eq!(sim.stats.water_displaced, 255);
}

/// The scripted session the determinism tests run: a tower on a floating
/// base that straddles a chunk boundary, a 3 x 3 slab dropped at once, a
/// far block that stays frozen, a dig and a block built into a fall; and
/// (M2b) water poured from the air onto the natural ground, a sand block
/// dropped into it and a stone built into it mid-spread.
struct Script {
    g: BlockPos,
    focus: DVec3,
}

impl Script {
    fn new(w: &World) -> Self {
        let (g, focus) = site(w);
        Self { g, focus }
    }

    /// Edits due before tick `t` runs.
    fn before_tick(&self, w: &World, t: u64) {
        let (air, stone, sand, gravel) = ids(w);
        let water = w.water_id();
        let g = self.g;
        let col = |i: u32, j: u32, k: u32| BlockPos { face: g.face, i, j, k };
        // The tower's base sits so that the tower crosses a chunk (cz) boundary.
        let boundary = ((g.k + 6) / 32 + 1) * 32;
        let base = boundary - 4;
        match t {
            1 => {
                w.set_block(col(g.i, g.j, base), stone);
                for dk in 1..7 {
                    w.set_block(col(g.i, g.j, base + dk), if dk % 3 == 0 { gravel } else { sand });
                }
                for di in 0..3 {
                    for dj in 0..3 {
                        w.set_block(col(g.i + 3 + di, g.j + 3 + dj, g.k + 9), sand);
                    }
                }
                w.set_block(col(g.i + 400, g.j, g.k + 30), sand);
            }
            2 => {
                // Three cells of water in the air to the -i side: they fall
                // onto the ground and spread over it.
                for dk in 0..3 {
                    w.set_water(col(g.i - 6, g.j, g.k + 6 + dk), water, 255);
                }
            }
            4 => {
                w.set_block(col(g.i, g.j, base), air);
            }
            9 => {
                // Sand dropped into the spreading water, and a stone built into it.
                w.set_block(col(g.i - 6, g.j, g.k + 12), sand);
                w.set_block(col(g.i - 7, g.j, g.k + 1), stone);
            }
            12 => {
                // Wall off the first falling block's cell, whatever it is.
                w.set_block(col(g.i, g.j, g.k + 5), stone);
            }
            _ => {}
        }
    }
}

fn run_ticks(w: &World, sim: &mut Sim, script: &Script, from: u64, to: u64) {
    for t in from + 1..=to {
        script.before_tick(w, t);
        sim.step(w, script.focus);
        assert_eq!(sim.tick(), t);
    }
}

fn world_sharing(w: &World, edits: Arc<EditStore>) -> World {
    World::new(w.generator.clone(), w.registry.clone(), edits)
}

/// Ticks in a run, and the tick the reload test saves at (mid-fall of the tower).
const TICKS: u64 = 80;
const MID: u64 = 20;

/// One full run from a fresh world: the hash at the end and the hash at MID.
fn straight_run(radius: f64) -> (String, String, u64, u64, u64) {
    let w = test_world();
    let script = Script::new(&w);
    let mut sim = Sim::new(SimConfig { radius_m: radius, ..SimConfig::default() });
    run_ticks(&w, &mut sim, &script, 0, MID);
    let mid = sim.state_hash(&w);
    run_ticks(&w, &mut sim, &script, MID, TICKS);
    assert!(sim.falling().is_empty(), "everything has landed by the end");
    (mid, sim.state_hash(&w), sim.stats.falls, sim.stats.slides, sim.stats.landings)
}

#[test]
fn the_same_inputs_give_the_same_hash_twice_and_the_scenario_is_not_trivial() {
    let (mid_a, a, falls, slides, landings) = straight_run(112.0);
    let (mid_b, b, ..) = straight_run(112.0);
    assert_eq!(a, b);
    assert_eq!(mid_a, mid_b);
    assert_ne!(mid_a, a, "the state moved on after the midpoint");
    assert!(falls + slides >= 15 && landings >= 15, "falls {falls} slides {slides} landings {landings}");
    assert!(slides > 0, "the scenario has blocks sliding too");
    // Another input (a region wide enough to reach the far block, which then falls) hashes differently.
    let (_, c, ..) = straight_run(600.0);
    assert_ne!(a, c);
}

#[test]
fn the_hash_is_the_same_across_rayon_pools_of_one_and_eight_threads() {
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| straight_run(112.0));
    let eight = rayon::ThreadPoolBuilder::new().num_threads(8).build().unwrap().install(|| straight_run(112.0));
    assert_eq!(one.1, eight.1);
    assert_eq!(one.0, eight.0);
}

#[test]
fn the_hash_is_the_same_across_a_save_and_a_reload_in_the_middle() {
    let (_, straight, ..) = straight_run(112.0);
    let w = test_world();
    let script = Script::new(&w);
    let mut sim = Sim::new(SimConfig::default());
    run_ticks(&w, &mut sim, &script, 0, MID);
    assert!(!sim.falling().is_empty(), "the save happens mid-fall");
    assert!(sim.queued() > 0 && sim.stats.water_moved > 0, "and mid-flow: {} queued, {} units moved", sim.queued(), sim.stats.water_moved);
    // Save: chunks and the simulation, through the real save directory.
    let dir = SaveDir::new(std::env::temp_dir().join(format!("triangulum-sim-reload-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.path);
    let cfg = crate::grid::WorldConfig { name: "test".into(), seed: 42, block_size_m: 1.0, world_height: 2048, sea_level: 512, columns_per_face: 131072, bake: String::new(), vertical_scale: None, roughness: 1.0, landform: 1.0 };
    let bake = "ab".repeat(32);
    let snap = sim.snapshot(&w);
    let written = dir.flush(&w.edits, &cfg, &w.registry, &bake, Some(&snap.to_bytes())).unwrap();
    assert!(written >= 2, "several chunks: {written}");
    // Reload into a fresh store, world and simulation.
    let edits = Arc::new(EditStore::new());
    let report = dir.load_into(&cfg, &w.registry, &bake, &edits).unwrap();
    assert_eq!(report.chunks, written);
    let w2 = world_sharing(&w, edits);
    let mut sim2 = Sim::from_snapshot(sim.config, Snapshot::from_bytes(&dir.read_sim().unwrap().unwrap()).unwrap());
    assert_eq!(sim2.state_hash(&w2), sim.state_hash(&w), "the reloaded state is the saved state");
    run_ticks(&w2, &mut sim2, &script, MID, TICKS);
    assert_eq!(sim2.state_hash(&w2), straight);
    let _ = std::fs::remove_dir_all(&dir.path);
}

/// A second world over a copy of `w`'s edited chunks (no pending changes).
pub(super) fn copy_world(w: &World) -> World {
    let edits = Arc::new(EditStore::new());
    for d in w.edits.snapshot_full() {
        edits.insert_chunk(d.key, d.ids, d.water, false);
    }
    world_sharing(w, edits)
}

/// Review 3 #4: the active region depends on where in its chunk it was
/// computed, so a restore must bring that back; before, it was recomputed
/// around wherever the player stood at the reload, and two states with
/// the same hash simulated different cells.
#[test]
fn a_reload_keeps_the_active_region_it_was_computed_for() {
    let w = test_world();
    let (g, _) = site(&w);
    let p = w.planet;
    let ck = World::chunk_of(g).0;
    let (i0, j0, k0) = ck.origin_blocks();
    let at = |x: f64| p.pos(ck.face, i0 as f64 + x, j0 as f64 + 16.0, k0 as f64 + 16.0);
    let (edge, middle) = (at(31.5), at(16.0));
    let cfg = SimConfig { radius_m: 1.0, ..SimConfig::default() };
    // A sand block in the air of the +i neighbour chunk: simulated from the
    // edge's region, not from the middle's.
    let (air, _, sand, _) = ids(&w);
    let col = |k: u64| BlockPos { face: ck.face, i: (i0 + 33) as u32, j: (j0 + 16) as u32, k: k as u32 };
    let k = (k0 + 1..k0 + 31).rev().find(|&k| w.block_at(col(k)) == air && w.block_at(col(k - 1)) == air).expect("air in the neighbour chunk");
    let next = World::chunk_of(col(k)).0;
    let mut probe = Sim::new(cfg);
    probe.step(&w, middle);
    let mut sim = Sim::new(cfg);
    sim.step(&w, edge);
    let from_edge = sim.active_chunks();
    assert!(sim.is_active(next) && !probe.is_active(next), "the scenario needs a chunk the region holds from the edge only");
    assert!(w.set_block(col(k), sand));
    // Walk to the middle of the same chunk (no recompute), then save and restore.
    sim.step(&w, middle);
    assert_eq!(sim.active_chunks(), from_edge, "walking inside a chunk keeps the region");
    let w2 = copy_world(&w);
    let mut back = Sim::from_snapshot(cfg, Snapshot::from_bytes(&sim.snapshot(&w).to_bytes()).unwrap());
    assert_eq!(back.state_hash(&w2), sim.state_hash(&w), "restored as saved");
    for _ in 0..10 {
        sim.step(&w, middle);
        back.step(&w2, middle);
    }
    assert_eq!(back.active_chunks(), sim.active_chunks(), "the restored region is the saved one");
    assert!(sim.stats.falls > 0, "the sand fell in the original");
    assert_eq!(back.state_hash(&w2), sim.state_hash(&w), "equal hashes, equal futures");
}

/// Review 3 #4: what decides the future is in the hash: the region's
/// radius, whether the simulation runs, the water period, the flow settings.
#[test]
fn the_hash_covers_the_simulation_settings() {
    let w = test_world();
    let base = Sim::new(SimConfig::default()).state_hash(&w);
    for (what, cfg) in [
        ("radius", SimConfig { radius_m: 50.0, ..SimConfig::default() }),
        ("enabled", SimConfig { enabled: false, ..SimConfig::default() }),
        ("water period", SimConfig { water_period: 3, ..SimConfig::default() }),
        ("slide", straight_down()),
    ] {
        assert_ne!(Sim::new(cfg).state_hash(&w), base, "{what} is in the hash");
    }
    // The block rules are in it; the colours (look, hot-edited by Andrew's
    // panel) are not.
    let sim = Sim::new(SimConfig::default());
    let with = |edit: &dyn Fn(&mut Vec<crate::world::BlockDef>)| {
        let mut defs = w.registry.defs().to_vec();
        edit(&mut defs);
        World::new(w.generator.clone(), Arc::new(crate::world::Registry::from_defs(defs).unwrap()), w.edits.clone())
    };
    let no_gravity = with(&|d| d.iter_mut().find(|b| b.name == "sand").unwrap().gravity = false);
    assert_ne!(sim.state_hash(&no_gravity), base, "sand that does not fall is another future");
    let recoloured = with(&|d| d.iter_mut().find(|b| b.name == "sand").unwrap().color = [0.9, 0.1, 0.5]);
    assert_eq!(sim.state_hash(&recoloured), base, "a colour is not");
}

/// Not part of `cargo test`: 5,041 sand blocks dropped at once, with the
/// per-tick cost printed. `cargo test --release -- --ignored stress`.
#[test]
#[ignore]
fn stress_five_thousand_sand_blocks_falling_at_once() {
    let w = test_world();
    let (g, focus) = site(&w);
    let (_, _, sand, _) = ids(&w);
    let side = 71u32;
    let t0 = Instant::now();
    for di in 0..side {
        for dj in 0..side {
            w.set_block(BlockPos { face: g.face, i: g.i + di - side / 2, j: g.j + dj - side / 2, k: g.k + 20 }, sand);
        }
    }
    println!("placed {} sand blocks in {:.1} ms", side * side, t0.elapsed().as_secs_f64() * 1e3);
    let mut sim = Sim::new(SimConfig::default());
    let mut costs = Vec::new();
    for _ in 0..200 {
        sim.step(&w, focus);
        costs.push((sim.tick(), sim.stats.last_us, sim.falling().len(), sim.stats.landings));
        if sim.queued() == 0 && sim.falling().is_empty() && sim.tick() > 2 {
            break;
        }
    }
    for (t, us, n, landed) in &costs {
        println!("tick {t:3}: {:8.3} ms  falling {n:5}  landed {landed:5}", *us as f64 / 1e3);
    }
    let worst = costs.iter().map(|c| c.1).max().unwrap();
    let steady: Vec<u32> = costs.iter().filter(|c| c.2 > 0).map(|c| c.1).collect();
    let steady_avg = steady.iter().map(|&u| u as f64).sum::<f64>() / steady.len().max(1) as f64;
    println!("falls {} slides {} landings {} vanished {}; worst tick {:.3} ms; average tick while blocks were in flight {:.3} ms over {} ticks", sim.stats.falls, sim.stats.slides, sim.stats.landings, sim.stats.vanished, worst as f64 / 1e3, steady_avg / 1e3, steady.len());
    assert_eq!(sim.stats.falls, (side * side) as u64);
    assert_eq!(sim.stats.landings + sim.stats.vanished, sim.stats.falls + sim.stats.slides, "every fall and slide ended");
    assert!(sim.falling().is_empty());
}

/// The process's private bytes (Windows), for the memory measurement.
#[cfg(windows)]
fn private_bytes() -> usize {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
    // SAFETY: a valid pseudo-handle and a correctly sized, writable struct.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    assert!(ok != 0, "K32GetProcessMemoryInfo failed");
    c.private
}
#[cfg(not(windows))]
fn private_bytes() -> usize {
    0
}

/// Not part of `cargo test` (review 2 #17): what edited chunks cost as they
/// pile up. For 100, 1,000 and 10,000 edited chunks (one block each, dry or
/// one water cell each): the memory they hold, the state hash, the first
/// tick (each edit's seven updates frozen far from the player), crossing
/// chunk borders with that many frozen chunks, and the simulation snapshot
/// a save writes. Run alone:
/// `cargo test --release -- --ignored edit_memory --nocapture --test-threads=1`
/// (EDIT_COUNTS=100,1000 to pick the sizes).
#[test]
#[ignore]
fn edit_memory_and_costs_by_edited_chunks() {
    let counts: Vec<usize> = std::env::var("EDIT_COUNTS").ok().map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or(vec![100, 1000, 10_000]);
    for &n in &counts {
        for wet in [false, true] {
            let w = test_world();
            let (g, focus) = site(&w);
            let p = w.planet;
            let marker = w.registry.id("marker_red").unwrap();
            let side = (n as f64).sqrt().ceil() as u32;
            let gc = World::chunk_of(g).0;
            // Chunks high in the air (layers 1600..1631), 320 m and more from the player.
            let keys: Vec<ChunkKey> = (0..n as u32).map(|m| ChunkKey::neisor(g.face, 0, (gc.cx + 10 + m % side) % p.chunks_per_edge(0), (gc.cy + 10 + m / side) % p.chunks_per_edge(0), 50)).collect();
            let bytes0 = private_bytes();
            let t = Instant::now();
            for k in &keys {
                let (i0, j0, k0) = k.origin_blocks();
                let bp = BlockPos { face: k.face, i: i0 as u32 + 16, j: j0 as u32 + 16, k: k0 as u32 + 16 };
                let changed = if wet { w.set_water(bp, w.water_id(), 200) } else { w.set_block(bp, marker) };
                assert!(changed);
            }
            let make_ms = t.elapsed().as_secs_f64() * 1e3;
            let bytes1 = private_bytes();
            assert_eq!(w.edits.len(), n);
            let per_chunk = (bytes1 as f64 - bytes0 as f64) / n as f64;
            let sim0 = Sim::new(SimConfig::default());
            let peak0 = private_bytes();
            let t = Instant::now();
            let _ = sim0.state_hash(&w);
            let hash_ms = t.elapsed().as_secs_f64() * 1e3;
            let t = Instant::now();
            let _ = sim0.state_hash(&w);
            let hash2_ms = t.elapsed().as_secs_f64() * 1e3;
            let _ = peak0;
            let mut sim = Sim::new(SimConfig::default());
            let t = Instant::now();
            sim.step(&w, focus);
            let first_ms = t.elapsed().as_secs_f64() * 1e3;
            assert_eq!(sim.frozen_count(), 7 * n, "seven frozen updates per edit");
            let (mut worst, mut sum) = (0u32, 0u64);
            let steps = 20;
            for s in 0..steps {
                let f = p.pos(g.face, g.i as f64 + 0.5 + 32.0 * ((s + 1) % 2) as f64, g.j as f64 + 0.5, g.k as f64 + 2.0);
                let before = sim.stats.refocuses;
                sim.step(&w, f);
                assert_eq!(sim.stats.refocuses, before + 1, "every step crosses a chunk border");
                worst = worst.max(sim.stats.refocus_us);
                sum += sim.stats.refocus_us as u64;
            }
            let t = Instant::now();
            let snap = sim.snapshot(&w).to_bytes();
            let snap_ms = t.elapsed().as_secs_f64() * 1e3;
            println!(
                "{n:6} {} chunks: {:6.1} KiB each ({:.0} MB), made in {make_ms:.0} ms; state hash {hash_ms:.0} ms (again {hash2_ms:.0} ms); first tick {first_ms:.1} ms; refocus avg {:.3} ms worst {:.3} ms with {} frozen chunks; sim snapshot {snap_ms:.1} ms, {} KB",
                if wet { "wet" } else { "dry" },
                per_chunk / 1024.0,
                (bytes1 as f64 - bytes0 as f64) / 1e6,
                sum as f64 / steps as f64 / 1e3,
                worst as f64 / 1e3,
                n,
                snap.len() / 1000
            );
            println!("        the edit store's own count: {:.1} KiB of ids and levels per chunk", w.edits.heap_bytes() as f64 / n as f64 / 1024.0);
        }
    }
}

#[test]
fn a_falling_block_draws_as_the_cube_of_its_id() {
    // Pure check of the quad format against mesh.rs's layout.
    let q = draw::FallingDraws::cube_for_test(4);
    assert_eq!(q.len(), 6);
    for (n, quad) in q.iter().enumerate() {
        let axis = quad[0] & 3;
        let sign = (quad[0] >> 2) & 1;
        assert_eq!(axis as usize, n / 2);
        assert_eq!(sign as usize, n % 2);
        assert_eq!((quad[0] >> 3) & 31, 0, "slice 0");
        assert_eq!((quad[0] >> 18) & 31, 0, "width 1");
        assert_eq!((quad[0] >> 23) & 31, 0, "height 1");
        assert_eq!(quad[1] & 0xFFFF, 4);
        assert_eq!((quad[1] >> 16) & 0xFF, 0xFF, "full brightness");
    }
    let _ = Planet::new(1.0, 2048, 512, 131072);
}

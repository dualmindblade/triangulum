use super::*;
use crate::grid::Planet;
use crate::world::save::SaveDir;
use crate::world::tests::{hills_ground, test_world};
use std::sync::Arc;

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
    };
    let bytes = snap.to_bytes();
    assert_eq!(Snapshot::from_bytes(&bytes).unwrap(), snap);
    assert_eq!(Snapshot::default().to_bytes().len(), 4 + 4 + 8 + 8 * 3);
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
    let mut sim = Sim::new(SimConfig::default());
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
    let mut sim = Sim::new(SimConfig::default());
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
    let mut sim = Sim::new(SimConfig { radius_m: 50.0, enabled: true, water_period: 1 });
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
    let mut sim3 = Sim::new(SimConfig { radius_m: 50.0, enabled: true, water_period: 1 });
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
fn straight_run(radius: f64) -> (String, String, u64, u64) {
    let w = test_world();
    let script = Script::new(&w);
    let mut sim = Sim::new(SimConfig { radius_m: radius, enabled: true, water_period: 1 });
    run_ticks(&w, &mut sim, &script, 0, MID);
    let mid = sim.state_hash(&w.edits);
    run_ticks(&w, &mut sim, &script, MID, TICKS);
    assert!(sim.falling().is_empty(), "everything has landed by the end");
    (mid, sim.state_hash(&w.edits), sim.stats.falls, sim.stats.landings)
}

#[test]
fn the_same_inputs_give_the_same_hash_twice_and_the_scenario_is_not_trivial() {
    let (mid_a, a, falls, landings) = straight_run(112.0);
    let (mid_b, b, _, _) = straight_run(112.0);
    assert_eq!(a, b);
    assert_eq!(mid_a, mid_b);
    assert_ne!(mid_a, a, "the state moved on after the midpoint");
    assert!(falls >= 15 && landings >= 15, "falls {falls} landings {landings}");
    // Another input (a region wide enough to reach the far block, which then falls) hashes differently.
    let (_, c, _, _) = straight_run(600.0);
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
    let (_, straight, _, _) = straight_run(112.0);
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
    assert_eq!(sim2.state_hash(&w2.edits), sim.state_hash(&w.edits), "the reloaded state is the saved state");
    run_ticks(&w2, &mut sim2, &script, MID, TICKS);
    assert_eq!(sim2.state_hash(&w2.edits), straight);
    let _ = std::fs::remove_dir_all(&dir.path);
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
        if sim.stats.landings >= (side * side) as u64 - sim.stats.vanished && sim.falling().is_empty() && sim.tick() > 2 {
            break;
        }
    }
    for (t, us, n, landed) in &costs {
        println!("tick {t:3}: {:8.3} ms  falling {n:5}  landed {landed:5}", *us as f64 / 1e3);
    }
    let worst = costs.iter().map(|c| c.1).max().unwrap();
    let steady: Vec<u32> = costs.iter().filter(|c| c.2 > 0).map(|c| c.1).collect();
    let steady_avg = steady.iter().map(|&u| u as f64).sum::<f64>() / steady.len().max(1) as f64;
    println!("falls {} landings {} vanished {}; worst tick {:.3} ms; average tick while blocks were in flight {:.3} ms over {} ticks", sim.stats.falls, sim.stats.landings, sim.stats.vanished, worst as f64 / 1e3, steady_avg / 1e3, steady.len());
    assert_eq!(sim.stats.falls, (side * side) as u64);
    assert_eq!(sim.stats.landings + sim.stats.vanished, (side * side) as u64);
    assert!(sim.falling().is_empty());
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

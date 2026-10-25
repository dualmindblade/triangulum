//! M3b: adding a block keeps saves and replays working.
//!
//! Andrew adds blocks in the workshop while his world, its save and any
//! replays he recorded already exist. Ids are append-only, so a save made
//! before a block was added holds only the blocks it knew; its header's
//! names are a prefix of the new blocks.json (a renamed block answers to
//! its old name through "was"); and the simulation's state hash covers the
//! rules of the blocks a world can hold (the generator's, and any later one
//! an edit holds), so a block appended and not yet used changes no hash and
//! a replay recorded before still plays back exactly. A real rule change
//! (a block that now falls) is still caught.

use crate::engine::Engine;
use crate::grid::{BlockPos, WorldConfig};
use crate::mesh::MeshOptions;
use crate::player::{Input, Mode, Player, STEP};
use crate::replay::{Runner, read, scratch_dir, session_from};
use crate::session::{Action, Button, Session};
use crate::sim::{Sim, SimConfig, Snapshot, rules_extend};
use crate::stream::LodParams;
use crate::world::blocks::{BlockColours, BlockDef, defs_into_text};
use crate::world::save::{self, SaveDir};
use crate::world::{BlockId, EditStore, Registry};
use std::path::PathBuf;
use std::sync::Arc;

const BAKE: &str = "C:/code/triangulum/game/assets/world/seed42";
const SHIPPED: &str = include_str!("../assets/blocks.json");

/// An engine over `registry` (its own; the other tests share the shipped one).
fn engine(registry: Arc<Registry>) -> (Arc<Engine>, String) {
    let root = env!("CARGO_MANIFEST_DIR");
    let mut config = WorldConfig::load(&format!("{root}/assets/world.json")).unwrap();
    config.bake = BAKE.into();
    let bake: Arc<dyn crate::worldgen::bake::Bake> = Arc::new(crate::worldgen::bake::open(BAKE).expect("world bake"));
    let sha = save::bake_identity(BAKE).unwrap();
    let e = Engine::build(config, registry, bake, Arc::new(EditStore::new()), LodParams { factor: 5.0, max_inflight: 4 }, MeshOptions { split: true, ao: true }, 2, false, Vec::new(), false).unwrap();
    (Arc::new(e), sha)
}

fn temp(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("triangulum-m3b-{tag}-{}", std::process::id()))
}

fn steps(s: &mut Session, n: usize) {
    for _ in 0..n {
        s.step(&Input::default(), STEP);
    }
}

/// Play a replay file in a session built from `template`'s engine.
fn play(template: &Arc<Engine>, sha: &str, path: &std::path::Path, tag: &str) -> crate::replay::Report {
    let t = Session::new(template.clone(), Player::new(glam::DVec3::X * 1e7, 0.0, 0.0, Mode::Fly, 1.0), None, sha.to_string(), Sim::new(SimConfig::default()));
    let rep = read(path).unwrap();
    let scratch = scratch_dir(&format!("m3b-{tag}-{}", std::process::id()));
    let mut s = session_from(&t, &rep, &scratch).unwrap();
    let report = Runner::new(&rep, &s).run(&mut s);
    let _ = std::fs::remove_dir_all(&scratch);
    report
}

#[test]
fn a_save_and_a_replay_made_before_a_block_was_added_still_work() {
    // ---- before: the shipped blocks and "brick", which Andrew had added.
    let before = Arc::new(Registry::parse(SHIPPED).unwrap());
    let brick = before.add(BlockDef::new_solid("brick", BlockColours::flat([0.4, 0.1, 0.08]), false)).unwrap();
    let file_before = defs_into_text(SHIPPED, &before).unwrap();
    let id = |n: &str| before.id(n).unwrap();
    let (eng, sha) = engine(before.clone());
    let eng = Arc::new(eng.with_hotbar(vec![brick, id("stone"), id("sand"), id("water")]));
    let dir = temp("save");
    let _ = std::fs::remove_dir_all(&dir);
    let pr = eng.presets()[0].clone();
    let p = eng.planet;
    let player = Player::new(pr.dir * (p.radius + eng.ground(pr.dir) + 2.0), pr.heading_deg, -32.0, Mode::Walk, p.block_size);
    let mut s = Session::new(eng.clone(), player, Some(SaveDir::new(&dir)), sha.clone(), Sim::new(SimConfig::default()));
    s.teleport(pr.dir, None);
    steps(&mut s, 30);
    // A brick in the world, saved.
    s.apply(Action::Select(0)).unwrap();
    let placed: BlockPos = s.apply(Action::Click(Button::Place)).unwrap().expect("a brick placed");
    assert_eq!(s.engine.world.block_at(placed), brick);
    steps(&mut s, 10);
    assert!(s.save_now().unwrap() > 0);

    // ---- a replay recorded in that world: bricks, sand that falls, water.
    let rp = temp("replay.replay");
    s.start_recording(&rp, "m3b compatibility test").unwrap();
    s.apply(Action::Select(2)).unwrap();
    s.apply(Action::Click(Button::Place)).unwrap();
    steps(&mut s, 40);
    s.apply(Action::Select(0)).unwrap();
    s.apply(Action::Click(Button::Place)).unwrap();
    steps(&mut s, 20);
    s.apply(Action::Click(Button::Break)).unwrap();
    s.apply(Action::Select(3)).unwrap();
    s.apply(Action::Click(Button::Place)).unwrap();
    steps(&mut s, 150);
    let summary = s.stop_recording().unwrap();
    s.save_now().unwrap();
    assert!(s.sim.stats.water_placed > 0, "the replay poured water");
    assert!(play(&eng, &sha, &summary.path, "same").ok(), "the replay plays back as recorded");

    // ---- after: Andrew renames brick and adds another block; the game
    // restarts and reads the new blocks.json.
    let edited = Registry::parse(&file_before).unwrap();
    edited.rename(brick, "red_brick").unwrap();
    let tile = edited.add(BlockDef::new_solid("tile", BlockColours::flat([0.6, 0.6, 0.55]), false)).unwrap();
    let file_after = defs_into_text(&file_before, &edited).unwrap();
    assert!(file_after.contains("\"was\": [\"brick\"]") && file_after.contains("\"tile\""));
    let after = Arc::new(Registry::parse(&file_after).unwrap());
    assert_eq!(after.len(), before.len() + 1);

    // The save loads: its header names "brick" for id 19, which now
    // answers to it, and its chunks hold the brick.
    let store = Arc::new(EditStore::new());
    let (eng_after, _) = engine(after.clone());
    let report = SaveDir::new(&dir).load_into(&eng_after.config, &after, &sha, &store).expect("the old save loads after the new block");
    assert!(report.chunks > 0);
    let world_after = eng_after.with_edits(store);
    assert_eq!(world_after.world.block_at(placed), brick, "the saved brick is still block {brick}");
    assert_eq!(after.name(brick), "red_brick");
    // Its simulation state was made under rules the new blocks extend.
    let snap = Snapshot::from_bytes(&SaveDir::new(&dir).read_sim().unwrap().unwrap()).unwrap();
    assert!(rules_extend(&after, &snap.rules.unwrap()), "no 'other block rules' note");
    // Saving again records the new block in the header.
    {
        let mut s2 = Session::new(Arc::new(world_after), Player::new(placed_dir(&p, placed), 0.0, 0.0, Mode::Fly, 1.0), Some(SaveDir::new(&dir)), sha.clone(), Sim::new(SimConfig::default()));
        let tile_spot = BlockPos { k: placed.k + 3, ..placed };
        assert!(s2.engine.world.set_block(tile_spot, tile));
        s2.save_now().unwrap();
        let header = SaveDir::new(&dir).read_header().unwrap().unwrap();
        assert!(header.blocks.iter().any(|(i, n)| *i == tile && n == "tile"), "{:?}", header.blocks);
        assert!(header.blocks.iter().any(|(i, n)| *i == brick && n == "brick"), "the old name stays in the header");
    }

    // The replay recorded before plays back exactly with the new blocks.
    let r = play(&eng_after, &sha, &summary.path, "after");
    assert!(r.ok(), "{}", r.text());
    assert_eq!(r.matched, r.checks);
    let (rec, got) = r.end_sha.clone().unwrap();
    assert_eq!(rec, got);

    // The same while the game runs: blocks added to the live registry (the
    // panel's Add block) change nothing either.
    before.add(BlockDef::new_solid("glass_like", BlockColours::flat([0.7, 0.8, 0.9]), false)).unwrap();
    before.add(BlockDef::new_solid("planks", BlockColours::flat([0.5, 0.35, 0.2]), true)).unwrap();
    let r = play(&eng, &sha, &summary.path, "live");
    assert!(r.ok(), "{}", r.text());

    // A real change to the rules is still caught: brick that falls.
    let falls = Arc::new(Registry::parse(&file_after).unwrap());
    falls.set_falls(brick, true).unwrap();
    let (eng_falls, _) = engine(falls);
    let r = play(&eng_falls, &sha, &summary.path, "falls");
    assert!(!r.ok(), "a brick that falls is another future");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&rp);
}

fn placed_dir(p: &crate::grid::Planet, b: BlockPos) -> glam::DVec3 {
    p.dir(b.face, b.i as f64 + 0.5, b.j as f64 + 0.5) * p.radius_at(b.k as f64 + 2.0)
}

/// The header check: a renamed block answers to its old name; a
/// renumbered one (the file says something else and does not remember
/// the old name) is still refused, with a hint.
#[test]
fn header_names_survive_a_rename_and_catch_a_renumbering() {
    let before = Registry::parse(SHIPPED).unwrap();
    let brick = before.add(BlockDef::new_solid("brick", BlockColours::flat([0.4, 0.1, 0.08]), false)).unwrap();
    let root = env!("CARGO_MANIFEST_DIR");
    let config = WorldConfig::load(&format!("{root}/assets/world.json")).unwrap();
    let header = save::Header::for_world(&config, &before, "bake");
    let renamed = Registry::parse(&defs_into_text(SHIPPED, &before).unwrap()).unwrap();
    renamed.rename(brick, "red_brick").unwrap();
    assert!(header.check(&config, &renamed, "bake").is_ok());
    let renumbered = Registry::parse(SHIPPED).unwrap();
    renumbered.add(BlockDef::new_solid("glass", BlockColours::flat([0.7, 0.8, 0.9]), false)).unwrap();
    let err = header.check(&config, &renumbered, "bake").unwrap_err();
    assert!(err.contains("block id 19 is 'brick' in the save but 'glass'") && err.contains("\"was\""), "{err}");
    let _: BlockId = brick;
}

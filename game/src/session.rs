//! A play session: the player in a world, with picking, breaking, placing,
//! saving and the state the HUD and the harness report. The windowed app
//! and the scripted harness both drive this, so what a script checks is
//! what the game does.
//!
//! The driver (M2c). Everything a player does reaches the game in one of
//! two ways, both at a fixed 60 Hz step boundary: the held movement keys
//! go into `step`, and every other input is an `Action` given to `apply`
//! before the step it belongs to. The app queues window events and applies
//! them at the next step (`controls.rs`), the harness applies them as its
//! script says, and a replay applies them from its file, so a session is a
//! function of its step-positioned inputs and nothing else: not the frame
//! rate, not the wall clock. While a replay is being recorded (`recorder`)
//! the same two calls write it.

use crate::engine::Engine;
use crate::grid::BlockPos;
use crate::player::{self, Frame, Hit, Input, Mode, Player, REACH_M};
use crate::replay::Recorder;
use crate::sim::{STEPS_PER_TICK, Sim, Snapshot};
use crate::sky::Clock;
use crate::world::save::{self, SaveDir};
use crate::world::{BlockId, EditStore};
use glam::DVec3;
use std::sync::Arc;
use std::time::Instant;

#[cfg(test)]
#[path = "save_tests.rs"]
mod save_tests;

/// What a mouse button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Left: break the targeted block (or remove a water cell).
    Break,
    /// Right: place the selected block.
    Place,
}

/// Mouse look: radians per raw mouse count.
pub const LOOK_RAD_PER_COUNT: f64 = 0.0022;
/// Pitch limit (radians).
pub const PITCH_MAX: f64 = 1.55;
/// Holding a mouse button repeats its action every this many fixed steps
/// (0.25 s), counted in steps, so a held click breaks the same blocks at
/// the same steps whatever the frame rate (review 2 #16).
pub const REPEAT_STEPS: u64 = 15;

/// A teleport with an altitude stops this far above the ground, and this far
/// above sea level (§15.7).
pub const TELEPORT_MIN_AGL_M: f64 = 2.5;
pub const TELEPORT_MAX_ALT_M: f64 = 80_000_000.0;
/// A teleport with no altitude stands in water up to this deep; over anything
/// deeper it hovers this far above the surface, not on the bottom.
const TELEPORT_WADE_M: f64 = 1.5;
const TELEPORT_ABOVE_WATER_M: f64 = 30.0;

/// One input, applied at a step boundary (`Session::apply`). Toggles are
/// turned into the state they set before they are recorded (`ToggleMode`
/// is recorded as the `SetMode` it became), so a replay file reads as what
/// happened.
#[derive(Clone, Debug)]
pub enum Action {
    /// Raw mouse motion in counts (x right, y down) since the last one.
    Look(f64, f64),
    /// A mouse button went down: it acts now, then every `REPEAT_STEPS`
    /// while held.
    Press(Button),
    /// The held button went up (or the window lost focus).
    Release,
    /// One click: acts once (the harness's `tap break|place`).
    Click(Button),
    /// Hotbar slot (0-based).
    Select(usize),
    ToggleMode,
    SetMode(Mode),
    /// R: drop to the ground here, in walk mode.
    Ground,
    /// P: go to preset place n (`Engine::presets`).
    Preset(usize),
    ToggleWater,
    /// Hide (true) or show water: a view flag.
    HideWater(bool),
    /// Scroll wheel, in lines: fly speed times 1.3 per line.
    Scroll(f64),
    /// `]` (true) or `[`: the next sky clock speed.
    Time(bool),
    /// Set the sky clock (seconds and speed).
    Sky(Clock),
    /// Run or pause the simulation.
    Sim(bool),
    SimRadius(f64),
    WaterPeriod(u64),
    /// How powder and water move beyond straight down.
    Flow(crate::sim::Flow),
    /// Put the player exactly in this state (the harness's teleports and
    /// look commands, as the recorder sees them).
    Pose(Box<Player>),
    /// Save, then rebuild the world and the simulation from the save.
    Reload,
}

pub struct Session {
    pub engine: Arc<Engine>,
    pub player: Player,
    /// None: never load or write saves (bench, shots).
    pub save: Option<SaveDir>,
    pub bake_sha256: String,
    pub last_save: Option<Instant>,
    pub saved_chunks: usize,
    save_worker: save::SaveWorker,
    save_exists: bool,
    /// Persistent until a successful retry; the app shows this even with HUD off.
    pub save_error: Option<String>,
    retry_save_at: Option<Instant>,
    saving_revision: Option<u64>,
    saved_state: Option<(u64, [u8; 32])>,
    /// The simulation (M2a): ticks every `STEPS_PER_TICK` player steps.
    pub sim: Sim,
    steps: u64,
    /// The sky clock, advanced on the same fixed steps (M2c: the app, the
    /// harness and a replay all keep it here; renderers copy it).
    pub sky: Clock,
    /// The mouse button being held and the step it last acted on.
    held: Option<(Button, u64)>,
    /// The report of the last `Action::Reload`.
    pub last_reload: Option<save::LoadReport>,
    /// A replay being written (M2c): `apply` and `step` feed it.
    pub recorder: Option<Recorder>,
    /// What the picture is made of (`visual.rs`), when the driver knows: the
    /// app and the harness set it, a recording holds it, and a new value during
    /// a recording is written down. Nothing in the simulation reads it.
    pub visual: Option<Arc<crate::visual::Visual>>,
}

/// Quit's small state machine is driven by redraws and completion polls. The
/// first waiting frame must be presented before any final snapshot is taken.
#[derive(Default)]
pub struct SaveOnExit {
    displayed: bool,
    final_started: bool,
}

impl SaveOnExit {
    pub fn presented(&mut self) { self.displayed = true; }

    /// true only when the in-flight generation AND the final state are durable.
    /// An error cancels quit; the session retains the dirt and the visible error.
    pub fn poll(&mut self, session: &mut Session) -> Result<bool, String> {
        if !self.displayed { return Ok(false); }
        if session.save_in_flight() {
            match session.poll_save() {
                None => return Ok(false),
                Some(result) => { result?; }
            }
        }
        if self.final_started { return Ok(true); }
        self.final_started = true;
        Ok(!session.request_save()?)
    }
}

impl Session {
    pub fn new(engine: Arc<Engine>, player: Player, save: Option<SaveDir>, bake_sha256: String, sim: Sim) -> Self {
        engine.edits.set_slopes(sim.config.flow.powder != crate::sim::Powder::Blocks);
        let save_exists = save.as_ref().is_some_and(SaveDir::exists);
        Self { engine, player, save, bake_sha256, last_save: None, saved_chunks: 0,
            save_worker: save::SaveWorker::default(), save_exists, save_error: None, retry_save_at: None, saving_revision: None, saved_state: None,
            sim, steps: 0, sky: Clock { seconds: 0.0, speed: 0.0 }, held: None, last_reload: None, recorder: None, visual: None }
    }

    pub fn planet(&self) -> &crate::grid::Planet {
        &self.engine.planet
    }

    /// One fixed 60 Hz step with these keys held: the player, every third
    /// step one simulation tick (20 Hz), the sky clock at its own speed
    /// (it never reaches the simulation), and a held mouse button's repeat.
    /// Everything on this thread.
    pub fn step(&mut self, input: &Input, dt: f64) {
        if let Some(mut rec) = self.recorder.take() {
            rec.before_step(self, input, dt);
            self.recorder = Some(rec);
        }
        let world = self.engine.world.clone();
        self.player.step(world.as_ref(), &self.engine.planet, input, dt);
        self.steps += 1;
        if self.sim.config.enabled && self.steps % STEPS_PER_TICK as u64 == 0 {
            self.sim.step(world.as_ref(), self.player.pos);
        }
        self.sky.advance(dt);
        if let Some((button, since)) = self.held {
            if self.steps - since >= REPEAT_STEPS {
                self.click(button);
                self.held = Some((button, self.steps));
            }
        }
        if let Some(mut rec) = self.recorder.take() {
            rec.after_step(self);
            self.recorder = Some(rec);
        }
    }

    /// Fixed steps taken so far.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The held mouse button and the step it last acted on.
    pub fn held(&self) -> Option<(Button, u64)> {
        self.held
    }

    /// Restore the step counter and the held button (a replay's start: the
    /// tick falls on every third step, and a held button repeats on its
    /// own schedule).
    pub fn restore_steps(&mut self, steps: u64, held: Option<(Button, u64)>) {
        self.steps = steps;
        self.held = held;
    }

    /// Apply one input at the current step boundary (see the module doc).
    /// Returns the block a click changed, if any; an error only from a
    /// failed reload.
    pub fn apply(&mut self, action: Action) -> Result<Option<BlockPos>, String> {
        let action = match action {
            Action::ToggleMode => Action::SetMode(if self.player.mode == Mode::Walk { Mode::Fly } else { Mode::Walk }),
            Action::ToggleWater => Action::HideWater(!self.engine.hide_water),
            Action::Time(faster) => {
                let mut c = self.sky;
                c.change_speed(faster);
                Action::Sky(c)
            }
            a => a,
        };
        if let Some(mut rec) = self.recorder.take() {
            rec.before_action(self, &action);
            self.recorder = Some(rec);
        }
        let mut out = Ok(None);
        match action {
            Action::Look(dx, dy) => {
                let p = &mut self.player;
                p.heading += dx * LOOK_RAD_PER_COUNT;
                p.pitch = (p.pitch - dy * LOOK_RAD_PER_COUNT).clamp(-PITCH_MAX, PITCH_MAX);
            }
            Action::Press(b) => {
                out = Ok(self.click(b));
                self.held = Some((b, self.steps));
            }
            Action::Release => self.held = None,
            Action::Click(b) => out = Ok(self.click(b)),
            Action::Select(slot) => self.select(slot),
            Action::SetMode(m) => self.set_mode(m),
            Action::Ground => {
                let dir = self.player.pos.normalize();
                self.set_mode(Mode::Walk);
                self.teleport(dir, None);
            }
            Action::Preset(i) => self.apply_preset(i),
            Action::HideWater(hide) => {
                self.set_hide_water(hide);
            }
            Action::Scroll(lines) => {
                let p = &mut self.player;
                p.fly_speed = (p.fly_speed * 1.3f64.powf(lines)).clamp(1.0, 5000.0);
            }
            Action::Sky(c) => self.sky = c,
            Action::Sim(on) => self.sim.config.enabled = on,
            Action::SimRadius(m) => self.sim.set_radius(m),
            Action::WaterPeriod(n) => self.sim.config.water_period = n.max(1),
            Action::Flow(f) => {
                self.sim.config.flow = f;
                // Partial powder stays what it is if the rule goes back: only
                // the switch on says so, and a script's next scene is its own.
                if f.powder != crate::sim::Powder::Blocks {
                    self.engine.edits.set_slopes(true);
                }
            }
            Action::Pose(p) => self.player = *p,
            Action::Reload => match self.reload() {
                Ok((_, report)) => self.last_reload = Some(report),
                Err(e) => out = Err(e),
            },
            Action::ToggleMode | Action::ToggleWater | Action::Time(_) => unreachable!("normalised above"),
        }
        if let Some(mut rec) = self.recorder.take() {
            rec.after(self);
            self.recorder = Some(rec);
        }
        out
    }

    fn click(&mut self, b: Button) -> Option<BlockPos> {
        match b {
            Button::Break => self.break_block(),
            Button::Place => self.place_block(),
        }
    }

    /// Go to preset place `i` (P in the app): in walk mode on the ground,
    /// or flying when the preset is high.
    pub fn apply_preset(&mut self, i: usize) {
        let presets = self.engine.presets();
        if presets.is_empty() {
            return;
        }
        let p = presets[i % presets.len()].clone();
        self.player.heading = p.heading_deg.to_radians();
        self.player.pitch = p.pitch_deg.to_radians();
        if p.alt_agl > 50.0 {
            self.set_mode(Mode::Fly);
            let ground = self.engine.ground(p.dir);
            self.teleport(p.dir, Some(ground + p.alt_agl));
        } else {
            self.set_mode(Mode::Walk);
            self.teleport(p.dir, None);
        }
        println!("preset {}: {}", i % presets.len(), p.name);
    }

    /// Is a replay being recorded?
    pub fn recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// Start recording a replay to `path` (F10 in the app, `replay record`
    /// in the harness). The save is brought up to date first when there is
    /// one (the replay names its generation); the whole current state goes
    /// into the file as its starting snapshot.
    pub fn start_recording(&mut self, path: &std::path::Path, note: &str) -> Result<std::path::PathBuf, String> {
        if self.recorder.is_some() {
            return Err("already recording a replay".into());
        }
        self.save_now()?;
        let rec = Recorder::start(self, path, note)?;
        let p = rec.path().to_path_buf();
        self.recorder = Some(rec);
        Ok(p)
    }

    /// Finish the replay being recorded: a last check, the end line with
    /// the full state hash, and the file closed.
    pub fn stop_recording(&mut self) -> Result<crate::replay::Summary, String> {
        let Some(rec) = self.recorder.take() else { return Err("not recording a replay".into()) };
        rec.finish(self)
    }

    /// The world-state hash (`Sim::state_hash`), after absorbing pending changes.
    pub fn sim_hash(&mut self) -> String {
        let world = self.engine.world.clone();
        let _ = self.sim.snapshot(world.as_ref());
        self.sim.state_hash(world.as_ref())
    }

    /// Restore the simulation from a save directory's `sim.bin`, or start
    /// it fresh when there is none.
    /// The settings are `config` (the command line's, or the running
    /// session's on a reload); a save made under other settings or other
    /// block rules says so.
    ///
    /// Partial powder (B7) lives in a save of its own: a save and a game
    /// that disagree about `--powder levels` refuse each other, either way,
    /// and neither is touched. To the normal game partial powder would be whole
    /// blocks, and sand would multiply.
    pub fn load_sim(dir: Option<&SaveDir>, config: crate::sim::SimConfig, registry: &crate::world::Registry) -> Result<Sim, String> {
        use crate::sim::Powder;
        let levels = config.flow.powder != Powder::Blocks;
        let bytes = dir.map(|d| d.read_sim()).transpose()?.flatten();
        if let (true, None, Some(d)) = (levels, &bytes, dir)
            && d.exists()
        {
            return Err(format!("{} is not a --powder levels save, so this game will not touch it. Without --save it keeps its own, beside the real one", d.path.display()));
        }
        match bytes {
            Some(bytes) => {
                let snap = Snapshot::from_bytes(&bytes).map_err(|e| format!("refusing to load the simulation state of {}: {e}", dir.unwrap().path.display()))?;
                if snap.config.is_some_and(|c| c.flow.powder != Powder::Blocks) != levels {
                    let d = dir.unwrap().path.display();
                    return Err(if levels {
                        format!("{d} is not a --powder levels save, so this game will not touch it. Without --save it keeps its own, beside the real one")
                    } else {
                        format!("{d} was played with --powder levels and holds partial powder, which this game would turn into whole blocks. It is left as it is: start with --powder levels to go on with it")
                    });
                }
                let header = dir.unwrap().read_header()?.ok_or("simulation save has no header.json")?;
                crate::sim::check_saved_rules(header.sim_rules)?;
                if let Some(note) = crate::sim::check_saved_rules(Some(snap.sim_rules))? {
                    println!("note: {note}");
                }
                let names: std::collections::HashMap<_, _> = header.blocks.iter().map(|(id, name)| (*id, name)).collect();
                for f in &snap.falling {
                    let name = names.get(&f.id).ok_or_else(|| format!("falling block id {} has no recorded identity in header.json", f.id))?;
                    if f.id as usize >= registry.len() || !registry.def(f.id).answers_to(name) {
                        return Err(format!("falling block id {} does not match its saved identity {name:?}", f.id));
                    }
                }
                if let Some(saved) = snap.config.filter(|c| *c != config) {
                    println!("note: the save's simulation ran with radius {} m, {}, water period {}, {}; it continues with radius {} m, {}, water period {}, {}", saved.radius_m, if saved.enabled { "on" } else { "off" }, saved.water_period, saved.flow.text(), config.radius_m, if config.enabled { "on" } else { "off" }, config.water_period, config.flow.text());
                }
                if snap.rules.is_some_and(|r| !crate::sim::rules_extend(registry, &r)) {
                    println!("note: the save was made under other block rules (blocks.json's solid, liquid, gravity or replaceable flags changed); it continues under the current ones");
                }
                Ok(Sim::from_snapshot(config, snap))
            }
            None => Ok(Sim::new(config)),
        }
    }

    /// The block under the crosshair within reach. Holding water (the
    /// bucket, M2b) the ray also stops at water, so a click removes or
    /// places water; holding anything else it passes through water to the
    /// floor, as before.
    pub fn pick(&self) -> Option<Hit> {
        let liquids = self.engine.registry.is_liquid(self.selected_block());
        player::pick(self.engine.world.as_ref(), &self.engine.planet, self.player.eye(), self.player.look_dir(), REACH_M, liquids)
    }

    pub fn selected_block(&self) -> BlockId {
        let hb = &self.engine.hotbar;
        hb[self.player.selected.min(hb.len().saturating_sub(1))]
    }

    pub fn select(&mut self, slot: usize) {
        if slot < self.engine.hotbar.len() {
            self.player.selected = slot;
        }
    }

    /// Left click: the picked block becomes air. A water cell (picked while
    /// holding water) is removed whole, and the units are counted.
    pub fn break_block(&mut self) -> Option<BlockPos> {
        let hit = self.pick()?;
        let air = self.engine.generator.blocks.air;
        let world = self.engine.world.as_ref();
        let (id, level) = world.block_and_level_at(hit.block);
        let changed = world.set_block(hit.block, air);
        if changed && self.engine.registry.is_liquid(id) {
            self.sim.stats.water_removed += level as u64;
        }
        changed.then_some(hit.block)
    }

    /// Right click: the selected block goes into the cell in front of the
    /// picked face, unless that would be inside the player or outside the
    /// world. Water places a full cell (topping up a partial one). A solid
    /// block placed into water pushes the water up its column (M2b
    /// displacement); water that finds no room is lost and counted.
    pub fn place_block(&mut self) -> Option<BlockPos> {
        let hit = self.pick()?;
        let cell = hit.place_cell();
        let bp = self.engine.world.resolve_block(hit.face, cell.0, cell.1, cell.2)?;
        if self.player.mode == Mode::Walk && self.player.overlaps_cell(&self.engine.planet, bp.face, (bp.i as i64, bp.j as i64, bp.k as i64)) {
            return None;
        }
        let id = self.selected_block();
        let world = self.engine.world.as_ref();
        let reg = &self.engine.registry;
        let (mut bp, (mut cur, mut level)) = (bp, world.block_and_level_at(bp));
        // A ray that crossed a partial cell above its slope (B7) points at
        // the cube behind it, and the cell in front of that is the partial one:
        // what goes there goes on top of it.
        if reg.has_gravity(cur) && level != 0 && self.engine.edits.slopes_on() {
            bp = world.resolve_block(bp.face, bp.i as i64, bp.j as i64, bp.k as i64 + 1)?;
            (cur, level) = world.block_and_level_at(bp);
        }
        if reg.is_solid(cur) {
            return None;
        }
        if reg.is_liquid(id) {
            let changed = world.set_block(bp, id);
            if changed {
                self.sim.stats.water_placed += (crate::world::edits::WATER_FULL - level) as u64;
            }
            return changed.then_some(bp);
        }
        let (mut writes, lost) = world.displace_writes(bp);
        writes.insert(0, (bp, id, None));
        let changed = world.commit_levels(&writes) > 0;
        if changed && reg.is_liquid(cur) {
            self.sim.stats.water_displaced += level as u64 - lost as u64;
            self.sim.stats.water_lost += lost as u64;
        }
        changed.then_some(bp)
    }

    pub fn toggle_mode(&mut self) {
        let mode = if self.player.mode == Mode::Walk { Mode::Fly } else { Mode::Walk };
        self.set_mode(mode);
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.player.mode = mode;
        self.player.vel = DVec3::ZERO;
        if mode == Mode::Walk {
            // Leaving fly mode inside rock: climb out so the first step does not.
            let f = Frame::at(&self.engine.planet, self.player.pos);
            let world = self.engine.world.as_ref();
            let cell = (f.i.floor() as i64, f.j.floor() as i64, (f.k + 0.05).floor() as i64);
            if self.engine.registry.is_solid(world.block_lattice(f.face, cell.0, cell.1, cell.2)) {
                let dir = self.player.pos.normalize();
                let k = f.k;
                self.player.place_on_ground(world, &self.engine.planet, dir, k);
            }
        }
    }

    /// Teleport to stand on the ground under `dir` (walk) or hover at `alt_m` above sea level (fly).
    pub fn teleport(&mut self, dir: DVec3, alt_asl_m: Option<f64>) {
        let planet = self.engine.planet;
        match alt_asl_m {
            Some(alt) => {
                self.player.pos = dir * (planet.radius + alt);
                self.player.vel = DVec3::ZERO;
                self.player.on_ground = false;
            }
            None => {
                let ground = self.engine.ground(dir);
                let k_hint = planet.sea_level as f64 + ground / planet.block_size + 2.0;
                self.player.place_on_ground(self.engine.world.as_ref(), &planet, dir, k_hint);
            }
        }
    }

    /// The player after a teleport to `dir`, with this session unchanged: the
    /// T window hands this to `Action::Pose`, so a recording holds where it
    /// went. With an altitude (m above sea level) it hovers there, in fly
    /// mode, clamped above the ground; without one it stands on the ground in
    /// walk mode, or hovers over water too deep to stand in.
    pub fn teleport_pose(&self, dir: DVec3, alt_asl_m: Option<f64>) -> Player {
        let planet = self.engine.planet;
        let mut p = self.player.clone();
        p.vel = DVec3::ZERO;
        p.on_ground = false;
        let hover = |p: &mut Player, alt: f64| {
            p.mode = Mode::Fly;
            p.pos = dir * (planet.radius + alt);
        };
        match alt_asl_m {
            Some(alt) => {
                let low = self.engine.ground(dir) + TELEPORT_MIN_AGL_M;
                hover(&mut p, alt.max(low).min(TELEPORT_MAX_ALT_M));
            }
            None => {
                let c = self.engine.generator.column(dir, planet.block_size);
                if c.wet() && c.water_m - c.height_m as f64 > TELEPORT_WADE_M {
                    hover(&mut p, c.water_m + TELEPORT_ABOVE_WATER_M);
                } else {
                    p.mode = Mode::Walk;
                    let ground = self.engine.ground(dir);
                    let k_hint = planet.sea_level as f64 + ground / planet.block_size + 2.0;
                    p.place_on_ground(self.engine.world.as_ref(), &planet, dir, k_hint);
                }
            }
        }
        p
    }

    /// Stand on top of block (face, i, j, k); in fly mode, hover there with
    /// the feet at layer k + 1 exactly.
    pub fn teleport_block(&mut self, bp: BlockPos) {
        let planet = self.engine.planet;
        let dir = planet.dir(bp.face, bp.i as f64 + 0.5, bp.j as f64 + 0.5);
        if self.player.mode == Mode::Fly {
            self.player.pos = dir * planet.radius_at(bp.k as f64 + 1.0);
            self.player.vel = DVec3::ZERO;
        } else {
            self.player.place_on_ground(self.engine.world.as_ref(), &planet, dir, bp.k as f64 + 1.0);
        }
    }

    pub fn edits(&self) -> &Arc<EditStore> {
        &self.engine.edits
    }

    /// Poll completion, then start a snapshot if the debounce says so.
    /// A return value counts durable completions only, never submitted work.
    pub fn save_if_due(&mut self, now: Instant) -> Result<usize, String> {
        if let Some(result) = self.poll_save() { return result; }
        if !self.save_in_flight() && self.retry_save_at.is_none_or(|at| now >= at)
            && self.save.is_some() && (self.engine.edits.due_for_save(now) || self.save_error.is_some()) {
            self.request_save()?;
        }
        Ok(0)
    }

    pub fn save_in_flight(&self) -> bool { self.save_worker.busy() }

    fn save_failed(&mut self, error: String) -> String {
        self.save_error = Some(error.clone());
        self.retry_save_at = Some(Instant::now() + std::time::Duration::from_secs(2));
        error
    }

    fn save_state(&self) -> (u64, [u8; 32]) {
        (self.engine.edits.generation(), crate::sha256::sha256(&self.sim.snapshot_for_save(&self.engine.world).to_bytes()))
    }

    /// Used before detaching the real world for a terrain preview. A completed
    /// save changes no simulation flags, so compare against its captured state.
    pub fn has_unsaved_changes(&self) -> bool {
        self.save_in_flight() || self.save_error.is_some() || self.engine.edits.dirty_count() > 0
            || self.saved_state.map_or_else(|| self.sim.is_dirty(), |s| s != self.save_state())
    }

    /// Capture now, write on the worker. Busy requests are coalesced without
    /// allocating a queued snapshot; remaining dirt is saved on the next turn.
    /// No filesystem operation or simulation mutation runs here.
    pub fn request_save(&mut self) -> Result<bool, String> {
        if self.save_in_flight() { return Ok(false); }
        let Some(dir) = self.save.clone() else { return Ok(false) };
        if !(self.engine.edits.dirty_count() > 0 || self.sim.is_dirty() || self.sim.has_state() || self.save_exists) {
            return Ok(false);
        }
        let snapshot = save::SaveSnapshot::capture(&self.engine, &self.bake_sha256, &self.sim, save::SNAPSHOT_LIMIT)
            .map_err(|e| self.save_failed(e))?;
        debug_assert!(snapshot.bytes <= save::SNAPSHOT_LIMIT);
        self.saving_revision = Some(self.engine.edits.generation());
        self.save_worker.start(dir, snapshot, self.engine.edits.clone()).map_err(|e| self.save_failed(e))?;
        Ok(true)
    }

    fn finish_save(&mut self, wait: bool) -> Option<Result<usize, String>> {
        self.save_worker.finish(wait).map(|result| match result {
            Ok(saved) => {
                let n = saved.chunks;
                self.last_save = Some(Instant::now());
                self.saved_chunks += n;
                self.save_exists = true;
                self.saved_state = self.saving_revision.take().map(|rev| (rev, saved.sim_digest));
                self.save_error = None;
                self.retry_save_at = None;
                Ok(n)
            }
            Err(e) => {
                self.saving_revision = None;
                Err(self.save_failed(e))
            }
        })
    }

    pub fn poll_save(&mut self) -> Option<Result<usize, String>> { self.finish_save(false) }

    pub fn wait_for_save(&mut self) -> Result<usize, String> { self.finish_save(true).unwrap_or(Ok(0)) }

    /// Barrier for the harness, reload and recording: finish the old snapshot,
    /// then capture and finish the latest state. Simulation stays paused here.
    pub fn save_now(&mut self) -> Result<usize, String> {
        let n = self.wait_for_save()?;
        self.request_save()?;
        Ok(n + self.wait_for_save()?)
    }

    /// Flush, then rebuild the world and the simulation from the save on
    /// disk (what quitting and starting again does). Returns the new
    /// engine; the caller resets its streamer.
    pub fn reload(&mut self) -> Result<(Arc<Engine>, save::LoadReport), String> {
        self.save_now()?;
        let edits = Arc::new(EditStore::new());
        let report = match &self.save {
            Some(dir) => dir.load_into(&self.engine.config, &self.engine.registry, &self.bake_sha256, &edits)?,
            None => save::LoadReport { chunks: 0, warnings: Vec::new(), generation: 0 },
        };
        edits.set_slopes(self.engine.edits.slopes_on());
        let engine = Arc::new(self.engine.with_edits(edits));
        self.engine = engine.clone();
        self.sim = Self::load_sim(self.save.as_ref(), self.sim.config, &self.engine.registry)?;
        self.saved_state = Some(self.save_state());
        Ok((engine, report))
    }

    /// Swap water on or off (a new generator; edits are shared).
    pub fn set_hide_water(&mut self, hide: bool) -> Arc<Engine> {
        let engine = Arc::new(self.engine.with_flags(hide, self.engine.markers_on));
        self.engine = engine.clone();
        engine
    }

    /// The machine-readable state: what the HUD shows and scripts assert on.
    pub fn state(&self) -> serde_json::Value {
        let planet = &self.engine.planet;
        let p = &self.player;
        let f = Frame::at(planet, p.pos);
        let dir = p.pos.normalize();
        let (lat, lon) = crate::grid::Planet::lat_lon_deg(dir);
        let alt = p.pos.length() - planet.radius;
        let ground = self.engine.ground(dir);
        let (fi, fj, fk) = f.feet_cell();
        let world = self.engine.world.as_ref();
        let name = |id: BlockId| self.engine.registry.def(id).name.clone();
        let below = world.block_lattice(f.face, fi, fj, (f.k - 0.01).floor() as i64);
        let up = dir;
        let vh = p.vel - up * p.vel.dot(up);
        let pick = self.pick();
        let mut v = serde_json::json!({
            "mode": if p.mode == Mode::Walk { "walk" } else { "fly" },
            "lat": lat, "lon": lon,
            "alt": alt, "alt_agl": alt - ground, "ground": ground,
            "face": f.face, "i": fi, "j": fj, "k": fk,
            "fi": f.i, "fj": f.j, "fk": f.k,
            "x": p.pos.x, "y": p.pos.y, "z": p.pos.z,
            "heading": p.heading.to_degrees(), "pitch": p.pitch.to_degrees(),
            "on_ground": p.on_ground, "in_water": p.in_water, "head_in_water": p.head_in_water,
            "vel_up": p.vel.dot(up), "speed": vh.length(),
            "selected": p.selected + 1, "selected_block": name(self.selected_block()),
            "block_below": name(below),
            "edits": self.engine.edits.len(), "dirty": self.engine.edits.dirty_count(),
            "saved_chunks": self.saved_chunks,
            "hide_water": self.engine.hide_water,
            "crosshair": serde_json::Value::Null,
        });
        // The simulation (M2a); a second object, the json! macro has a depth limit.
        let sim = serde_json::json!({
            "sim_enabled": self.sim.config.enabled,
            "sim_tick": self.sim.tick(),
            "falling": self.sim.falling().len(),
            "queued": self.sim.queued(),
            "frozen": self.sim.frozen_count(),
            "sim_active_chunks": self.sim.active_chunks(),
            "falls": self.sim.stats.falls, "slides": self.sim.stats.slides, "landings": self.sim.stats.landings, "vanished": self.sim.stats.vanished,
            "tick_ms_last": self.sim.stats.last_us as f64 / 1e3,
            "tick_ms_avg": self.sim.stats.avg_us() / 1e3,
            "tick_ms_max": self.sim.stats.max_us as f64 / 1e3,
            "refocus_ms": self.sim.stats.refocus_us as f64 / 1e3,
        });
        // Water (M2b), in level units: 255 = one full cell.
        let ws = &self.sim.stats;
        let water = serde_json::json!({
            "water_moved": ws.water_moved, "water_writes": ws.water_writes,
            "water_in": ws.water_in, "water_out": ws.water_out,
            "water_placed": ws.water_placed, "water_removed": ws.water_removed,
            "water_displaced": ws.water_displaced, "water_lost": ws.water_lost,
            "water_dried": ws.water_dried, "drying": self.sim.drying_count(),
            "water_balance": ws.water_balance(),
            "water_updates_last": ws.water_updates_last,
            "water_period": self.sim.config.water_period,
            "powder_moves": ws.powder_moves, "powder_moved": ws.powder_moved,
            "flow": self.sim.config.flow.text(),
            "sim_radius": self.sim.config.radius_m,
            // The sky clock (M2c: the session advances it on its steps).
            "sky_seconds": self.sky.seconds, "sky_speed": self.sky.speed,
            "steps": self.steps,
            "recording": self.recorder.is_some(),
        });
        if let (serde_json::Value::Object(m), serde_json::Value::Object(s), serde_json::Value::Object(w)) = (&mut v, sim, water) {
            m.extend(s);
            m.extend(w);
        }
        if let Some(h) = pick {
            v["crosshair"] = serde_json::Value::String(name(h.id));
            v["crosshair_face"] = h.block.face.into();
            v["crosshair_i"] = h.block.i.into();
            v["crosshair_j"] = h.block.j.into();
            v["crosshair_k"] = h.block.k.into();
            v["crosshair_dist"] = h.t.into();
        }
        v
    }

    /// Block name at lattice coordinates (for `block[f,i,j,k]` asserts).
    pub fn block_name(&self, face: u8, i: i64, j: i64, k: i64) -> String {
        self.engine.registry.def(self.engine.world.block_lattice(face, i, j, k)).name.clone()
    }

    /// Block name relative to the feet block.
    pub fn block_name_rel(&self, di: i64, dj: i64, dk: i64) -> String {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, ck) = f.feet_cell();
        self.block_name(f.face, ci + di, cj + dj, ck + dk)
    }

    /// Water level (0..255) at lattice coordinates (for `level[f,i,j,k]`
    /// asserts); 0 outside the world.
    pub fn level_at(&self, face: u8, i: i64, j: i64, k: i64) -> u8 {
        self.engine.world.resolve_block(face, i, j, k).map_or(0, |bp| self.engine.world.water_level_at(bp))
    }

    /// Water level relative to the feet block.
    pub fn level_rel(&self, di: i64, dj: i64, dk: i64) -> u8 {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, ck) = f.feet_cell();
        self.level_at(f.face, ci + di, cj + dj, ck + dk)
    }

    /// Total water (units) in a box of cells relative to the feet block,
    /// corners inclusive (for `water_rel[...]` asserts).
    pub fn water_in_box_rel(&self, a: (i64, i64, i64), b: (i64, i64, i64)) -> u64 {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, ck) = f.feet_cell();
        let mut sum = 0u64;
        for di in a.0.min(b.0)..=a.0.max(b.0) {
            for dj in a.1.min(b.1)..=a.1.max(b.1) {
                for dk in a.2.min(b.2)..=a.2.max(b.2) {
                    sum += self.level_at(f.face, ci + di, cj + dj, ck + dk) as u64;
                }
            }
        }
        sum
    }

    /// Wet cells and level-1 cells in a box of cells relative to the feet
    /// block, corners inclusive (for `wet_rel[...]` and `film_rel[...]`).
    pub fn wet_in_box_rel(&self, a: (i64, i64, i64), b: (i64, i64, i64)) -> (u64, u64) {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, ck) = f.feet_cell();
        let (mut wet, mut film) = (0, 0);
        for di in a.0.min(b.0)..=a.0.max(b.0) {
            for dj in a.1.min(b.1)..=a.1.max(b.1) {
                for dk in a.2.min(b.2)..=a.2.max(b.2) {
                    let l = self.level_at(f.face, ci + di, cj + dj, ck + dk);
                    wet += (l > 0) as u64;
                    film += (l == 1) as u64;
                }
            }
        }
        (wet, film)
    }

    /// The powder (units: 255 is a whole block) in a box of cells relative
    /// to the feet block, corners inclusive (for `powder_rel[...]`).
    pub fn powder_in_box_rel(&self, a: (i64, i64, i64), b: (i64, i64, i64)) -> u64 {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, ck) = f.feet_cell();
        let world = &self.engine.world;
        let mut sum = 0u64;
        for di in a.0.min(b.0)..=a.0.max(b.0) {
            for dj in a.1.min(b.1)..=a.1.max(b.1) {
                for dk in a.2.min(b.2)..=a.2.max(b.2) {
                    let Some(bp) = world.resolve_block(f.face, ci + di, cj + dj, ck + dk) else { continue };
                    let (id, level) = world.block_and_level_at(bp);
                    if self.engine.registry.has_gravity(id) {
                        sum += crate::sim::powder::units(level) as u64;
                    }
                }
            }
        }
        sum
    }

    /// How high the powder in column DI, DJ stands, in blocks over the top of
    /// layer DK_LOW (the feet block's layers), looking down from DK_HIGH: for
    /// the harness's `surface` and for measuring an angle.
    pub fn surface_rel(&self, di: i64, dj: i64, low: i64, high: i64) -> Option<f64> {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, ck) = f.feet_cell();
        let world = &self.engine.world;
        let bp = world.resolve_block(f.face, ci + di, cj + dj, ck)?;
        (low + 1..=high).rev().find_map(|dk| {
            let (id, level) = world.block_and_level_at(BlockPos { k: (ck + dk) as u32, ..bp });
            self.engine.registry.has_gravity(id).then(|| (dk - low - 1) as f64 + crate::sim::powder::units(level) as f64 / 255.0)
        })
    }

    /// Set a box of cells relative to the feet block, corners inclusive (the
    /// harness's `fill`: building a scene without a click per cell). Returns
    /// the cells that changed.
    pub fn fill_rel(&self, a: (i64, i64, i64), b: (i64, i64, i64), id: BlockId, level: Option<u8>) -> usize {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, ck) = f.feet_cell();
        let world = &self.engine.world;
        let mut writes = Vec::new();
        for di in a.0.min(b.0)..=a.0.max(b.0) {
            for dj in a.1.min(b.1)..=a.1.max(b.1) {
                for dk in a.2.min(b.2)..=a.2.max(b.2) {
                    if let Some(bp) = world.resolve_block(f.face, ci + di, cj + dj, ck + dk) {
                        writes.push((bp, id, level));
                    }
                }
            }
        }
        world.commit_levels(&writes)
    }

    /// Blocks in flight in the column DI, DJ from the feet (for `falling_rel[DI,DJ]` asserts).
    pub fn falling_in_column_rel(&self, di: i64, dj: i64) -> usize {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, _) = f.feet_cell();
        let Some(bp) = self.engine.world.resolve_block(f.face, ci + di, cj + dj, 0) else { return 0 };
        self.sim.falling().iter().filter(|b| b.face == bp.face && b.i == bp.i && b.j == bp.j).count()
    }
}

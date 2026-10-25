//! A play session: the player in a world, with picking, breaking, placing,
//! saving and the state the HUD and the harness report. The windowed app
//! and the scripted harness both drive this, so what a script checks is
//! what the game does.

use crate::engine::Engine;
use crate::grid::BlockPos;
use crate::player::{self, Frame, Hit, Input, Mode, Player, REACH_M};
use crate::sim::{STEPS_PER_TICK, Sim, Snapshot};
use crate::world::save::{self, SaveDir};
use crate::world::{BlockId, EditStore};
use glam::DVec3;
use std::sync::Arc;
use std::time::Instant;

pub struct Session {
    pub engine: Arc<Engine>,
    pub player: Player,
    /// None: never load or write saves (bench, shots).
    pub save: Option<SaveDir>,
    pub bake_sha256: String,
    pub last_save: Option<Instant>,
    pub saved_chunks: usize,
    /// The simulation (M2a): ticks every `STEPS_PER_TICK` player steps.
    pub sim: Sim,
    steps: u64,
}

impl Session {
    pub fn new(engine: Arc<Engine>, player: Player, save: Option<SaveDir>, bake_sha256: String, sim: Sim) -> Self {
        Self { engine, player, save, bake_sha256, last_save: None, saved_chunks: 0, sim, steps: 0 }
    }

    pub fn planet(&self) -> &crate::grid::Planet {
        &self.engine.planet
    }

    /// One fixed 60 Hz step of the player, and every third step one
    /// simulation tick (20 Hz), on this thread. The sky clock is advanced
    /// by the caller on the same steps with its own speed; it never reaches
    /// the simulation.
    pub fn step(&mut self, input: &Input, dt: f64) {
        let world = self.engine.world.clone();
        self.player.step(world.as_ref(), &self.engine.planet, input, dt);
        self.steps += 1;
        if self.sim.config.enabled && self.steps % STEPS_PER_TICK as u64 == 0 {
            self.sim.step(world.as_ref(), self.player.pos);
        }
    }

    /// Fixed steps taken so far (the app's input repeat counts these).
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The world-state hash (`Sim::state_hash`), after absorbing pending changes.
    pub fn sim_hash(&mut self) -> String {
        let world = self.engine.world.clone();
        let _ = self.sim.snapshot(world.as_ref());
        self.sim.state_hash(&self.engine.edits)
    }

    /// Restore the simulation from a save directory's `sim.bin`, or start
    /// it fresh when there is none.
    pub fn load_sim(dir: Option<&SaveDir>, config: crate::sim::SimConfig) -> Result<Sim, String> {
        match dir.map(|d| d.read_sim()).transpose()?.flatten() {
            Some(bytes) => {
                let snap = Snapshot::from_bytes(&bytes).map_err(|e| format!("refusing to load the simulation state of {}: {e}", dir.unwrap().path.display()))?;
                Ok(Sim::from_snapshot(config, snap))
            }
            None => Ok(Sim::new(config)),
        }
    }

    /// The block under the crosshair within reach.
    pub fn pick(&self) -> Option<Hit> {
        player::pick(self.engine.world.as_ref(), &self.engine.planet, self.player.eye(), self.player.look_dir(), REACH_M)
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

    /// Left click: the picked block becomes air.
    pub fn break_block(&mut self) -> Option<BlockPos> {
        let hit = self.pick()?;
        let air = self.engine.generator.blocks.air;
        self.engine.world.set_block(hit.block, air).then_some(hit.block)
    }

    /// Right click: the selected block goes into the cell in front of the
    /// picked face, unless that would be inside the player or outside the world.
    pub fn place_block(&mut self) -> Option<BlockPos> {
        let hit = self.pick()?;
        let cell = hit.place_cell();
        let bp = self.engine.world.resolve_block(hit.face, cell.0, cell.1, cell.2)?;
        if self.player.mode == Mode::Walk && self.player.overlaps_cell(&self.engine.planet, hit.face, cell) {
            return None;
        }
        let id = self.selected_block();
        let cur = self.engine.world.block_at(bp);
        if self.engine.registry.is_solid(cur) {
            return None;
        }
        self.engine.world.set_block(bp, id).then_some(bp)
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

    /// Write dirty chunks if the debounce says so.
    pub fn save_if_due(&mut self, now: Instant) -> Result<usize, String> {
        if self.save.is_some() && self.engine.edits.due_for_save(now) {
            return self.save_now();
        }
        Ok(0)
    }

    /// Write the dirty chunks and the simulation state. The state goes
    /// along whenever chunks are written, whenever it changed, and whenever
    /// the save already exists (so its tick stays current); an untouched
    /// world creates nothing.
    pub fn save_now(&mut self) -> Result<usize, String> {
        let Some(dir) = &self.save else { return Ok(0) };
        let world = self.engine.world.clone();
        let write_sim = self.engine.edits.dirty_count() > 0 || self.sim.is_dirty() || self.sim.has_state() || dir.exists();
        let snap = write_sim.then(|| self.sim.snapshot(world.as_ref()).to_bytes());
        let n = dir.flush(&self.engine.edits, &self.engine.config, &self.engine.registry, &self.bake_sha256, snap.as_deref())?;
        if n > 0 {
            self.last_save = Some(Instant::now());
            self.saved_chunks += n;
        }
        Ok(n)
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
        let engine = Arc::new(self.engine.with_edits(edits));
        self.engine = engine.clone();
        self.sim = Self::load_sim(self.save.as_ref(), self.sim.config)?;
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
            "falls": self.sim.stats.falls, "landings": self.sim.stats.landings, "vanished": self.sim.stats.vanished,
            "tick_ms_last": self.sim.stats.last_us as f64 / 1e3,
            "tick_ms_avg": self.sim.stats.avg_us() / 1e3,
            "tick_ms_max": self.sim.stats.max_us as f64 / 1e3,
            "refocus_ms": self.sim.stats.refocus_us as f64 / 1e3,
        });
        if let (serde_json::Value::Object(m), serde_json::Value::Object(s)) = (&mut v, sim) {
            m.extend(s);
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

    /// Blocks in flight in the column DI, DJ from the feet (for `falling_rel[DI,DJ]` asserts).
    pub fn falling_in_column_rel(&self, di: i64, dj: i64) -> usize {
        let f = Frame::at(&self.engine.planet, self.player.pos);
        let (ci, cj, _) = f.feet_cell();
        let Some(bp) = self.engine.world.resolve_block(f.face, ci + di, cj + dj, 0) else { return 0 };
        self.sim.falling().iter().filter(|b| b.face == bp.face && b.i == bp.i && b.j == bp.j).count()
    }
}

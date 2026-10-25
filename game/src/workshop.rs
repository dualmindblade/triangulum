//! Andrew's workshop, first piece (M3a): the look panel (F1).
//!
//! The panel puts the look's dials in Andrew's hands: the sky and the
//! light, the water, every block's colours (top, sides, bottom), the
//! distance detail, and the terrain. Every change applies live:
//!
//! - render dials go into the sky uniform the next frame (`render/sky.rs`);
//! - block colours rewrite the GPU colour table (meshes hold block ids, so
//!   nothing is remeshed and everything in view changes at once);
//! - the LOD factor moves the streamer's split distance;
//! - terrain dials rebuild the generator and the streamer as a *preview
//!   world*: a fresh, empty edit store, saving off, the real save, its
//!   edits and its simulation set aside untouched until "Back to my world".
//!
//! "Save look" writes `look.json` and the colours into `blocks.json` (its
//! style and ids kept). Terrain reaches `world.json` only through "Make
//! this the default", which says that existing saves will not load with
//! it. All three files are watched: an edit in a text editor (or a pull)
//! applies within half a second. The screenshot button writes the frame
//! without the panel, and a JSON sidecar with every look value.
//!
//! The app calls `update` before it renders, `ui` inside its egui pass and
//! `shot_pending` / `save_frame` after painting. With the panel closed the
//! per-frame cost is a few comparisons and, twice a second, three file
//! metadata reads.

use crate::engine::Engine;
use crate::grid::WorldConfig;
use crate::look::{self, FileWatch, Look, Terrain};
use crate::player::Mode;
use crate::render::Renderer;
use crate::session::Session;
use crate::sim::Sim;
use crate::sim::draw::FallingDraws;
use crate::sky::Settings;
use crate::stream::Streamer;
use crate::world::blocks::{self, BlockColours};
use crate::world::save::SaveDir;
use crate::world::{BlockId, EditStore, Registry};
use serde_json::{Value, json};
use std::f64::consts::{PI, TAU};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// The blocks file as shipped with this build: "Reset" in the Blocks section.
const SHIPPED_BLOCKS: &str = include_str!("../assets/blocks.json");
const POLL: Duration = Duration::from_millis(500);
/// A terrain dial applies this long after it last moved (and once released).
const TERRAIN_SETTLE: Duration = Duration::from_millis(350);
const NOTE_LIFE: Duration = Duration::from_secs(25);

/// The real world, set aside while a preview world is shown.
struct Home {
    engine: Arc<Engine>,
    save: Option<SaveDir>,
    sim: Sim,
}

pub struct Workshop {
    pub open: bool,
    /// The mouse was captured when the panel opened; closing gives it back.
    pub recapture: bool,
    look: Look,
    /// What the renderer and streamer last received.
    look_applied: Option<Look>,
    /// What look.json and blocks.json hold (as last read or written).
    saved_look: Look,
    saved_colours: Vec<BlockColours>,
    defaults: Look,
    look_path: PathBuf,
    blocks_path: PathBuf,
    world_path: PathBuf,
    /// look.json, blocks.json, world.json.
    watches: [FileWatch; 3],
    last_poll: Instant,
    names: Vec<String>,
    /// Colours per id: as the panel shows them, and as shipped.
    colours: Vec<BlockColours>,
    colours_default: Vec<BlockColours>,
    colours_dirty: bool,
    water: Option<BlockId>,
    deep_ocean: Option<BlockId>,
    /// Mean strata rock under the far ocean, solved from the shipped
    /// colours (deep_ocean = water at the default opacity over this rock).
    ocean_floor: [f32; 3],
    /// Terrain dials: shown, built, the save's world's, world.json's.
    terrain: Terrain,
    terrain_running: Terrain,
    terrain_home: Terrain,
    terrain_file: Terrain,
    terrain_moved: Option<Instant>,
    terrain_held: bool,
    earth_vscale: f64,
    home: Option<Home>,
    confirm_default: bool,
    notes: Vec<(Instant, String)>,
    hour_now: f64,
    speed_now: f64,
    set_hour: Option<f64>,
    set_speed: Option<f64>,
    shot_requested: bool,
    shot_now: bool,
    styled: bool,
    /// Evidence screenshots: `TRIANGULUM_PANEL_SECTION=blocks` opens the
    /// panel on that section alone for its first frames.
    focus: Option<(String, u32)>,
}

impl Workshop {
    /// The workshop for a session, with the files and look main installed
    /// (`look::install`); without them, the shipped files and the M1 look.
    pub fn new(engine: &Engine) -> Self {
        let st = look::startup().cloned().unwrap_or_else(|| {
            let root = env!("CARGO_MANIFEST_DIR");
            let d = Look::defaults(&Settings::default(), engine.planet.radius);
            look::Startup { look: d.clone(), defaults: d, look_path: format!("{root}/assets/look.json").into(), blocks_path: format!("{root}/assets/blocks.json").into(), world_path: format!("{root}/assets/world.json").into(), panel_open: false }
        });
        let terrain = Terrain { roughness: engine.config.roughness, landform: engine.config.landform, vscale: engine.vscale() };
        let mut w = Self::from_parts(&engine.registry, st, terrain, engine.planet.radius / 6_371_000.0);
        // A --lod-factor on the command line wins over the file for this run.
        w.look.detail.lod_factor = engine.lod.factor;
        w
    }

    /// The workshop over a registry and terrain (no engine: the tests).
    pub fn from_parts(registry: &Registry, st: look::Startup, terrain: Terrain, earth_vscale: f64) -> Self {
        let names: Vec<String> = registry.defs().iter().map(|d| d.name.clone()).collect();
        let colours = registry.colours();
        // Shipped colours by name (a --blocks file may number differently).
        let shipped = Registry::parse(SHIPPED_BLOCKS).ok();
        let colours_default: Vec<BlockColours> = names
            .iter()
            .zip(&colours)
            .map(|(n, c)| shipped.as_ref().and_then(|s| s.id(n).ok().map(|id| s.def(id).colours())).unwrap_or(*c))
            .collect();
        let find = |n: &str| registry.id(n).ok();
        let (water, deep_ocean) = (registry.defs().iter().find(|d| d.liquid).map(|d| d.id), find("deep_ocean"));
        let a = st.defaults.water.opacity as f32;
        let ocean_floor = match (water, deep_ocean) {
            (Some(w), Some(d)) if a < 1.0 => {
                let (w, d) = (colours_default[w as usize].faces()[0], colours_default[d as usize].color);
                [0, 1, 2].map(|i| ((d[i] - a * w[i]) / (1.0 - a)).max(0.0))
            }
            _ => [0.28, 0.27, 0.25],
        };
        let terrain_file = WorldConfig::load(&st.world_path.to_string_lossy())
            .map(|c| Terrain { roughness: c.roughness, landform: c.landform, vscale: c.vertical_scale.unwrap_or(earth_vscale) })
            .unwrap_or(terrain);
        let watches = [FileWatch::new(st.look_path.clone()), FileWatch::new(st.blocks_path.clone()), FileWatch::new(st.world_path.clone())];
        Self {
            open: st.panel_open,
            recapture: false,
            look: st.look.clone(),
            look_applied: None,
            saved_look: st.look.tidy(),
            saved_colours: colours.clone(),
            defaults: st.defaults.clone(),
            look_path: st.look_path,
            blocks_path: st.blocks_path,
            world_path: st.world_path,
            watches,
            last_poll: Instant::now(),
            names,
            colours,
            colours_default,
            colours_dirty: true,
            water,
            deep_ocean,
            ocean_floor,
            terrain,
            terrain_running: terrain,
            terrain_home: terrain,
            terrain_file,
            terrain_moved: None,
            terrain_held: false,
            earth_vscale,
            home: None,
            confirm_default: false,
            notes: Vec::new(),
            hour_now: 12.0,
            speed_now: 1.0,
            set_hour: None,
            set_speed: None,
            shot_requested: false,
            shot_now: false,
            styled: false,
            focus: std::env::var("TRIANGULUM_PANEL_SECTION").ok().map(|s| (s, 5)),
        }
    }

    pub fn look(&self) -> &Look {
        &self.look
    }

    /// Is the session showing a preview world (saving off)?
    pub fn previewing(&self) -> bool {
        self.home.is_some()
    }

    fn note(&mut self, text: impl Into<String>) {
        let text = text.into();
        println!("look panel: {text}");
        self.notes.push((Instant::now(), text));
        if self.notes.len() > 6 {
            self.notes.remove(0);
        }
    }

    /// Does a text field in the panel have the keyboard (so game keys wait)?
    pub fn typing(&self, ctx: &egui::Context) -> bool {
        self.open && ctx.egui_wants_keyboard_input()
    }

    // ------------------------------------------------------- per frame

    /// Apply what changed: files on disk, the panel's edits, time, terrain.
    pub fn update(&mut self, session: &mut Session, streamer: &mut Streamer, falling: &mut FallingDraws, gpu: &mut Renderer) {
        let now = Instant::now();
        if now.duration_since(self.last_poll) >= POLL {
            self.last_poll = now;
            self.poll_files();
        }
        // Time of day and its speed.
        let lon = session.player.pos.y.atan2(session.player.pos.x);
        if let Some(h) = self.set_hour.take() {
            gpu.sky.clock.seconds = seek_hour(&gpu.sky.settings, gpu.sky.clock.seconds, lon, h);
        }
        if let Some(s) = self.set_speed.take() {
            gpu.sky.clock.speed = s;
        }
        if self.open {
            self.hour_now = local_hour(&gpu.sky.settings, gpu.sky.clock.seconds, lon);
            self.speed_now = gpu.sky.clock.speed;
        }
        // The look and the distance detail.
        if self.look_applied.as_ref() != Some(&self.look) {
            gpu.sky.look = self.look.clone();
            streamer.params.factor = self.look.detail.lod_factor;
            streamer.cover.factor = self.look.detail.lod_factor;
            self.look_applied = Some(self.look.clone());
        }
        // Block colours: the GPU table, no remeshing.
        if self.apply_colours(&session.engine.registry) {
            gpu.set_colours(&session.engine.registry.colour_table());
        }
        // Terrain, once a dial has come to rest.
        if self.terrain_moved.is_some_and(|t| !self.terrain_held && now.duration_since(t) >= TERRAIN_SETTLE) {
            self.terrain_moved = None;
            if !self.terrain.same(&self.terrain_running) {
                self.rebuild(session, streamer, falling, gpu);
            }
        }
        self.notes.retain(|(t, _)| now.duration_since(*t) < NOTE_LIFE);
        // A requested screenshot is this frame, drawn without the panel.
        self.shot_now = std::mem::take(&mut self.shot_requested);
    }

    /// Keep the far ocean in step with the water, then push the colours
    /// into the registry. True when the registry's colours changed.
    pub fn apply_colours(&mut self, registry: &Registry) -> bool {
        self.follow_water();
        if !std::mem::take(&mut self.colours_dirty) {
            return false;
        }
        registry.set_colours(&self.colours)
    }

    /// The far ocean (`deep_ocean`, the opaque sea of the coarsest
    /// chunks) = the water colour at the water's opacity over the ocean
    /// floor, when the look says it follows the water.
    fn follow_water(&mut self) {
        if let (true, Some(w), Some(d)) = (self.look.water.far_ocean_follows_water, self.water, self.deep_ocean) {
            let a = self.look.water.opacity as f32;
            let wc = self.colours[w as usize].faces()[0];
            let far = [0, 1, 2].map(|i| wc[i] * a + self.ocean_floor[i] * (1.0 - a));
            let c = &mut self.colours[d as usize];
            if (0..3).any(|i| (c.color[i] - far[i]).abs() > 5e-5) || c.per_face() {
                *c = BlockColours { color: far, top: None, side: None, bottom: None };
                self.colours_dirty = true;
            }
        }
    }

    /// Reload whichever of the three files changed on disk.
    pub fn poll_files(&mut self) {
        if self.watches[0].changed() {
            match Look::load(&self.look_path, &self.defaults) {
                Ok(l) => {
                    if l != self.saved_look {
                        self.look = l.clone();
                        self.saved_look = l;
                        self.note("look.json changed on disk: applied.");
                    }
                }
                Err(e) => self.note(format!("look.json changed but was not applied: {e}")),
            }
        }
        if self.watches[1].changed() {
            match Registry::load(&self.blocks_path.to_string_lossy()) {
                Ok(r) => {
                    // Blocks whose colours changed in the file take them;
                    // unsaved panel edits to other blocks stay.
                    let mut n = 0;
                    for d in r.defs() {
                        let id = d.id as usize;
                        if self.names.get(id) == Some(&d.name) && self.saved_colours[id] != d.colours() {
                            self.colours[id] = d.colours();
                            self.saved_colours[id] = d.colours();
                            n += 1;
                        }
                    }
                    self.colours_dirty = true;
                    let restart = r.defs().len() != self.names.len() || r.defs().iter().any(|d| self.names.get(d.id as usize) != Some(&d.name));
                    self.note(format!(
                        "blocks.json changed on disk: {n} block colour(s) applied.{}",
                        if restart { " New or renamed blocks (and their flags) need a restart." } else { "" }
                    ));
                }
                Err(e) => self.note(format!("blocks.json changed but was not applied: {e}")),
            }
        }
        if self.watches[2].changed() {
            match WorldConfig::load(&self.world_path.to_string_lossy()) {
                Ok(c) => {
                    let t = Terrain { roughness: c.roughness, landform: c.landform, vscale: c.vertical_scale.unwrap_or(self.earth_vscale) };
                    // Only a change to the terrain dials themselves: other
                    // world.json fields (size, sky calendar) need a restart.
                    if !t.same(&self.terrain_file) {
                        self.terrain_file = t;
                        if !t.same(&self.terrain_running) {
                            self.terrain = t;
                            self.terrain_moved = Some(Instant::now() - TERRAIN_SETTLE);
                            self.note("world.json's terrain changed on disk: showing it as a preview world (saving off).");
                        }
                    }
                }
                Err(e) => self.note(format!("world.json changed but was not read: {e}")),
            }
        }
    }

    // ------------------------------------------------------- terrain

    /// Build the world for the terrain dials: the save's own world when
    /// they are its dials, otherwise a preview world with saving off.
    fn rebuild(&mut self, session: &mut Session, streamer: &mut Streamer, falling: &mut FallingDraws, gpu: &mut Renderer) {
        let target = self.terrain;
        if target.same(&self.terrain_home) {
            self.go_home(session, streamer, falling, gpu);
            return;
        }
        let t0 = Instant::now();
        let base = self.home.as_ref().map_or_else(|| session.engine.clone(), |h| h.engine.clone());
        let mut config = base.config.clone();
        config.roughness = target.roughness;
        config.landform = target.landform;
        config.vertical_scale = Some(target.vscale);
        let mut lod = base.lod;
        lod.factor = self.look.detail.lod_factor;
        let built = Engine::build(config, base.registry.clone(), base.bake.clone(), Arc::new(EditStore::new()), lod, base.mesh_opts, base.skirt, session.engine.hide_water, base.markers.clone(), base.markers_on);
        let mut engine = match built {
            Ok(e) => e,
            Err(e) => {
                self.terrain = self.terrain_running;
                self.note(format!("That terrain could not be built: {e}"));
                return;
            }
        };
        engine.hotbar = base.hotbar.clone();
        if self.home.is_none() {
            // Leave the real world saved: unsaved edits (or simulation
            // changes) go to disk now, and nothing reaches its save until
            // we come back. With nothing unsaved the save is not touched.
            let unsaved = session.engine.edits.dirty_count() > 0 || session.sim.is_dirty();
            if let Err(e) = if unsaved { session.save_now().map(|_| ()) } else { Ok(()) } {
                self.terrain = self.terrain_running;
                self.note(format!("Your world could not be saved, so the preview did not start: {e}"));
                return;
            }
            let fresh = Sim::new(session.sim.config);
            let sim = std::mem::replace(&mut session.sim, fresh);
            self.home = Some(Home { engine: session.engine.clone(), save: session.save.take(), sim });
        } else {
            session.sim = Sim::new(session.sim.config);
        }
        session.engine = Arc::new(engine);
        self.swap_world(session, streamer, falling, gpu);
        self.terrain_running = target;
        self.note(format!(
            "Preview world built in {:.1} s: roughness {:.2}, landforms {:.2}, vertical scale 1/{:.0}. Saving is off and your save is untouched; anything you dig here is thrown away.",
            t0.elapsed().as_secs_f64(),
            target.roughness,
            target.landform,
            1.0 / target.vscale
        ));
    }

    fn go_home(&mut self, session: &mut Session, streamer: &mut Streamer, falling: &mut FallingDraws, gpu: &mut Renderer) {
        self.terrain = self.terrain_home;
        self.terrain_moved = None;
        let Some(h) = self.home.take() else { return };
        let hide_water = session.engine.hide_water;
        session.engine = h.engine;
        session.save = h.save;
        session.sim = h.sim;
        if session.engine.hide_water != hide_water {
            // F was pressed in the preview: a view flag, keep it.
            session.set_hide_water(hide_water);
        }
        self.swap_world(session, streamer, falling, gpu);
        self.terrain_running = self.terrain_home;
        self.note("Back in your world: saving is on again.");
    }

    /// Drop every chunk of the old world and stream the session's engine;
    /// put the player back on (or above) the new ground.
    fn swap_world(&mut self, session: &mut Session, streamer: &mut Streamer, falling: &mut FallingDraws, gpu: &mut Renderer) {
        falling.clear(gpu);
        streamer.clear(gpu);
        *streamer = session.engine.streamer();
        streamer.params.factor = self.look.detail.lod_factor;
        streamer.cover.factor = self.look.detail.lod_factor;
        let dir = session.player.pos.normalize();
        if session.player.mode == Mode::Walk {
            session.teleport(dir, None);
        } else {
            let ground = session.engine.ground(dir);
            if session.player.pos.length() - session.engine.planet.radius < ground + 2.0 {
                session.teleport(dir, Some(ground + 2.0));
            }
        }
    }

    fn make_default(&mut self) {
        match look::save_terrain_default(&self.world_path, &self.terrain_running, self.earth_vscale) {
            Ok(()) => {
                self.watches[2].mark();
                self.terrain_file = self.terrain_running;
                self.note(format!(
                    "Wrote this terrain to {} as the default. Existing saves will NOT load with it: next time the game starts, a save made with the old terrain is refused. Play it again by putting world.json back (git checkout game/assets/world.json), or start fresh with --save saves/another-name.",
                    self.world_path.display()
                ));
            }
            Err(e) => self.note(format!("world.json was not written: {e}")),
        }
    }

    // ------------------------------------------------------- files

    fn save_look(&mut self) {
        self.follow_water();
        let look = self.look.tidy();
        let a = look.save(&self.look_path);
        self.watches[0].mark();
        let b = blocks::save_colours(&self.blocks_path.to_string_lossy(), &self.colours);
        self.watches[1].mark();
        match (a, b) {
            (Ok(()), Ok(())) => {
                self.look = look.clone();
                self.saved_look = look;
                self.saved_colours = self.colours.clone();
                self.note(format!("Saved {} and {}. To share the look, commit and push those two files (or send them).", self.look_path.display(), self.blocks_path.display()))
            }
            (a, b) => self.note(format!("Saving failed: {}", [a.err(), b.err()].into_iter().flatten().collect::<Vec<_>>().join("; "))),
        }
    }

    /// Unsaved changes to the look or the block colours?
    pub fn unsaved(&self) -> bool {
        self.look.tidy() != self.saved_look || self.colours != self.saved_colours
    }

    fn revert(&mut self) {
        match Look::load(&self.look_path, &self.defaults) {
            Ok(l) => {
                self.look = l.clone();
                self.saved_look = l;
            }
            Err(e) => self.note(format!("look.json: {e}")),
        }
        match Registry::load(&self.blocks_path.to_string_lossy()) {
            Ok(r) => {
                for d in r.defs() {
                    if self.names.get(d.id as usize) == Some(&d.name) {
                        self.colours[d.id as usize] = d.colours();
                        self.saved_colours[d.id as usize] = d.colours();
                    }
                }
                self.colours_dirty = true;
            }
            Err(e) => self.note(format!("blocks.json: {e}")),
        }
        self.note("Back to the look as last saved.");
    }

    // ------------------------------------------------------- screenshots

    /// True on the frame a screenshot was asked for: the app reads the
    /// frame back after painting and hands it to `save_frame`.
    pub fn shot_pending(&self) -> bool {
        self.shot_now
    }

    /// Where the screenshot button writes: `game/out/look/look-<UTC time>.png`.
    pub fn shot_path() -> PathBuf {
        let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let (y, mo, d) = civil_from_days((secs / 86_400) as i64);
        let t = secs % 86_400;
        PathBuf::from(format!("{}/out/look/look-{y:04}-{mo:02}-{d:02}-{:02}{:02}{:02}.png", env!("CARGO_MANIFEST_DIR"), t / 3600, t / 60 % 60, t % 60))
    }

    /// What a screenshot's JSON sidecar says: every look value, every
    /// block's face colours, the terrain, the time, and where you stood.
    pub fn sidecar(&self, session: &Session, gpu: &Renderer, png: &Path) -> Value {
        let blocks: serde_json::Map<String, Value> = self
            .names
            .iter()
            .zip(&self.colours)
            .filter(|(n, _)| !n.starts_with("unused_") && n.as_str() != "air")
            .map(|(n, c)| {
                let [t, s, b] = c.faces().map(|f| f.map(|x| (x as f64 * 1e4).round() / 1e4));
                (n.clone(), json!({ "top": t, "side": s, "bottom": b }))
            })
            .collect();
        // Through text, so f32 colours read 0.55 and not 0.550000011920929.
        let look: Value = serde_json::to_string(&self.look.tidy()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        json!({
            "png": png.display().to_string(),
            "look": look,
            "blocks": blocks,
            "terrain": {
                "roughness": self.terrain_running.roughness,
                "landform": self.terrain_running.landform,
                "vertical_scale": self.terrain_running.vscale,
                "preview_world": self.previewing(),
            },
            "time": {
                "local_hour": local_hour(&gpu.sky.settings, gpu.sky.clock.seconds, session.player.pos.y.atan2(session.player.pos.x)),
                "speed": gpu.sky.clock.speed,
                "clock_seconds": gpu.sky.clock.seconds,
            },
            "world": session.engine.config.name,
            "state": session.state(),
        })
    }

    /// Write a frame (RGBA8) as `png` and its sidecar beside it (`.json`).
    pub fn save_frame(&mut self, png: &Path, rgba: &[u8], width: u32, height: u32, session: &Session, gpu: &Renderer) -> Result<(), String> {
        if let Some(dir) = png.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        crate::render::write_png(&png.to_string_lossy(), width, height, rgba);
        let side = png.with_extension("json");
        let text = serde_json::to_string_pretty(&self.sidecar(session, gpu, png)).map_err(|e| e.to_string())?;
        std::fs::write(&side, text).map_err(|e| format!("write {}: {e}", side.display()))?;
        self.note(format!("Screenshot saved: {} (the look's values are in {}).", png.display(), side.file_name().map_or(String::new(), |s| s.to_string_lossy().into_owned())));
        Ok(())
    }

    // ------------------------------------------------------- the panel

    /// Draw the panel (nothing when closed or on a screenshot frame).
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open || self.shot_now {
            return;
        }
        if !self.styled {
            self.styled = true;
            ctx.all_styles_mut(|s| {
                use egui::{FontId, TextStyle};
                s.text_styles.insert(TextStyle::Body, FontId::proportional(17.0));
                s.text_styles.insert(TextStyle::Button, FontId::proportional(17.0));
                s.text_styles.insert(TextStyle::Heading, FontId::proportional(21.0));
                s.text_styles.insert(TextStyle::Small, FontId::proportional(14.5));
                s.text_styles.insert(TextStyle::Monospace, FontId::monospace(15.0));
                s.spacing.slider_width = 230.0;
                s.spacing.interact_size = egui::vec2(44.0, 24.0);
                s.spacing.item_spacing = egui::vec2(10.0, 7.0);
            });
        }
        let height = ctx.content_rect().height() - 24.0;
        let focus = self.focus.as_mut().map(|(name, frames)| {
            *frames = frames.saturating_sub(1);
            name.clone()
        });
        if self.focus.as_ref().is_some_and(|f| f.1 == 0) {
            self.focus = None;
        }
        let header = |title: &str, key: &str, open: bool| {
            let h = egui::CollapsingHeader::new(egui::RichText::new(title).heading()).default_open(open);
            match &focus {
                Some(f) => h.open(Some(f == key)),
                None => h,
            }
        };
        egui::Window::new("Look panel")
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
            .default_width(590.0)
            .min_width(560.0)
            .default_height(height)
            .min_height(height.min(900.0))
            .max_height(height)
            .collapsible(false)
            .resizable(false)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("F1 closes the panel. Every change shows at once; Save look keeps it.").weak());
                ui.separator();
                header("Sky and light", "sky", true).show(ui, |ui| self.ui_sky(ui));
                header("Water", "water", true).show(ui, |ui| self.ui_water(ui));
                header("Blocks", "blocks", false).show(ui, |ui| self.ui_blocks(ui));
                header("Distance detail", "detail", false).show(ui, |ui| self.ui_detail(ui));
                header("Terrain", "terrain", false).show(ui, |ui| self.ui_terrain(ui));
                ui.separator();
                self.ui_footer(ui);
            });
    }

    fn ui_sky(&mut self, ui: &mut egui::Ui) {
        let d = self.defaults.clone();
        egui::Grid::new("sky").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            let mut hour = self.hour_now;
            ui.label("Time of day").on_hover_text("Local sun time where you are. Not saved in the look; a screenshot notes it.");
            let r = ui.add(egui::Slider::new(&mut hour, 0.0..=23.99).custom_formatter(|h, _| format!("{:02}:{:02}", h.floor() as u32, ((h.fract() * 60.0).floor() as u32).min(59))));
            if r.changed() {
                self.set_hour = Some(hour);
            }
            ui.end_row();
            let mut speed = self.speed_now;
            ui.label("Time speed").on_hover_text("How fast the day goes: 1 is a 24-minute day, 0 stops the sun. [ and ] still work.");
            ui.horizontal(|ui| {
                if ui.add(egui::Slider::new(&mut speed, 0.0..=3600.0).logarithmic(true).smallest_positive(0.1).suffix(" x")).changed() {
                    self.set_speed = Some(speed);
                }
                if ui.button("stop").clicked() {
                    self.set_speed = Some(0.0);
                }
                if ui.button("1x").clicked() {
                    self.set_speed = Some(1.0);
                }
            });
            ui.end_row();
            let s = &mut self.look.sky;
            slider(ui, "Atmosphere height", "How high the air goes. The sky turns toward black space a few times higher than this. The M1 value was scaled to the small planet (640 m), so the sky went dark above about 1.5 km.", &mut s.atmosphere_height_m, 100.0..=30_000.0, true, " m");
            hint(ui, format!("the sky is space-dark by about {:.1} km up", s.atmosphere_height_m * 3.0 / 1000.0));
            slider(ui, "Fog density", "Haze over distance. Higher means distant land fades sooner.", &mut s.fog_density_per_km, 0.0..=5.0, true, " per km");
            hint(ui, if s.fog_density_per_km > 0.0 { format!("distant things fade over about {:.1} km", 1.0 / s.fog_density_per_km) } else { "no fog".into() });
            slider(ui, "Fog strength", "The most of a far view the fog may cover (1 = all of it).", &mut s.fog_strength, 0.0..=1.0, false, "");
            colour(ui, "Sky at the horizon", "The sky low down, and the colour of the fog.", &mut s.horizon_colour);
            colour(ui, "Sky overhead", "The sky straight up.", &mut s.zenith_colour);
            colour(ui, "Sunset glow", "The glow around a low sun at dawn and dusk.", &mut s.sunset_colour);
            slider(ui, "Sunset strength", "How strong that glow is (golden hour).", &mut s.sunset_strength, 0.0..=1.0, false, "");
            slider(ui, "Night sky", "How bright the empty sky stays at night, as a part of the day sky.", &mut s.night_sky, 0.0..=0.5, false, "");
            slider(ui, "Star brightness", "How bright the stars are.", &mut s.star_brightness, 0.0..=5.0, false, "");
            let l = &mut self.look.light;
            slider(ui, "Sun strength", "Direct sunlight on faces turned to the sun.", &mut l.sun_strength, 0.0..=3.0, false, "");
            slider(ui, "Sky light", "Light from the sky on everything that faces up, by day.", &mut l.sky_ambient, 0.0..=2.0, false, "");
            slider(ui, "Light in shadow", "Light everywhere, day and night, even in shade and in tunnels.", &mut l.shadow_light, 0.0..=1.0, false, "");
            slider(ui, "Night brightness", "Extra light at night only. 0 is the M1 night.", &mut l.night_light, 0.0..=1.0, false, "");
            slider(ui, "Moonlight", "How strong the moon's light is (1 = the M1 moon).", &mut l.moonlight, 0.0..=10.0, false, "");
            colour(ui, "Moonlight colour", "The tint of moonlight.", &mut l.moon_colour);
            slider(ui, "Side faces", "How bright the sides of blocks are, next to their tops (1 = as bright).", &mut l.side_shade, 0.0..=1.5, false, "");
            slider(ui, "Undersides", "How bright the bottoms of blocks are (ceilings, overhangs).", &mut l.underside_shade, 0.0..=1.5, false, "");
            slider(ui, "Corner shadows", "Darkening in corners and creases between blocks (0 = none).", &mut l.corner_shadows, 0.0..=1.0, false, "");
        });
        if ui.button("Reset sky and light to default").clicked() {
            self.look.sky = d.sky;
            self.look.light = d.light;
        }
    }

    fn ui_water(&mut self, ui: &mut egui::Ui) {
        let d = self.defaults.water.clone();
        egui::Grid::new("water").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            if let Some(w) = self.water {
                let c = &mut self.colours[w as usize];
                ui.label("Water colour").on_hover_text("The colour of water (the water block in blocks.json).");
                if pick(ui, &mut c.color).changed() {
                    self.colours_dirty = true;
                }
                ui.end_row();
            }
            let w = &mut self.look.water;
            slider(ui, "Opacity", "How much you can see through water looking down (0 = clear, 1 = solid colour).", &mut w.opacity, 0.0..=1.0, false, "");
            slider(ui, "Opacity at the edge", "How solid water looks when you look along it, far away.", &mut w.edge_opacity, 0.0..=1.0, false, "");
            slider(ui, "Sparkle", "The sun's reflection on the water.", &mut w.glint, 0.0..=3.0, false, "");
            slider(ui, "Sparkle tightness", "Small = a wide shine, big = a tight bright spot.", &mut w.glint_sharpness, 2.0..=500.0, true, "");
            colour(ui, "Underwater tint", "The colour everything takes when your eyes are under water.", &mut w.underwater_colour);
            slider(ui, "Underwater tint strength", "How strong that tint is.", &mut w.underwater_strength, 0.0..=1.0, false, "");
            ui.label("Far ocean");
            ui.checkbox(&mut w.far_ocean_follows_water, "follows the water colour").on_hover_text("From high up the sea is drawn as a solid 'deep_ocean' block. Keep it matching the water colour and opacity.");
            ui.end_row();
        });
        if ui.button("Reset water to default").clicked() {
            self.look.water = d;
            if let Some(w) = self.water {
                self.colours[w as usize] = self.colours_default[w as usize];
                self.colours_dirty = true;
            }
        }
    }

    fn ui_blocks(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Each block's colour. Tick \"faces\" to give the top, the sides and the bottom their own colours.").weak());
        let following = self.look.water.far_ocean_follows_water;
        egui::Grid::new("blocks").num_columns(6).striped(true).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(egui::RichText::new("block").strong());
            ui.label(egui::RichText::new("colour").strong());
            ui.label(egui::RichText::new("faces").strong());
            ui.label(egui::RichText::new("top").strong());
            ui.label(egui::RichText::new("sides").strong());
            ui.label(egui::RichText::new("bottom").strong());
            ui.end_row();
            for id in 1..self.names.len() {
                let name = self.names[id].clone();
                if name.starts_with("unused_") {
                    continue;
                }
                let locked = following && Some(id as BlockId) == self.deep_ocean;
                let default = self.colours_default[id];
                let c = &mut self.colours[id];
                let mut changed = false;
                ui.label(name.replace('_', " ")).on_hover_text(if locked { "The sea seen from far away. It follows the water colour; untick that in Water to set it here." } else { "" });
                ui.add_enabled_ui(!locked, |ui| {
                    changed |= pick(ui, &mut c.color).changed();
                });
                let mut faces = c.per_face();
                if ui.add_enabled(!locked, egui::Checkbox::without_text(&mut faces)).changed() {
                    if faces {
                        let [t, s, b] = c.faces();
                        (c.top, c.side, c.bottom) = (Some(t), Some(s), Some(b));
                    } else {
                        (c.top, c.side, c.bottom) = (None, None, None);
                    }
                    changed = true;
                }
                if c.per_face() {
                    for f in [&mut c.top, &mut c.side, &mut c.bottom] {
                        let mut v = f.unwrap_or(c.color);
                        if pick(ui, &mut v).changed() {
                            *f = Some(v);
                            changed = true;
                        }
                    }
                } else {
                    ui.label("");
                    ui.label("");
                    ui.label("");
                }
                if !locked && *c != default && ui.small_button("reset").on_hover_text("Back to this block's shipped colours").clicked() {
                    *c = default;
                    changed = true;
                }
                ui.end_row();
                self.colours_dirty |= changed;
            }
        });
        if ui.button("Reset all block colours to default").clicked() {
            self.colours = self.colours_default.clone();
            self.colours_dirty = true;
        }
    }

    fn ui_detail(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("detail").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            slider(ui, "Distance detail", "How far out the world keeps its fine detail. Higher shows finer far terrain and costs more frame time; 5 is the default.", &mut self.look.detail.lod_factor, 1.0..=20.0, true, "");
        });
        if ui.button("Reset distance detail to default").clicked() {
            self.look.detail = self.defaults.detail.clone();
        }
    }

    fn ui_terrain(&mut self, ui: &mut egui::Ui) {
        if self.previewing() {
            ui.colored_label(egui::Color32::from_rgb(255, 190, 90), "PREVIEW WORLD: saving is off. Your own world and its save are untouched, and anything dug here is thrown away.");
        } else {
            ui.label(egui::RichText::new("These reshape the land, so the world is rebuilt (a few seconds) and shown as a preview with saving off. Your save is never changed by them.").weak());
        }
        let mut held = false;
        let mut moved = false;
        egui::Grid::new("terrain").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
            let t = &mut self.terrain;
            for (label, help, v, range) in [
                ("Roughness", "The small bumps: metre-to-50-metre texture of the ground.", &mut t.roughness, 0.0..=3.0),
                ("Landform strength", "Ridges, valleys, cliffs and mesas between the big shapes and the small bumps (0 = none).", &mut t.landform, 0.0..=3.0),
            ] {
                ui.label(label).on_hover_text(help);
                let r = ui.add(egui::Slider::new(v, range)).on_hover_text(help);
                moved |= r.changed();
                held |= r.is_pointer_button_down_on() || r.dragged();
                ui.end_row();
            }
            ui.label("Vertical scale").on_hover_text("How tall mountains and how deep seas are. 1/70 keeps Earth's slopes; 1/8 gives real mountains (and is usually played in a taller world: --height 4096 --sea-level 1536, a restart setting).");
            let r = ui.add(egui::Slider::new(&mut t.vscale, 0.01..=0.5).logarithmic(true).custom_formatter(|v, _| format!("1/{:.0}", 1.0 / v)).custom_parser(|s| s.trim().trim_start_matches("1/").parse::<f64>().ok().map(|n| if n > 1.0 { 1.0 / n } else { n })));
            moved |= r.changed();
            held |= r.is_pointer_button_down_on() || r.dragged();
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                for (n, help) in [(70.0, "Earth's slopes (the M1 world)"), (20.0, "in between"), (8.0, "real mountains")] {
                    if ui.button(format!("1/{n:.0}")).on_hover_text(help).clicked() {
                        t.vscale = if n == 70.0 { self.earth_vscale } else { 1.0 / n };
                        moved = true;
                    }
                }
            });
            ui.end_row();
        });
        if moved {
            self.terrain_moved = Some(Instant::now());
        }
        self.terrain_held = held;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.previewing(), egui::Button::new("Back to my world")).clicked() {
                self.terrain = self.terrain_home;
                self.terrain_moved = Some(Instant::now() - TERRAIN_SETTLE);
            }
            let is_default = self.terrain_running.same(&self.terrain_file);
            if ui.add_enabled(!is_default && !self.confirm_default, egui::Button::new("Make this the default...")).on_hover_text("Write this terrain into world.json, so new worlds start with it.").clicked() {
                self.confirm_default = true;
            }
        });
        if self.confirm_default {
            ui.colored_label(
                egui::Color32::from_rgb(255, 120, 100),
                "This writes world.json. Existing saves will NOT load with the new terrain: the game refuses them at startup (your current save included). Put world.json back to play them again.",
            );
            ui.horizontal(|ui| {
                if ui.button("Yes, make it the default").clicked() {
                    self.confirm_default = false;
                    self.make_default();
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_default = false;
                }
            });
        }
    }

    fn ui_footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(egui::RichText::new("Save look").strong()).on_hover_text("Write look.json and the block colours into blocks.json.").clicked() {
                self.save_look();
            }
            if ui.button("Revert to saved").on_hover_text("Throw away changes since the last save.").clicked() {
                self.revert();
            }
            if ui.button("Screenshot").on_hover_text("Save exactly what you see (without this panel) to game/out/look/, with a note of every look value beside it.").clicked() {
                self.shot_requested = true;
            }
        });
        if self.unsaved() {
            ui.label(egui::RichText::new("The look has unsaved changes (Save look keeps them).").weak());
        }
        for (_, text) in self.notes.iter().rev() {
            ui.label(egui::RichText::new(text).small());
        }
    }
}

/// A labelled slider row in a two-column grid.
fn slider(ui: &mut egui::Ui, label: &str, help: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, log: bool, suffix: &str) -> egui::Response {
    ui.label(label).on_hover_text(help);
    let mut s = egui::Slider::new(v, range).logarithmic(log).suffix(suffix);
    if log {
        s = s.smallest_positive(0.01);
    }
    let r = ui.add(s).on_hover_text(help);
    ui.end_row();
    r
}

/// A grey note under the row above it.
fn hint(ui: &mut egui::Ui, text: String) {
    ui.label("");
    ui.label(egui::RichText::new(text).small().weak());
    ui.end_row();
}

/// A colour button (linear RGB). egui's picker round-trips the colour
/// through HSVA every frame and writes back a value a few ULPs off, so it
/// edits a copy and the colour changes only when Andrew changes it.
fn pick(ui: &mut egui::Ui, c: &mut [f32; 3]) -> egui::Response {
    let mut v = *c;
    let r = egui::color_picker::color_edit_button_rgb(ui, &mut v);
    if r.changed() {
        *c = v;
    }
    r
}

/// A labelled colour picker row.
fn colour(ui: &mut egui::Ui, label: &str, help: &str, c: &mut [f32; 3]) -> bool {
    ui.label(label).on_hover_text(help);
    let changed = pick(ui, c).on_hover_text(help).changed();
    ui.end_row();
    changed
}

/// Local solar hour at longitude `lon` (radians) at clock `seconds`.
pub fn local_hour(settings: &Settings, seconds: f64, lon: f64) -> f64 {
    let s = settings.at(seconds).sun;
    (12.0 + (lon - s.y.atan2(s.x)) * 24.0 / TAU).rem_euclid(24.0)
}

/// The clock moved the least (forward or back, at most half a day) to make
/// it `hour` local solar time at `lon`. The calendar keeps its day.
pub fn seek_hour(settings: &Settings, seconds: f64, lon: f64, hour: f64) -> f64 {
    let target = lon + (12.0 - hour) * TAU / 24.0;
    let mut t = seconds;
    for _ in 0..6 {
        let s = settings.at(t).sun;
        let error = (s.y.atan2(s.x) - target + PI).rem_euclid(TAU) - PI;
        t += error / TAU * settings.day_length_s;
    }
    t
}

/// (year, month, day) of a day count since 1970-01-01 (Howard Hinnant's
/// civil_from_days), for screenshot names.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("triangulum-workshop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["look.json", "blocks.json", "world.json"] {
            std::fs::copy(format!("{}/assets/{f}", env!("CARGO_MANIFEST_DIR")), dir.join(f)).unwrap();
        }
        dir
    }

    fn workshop(dir: &Path, registry: &Registry) -> Workshop {
        let d = Look::defaults(&Settings::default(), 90_569.6);
        let st = look::Startup { look: Look::load(&dir.join("look.json"), &d).unwrap(), defaults: d, look_path: dir.join("look.json"), blocks_path: dir.join("blocks.json"), world_path: dir.join("world.json"), panel_open: false };
        Workshop::from_parts(registry, st, Terrain { roughness: 1.0, landform: 1.0, vscale: 0.0142 }, 0.0142)
    }

    /// Hot reload: a colour changed in blocks.json with a text editor
    /// reaches the registry's GPU colour table on the next poll, and a
    /// changed look.json reaches the look; the game's own save does not
    /// read back as a change, and a broken file is reported, not applied.
    #[test]
    fn hot_reload_applies_a_changed_colour() {
        let dir = scratch("reload");
        let registry = Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap();
        let mut w = workshop(&dir, &registry);
        let grass = registry.id("grass").unwrap() as usize;
        assert!(!w.apply_colours(&registry), "nothing to change at first");
        let before = registry.colour_table()[grass * 3];

        // Andrew edits grass's colour in a text editor.
        let path = dir.join("blocks.json");
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = text.replace("\"name\": \"grass\",        \"color\": [0.090, 0.260, 0.050]", "\"name\": \"grass\",        \"color\": [0.500, 0.100, 0.900]");
        assert_ne!(edited, text);
        std::fs::write(&path, edited).unwrap();
        w.poll_files();
        assert!(w.apply_colours(&registry), "the edit reaches the registry");
        let after = registry.colour_table();
        assert_eq!(after[grass * 3], [0.5, 0.1, 0.9, 1.0], "grass top is the new colour");
        assert_eq!(after[grass * 3 + 1], [0.18, 0.11, 0.06, 1.0], "its dirt sides stay");
        assert_ne!(after[grass * 3], before);

        // look.json: raise the atmosphere.
        let lp = dir.join("look.json");
        let lt = std::fs::read_to_string(&lp).unwrap();
        let raised = lt.replace("\"atmosphere_height_m\": 639.7", "\"atmosphere_height_m\": 4000.0");
        assert_ne!(raised, lt);
        std::fs::write(&lp, raised).unwrap();
        w.poll_files();
        assert_eq!(w.look().sky.atmosphere_height_m, 4000.0);

        // The game's own save is not a change; a broken file is not applied.
        w.save_look();
        w.poll_files();
        assert!(w.notes.iter().all(|(_, n)| !n.contains("changed on disk: 0")), "{:?}", w.notes);
        std::fs::write(&lp, "{ \"sky\": { \"atmosphere_height_m\": -5 } }").unwrap();
        w.poll_files();
        assert_eq!(w.look().sky.atmosphere_height_m, 4000.0);
        assert!(w.notes.last().unwrap().1.contains("was not applied"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save look writes both files; they read back to what the panel held.
    #[test]
    fn save_look_round_trips_both_files() {
        let dir = scratch("save");
        let registry = Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap();
        let mut w = workshop(&dir, &registry);
        w.look.water.opacity = 0.4321;
        w.look.sky.zenith_colour = [0.05, 0.2, 0.9];
        let snow = registry.id("snow").unwrap() as usize;
        w.colours[snow].side = Some([0.7, 0.7, 0.75]);
        w.save_look();
        let back = Look::load(&dir.join("look.json"), &w.defaults).unwrap();
        assert_eq!(back.water.opacity, 0.4321);
        assert_eq!(back.sky.zenith_colour, [0.05, 0.2, 0.9]);
        let r = Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap();
        assert_eq!(r.def(snow as BlockId).side, Some([0.7, 0.7, 0.75]));
        assert!(r.same_blocks(&registry));
        // The far ocean followed the water's new opacity.
        let deep = registry.id("deep_ocean").unwrap();
        let water = registry.id("water").unwrap();
        let (wc, fc) = (r.def(water).color, r.def(deep).color);
        assert!((fc[2] - (wc[2] * 0.4321 + w.ocean_floor[2] * (1.0 - 0.4321))).abs() < 1e-3, "{fc:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The far ocean formula reproduces the shipped deep_ocean colour at
    /// the shipped water colour and opacity (no change until Andrew moves one).
    #[test]
    fn far_ocean_matches_the_shipped_colour() {
        let dir = scratch("ocean");
        let registry = Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap();
        let shipped = registry.def(registry.id("deep_ocean").unwrap()).color;
        let mut w = workshop(&dir, &registry);
        w.apply_colours(&registry);
        let now = registry.colour(registry.id("deep_ocean").unwrap());
        for i in 0..3 {
            assert!((now[i] - shipped[i]).abs() < 1e-4, "{now:?} vs {shipped:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn time_of_day_seeks_the_shortest_way() {
        let s = Settings::default();
        let lon = 0.7;
        let t0 = 5000.0;
        for hour in [0.0, 5.5, 12.0, 17.5, 23.9] {
            let t = seek_hour(&s, t0, lon, hour);
            let h = local_hour(&s, t, lon);
            let err = (h - hour + 12.0).rem_euclid(24.0) - 12.0;
            assert!(err.abs() < 1e-6, "{hour}: {h}");
            assert!((t - t0).abs() <= s.day_length_s / 2.0 + 1.0, "moved at most half a day");
        }
    }

    #[test]
    fn screenshot_names() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_386), (2025, 10, 25));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }
}

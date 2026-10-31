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
//! applies within half a second. The screenshot button takes a photo of the
//! frame without the panel (`photos.rs`: every look value is in its sidecar).
//!
//! M3b, blocks and textures (the Blocks section, `workshop/blocks_panel.rs`):
//!
//! - **Add a block**: a name, its colours, solid or falls like sand. It
//!   takes the next id and is written to blocks.json at once (a save that
//!   holds it must always find it); ids are never deleted or renumbered,
//!   a block Andrew stops wanting is hidden from the hotbar instead.
//! - **Change a block**: rename it (the old name is kept in `was`, so saves
//!   and replays that know it still work), switch whether it falls (the
//!   panel says what that does first), hide it.
//! - **The hotbar**: what keys 1-9 hold, saved in blocks.json.
//! - **Textures**: a picture per face (top, sides, bottom), painted in a
//!   pixel editor (`workshop/editor.rs`), imported from any PNG, or one of
//!   the examples; every change shows on the blocks in the world at once.
//!   Save look writes the PNGs into `textures/` beside blocks.json, and the
//!   game reloads a texture file edited in a paint program.
//!
//! The app calls `update` before it renders, `ui` inside its egui pass and
//! `shot_pending` / `save_frame` after painting. With the panel closed the
//! per-frame cost is a few comparisons and, twice a second, three file
//! metadata reads (and, when textures are in use, the metadata of a few
//! texture files each frame, round robin).

mod blocks_panel;
mod editor;

use crate::engine::Engine;
use crate::grid::WorldConfig;
use crate::look::{self, FileWatch, Look, Terrain};
use crate::player::Mode;
use crate::render::textures::{self, Picture, TextureSet};
use crate::render::regions::{self, RegionData, RegionsState};
use crate::render::{Lines, MeshView, Renderer, Tint};
use crate::session::{Action, Session};
use crate::sim::Sim;
use crate::sky::Clock;
use crate::sim::draw::FallingDraws;
use crate::sky::Settings;
use crate::stream::Streamer;
use crate::world::blocks::{self, BlockColours, BlockDef, BlockTextures};
use crate::world::save::SaveDir;
use crate::world::{BlockId, EditStore, Registry};
use std::collections::HashMap;
use std::f64::consts::{PI, TAU};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// The blocks file as shipped with this build: "Reset" in the Blocks section.
const SHIPPED_BLOCKS: &str = include_str!("../assets/blocks.json");
const POLL: Duration = Duration::from_millis(500);
/// A terrain dial applies this long after it last moved (and once released).
const TERRAIN_SETTLE: Duration = Duration::from_millis(350);
const NOTE_LIFE: Duration = Duration::from_secs(25);
/// Texture files checked for changes per frame (round robin).
const TEXTURE_POLL_PER_FRAME: usize = 4;

/// A change to the blocks themselves, asked for in the panel and made at
/// the next update (it needs the session: a replay being recorded stops
/// first when the change would make it play differently).
enum Request {
    Add { name: String, colour: [f32; 3], falls: bool },
    Rename(BlockId, String),
    Falls(BlockId, bool),
    Hidden(BlockId, bool),
    /// The hotbar's blocks (None: back to the default).
    Hotbar(Option<Vec<BlockId>>),
}

/// The Blocks section's own state (`blocks_panel.rs`).
struct BlocksUi {
    /// The block whose details and textures are shown.
    selected: Option<BlockId>,
    /// The face being painted: 0 top, 1 sides, 2 bottom, 3 every face.
    face: usize,
    add_open: bool,
    add_label: String,
    add_colour: [f32; 3],
    add_falls: bool,
    add_error: Option<String>,
    rename: String,
    /// A falling change waiting for "Yes".
    confirm_falls: Option<(BlockId, bool)>,
    /// Scroll the panel to the block's details (it was just picked).
    scroll_to: bool,
    hotbar_slot: usize,
    editor: editor::PixelEditor,
    /// The file the editor's picture came from, if any.
    editing: Option<String>,
    /// Where an import goes (block, face) and the file dialog's answer.
    import: Option<((BlockId, usize), mpsc::Receiver<Option<PathBuf>>)>,
}

impl Default for BlocksUi {
    fn default() -> Self {
        Self {
            selected: None,
            face: 3,
            add_open: false,
            add_label: String::new(),
            add_colour: [0.4, 0.12, 0.08],
            add_falls: false,
            add_error: None,
            rename: String::new(),
            confirm_falls: None,
            scroll_to: false,
            hotbar_slot: 0,
            editor: editor::PixelEditor::default(),
            editing: None,
            import: None,
        }
    }
}

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
    /// The shared registry (M3b: the panel adds and changes blocks in it).
    registry: Arc<Registry>,
    /// Every block's definition as last read from the registry.
    blocks: Vec<BlockDef>,
    /// Colours per id: as the panel shows them, and as shipped.
    colours: Vec<BlockColours>,
    colours_default: Vec<BlockColours>,
    colours_dirty: bool,
    /// Textures per id (M3b): as the panel shows them, and as blocks.json
    /// holds them.
    textures: Vec<BlockTextures>,
    saved_textures: Vec<BlockTextures>,
    textures_dirty: bool,
    /// The pictures (files, layers, pixels) and whether the GPU has them all.
    texset: TextureSet,
    gpu_textures: bool,
    /// The registry's colour table needs uploading.
    table_dirty: bool,
    /// The example pictures in textures/examples/ (file, picture).
    examples: Vec<(String, Picture)>,
    /// Blocks the world generator finds by name (and the scripts and tools
    /// with it): their names are fixed.
    fixed: Vec<BlockId>,
    requests: Vec<Request>,
    /// What keys 1-9 hold, as the panel last set them.
    hotbar: Vec<BlockId>,
    bui: BlocksUi,
    /// Small pictures for the panel and the hotbar: key -> (texture, picture).
    thumbs: HashMap<String, (egui::TextureHandle, Picture)>,
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
    /// The mesh view (bug round #6): M and Shift+M step it, the panel sets
    /// it, the renderer draws it. A way of looking, so no file saves it.
    pub mesh_view: MeshView,
    /// The bake's region layers for its tints, read when one is first asked
    /// for: on their way, and what came of it.
    regions_coming: Option<mpsc::Receiver<Result<RegionData, String>>>,
    pub regions_state: RegionsState,
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
        let mut w = Self::from_parts(engine.registry.clone(), st, terrain, engine.planet.radius / 6_371_000.0);
        // A --lod-factor on the command line wins over the file for this run.
        w.look.detail.lod_factor = engine.lod.factor;
        w.hotbar = engine.hotbar.clone();
        w.fixed = engine.generator.block_ids();
        // Evidence screenshots: `TRIANGULUM_PANEL_BLOCK=stone` opens that
        // block's details and texture editor.
        if let Some(id) = std::env::var("TRIANGULUM_PANEL_BLOCK").ok().and_then(|n| w.registry.id(&n).ok()) {
            w.select(Some(id));
        }
        w
    }

    /// The workshop over a registry and terrain (no engine: the tests).
    pub fn from_parts(registry: Arc<Registry>, st: look::Startup, terrain: Terrain, earth_vscale: f64) -> Self {
        let blocks_list = registry.defs();
        let names: Vec<String> = blocks_list.iter().map(|d| d.name.clone()).collect();
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
        let texset = TextureSet::for_registry(&registry, textures::dir_for(&st.blocks_path));
        let examples = load_examples(&texset);
        let textures_now = registry.textures();
        let hotbar = registry.default_hotbar();
        let _ = &names;
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
            blocks: blocks_list,
            colours,
            colours_default,
            colours_dirty: true,
            saved_textures: textures_now.clone(),
            textures: textures_now,
            textures_dirty: false,
            texset,
            gpu_textures: false,
            table_dirty: true,
            examples,
            fixed: Vec::new(),
            requests: Vec::new(),
            hotbar,
            bui: BlocksUi::default(),
            thumbs: HashMap::new(),
            registry,
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
            mesh_view: MeshView::NORMAL,
            regions_coming: None,
            regions_state: RegionsState::default(),
            focus: std::env::var("TRIANGULUM_PANEL_SECTION").ok().map(|s| (s, 5)),
        }
    }

    pub fn look(&self) -> &Look {
        &self.look
    }

    /// The pictures as they are live, painted ones included (B6: a recording
    /// holds their identity).
    pub fn texture_set(&self) -> &TextureSet {
        &self.texset
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
            self.poll_files_with(Some(session));
        }
        // Blocks added or changed in the panel (M3b).
        self.apply_requests(session);
        // Time of day and its speed: the session's sky clock, set as an
        // input (a replay records it, as it does [ and ]).
        let lon = session.player.pos.y.atan2(session.player.pos.x);
        if let Some(h) = self.set_hour.take() {
            let seconds = seek_hour(&gpu.sky.settings, session.sky.seconds, lon, h);
            let _ = session.apply(Action::Sky(Clock { seconds, speed: session.sky.speed }));
        }
        if let Some(speed) = self.set_speed.take() {
            let _ = session.apply(Action::Sky(Clock { seconds: session.sky.seconds, speed }));
        }
        if self.open {
            self.hour_now = local_hour(&gpu.sky.settings, session.sky.seconds, lon);
            self.speed_now = session.sky.speed;
        }
        // The look and the distance detail.
        if self.look_applied.as_ref() != Some(&self.look) {
            gpu.sky.look = self.look.clone();
            streamer.params.factor = self.look.detail.lod_factor;
            streamer.cover.factor = self.look.detail.lod_factor;
            self.look_applied = Some(self.look.clone());
        }
        // Block colours and textures: the GPU table and texture layers, no
        // remeshing.
        if self.apply_colours(&session.engine.registry) {
            self.table_dirty = true;
        }
        self.apply_textures(gpu);
        if std::mem::take(&mut self.table_dirty) {
            gpu.set_colours(&self.registry.colour_table());
        }
        // Terrain, once a dial has come to rest.
        if self.terrain_moved.is_some_and(|t| !self.terrain_held && now.duration_since(t) >= TERRAIN_SETTLE) {
            self.terrain_moved = None;
            if !self.terrain.same(&self.terrain_running) {
                self.rebuild(session, streamer, falling, gpu);
            }
        }
        self.notes.retain(|(t, _)| now.duration_since(*t) < NOTE_LIFE);
        gpu.mesh_view = self.mesh_view;
        self.follow_regions(session, gpu);
        // A requested screenshot is this frame, drawn without the panel.
        self.shot_now = std::mem::take(&mut self.shot_requested);
    }

    /// The first tint that needs the bake's layers reads them, off this
    /// thread; the view shows flat grey until they land.
    fn follow_regions(&mut self, session: &Session, gpu: &mut Renderer) {
        if self.mesh_view.tint.uses_bake() && self.regions_state == RegionsState::NotAsked {
            self.regions_state = RegionsState::Loading;
            let (tx, rx) = mpsc::channel();
            let dir = session.engine.config.bake.clone();
            std::thread::spawn(move || {
                let _ = tx.send(regions::load(&dir));
            });
            self.regions_coming = Some(rx);
        }
        if let Some(Ok(result)) = self.regions_coming.as_ref().map(|rx| rx.try_recv()) {
            self.regions_coming = None;
            self.regions_state = match result {
                Ok(data) => {
                    let plates = data.plates;
                    gpu.set_regions(Arc::new(data));
                    RegionsState::Ready { plates }
                }
                Err(e) => RegionsState::Missing(e),
            };
        }
    }

    /// Textures (M3b): the panel's choices into the registry, layers for
    /// any new files, pictures changed by painting or on disk to the GPU.
    pub fn apply_textures(&mut self, gpu: &mut Renderer) {
        match self.sync_textures() {
            None => gpu.set_textures(self.texset.size, &self.texset.pictures()),
            Some(dirty) => {
                for i in dirty {
                    if !gpu.set_texture_layer(i as u32, self.texset.picture_at(i)) {
                        gpu.set_textures(self.texset.size, &self.texset.pictures());
                        break;
                    }
                }
            }
        }
    }

    /// The CPU side of `apply_textures`: the layers the GPU needs again
    /// (None: all of them).
    fn sync_textures(&mut self) -> Option<Vec<usize>> {
        if self.registry.texture_size() != self.texset.size {
            self.texset.set_size(self.registry.texture_size());
            self.gpu_textures = false;
            self.textures_dirty = true;
            self.bui.editing = None;
        }
        if !self.texset.is_empty() {
            let (notes, appeared) = self.texset.poll(TEXTURE_POLL_PER_FRAME);
            for n in notes {
                self.note(format!("Texture {n}"));
            }
            self.textures_dirty |= appeared;
        }
        let dirty = self.texset.take_dirty();
        if std::mem::take(&mut self.textures_dirty) || !dirty.is_empty() {
            self.registry.set_textures(&self.textures);
            self.texset.assign(&self.registry);
            self.table_dirty = true;
        }
        let dirty: Vec<usize> = dirty.into_iter().chain(self.texset.take_dirty()).collect();
        if !self.gpu_textures {
            self.gpu_textures = true;
            return None;
        }
        Some(dirty)
    }

    /// Re-read the block definitions after the registry changed: blocks
    /// added since get their colours and textures in the panel's lists.
    fn refresh_blocks(&mut self) {
        let defs = self.registry.defs();
        let (colours, textures) = (self.registry.colours(), self.registry.textures());
        for d in defs.iter().skip(self.colours.len()) {
            let id = d.id as usize;
            self.colours.push(colours[id]);
            self.saved_colours.push(d.colours());
            self.colours_default.push(d.colours());
            self.textures.push(textures[id].clone());
            self.saved_textures.push(d.textures());
        }
        self.blocks = defs;
        self.table_dirty = true;
        // A block that came with textures needs its layers.
        self.textures_dirty = true;
    }

    /// Write the definitions (and hotbar) to blocks.json now: a block that
    /// exists must be in the file before any save can hold it.
    fn write_defs(&mut self) -> Result<(), String> {
        let r = blocks::save_defs(&self.blocks_path.to_string_lossy(), &self.registry);
        self.watches[1].mark();
        r
    }

    /// Make the changes the panel asked for.
    fn apply_requests(&mut self, session: &mut Session) {
        for r in std::mem::take(&mut self.requests) {
            match r {
                Request::Add { name, colour, falls } => {
                    let label = name.replace('_', " ");
                    match self.registry.add(BlockDef::new_solid(&name, BlockColours::flat(colour), falls)) {
                        Ok(id) => {
                            self.refresh_blocks();
                            self.select(Some(id));
                            self.bui.add_label.clear();
                            self.bui.add_open = false;
                            self.bui.add_error = None;
                            match self.write_defs() {
                                Ok(()) => self.note(format!("Added {label} (block {id}) and saved it to blocks.json. Put it on the hotbar below to place it.")),
                                Err(e) => self.note(format!("Added {label}, but blocks.json was not written ({e}): fix that before quitting, or a save holding {label} will not load.")),
                            }
                        }
                        Err(e) => self.bui.add_error = Some(e),
                    }
                }
                Request::Rename(id, new) => {
                    let old = self.registry.name(id);
                    match self.registry.rename(id, &new) {
                        Ok(()) => {
                            // The hotbar in the file names it by its new name too.
                            if let Some(h) = self.registry.hotbar_names() {
                                self.registry.set_hotbar_names(Some(h.into_iter().map(|n| if n == old { new.clone() } else { n }).collect()));
                            }
                            self.refresh_blocks();
                            let saved = self.write_defs();
                            self.note(match saved {
                                Ok(()) => format!("Renamed {old} to {new}. blocks.json keeps the old name in \"was\", so saves and replays that know {old} still work."),
                                Err(e) => format!("Renamed {old} to {new}, but blocks.json was not written: {e}"),
                            });
                        }
                        Err(e) => self.note(format!("Not renamed: {e}")),
                    }
                }
                Request::Falls(id, falls) => {
                    self.stop_recording_for(session, "the block rules changed");
                    match self.registry.set_falls(id, falls) {
                        Ok(()) => {
                            self.refresh_blocks();
                            let name = self.registry.name(id).replace('_', " ");
                            let saved = self.write_defs().err().map_or(String::new(), |e| format!(" (blocks.json was not written: {e})"));
                            self.note(if falls {
                                format!("{name} now falls like sand: dig under it to see{saved}.")
                            } else {
                                format!("{name} no longer falls{saved}.")
                            });
                        }
                        Err(e) => self.note(format!("Not changed: {e}")),
                    }
                }
                Request::Hidden(id, hidden) => {
                    if self.registry.set_hidden(id, hidden).is_ok() {
                        self.refresh_blocks();
                        if let Err(e) = self.write_defs() {
                            self.note(format!("blocks.json was not written: {e}"));
                        }
                    }
                }
                Request::Hotbar(h) => {
                    self.stop_recording_for(session, "the hotbar changed");
                    let ids = h.clone().unwrap_or_else(|| self.registry.default_hotbar());
                    self.registry.set_hotbar_names(h.map(|h| h.iter().map(|&id| self.registry.name(id)).collect()));
                    self.set_hotbar(session, ids);
                    if let Err(e) = self.write_defs() {
                        self.note(format!("The hotbar changed, but blocks.json was not written: {e}"));
                    }
                }
            }
        }
    }

    /// Put `ids` on keys 1-9 in the session (and in the real world when a
    /// preview is showing).
    fn set_hotbar(&mut self, session: &mut Session, ids: Vec<BlockId>) {
        self.hotbar = ids.clone();
        if session.engine.hotbar != ids {
            session.engine = Arc::new(session.engine.with_hotbar(ids.clone()));
        }
        if let Some(h) = &mut self.home {
            h.engine = Arc::new(h.engine.with_hotbar(ids));
        }
    }

    /// A replay can't cross a change to the block rules or the hotbar: one
    /// being recorded ends, saved, first.
    fn stop_recording_for(&mut self, session: &mut Session, why: &str) {
        if session.recording() {
            match session.stop_recording() {
                Ok(s) => self.note(format!("Replay recording stopped and saved to {} ({why}).", s.path.display())),
                Err(e) => self.note(format!("Replay recording stopped: {e}")),
            }
        }
    }

    /// The pictures for keys 1-9 in the HUD: each block's top texture (or
    /// its sides'), None for a block with no texture (the HUD shows its
    /// colour).
    pub fn hotbar_thumbs(&mut self, ctx: &egui::Context, hotbar: &[BlockId]) -> Vec<Option<egui::TextureId>> {
        hotbar
            .iter()
            .map(|&id| {
                let t = self.textures.get(id as usize)?;
                let [top, side, _] = t.faces();
                let file = top.or(side)?.to_string();
                let pic = self.texset.picture(&file)?.clone();
                Some(self.thumb(ctx, &file, &pic))
            })
            .collect()
    }

    /// An egui texture of a picture, kept and refreshed when it changes.
    fn thumb(&mut self, ctx: &egui::Context, key: &str, pic: &Picture) -> egui::TextureId {
        let image = || egui::ColorImage::from_rgba_unmultiplied([pic.size as usize, pic.size as usize], &pic.rgba);
        match self.thumbs.get_mut(key) {
            Some((h, p)) => {
                if p != pic {
                    h.set(image(), egui::TextureOptions::NEAREST);
                    *p = pic.clone();
                }
                h.id()
            }
            None => {
                let h = ctx.load_texture(format!("tex:{key}"), image(), egui::TextureOptions::NEAREST);
                let id = h.id();
                self.thumbs.insert(key.to_string(), (h, pic.clone()));
                id
            }
        }
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
        self.poll_files_with(None);
    }

    /// As `poll_files`; with the session, a change to the hotbar or the
    /// block rules ends a replay recording first and the hotbar follows.
    fn poll_files_with(&mut self, mut session: Option<&mut Session>) {
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
                    // New blocks, renames, falling, hiding and the hotbar
                    // (M3b); a change that would replay differently ends a
                    // recording first.
                    let rules = r.defs().iter().zip(self.registry.defs()).any(|(a, b)| a.gravity != b.gravity) || r.hotbar_names() != self.registry.hotbar_names();
                    if let (true, Some(s)) = (rules, session.as_deref_mut()) {
                        self.stop_recording_for(s, "blocks.json changed the block rules or the hotbar");
                    }
                    let merged = self.registry.merge_file(&r);
                    let mut said = Vec::new();
                    match &merged {
                        Ok(done) if !done.is_empty() => {
                            self.refresh_blocks();
                            said.push(done.join(", "));
                            if let (true, Some(s)) = (done.iter().any(|d| d == "hotbar"), session.as_deref_mut()) {
                                let ids = match self.registry.saved_hotbar() {
                                    Some(Ok(h)) if !h.is_empty() => h,
                                    _ => self.registry.default_hotbar(),
                                };
                                self.set_hotbar(s, ids);
                            }
                        }
                        Ok(_) => {}
                        Err(e) => said.push(format!("its blocks were not applied ({e})")),
                    }
                    // Blocks whose colours or textures changed in the file
                    // take them; unsaved panel edits to other blocks stay.
                    let (mut n, mut t) = (0, 0);
                    for d in r.defs() {
                        let id = d.id as usize;
                        if self.blocks.get(id).is_some_and(|b| b.answers_to(&d.name)) {
                            if self.saved_colours[id] != d.colours() {
                                self.colours[id] = d.colours();
                                self.saved_colours[id] = d.colours();
                                n += 1;
                            }
                            if self.saved_textures[id] != d.textures() {
                                self.textures[id] = d.textures();
                                self.saved_textures[id] = d.textures();
                                t += 1;
                            }
                        }
                    }
                    self.colours_dirty = true;
                    self.textures_dirty |= t > 0;
                    said.push(format!("{n} block colour(s) and {t} block texture(s) applied"));
                    self.note(format!("blocks.json changed on disk: {}.", said.join("; ")));
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
        engine.hotbar = self.hotbar.clone();
        self.stop_recording(session);
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
        self.stop_recording(session);
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

    /// A replay (M2c) cannot cross a change of world: one being recorded
    /// ends, saved, before the world is swapped.
    fn stop_recording(&mut self, session: &mut Session) {
        if session.recording() {
            match session.stop_recording() {
                Ok(s) => self.note(format!("Replay recording stopped and saved to {} (a replay cannot cross a terrain change).", s.path.display())),
                Err(e) => self.note(format!("Replay recording stopped: {e}")),
            }
        }
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
        // The pictures first, so blocks.json never names a file that is
        // not there yet.
        let pictures = self.texset.save();
        let b = match &pictures {
            Ok(_) => blocks::save_look(&self.blocks_path.to_string_lossy(), &self.colours, &self.textures),
            Err(_) => blocks::save_colours(&self.blocks_path.to_string_lossy(), &self.colours),
        };
        self.watches[1].mark();
        let c = pictures.as_ref().map(|_| ()).map_err(Clone::clone);
        match (a, b, c) {
            (Ok(()), Ok(()), Ok(())) => {
                self.look = look.clone();
                self.saved_look = look;
                self.saved_colours = self.colours.clone();
                self.saved_textures = self.textures.clone();
                let n = pictures.map_or(0, |p| p.len());
                let pics = if n > 0 { format!(" and {n} picture(s) in {}", self.texset.dir.display()) } else { String::new() };
                self.note(format!(
                    "Saved {} and {}{pics}. To share the look, commit and push them (the textures folder too), or send them.",
                    self.look_path.display(),
                    self.blocks_path.display()
                ))
            }
            (a, b, c) => self.note(format!("Saving failed: {}", [a.err(), b.err(), c.err()].into_iter().flatten().collect::<Vec<_>>().join("; "))),
        }
    }

    /// Unsaved changes to the look, the block colours or the textures?
    pub fn unsaved(&self) -> bool {
        self.look.tidy() != self.saved_look || self.colours != self.saved_colours || self.textures != self.saved_textures || !self.texset.unsaved().is_empty()
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
                    let id = d.id as usize;
                    if self.blocks.get(id).is_some_and(|b| b.answers_to(&d.name)) {
                        self.colours[id] = d.colours();
                        self.saved_colours[id] = d.colours();
                        self.textures[id] = d.textures();
                        self.saved_textures[id] = d.textures();
                    }
                }
                self.colours_dirty = true;
                self.textures_dirty = true;
            }
            Err(e) => self.note(format!("blocks.json: {e}")),
        }
        self.texset.revert();
        self.bui.editing = None;
        self.note("Back to the look as last saved.");
    }

    // ------------------------------------------------------- screenshots

    /// True on the frame a screenshot was asked for: the app reads the
    /// frame back after painting and writes it as a photo.
    pub fn shot_pending(&self) -> bool {
        self.shot_now
    }

    /// Say something in the panel (a photo was saved, or wasn't).
    pub fn say(&mut self, text: impl Into<String>) {
        self.note(text);
    }

    // ------------------------------------------------------- the panel

    /// Draw the panel (nothing when closed or on a screenshot frame).
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open || self.shot_now {
            return;
        }
        if !self.styled {
            self.styled = true;
            crate::ui::apply_panel_style(ctx);
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
        self.take_drops(ctx);
        egui::Window::new("Look panel")
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0))
            .default_width(590.0)
            .min_width(560.0)
            .max_width(640.0)
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
                header("Mesh view", "mesh", true).show(ui, |ui| self.ui_mesh(ui));
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

    fn ui_mesh(&mut self, ui: &mut egui::Ui) {
        ui.label("Colour the mesh by").on_hover_text("M steps through these.");
        ui.horizontal_wrapped(|ui| {
            for t in Tint::ALL {
                ui.selectable_value(&mut self.mesh_view.tint, t, t.label()).on_hover_text(t.hint());
            }
        });
        ui.label("Lines").on_hover_text("Shift+M steps through these.");
        ui.horizontal(|ui| {
            for l in Lines::ALL {
                ui.selectable_value(&mut self.mesh_view.lines, l, l.label());
            }
        });
        ui.label(egui::RichText::new("The game's own mesh, coloured by the region each part belongs to. A heavy line is a chunk's border, and the thin diagonal shows a quad's two triangles. The names are on screen while one of these is on.").small().weak());
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
            if ui.button("Screenshot").on_hover_text("Save exactly what you see (without this panel) as a photo in game/interchange/, with every look value in its sidecar.").clicked() {
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

/// A linear colour as egui shows it.
fn srgb32(c: [f32; 3]) -> egui::Color32 {
    let t = |x: f32| textures::to_srgb(x);
    egui::Color32::from_rgb(t(c[0]), t(c[1]), t(c[2]))
}

/// A labelled colour picker row.
fn colour(ui: &mut egui::Ui, label: &str, help: &str, c: &mut [f32; 3]) -> bool {
    ui.label(label).on_hover_text(help);
    let changed = pick(ui, c).on_hover_text(help).changed();
    ui.end_row();
    changed
}

/// The example pictures (textures/examples/*.png), at the texture size.
fn load_examples(set: &TextureSet) -> Vec<(String, Picture)> {
    let dir = set.dir.join("examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("png"))).collect()).unwrap_or_default();
    files.sort();
    files.into_iter().filter_map(|p| Some((format!("examples/{}", p.file_name()?.to_string_lossy()), Picture::load(&p, set.size).ok()?))).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("triangulum-workshop-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["look.json", "blocks.json", "world.json"] {
            std::fs::copy(format!("{}/assets/{f}", env!("CARGO_MANIFEST_DIR")), dir.join(f)).unwrap();
        }
        // The example textures (M3b), as shipped.
        let ex = dir.join("textures/examples");
        std::fs::create_dir_all(&ex).unwrap();
        for e in std::fs::read_dir(format!("{}/assets/textures/examples", env!("CARGO_MANIFEST_DIR"))).unwrap().flatten() {
            std::fs::copy(e.path(), ex.join(e.file_name())).unwrap();
        }
        dir
    }

    fn workshop(dir: &Path, registry: &Arc<Registry>) -> Workshop {
        let d = Look::defaults(&Settings::default(), 90_569.6);
        let st = look::Startup { look: Look::load(&dir.join("look.json"), &d).unwrap(), defaults: d, look_path: dir.join("look.json"), blocks_path: dir.join("blocks.json"), world_path: dir.join("world.json"), panel_open: false };
        Workshop::from_parts(registry.clone(), st, Terrain { roughness: 1.0, landform: 1.0, vscale: 0.0142 }, 0.0142)
    }

    /// Hot reload: a colour changed in blocks.json with a text editor
    /// reaches the registry's GPU colour table on the next poll, and a
    /// changed look.json reaches the look; the game's own save does not
    /// read back as a change, and a broken file is reported, not applied.
    #[test]
    fn hot_reload_applies_a_changed_colour() {
        let dir = scratch("reload");
        let registry = Arc::new(Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap());
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
        assert_eq!(after[grass * 3], [0.5, 0.1, 0.9, -1.0], "grass top is the new colour");
        assert_eq!(after[grass * 3 + 1], [0.18, 0.11, 0.06, -1.0], "its dirt sides stay");
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
        let registry = Arc::new(Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap());
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
        let registry = Arc::new(Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap());
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

    /// M3b, painting without a window: an example goes on stone with one
    /// click; painting on it makes stone's own file (the example is never
    /// changed); the world's colour table samples it at once; Save look
    /// writes the PNG and blocks.json, and both read back; Revert throws
    /// unsaved painting away.
    #[test]
    fn paint_a_texture_save_and_read_back() {
        let dir = scratch("paint");
        let registry = Arc::new(Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap());
        let mut w = workshop(&dir, &registry);
        assert!(w.examples.iter().any(|(f, _)| f == "examples/stone.png"), "the examples are offered");
        let stone = registry.id("stone").unwrap();
        let example_bytes = std::fs::read(dir.join("textures/examples/stone.png")).unwrap();
        // One click: the example on every face.
        w.set_faces(stone, 3, Some("examples/stone.png".into()));
        assert_eq!(w.sync_textures(), None, "the first sync uploads everything");
        let layer = registry.colour_table()[stone as usize * 3][3];
        assert!(layer >= 0.0, "stone's top samples a texture");
        assert!(w.unsaved());
        // Paint a pixel on it.
        w.select(Some(stone));
        w.sync_editor(stone, 3);
        let mut pic = w.bui.editor.picture.clone().expect("the example is in the editor");
        pic.set(3, 4, [250, 20, 20, 255]);
        w.bui.editor.replace(pic.clone());
        w.commit_paint(stone, 3);
        assert_eq!(w.textures[stone as usize].all.as_deref(), Some("stone.png"), "painting an example makes the block's own copy");
        let dirty = w.sync_textures().unwrap();
        assert!(!dirty.is_empty(), "the painted layer goes to the GPU");
        assert_eq!(w.texset.picture("stone.png"), Some(&pic));
        assert_ne!(w.texset.picture("examples/stone.png"), Some(&pic));
        // A second stroke paints the same file (it is stone's own now).
        pic.set(5, 5, [20, 250, 20, 255]);
        w.bui.editor.replace(pic.clone());
        w.commit_paint(stone, 3);
        assert_eq!(w.textures[stone as usize].all.as_deref(), Some("stone.png"));
        // Only the top: a copy for the top face, the sides keep stone.png.
        w.bui.face = 0;
        w.bui.editing = None;
        w.sync_editor(stone, 0);
        w.commit_paint(stone, 0);
        assert_eq!(w.textures[stone as usize].faces(), [Some("stone_top.png"), Some("stone.png"), Some("stone.png")]);
        // Save look writes both pictures and blocks.json.
        w.save_look();
        assert!(!w.unsaved(), "{:?}", w.notes);
        assert_eq!(Picture::load(&dir.join("textures/stone.png"), 16).unwrap(), pic);
        assert!(dir.join("textures/stone_top.png").exists());
        assert_eq!(std::fs::read(dir.join("textures/examples/stone.png")).unwrap(), example_bytes, "the example file is untouched");
        let back = Registry::load(&dir.join("blocks.json").to_string_lossy()).unwrap();
        assert_eq!(back.def(stone).textures().faces(), [Some("stone_top.png"), Some("stone.png"), Some("stone.png")]);
        // Revert after more painting: back to the saved picture.
        let mut more = pic.clone();
        more.set(0, 0, [0, 0, 255, 255]);
        w.bui.face = 1;
        w.bui.editing = None;
        w.sync_editor(stone, 1);
        w.bui.editor.replace(more);
        w.commit_paint(stone, 1);
        assert!(w.unsaved());
        w.revert();
        w.sync_textures();
        assert_eq!(w.texset.picture("stone.png"), Some(&pic));
        assert!(!w.unsaved(), "{:?}", w.notes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M3b hot reload: blocks.json changed on disk (a text editor, a git
    /// pull) with a new block and a texture: the new block is in the
    /// running registry at once, the texture reaches the colour table, and
    /// the panel's lists grow. A file that renumbers is refused.
    #[test]
    fn hot_reload_brings_in_a_new_block_and_a_texture() {
        let dir = scratch("newblock");
        let path = dir.join("blocks.json");
        let registry = Arc::new(Registry::load(&path.to_string_lossy()).unwrap());
        let mut w = workshop(&dir, &registry);
        w.sync_textures();
        let n = registry.len();
        let text = std::fs::read_to_string(&path).unwrap();
        let other = Registry::parse(&text).unwrap();
        let brick = other.add(BlockDef::new_solid("brick", BlockColours::flat([0.4, 0.1, 0.08]), false)).unwrap();
        let mut t = other.textures();
        t[brick as usize].all = Some("examples/dirt.png".into());
        other.set_textures(&t);
        let new_text = blocks::look_into_text(&blocks::defs_into_text(&text, &other).unwrap(), &other.colours(), Some(&other.textures())).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, &new_text).unwrap();
        w.poll_files();
        assert_eq!(registry.len(), n + 1);
        assert_eq!(registry.id("brick").unwrap(), brick);
        assert_eq!(w.blocks.len(), n + 1);
        assert_eq!(w.colours.len(), n + 1);
        assert!(w.notes.iter().any(|(_, m)| m.contains("added brick")), "{:?}", w.notes);
        w.sync_textures();
        assert!(registry.colour_table()[brick as usize * 3][3] >= 0.0, "brick samples the dirt picture");
        assert!(!w.unsaved(), "what came from the file is saved");
        // A renumbered file: refused, nothing changes.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&path, new_text.replace("\"name\": \"brick\"", "\"name\": \"glass\"")).unwrap();
        w.poll_files();
        assert_eq!(registry.name(brick), "brick");
        assert!(w.notes.last().unwrap().1.contains("never renumber"), "{:?}", w.notes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M3b: the panel's block changes through a real session: Add a block
    /// (written to blocks.json at once), Rename it (the old name kept),
    /// make it fall (the registry's rule changes), and put it on the hotbar
    /// (the session's keys change and the file remembers them).
    #[test]
    fn add_rename_fall_and_hotbar_through_a_session() {
        use crate::player::{Mode, Player};
        use crate::sim::SimConfig;
        let dir = scratch("requests");
        let path = dir.join("blocks.json");
        let registry = Arc::new(Registry::load(&path.to_string_lossy()).unwrap());
        let bake = "C:/code/triangulum/game/assets/world/seed42";
        let mut config = WorldConfig::load(&dir.join("world.json").to_string_lossy()).unwrap();
        config.bake = bake.into();
        let b: Arc<dyn crate::worldgen::bake::Bake> = Arc::new(crate::worldgen::bake::open(bake).unwrap());
        let engine = Engine::build(config, registry.clone(), b, Arc::new(EditStore::new()), crate::stream::LodParams { factor: 5.0, max_inflight: 4 }, crate::mesh::MeshOptions::default(), 2, false, Vec::new(), false).unwrap();
        let engine = Arc::new(engine);
        let mut s = Session::new(engine.clone(), Player::new(glam::DVec3::X * 1e7, 0.0, 0.0, Mode::Fly, 1.0), None, String::new(), Sim::new(SimConfig::default()));
        let mut w = workshop(&dir, &registry);
        w.hotbar = engine.hotbar.clone();
        w.requests.push(Request::Add { name: "brick".into(), colour: [0.4, 0.1, 0.08], falls: false });
        w.apply_requests(&mut s);
        let brick = registry.id("brick").expect("added");
        assert!(registry.is_solid(brick) && !registry.has_gravity(brick));
        assert_eq!(Registry::load(&path.to_string_lossy()).unwrap().id("brick").unwrap(), brick, "written to blocks.json at once");
        assert_eq!(w.bui.selected, Some(brick));
        w.requests.push(Request::Rename(brick, "red_brick".into()));
        w.requests.push(Request::Falls(brick, true));
        let mut hb = w.hotbar.clone();
        hb[0] = brick;
        w.requests.push(Request::Hotbar(Some(hb)));
        w.apply_requests(&mut s);
        assert!(registry.has_gravity(brick));
        assert_eq!(s.engine.hotbar[0], brick, "key 1 holds the new block");
        assert_eq!(s.selected_block(), brick);
        let back = Registry::load(&path.to_string_lossy()).unwrap();
        assert_eq!(back.name(brick), "red_brick");
        assert_eq!(back.id("brick").unwrap(), brick, "the old name still answers");
        assert!(back.has_gravity(brick));
        assert_eq!(back.saved_hotbar().unwrap().unwrap()[0], brick);
        // A later rename renames it on the hotbar in the file too.
        w.requests.push(Request::Rename(brick, "fire_brick".into()));
        w.apply_requests(&mut s);
        let back = Registry::load(&path.to_string_lossy()).unwrap();
        assert_eq!(back.hotbar_names().unwrap()[0], "fire_brick");
        assert_eq!(back.def(brick).was, vec!["brick".to_string(), "red_brick".to_string()]);
        // Back to the default hotbar: the key goes from the file too.
        w.requests.push(Request::Hotbar(None));
        w.apply_requests(&mut s);
        assert_eq!(s.engine.hotbar, registry.default_hotbar());
        assert!(Registry::load(&path.to_string_lossy()).unwrap().saved_hotbar().is_none());
        assert!(!std::fs::read_to_string(&path).unwrap().contains("\"hotbar\""));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

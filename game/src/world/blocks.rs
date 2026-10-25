//! Block definitions, read from `assets/blocks.json` at startup.
//!
//! M3a: colours may change while the game runs (Andrew's look panel and hot
//! reload of the file). They reach the GPU as a table of three entries per
//! id (top, sides, bottom), so a colour change rewrites that small table and
//! nothing is remeshed.
//!
//! M3b: the registry also changes while the game runs. Andrew adds a block
//! in the workshop, renames one (the old name is kept in `was`, so a save or
//! a replay that knows the block by it still finds it), switches whether it
//! falls like sand, hides it from the hotbar, and gives its faces textures
//! (PNG files in the `textures/` folder beside blocks.json). Ids are
//! append-only and stable: a block is never deleted or renumbered.
//!
//! Every holder of the shared `Arc<Registry>` (the world, the mesher
//! threads, the simulation, the player) sees a change at once: definitions
//! and looks sit behind locks, and the four flags the hot loops read are
//! one relaxed atomic load each (the cost of the plain bools they replace),
//! with room for every id a `BlockId` can hold, so an id past the end reads
//! as air-like instead of panicking.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicUsize, Ordering};

pub type BlockId = u16;

/// Ids the flag table has room for: all of them.
const CAPACITY: usize = 1 << 16;
const SOLID: u8 = 1;
const LIQUID: u8 = 2;
const GRAVITY: u8 = 4;
const REPLACEABLE: u8 = 8;

/// Texture size when blocks.json names none: 16 x 16 pixels.
pub const DEFAULT_TEXTURE_SIZE: u32 = 16;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BlockDef {
    pub id: BlockId,
    pub name: String,
    /// Linear RGB of every face that has no colour of its own below.
    pub color: [f32; 3],
    /// Optional per-face colours (M3a): the face pointing up, the four
    /// sides, the face pointing down. Grass is green on top and dirt
    /// elsewhere, so a tunnel ceiling under it is not green.
    #[serde(default)]
    pub top: Option<[f32; 3]>,
    #[serde(default)]
    pub side: Option<[f32; 3]>,
    #[serde(default)]
    pub bottom: Option<[f32; 3]>,
    /// Optional textures (M3b): a PNG in `textures/` beside blocks.json for
    /// every face, unless a face names its own. A face without a texture
    /// is its flat colour, as before.
    #[serde(default)]
    pub texture: Option<String>,
    #[serde(default)]
    pub top_texture: Option<String>,
    #[serde(default)]
    pub side_texture: Option<String>,
    #[serde(default)]
    pub bottom_texture: Option<String>,
    #[serde(default)]
    pub solid: bool,
    #[serde(default)]
    pub liquid: bool,
    /// Falls when the cell below is replaceable (sand, gravel): the
    /// simulation's gravity rule reads this, nothing is hard-coded by name.
    #[serde(default)]
    pub gravity: bool,
    /// A falling block may enter this cell and take its place (air, water).
    #[serde(default)]
    pub replaceable: bool,
    /// Not offered on the hotbar (M3b). Blocks are never deleted: a block
    /// Andrew no longer wants is hidden, and the world keeps any he placed.
    #[serde(default)]
    pub hidden: bool,
    /// Names the block had before (M3b), oldest first: a save or a replay
    /// that names it by one of them still finds it.
    #[serde(default)]
    pub was: Vec<String>,
    /// The block's own `_comment` in the file.
    #[serde(default, rename = "_comment")]
    pub comment: Option<String>,
}

impl BlockDef {
    /// This block's colours as the file gives them.
    pub fn colours(&self) -> BlockColours {
        BlockColours { color: self.color, top: self.top, side: self.side, bottom: self.bottom }
    }
    /// This block's textures as the file gives them.
    pub fn textures(&self) -> BlockTextures {
        BlockTextures { all: self.texture.clone(), top: self.top_texture.clone(), side: self.side_texture.clone(), bottom: self.bottom_texture.clone() }
    }
    /// Does this block answer to `name` (its name or a former one)?
    pub fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.was.iter().any(|w| w == name)
    }
    /// The name the block was first given: its identity in the rules
    /// digest, so a rename changes no replay or save.
    pub fn first_name(&self) -> &str {
        self.was.first().unwrap_or(&self.name)
    }
    /// The name as the panel shows it ("dry_grass" is "dry grass").
    pub fn label(&self) -> String {
        self.name.replace('_', " ")
    }
    fn flags(&self) -> u8 {
        (self.solid as u8 * SOLID) | (self.liquid as u8 * LIQUID) | (self.gravity as u8 * GRAVITY) | (self.replaceable as u8 * REPLACEABLE)
    }
    /// A new solid block (the workshop's "Add a block").
    pub fn new_solid(name: &str, colours: BlockColours, falls: bool) -> BlockDef {
        BlockDef {
            id: 0,
            name: name.to_string(),
            color: colours.color,
            top: colours.top,
            side: colours.side,
            bottom: colours.bottom,
            texture: None,
            top_texture: None,
            side_texture: None,
            bottom_texture: None,
            solid: true,
            liquid: false,
            gravity: falls,
            replaceable: false,
            hidden: false,
            was: Vec::new(),
            comment: None,
        }
    }
    fn unused(i: usize) -> BlockDef {
        BlockDef { name: format!("unused_{i}"), color: [1.0, 0.0, 1.0], solid: false, id: i as BlockId, ..BlockDef::new_solid("", BlockColours::flat([1.0, 0.0, 1.0]), false) }
    }
}

/// A block's colours: the base colour and the optional per-face ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockColours {
    pub color: [f32; 3],
    pub top: Option<[f32; 3]>,
    pub side: Option<[f32; 3]>,
    pub bottom: Option<[f32; 3]>,
}

impl BlockColours {
    pub fn flat(c: [f32; 3]) -> Self {
        Self { color: c, top: None, side: None, bottom: None }
    }
    /// The colour each face is drawn with: [top, sides, bottom].
    pub fn faces(&self) -> [[f32; 3]; 3] {
        [self.top.unwrap_or(self.color), self.side.unwrap_or(self.color), self.bottom.unwrap_or(self.color)]
    }
    /// Does any face have a colour of its own?
    pub fn per_face(&self) -> bool {
        self.top.is_some() || self.side.is_some() || self.bottom.is_some()
    }
}

/// A block's textures (M3b): file names relative to the textures folder,
/// for every face (`all`) and per face.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockTextures {
    pub all: Option<String>,
    pub top: Option<String>,
    pub side: Option<String>,
    pub bottom: Option<String>,
}

impl BlockTextures {
    /// The texture each face is drawn with: [top, sides, bottom].
    pub fn faces(&self) -> [Option<&str>; 3] {
        let all = self.all.as_deref();
        [self.top.as_deref().or(all), self.side.as_deref().or(all), self.bottom.as_deref().or(all)]
    }
    pub fn any(&self) -> bool {
        self.faces().iter().any(|f| f.is_some())
    }
    pub fn per_face(&self) -> bool {
        self.top.is_some() || self.side.is_some() || self.bottom.is_some()
    }
    /// Set one face (0 top, 1 sides, 2 bottom) to `file`, keeping what the
    /// other faces show.
    pub fn set_face(&mut self, face: usize, file: Option<String>) {
        let [t, s, b] = self.faces().map(|f| f.map(str::to_string));
        let mut f = [t, s, b];
        f[face] = file;
        self.set_faces(f);
    }
    /// Every face from a list, written as `all` when they agree.
    pub fn set_faces(&mut self, f: [Option<String>; 3]) {
        if f[0] == f[1] && f[1] == f[2] {
            *self = BlockTextures { all: f[0].clone(), ..Default::default() };
        } else {
            let [t, s, b] = f;
            *self = BlockTextures { all: None, top: t, side: s, bottom: b };
        }
    }
}

/// Entries per id in the GPU colour table: top, sides, bottom (the shader
/// reads `colours[id * 3 + face]`).
pub const FACES_PER_BLOCK: usize = 3;

/// The face index of the colour table: 0 top, 1 sides, 2 bottom.
pub const FACE_NAMES: [&str; 3] = ["top", "sides", "bottom"];

#[derive(Deserialize)]
struct File {
    blocks: Vec<BlockDef>,
    /// The hotbar's nine slots, by block name (M3b; the workshop writes it).
    #[serde(default)]
    hotbar: Option<Vec<String>>,
    /// Texture size in pixels (M3b; Andrew's setting, 16 by default).
    #[serde(default)]
    texture_size: Option<u32>,
}

/// A face's texture on the GPU: its layer in the texture array and its
/// average colour (linear), set by the texture set (`render::textures`).
pub type FaceLayer = Option<(u32, [f32; 3])>;

struct Defs {
    list: Vec<BlockDef>,
    /// Every name and former name, to its id.
    by_name: HashMap<String, BlockId>,
}

/// Every block the world can contain, indexed by id.
pub struct Registry {
    defs: RwLock<Defs>,
    flags: Box<[AtomicU8]>,
    len: AtomicUsize,
    /// The live colours and textures, one per id. `defs` keeps what the
    /// registry was loaded with (or last told); the look panel and hot
    /// reload change these.
    colours: RwLock<Vec<BlockColours>>,
    textures: RwLock<Vec<BlockTextures>>,
    layers: RwLock<Vec<[FaceLayer; 3]>>,
    hotbar: RwLock<Option<Vec<String>>>,
    texture_size: AtomicU32,
}

/// Is `name` a good block name? Lower-case letters, digits and `_`,
/// starting with a letter, at most 40 characters.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a block needs a name".into());
    }
    if name.len() > 40 {
        return Err("that name is too long (40 letters at most)".into());
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(format!("'{name}': a name is small letters, digits and _, starting with a letter"));
    }
    if name.starts_with("unused_") {
        return Err("names starting with unused_ are kept for gaps in the ids".into());
    }
    Ok(())
}

/// The name for a label typed in the panel: "Red Brick" is "red_brick".
pub fn name_from_label(label: &str) -> String {
    let mut out = String::new();
    for c in label.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

/// Is `file` a texture reference the game accepts: a `.png` path inside
/// the textures folder (no drive, no leading slash, no `..`)?
pub fn check_texture_ref(file: &str) -> Result<(), String> {
    let f = file.replace('\\', "/");
    let bad = f.is_empty() || f.starts_with('/') || f.contains(':') || f.split('/').any(|p| p == ".." || p.is_empty());
    if bad || !f.to_ascii_lowercase().ends_with(".png") {
        return Err(format!("texture '{file}' must be a .png file inside the textures folder"));
    }
    Ok(())
}

fn validate(defs: &[BlockDef]) -> Result<(), String> {
    if defs.is_empty() || defs[0].name != "air" || defs[0].solid {
        return Err("block 0 must be 'air' and not solid".into());
    }
    if !defs[0].replaceable {
        return Err("block 0 (air) must be replaceable, or nothing could ever fall".into());
    }
    if let Some(d) = defs.iter().find(|d| d.gravity && !d.solid) {
        return Err(format!("block {} has gravity but is not solid; only solid blocks fall", d.name));
    }
    if let Some(d) = defs.iter().find(|d| d.replaceable && d.solid) {
        return Err(format!("block {} is replaceable but solid; a falling block cannot take a solid cell", d.name));
    }
    for d in defs {
        let c = d.colours();
        for (face, v) in [("color", Some(c.color)), ("top", c.top), ("side", c.side), ("bottom", c.bottom)] {
            if v.is_some_and(|v| v.iter().any(|x| !x.is_finite() || *x < 0.0)) {
                return Err(format!("block {}: \"{face}\" must be three finite numbers of at least 0", d.name));
            }
        }
        for t in [&d.texture, &d.top_texture, &d.side_texture, &d.bottom_texture].into_iter().flatten() {
            check_texture_ref(t).map_err(|e| format!("block {}: {e}", d.name))?;
        }
    }
    Ok(())
}

impl Registry {
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::parse(&text).map_err(|e| format!("{path}: {e}"))
    }

    /// Parse the text of a blocks file.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|e| format!("parse: {e}"))?;
        let r = Self::from_defs(file.blocks)?;
        if let Some(n) = file.texture_size {
            if !(4..=256).contains(&n) || !n.is_power_of_two() {
                return Err(format!("texture_size {n}: a power of two from 4 to 256 (16 and 32 are the usual)"));
            }
            r.texture_size.store(n, Ordering::Relaxed);
        }
        if let Some(h) = &file.hotbar {
            if h.len() > 9 {
                return Err(format!("the hotbar has nine slots, the file names {}", h.len()));
            }
        }
        *r.hotbar.write().unwrap() = file.hotbar;
        Ok(r)
    }

    pub fn from_defs(mut defs: Vec<BlockDef>) -> Result<Self, String> {
        defs.sort_by_key(|d| d.id);
        let max = defs.last().map(|d| d.id).unwrap_or(0) as usize;
        let mut dense: Vec<Option<BlockDef>> = vec![None; max + 1];
        let mut by_name = HashMap::new();
        for d in defs {
            let id = d.id as usize;
            if dense[id].is_some() {
                return Err(format!("duplicate block id {}", d.id));
            }
            for n in std::iter::once(&d.name).chain(&d.was) {
                if by_name.insert(n.clone(), d.id).is_some() {
                    return Err(format!("duplicate block name {n}"));
                }
            }
            dense[id] = Some(d);
        }
        let defs: Vec<BlockDef> = dense.into_iter().enumerate().map(|(i, d)| d.unwrap_or_else(|| BlockDef::unused(i))).collect();
        validate(&defs)?;
        let flags: Box<[AtomicU8]> = (0..CAPACITY).map(|i| AtomicU8::new(defs.get(i).map_or(0, |d| d.flags()))).collect();
        let n = defs.len();
        Ok(Self {
            colours: RwLock::new(defs.iter().map(|d| d.colours()).collect()),
            textures: RwLock::new(defs.iter().map(|d| d.textures()).collect()),
            layers: RwLock::new(vec![[None; 3]; n]),
            defs: RwLock::new(Defs { list: defs, by_name }),
            flags,
            len: AtomicUsize::new(n),
            hotbar: RwLock::new(None),
            texture_size: AtomicU32::new(DEFAULT_TEXTURE_SIZE),
        })
    }

    /// The id of a block by its name or a former name.
    pub fn id(&self, name: &str) -> Result<BlockId, String> {
        self.defs.read().unwrap().by_name.get(name).copied().ok_or_else(|| format!("no block named {name}"))
    }
    /// A copy of a block's definition (the registry can change under it).
    pub fn def(&self, id: BlockId) -> BlockDef {
        self.defs.read().unwrap().list[id as usize].clone()
    }
    pub fn name(&self, id: BlockId) -> String {
        self.defs.read().unwrap().list.get(id as usize).map_or_else(|| format!("unknown_{id}"), |d| d.name.clone())
    }
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[inline]
    fn flag(&self, id: BlockId, f: u8) -> bool {
        self.flags[id as usize].load(Ordering::Relaxed) & f != 0
    }
    #[inline]
    pub fn is_solid(&self, id: BlockId) -> bool {
        self.flag(id, SOLID)
    }
    #[inline]
    pub fn is_liquid(&self, id: BlockId) -> bool {
        self.flag(id, LIQUID)
    }
    /// Does this block fall when unsupported (`"gravity": true` in blocks.json)?
    #[inline]
    pub fn has_gravity(&self, id: BlockId) -> bool {
        self.flag(id, GRAVITY)
    }
    /// May a falling block land in or pass through a cell holding this block?
    #[inline]
    pub fn is_replaceable(&self, id: BlockId) -> bool {
        self.flag(id, REPLACEABLE)
    }
    /// A copy of every definition, by id.
    pub fn defs(&self) -> Vec<BlockDef> {
        self.defs.read().unwrap().list.clone()
    }
    /// Texture size in pixels (`texture_size` in blocks.json; 16).
    pub fn texture_size(&self) -> u32 {
        self.texture_size.load(Ordering::Relaxed)
    }
    pub fn set_texture_size(&self, n: u32) {
        self.texture_size.store(n, Ordering::Relaxed);
    }

    /// Colour table for the GPU from the live colours and textures: three
    /// RGBA per id (top, sides, bottom). A flat face is its colour with
    /// alpha -1; a textured face (M3b) is the texture's average colour with
    /// the texture's layer in alpha, and the shader samples the layer.
    pub fn colour_table(&self) -> Vec<[f32; 4]> {
        let colours = self.colours.read().unwrap();
        let layers = self.layers.read().unwrap();
        let mut out = Vec::with_capacity(colours.len() * FACES_PER_BLOCK);
        for (id, c) in colours.iter().enumerate() {
            let faces = c.faces();
            for f in 0..FACES_PER_BLOCK {
                out.push(match layers.get(id).and_then(|l| l[f]) {
                    Some((layer, avg)) => [avg[0], avg[1], avg[2], layer as f32],
                    None => [faces[f][0], faces[f][1], faces[f][2], -1.0],
                });
            }
        }
        out
    }
    /// The live colours of every id.
    pub fn colours(&self) -> Vec<BlockColours> {
        self.colours.read().unwrap().clone()
    }
    /// A block's live swatch colour (the hotbar): its top face's texture
    /// average when it has one, else its base colour.
    pub fn colour(&self, id: BlockId) -> [f32; 3] {
        if let Some(Some((_, avg))) = self.layers.read().unwrap().get(id as usize).map(|l| l[0]) {
            return avg;
        }
        self.colours.read().unwrap()[id as usize].color
    }
    /// Replace the live colours, one per id from 0 (ids past the end of
    /// `new` keep theirs). Returns whether anything changed.
    pub fn set_colours(&self, new: &[BlockColours]) -> bool {
        let mut colours = self.colours.write().unwrap();
        let mut changed = false;
        for (c, n) in colours.iter_mut().zip(new) {
            changed |= *c != *n;
            *c = *n;
        }
        changed
    }
    /// The live textures of every id (M3b).
    pub fn textures(&self) -> Vec<BlockTextures> {
        self.textures.read().unwrap().clone()
    }
    /// Replace the live textures (as `set_colours`). Returns whether
    /// anything changed; the texture set then assigns layers.
    pub fn set_textures(&self, new: &[BlockTextures]) -> bool {
        let mut t = self.textures.write().unwrap();
        let mut changed = false;
        for (c, n) in t.iter_mut().zip(new) {
            if c != n {
                changed = true;
                *c = n.clone();
            }
        }
        changed
    }
    /// The GPU layer and average colour of every id's faces (from the
    /// texture set). Returns whether anything changed.
    pub fn set_layers(&self, new: &[[FaceLayer; 3]]) -> bool {
        let mut l = self.layers.write().unwrap();
        let n = l.len();
        let mut changed = false;
        for (c, v) in l.iter_mut().zip(new.iter().take(n)) {
            changed |= c != v;
            *c = *v;
        }
        changed
    }

    /// The first eight solid blocks by id and then the first liquid (water on
    /// key 9, M2b): what keys 1-9 place unless the file's hotbar or
    /// `--hotbar` says otherwise. Fixed whatever is hidden (the play
    /// scripts press 1-9 expecting it); new blocks come after the shipped
    /// ones, so they never move it.
    pub fn default_hotbar(&self) -> Vec<BlockId> {
        let defs = self.defs.read().unwrap();
        let mut hb: Vec<BlockId> = defs.list.iter().filter(|d| d.solid && !d.name.starts_with("unused_")).map(|d| d.id).take(8).collect();
        if let Some(l) = defs.list.iter().find(|d| d.liquid) {
            hb.push(l.id);
        }
        hb
    }
    /// The hotbar blocks.json names (M3b), resolved; None when it names none.
    pub fn saved_hotbar(&self) -> Option<Result<Vec<BlockId>, String>> {
        let names = self.hotbar.read().unwrap().clone()?;
        Some(names.iter().map(|n| self.id(n)).collect::<Result<Vec<_>, _>>().map_err(|e| format!("blocks.json's hotbar: {e}")))
    }
    /// The hotbar names the file holds (or will, once written).
    pub fn hotbar_names(&self) -> Option<Vec<String>> {
        self.hotbar.read().unwrap().clone()
    }
    pub fn set_hotbar_names(&self, names: Option<Vec<String>>) {
        *self.hotbar.write().unwrap() = names;
    }
    /// Resolve a comma-separated list of block names (for `--hotbar`).
    pub fn hotbar_from(&self, names: &str) -> Result<Vec<BlockId>, String> {
        let ids: Vec<BlockId> = names.split(',').map(|n| self.id(n.trim())).collect::<Result<_, _>>()?;
        if ids.is_empty() || ids.len() > 9 {
            return Err(format!("the hotbar takes 1 to 9 blocks, got {}", ids.len()));
        }
        Ok(ids)
    }
    /// Is `other` the same set of blocks (ids, names and flags), so that
    /// only colours and textures differ?
    pub fn same_blocks(&self, other: &Registry) -> bool {
        let (a, b) = (self.defs(), other.defs());
        a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| a.id == b.id && a.name == b.name && a.flags() == b.flags() && a.hidden == b.hidden && a.was == b.was)
    }

    // ------------------------------------------------------- changing blocks (M3b)

    /// Append a block: it takes the next id, which it keeps forever.
    pub fn add(&self, def: BlockDef) -> Result<BlockId, String> {
        check_name(&def.name)?;
        self.push(def)
    }

    /// Append a definition (its own former names included) at the next id.
    fn push(&self, mut def: BlockDef) -> Result<BlockId, String> {
        let mut defs = self.defs.write().unwrap();
        if let Some(n) = std::iter::once(&def.name).chain(&def.was).find(|n| defs.by_name.contains_key(*n)) {
            return Err(format!("there is already a block called '{n}'"));
        }
        let id = defs.list.len();
        if id >= CAPACITY {
            return Err("no ids left".into());
        }
        def.id = id as BlockId;
        let mut probe = defs.list.clone();
        probe.push(def.clone());
        validate(&probe)?;
        // Everything an id indexes grows before the id is published (len).
        self.colours.write().unwrap().push(def.colours());
        self.textures.write().unwrap().push(def.textures());
        self.layers.write().unwrap().push([None; 3]);
        self.flags[id].store(def.flags(), Ordering::Relaxed);
        for n in std::iter::once(&def.name).chain(&def.was) {
            defs.by_name.insert(n.clone(), def.id);
        }
        defs.list.push(def);
        self.len.store(id + 1, Ordering::Release);
        Ok(id as BlockId)
    }

    /// Give a block a new name. The old one is kept in `was`, so saves and
    /// replays that name the block by it still find it.
    pub fn rename(&self, id: BlockId, new: &str) -> Result<(), String> {
        check_name(new)?;
        self.rename_to(id, new)
    }

    fn rename_to(&self, id: BlockId, new: &str) -> Result<(), String> {
        let mut defs = self.defs.write().unwrap();
        let d = defs.list.get(id as usize).ok_or("no such block")?;
        if d.name == new {
            return Ok(());
        }
        if let Some(&other) = defs.by_name.get(new) {
            if other != id {
                return Err(format!("there is already a block called '{new}'"));
            }
        }
        let d = &mut defs.list[id as usize];
        let old = std::mem::replace(&mut d.name, new.to_string());
        if !d.was.contains(&old) {
            d.was.push(old);
        }
        d.was.retain(|w| w != new);
        defs.by_name.insert(new.to_string(), id);
        Ok(())
    }

    /// Does this solid block fall like sand? (Changes the game's rules.)
    pub fn set_falls(&self, id: BlockId, falls: bool) -> Result<(), String> {
        let mut defs = self.defs.write().unwrap();
        let d = defs.list.get_mut(id as usize).ok_or("no such block")?;
        if falls && !d.solid {
            return Err(format!("{} is not solid; only solid blocks fall", d.name));
        }
        d.gravity = falls;
        self.flags[id as usize].store(d.flags(), Ordering::Relaxed);
        Ok(())
    }

    pub fn set_hidden(&self, id: BlockId, hidden: bool) -> Result<(), String> {
        let mut defs = self.defs.write().unwrap();
        defs.list.get_mut(id as usize).ok_or("no such block")?.hidden = hidden;
        Ok(())
    }

    /// Bring this registry up to `file` (blocks.json changed on disk): new
    /// blocks are appended, renames (the new definition lists the old name
    /// in `was`), falling and hiding follow the file, and colours and
    /// textures are left to the caller (the look panel keeps unsaved
    /// edits). Refused whole when the file renumbers or drops a block, or
    /// changes solid, liquid or replaceable (those need a restart: every
    /// chunk would have to be rebuilt). Returns what changed, in words.
    pub fn merge_file(&self, file: &Registry) -> Result<Vec<String>, String> {
        let (mine, theirs) = (self.defs(), file.defs());
        if theirs.len() < mine.len() {
            return Err(format!("it has {} blocks and the game has {}: ids are never deleted (hide a block instead)", theirs.len(), mine.len()));
        }
        for (a, b) in mine.iter().zip(&theirs) {
            if a.name != b.name && !b.was.contains(&a.name) {
                return Err(format!("block {} is '{}' in the game but '{}' in the file (ids are stable: append, never renumber; a rename keeps the old name in \"was\")", a.id, a.name, b.name));
            }
            let (fa, fb) = (a.flags() & !GRAVITY, b.flags() & !GRAVITY);
            if fa != fb {
                return Err(format!("{}'s solid, liquid or replaceable flag changed: that needs a restart", b.name));
            }
        }
        // Names the new blocks bring must be free here (nothing is applied
        // until everything has been checked).
        {
            let defs = self.defs.read().unwrap();
            for b in &theirs[mine.len()..] {
                if let Some(n) = std::iter::once(&b.name).chain(&b.was).find(|n| defs.by_name.get(*n).is_some_and(|&id| (id as usize) < mine.len() && !theirs[id as usize].answers_to(n))) {
                    return Err(format!("the new block '{}' takes the name '{n}', which another block has", b.name));
                }
            }
        }
        let mut done = Vec::new();
        for (a, b) in mine.iter().zip(&theirs) {
            if a.name != b.name {
                self.rename_to(a.id, &b.name)?;
                done.push(format!("renamed {} to {}", a.name, b.name));
            }
            if a.gravity != b.gravity {
                self.set_falls(a.id, b.gravity)?;
                done.push(format!("{} {}", b.name, if b.gravity { "now falls" } else { "no longer falls" }));
            }
            if a.hidden != b.hidden {
                self.set_hidden(a.id, b.hidden)?;
                done.push(format!("{} {}", b.name, if b.hidden { "hidden from the hotbar" } else { "shown on the hotbar again" }));
            }
        }
        for b in &theirs[mine.len()..] {
            self.push(b.clone())?;
            done.push(format!("added {}", b.name));
        }
        if file.hotbar_names() != self.hotbar_names() {
            self.set_hotbar_names(file.hotbar_names());
            done.push("hotbar".into());
        }
        if file.texture_size() != self.texture_size() {
            self.set_texture_size(file.texture_size());
            done.push(format!("texture size {}", file.texture_size()));
        }
        Ok(done)
    }
}

// ------------------------------------------------------------ writing

/// A colour component as the blocks file writes it: at least three
/// decimals (the file's style), a fourth when it matters (dark colours).
fn colour_number(x: f32) -> String {
    let s = format!("{:.4}", x.max(0.0));
    let s = if s.ends_with('0') { &s[..s.len() - 1] } else { &s[..] };
    s.to_string()
}

fn colour_value(c: [f32; 3]) -> String {
    format!("[{}, {}, {}]", colour_number(c[0]), colour_number(c[1]), colour_number(c[2]))
}

/// The keys of a block entry, in the order the file writes them.
const ORDER: [&str; 16] = ["id", "name", "color", "top", "side", "bottom", "texture", "top_texture", "side_texture", "bottom_texture", "solid", "liquid", "gravity", "replaceable", "hidden", "was"];

/// The blocks file's text in its own style: the top-level fields, then one
/// line per block in id order with the ids and names in aligned columns.
/// Keys keep the order id, name, colours, textures, flags, then anything
/// else. Lines end with `nl` (the file's own line ending: git checks text
/// out with CRLF on Windows).
pub fn format_blocks_file(top: &Map<String, Value>, blocks: &[Map<String, Value>], nl: &str) -> String {
    let value = |k: &str, v: &Value| -> String {
        let colour = matches!(k, "color" | "top" | "side" | "bottom");
        match v.as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect::<Vec<f32>>()) {
            Some(c) if colour && c.len() == 3 => colour_value([c[0], c[1], c[2]]),
            _ => v.to_string(),
        }
    };
    // Each row's fields: (column, `"key": value`). The known keys are
    // columns; anything else (a block's `_comment`) trails unaligned.
    let rows: Vec<Vec<(Option<usize>, String)>> = blocks
        .iter()
        .map(|b| {
            let mut f: Vec<(Option<usize>, String)> = ORDER.iter().enumerate().filter_map(|(c, k)| b.get(*k).map(|v| (Some(c), format!("\"{k}\": {}", value(k, v))))).collect();
            f.extend(b.iter().filter(|(k, _)| !ORDER.contains(&k.as_str())).map(|(k, v)| (None, format!("{}: {}", Value::String(k.clone()), v))));
            f
        })
        .collect();
    // A column is as wide as its widest field plus its comma and a space,
    // so the next column lines up wherever a row has the same keys.
    let mut width = [0usize; ORDER.len()];
    for row in &rows {
        for (c, text) in row.iter().take(row.len().saturating_sub(1)) {
            if let Some(c) = c {
                width[*c] = width[*c].max(text.len() + 2);
            }
        }
    }
    let mut out = format!("{{{nl}");
    for (k, v) in top.iter().filter(|(k, _)| k.as_str() != "blocks") {
        out.push_str(&format!("  {}: {},{nl}", Value::String(k.clone()), v));
    }
    out.push_str(&format!("  \"blocks\": [{nl}"));
    for (n, row) in rows.iter().enumerate() {
        let mut line = String::new();
        for (i, (c, text)) in row.iter().enumerate() {
            if i + 1 == row.len() {
                line.push_str(text);
            } else {
                let w = c.map_or(0, |c| width[c]);
                line.push_str(&format!("{:<w$}", format!("{text}, ")));
            }
        }
        let comma = if n + 1 < blocks.len() { "," } else { "" };
        out.push_str(&format!("    {{ {line} }}{comma}{nl}"));
    }
    out.push_str(&format!("  ]{nl}}}{nl}"));
    out
}

/// The blocks file `text` changed by `edit`, which gets its top-level
/// fields and its block entries (sorted by id), in the file's own style.
pub fn edit_text(text: &str, edit: impl FnOnce(&mut Map<String, Value>, &mut Vec<Map<String, Value>>) -> Result<(), String>) -> Result<String, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("parse: {e}"))?;
    let mut top = doc.as_object().ok_or("not a JSON object")?.clone();
    let mut blocks: Vec<Map<String, Value>> = top.get("blocks").and_then(|b| b.as_array()).ok_or("no \"blocks\" list")?.iter().map(|b| b.as_object().cloned().unwrap_or_default()).collect();
    blocks.sort_by_key(|b| b.get("id").and_then(|v| v.as_u64()).unwrap_or(u64::MAX));
    edit(&mut top, &mut blocks)?;
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    Ok(format_blocks_file(&top, &blocks, nl))
}

/// Rewrite the blocks file at `path` through `edit`: written beside the
/// file and renamed over it, so a crash leaves the old file.
pub fn edit_file(path: &str, edit: impl FnOnce(&mut Map<String, Value>, &mut Vec<Map<String, Value>>) -> Result<(), String>) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let new = edit_text(&text, edit).map_err(|e| format!("{path}: {e}"))?;
    if new == text {
        return Ok(());
    }
    let tmp = format!("{path}.saving");
    std::fs::write(&tmp, new).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {path}: {e}"))
}

fn colour_json(c: [f32; 3]) -> Value {
    Value::Array(c.iter().map(|&x| serde_json::Number::from_f64(x as f64).map_or(Value::Null, Value::Number)).collect())
}

fn set_opt(b: &mut Map<String, Value>, k: &str, v: Option<Value>) {
    match v {
        Some(v) => {
            b.insert(k.into(), v);
        }
        None => {
            b.remove(k);
        }
    }
}

fn put_colours(b: &mut Map<String, Value>, c: &BlockColours) {
    b.insert("color".into(), colour_json(c.color));
    for (k, v) in [("top", c.top), ("side", c.side), ("bottom", c.bottom)] {
        set_opt(b, k, v.map(colour_json));
    }
}

fn put_textures(b: &mut Map<String, Value>, t: &BlockTextures) {
    for (k, v) in [("texture", &t.all), ("top_texture", &t.top), ("side_texture", &t.side), ("bottom_texture", &t.bottom)] {
        set_opt(b, k, v.clone().map(Value::String));
    }
}

fn entry_id(b: &Map<String, Value>) -> Option<usize> {
    b.get("id").and_then(|v| v.as_u64()).map(|v| v as usize)
}

/// The blocks file `text` with its colours replaced by `colours` (by id).
pub fn colours_into_text(text: &str, colours: &[BlockColours]) -> Result<String, String> {
    look_into_text(text, colours, None)
}

/// The blocks file `text` with its colours (and, when given, its
/// textures) replaced by the live ones, by id; everything else kept.
pub fn look_into_text(text: &str, colours: &[BlockColours], textures: Option<&[BlockTextures]>) -> Result<String, String> {
    edit_text(text, |_, blocks| {
        for b in blocks.iter_mut() {
            let Some(id) = entry_id(b) else { continue };
            if let Some(c) = colours.get(id) {
                put_colours(b, c);
            }
            if let Some(t) = textures.and_then(|t| t.get(id)) {
                put_textures(b, t);
            }
        }
        Ok(())
    })
}

/// Write the live colours into the blocks file at `path`, keeping its
/// style, its stable ids and every other field as it is on disk.
pub fn save_colours(path: &str, colours: &[BlockColours]) -> Result<(), String> {
    edit_file(path, |top, blocks| {
        let _ = top;
        for b in blocks.iter_mut() {
            if let Some(c) = entry_id(b).and_then(|id| colours.get(id)) {
                put_colours(b, c);
            }
        }
        Ok(())
    })
}

/// "Save look" (M3b): the live colours and textures of every block.
pub fn save_look(path: &str, colours: &[BlockColours], textures: &[BlockTextures]) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let new = look_into_text(&text, colours, Some(textures)).map_err(|e| format!("{path}: {e}"))?;
    if new == text {
        return Ok(());
    }
    let tmp = format!("{path}.saving");
    std::fs::write(&tmp, new).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {path}: {e}"))
}

/// The blocks file `text` with every block's definition as the registry
/// holds it (M3b): names, former names, flags and hiding for the blocks it
/// has, whole entries (with their colours and textures) appended for the
/// blocks it lacks, and the hotbar and texture size. Colours and textures
/// of blocks already in the file are left as they are on disk: those wait
/// for Save look.
pub fn defs_into_text(text: &str, registry: &Registry) -> Result<String, String> {
    let defs = registry.defs();
    let (colours, textures) = (registry.colours(), registry.textures());
    let hotbar = registry.hotbar_names();
    let size = registry.texture_size();
    edit_text(text, |top, blocks| {
        let have: std::collections::HashSet<usize> = blocks.iter().filter_map(entry_id).collect();
        for d in defs.iter().filter(|d| !have.contains(&(d.id as usize)) && !d.name.starts_with("unused_")) {
            let mut b = Map::new();
            b.insert("id".into(), Value::from(d.id));
            put_colours(&mut b, &colours[d.id as usize]);
            put_textures(&mut b, &textures[d.id as usize]);
            blocks.push(b);
        }
        for b in blocks.iter_mut() {
            let Some(d) = entry_id(b).and_then(|id| defs.get(id)) else { continue };
            b.insert("name".into(), Value::String(d.name.clone()));
            b.insert("solid".into(), Value::Bool(d.solid));
            b.insert("liquid".into(), Value::Bool(d.liquid));
            // Leave a default false out, as the shipped file does.
            for (k, v) in [("gravity", d.gravity), ("replaceable", d.replaceable), ("hidden", d.hidden)] {
                set_opt(b, k, (v || b.contains_key(k)).then_some(Value::Bool(v)));
            }
            set_opt(b, "was", (!d.was.is_empty()).then(|| Value::Array(d.was.iter().cloned().map(Value::String).collect())));
        }
        blocks.sort_by_key(|b| entry_id(b).unwrap_or(usize::MAX));
        set_opt(top, "hotbar", hotbar.map(|h| Value::Array(h.into_iter().map(Value::String).collect())));
        if size != DEFAULT_TEXTURE_SIZE || top.contains_key("texture_size") {
            top.insert("texture_size".into(), Value::from(size));
        }
        Ok(())
    })
}

/// Write the registry's definitions into the blocks file at `path` at once
/// (a new block must reach the file before any save can hold it).
pub fn save_defs(path: &str, registry: &Registry) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let new = defs_into_text(&text, registry).map_err(|e| format!("{path}: {e}"))?;
    if new == text {
        return Ok(());
    }
    let tmp = format!("{path}.saving");
    std::fs::write(&tmp, new).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json");

    #[test]
    fn loads_the_shipped_definitions() {
        let r = Registry::load(SHIPPED).unwrap();
        assert_eq!(r.id("air").unwrap(), 0);
        assert!(r.is_solid(r.id("stone").unwrap()));
        assert!(r.is_liquid(r.id("water").unwrap()));
        assert!(!r.is_solid(r.id("water").unwrap()));
        assert!(r.len() >= 16);
        // The simulation's flags: sand and gravel fall, air and water give way.
        assert!(r.has_gravity(r.id("sand").unwrap()) && r.has_gravity(r.id("gravel").unwrap()));
        assert!(!r.has_gravity(r.id("stone").unwrap()) && !r.has_gravity(r.id("dirt").unwrap()));
        assert!(r.is_replaceable(0) && r.is_replaceable(r.id("water").unwrap()));
        assert!(!r.is_replaceable(r.id("sand").unwrap()));
        // No textures ship enabled: every face is its flat colour (alpha -1).
        assert!(r.colour_table().iter().all(|e| e[3] == -1.0));
        assert_eq!(r.texture_size(), DEFAULT_TEXTURE_SIZE);
        // Ids past the end read as air-like rather than panicking.
        assert!(!r.is_solid(60_000) && !r.is_liquid(60_000));
    }

    #[test]
    fn contradictory_flags_are_refused() {
        let mut defs: Vec<BlockDef> = Registry::load(SHIPPED).unwrap().defs().to_vec();
        let err = |d: Vec<BlockDef>| Registry::from_defs(d).err().expect("refused");
        defs[8].solid = true; // water
        assert!(err(defs.clone()).contains("replaceable but solid"));
        defs[8].solid = false;
        defs[8].gravity = true;
        assert!(err(defs.clone()).contains("gravity but is not solid"));
        defs[8].gravity = false;
        defs[0].replaceable = false;
        assert!(err(defs.clone()).contains("air) must be replaceable"));
        defs[0].replaceable = true;
        defs[3].top = Some([0.1, f32::NAN, 0.1]);
        assert!(err(defs.clone()).contains("\"top\" must be"));
        defs[3].top = None;
        defs[3].texture = Some("../outside.png".into());
        assert!(err(defs.clone()).contains("inside the textures folder"));
        defs[3].texture = Some("C:/x.png".into());
        assert!(err(defs.clone()).contains("inside the textures folder"));
        defs[3].texture = Some("grass.jpg".into());
        assert!(err(defs).contains("inside the textures folder"));
    }

    /// M3a: grass, dry grass and snow sit on dirt, so their undersides (a
    /// tunnel ceiling) are dirt; grass and dry grass have dirt sides too
    /// (a shaft wall), and every other block is one colour all round.
    #[test]
    fn per_face_colours_and_the_gpu_table() {
        let r = Registry::load(SHIPPED).unwrap();
        let dirt = r.def(r.id("dirt").unwrap()).color;
        let table = r.colour_table();
        assert_eq!(table.len(), r.len() * FACES_PER_BLOCK);
        let face = |name: &str, f: usize| {
            let t = table[r.id(name).unwrap() as usize * FACES_PER_BLOCK + f];
            [t[0], t[1], t[2]]
        };
        for name in ["grass", "dry_grass", "snow"] {
            let d = r.def(r.id(name).unwrap());
            assert_eq!(face(name, 0), d.color, "{name} top is its own colour");
            assert_eq!(face(name, 2), dirt, "{name} underside is dirt");
        }
        assert_eq!(face("grass", 1), dirt);
        assert_eq!(face("dry_grass", 1), dirt);
        assert_eq!(face("snow", 1), r.def(r.id("snow").unwrap()).color, "a snow cap is white at its sides");
        assert!(face("grass", 0)[1] > face("grass", 0)[0] * 2.0, "grass top stays green");
        let stone = r.def(r.id("stone").unwrap()).color;
        assert_eq!([face("stone", 0), face("stone", 1), face("stone", 2)], [stone; 3]);
        // Live colours: changing one reaches the table, and nothing else moves.
        let mut c = r.colours();
        let g = r.id("grass").unwrap() as usize;
        c[g].top = Some([0.5, 0.25, 0.125]);
        assert!(r.set_colours(&c));
        assert!(!r.set_colours(&c), "setting the same colours again changes nothing");
        let t2 = r.colour_table();
        assert_eq!(t2[g * 3], [0.5, 0.25, 0.125, -1.0]);
        assert_eq!(t2.iter().enumerate().filter(|(n, x)| **x != table[*n]).count(), 1);
        // A textured face carries its layer and average colour (M3b).
        let mut layers = vec![[None; 3]; r.len()];
        layers[g][1] = Some((7, [0.2, 0.1, 0.05]));
        assert!(r.set_layers(&layers));
        let t3 = r.colour_table();
        assert_eq!(t3[g * 3 + 1], [0.2, 0.1, 0.05, 7.0]);
        assert_eq!(t3[g * 3 + 2], t2[g * 3 + 2], "the other faces stay flat");
    }

    /// The JSON round trip: the shipped file rewritten from its own
    /// colours parses back to the same blocks, keeps its comments, ids and
    /// style, and writing it a second time changes nothing.
    #[test]
    fn blocks_file_round_trip() {
        let text = std::fs::read_to_string(SHIPPED).unwrap();
        let r = Registry::parse(&text).unwrap();
        let once = colours_into_text(&text, &r.colours()).unwrap();
        let twice = colours_into_text(&once, &Registry::parse(&once).unwrap().colours()).unwrap();
        assert_eq!(once, twice, "writing is idempotent");
        let back = Registry::parse(&once).unwrap();
        assert!(back.same_blocks(&r));
        for (a, b) in r.defs().iter().zip(back.defs()) {
            assert_eq!(a.colours(), b.colours(), "{}", a.name);
            assert_eq!(a.comment, b.comment);
        }
        // The file's own style survives: the top comment, a line per block,
        // aligned names, and the shipped text is already in that style.
        assert!(once.replace("\r\n", "\n").starts_with("{\n  \"_comment\": "));
        assert_eq!(once.lines().filter(|l| l.trim_start().starts_with("{ \"id\":")).count(), r.defs().len());
        assert!(once.contains("{ \"id\": 1,  \"name\": \"stone\",        \"color\": [0.300, 0.290, 0.280], \"solid\": true,  \"liquid\": false }"));
        assert_eq!(once, text, "the shipped blocks.json is in the writer's style");
        // The definitions writer leaves the shipped file as it is too.
        assert_eq!(defs_into_text(&text, &r).unwrap(), text, "writing the shipped definitions back changes nothing");
        // A changed colour is written, a removed per-face colour goes away,
        // and a dark colour keeps its fourth decimal.
        let mut c = r.colours();
        let g = r.id("grass").unwrap() as usize;
        c[g].top = Some([0.0153, 0.4, 0.1]);
        c[g].side = None;
        let changed = colours_into_text(&text, &c).unwrap();
        let back = Registry::parse(&changed).unwrap();
        let bc = back.def(g as BlockId).colours();
        assert!((bc.top.unwrap()[0] - 0.0153).abs() < 1e-6 && bc.side.is_none() && bc.bottom == r.def(g as BlockId).bottom);
        assert!(changed.contains("\"top\": [0.0153, 0.400, 0.100]"));
    }

    /// M3b: blocks.json with textures round-trips. A block with one texture
    /// for every face and another with its own top and sides read back to
    /// the same textures, the file stays in its style, writing it twice
    /// changes nothing, and taking a texture away removes its key.
    #[test]
    fn blocks_file_round_trip_with_textures() {
        let text = std::fs::read_to_string(SHIPPED).unwrap();
        let r = Registry::parse(&text).unwrap();
        let mut t = r.textures();
        let (stone, grass) = (r.id("stone").unwrap() as usize, r.id("grass").unwrap() as usize);
        t[stone].all = Some("examples/stone.png".into());
        t[grass] = BlockTextures { all: None, top: Some("grass_top.png".into()), side: Some("examples/dirt.png".into()), bottom: None };
        let once = look_into_text(&text, &r.colours(), Some(&t)).unwrap();
        let back = Registry::parse(&once).unwrap();
        assert_eq!(back.textures(), t);
        assert_eq!(back.def(grass as BlockId).textures().faces(), [Some("grass_top.png"), Some("examples/dirt.png"), None]);
        assert!(back.same_blocks(&r));
        let twice = look_into_text(&once, &back.colours(), Some(&back.textures())).unwrap();
        assert_eq!(once, twice, "writing is idempotent");
        assert!(once.contains("\"name\": \"stone\",") && once.contains("\"texture\": \"examples/stone.png\""), "{once}");
        assert_eq!(once.lines().filter(|l| l.trim_start().starts_with("{ \"id\":")).count(), r.len());
        // Colours untouched, comments kept.
        for (a, b) in r.defs().iter().zip(back.defs()) {
            assert_eq!(a.colours(), b.colours());
            assert_eq!(a.comment, b.comment);
        }
        // Back to flat: the keys go and the file is the shipped one again.
        let flat = look_into_text(&once, &r.colours(), Some(&r.textures())).unwrap();
        assert_eq!(flat, text);
        // A texture outside the folder is refused on reading.
        assert!(Registry::parse(&once.replace("examples/stone.png", "../stone.png")).is_err());
    }

    /// M3b: adding, renaming, falling and hiding. Ids are append-only; a
    /// rename keeps the old name as an alias; the file written from the
    /// registry reads back to the same blocks, and a later merge of that
    /// file into a registry loaded before the changes reproduces them.
    #[test]
    fn add_rename_fall_hide_and_write() {
        let text = std::fs::read_to_string(SHIPPED).unwrap();
        let r = Registry::parse(&text).unwrap();
        let before = Registry::parse(&text).unwrap();
        let n = r.len();
        let brick = r.add(BlockDef::new_solid("brick", BlockColours::flat([0.4, 0.1, 0.08]), false)).unwrap();
        assert_eq!(brick as usize, n, "a new block takes the next id");
        assert!(r.is_solid(brick) && !r.has_gravity(brick));
        assert_eq!(r.colour_table().len(), (n + 1) * 3);
        assert!(r.add(BlockDef::new_solid("brick", BlockColours::flat([0.0; 3]), false)).unwrap_err().contains("already"));
        assert!(r.add(BlockDef::new_solid("Bad Name", BlockColours::flat([0.0; 3]), false)).is_err());
        assert_eq!(name_from_label("  Red Brick! "), "red_brick");
        r.rename(brick, "red_brick").unwrap();
        assert_eq!(r.id("red_brick").unwrap(), brick);
        assert_eq!(r.id("brick").unwrap(), brick, "the old name still finds it");
        assert_eq!(r.def(brick).first_name(), "brick");
        assert!(r.rename(brick, "stone").is_err());
        r.set_falls(brick, true).unwrap();
        assert!(r.has_gravity(brick));
        assert!(r.set_falls(0, true).is_err(), "air cannot fall");
        r.set_hidden(r.id("marker_red").unwrap(), true).unwrap();
        // Hiding never moves the default hotbar (the play scripts press 1-9).
        let default = r.default_hotbar();
        r.set_hidden(1, true).unwrap();
        assert_eq!(r.default_hotbar(), default);
        r.set_hidden(1, false).unwrap();
        r.set_hotbar_names(Some(vec!["red_brick".into(), "stone".into(), "water".into()]));

        let written = defs_into_text(&text, &r).unwrap();
        assert_eq!(defs_into_text(&written, &r).unwrap(), written, "idempotent");
        let back = Registry::parse(&written).unwrap();
        assert!(back.same_blocks(&r), "{written}");
        assert_eq!(back.saved_hotbar().unwrap().unwrap(), vec![brick, 1, 8]);
        assert!(written.contains("\"name\": \"red_brick\""));
        assert!(written.contains("\"was\": [\"brick\"]"));
        assert!(written.contains("\"hotbar\": [\"red_brick\",\"stone\",\"water\"]"));
        // Every other block's line is as shipped.
        for l in text.lines().filter(|l| l.contains("{ \"id\":") && !l.contains("marker_red")) {
            assert!(written.contains(l), "{l}");
        }
        // The file merged into a registry loaded before: the same blocks.
        let done = before.merge_file(&back).unwrap();
        assert!(before.same_blocks(&r), "{done:?}");
        assert_eq!(before.id("brick").unwrap(), brick);
        // A renumbered file and a shortened one are refused.
        let renumbered = Registry::parse(&written.replace("\"name\": \"red_brick\", ", "\"name\": \"glass\", ").replace("\"was\": [\"brick\"]", "\"hidden\": false")).unwrap();
        assert!(r.merge_file(&renumbered).unwrap_err().contains("never renumber"));
        assert!(r.merge_file(&Registry::parse(&text).unwrap()).unwrap_err().contains("never deleted"));
    }
}

//! Block definitions, read from `assets/blocks.json` at startup.
//!
//! Colours are the one part of a block that may change while the game runs
//! (Andrew's look panel, M3a, and hot reload of the file). They live behind
//! a lock here and reach the GPU as a table of three colours per id (top,
//! sides, bottom), so a colour change rewrites that small table and nothing
//! is remeshed. Every other field is fixed for the life of the registry.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::RwLock;

pub type BlockId = u16;

#[derive(Clone, Debug, Deserialize)]
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
    /// The block's own `_comment` in the file.
    #[serde(default, rename = "_comment")]
    pub comment: Option<String>,
}

impl BlockDef {
    /// This block's colours as the file gives them.
    pub fn colours(&self) -> BlockColours {
        BlockColours { color: self.color, top: self.top, side: self.side, bottom: self.bottom }
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
    /// The colour each face is drawn with: [top, sides, bottom].
    pub fn faces(&self) -> [[f32; 3]; 3] {
        [self.top.unwrap_or(self.color), self.side.unwrap_or(self.color), self.bottom.unwrap_or(self.color)]
    }
    /// Does any face have a colour of its own?
    pub fn per_face(&self) -> bool {
        self.top.is_some() || self.side.is_some() || self.bottom.is_some()
    }
}

/// Entries per id in the GPU colour table: top, sides, bottom (the shader
/// reads `colours[id * 3 + face]`).
pub const FACES_PER_BLOCK: usize = 3;

#[derive(Deserialize)]
struct File {
    blocks: Vec<BlockDef>,
}

/// Every block the world can contain, indexed by id.
pub struct Registry {
    defs: Vec<BlockDef>,
    by_name: HashMap<String, BlockId>,
    solid: Vec<bool>,
    liquid: Vec<bool>,
    gravity: Vec<bool>,
    replaceable: Vec<bool>,
    /// The live colours, one per id. `defs` keeps the colours the registry
    /// was loaded with; the look panel and hot reload change these.
    colours: RwLock<Vec<BlockColours>>,
}

impl Registry {
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::parse(&text).map_err(|e| format!("{path}: {e}"))
    }

    /// Parse the text of a blocks file.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|e| format!("parse: {e}"))?;
        Self::from_defs(file.blocks)
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
            if by_name.insert(d.name.clone(), d.id).is_some() {
                return Err(format!("duplicate block name {}", d.name));
            }
            dense[id] = Some(d);
        }
        let defs: Vec<BlockDef> = dense
            .into_iter()
            .enumerate()
            .map(|(i, d)| {
                d.unwrap_or(BlockDef {
                    id: i as BlockId,
                    name: format!("unused_{i}"),
                    color: [1.0, 0.0, 1.0],
                    top: None,
                    side: None,
                    bottom: None,
                    solid: false,
                    liquid: false,
                    gravity: false,
                    replaceable: false,
                    comment: None,
                })
            })
            .collect();
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
        for d in &defs {
            let c = d.colours();
            for (face, v) in [("color", Some(c.color)), ("top", c.top), ("side", c.side), ("bottom", c.bottom)] {
                if v.is_some_and(|v| v.iter().any(|x| !x.is_finite() || *x < 0.0)) {
                    return Err(format!("block {}: \"{face}\" must be three finite numbers of at least 0", d.name));
                }
            }
        }
        let solid = defs.iter().map(|d| d.solid).collect();
        let liquid = defs.iter().map(|d| d.liquid).collect();
        let gravity = defs.iter().map(|d| d.gravity).collect();
        let replaceable = defs.iter().map(|d| d.replaceable).collect();
        let colours = RwLock::new(defs.iter().map(|d| d.colours()).collect());
        Ok(Self { defs, by_name, solid, liquid, gravity, replaceable, colours })
    }

    pub fn id(&self, name: &str) -> Result<BlockId, String> {
        self.by_name.get(name).copied().ok_or_else(|| format!("no block named {name}"))
    }
    pub fn def(&self, id: BlockId) -> &BlockDef {
        &self.defs[id as usize]
    }
    pub fn len(&self) -> usize {
        self.defs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
    #[inline]
    pub fn is_solid(&self, id: BlockId) -> bool {
        self.solid[id as usize]
    }
    #[inline]
    pub fn is_liquid(&self, id: BlockId) -> bool {
        self.liquid[id as usize]
    }
    /// Does this block fall when unsupported (`"gravity": true` in blocks.json)?
    #[inline]
    pub fn has_gravity(&self, id: BlockId) -> bool {
        self.gravity[id as usize]
    }
    /// May a falling block land in or pass through a cell holding this block?
    #[inline]
    pub fn is_replaceable(&self, id: BlockId) -> bool {
        self.replaceable[id as usize]
    }
    pub fn defs(&self) -> &[BlockDef] {
        &self.defs
    }
    /// Colour table for the GPU from the live colours: three RGBA per id
    /// (top, sides, bottom).
    pub fn colour_table(&self) -> Vec<[f32; 4]> {
        let colours = self.colours.read().unwrap();
        colours.iter().flat_map(|c| c.faces().map(|f| [f[0], f[1], f[2], 1.0])).collect()
    }
    /// The live colours of every id.
    pub fn colours(&self) -> Vec<BlockColours> {
        self.colours.read().unwrap().clone()
    }
    /// A block's live base colour (the hotbar swatch).
    pub fn colour(&self, id: BlockId) -> [f32; 3] {
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
    /// The first eight solid blocks by id and then the first liquid (water on
    /// key 9, M2b): what keys 1-9 place unless `--hotbar` says otherwise.
    pub fn default_hotbar(&self) -> Vec<BlockId> {
        let mut hb: Vec<BlockId> = self.defs.iter().filter(|d| d.solid && !d.name.starts_with("unused_")).map(|d| d.id).take(8).collect();
        if let Some(l) = self.defs.iter().find(|d| d.liquid) {
            hb.push(l.id);
        }
        hb
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
    /// only colours differ? Hot reload applies colours alone; anything
    /// else needs a restart.
    pub fn same_blocks(&self, other: &Registry) -> bool {
        self.defs.len() == other.defs.len()
            && self.defs.iter().zip(&other.defs).all(|(a, b)| {
                a.id == b.id && a.name == b.name && a.solid == b.solid && a.liquid == b.liquid && a.gravity == b.gravity && a.replaceable == b.replaceable
            })
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

/// The blocks file's text in its own style: the top-level comment, then one
/// line per block in id order with the ids and names in aligned columns.
/// Keys keep the order id, name, colours, flags, then anything else.
/// Lines end with `nl` (the file's own line ending: git checks text out
/// with CRLF on Windows).
pub fn format_blocks_file(top: &Map<String, Value>, blocks: &[Map<String, Value>], nl: &str) -> String {
    const ORDER: [&str; 10] = ["id", "name", "color", "top", "side", "bottom", "solid", "liquid", "gravity", "replaceable"];
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

/// Write the live colours into the blocks file at `path`, keeping its
/// style, its stable ids and every other field as it is on disk. Written
/// beside the file and renamed over it, so a crash leaves the old file.
pub fn save_colours(path: &str, colours: &[BlockColours]) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let new = colours_into_text(&text, colours).map_err(|e| format!("{path}: {e}"))?;
    let tmp = format!("{path}.saving");
    std::fs::write(&tmp, new).map_err(|e| format!("write {tmp}: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {path}: {e}"))
}

/// The blocks file `text` with its colours replaced by `colours` (by id).
pub fn colours_into_text(text: &str, colours: &[BlockColours]) -> Result<String, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("parse: {e}"))?;
    let top = doc.as_object().ok_or("not a JSON object")?;
    let mut blocks: Vec<Map<String, Value>> = top.get("blocks").and_then(|b| b.as_array()).ok_or("no \"blocks\" list")?.iter().map(|b| b.as_object().cloned().unwrap_or_default()).collect();
    blocks.sort_by_key(|b| b.get("id").and_then(|v| v.as_u64()).unwrap_or(u64::MAX));
    let arr = |c: [f32; 3]| Value::Array(c.iter().map(|&x| serde_json::Number::from_f64(x as f64).map_or(Value::Null, Value::Number)).collect());
    for b in blocks.iter_mut() {
        let Some(c) = b.get("id").and_then(|v| v.as_u64()).and_then(|id| colours.get(id as usize)) else { continue };
        b.insert("color".into(), arr(c.color));
        for (k, v) in [("top", c.top), ("side", c.side), ("bottom", c.bottom)] {
            match v {
                Some(v) => b.insert(k.into(), arr(v)),
                None => b.remove(k),
            };
        }
    }
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    Ok(format_blocks_file(top, &blocks, nl))
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
        assert!(err(defs).contains("\"top\" must be"));
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
        assert_eq!(t2[g * 3], [0.5, 0.25, 0.125, 1.0]);
        assert_eq!(t2.iter().enumerate().filter(|(n, x)| **x != table[*n]).count(), 1);
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
}

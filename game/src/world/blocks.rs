//! Block definitions, read from `assets/blocks.json` at startup.

use serde::Deserialize;
use std::collections::HashMap;

pub type BlockId = u16;

#[derive(Clone, Debug, Deserialize)]
pub struct BlockDef {
    pub id: BlockId,
    pub name: String,
    pub color: [f32; 3],
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
}

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
}

impl Registry {
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        let file: File = serde_json::from_str(&text).map_err(|e| format!("parse {path}: {e}"))?;
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
                    solid: false,
                    liquid: false,
                    gravity: false,
                    replaceable: false,
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
        let solid = defs.iter().map(|d| d.solid).collect();
        let liquid = defs.iter().map(|d| d.liquid).collect();
        let gravity = defs.iter().map(|d| d.gravity).collect();
        let replaceable = defs.iter().map(|d| d.replaceable).collect();
        Ok(Self { defs, by_name, solid, liquid, gravity, replaceable })
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
    /// Colour table for the GPU: one RGBA per id.
    pub fn colour_table(&self) -> Vec<[f32; 4]> {
        self.defs.iter().map(|d| [d.color[0], d.color[1], d.color[2], 1.0]).collect()
    }
    /// The first nine solid blocks by id: what keys 1-9 place unless `--hotbar` says otherwise.
    pub fn default_hotbar(&self) -> Vec<BlockId> {
        self.defs.iter().filter(|d| d.solid && !d.name.starts_with("unused_")).map(|d| d.id).take(9).collect()
    }
    /// Resolve a comma-separated list of block names (for `--hotbar`).
    pub fn hotbar_from(&self, names: &str) -> Result<Vec<BlockId>, String> {
        let ids: Vec<BlockId> = names.split(',').map(|n| self.id(n.trim())).collect::<Result<_, _>>()?;
        if ids.is_empty() || ids.len() > 9 {
            return Err(format!("the hotbar takes 1 to 9 blocks, got {}", ids.len()));
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_shipped_definitions() {
        let r = Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap();
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
        let mut defs: Vec<BlockDef> = Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap().defs().to_vec();
        let err = |d: Vec<BlockDef>| Registry::from_defs(d).err().expect("refused");
        defs[8].solid = true; // water
        assert!(err(defs.clone()).contains("replaceable but solid"));
        defs[8].solid = false;
        defs[8].gravity = true;
        assert!(err(defs.clone()).contains("gravity but is not solid"));
        defs[8].gravity = false;
        defs[0].replaceable = false;
        assert!(err(defs).contains("air) must be replaceable"));
    }
}

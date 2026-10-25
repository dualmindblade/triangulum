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
                })
            })
            .collect();
        if defs.is_empty() || defs[0].name != "air" || defs[0].solid {
            return Err("block 0 must be 'air' and not solid".into());
        }
        let solid = defs.iter().map(|d| d.solid).collect();
        let liquid = defs.iter().map(|d| d.liquid).collect();
        Ok(Self { defs, by_name, solid, liquid })
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
    pub fn defs(&self) -> &[BlockDef] {
        &self.defs
    }
    /// Colour table for the GPU: one RGBA per id.
    pub fn colour_table(&self) -> Vec<[f32; 4]> {
        self.defs.iter().map(|d| [d.color[0], d.color[1], d.color[2], 1.0]).collect()
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
    }
}

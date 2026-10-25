//! Saves: only the chunks that changed, whatever changed them.
//!
//! Layout of `game/saves/<world>/`:
//!   header.json             the manifest: format version, generator
//!                           version, body id, the world's identity (seed,
//!                           block size, H, sea level, N, vertical scale,
//!                           roughness), the projection and its table's
//!                           SHA-256, the bake's SHA-256, and the block-id
//!                           table
//!   chunks/b{B}_f{F}_{X}_{Y}_{Z}.chunk   one LOD-0 chunk each: magic, key,
//!                           then `Chunk::to_bytes`
//!
//! A save whose manifest does not match the running world is refused with
//! a clear message rather than loaded into the wrong lattice or the wrong
//! terrain. Only the world's display name may differ (it warns).

use super::chunk::VOXELS;
use super::edits::EditStore;
use super::{BlockId, Chunk, Registry};
use crate::grid::{ChunkKey, WorldConfig};
pub use crate::grid::BODY_NEISOR;
use crate::sha256::{Sha256, hex, sha256};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const FORMAT_VERSION: u32 = 2;

/// Bump when the generator's output changes (worldgen/ is versioned here
/// because saves, not generation, care about it). A save made with another
/// generator version is refused: its edits sit in terrain that no longer
/// exists around them.
pub const GENERATOR_VERSION: u32 = 2; // 2: landforms v2 (ridges, gullies, valleys, mesas)

const CHUNK_MAGIC: &[u8; 4] = b"TRC2";

/// The projection table `crates/cubesphere` ships, hashed here so the
/// manifest records exactly which grid the lattice was built on.
const EQUAL_AREA_COBE_TABLE: &[u8] = include_bytes!("../../../crates/cubesphere/data/equal_area_cobe.bin");
/// The digest the crate documents in its README and FINDINGS.
pub const DOCUMENTED_TABLE_SHA256: &str = "f99d69da39ad7cdcb55f7396d29c3544de0914c60d64ce1846a5b41aaba81ee5";

pub fn projection_table_sha256() -> &'static str {
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST.get_or_init(|| hex(&sha256(EQUAL_AREA_COBE_TABLE)))
}

/// The bake's identity: SHA-256 over meta.json and the name and size of
/// every face raster (hashing 550 MB of rasters each start would cost
/// seconds; a changed bake changes meta.json or the sizes).
pub fn bake_identity(dir: &str) -> Result<String, String> {
    let dir = Path::new(dir);
    let meta = dir.join("meta.json");
    let bytes = std::fs::read(&meta).map_err(|e| format!("read {}: {e}", meta.display()))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    for face in 0..6 {
        let p = dir.join(format!("face_{face}.bin"));
        let len = std::fs::metadata(&p).map_err(|e| format!("stat {}: {e}", p.display()))?.len();
        h.update(format!("face_{face}.bin").as_bytes());
        h.update(&len.to_le_bytes());
    }
    Ok(hex(&h.finish()))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Identity {
    pub name: String,
    pub seed: u64,
    pub block_size_m: f64,
    pub world_height: u32,
    pub sea_level: u32,
    pub columns_per_face: u32,
    pub vertical_scale: Option<f64>,
    pub roughness: f64,
    /// Landform strength dial; absent in version-1 headers.
    #[serde(default = "default_landform")]
    pub landform: f64,
}

fn default_landform() -> f64 {
    1.0
}

impl Identity {
    pub fn of(config: &WorldConfig) -> Self {
        Self {
            name: config.name.clone(),
            seed: config.seed,
            block_size_m: config.block_size_m,
            world_height: config.world_height,
            sea_level: config.sea_level,
            columns_per_face: config.columns_per_face,
            vertical_scale: config.vertical_scale,
            roughness: config.roughness,
            landform: config.landform,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    pub format_version: u32,
    pub generator_version: u32,
    pub body: u8,
    pub world: Identity,
    pub projection: String,
    pub projection_table_sha256: String,
    pub bake_sha256: String,
    /// (id, name) for every block the registry knew when the save was written.
    pub blocks: Vec<(BlockId, String)>,
}

impl Header {
    pub fn for_world(config: &WorldConfig, registry: &Registry, bake_sha256: &str) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            generator_version: GENERATOR_VERSION,
            body: BODY_NEISOR,
            world: Identity::of(config),
            projection: "EqualAreaCobe".into(),
            projection_table_sha256: projection_table_sha256().to_string(),
            bake_sha256: bake_sha256.to_string(),
            blocks: registry.defs().iter().filter(|d| !d.name.starts_with("unused_")).map(|d| (d.id, d.name.clone())).collect(),
        }
    }

    /// Ok(warnings) when this save may be loaded into `config`; Err(reasons) when it must not.
    pub fn check(&self, config: &WorldConfig, registry: &Registry, bake_sha256: &str) -> Result<Vec<String>, String> {
        if self.format_version != FORMAT_VERSION {
            return Err(format!("save format version {} but this build reads version {}", self.format_version, FORMAT_VERSION));
        }
        let want = Identity::of(config);
        let have = &self.world;
        let mut bad = Vec::new();
        if self.body != BODY_NEISOR {
            bad.push(format!("body {} vs {}", self.body, BODY_NEISOR));
        }
        if have.seed != want.seed {
            bad.push(format!("seed {} vs {}", have.seed, want.seed));
        }
        if have.block_size_m != want.block_size_m {
            bad.push(format!("block size {} m vs {} m", have.block_size_m, want.block_size_m));
        }
        if have.world_height != want.world_height {
            bad.push(format!("world height H {} vs {}", have.world_height, want.world_height));
        }
        if have.sea_level != want.sea_level {
            bad.push(format!("sea level {} vs {}", have.sea_level, want.sea_level));
        }
        if have.columns_per_face != want.columns_per_face {
            bad.push(format!("columns per face N {} vs {}", have.columns_per_face, want.columns_per_face));
        }
        if have.vertical_scale != want.vertical_scale {
            bad.push(format!("vertical scale {:?} vs {:?}", have.vertical_scale, want.vertical_scale));
        }
        if have.roughness != want.roughness {
            bad.push(format!("roughness {} vs {}", have.roughness, want.roughness));
        }
        if have.landform != want.landform {
            bad.push(format!("landform {} vs {}", have.landform, want.landform));
        }
        if self.generator_version != GENERATOR_VERSION {
            bad.push(format!("generator version {} vs {}", self.generator_version, GENERATOR_VERSION));
        }
        if self.projection != "EqualAreaCobe" || self.projection_table_sha256 != projection_table_sha256() {
            bad.push(format!("projection {} table {}.. vs EqualAreaCobe table {}..", self.projection, &self.projection_table_sha256[..self.projection_table_sha256.len().min(12)], &projection_table_sha256()[..12]));
        }
        if self.bake_sha256 != bake_sha256 {
            bad.push(format!("bake {}.. vs {}..", &self.bake_sha256[..self.bake_sha256.len().min(12)], &bake_sha256[..bake_sha256.len().min(12)]));
        }
        for (id, name) in &self.blocks {
            let cur = registry.defs().get(*id as usize).map(|d| d.name.as_str());
            if cur != Some(name.as_str()) {
                bad.push(format!("block id {id} is '{name}' in the save but '{}' in blocks.json (ids are stable: append, never renumber)", cur.unwrap_or("missing")));
            }
        }
        if !bad.is_empty() {
            return Err(format!("the save was made for a different world ({}); start with the saved settings or point --save at another directory", bad.join("; ")));
        }
        let mut warnings = Vec::new();
        if have.name != want.name {
            warnings.push(format!("world name '{}' in the save, '{}' now", have.name, want.name));
        }
        Ok(warnings)
    }
}

/// Compress dense ids for storage; the water layer is full wherever the
/// block is water (M2's levels replace this rule).
pub fn chunk_of_dense(dense: &[BlockId], water_id: BlockId) -> Chunk {
    if dense.iter().any(|&b| b == water_id) {
        let water: Vec<u8> = dense.iter().map(|&b| if b == water_id { 255 } else { 0 }).collect();
        Chunk::from_dense(dense, Some(&water))
    } else {
        Chunk::from_dense(dense, None)
    }
}

#[derive(Debug)]
pub struct LoadReport {
    pub chunks: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SaveDir {
    pub path: PathBuf,
}

impl SaveDir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn header_path(&self) -> PathBuf {
        self.path.join("header.json")
    }

    pub fn chunks_dir(&self) -> PathBuf {
        self.path.join("chunks")
    }

    pub fn exists(&self) -> bool {
        self.header_path().exists()
    }

    pub fn read_header(&self) -> Result<Option<Header>, String> {
        let p = self.header_path();
        if !p.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
        serde_json::from_str(&text).map(Some).map_err(|e| format!("parse {}: {e}", p.display()))
    }

    pub fn write_header(&self, config: &WorldConfig, registry: &Registry, bake_sha256: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.path).map_err(|e| format!("create {}: {e}", self.path.display()))?;
        let h = Header::for_world(config, registry, bake_sha256);
        let text = serde_json::to_string_pretty(&h).map_err(|e| e.to_string())?;
        write_atomic(&self.header_path(), text.as_bytes())
    }

    fn chunk_path(&self, key: ChunkKey) -> PathBuf {
        self.chunks_dir().join(format!("b{}_f{}_{}_{}_{}.chunk", key.body, key.face, key.cx, key.cy, key.cz))
    }

    pub fn write_chunk(&self, key: ChunkKey, chunk: &Chunk) -> Result<(), String> {
        assert_eq!(key.lod, 0, "only LOD-0 chunks are saved");
        assert_eq!(key.body, BODY_NEISOR, "a Neisor save holds Neisor chunks");
        let dir = self.chunks_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(CHUNK_MAGIC);
        bytes.push(key.body);
        bytes.push(key.face);
        bytes.push(key.lod);
        bytes.extend_from_slice(&key.cx.to_le_bytes());
        bytes.extend_from_slice(&key.cy.to_le_bytes());
        bytes.extend_from_slice(&key.cz.to_le_bytes());
        bytes.extend_from_slice(&chunk.to_bytes());
        write_atomic(&self.chunk_path(key), &bytes)
    }

    pub fn read_chunks(&self) -> Result<Vec<(ChunkKey, Chunk)>, String> {
        let dir = self.chunks_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("list {}: {e}", dir.display()))? {
            let p = entry.map_err(|e| e.to_string())?.path();
            if p.extension().and_then(|e| e.to_str()) != Some("chunk") {
                continue;
            }
            let bytes = std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
            out.push(parse_chunk_file(&bytes).map_err(|e| format!("{}: {e}", p.display()))?);
        }
        out.sort_by_key(|(k, _)| *k);
        Ok(out)
    }

    /// Load every saved chunk into `store` after checking the manifest.
    /// A missing save is an empty one.
    pub fn load_into(&self, config: &WorldConfig, registry: &Registry, bake_sha256: &str, store: &EditStore) -> Result<LoadReport, String> {
        let Some(header) = self.read_header()? else {
            return Ok(LoadReport { chunks: 0, warnings: Vec::new() });
        };
        let warnings = header.check(config, registry, bake_sha256).map_err(|e| format!("refusing to load {}: {e}", self.path.display()))?;
        let chunks = self.read_chunks()?;
        let n = chunks.len();
        for (key, chunk) in chunks {
            store.insert_chunk(key, chunk.to_dense(), false);
        }
        Ok(LoadReport { chunks: n, warnings })
    }

    /// Write the manifest (if missing) and every dirty chunk. Returns how many chunks were written.
    pub fn flush(&self, store: &EditStore, config: &WorldConfig, registry: &Registry, bake_sha256: &str, water_id: BlockId) -> Result<usize, String> {
        let dirty = store.take_dirty();
        if dirty.is_empty() {
            return Ok(0);
        }
        if !self.exists() {
            self.write_header(config, registry, bake_sha256)?;
        }
        for (key, dense) in &dirty {
            self.write_chunk(*key, &chunk_of_dense(dense, water_id))?;
        }
        Ok(dirty.len())
    }
}

fn parse_chunk_file(bytes: &[u8]) -> Result<(ChunkKey, Chunk), String> {
    const HEAD: usize = 4 + 3 + 12;
    if bytes.len() < HEAD || &bytes[..4] != CHUNK_MAGIC {
        return Err("not a chunk file".into());
    }
    if bytes[4] != BODY_NEISOR {
        return Err(format!("chunk of body {} in a body-{} save", bytes[4], BODY_NEISOR));
    }
    let u = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let key = ChunkKey { body: bytes[4], face: bytes[5], lod: bytes[6], cx: u(7), cy: u(11), cz: u(15) };
    if key.lod != 0 || key.face > 5 {
        return Err(format!("bad chunk key {key:?}"));
    }
    let chunk = Chunk::from_bytes(&bytes[HEAD..])?;
    debug_assert_eq!(chunk.to_dense().len(), VOXELS);
    Ok((key, chunk))
}

/// Write via a temporary file and rename, so a crash mid-write never leaves a torn file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("replace {}: {e}", path.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::BlockDef;

    const BAKE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn config() -> WorldConfig {
        WorldConfig { name: "test".into(), seed: 42, block_size_m: 1.0, world_height: 2048, sea_level: 512, columns_per_face: 131072, bake: String::new(), vertical_scale: None, roughness: 1.0, landform: 1.0 }
    }

    fn registry() -> Registry {
        Registry::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/blocks.json")).unwrap()
    }

    fn temp_dir(tag: &str) -> SaveDir {
        let p = std::env::temp_dir().join(format!("triangulum-save-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        SaveDir::new(p)
    }

    #[test]
    fn the_projection_table_is_the_documented_one() {
        assert_eq!(projection_table_sha256(), DOCUMENTED_TABLE_SHA256, "crates/cubesphere/data/equal_area_cobe.bin changed; update DOCUMENTED_TABLE_SHA256 and the crate's docs together");
    }

    #[test]
    fn bake_identity_follows_meta_and_sizes() {
        let dir = std::env::temp_dir().join(format!("triangulum-bake-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(bake_identity(dir.to_str().unwrap()).unwrap_err().contains("meta.json"));
        std::fs::write(dir.join("meta.json"), b"{\"seed\": 42}").unwrap();
        for f in 0..6 {
            std::fs::write(dir.join(format!("face_{f}.bin")), vec![0u8; 100 + f]).unwrap();
        }
        let a = bake_identity(dir.to_str().unwrap()).unwrap();
        std::fs::write(dir.join("face_3.bin"), vec![0u8; 999]).unwrap();
        let b = bake_identity(dir.to_str().unwrap()).unwrap();
        assert_ne!(a, b, "a raster of another size is another bake");
        std::fs::write(dir.join("face_3.bin"), vec![1u8; 103]).unwrap();
        assert_eq!(bake_identity(dir.to_str().unwrap()).unwrap(), a, "contents of equal size are not hashed (by design)");
        std::fs::write(dir.join("meta.json"), b"{\"seed\": 43}").unwrap();
        assert_ne!(bake_identity(dir.to_str().unwrap()).unwrap(), a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trip_only_the_changed_chunks() {
        let dir = temp_dir("rt");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        let key = ChunkKey::neisor(2, 0, 100, 200, 17);
        let key2 = ChunkKey::neisor(4, 0, 1, 2, 3);
        let natural = || vec![1u16; VOXELS];
        assert!(store.set(key, 5, 6, 7, 0, natural));
        assert!(store.set(key, 5, 6, 8, 8, natural)); // water
        assert!(store.set(key2, 31, 31, 31, 16, natural));
        assert!(!store.set(key2, 0, 0, 0, 1, natural)); // no change
        let written = dir.flush(&store, &cfg, &reg, BAKE, 8).unwrap();
        assert_eq!(written, 2);
        assert_eq!(dir.flush(&store, &cfg, &reg, BAKE, 8).unwrap(), 0, "nothing dirty after a flush");
        assert_eq!(std::fs::read_dir(dir.chunks_dir()).unwrap().count(), 2, "one file per changed chunk");

        let back = EditStore::new();
        let report = dir.load_into(&cfg, &reg, BAKE, &back).unwrap();
        assert_eq!(report.chunks, 2);
        assert!(report.warnings.is_empty());
        assert_eq!(back.snapshot(), store.snapshot());
        assert_eq!(back.get(key, 5, 6, 8), Some(8));
        // The stored chunk carries its water layer.
        let chunks = dir.read_chunks().unwrap();
        let (k, c) = chunks.iter().find(|(k, _)| *k == key).unwrap();
        assert_eq!(*k, key);
        assert_eq!(c.water_level(5, 6, 8), 255);
        assert_eq!(c.water_level(5, 6, 7), 0);
        // A second edit after the flush writes only that chunk again.
        assert!(back.set(key2, 0, 0, 0, 2, natural));
        assert_eq!(dir.flush(&back, &cfg, &reg, BAKE, 8).unwrap(), 1);
        let header = dir.read_header().unwrap().unwrap();
        assert_eq!(header.projection_table_sha256, DOCUMENTED_TABLE_SHA256);
        assert_eq!(header.bake_sha256, BAKE);
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    #[test]
    fn mismatched_manifest_is_refused_with_the_reasons() {
        let dir = temp_dir("mismatch");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        store.set(ChunkKey::neisor(0, 0, 0, 0, 0), 0, 0, 0, 0, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, 8).unwrap();
        let header = dir.read_header().unwrap().unwrap();
        assert_eq!(header.format_version, FORMAT_VERSION);
        assert!(header.blocks.iter().any(|(id, n)| *id == 8 && n == "water"));

        let mut half = cfg.clone();
        half.block_size_m = 0.5;
        half.columns_per_face = 262144;
        let err = dir.load_into(&half, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("block size 1 m vs 0.5 m"), "{err}");
        assert!(err.contains("columns per face N 131072 vs 262144"), "{err}");
        assert!(err.contains("refusing to load"), "{err}");

        let mut tall = cfg.clone();
        tall.world_height = 4096;
        assert!(dir.load_into(&tall, &reg, BAKE, &EditStore::new()).unwrap_err().contains("world height H 2048 vs 4096"));

        let mut other_seed = cfg.clone();
        other_seed.seed = 7;
        assert!(dir.load_into(&other_seed, &reg, BAKE, &EditStore::new()).unwrap_err().contains("seed 42 vs 7"));

        let mut scaled = cfg.clone();
        scaled.vertical_scale = Some(0.125);
        assert!(dir.load_into(&scaled, &reg, BAKE, &EditStore::new()).unwrap_err().contains("vertical scale"));

        let other_bake = "ffff".repeat(16);
        assert!(dir.load_into(&cfg, &reg, &other_bake, &EditStore::new()).unwrap_err().contains("bake "));

        // A renumbered block table is refused too.
        let mut defs: Vec<BlockDef> = reg.defs().to_vec();
        defs.swap(1, 2);
        defs[1].id = 1;
        defs[2].id = 2;
        let renamed = Registry::from_defs(defs).unwrap();
        let err = dir.load_into(&cfg, &renamed, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("block id 1 is 'stone' in the save but 'dirt'"), "{err}");

        // Another generator version or projection table is refused.
        let mut h = header.clone();
        h.generator_version += 1;
        assert!(h.check(&cfg, &reg, BAKE).unwrap_err().contains("generator version"));
        let mut h = header.clone();
        h.projection_table_sha256 = "deadbeef".repeat(8);
        assert!(h.check(&cfg, &reg, BAKE).unwrap_err().contains("projection"));

        // Only the display name may differ.
        let mut renamed_world = cfg.clone();
        renamed_world.name = "Neisor II".into();
        let report = dir.load_into(&renamed_world, &reg, BAKE, &EditStore::new()).unwrap();
        assert_eq!(report.chunks, 1);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    #[test]
    fn missing_save_is_empty_and_damaged_files_are_reported() {
        let dir = temp_dir("missing");
        let report = dir.load_into(&config(), &registry(), BAKE, &EditStore::new()).unwrap();
        assert_eq!(report.chunks, 0);
        dir.write_header(&config(), &registry(), BAKE).unwrap();
        std::fs::create_dir_all(dir.chunks_dir()).unwrap();
        std::fs::write(dir.chunks_dir().join("b0_f0_0_0_0.chunk"), b"garbage").unwrap();
        let err = dir.load_into(&config(), &registry(), BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("not a chunk file"), "{err}");
        let _ = std::fs::remove_dir_all(&dir.path);
    }
}

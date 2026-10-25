//! Saves: only the chunks that changed, whatever changed them, committed
//! all-or-nothing.
//!
//! Layout of `game/saves/<world>/`:
//!   header.json             the world's identity: format version, generator
//!                           version, body id, seed, block size, H, sea
//!                           level, N, vertical scale, roughness, the
//!                           projection and its table's SHA-256, the bake's
//!                           content SHA-256, and the block-id table
//!   commit.json             the current generation: the file that holds
//!                           each chunk, the simulation state file, and the
//!                           SHA-256 of every one of them. Written last, by
//!                           rename, so a save is one snapshot: a crash
//!                           before the switch leaves the previous
//!                           generation whole (review #1)
//!   chunks/b{B}_f{F}_{X}_{Y}_{Z}.g{G}.chunk
//!                           one LOD-0 chunk each, written in generation G:
//!                           magic, key, then `Chunk::to_bytes` (ids and
//!                           water levels)
//!   sim.g{G}.bin            the simulation: tick, the block-update queue,
//!                           frozen updates and blocks in flight
//!                           (`sim::Snapshot::to_bytes`)
//!
//! Files `commit.json` does not name are leftovers of an interrupted save;
//! loading ignores them and the next save removes the ones it supersedes.
//! Every file is verified against its recorded digest, every chunk key
//! against the world's bounds and every palette id against the registry
//! before anything is installed (review #14). Dirty chunks are acknowledged
//! to the store only once the commit is on disk (review #2).
//!
//! A save whose header does not match the running world is refused with a
//! clear message rather than loaded into the wrong lattice or the wrong
//! terrain. Only the world's display name may differ (it warns).

use super::chunk::VOXELS;
use super::edits::EditStore;
use super::{BlockId, Chunk, Registry};
pub use crate::grid::BODY_NEISOR;
use crate::grid::{ChunkKey, FastMap, Planet, WorldConfig};
use crate::sha256::{Sha256, hex, sha256};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 3: generation commits, per-file digests, stored water levels and the
/// simulation state (M2a). A version-2 save has none of these, so it is
/// refused like any other mismatch rather than silently resumed.
pub const FORMAT_VERSION: u32 = 3;

/// Bump when the generator's output changes (worldgen/ is versioned here
/// because saves, not generation, care about it). A save made with another
/// generator version is refused: its edits sit in terrain that no longer
/// exists around them.
pub const GENERATOR_VERSION: u32 = 3; // 2: landforms v2 (ridges, gullies, valleys, mesas); 3: lakes and rivers (hydro.bin)

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

/// One raster of the bake as the digest cache remembers it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BakeFile {
    name: String,
    len: u64,
    mtime_ns: u64,
}

/// `content.sha256.json` beside the bake: the content digest and what it
/// was computed from, so a start-up only re-hashes 550 MB when a raster
/// changed size or time.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct BakeDigestCache {
    meta_sha256: String,
    files: Vec<BakeFile>,
    digest: String,
}

/// The bake's identity: SHA-256 over meta.json and the full contents of
/// every face raster (review #4: sizes and names alone let a same-size bake
/// with other values pass), and over `hydro.bin` (rivers and lakes) when the
/// bake has one. Hashing the rasters is cached beside the bake, keyed on the
/// rasters' names, sizes and modification times (the cache holds the
/// rasters' digest only, so engines that do not read hydro.bin share it); a
/// bake copied elsewhere is hashed once more and gets the same digest.
pub fn bake_identity(dir: &str) -> Result<String, String> {
    let rasters = raster_identity(dir)?;
    let hydro = Path::new(dir).join("hydro.bin");
    match std::fs::read(&hydro) {
        Ok(bytes) => {
            let mut h = Sha256::new();
            h.update(rasters.as_bytes());
            h.update(b"hydro.bin");
            h.update(&sha256(&bytes));
            Ok(hex(&h.finish()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(rasters),
        Err(e) => Err(format!("read {}: {e}", hydro.display())),
    }
}

fn raster_identity(dir: &str) -> Result<String, String> {
    let dir = Path::new(dir);
    let meta_path = dir.join("meta.json");
    let meta = std::fs::read(&meta_path).map_err(|e| format!("read {}: {e}", meta_path.display()))?;
    let meta_sha256 = hex(&sha256(&meta));
    let mut files = Vec::new();
    for face in 0..6 {
        let name = format!("face_{face}.bin");
        let md = std::fs::metadata(dir.join(&name)).map_err(|e| format!("stat {}: {e}", dir.join(&name).display()))?;
        let mtime_ns = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
        files.push(BakeFile { name, len: md.len(), mtime_ns });
    }
    let cache_path = dir.join("content.sha256.json");
    if let Ok(text) = std::fs::read_to_string(&cache_path) {
        if let Ok(c) = serde_json::from_str::<BakeDigestCache>(&text) {
            if c.meta_sha256 == meta_sha256 && c.files == files && c.digest.len() == 64 {
                return Ok(c.digest);
            }
        }
    }
    let mut h = Sha256::new();
    h.update(&meta);
    let mut buf = vec![0u8; 8 << 20];
    for f in &files {
        h.update(f.name.as_bytes());
        h.update(&f.len.to_le_bytes());
        let p = dir.join(&f.name);
        let mut file = std::fs::File::open(&p).map_err(|e| format!("open {}: {e}", p.display()))?;
        loop {
            let n = file.read(&mut buf).map_err(|e| format!("read {}: {e}", p.display()))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    let digest = hex(&h.finish());
    // Best effort: an unwritable bake directory just hashes again next time.
    if let Ok(text) = serde_json::to_string_pretty(&BakeDigestCache { meta_sha256, files, digest: digest.clone() }) {
        let _ = std::fs::write(&cache_path, text);
    }
    Ok(digest)
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

/// Compress dense ids for a chunk that carries no stored levels (coarse
/// LODs): the water layer is full wherever the block is water.
pub fn chunk_of_dense(dense: &[BlockId], water_id: BlockId) -> Chunk {
    if dense.iter().any(|&b| b == water_id) {
        let water: Vec<u8> = dense.iter().map(|&b| if b == water_id { 255 } else { 0 }).collect();
        Chunk::from_dense(dense, Some(&water))
    } else {
        Chunk::from_dense(dense, None)
    }
}

/// A chunk's place in the current generation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitEntry {
    /// body, face, cx, cy, cz (LOD 0).
    pub key: [u32; 5],
    pub file: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitFile {
    pub file: String,
    pub sha256: String,
}

/// `commit.json`: one whole snapshot of the save.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Commit {
    pub generation: u64,
    pub chunks: Vec<CommitEntry>,
    pub sim: Option<CommitFile>,
}

fn key_array(key: ChunkKey) -> [u32; 5] {
    [key.body as u32, key.face as u32, key.cx, key.cy, key.cz]
}

#[derive(Debug)]
pub struct LoadReport {
    pub chunks: usize,
    pub warnings: Vec<String>,
    pub generation: u64,
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

    pub fn commit_path(&self) -> PathBuf {
        self.path.join("commit.json")
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

    /// The current generation; None when nothing was ever committed.
    pub fn read_commit(&self) -> Result<Option<Commit>, String> {
        let p = self.commit_path();
        if !p.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
        serde_json::from_str(&text).map(Some).map_err(|e| format!("parse {}: {e}", p.display()))
    }

    fn chunk_file(key: ChunkKey, generation: u64) -> String {
        format!("chunks/b{}_f{}_{}_{}_{}.g{generation}.chunk", key.body, key.face, key.cx, key.cy, key.cz)
    }

    /// Read a committed file and check its digest.
    fn read_verified(&self, file: &str, sha256_hex: &str) -> Result<Vec<u8>, String> {
        let p = self.path.join(file);
        let bytes = std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
        let got = hex(&sha256(&bytes));
        if got != sha256_hex {
            return Err(format!("{}: digest {}.. but commit.json records {}.. (damaged or tampered)", p.display(), &got[..12], &sha256_hex[..sha256_hex.len().min(12)]));
        }
        Ok(bytes)
    }

    /// The simulation state as committed (`sim::Snapshot::from_bytes` reads
    /// it); None when the save has none.
    pub fn read_sim(&self) -> Result<Option<Vec<u8>>, String> {
        match self.read_commit()?.and_then(|c| c.sim) {
            Some(f) => self.read_verified(&f.file, &f.sha256).map(Some),
            None => Ok(None),
        }
    }

    pub fn sim_path(&self) -> PathBuf {
        self.commit_path()
    }

    /// Every committed chunk, verified and sorted by key.
    pub fn read_chunks(&self) -> Result<Vec<(ChunkKey, Chunk)>, String> {
        let Some(commit) = self.read_commit()? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::with_capacity(commit.chunks.len());
        for e in &commit.chunks {
            let bytes = self.read_verified(&e.file, &e.sha256)?;
            let (key, chunk) = parse_chunk_file(&bytes).map_err(|err| format!("{}: {err}", self.path.join(&e.file).display()))?;
            if key_array(key) != e.key {
                return Err(format!("{}: holds chunk {:?} but commit.json lists it as {:?}", self.path.join(&e.file).display(), key_array(key), e.key));
            }
            out.push((key, chunk));
        }
        out.sort_by_key(|(k, _)| *k);
        Ok(out)
    }

    /// Load every committed chunk into `store` after checking the header,
    /// each file's digest, each key's bounds and each palette id. A missing
    /// save is an empty one.
    pub fn load_into(&self, config: &WorldConfig, registry: &Registry, bake_sha256: &str, store: &EditStore) -> Result<LoadReport, String> {
        let Some(header) = self.read_header()? else {
            return Ok(LoadReport { chunks: 0, warnings: Vec::new(), generation: 0 });
        };
        let warnings = header.check(config, registry, bake_sha256).map_err(|e| format!("refusing to load {}: {e}", self.path.display()))?;
        let generation = self.read_commit()?.map_or(0, |c| c.generation);
        let planet = Planet::try_new(config.block_size_m, config.world_height, config.sea_level, config.columns_per_face)?;
        let (per_edge, per_col) = (planet.chunks_per_edge(0), planet.chunks_per_column(0));
        let chunks = self.read_chunks()?;
        let mut seen = std::collections::HashSet::new();
        for (key, chunk) in &chunks {
            if key.cx >= per_edge || key.cy >= per_edge || key.cz >= per_col {
                return Err(format!("refusing to load {}: chunk {key:?} lies outside the world ({per_edge} chunks per edge, {per_col} per column)", self.path.display()));
            }
            if !seen.insert(*key) {
                return Err(format!("refusing to load {}: chunk {key:?} is listed twice", self.path.display()));
            }
            if let Some(&id) = chunk.palette().iter().find(|&&id| id as usize >= registry.len()) {
                return Err(format!("refusing to load {}: chunk {key:?} holds block id {id}, unknown to blocks.json ({} blocks)", self.path.display(), registry.len()));
            }
        }
        let n = chunks.len();
        for (key, chunk) in chunks {
            store.insert_chunk(key, chunk.to_dense(), chunk.water_dense(), false);
        }
        Ok(LoadReport { chunks: n, warnings, generation })
    }

    /// Write one generation: the header (if missing), every dirty chunk
    /// and, when given, the simulation state, each to a new file; then
    /// switch `commit.json` to the new generation in one rename; then
    /// acknowledge the written revisions to the store and remove the files
    /// the new generation superseded. Returns how many chunks were
    /// written. Nothing is created when there is nothing to write.
    pub fn flush(&self, store: &EditStore, config: &WorldConfig, registry: &Registry, bake_sha256: &str, sim: Option<&[u8]>) -> Result<usize, String> {
        let dirty = store.dirty_snapshot();
        if dirty.is_empty() && sim.is_none() {
            return Ok(0);
        }
        if !self.exists() {
            self.write_header(config, registry, bake_sha256)?;
        }
        let dir = self.chunks_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let mut commit = self.read_commit()?.unwrap_or_default();
        let generation = commit.generation + 1;
        let mut index: FastMap<[u32; 5], usize> = commit.chunks.iter().enumerate().map(|(i, e)| (e.key, i)).collect();
        let mut superseded = Vec::new();
        let mut acked = Vec::with_capacity(dirty.len());
        for d in &dirty {
            assert_eq!(d.key.lod, 0, "only LOD-0 chunks are saved");
            assert_eq!(d.key.body, BODY_NEISOR, "a Neisor save holds Neisor chunks");
            let file = Self::chunk_file(d.key, generation);
            let bytes = chunk_file_bytes(d.key, &Chunk::from_dense(&d.ids, d.water.as_deref()));
            write_atomic(&self.path.join(&file), &bytes)?;
            let entry = CommitEntry { key: key_array(d.key), file, sha256: hex(&sha256(&bytes)) };
            match index.get(&entry.key) {
                Some(&i) => {
                    superseded.push(std::mem::replace(&mut commit.chunks[i], entry).file);
                }
                None => {
                    index.insert(entry.key, commit.chunks.len());
                    commit.chunks.push(entry);
                }
            }
            acked.push((d.key, d.rev));
        }
        if let Some(bytes) = sim {
            let file = format!("sim.g{generation}.bin");
            write_atomic(&self.path.join(&file), bytes)?;
            if let Some(old) = commit.sim.replace(CommitFile { file, sha256: hex(&sha256(bytes)) }) {
                superseded.push(old.file);
            }
        }
        commit.generation = generation;
        commit.chunks.sort_by_key(|e| e.key);
        let text = serde_json::to_string_pretty(&commit).map_err(|e| e.to_string())?;
        write_atomic(&self.commit_path(), text.as_bytes())?;
        store.acknowledge(&acked);
        for f in superseded {
            let _ = std::fs::remove_file(self.path.join(f));
        }
        Ok(dirty.len())
    }
}

fn chunk_file_bytes(key: ChunkKey, chunk: &Chunk) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(CHUNK_MAGIC);
    bytes.push(key.body);
    bytes.push(key.face);
    bytes.push(key.lod);
    bytes.extend_from_slice(&key.cx.to_le_bytes());
    bytes.extend_from_slice(&key.cy.to_le_bytes());
    bytes.extend_from_slice(&key.cz.to_le_bytes());
    bytes.extend_from_slice(&chunk.to_bytes());
    bytes
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

/// Write via a temporary file, fsync it and rename it over the destination
/// (a rename replaces atomically on Windows and on Unix), so a crash never
/// leaves a torn or missing file. The directory is synced where the
/// platform allows it (Unix).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        f.write_all(bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        f.sync_all().map_err(|e| format!("sync {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
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

    /// Review #4: the identity follows the rasters' contents, with a cache
    /// that only skips the hashing while names, sizes and times are unchanged.
    #[test]
    fn bake_identity_follows_the_contents_and_caches_the_digest() {
        let dir = std::env::temp_dir().join(format!("triangulum-bake-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(bake_identity(dir.to_str().unwrap()).unwrap_err().contains("meta.json"));
        std::fs::write(dir.join("meta.json"), b"{\"seed\": 42}").unwrap();
        for f in 0..6 {
            std::fs::write(dir.join(format!("face_{f}.bin")), vec![0u8; 100 + f]).unwrap();
        }
        let a = bake_identity(dir.to_str().unwrap()).unwrap();
        assert_eq!(a.len(), 64);
        assert!(dir.join("content.sha256.json").exists(), "the digest is cached beside the bake");
        assert_eq!(bake_identity(dir.to_str().unwrap()).unwrap(), a, "the cache answers the same");
        std::fs::write(dir.join("face_3.bin"), vec![0u8; 999]).unwrap();
        let b = bake_identity(dir.to_str().unwrap()).unwrap();
        assert_ne!(a, b, "a raster of another size is another bake");
        std::fs::write(dir.join("face_3.bin"), vec![1u8; 103]).unwrap();
        let c = bake_identity(dir.to_str().unwrap()).unwrap();
        assert_ne!(c, a, "the same size with other contents is another bake");
        // Rewriting the same bytes (a new mtime) re-hashes to the same digest.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("face_3.bin"), vec![1u8; 103]).unwrap();
        assert_eq!(bake_identity(dir.to_str().unwrap()).unwrap(), c);
        // A stale cache file is ignored when the rasters do not match it.
        std::fs::write(dir.join("face_3.bin"), vec![0u8; 103]).unwrap();
        assert_ne!(bake_identity(dir.to_str().unwrap()).unwrap(), c);
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
        // Water, through the batch path with the liquid predicate (as World::commit does).
        assert_eq!(store.commit(&[crate::world::edits::Write { key, x: 5, y: 6, z: 8, id: 8, level: None }], |_| (natural(), None), |id| id == 8), vec![true]);
        assert!(store.set(key2, 31, 31, 31, 16, natural));
        assert!(!store.set(key2, 0, 0, 0, 1, natural)); // no change
        let written = dir.flush(&store, &cfg, &reg, BAKE, None).unwrap();
        assert_eq!(written, 2);
        assert_eq!(store.dirty_count(), 0, "acknowledged after the commit");
        assert_eq!(dir.flush(&store, &cfg, &reg, BAKE, None).unwrap(), 0, "nothing dirty after a flush");
        assert_eq!(std::fs::read_dir(dir.chunks_dir()).unwrap().count(), 2, "one file per changed chunk");
        let commit = dir.read_commit().unwrap().unwrap();
        assert_eq!(commit.generation, 1);
        assert_eq!(commit.chunks.len(), 2);
        assert!(commit.sim.is_none());

        let back = EditStore::new();
        let report = dir.load_into(&cfg, &reg, BAKE, &back).unwrap();
        assert_eq!(report.chunks, 2);
        assert_eq!(report.generation, 1);
        assert!(report.warnings.is_empty());
        assert_eq!(back.snapshot(), store.snapshot());
        assert_eq!(back.get(key, 5, 6, 8), Some(8));
        assert_eq!(back.water_level(key, 5, 6, 8), Some(255), "a placed water block is full");
        // The stored chunk carries its water layer.
        let chunks = dir.read_chunks().unwrap();
        let (k, c) = chunks.iter().find(|(k, _)| *k == key).unwrap();
        assert_eq!(*k, key);
        assert_eq!(c.water_level(5, 6, 8), 255);
        assert_eq!(c.water_level(5, 6, 7), 0);
        // A second edit after the flush writes only that chunk again, as generation 2,
        // and the superseded file goes.
        assert!(back.set(key2, 0, 0, 0, 2, natural));
        assert_eq!(dir.flush(&back, &cfg, &reg, BAKE, None).unwrap(), 1);
        let commit = dir.read_commit().unwrap().unwrap();
        assert_eq!(commit.generation, 2);
        assert!(commit.chunks.iter().any(|e| e.file.ends_with(".g2.chunk")) && commit.chunks.iter().any(|e| e.file.ends_with(".g1.chunk")));
        assert_eq!(std::fs::read_dir(dir.chunks_dir()).unwrap().count(), 2);
        let header = dir.read_header().unwrap().unwrap();
        assert_eq!(header.projection_table_sha256, DOCUMENTED_TABLE_SHA256);
        assert_eq!(header.bake_sha256, BAKE);
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    /// Review #15: water levels other than full survive a save and a reload.
    #[test]
    fn water_levels_round_trip_exactly() {
        let dir = temp_dir("levels");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        let key = ChunkKey::neisor(2, 0, 100, 200, 17);
        let w = |x: usize, level: Option<u8>| crate::world::edits::Write { key, x, y: 1, z: 2, id: 8, level };
        store.commit(&[w(0, Some(17)), w(1, Some(255)), w(2, Some(1))], |_| (vec![1u16; VOXELS], None), |id| id == 8);
        dir.flush(&store, &cfg, &reg, BAKE, None).unwrap();
        let back = EditStore::new();
        dir.load_into(&cfg, &reg, BAKE, &back).unwrap();
        assert_eq!(back.water_level(key, 0, 1, 2), Some(17));
        assert_eq!(back.water_level(key, 1, 1, 2), Some(255));
        assert_eq!(back.water_level(key, 2, 1, 2), Some(1));
        assert_eq!(back.water_level(key, 3, 1, 2), Some(0));
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    #[test]
    fn mismatched_manifest_is_refused_with_the_reasons() {
        let dir = temp_dir("mismatch");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        store.set(ChunkKey::neisor(0, 0, 0, 0, 0), 0, 0, 0, 0, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, None).unwrap();
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
        // A version-2 save (no commit, no simulation state) is refused by version alone.
        let mut h = header.clone();
        h.format_version = 2;
        let err = h.check(&cfg, &reg, BAKE).unwrap_err();
        assert!(err.contains("save format version 2 but this build reads version 3"), "{err}");
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    #[test]
    fn missing_save_is_empty_and_damaged_files_are_reported() {
        let dir = temp_dir("missing");
        let (cfg, reg) = (config(), registry());
        let report = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap();
        assert_eq!(report.chunks, 0);
        // A committed chunk file whose bytes changed fails its digest.
        let store = EditStore::new();
        store.set(ChunkKey::neisor(0, 0, 0, 0, 0), 0, 0, 0, 0, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, None).unwrap();
        let commit = dir.read_commit().unwrap().unwrap();
        let file = dir.path.join(&commit.chunks[0].file);
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[20] ^= 0xFF;
        std::fs::write(&file, &bytes).unwrap();
        let err = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("digest"), "{err}");
        // Garbage with a matching digest is still not a chunk.
        let garbage = b"garbage".to_vec();
        std::fs::write(&file, &garbage).unwrap();
        let mut c = commit.clone();
        c.chunks[0].sha256 = hex(&sha256(&garbage));
        std::fs::write(dir.commit_path(), serde_json::to_string(&c).unwrap()).unwrap();
        let err = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("not a chunk file"), "{err}");
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    /// Review #14: keys outside the world, duplicate keys and unknown block
    /// ids are refused before anything is installed.
    #[test]
    fn out_of_bounds_duplicate_and_unknown_id_chunks_are_refused() {
        let dir = temp_dir("validate");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        let good = ChunkKey::neisor(1, 0, 10, 20, 30);
        store.set(good, 0, 0, 0, 2, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, None).unwrap();
        let commit = dir.read_commit().unwrap().unwrap();
        let good_entry = commit.chunks[0].clone();
        let rewrite = |c: &Commit| std::fs::write(dir.commit_path(), serde_json::to_string(c).unwrap()).unwrap();
        // Out of bounds: a chunk file whose key is past the face edge (N / 32 = 4096 chunks per edge).
        let far = ChunkKey::neisor(1, 0, 4096, 0, 0);
        let bytes = chunk_file_bytes(far, &Chunk::uniform(2));
        std::fs::write(dir.path.join("chunks/far.chunk"), &bytes).unwrap();
        let mut c = commit.clone();
        c.chunks = vec![CommitEntry { key: key_array(far), file: "chunks/far.chunk".into(), sha256: hex(&sha256(&bytes)) }];
        rewrite(&c);
        let err = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("outside the world"), "{err}");
        // Duplicate: the same chunk listed twice.
        let mut c = commit.clone();
        c.chunks = vec![good_entry.clone(), good_entry.clone()];
        rewrite(&c);
        let err = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("listed twice"), "{err}");
        // Unknown id: a palette entry past the registry.
        let bytes = chunk_file_bytes(good, &Chunk::uniform(65535));
        std::fs::write(dir.path.join("chunks/bad-id.chunk"), &bytes).unwrap();
        let mut c = commit.clone();
        c.chunks = vec![CommitEntry { key: key_array(good), file: "chunks/bad-id.chunk".into(), sha256: hex(&sha256(&bytes)) }];
        rewrite(&c);
        let err = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("block id 65535"), "{err}");
        // A file whose embedded key disagrees with the manifest.
        let mut c = commit.clone();
        c.chunks = vec![CommitEntry { key: key_array(ChunkKey::neisor(1, 0, 11, 20, 30)), ..good_entry.clone() }];
        rewrite(&c);
        let err = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("commit.json lists it as"), "{err}");
        // Restored, it loads.
        rewrite(&commit);
        assert_eq!(dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap().chunks, 1);
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    /// Review #1: a save is one snapshot. Files written for a generation
    /// whose commit never landed are invisible, and the previous generation
    /// loads whole; the next successful save cleans them up.
    #[test]
    fn an_interrupted_save_leaves_the_previous_generation_whole() {
        let dir = temp_dir("interrupted");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        let a = ChunkKey::neisor(2, 0, 100, 200, 17);
        let b = ChunkKey::neisor(2, 0, 100, 200, 16);
        store.set(a, 0, 0, 0, 4, || vec![1; VOXELS]);
        store.set(b, 0, 0, 0, 5, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, Some(b"sim-1")).unwrap();
        let before = dir.read_commit().unwrap().unwrap();
        // "Generation 2" crashes after writing its files but before the switch.
        for (k, id) in [(a, 0u16), (b, 0u16)] {
            std::fs::write(dir.path.join(SaveDir::chunk_file(k, 2)), chunk_file_bytes(k, &Chunk::uniform(id))).unwrap();
        }
        std::fs::write(dir.path.join("sim.g2.bin"), b"sim-2").unwrap();
        std::fs::write(dir.path.join("commit.tmp"), b"{ torn").unwrap();
        assert_eq!(std::fs::read_dir(dir.chunks_dir()).unwrap().count(), 4);
        let back = EditStore::new();
        assert_eq!(dir.load_into(&cfg, &reg, BAKE, &back).unwrap().generation, 1);
        assert_eq!(back.get(a, 0, 0, 0), Some(4), "generation 1's contents");
        assert_eq!(back.get(b, 0, 0, 0), Some(5));
        assert_eq!(dir.read_sim().unwrap().as_deref(), Some(&b"sim-1"[..]));
        // The next real save becomes generation 2 and supersedes the leftovers by name.
        store.set(a, 0, 0, 1, 6, || vec![1; VOXELS]);
        assert_eq!(dir.flush(&store, &cfg, &reg, BAKE, Some(b"sim-3")).unwrap(), 1);
        let after = dir.read_commit().unwrap().unwrap();
        assert_eq!(after.generation, 2);
        assert_eq!(after.chunks.len(), 2);
        assert_eq!(dir.read_sim().unwrap().as_deref(), Some(&b"sim-3"[..]));
        let back = EditStore::new();
        dir.load_into(&cfg, &reg, BAKE, &back).unwrap();
        assert_eq!(back.get(a, 0, 0, 1), Some(6));
        assert_eq!(back.get(b, 0, 0, 0), Some(5), "b is still generation 1's file");
        assert!(after.chunks.iter().any(|e| e.file == before.chunks.iter().find(|e| e.key == key_array(b)).unwrap().file));
        assert!(!dir.path.join("sim.g1.bin").exists(), "the superseded sim file is removed");
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    #[test]
    fn the_simulation_state_rides_along_with_a_flush() {
        let dir = temp_dir("sim");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        assert_eq!(dir.read_sim().unwrap(), None);
        // Nothing dirty and no state: nothing is created.
        assert_eq!(dir.flush(&store, &cfg, &reg, BAKE, None).unwrap(), 0);
        assert!(!dir.exists());
        // State alone writes the header, the commit and the state file.
        assert_eq!(dir.flush(&store, &cfg, &reg, BAKE, Some(b"state-1")).unwrap(), 0);
        assert!(dir.exists());
        assert_eq!(dir.read_sim().unwrap().as_deref(), Some(&b"state-1"[..]));
        store.set(ChunkKey::neisor(2, 0, 100, 200, 17), 5, 6, 7, 0, || vec![1u16; VOXELS]);
        assert_eq!(dir.flush(&store, &cfg, &reg, BAKE, Some(b"state-2")).unwrap(), 1);
        assert_eq!(dir.read_sim().unwrap().as_deref(), Some(&b"state-2"[..]));
        assert!(!dir.path.join("sim.g1.bin").exists() && dir.path.join("sim.g2.bin").exists());
        // A damaged state file is refused.
        std::fs::write(dir.path.join("sim.g2.bin"), b"state-X").unwrap();
        assert!(dir.read_sim().unwrap_err().contains("digest"));
        let _ = std::fs::remove_dir_all(&dir.path);
    }
}

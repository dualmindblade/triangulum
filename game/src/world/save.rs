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
//!
//! Durability (review 3 #1, `flush`): a generation's chunk and simulation
//! files have names no committed generation uses, so they are written in
//! place and flushed to the device (`FlushFileBuffers` / `fsync`); then
//! their directories are flushed; then `commit.json` is published by a
//! durable replace (Windows: `MoveFileExW` with `MOVEFILE_WRITE_THROUGH`,
//! which returns only once the rename is on the disk; Unix: `rename` and an
//! `fsync` of the directory) and the save directory flushed. Only then are
//! the revisions acknowledged and the previous generation's superseded
//! files removed, so until the new generation is durable the previous one
//! stays whole on disk. Any flush failure is an error (the save is retried;
//! nothing is acknowledged or removed). All of this goes through `SaveFs`,
//! so the tests check the order and every failure point.
//!
//! Every file is verified against its recorded digest, every chunk key
//! against the world's bounds and every palette id against the registry
//! before anything is installed (review #14). Dirty chunks are acknowledged
//! to the store only once the commit is on disk (review #2).
//! The session captures an owned, packed snapshot and gives it to one worker
//! (`save_worker.rs`); completion returns revisions to the game thread.
//!
//! A save whose header does not match the running world is refused with a
//! clear message rather than loaded into the wrong lattice or the wrong
//! terrain. Only the world's display name may differ (it warns).

use super::chunk::VOXELS;
use super::edits::EditStore;
use super::{BlockId, Chunk, Registry};
pub use crate::grid::BODY_NEISOR;
use crate::grid::{ChunkKey, FastMap, Planet, WorldConfig};
use crate::sha256::{hex, sha256};
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[path = "save_worker.rs"]
mod worker;
pub use worker::{SNAPSHOT_LIMIT, SaveSnapshot, SaveWorker};

/// 3: generation commits, per-file digests, stored water levels and the
/// simulation state (M2a). A version-2 save has none of these, so it is
/// refused like any other mismatch rather than silently resumed.
pub const FORMAT_VERSION: u32 = 3;

/// Bump when the generator's output changes (worldgen/ is versioned here
/// because saves, not generation, care about it). A save made with another
/// generator version is refused: its edits sit in terrain that no longer
/// exists around them.
pub const GENERATOR_VERSION: u32 = 4; // 2: landforms v2 (ridges, gullies, valleys, mesas); 3: lakes and rivers (hydro.bin); 4: coasts (shoreline from the bake's coast, beaches, sea and lake floors)

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

/// The bake's identity from its files as they are now: SHA-256 over
/// meta.json and the full contents of every face raster (review #4: sizes
/// and names alone let a same-size bake with other values pass), and over
/// `hydro.bin` when the bake has one (`worldgen::bake::BakeIdentity`). The
/// game itself takes the identity the loader hashed from the bytes it
/// loaded (`Bake::identity`, review 3 #5); this is for tools and tests.
/// Nothing is cached: the old cache beside the bake (content.sha256.json,
/// keyed on names, sizes and times) vouched for contents it never saw, and
/// is no longer read or written.
pub fn bake_identity(dir: &str) -> Result<String, String> {
    crate::worldgen::bake::content_identity(dir)
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
    /// Last writer's simulation algorithm, independent of the block flags.
    /// Old headers omitted it; their snapshot layout identifies the rules.
    #[serde(default)]
    pub sim_rules: Option<u32>,
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
            sim_rules: Some(crate::sim::RULES_VERSION),
            body: BODY_NEISOR,
            world: Identity::of(config),
            projection: "EqualAreaCobe".into(),
            projection_table_sha256: projection_table_sha256().to_string(),
            bake_sha256: bake_sha256.to_string(),
            blocks: registry.defs().iter().filter(|d| !d.name.starts_with("unused_")).map(|d| (d.id, d.first_name().to_string())).collect(),
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
        let old_terrain = self.generator_version != GENERATOR_VERSION;
        if old_terrain {
            bad.push(format!("generator version {} vs {}", self.generator_version, GENERATOR_VERSION));
        }
        if self.projection != "EqualAreaCobe" || self.projection_table_sha256 != projection_table_sha256() {
            bad.push(format!("projection {} table {}.. vs EqualAreaCobe table {}..", self.projection, &self.projection_table_sha256[..self.projection_table_sha256.len().min(12)], &projection_table_sha256()[..12]));
        }
        if self.bake_sha256 != bake_sha256 {
            bad.push(format!("bake {}.. vs {}..", &self.bake_sha256[..self.bake_sha256.len().min(12)], &bake_sha256[..bake_sha256.len().min(12)]));
        }
        // M3b: blocks.json may have grown since (new blocks append), and a
        // renamed block keeps its old name in "was", so it still answers.
        let defs = registry.defs();
        for (id, name) in &self.blocks {
            let cur = defs.get(*id as usize);
            if !cur.is_some_and(|d| d.answers_to(name)) {
                bad.push(format!(
                    "block id {id} is '{name}' in the save but '{}' in blocks.json (ids are stable: append, never renumber; a block renamed by hand keeps its old name in \"was\")",
                    cur.map_or("missing", |d| d.name.as_str())
                ));
            }
        }
        if !bad.is_empty() {
            // No setting brings an old generator back: say so, and that the save is safe.
            if old_terrain {
                return Err(format!(
                    "this save was made with an older version of the terrain ({}), so this build can't continue it: its edited chunks wouldn't fit the land around them. The save is untouched, and the build that made it still plays it. To play this build, start a new world by adding --save saves/another-name",
                    bad.join("; ")
                ));
            }
            return Err(format!("the save was made for a different world ({}); start with the saved settings or point --save at another directory", bad.join("; ")));
        }
        let mut warnings = Vec::new();
        if let Some(note) = crate::sim::check_saved_rules(self.sim_rules)? {
            warnings.push(note);
        }
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

impl Commit {
    /// A manifest is untrusted too, including when it arrived inside a replay.
    /// References must name this save's own generation files, never metadata
    /// or a path supplied by another save.
    fn validate(&self) -> Result<(), String> {
        let generation_of = |file: &str, prefix: &str, suffix: &str| -> bool {
            file.strip_prefix(prefix).and_then(|s| s.strip_suffix(suffix)).and_then(|s| s.parse::<u64>().ok())
                .is_some_and(|g| g > 0 && g <= self.generation && file == format!("{prefix}{g}{suffix}"))
        };
        if self.generation == u64::MAX {
            return Err("refusing commit.json: generation is exhausted".into());
        }
        let mut names = std::collections::HashSet::new();
        for e in &self.chunks {
            let [b, f, x, y, z] = e.key;
            let prefix = format!("chunks/b{b}_f{f}_{x}_{y}_{z}.g");
            if !names.insert(&e.file) {
                return Err(format!("refusing commit.json: {:?} is listed twice", e.file));
            }
            if !generation_of(&e.file, &prefix, ".chunk") {
                return Err(format!("refusing commit.json chunk reference {:?}", e.file));
            }
        }
        if let Some(e) = &self.sim {
            if !generation_of(&e.file, "sim.g", ".bin") {
                return Err(format!("refusing commit.json simulation reference {:?}", e.file));
            }
        }
        Ok(())
    }
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

    /// Is there a save here (review 3 #2)? Ok(false): no header and nothing
    /// committed, an absent save, which loads as the natural world. Ok(true):
    /// a header (with or without a commit yet). Err: a commit without its
    /// header. That save is damaged, not empty: loading it as an empty
    /// world would drop its chunks while its simulation state resumed, and
    /// the next save would write a fresh header over the old generation.
    pub fn present(&self) -> Result<bool, String> {
        if self.header_path().exists() {
            return Ok(true);
        }
        if self.commit_path().exists() {
            let tmp = self.path.join("header.tmp");
            let hint = if tmp.exists() { " (a header.tmp is there: a header write was interrupted)" } else { "" };
            return Err(format!(
                "refusing to use {}: commit.json is there but header.json is missing{hint}, so the save is damaged, not empty. Put back header.json from a backup, or move the directory away to start a new save",
                self.path.display()
            ));
        }
        Ok(false)
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
        self.write_header_with(&OsFs, config, registry, bake_sha256)
    }

    fn write_header_with(&self, fs: &dyn SaveFs, config: &WorldConfig, registry: &Registry, bake_sha256: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.path).map_err(|e| format!("create {}: {e}", self.path.display()))?;
        let h = Header::for_world(config, registry, bake_sha256);
        let text = serde_json::to_string_pretty(&h).map_err(|e| e.to_string())?;
        write_atomic(fs, &self.header_path(), text.as_bytes())?;
        sync_dir(fs, &self.path)
    }

    /// M3b: blocks added to blocks.json since the header was written are
    /// recorded in it (only the block table changes), so the header names
    /// every id a chunk of this save can hold.
    fn extend_header_blocks(&self, fs: &dyn SaveFs, registry: &Registry) -> Result<(), String> {
        let Some(mut h) = self.read_header()? else { return Ok(()) };
        let known: std::collections::HashSet<BlockId> = h.blocks.iter().map(|(id, _)| *id).collect();
        let new: Vec<(BlockId, String)> = registry.defs().iter().filter(|d| !known.contains(&d.id) && !d.name.starts_with("unused_")).map(|d| (d.id, d.first_name().to_string())).collect();
        if new.is_empty() && h.sim_rules == Some(crate::sim::RULES_VERSION) {
            return Ok(());
        }
        h.blocks.extend(new);
        h.sim_rules = Some(crate::sim::RULES_VERSION);
        let text = serde_json::to_string_pretty(&h).map_err(|e| e.to_string())?;
        write_atomic(fs, &self.header_path(), text.as_bytes())?;
        sync_dir(fs, &self.path)
    }

    /// The current generation; None when nothing was ever committed.
    pub fn read_commit(&self) -> Result<Option<Commit>, String> {
        let p = self.commit_path();
        if !p.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
        let commit: Commit = serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", p.display()))?;
        commit.validate()?;
        Ok(Some(commit))
    }

    fn chunk_file(key: ChunkKey, generation: u64) -> String {
        format!("chunks/b{}_f{}_{}_{}_{}.g{generation}.chunk", key.body, key.face, key.cx, key.cy, key.cz)
    }

    /// Read a committed file and check its digest.
    fn read_verified(&self, file: &str, sha256_hex: &str) -> Result<Vec<u8>, String> {
        let p = self.owned_file(file)?;
        let bytes = std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
        let got = hex(&sha256(&bytes));
        if got != sha256_hex {
            return Err(format!("{}: digest {}.. but commit.json records {}.. (damaged or tampered)", p.display(), &got[..12], &sha256_hex[..sha256_hex.len().min(12)]));
        }
        Ok(bytes)
    }

    fn owned_file(&self, file: &str) -> Result<PathBuf, String> {
        if !is_save_file_name(file) {
            return Err(format!("refusing save file reference {file:?}"));
        }
        let root = self.path.canonicalize().map_err(|e| format!("{}: {e}", self.path.display()))?;
        let path = self.path.join(file);
        let resolved = path.canonicalize().map_err(|e| format!("{}: {e}", path.display()))?;
        if !resolved.starts_with(&root) {
            return Err(format!("refusing save file outside {}: {file}", self.path.display()));
        }
        Ok(path)
    }

    /// The simulation state as committed (`sim::Snapshot::from_bytes` reads
    /// it); None when the save has none.
    pub fn read_sim(&self) -> Result<Option<Vec<u8>>, String> {
        if !self.present()? {
            return Ok(None);
        }
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
    /// each file's digest, each key's bounds and each palette id. An absent
    /// save is an empty one; a commit without its header is refused
    /// (`present`).
    pub fn load_into(&self, config: &WorldConfig, registry: &Registry, bake_sha256: &str, store: &EditStore) -> Result<LoadReport, String> {
        if !self.present()? {
            return Ok(LoadReport { chunks: 0, warnings: Vec::new(), generation: 0 });
        }
        let Some(header) = self.read_header()? else {
            return Err(format!("refusing to load {}: header.json disappeared while loading", self.path.display()));
        };
        let warnings = header.check(config, registry, bake_sha256).map_err(|e| format!("refusing to load {}: {e}", self.path.display()))?;
        let generation = self.read_commit()?.map_or(0, |c| c.generation);
        let planet = Planet::try_new(config.block_size_m, config.world_height, config.sea_level, config.columns_per_face)?;
        let (per_edge, per_col) = (planet.chunks_per_edge(0), planet.chunks_per_column(0));
        let chunks = self.read_chunks()?;
        let mut seen = std::collections::HashSet::new();
        let identities: std::collections::HashSet<BlockId> = header.blocks.iter().map(|(id, _)| *id).collect();
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
            if let Some(&id) = chunk.palette().iter().find(|id| !identities.contains(id)) {
                return Err(format!("refusing to load {}: chunk {key:?} uses block id {id} without a recorded identity in header.json; restore its original block table before loading", self.path.display()));
            }
        }
        let n = chunks.len();
        for (key, chunk) in chunks {
            store.insert_chunk(key, chunk.to_dense(), chunk.water_dense(), false);
        }
        Ok(LoadReport { chunks: n, warnings, generation })
    }

    /// Write one generation: the header (if missing), every dirty chunk
    /// and, when given, the simulation state, each to a new file, flushed;
    /// then their directories; then switch `commit.json` to the new
    /// generation in one durable replace; then remove superseded files and
    /// acknowledge the written revisions to the store (module doc,
    /// "Durability"). Returns how many chunks were
    /// written. Nothing is created when there is nothing to write.
    pub fn flush(&self, store: &EditStore, config: &WorldConfig, registry: &Registry, bake_sha256: &str, sim: Option<&[u8]>) -> Result<usize, String> {
        self.flush_with(&OsFs, store, config, registry, bake_sha256, sim)
    }

    pub(crate) fn flush_with(&self, fs: &dyn SaveFs, store: &EditStore, config: &WorldConfig, registry: &Registry, bake_sha256: &str, sim: Option<&[u8]>) -> Result<usize, String> {
        let (dirty, _) = store.save_snapshot(usize::MAX)?;
        let acked = self.flush_snapshot_with(fs, &dirty, config, registry, bake_sha256, sim)?;
        store.acknowledge(&acked);
        Ok(acked.len())
    }

    fn flush_snapshot_with(&self, fs: &dyn SaveFs, dirty: &[super::edits::SaveChunk], config: &WorldConfig, registry: &Registry, bake_sha256: &str, sim: Option<&[u8]>) -> Result<Vec<(ChunkKey, u64)>, String> {
        if dirty.is_empty() && sim.is_none() {
            return Ok(Vec::new());
        }
        // Validate before changing even the header, and before a reference
        // can reach superseded-file cleanup. Check filesystem links as well.
        let mut commit = self.read_commit()?.unwrap_or_default();
        for file in commit.chunks.iter().map(|e| &e.file).chain(commit.sim.iter().map(|e| &e.file)) {
            self.owned_file(file)?;
        }
        if !self.present()? {
            self.write_header_with(fs, config, registry, bake_sha256)?;
        } else {
            self.read_header()?.ok_or("save header disappeared")?.check(config, registry, bake_sha256)?;
            self.extend_header_blocks(fs, registry)?;
        }
        let dir = self.chunks_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let generation = commit.generation + 1;
        let mut index: FastMap<[u32; 5], usize> = commit.chunks.iter().enumerate().map(|(i, e)| (e.key, i)).collect();
        let mut superseded = Vec::new();
        let mut acked = Vec::with_capacity(dirty.len());
        for d in dirty {
            assert_eq!(d.key.lod, 0, "only LOD-0 chunks are saved");
            assert_eq!(d.key.body, BODY_NEISOR, "a Neisor save holds Neisor chunks");
            let file = Self::chunk_file(d.key, generation);
            let bytes = chunk_file_bytes(d.key, &Chunk::from_dense(&d.ids.to_dense(), d.water.as_deref()));
            // A name no committed generation uses: written in place.
            write_new(fs, &self.path.join(&file), &bytes)?;
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
            write_new(fs, &self.path.join(&file), bytes)?;
            if let Some(old) = commit.sim.replace(CommitFile { file, sha256: hex(&sha256(bytes)) }) {
                superseded.push(old.file);
            }
        }
        // The new generation's files and their names are on the device...
        if !dirty.is_empty() {
            sync_dir(fs, &dir)?;
        }
        sync_dir(fs, &self.path)?;
        // ...before the commit that names them is published, durably.
        commit.generation = generation;
        commit.chunks.sort_by_key(|e| e.key);
        let text = serde_json::to_string_pretty(&commit).map_err(|e| e.to_string())?;
        write_atomic(fs, &self.commit_path(), text.as_bytes())?;
        sync_dir(fs, &self.path)?;
        // Only now is the previous generation no longer needed.
        for f in superseded {
            // A file that will not go is garbage, not damage: loading ignores it.
            let _ = fs.remove(&self.path.join(f));
        }
        Ok(acked)
    }
}

/// A whole save as files, built in memory from every edited chunk and a
/// simulation state: generation 1 of a fresh save (`header.json`,
/// `commit.json`, one file per chunk, `sim.g1.bin`), byte for byte what
/// `flush` would write into an empty directory. A replay (M2c) embeds this
/// as its starting snapshot; nothing touches the disk.
pub fn snapshot_image(store: &EditStore, config: &WorldConfig, registry: &Registry, bake_sha256: &str, sim: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let generation = 1;
    let header = serde_json::to_string_pretty(&Header::for_world(config, registry, bake_sha256)).map_err(|e| e.to_string())?;
    let mut files = vec![("header.json".to_string(), header.into_bytes())];
    let mut commit = Commit { generation, chunks: Vec::new(), sim: None };
    for d in store.snapshot_full() {
        let file = SaveDir::chunk_file(d.key, generation);
        let bytes = chunk_file_bytes(d.key, &Chunk::from_dense(&d.ids, d.water.as_deref()));
        commit.chunks.push(CommitEntry { key: key_array(d.key), file: file.clone(), sha256: hex(&sha256(&bytes)) });
        files.push((file, bytes));
    }
    let sim_file = format!("sim.g{generation}.bin");
    commit.sim = Some(CommitFile { file: sim_file.clone(), sha256: hex(&sha256(sim)) });
    files.push((sim_file, sim.to_vec()));
    commit.chunks.sort_by_key(|e| e.key);
    let text = serde_json::to_string_pretty(&commit).map_err(|e| e.to_string())?;
    files.insert(1, ("commit.json".to_string(), text.into_bytes()));
    Ok(files)
}

/// Is `name` a file a save may hold (`header.json`, `commit.json`,
/// `sim.gN.bin`, `chunks/<name>.gN.chunk`)? Replays come from other
/// machines: an image is never allowed to name a path outside its directory.
pub fn is_save_file_name(name: &str) -> bool {
    let plain = |s: &str| s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.') && !s.contains("..");
    match name {
        "header.json" | "commit.json" => true,
        _ => match name.strip_prefix("chunks/") {
            Some(rest) => plain(rest) && rest.ends_with(".chunk"),
            None => plain(name) && name.starts_with("sim.g") && name.ends_with(".bin"),
        },
    }
}

impl SaveDir {
    /// Write a save image (`snapshot_image`) into this directory, which
    /// must not hold a save yet. Names are checked first; nothing outside
    /// the directory can be written. Loading then verifies every digest as
    /// for any save.
    pub fn install_image(&self, files: &[(String, Vec<u8>)]) -> Result<(), String> {
        if let Some((bad, _)) = files.iter().find(|(n, _)| !is_save_file_name(n)) {
            return Err(format!("refusing a save image with the file name {bad:?}"));
        }
        let mut image = std::collections::HashMap::new();
        for (name, bytes) in files {
            if image.insert(name.as_str(), bytes).is_some() {
                return Err(format!("refusing duplicate save image file {name:?}"));
            }
        }
        let header = image.get("header.json").ok_or("save image has no header.json")?;
        serde_json::from_slice::<Header>(header).map_err(|e| format!("save image header.json: {e}"))?;
        let commit: Commit = serde_json::from_slice(image.get("commit.json").ok_or("save image has no commit.json")?)
            .map_err(|e| format!("save image commit.json: {e}"))?;
        commit.validate()?;
        let committed: std::collections::HashSet<&str> = commit.chunks.iter().map(|e| e.file.as_str()).chain(commit.sim.iter().map(|e| e.file.as_str())).chain(["header.json", "commit.json"]).collect();
        if let Some((name, _)) = files.iter().find(|(name, _)| !committed.contains(name.as_str())) {
            return Err(format!("save image contains uncommitted file {name:?}"));
        }
        for (name, digest) in commit.chunks.iter().map(|e| (&e.file, &e.sha256)).chain(commit.sim.iter().map(|e| (&e.file, &e.sha256))) {
            let bytes = image.get(name.as_str()).ok_or_else(|| format!("save image omits committed file {name:?}"))?;
            if hex(&sha256(bytes)) != *digest {
                return Err(format!("save image file {name:?} has the wrong digest"));
            }
        }
        if self.exists() || self.commit_path().exists() {
            return Err(format!("{} already holds a save; a save image is only installed into an empty directory", self.path.display()));
        }
        if self.path.exists() && std::fs::read_dir(&self.path).map_err(|e| e.to_string())?.next().is_some() {
            return Err(format!("{} is not empty; refusing to install a save image", self.path.display()));
        }
        std::fs::create_dir_all(self.chunks_dir()).map_err(|e| format!("create {}: {e}", self.chunks_dir().display()))?;
        // commit.json last, as `flush` does.
        let fs = OsFs;
        for (name, bytes) in files.iter().filter(|(n, _)| n != "commit.json") {
            write_atomic(&fs, &self.path.join(name), bytes)?;
        }
        sync_dir(&fs, &self.chunks_dir())?;
        sync_dir(&fs, &self.path)?;
        for (name, bytes) in files.iter().filter(|(n, _)| n == "commit.json") {
            write_atomic(&fs, &self.path.join(name), bytes)?;
        }
        sync_dir(&fs, &self.path)
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

/// The file operations a save makes, so the tests can record their order
/// and fail any one of them (review 3 #1). `OsFs` is the real one.
pub(crate) trait SaveFs {
    /// Create or truncate `path`, write `bytes` and flush them to the device.
    fn write_synced(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;
    /// Replace `to` by `from` so that the rename is on the device when this
    /// returns (Windows: write-through; Unix: rename, then the directory).
    fn replace_durably(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    /// Flush a directory's entries to the device.
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()>;
    fn remove(&self, path: &Path) -> std::io::Result<()>;
}

/// The real file system.
pub(crate) struct OsFs;

impl SaveFs for OsFs {
    fn write_synced(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let mut f = std::fs::File::create(path)?;
        f.write_all(bytes)?;
        f.sync_all()
    }

    fn replace_durably(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            windows::move_write_through(from, to)
        }
        #[cfg(not(windows))]
        {
            std::fs::rename(from, to)?;
            match to.parent() {
                Some(dir) => self.sync_dir(dir),
                None => Ok(()),
            }
        }
    }

    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        #[cfg(windows)]
        {
            windows::flush_directory(dir)
        }
        #[cfg(not(windows))]
        {
            std::fs::File::open(dir)?.sync_all()
        }
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
}

#[cfg(windows)]
mod windows {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    /// "The function does not return until the file is actually moved on
    /// the disk" (the Win32 documentation of MoveFileExW).
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    /// Lets CreateFile open a directory.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub fn move_write_through(from: &Path, to: &Path) -> std::io::Result<()> {
        let (f, t) = (wide(from), wide(to));
        // SAFETY: two NUL-terminated UTF-16 paths that outlive the call.
        let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) };
        if ok == 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }

    /// FlushFileBuffers on the directory. It needs a handle with write
    /// access (a read-only handle is refused with "access denied"); NTFS
    /// takes it and commits the directory's entries.
    pub fn flush_directory(dir: &Path) -> std::io::Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).write(true).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(dir)?.sync_all()
    }
}

/// A file under a name nothing committed uses (a new generation's chunk or
/// simulation file): written in place and flushed. A crash leaves at most
/// a torn file that no commit names, overwritten by the next attempt.
fn write_new(fs: &dyn SaveFs, path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs.write_synced(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Replace a file that a reader may be using (`commit.json`, `header.json`)
/// all-or-nothing and durably: a temporary file, flushed, then a durable
/// replace (`SaveFs::replace_durably`). A crash leaves the old file or the
/// new one, never a torn or missing one, and when this returns the new one
/// is on the device.
fn write_atomic(fs: &dyn SaveFs, path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs.write_synced(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs.replace_durably(&tmp, path).map_err(|e| format!("replace {} by {}: {e}", path.display(), tmp.display()))
}

fn sync_dir(fs: &dyn SaveFs, dir: &Path) -> Result<(), String> {
    fs.sync_dir(dir).map_err(|e| format!("flush the directory {}: {e}", dir.display()))
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
    fn r4_image_manifest_cannot_reference_external_or_missing_files() {
        let image = snapshot_image(&EditStore::new(), &config(), &registry(), BAKE, b"state").unwrap();
        for (n, file) in ["../sim.g1.bin", "chunks/../../sim.g1.bin", "C:/sim.g1.bin", "C:\\sim.g1.bin", "sim.g9.bin", "header.json"].iter().enumerate() {
            let mut evil = image.clone();
            let bytes = &mut evil.iter_mut().find(|(n, _)| n == "commit.json").unwrap().1;
            let mut commit: Commit = serde_json::from_slice(bytes).unwrap();
            commit.sim.as_mut().unwrap().file = file.to_string();
            *bytes = serde_json::to_vec(&commit).unwrap();
            let dir = temp_dir(&format!("r4-escape-{n}"));
            assert!(dir.install_image(&evil).is_err(), "accepted manifest reference {file}");
            assert!(!dir.path.exists(), "validate before installing anything");
        }
        let mut missing = image.clone();
        missing.retain(|(name, _)| name != "sim.g1.bin");
        assert!(temp_dir("r4-missing").install_image(&missing).is_err());
        let mut duplicate = image.clone();
        duplicate.push(image[0].clone());
        assert!(temp_dir("r4-duplicate").install_image(&duplicate).is_err());
    }

    #[test]
    fn r4_manifest_is_checked_before_reading_or_flushing() {
        let dir = temp_dir("r4-direct");
        let (cfg, reg) = (config(), registry());
        dir.flush(&EditStore::new(), &cfg, &reg, BAKE, Some(b"state")).unwrap();
        let mut commit = dir.read_commit().unwrap().unwrap();
        commit.sim.as_mut().unwrap().file = "../outside.bin".into();
        let evil = serde_json::to_vec(&commit).unwrap();
        std::fs::write(dir.commit_path(), &evil).unwrap();
        assert!(dir.read_commit().is_err(), "an unsafe manifest must never reach a reader or cleanup");
        assert!(dir.read_sim().is_err());
        assert!(dir.flush(&EditStore::new(), &cfg, &reg, BAKE, Some(b"next")).is_err());
        assert_eq!(std::fs::read(dir.commit_path()).unwrap(), evil);
        std::fs::remove_dir_all(dir.path).unwrap();
    }

    #[test]
    fn r4_used_ids_require_a_saved_identity() {
        let dir = temp_dir("r4-identity");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        store.set(ChunkKey::neisor(2, 0, 100, 200, 17), 1, 2, 3, 4, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, None).unwrap();
        let mut header = dir.read_header().unwrap().unwrap();
        header.blocks.retain(|(id, _)| *id != 4);
        std::fs::write(dir.header_path(), serde_json::to_vec(&header).unwrap()).unwrap();
        let result = dir.load_into(&cfg, &reg, BAKE, &EditStore::new());
        assert!(result.is_err(), "a current numeric id cannot establish a missing saved identity");
        assert!(result.unwrap_err().contains("identity"));
        std::fs::remove_dir_all(dir.path).unwrap();
    }

    #[test]
    fn r4_header_extension_fills_id_gaps() {
        let dir = temp_dir("r4-gap");
        let (cfg, reg) = (config(), registry());
        let mut defs = reg.defs();
        let gap = defs.len() as u16;
        let mut later = BlockDef::new_solid("later", super::super::blocks::BlockColours::flat([0.2; 3]), false);
        later.id = gap + 1;
        defs.push(later);
        let sparse = Registry::from_defs(defs.clone()).unwrap();
        dir.flush(&EditStore::new(), &cfg, &sparse, BAKE, Some(b"state")).unwrap();
        let mut filled = BlockDef::new_solid("filled", super::super::blocks::BlockColours::flat([0.3; 3]), false);
        filled.id = gap;
        defs.push(filled);
        let filled = Registry::from_defs(defs).unwrap();
        let store = EditStore::new();
        store.set(ChunkKey::neisor(2, 0, 100, 200, 17), 1, 2, 3, gap, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &filled, BAKE, None).unwrap();
        assert!(dir.read_header().unwrap().unwrap().blocks.contains(&(gap, "filled".into())));
        dir.load_into(&cfg, &filled, BAKE, &EditStore::new()).unwrap();
        std::fs::remove_dir_all(dir.path).unwrap();
    }

    #[test]
    fn r4_falling_ids_require_a_saved_identity() {
        let dir = temp_dir("r4-falling-identity");
        let (cfg, reg) = (config(), registry());
        let snap = crate::sim::Snapshot {
            falling: vec![crate::sim::Falling { id: 4, face: 2, i: 100, j: 200, z: 612.0, vel: -1.0 }],
            ..Default::default()
        };
        dir.flush(&EditStore::new(), &cfg, &reg, BAKE, Some(&snap.to_bytes())).unwrap();
        let mut header = dir.read_header().unwrap().unwrap();
        header.blocks.retain(|(id, _)| *id != 4);
        std::fs::write(dir.header_path(), serde_json::to_vec(&header).unwrap()).unwrap();
        assert!(crate::session::Session::load_sim(Some(&dir), crate::sim::SimConfig::default(), &reg).is_err());
        std::fs::remove_dir_all(dir.path).unwrap();
    }

    #[test]
    fn r4_save_records_and_checks_simulation_algorithm_version() {
        let (cfg, reg) = (config(), registry());
        let mut json = serde_json::to_value(Header::for_world(&cfg, &reg, BAKE)).unwrap();
        assert_eq!(json["sim_rules"], crate::sim::RULES_VERSION);
        json["sim_rules"] = serde_json::json!(crate::sim::RULES_VERSION + 1);
        let future: Header = serde_json::from_value(json.clone()).unwrap();
        assert!(future.check(&cfg, &reg, BAKE).unwrap_err().contains("simulation"));
        for old in [serde_json::Value::Null, serde_json::json!(1), serde_json::json!(2)] {
            json["sim_rules"] = old;
            let legacy: Header = serde_json::from_value(json.clone()).unwrap();
            assert!(!legacy.check(&cfg, &reg, BAKE).unwrap().is_empty(), "safe continuation is explicit");
        }
    }

    #[test]
    fn the_projection_table_is_the_documented_one() {
        assert_eq!(projection_table_sha256(), DOCUMENTED_TABLE_SHA256, "crates/cubesphere/data/equal_area_cobe.bin changed; update DOCUMENTED_TABLE_SHA256 and the crate's docs together");
    }

    /// Review #4: the identity follows the rasters' contents, with a cache
    /// that only skips the hashing while names, sizes and times are unchanged.
    #[test]
    fn bake_identity_follows_the_contents() {
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
        assert!(!dir.join("content.sha256.json").exists(), "nothing is cached beside the bake (review 3 #5)");
        assert_eq!(bake_identity(dir.to_str().unwrap()).unwrap(), a, "the same files, the same identity");
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
        std::fs::write(dir.join("face_3.bin"), vec![0u8; 103]).unwrap();
        assert_ne!(bake_identity(dir.to_str().unwrap()).unwrap(), c);
        std::fs::write(dir.join("meta.json"), b"{\"seed\": 43}").unwrap();
        assert_ne!(bake_identity(dir.to_str().unwrap()).unwrap(), a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review 3 #5: the cache may not vouch for contents it cannot see. A
    /// raster rewritten with other bytes of the same size, its modification
    /// time put back, is another bake.
    #[test]
    fn bake_identity_follows_contents_even_when_size_and_time_match() {
        let dir = std::env::temp_dir().join(format!("triangulum-bake-same-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.json"), b"{\"seed\": 42}").unwrap();
        for f in 0..6 {
            std::fs::write(dir.join(format!("face_{f}.bin")), vec![f as u8; 4096]).unwrap();
        }
        let a = bake_identity(dir.to_str().unwrap()).unwrap();
        let face = dir.join("face_3.bin");
        let mtime = std::fs::metadata(&face).unwrap().modified().unwrap();
        std::fs::write(&face, vec![0xEEu8; 4096]).unwrap();
        std::fs::File::options().write(true).open(&face).unwrap().set_modified(mtime).unwrap();
        assert_eq!(std::fs::metadata(&face).unwrap().len(), 4096);
        assert_eq!(std::fs::metadata(&face).unwrap().modified().unwrap(), mtime, "same size, same time");
        let b = bake_identity(dir.to_str().unwrap()).unwrap();
        assert_ne!(a, b, "other bytes are another bake, whatever the file times say");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Review 3 #2: a committed save whose header.json is gone is damaged,
    /// not absent. Loading it as an empty world dropped its chunks while
    /// its simulation state resumed against natural terrain, and the next
    /// save wrote a fresh header over the old generation.
    #[test]
    fn a_committed_save_without_its_header_is_refused_not_loaded_empty() {
        let dir = temp_dir("headless");
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        store.set(ChunkKey::neisor(2, 0, 100, 200, 17), 1, 2, 3, 4, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, Some(b"sim-state")).unwrap();
        std::fs::remove_file(dir.header_path()).unwrap();
        let loaded = dir.load_into(&cfg, &reg, BAKE, &EditStore::new());
        assert!(loaded.is_err(), "a save with commit.json but no header.json must not load: {:?}", loaded.map(|r| r.chunks));
        let err = loaded.unwrap_err();
        assert!(err.contains("header.json"), "{err}");
        assert!(dir.read_sim().is_err(), "nor may its simulation state resume");
        store.set(ChunkKey::neisor(2, 0, 100, 200, 17), 1, 2, 4, 4, || vec![1; VOXELS]);
        assert!(dir.flush(&store, &cfg, &reg, BAKE, None).is_err(), "a save is not given a new header behind its back");
        assert!(!dir.header_path().exists());
        // An absent save is still an empty one; so is a header with nothing committed yet.
        let absent = temp_dir("absent");
        assert_eq!(absent.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap().chunks, 0);
        assert_eq!(absent.read_sim().unwrap(), None);
        absent.write_header(&cfg, &reg, BAKE).unwrap();
        assert_eq!(absent.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap().chunks, 0);
        let _ = std::fs::remove_dir_all(&dir.path);
        let _ = std::fs::remove_dir_all(&absent.path);
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
        let err = h.check(&cfg, &reg, BAKE).unwrap_err();
        assert!(err.contains("generator version"), "{err}");
        // No setting brings old terrain back, so the message says what does.
        assert!(err.contains("--save saves/another-name") && err.contains("untouched") && !err.contains("saved settings"), "{err}");
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
        let far_file = SaveDir::chunk_file(far, 1);
        std::fs::write(dir.path.join(&far_file), &bytes).unwrap();
        let mut c = commit.clone();
        c.chunks = vec![CommitEntry { key: key_array(far), file: far_file, sha256: hex(&sha256(&bytes)) }];
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
        let bad_file = SaveDir::chunk_file(good, 2);
        std::fs::write(dir.path.join(&bad_file), &bytes).unwrap();
        let mut c = commit.clone();
        c.generation = 2;
        c.chunks = vec![CommitEntry { key: key_array(good), file: bad_file, sha256: hex(&sha256(&bytes)) }];
        rewrite(&c);
        let err = dir.load_into(&cfg, &reg, BAKE, &EditStore::new()).unwrap_err();
        assert!(err.contains("block id 65535"), "{err}");
        // A file whose embedded key disagrees with the manifest.
        let mut c = commit.clone();
        let wrong_key = ChunkKey::neisor(1, 0, 11, 20, 30);
        let wrong_file = SaveDir::chunk_file(wrong_key, 1);
        std::fs::copy(dir.path.join(&good_entry.file), dir.path.join(&wrong_file)).unwrap();
        c.chunks = vec![CommitEntry { key: key_array(wrong_key), file: wrong_file, ..good_entry.clone() }];
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

    /// Every file operation a save makes, in order, relative to the save
    /// directory; it fails the operation numbered `fail_at` (and does it
    /// for real otherwise).
    struct RecordingFs {
        root: PathBuf,
        log: std::cell::RefCell<Vec<String>>,
        fail_at: Option<usize>,
    }

    impl RecordingFs {
        fn new(root: &Path, fail_at: Option<usize>) -> Self {
            Self { root: root.to_path_buf(), log: Default::default(), fail_at }
        }
        fn rel(&self, p: &Path) -> String {
            let r = p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().replace('\\', "/");
            if r.is_empty() { ".".into() } else { r }
        }
        fn op(&self, what: String) -> std::io::Result<()> {
            let n = self.log.borrow().len();
            self.log.borrow_mut().push(what);
            if self.fail_at == Some(n) { Err(std::io::Error::other("injected failure")) } else { Ok(()) }
        }
        fn ops(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    impl SaveFs for RecordingFs {
        fn write_synced(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            self.op(format!("write {}", self.rel(path)))?;
            OsFs.write_synced(path, bytes)
        }
        fn replace_durably(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            self.op(format!("replace {} -> {}", self.rel(from), self.rel(to)))?;
            OsFs.replace_durably(from, to)
        }
        fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
            self.op(format!("sync_dir {}", self.rel(dir)))?;
            OsFs.sync_dir(dir)
        }
        fn remove(&self, path: &Path) -> std::io::Result<()> {
            self.op(format!("remove {}", self.rel(path)))?;
            OsFs.remove(path)
        }
    }

    /// Generation 1 (chunks a and b, state "sim-1") on disk, then a and c
    /// edited: the store ready for generation 2.
    fn two_generations(tag: &str) -> (SaveDir, EditStore, [ChunkKey; 3]) {
        let dir = temp_dir(tag);
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        let keys = [ChunkKey::neisor(2, 0, 100, 200, 17), ChunkKey::neisor(2, 0, 100, 200, 16), ChunkKey::neisor(3, 0, 5, 6, 7)];
        store.set(keys[0], 0, 0, 0, 4, || vec![1; VOXELS]);
        store.set(keys[1], 0, 0, 0, 5, || vec![1; VOXELS]);
        dir.flush(&store, &cfg, &reg, BAKE, Some(b"sim-1")).unwrap();
        store.set(keys[0], 0, 0, 0, 6, || vec![1; VOXELS]);
        store.set(keys[2], 0, 0, 0, 7, || vec![1; VOXELS]);
        (dir, store, keys)
    }

    /// Review 3 #1: the order of a save. The new generation's files are
    /// written and flushed, then their directories, then commit.json is
    /// published by a durable replace and the directory flushed, and only
    /// after that are the superseded files of the previous generation
    /// removed (the store is acknowledged at the same point).
    #[test]
    fn a_save_makes_its_new_generation_durable_before_it_lets_go_of_the_old_one() {
        let (dir, store, _) = two_generations("order");
        let (cfg, reg) = (config(), registry());
        let fs = RecordingFs::new(&dir.path, None);
        assert_eq!(dir.flush_with(&fs, &store, &cfg, &reg, BAKE, Some(b"sim-2")).unwrap(), 2);
        let ops = fs.ops();
        println!("{ops:#?}");
        let at = |s: &str| ops.iter().position(|o| o == s).unwrap_or_else(|| panic!("{s} not in {ops:?}"));
        let commit_write = at("write commit.tmp");
        let publish = at("replace commit.tmp -> commit.json");
        assert_eq!(publish, commit_write + 1, "the commit is published right after it is written and flushed");
        for (i, o) in ops.iter().enumerate() {
            if o.starts_with("write chunks/") || o == "write sim.g2.bin" {
                assert!(i < commit_write && o.contains(".g2."), "{o} is a generation-2 file written before the commit");
            }
            if o.starts_with("remove ") {
                assert!(i > publish + 1, "{o} only after the published commit is flushed");
                assert!(o.contains(".g1."), "{o} is a superseded generation-1 file");
            }
        }
        let last_file = ops.iter().rposition(|o| o.starts_with("write chunks/") || o == "write sim.g2.bin").unwrap();
        let dirs_before: Vec<&String> = ops[last_file..commit_write].iter().collect();
        assert!(dirs_before.contains(&&"sync_dir chunks".to_string()) && dirs_before.contains(&&"sync_dir .".to_string()), "the new files' directories are flushed before the commit: {dirs_before:?}");
        assert_eq!(ops[publish + 1], "sync_dir .", "and the save directory after it");
        assert_eq!(ops.iter().filter(|o| o.starts_with("remove ")).count(), 2, "a's generation-1 file and sim.g1.bin");
        assert_eq!(store.dirty_count(), 0);
        let _ = std::fs::remove_dir_all(&dir.path);
    }

    /// Review 3 #1: whichever step fails (a file's flush, a directory's, the
    /// publication), the save reports it, acknowledges nothing (the next
    /// save writes it all again) and removes nothing: the previous
    /// generation stays whole and loads. A failed removal of a superseded
    /// file is only garbage left behind; the save stands.
    #[test]
    fn a_failure_at_any_step_of_a_save_keeps_the_previous_generation() {
        let (dir, store, _) = two_generations("count");
        let (cfg, reg) = (config(), registry());
        let fs = RecordingFs::new(&dir.path, None);
        dir.flush_with(&fs, &store, &cfg, &reg, BAKE, Some(b"sim-2")).unwrap();
        let ops = fs.ops();
        let _ = std::fs::remove_dir_all(&dir.path);
        for (i, op) in ops.iter().enumerate() {
            let (dir, store, keys) = two_generations(&format!("fail{i}"));
            let gen1_files: Vec<PathBuf> = std::fs::read_dir(dir.chunks_dir()).unwrap().map(|e| e.unwrap().path()).chain([dir.path.join("sim.g1.bin")]).collect();
            let fs = RecordingFs::new(&dir.path, Some(i));
            let result = dir.flush_with(&fs, &store, &cfg, &reg, BAKE, Some(b"sim-2"));
            let back = EditStore::new();
            let report = dir.load_into(&cfg, &reg, BAKE, &back).unwrap_or_else(|e| panic!("failing {op}: the save no longer loads: {e}"));
            if op.starts_with("remove ") {
                assert!(result.is_ok(), "failing {op}: a removal that fails is not a failed save");
                assert_eq!(report.generation, 2);
                continue;
            }
            assert!(result.is_err(), "failing {op} was not reported");
            assert_eq!(store.dirty_count(), 2, "failing {op}: nothing acknowledged");
            for f in &gen1_files {
                assert!(f.exists(), "failing {op}: {} was removed", f.display());
            }
            // The commit on disk is generation 1, or 2 when only the flush after
            // its publication failed; either way it loads whole.
            match report.generation {
                1 => {
                    assert_eq!(back.get(keys[0], 0, 0, 0), Some(4), "failing {op}: generation 1's a");
                    assert_eq!(back.get(keys[2], 0, 0, 0), None);
                    assert_eq!(dir.read_sim().unwrap().as_deref(), Some(&b"sim-1"[..]));
                }
                2 => {
                    assert!(op.starts_with("sync_dir") && i > ops.iter().position(|o| o == "replace commit.tmp -> commit.json").unwrap(), "failing {op}: published early");
                    assert_eq!(back.get(keys[0], 0, 0, 0), Some(6));
                    assert_eq!(back.get(keys[2], 0, 0, 0), Some(7));
                }
                g => panic!("failing {op}: generation {g}"),
            }
            assert_eq!(back.get(keys[1], 0, 0, 0), Some(5), "failing {op}: b");
            // And the next save, with nothing failing, goes through.
            assert!(dir.flush(&store, &cfg, &reg, BAKE, Some(b"sim-3")).is_ok());
            assert_eq!(store.dirty_count(), 0);
            let _ = std::fs::remove_dir_all(&dir.path);
        }
        println!("{} failure points checked: {ops:?}", ops.len());
    }

    /// The synchronous disk protocol's cost for N dirty chunks (S1 measures
    /// Session requests separately). `cargo test --release -- --ignored
    /// save_cost --nocapture --test-threads=1`.
    #[test]
    #[ignore]
    fn save_cost_by_dirty_chunks() {
        let (cfg, reg) = (config(), registry());
        for n in [1u32, 100, 400] {
            let dir = temp_dir(&format!("cost{n}"));
            let store = EditStore::new();
            for m in 0..n {
                store.set(ChunkKey::neisor(2, 0, m % 64, m / 64, 17), 1, 2, 3, 4, || vec![1; VOXELS]);
            }
            let t = std::time::Instant::now();
            dir.flush(&store, &cfg, &reg, BAKE, Some(b"sim-1")).unwrap();
            let first = t.elapsed().as_secs_f64() * 1e3;
            for m in 0..n {
                store.set(ChunkKey::neisor(2, 0, m % 64, m / 64, 17), 1, 2, 3, 5, || vec![1; VOXELS]);
            }
            let t = std::time::Instant::now();
            dir.flush(&store, &cfg, &reg, BAKE, Some(b"sim-2")).unwrap();
            let second = t.elapsed().as_secs_f64() * 1e3;
            println!("save of {n:4} dirty chunks: first {first:7.1} ms ({:.2} ms per chunk), again {second:7.1} ms ({:.2} ms per chunk)", first / n as f64, second / n as f64);
            let _ = std::fs::remove_dir_all(&dir.path);
        }
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

    #[test]
    fn a_save_image_installs_and_loads_like_the_store_it_came_from() {
        let (cfg, reg) = (config(), registry());
        let store = EditStore::new();
        let (a, b) = (ChunkKey::neisor(2, 0, 100, 200, 17), ChunkKey::neisor(3, 0, 5, 6, 7));
        store.set(a, 1, 2, 3, 4, || vec![1; VOXELS]);
        store.set(b, 0, 0, 0, 5, || vec![1; VOXELS]);
        let image = snapshot_image(&store, &cfg, &reg, BAKE, b"sim-state").unwrap();
        let names: Vec<&str> = image.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names[..2], ["header.json", "commit.json"]);
        assert!(names.iter().all(|n| is_save_file_name(n)), "{names:?}");
        let dir = temp_dir("image");
        dir.install_image(&image).unwrap();
        let back = EditStore::new();
        let report = dir.load_into(&cfg, &reg, BAKE, &back).unwrap();
        assert_eq!((report.chunks, report.generation), (2, 1));
        assert_eq!(back.snapshot(), store.snapshot());
        assert_eq!(dir.read_sim().unwrap().as_deref(), Some(&b"sim-state"[..]));
        // Never over an existing save, never outside the directory.
        assert!(dir.install_image(&image).unwrap_err().contains("already holds"));
        let _ = std::fs::remove_dir_all(&dir.path);
        for bad in ["../x.json", "chunks/../../evil.chunk", "C:/evil.chunk", "chunks/a/b.chunk", "/etc/passwd", "sim.g1.exe", "chunks/.chunk", "header.json/x"] {
            assert!(!is_save_file_name(bad), "{bad}");
            let mut evil = image.clone();
            evil.push((bad.to_string(), b"x".to_vec()));
            assert!(temp_dir("evil").install_image(&evil).unwrap_err().contains("refusing"), "{bad}");
        }
    }
}

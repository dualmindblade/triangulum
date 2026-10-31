//! One owned snapshot and one writer. There is deliberately no job queue.
use super::{OsFs, SaveDir, SaveFs};
use crate::engine::Engine;
use crate::grid::{ChunkKey, WorldConfig};
use crate::sim::{Sim, Snapshot};
use crate::world::edits::SaveChunk;
use crate::world::{EditStore, Registry};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Payload budget, checked before copying chunks or simulation queues.
/// A busy writer accepts no second snapshot (not even a partial one).
pub const SNAPSHOT_LIMIT: usize = 256 * 1024 * 1024;

pub struct SaveSnapshot {
    chunks: Vec<SaveChunk>,
    config: WorldConfig,
    registry: Registry,
    bake: String,
    sim: Snapshot,
    pub bytes: usize,
}

impl SaveSnapshot {
    pub fn capture(engine: &Engine, bake: &str, sim: &Sim, limit: usize) -> Result<Self, String> {
        // Conservative allowance for the frozen registry's maps, strings,
        // colours and flags as well as config, acknowledgements and Vec headers.
        let metadata = engine.registry.with_defs(|defs| {
            let strings: usize = defs.iter().map(|d| {
                d.name.len() + d.was.iter().map(|s| s.len() + 32).sum::<usize>()
                    + [&d.identity, &d.comment, &d.texture, &d.top_texture, &d.side_texture, &d.bottom_texture]
                        .into_iter().flatten().map(String::len).sum::<usize>()
            }).sum();
            1024 * 1024 + 8 * (std::mem::size_of_val(defs) + strings)
                + engine.config.name.len() + engine.config.bake.len() + bake.len()
        });
        let remaining = limit.checked_sub(metadata).ok_or_else(|| budget_error(limit))?;
        let sim_bytes = sim.save_snapshot_bound(&engine.world);
        let remaining = remaining.checked_sub(sim_bytes).ok_or_else(|| budget_error(limit))?;
        let (chunks, chunk_bytes) = engine.edits.save_snapshot(remaining).map_err(|_| budget_error(limit))?;
        let registry = Registry::from_defs(engine.registry.defs())?;
        let sim = sim.snapshot_for_save(&engine.world);
        Ok(Self { chunks, config: engine.config.clone(), registry, bake: bake.to_string(), sim, bytes: metadata + sim_bytes + chunk_bytes })
    }

    fn write(self, dir: &SaveDir, fs: &dyn SaveFs) -> Result<Written, String> {
        let sim = self.sim.to_bytes();
        let revisions = dir.flush_snapshot_with(fs, &self.chunks, &self.config, &self.registry, &self.bake, Some(&sim))?;
        Ok(Written { revisions, sim_digest: crate::sha256::sha256(&sim) })
    }
}

fn budget_error(limit: usize) -> String {
    format!("save snapshot exceeds the {} MiB memory limit; changes are still unsaved", limit / (1024 * 1024))
}

struct Pending {
    job: JoinHandle<Result<Written, String>>,
    store: Arc<EditStore>,
}

struct Written {
    revisions: Vec<(ChunkKey, u64)>,
    sim_digest: [u8; 32],
}

pub struct Saved {
    pub chunks: usize,
    pub sim_digest: [u8; 32],
}

pub struct SaveWorker {
    pending: Option<Pending>,
    fs: Arc<dyn SaveFs + Send + Sync>,
}

impl Default for SaveWorker {
    fn default() -> Self {
        Self { pending: None, fs: Arc::new(OsFs) }
    }
}

impl SaveWorker {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn start(&mut self, dir: SaveDir, snapshot: SaveSnapshot, store: Arc<EditStore>) -> Result<(), String> {
        if self.busy() { return Err("a save is already in flight".into()); }
        let fs = self.fs.clone();
        let job = std::thread::Builder::new().name("save-writer".into())
            .spawn(move || snapshot.write(&dir, fs.as_ref())).map_err(|e| format!("start save worker: {e}"))?;
        self.pending = Some(Pending { job, store });
        Ok(())
    }

    /// The game thread acknowledges only a durable, successful result.
    /// Joining also reports a worker panic and makes the next attempt possible.
    pub fn finish(&mut self, wait: bool) -> Option<Result<Saved, String>> {
        let pending = self.pending.as_ref()?;
        if !wait && !pending.job.is_finished() { return None; }
        let Pending { job, store } = self.pending.take().unwrap();
        Some(job.join().map_err(|_| "save worker panicked; changes are still unsaved".to_string())
            .and_then(|r| r).map(|written| {
                store.acknowledge(&written.revisions);
                Saved { chunks: written.revisions.len(), sim_digest: written.sim_digest }
            }))
    }

    #[cfg(test)]
    pub(crate) fn with_fs(fs: Arc<dyn SaveFs + Send + Sync>) -> Self {
        Self { pending: None, fs }
    }
}

impl Drop for SaveWorker {
    fn drop(&mut self) {
        // Normal quit uses the explicit barrier, including the final edits.
        // Unwinding/dropping a session must not detach a writer into a new load.
        if let Some(Err(e)) = self.finish(true) { eprintln!("save: {e}"); }
    }
}

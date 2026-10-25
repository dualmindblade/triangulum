//! The world: block definitions and chunk storage.

pub mod blocks;
pub mod chunk;

pub use blocks::{BlockDef, BlockId, Registry};
pub use chunk::Chunk;

use crate::grid::{ChunkKey, FastMap};

use std::sync::Arc;

/// Loaded chunks at every LOD, keyed by chunk. Chunks are immutable once
/// generated (edits arrive in M1b and will replace the Arc).
#[derive(Default)]
pub struct ChunkStore {
    chunks: FastMap<ChunkKey, Arc<Chunk>>,
}

impl ChunkStore {
    pub fn insert(&mut self, key: ChunkKey, chunk: Arc<Chunk>) {
        self.chunks.insert(key, chunk);
    }
    pub fn remove(&mut self, key: &ChunkKey) -> Option<Arc<Chunk>> {
        self.chunks.remove(key)
    }
    pub fn get(&self, key: &ChunkKey) -> Option<&Arc<Chunk>> {
        self.chunks.get(key)
    }
    pub fn len(&self) -> usize {
        self.chunks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
    /// Bytes held by palettes, indices and water layers.
    pub fn bytes(&self) -> usize {
        self.chunks.values().map(|c| c.bytes()).sum()
    }
}

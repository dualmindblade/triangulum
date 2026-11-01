//! Drawing falling blocks: one unit cube per block through the terrain
//! pipeline, with a curvature-grid record of its own at the block's current
//! radius, so a block in flight is lit, fogged and warped exactly like the
//! chunk it left. No renderer changes: a cube is six quads in `mesh.rs`'s
//! format allocated once per block id, and each block gets a grid slot for
//! the frame through `Renderer::add_mesh` with no quads.
//!
//! Where a block is drawn (C2, Andrew's #5). `Sim::view` says where each
//! block is this frame, part way through its tick; a block that slid over
//! a ledge is drawn on the straight way from the centre of the cell it
//! left to the centre of its own column, `Drawn::slide_progress` of the
//! way along, so the move reads as one block going over an edge (across a
//! chunk border, a cube edge or a cube corner alike: the way is a chord in
//! world space, and the cube keeps its own column's frame, which differs
//! from the old cell's by a hundred-thousandth of a radian).
//!
//! The second copy. A chunk is remeshed asynchronously (`stream.rs`), so
//! for a frame or more after a block leaves a cell the chunk's mesh still
//! shows it there, and for a frame or more after it lands the mesh shows
//! nothing where it is. Each block names the write it is waiting on
//! (`Drawn::wrote`: the cell and that chunk's edit revision) and the
//! streamer records which revision each resident LOD-0 mesh was built
//! from (`Renderer::meshed_at`), so this module can tell: a block whose
//! old cell is still in the mesh is held there, drawn exactly where the
//! mesh shows it, and starts its way the frame the mesh drops it (its
//! slide clock starts then too, so nothing jumps); a landed block is drawn
//! as a cube until the mesh shows it and not a frame longer. Where the
//! renderer cannot tell (no LOD-0 mesh of that chunk, or a reload, which
//! records no revision), a block is drawn on its way at once and a landed
//! one for `GHOST_TICKS`, as before C2. `MotionStats` counts what this
//! prevented, for the harness.

use super::{Drawn, GHOST_TICKS, SLIDE_TICKS};
use crate::grid::{BlockPos, CHUNK, FastMap, Planet};
use crate::mesh::{GRID_CELLS, GRID_CORNERS, GRID_F32S, GRID_R0_INDEX, Quad};
use crate::render::{DrawRecord, GpuMesh, Renderer};
use crate::world::{BlockId, World};
use glam::DVec3;

/// What the last frame's draw found and prevented (the harness reports it).
#[derive(Default, Clone, Copy, Debug)]
pub struct MotionStats {
    pub frames: u64,
    /// Blocks drawn this frame (in flight and landed).
    pub drawn: usize,
    /// Blocks in flight whose old cell's mesh still showed them while they
    /// had visibly moved on: a second copy, had they been drawn on their
    /// way (what the draw before C2 did). Held instead.
    pub doubles: usize,
    pub doubles_max: usize,
    /// Blocks held at the cell they left this frame, and the most at once.
    pub held: usize,
    pub held_max: usize,
    /// Landed blocks drawn as cubes because their chunk's mesh did not yet
    /// show them, and those among them older than `GHOST_TICKS`: a gap,
    /// had the cube been dropped by age (as before C2).
    pub ghosts: usize,
    pub ghosts_max: usize,
    pub gaps: usize,
    pub gaps_max: usize,
    /// The oldest block (ticks since its write) whose chunk's mesh was
    /// still behind it: how late a mesh has been.
    pub stale_max_ticks: f64,
    /// Blocks whose chunk's mesh revision was unknown this frame.
    pub unknown: usize,
    /// The longest `emit` took, microseconds, and the sum over the frames
    /// that drew anything (`emit_frames`): the cost of the look.
    pub emit_us_max: u32,
    pub emit_us_sum: u64,
    pub emit_frames: u64,
}

/// Where to draw a block, given whether its chunk's mesh has caught up
/// (`fresh`: Some(true) it has, Some(false) it has not, None unknown) and,
/// for a block in flight, when it was released from a hold
/// (`released`: the `since` at which its way began to be drawn).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// At the cell it left, exactly where the mesh still shows it.
    Hold,
    /// On its way, this many ticks along it.
    Go(f64),
    /// Not at all: the mesh shows it.
    Skip,
}

pub fn place(b: &Drawn, fresh: Option<bool>, released: Option<f64>) -> Place {
    if b.ghost {
        return match fresh {
            Some(true) => Place::Skip,
            Some(false) => Place::Go(b.since),
            None if b.since < GHOST_TICKS as f64 => Place::Go(b.since),
            None => Place::Skip,
        };
    }
    match (fresh, b.wrote) {
        (Some(false), Some(_)) => Place::Hold,
        _ => Place::Go((b.since - released.unwrap_or(0.0)).max(0.0)),
    }
}

/// Has a mesh built from edit revision `meshed` caught up with the write
/// at revision `rev`? A write of revision 0 (a reload) counts as caught up.
pub fn caught_up(meshed: Option<u64>, rev: u64) -> Option<bool> {
    if rev == 0 {
        return Some(true);
    }
    meshed.map(|m| m >= rev)
}

#[derive(Default)]
pub struct FallingDraws {
    cubes: FastMap<BlockId, GpuMesh>,
    /// Grid slots used by the last frame; freed when the next begins.
    slots: Vec<GpuMesh>,
    pub drawn: usize,
    /// Blocks in flight seen last frame, by their column and the cell they
    /// left: None while held, else the `since` at which they were released.
    holds: FastMap<(u8, u32, u32, BlockPos), Option<f64>>,
    pub stats: MotionStats,
}

impl FallingDraws {
    /// The six faces of a unit cube at voxel (0, 0, 0): axis, sign, slice 0,
    /// u 0, v 0, 1 x 1, full brightness (AO 3 at every corner).
    fn cube(id: BlockId) -> [Quad; 6] {
        let hi = id as u32 | (0xFF << 16);
        let mut q = [[0u32, hi]; 6];
        for axis in 0..3u32 {
            for sign in 0..2u32 {
                q[(axis * 2 + sign) as usize] = [axis | (sign << 2), hi];
            }
        }
        q
    }

    /// Append one draw per block to `out` (camera-relative, like the
    /// streamer's records). Last frame's grid slots are reused: that frame
    /// was submitted before these writes are queued.
    pub fn emit(&mut self, gpu: &mut Renderer, planet: &Planet, blocks: &[Drawn], cam: DVec3, out: &mut Vec<DrawRecord>) {
        let t0 = std::time::Instant::now();
        for gm in self.slots.drain(..) {
            gpu.free_mesh(gm);
        }
        self.drawn = 0;
        let st = &mut self.stats;
        st.frames += 1;
        (st.drawn, st.doubles, st.held, st.ghosts, st.gaps, st.unknown) = (0, 0, 0, 0, 0, 0);
        let holds = std::mem::take(&mut self.holds);
        if blocks.is_empty() {
            return;
        }
        let mut grid = vec![0f32; GRID_F32S];
        let voxel = planet.block_size as f32;
        for b in blocks {
            let fresh = match b.wrote {
                None => Some(true),
                Some((cell, rev)) => caught_up(gpu.meshed_at.get(&World::chunk_of(cell).0).copied(), rev),
            };
            if fresh.is_none() {
                st.unknown += 1;
            }
            let key = b.wrote.map(|(cell, _)| (b.face, b.i, b.j, cell));
            let released = key.and_then(|k| holds.get(&k).copied()).flatten();
            let placed = place(b, fresh, released);
            if fresh == Some(false) {
                st.stale_max_ticks = st.stale_max_ticks.max(b.since);
            }
            // Where this frame draws it: its own column at `z`, or the cell
            // it left, or part way between.
            let (z, offset) = match placed {
                Place::Skip => continue,
                Place::Hold => {
                    let cell = b.wrote.expect("a hold names the cell").0;
                    st.held += 1;
                    let moved = (cell.face, cell.i, cell.j) != (b.face, b.i, b.j) || b.z <= cell.k as f64 - 0.3;
                    st.doubles += moved as usize;
                    if let Some(k) = key {
                        self.holds.insert(k, None);
                    }
                    (cell.k as f64, Some((cell, 0.0)))
                }
                Place::Go(since) => {
                    if b.ghost {
                        st.ghosts += 1;
                        st.gaps += (fresh == Some(false) && b.since >= GHOST_TICKS as f64) as usize;
                        (b.z, None)
                    } else {
                        if let Some(k) = key {
                            self.holds.insert(k, Some(released.unwrap_or(0.0).max(b.since - since)));
                        }
                        let along = b.slid_from().filter(|_| since < SLIDE_TICKS).map(|cell| (cell, Drawn::progress(since)));
                        (b.z, along)
                    }
                }
            };
            let cube = match self.cubes.get(&b.id) {
                Some(g) => *g,
                None => {
                    let Some(g) = gpu.add_mesh(&Self::cube(b.id), &[], &grid) else { continue };
                    self.cubes.insert(b.id, g);
                    g
                }
            };
            // A virtual LOD-0 chunk whose lattice corner is this column at the
            // block's bottom: the shader's bilinear grid over 4 x 4 cells of 8
            // blocks, of which the cube uses the first eighth of cell (0, 0).
            let r0 = planet.radius_at(z);
            let (i0, j0) = (b.i as f64, b.j as f64);
            let mut dirs = [DVec3::ZERO; GRID_CORNERS * GRID_CORNERS];
            for gy in 0..GRID_CORNERS {
                for gx in 0..GRID_CORNERS {
                    dirs[gy * GRID_CORNERS + gx] = planet.dir(b.face, i0 + (gx * CHUNK / GRID_CELLS) as f64, j0 + (gy * CHUNK / GRID_CELLS) as f64);
                }
            }
            let mut origin = dirs[0] * r0;
            if let Some((cell, s)) = offset {
                // The way from the centre of the cell it left to the centre
                // of its own column, at the height it left from: a chord.
                let k = cell.k as f64;
                let way = planet.pos(cell.face, cell.i as f64 + 0.5, cell.j as f64 + 0.5, k) - planet.pos(b.face, i0 + 0.5, j0 + 0.5, k);
                origin += way * (1.0 - s);
            }
            for (g, d) in dirs.iter().enumerate() {
                let off = *d * r0 - (dirs[0] * r0);
                grid[g * 4..g * 4 + 3].copy_from_slice(&[off.x as f32, off.y as f32, off.z as f32]);
                let base = (GRID_CORNERS * GRID_CORNERS + g) * 4;
                grid[base..base + 3].copy_from_slice(&[d.x as f32, d.y as f32, d.z as f32]);
            }
            grid[GRID_R0_INDEX] = r0 as f32;
            let Some(gm) = gpu.add_mesh(&[], &[], &grid) else { continue };
            let o = (origin - cam).as_vec3();
            out.push(DrawRecord { origin: o.to_array(), voxel, slot: gm.slot, quad_base: cube.opaque_base, quad_count: cube.opaque_count, pad: 0 });
            self.slots.push(gm);
            self.drawn += 1;
        }
        st.drawn = self.drawn;
        st.doubles_max = st.doubles_max.max(st.doubles);
        st.held_max = st.held_max.max(st.held);
        st.ghosts_max = st.ghosts_max.max(st.ghosts);
        st.gaps_max = st.gaps_max.max(st.gaps);
        let us = t0.elapsed().as_micros() as u32;
        st.emit_us_max = st.emit_us_max.max(us);
        st.emit_us_sum += us as u64;
        st.emit_frames += 1;
    }

    #[cfg(test)]
    pub fn cube_for_test(id: BlockId) -> [Quad; 6] {
        Self::cube(id)
    }

    /// Give everything back (the renderer is being cleared).
    pub fn clear(&mut self, gpu: &mut Renderer) {
        for gm in self.slots.drain(..) {
            gpu.free_mesh(gm);
        }
        for (_, gm) in self.cubes.drain() {
            gpu.free_mesh(gm);
        }
        self.holds.clear();
        self.drawn = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(ghost: bool, since: f64, rev: u64) -> Drawn {
        let cell = BlockPos { face: 2, i: 10, j: 10, k: 600 };
        Drawn { id: 4, face: 2, i: 11, j: 10, z: 600.0 - since * 0.05, since, ghost, wrote: Some((cell, rev)) }
    }

    /// The decision the second copy and the blink turn on, without a GPU.
    #[test]
    fn a_block_is_held_while_its_old_cell_is_still_meshed_and_a_landed_one_drawn_until_it_is() {
        // In flight: a stale mesh holds it; a current or unknown one lets it go.
        assert_eq!(place(&block(false, 0.0, 7), Some(false), None), Place::Hold);
        assert_eq!(place(&block(false, 2.5, 7), Some(false), None), Place::Hold, "however long the mesh is late");
        assert_eq!(place(&block(false, 0.0, 7), Some(true), None), Place::Go(0.0));
        assert_eq!(place(&block(false, 1.0, 7), None, None), Place::Go(1.0), "unknown: on its way at once, as before C2");
        // Released after a hold: its way starts from the release, so nothing jumps.
        assert_eq!(place(&block(false, 1.5, 7), Some(true), Some(1.0)), Place::Go(0.5));
        assert_eq!(place(&block(false, 0.5, 7), Some(true), Some(1.0)), Place::Go(0.0), "never before the start");
        // Landed: drawn exactly until the mesh shows it; by age only when nobody can tell.
        assert_eq!(place(&block(true, 0.0, 7), Some(true), None), Place::Skip);
        assert_eq!(place(&block(true, 12.0, 7), Some(false), None), Place::Go(12.0), "a gap under the old rule, a cube now");
        assert_eq!(place(&block(true, 4.9, 7), None, None), Place::Go(4.9));
        assert_eq!(place(&block(true, 5.0, 7), None, None), Place::Skip, "GHOST_TICKS when the mesh revision is unknown");
        // A write of revision 0 (a reload) counts as caught up.
        assert_eq!(caught_up(None, 0), Some(true));
        assert_eq!(caught_up(None, 3), None);
        assert_eq!(caught_up(Some(2), 3), Some(false));
        assert_eq!(caught_up(Some(3), 3), Some(true));
        assert_eq!(caught_up(Some(9), 3), Some(true));
        // The slide's way: fast off the edge, then the fall takes over.
        assert_eq!(Drawn::progress(0.0), 0.0);
        assert!(Drawn::progress(1.0) > 0.4 && Drawn::progress(2.0) > 0.7, "{} {}", Drawn::progress(1.0), Drawn::progress(2.0));
        assert_eq!(Drawn::progress(SLIDE_TICKS), 1.0);
        assert_eq!(Drawn::progress(99.0), 1.0);
        assert_eq!(block(false, 0.0, 1).slid_from(), Some(BlockPos { face: 2, i: 10, j: 10, k: 600 }));
        assert_eq!(Drawn { i: 10, ..block(false, 0.0, 1) }.slid_from(), None, "a straight fall left its own column");
    }
}

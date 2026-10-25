//! Drawing falling blocks: one unit cube per block through the terrain
//! pipeline, with a curvature-grid record of its own at the block's current
//! radius, so a block in flight is lit, fogged and warped exactly like the
//! chunk it left. No renderer changes: a cube is six quads in `mesh.rs`'s
//! format allocated once per block id, and each block gets a grid slot for
//! the frame through `Renderer::add_mesh` with no quads.

use super::Falling;
use crate::grid::{CHUNK, FastMap, Planet};
use crate::mesh::{GRID_CELLS, GRID_CORNERS, GRID_F32S, GRID_R0_INDEX, Quad};
use crate::render::{DrawRecord, GpuMesh, Renderer};
use crate::world::BlockId;
use glam::DVec3;

#[derive(Default)]
pub struct FallingDraws {
    cubes: FastMap<BlockId, GpuMesh>,
    /// Grid slots used by the last frame; freed when the next begins.
    slots: Vec<GpuMesh>,
    pub drawn: usize,
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
    pub fn emit(&mut self, gpu: &mut Renderer, planet: &Planet, blocks: &[Falling], cam: DVec3, out: &mut Vec<DrawRecord>) {
        for gm in self.slots.drain(..) {
            gpu.free_mesh(gm);
        }
        self.drawn = 0;
        if blocks.is_empty() {
            return;
        }
        let mut grid = vec![0f32; GRID_F32S];
        let voxel = planet.block_size as f32;
        for b in blocks {
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
            let r0 = planet.radius_at(b.z);
            let (i0, j0) = (b.i as f64, b.j as f64);
            let mut dirs = [DVec3::ZERO; GRID_CORNERS * GRID_CORNERS];
            for gy in 0..GRID_CORNERS {
                for gx in 0..GRID_CORNERS {
                    dirs[gy * GRID_CORNERS + gx] = planet.dir(b.face, i0 + (gx * CHUNK / GRID_CELLS) as f64, j0 + (gy * CHUNK / GRID_CELLS) as f64);
                }
            }
            let origin = dirs[0] * r0;
            for (g, d) in dirs.iter().enumerate() {
                let off = *d * r0 - origin;
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
        self.drawn = 0;
    }
}

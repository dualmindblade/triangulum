//! wgpu renderer: pipeline setup, per-tile GPU buffers, frame drawing, and an
//! offscreen capture path (renders to a texture and saves a PNG — no window).

use crate::camera::Camera;
use crate::planet::Planet;
use crate::terrain::{build_tile, select_tiles, TileKey, TileMesh, Vertex};
use crate::voxel::{build_chunk, select_chunks, ChunkKey, Edits};
use anyhow::Result;
use glam::{DVec3, Mat4};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

pub const VOXEL_MAX_ALT_KM: f64 = 2.5;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DrawKey {
    Tile(TileKey),
    Chunk(ChunkKey),
}

const TILE_UNIFORM_STRIDE: u64 = 256; // min dynamic-offset alignment
const MAX_TILES: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    // inverse view-projection: the sky pass unprojects screen corners into
    // world-space view rays (camera-relative space, so the eye is origin)
    inv_view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    // disc cut out of the heightfield where voxel chunks own the ground:
    // xyz = disc center relative to the camera (km), w = radius (0 = off)
    hole: [f32; 4],
    // unit radial at the disc center (w = underwater flag)
    hole_up: [f32; 4],
    // xyz = camera radial up, w = camera height above the nominal sphere
    // (km) — drives the sky gradient and how the atmosphere thins to space
    sky: [f32; 4],
}

struct GpuTile {
    origin_km: DVec3,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    last_used: u64,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    tiles_buf: wgpu::Buffer,
    depth: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    cache: HashMap<TileKey, GpuTile>,
    chunk_cache: HashMap<ChunkKey, GpuTile>,
    frame_counter: u64,
    pub exaggeration: f64,
    /// None = sun follows the camera (always day where you are);
    /// Some(dir) = fixed sun direction.
    pub sun_dir: Option<DVec3>,
    /// Eye below a water surface: the shader tints the whole view.
    pub underwater: bool,
}

impl Renderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        exaggeration: f64,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tiles_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tile uniforms"),
            size: TILE_UNIFORM_STRIDE * MAX_TILES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &tiles_buf,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                },
            ],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                // no culling: skirt quads have mixed winding, and culled
                // skirts reopen the very cracks they exist to hide
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // reversed-Z: near = 1, far = 0 (see camera::view_proj)
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        // sky: a fullscreen triangle drawn at infinity (reversed-Z depth 0)
        // after the terrain, in the same pass — it only wins where nothing
        // else drew
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sky"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sky"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sky"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let depth = Self::make_depth(&device, size);
        Self {
            device,
            queue,
            pipeline,
            sky_pipeline,
            bind_group,
            globals_buf,
            tiles_buf,
            depth,
            size,
            format,
            cache: HashMap::new(),
            chunk_cache: HashMap::new(),
            frame_counter: 0,
            exaggeration,
            sun_dir: None,
            underwater: false,
        }
    }

    fn make_depth(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        if size.0 > 0 && size.1 > 0 {
            self.size = size;
            self.depth = Self::make_depth(&self.device, size);
        }
    }

    /// Drop cached chunk meshes (after edits) so they rebuild next frame.
    pub fn invalidate_chunks(&mut self, keys: &[ChunkKey]) {
        for k in keys {
            self.chunk_cache.remove(k);
        }
    }

    /// Draw one frame into `target`. Returns the number of tiles drawn.
    pub fn draw(
        &mut self,
        target: &wgpu::TextureView,
        planet: &Planet,
        camera: &Camera,
        edits: &Edits,
    ) -> usize {
        self.frame_counter += 1;
        let cam_pos = camera.position();
        let voxel_radius_m = 200.0 + (VOXEL_MAX_ALT_KM - camera.altitude_km).max(0.0) * 120.0;
        let focus = (camera.altitude_km < VOXEL_MAX_ALT_KM)
            .then(|| (cam_pos.normalize(), voxel_radius_m / 1000.0 + 0.2));
        let keys = select_tiles(cam_pos, planet.radius_km, 0.35, focus);

        // upload globals (camera-relative view-projection, f64 -> f32 at the end)
        let vp = camera.view_proj(self.size.0 as f64 / self.size.1 as f64);
        let vp32 = Mat4::from_cols_array(&vp.to_cols_array().map(|x| x as f32));
        let sun = self.sun_dir.unwrap_or_else(|| cam_pos.normalize());
        // where voxels are active, cut the heightfield away under them so the
        // two systems never overlap (no mesh poking up between block tops)
        let mut hole = [0.0f32; 4];
        let mut hole_up = [0.0f32; 4];
        if focus.is_some() {
            let r_km = crate::voxel::safe_hole_radius_km(voxel_radius_m);
            if r_km > 0.0 {
                let up = cam_pos.normalize();
                let hc = -up * camera.altitude_km; // camera ground point, camera-relative
                hole = [hc.x as f32, hc.y as f32, hc.z as f32, r_km as f32];
                hole_up = [up.x as f32, up.y as f32, up.z as f32, 0.0];
            }
        }
        hole_up[3] = if self.underwater { 1.0 } else { 0.0 };
        let inv32 = Mat4::from_cols_array(&vp.inverse().to_cols_array().map(|x| x as f32));
        let up = cam_pos.normalize();
        let cam_h_km = (cam_pos.length() - planet.radius_km).max(0.0);
        let globals = Globals {
            view_proj: vp32.to_cols_array_2d(),
            inv_view_proj: inv32.to_cols_array_2d(),
            sun_dir: [sun.x as f32, sun.y as f32, sun.z as f32, 0.0],
            hole,
            hole_up,
            sky: [up.x as f32, up.y as f32, up.z as f32, cam_h_km as f32],
        };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        let exagg = self.exaggeration;
        // build missing tiles in parallel (rayon), then upload sequentially
        let missing: Vec<TileKey> = keys
            .iter()
            .filter(|k| !self.cache.contains_key(k))
            .copied()
            .collect();
        let built: Vec<(TileKey, TileMesh)> = {
            use rayon::prelude::*;
            missing.par_iter().map(|k| (*k, build_tile(planet, *k, exagg))).collect()
        };
        for (k, mesh) in built {
            let gpu = self.upload(mesh);
            self.cache.insert(k, gpu);
        }

        // near the ground, add true voxel chunks around the camera footprint
        let mut chunk_keys: Vec<ChunkKey> = Vec::new();
        if camera.altitude_km < VOXEL_MAX_ALT_KM {
            chunk_keys = select_chunks(cam_pos, planet, voxel_radius_m);
            let missing_c: Vec<ChunkKey> = chunk_keys
                .iter()
                .filter(|k| !self.chunk_cache.contains_key(k))
                .copied()
                .collect();
            let built_c: Vec<(ChunkKey, TileMesh)> = {
                use rayon::prelude::*;
                missing_c
                    .par_iter()
                    .map(|k| (*k, build_chunk(planet, edits, *k, exagg)))
                    .collect()
            };
            for (k, mesh) in built_c {
                let gpu = self.upload(mesh);
                self.chunk_cache.insert(k, gpu);
            }
        }

        let mut draws: Vec<(DrawKey, u32)> = Vec::new();
        let all_keys = keys
            .iter()
            .map(|k| DrawKey::Tile(*k))
            .chain(chunk_keys.iter().map(|k| DrawKey::Chunk(*k)));
        for (slot, key) in all_keys.enumerate().take(MAX_TILES) {
            let tile = match key {
                DrawKey::Tile(k) => self.cache.get_mut(&k).unwrap(),
                DrawKey::Chunk(k) => self.chunk_cache.get_mut(&k).unwrap(),
            };
            tile.last_used = self.frame_counter;
            let off = tile.origin_km - cam_pos;
            // w: 0 = voxel chunk, 1 = far tile (soft water tint), 2 = deep
            // tile (crisp stepped water). Tiles (>0) get the hole cut.
            let flag = match key {
                DrawKey::Chunk(_) => 0.0f32,
                DrawKey::Tile(k) if k.deep => 2.0,
                DrawKey::Tile(_) => 1.0,
            };
            let data = [off.x as f32, off.y as f32, off.z as f32, flag];
            self.queue.write_buffer(
                &self.tiles_buf,
                slot as u64 * TILE_UNIFORM_STRIDE,
                bytemuck::bytes_of(&data),
            );
            draws.push((key, slot as u32));
        }
        self.evict();

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("terrain pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.002,
                            g: 0.003,
                            b: 0.008,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        // reversed-Z: the far plane is 0
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            for (key, slot) in &draws {
                let tile = match key {
                    DrawKey::Tile(k) => &self.cache[k],
                    DrawKey::Chunk(k) => &self.chunk_cache[k],
                };
                pass.set_bind_group(0, &self.bind_group, &[*slot * TILE_UNIFORM_STRIDE as u32]);
                pass.set_vertex_buffer(0, tile.vertex_buf.slice(..));
                pass.set_index_buffer(tile.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..tile.index_count, 0, 0..1);
            }
            // sky fills whatever the terrain left at the far plane
            pass.set_pipeline(&self.sky_pipeline);
            pass.set_bind_group(0, &self.bind_group, &[0]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        draws.len()
    }

    fn upload(&self, mesh: TileMesh) -> GpuTile {
        GpuTile {
            origin_km: mesh.origin_km,
            vertex_buf: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tile vb"),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            index_buf: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tile ib"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            index_count: mesh.indices.len() as u32,
            last_used: self.frame_counter,
        }
    }

    fn evict(&mut self) {
        let cutoff = self.frame_counter.saturating_sub(120);
        if self.cache.len() > 1500 {
            self.cache.retain(|_, t| t.last_used >= cutoff);
        }
        if self.chunk_cache.len() > 2500 {
            self.chunk_cache.retain(|_, t| t.last_used >= cutoff);
        }
    }

    /// Render one frame offscreen and save it as a PNG (no window required).
    pub fn capture(
        &mut self,
        planet: &Planet,
        camera: &Camera,
        edits: &Edits,
        path: &str,
    ) -> Result<usize> {
        let (w, h) = self.size;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let n_tiles = self.draw(&view, planet, camera, edits);

        let bytes_per_row = (w * 4 + 255) / 256 * 256;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bytes_per_row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow::anyhow!("device poll: {e:?}"))?;
        let data = slice
            .get_mapped_range()
            .map_err(|e| anyhow::anyhow!("map readback: {e:?}"))?;

        // strip row padding; swizzle BGRA -> RGBA if needed
        let bgra = matches!(self.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for row in 0..h {
            let start = (row * bytes_per_row) as usize;
            for px in 0..w as usize {
                let p = &data[start + px * 4..start + px * 4 + 4];
                if bgra {
                    pixels.extend_from_slice(&[p[2], p[1], p[0], 255]);
                } else {
                    pixels.extend_from_slice(&[p[0], p[1], p[2], 255]);
                }
            }
        }
        drop(data);
        readback.unmap();

        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&pixels)?;
        Ok(n_tiles)
    }
}

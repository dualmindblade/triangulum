//! wgpu renderer: pipeline setup, per-tile GPU buffers, frame drawing, and an
//! offscreen capture path (renders to a texture and saves a PNG — no window).

use crate::camera::Camera;
use crate::planet::Planet;
use crate::terrain::{build_tile, select_tiles, TileKey, TileMesh, Vertex};
use crate::voxel::{build_chunk, select_chunks, ChunkKey, Edits, Torches};
use anyhow::Result;
use glam::{DVec3, Mat4};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const VOXEL_MAX_ALT_KM: f64 = 2.5;

/// Screen-space-error target for tile selection. The geomorph bands are
/// derived from this: with tau = err_target, a tile of size S is selected
/// while its center distance is in [S/tau, 2S/tau). Morphing must complete
/// before any swap, at every vertex:
///   - a child appearing/vanishing at parent-center dist 2S/tau has its
///     nearest vertex at 2S/tau - sqrt(2)*S  ->  morph_end <= (2/tau - 1.41)*S
///   - a tile must still be UNmorphed where its children hand off: farthest
///     vertex at own-center dist S/tau is at S/tau + 0.71*S
///     ->  morph_start >= (1/tau + 0.71)*S
/// With tau = 0.35 that window is [3.57, 4.30]*S — we use [3.61, 4.26]*S.
/// Cross-level tile edges agree for free: the finer neighbor is fully
/// morphed (= parent geometry) exactly where the coarser one is unmorphed.
const ERR_TARGET: f64 = 0.35;

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
    // xyz = planet center relative to the camera (km) — the geomorph slide
    // direction is radial, and camera-relative space has no planet center
    // otherwise. w = the voxel patch lift (km) for the block rim-sink.
    center: [f32; 4],
    // x = number of active torch lights; rest spare
    misc: [f32; 4],
    // placed-torch point lights: xyz camera-relative (km), w intensity
    lights: [[f32; 4]; MAX_LIGHTS],
}

/// How many placed torches can light a frame at once (nearest win).
pub const MAX_LIGHTS: usize = 16;

struct GpuTile {
    origin_km: DVec3,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,
    last_used: u64,
    bytes: u64,
}

/// VRAM ceiling for cached chunk meshes: past it, least-recently-used
/// chunks are dropped regardless of age. Sized so --patch 2.0 fits with
/// room for the rest of the frame on a 6 GB card.
const CHUNK_VRAM_BUDGET: u64 = 1500 << 20;

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
    /// Player-placed torches — meshed into their chunks, and the nearest
    /// few become real point lights each frame. Kept in sync by the app.
    pub torches: Torches,
    /// Day/night cycle: seconds per full day. 0 disables the cycle (the
    /// sun follows the camera — always noon where you are). A pinned
    /// `sun_dir` always wins. The sun stands still in space while the
    /// planet turns, so local time depends on longitude and the
    /// terminator is visible from orbit.
    pub day_len_s: f64,
    /// Longitude (radians) that starts the cycle at mid-morning.
    pub sun_ref_lon: f64,
    start: std::time::Instant,
    /// Voxel patch radius multiplier (--patch): 1.0 = the classic
    /// 200–500 m disc, 2.0 = twice the radius (4x the chunks — streaming
    /// makes that affordable).
    pub patch_scale: f64,
    /// Chunks being built on background threads right now. A chunk whose
    /// key is removed from here (edit invalidation) has its late result
    /// dropped on arrival and is rebuilt fresh.
    chunk_pending: HashSet<ChunkKey>,
    chunk_tx: mpsc::Sender<(ChunkKey, TileMesh)>,
    chunk_rx: mpsc::Receiver<(ChunkKey, TileMesh)>,
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
                        min_binding_size: wgpu::BufferSize::new(32),
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
                        size: wgpu::BufferSize::new(32),
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
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4, 4 => Float32, 5 => Float32, 6 => Float32],
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
        let (tx, rx) = mpsc::channel();
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
            torches: Torches::default(),
            day_len_s: 0.0,
            sun_ref_lon: 0.0,
            start: std::time::Instant::now(),
            patch_scale: 1.0,
            chunk_pending: HashSet::new(),
            chunk_tx: tx,
            chunk_rx: rx,
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
    /// In-flight builds of these chunks are orphaned: removing the key from
    /// the pending set makes their (stale) results get dropped on arrival.
    pub fn invalidate_chunks(&mut self, keys: &[ChunkKey]) {
        for k in keys {
            self.chunk_cache.remove(k);
            self.chunk_pending.remove(k);
        }
    }

    /// Collect finished background chunk builds (non-blocking).
    fn drain_chunks(&mut self) {
        while let Ok((k, mesh)) = self.chunk_rx.try_recv() {
            if self.chunk_pending.remove(&k) {
                let gpu = self.upload(mesh);
                self.chunk_cache.insert(k, gpu);
            }
        }
    }

    /// Draw one frame into `target`. Returns the number of tiles drawn.
    pub fn draw(
        &mut self,
        target: &wgpu::TextureView,
        planet: &Arc<Planet>,
        camera: &Camera,
        edits: &Edits,
    ) -> usize {
        self.frame_counter += 1;
        let cam_pos = camera.position();
        let voxel_radius_m =
            (200.0 + (VOXEL_MAX_ALT_KM - camera.altitude_km).max(0.0) * 120.0)
                * self.patch_scale;
        let focus = (camera.altitude_km < VOXEL_MAX_ALT_KM)
            .then(|| (cam_pos.normalize(), voxel_radius_m / 1000.0 + 0.2));
        let keys = select_tiles(cam_pos, planet.radius_km, ERR_TARGET, focus);

        // upload globals (camera-relative view-projection, f64 -> f32 at the end)
        let vp = camera.view_proj(self.size.0 as f64 / self.size.1 as f64);
        let vp32 = Mat4::from_cols_array(&vp.to_cols_array().map(|x| x as f32));
        let t_s = self.start.elapsed().as_secs_f64();
        let sun = self.sun_dir.unwrap_or_else(|| {
            if self.day_len_s > 0.0 {
                // the sun hangs in space and the planet turns under it:
                // start ~mid-morning at the reference longitude, sweep
                // westward, gentle 10 deg declination for softer noons
                let lon = self.sun_ref_lon + 0.7
                    - t_s / self.day_len_s * std::f64::consts::TAU;
                let lat = 10f64.to_radians();
                DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
            } else {
                cam_pos.normalize()
            }
        });
        let inv32 = Mat4::from_cols_array(&vp.inverse().to_cols_array().map(|x| x as f32));
        let up = cam_pos.normalize();
        let cam_h_km = (cam_pos.length() - planet.radius_km).max(0.0);
        let lift = crate::voxel::lift_km(self.exaggeration);

        // placed torches: the nearest MAX_LIGHTS become point lights. Their
        // exact height needs a terrain sample, so rank cheaply by direction
        // first and only sample the winners.
        let mut lights = [[0.0f32; 4]; MAX_LIGHTS];
        let mut n_lights = 0usize;
        if camera.altitude_km < VOXEL_MAX_ALT_KM && !self.torches.is_empty() {
            let nn = crate::voxel::COLUMNS_PER_FACE as f64;
            let mut ranked: Vec<(f64, DVec3)> = self
                .torches
                .iter()
                .map(|&(f, ci, cj)| {
                    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
                    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
                    let dir = crate::planet::face_dir(f as usize, u, v);
                    ((dir * cam_pos.length() - cam_pos).length_squared(), dir)
                })
                .collect();
            ranked.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            for &(_, dir) in ranked.iter().take(MAX_LIGHTS) {
                let top = crate::voxel::surface_height_km(planet, edits, dir, self.exaggeration);
                let pos = dir * (planet.radius_km + top + 0.55 * crate::voxel::VOXEL_KM * self.exaggeration)
                    - cam_pos;
                // each flame breathes on its own phase
                let flicker =
                    (0.88 + 0.18 * (t_s * 9.0 + n_lights as f64 * 2.4).sin()) as f32;
                lights[n_lights] = [pos.x as f32, pos.y as f32, pos.z as f32, flicker];
                n_lights += 1;
            }
        }
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

        // near the ground, stream voxel chunks around the camera footprint:
        // finished background builds land this frame, missing chunks are
        // queued (nearest first — select_chunks sorts by distance), and the
        // frame renders whatever is built RIGHT NOW. The heightfield hole
        // below only opens over guaranteed coverage, so an unbuilt chunk
        // shows mesh terrain, never a hole to the sky.
        let mut chunk_keys: Vec<ChunkKey> = Vec::new();
        let mut unbuilt_min_km = f64::INFINITY;
        if camera.altitude_km < VOXEL_MAX_ALT_KM {
            self.drain_chunks();
            chunk_keys = select_chunks(cam_pos, planet, voxel_radius_m);
            let center = cam_pos.normalize() * planet.radius_km;
            let nn = crate::voxel::COLUMNS_PER_FACE as f64;
            for k in &chunk_keys {
                if self.chunk_cache.contains_key(k) {
                    continue;
                }
                let u = -1.0 + 2.0 * ((k.cx * crate::voxel::CHUNK + 16) as f64 + 0.5) / nn;
                let v = -1.0 + 2.0 * ((k.cy * crate::voxel::CHUNK + 16) as f64 + 0.5) / nn;
                let cdir = crate::planet::face_dir(k.face as usize, u, v);
                unbuilt_min_km = unbuilt_min_km.min((cdir * planet.radius_km - center).length());
                if self.chunk_pending.insert(*k) {
                    let tx = self.chunk_tx.clone();
                    let planet = Arc::clone(planet);
                    let edits = edits.clone();
                    let torches = self.torches.clone();
                    let key = *k;
                    rayon::spawn(move || {
                        let mesh = build_chunk(&planet, &edits, &torches, key, exagg);
                        let _ = tx.send((key, mesh));
                    });
                }
            }
            chunk_keys.retain(|k| self.chunk_cache.contains_key(k));
        }

        // the heightfield hole: never larger than the classic safe radius,
        // and never reaching past the nearest chunk that isn't built yet
        // (minus a chunk's worth of margin). Fully built -> full hole;
        // freshly teleported -> no hole, mesh shows while blocks stream in.
        let mut hole = [0.0f32; 4];
        let mut hole_up = [0.0f32; 4];
        if focus.is_some() {
            let r_km = crate::voxel::safe_hole_radius_km(voxel_radius_m)
                .min((unbuilt_min_km - 0.096).max(0.0));
            // center + up are set whenever the patch exists (the rim-sink
            // needs them even while the hole itself is still closed)
            let hup = cam_pos.normalize();
            let hc = -hup * camera.altitude_km; // camera ground point, camera-relative
            hole = [hc.x as f32, hc.y as f32, hc.z as f32, r_km.max(0.0) as f32];
            hole_up = [hup.x as f32, hup.y as f32, hup.z as f32, 0.0];
        }
        hole_up[3] = if self.underwater { 1.0 } else { 0.0 };

        let globals = Globals {
            view_proj: vp32.to_cols_array_2d(),
            inv_view_proj: inv32.to_cols_array_2d(),
            sun_dir: [sun.x as f32, sun.y as f32, sun.z as f32, 0.0],
            hole,
            hole_up,
            sky: [up.x as f32, up.y as f32, up.z as f32, cam_h_km as f32],
            center: [
                (-cam_pos.x) as f32,
                (-cam_pos.y) as f32,
                (-cam_pos.z) as f32,
                lift as f32,
            ],
            misc: [
                n_lights as f32,
                (t_s % 3600.0) as f32,
                // the rim-sink band follows the SELECTED patch radius, not
                // the (coverage-dependent) hole, so the patch edge stays
                // feathered even while chunks are still streaming in
                (voxel_radius_m / 1000.0) as f32,
                0.0,
            ],
            lights,
        };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

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
            // geomorph band (see ERR_TARGET) — zero for chunks (no morph).
            // Dev switches for verification/tuning: TRI_NO_MORPH renders raw
            // levels (pops back), TRI_FORCE_MORPH renders every tile as its
            // parent's geometry (a sign/scale bug shows as spikes at once).
            let morph = match key {
                DrawKey::Tile(k)
                    if k.level > 0 && std::env::var_os("TRI_NO_MORPH").is_none() =>
                {
                    if std::env::var_os("TRI_FORCE_MORPH").is_some() {
                        [0.0001, 0.0002, 0.0, 0.0]
                    } else {
                        let s = k.size_km(planet.radius_km);
                        let start = s * (1.0 / ERR_TARGET + 0.75);
                        let end = s * (2.0 / ERR_TARGET - 1.45);
                        [start as f32, end as f32, 0.0, 0.0]
                    }
                }
                _ => [0.0f32; 4],
            };
            let data = [
                off.x as f32,
                off.y as f32,
                off.z as f32,
                flag,
                morph[0],
                morph[1],
                morph[2],
                morph[3],
            ];
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
        let bytes = (mesh.vertices.len() * std::mem::size_of::<Vertex>()
            + mesh.indices.len() * 4) as u64;
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
            bytes,
        }
    }

    fn evict(&mut self) {
        let cutoff = self.frame_counter.saturating_sub(120);
        if self.cache.len() > 1500 {
            self.cache.retain(|_, t| t.last_used >= cutoff);
        }
        // sized for --patch 2.0 (a ~1 km disc is ~1600 chunks)
        if self.chunk_cache.len() > 4000 {
            self.chunk_cache.retain(|_, t| t.last_used >= cutoff);
        }
        // hard VRAM budget: newest chunks win, the LRU tail is dropped —
        // without this, fast flight at big patch sizes accumulates buffers
        // faster than the age-based eviction can retire them (OOM)
        let total: u64 = self.chunk_cache.values().map(|t| t.bytes).sum();
        if total > CHUNK_VRAM_BUDGET {
            let mut by_age: Vec<(u64, ChunkKey, u64)> = self
                .chunk_cache
                .iter()
                .map(|(k, t)| (t.last_used, *k, t.bytes))
                .collect();
            by_age.sort_unstable_by(|a, b| b.0.cmp(&a.0)); // newest first
            let mut acc = 0u64;
            let mut keep = HashSet::new();
            for (_, k, b) in by_age {
                if acc + b <= CHUNK_VRAM_BUDGET {
                    acc += b;
                    keep.insert(k);
                }
            }
            self.chunk_cache.retain(|k, _| keep.contains(k));
        }
    }

    /// Render one frame offscreen and save it as a PNG (no window required).
    pub fn capture(
        &mut self,
        planet: &Arc<Planet>,
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
        let mut n_tiles = self.draw(&view, planet, camera, edits);
        // a screenshot is a complete frame: block (bounded) until every
        // streamed chunk has landed, then draw again with full coverage
        let mut waited = false;
        for _ in 0..8192 {
            if self.chunk_pending.is_empty() {
                break;
            }
            match self.chunk_rx.recv_timeout(std::time::Duration::from_secs(30)) {
                Ok((k, mesh)) => {
                    if self.chunk_pending.remove(&k) {
                        let gpu = self.upload(mesh);
                        self.chunk_cache.insert(k, gpu);
                    }
                    waited = true;
                }
                Err(_) => break,
            }
        }
        if waited {
            n_tiles = self.draw(&view, planet, camera, edits);
        }

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

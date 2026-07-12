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

#[derive(Clone, Copy, Debug)]
pub struct SunState {
    pub dir: DVec3,
    pub day_time_s: f64,
}

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
/// morphed to the same parent-triangle height exactly where the coarser one
/// is unmorphed. The radial-only slide differs from the parent's 3-D chord by
/// at most 0.13 m in the hostile level-9 V-6 probes (well below its swap
/// distance); an exact vec3 target was not worth widening every vertex.
const ERR_TARGET: f64 = 0.35;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DrawKey {
    Tile(TileKey),
    Chunk(ChunkKey),
}

const TILE_UNIFORM_STRIDE: u64 = 256; // min dynamic-offset alignment
// Per-frame draw slots (tiles + voxel chunks share this pool). Sized well
// above observed peaks (~3.7k draws at --patch 2.0, low altitude) so the
// near-field chunks are never truncated away; the uniform buffer is
// MAX_TILES * 256 B (2 MiB at 8192).
const MAX_TILES: usize = 8192;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    // inverse view-projection: the sky pass unprojects screen corners into
    // world-space view rays (camera-relative space, so the eye is origin)
    inv_view_proj: [[f32; 4]; 4],
    sun_dir: [f32; 4],
    // xyz = unit direction to the moon (deterministic anti-solar + tilt, so
    // screenshots stay reproducible); w spare. Drives the night moon disc and
    // the cool moonlight lift on night terrain.
    moon_dir: [f32; 4],
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
    // weather at the camera (WEATHER.md Layer 3): x cloud cover 0..1,
    // y precip 0..1, z snow fraction 0..1, w air temp (C)
    weather: [f32; 4],
    // xyz = cloud-domain drift (the synoptic advection, precomputed on the
    // CPU so the shader's deck scrolls with the same wind as the field),
    // w = fraction of direct sun surviving full overcast
    weather2: [f32; 4],
    // x = rain ground-darkening cap, y = full-dusting cold depth (C),
    // z = low cloud-shell altitude (km), w = ground/orbit handoff height (km)
    weather3: [f32; 4],
    // xyz = instantaneous wind (km/s, camera-relative frame) for particle
    // slant; w = absolute weather clock modulo one hour for particle motion
    weather4: [f32; 4],
    // x/y = middle/high cloud-shell altitudes (km), z = active layer count,
    // w = hard orbital cloud-opacity cap (WEATHER.md W3)
    weather5: [f32; 4],
    // xyz = low/mid/high shell noise scales, w = D-8 rain crevice bias
    weather6: [f32; 4],
    // xyz = low/mid/high shell density multipliers; w spare
    weather7: [f32; 4],
    // premultiplied procedural seeds. xyz are the karst breach hint (V-10):
    // low 32 bits of (seed+K).wrapping_mul(0x9E37_79B1) for K = 40961
    // (region gate), 31337 (tube n1), 51413 (tube n2); w is the independent
    // clouds-v2 layout seed (+70001). Shader u32 hashes retain exact low bits.
    karst: [u32; 4],
    // dusting-dither anchor: planet-centered f32 positions quantize at
    // ~0.24 m, so noise fed raw wp renders 25 cm plateaus that crawl with
    // the camera (Andrew's checkerboard, 60.74 89.68). The CPU snaps the
    // camera's f64 planet position to the dither lattice (40 cells/km):
    // danchor_cell = the exact lattice cell (integers), danchor.xyz = that
    // corner relative to the camera (<= 25 m, so f32-precise). The shader
    // evaluates dither noise as danchor_cell + (rel - danchor)*40 - full
    // precision near the eye, world-stable everywhere.
    danchor: [f32; 4],
    danchor_cell: [i32; 4],
    // placed-torch point lights: xyz camera-relative (km), w intensity
    lights: [[f32; 4]; MAX_LIGHTS],
}

/// How many placed torches can light a frame at once (nearest win).
pub const MAX_LIGHTS: usize = 16;

fn torch_phase(face: u8, ci: u64, cj: u64) -> f64 {
    let mut x = ci
        ^ cj.rotate_left(21)
        ^ (face as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x as f64 / u64::MAX as f64) * std::f64::consts::TAU
}

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
    precip_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    tiles_buf: wgpu::Buffer,
    depth: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    cache: HashMap<TileKey, GpuTile>,
    chunk_cache: HashMap<ChunkKey, GpuTile>,
    frame_counter: u64,
    /// Frame cadence instrumentation: wall-clock intervals between draw()
    /// calls and the CPU cost of each draw body (encode+submit), last ~4 s.
    /// The title-bar HUD and shot sidecars read frame_stats() from these —
    /// an objective framerate record instead of "feels smooth".
    /// True when this frame deferred tile builds under the per-frame
    /// budget - captures loop draw until it clears so screenshots stay
    /// byte-deterministic.
    pub tiles_deferred: bool,
    frame_mark: Option<std::time::Instant>,
    frame_intervals_ms: std::collections::VecDeque<f32>,
    draw_cost_ms: std::collections::VecDeque<f32>,
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
    /// Base deterministic simulation clock. Sun/day cycle and non-weather
    /// animation read this directly.
    render_time_s: f64,
    /// Weather is the same deterministic clock plus a seek offset. Replaying
    /// an absolute storm time therefore does not move the sun/day phase.
    weather_time_offset_s: f64,
    /// Voxel patch radius multiplier (--patch): 1.0 = the classic
    /// 200–500 m disc, 2.0 = twice the radius (4x the chunks — streaming
    /// makes that affordable).
    pub patch_scale: f64,
    /// Master switch for the voxel near-field (`voxels off` in scripts,
    /// --no-voxels). False = pure heightfield-mesh render: no chunk
    /// streaming, no hole cut, no rim sink. The mesh keeps its focus
    /// refinement so this shows the BEST mesh-only frame — the sync-diff
    /// harness diffs it against the normal render to measure exactly what
    /// appearance the voxel patch changes.
    pub voxels_on: bool,
    /// Chunks being built on background threads right now, mapped to the
    /// epoch of the request that spawned them. A build's result is accepted
    /// only if its epoch still matches (see `chunk_epoch`): invalidation
    /// removes the key, a fresh request re-inserts it with a NEW epoch, so a
    /// stale in-flight build (older epoch) is dropped on arrival instead of
    /// racing the fresh one and winning — which left edited blocks/torches
    /// visually stale.
    chunk_pending: HashMap<ChunkKey, u64>,
    /// Edited chunks whose cached mesh is now out of date but is kept on
    /// screen until the rebuild lands — so a place/break never flashes the
    /// heightfield (or a hole to the sky) where the block just changed. Draw
    /// re-queues these; drain swaps the mesh in place and clears the flag.
    chunk_stale: HashSet<ChunkKey>,
    /// Monotonic request counter: every queued chunk build carries the epoch
    /// it was requested at.
    chunk_epoch: u64,
    chunk_tx: mpsc::Sender<(ChunkKey, u64, TileMesh)>,
    chunk_rx: mpsc::Receiver<(ChunkKey, u64, TileMesh)>,
    /// Async TILE builds, mirroring the chunk pipeline: LOD ring re-splits
    /// no longer build synchronously inside draw (the single-frame stall
    /// Andrew isolated with his zoom-only experiment). Selected-but-pending
    /// tiles draw a cached ancestor or their four cached children instead.
    tile_pending: HashMap<TileKey, u64>,
    tile_epoch: u64,
    tile_tx: mpsc::Sender<(TileKey, u64, TileMesh)>,
    tile_rx: mpsc::Receiver<(TileKey, u64, TileMesh)>,
    /// Immutable world-state snapshots shared by in-flight chunk builders.
    /// Refreshed only when edits/torches change, so queuing many chunks does
    /// not clone the whole edited world per request.
    edit_snapshot: Arc<Edits>,
    torch_snapshot: Arc<Torches>,
    /// Living weather (WEATHER.md). None = no weather.bin, sky stays clear.
    pub weather_field: Option<crate::weather::WeatherField>,
    pub weather_tuning: crate::weather::WeatherTuning,
    /// Master switch (--weather off, `weather off` in scripts).
    pub weather_on: bool,
    /// Some((cover, precip)) pins the sky for art shots and regression
    /// scripts, overriding the live field (like `sun` pins the sun).
    pub weather_pin: Option<(f32, f32)>,
    /// Last frame's camera weather sample — photo sidecars record it so a
    /// storm shot is a coordinate you can teleport back into.
    pub last_weather: crate::weather::Weather,
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
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4, 4 => Float32, 5 => Float32, 6 => Float32, 7 => Float32, 8 => Snorm8x4],
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
        let (ttx, trx) = mpsc::channel();
        // precipitation: instanced quads with no vertex buffers (positions
        // are hashed from the instance index in vs_precip), alpha-blended,
        // depth-TESTED against the terrain but never written
        let precip_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("precip"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_precip"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_precip"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            device,
            queue,
            pipeline,
            sky_pipeline,
            precip_pipeline,
            bind_group,
            globals_buf,
            tiles_buf,
            depth,
            size,
            format,
            cache: HashMap::new(),
            chunk_cache: HashMap::new(),
            chunk_stale: HashSet::new(),
            frame_counter: 0,
            tiles_deferred: false,
            frame_mark: None,
            frame_intervals_ms: std::collections::VecDeque::new(),
            draw_cost_ms: std::collections::VecDeque::new(),
            exaggeration,
            sun_dir: None,
            underwater: false,
            torches: Torches::default(),
            day_len_s: 0.0,
            sun_ref_lon: 0.0,
            render_time_s: 0.0,
            weather_time_offset_s: 0.0,
            patch_scale: 1.0,
            voxels_on: true,
            chunk_pending: HashMap::new(),
            chunk_epoch: 0,
            chunk_tx: tx,
            chunk_rx: rx,
            tile_pending: HashMap::new(),
            tile_epoch: 0,
            tile_tx: ttx,
            tile_rx: trx,
            edit_snapshot: Arc::new(Edits::default()),
            torch_snapshot: Arc::new(Torches::default()),
            weather_field: None,
            weather_tuning: crate::weather::WeatherTuning::default(),
            weather_on: true,
            weather_pin: None,
            last_weather: crate::weather::Weather::default(),
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

    pub fn set_render_time_s(&mut self, t_s: f64) {
        if t_s.is_finite() {
            self.render_time_s = t_s.max(0.0);
        }
    }

    /// The deterministic simulation/render clock used by the sun and general
    /// animation. Weather normally follows it but can carry a replay offset.
    pub fn render_time_s(&self) -> f64 {
        self.render_time_s
    }

    /// Absolute deterministic weather time. This is the time recorded in
    /// photo sidecars and fed to every climatology/synoptic/presentation path.
    pub fn weather_time_s(&self) -> f64 {
        self.render_time_s + self.weather_time_offset_s
    }

    /// Seek weather without changing the sun/day-cycle clock. The offset is
    /// retained as simulation time advances, so a restored front keeps moving.
    pub fn set_weather_time_s(&mut self, t_s: f64) {
        if t_s.is_finite() {
            self.weather_time_offset_s = t_s.max(0.0) - self.render_time_s;
        }
    }

    pub fn advance_render_time_s(&mut self, dt_s: f64) {
        if dt_s.is_finite() && dt_s > 0.0 {
            self.render_time_s += dt_s;
        }
    }

    pub fn refresh_edits_snapshot(&mut self, edits: &Edits) {
        self.edit_snapshot = Arc::new(edits.clone());
    }

    pub fn refresh_world_snapshot(&mut self, edits: &Edits) {
        self.refresh_edits_snapshot(edits);
        self.torch_snapshot = Arc::new(self.torches.clone());
    }

    pub fn set_torches(&mut self, torches: Torches) {
        self.torch_snapshot = Arc::new(torches.clone());
        self.torches = torches;
    }

    pub fn sun_state(&self, cam_pos: DVec3) -> SunState {
        let t_s = self.render_time_s;
        let dir = self.sun_dir.unwrap_or_else(|| {
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
        SunState {
            dir,
            day_time_s: if self.day_len_s > 0.0 {
                t_s.rem_euclid(self.day_len_s)
            } else {
                0.0
            },
        }
    }

    /// Jump the day/night cycle so the CURRENT moment sits `t_s` seconds
    /// into the day — the photo map's optional "restore time of day".
    /// No-op when the sun is pinned or the cycle is off. The weather clock is
    /// preserved; recorded weather has its own absolute restore coordinate.
    pub fn set_day_time_s(&mut self, t_s: f64) {
        if self.day_len_s > 0.0 && self.sun_dir.is_none() {
            let weather_t_s = self.weather_time_s();
            self.render_time_s = t_s.rem_euclid(self.day_len_s);
            self.weather_time_offset_s = weather_t_s - self.render_time_s;
        }
    }

    /// Drop cached chunk meshes (after edits) so they rebuild next frame.
    /// In-flight builds of these chunks are orphaned: removing the key from
    /// the pending set makes their (stale) results get dropped on arrival.
    pub fn invalidate_chunks(&mut self, keys: &[ChunkKey]) {
        for k in keys {
            // cancel any in-flight build (its result is rejected by epoch),
            // then either mark the cached mesh stale (keep drawing it until the
            // rebuild lands — no flash) or, if we never had it, drop it so draw
            // builds it fresh
            self.chunk_pending.remove(k);
            if self.chunk_cache.contains_key(k) {
                self.chunk_stale.insert(*k);
            } else {
                self.chunk_cache.remove(k);
            }
        }
    }

    /// Collect finished background chunk builds (non-blocking). A result is
    /// accepted only if its epoch still matches the pending request: a build
    /// left over from before an invalidation (stale edits) carries an older
    /// epoch and is dropped, so the fresh rebuild is the one that lands.
    fn drain_chunks(&mut self) {
        while let Ok((k, epoch, mesh)) = self.chunk_rx.try_recv() {
            if self.chunk_pending.get(&k) == Some(&epoch) {
                self.chunk_pending.remove(&k);
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
        let draw_start = std::time::Instant::now();
        if let Some(prev) = self.frame_mark.replace(draw_start) {
            let dt_ms = draw_start.duration_since(prev).as_secs_f32() * 1000.0;
            // a pause (alt-tab, teleport prompt, capture wait) is not a
            // frame-time signal — drop outliers past half a second
            if dt_ms < 500.0 {
                self.frame_intervals_ms.push_back(dt_ms);
                if self.frame_intervals_ms.len() > 240 {
                    self.frame_intervals_ms.pop_front();
                }
            }
        }
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
        let t_s = self.render_time_s;
        let weather_t_s = self.weather_time_s();
        let sun = self.sun_state(cam_pos).dir;
        // the moon rides opposite the sun (rises at sunset, near-full), tilted
        // ~18 deg off the solar path about the world X axis so it isn't a
        // mirror image of the sun and clears the horizon on its own arc. Tied
        // to the sun, so a pinned sun pins the moon (reproducible captures).
        let moon = {
            let a: f64 = 18f64.to_radians();
            let (s, c) = (a.sin(), a.cos());
            let m = -sun;
            DVec3::new(m.x, m.y * c - m.z * s, m.y * s + m.z * c).normalize()
        };
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
            let mut ranked: Vec<(f64, (u8, u64, u64), DVec3)> = self
                .torches
                .iter()
                .map(|&(f, ci, cj)| {
                    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
                    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
                    let dir = crate::planet::face_dir(f as usize, u, v);
                    ((dir * cam_pos.length() - cam_pos).length_squared(), (f, ci, cj), dir)
                })
                .collect();
            ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
            for &(_, (f, ci, cj), dir) in ranked.iter().take(MAX_LIGHTS) {
                let top = crate::voxel::surface_height_km(planet, edits, dir, self.exaggeration);
                let pos = dir * (planet.radius_km + top + 0.55 * crate::voxel::VOXEL_KM * self.exaggeration)
                    - cam_pos;
                // each flame breathes on its own phase
                let flicker =
                    (0.88 + 0.18 * (t_s * 9.0 + torch_phase(f, ci, cj)).sin()) as f32;
                lights[n_lights] = [pos.x as f32, pos.y as f32, pos.z as f32, flicker];
                n_lights += 1;
            }
        }
        let exagg = self.exaggeration;
        // build missing tiles in parallel (rayon), then upload sequentially
        // accept tile builds that finished on background threads
        while let Ok((k, epoch, mesh)) = self.tile_rx.try_recv() {
            if self.tile_pending.get(&k) == Some(&epoch) {
                self.tile_pending.remove(&k);
                if !self.cache.contains_key(&k) {
                    let gpu = self.upload(mesh);
                    self.cache.insert(k, gpu);
                }
            }
        }
        let missing: Vec<TileKey> = keys
            .iter()
            .filter(|k| !self.cache.contains_key(k))
            .copied()
            .collect();
        // a missing tile is COVERED if a cached ancestor or all four cached
        // children can stand in (geomorph makes either near-identical at
        // the swap distance). Covered tiles build asynchronously; uncovered
        // ones (fresh teleports, horizon reveals) must build this frame.
        let covered = |cache: &HashMap<TileKey, GpuTile>, k: &TileKey| -> bool {
            let mut cur = *k;
            while cur.level > 0 {
                cur = TileKey {
                    face: cur.face,
                    level: cur.level - 1,
                    ix: cur.ix / 2,
                    iy: cur.iy / 2,
                    deep: false,
                };
                if cache.contains_key(&cur) {
                    return true;
                }
            }
            let (cx, cy) = (k.ix as u32 * 2, k.iy as u32 * 2);
            if cx < u16::MAX as u32 && cy < u16::MAX as u32 {
                let child = |dx: u16, dy: u16| TileKey {
                    face: k.face,
                    level: k.level + 1,
                    ix: k.ix * 2 + dx,
                    iy: k.iy * 2 + dy,
                    deep: false,
                };
                if (0..4).all(|i| {
                    self_cache_has(cache, child(i % 2, i / 2))
                }) {
                    return true;
                }
            }
            false
        };
        fn self_cache_has(cache: &HashMap<TileKey, GpuTile>, k: TileKey) -> bool {
            cache.contains_key(&k)
        }
        let mut urgent: Vec<TileKey> = Vec::new();
        for k in &missing {
            // DEEP tiles never defer and never accept stand-ins: they exist
            // to match the voxel patch at 12 octaves, and an 8-octave
            // ancestor standing in interpenetrates the blocks - two offset
            // grids on the ground, mesh fragments among voxels, pale water
            // plane bits reading as ground clouds (Andrew's field report,
            // 2026-07-12). They are few (patch-adjacent only).
            if !k.deep && covered(&self.cache, k) {
                if !self.tile_pending.contains_key(k) {
                    let epoch = self.tile_epoch;
                    self.tile_epoch = self.tile_epoch.wrapping_add(1);
                    self.tile_pending.insert(*k, epoch);
                    let tx = self.tile_tx.clone();
                    let planet = Arc::clone(planet);
                    let key = *k;
                    rayon::spawn(move || {
                        let mesh = build_tile(&planet, key, exagg);
                        let _ = tx.send((key, epoch, mesh));
                    });
                }
            } else {
                urgent.push(*k);
            }
        }
        let built: Vec<(TileKey, TileMesh)> = {
            use rayon::prelude::*;
            urgent.par_iter().map(|k| (*k, build_tile(planet, *k, exagg))).collect()
        };
        for (k, mesh) in built {
            let gpu = self.upload(mesh);
            self.cache.insert(k, gpu);
        }
        self.tiles_deferred = keys.iter().any(|k| !self.cache.contains_key(k));
        // parent PREFETCH: ascending selects coarser rings whose ancestors
        // were often never built, and partially-cached children cannot
        // stand in without holes - those tiles went urgent and spiked
        // (Andrew: rare 1 s pauses, mostly while ascending). Speculatively
        // build a few parents of the CURRENT rings per frame in the
        // background so the coarser ring is ready before it is selected.
        let mut prefetched = 0usize;
        for k in &keys {
            if prefetched >= 4 {
                break;
            }
            if k.level == 0 {
                continue;
            }
            let parent = TileKey {
                face: k.face,
                level: k.level - 1,
                ix: k.ix / 2,
                iy: k.iy / 2,
                deep: false,
            };
            if !self.cache.contains_key(&parent) && !self.tile_pending.contains_key(&parent)
            {
                let epoch = self.tile_epoch;
                self.tile_epoch = self.tile_epoch.wrapping_add(1);
                self.tile_pending.insert(parent, epoch);
                let tx = self.tile_tx.clone();
                let planet = Arc::clone(planet);
                rayon::spawn(move || {
                    let mesh = build_tile(&planet, parent, exagg);
                    let _ = tx.send((parent, epoch, mesh));
                });
                prefetched += 1;
            }
        }
        // coverage resolution for the budget: any selected tile still
        // unbuilt draws its nearest CACHED ancestor instead (dedup) - the
        // geomorph makes a parent at swap distance near-identical, so a
        // one-or-two-frame stand-in is invisible. If no ancestor is cached
        // (fresh teleport), build the tile now: correctness over budget.
        let keys: Vec<TileKey> = {
            let mut resolved: Vec<TileKey> = Vec::with_capacity(keys.len());
            let mut seen: HashSet<TileKey> = HashSet::with_capacity(keys.len());
            let mut leftovers: Vec<TileKey> = Vec::new();
            for k in keys {
                if self.cache.contains_key(&k) {
                    if seen.insert(k) {
                        resolved.push(k);
                    }
                    continue;
                }
                // ancestor stand-in (descent case)
                let mut cur = k;
                let mut found = None;
                while cur.level > 0 {
                    cur = TileKey {
                        face: cur.face,
                        level: cur.level - 1,
                        ix: cur.ix / 2,
                        iy: cur.iy / 2,
                        deep: false,
                    };
                    if self.cache.contains_key(&cur) {
                        found = Some(cur);
                        break;
                    }
                }
                if let Some(a) = found {
                    if seen.insert(a) {
                        resolved.push(a);
                    }
                    continue;
                }
                // four-children stand-in (ascent case: finer rings are
                // cached, the coarser ancestor never was)
                let child = |dx: u16, dy: u16| TileKey {
                    face: k.face,
                    level: k.level + 1,
                    ix: k.ix * 2 + dx,
                    iy: k.iy * 2 + dy,
                    deep: false,
                };
                let kids = [child(0, 0), child(1, 0), child(0, 1), child(1, 1)];
                if kids.iter().all(|c| self.cache.contains_key(c)) {
                    for c in kids {
                        if seen.insert(c) {
                            resolved.push(c);
                        }
                    }
                    continue;
                }
                // nothing can stand in (rare; urgent builds above should
                // cover it) - collect for ONE parallel batch below instead
                // of building sequentially here (a sequential burst of
                // these was the multi-second frame class)
                leftovers.push(k);
                if seen.insert(k) {
                    resolved.push(k);
                }
            }
            if !leftovers.is_empty() {
                use rayon::prelude::*;
                let built: Vec<(TileKey, TileMesh)> = leftovers
                    .par_iter()
                    .map(|k| (*k, build_tile(planet, *k, exagg)))
                    .collect();
                for (k, mesh) in built {
                    let gpu = self.upload(mesh);
                    self.cache.insert(k, gpu);
                }
            }
            resolved
        };

        // near the ground, stream voxel chunks around the camera footprint:
        // finished background builds land this frame, missing chunks are
        // queued (nearest first — select_chunks sorts by distance), and the
        // frame renders whatever is built RIGHT NOW. The heightfield hole
        // below only opens over guaranteed coverage, so an unbuilt chunk
        // shows mesh terrain, never a hole to the sky.
        let mut chunk_keys: Vec<ChunkKey> = Vec::new();
        let mut unbuilt_min_km = f64::INFINITY;
        if self.voxels_on && camera.altitude_km < VOXEL_MAX_ALT_KM {
            self.drain_chunks();
            chunk_keys = select_chunks(cam_pos, planet, voxel_radius_m);
            let center = cam_pos.normalize() * planet.radius_km;
            let nn = crate::voxel::COLUMNS_PER_FACE as f64;
            for k in &chunk_keys {
                let cached = self.chunk_cache.contains_key(k);
                let stale = self.chunk_stale.contains(k);
                if cached && !stale {
                    continue; // current mesh already on screen
                }
                // a chunk with no built mesh yet: its area must stay heightfield
                // (the hole is clamped to unbuilt_min_km). A stale chunk still
                // has its old mesh drawn, so it does NOT count as unbuilt —
                // coverage holds and there is no hole/flash while it rebuilds.
                if !cached {
                    let u = -1.0 + 2.0 * ((k.cx * crate::voxel::CHUNK + 16) as f64 + 0.5) / nn;
                    let v = -1.0 + 2.0 * ((k.cy * crate::voxel::CHUNK + 16) as f64 + 0.5) / nn;
                    let cdir = crate::planet::face_dir(k.face as usize, u, v);
                    unbuilt_min_km =
                        unbuilt_min_km.min((cdir * planet.radius_km - center).length());
                }
                if !self.chunk_pending.contains_key(k) {
                    let epoch = self.chunk_epoch;
                    self.chunk_epoch = self.chunk_epoch.wrapping_add(1);
                    self.chunk_pending.insert(*k, epoch);
                    self.chunk_stale.remove(k); // rebuilt with the current edits
                    let tx = self.chunk_tx.clone();
                    let planet = Arc::clone(planet);
                    let edits = Arc::clone(&self.edit_snapshot);
                    let torches = Arc::clone(&self.torch_snapshot);
                    let key = *k;
                    rayon::spawn(move || {
                        let mesh = build_chunk(&planet, edits.as_ref(), torches.as_ref(), key, exagg);
                        let _ = tx.send((key, epoch, mesh));
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
        if focus.is_some() && self.voxels_on {
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

        // living weather: ONE field sample per frame, at the camera. The
        // shaders add per-pixel detail from the same uniforms, so weather
        // stays a pure function of (seed, position, weather time) — the
        // determinism contract in WEATHER.md.
        let wx = if self.weather_on
            && let Some(field) = &self.weather_field
        {
            let mut w = crate::weather::weather_at(
                field,
                planet,
                cam_pos.normalize(),
                weather_t_s,
                self.day_len_s,
                &self.weather_tuning,
            );
            if let Some((c, p)) = self.weather_pin {
                w.cloud_cover = c as f64;
                w.precip = p as f64;
            }
            w
        } else {
            // temp 999 = the shader sentinel for "weather off": no ground
            // dusting, no darkening (a real 999 C never happens)
            crate::weather::Weather { temp_c: 999.0, ..Default::default() }
        };
        self.last_weather = wx;
        // the cloud deck scrolls with the same advection the field uses:
        // precompute the domain drift here so shader noise and CPU weather
        // agree on where the storm is. The instantaneous wind vector drives
        // particle slant.
        let (drift, wind_kms) = {
            let dirn = cam_pos.normalize();
            let east0 = glam::DVec3::Z.cross(dirn);
            let east =
                if east0.length_squared() < 1e-9 { glam::DVec3::Y } else { east0.normalize() };
            let north = dirn.cross(east);
            let wind = east * wx.wind_e + north * wx.wind_n;
            let scale = self.weather_tuning.synoptic_speed * weather_t_s
                / (1000.0 * planet.radius_km);
            // The camera-local basis above is right IN the weather (it is
            // how the deck scrolls with the local wind) and nonsense from
            // orbit: panning the camera rotates east/north, and a
            // t-proportional offset swinging its direction spun the cloud
            // shells in layer-dependent ways (Austin, 2026-07-12, "moving
            // speeds up the weather sim"). Above the shell handoff the
            // drift blends to a FIXED planetary-east vector (constant in
            // world space, camera-independent): from orbit the whole deck
            // slides steadily east instead; at ground level m = 0 exactly
            // and today's look is untouched. Same fade the orbital cloud
            // composite itself uses.
            let m = {
                let a = self.weather_tuning.shell_alt_km;
                let b = self.weather_tuning.shell_fade_km;
                let t = ((cam_h_km - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            // fixed 6 m/s reference speed: the orbital drift must not read
            // ANYTHING at the camera or the swing just comes back smaller
            let zonal = glam::DVec3::Y * 6.0;
            let advect = wind * (1.0 - m) + zonal * m;
            (advect * scale, wind * 0.001)
        };
        // precipitation particles live in a volume around the camera; none
        // underwater, and they thin away as the camera climbs through the
        // low deck toward the ground/orbit cloud handoff.
        let precip_fade = {
            let t = (cam_h_km - self.weather_tuning.shell_alt_km)
                / (self.weather_tuning.shell_fade_km - self.weather_tuning.shell_alt_km).max(1e-6);
            1.0 - t.clamp(0.0, 1.0)
        };
        // D-8 particle interpolation reuses the same signed sub-raster
        // elevation residual carried to ground fragments. This costs one
        // already-resident bilinear elevation lookup, not another full
        // terrain sample. Snow density stays unchanged; Andrew's verdict is
        // specifically about rain collecting in crevices.
        let particle_precip = if wx.precip > 0.0 && wx.snow_frac < 1.0 {
            let dir = cam_pos.normalize();
            let (face, u, v) = crate::planet::face_from_dir(dir);
            let coarse = planet.elevation(face, u, v) as f64;
            let local = camera.ground_km / self.exaggeration.max(1e-6);
            // Open water has no peak/crevice distinction and therefore stays
            // at the unmodified regional particle rate.
            let concavity = if planet.ocean(face, u, v) > 0.5 {
                0.0
            } else {
                crate::terrain::rain_concavity_proxy(coarse, local) as f64
            };
            let redistribute = 1.0
                + self.weather_tuning.rain_crevice_bias
                    * concavity
                    * (1.0 - wx.snow_frac);
            (wx.precip * redistribute).clamp(0.0, 1.0)
        } else {
            wx.precip
        };
        let n_precip = if self.underwater {
            0u32
        } else {
            (particle_precip * precip_fade * self.weather_tuning.particles_max as f64) as u32
        };

        let globals = Globals {
            view_proj: vp32.to_cols_array_2d(),
            inv_view_proj: inv32.to_cols_array_2d(),
            sun_dir: [sun.x as f32, sun.y as f32, sun.z as f32, 0.0],
            moon_dir: [moon.x as f32, moon.y as f32, moon.z as f32, 0.0],
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
                // elevation of the ground under the camera (km above the
                // sphere) - the weather sample's REFERENCE altitude. The
                // shader lapses pixel temperature from here, not from the
                // eye: lapsing from sky.w made the ground 6.5 C warmer per
                // km of camera CLIMB (Sol review #2 finding 1 - zooming
                // out melted the snow line).
                (cam_h_km - camera.altitude_km).max(0.0) as f32,
            ],
            weather: [
                wx.cloud_cover as f32,
                wx.precip as f32,
                wx.snow_frac as f32,
                wx.temp_c as f32,
            ],
            weather2: [
                drift.x as f32,
                drift.y as f32,
                drift.z as f32,
                self.weather_tuning.overcast_sun_floor as f32,
            ],
            weather3: [
                self.weather_tuning.rain_darken as f32,
                self.weather_tuning.dust_full_c as f32,
                self.weather_tuning.shell_alt_km as f32,
                self.weather_tuning.shell_fade_km as f32,
            ],
            weather4: [
                wind_kms.x as f32,
                wind_kms.y as f32,
                wind_kms.z as f32,
                (weather_t_s % 3600.0) as f32,
            ],
            weather5: [
                self.weather_tuning.cloud_mid_alt_km as f32,
                self.weather_tuning.cloud_high_alt_km as f32,
                self.weather_tuning.cloud_layer_count as f32,
                self.weather_tuning.orbit_cloud_opacity_cap as f32,
            ],
            weather6: [
                self.weather_tuning.cloud_low_scale as f32,
                self.weather_tuning.cloud_mid_scale as f32,
                self.weather_tuning.cloud_high_scale as f32,
                self.weather_tuning.rain_crevice_bias as f32,
            ],
            weather7: [
                self.weather_tuning.cloud_low_density as f32,
                self.weather_tuning.cloud_mid_density as f32,
                self.weather_tuning.cloud_high_density as f32,
                0.0,
            ],
            karst: {
                let kseed = |k: i64| {
                    (planet.seed.wrapping_add(k).wrapping_mul(0x9E37_79B1) & 0xFFFF_FFFF)
                        as u32
                };
                [kseed(40961), kseed(31337), kseed(51413), kseed(70001)]
            },
            danchor: {
                let ax = (cam_pos.x * 40.0).floor();
                let ay = (cam_pos.y * 40.0).floor();
                let az = (cam_pos.z * 40.0).floor();
                [
                    (ax / 40.0 - cam_pos.x) as f32,
                    (ay / 40.0 - cam_pos.y) as f32,
                    (az / 40.0 - cam_pos.z) as f32,
                    0.0,
                ]
            },
            danchor_cell: [
                (cam_pos.x * 40.0).floor() as i32,
                (cam_pos.y * 40.0).floor() as i32,
                (cam_pos.z * 40.0).floor() as i32,
                0,
            ],
            lights,
        };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        let mut draws: Vec<(DrawKey, u32)> = Vec::new();
        // near-field voxel chunks FIRST: if the slot pool ever fills, the
        // truncation victim must be a distant mesh tile, never a near chunk
        // (a dropped chunk leaves a see-through hole where the heightfield
        // was already cut away underneath it).
        let all_keys = chunk_keys
            .iter()
            .map(|k| DrawKey::Chunk(*k))
            .chain(keys.iter().map(|k| DrawKey::Tile(*k)));
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
            // precipitation: instanced rain streaks / snow flakes around the
            // camera, alpha-blended over everything, occluded by terrain
            if n_precip > 0 {
                pass.set_pipeline(&self.precip_pipeline);
                pass.set_bind_group(0, &self.bind_group, &[0]);
                pass.draw(0..6, 0..n_precip);
            }
        }
        self.queue.submit([encoder.finish()]);
        let cost = draw_start.elapsed().as_secs_f32() * 1000.0;
        self.draw_cost_ms.push_back(cost);
        if self.draw_cost_ms.len() > 240 {
            self.draw_cost_ms.pop_front();
        }
        draws.len()
    }

    /// (avg frame ms, p95 frame ms, avg draw-body CPU ms) over the last
    /// ~240 frames, or None until enough frames exist to mean anything.
    /// Frame times include vsync waits — steady 16.7 = a locked 60 Hz,
    /// and the p95 is where hitches (chunk builds, tile uploads) show.
    pub fn frame_stats(&self) -> Option<(f32, f32, f32)> {
        if self.frame_intervals_ms.len() < 30 {
            return None;
        }
        let mut sorted: Vec<f32> = self.frame_intervals_ms.iter().copied().collect();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let avg = sorted.iter().sum::<f32>() / sorted.len() as f32;
        let p95 = sorted[((sorted.len() as f32 * 0.95) as usize).min(sorted.len() - 1)];
        let cost = self.draw_cost_ms.iter().sum::<f32>() / self.draw_cost_ms.len().max(1) as f32;
        Some((avg, p95, cost))
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
            for (used, k, b) in by_age {
                // chunks used THIS frame are about to be drawn (the render
                // pass indexes chunk_cache[k] unconditionally) — never evict
                // them, even if the visible set alone exceeds the budget, or
                // the draw call panics on a missing key.
                if used == self.frame_counter || acc + b <= CHUNK_VRAM_BUDGET {
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
        let (pixels, n_tiles) = self.capture_rgba(planet, camera, edits)?;
        let (w, h) = self.size;
        Self::write_png(path, w, h, &pixels)?;
        Ok(n_tiles)
    }

    /// Encode RGBA pixels as a PNG — shared by capture() and the in-game
    /// sync-delta pair (which diffs pixel buffers before ever touching disk).
    pub fn write_png(path: &str, w: u32, h: u32, pixels: &[u8]) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(pixels)?;
        Ok(())
    }

    /// Render one COMPLETE frame offscreen (blocks until streamed chunks
    /// land, like a screenshot) and return its RGBA pixels + draw count.
    pub fn capture_rgba(
        &mut self,
        planet: &Arc<Planet>,
        camera: &Camera,
        edits: &Edits,
    ) -> Result<(Vec<u8>, usize)> {
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
                Ok((k, epoch, mesh)) => {
                    if self.chunk_pending.get(&k) == Some(&epoch) {
                        self.chunk_pending.remove(&k);
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
        // a screenshot is a complete frame for TILES too: drain the
        // build budget so captures are independent of how many frames
        // preceded them (byte-determinism for every instrument)
        for _ in 0..256 {
            if !self.tiles_deferred {
                break;
            }
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
        Ok((pixels, n_tiles))
    }
}

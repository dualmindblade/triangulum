//! wgpu side: one big quad storage buffer with a free-list allocator,
//! per-chunk curvature grid slots, per-frame draw records drawn with
//! multi-draw-indirect (opaque pass, then translucent water), a sky
//! triangle, timestamp queries and PNG readback. Block textures (M3b) are
//! one texture array with mipmaps (`textures.rs`).

use crate::mesh::GRID_BYTES;
use bytemuck::{Pod, Zeroable};
use std::sync::mpsc;

pub mod meshview;
pub mod regions;
mod sky;
pub use meshview::{Lines, MeshView, Tint};
pub use sky::Sky;
pub mod reel;
pub mod textures;
use textures::Picture;

pub const QUAD_BYTES: u64 = 8;
/// Largest chunk mesh one draw can address (index buffer size).
pub const MAX_QUADS_PER_CHUNK: u32 = 1 << 17;
pub const MAX_DRAWS: usize = 1 << 17;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    // Retained for compatibility with camera/bench constructors owned by
    // other missions. SkyUniform supplies the ephemeris, optical values and
    // the look (M3a: water opacity too); the shader consumes only the view
    // matrices here, and cam_up when no observer was set.
    pub sun_dir: [f32; 4],
    pub cam_up: [f32; 4],
    pub fog: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub sky_zenith: [f32; 4],
    pub light: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct DrawRecord {
    pub origin: [f32; 3],
    pub voxel: f32,
    pub slot: u32,
    pub quad_base: u32,
    pub quad_count: u32,
    pub pad: u32,
}

/// First-fit free-list allocator over quad indices.
pub struct Allocator {
    free: Vec<(u32, u32)>, // (offset, len), sorted by offset
    pub capacity: u32,
    pub used: u64,
    pub peak: u64,
    pub failures: u64,
}

impl Allocator {
    pub fn new(capacity: u32) -> Self {
        Self { free: vec![(0, capacity)], capacity, used: 0, peak: 0, failures: 0 }
    }
    pub fn alloc(&mut self, n: u32) -> Option<u32> {
        for i in 0..self.free.len() {
            let (off, len) = self.free[i];
            if len >= n {
                if len == n {
                    self.free.remove(i);
                } else {
                    self.free[i] = (off + n, len - n);
                }
                self.used += n as u64;
                self.peak = self.peak.max(self.used);
                return Some(off);
            }
        }
        self.failures += 1;
        None
    }
    pub fn free(&mut self, off: u32, n: u32) {
        if n == 0 {
            return;
        }
        self.used -= n as u64;
        let i = self.free.partition_point(|&(o, _)| o < off);
        self.free.insert(i, (off, n));
        if i + 1 < self.free.len() && self.free[i].0 + self.free[i].1 == self.free[i + 1].0 {
            self.free[i].1 += self.free[i + 1].1;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == self.free[i].0 {
            self.free[i - 1].1 += self.free[i].1;
            self.free.remove(i);
        }
    }
}

pub struct FrameStats {
    pub draws: u32,
    pub water_draws: u32,
    pub triangles: u64,
    pub water_triangles: u64,
}

/// A chunk's GPU residency: quad ranges and a grid slot.
#[derive(Clone, Copy, Debug)]
pub struct GpuMesh {
    pub slot: u32,
    pub opaque_base: u32,
    pub opaque_count: u32,
    pub water_base: u32,
    pub water_count: u32,
    /// Sloped powder (B7): three slots a quad, drawn opaque.
    pub slope_base: u32,
    pub slope_count: u32,
    /// The chunk's word for the mesh view (`meshview::region_word`): set once
    /// at upload, copied into its draw records, read by no normal pipeline.
    pub region: u32,
}

pub struct Renderer {
    pub sky: Sky,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
    color: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    camera_buf: wgpu::Buffer,
    sky_buf: wgpu::Buffer,
    quads_buf: wgpu::Buffer,
    grids_buf: wgpu::Buffer,
    draws_buf: wgpu::Buffer,
    indirect_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    colours_buf: wgpu::Buffer,
    /// Entries the colour buffer holds (it grows when blocks are added).
    colours_cap: usize,
    /// Block textures (M3b): `tex_layers` layers of `tex_size` squared,
    /// with every mip level; at least one layer, so the bind group always
    /// has a texture even when no block uses one.
    block_tex: wgpu::Texture,
    block_view: wgpu::TextureView,
    block_sampler: wgpu::Sampler,
    pub tex_size: u32,
    tex_layers: u32,
    bind_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    terrain_pipeline: wgpu::RenderPipeline,
    slope_pipeline: wgpu::RenderPipeline,
    /// This frame's sloped powder: the streamer fills it, no draw list passes
    /// through a caller, and it stays empty unless `--powder levels` is on.
    pub slope_draws: Vec<DrawRecord>,
    water_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    /// The mesh view (M and Shift+M in the game), off by default.
    pub mesh_view: MeshView,
    /// Its pipelines, built the first time it is switched on.
    mesh_pipelines: Option<meshview::Pipelines>,
    /// The bake's region layers, waiting for those pipelines.
    mesh_regions: Option<std::sync::Arc<regions::RegionData>>,
    query_set: wgpu::QuerySet,
    query_resolve: wgpu::Buffer,
    query_read: wgpu::Buffer,
    query_pending: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    timestamp_period: f32,
    pub alloc: Allocator,
    slot_free: Vec<u32>,
    pub slots_used: u32,
    pub slot_capacity: u32,
    pub last_gpu_ms: f64,
}

pub fn make_instance(backend: Option<&str>) -> wgpu::Instance {
    let backends = match backend {
        Some("dx12") => wgpu::Backends::DX12,
        Some("vulkan") => wgpu::Backends::VULKAN,
        _ => wgpu::Backends::VULKAN | wgpu::Backends::DX12,
    };
    wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() })
}

impl Renderer {
    pub fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        width: u32,
        height: u32,
        quad_buffer_mb: u64,
        slot_capacity: u32,
        colours: &[[f32; 4]],
    ) -> Self {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            ..Default::default()
        }))
        .expect("no adapter");
        let info = adapter.get_info();
        println!("adapter: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
        let alimits = adapter.limits();
        let features = wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::INDIRECT_FIRST_INSTANCE;
        assert!(adapter.features().contains(features), "adapter lacks {:?}", features - adapter.features());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("triangulum"),
            required_features: features,
            required_limits: alimits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .expect("device");

        let format = wgpu::TextureFormat::Bgra8UnormSrgb;
        let (color, color_view, depth_view) = Self::make_targets(&device, width, height, format);

        let quad_bytes = (quad_buffer_mb << 20).min(alimits.max_storage_buffer_binding_size as u64).min(alimits.max_buffer_size);
        let quad_capacity = (quad_bytes / QUAD_BYTES) as u32;
        println!("quad buffer: {} MB ({} M quads); grid slots {}", quad_bytes >> 20, quad_capacity / 1_000_000, slot_capacity);
        let mk = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
        };
        use wgpu::BufferUsages as U;
        let camera_buf = mk("camera", std::mem::size_of::<CameraUniform>() as u64, U::UNIFORM | U::COPY_DST);
        let sky_buf = mk("sky", std::mem::size_of::<sky::SkyUniform>() as u64, U::UNIFORM | U::COPY_DST);
        let quads_buf = mk("quads", quad_capacity as u64 * QUAD_BYTES, U::STORAGE | U::COPY_DST);
        let grids_buf = mk("grids", slot_capacity as u64 * GRID_BYTES as u64, U::STORAGE | U::COPY_DST);
        let draws_buf = mk("draws", (MAX_DRAWS * std::mem::size_of::<DrawRecord>()) as u64, U::STORAGE | U::COPY_DST);
        let indirect_buf = mk("indirect", (MAX_DRAWS * 20) as u64, U::INDIRECT | U::COPY_DST);
        let colours_buf = mk("colours", (colours.len().max(1) * 16) as u64, U::STORAGE | U::COPY_DST);
        queue.write_buffer(&colours_buf, 0, bytemuck::cast_slice(colours));
        // Shared quad index pattern: 4 vertices per quad, 6 indices.
        let mut indices: Vec<u32> = Vec::with_capacity(MAX_QUADS_PER_CHUNK as usize * 6);
        for q in 0..MAX_QUADS_PER_CHUNK {
            let b = q * 4;
            indices.extend_from_slice(&[b, b + 1, b + 2, b + 2, b + 1, b + 3]);
        }
        let index_buf = mk("indices", indices.len() as u64 * 4, U::INDEX | U::COPY_DST);
        queue.write_buffer(&index_buf, 0, bytemuck::cast_slice(&indices));

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let tex_entry = wgpu::BindGroupLayoutEntry {
            binding: 6,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
            count: None,
        };
        let samp_entry = wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain-bgl"),
            entries: &[entry(0, wgpu::BufferBindingType::Uniform), entry(1, ro), entry(2, ro), entry(3, ro), entry(4, ro), entry(5, wgpu::BufferBindingType::Uniform), tex_entry, samp_entry],
        });
        // Block textures: what main installed (M3b), or one blank layer.
        let installed = textures::installed();
        let tex_size = installed.map_or(crate::world::blocks::DEFAULT_TEXTURE_SIZE, |t| t.size);
        let pictures: Vec<&Picture> = installed.map_or(Vec::new(), |t| t.pictures.iter().collect());
        let tex_layers = (pictures.len() as u32).max(1).next_power_of_two().max(4);
        let (block_tex, block_view) = make_block_texture(&device, tex_size, tex_layers);
        for (i, p) in pictures.iter().enumerate() {
            write_layer(&queue, &block_tex, i as u32, p);
        }
        // Pixel art up close (nearest), the mip chain far away (linear
        // between levels): the 1 x 1 level is the average colour.
        let block_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("block-textures"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let bind_group = make_bind_group(&device, &bgl, [&camera_buf, &quads_buf, &grids_buf, &draws_buf, &colours_buf, &sky_buf], &block_view, &block_sampler);
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let depth = |write: bool, compare: wgpu::CompareFunction| {
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let pipeline = |label: &str, vs: &str, fs: &str, blend: Option<wgpu::BlendState>, cull: Option<wgpu::Face>, ds: Option<wgpu::DepthStencilState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: ds,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let terrain_pipeline = pipeline("terrain", "vs_main", "fs_main", None, Some(wgpu::Face::Back), depth(true, wgpu::CompareFunction::Greater));
        let slope_pipeline = pipeline("slope", "vs_slope", "fs_main", None, Some(wgpu::Face::Back), depth(true, wgpu::CompareFunction::Greater));
        let water_pipeline = pipeline(
            "water",
            "vs_water",
            "fs_water",
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            None,
            depth(false, wgpu::CompareFunction::Greater),
        );
        let sky_pipeline = pipeline("sky", "vs_sky", "fs_sky", None, None, depth(false, wgpu::CompareFunction::GreaterEqual));
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("ts"), ty: wgpu::QueryType::Timestamp, count: 2 });
        let query_resolve = mk("ts-resolve", 16, U::QUERY_RESOLVE | U::COPY_SRC);
        let query_read = mk("ts-read", 16, U::MAP_READ | U::COPY_DST);
        let timestamp_period = queue.get_timestamp_period();

        Self {
            sky: Sky::new(),
            device,
            queue,
            width,
            height,
            format,
            color,
            color_view,
            depth_view,
            camera_buf,
            sky_buf,
            quads_buf,
            grids_buf,
            draws_buf,
            indirect_buf,
            index_buf,
            colours_buf,
            colours_cap: colours.len().max(1),
            block_tex,
            block_view,
            block_sampler,
            tex_size,
            tex_layers,
            bind_layout: bgl,
            bind_group,
            terrain_pipeline,
            slope_pipeline,
            slope_draws: Vec::new(),
            water_pipeline,
            sky_pipeline,
            mesh_view: MeshView::NORMAL,
            mesh_pipelines: None,
            mesh_regions: None,
            query_set,
            query_resolve,
            query_read,
            query_pending: None,
            timestamp_period,
            alloc: Allocator::new(quad_capacity),
            slot_free: (0..slot_capacity).rev().collect(),
            slots_used: 0,
            slot_capacity,
            last_gpu_ms: 0.0,
        }
    }

    fn make_targets(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> (wgpu::Texture, wgpu::TextureView, wgpu::TextureView) {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("color"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        (color.create_view(&Default::default()), depth.create_view(&Default::default())).1;
        let cv = color.create_view(&Default::default());
        let dv = depth.create_view(&Default::default());
        (color, cv, dv)
    }

    /// Replace the block colour table (three RGBA per id: top, sides,
    /// bottom; `Registry::colour_table`). Chunk meshes carry block ids,
    /// not colours, so a colour change needs no remeshing: the next frame
    /// draws everything already resident in the new colours.
    pub fn set_colours(&mut self, colours: &[[f32; 4]]) {
        if colours.len() > self.colours_cap {
            self.colours_cap = colours.len().next_power_of_two();
            self.colours_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("colours"),
                size: (self.colours_cap * 16) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.rebind();
        }
        self.queue.write_buffer(&self.colours_buf, 0, bytemuck::cast_slice(colours));
    }

    fn rebind(&mut self) {
        self.bind_group = make_bind_group(&self.device, &self.bind_layout, [&self.camera_buf, &self.quads_buf, &self.grids_buf, &self.draws_buf, &self.colours_buf, &self.sky_buf], &self.block_view, &self.block_sampler);
    }

    /// Replace every block texture (M3b): `pictures[layer]`, all of
    /// `size`. The array is rebuilt when it is too small or the size
    /// changed; the colour table names the layers each face samples.
    pub fn set_textures(&mut self, size: u32, pictures: &[&Picture]) {
        let need = (pictures.len() as u32).max(1);
        if size != self.tex_size || need > self.tex_layers {
            self.tex_size = size;
            self.tex_layers = need.next_power_of_two().max(4);
            let (t, v) = make_block_texture(&self.device, size, self.tex_layers);
            self.block_tex = t;
            self.block_view = v;
            self.rebind();
        }
        for (i, p) in pictures.iter().enumerate() {
            write_layer(&self.queue, &self.block_tex, i as u32, p);
        }
    }

    /// Replace one layer's picture (painting in the panel, a file edited on
    /// disk). False when the layer does not fit: call `set_textures`.
    pub fn set_texture_layer(&mut self, layer: u32, picture: &Picture) -> bool {
        if layer >= self.tex_layers || picture.size != self.tex_size {
            return false;
        }
        write_layer(&self.queue, &self.block_tex, layer, picture);
        true
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return;
        }
        self.width = width;
        self.height = height;
        let (c, cv, dv) = Self::make_targets(&self.device, width, height, self.format);
        self.color = c;
        self.color_view = cv;
        self.depth_view = dv;
    }

    /// Upload one chunk mesh; None when out of room. Water quads are 16
    /// bytes (mesh.rs), so each takes two slots of the quad buffer; the
    /// draw record still counts quads, and the water vertex shader reads
    /// slot `quad_base + 2 (q - quad_base)` for its quad q.
    pub fn add_mesh(&mut self, opaque: &[[u32; 2]], water: &[[u32; 4]], grid: &[f32]) -> Option<GpuMesh> {
        let no = opaque.len().min(MAX_QUADS_PER_CHUNK as usize) as u32;
        let nw = water.len().min(MAX_QUADS_PER_CHUNK as usize) as u32;
        let ob = if no > 0 { self.alloc.alloc(no)? } else { 0 };
        let wb = if nw > 0 {
            match self.alloc.alloc(2 * nw) {
                Some(b) => b,
                None => {
                    self.alloc.free(ob, no);
                    return None;
                }
            }
        } else {
            0
        };
        let Some(slot) = self.slot_free.pop() else {
            self.alloc.free(ob, no);
            self.alloc.free(wb, 2 * nw);
            return None;
        };
        self.slots_used += 1;
        if no > 0 {
            self.queue.write_buffer(&self.quads_buf, ob as u64 * QUAD_BYTES, bytemuck::cast_slice(&opaque[..no as usize]));
        }
        if nw > 0 {
            self.queue.write_buffer(&self.quads_buf, wb as u64 * QUAD_BYTES, bytemuck::cast_slice(&water[..nw as usize]));
        }
        self.queue.write_buffer(&self.grids_buf, slot as u64 * GRID_BYTES as u64, bytemuck::cast_slice(grid));
        Some(GpuMesh { slot, opaque_base: ob, opaque_count: no, water_base: wb, water_count: nw, slope_base: 0, slope_count: 0, region: 0 })
    }

    /// Add a chunk's sloped powder to the mesh `add_mesh` made. None, and the
    /// mesh given back, when out of room; no slopes, no cost.
    pub fn add_slopes(&mut self, mut m: GpuMesh, slopes: &[[u32; 6]]) -> Option<GpuMesh> {
        if slopes.is_empty() {
            return Some(m);
        }
        let ns = slopes.len().min(MAX_QUADS_PER_CHUNK as usize) as u32;
        let Some(sb) = self.alloc.alloc(3 * ns) else {
            self.free_mesh(m);
            return None;
        };
        self.queue.write_buffer(&self.quads_buf, sb as u64 * QUAD_BYTES, bytemuck::cast_slice(&slopes[..ns as usize]));
        m.slope_base = sb;
        m.slope_count = ns;
        Some(m)
    }

    pub fn free_mesh(&mut self, m: GpuMesh) {
        self.alloc.free(m.opaque_base, m.opaque_count);
        self.alloc.free(m.water_base, 2 * m.water_count);
        self.alloc.free(m.slope_base, 3 * m.slope_count);
        self.slot_free.push(m.slot);
        self.slots_used -= 1;
    }

    /// The bake's region layers for the mesh view's tints.
    pub fn set_regions(&mut self, data: std::sync::Arc<regions::RegionData>) {
        self.mesh_regions = Some(data);
    }

    /// Resident mesh bytes (quads + grid records).
    pub fn resident_bytes(&self) -> u64 {
        self.alloc.used * QUAD_BYTES + self.slots_used as u64 * GRID_BYTES as u64
    }

    /// Encode and submit one frame into `target` (None = offscreen colour texture).
    pub fn render(&mut self, cam: &CameraUniform, opaque: &[DrawRecord], water: &[DrawRecord], target: Option<&wgpu::TextureView>) -> FrameStats {
        let opaque = &opaque[..opaque.len().min(MAX_DRAWS)];
        let water = &water[..water.len().min(MAX_DRAWS - opaque.len())];
        let slopes = self.slope_draws.len().min(MAX_DRAWS - opaque.len() - water.len());
        self.queue.write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(cam));
        let sky = self.sky.uniform(cam);
        self.queue.write_buffer(&self.sky_buf, 0, bytemuck::bytes_of(&sky));
        let (mut tris, mut wtris) = (0u64, 0u64);
        let total = opaque.len() + water.len() + slopes;
        if total > 0 {
            let mut recs: Vec<DrawRecord> = Vec::with_capacity(total);
            recs.extend_from_slice(opaque);
            recs.extend_from_slice(water);
            recs.extend_from_slice(&self.slope_draws[..slopes]);
            self.queue.write_buffer(&self.draws_buf, 0, bytemuck::cast_slice(&recs));
            let args: Vec<wgpu::util::DrawIndexedIndirectArgs> = recs
                .iter()
                .enumerate()
                .map(|(i, d)| wgpu::util::DrawIndexedIndirectArgs {
                    index_count: d.quad_count * 6,
                    instance_count: 1,
                    first_index: 0,
                    base_vertex: (d.quad_base * 4) as i32,
                    first_instance: i as u32,
                })
                .collect();
            // DrawIndexedIndirectArgs is repr(C) of five u32s.
            let bytes: &[u8] = unsafe { std::slice::from_raw_parts(args.as_ptr() as *const u8, args.len() * 20) };
            self.queue.write_buffer(&self.indirect_buf, 0, bytes);
            for d in opaque {
                tris += d.quad_count as u64 * 2;
            }
            for d in water {
                wtris += d.quad_count as u64 * 2;
            }
        }
        let view_setting = self.mesh_view;
        if !view_setting.is_normal() {
            let pipes = self.mesh_pipelines.get_or_insert_with(|| meshview::Pipelines::new(&self.device, &self.bind_layout, self.format));
            if let Some(data) = self.mesh_regions.take() {
                pipes.set_regions(&self.device, &self.queue, &data);
            }
            pipes.set_view(&self.queue, view_setting);
        }
        let mesh = if view_setting.is_normal() { None } else { self.mesh_pipelines.as_ref() };
        let (tinted, only) = (view_setting.tint != Tint::Off, view_setting.lines == Lines::Only);
        let view = target.unwrap_or(&self.color_view);
        let take_query = self.query_pending.is_none();
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: take_query.then_some(wgpu::RenderPassTimestampWrites {
                    query_set: &self.query_set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
            if let Some(m) = mesh {
                pass.set_bind_group(1, &m.bind, &[]);
            }
            if !opaque.is_empty() {
                pass.set_pipeline(match mesh {
                    Some(m) if only => &m.fill,
                    Some(m) if tinted => &m.tint_terrain,
                    _ => &self.terrain_pipeline,
                });
                pass.multi_draw_indexed_indirect(&self.indirect_buf, 0, opaque.len() as u32);
            }
            if slopes > 0 {
                pass.set_pipeline(&self.slope_pipeline);
                pass.multi_draw_indexed_indirect(&self.indirect_buf, (opaque.len() + water.len()) as u64 * 20, slopes as u32);
            }
            if !only {
                // Reversed-Z infinity: fill only pixels the opaque geometry left
                // at clear depth. Water composites over this sky afterwards.
                pass.set_pipeline(&self.sky_pipeline);
                pass.draw(0..3, 0..1);
                if !water.is_empty() {
                    pass.set_pipeline(match mesh {
                        Some(m) if tinted => &m.tint_water,
                        _ => &self.water_pipeline,
                    });
                    pass.multi_draw_indexed_indirect(&self.indirect_buf, opaque.len() as u64 * 20, water.len() as u32);
                }
            }
            // The mesh view's lines, over everything drawn above.
            if let (Some(m), true) = (mesh, view_setting.lines != Lines::Off) {
                if !opaque.is_empty() {
                    pass.set_pipeline(&m.lines_terrain);
                    pass.multi_draw_indexed_indirect(&self.indirect_buf, 0, opaque.len() as u32);
                }
                if !water.is_empty() {
                    pass.set_pipeline(&m.lines_water);
                    pass.multi_draw_indexed_indirect(&self.indirect_buf, opaque.len() as u64 * 20, water.len() as u32);
                }
            }
        }
        if take_query {
            enc.resolve_query_set(&self.query_set, 0..2, &self.query_resolve, 0);
            enc.copy_buffer_to_buffer(&self.query_resolve, 0, &self.query_read, 0, 16);
        }
        self.queue.submit([enc.finish()]);
        if take_query {
            let (tx, rx) = mpsc::channel();
            self.query_read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            self.query_pending = Some(rx);
        }
        FrameStats { draws: opaque.len() as u32, water_draws: water.len() as u32, triangles: tris, water_triangles: wtris }
    }

    fn read_query(&mut self) -> f64 {
        let ms = {
            let data = self.query_read.slice(..).get_mapped_range().expect("map range");
            let ts: &[u64] = bytemuck::cast_slice(&data);
            (ts[1].wrapping_sub(ts[0])) as f64 * self.timestamp_period as f64 * 1e-6
        };
        self.query_read.unmap();
        self.query_pending = None;
        self.last_gpu_ms = ms;
        ms
    }

    /// Block until the GPU finished the last submit and return the pass time in ms.
    pub fn wait_gpu_ms(&mut self) -> f64 {
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).expect("poll");
        if let Some(rx) = &self.query_pending {
            rx.recv().expect("map").expect("map ok");
            return self.read_query();
        }
        self.last_gpu_ms
    }

    /// Non-blocking: pick up the last finished timestamp query if it is ready.
    pub fn poll_gpu_ms(&mut self) -> f64 {
        let _ = self.device.poll(wgpu::PollType::Poll);
        if let Some(rx) = &self.query_pending {
            if let Ok(r) = rx.try_recv() {
                r.expect("map ok");
                return self.read_query();
            }
        }
        self.last_gpu_ms
    }

    /// Fill a target with one dark colour: the annotator's background, in place
    /// of the world (it draws nothing else, so it stays quick).
    pub fn clear(&self, target: &wgpu::TextureView) {
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("clear") });
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.01, g: 0.01, b: 0.01, a: 1.0 }), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.queue.submit([enc.finish()]);
    }

    /// Read the offscreen colour target back as RGBA8.
    pub fn read_rgba(&self) -> Vec<u8> {
        self.read_texture(&self.color, self.width, self.height)
    }

    /// Read any BGRA8 render target (created with COPY_SRC) back as RGBA8.
    pub fn read_texture(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<u8> {
        let bpr = (width * 4).div_ceil(256) * 256;
        let size = bpr as u64 * height as u64;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("png"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: None } },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let (tx, rx) = mpsc::channel();
        buf.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).expect("poll");
        rx.recv().expect("map").expect("map ok");
        let data = buf.slice(..).get_mapped_range().expect("map range");
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height as usize {
            let line = &data[row * bpr as usize..row * bpr as usize + width as usize * 4];
            for px in line.chunks_exact(4) {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        drop(data);
        buf.unmap();
        rgba
    }

    /// Save the offscreen colour target as a PNG.
    pub fn save_png(&self, path: &str) {
        let rgba = self.read_rgba();
        write_png(path, self.width, self.height, &rgba);
    }
}

/// The terrain bind group: camera, quads, grids, draws, colours, sky,
/// then the block textures and their sampler (M3b).
fn make_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, bufs: [&wgpu::Buffer; 6], tex: &wgpu::TextureView, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
    let mut entries: Vec<wgpu::BindGroupEntry> = bufs.iter().enumerate().map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() }).collect();
    entries.push(wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(tex) });
    entries.push(wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(sampler) });
    device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("terrain-bg"), layout, entries: &entries })
}

/// The block texture array: `layers` layers of `size` squared, sRGB, with
/// every mip level down to 1 x 1.
fn make_block_texture(device: &wgpu::Device, size: u32, layers: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("block-textures"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
        mip_level_count: size.ilog2() + 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    (tex, view)
}

/// Upload one picture and its mip chain into a layer.
fn write_layer(queue: &wgpu::Queue, tex: &wgpu::Texture, layer: u32, p: &Picture) {
    for (level, data) in p.mips().iter().enumerate() {
        let s = (p.size >> level).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: level as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * s), rows_per_image: Some(s) },
            wgpu::Extent3d { width: s, height: s, depth_or_array_layers: 1 },
        );
    }
}

/// Read an 8-bit RGB or RGBA PNG: (width, height, RGBA8).
pub fn read_png(path: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().map_err(|e| format!("{path}: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or_else(|| format!("{path}: too large"))?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{path}: {e}"))?;
    buf.truncate(info.buffer_size());
    let rgba = match (info.color_type, info.bit_depth) {
        (png::ColorType::Rgba, png::BitDepth::Eight) => buf,
        (png::ColorType::Rgb, png::BitDepth::Eight) => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        (c, d) => return Err(format!("{path}: {c:?} {d:?}, wanted 8-bit RGB or RGBA")),
    };
    Ok((info.width, info.height, rgba))
}

pub fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(rgba).expect("png data");
}

//! The egui overlay (stats HUD, crosshair, captions) and the small wgpu
//! painter it draws through. egui-wgpu 0.35 pins wgpu 29 while the engine
//! uses wgpu 30, so egui's output (clipped, textured, vertex-coloured
//! triangles in pixel space plus one texture manager) is uploaded here, as
//! the old viewer did.

use crate::camera::Camera;
use crate::grid::{BlockPos, Planet};
use egui::epaint::{ImageDelta, Primitive};
use egui::{ClippedPrimitive, TextureId, TexturesDelta};
use glam::Vec4;

/// What the HUD shows.
#[derive(Default, Clone)]
pub struct Hud {
    pub fps: f64,
    pub low1_fps: f64,
    pub gpu_ms: f64,
    pub cpu_ms: f64,
    pub frame_ms: f64,
    pub drawn_per_lod: [u32; 16],
    pub loaded_per_lod: [u32; 16],
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub alt_agl: f64,
    /// The block under the crosshair within reach, its name and distance.
    pub block: Option<(BlockPos, String, f64)>,
    pub speed: f64,
    pub vsync: bool,
    pub loaded: usize,
    pub inflight: usize,
    pub pending: usize,
    pub tris: u64,
    pub water_tris: u64,
    pub draws: usize,
    pub vram_mb: f64,
    pub preset: String,
    pub hide_water: bool,
    pub captured: bool,
    /// "walk" or "fly".
    pub mode: String,
    pub on_ground: bool,
    pub in_water: bool,
    /// Hotbar slots: (name, linear RGB).
    pub hotbar: Vec<(String, [f32; 3])>,
    /// The slots' textures (M3b), where a block has one (the look panel's
    /// egui textures); the colour otherwise.
    pub hotbar_tex: Vec<Option<egui::TextureId>>,
    pub selected: usize,
    /// Outline of the targeted block: screen-space segments in pixels.
    pub outline: Vec<([f32; 2], [f32; 2])>,
    pub edits: usize,
    pub dirty: usize,
    pub save_note: String,
    /// The simulation's one-line status (`Sim::describe`), stats panel only.
    pub sim_line: String,
}

fn srgb(c: [f32; 3]) -> egui::Color32 {
    let f = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        let s = if x <= 0.0031308 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0).round() as u8
    };
    egui::Color32::from_rgb(f(c[0]), f(c[1]), f(c[2]))
}

/// LOD-0 block corners, using the mesher/shader's eight-voxel curvature
/// grid and f32 interpolation. Always evaluate in the block's own face;
/// wrapping eight out-of-face corners independently folds a cube corner.
pub(crate) fn outline_corners(planet: &Planet, block: BlockPos) -> [glam::DVec3; 8] {
    let (i0, j0, k0) = (block.i / 32 * 32, block.j / 32 * 32, block.k / 32 * 32);
    let r0 = planet.radius_at(k0 as f64);
    let origin = planet.dir(block.face, i0 as f64, j0 as f64) * r0;
    std::array::from_fn(|c| {
        let i = block.i + (c & 1) as u32;
        let j = block.j + ((c >> 1) & 1) as u32;
        let k = block.k + ((c >> 2) & 1) as u32;
        let gx = ((i - i0) / 8).min(3);
        let gy = ((j - j0) / 8).min(3);
        let fx = (i - i0 - gx * 8) as f32 / 8.0;
        let fy = (j - j0 - gy * 8) as f32 / 8.0;
        let grid = |dx: u32, dy: u32| {
            let dir = planet.dir(block.face, (i0 + (gx + dx) * 8) as f64, (j0 + (gy + dy) * 8) as f64);
            ((dir * r0 - origin).as_vec3(), dir.as_vec3())
        };
        let (a, b, c, d) = (grid(0, 0), grid(1, 0), grid(0, 1), grid(1, 1));
        let off = a.0.lerp(b.0, fx).lerp(c.0.lerp(d.0, fx), fy);
        let up = a.1.lerp(b.1, fx).lerp(c.1.lerp(d.1, fx), fy).normalize();
        origin + (off + up * ((k - k0) as f32 * planet.block_size as f32)).as_dvec3()
    })
}

/// Screen-space segments (pixels) outlining the canonical target block.
/// Segments with an endpoint behind the camera are dropped.
pub fn outline_segments(planet: &Planet, cam: &Camera, aspect: f64, width: u32, height: u32, block: BlockPos) -> Vec<([f32; 2], [f32; 2])> {
    let vp = cam.view_proj(aspect);
    let (w, h) = (width as f32, height as f32);
    let corner = |p: glam::DVec3| -> Option<[f32; 2]> {
        let rel = (p - cam.pos).as_vec3();
        let c = vp * Vec4::new(rel.x, rel.y, rel.z, 1.0);
        if c.w <= 1e-6 {
            return None;
        }
        Some([(c.x / c.w * 0.5 + 0.5) * w, (0.5 - c.y / c.w * 0.5) * h])
    };
    let corners = outline_corners(planet, block).map(corner);
    const EDGES: [(usize, usize); 12] = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    EDGES.iter().filter_map(|&(a, b)| Some((corners[a]?, corners[b]?))).collect()
}

pub fn draw_hud(ctx: &egui::Context, h: &Hud, stats: bool) {
    let ppp = ctx.pixels_per_point();
    if stats {
        egui::Window::new("stats")
            .title_bar(false)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(egui::Color32::from_black_alpha(150)).inner_margin(8.0))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(8.0, 8.0))
            .show(ctx, |ui| {
                ui.set_min_width(760.0);
                ui.style_mut().override_font_id = Some(egui::FontId::monospace(13.0));
                let mut line = |s: String| {
                    ui.label(egui::RichText::new(s).color(egui::Color32::from_rgb(235, 235, 235)));
                };
                line(format!("fps {:6.1}   1% low (5 s) {:6.1}", h.fps, h.low1_fps));
                line(format!("frame {:5.2} ms  cpu {:5.2} ms  gpu {:5.2} ms", h.frame_ms, h.cpu_ms, h.gpu_ms));
                line(format!("tris {:.2}M (+{:.2}M water)  draws {}  VRAM {:.0} MB", h.tris as f64 / 1e6, h.water_tris as f64 / 1e6, h.draws, h.vram_mb));
                let per: Vec<String> = h.drawn_per_lod.iter().enumerate().filter(|(_, n)| **n > 0).map(|(l, n)| format!("L{l}:{n}")).collect();
                line(format!("drawn/LOD  {}", per.join(" ")));
                let per: Vec<String> = h.loaded_per_lod.iter().enumerate().filter(|(_, n)| **n > 0).map(|(l, n)| format!("L{l}:{n}")).collect();
                line(format!("loaded/LOD {}  (total {}, inflight {}, pending {})", per.join(" "), h.loaded, h.inflight, h.pending));
                line(format!("lat {:9.5}  lon {:10.5}  alt {:.1} m  ({:.1} m AGL)", h.lat, h.lon, h.alt, h.alt_agl));
                match &h.block {
                    Some((b, name, dist)) => line(format!("crosshair: face {} i {} j {} k {}  {}  ({:.1} m)", b.face, b.i, b.j, b.k, name, dist)),
                    None => line("crosshair: nothing within reach".into()),
                }
                let state = if h.mode == "fly" {
                    format!("fly {:.1} m/s", h.speed)
                } else if h.in_water {
                    "walk: swimming".to_string()
                } else if h.on_ground {
                    "walk: on ground".to_string()
                } else {
                    "walk: in the air".to_string()
                };
                line(format!(
                    "{state}  vsync {}  water {}  preset {}  edits {} chunks ({} unsaved){}  [{}]",
                    if h.vsync { "on" } else { "off" },
                    if h.hide_water { "hidden" } else { "shown" },
                    h.preset,
                    h.edits,
                    h.dirty,
                    if h.save_note.is_empty() { String::new() } else { format!("  {}", h.save_note) },
                    if h.captured { "mouse captured: Esc releases" } else { "click to capture mouse" }
                ));
                if !h.sim_line.is_empty() {
                    line(h.sim_line.clone());
                }
                line("WASD move  Space jump/up  Ctrl sneak/down  Shift sprint  G walk/fly  LMB break  RMB place  1-9 block  scroll fly speed".into());
                line("P preset  R to ground  V vsync  F water  H hud  F1 look panel  Q quit".into());
            });
    }
    // Under every window (the look panel covers the hotbar's corner, not
    // the other way round).
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("crosshair")));
    let c = ctx.content_rect().center();
    let col = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 180);
    painter.line_segment([c - egui::vec2(10.0, 0.0), c + egui::vec2(10.0, 0.0)], egui::Stroke::new(1.5, col));
    painter.line_segment([c - egui::vec2(0.0, 10.0), c + egui::vec2(0.0, 10.0)], egui::Stroke::new(1.5, col));
    // Targeted block outline: a dark line under a light one so it reads on any block.
    for (a, b) in &h.outline {
        let pa = egui::pos2(a[0] / ppp, a[1] / ppp);
        let pb = egui::pos2(b[0] / ppp, b[1] / ppp);
        painter.line_segment([pa, pb], egui::Stroke::new(3.0, egui::Color32::from_black_alpha(160)));
        painter.line_segment([pa, pb], egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 230)));
    }
    // Hotbar along the bottom.
    if !h.hotbar.is_empty() {
        let slot = 46.0;
        let total = slot * h.hotbar.len() as f32;
        let rect = ctx.content_rect();
        let x0 = rect.center().x - total / 2.0;
        let y0 = rect.max.y - slot - 12.0;
        painter.rect_filled(egui::Rect::from_min_size(egui::pos2(x0 - 4.0, y0 - 4.0), egui::vec2(total + 8.0, slot + 8.0)), 4.0, egui::Color32::from_black_alpha(150));
        for (n, (name, colour)) in h.hotbar.iter().enumerate() {
            let r = egui::Rect::from_min_size(egui::pos2(x0 + slot * n as f32, y0), egui::vec2(slot, slot)).shrink(5.0);
            match h.hotbar_tex.get(n).copied().flatten() {
                Some(tex) => {
                    painter.image(tex, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                }
                None => {
                    painter.rect_filled(r, 3.0, srgb(*colour));
                }
            }
            let stroke = if n == h.selected { egui::Stroke::new(3.0, egui::Color32::WHITE) } else { egui::Stroke::new(1.0, egui::Color32::from_gray(90)) };
            painter.rect_stroke(r, 3.0, stroke, egui::StrokeKind::Outside);
            painter.text(r.left_top() + egui::vec2(3.0, 1.0), egui::Align2::LEFT_TOP, format!("{}", n + 1), egui::FontId::monospace(11.0), egui::Color32::WHITE);
            if n == h.selected {
                painter.text(egui::pos2(rect.center().x, y0 - 10.0), egui::Align2::CENTER_BOTTOM, name, egui::FontId::proportional(15.0), egui::Color32::WHITE);
            }
        }
    }
}

/// A caption band along the bottom of the frame (for saved renders).
pub fn draw_caption(ctx: &egui::Context, text: &str) {
    egui::Window::new("caption")
        .title_bar(false)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(egui::Color32::from_black_alpha(170)).inner_margin(10.0))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(ctx.content_rect().width() - 20.0);
            ui.label(egui::RichText::new(text).monospace().size(18.0).color(egui::Color32::WHITE));
        });
}

/// Run one egui pass without a window and paint it over the renderer's
/// offscreen colour target.
pub fn paint_offscreen(ctx: &egui::Context, paint: &mut EguiPaint, gpu: &crate::render::Renderer, build: impl Fn(&egui::Context)) {
    let (w, h) = (gpu.width, gpu.height);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
        ..Default::default()
    };
    // egui sizes a window on its first pass and lays it out properly on the
    // next, so run two passes and paint the second (fonts are uploaded by
    // whichever pass produces them, so both texture deltas are applied).
    // Animations are off: a window fade-in needs wall-clock frames that an
    // offscreen capture never gives it, and left captions half transparent.
    ctx.all_styles_mut(|s| s.animation_time = 0.0);
    let build: &dyn Fn(&egui::Context) = &build;
    let warm = ctx.run_ui(raw.clone(), |ui| build(ui.ctx()));
    let mut deltas = warm.textures_delta;
    let full = ctx.run_ui(raw, |ui| build(ui.ctx()));
    deltas.append(full.textures_delta);
    let prims = ctx.tessellate(full.shapes, full.pixels_per_point);
    paint.paint(&gpu.device, &gpu.queue, &gpu.color_view, (w, h), full.pixels_per_point, &prims, &deltas);
}

// ----------------------------------------------------- egui paint (wgpu 30)

/// Minimal egui renderer against the engine's own wgpu version.
pub struct EguiPaint {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind_layout: wgpu::BindGroupLayout,
    textures: std::collections::HashMap<TextureId, (wgpu::Texture, wgpu::BindGroup)>,
    vbuf: Option<wgpu::Buffer>,
    ibuf: Option<wgpu::Buffer>,
    uniform: wgpu::Buffer,
    uniform_bind: wgpu::BindGroup,
}

const EGUI_SHADER: &str = r#"
struct Screen { size_px: vec2<f32>, _pad: vec2<f32> };
@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c < vec3<f32>(0.04045));
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(in.pos.x / screen.size_px.x * 2.0 - 1.0, 1.0 - in.pos.y / screen.size_px.y * 2.0, 0.0, 1.0);
    out.uv = in.uv;
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, in.uv);
    // egui colours and textures are already premultiplied.
    return in.color * t;
}
"#;

impl EguiPaint {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("egui"), source: wgpu::ShaderSource::Wgsl(EGUI_SHADER.into()) });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("egui-tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: true } },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("egui"), bind_group_layouts: &[Some(&uniform_layout), Some(&bind_layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("egui"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 16, shader_location: 2 },
                    ],
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                        alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("egui"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("egui-screen"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let uniform_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("egui-screen"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        Self { pipeline, sampler, bind_layout, textures: Default::default(), vbuf: None, ibuf: None, uniform, uniform_bind }
    }

    fn apply_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, id: TextureId, delta: &ImageDelta) {
        let size = delta.image.size();
        let pixels: Vec<u8> = match &delta.image {
            egui::ImageData::Color(img) => img.pixels.iter().flat_map(|c| c.to_array()).collect(),
        };
        if delta.pos.is_none() {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("egui-tex"),
                size: wgpu::Extent3d { width: size[0] as u32, height: size[1] as u32, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("egui-tex"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            self.textures.insert(id, (tex, bind));
        }
        if let Some((tex, _)) = self.textures.get(&id) {
            let (x, y) = delta.pos.map_or((0, 0), |p| (p[0] as u32, p[1] as u32));
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                &pixels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * size[0] as u32), rows_per_image: None },
                wgpu::Extent3d { width: size[0] as u32, height: size[1] as u32, depth_or_array_layers: 1 },
            );
        }
    }

    /// Paint one egui frame onto `view` (which already holds the scene).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView, size_px: (u32, u32), pixels_per_point: f32, primitives: &[ClippedPrimitive], deltas: &TexturesDelta) {
        for (id, delta) in &deltas.set {
            self.apply_texture(device, queue, *id, delta);
        }
        let mut verts: Vec<u8> = Vec::new();
        let mut idxs: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        let mut vcount = 0u32;
        for cp in primitives {
            let Primitive::Mesh(mesh) = &cp.primitive else { continue };
            let istart = idxs.len() as u32;
            idxs.extend(mesh.indices.iter().map(|&i| i + vcount));
            for v in &mesh.vertices {
                verts.extend_from_slice(&v.pos.x.to_le_bytes());
                verts.extend_from_slice(&v.pos.y.to_le_bytes());
                verts.extend_from_slice(&v.uv.x.to_le_bytes());
                verts.extend_from_slice(&v.uv.y.to_le_bytes());
                verts.extend_from_slice(&v.color.to_array());
            }
            vcount += mesh.vertices.len() as u32;
            draws.push((cp.clip_rect, mesh.texture_id, istart..idxs.len() as u32));
        }
        if draws.is_empty() {
            for id in &deltas.free {
                self.textures.remove(id);
            }
            return;
        }
        let ibytes: &[u8] = bytemuck::cast_slice(&idxs);
        let need_v = verts.len() as u64;
        let need_i = ibytes.len() as u64;
        if self.vbuf.as_ref().is_none_or(|b| b.size() < need_v) {
            self.vbuf = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("egui-v"), size: need_v.next_power_of_two().max(1 << 16), usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        if self.ibuf.as_ref().is_none_or(|b| b.size() < need_i) {
            self.ibuf = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("egui-i"), size: need_i.next_power_of_two().max(1 << 16), usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        }
        let (vbuf, ibuf) = (self.vbuf.as_ref().unwrap(), self.ibuf.as_ref().unwrap());
        queue.write_buffer(vbuf, 0, &verts);
        queue.write_buffer(ibuf, 0, ibytes);
        let logical = [size_px.0 as f32 / pixels_per_point, size_px.1 as f32 / pixels_per_point, 0.0, 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&logical));
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("egui") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.uniform_bind, &[]);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            pass.set_index_buffer(ibuf.slice(..), wgpu::IndexFormat::Uint32);
            for (clip, tex_id, range) in draws {
                let Some((_, bind)) = self.textures.get(&tex_id) else { continue };
                let cx = (clip.min.x * pixels_per_point).max(0.0) as u32;
                let cy = (clip.min.y * pixels_per_point).max(0.0) as u32;
                let cx1 = ((clip.max.x * pixels_per_point) as u32).min(size_px.0);
                let cy1 = ((clip.max.y * pixels_per_point) as u32).min(size_px.1);
                if cx1 <= cx || cy1 <= cy {
                    continue;
                }
                pass.set_scissor_rect(cx, cy, cx1 - cx, cy1 - cy);
                pass.set_bind_group(1, bind, &[]);
                pass.draw_indexed(range, 0, 0..1);
            }
        }
        queue.submit([enc.finish()]);
        for id in &deltas.free {
            self.textures.remove(id);
        }
    }
}

//! The mesh view (bug round #6): the real mesh of the terrain and the water,
//! tinted by the region it belongs to, so the boundaries between regions can
//! be read off the screen. Two settings that combine:
//!
//! - **Tint** (the M key): nothing, LOD level, chunk, cube face, greedy quad,
//!   block, Koppen biome, tectonic plate, or crust. The last three are layers
//!   of the bake (`regions.rs`), looked up per pixel at the bake's own
//!   resolution.
//! - **Lines** (Shift+M): none, edges over the picture, or edges only (dark
//!   terrain, black sky, the lines in the tint's colours).
//!
//! It stays off the fast path. The pipelines and their shader module are
//! built the first time the view is switched on; with it off, `render` takes
//! the same draws as before behind one comparison. The edges are found in the
//! fragment shader (`mesh_view.wgsl`), not by a polygon-line pipeline: that
//! needs a device feature the game otherwise does without, draws one-pixel
//! lines only, and cannot tell a quad's edge from its diagonal.
//!
//! It changes nothing but the picture: no chunk, no replay and no simulation
//! state knows it exists. It does not touch the normal view, and it is not a
//! distant mesh: everything it draws is the voxel mesh the game already has.

use super::regions::RegionData;
use bytemuck::{Pod, Zeroable};
use std::sync::OnceLock;

/// What the mesh is coloured by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tint {
    /// The game's own colours.
    #[default]
    Off,
    Lod,
    Chunk,
    Face,
    Quad,
    Block,
    Biome,
    Plate,
    Crust,
}

impl Tint {
    pub const ALL: [Tint; 9] = [Tint::Off, Tint::Lod, Tint::Chunk, Tint::Face, Tint::Quad, Tint::Block, Tint::Biome, Tint::Plate, Tint::Crust];

    /// The next setting (the M key).
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    /// The word scripts and sidecars use.
    pub fn name(self) -> &'static str {
        match self {
            Tint::Off => "off",
            Tint::Lod => "lod",
            Tint::Chunk => "chunk",
            Tint::Face => "face",
            Tint::Quad => "quad",
            Tint::Block => "block",
            Tint::Biome => "biome",
            Tint::Plate => "plate",
            Tint::Crust => "crust",
        }
    }

    /// What the look panel and the on-screen legend call it.
    pub fn label(self) -> &'static str {
        match self {
            Tint::Off => "Normal",
            Tint::Lod => "LOD level",
            Tint::Chunk => "Chunk",
            Tint::Face => "Cube face",
            Tint::Quad => "Quad",
            Tint::Block => "Block",
            Tint::Biome => "Biome",
            Tint::Plate => "Tectonic plate",
            Tint::Crust => "Crust",
        }
    }

    /// One line on what the colours mean.
    pub fn hint(self) -> &'static str {
        match self {
            Tint::Off => "The game's own colours.",
            Tint::Lod => "Each level of detail in its own colour.",
            Tint::Chunk => "Every chunk in its own colour, in a light and dark checkerboard.",
            Tint::Face => "The six faces of the cube the planet is built on.",
            Tint::Quad => "Every greedy quad in its own colour.",
            Tint::Block => "Each kind of block in its own colour (water included).",
            Tint::Biome => "The Koppen climate class from the world bake.",
            Tint::Plate => "The tectonic plates from the world bake.",
            Tint::Crust => "Oceanic and continental crust from the world bake.",
        }
    }

    /// Does this tint need the bake's region layers?
    pub fn uses_bake(self) -> bool {
        matches!(self, Tint::Biome | Tint::Plate | Tint::Crust)
    }
}

/// What lines are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lines {
    #[default]
    Off,
    /// Over the picture.
    Edges,
    /// Alone: dark terrain, black sky, water with no fill.
    Only,
}

impl Lines {
    pub const ALL: [Lines; 3] = [Lines::Off, Lines::Edges, Lines::Only];

    /// The next setting (Shift+M).
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    pub fn name(self) -> &'static str {
        match self {
            Lines::Off => "nolines",
            Lines::Edges => "edges",
            Lines::Only => "wire",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lines::Off => "No lines",
            Lines::Edges => "Edges",
            Lines::Only => "Edges only",
        }
    }
}

/// The whole setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshView {
    pub tint: Tint,
    pub lines: Lines,
}

impl MeshView {
    pub const NORMAL: MeshView = MeshView { tint: Tint::Off, lines: Lines::Off };

    pub fn is_normal(self) -> bool {
        self == Self::NORMAL
    }

    /// As scripts and sidecars write it: `normal`, `lod`, `edges`, `wire`,
    /// `lod+edges`.
    pub fn name(self) -> String {
        match (self.tint, self.lines) {
            (Tint::Off, Lines::Off) => "normal".into(),
            (Tint::Off, l) => l.name().into(),
            (t, Lines::Off) => t.name().into(),
            (t, l) => format!("{}+{}", t.name(), l.name()),
        }
    }

    /// What the legend calls it.
    pub fn label(self) -> String {
        match (self.tint, self.lines) {
            (Tint::Off, Lines::Off) => "Normal".into(),
            (Tint::Off, l) => l.label().into(),
            (t, Lines::Off) => t.label().into(),
            (t, l) => format!("{} + {}", t.label(), l.label().to_lowercase()),
        }
    }

    /// Read words like `lod`, `wire`, `lod edges`, `lod+edges`, `normal`.
    pub fn parse(words: &[&str]) -> Option<Self> {
        let mut v = Self::NORMAL;
        let mut any = false;
        for w in words.iter().flat_map(|w| w.split('+')) {
            any = true;
            if w == "normal" {
                continue;
            }
            if let Some(t) = Tint::ALL.into_iter().find(|t| t.name() == w) {
                v.tint = t;
            } else if let Some(l) = Lines::ALL.into_iter().find(|l| l.name() == w) {
                v.lines = l;
            } else {
                return None;
            }
        }
        any.then_some(v)
    }
}

/// A chunk's word for the shader, kept in its draw record: face (3 bits),
/// LOD (4), which square of a 3D checkerboard (1), then a hash of its key
/// (24), for a colour of its own.
pub fn region_word(face: u8, lod: u8, cx: u32, cy: u32, cz: u32) -> u32 {
    let checker = (cx ^ cy ^ cz) & 1;
    let mut h = (face as u64) | (lod as u64) << 3 | (cx as u64) << 8 | (cy as u64) << 32;
    h ^= (cz as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h = (h ^ (h >> 32)).wrapping_mul(0x94D0_49BB_1331_11EB);
    let hash = ((h >> 40) & 0xFF_FFFF) as u32;
    (face as u32 & 7) | (lod as u32 & 15) << 3 | checker << 7 | hash << 8
}

const PALETTE_SIZE: usize = 64;

fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let channel = |k: f32| {
        let p = ((h + k).fract() * 6.0 - 3.0).abs();
        v * (1.0 + ((p - 1.0).clamp(0.0, 1.0) - 1.0) * s)
    };
    [channel(1.0), channel(2.0 / 3.0), channel(1.0 / 3.0)]
}

/// The categorical colours (linear RGB): the legend and the shader read this
/// one table. Neighbouring numbers are far apart in hue and differ in
/// strength, so LODs (which sit next to each other in order) and classes
/// numbered side by side contrast at a boundary.
pub fn palette() -> &'static [[f32; 3]; PALETTE_SIZE] {
    static P: OnceLock<[[f32; 3]; PALETTE_SIZE]> = OnceLock::new();
    P.get_or_init(|| std::array::from_fn(|i| hsv((0.13 + i as f32 * 0.618_034).fract(), if i % 2 == 0 { 0.82 } else { 0.62 }, if i % 3 == 2 { 0.62 } else { 0.95 })))
}

/// Palette entry `i` as display bytes (the legend).
pub fn palette_srgb(i: usize) -> [u8; 3] {
    palette()[i % PALETTE_SIZE].map(super::textures::to_srgb)
}

/// The Koppen-Geiger classes in the bake's numbering (planetgen's list).
pub const KOPPEN: [&str; 30] = ["Af", "Am", "Aw", "BWh", "BWk", "BSh", "BSk", "Csa", "Csb", "Csc", "Cwa", "Cwb", "Cwc", "Cfa", "Cfb", "Cfc", "Dsa", "Dsb", "Dsc", "Dsd", "Dwa", "Dwb", "Dwc", "Dwd", "Dfa", "Dfb", "Dfc", "Dfd", "ET", "EF"];
/// Cube faces, by `cubesphere::FACES` order.
pub const FACE_NAMES: [&str; 6] = ["+X", "-X", "+Y", "-Y", "+Z north", "-Z south"];

/// What the on-screen legend shows: a title, and named colours where naming
/// them helps (chunks and quads are numberless, so they get a sentence).
#[derive(Clone, Debug)]
pub struct Legend {
    pub title: String,
    pub note: String,
    pub items: Vec<(String, [u8; 3])>,
}

/// The legend for a setting. `drawn` is chunks drawn per LOD, `blocks` the
/// (id, name) of the blocks worth listing, `plates` how many plates the bake
/// has, `loading` true while the bake's layers are on their way, `missing` when
/// the bake could not give them.
pub fn legend(view: MeshView, drawn: &[u32; 16], blocks: &[(u16, String)], plates: u32, loading: bool, missing: Option<&str>) -> Legend {
    let named = |names: Vec<(usize, String)>| names.into_iter().map(|(i, n)| (n, palette_srgb(i))).collect::<Vec<_>>();
    let mut note = view.tint.hint().to_string();
    if view.tint == Tint::Off {
        note.clear();
    }
    let mut items = match view.tint {
        Tint::Lod => named(drawn.iter().enumerate().filter(|(_, n)| **n > 0).map(|(l, _)| (l, format!("L{l}"))).collect()),
        Tint::Off if view.lines != Lines::Off => named(drawn.iter().enumerate().filter(|(_, n)| **n > 0).map(|(l, _)| (l, format!("L{l}"))).collect()),
        Tint::Face => named(FACE_NAMES.iter().enumerate().map(|(i, n)| (i, n.to_string())).collect()),
        Tint::Block => named(blocks.iter().map(|(id, n)| (*id as usize, n.clone())).collect()),
        Tint::Biome => named(KOPPEN.iter().enumerate().map(|(i, n)| (i, n.to_string())).collect()),
        Tint::Plate => named((0..plates as usize).map(|i| (i, format!("{i}"))).collect()),
        Tint::Crust => named(vec![(0, "oceanic".to_string()), (1, "continental".to_string())]),
        _ => Vec::new(),
    };
    if view.tint.uses_bake() {
        if let Some(why) = missing {
            items.clear();
            note = format!("Not available: {why}");
        } else if loading {
            items.clear();
            note = "Loading from the bake...".into();
        }
    }
    Legend { title: view.label(), note, items }
}

/// `legend` for a running game, None with the view off.
pub fn legend_for(view: MeshView, drawn: &[u32; 16], registry: &crate::world::Registry, regions: &super::regions::RegionsState) -> Option<Legend> {
    use super::regions::RegionsState as R;
    if view.is_normal() {
        return None;
    }
    let blocks: Vec<(u16, String)> = if view.tint == Tint::Block { registry.defs().iter().filter(|d| d.id != 0 && !d.name.starts_with("unused_") && !d.name.starts_with("marker_")).map(|d| (d.id, d.name.replace('_', " "))).collect() } else { Vec::new() };
    let (plates, loading, missing) = match regions {
        R::Ready { plates } => (*plates, false, None),
        R::Missing(e) => (0, false, Some(e.as_str())),
        _ => (0, true, None),
    };
    Some(legend(view, drawn, &blocks, plates, loading, missing))
}

/// The view's settings and tables, as the shader reads them.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    tint: u32,
    lines: u32,
    regions_ready: u32,
    pad: u32,
    faces: [[f32; 4]; 18],
    palette: [[f32; 4]; PALETTE_SIZE],
}

/// The view's pipelines and data. Built once, when first needed.
pub struct Pipelines {
    /// Edges only: terrain as a dark fill, written to depth.
    pub fill: wgpu::RenderPipeline,
    /// The tinted surfaces, in place of the terrain and water pipelines.
    pub tint_terrain: wgpu::RenderPipeline,
    pub tint_water: wgpu::RenderPipeline,
    /// Terrain lines over what was drawn, tested against the terrain's depth.
    pub lines_terrain: wgpu::RenderPipeline,
    /// Water lines: both sides of a quad, tested the same way.
    pub lines_water: wgpu::RenderPipeline,
    pub bind: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    regions: wgpu::Texture,
    regions_ready: bool,
    /// What `params` holds.
    written: Option<(MeshView, bool)>,
}

fn region_texture(device: &wgpu::Device, res: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mesh-regions"),
        size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: 6 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, params: &wgpu::Buffer, regions: &wgpu::Texture) -> wgpu::BindGroup {
    let view = regions.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("mesh-view"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) }],
    })
}

impl Pipelines {
    pub fn new(device: &wgpu::Device, terrain_layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> Self {
        let text = concat!(include_str!("shader.wgsl"), "\n", include_str!("mesh_view.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("mesh-view"), source: wgpu::ShaderSource::Wgsl(text.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh-view"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Uint, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("mesh-view"), bind_group_layouts: &[Some(terrain_layout), Some(&bgl)], immediate_size: 0 });
        let pipeline = |label: &str, vs: &str, fs: &str, blend: Option<wgpu::BlendState>, cull: Option<wgpu::Face>, ds: wgpu::DepthStencilState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, front_face: wgpu::FrontFace::Ccw, cull_mode: cull, ..Default::default() },
                depth_stencil: Some(ds),
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
        let depth = |write: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias: Default::default(),
        };
        // Over a surface already in the depth buffer: reversed Z, so a
        // positive bias moves the lines toward the eye, and a line never
        // loses to the face it lies on.
        let over = wgpu::DepthStencilState { bias: wgpu::DepthBiasState { constant: 8, slope_scale: 1.0, clamp: 0.0 }, ..depth(false, wgpu::CompareFunction::GreaterEqual) };
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        use wgpu::CompareFunction::Greater;
        let fill = pipeline("mesh-fill", "vs_mesh", "fs_fill", None, Some(wgpu::Face::Back), depth(true, Greater));
        let tint_terrain = pipeline("mesh-tint", "vs_mesh", "fs_tint", None, Some(wgpu::Face::Back), depth(true, Greater));
        let tint_water = pipeline("mesh-tint-water", "vs_mesh_water", "fs_tint_water", blend, None, depth(false, Greater));
        let lines_terrain = pipeline("mesh-lines", "vs_mesh", "fs_lines", blend, Some(wgpu::Face::Back), over.clone());
        let lines_water = pipeline("mesh-lines-water", "vs_mesh_water", "fs_lines_water", blend, None, over);
        let params = device.create_buffer(&wgpu::BufferDescriptor { label: Some("mesh-view"), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        // One empty texel per face until the bake's layers arrive.
        let regions = region_texture(device, 1);
        let bind = bind_group(device, &bgl, &params, &regions);
        Self { fill, tint_terrain, tint_water, lines_terrain, lines_water, bind, layout: bgl, params, regions, regions_ready: false, written: None }
    }

    /// Bring the shader's settings up to `view`.
    pub fn set_view(&mut self, queue: &wgpu::Queue, view: MeshView) {
        if self.written == Some((view, self.regions_ready)) {
            return;
        }
        self.written = Some((view, self.regions_ready));
        let mut faces = [[0.0; 4]; 18];
        for (f, (axis, right, up)) in cubesphere::FACES.iter().enumerate() {
            for (n, v) in [axis, right, up].into_iter().enumerate() {
                faces[f * 3 + n] = [v.x as f32, v.y as f32, v.z as f32, 0.0];
            }
        }
        let p = Params { tint: view.tint as u32, lines: view.lines as u32, regions_ready: self.regions_ready as u32, pad: 0, faces, palette: palette().map(|c| [c[0], c[1], c[2], 1.0]) };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
    }

    /// Take the bake's region layers.
    pub fn set_regions(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &RegionData) {
        self.regions = region_texture(device, data.res);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.regions, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &data.rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * data.res), rows_per_image: Some(data.res) },
            wgpu::Extent3d { width: data.res, height: data.res, depth_or_array_layers: 6 },
        );
        self.bind = bind_group(device, &self.layout, &self.params, &self.regions);
        self.regions_ready = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M and Shift+M step through their settings and come back; scripts and
    /// sidecars name every combination.
    #[test]
    fn the_settings_cycle_and_have_names() {
        let mut t = Tint::default();
        let mut seen = Vec::new();
        for _ in 0..Tint::ALL.len() {
            seen.push(t.name());
            t = t.next();
        }
        assert_eq!(t, Tint::Off);
        assert_eq!(seen, ["off", "lod", "chunk", "face", "quad", "block", "biome", "plate", "crust"]);
        assert_eq!(Lines::Only.next(), Lines::Off);
        for tint in Tint::ALL {
            for lines in Lines::ALL {
                let v = MeshView { tint, lines };
                assert_eq!(MeshView::parse(&[&v.name()]), Some(v), "{}", v.name());
            }
        }
        assert_eq!(MeshView::parse(&["lod", "edges"]), Some(MeshView { tint: Tint::Lod, lines: Lines::Edges }));
        assert_eq!(MeshView::parse(&["normal"]), Some(MeshView::NORMAL));
        assert_eq!(MeshView::parse(&["solid"]), None);
        assert_eq!(MeshView::parse(&[]), None);
        assert!(MeshView::default().is_normal());
    }

    /// The shader's numbering is the enum's.
    #[test]
    fn the_shader_numbers_the_tints_as_the_enum_does() {
        assert_eq!((Tint::Chunk as u32, Tint::Face as u32, Tint::Quad as u32, Tint::Block as u32), (2, 3, 4, 5));
        assert_eq!((Tint::Biome as u32, Tint::Plate as u32, Tint::Crust as u32), (6, 7, 8));
        assert_eq!((Lines::Off as u32, Lines::Edges as u32, Lines::Only as u32), (0, 1, 2));
    }

    /// Colours next to each other in the table differ plainly: a boundary
    /// between LOD n and n + 1, or classes n and n + 1, can be seen.
    #[test]
    fn neighbouring_palette_colours_are_far_apart() {
        for i in 0..PALETTE_SIZE - 1 {
            let (a, b) = (palette_srgb(i), palette_srgb(i + 1));
            let d = (0..3).map(|c| (a[c] as i32 - b[c] as i32).abs()).max().unwrap();
            assert!(d > 60, "{i}: {a:?} vs {b:?}");
        }
    }

    /// Chunks that share a face are always on opposite squares of the
    /// checkerboard (light against dark); the hash gives each its own hue.
    #[test]
    fn neighbouring_chunks_never_share_a_square() {
        let w = |x, y, z| region_word(2, 0, x, y, z);
        for (x, y, z) in [(10, 20, 30), (11, 20, 30), (0, 0, 0), (4095, 7, 1)] {
            for (a, b, c) in [(x + 1, y, z), (x, y + 1, z), (x, y, z + 1)] {
                assert_ne!(w(x, y, z) >> 7 & 1, w(a, b, c) >> 7 & 1);
                assert_ne!(w(x, y, z) >> 8, w(a, b, c) >> 8);
            }
        }
        // The word holds the face and the LOD where the shader reads them.
        let v = region_word(5, 9, 3, 4, 5);
        assert_eq!((v & 7, v >> 3 & 15), (5, 9));
    }

    /// Each setting names itself; a legend lists what there is to list.
    #[test]
    fn every_setting_has_a_legend() {
        let mut drawn = [0u32; 16];
        drawn[0] = 5;
        drawn[3] = 2;
        let blocks = vec![(3u16, "grass".to_string()), (8, "water".to_string())];
        let at = |tint, lines| legend(MeshView { tint, lines }, &drawn, &blocks, 7, false, None);
        for tint in Tint::ALL {
            for lines in Lines::ALL {
                let l = at(tint, lines);
                assert!(!l.title.is_empty());
            }
        }
        assert_eq!(at(Tint::Lod, Lines::Off).items.iter().map(|i| i.0.as_str()).collect::<Vec<_>>(), ["L0", "L3"]);
        assert_eq!(at(Tint::Lod, Lines::Off).items[0].1, palette_srgb(0));
        assert_eq!(at(Tint::Off, Lines::Only).items.len(), 2, "lines take the LOD colours");
        assert_eq!(at(Tint::Face, Lines::Off).items.len(), 6);
        assert_eq!(at(Tint::Block, Lines::Off).items[1], ("water".to_string(), palette_srgb(8)));
        assert_eq!(at(Tint::Biome, Lines::Off).items.len(), 30);
        assert_eq!(at(Tint::Plate, Lines::Off).items.len(), 7);
        assert_eq!(at(Tint::Crust, Lines::Edges).title, "Crust + edges");
        assert!(at(Tint::Chunk, Lines::Off).items.is_empty() && !at(Tint::Chunk, Lines::Off).note.is_empty());
        let l = legend(MeshView { tint: Tint::Biome, lines: Lines::Off }, &drawn, &blocks, 0, false, Some("this bake has no koppen layer"));
        assert!(l.items.is_empty() && l.note.contains("koppen"));
    }
}

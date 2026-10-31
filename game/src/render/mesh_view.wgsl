// The mesh view (bug round #6; render/meshview.rs): the real mesh, tinted by
// the region it belongs to, with its quads outlined or alone. This file is
// appended to shader.wgsl for its own shader module, built the first time the
// view is switched on; the normal pipelines never see it. Every position comes
// from shader.wgsl's `vertex`, so what is tinted and outlined here is exactly
// what the normal pass draws, sloped water corners and all.
//
// A tinted surface has a fixed light (no sun, no fog), so a region reads the
// same at any hour and any distance.
//
// A quad's corners carry their coordinates across it, so its edges are found
// per pixel and keep a steady width at any distance. The two triangles share
// a vertex pair, the diagonal x + y = 1: drawn thinner, so a quad and its
// triangles read differently. A chunk's border is drawn heavier and whiter.

struct MeshParams {
    tint: u32,          // Tint, as numbered in meshview.rs
    lines: u32,         // 0 none, 1 edges, 2 edges only
    regions_ready: u32, // the bake's region layers are in `regions`
    pad: u32,
    faces: array<vec4<f32>, 18>,  // axis, right, up of each cube face
    palette: array<vec4<f32>, 64>,
};

@group(1) @binding(0) var<uniform> mesh: MeshParams;
// Koppen class, tectonic plate, crust: one layer per cube face, as the bake.
@group(1) @binding(1) var regions: texture_2d_array<u32>;

struct MeshOut {
    @builtin(position) pos: vec4<f32>,
    // 0..1 across the quad, along its own u and v.
    @location(0) quad: vec2<f32>,
    // Voxels from the chunk's corner along u and v (0..32 inside the chunk).
    @location(1) lattice: vec2<f32>,
    @location(2) view: vec3<f32>,
    @location(3) normal: vec3<f32>,
    @location(4) shade: f32,
    // Voxel size (m), and the chunk's region word (meshview::region_word).
    @location(5) @interpolate(flat) voxel: f32,
    // Region word, block id, quad number within the chunk.
    @location(6) @interpolate(flat) ids: vec3<u32>,
};

const MESH_CHUNK: f32 = 32.0;
const MESH_WATER_LINE: vec3<f32> = vec3<f32>(0.10, 0.95, 1.00);
const MESH_FLAT: vec3<f32> = vec3<f32>(0.42, 0.42, 0.42);
const MESH_QUARTER_PI: f32 = 0.7853981633974483;

fn mesh_vertex(vi: u32, ii: u32, water: bool) -> MeshOut {
    let d = draws[ii];
    // The quad's two words, stored as in `vertex`.
    var q: vec2<u32>;
    if (water) {
        q = quads[d.quad_base + 2u * ((vi >> 2u) - d.quad_base)];
    } else {
        q = quads[vi >> 2u];
    }
    let axis = q.x & 3u;
    let sign = (q.x >> 2u) & 1u;
    let u0 = (q.x >> 8u) & 31u;
    let v0 = (q.x >> 13u) & 31u;
    let w = ((q.x >> 18u) & 31u) + 1u;
    let h = ((q.x >> 23u) & 31u) + 1u;
    let c = vi & 3u;
    var cu = c & 1u;
    var cv = c >> 1u;
    if ((sign == 0u) != (axis == 1u)) {
        let t = cu;
        cu = cv;
        cv = t;
    }
    let v = vertex(vi, ii, water);
    var out: MeshOut;
    out.pos = v.pos;
    out.quad = vec2<f32>(f32(cu), f32(cv));
    out.lattice = vec2<f32>(f32(u0 + cu * w), f32(v0 + cv * h));
    out.view = v.view;
    out.normal = v.normal;
    out.shade = v.face_shade * v.ao;
    out.voxel = d.voxel;
    // Counted from the draw's first quad: where a chunk landed in the quad
    // buffer differs between runs, and a quad's colour shouldn't.
    out.ids = vec3<u32>(d.pad1, q.y & 0xFFFFu, (vi >> 2u) - d.quad_base);
    return out;
}

@vertex
fn vs_mesh(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MeshOut {
    return mesh_vertex(vi, ii, false);
}

@vertex
fn vs_mesh_water(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MeshOut {
    return mesh_vertex(vi, ii, true);
}

fn mesh_palette(i: u32) -> vec3<f32> {
    return mesh.palette[i % 64u].rgb;
}

fn mesh_hsv(h: f32, s: f32, v: f32) -> vec3<f32> {
    let k = vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0);
    let p = abs(fract(vec3<f32>(h) + k) * 6.0 - vec3<f32>(3.0));
    return v * mix(vec3<f32>(1.0), clamp(p - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0)), s);
}

fn mesh_lod(voxel: f32) -> u32 {
    // sky.tex.x is blocks per metre, so this is the voxel's size in blocks.
    return u32(round(log2(max(voxel * sky.tex.x, 1.0))));
}

// The bake's region layers at a point (camera-relative metres): the texel the
// bake's own lookup picks (`RasterBake::texel_coords`), nearest.
fn mesh_region(rel: vec3<f32>) -> vec4<u32> {
    let d = normalize(sky.eye.xyz * sky.eye.w + rel);
    let m = abs(d);
    var f = 0u;
    if (m.x >= m.y && m.x >= m.z) {
        f = select(1u, 0u, d.x > 0.0);
    } else if (m.y >= m.z) {
        f = select(3u, 2u, d.y > 0.0);
    } else {
        f = select(5u, 4u, d.z > 0.0);
    }
    let z = dot(d, mesh.faces[f * 3u].xyz);
    let a = dot(d, mesh.faces[f * 3u + 1u].xyz) / z;
    let b = dot(d, mesh.faces[f * 3u + 2u].xyz) / z;
    let top = f32(textureDimensions(regions).x) - 1.0;
    let x = (atan(a) / MESH_QUARTER_PI * 0.5 + 0.5) * top;
    let y = (atan(b) / MESH_QUARTER_PI * 0.5 + 0.5) * top;
    return textureLoad(regions, vec2<i32>(i32(round(x)), i32(round(y))), i32(f), 0);
}

// The colour of this pixel's region.
fn mesh_tint(in: MeshOut) -> vec3<f32> {
    let word = in.ids.x;
    switch mesh.tint {
        // Chunk: a checkerboard of light and dark, so face neighbours always
        // differ, in a hue of the chunk's own.
        case 2u: {
            let hue = f32(word >> 8u) / 16777216.0;
            return mesh_hsv(hue, 0.78, select(0.42, 1.0, ((word >> 7u) & 1u) == 1u));
        }
        case 3u: { return mesh_palette(word & 7u); }
        // Quad: quads next to each other in a chunk are near in the list, and
        // the golden ratio keeps near numbers' hues apart.
        case 4u: {
            let hue = fract(f32(in.ids.z & 0xFFFFu) * 0.6180339887 + f32(word >> 8u) / 16777216.0);
            return mesh_hsv(hue, 0.75, select(0.6, 1.0, (in.ids.z & 1u) == 1u));
        }
        case 5u: { return mesh_palette(in.ids.y); }
        case 6u, 7u, 8u: {
            if (mesh.regions_ready == 0u) {
                return MESH_FLAT;
            }
            let r = mesh_region(in.view);
            let id = r[mesh.tint - 6u];
            // 255: the sea has no Koppen class.
            if (id == 255u) {
                return vec3<f32>(0.04, 0.09, 0.20);
            }
            return mesh_palette(id);
        }
        default: { return mesh_palette(mesh_lod(in.voxel)); }
    }
}

fn mesh_shaded(in: MeshOut) -> vec3<f32> {
    let up = normalize(sky.eye.xyz + in.view / sky.eye.w);
    let lit = 0.62 + 0.38 * max(dot(normalize(in.normal), up), 0.0);
    return mesh_tint(in) * lit * in.shade;
}

@fragment
fn fs_tint(in: MeshOut) -> @location(0) vec4<f32> {
    return vec4<f32>(mesh_shaded(in), 1.0);
}

@fragment
fn fs_tint_water(in: MeshOut) -> @location(0) vec4<f32> {
    let a = 0.62;
    return vec4<f32>(mesh_shaded(in) * a, a);
}

// Edges only: terrain is a dark fill that hides what is behind it (the sky
// stays black, so a crack in the mesh is a black hole in the slate).
@fragment
fn fs_fill(in: MeshOut) -> @location(0) vec4<f32> {
    let lit = 0.55 + 0.45 * in.shade;
    return vec4<f32>(vec3<f32>(0.030, 0.038, 0.050) * lit, 1.0);
}

// The lines at this pixel: x = quad edge or diagonal coverage, y = chunk border.
fn mesh_lines(in: MeshOut) -> vec2<f32> {
    let width = max(fwidth(in.quad), vec2<f32>(1e-6));
    let to_edge = min(in.quad, vec2<f32>(1.0) - in.quad) / width;
    let edge = 1.0 - smoothstep(0.4, 1.3, min(to_edge.x, to_edge.y));
    let s = in.quad.x + in.quad.y - 1.0;
    let diagonal = 1.0 - smoothstep(0.1, 0.8, abs(s) / max(fwidth(s), 1e-6));
    let cells = max(fwidth(in.lattice), vec2<f32>(1e-6));
    let to_border = min(in.lattice, vec2<f32>(MESH_CHUNK) - in.lattice) / cells;
    let border = 1.0 - smoothstep(1.0, 2.2, min(to_border.x, to_border.y));
    // Quads a pixel or two across would be solid line: let them fade to a
    // wash of their colour instead of hiding what is behind them.
    let crowd = max(width.x, width.y);
    let thin = mix(1.0, 0.35, smoothstep(0.25, 0.8, crowd));
    return vec2<f32>(max(edge, diagonal * 0.45) * thin, border);
}

fn mesh_line_colour(in: MeshOut, water: bool) -> vec4<f32> {
    let l = mesh_lines(in);
    let a = max(l.x, l.y);
    if (a < 0.004) {
        discard;
    }
    var c: vec3<f32>;
    if (mesh.tint != 0u && mesh.lines == 2u) {
        c = mesh_tint(in);
    } else if (mesh.tint != 0u) {
        // Over a tinted surface: dark lines read on every colour.
        c = vec3<f32>(0.02, 0.02, 0.025);
    } else {
        c = mesh_palette(mesh_lod(in.voxel));
        if (water) {
            c = mix(c, MESH_WATER_LINE, 0.75);
        }
    }
    c = mix(c, vec3<f32>(1.0), l.y * 0.75);
    return vec4<f32>(c * a, a);
}

@fragment
fn fs_lines(in: MeshOut) -> @location(0) vec4<f32> {
    return mesh_line_colour(in, false);
}

@fragment
fn fs_lines_water(in: MeshOut) -> @location(0) vec4<f32> {
    return mesh_line_colour(in, true);
}

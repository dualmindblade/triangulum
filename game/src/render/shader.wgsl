// Vertex-pulled greedy quads on a cube-sphere, camera-relative.
//
// Quad: lo = axis:2 | sign:1 | slice:5 | u:5 | v:5 | w-1:5 | h-1:5;
//       hi = block id:16 | ao:8 (2 bits per corner, corner = cu + 2 cv).

struct Camera {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,      // xyz world sun direction
    cam_up: vec4<f32>,       // xyz local up at camera, w = altitude (m)
    fog: vec4<f32>,          // x = 1/fog range (m), y = sky darkening (0 ground .. 1 space), z = water alpha
    sky_horizon: vec4<f32>,
    sky_zenith: vec4<f32>,
    light: vec4<f32>,        // x = ambient, y = diffuse, z = AO strength, w = unused
};

struct Draw {
    origin: vec3<f32>,  // chunk lattice corner, camera-relative
    voxel: f32,         // voxel size in metres
    slot: u32,          // curvature grid slot
    quad_base: u32,
    pad0: u32,
    pad1: u32,
};

struct Grid {
    off: array<vec4<f32>, 25>,
    dir: array<vec4<f32>, 25>,
};

@group(0) @binding(0) var<uniform> cam: Camera;
@group(0) @binding(1) var<storage, read> quads: array<vec2<u32>>;
@group(0) @binding(2) var<storage, read> grids: array<Grid>;
@group(0) @binding(3) var<storage, read> draws: array<Draw>;
@group(0) @binding(4) var<storage, read> colours: array<vec4<f32>>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) fog: f32,
    @location(2) ao: f32,
    @location(3) light: f32,
    @location(4) view: vec3<f32>,
    @location(5) normal: vec3<f32>,
};

// Shared vertex transform: returns the camera-relative world position and
// fills the shading inputs.
fn vertex(vi: u32, ii: u32) -> VOut {
    let d = draws[ii];
    let q = quads[vi >> 2u];
    let c = vi & 3u;
    let axis = q.x & 3u;
    let sign = (q.x >> 2u) & 1u;
    let slice = (q.x >> 3u) & 31u;
    let u0 = (q.x >> 8u) & 31u;
    let v0 = (q.x >> 13u) & 31u;
    let w = ((q.x >> 18u) & 31u) + 1u;
    let h = ((q.x >> 23u) & 31u) + 1u;
    let block = q.y & 0xFFFFu;

    // Winding: normal = U x V must point along +axis for sign=1. For axes X
    // and Z, U x V is +axis; for Y it is -axis.
    let flip = (sign == 0u) != (axis == 1u);
    var cu = c & 1u;
    var cv = c >> 1u;
    if (flip) {
        let t = cu;
        cu = cv;
        cv = t;
    }
    let ao_corner = cu + 2u * cv;
    let ao = f32((q.y >> (16u + 2u * ao_corner)) & 3u) / 3.0;
    let pu = f32(u0 + cu * w);
    let pv = f32(v0 + cv * h);
    let ps = f32(slice + sign);
    var lp: vec3<f32>;
    switch axis {
        case 0u: { lp = vec3<f32>(ps, pu, pv); }
        case 1u: { lp = vec3<f32>(pu, ps, pv); }
        default: { lp = vec3<f32>(pu, pv, ps); }
    }

    // Bilinear curvature grid over the footprint (4x4 cells of 8 voxels).
    let g = lp.xy * 0.125;
    let gi = min(vec2<u32>(g), vec2<u32>(3u, 3u));
    let gf = g - vec2<f32>(gi);
    let b = gi.y * 5u + gi.x;
    let o00 = grids[d.slot].off[b].xyz;
    let o10 = grids[d.slot].off[b + 1u].xyz;
    let o01 = grids[d.slot].off[b + 5u].xyz;
    let o11 = grids[d.slot].off[b + 6u].xyz;
    let n00 = grids[d.slot].dir[b].xyz;
    let n10 = grids[d.slot].dir[b + 1u].xyz;
    let n01 = grids[d.slot].dir[b + 5u].xyz;
    let n11 = grids[d.slot].dir[b + 6u].xyz;
    let off = mix(mix(o00, o10, gf.x), mix(o01, o11, gf.x), gf.y);
    let up = mix(mix(n00, n10, gf.x), mix(n01, n11, gf.x), gf.y);
    let world = d.origin + off + up * (lp.z * d.voxel);

    // Face normal in world space.
    let sgn = select(-1.0, 1.0, sign == 1u);
    var n: vec3<f32>;
    switch axis {
        case 0u: { n = normalize(o10 - o00) * sgn; }
        case 1u: { n = normalize(o01 - o00) * sgn; }
        default: { n = normalize(up) * sgn; }
    }
    let ndl = max(dot(n, cam.sun_dir.xyz), 0.0);
    let dist = length(world);
    let f = 1.0 - exp(-(dist * dist) * (cam.fog.x * cam.fog.x));

    var out: VOut;
    out.pos = cam.view_proj * vec4<f32>(world, 1.0);
    out.color = colours[block].rgb;
    out.fog = f;
    out.ao = ao;
    out.light = ndl;
    out.view = world;
    out.normal = n;
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    return vertex(vi, ii);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    // Ambient scaled by AO, diffuse lightly scaled by it too (sunlight
    // still reaches most of an open corner).
    let ao = mix(1.0 - cam.light.z, 1.0, in.ao);
    let light = cam.light.x * ao + cam.light.y * in.light * mix(0.6, 1.0, in.ao);
    let col = in.color * light;
    return vec4<f32>(mix(col, cam.sky_horizon.xyz, in.fog), 1.0);
}

@vertex
fn vs_water(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    return vertex(vi, ii);
}

@fragment
fn fs_water(in: VOut) -> @location(0) vec4<f32> {
    let v = normalize(-in.view);
    let n = normalize(in.normal);
    let facing = abs(dot(v, n));
    // Fresnel-ish: more opaque and brighter at grazing angles; a sun glint.
    let alpha = mix(cam.fog.z, 0.95, pow(1.0 - facing, 3.0));
    let r = reflect(-v, n);
    let glint = pow(max(dot(r, cam.sun_dir.xyz), 0.0), 60.0) * 0.6;
    let light = cam.light.x + cam.light.y * in.light;
    var col = in.color * light + vec3<f32>(glint);
    col = mix(col, cam.sky_horizon.xyz, in.fog);
    return vec4<f32>(col * alpha, alpha);
}

// ---- sky ----
struct SkyOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    var p = vec2<f32>(-1.0, -1.0);
    if (vi == 1u) { p = vec2<f32>(3.0, -1.0); }
    if (vi == 2u) { p = vec2<f32>(-1.0, 3.0); }
    var o: SkyOut;
    o.pos = vec4<f32>(p, 0.0, 1.0);
    o.ndc = p;
    return o;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    let pw = cam.inv_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let dir = normalize(pw.xyz / pw.w);
    let e = dot(dir, cam.cam_up.xyz);
    let t = clamp(e * 2.5, 0.0, 1.0);
    var col = mix(cam.sky_horizon.xyz, cam.sky_zenith.xyz, t);
    // A soft sun disc and glow.
    let s = max(dot(dir, cam.sun_dir.xyz), 0.0);
    col += vec3<f32>(1.0, 0.95, 0.8) * (pow(s, 600.0) * 1.5 + pow(s, 8.0) * 0.08);
    // Space darkening with altitude; keep a faint horizon glow.
    let space = vec3<f32>(0.01, 0.01, 0.02);
    col = mix(col, space, cam.fog.y);
    return vec4<f32>(col, 1.0);
}

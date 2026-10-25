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

// Kept separate from the existing camera/mesh interface. Positions in radii
// keep the ephemeris small; vertex positions remain camera-relative metres.
struct Sky {
    sun: vec4<f32>,
    sun_position: vec4<f32>,
    moon_position: vec4<f32>,
    eye: vec4<f32>,
    atmosphere: vec4<f32>,
    fixed_to_inertial: mat4x4<f32>,
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
@group(0) @binding(5) var<uniform> sky: Sky;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) face_shade: f32,
    @location(2) ao: f32,
    @location(3) @interpolate(flat) side: f32,
    @location(4) view: vec3<f32>,
    @location(5) normal: vec3<f32>,
};

// Lagrange weights of the five grid corners (at 0, 1, 2, 3, 4) for t in [0, 4].
fn lagrange5(t: f32) -> array<f32, 5> {
    let t0 = t;
    let t1 = t - 1.0;
    let t2 = t - 2.0;
    let t3 = t - 3.0;
    let t4 = t - 4.0;
    var w: array<f32, 5>;
    w[0] = t1 * t2 * t3 * t4 / 24.0;
    w[1] = t0 * t2 * t3 * t4 / -6.0;
    w[2] = t0 * t1 * t3 * t4 / 4.0;
    w[3] = t0 * t1 * t2 * t4 / -6.0;
    w[4] = t0 * t1 * t2 * t3 / 24.0;
    return w;
}

// Shared vertex transform: returns the camera-relative world position and
// fills the shading inputs. Water quads (M2b) are two slots each: the usual
// two words, then four 8-bit corner heights that lower the top corners of
// a partial cell's faces (255 = the voxel's top, the geometry of full water).
fn vertex(vi: u32, ii: u32, water: bool) -> VOut {
    let d = draws[ii];
    var q: vec2<u32>;
    var heights = 0xFFFFFFFFu;
    if (water) {
        let e = d.quad_base + 2u * ((vi >> 2u) - d.quad_base);
        q = quads[e];
        heights = quads[e + 1u].x;
    } else {
        q = quads[vi >> 2u];
    }
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
    // A water surface sits at its corner height inside the voxel: the top
    // face's four corners and a side face's two top corners drop from the
    // voxel top by (255 - height) / 255 voxels; bottoms and full water stay.
    if (water) {
        let top = (axis == 2u && sign == 1u) || (axis != 2u && cv == 1u);
        if (top) {
            let hc = f32((heights >> (8u * ao_corner)) & 0xFFu);
            lp.z += (hc - 255.0) / 255.0;
        }
    }

    // Curvature grid over the footprint (4x4 cells of 8 voxels): bilinear
    // within a cell, or, at coarse LODs, the quartic through the chunk's
    // five corners along each axis (review 2 #9: a cell is 4 km at LOD 9
    // and the lattice lines curve, so a bilinear mid-cell vertex sat up to
    // 30 m beside the finer neighbour's corner on the same lattice point;
    // the quartic puts it within centimetres, and two chunks of one LOD
    // still evaluate a shared edge from the same five corners).
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
    var off = mix(mix(o00, o10, gf.x), mix(o01, o11, gf.x), gf.y);
    var up = mix(mix(n00, n10, gf.x), mix(n01, n11, gf.x), gf.y);
    if (d.voxel >= 64.0) {
        let wx = lagrange5(g.x);
        let wy = lagrange5(g.y);
        off = vec3<f32>(0.0);
        up = vec3<f32>(0.0);
        for (var j = 0u; j < 5u; j++) {
            for (var i = 0u; i < 5u; i++) {
                let w = wx[i] * wy[j];
                off += grids[d.slot].off[j * 5u + i].xyz * w;
                up += grids[d.slot].dir[j * 5u + i].xyz * w;
            }
        }
    }
    // Exact radius (review #11): the bilinear chord sags r0 (1 - |up|)
    // below the sphere mid-cell (23 m at LOD 9), so coarse and fine water
    // surfaces did not meet. The mesher puts r0, the lattice floor's
    // radius, in off[0].w (mesh.rs GRID_R0_INDEX). Skipped at the three
    // finest LODs, where the sag is under 2 mm and f32 noise in the
    // correction would show up close.
    let len = length(up);
    var sag = 0.0;
    if (d.voxel >= 8.0) { sag = grids[d.slot].off[0].w * (1.0 - len); }
    let world = d.origin + off + (up / len) * (lp.z * d.voxel + sag);

    // Each column's own grid frame, never a position/chunk-dependent guess.
    // Equal-area grid tangents need not be orthogonal: cross products give
    // the actual prism side normals. Keep cube normals for edits/overhangs.
    let sgn = select(-1.0, 1.0, sign == 1u);
    let du = mix(o10 - o00, o11 - o01, gf.y) + mix(n10 - n00, n11 - n01, gf.y) * (lp.z * d.voxel);
    let dv = mix(o01 - o00, o11 - o10, gf.x) + mix(n01 - n00, n11 - n10, gf.x) * (lp.z * d.voxel);
    var n: vec3<f32>;
    switch axis {
        case 0u: { n = normalize(cross(dv, up)) * sgn; }
        case 1u: { n = normalize(cross(up, du)) * sgn; }
        default: { n = normalize(up) * sgn; }
    }

    var out: VOut;
    out.pos = cam.view_proj * vec4<f32>(world, 1.0);
    out.color = colours[block].rgb;
    // July risers 0.90, undersides 0.55; no extra darkening from the old
    // 0.72 riser multiplier or a normal inferred from integer terraces.
    out.face_shade = select(0.90, select(0.55, 1.0, sign == 1u), axis == 2u);
    // Packed AO is brightness 0..3. July's four levels are .55/.70/.85/1.
    // Reduce its footprint at coarse LODs instead of darkening huge spans.
    out.ao = 1.0 - 0.45 * (1.0 - ao) / max(1.0, sqrt(d.voxel));
    out.side = select(1.0, 0.0, axis == 2u);
    out.view = world;
    out.normal = n;
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    return vertex(vi, ii, false);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let col = lighting(in.color * in.face_shade * in.ao, normalize(in.normal), in.view);
    return vec4<f32>(atmosphere(col, in.view), 1.0);
}

fn lighting(base: vec3<f32>, n: vec3<f32>, rel: vec3<f32>) -> vec3<f32> {
    let point = sky.eye.xyz + rel / sky.eye.w;
    let up = normalize(point);
    let sun = normalize(sky.sun_position.xyz - point);
    // Per-fragment horizon everywhere: the orbit's far hemisphere stays dark.
    let day = smoothstep(-0.08, 0.15, dot(sun, up));
    let hemi = clamp(0.5 + 0.5 * dot(n, up), 0.0, 1.0);
    let ambient = 0.10 + 0.40 * day * hemi;
    // A set sun MUST NOT light faces pointing down toward it (July fix).
    let sun_coeff = mix(1.0, 0.60, day) * day;
    var c = base * (ambient + sun_coeff * max(dot(n, sun), 0.0));
    if (day < 1.0) {
        let moon = normalize(sky.moon_position.xyz - point);
        let phase = 0.5 + 0.5 * dot(normalize(sky.sun_position.xyz - sky.moon_position.xyz), -moon);
        c += base * vec3<f32>(0.40, 0.50, 0.72)
            * (0.10 * max(dot(n, moon), 0.0) + 0.015 + 0.05 * hemi)
            * (1.0 - day) * smoothstep(0.0, 0.15, dot(moon, up)) * phase * 1.15;
    }
    return c;
}

fn air_density() -> f32 {
    // Camera-constant exponential is evaluated once in f64 on the CPU.
    return sky.atmosphere.x;
}

fn underwater(c: vec3<f32>) -> vec3<f32> {
    return mix(c, vec3<f32>(0.02, 0.07, 0.16), 0.6 * sky.atmosphere.w);
}

fn atmosphere(c: vec3<f32>, rel: vec3<f32>) -> vec3<f32> {
    let up = normalize(sky.eye.xyz + rel / sky.eye.w);
    let day = smoothstep(-0.08, 0.15, dot(normalize(sky.sun_position.xyz), up));
    let fog = (1.0 - exp(-length(rel) * sky.atmosphere.z)) * air_density() * 0.7;
    return underwater(mix(c, vec3<f32>(0.55, 0.70, 0.88) * (0.15 + 0.85 * day), fog));
}

@vertex
fn vs_water(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
    return vertex(vi, ii, true);
}

@fragment
fn fs_water(in: VOut) -> @location(0) vec4<f32> {
    let v = normalize(-in.view);
    let up = normalize(sky.eye.xyz + in.view / sky.eye.w);
    // July water-step fix: gently sky-facing sides avoid bright/black bands.
    let n = normalize(mix(in.normal, in.normal * 0.18 + up, in.side));
    let facing = abs(dot(v, n));
    // Fresnel-ish: more opaque and brighter at grazing angles; a sun glint.
    let alpha = mix(cam.fog.z, 0.95, pow(1.0 - facing, 3.0));
    let r = reflect(-v, n);
    let sun = normalize(sky.sun_position.xyz);
    let day = smoothstep(-0.08, 0.15, dot(sun, up));
    let glint = pow(max(dot(r, sun), 0.0), 60.0) * 0.6 * day;
    var col = lighting(in.color * mix(1.0, 0.93, in.side), n, in.view) + vec3<f32>(glint);
    col = atmosphere(col, in.view);
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
    // Unproject the NEAR plane: infinity (z=0) is singular for reversed-Z.
    let pw = cam.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(pw.xyz / pw.w);
    let up = normalize(sky.eye.xyz);
    let h = dot(dir, up);
    let sunh = dot(sky.sun.xyz, up);
    let day = smoothstep(-0.08, 0.15, sunh);
    let d = dot(dir, sky.sun.xyz);
    let toward = pow(max(d, 0.0), 6.0);
    let low = 1.0 - smoothstep(0.05, 0.35, sunh);
    var c = mix(vec3<f32>(0.55, 0.70, 0.88), vec3<f32>(0.10, 0.28, 0.62), pow(clamp(h, 0.0, 1.0), 0.55)) * (0.03 + 0.97 * day);
    c = mix(c, vec3<f32>(0.95, 0.55, 0.30), toward * low * 0.55 * day);
    let tint = vec3<f32>(1.0, 0.96, 0.86);
    c += tint * pow(max(d, 0.0), 800.0) * 0.50 * day;
    let atm = air_density();
    c *= smoothstep(-0.10, 0.03, h) * atm;

    // July limb, with optical lengths relative to the smaller planet.
    let center = -sky.eye.xyz;
    let along = dot(center, dir);
    if (atm < 0.85 && along > 0.0) {
        let miss = dir * along - center;
        let b = length(miss);
        let lit = smoothstep(-0.15, 0.35, dot(normalize(miss), sky.sun.xyz));
        let shell = exp(-max(b - 1.0, 0.0) * sky.eye.w * sky.atmosphere.y) * smoothstep(0.6, 1.0, b);
        c += vec3<f32>(0.30, 0.50, 0.90) * shell * lit * (1.0 - atm) * 0.8;
    }
    let sky_lum = dot(c, vec3<f32>(0.35, 0.45, 0.20));
    let star_visibility = 1.0 - smoothstep(0.01, 0.18, sky_lum);
    if (star_visibility > 0.0) {
        let star_dir = (sky.fixed_to_inertial * vec4<f32>(dir, 0.0)).xyz;
        c += stars(star_dir) * star_visibility;
    }

    // Analytic silhouette of the same 109 R / ~12190 R sphere as July.
    // No additional mesh/pass; derivative AA keeps the ~0.512-degree radius.
    let edge = max(fwidth(d), 0.0000001);
    let disc = smoothstep(sky.sun.w - edge, sky.sun.w + edge, d);
    let visibility = mix(smoothstep(-0.08, 0.03, sunh), 1.0, 1.0 - atm);
    c = mix(c, tint * 3.2, disc * visibility);
    return vec4<f32>(underwater(c), 1.0);
}

// July's directional cells, jitter, temperatures and brightness. The input
// now rotates into inertial space, so stars wheel instead of following terrain.
fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(443.897, 441.423, 437.195));
    q += dot(q, q.yzx + 19.19);
    return fract((q.x + q.y) * q.z);
}

fn stars(dir: vec3<f32>) -> vec3<f32> {
    let sp = dir * 220.0;
    let cell = floor(sp);
    let h = hash31(cell);
    if (h < 0.92) { return vec3<f32>(0.0); }
    let jitter = vec3<f32>(hash31(cell + 7.1), hash31(cell + 13.7), hash31(cell + 29.3));
    let d = length(fract(sp) - clamp(jitter, vec3<f32>(0.15), vec3<f32>(0.85)));
    let bright = (h - 0.92) / 0.08;
    let disc = 1.0 - smoothstep(0.0, 0.10 + 0.08 * bright, d);
    let tint = mix(vec3<f32>(0.75, 0.82, 1.0), vec3<f32>(1.0, 0.92, 0.78), hash31(cell + 3.3));
    return tint * disc * (0.25 + 1.6 * bright * bright);
}

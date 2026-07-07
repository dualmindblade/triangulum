// Phase 0 shader: camera-relative vertices, lambert sun + ambient, gamma out.

struct Globals {
    view_proj: mat4x4<f32>,
    // inverse view-projection for the sky pass (camera at the origin, so an
    // unprojected far-plane point IS the view ray)
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    // disc cut out of the heightfield where voxel chunks own the ground:
    // xyz = center relative to camera (km), w = radius in km (0 = off)
    hole: vec4<f32>,
    hole_up: vec4<f32>,
    // xyz = camera radial up, w = camera height above the sphere (km)
    sky: vec4<f32>,
    // xyz = planet center relative to the camera (km), w = voxel patch lift
    center: vec4<f32>,
    // x = number of active torch lights, y = time (s, wraps hourly)
    misc: vec4<f32>,
    // placed-torch point lights: xyz camera-relative (km), w intensity
    lights: array<vec4<f32>, 16>,
};
struct Tile {
    // xyz: tile origin minus camera position, in km (computed in f64 on the
    // CPU). w: 1 for heightfield tiles (subject to the hole cut), 0 for
    // voxel chunks.
    offset: vec4<f32>,
    // x, y = geomorph band start/end distances (km); vertices slide toward
    // the parent level's geometry across [x, y] so LOD swaps never pop.
    // Zero for voxel chunks and level-0 tiles.
    morph: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<uniform> tile: Tile;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // rgb = water color, a = wetness flag on mesh tiles / cave-darkness
    // factor on voxel chunks
    @location(3) water: vec4<f32>,
    // radial delta (km) to the parent LOD level's height here (geomorphing)
    @location(4) morph_dh: f32,
    // wetness the parent LOD level paints here (the river-thread width is
    // level-dependent, so unmorphed paint pops at every tile split)
    @location(5) morph_wet: f32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) dist_km: f32,
    // camera-relative position (xyz, km) + the tile/chunk flag (w)
    @location(3) rel_flag: vec4<f32>,
    @location(4) water: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    var rel = in.pos + tile.offset.xyz;
    let d = length(rel);
    var wet = in.water.a;
    if (tile.morph.y > 0.0) {
        // geomorphing: slide toward the parent level's geometry (and its
        // river paint) as this vertex nears the tile's merge distance — the
        // LOD swap then exchanges identical tiles and nothing pops
        let m = clamp((d - tile.morph.x) / (tile.morph.y - tile.morph.x), 0.0, 1.0);
        let radial = normalize(rel - globals.center.xyz);
        rel += radial * (in.morph_dh * m);
        wet = mix(in.water.a, in.morph_wet, m);
    }
    if (tile.offset.w < 0.5 && globals.hole.w > 0.0) {
        // voxel chunks: past the guaranteed-covered disc the blocks sink
        // from their lift down flush with the mesh, so the patch ends in a
        // feathered shoreline instead of a floating one-block cliff
        let q = rel - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = length(q - globals.hole_up.xyz * vert);
        let sink = smoothstep(globals.hole.w * 1.02, globals.hole.w * 1.35, horiz);
        rel -= globals.hole_up.xyz * (globals.center.w * 1.1 * sink);
    }
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    out.dist_km = length(rel);
    out.rel_flag = vec4<f32>(rel, tile.offset.w);
    out.water = vec4<f32>(in.water.rgb, wet);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // cut the heightfield away inside the voxel patch: every pixel belongs
    // to exactly one system (blocks own the near disc, the mesh the rest).
    // The vertical slab test keeps far-side geometry out of the cylinder.
    if (in.rel_flag.w > 0.5 && globals.hole.w > 0.0) {
        let q = in.rel_flag.xyz - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = q - globals.hole_up.xyz * vert;
        if (abs(vert) < 25.0 && length(horiz) < globals.hole.w) {
            discard;
        }
    }
    // deep tiles (near the voxel patch) resolve rivers: step the wetness
    // for crisp per-pixel water edges. Far tiles can't — a soft wet tint
    // reads as a distant river valley instead of shattered polygons.
    // (Voxel chunks reuse water.a as their cave-darkness factor instead.)
    var wet = 0.0;
    if (in.rel_flag.w > 0.5) {
        wet = in.water.a * 0.75;
        if (in.rel_flag.w > 1.5) {
            wet = step(0.55, in.water.a);
        }
    }
    let base = mix(in.color, in.water.rgb, clamp(wet, 0.0, 1.0));
    let n = normalize(in.normal);
    let light = max(dot(n, globals.sun_dir.xyz), 0.0);
    var c = base * (0.10 + 1.0 * light);
    if (in.rel_flag.w < 0.5) {
        if (in.water.a > 1.5) {
            // emissive block face (torch flames): full-bright, no sun, no
            // dim — breathing a little on a position-hashed phase
            let fl = 0.86 + 0.22 * sin(globals.misc.y * 9.0
                + dot(in.rel_flag.xyz, vec3<f32>(310.0, 470.0, 130.0)));
            return vec4<f32>(in.color * fl, 1.0);
        }
        // cave darkness, applied here (not baked) so the player's torch can
        // push it back: a warm pool of light that reaches ~10 m and only
        // acts where the dark is — daylight terrain is untouched
        let dim = in.water.a;
        let reach = clamp(1.0 - in.dist_km / 0.010, 0.0, 1.0);
        let torch = reach * reach * (1.0 - dim);
        c = c * dim + base * vec3<f32>(1.0, 0.78, 0.48) * torch * 0.85;
    }
    // placed torches: warm point-light pools on blocks and mesh alike —
    // they light the night and push back cave darkness around themselves
    let n_lights = u32(globals.misc.x);
    for (var i = 0u; i < n_lights; i = i + 1u) {
        let lp = globals.lights[i];
        let dist = distance(in.rel_flag.xyz, lp.xyz);
        let a = clamp(1.0 - dist / 0.011, 0.0, 1.0);
        c += base * vec3<f32>(1.0, 0.70, 0.36) * (a * a * lp.w);
    }
    // aerial perspective: distant terrain melts toward the same horizon
    // color the sky pass uses; the effect thins away with camera altitude
    let atm = exp(-max(globals.sky.w, 0.0) / 45.0);
    let day = smoothstep(-0.08, 0.15, dot(globals.sun_dir.xyz, globals.sky.xyz));
    let fog = (1.0 - exp(-in.dist_km * 0.0035)) * atm * 0.7;
    c = mix(c, vec3<f32>(0.55, 0.70, 0.88) * (0.15 + 0.85 * day), fog);
    // wading with your eyes under the surface: everything goes water-blue
    if (globals.hole_up.w > 0.5) {
        c = mix(c, vec3<f32>(0.02, 0.07, 0.16), 0.6);
    }
    // target format is sRGB: linear values are converted on write
    return vec4<f32>(c, 1.0);
}

// ------------------------------------------------------------------- sky

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(443.897, 441.423, 437.195));
    q += dot(q, q.yzx + 19.19);
    return fract((q.x + q.y) * q.z);
}

// procedural star field: hash the view direction into cells on a cube
// lattice; a sparse subset of cells hold one star each, rendered as a tiny
// smooth disc with hashed brightness/temperature. Purely directional, so
// the stars are fixed to the sky.
fn stars(dir: vec3<f32>) -> vec3<f32> {
    let sp = dir * 220.0;
    let cell = floor(sp);
    let h = hash31(cell);
    if (h < 0.92) {
        return vec3<f32>(0.0);
    }
    // star position jittered inside the cell; distance to it in cell units
    let jitter = vec3<f32>(hash31(cell + 7.1), hash31(cell + 13.7), hash31(cell + 29.3));
    let d = length(fract(sp) - clamp(jitter, vec3<f32>(0.15), vec3<f32>(0.85)));
    let bright = (h - 0.92) / 0.08; // 0..1, few bright ones
    let disc = smoothstep(0.10 + 0.08 * bright, 0.0, d);
    // temperature tint: cool white to warm white
    let warm = hash31(cell + 3.3);
    let tint = mix(vec3<f32>(0.75, 0.82, 1.0), vec3<f32>(1.0, 0.92, 0.78), warm);
    return tint * disc * (0.25 + 1.6 * bright * bright);
}

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    var out: SkyOut;
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    // reversed-Z: depth 0.0 is infinity, so GreaterEqual only wins on
    // untouched (cleared) pixels
    out.clip = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // unproject the NEAR plane (reversed-Z ndc z = 1): the camera sits at
    // the origin of this space, so the near-plane point IS the ray
    // direction, and the near column of the inverse VP is numerically
    // clean. (Unprojecting the far plane cancelled to ~1/far — below f32
    // rounding noise once the near plane shrank close to the ground, which
    // flipped rays and blacked out the whole sky under ~8 m altitude.)
    let p = globals.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(p.xyz / p.w);
    let up = globals.sky.xyz;
    let sun = globals.sun_dir.xyz;
    let h = dot(dir, up);
    let sunh = dot(sun, up);
    let day = smoothstep(-0.08, 0.15, sunh);
    // day gradient: bright horizon band to deeper zenith blue
    let zen = vec3<f32>(0.10, 0.28, 0.62);
    let hor = vec3<f32>(0.55, 0.70, 0.88);
    var c = mix(hor, zen, pow(clamp(h, 0.0, 1.0), 0.55)) * (0.03 + 0.97 * day);
    // a low sun warms the sky around it
    let toward = pow(max(dot(dir, sun), 0.0), 6.0);
    let low = 1.0 - smoothstep(0.05, 0.35, sunh);
    c = mix(c, vec3<f32>(0.95, 0.55, 0.30), toward * low * 0.55 * day);
    // sun disc + glow
    let d = dot(dir, sun);
    c += vec3<f32>(1.0, 0.96, 0.86)
        * (smoothstep(0.99965, 0.99996, d) * 3.0 + pow(max(d, 0.0), 800.0) * 0.5) * day;
    // below the horizon line, fade toward space-dark
    c *= smoothstep(-0.10, 0.03, h);
    // the atmosphere thins away with altitude: space is black
    let atm = exp(-max(globals.sky.w, 0.0) / 45.0);
    c *= atm;
    // limb glow: from orbit, rays that graze past the planet pass through
    // its atmosphere shell — a thin lit rim hugging the dark disc. The ray
    // is measured by its closest approach to the planet center; the glow
    // hugs the surface radius and shows only on the sunlit side.
    if (atm < 0.85) {
        let ctr = globals.center.xyz;
        let along = dot(ctr, dir);
        if (along > 0.0) {
            let r_planet = length(ctr) - max(globals.sky.w, 0.0);
            let cp = dir * along; // closest point on the ray to the center
            let b = length(cp - ctr); // miss distance from planet center
            let n = normalize(cp - ctr);
            let lit = smoothstep(-0.15, 0.35, dot(n, sun));
            let shell = exp(-max(b - r_planet, 0.0) / 20.0)
                * smoothstep(r_planet * 0.6, r_planet, b);
            c += vec3<f32>(0.30, 0.50, 0.90) * shell * lit * (1.0 - atm) * 0.8;
        }
    }
    // stars own the dark: night ground and open space alike. They dim away
    // wherever the sky itself has light, so daylight hides them.
    let sky_lum = dot(c, vec3<f32>(0.35, 0.45, 0.20));
    c += stars(dir) * (1.0 - smoothstep(0.01, 0.18, sky_lum));
    if (globals.hole_up.w > 0.5) {
        c = mix(c, vec3<f32>(0.02, 0.07, 0.16), 0.6);
    }
    return vec4<f32>(c, 1.0);
}

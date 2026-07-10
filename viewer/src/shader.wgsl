// Phase 0 shader: camera-relative vertices, lambert sun + ambient, gamma out.

struct Globals {
    view_proj: mat4x4<f32>,
    // inverse view-projection for the sky pass (camera at the origin, so an
    // unprojected far-plane point IS the view ray)
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    // xyz = unit direction to the moon; drives the night moon disc + moonlight
    moon_dir: vec4<f32>,
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
    // weather at the camera: x cloud cover, y precip, z snow fraction,
    // w air temp C (999 = weather off, disables ground responses)
    weather: vec4<f32>,
    // xyz = cloud-domain drift (synoptic advection), w = overcast sun floor
    weather2: vec4<f32>,
    // x rain darkening cap, y full-dusting cold depth (C),
    // z cloud shell altitude (km), w shell fade-out camera height (km)
    weather3: vec4<f32>,
    // xyz = instantaneous wind (km/s, camera-relative) for particle slant
    weather4: vec4<f32>,
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
    // 1.0 on a sea/lake water-surface vertex: the heightfield hole does NOT
    // cut these, so the mesh water plane stays under the voxel patch and
    // backs any perimeter crack with water instead of the (void) sky.
    @location(6) wflag: f32,
    // signed water-minus-ground delta (km): its interpolated zero crossing
    // IS the shoreline, stepped per fragment (-1 = no standing water)
    @location(7) shore: f32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) dist_km: f32,
    // camera-relative position (xyz, km) + the tile/chunk flag (w)
    @location(3) rel_flag: vec4<f32>,
    @location(4) water: vec4<f32>,
    @location(5) wflag: f32,
    @location(6) shore: f32,
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
    if (tile.offset.w < 0.5 && globals.misc.z > 0.0) {
        // voxel chunks: toward the SELECTED patch radius (misc.z) the
        // blocks sink from their lift down flush with the mesh, so the
        // patch ends in a feathered shoreline instead of a floating
        // one-block cliff. Tied to the selected radius, not the hole —
        // the hole tracks built coverage and may lag while streaming.
        let q = rel - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = length(q - globals.hole_up.xyz * vert);
        let sink = smoothstep(globals.misc.z * 0.85, globals.misc.z * 1.06, horiz);
        rel -= globals.hole_up.xyz * (globals.center.w * 1.1 * sink);
    }
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    out.dist_km = length(rel);
    out.rel_flag = vec4<f32>(rel, tile.offset.w);
    out.water = vec4<f32>(in.water.rgb, wet);
    out.wflag = in.wflag;
    out.shore = in.shore;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // cut the heightfield away inside the voxel patch: every pixel belongs
    // to exactly one system (blocks own the near disc, the mesh the rest).
    // The vertical slab test keeps far-side geometry out of the cylinder.
    // EXCEPTION: sea/lake water surfaces (wflag) are NOT cut — block water
    // and mesh water are the same surface, so cutting the mesh water opens a
    // see-through crack at the patch boundary (a black void underwater).
    // Leaving the water plane under the patch backs any crack with water;
    // the opaque blocks still draw over it, so nothing double-shows.
    // The exemption needs wflag ~ 1: it interpolates across a triangle, so
    // a MIXED shore triangle (two sea verts + one land vert) kept its
    // wflag>0.5 half alive inside the patch — a floating pale shore-ramp
    // fragment over the block water (photographed at 9.20 114.05). Only
    // all-water triangles (wflag 1 at every vertex) may skip the cut.
    if (in.rel_flag.w > 0.5 && globals.hole.w > 0.0 && in.wflag < 0.98) {
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
        // per-pixel shorelines (TRANSITIONS.md B, kills V-5): the signed
        // water-minus-ground field interpolates smoothly across triangles;
        // stepping its zero crossing per fragment with derivative AA draws
        // the sea/lake edge at pixel resolution instead of vertex
        // resolution — no more angular lake polygons or orphan blue cells.
        // Rivers keep their own wetness paint (water.a) untouched.
        let e = max(fwidth(in.shore), 1e-5);
        wet = max(wet, smoothstep(-e, e, in.shore));
    }
    // ---- weather on the ground (WEATHER.md Layer 3) ----
    // Dust and darken the GROUND color before the water mix so lakes and
    // rivers stay water. Pixels lapse-colder than the camera's air whiten
    // first (~6.5 C/km), the line broken up by a world-anchored hash so it
    // reads organic instead of contoured; dusting deepens while snow
    // actually falls. Rain soaks the ground toward dark.
    var ground = in.color;
    let up = globals.sky.xyz;
    let day = smoothstep(-0.08, 0.15, dot(globals.sun_dir.xyz, up));
    // ---- shared micro-texture (TRANSITIONS.md A, kills the V-1 disk) ----
    // The far mesh evaluates the same ±10% block-brightness fabric the
    // voxel columns bake, so the patch rim becomes a resolution change
    // instead of a material change. STATISTICS match (amplitude, ~block
    // scale); cell identity cannot — f32 world coordinates carry ~0.5 m of
    // noise at planet radius, so the blocks' exact u64 lattice is out of
    // reach: same fabric, not the same stitches. Fades over ~2 km so the
    // far field keeps today's clean flat reading; blocks skip it (their
    // hash is baked per column+height).
    if (in.rel_flag.w > 0.5) {
        let wp = in.rel_flag.xyz - globals.center.xyz;
        let cell = vec3<i32>(floor(wp * 480.0)); // ~2 m cells (km units)
        let tex = 0.9 + 0.2 * hash3i(cell);
        let tex_fade = exp(-in.dist_km / 1.8);
        ground = ground * mix(1.0, tex, tex_fade);
    }
    if (globals.weather.w < 500.0) {
        let wp = in.rel_flag.xyz - globals.center.xyz;
        let r_sphere = length(globals.center.xyz) - max(globals.sky.w, 0.0);
        let elev_km = length(wp) - r_sphere;
        let t_pix = globals.weather.w - 6.5 * (elev_km - max(globals.sky.w, 0.0));
        let dith = hash31(floor(wp * 40.0)) * 1.8 - 0.9;
        let cold = 1.0 - smoothstep(-globals.weather3.y, 1.0, t_pix + dith);
        let dust = cold * (0.45 + 0.55 * globals.weather.z * globals.weather.y);
        ground = mix(ground, vec3<f32>(0.88, 0.90, 0.94), dust);
        let rain = globals.weather.y * (1.0 - globals.weather.z);
        ground = ground * (1.0 - globals.weather3.x * rain);
    }
    let base = mix(ground, in.water.rgb, clamp(wet, 0.0, 1.0));
    let n = normalize(in.normal);
    let light = max(dot(n, globals.sun_dir.xyz), 0.0);
    let sky_hemi = clamp(0.5 + 0.5 * dot(n, up), 0.0, 1.0);
    // overcast: direct sun dims toward its tunable floor and the ambient
    // flattens a touch — a grey day, not a dark one (night is untouched:
    // the dimming scales with `day`)
    let odim = mix(1.0, globals.weather2.w, globals.weather.x * day);
    let ambient = (0.10 + 0.40 * day * sky_hemi) * mix(1.0, 0.85, globals.weather.x * day);
    // ...and the direct term itself dies with the horizon (* day): the
    // below-horizon sun otherwise kept lighting at FULL coefficient — tree
    // canopy sides facing the set sun glowed all night, opposite the moon
    // ("tree shading is backwards", night photo at 0.626 68.962)
    let sun_coeff = mix(1.0, 0.60, day) * odim * day;
    var c = base * (ambient + sun_coeff * light);
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
    // moonlight: a cool directional lift plus a faint ambient floor so night
    // terrain reads as moonlit rather than flat black. Present only at night
    // (fades as the sun rises) and only while the moon is above the horizon.
    let moon = globals.moon_dir.xyz;
    let moon_up = smoothstep(0.0, 0.15, dot(moon, globals.sky.xyz));
    let moonlit = max(dot(n, moon), 0.0);
    // sky-shaped night floor: with a flat floor and a LOW moon, vertical
    // step faces toward the moon glowed brighter than the tops around them
    // and terraces read as bright contour stripes (2026-07-08 night shots).
    // Faces that see more sky keep more of the floor, restoring the
    // tops-over-sides hierarchy while the directional term still lets a low
    // moon rake across scarps.
    let hemi_n = 0.5 + 0.5 * dot(n, globals.sky.xyz);
    c += base * vec3<f32>(0.40, 0.50, 0.72)
        * (moonlit * 0.10 + 0.015 + 0.05 * hemi_n) * (1.0 - day) * moon_up;
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

// integer-lattice hash (PCG-style mixing): float hashes built on fract()
// lose their low bits at large coordinates — the cloud domain reaches the
// thousands and hash31 there degenerates into straight diagonal shards
fn hash3i(p: vec3<i32>) -> f32 {
    var v = bitcast<vec3<u32>>(p) * vec3<u32>(1664525u, 1013904223u, 2654435769u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    return f32(v.x & 0xFFFFFFu) / 16777216.0;
}

// trilinear value noise + a 4-octave fbm — the cloud deck's fabric
fn vnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let c000 = hash3i(i);
    let c100 = hash3i(i + vec3<i32>(1, 0, 0));
    let c010 = hash3i(i + vec3<i32>(0, 1, 0));
    let c110 = hash3i(i + vec3<i32>(1, 1, 0));
    let c001 = hash3i(i + vec3<i32>(0, 0, 1));
    let c101 = hash3i(i + vec3<i32>(1, 0, 1));
    let c011 = hash3i(i + vec3<i32>(0, 1, 1));
    let c111 = hash3i(i + vec3<i32>(1, 1, 1));
    let x00 = mix(c000, c100, u.x);
    let x10 = mix(c010, c110, u.x);
    let x01 = mix(c001, c101, u.x);
    let x11 = mix(c011, c111, u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

fn cloud_fbm(p: vec3<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i = i + 1) {
        v += amp * vnoise(q);
        q = q * 2.13 + vec3<f32>(31.7, 17.3, 51.1);
        amp *= 0.5;
    }
    return v;
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
    // heavy weather greys the whole vault, not just where the deck draws:
    // the blue between clouds desaturates and sinks toward overcast murk
    let wcover = globals.weather.x;
    let murk = wcover * (0.35 + 0.45 * globals.weather.y);
    c = mix(c, vec3<f32>(0.52, 0.56, 0.62) * (0.05 + 0.95 * day), murk * day);
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
    // the moon: a phase-lit sphere opposite the sun. A ray within the moon's
    // angular radius reconstructs the sphere normal (center faces us, limb is
    // tangential) and lights it by the sun for a real terminator; the dark
    // side keeps a faint earthshine. It fades out in daylight and below the
    // horizon, and a soft halo rings it.
    let moon = globals.moon_dir.xyz;
    let moon_vis = (1.0 - 0.9 * day) * smoothstep(-0.06, 0.06, dot(moon, up));
    if (moon_vis > 0.001) {
        let dm = dot(dir, moon);
        let ang = acos(clamp(dm, -1.0, 1.0));
        let R = 0.021; // angular radius (rad, ~1.2 deg — reads as a moon)
        if (ang < R) {
            let off = normalize(dir - moon * dm);
            let t = ang / R; // 0 at center, 1 at the limb
            let nrm = normalize(moon * cos(t * 1.5707963) + off * sin(t * 1.5707963));
            let lit = max(dot(nrm, sun), 0.0);
            // faint surface mottling (maria) so it isn't a flat disc
            let mott = 0.86 + 0.14 * hash31(floor(off * 70.0));
            let disc = smoothstep(1.0, 0.90, t);
            c += vec3<f32>(0.86, 0.89, 0.97) * mott * disc
                * (0.05 + 0.95 * lit) * moon_vis;
        }
        // tight halo hugging the disc (not a broad blob)
        c += vec3<f32>(0.55, 0.62, 0.80) * pow(max(dm, 0.0), 2600.0) * 0.30 * moon_vis;
    }
    // ---- W1 cloud deck (WEATHER.md): a wind-scrolled noise shell a few
    // km up. Drawn after the sun/moon (clouds cover them) and before the
    // stars (which dim behind lit cloud via sky_lum). Fades out entirely
    // by weather3.w camera altitude: W1 renders NO clouds from space — the
    // capped-opacity orbital layer is W3's job, so the "never obscure the
    // ground from orbit" constraint holds trivially for now.
    let cover = globals.weather.x;
    let deck_fade =
        1.0 - smoothstep(globals.weather3.z, globals.weather3.w, max(globals.sky.w, 0.0));
    if (cover > 0.004 && deck_fade > 0.003) {
        let ctr = globals.center.xyz;
        let r_shell = length(ctr) - max(globals.sky.w, 0.0) + globals.weather3.z;
        let b2 = dot(ctr, dir);
        let qd = dot(ctr, ctr) - r_shell * r_shell; // < 0 while under the deck
        let disc2 = b2 * b2 - qd;
        if (disc2 > 0.0 && qd < 0.0) {
            let t_hit = b2 + sqrt(disc2);
            let sdir = normalize(dir * t_hit - ctr); // deck point on its sphere
            // the deck scrolls with the same synoptic drift the weather
            // field advects by — the storm you see IS the storm you get
            let den = cloud_fbm((sdir - globals.weather2.xyz) * 900.0);
            let a_thr = 0.68 - 0.40 * cover;
            var alpha = smoothstep(a_thr, a_thr + 0.18, den);
            // grazing rays pack more deck toward the horizon, then the
            // deck fades below it and thins away to space with the air
            alpha = min(alpha * (1.0 + 0.7 * (1.0 - clamp(h, 0.0, 1.0))), 1.0);
            alpha = alpha * smoothstep(-0.02, 0.08, h) * deck_fade * atm;
            // bright lit tops; heavy rain bellies go slate; a low sun warms
            // the deck the way it warms the sky
            let heavy = clamp(globals.weather.y * 0.9 + cover * 0.45, 0.0, 1.0);
            var ccol = mix(
                vec3<f32>(0.96, 0.97, 1.00),
                vec3<f32>(0.36, 0.39, 0.46),
                clamp(heavy * (0.35 + 0.85 * den), 0.0, 1.0),
            );
            ccol = ccol * (0.10 + 0.95 * day);
            ccol = mix(ccol, vec3<f32>(0.98, 0.62, 0.38), toward * low * 0.4 * day * (1.0 - heavy));
            c = mix(c, ccol, alpha);
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

// --------------------------------------------------------- precipitation

struct PrecipOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) kind: f32, // 0 = rain streak, 1 = flake
    @location(2) alpha: f32,
};

// integer hash for particle identity — stable at any instance count
fn hash1u(n: u32) -> f32 {
    var x = n * 1664525u + 1013904223u;
    x ^= x >> 16u;
    x *= 2246822519u;
    x ^= x >> 13u;
    return f32(x & 0xFFFFFFu) / 16777216.0;
}

// Instanced rain/snow around the camera: identity is hashed from the
// instance index, motion is a pure function of the (wrapping) frame clock,
// so captures reproduce exactly. Rain falls as wind-slanted streaks; snow
// drifts down as swaying flakes. The volume rides with the camera (W1;
// world-anchoring at f32 precision on a 6371 km sphere jitters worse than
// the ride-along reads — revisit in W2 with camera-local anchoring).
@vertex
fn vs_precip(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PrecipOut {
    var out: PrecipOut;
    let up = globals.sky.xyz;
    let t = globals.misc.y;
    let h0 = hash1u(ii * 3u + 0u);
    let h1 = hash1u(ii * 3u + 1u);
    let h2 = hash1u(ii * 3u + 2u);
    let snow = select(0.0, 1.0, h2 < globals.weather.z);
    let box_r = 0.045; // km around the camera
    let box_h = 0.038;
    var e1 = cross(up, vec3<f32>(0.0, 0.0, 1.0));
    if (dot(e1, e1) < 0.01) {
        e1 = cross(up, vec3<f32>(0.0, 1.0, 0.0));
    }
    e1 = normalize(e1);
    let e2 = normalize(cross(up, e1));
    let speed = mix(0.0095, 0.0016, snow); // km/s straight-down component
    let phase = fract(h1 - t * speed / box_h);
    let sway = (vnoise(vec3<f32>(t * 0.6 + h0 * 23.0, h1 * 17.0, h2 * 11.0)) - 0.5)
        * mix(0.0004, 0.0035, snow);
    let center = e1 * ((h0 - 0.5) * 2.0 * box_r + sway)
        + e2 * ((fract(h2 * 7.31) - 0.5) * 2.0 * box_r)
        + up * ((phase - 0.5) * 2.0 * box_h);
    // fall direction bends with the wind; streaks align to it
    let fall = normalize(-up * speed + globals.weather4.xyz * 0.4);
    let to_p = center / max(length(center), 0.0015);
    // a particle straight below a down-looking camera has to_p parallel to
    // fall: cross() collapses and normalize() exploded the quad into a
    // ~100 m garbage slab (photographed at 9.20 114.05 in heavy snowfall,
    // pitch -82). Fall back to a horizontal frame axis when degenerate.
    var cr = cross(to_p, fall);
    if (dot(cr, cr) < 1e-6) {
        cr = cross(fall, e1);
    }
    let across = normalize(cr);
    let half_len = mix(0.00040, 0.00006, snow);
    let half_wid = mix(0.000040, 0.00006, snow);
    // vi 0..5 -> two triangles of a quad
    var cid = vi;
    if (vi == 3u) { cid = 2u; }
    if (vi == 4u) { cid = 1u; }
    if (vi == 5u) { cid = 3u; }
    let cx = f32(cid & 1u) * 2.0 - 1.0;
    let cy = f32(cid >> 1u) * 2.0 - 1.0;
    let pos = center + across * (cx * half_wid) + fall * (cy * half_len);
    out.clip = globals.view_proj * vec4<f32>(pos, 1.0);
    out.uv = vec2<f32>(cx, cy);
    out.kind = snow;
    let d = length(center);
    let fade = smoothstep(0.0018, 0.0060, d) * (1.0 - smoothstep(box_r * 0.7, box_r, d));
    out.alpha = fade * mix(0.16, 0.85, snow);
    return out;
}

@fragment
fn fs_precip(in: PrecipOut) -> @location(0) vec4<f32> {
    var a = in.alpha;
    if (in.kind > 0.5) {
        // soft round flake
        a *= 1.0 - smoothstep(0.45, 1.0, length(in.uv));
    } else {
        // streak with soft ends and edges
        a *= (1.0 - smoothstep(0.55, 1.0, abs(in.uv.y))) * (1.0 - abs(in.uv.x) * 0.45);
    }
    let day = smoothstep(-0.08, 0.15, dot(globals.sun_dir.xyz, globals.sky.xyz));
    let col = mix(vec3<f32>(0.72, 0.78, 0.88), vec3<f32>(0.97, 0.98, 1.00), in.kind)
        * (0.30 + 0.70 * day);
    return vec4<f32>(col, a);
}

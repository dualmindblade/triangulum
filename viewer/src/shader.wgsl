// Phase 0 shader: camera-relative vertices, lambert sun + ambient, gamma out.

struct Globals {
    view_proj: mat4x4<f32>,
    // inverse view-projection for the sky pass (camera at the origin, so an
    // unprojected far-plane point IS the view ray)
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    // xyz = unit direction to the moon; drives the night moon disc + moonlight
    moon_dir: vec4<f32>,
    // Physical centers relative to the camera, w = radius (km). CPU f64
    // subtraction happens before upload; these are never raw world positions.
    sun_body: vec4<f32>,
    moon_body: vec4<f32>,
    // x solar occlusion at camera, y lunar shadow, z planet contact gate,
    // w sun halo gain
    eclipse: vec4<f32>,
    sun_tint: vec4<f32>,
    moon_tint: vec4<f32>,
    moon_copper_tint: vec4<f32>,
    // disc cut out of the heightfield where voxel chunks own the ground:
    // xyz = center relative to camera (km), w = radius in km (0 = off)
    hole: vec4<f32>,
    hole_up: vec4<f32>,
    // xyz = camera radial up, w = camera height above the sphere (km)
    sky: vec4<f32>,
    // xyz = planet center relative to the camera (km), w = voxel patch lift
    center: vec4<f32>,
    // xyz = focused body center relative to camera; w = numeric body id.
    body_frame: vec4<f32>,
    // x = number of active torch lights, y = time (s, wraps hourly)
    misc: vec4<f32>,
    // weather at the camera: x cloud cover, y precip, z snow fraction,
    // w air temp C (999 = weather off, disables ground responses)
    weather: vec4<f32>,
    // xyz = cloud-domain drift (synoptic advection), w = overcast sun floor
    weather2: vec4<f32>,
    // x rain darkening cap, y full-dusting cold depth (C),
    // z low cloud-shell altitude (km), w ground/orbit handoff height (km)
    weather3: vec4<f32>,
    // xyz = instantaneous wind (km/s, camera-relative) for particle slant,
    // w = absolute weather time (s, wraps hourly) for particle motion
    weather4: vec4<f32>,
    // x/y = middle/high cloud-shell altitudes (km), z = active layer count,
    // w = hard orbital opacity cap
    weather5: vec4<f32>,
    // xyz = low/mid/high cloud noise scales, w = rain crevice-bias strength
    weather6: vec4<f32>,
    // xyz = low/mid/high cloud density multipliers; w marks a pinned deck
    // (the uniform raster still exists, while exact scalar pins preserve
    // pre-W2.5 capture bytes)
    weather7: vec4<f32>,
    // cloud DECK scalar compatibility inputs: xy retain exact pins, z is the
    // camera-to-planet-mean temperature handoff, w is Neisor camera altitude.
    // Live cover/precip SHAPE comes from the synoptic raster below; ground
    // responses keep reading `weather`, which stays the camera sample.
    weather8: vec4<f32>,
    // W2: x ground cloud-shadow strength, y orbital cloud-alpha darkening,
    // z fog density, w fog ceiling above the camera-ground reference (km)
    weather9: vec4<f32>,
    // W2 fog state: x dawn window, y humidity, z camera concavity,
    // w broad saturated cover+precip bank
    weather10: vec4<f32>,
    // W2 storm-edge first-order field: xyz tangent gradient, w base load
    weather11: vec4<f32>,
    // W2 storm art direction: x gloom strength, y warm/green cast;
    // zw = inverse viewport size for a terrain-independent orbital ray
    weather12: vec4<f32>,
    // W-MOTION pass 2: x/y accumulated base/shear differential rotation,
    // x accumulated rigid base rotation, y bounded zonal shear phase,
    // z cyclone angular radius, w active bounded system count.
    weather13: vec4<f32>,
    // x unused (core wrap rides in cyclone_fronts.w), y/z cover/precip boosts, w front
    // strength.
    weather14: vec4<f32>,
    // xy comma-tail front leading/trailing cross widths and z its outer
    // radial extent, in cyclone-radius units; w spiral-arm strength.
    weather15: vec4<f32>,
    // Spiral arms: x arm count, y log-spiral twist, z/w unused.
    weather16: vec4<f32>,
    // Planet-frame center.xyz + lifecycle-scaled intensity; fronts carry
    // only the hemisphere-signed bounded core wrap angle in w.
    cyclone_centers: array<vec4<f32>, 4>,
    cyclone_fronts: array<vec4<f32>, 4>,
    // x hemisphere-signed rotating arm-pattern phase (radians), y the
    // comma-tail front's base azimuth in the storm's polar frame.
    cyclone_arms: array<vec4<f32>, 4>,
    // premultiplied cave-noise seeds (low 32 bits of
    // (seed+K).wrapping_mul(0x9E37_79B1)) for the karst breach hint:
    // x = region gate (+40961), y = tube n1 (+31337), z = tube n2 (+51413),
    // w = independent clouds-v2 layout seed (+70001); the range-biome
    // comparator's octave-zero seedmul rides danchor_cell.w instead
    karst: vec4<u32>,
    // dusting-dither anchor (see renderer.rs Globals): xyz = the camera's
    // dither-lattice corner relative to the camera (km, <= 25 m), and the
    // matching exact lattice cell - together they give the dither noise a
    // camera-precise, world-stable domain (raw planet-centered f32
    // positions quantize at ~0.24 m and rendered as crawling plateaus)
    danchor: vec4<f32>,
    danchor_cell: vec4<i32>,
    // placed-torch point lights: xyz camera-relative (km), w intensity
    lights: array<vec4<f32>, 16>,
};
struct Tile {
    // xyz: tile origin minus camera position, in km (computed in f64 on the
    // CPU). w: 1 for heightfield tiles (subject to the hole cut), 0 for
    // voxel chunks.
    offset: vec4<f32>,
    // x, y = geomorph band start/end distances (km); vertices slide toward
    // the parent triangle across [x, y] to retire the visible LOD pop.
    // Zero for voxel chunks and level-0 tiles.
    morph: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<uniform> tile: Tile;
@group(0) @binding(2) var synoptic_raster: texture_2d_array<f32>;
@group(0) @binding(3) var synoptic_sampler: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // rgb = water color, a = wetness flag on mesh tiles / cave-darkness
    // factor on voxel chunks
    @location(3) water: vec4<f32>,
    // x = parent height delta, y = parent wetness, z = standing-water flag,
    // w = signed shoreline field. These four contiguous f32 struct members
    // share one attribute so the compact categorical payload stays within
    // the guaranteed vertex-attribute count.
    @location(4) morph: vec4<f32>,
    // Fixed Koppen-family endpoints. RGB is linear UNORM8; alpha is the
    // cumulative class-coverage threshold. Descending thresholds disable the
    // payload on voxels, water, and impostors.
    @location(5) biome0: vec4<f32>,
    @location(6) biome1: vec4<f32>,
    @location(7) biome2: vec4<f32>,
    @location(8) biome3: vec4<f32>,
    @location(9) biome4: vec4<f32>,
    @location(10) biome5: vec4<f32>,
    @location(11) biome6: vec4<f32>,
    @location(12) biome7: vec4<f32>,
    // x = coverage, y = roughness-adapted fine gain, z = valid marker,
    // w = D-8's signed rain-concavity proxy in UNORM (c/2 + 0.5)
    @location(13) beach: vec4<f32>,
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
    @location(7) biome0: vec4<f32>,
    @location(8) biome1: vec4<f32>,
    @location(9) biome2: vec4<f32>,
    @location(10) biome3: vec4<f32>,
    @location(11) biome4: vec4<f32>,
    @location(12) biome5: vec4<f32>,
    @location(13) biome6: vec4<f32>,
    @location(14) biome7: vec4<f32>,
    @location(15) beach: vec4<f32>,
};

// Keep the approved local 300 m ecotone through near flight, then hand off
// continuously to the wide multiscale boundary before the 15 km review range.
const BIOME_RANGE_BLEND_START_KM = 4.0;
const BIOME_RANGE_BLEND_END_KM = 8.0;
// L-1(a): keep the categorical edge one pixel wide, independent of the
// comparator's local slope or orientation. This is the TOTAL smoothstep band;
// half is placed on either side of the zero-margin contour.
const BIOME_BORDER_AA_BAND_PX = 1.0;
// Retire only comparator octaves whose gradient lattice is crossing Nyquist.
// The shoulder makes a moving camera shed an octave continuously; the class
// decision remains hard (apart from the one-pixel coverage band) at the last
// prefix a pixel can represent. Units are noise-lattice cells per screen px.
const BIOME_BORDER_OCTAVE_FULL_CELLS_PER_PX = 0.25;
const BIOME_BORDER_OCTAVE_CUTOFF_CELLS_PER_PX = 0.50;
// L-1(b): range endpoints are representative zero-slope palette colors, while
// arrival uses the approved exact local palette/shading. Retain only the
// measured arrival-equivalent share of their deviation around the conserved
// range mean. One-family interiors have zero deviation and remain unchanged.
const BIOME_RANGE_CATEGORICAL_CONTRAST = 0.45;
// Keep the orbital endpoint separate for Andrew even though it currently
// matches the medium/arrival dial. The 120..240 km handoff still retires the
// nested beach carrier below; equal contrast endpoints prevent the old
// medium-range exaggeration from returning or forming a second altitude ramp.
const BIOME_ORBIT_CONTRAST_FADE_START_KM = 120.0;
const BIOME_ORBIT_CONTRAST_FADE_END_KM = 240.0;
const BIOME_ORBIT_CATEGORICAL_CONTRAST = 0.45;

// ---- Track C landscape color knobs ----------------------------------------
// Strata are a modulation of the approved material color, never a replacement.
// Thickness is a scale so art direction can move the complete bed sequence
// without changing its 9 m physical reference. Tilt is the vertical fold
// displacement in metres; its noise wavelength is about 175 km on Neisor.
const STRATA_BAND_STRENGTH: f32 = 0.28;
const STRATA_BAND_THICKNESS_SCALE: f32 = 1.0;
const STRATA_TILT_AMPLITUDE_M: f32 = 260.0;
const STRATA_REFERENCE_THICKNESS_M: f32 = 9.0;
const STRATA_FOLD_FREQUENCY: f32 = 96.0;
// (85021 - karst.w's 70001) * 0x9E37_79B1, modulo 2^32. Adding this to
// karst.w preserves the established premultiplied-seed pattern while giving
// the fold and bed palette an independent deterministic stream.
const STRATA_SEED_DELTA: u32 = 0xDED7DCECu;
// L-1(c) ground patchiness (Andrew dials). Strength is the peak fractional
// luminance swing; the three octaves land at ~1.4 km / ~350 m / ~88 m on the
// surface, chosen to carry exactly the medium-altitude band where flat-color
// terrain had no texture octave at all (finer texture belongs to voxels and
// features, coarser to the biome fields themselves).
const PATCH_STRENGTH: f32 = 0.11;
const PATCH_FREQ_BASE: f32 = 4500.0;
// (91009 - karst.w's 70001) * 0x9E37_79B1 mod 2^32: an independent
// deterministic stream in the established premultiplied-seed pattern.
const PATCH_SEED_DELTA: u32 = 0xA8724D10u;
const STRATA_PALETTE = array<vec3<f32>, 5>(
    vec3<f32>(0.72, 0.80, 0.88), // cool shale
    vec3<f32>(1.18, 1.09, 0.91), // pale ochre
    vec3<f32>(0.82, 0.77, 0.72), // umber
    vec3<f32>(1.10, 0.98, 0.82), // warm sandstone
    vec3<f32>(0.91, 0.97, 1.05), // pale mineral bed
);

// The existing true-ocean depth ramp carries a sea-only 0..20 m teal
// residual in RGB. The fragment has no scalar depth/class payload, so the
// helper below recovers that carrier without touching the vertex contract.
// A larger falloff scale holds the shallow tint farther through that carrier.
const BATHYMETRY_SHALLOW_TINT: vec3<f32> = vec3<f32>(0.035, 0.42, 0.36);
const BATHYMETRY_DEPTH_FALLOFF_SCALE: f32 = 1.0;
const BATHYMETRY_SHALLOW_MIX: f32 = 0.55;

// Karst twin of noise.rs GRAD (generated from noise_grad.rs - the
// planetgen parity table; do not hand-edit).
const KGRAD = array<vec3<f32>, 256>(
    vec3<f32>(-0.276032466, -0.241130319, 0.93040972),
    vec3<f32>(0.991480371, 0.04028209, 0.123871007),
    vec3<f32>(-0.743505967, -0.585643918, -0.322831347),
    vec3<f32>(0.392402975, 0.286014546, -0.874194249),
    vec3<f32>(0.80440911, 0.455421838, -0.381466818),
    vec3<f32>(0.357206137, -0.492564879, 0.793589073),
    vec3<f32>(-0.0841678571, -0.766706561, 0.636456456),
    vec3<f32>(-0.892438004, -0.00305680084, 0.451159689),
    vec3<f32>(-0.661071603, -0.733827554, -0.156465513),
    vec3<f32>(0.56451794, 0.0716952637, 0.822301213),
    vec3<f32>(-0.561246956, -0.716753187, 0.413843839),
    vec3<f32>(-0.467297399, -0.259865656, -0.845046142),
    vec3<f32>(0.900654415, -0.359400011, 0.244240161),
    vec3<f32>(-0.334292427, 0.885581167, 0.322481891),
    vec3<f32>(-0.919026082, -0.196467252, -0.341747976),
    vec3<f32>(-0.247348586, 0.955824385, 0.158803092),
    vec3<f32>(0.834210297, -0.32992149, -0.441865353),
    vec3<f32>(-0.962683697, 0.212652103, -0.167389316),
    vec3<f32>(0.92626537, -0.229556355, -0.29889186),
    vec3<f32>(0.879159774, -0.222478711, 0.421403979),
    vec3<f32>(-0.0948048811, 0.446197852, 0.889898596),
    vec3<f32>(0.413218212, -0.91021099, 0.0276886958),
    vec3<f32>(-0.191944698, -0.718217561, 0.66882043),
    vec3<f32>(0.527572594, -0.782164072, -0.331491362),
    vec3<f32>(-0.158135314, 0.0870774729, -0.983570402),
    vec3<f32>(0.926047755, -0.358307248, -0.118538904),
    vec3<f32>(0.844739235, 0.416316312, 0.336298014),
    vec3<f32>(-0.0178833604, 0.723249941, 0.69035477),
    vec3<f32>(-0.522158047, 0.429313923, -0.736912837),
    vec3<f32>(0.0434986213, -0.447932188, -0.893008749),
    vec3<f32>(-0.463207253, 0.879862867, -0.106209117),
    vec3<f32>(0.646764584, 0.392698928, 0.653821936),
    vec3<f32>(0.224490004, -0.415077172, -0.881654796),
    vec3<f32>(-0.962134816, -0.198129669, 0.187193031),
    vec3<f32>(0.930964332, -0.355814074, 0.081864268),
    vec3<f32>(-0.961892279, 0.0853028185, 0.259781973),
    vec3<f32>(0.811261317, -0.407150137, 0.419623453),
    vec3<f32>(0.703017124, -0.63026949, -0.32943481),
    vec3<f32>(0.386658151, -0.339026756, 0.857645809),
    vec3<f32>(0.432356706, 0.871866684, -0.230035137),
    vec3<f32>(0.850235093, -0.0288870647, 0.525609955),
    vec3<f32>(-0.531380491, 0.24650952, -0.810473831),
    vec3<f32>(-0.0347171227, -0.744163163, 0.667095127),
    vec3<f32>(-0.912043835, 0.409322006, -0.0251304338),
    vec3<f32>(-0.691338759, -0.555773466, 0.461699659),
    vec3<f32>(-0.980809233, -0.180984577, 0.0725109038),
    vec3<f32>(-0.90234931, -0.430246921, 0.0255599378),
    vec3<f32>(0.052947524, 0.952511984, -0.299862435),
    vec3<f32>(-0.173202039, 0.953852632, -0.245288016),
    vec3<f32>(-0.497829308, 0.864162178, 0.0734146402),
    vec3<f32>(0.916452405, -0.113076699, 0.383834143),
    vec3<f32>(0.431640852, 0.865067449, 0.255625671),
    vec3<f32>(-0.677675807, 0.699383382, -0.227196797),
    vec3<f32>(0.000282057278, 0.404560826, 0.914511049),
    vec3<f32>(-0.0460834508, -0.9987779, -0.0178611759),
    vec3<f32>(-0.928116314, -0.318852143, -0.192180692),
    vec3<f32>(-0.0617358404, -0.971678354, -0.228100551),
    vec3<f32>(0.246749832, 0.797304255, -0.550836133),
    vec3<f32>(0.214014395, -0.150437887, -0.965176813),
    vec3<f32>(0.279872651, -0.0757977559, 0.957040229),
    vec3<f32>(0.98400028, -0.0494065353, 0.171179563),
    vec3<f32>(0.0305211621, -0.790295299, 0.611965521),
    vec3<f32>(0.744553932, 0.0758574625, 0.663238334),
    vec3<f32>(0.892359215, 0.141799109, 0.428471753),
    vec3<f32>(-0.0981353238, -0.623043691, 0.776006455),
    vec3<f32>(-0.917712563, 0.113344754, -0.380731687),
    vec3<f32>(0.0369537423, 0.835935546, 0.547582125),
    vec3<f32>(-0.304622294, -0.0887924602, -0.948325449),
    vec3<f32>(0.711129038, 0.700984175, 0.0540062776),
    vec3<f32>(0.63742031, -0.683235686, 0.356208289),
    vec3<f32>(-0.981787691, 0.186412004, -0.0366537146),
    vec3<f32>(-0.273268774, 0.889222372, -0.366889289),
    vec3<f32>(0.142881906, 0.123286768, -0.982031127),
    vec3<f32>(0.137983549, 0.435042061, 0.88977466),
    vec3<f32>(0.939563426, -0.342368658, 0.00206644081),
    vec3<f32>(0.181358537, 0.850869678, 0.493082013),
    vec3<f32>(0.0893046888, -0.995881789, -0.0156248435),
    vec3<f32>(-0.0088380908, -0.98859122, -0.15036385),
    vec3<f32>(0.337899178, -0.460840491, 0.820640108),
    vec3<f32>(-0.0132037926, 0.283430951, 0.958901745),
    vec3<f32>(-0.216379523, 0.265055479, 0.939641152),
    vec3<f32>(0.973343725, -0.200225914, -0.111855159),
    vec3<f32>(0.49413138, 0.46619012, -0.733826241),
    vec3<f32>(0.0220358097, 0.211775185, -0.977069953),
    vec3<f32>(0.16688953, 0.179411352, 0.96951506),
    vec3<f32>(0.186365445, -0.223437212, -0.956735979),
    vec3<f32>(0.670117255, 0.359557738, 0.64935437),
    vec3<f32>(-0.696773289, 0.496068882, -0.518095211),
    vec3<f32>(0.821370067, -0.394040665, 0.412411405),
    vec3<f32>(-0.227416005, 0.528559262, 0.81786739),
    vec3<f32>(-0.0544659821, 0.262332601, -0.963439185),
    vec3<f32>(-0.599383849, 0.694259341, -0.398425614),
    vec3<f32>(-0.197928924, 0.211071236, -0.957221539),
    vec3<f32>(-0.413940726, 0.892985024, 0.176722443),
    vec3<f32>(-0.171905371, 0.290684031, 0.94124988),
    vec3<f32>(0.787023181, 0.603850762, 0.126328023),
    vec3<f32>(0.816089442, -0.493482163, 0.300787928),
    vec3<f32>(0.36208818, -0.866695775, 0.343118905),
    vec3<f32>(-0.7301804, -0.389383525, -0.561441941),
    vec3<f32>(0.771671336, -0.577609247, -0.266253464),
    vec3<f32>(-0.405038302, -0.876353853, -0.260668175),
    vec3<f32>(0.211696974, -0.440611001, -0.872379698),
    vec3<f32>(0.140663869, 0.447924442, -0.882936787),
    vec3<f32>(0.590045122, 0.12112019, 0.798233458),
    vec3<f32>(0.0940180276, 0.692486036, -0.715278757),
    vec3<f32>(-0.0296743626, 0.997852276, 0.0583974968),
    vec3<f32>(0.764896974, 0.636412263, 0.0995592774),
    vec3<f32>(-0.778775318, 0.532720543, 0.331236816),
    vec3<f32>(-0.33581108, 0.525011498, 0.782044657),
    vec3<f32>(0.404411734, 0.702125233, -0.586064252),
    vec3<f32>(0.188152016, 0.302334981, -0.934447633),
    vec3<f32>(0.0315604529, 0.593961092, 0.803874467),
    vec3<f32>(-0.691569822, 0.710371983, -0.130777777),
    vec3<f32>(0.293605458, -0.538721226, 0.789667826),
    vec3<f32>(-0.809976437, 0.408424932, 0.420864878),
    vec3<f32>(0.640349956, 0.759475054, -0.1146716),
    vec3<f32>(-0.0927443115, -0.762660485, 0.640115206),
    vec3<f32>(0.930610355, -0.266706119, -0.250663547),
    vec3<f32>(0.291364254, -0.949559689, -0.115945107),
    vec3<f32>(-0.750244828, 0.362027305, 0.553234967),
    vec3<f32>(0.243608163, 0.959128138, 0.143973183),
    vec3<f32>(0.0551593311, 0.00556649844, 0.998462048),
    vec3<f32>(-0.686078867, -0.134107112, -0.715060186),
    vec3<f32>(-0.456262394, 0.161381437, -0.875088944),
    vec3<f32>(0.547235689, -0.325181154, 0.771226502),
    vec3<f32>(0.560259019, 0.684030459, -0.467131847),
    vec3<f32>(-0.507229031, -0.80475747, -0.308357139),
    vec3<f32>(-0.405511944, 0.683705436, 0.606718171),
    vec3<f32>(0.849951023, 0.0842158564, 0.520087443),
    vec3<f32>(-0.962719717, 0.0434632105, 0.26698632),
    vec3<f32>(0.6891646, 0.0302225643, 0.723974275),
    vec3<f32>(-0.493805958, 0.477678019, -0.7266219),
    vec3<f32>(-0.352194522, -0.619114589, -0.701894682),
    vec3<f32>(-0.842369793, -0.371691813, 0.390202931),
    vec3<f32>(-0.133158175, 0.889284051, -0.437541744),
    vec3<f32>(-0.296630401, 0.209082782, 0.931823371),
    vec3<f32>(0.985119923, 0.170787567, 0.0192443331),
    vec3<f32>(0.447586202, -0.0614819557, 0.892124745),
    vec3<f32>(-0.386723643, 0.861379876, -0.32934713),
    vec3<f32>(0.297881548, 0.0826213155, 0.951020663),
    vec3<f32>(-0.522436196, -0.343142958, -0.780585249),
    vec3<f32>(0.330618443, 0.365252239, -0.870219654),
    vec3<f32>(-0.0621210476, -0.945362758, 0.320047234),
    vec3<f32>(-0.567057813, 0.609271189, -0.554286979),
    vec3<f32>(0.798538414, 0.329296786, 0.503884935),
    vec3<f32>(0.351783621, 0.715478403, 0.603604952),
    vec3<f32>(-0.673757458, -0.0360521628, -0.738072577),
    vec3<f32>(-0.182456273, -0.255421717, -0.949457453),
    vec3<f32>(-0.432594955, 0.573826397, 0.695402669),
    vec3<f32>(-0.663323759, -0.485617291, -0.569365821),
    vec3<f32>(-0.397947229, -0.431567606, 0.809560006),
    vec3<f32>(-0.582849286, 0.47062894, 0.662416116),
    vec3<f32>(0.173384965, 0.01214674, 0.984779219),
    vec3<f32>(0.0893690977, 0.160169028, -0.983035628),
    vec3<f32>(0.790482157, 0.424137682, -0.441865575),
    vec3<f32>(0.324454931, -0.292128944, -0.899660868),
    vec3<f32>(0.328304546, -0.25622218, 0.909156928),
    vec3<f32>(0.541503283, 0.0976770899, -0.835005018),
    vec3<f32>(-0.0493433009, -0.997933, -0.041169959),
    vec3<f32>(-0.764958918, -0.392519636, -0.51065271),
    vec3<f32>(-0.577839993, -0.0337919581, 0.815450211),
    vec3<f32>(-0.629590435, -0.759555102, 0.163376655),
    vec3<f32>(-0.961263276, -0.135227293, -0.240180127),
    vec3<f32>(0.563623227, -0.0776193005, 0.822377105),
    vec3<f32>(0.0414185461, 0.676050253, 0.735690532),
    vec3<f32>(0.219209636, 0.974905676, 0.038807963),
    vec3<f32>(-0.809674613, 0.586632351, -0.0170148641),
    vec3<f32>(-0.495498842, 0.75636396, -0.427076641),
    vec3<f32>(0.769278384, 0.524217469, -0.365248974),
    vec3<f32>(0.425023643, -0.255286349, 0.868437553),
    vec3<f32>(-0.794295871, -0.141637985, 0.590789938),
    vec3<f32>(-0.325914011, -0.356948928, 0.875424194),
    vec3<f32>(-0.715950975, -0.169736882, -0.67720277),
    vec3<f32>(0.593339899, -0.528769479, -0.606918943),
    vec3<f32>(-0.896487884, 0.211339549, 0.389416317),
    vec3<f32>(-0.948451912, 0.276360387, -0.155125454),
    vec3<f32>(0.493269829, 0.445979208, 0.746851673),
    vec3<f32>(0.859039836, -0.491263903, -0.143910864),
    vec3<f32>(-0.729847935, 0.540908854, -0.418018663),
    vec3<f32>(0.209031498, -0.181834534, 0.960854846),
    vec3<f32>(0.117162773, -0.634744769, -0.763787904),
    vec3<f32>(-0.298839711, 0.947543321, -0.113386425),
    vec3<f32>(-0.790810525, 0.227376373, 0.568259358),
    vec3<f32>(-0.583718383, 0.811946709, 0.00392313892),
    vec3<f32>(0.51160152, 0.808500417, 0.290845252),
    vec3<f32>(-0.24149277, 0.481271425, 0.842650021),
    vec3<f32>(-0.279860569, 0.952684487, 0.118618419),
    vec3<f32>(0.200600462, -0.775842578, 0.598187051),
    vec3<f32>(0.108970218, 0.983957954, -0.141252387),
    vec3<f32>(-0.361610633, -0.880562269, 0.306345948),
    vec3<f32>(-0.802219789, 0.572256665, -0.170193184),
    vec3<f32>(-0.105614569, -0.600130489, 0.792899085),
    vec3<f32>(0.795024365, 0.601026642, 0.0818732856),
    vec3<f32>(-0.786899314, -0.427478128, 0.445030246),
    vec3<f32>(0.42508356, 0.159933061, -0.890912669),
    vec3<f32>(0.270044675, -0.561459346, -0.782201558),
    vec3<f32>(-0.587715155, -0.595400123, 0.547804336),
    vec3<f32>(0.257361019, 0.461375065, -0.849057333),
    vec3<f32>(-0.375537759, 0.407463184, 0.832433268),
    vec3<f32>(-0.531339941, 0.845929055, 0.0456278467),
    vec3<f32>(-0.545671705, -0.682830069, -0.485783375),
    vec3<f32>(0.367475068, -0.278713962, 0.887288342),
    vec3<f32>(-0.91624388, -0.257187307, -0.30716745),
    vec3<f32>(0.441561174, 0.583170464, -0.681862112),
    vec3<f32>(-0.34009252, 0.528415108, -0.777891092),
    vec3<f32>(-0.979890557, 0.191105196, 0.0573872876),
    vec3<f32>(-0.625995602, 0.31709819, -0.712445257),
    vec3<f32>(0.21450976, -0.0526686213, -0.975300764),
    vec3<f32>(0.745060725, 0.478980638, 0.464178914),
    vec3<f32>(0.0244852159, 0.519895207, 0.85387906),
    vec3<f32>(0.699670919, 0.594563189, 0.396175743),
    vec3<f32>(-0.852537704, -0.250582717, 0.458680461),
    vec3<f32>(0.134106828, 0.89500603, -0.425416932),
    vec3<f32>(0.970360783, -0.166381146, 0.175263415),
    vec3<f32>(-0.969198568, -0.233352219, -0.0787456571),
    vec3<f32>(-0.51404617, -0.515671057, 0.685448682),
    vec3<f32>(-0.507233736, 0.137257116, 0.850808099),
    vec3<f32>(0.159874524, -0.422126176, -0.892328206),
    vec3<f32>(-0.672674459, 0.708940099, 0.211926892),
    vec3<f32>(0.63991845, -0.746945539, -0.180490273),
    vec3<f32>(0.347880643, -0.829322541, -0.437267859),
    vec3<f32>(-0.901118332, 0.351059644, -0.254446219),
    vec3<f32>(-0.597016042, -0.0146101101, -0.802096247),
    vec3<f32>(0.925977992, -0.0315549991, 0.376256614),
    vec3<f32>(0.872485909, 0.0445222042, -0.486606732),
    vec3<f32>(-0.0658411323, 0.765633273, 0.639898927),
    vec3<f32>(-0.808789749, 0.47708446, 0.343874338),
    vec3<f32>(-0.403455262, -0.860233361, 0.311805093),
    vec3<f32>(0.325483351, 0.860366203, -0.392212423),
    vec3<f32>(-0.013876593, -0.366774794, -0.930206262),
    vec3<f32>(0.493826746, 0.149543463, 0.856604867),
    vec3<f32>(-0.88008479, -0.363934748, -0.304962722),
    vec3<f32>(-0.456221127, -0.889066667, -0.0377192834),
    vec3<f32>(-0.305954581, 0.613482815, 0.728032026),
    vec3<f32>(-0.817555204, 0.226670094, 0.529362028),
    vec3<f32>(0.253512351, -0.875994197, 0.410323842),
    vec3<f32>(-0.0125429371, 0.570953698, -0.820886441),
    vec3<f32>(0.731156613, 0.507331164, -0.456097683),
    vec3<f32>(0.432170282, 0.423884706, 0.79595892),
    vec3<f32>(0.555898894, -0.103206168, -0.824818105),
    vec3<f32>(-0.813088974, -0.574960231, 0.0911430348),
    vec3<f32>(-0.974757902, 0.223206386, -0.00509326911),
    vec3<f32>(-0.154445551, -0.927594176, 0.340169983),
    vec3<f32>(0.743178611, -0.643266691, -0.184101918),
    vec3<f32>(-0.194825419, 0.893176392, -0.405313446),
    vec3<f32>(0.307074424, 0.841558455, -0.444392467),
    vec3<f32>(0.945351513, 0.30589831, 0.112857172),
    vec3<f32>(0.22056963, 0.12905243, 0.966796001),
    vec3<f32>(-0.664541243, -0.589779251, -0.458852233),
    vec3<f32>(-0.380935604, -0.0860154076, -0.920591883),
    vec3<f32>(0.189632119, -0.543279607, -0.817855078),
    vec3<f32>(-0.297834738, -0.0965324716, 0.949724145),
    vec3<f32>(-0.885106, 0.439976463, 0.151684149),
    vec3<f32>(-0.275089116, -0.279066825, 0.920025916),
    vec3<f32>(-0.843058137, 0.534687665, -0.0579834393),
    vec3<f32>(-0.267546352, 0.73302797, -0.625371046),
);

// Bit-exact u32 twin of noise.rs hash(): every step of the Rust i64
// arithmetic is masked to 32 bits, so u32 wrapping ops reproduce it
// exactly. seedmul = low 32 bits of seed.wrapping_mul(0x9E37_79B1),
// premultiplied on the CPU (globals.karst).
fn khash(ix: i32, iy: i32, iz: i32, seedmul: u32) -> u32 {
    var h: u32 = bitcast<u32>(ix) * 0x8DA6B343u
        + bitcast<u32>(iy) * 0xD8163841u
        + bitcast<u32>(iz) * 0xCB1AB31Fu
        + seedmul;
    h = (h ^ (h >> 13u)) * 0xC2B2AE35u;
    h = h ^ (h >> 16u);
    return h & 255u;
}

// f32 twin of noise.rs gradient_noise: same lattice, same gradients, same
// fade - worst-case f32 drift is millimetres against the 8 m tube bands.
fn kgnoise(p: vec3<f32>, seedmul: u32) -> f32 {
    let pi = floor(p);
    let pf = p - pi;
    let ii = vec3<i32>(pi);
    let f = pf * pf * pf * (pf * (pf * 6.0 - 15.0) + 10.0);
    var total = 0.0;
    for (var dx = 0; dx < 2; dx = dx + 1) {
        let wx = select(1.0 - f.x, f.x, dx == 1);
        for (var dy = 0; dy < 2; dy = dy + 1) {
            let wy = select(1.0 - f.y, f.y, dy == 1);
            for (var dz = 0; dz < 2; dz = dz + 1) {
                let wz = select(1.0 - f.z, f.z, dz == 1);
                let g = KGRAD[khash(ii.x + dx, ii.y + dy, ii.z + dz, seedmul)];
                let d = pf - vec3<f32>(f32(dx), f32(dy), f32(dz));
                total = total + wx * wy * wz * dot(g, d);
            }
        }
    }
    return total * 1.9;
}

// Rock/stone/gravel are low-chroma mid-value materials. Dirt and sedimentary
// sand are ordered warm earth colors. Expressing both tests as ratios keeps
// the classification stable through the blocks' baked brightness/AO, while
// the value floor excludes tree trunks and the high-value ceiling excludes
// snow/ice. Vivid grass and leaves fail both chroma families.
fn strata_material_family(color: vec3<f32>) -> f32 {
    let hi = max(max(color.r, color.g), color.b);
    let lo = min(min(color.r, color.g), color.b);
    let inv_hi = 1.0 / max(hi, 1e-4);
    let chroma = (hi - lo) * inv_hi;
    let neutral = (1.0 - smoothstep(0.10, 0.22, chroma))
        * smoothstep(0.12, 0.18, hi)
        * (1.0 - smoothstep(0.43, 0.52, hi));
    let red_over_green = (color.r - color.g) * inv_hi;
    let green_over_blue = (color.g - color.b) * inv_hi;
    let earth = smoothstep(0.07, 0.14, red_over_green)
        * smoothstep(0.10, 0.22, green_over_blue)
        * smoothstep(0.27, 0.34, hi)
        * (1.0 - smoothstep(0.70, 0.78, hi));
    return clamp(max(neutral, earth), 0.0, 1.0);
}

fn strata_multiplier(
    rel: vec3<f32>,
    normal: vec3<f32>,
    material_color: vec3<f32>,
    dist_km: f32,
) -> vec3<f32> {
    if (dist_km >= 80.0 || STRATA_BAND_STRENGTH <= 0.0) {
        return vec3<f32>(1.0);
    }
    // Camera up differs from local radial up by <1.7 degrees inside the
    // 80 km effect horizon. It is a deliberately cheap conservative reject:
    // interpolated normals can only admit extra candidates, never hide a real
    // cliff. Exact local steepness follows only for those candidates.
    let coarse_steepness = 1.0 - clamp(abs(dot(normal, globals.sky.xyz)), 0.0, 1.0);
    if (coarse_steepness <= 0.075) {
        return vec3<f32>(1.0);
    }
    let family = strata_material_family(material_color);
    if (family <= 0.001) {
        return vec3<f32>(1.0);
    }
    let wp = rel - globals.center.xyz;
    let surface_dir = normalize(wp);
    let steepness = 1.0 - abs(dot(normalize(normal), surface_dir));
    let exposed = smoothstep(0.08, 0.28, steepness)
        * family
        * (1.0 - smoothstep(25.0, 80.0, dist_km));
    if (exposed <= 0.001) {
        return vec3<f32>(1.0);
    }

    // Camera-relative radial expansion, shared with the karst depth path:
    // raw length(wp) at planet magnitude quantizes by ~1 m and makes thin beds
    // crawl. Every varying term here stays local/f32-precise; the quadratic
    // term restores planetary curvature across the visible ground patch.
    let camera_up = normalize(-globals.center.xyz);
    let eye_radius = max(length(globals.center.xyz), 1.0);
    let radial = dot(rel, camera_up);
    let d2 = dot(rel, rel);
    let elevation_m = (globals.weather8.w + radial
        + (d2 - radial * radial) / (2.0 * eye_radius)) * 1000.0;

    let seedmul = globals.karst.w + STRATA_SEED_DELTA;
    let folded_m = elevation_m + STRATA_TILT_AMPLITUDE_M
        * kgnoise(surface_dir * STRATA_FOLD_FREQUENCY, seedmul);
    let thickness_m = STRATA_REFERENCE_THICKNESS_M
        * max(STRATA_BAND_THICKNESS_SCALE, 0.05);
    let bed = folded_m / thickness_m;
    let bed_base = i32(floor(bed));
    let phase = fract(bed);
    let footprint = fwidth(bed);
    // Retire beds once a pixel spans more than one layer; this is both the
    // medium-altitude anti-alias and the orbital cost/moire cut-line.
    let resolved = 1.0 - smoothstep(0.55, 1.15, footprint);
    let aa = clamp(footprint * 0.5, 0.015, 0.35);
    let palette0 = khash(bed_base, 17, -31, seedmul) % 5u;
    let palette1 = khash(bed_base + 1, 17, -31, seedmul) % 5u;
    let band_color = mix(
        STRATA_PALETTE[palette0],
        STRATA_PALETTE[palette1],
        smoothstep(1.0 - aa, 1.0, phase),
    );
    return mix(
        vec3<f32>(1.0),
        band_color,
        exposed * resolved * STRATA_BAND_STRENGTH,
    );
}

// L-1(c): away from cliffs (strata) and features, land rendered as one flat
// tone across hundreds of meters at medium altitude. Three world-anchored
// gradient-noise octaves modulate ground luminance a few percent. Each octave
// retires by its own screen footprint (the strata anti-alias rule), so orbit
// pays nothing and shows no shimmer, and the multiplier converges to 1.0
// instead of to a gray veil. One fragment-owned field serves heightfield
// tiles and voxel chunks identically.
fn patch_multiplier(rel: vec3<f32>, material_color: vec3<f32>, dist_km: f32) -> vec3<f32> {
    if (dist_km >= 220.0 || PATCH_STRENGTH <= 0.0) {
        return vec3<f32>(1.0);
    }
    // Water and ice ride their own color lanes/whiteout; a blue-dominant or
    // near-white pixel keeps its approved tone. Snow keeps a hint (drifts).
    let hi = max(max(material_color.r, material_color.g), material_color.b);
    let waterish = smoothstep(0.005, 0.03, material_color.b - max(material_color.r, material_color.g));
    let snowish = smoothstep(0.55, 0.72, hi);
    let gate = (1.0 - waterish) * mix(1.0, 0.35, snowish);
    if (gate <= 0.001) {
        return vec3<f32>(1.0);
    }
    let surface_dir = normalize(rel - globals.center.xyz);
    let seedmul = globals.karst.w + PATCH_SEED_DELTA;
    // Shared screen-footprint estimate: lattice cells per pixel at the base
    // frequency; each finer octave scales it by its frequency ratio.
    let footprint = length(fwidth(surface_dir)) * PATCH_FREQ_BASE;
    var value = 0.0;
    var frequency = 1.0;
    var amplitude = 1.0;
    for (var octave = 0u; octave < 3u; octave = octave + 1u) {
        let resolved = 1.0 - smoothstep(0.35, 0.9, footprint * frequency);
        if (resolved > 0.001) {
            value += amplitude * resolved
                * kgnoise(surface_dir * (PATCH_FREQ_BASE * frequency), seedmul + octave);
        }
        frequency *= 4.0;
        amplitude *= 0.62;
    }
    // Bright patches skew warm (dry grass, thin soil), dark patches skew
    // cool (lush, damp) — reads as ground variation, not a gray veil.
    let swing = clamp(PATCH_STRENGTH * gate * value, -0.16, 0.16);
    return clamp(
        vec3<f32>(1.0 + swing * 1.18, 1.0 + swing, 1.0 + swing * 0.72),
        vec3<f32>(0.8),
        vec3<f32>(1.2),
    );
}

fn coastal_bathymetry_color(water_color: vec3<f32>) -> vec3<f32> {
    // The untinted fresh/deep ramp is affine: g = 0.6408163*b - 0.0222449.
    // Only true sea inside the established 20 m shoal carrier rises above
    // that line. Interpolation preserves the residual exactly; fresh rivers
    // and lakes remain zero. Mineral-pale salt water is rejected by red.
    let base_green = 0.6408163 * water_color.b - 0.0222449;
    let carrier = clamp((water_color.g - base_green) / 0.0825510, 0.0, 1.0)
        * (1.0 - smoothstep(0.12, 0.20, water_color.r));
    let shallow = clamp(carrier * BATHYMETRY_DEPTH_FALLOFF_SCALE, 0.0, 1.0);
    return mix(
        water_color,
        BATHYMETRY_SHALLOW_TINT,
        shallow * BATHYMETRY_SHALLOW_MIX,
    );
}

// The categorical boundary field's three range-safe rotated domains. This is
// the f32 twin of planet.rs BIOME_WARP_AXES; kgnoise supplies the exact shared
// gradient lattice; danchor_cell.w carries its octave-zero premultiplied
// seed (karst.w belongs to the cloud layout).
const BIOME_RANGE_AXES = array<mat3x3<f32>, 3>(
    mat3x3<f32>(
        vec3<f32>(0.811107106, 0.324442842, -0.486664263),
        vec3<f32>(-0.235702260, 0.942809042, 0.235702260),
        vec3<f32>(0.371390676, -0.557086015, 0.742781353),
    ),
    mat3x3<f32>(
        vec3<f32>(-0.685994341, 0.514495755, 0.514495755),
        vec3<f32>(0.696310624, 0.696310624, -0.174077656),
        vec3<f32>(-0.365148371, 0.182574185, 0.912870929),
    ),
    mat3x3<f32>(
        vec3<f32>(0.486664263, -0.811107106, -0.324442842),
        vec3<f32>(0.169030851, 0.507092553, 0.845154255),
        vec3<f32>(0.745355992, -0.298142397, 0.596284794),
    ),
);

fn biome_normal_cdf(x: f32) -> f32 {
    let z = abs(x);
    let t = 1.0 / (1.0 + 0.2316419 * z);
    let tail = 0.39894228 * exp(-0.5 * z * z) * t
        * (0.31938153 + t * (-0.356563782
            + t * (1.781477937 + t * (-1.821255978 + t * 1.330274429))));
    return select(1.0 - tail, tail, x < 0.0);
}

fn biome_range_comparator(dir: vec3<f32>) -> f32 {
    // Production climate raster is 1024: 1 / ((2/1023) * 5 texels).
    // Continue the exact shared field below the old fixed five-octave prefix,
    // but stop before a carrier becomes sub-pixel. On Neisor the ten layers
    // are approximately 85 km .. 0.8 m; a 6-8 km view normally resolves the
    // ~40 m layer, while a 23 km view resolves the ~140 m layer.
    let pixel_angle = max(length(dpdx(dir)), length(dpdy(dir)));
    var frequency = 102.3;
    var sum = 0.0;
    var variance = 0.0;
    for (var octave = 0; octave < 10; octave = octave + 1) {
        let axes = BIOME_RANGE_AXES[octave % 3];
        let domain = vec3<f32>(dot(dir, axes[0]), dot(dir, axes[1]), dot(dir, axes[2]));
        var amplitude = 1.0;
        if (octave == 1) {
            amplitude = 0.55;
        } else if (octave > 2) {
            amplitude = pow(0.5, f32(octave - 2));
        }
        let octave_footprint = pixel_angle * frequency;
        let resolved = 1.0 - smoothstep(
            BIOME_BORDER_OCTAVE_FULL_CELLS_PER_PX,
            BIOME_BORDER_OCTAVE_CUTOFF_CELLS_PER_PX,
            octave_footprint,
        );
        // The frequency only rises, so all later octaves are also retired.
        // Octave zero is a numerical/planet-thumbnail fallback: it remains
        // the coarsest deterministic class decision if no layer reaches 2 px.
        if (octave > 0 && resolved <= 0.0) {
            break;
        }
        let adapted = amplitude * select(resolved, 1.0, octave == 0);
        let seedmul = bitcast<u32>(globals.danchor_cell.w)
            + u32(octave) * 131u * 0x9E3779B1u;
        sum += adapted * kgnoise(domain * frequency, seedmul);
        variance += adapted * adapted;
        frequency *= 3.6;
    }
    return clamp(biome_normal_cdf(-2.70 * sum / sqrt(variance)), 0.0, 1.0);
}

fn biome_owner_coverage(margin: f32) -> f32 {
    // Euclidean screen gradient makes the named TOTAL band orientation-free;
    // fwidth is an L1 norm and the former +/- full-fwidth radius produced a
    // 2..2.8 px edge before its additional 2/255 minimum-width floor.
    let margin_per_px = length(vec2<f32>(dpdx(margin), dpdy(margin)));
    let half_band = max(0.5 * BIOME_BORDER_AA_BAND_PX * margin_per_px, 1e-6);
    return smoothstep(-half_band, half_band, margin);
}

fn beach_range_comparator(dir: vec3<f32>, fine_gain: f32) -> f32 {
    // COLUMNS_PER_FACE / (2 * ECOTONE_BASE_PATCH_COLUMNS).
    // The seed delta converts the range-biome base seed in danchor_cell.w
    // into ((planet_seed + BEACH_FIELD_SEED_OFFSET) * 7919) * hash multiplier.
    var frequency = 1220.703125;
    var amplitude = 1.0;
    var sum = 0.0;
    var variance = 0.0;
    for (var octave = 0; octave < 2; octave = octave + 1) {
        let adapted = amplitude * select(fine_gain, 1.0, octave == 0);
        let seedmul = bitcast<u32>(globals.danchor_cell.w) + 0xAB20C0C2u
            + u32(octave) * 131u * 0x9E3779B1u;
        sum += adapted * kgnoise(dir * frequency, seedmul);
        variance += adapted * adapted;
        frequency *= 16.0;
        amplitude *= 0.5;
    }
    return clamp(biome_normal_cdf(2.56 * sum / sqrt(variance)), 0.0, 1.0);
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    var rel = in.pos + tile.offset.xyz;
    let d = length(rel);
    var wet = in.water.a;
    var shore = in.morph.w;
    if (tile.morph.y > 0.0) {
        // Geomorphing: slide to the parent triangle's height and its actual
        // triangle-interpolated river paint. The scalar radial slide retains
        // only the measured <= 0.13 m residual in the V-6 level-9 probes.
        // morph.z is the TEMPORAL ease: a freshly-landed async tile starts
        // at its parent's geometry and settles in, so late builds arrive
        // as a slide instead of a snap.
        let m = max(
            clamp((d - tile.morph.x) / (tile.morph.y - tile.morph.x), 0.0, 1.0),
            tile.morph.z,
        );
        let radial = normalize(rel - globals.center.xyz);
        rel += radial * (in.morph.x * m);
        wet = mix(in.water.a, in.morph.y, m);
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
        if (in.morph.z > 1.5) {
            // A-5: this marked lake/river-overlap top first moves from its
            // lattice ceiling to the shared analog level (morph_dh), then
            // sheds the patch lift through the established outer rim band.
            // At the selected-radius handoff it is exactly coplanar with the
            // always-retained mesh safety plane; the fragment stage gives the
            // outer side to that one plane before the voxel top can cross it.
            let handoff = globals.misc.z;
            let flush = smoothstep(handoff * 0.85, handoff, horiz);
            let top_weight = clamp(in.morph.y, 0.0, 1.0);
            rel += globals.hole_up.xyz
                * (in.morph.x - globals.center.w * flush * top_weight);
            shore = handoff - horiz;
        } else {
            let sink = smoothstep(globals.misc.z * 0.85, globals.misc.z * 1.06, horiz);
            rel -= globals.hole_up.xyz * (globals.center.w * 1.1 * sink);
        }
        // temporal emergence: freshly-landed chunks start sunk flush with
        // the mesh (full lift) and rise in over ~18 frames - the block
        // counterpart of the tile ease above (morph.z, captures settle it)
        rel -= globals.hole_up.xyz * (globals.center.w * tile.morph.z);
    }
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    out.dist_km = length(rel);
    out.rel_flag = vec4<f32>(rel, tile.offset.w);
    out.water = vec4<f32>(in.water.rgb, wet);
    out.wflag = in.morph.z;
    out.shore = shore;
    out.biome0 = in.biome0;
    out.biome1 = in.biome1;
    out.biome2 = in.biome2;
    out.biome3 = in.biome3;
    out.biome4 = in.biome4;
    out.biome5 = in.biome5;
    out.biome6 = in.biome6;
    out.biome7 = in.biome7;
    out.beach = in.beach;
    return out;
}

// Moon terrain uses the same compact vertex payload as Neisor tiles, but its
// radial morph center is the physical moon rather than globals.center
// (Neisor).  Tile origins and the body center were combined in f64 CPU space;
// this shader only ever sees their camera-relative remainder.
@vertex
fn vs_moon(in: VsIn) -> VsOut {
    var out: VsOut;
    var rel = in.pos + tile.offset.xyz;
    let d = length(rel);
    if (tile.morph.y > 0.0) {
        let m = clamp((d - tile.morph.x) / (tile.morph.y - tile.morph.x), 0.0, 1.0);
        let radial = normalize(rel - globals.moon_body.xyz);
        rel += radial * (in.morph.x * m);
    }
    if (tile.offset.w > 3.5 && globals.misc.z > 0.0) {
        let q = rel - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = length(q - globals.hole_up.xyz * vert);
        let sink = smoothstep(globals.misc.z * 0.85, globals.misc.z * 1.06, horiz);
        rel -= globals.hole_up.xyz * (globals.center.w * 1.1 * sink);
        rel -= globals.hole_up.xyz * (globals.center.w * tile.morph.z);
    }
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    out.dist_km = d;
    out.rel_flag = vec4<f32>(rel, tile.offset.w);
    // MoonGenerator stores mare smoothness in water.a and ray survival in
    // wflag; neither channel invokes Neisor water/weather behavior here.
    out.water = in.water;
    out.wflag = in.morph.z;
    out.shore = 0.0;
    // moon vertices carry the payload-off biome marker; pass it through so
    // fs_main-family interpolants stay defined (fs_moon ignores them)
    out.biome0 = in.biome0;
    out.biome1 = in.biome1;
    out.biome2 = in.biome2;
    out.biome3 = in.biome3;
    out.biome4 = in.biome4;
    out.biome5 = in.biome5;
    out.biome6 = in.biome6;
    out.biome7 = in.biome7;
    out.beach = in.beach;
    return out;
}

@fragment
fn fs_moon(in: VsOut) -> @location(0) vec4<f32> {
    // Lunar rock stand-ins reuse the tree landing contract: beach.y marks
    // impostor geometry, beach.x marks members already present on the parent
    // stride. Only the density delta dithers in as a finer tile settles.
    if (in.beach.y > 0.5 && tile.morph.z > 0.0) {
        let n = hash31(vec3<f32>(floor(in.clip.xy), 11.0));
        if (n < tile.morph.z * (1.0 - in.beach.x)) {
            discard;
        }
    }
    let n = normalize(in.normal);
    // Cube-sphere selection is conservative at the horizon and skirts are
    // two-sided; retain only the camera-facing physical surface.
    if (in.rel_flag.w < 3.5 && dot(n, -normalize(in.rel_flag.xyz)) <= 0.0) {
        discard;
    }
    if (in.rel_flag.w < 3.5 && globals.body_frame.w > 0.5 && globals.hole.w > 0.0) {
        let q = in.rel_flag.xyz - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = q - globals.hole_up.xyz * vert;
        if (abs(vert) < 25.0 && length(horiz) < globals.hole.w) {
            discard;
        }
    }
    let to_sun = normalize(globals.sun_body.xyz - globals.moon_body.xyz);
    let lambert = max(dot(n, to_sun), 0.0);
    // Smooth maria carry a slightly softer terminator than disturbed
    // highlands.  The distinction is intentionally subtle; their identity is
    // albedo and broad relief first.
    let lit = mix(lambert, sqrt(lambert), clamp(in.water.a, 0.0, 1.0) * 0.12);
    let lunar = globals.eclipse.y;
    let tint = mix(globals.moon_tint.rgb, globals.moon_copper_tint.rgb, lunar);
    let brightness = mix(0.05 + 0.95 * lit, 0.22 + 0.18 * lit, lunar);
    let surface = clamp(in.color, vec3<f32>(0.0), vec3<f32>(1.0));

    // Preserve P1's approved atmospheric fade from Neisor.  Once the camera
    // is actually near the moon there is no Neisor horizon/atmosphere gate.
    let sunh = dot(globals.sun_dir.xyz, globals.sky.xyz);
    let day = smoothstep(-0.08, 0.15, sunh) * (1.0 - globals.eclipse.x);
    let above = smoothstep(-0.06, 0.06, dot(globals.moon_dir.xyz, globals.sky.xyz));
    let atmospheric_visibility = max(1.0 - 0.9 * day, globals.eclipse.x) * above;
    let near_moon = length(globals.moon_body.xyz) < globals.moon_body.w * 12.0;
    let visibility = select(atmospheric_visibility, 1.0, near_moon);
    return vec4<f32>(tint * surface * brightness, visibility);
}

// Exact two-circle intersection as a fraction of the SOURCE disc. The same
// equation lives in orbits.rs for evidence/assertions; here it is evaluated
// from each terrain pixel so the moon's penumbra crosses Neisor in orbit.
fn circle_overlap_fraction(r: f32, q: f32, d: f32) -> f32 {
    if (r <= 0.0 || q <= 0.0 || d >= r + q) {
        return 0.0;
    }
    if (d <= abs(q - r)) {
        if (q >= r) {
            return 1.0;
        }
        return clamp(q * q / (r * r), 0.0, 1.0);
    }
    let ar = acos(clamp((d * d + r * r - q * q) / (2.0 * d * r), -1.0, 1.0));
    let aq = acos(clamp((d * d + q * q - r * r) / (2.0 * d * q), -1.0, 1.0));
    let lens = r * r * ar + q * q * aq - 0.5 * sqrt(max(
        (-d + r + q) * (d + r - q) * (d - r + q) * (d + r + q),
        0.0,
    ));
    return clamp(lens / (3.14159265 * r * r), 0.0, 1.0);
}

fn solar_occlusion_at(rel: vec3<f32>) -> f32 {
    let sv = globals.sun_body.xyz - rel;
    let mv = globals.moon_body.xyz - rel;
    let sd = length(sv);
    let md = length(mv);
    if (md >= sd || sd <= globals.sun_body.w || md <= globals.moon_body.w) {
        return 0.0;
    }
    let sr = asin(clamp(globals.sun_body.w / sd, 0.0, 1.0));
    let mr = asin(clamp(globals.moon_body.w / md, 0.0, 1.0));
    let separation = acos(clamp(dot(sv / sd, mv / md), -1.0, 1.0));
    return circle_overlap_fraction(sr, mr, separation);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Freshly-landed tiles (tile.morph.z = temporal ease) DISSOLVE their
    // impostor trees in with a screen dither instead of materializing a
    // denser LOD's canopy in one frame - the flashiest motion pop. The
    // marker is heuristic but tight: tile geometry (rel_flag.w), payload
    // off (beach.z), not water (wflag), no shore field - only impostor
    // quads satisfy all four.
    if (in.rel_flag.w > 0.5 && tile.morph.z > 0.0
        && in.beach.z < 0.5 && in.wflag < 0.5 && in.shore < -0.5) {
        // beach.x = 1 marks parent-lattice trees: they were ALREADY standing
        // on the coarser stand-in, so they must not blink out and refade
        // (Andrew's approach-fade report) - only the density DELTA dissolves.
        let n = hash31(vec3<f32>(floor(in.clip.xy), 7.0));
        if (n < tile.morph.z * (1.0 - in.beach.x)) {
            discard;
        }
    }
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
    // Track-B river-fall faces (wflag 3+) are the adaptive mesh's shared
    // profile, not a separate safety plane. Let voxel water replace them in
    // the near disc; only the established sea/lake carrier around wflag 1
    // remains beneath the patch.
    if (in.rel_flag.w > 0.5 && globals.hole.w > 0.0
        && (in.wflag < 0.98 || in.wflag > 1.5)) {
        let q = in.rel_flag.xyz - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = q - globals.hole_up.xyz * vert;
        if (abs(vert) < 25.0 && length(horiz) < globals.hole.w) {
            discard;
        }
    }
    // Marked A-5 voxel water owns the complementary inner half of the exact
    // same radial handoff. Outside it, only the retained continuous mesh plane
    // is visible, so no lattice side/foam face follows the camera at the rim.
    if (in.rel_flag.w < 0.5
        && in.wflag > 1.5
        && in.wflag < 2.5
        && in.shore < 0.0) {
        discard;
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
    let biome_range = smoothstep(
        BIOME_RANGE_BLEND_START_KM,
        BIOME_RANGE_BLEND_END_KM,
        in.dist_km,
    );
    // Range ownership is a CONTOUR, not an interpolated vertex color.
    // Climate uses the shared 85/24/6.5/1.8/0.5 km comparator. Generic beach
    // reuses the same broad 7 km + 440 m prefix as exact voxel beach material;
    // its coverage no longer gets smoothly mixed into every climate endpoint.
    // Invalid early threshold ordering remains the payload-off marker for
    // voxel/water/impostor vertices.
    // GATED on the range blend actually applying: below the 4 km blend start
    // biome_range is exactly 0 and this whole reconstruction (five kgnoise
    // octaves + the 8-family loop per fragment) multiplied away to nothing -
    // ground frames paid ~6x for zero pixels (2026-07-12 fps collapse).
    var ground = in.color;
    if (biome_range > 0.0) {
    let surface_dir = normalize(in.rel_flag.xyz - globals.center.xyz);
    let biome_q = biome_range_comparator(surface_dir);
    let biome_endpoints = array<vec4<f32>, 8>(
        in.biome0, in.biome1, in.biome2, in.biome3,
        in.biome4, in.biome5, in.biome6, in.biome7,
    );
    var biome_below: array<f32, 8>;
    var biome_valid = true;
    for (var family = 0; family < 8; family = family + 1) {
        let margin = biome_endpoints[family].a - biome_q;
        biome_below[family] = biome_owner_coverage(margin);
        if (family > 0) {
            biome_valid = biome_valid
                && biome_endpoints[family].a >= biome_endpoints[family - 1].a;
        }
    }
    // STRICT validity: in.beach.z interpolates 0..1 across a triangle
    // that spans land (valid) and water/voxel (payload-off) vertices - a
    // 0.5 cut painted half of every shoreline triangle with half-blended
    // sand (tan streaks ON the lake, 2026-07-12 flicker triage). Only a
    // fully-valid interior owns beach; the blocks own the exact shoreline.
    let beach_mode = in.beach.z > 0.98;
    var range_ground = in.color;
    if (biome_valid) {
        range_ground = vec3<f32>(0.0);
        var range_mean = vec3<f32>(0.0);
        var range_weight = 0.0;
        var mean_weight = 0.0;
        var previous = 0.0;
        var previous_threshold = 0.0;
        for (var family = 0; family < 7; family = family + 1) {
            let interval = biome_endpoints[family].a - previous_threshold;
            let exists = smoothstep(0.5 / 255.0, 1.5 / 255.0, interval);
            let weight = max(biome_below[family] - previous, 0.0) * exists;
            range_ground += biome_endpoints[family].rgb * weight;
            range_weight += weight;
            range_mean += biome_endpoints[family].rgb * interval;
            mean_weight += interval;
            previous = biome_below[family];
            previous_threshold = biome_endpoints[family].a;
        }
        let final_exists = smoothstep(
            0.5 / 255.0,
            1.5 / 255.0,
            1.0 - previous_threshold,
        );
        let final_weight = max(1.0 - previous, 0.0) * final_exists;
        range_ground += biome_endpoints[7].rgb * final_weight;
        range_weight += final_weight;
        let final_interval = max(1.0 - previous_threshold, 0.0);
        range_mean += biome_endpoints[7].rgb * final_interval;
        mean_weight += final_interval;
        range_ground /= max(range_weight, 1e-5);
        range_mean /= max(mean_weight, 1e-5);
        if (beach_mode) {
            let beach_coverage = clamp(in.beach.x, 0.0, 1.0);
            // CPU endpoint centering makes this combined expectation exactly
            // the former smooth beach mean, so orbit can reduce contrast
            // without shifting the approved continental/coastal color.
            range_mean = mix(range_mean, biome_endpoints[6].rgb, beach_coverage);
            if (beach_coverage > 0.5 / 255.0) {
                // The ~440 m carrier is safe through the 95 km review range
                // but undersampled from orbit. Retire only that nested detail
                // with the existing orbital handoff; the broad 7 km beach
                // owner and its 0.45 contrast remain.
                let beach_detail = 1.0 - smoothstep(
                    BIOME_ORBIT_CONTRAST_FADE_START_KM,
                    BIOME_ORBIT_CONTRAST_FADE_END_KM,
                    in.dist_km,
                );
                let fine_gain = clamp(in.beach.y, 0.08, 1.0) * beach_detail;
                let beach_q = beach_range_comparator(surface_dir, fine_gain);
                let margin = beach_coverage - beach_q;
                let beach_owner = biome_owner_coverage(margin);
                range_ground = mix(
                    range_ground,
                    biome_endpoints[6].rgb,
                    beach_owner,
                );
            }
        }
        let orbit_undersampling = smoothstep(
            BIOME_ORBIT_CONTRAST_FADE_START_KM,
            BIOME_ORBIT_CONTRAST_FADE_END_KM,
            in.dist_km,
        );
        let categorical_contrast = mix(
            BIOME_RANGE_CATEGORICAL_CONTRAST,
            BIOME_ORBIT_CATEGORICAL_CONTRAST,
            orbit_undersampling,
        );
        range_ground = mix(range_mean, range_ground, categorical_contrast);
    }
    ground = mix(in.color, range_ground, biome_range);
    }
    // One fragment-owned field serves heightfield tiles and voxel chunks.
    // Material classification reads the original approved albedo so the
    // medium-range biome reconstruction cannot turn grass/snow into strata;
    // the returned multiplier then preserves whichever ground color won.
    ground *= strata_multiplier(
        in.rel_flag.xyz,
        in.normal,
        in.color,
        in.dist_km,
    );
    // L-1(c): mid-scale patchiness on the winning ground color (post-biome,
    // post-strata), so gates read the tone that actually renders.
    ground *= patch_multiplier(in.rel_flag.xyz, ground, in.dist_km);
    // Planet lighting belongs to THIS pixel, not the camera. The vector is
    // reconstructed from camera-relative inputs, so orbital f32 precision
    // never enters through a raw world coordinate. At ground range pixel_up
    // converges to camera up; in orbit it exposes the full terminator.
    let camera_day_geom = smoothstep(-0.08, 0.15, dot(globals.sun_dir.xyz, globals.sky.xyz));
    let planet_day_weight = smoothstep(2.5, 40.0, max(globals.weather8.w, 0.0));
    var pixel_up = globals.sky.xyz;
    var pixel_sun = globals.sun_dir.xyz;
    var pixel_day_geom = camera_day_geom;
    // Keep the overwhelmingly common ground path byte-cheap as well as
    // byte-equivalent. Orbital frames and actual eclipse windows pay for the
    // camera-relative body/pixel geometry; normal ground frames do not.
    if (planet_day_weight > 0.0 || globals.eclipse.z > 0.5) {
        pixel_up = normalize(in.rel_flag.xyz - globals.center.xyz);
        pixel_sun = normalize(globals.sun_body.xyz - in.rel_flag.xyz);
        pixel_day_geom = smoothstep(-0.08, 0.15, dot(pixel_sun, pixel_up));
    }
    var eclipse = 0.0;
    if (globals.eclipse.z > 0.5) {
        eclipse = solar_occlusion_at(in.rel_flag.xyz);
    }
    // Below the voxel/weather layer every visible ground pixel is locally
    // co-located for this broad ambient term: converge EXACTLY to the old
    // camera result. Hand off continuously to true per-pixel planet day by
    // 40 km, long before an orbital hemisphere/terminator is visible.
    let day_geom = mix(camera_day_geom, pixel_day_geom, planet_day_weight);
    let day = day_geom * (1.0 - eclipse);
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
    // ---- karst breach hint (BUGS.md V-10) ----
    // The mesh evaluates the SAME cave noises the voxel columns carve
    // with (bit-exact u32 lattice hash, kgnoise; seeds in globals.karst),
    // so surface cave mouths stop popping at the patch boundary: where
    // the tube grazes the surface, a pool joins the water pipeline when
    // the water table is near (the shore field within ~4 m of ground),
    // else the ground darkens into a dry pit mouth. Full strength out to
    // the patch rim (misc.z) so the handoff matches the real carved
    // blocks, then fades - a 10 m pit is sub-pixel long before that.
    // gate by the CAMERA's air temperature (the hint fades by ~2.4 km, so
    // camera temp ~= pit temp): in frozen country the voxels bury cave
    // mouths under solid ice, but the mesh hint still painted pits/pools
    // (Andrew's 'breaches on mesh' ice photos at 73.9 -76). weather-off
    // sentinel is 999, which passes.
    if (in.rel_flag.w > 0.5 && wet < 0.98 && globals.weather.w > -4.0) {
        let kfade = 1.0 - smoothstep(globals.misc.z + 0.4, globals.misc.z + 2.4, in.dist_km);
        if (kfade > 0.02) {
            let wp = in.rel_flag.xyz - globals.center.xyz;
            let dirp = normalize(wp);
            let region = kgnoise(dirp * 90.0, globals.karst.x);
            if (region > -0.05) {
                // depth input to the tube field, in metres like col_ctx's
                // ground0 — computed CAMERA-RELATIVELY. length(wp) at
                // planet magnitude is f32-quantized to ~1 m, and every
                // quantum shifts the tube field ~8 m, which rendered as
                // moire scanlines. Expanding |eye + rel| - R around the
                // camera keeps every term small: sky.w is the camera's
                // height, dot(rel, up) the local slope, and the quadratic
                // term the planet's curvature (~0.3 m at 2 km).
                let u_r = normalize(-globals.center.xyz);
                let e_big = length(globals.center.xyz);
                let radial = dot(in.rel_flag.xyz, u_r);
                let d2 = dot(in.rel_flag.xyz, in.rel_flag.xyz);
                let zm = (globals.weather8.w + radial + (d2 - radial * radial) / (2.0 * e_big))
                    * 1000.0;
                let a1 = abs(kgnoise(dirp * (90000.0 + zm / 12.0), globals.karst.y));
                if (a1 < 0.085) {
                    let a2 = abs(kgnoise(dirp * (76000.0 + zm / 9.0), globals.karst.z));
                    // blocks carve on a hard |n| < 0.085 — keep the hint
                    // near-binary too (full strength to 0.076, AA band to
                    // the true edge) or pools render thinner than their
                    // block twins
                    let t = (1.0 - smoothstep(0.076, 0.085, a1))
                        * (1.0 - smoothstep(0.076, 0.085, a2))
                        * smoothstep(-0.05, -0.02, region)
                        * kfade;
                    // flooded iff the pit reaches below the water table.
                    // The pit's depth is a DEPTH LADDER of the tube field:
                    // any breach carves at least its surface block (floods
                    // a table within ~1.5 m); a tube still open ~1.5 m
                    // down makes a 2-3 block pit (table to ~3 m); open
                    // ~3 m down, a shaft (table to the -5 m shore clamp,
                    // which doubles as the visibility cap - deeper pools
                    // hide behind their pit walls). One fixed probe depth
                    // either invented pools on high ground (13.349 -4.798)
                    // or dried shallow-table channels (the k150 field).
                    var flooded = in.shore > -0.0015;
                    if (!flooded && in.shore > -0.0028) {
                        flooded = max(
                            abs(kgnoise(dirp * (90000.0 + (zm - 1.5) / 12.0), globals.karst.y)),
                            abs(kgnoise(dirp * (76000.0 + (zm - 1.5) / 9.0), globals.karst.z)),
                        ) < 0.085;
                    }
                    if (!flooded && in.shore > -0.0049) {
                        flooded = max(
                            abs(kgnoise(dirp * (90000.0 + (zm - 3.0) / 12.0), globals.karst.y)),
                            abs(kgnoise(dirp * (76000.0 + (zm - 3.0) / 9.0), globals.karst.z)),
                        ) < 0.085;
                    }
                    if (flooded) {
                        wet = max(wet, t); // pool: the water pipeline colors it
                    } else {
                        // dry pit mouth: the blocks expose DIRT walls
                        // (Mat::Dirt albedo), lightly shadowed - a plain
                        // darken read as olive smears on grass
                        // ("difficulty lake" photo, 13.336 -4.798)
                        let dirt = vec3<f32>(0.33, 0.235, 0.135);
                        ground = mix(ground, dirt * 0.72, 0.85 * t);
                    }
                }
            }
        }
    }
    // Weather is sampled once per frame AT THE CAMERA's ground point and
    // applied globally - correct when you are IN that weather, nonsense
    // from orbit: flying at 6000 km sweeps synoptic cells below and the
    // whole hemisphere's brightness pulsed with each one (Austin's polar
    // flicker, ice 126 vs 152 RGB across 1.7 deg of latitude). All
    // camera-anchored ground responses fade to neutral above the weather
    // layer: full in the troposphere, gone by 40 km.
    let wcam = 1.0 - smoothstep(8.0, 40.0, max(globals.weather8.w, 0.0));
    if (globals.weather.w < 500.0 && wcam > 0.0) {
        let wp = in.rel_flag.xyz - globals.center.xyz;
        let r_sphere = length(globals.center.xyz) - max(globals.weather8.w, 0.0);
        let elev_km = length(wp) - r_sphere;
        // lapse from the WEATHER SAMPLE's ground elevation (misc.w), not
        // the eye: the sample is surface air under the camera, and lapsing
        // from eye height warmed the ground 6.5 C per km of climb (review
        // #2 finding 1 - the snow line receded as the camera rose)
        let t_pix = globals.weather.w - 6.5 * (elev_km - globals.misc.w);
        // Smooth anchored noise, in three acts of one bug family: hash31
        // at ~82k cells degenerated into world-axis stripes (Andrew's
        // banding photos); a cell-constant hash3i re-read as a 25 m
        // checkerboard; and any noise fed raw planet-centered wp sits on
        // 0.24 m f32 plateaus that crawl with the camera. So: two smooth
        // vnoise octaves (25 m and 8.3 m - integer 3x so one anchor
        // serves both) on the camera-anchored exact lattice.
        let dp = (in.rel_flag.xyz - globals.danchor.xyz) * 40.0;
        let dith = (0.6 * vnoise_at(globals.danchor_cell.xyz, dp)
            + 0.4 * vnoise_at(globals.danchor_cell.xyz * 3, dp * 3.0))
            * 1.8 - 0.9;
        let cold = 1.0 - smoothstep(-globals.weather3.y, 1.0, t_pix + dith);
        // x(1-wflag): open water takes neither snow dusting nor rain
        // soaking - on mesh tiles the wet mix already masked this, but
        // block water quads went through the ground path and whitened
        // while the mesh sea stayed liquid (a +4 lum whole-sea split)
        let dry_px = 1.0 - clamp(in.wflag, 0.0, 1.0);
        let dust =
            cold * (0.45 + 0.55 * globals.weather.z * globals.weather.y) * wcam * dry_px;
        ground = mix(ground, vec3<f32>(0.88, 0.90, 0.94), dust);
        // Redistribute (do not create) at most weather6.w of rain from local
        // highs toward sub-raster troughs. The signed residual was computed
        // from smooth raster elevation minus detailed/carved elevation on
        // both mesh and voxel vertices; snow remains on its existing rule.
        let rain_local = clamp(
            globals.weather.y * (1.0 + globals.weather6.w * (in.beach.w * 2.0 - 1.0)),
            0.0,
            1.0,
        );
        let rain = rain_local * (1.0 - globals.weather.z) * wcam * dry_px;
        ground = ground * (1.0 - globals.weather3.x * rain);
    }
    // Mesh liquid carries its color in water.rgb; block liquid carries the
    // same terrain::water_surface_color result as color with wflag marking the
    // open top/side. Run both through one sea-carrier decoder. Fresh rivers,
    // lakes, salt water, and ice return bit-identically from the helper.
    if (in.rel_flag.w < 0.5 && in.wflag > 0.98) {
        ground = coastal_bathymetry_color(ground);
    }
    var water_color = in.water.rgb;
    if (wet > 0.001) {
        water_color = coastal_bathymetry_color(water_color);
    }
    let base = mix(ground, water_color, clamp(wet, 0.0, 1.0));
    var n = normalize(in.normal);
    // ---- per-pixel normal detail (TRANSITIONS.md: mesh reads flat) ----
    // The voxel patch gets surface relief for free — stepped 1 m column
    // tops shade individually — while the mesh interpolates one normal
    // across 26 m triangles and reads as vinyl. Perturb the mesh normal
    // per ~2 m hash cell (same lattice family as the micro-texture), with
    // the amplitude scaled by SLOPE: flat ground stays smooth exactly like
    // coplanar block tops do, hillsides roughen exactly where the columns
    // step. Water keeps its mirror (×(1−wet)), and the same ~2 km fade as
    // the micro-texture returns the far field to today's clean reading.
    if (in.rel_flag.w > 0.5) {
        let wp = in.rel_flag.xyz - globals.center.xyz;
        let cell = vec3<i32>(floor(wp * 480.0));
        let jit = vec3<f32>(
            hash3i(cell + vec3<i32>(11, 0, 0)),
            hash3i(cell + vec3<i32>(0, 7, 0)),
            hash3i(cell + vec3<i32>(0, 0, 3)),
        ) - 0.5;
        let pup = normalize(wp);
        let slope = 1.0 - dot(n, pup);
        let amp = 0.9 * smoothstep(0.01, 0.20, slope)
            * exp(-in.dist_km / 1.8) * (1.0 - clamp(wet, 0.0, 1.0));
        n = normalize(n + amp * jit);
    }
    let light = max(dot(n, pixel_sun), 0.0);
    let sky_hemi = clamp(0.5 + 0.5 * dot(n, globals.sky.xyz), 0.0, 1.0);
    // overcast: direct sun dims toward its tunable floor and the ambient
    // flattens a touch — a grey day, not a dark one (night is untouched:
    // the dimming scales with `day`)
    // overcast dimming and ambient flattening are camera-weather too:
    // scale by wcam so orbit sees the planet's true lighting (the flicker)
    let odim = mix(1.0, globals.weather2.w, globals.weather.x * day * wcam);
    let ambient =
        (0.10 + 0.40 * day * sky_hemi) * mix(1.0, 0.85, globals.weather.x * day * wcam);
    // ...and the direct term itself dies with the horizon (* day): the
    // below-horizon sun otherwise kept lighting at FULL coefficient — tree
    // canopy sides facing the set sun glowed all night, opposite the moon
    // ("tree shading is backwards", night photo at 0.626 68.962)
    // W2 cloud shadows use the exact same shell intersections, fabric,
    // seed, advection, and W-MOTION evolution as the visible clouds. This
    // fragment path is shared by mesh tiles and voxel chunks by construction.
    var cloud_shadow = 0.0;
    if (globals.weather.w < 500.0
        && day > 0.003
        && wcam > 0.003
        && globals.weather9.x > 0.0) {
        cloud_shadow = cloud_shadow_at_ground(in.rel_flag.xyz, pixel_sun);
    }
    let sun_coeff = mix(1.0, 0.60, day) * odim * day
        * (1.0 - globals.weather9.x * cloud_shadow * wcam);
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
    let atm = exp(-max(globals.weather8.w, 0.0) / 45.0);
    // moonlight: a cool directional lift plus a faint ambient floor so night
    // terrain reads as moonlit rather than flat black. Present only at night
    // (fades as the sun rises) and only while the moon is above the horizon.
    let moon = globals.moon_dir.xyz;
    let moon_up = smoothstep(0.0, 0.15, dot(moon, globals.sky.xyz));
    let moonlit = max(dot(n, moon), 0.0);
    let moon_phase = 0.5 + 0.5 * dot(
        normalize(globals.sun_body.xyz - globals.moon_body.xyz),
        normalize(in.rel_flag.xyz - globals.moon_body.xyz),
    );
    // sky-shaped night floor: with a flat floor and a LOW moon, vertical
    // step faces toward the moon glowed brighter than the tops around them
    // and terraces read as bright contour stripes (2026-07-08 night shots).
    // Faces that see more sky keep more of the floor, restoring the
    // tops-over-sides hierarchy while the directional term still lets a low
    // moon rake across scarps.
    let hemi_n = 0.5 + 0.5 * dot(n, globals.sky.xyz);
    c += base * vec3<f32>(0.40, 0.50, 0.72)
        * (moonlit * 0.10 + 0.015 + 0.05 * hemi_n)
        // Physical default moon sits ~2 deg from the legacy art direction;
        // this gain preserves the pinned ground-level moonlight reel while
        // `moon_phase` supplies the new geometric waxing/waning behavior.
        * (1.0 - day) * moon_up * moon_phase * 1.15;
    // Track-B falls: wflag 3+ carries the Rust river_tuning sheet strength
    // on both the adaptive shared-profile face and matching voxel water
    // steps. Motion reads the absolute deterministic weather clock, never
    // wall time.
    if (in.wflag > 2.9) {
        let feature = clamp(in.wflag - 3.0, 0.0, 1.0);
        let planet_pos = in.rel_flag.xyz - globals.center.xyz;
        let streak = 0.5 + 0.5 * sin(
            dot(planet_pos, vec3<f32>(83.0, 47.0, 109.0)) * 145.0
                - globals.weather4.w * 5.5,
        );
        // River-interior and voxel faces carry a negative shore value and
        // intentionally receive the stronger plunge/white-water treatment.
        let plunge = 1.0 - smoothstep(0.03, 0.38, max(in.shore, 0.0));
        let whitewater = clamp(feature * (0.30 + 0.42 * streak + 0.22 * plunge), 0.0, 0.92);
        c = mix(c, vec3<f32>(0.94, 0.985, 1.0), whitewater);
    }
    // W2 storm edge on distant terrain. The CPU's eight-probe fit is in the
    // same planet frame as this camera-relative ray, so the dark horizon
    // stays attached to the approaching front while the camera pans.
    let storm_view = normalize(in.rel_flag.xyz);
    let storm_load = clamp(
        globals.weather11.w + dot(storm_view, globals.weather11.xyz),
        0.0,
        1.0,
    );
    let storm_far = smoothstep(6.0, 90.0, in.dist_km) * wcam * day;
    let storm_gloom = storm_load * storm_far * globals.weather12.x;
    c *= 1.0 - 0.58 * storm_gloom;
    c = mix(
        c,
        vec3<f32>(0.37, 0.42, 0.29) * (0.18 + 0.82 * day),
        storm_gloom * globals.weather12.y,
    );
    // Clouds v2 / W3: once the camera rises above the high shell, composite
    // the same three deterministic formations over terrain pixels. The
    // helper stacks far-to-near shell hits and hard-caps the combined alpha,
    // so clouds can never fully obscure orbital ground.
    if (globals.weather.w < 500.0 && globals.weather8.w > globals.weather5.y) {
        // Reconstruct from the fragment coordinate exactly as the sky pass
        // does. Interpolated terrain position can reach the same screen ray
        // through different asynchronous triangulations with different last
        // bits, which sharp cloud thresholds amplified into non-byte-stable
        // orbital captures.
        let orbital_ndc = vec2<f32>(
            in.clip.x * globals.weather12.z * 2.0 - 1.0,
            1.0 - in.clip.y * globals.weather12.w * 2.0,
        );
        let orbital_p = globals.inv_view_proj * vec4<f32>(orbital_ndc, 1.0, 1.0);
        let orbital_ray = normalize(orbital_p.xyz / orbital_p.w);
        let orbital = cloud_orbit_composite(orbital_ray);
        // The unclouded output already hides last-bit asynchronous LOD
        // lighting differences below one sRGB byte. Cloud blending can move
        // those same values onto opposite byte boundaries, so stabilize the
        // pre-cloud ground far below one display step.
        c = floor(c * 4096.0 + vec3<f32>(0.5)) / 4096.0;
        // From orbit there is no camera-local sun projection. The same
        // capped composite alpha instead gives its ground a subtle footprint.
        c *= 1.0 - globals.weather9.y * orbital.a * day;
        c = mix(c, orbital.rgb, orbital.a);
    }
    // W2 height fog: dawn humidity pools in locally concave D-8 terrain;
    // near-saturated cover+precip contributes a broad bank. A horizontal
    // ceiling keeps peaks readable above the valley veil.
    if (globals.weather.w < 500.0 && wcam > 0.0 && globals.weather9.z > 0.0) {
        let concavity = smoothstep(0.04, 0.72, in.beach.w * 2.0 - 1.0);
        let humid = smoothstep(0.55, 0.88, globals.weather10.y);
        let valley_mist = concavity * humid * globals.weather10.x;
        let mist_source = max(valley_mist, globals.weather10.w);
        if (mist_source > 0.001) {
            let wp_fog = in.rel_flag.xyz - globals.center.xyz;
            let r_fog = length(globals.center.xyz) - max(globals.weather8.w, 0.0);
            let elev_fog = length(wp_fog) - r_fog;
            let above_floor = elev_fog - globals.misc.w;
            let below_ceiling = 1.0 - smoothstep(
                globals.weather9.w * 0.68,
                globals.weather9.w,
                above_floor,
            );
            let mist = (1.0 - exp(
                -in.dist_km * globals.weather9.z * mist_source,
            )) * below_ceiling * wcam * atm;
            let mist_col = mix(
                vec3<f32>(0.55, 0.62, 0.66),
                vec3<f32>(0.48, 0.51, 0.45),
                globals.weather.x * 0.35,
            ) * (0.18 + 0.82 * day);
            c = mix(c, mist_col, clamp(mist, 0.0, 0.88));
        }
    }
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

// trilinear value noise on an offset lattice: cell identity is base +
// floor(p), so callers with an exact integer anchor (the dusting dither)
// keep world-stable cells while feeding a small, camera-precise p
fn vnoise_at(base: vec3<i32>, p: vec3<f32>) -> f32 {
    let i = base + vec3<i32>(floor(p));
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

// Trilinear value noise is the shared fabric for the faked-3D cloud shells.
// Layer functions deliberately use only 2-3 taps apiece: shell parallax and
// weather-driven shape changes buy more depth than a volume march would.
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

fn cloud_seed_offset(layer: u32) -> vec3<f32> {
    // Planet seed is carried in karst.w as an independent premixed u32.
    // Integer mixing keeps the layout seed-dependent without asking large
    // f32 world coordinates to preserve low seed bits.
    var s = globals.karst.w ^ (layer * 747796405u + 2891336453u);
    s ^= s >> 16u;
    s *= 2246822519u;
    s ^= s >> 13u;
    return vec3<f32>(
        f32(s & 1023u),
        f32((s >> 10u) & 1023u),
        f32((s >> 20u) & 1023u),
    ) * 0.03125;
}

struct CloudStructure {
    domain: vec3<f32>,
    cyclone_signed: f32,
    cyclone_positive: f32,
    front: f32,
    eye: f32,
    // Signed spiral-arm density wave, separate from the additive loads:
    // cover saturates inside a storm, so arms only read as a post-clamp
    // multiplier on the shell alpha (clear lanes between dense arms).
    arm: f32,
    // Eyewall ring, max-composed like `eye`; drives the cell-formation
    // threshold toward a dense ring around the cleared eye.
    wall: f32,
};

fn cloud_rotate_axis(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let sn = sin(angle);
    let cs = cos(angle);
    // Rodrigues preserves length for unit inputs/axes; renormalizing every
    // shell/system only paid an inverse sqrt to correct roundoff that noise
    // cannot resolve.
    return v * cs + cross(axis, v) * sn
        + axis * dot(axis, v) * (1.0 - cs);
}

fn cloud_rotate_z(v: vec3<f32>, angle: f32) -> vec3<f32> {
    let sn = sin(angle);
    let cs = cos(angle);
    return vec3<f32>(v.x * cs - v.y * sn, v.x * sn + v.y * cs, v.z);
}

// W-MOTION pass 2's complete structured field. The physical sample direction
// selects storm/front presence while `domain` is inverse-mapped into each
// nearby vortex's co-rotating frame and then through
// theta(lat,t) = w0*t + slosh(t)*cos^2(lat). All centers, wrap angles, and
// the bounded shear phase came from f64 absolute weather time on the CPU;
// the hourly particle clock never resets these terms, and no term
// accumulates unbounded differential shear.
fn cloud_structure(sdir: vec3<f32>) -> CloudStructure {
    var mapped = sdir;
    var cyclone_signed = 0.0;
    var cyclone_positive = 0.0;
    var front_load = 0.0;
    var eye_load = 0.0;
    var arm_load = 0.0;
    var wall_load = 0.0;
    let radius = max(globals.weather13.z, 1e-6);
    let radius2 = radius * radius;
    let count = u32(globals.weather13.w + 0.5);
    for (var i = 0u; i < 4u; i += 1u) {
        if (i >= count) {
            break;
        }
        let packed = globals.cyclone_centers[i];
        let center = packed.xyz;
        let intensity = packed.w;
        if (intensity <= 1e-5) {
            continue;
        }
        let chord2 = max(2.0 * (1.0 - dot(sdir, center)), 0.0);
        if (chord2 < radius2 * 9.0) {
            let rn2 = chord2 / radius2;
            let rn = sqrt(rn2);
            let envelope = exp(-rn2);
            let eye = 1.0 - smoothstep(0.04, 0.18, rn);
            let eyewall = 1.0 - smoothstep(0.0, 0.16, abs(rn - 0.28));
            let profile = intensity
                * (0.75 * envelope + 0.75 * eyewall - 1.40 * eye);
            cyclone_signed += profile;
            cyclone_positive += max(profile, 0.0);
            eye_load = max(eye_load, intensity * eye);
            wall_load = max(wall_load, intensity * eyewall);

            let east = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), center));
            let north = cross(center, east);
            let offset = sdir - center * dot(sdir, center);
            let azimuth = atan2(dot(offset, north), dot(offset, east));
            let hemi = select(-1.0, 1.0, center.z >= 0.0);
            // Spiral density wave (matches the CPU term in
            // structured_loads): the arm PATTERN rotates with storm age;
            // the fabric stays put, so cells seed and disperse as an arm
            // sweeps over them. Rigid in angle-space - zero shear. Kept out
            // of `profile`: additive cover saturates inside a storm; the
            // lanes survive as a formation-threshold shift instead.
            if (globals.weather16.x > 0.5) {
                let wave = cos(globals.weather16.x * azimuth
                    + hemi * globals.weather16.y * log(max(rn, 0.15))
                    - globals.cyclone_arms[i].x);
                // signed sharpening: narrower, more legible arm crests
                let spiral = wave * abs(wave);
                // Wide radial window (not the storm envelope): rain bands
                // live outside the core; exp(-rn^2) throttled them there.
                let arm_env = smoothstep(0.30, 0.60, rn)
                    * (1.0 - smoothstep(1.7, 2.7, rn));
                arm_load += intensity * spiral * arm_env;
            }

            // COMMA-TAIL FRONT (matches structured_loads): one curved band
            // spiraling off the circulation, rotating at the arm pattern's
            // visible rate. delta*rn is the azimuthal arc distance to the
            // log-spiral centerline; weather15.xyz carry leading/trailing
            // widths and the outer radial extent in cyclone-radius units.
            if (rn > 0.05) {
                let pattern = globals.cyclone_arms[i].x
                    / max(globals.weather16.x, 1.0);
                let center_phi = globals.cyclone_arms[i].y + pattern
                    + hemi * globals.weather16.y * log(max(rn, 0.15));
                let d = azimuth - center_phi;
                let delta = d - 6.2831853 * floor(d / 6.2831853 + 0.5);
                let cross_arc = delta * rn;
                let width = select(
                    globals.weather15.y,
                    globals.weather15.x,
                    cross_arc >= 0.0,
                );
                let cross_profile = 1.0
                    - smoothstep(0.0, 1.5, abs(cross_arc) / width);
                let radial = smoothstep(0.55, 0.85, rn)
                    * (1.0 - smoothstep(globals.weather15.z - 0.5,
                        globals.weather15.z, rn));
                front_load += intensity * cross_profile * radial;
            }

            let cyclone_frame = globals.cyclone_fronts[i];
            let falloff = envelope * intensity;
            // cyclone_fronts.w is the hemisphere-signed BOUNDED core wrap
            // (tanh-saturated on the CPU): long clocks tighten toward a
            // fixed spiral and unwind as the storm's life envelope dies.
            mapped = cloud_rotate_axis(mapped, center, cyclone_frame.w * falloff);
        }
    }
    let cos2_lat = max(1.0 - sdir.z * sdir.z, 0.0);
    let theta = globals.weather13.x + globals.weather13.y * cos2_lat;
    mapped = cloud_rotate_z(mapped, -theta);
    return CloudStructure(
        mapped,
        clamp(cyclone_signed, -1.0, 2.0),
        clamp(cyclone_positive, 0.0, 2.0),
        clamp(front_load, 0.0, 1.5),
        clamp(eye_load, 0.0, 1.0),
        clamp(arm_load, -1.0, 1.0),
        clamp(wall_load, 0.0, 1.0),
    );
}

fn cloud_shell_hit(ray: vec3<f32>, altitude_km: f32) -> vec4<f32> {
    // Below a shell the positive far root is overhead; outside it the near
    // root lies between an orbital eye and the ground. w < 0 means no hit.
    let ctr = globals.center.xyz;
    let r_ground = length(ctr) - max(globals.weather8.w, 0.0);
    let r_shell = r_ground + altitude_km;
    let b = dot(ctr, ray);
    let qd = dot(ctr, ctr) - r_shell * r_shell;
    let disc = b * b - qd;
    if (disc <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    let root = sqrt(disc);
    let t_hit = select(b - root, b + root, qd < 0.0);
    if (t_hit <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    return vec4<f32>(normalize(ray * t_hit - ctr), t_hit);
}

fn cloud_color(
    sdir: vec3<f32>,
    view_dir: vec3<f32>,
    kind: u32,
    fabric: f32,
    cover: f32,
    precip: f32,
) -> vec3<f32> {
    let sun_h = dot(sdir, globals.sun_dir.xyz);
    let local_day = smoothstep(-0.08, 0.15, sun_h);
    var col = vec3<f32>(0.96, 0.97, 1.00);
    if (kind == 0u) {
        // Low rain-bearing bellies are the darkest formation.
        let heavy = clamp(0.35 * cover + 0.85 * precip, 0.0, 1.0);
        col = mix(
            vec3<f32>(0.82, 0.85, 0.90),
            vec3<f32>(0.24, 0.27, 0.34),
            clamp(heavy * (0.65 + 0.45 * fabric), 0.0, 1.0),
        );
    } else if (kind == 1u) {
        // Broken cumulus keeps bright crowns until precipitation builds.
        let heavy = clamp(0.22 * cover + 0.68 * precip, 0.0, 1.0);
        col = mix(
            vec3<f32>(0.97, 0.98, 1.00),
            vec3<f32>(0.39, 0.42, 0.49),
            clamp(heavy * (0.35 + 0.75 * fabric), 0.0, 1.0),
        );
    } else {
        // Ice-rich cirrus stays thin, pale, and slightly blue.
        col = mix(vec3<f32>(0.93, 0.96, 1.00), vec3<f32>(0.75, 0.82, 0.92), 0.18 * cover);
    }
    col *= 0.10 + 0.95 * local_day;
    let toward = pow(max(dot(view_dir, globals.sun_dir.xyz), 0.0), 6.0);
    let low_sun = 1.0 - smoothstep(0.05, 0.35, sun_h);
    let sunset = toward * low_sun * local_day * select(0.28, 0.10, kind == 0u);
    return mix(col, vec3<f32>(0.98, 0.62, 0.38), sunset);
}

/// Project a planet direction into the viewer's edge-inclusive cube-face
/// convention (planet.rs::FACES), then sample the exact CPU-baked
/// cover/precip bytes through a smoothstep-warped bilinear lookup. The warp
/// makes filtering C1: node values are exact and the interpolation contour
/// creases vanish. Its predecessor - a cell-constant hash jitter on a
/// floor(sdir*512) lattice - hid those creases but etched its own ~12 km
/// grid of brightness steps into smooth overcast decks (Austin's grid
/// report, 2026-07-14): a constant offset per cell jumps at cell borders.
fn synoptic_deck_sample(sdir_in: vec3<f32>) -> vec2<f32> {
    let d = normalize(sdir_in);
    let ad = abs(d);
    var face = 0i;
    var uv = vec2<f32>(0.0);
    // Ties prefer X, then Y, then Z, matching face_from_dir's face order.
    if (ad.x >= ad.y && ad.x >= ad.z) {
        if (d.x >= 0.0) {
            face = 0i;
            uv = vec2<f32>(d.y, d.z) / ad.x;
        } else {
            face = 1i;
            uv = vec2<f32>(-d.y, d.z) / ad.x;
        }
    } else if (ad.y >= ad.z) {
        if (d.y >= 0.0) {
            face = 2i;
            uv = vec2<f32>(-d.x, d.z) / ad.y;
        } else {
            face = 3i;
            uv = vec2<f32>(d.x, d.z) / ad.y;
        }
    } else if (d.z >= 0.0) {
        face = 4i;
        uv = vec2<f32>(d.y, -d.x) / ad.z;
    } else {
        face = 5i;
        uv = vec2<f32>(d.y, d.x) / ad.z;
    }
    // Texel nodes include u/v = +/-1. Work in node space (integer = node),
    // smoothstep the fractional part, and hand the warped coordinate to
    // hardware linear filtering: node values match sample_face() exactly
    // and the in-between interpolation becomes C1 (no crease lattice).
    let node = (uv * 0.5 + vec2<f32>(0.5)) * 63.0;
    let cell = floor(node);
    let f = node - cell;
    let fw = f * f * (3.0 - 2.0 * f);
    let tc = (cell + fw + vec2<f32>(0.5)) / 64.0;
    return textureSampleLevel(synoptic_raster, synoptic_sampler, tc, face, 0.0).rg;
}

// kind: 0 low storm base, 1 middle cumulus, 2 high cirrus. Shape, coverage,
// and color read cover/precip/temp; shell position reads only seed plus the
// advected weather clock, preserving the determinism contract.
// ---- W-MOTION pass 1 (WEATHER.md): the deck EVOLVES ----
// Three closed-form terms, each a pure function of (seed, direction,
// weather clock) - no state, byte-reproducible at any seek:
//  1. evolving domain warp: a slow vector field, itself drifting at a
//     different rate than the deck, displaces every sample - filaments
//     stretch and FOLD, the visual signature of advection.
//  2. octave shear: fine octaves drift faster than their broad carriers
//     (an energy-cascade feel; the sheet stops moving as one poster).
//  3. in-place morph: the broad mask crossfades around a 12-phase lattice
//     ring on the weather clock (5-min period, continuous across the
//     hourly wrap; phase 0 offset is zero, so pinned time-0 captures keep
//     their unmorphed fabric).
fn cloud_layer_sample_structured(
    sdir: vec3<f32>,
    view_dir: vec3<f32>,
    kind: u32,
    structure: CloudStructure,
) -> vec4<f32> {
    let spatial = synoptic_deck_sample(sdir);
    // Pins still upload a uniform raster (map/instrument contract), but the
    // scalar bypass avoids UNORM8 rounding changes in established pinned
    // reels. Live shape always comes from the spatial raster.
    let base_weather = select(spatial, globals.weather8.xy, globals.weather7.w > 0.5);
    let cover = clamp(
        base_weather.x
            + globals.weather14.y * structure.cyclone_signed
            + globals.weather14.w * structure.front,
        0.0,
        1.0,
    );
    // Structured rain rides the arm density wave (matches the CPU term):
    // lanes go dry with their cover, crests rain harder.
    let arm_precip = clamp(1.0 + globals.weather15.w * structure.arm, 0.0, 1.5);
    let precip = clamp(
        base_weather.y
            + globals.weather14.z * structure.cyclone_positive * arm_precip
            + globals.weather14.w * 0.75 * structure.front,
        0.0,
        1.0,
    );
    let cold = 1.0 - smoothstep(-8.0, 12.0, globals.weather8.z);
    let warm = smoothstep(-2.0, 22.0, globals.weather8.z);
    // W-MOTION pass 1's warp/shear/morph are REVERTED (2026-07-13):
    // at high weather clock they decorrelated and over-energized the
    // fabric into global phantom speckle (regression zoo caught by the
    // orbital cyclone-hunt probes; the reels never saw it because they
    // pin t=0). Deck evolution now comes from pass 2's structured motion
    // only; a pass-1 redo must gate on LOOK at t in {0, 1800, 3500}.
    let sdir_w = structure.domain;
    // The storm's whole anatomy modulates the cell-FORMATION threshold, not
    // the alpha of formed cells (Andrew, 2026-07-14: alpha carves read as
    // ghost clouds; sparse regions should simply grow FEWER cells, forming
    // and dissolving by the same fabric dynamics as everywhere else).
    // Signed arms: crests lower the threshold, lanes raise it. The eye
    // raises it past the fabric ceiling (~zero cells); the eyewall and the
    // comma-tail front lower it (dense ring / textured band). All are
    // post-saturation by construction - thresholds never clamp at cover 1.
    let arm_shift = globals.weather15.w * structure.arm;
    let front_t = min(structure.front, 1.0);
    if (kind == 2u) {
        let p0 = (sdir_w - globals.weather2.xyz * 0.72) * globals.weather6.z
            + cloud_seed_offset(2u);
        // An anisotropic rotated domain stretches cells into fibrous cirrus.
        let p = vec3<f32>(
            p0.x * 0.24 + p0.z * 0.10,
            p0.y * 1.18 + p0.x * 0.05,
            p0.z * 0.54 - p0.x * 0.07,
        );
        let fabric = 0.68 * vnoise(p)
            + 0.32 * vnoise(p * 2.07 + vec3<f32>(13.1, 7.7, 29.3));
        let cirrus_shift = 0.10 * arm_shift - 0.45 * structure.eye;
        let wisps = smoothstep(
            0.57 - 0.035 * cold - cirrus_shift,
            0.70 - 0.025 * cold - cirrus_shift,
            fabric,
        );
        let legacy_presence = (0.16 + 0.62 * (1.0 - cover)) * (0.78 + 0.22 * cold);
        // Low cover still favors cirrus, but a genuinely clear SYNOPTIC lane
        // must remain visibly clear from orbit. The old inverse-cover law
        // made cover=0 the strongest global fine-speckle case (B-8). Pins
        // retain that exact law so established comparison reels do not move.
        let live_presence = legacy_presence * smoothstep(0.035, 0.22, cover);
        let presence = select(live_presence, legacy_presence, globals.weather7.w > 0.5);
        let alpha = clamp(wisps * presence * globals.weather7.z, 0.0, 1.0);
        return vec4<f32>(cloud_color(sdir, view_dir, kind, fabric, cover, precip), alpha);
    }

    // Low and middle shells share a broad mask. Different hit directions and
    // detail scales avoid duplication while storm cells align vertically.
    let broad_p = (sdir_w - globals.weather2.xyz) * (globals.weather6.x * 0.62)
        + cloud_seed_offset(7u);
    // The front no longer lifts the broad mask (that uniform fill was the
    // "slab" read); it lowers the formation threshold below instead, so
    // the band stays textured by real cells.
    let broad = clamp(vnoise(broad_p), 0.0, 1.0);
    if (kind == 1u) {
        let p = (sdir_w - globals.weather2.xyz) * globals.weather6.y
            + cloud_seed_offset(1u);
        let fabric = 0.52 * broad
            + 0.32 * vnoise(p)
            + 0.16 * vnoise(p * 2.03 + vec3<f32>(31.7, 17.3, 51.1));
        let threshold = 0.66 - 0.25 * cover - 0.05 * precip - 0.24 * arm_shift
            - 0.30 * structure.wall - 0.15 * front_t + 0.85 * structure.eye;
        let puffs = smoothstep(threshold, threshold + 0.12, fabric);
        let presence = smoothstep(0.10, 0.38, cover) * (0.86 + 0.14 * warm);
        let alpha = clamp(puffs * presence * globals.weather7.y, 0.0, 1.0);
        return vec4<f32>(cloud_color(sdir, view_dir, kind, fabric, cover, precip), alpha);
    }

    let p = (sdir_w - globals.weather2.xyz * 1.16) * globals.weather6.x
        + cloud_seed_offset(0u);
    let fabric = 0.68 * broad
        + 0.32 * vnoise(p * 1.73 + vec3<f32>(9.3, 41.7, 5.9));
    let threshold = 0.61 - 0.09 * cover - 0.13 * precip - 0.20 * arm_shift
        - 0.30 * structure.wall - 0.13 * front_t + 0.85 * structure.eye;
    let bases = smoothstep(threshold, threshold + 0.11, fabric);
    let storm_presence = max(
        smoothstep(0.12, 0.70, precip),
        0.38 * smoothstep(0.82, 0.98, cover),
    ) * (0.86 + 0.14 * warm);
    let alpha = clamp(bases * storm_presence * globals.weather7.x, 0.0, 1.0);
    return vec4<f32>(cloud_color(sdir, view_dir, kind, fabric, cover, precip), alpha);
}

fn cloud_layer_sample(sdir: vec3<f32>, view_dir: vec3<f32>, kind: u32) -> vec4<f32> {
    return cloud_layer_sample_structured(sdir, view_dir, kind, cloud_structure(sdir));
}

fn cloud_layer_on_ray(ray: vec3<f32>, altitude_km: f32, kind: u32) -> vec4<f32> {
    let hit = cloud_shell_hit(ray, altitude_km);
    if (hit.w < 0.0) {
        return vec4<f32>(0.0);
    }
    return cloud_layer_sample(hit.xyz, ray, kind);
}

fn cloud_layer_from_ground(
    rel: vec3<f32>,
    ray: vec3<f32>,
    altitude_km: f32,
    kind: u32,
) -> f32 {
    // General positive ray/sphere root: mountains can sit outside a low
    // shell, in which case an outward sun ray correctly finds no crossing.
    let wp = rel - globals.center.xyz;
    let r_ground = length(globals.center.xyz) - max(globals.weather8.w, 0.0);
    let r_shell = r_ground + altitude_km;
    let b = dot(wp, ray);
    let disc = b * b - (dot(wp, wp) - r_shell * r_shell);
    if (disc <= 0.0) {
        return 0.0;
    }
    let root = sqrt(disc);
    let near_t = -b - root;
    let far_t = -b + root;
    let t_hit = select(far_t, near_t, near_t > 0.0);
    if (t_hit <= 0.0) {
        return 0.0;
    }
    let sdir = normalize(wp + ray * t_hit);
    // Calling the visible layer's exact planet-direction function guarantees
    // shadows inherit every structured term without a camera/local proxy.
    return cloud_layer_sample(sdir, ray, kind).a;
}

fn cloud_shadow_at_ground(rel: vec3<f32>, sun: vec3<f32>) -> f32 {
    let count = u32(globals.weather5.z + 0.5);
    var transmittance = 1.0;
    if (count >= 3u) {
        transmittance *= 1.0 - cloud_layer_from_ground(
            rel, sun, globals.weather3.z, 0u,
        );
    }
    transmittance *= 1.0 - cloud_layer_from_ground(
        rel, sun, globals.weather5.x, 1u,
    );
    if (count >= 2u) {
        transmittance *= 1.0 - cloud_layer_from_ground(
            rel, sun, globals.weather5.y, 2u,
        );
    }
    return clamp(1.0 - transmittance, 0.0, 1.0);
}

fn cloud_over(acc: vec4<f32>, near_layer: vec4<f32>) -> vec4<f32> {
    let a = near_layer.a + acc.a * (1.0 - near_layer.a);
    if (a <= 1e-5) {
        return vec4<f32>(0.0);
    }
    let premul = near_layer.rgb * near_layer.a
        + acc.rgb * acc.a * (1.0 - near_layer.a);
    return vec4<f32>(premul / a, a);
}

fn cloud_orbit_composite(ray: vec3<f32>) -> vec4<f32> {
    let count = u32(globals.weather5.z + 0.5);
    var acc = vec4<f32>(0.0);
    // From orbit, low -> middle -> high is far-to-near.
    if (count >= 3u) {
        acc = cloud_over(acc, cloud_layer_on_ray(ray, globals.weather3.z, 0u));
    }
    acc = cloud_over(acc, cloud_layer_on_ray(ray, globals.weather5.x, 1u));
    if (count >= 2u) {
        acc = cloud_over(acc, cloud_layer_on_ray(ray, globals.weather5.y, 2u));
    }
    let handoff = smoothstep(globals.weather5.y, globals.weather3.w, globals.sky.w);
    // W3's 0.55 default is enforced after stacking, never per layer.
    acc.a = min(acc.a, globals.weather5.w) * handoff;
    return acc;
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
    // Sky/stars remain camera-based. Eclipse darkness uses the overlap at
    // the camera; terrain computes its own overlap per pixel below.
    let day_geom = smoothstep(-0.08, 0.15, sunh);
    let day = day_geom * (1.0 - globals.eclipse.x);
    // day gradient: bright horizon band to deeper zenith blue
    let zen = vec3<f32>(0.10, 0.28, 0.62);
    let hor = vec3<f32>(0.55, 0.70, 0.88);
    var c = mix(hor, zen, pow(clamp(h, 0.0, 1.0), 0.55)) * (0.03 + 0.97 * day);
    // heavy weather greys the whole vault, not just where the deck draws:
    // the blue between clouds desaturates and sinks toward overcast murk
    let wcover = globals.weather.x;
    let murk = wcover * (0.35 + 0.45 * globals.weather.y);
    c = mix(c, vec3<f32>(0.52, 0.56, 0.62) * (0.05 + 0.95 * day), murk * day);
    // W2 storm-edge sky: the first-order compass fit acts strongest at the
    // horizon, leaving the opposite sky visibly brighter. Camera anchoring
    // retires through the established wcam orbital fade.
    let wcam = 1.0 - smoothstep(8.0, 40.0, max(globals.weather8.w, 0.0));
    let storm_horizon = 1.0 - smoothstep(0.02, 0.58, max(h, 0.0));
    let storm_load = clamp(
        globals.weather11.w + dot(dir, globals.weather11.xyz),
        0.0,
        1.0,
    );
    let storm_gloom = storm_load * storm_horizon * globals.weather12.x * day * wcam;
    c *= 1.0 - 0.62 * storm_gloom;
    c = mix(
        c,
        vec3<f32>(0.37, 0.42, 0.29) * (0.12 + 0.88 * day),
        storm_gloom * globals.weather12.y,
    );
    // a low sun warms the sky around it
    let toward = pow(max(dot(dir, sun), 0.0), 6.0);
    let low = 1.0 - smoothstep(0.05, 0.35, sunh);
    c = mix(c, vec3<f32>(0.95, 0.55, 0.30), toward * low * 0.55 * day);
    // Physical Sun mesh supplies the disc; retain its established halo.
    let d = dot(dir, sun);
    c += globals.sun_tint.rgb
        * pow(max(d, 0.0), 800.0) * globals.eclipse.w * day;
    // below the horizon line, fade toward space-dark
    c *= smoothstep(-0.10, 0.03, h);
    // the atmosphere thins away with altitude: space is black
    let atmosphere = select(1.0, 0.0, globals.body_frame.w > 0.5);
    let atm = exp(-max(globals.sky.w, 0.0) / 45.0) * atmosphere;
    c *= atm;
    // W2 sky-side half of the height fog: when the eye is below the fog
    // ceiling, a soft horizontal veil meets the ground bank without drawing
    // geometry or storing state.
    if (globals.weather.w < 500.0 && globals.weather9.z > 0.0 && wcam > 0.0) {
        let humid = smoothstep(0.55, 0.88, globals.weather10.y);
        let pooled = smoothstep(0.04, 0.72, globals.weather10.z)
            * humid * globals.weather10.x;
        let mist_source = max(pooled, globals.weather10.w);
        let eye_above_ground = max(globals.weather8.w - globals.misc.w, 0.0);
        let below_ceiling = 1.0 - smoothstep(
            globals.weather9.w * 0.68,
            globals.weather9.w,
            eye_above_ground,
        );
        let horizontal = 1.0 - smoothstep(0.0, 0.22, abs(h));
        let veil = clamp(
            mist_source * globals.weather9.z * below_ceiling * horizontal * wcam,
            0.0,
            0.82,
        );
        let veil_col = mix(
            vec3<f32>(0.55, 0.62, 0.66),
            vec3<f32>(0.48, 0.51, 0.45),
            globals.weather.x * 0.35,
        ) * (0.18 + 0.82 * day);
        c = mix(c, veil_col, veil);
    }
    // limb glow: from orbit, rays that graze past the planet pass through
    // its atmosphere shell — a thin lit rim hugging the dark disc. The ray
    // is measured by its closest approach to the planet center; the glow
    // hugs the surface radius and shows only on the sunlit side.
    if (atm < 0.85) {
        let ctr = globals.center.xyz;
        let along = dot(ctr, dir);
        if (along > 0.0) {
            let r_planet = length(ctr) - max(globals.weather8.w, 0.0);
            let cp = dir * along; // closest point on the ray to the center
            let b = length(cp - ctr); // miss distance from planet center
            let n = normalize(cp - ctr);
            let lit = smoothstep(-0.15, 0.35, dot(n, sun));
            let shell = exp(-max(b - r_planet, 0.0) / 20.0)
                * smoothstep(r_planet * 0.6, r_planet, b);
            c += vec3<f32>(0.30, 0.50, 0.90) * shell * lit * (1.0 - atm) * 0.8;
        }
    }
    // Physical moon mesh supplies the disc; its atmospheric halo remains.
    let moon = globals.moon_dir.xyz;
    let moon_vis = (1.0 - 0.9 * day) * smoothstep(-0.06, 0.06, dot(moon, up));
    if (moon_vis > 0.001) {
        let dm = dot(dir, moon);
        let ang = acos(clamp(dm, -1.0, 1.0));
        let R = 0.0; // physical mesh owns the disc
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
    // ---- Clouds v2: three cheap concentric 2-D shells, composited
    // far-to-near. Different altitudes make head/camera motion produce real
    // parallax; weather selects cirrus, broken cumulus, or aligned storm
    // bases without a mutable simulation or volume march.
    let below_fade =
        1.0 - smoothstep(globals.weather5.y, globals.weather3.w, max(globals.sky.w, 0.0));
    if (globals.weather.w < 500.0 && below_fade > 0.003) {
        let count = u32(globals.weather5.z + 0.5);
        let horizon = smoothstep(-0.02, 0.08, h);
        let grazing = 1.0 + 0.55 * (1.0 - clamp(h, 0.0, 1.0));
        let visibility = horizon * below_fade * atm;
        var layer = vec4<f32>(0.0);
        // From below, high -> middle -> low is far-to-near.
        if (count >= 2u) {
            layer = cloud_layer_on_ray(dir, globals.weather5.y, 2u);
            layer.a = min(layer.a * grazing, 1.0) * visibility;
            c = mix(c, layer.rgb, layer.a);
        }
        layer = cloud_layer_on_ray(dir, globals.weather5.x, 1u);
        layer.a = min(layer.a * grazing, 1.0) * visibility;
        c = mix(c, layer.rgb, layer.a);
        if (count >= 3u) {
            layer = cloud_layer_on_ray(dir, globals.weather3.z, 0u);
            layer.a = min(layer.a * grazing, 1.0) * visibility;
            c = mix(c, layer.rgb, layer.a);
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

// ------------------------------------------------------ physical solar bodies

struct BodyIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct BodyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) local: vec3<f32>,
    @location(2) rel: vec3<f32>,
    @location(3) @interpolate(flat) kind: f32,
};

@vertex
fn vs_body(in: BodyIn) -> BodyOut {
    var out: BodyOut;
    let rel = tile.offset.xyz + in.position * tile.morph.x;
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = normalize(in.normal);
    out.local = in.position;
    out.rel = rel;
    out.kind = tile.offset.w;
    return out;
}

@fragment
fn fs_body(in: BodyOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    // The shared sphere is two-sided for robust winding at its pole seams;
    // retain only the camera-facing physical surface.
    if (dot(n, -normalize(in.rel)) <= 0.0) {
        discard;
    }
    let above = smoothstep(-0.08, 0.03, dot(globals.sun_dir.xyz, globals.sky.xyz));
    return vec4<f32>(globals.sun_tint.rgb * 3.2, above);
}

// ---------------------------------------------------- remote MP1 avatars

struct AvatarIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct AvatarOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_avatar(in: AvatarIn) -> AvatarOut {
    var out: AvatarOut;
    out.clip = globals.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = normalize(in.normal);
    out.color = in.color;
    return out;
}

@fragment
fn fs_avatar(in: AvatarOut) -> @location(0) vec4<f32> {
    let direct = max(dot(normalize(in.normal), globals.sun_dir.xyz), 0.0);
    let light = 0.42 + direct * 0.58;
    // Near-black label backplates retain their authored value; colored body
    // boxes receive the same simple directional read as the world.
    let is_backplate = select(0.0, 1.0, max(in.color.r, max(in.color.g, in.color.b)) < 0.05);
    let rgb = mix(in.color.rgb * light, in.color.rgb, is_backplate);
    return vec4<f32>(rgb, in.color.a);
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
// instance index, motion is a pure function of the (wrapping) weather clock,
// so captures reproduce exactly. Rain falls as wind-slanted streaks; snow
// drifts down as swaying flakes. The volume rides with the camera (W1;
// world-anchoring at f32 precision on a 6371 km sphere jitters worse than
// the ride-along reads — revisit in W2 with camera-local anchoring).
@vertex
fn vs_precip(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PrecipOut {
    var out: PrecipOut;
    let up = globals.sky.xyz;
    let t = globals.weather4.w;
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

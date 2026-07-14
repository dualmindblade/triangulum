//! wgpu renderer: pipeline setup, per-tile GPU buffers, frame drawing, and an
//! offscreen capture path (renders to a texture and saves a PNG — no window).

use crate::camera::Camera;
use crate::moon::{MoonGenerator, build_tile as build_moon_tile};
use crate::orbits::{BodyId, SolarState, SolarTuning};
use crate::planet::{Planet, boundary_shader_seedmul};
use crate::terrain::{TileKey, TileMesh, Vertex, build_tile_at_season, select_tiles};
use crate::voxel::{
    ChunkKey, Edits, LunarBody, SeasonalPlanet, Torches, VoxelBody, build_chunk, select_chunks,
};
use anyhow::Result;
use glam::{DQuat, DVec3, Mat4};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc;
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
const LIVE_TILE_PREFETCH_BUDGET: usize = 4;
const ASCENT_PREFETCH_MIN_LOOKAHEAD_KM: f64 = 10.0;
const ASCENT_PREFETCH_ALTITUDE_FACTOR: f64 = 4.0;

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
    // xyz = camera-to-physical-moon direction; w spare. Drives the halo and
    // cool moonlight lift while the mesh itself carries the visible disc.
    moon_dir: [f32; 4],
    // Physical body centers relative to the f64 camera, converted only here;
    // w = physical radius in km. Never send solar-system world coordinates
    // directly to f32 (SOLAR.md precision contract).
    sun_body: [f32; 4],
    moon_body: [f32; 4],
    // x = solar disc occlusion at the camera, y = lunar shadow fraction,
    // z = conservative whole-planet solar-contact gate, w = halo strength.
    eclipse: [f32; 4],
    sun_tint: [f32; 4],
    moon_tint: [f32; 4],
    moon_copper_tint: [f32; 4],
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
    // xyz = focused body center relative to camera; w = BodyId numeric id.
    body_frame: [f32; 4],
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
    // xyz = low/mid/high shell density multipliers; w = pinned-raster exact
    // compatibility path (uniform texture still uploads for instruments)
    weather7: [f32; 4],
    // cloud DECK scalar compatibility inputs: x/y exact cover/precip pins,
    // z temperature (C), w Neisor camera altitude. Live cover/precip SHAPE
    // comes from the synoptic raster, not these scalar lanes.
    weather8: [f32; 4],
    // W2 presentation knobs: x ground cloud-shadow strength, y orbital
    // cloud-alpha darkening, z fog density, w fog ceiling (km).
    weather9: [f32; 4],
    // W2 fog state: x dawn window, y humidity, z camera D-8 concavity,
    // w broad saturated-cover/precip fog bank.
    weather10: [f32; 4],
    // W2 storm-edge SH-1-ish field: xyz tangent gradient, w base load.
    weather11: [f32; 4],
    // W2 storm art direction: x directional strength, y warm/green cast.
    weather12: [f32; 4],
    // W-MOTION pass 2: x accumulated rigid base rotation, y bounded zonal
    // shear phase (slosh), z cyclone angular radius, w active bounded
    // system count.
    weather13: [f32; 4],
    // x unused (core wrap now rides per-system in cyclone_fronts.w),
    // y/z cover/precip boosts, w front strength.
    weather14: [f32; 4],
    // xy comma-tail front leading/trailing cross widths and z its outer
    // radial extent, all in cyclone-radius units; w spiral-arm strength.
    weather15: [f32; 4],
    // Spiral arms: x arm count, y log-spiral twist, z/w unused.
    weather16: [f32; 4],
    // Planet-frame moving centers + lifecycle-scaled intensity; fronts
    // carry only the hemisphere-signed bounded core wrap angle in w.
    cyclone_centers: [[f32; 4]; crate::weather::MAX_CYCLONES],
    cyclone_fronts: [[f32; 4]; crate::weather::MAX_CYCLONES],
    // x hemisphere-signed rotating arm-pattern phase (radians), y the
    // comma-tail front's base azimuth in the storm's polar frame.
    cyclone_arms: [[f32; 4]; crate::weather::MAX_CYCLONES],
    // premultiplied procedural seeds. xyz are the karst breach hint (V-10):
    // low 32 bits of (seed+K).wrapping_mul(0x9E37_79B1) for K = 40961
    // (region gate), 31337 (tube n1), 51413 (tube n2); w is the independent
    // clouds-v2 layout seed (+70001). Shader u32 hashes retain exact low
    // bits. (The range-biome comparator's octave-zero seedmul rides the
    // spare danchor_cell.w instead - both consumers wanted this slot.)
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BodyVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct AvatarVertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
}

/// Renderer-facing snapshot of an interpolated remote body-local pose. The
/// network module owns interpolation; the renderer owns the physical-body
/// transform and never receives a planet-global f32 coordinate.
#[derive(Clone, Debug)]
pub struct RemoteAvatar {
    pub name: String,
    pub body: BodyId,
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// Radial height above the nominal body radius, at eye level.
    pub alt_km: f64,
    pub yaw_deg: f64,
    pub tint: [f32; 3],
}

const MAX_REMOTE_AVATARS: usize = 16;
const MAX_AVATAR_VERTICES: usize = 131_072;

/// How many placed torches can light a frame at once (nearest win).
pub const MAX_LIGHTS: usize = 16;

fn torch_phase(face: u8, ci: u64, cj: u64) -> f64 {
    let mut x = ci ^ cj.rotate_left(21) ^ (face as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
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
    /// Frame this mesh last (re)entered the DRAWN set: fresh builds AND
    /// tiles released from an ancestor stand-in both ease from parent
    /// geometry (temporal morph) and dissolve their impostors in, instead
    /// of snapping - the two halves of the motion flicker. Refreshed in
    /// the draw loop whenever a tile was absent the previous frame;
    /// captures settle it out.
    shown_at: u64,
    /// Structural season snapshot used to build this geometry/material class.
    season_bucket: u32,
}

/// VRAM ceiling for cached chunk meshes: past it, least-recently-used chunks
/// are dropped regardless of age. Keep a full teleport's upload headroom
/// above the retained cache: eviction happens after streaming results land,
/// and the biome payload made that transient overlap large enough for the
/// 24-pose reel to exhaust smaller adapters at the old 1.5 GiB ceiling.
/// Current-frame chunks are exempt below, so --patch 2.0 remains complete.
const CHUNK_VRAM_BUDGET: u64 = 512 << 20;
/// Evict below the ceiling so a streamed batch has useful headroom before the
/// next LRU pass, instead of sorting after every individual chunk.
const CHUNK_VRAM_RETAIN: u64 = 384 << 20;
/// The tile cache gets the same byte discipline: its old 1500-count cap
/// allowed ~750 MiB once the biome payload grew vertices to 104 bytes, and
/// the 24-pose reel (24 teleports of ring accumulation) OOM'd smaller
/// adapters. Same current-frame exemption as chunks.
const TILE_VRAM_BUDGET: u64 = 512 << 20;
const TILE_VRAM_RETAIN: u64 = 384 << 20;
/// Absolute ceilings the anti-thrash recency protection may NOT exceed:
/// on a 6 GB adapter the caches plus swapchain/moon/app buffers must never
/// approach the physical limit (Andrew's in-game OOM crashes, 2026-07-12).
const TILE_VRAM_HARD: u64 = 1024 << 20;
const CHUNK_VRAM_HARD: u64 = 1024 << 20;
/// Moon tiles are much cheaper than voxel chunks but can accumulate during a
/// long low flyby.  Keep enough history for smooth reversals without letting
/// a circumnavigation grow without bound.
const MOON_VRAM_BUDGET: u64 = 384 << 20;

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    body_pipeline: wgpu::RenderPipeline,
    moon_pipeline: wgpu::RenderPipeline,
    precip_pipeline: wgpu::RenderPipeline,
    avatar_pipeline: wgpu::RenderPipeline,
    body_vertex_buf: wgpu::Buffer,
    body_index_buf: wgpu::Buffer,
    body_index_count: u32,
    avatar_vertex_buf: wgpu::Buffer,
    remote_avatars: Vec<RemoteAvatar>,
    bind_group: wgpu::BindGroup,
    globals_buf: wgpu::Buffer,
    tiles_buf: wgpu::Buffer,
    weather_raster_texture: wgpu::Texture,
    depth: wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
    cache: HashMap<TileKey, GpuTile>,
    moon_cache: HashMap<TileKey, GpuTile>,
    moon_generator: Arc<MoonGenerator>,
    moon_seed: i64,
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
    /// True while a (non-raw) capture settles the frame: temporal eases
    /// are zeroed so instruments stay byte-deterministic.
    settle_visuals: bool,
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
    /// unified-clock fast-forward (1.0 = real time); see weather_time_s
    time_scale: f64,
    time_anchor_abs_s: f64,
    time_anchor_render_s: f64,
    /// Weather is the same deterministic clock plus a seek offset. Replaying
    /// an absolute storm time also seeks the orbital clock; a separate daily
    /// rotation offset preserves exact photo-sidecar time-of-day restores.
    weather_time_offset_s: f64,
    day_time_offset_s: f64,
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
    chunk_tx: mpsc::Sender<(ChunkKey, u64, u32, TileMesh)>,
    chunk_rx: mpsc::Receiver<(ChunkKey, u64, u32, TileMesh)>,
    /// Async TILE builds, mirroring the chunk pipeline: LOD ring re-splits
    /// no longer build synchronously inside draw (the single-frame stall
    /// Andrew isolated with his zoom-only experiment). Selected-but-pending
    /// tiles draw a cached ancestor or their four cached children instead.
    tile_pending: HashMap<TileKey, u64>,
    tile_epoch: u64,
    /// Previous live-frame altitude, used only to ORDER the vertical
    /// prefetch forecasts (descent cover first when sinking). Scheduling
    /// state, never rendering state: the settle path ignores it.
    last_live_altitude_km: f64,
    tile_tx: mpsc::Sender<(TileKey, u64, u32, TileMesh)>,
    tile_rx: mpsc::Receiver<(TileKey, u64, u32, TileMesh)>,
    /// Immutable world-state snapshots shared by in-flight chunk builders.
    /// Refreshed only when edits/torches change, so queuing many chunks does
    /// not clone the whole edited world per request.
    edit_snapshot: Arc<Edits>,
    torch_snapshot: Arc<Torches>,
    /// Living weather (WEATHER.md). None = no weather.bin, sky stays clear.
    pub weather_field: Option<Arc<crate::weather::WeatherField>>,
    pub weather_tuning: crate::weather::WeatherTuning,
    /// Exact CPU bytes currently bound as the deck's spatial cover/precip
    /// field. The teleport map borrows this same raster.
    pub synoptic_raster: crate::weather::SynopticRaster,
    synoptic_raster_uploaded: bool,
    pub synoptic_raster_bakes: u64,
    pub synoptic_raster_last_bake_ms: f64,
    /// in-flight async live bake (LIVE frames never block on the ~4-10 ms
    /// bake: they draw the previous raster until the worker delivers -
    /// captures keep the synchronous deterministic path). Without this the
    /// bake ran inline on every 2 s weather boundary (a periodic hitch) and
    /// collapsed under time fast-forward, where 600x crosses hundreds of
    /// boundaries per real second (perf-reel catch: moon_orbit 3.3->7.8 ms).
    synoptic_raster_pending: Option<(
        crate::weather::SynopticRasterSource,
        std::sync::mpsc::Receiver<crate::weather::SynopticRaster>,
    )>,
    pub solar_tuning: SolarTuning,
    /// Master switch (--weather off, `weather off` in scripts).
    pub weather_on: bool,
    /// Some((cover, precip)) pins the sky for art shots and regression
    /// scripts, overriding the live field (like `sun` pins the sun).
    pub weather_pin: Option<(f32, f32)>,
    /// Last frame's camera weather sample — photo sidecars record it so a
    /// storm shot is a coordinate you can teleport back into.
    pub last_weather: crate::weather::Weather,
    /// Last frame's geometric eclipse factors, exposed to numeric play
    /// assertions and evidence sidecars.
    pub last_solar_occlusion: f64,
    pub last_lunar_shadow: f64,
}

/// Compact unit sphere for the physical Sun. The moon owns adaptive generated
/// tiles in P2, so no second placeholder surface law remains in the shader.
fn body_sphere_mesh() -> (Vec<BodyVertex>, Vec<u32>) {
    const LAT: u32 = 32;
    const LON: u32 = 64;
    let mut vertices = Vec::with_capacity(((LAT + 1) * (LON + 1)) as usize);
    for iy in 0..=LAT {
        let lat = -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * iy as f64 / LAT as f64;
        for ix in 0..=LON {
            let lon = std::f64::consts::TAU * ix as f64 / LON as f64;
            let n = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
            let normal = [n.x as f32, n.y as f32, n.z as f32];
            vertices.push(BodyVertex {
                position: normal,
                normal,
            });
        }
    }
    let mut indices = Vec::with_capacity((LAT * LON * 6) as usize);
    for iy in 0..LAT {
        for ix in 0..LON {
            let a = iy * (LON + 1) + ix;
            let b = a + LON + 1;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    (vertices, indices)
}

fn avatar_vertex(position: DVec3, normal: DVec3, color: [f32; 4], camera: DVec3) -> AvatarVertex {
    let relative = position - camera;
    AvatarVertex {
        position: [relative.x as f32, relative.y as f32, relative.z as f32],
        normal: [normal.x as f32, normal.y as f32, normal.z as f32],
        color,
    }
}

fn avatar_quad(
    out: &mut Vec<AvatarVertex>,
    points: [DVec3; 4],
    normal: DVec3,
    color: [f32; 4],
    camera: DVec3,
) {
    for index in [0usize, 1, 2, 0, 2, 3] {
        if out.len() >= MAX_AVATAR_VERTICES {
            return;
        }
        out.push(avatar_vertex(points[index], normal, color, camera));
    }
}

fn avatar_box(
    out: &mut Vec<AvatarVertex>,
    center: DVec3,
    right: DVec3,
    forward: DVec3,
    up: DVec3,
    color: [f32; 4],
    camera: DVec3,
) {
    let p = |r: f64, f: f64, u: f64| center + right * r + forward * f + up * u;
    avatar_quad(
        out,
        [
            p(-1.0, -1.0, -1.0),
            p(1.0, -1.0, -1.0),
            p(1.0, -1.0, 1.0),
            p(-1.0, -1.0, 1.0),
        ],
        -forward.normalize(),
        color,
        camera,
    );
    avatar_quad(
        out,
        [
            p(1.0, 1.0, -1.0),
            p(-1.0, 1.0, -1.0),
            p(-1.0, 1.0, 1.0),
            p(1.0, 1.0, 1.0),
        ],
        forward.normalize(),
        color,
        camera,
    );
    avatar_quad(
        out,
        [
            p(-1.0, 1.0, -1.0),
            p(-1.0, -1.0, -1.0),
            p(-1.0, -1.0, 1.0),
            p(-1.0, 1.0, 1.0),
        ],
        -right.normalize(),
        color,
        camera,
    );
    avatar_quad(
        out,
        [
            p(1.0, -1.0, -1.0),
            p(1.0, 1.0, -1.0),
            p(1.0, 1.0, 1.0),
            p(1.0, -1.0, 1.0),
        ],
        right.normalize(),
        color,
        camera,
    );
    avatar_quad(
        out,
        [
            p(-1.0, -1.0, 1.0),
            p(1.0, -1.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(-1.0, 1.0, 1.0),
        ],
        up.normalize(),
        color,
        camera,
    );
    avatar_quad(
        out,
        [
            p(-1.0, 1.0, -1.0),
            p(1.0, 1.0, -1.0),
            p(1.0, -1.0, -1.0),
            p(-1.0, -1.0, -1.0),
        ],
        -up.normalize(),
        color,
        camera,
    );
}

fn glyph_rows(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0f, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0f],
        'D' => [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0f, 0x10, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f],
        'J' => [0x01, 0x01, 0x01, 0x01, 0x11, 0x11, 0x0e],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0a],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x14, 0x04, 0x04, 0x04, 0x1f],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x10, 0x1e, 0x01, 0x01, 0x1e],
        '6' => [0x0e, 0x10, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x01, 0x0e],
        '-' => [0, 0, 0, 0x1f, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 0x1f],
        ' ' => [0; 7],
        _ => [0x0e, 0x11, 0x02, 0x04, 0x04, 0, 0x04],
    }
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
                    // FRAGMENT too: the impostor dissolve reads the tile's
                    // temporal ease (tile.morph.z) per fragment
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(32),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
        let weather_raster_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("synoptic weather raster"),
            size: wgpu::Extent3d {
                width: crate::weather::SYNOPTIC_RASTER_RES as u32,
                height: crate::weather::SYNOPTIC_RASTER_RES as u32,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let weather_raster_view =
            weather_raster_texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("synoptic weather raster array view"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        let weather_raster_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("synoptic weather bilinear sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &tiles_buf,
                        offset: 0,
                        size: wgpu::BufferSize::new(32),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&weather_raster_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&weather_raster_sampler),
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
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4, 5 => Unorm8x4, 6 => Unorm8x4, 7 => Unorm8x4, 8 => Unorm8x4, 9 => Unorm8x4, 10 => Unorm8x4, 11 => Unorm8x4, 12 => Unorm8x4, 13 => Unorm8x4],
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
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
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

        // The emissive Sun retains P1's compact sphere.  P2 moves the moon to
        // its own adaptive cube-sphere tiles below; both paths still use the
        // camera-relative dynamic Tile uniform and reversed-Z depth.
        let body_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("solar bodies"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_body"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<BodyVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_body"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let moon_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("adaptive moon terrain"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_moon"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4, 5 => Unorm8x4, 6 => Unorm8x4, 7 => Unorm8x4, 8 => Unorm8x4, 9 => Unorm8x4, 10 => Unorm8x4, 11 => Unorm8x4, 12 => Unorm8x4, 13 => Unorm8x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_moon"),
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
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let avatar_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("remote avatars"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_avatar"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<AvatarVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_avatar"),
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
                // Body boxes must self-occlude; glyph pixels sit 2 mm in
                // front of their label backplate so they remain stable too.
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let (body_vertices, body_indices) = body_sphere_mesh();
        let body_vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("solar body sphere vertices"),
            contents: bytemuck::cast_slice(&body_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let body_index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("solar body sphere indices"),
            contents: bytemuck::cast_slice(&body_indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let body_index_count = body_indices.len() as u32;
        let avatar_vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("remote avatar vertices"),
            size: (MAX_AVATAR_VERTICES * std::mem::size_of::<AvatarVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
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
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
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
            body_pipeline,
            moon_pipeline,
            precip_pipeline,
            avatar_pipeline,
            body_vertex_buf,
            body_index_buf,
            body_index_count,
            avatar_vertex_buf,
            remote_avatars: Vec::new(),
            bind_group,
            globals_buf,
            tiles_buf,
            weather_raster_texture,
            depth,
            size,
            format,
            cache: HashMap::new(),
            moon_cache: HashMap::new(),
            moon_generator: Arc::new(MoonGenerator::new(0)),
            moon_seed: 0,
            chunk_cache: HashMap::new(),
            chunk_stale: HashSet::new(),
            frame_counter: 0,
            tiles_deferred: false,
            settle_visuals: false,
            frame_mark: None,
            frame_intervals_ms: std::collections::VecDeque::new(),
            draw_cost_ms: std::collections::VecDeque::new(),
            exaggeration,
            sun_dir: None,
            underwater: false,
            torches: Torches::default(),
            day_len_s: SolarTuning::default().day_length_s,
            sun_ref_lon: 0.0,
            render_time_s: 0.0,
            time_scale: 1.0,
            time_anchor_abs_s: 0.0,
            time_anchor_render_s: 0.0,
            weather_time_offset_s: 0.0,
            day_time_offset_s: 0.0,
            patch_scale: 1.0,
            voxels_on: true,
            chunk_pending: HashMap::new(),
            chunk_epoch: 0,
            chunk_tx: tx,
            chunk_rx: rx,
            tile_pending: HashMap::new(),
            tile_epoch: 0,
            last_live_altitude_km: 0.0,
            tile_tx: ttx,
            tile_rx: trx,
            edit_snapshot: Arc::new(Edits::default()),
            torch_snapshot: Arc::new(Torches::default()),
            weather_field: None,
            weather_tuning: crate::weather::WeatherTuning::default(),
            synoptic_raster: crate::weather::SynopticRaster::off(),
            synoptic_raster_uploaded: false,
            synoptic_raster_bakes: 0,
            synoptic_raster_last_bake_ms: 0.0,
            synoptic_raster_pending: None,
            solar_tuning: SolarTuning::default(),
            weather_on: true,
            weather_pin: None,
            last_weather: crate::weather::Weather::default(),
            last_solar_occlusion: 0.0,
            last_lunar_shadow: 0.0,
        }
    }

    fn make_depth(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    fn upload_synoptic_raster(&mut self, raster: crate::weather::SynopticRaster) {
        let res = crate::weather::SYNOPTIC_RASTER_RES as u32;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.weather_raster_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            raster.bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(res * 4),
                rows_per_image: Some(res),
            },
            wgpu::Extent3d {
                width: res,
                height: res,
                depth_or_array_layers: 6,
            },
        );
        self.synoptic_raster = raster;
        self.synoptic_raster_uploaded = true;
        self.synoptic_raster_bakes = self.synoptic_raster_bakes.saturating_add(1);
    }

    fn refresh_synoptic_raster(&mut self, planet_arc: &Arc<Planet>, weather_time_s: f64) {
        let planet: &Planet = planet_arc;
        use crate::weather::{SynopticRaster, SynopticRasterSource};

        let live_time = crate::weather::synoptic_raster_time_s(weather_time_s);
        let desired = if !self.weather_on || self.weather_field.is_none() {
            SynopticRasterSource::Off
        } else if let Some((cover, precip)) = self.weather_pin {
            SynopticRasterSource::Pinned {
                seed: planet.seed,
                cover_bits: cover.to_bits(),
                precip_bits: precip.to_bits(),
            }
        } else {
            SynopticRasterSource::Live {
                seed: planet.seed,
                weather_time_bits: live_time.to_bits(),
            }
        };
        if self.synoptic_raster_uploaded && self.synoptic_raster.source() == desired {
            self.synoptic_raster_pending = None;
            return;
        }
        // LIVE frames bake off-thread; captures (settle_visuals) and the
        // pinned/off sources (trivially cheap) stay synchronous so every
        // instrument remains a pure function of weather time.
        let is_live = matches!(desired, SynopticRasterSource::Live { .. });
        if is_live && !self.settle_visuals {
            match &self.synoptic_raster_pending {
                Some((src, rx)) if *src == desired => {
                    if let Ok(raster) = rx.try_recv() {
                        self.upload_synoptic_raster(raster);
                        self.synoptic_raster_pending = None;
                    }
                    return; // draw with the previous raster meanwhile
                }
                _ => {}
            }
            let (tx, rx) = std::sync::mpsc::channel();
            let field = std::sync::Arc::clone(
                self.weather_field
                    .as_ref()
                    .expect("live source requires weather field"),
            );
            let planet = Arc::clone(planet_arc);
            let day_len = self.effective_day_len_s();
            let solar = self.solar_tuning.clone();
            let weather = self.weather_tuning.clone();
            rayon::spawn(move || {
                let raster = crate::weather::SynopticRaster::bake_live(
                    &field, &planet, live_time, day_len, &solar, &weather,
                );
                let _ = tx.send(raster);
            });
            self.synoptic_raster_pending = Some((desired, rx));
            return;
        }
        self.synoptic_raster_pending = None;

        let start = std::time::Instant::now();
        let raster = match desired {
            SynopticRasterSource::Off => SynopticRaster::off(),
            SynopticRasterSource::Pinned {
                cover_bits,
                precip_bits,
                ..
            } => SynopticRaster::pinned(
                planet.seed,
                f32::from_bits(cover_bits),
                f32::from_bits(precip_bits),
            ),
            SynopticRasterSource::Live { .. } => SynopticRaster::bake_live(
                self.weather_field
                    .as_deref()
                    .expect("live source requires weather field"),
                planet,
                live_time,
                self.effective_day_len_s(),
                &self.solar_tuning,
                &self.weather_tuning,
            ),
        };
        self.upload_synoptic_raster(raster);
        self.synoptic_raster_last_bake_ms = start.elapsed().as_secs_f64() * 1000.0;
        if std::env::var_os("TRI_WEATHER_RASTER_STATS").is_some() {
            eprintln!(
                "synoptic raster {:?}: {:.3} ms ({} bytes)",
                self.synoptic_raster.source(),
                self.synoptic_raster_last_bake_ms,
                self.synoptic_raster.bytes().len(),
            );
        }
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        if size.0 > 0 && size.1 > 0 {
            self.size = size;
            self.depth = Self::make_depth(&self.device, size);
        }
    }

    pub fn set_remote_avatars(&mut self, avatars: Vec<RemoteAvatar>) {
        self.remote_avatars = avatars.into_iter().take(MAX_REMOTE_AVATARS).collect();
    }

    fn build_avatar_vertices(
        &self,
        camera: &Camera,
        solar: SolarState,
        neisor_radius_km: f64,
    ) -> Vec<AvatarVertex> {
        let camera_pos = camera.position();
        let (_, label_up, label_right) = camera.view_basis();
        let moon_radius = self.solar_tuning.radius_km(BodyId::Moon, neisor_radius_km);
        let mut out = Vec::new();
        for avatar in &self.remote_avatars {
            let (body_center, radius) = match avatar.body {
                BodyId::Neisor => (DVec3::ZERO, neisor_radius_km),
                BodyId::Moon => (solar.moon_km, moon_radius),
                BodyId::Sun => continue,
            };
            if !avatar.lat_deg.is_finite()
                || !avatar.lon_deg.is_finite()
                || !avatar.alt_km.is_finite()
            {
                continue;
            }
            let lat = avatar.lat_deg.to_radians();
            let lon = avatar.lon_deg.to_radians();
            let up = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
            let eye = body_center + up * (radius + avatar.alt_km);
            let distance = (eye - camera_pos).length();
            // A metre-sized placeholder is not useful beyond this range. In
            // particular, a player on the moon is never reinterpreted as a
            // tiny Neisor-local offset when the observer is on Neisor.
            if !(0.00025..=10.0).contains(&distance) {
                continue;
            }
            let mut east = DVec3::Z.cross(up).normalize_or_zero();
            if east.length_squared() < 0.5 {
                east = DVec3::X;
            }
            let north = up.cross(east).normalize();
            let yaw = avatar.yaw_deg.to_radians();
            let forward = (north * yaw.cos() + east * yaw.sin()).normalize();
            let right = (north * (yaw + std::f64::consts::FRAC_PI_2).cos()
                + east * (yaw + std::f64::consts::FRAC_PI_2).sin())
            .normalize();
            let feet = eye - up * crate::player::EYE_KM;
            let color = [avatar.tint[0], avatar.tint[1], avatar.tint[2], 1.0];
            // Torso and head: deliberately simple MP1 block figure.
            avatar_box(
                &mut out,
                feet + up * 0.00072,
                right * 0.00032,
                forward * 0.00018,
                up * 0.00050,
                color,
                camera_pos,
            );
            avatar_box(
                &mut out,
                feet + up * 0.00151,
                right * 0.00023,
                forward * 0.00022,
                up * 0.00025,
                [
                    (avatar.tint[0] * 0.82 + 0.18).min(1.0),
                    (avatar.tint[1] * 0.82 + 0.18).min(1.0),
                    (avatar.tint[2] * 0.82 + 0.18).min(1.0),
                    1.0,
                ],
                camera_pos,
            );

            let chars = avatar.name.chars().take(12).collect::<Vec<_>>();
            if chars.is_empty() {
                continue;
            }
            let pixel = 0.00006;
            let label_width = chars.len() as f64 * 6.0 * pixel;
            let label_height = 7.0 * pixel;
            let anchor = eye + up * 0.00062;
            let facing = (camera_pos - anchor).normalize_or_zero();
            let half_r = label_right * (label_width * 0.5 + pixel * 0.65);
            let half_u = label_up * (label_height * 0.5 + pixel * 0.65);
            avatar_quad(
                &mut out,
                [
                    anchor - half_r - half_u,
                    anchor + half_r - half_u,
                    anchor + half_r + half_u,
                    anchor - half_r + half_u,
                ],
                facing,
                [0.005, 0.007, 0.012, 0.80],
                camera_pos,
            );
            let left = -label_width * 0.5;
            let top = label_height * 0.5;
            for (char_index, ch) in chars.into_iter().enumerate() {
                for (row, bits) in glyph_rows(ch).into_iter().enumerate() {
                    for col in 0..5 {
                        if bits & (1 << (4 - col)) == 0 {
                            continue;
                        }
                        let x = left + (char_index as f64 * 6.0 + col as f64 + 0.5) * pixel;
                        let y = top - (row as f64 + 0.5) * pixel;
                        let center = anchor + label_right * x + label_up * y + facing * 0.000002;
                        let hr = label_right * pixel * 0.43;
                        let hu = label_up * pixel * 0.43;
                        avatar_quad(
                            &mut out,
                            [
                                center - hr - hu,
                                center + hr - hu,
                                center + hr + hu,
                                center - hr + hu,
                            ],
                            facing,
                            [0.96, 0.98, 1.0, 1.0],
                            camera_pos,
                        );
                    }
                }
            }
        }
        out.truncate(MAX_AVATAR_VERTICES);
        out
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
    /// time_scale fast-forwards the WHOLE unified clock (sun, seasons,
    /// weather, orbits) relative to the anchor frozen when the scale was set;
    /// scale 1 with a zero anchor reduces to the plain offset form.
    pub fn weather_time_s(&self) -> f64 {
        self.time_anchor_abs_s
            + (self.render_time_s - self.time_anchor_render_s) * self.time_scale
            + self.weather_time_offset_s
    }

    pub fn time_scale(&self) -> f64 {
        self.time_scale
    }

    /// Change the clock rate WITHOUT jumping: the current absolute time is
    /// folded into the anchor so the seam is continuous, then time advances
    /// at the new rate. Pure function of (render clock, anchors) afterward -
    /// the play harness never touches this, so every instrument stays exact.
    pub fn set_time_scale(&mut self, scale: f64) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        let now_abs = self.weather_time_s();
        self.time_anchor_abs_s = now_abs - self.weather_time_offset_s;
        self.time_anchor_render_s = self.render_time_s;
        self.time_scale = scale;
    }

    pub fn effective_day_len_s(&self) -> f64 {
        if self.day_len_s.is_finite() && self.day_len_s > 0.0 {
            self.day_len_s
        } else {
            self.solar_tuning.day_length_s
        }
    }

    /// Seek the absolute weather/orbit coordinate. The offset is retained as
    /// simulation time advances, so restored fronts and bodies keep moving.
    pub fn set_weather_time_s(&mut self, t_s: f64) {
        if t_s.is_finite() {
            let scaled_base = self.time_anchor_abs_s
                + (self.render_time_s - self.time_anchor_render_s) * self.time_scale;
            self.weather_time_offset_s = t_s.max(0.0) - scaled_base;
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

    /// A join/disconnect swaps the entire mutable world rather than one
    /// column. Drop resident chunks so no mesh from the previous journal can
    /// survive the state boundary.
    pub fn replace_world_snapshot(&mut self, edits: &Edits) {
        self.refresh_edits_snapshot(edits);
        self.chunk_epoch = self.chunk_epoch.wrapping_add(1);
        self.chunk_pending.clear();
        self.chunk_stale.clear();
        self.chunk_cache.clear();
    }

    pub fn set_torches(&mut self, torches: Torches) {
        self.torch_snapshot = Arc::new(torches.clone());
        self.torches = torches;
    }

    /// Physical hierarchy in the rotating Neisor terrain frame. A pinned sun
    /// is an explicit art/repro override: rotate the complete frame (including
    /// the moon), rather than divorcing the visible disc from its body.
    pub fn solar_state(&self, cam_pos: DVec3, neisor_radius_km: f64) -> SolarState {
        let absolute_t_s = self.weather_time_s();
        let rotation_t_s = absolute_t_s + self.day_time_offset_s;
        let mut state = crate::orbits::state_at_with_day_length(
            &self.solar_tuning,
            absolute_t_s,
            rotation_t_s,
            self.effective_day_len_s(),
            neisor_radius_km,
        );
        state = state.rotated_about_neisor(DQuat::from_rotation_z(self.sun_ref_lon));
        let target = self.sun_dir.or_else(|| {
            // Legacy --day-len 0 remains an explicit local-noon mode. The
            // bodies stay physical relative to one another; the whole frame
            // follows the observer just as a pinned art frame does.
            (self.day_len_s <= 0.0).then(|| cam_pos.normalize())
        });
        if let Some(target) = target.filter(|v| v.length_squared() > 0.5) {
            // Rotate about NEISOR'S CENTER, not the observer: an observer
            // rotation moves the sun and moon while Neisor stays fixed at
            // the origin, so the sun-Neisor-moon geometry distorts and a
            // pinned sky can drag the moon into a shadow the real state
            // never had (the sky reel's t=0 moon pose measured
            // lunar_shadow 1.0 while the unpinned suite asserts ~0). An
            // origin rotation keeps eclipse geometry exactly physical; the
            // pinned direction as seen from the camera differs from the
            // target only by solar parallax over one planet radius
            // (~0.005 deg, far below the discs' angular sizes).
            let from = state.sun_km.normalize();
            state = state.rotated_about_neisor(DQuat::from_rotation_arc(from, target.normalize()));
        }
        state
    }

    pub fn sun_state(&self, cam_pos: DVec3, neisor_radius_km: f64) -> SunState {
        let state = self.solar_state(cam_pos, neisor_radius_km);
        SunState {
            dir: state.sun_km.normalize(),
            day_time_s: if self.day_len_s > 0.0 {
                (self.weather_time_s() + self.day_time_offset_s).rem_euclid(self.day_len_s)
            } else {
                0.0
            },
        }
    }

    /// Jump the day/night cycle so the CURRENT moment sits `t_s` seconds
    /// into the day — the photo map's optional "restore time of day".
    /// No-op when the sun is pinned or the cycle is off. Absolute weather and
    /// orbit time are preserved; only the daily body rotation gets an offset.
    pub fn set_day_time_s(&mut self, t_s: f64) {
        if self.day_len_s > 0.0 && self.sun_dir.is_none() {
            self.day_time_offset_s = t_s.rem_euclid(self.day_len_s) - self.weather_time_s();
        }
    }

    pub fn set_season_frac(&mut self, target: f64) {
        let absolute_t_s = self.weather_time_s();
        let day_len_s = self.effective_day_len_s();
        self.solar_tuning
            .set_season_frac(absolute_t_s, day_len_s, target);
    }

    /// One immutable W4 input derived from the already-unified orbital clock.
    /// Weather-off deliberately selects the byte-compatible annual law.
    pub fn structural_season(&self, planet: &Planet) -> crate::weather::StructuralSeason {
        if self.weather_on && planet.weather.is_some() {
            crate::weather::StructuralSeason::quantized(
                self.solar_tuning
                    .season_frac(self.weather_time_s(), self.effective_day_len_s()),
                &self.weather_tuning,
            )
        } else {
            crate::weather::StructuralSeason::annual()
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
        let mut landed = 0usize;
        while let Ok((k, epoch, bucket, mesh)) = self.chunk_rx.try_recv() {
            if self.chunk_pending.get(&k) == Some(&epoch) {
                self.chunk_pending.remove(&k);
                let gpu = self.upload(mesh, bucket);
                // re-uploads of a live key (edits, refreshes) keep their
                // ease state - only genuinely new arrivals rise/dissolve
                let mut gpu = gpu;
                if let Some(old) = self.chunk_cache.get(&k) {
                    gpu.shown_at = old.shown_at;
                }
                self.chunk_cache.insert(k, gpu);
                landed += 1;
                if landed.is_multiple_of(16) {
                    self.enforce_chunk_budget();
                }
            }
        }
        if landed > 0 {
            self.enforce_chunk_budget();
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
        let cam_local = camera.local_position();
        // Preserve the established body-relative landed patch footprint and
        // its two-regime mesh offset/handoff. With the moon's own 0.27-scale
        // lattice this now also means ~13.7x fewer columns/chunks than the old
        // shared-lattice path inside that unchanged lunar patch.
        let body_patch_ratio = if camera.body == BodyId::Moon {
            (camera.radius_km / planet.radius_km).clamp(0.05, 1.0)
        } else {
            1.0
        };
        let voxel_radius_m = (200.0 + (VOXEL_MAX_ALT_KM - camera.altitude_km).max(0.0) * 120.0)
            * self.patch_scale
            * body_patch_ratio;
        let focus = (camera.body == BodyId::Neisor && camera.altitude_km < VOXEL_MAX_ALT_KM)
            .then(|| (camera.local_direction(), voxel_radius_m / 1000.0 + 0.2));
        let keys = select_tiles(cam_pos, planet.radius_km, ERR_TARGET, focus);

        // upload globals (camera-relative view-projection, f64 -> f32 at the end)
        let t_s = self.render_time_s;
        let weather_t_s = self.weather_time_s();
        self.refresh_synoptic_raster(planet, weather_t_s);
        let structural_season = self.structural_season(planet);
        let solar = self.solar_state(cam_pos, planet.radius_km);
        let sun_rel = solar.sun_km - cam_pos;
        let moon_rel = solar.moon_km - cam_pos;
        let sun = sun_rel.normalize_or_zero();
        let moon = moon_rel.normalize_or_zero();
        let look = camera.look_dir();
        let body_visible = |center: DVec3, radius: f64| {
            let distance = center.length();
            if distance <= radius {
                return true;
            }
            let angular_radius = (radius / distance).clamp(0.0, 1.0).asin();
            // 65 deg vertical FOV and widescreen horizontal FOV fit inside a
            // conservative 70 deg cone. Include the body's own angular size.
            look.dot(center / distance)
                >= (70f64.to_radians() + angular_radius)
                    .min(std::f64::consts::PI)
                    .cos()
        };
        let moon_radius = self.solar_tuning.radius_km(BodyId::Moon, planet.radius_km);
        // Landed frames are draw-call bound by distant LOD rings long after
        // the voxel patch and its max-level rim tiles are settled. Relax only
        // those distant rings while within the voxel regime; the patch rim
        // still reaches MAX_LEVEL and samples the identical moon law.
        let moon_lod_error = if camera.body == BodyId::Moon && camera.altitude_km < VOXEL_MAX_ALT_KM
        {
            0.25
        } else {
            crate::moon::tuning::LOD_ERROR_TARGET
        };
        if self.moon_seed != planet.seed {
            self.moon_seed = planet.seed;
            self.moon_generator = Arc::new(MoonGenerator::new(planet.seed));
            self.moon_cache.clear();
        }
        let moon_focus = (camera.body == BodyId::Moon && camera.altitude_km < VOXEL_MAX_ALT_KM)
            .then(|| (camera.local_direction(), voxel_radius_m / 1000.0 + 0.2));
        let moon_keys = if body_visible(moon_rel, moon_radius) {
            // Selection is body-local f64.  The generic cube-sphere quadtree
            // does not know or care whether the center is Neisor or the moon.
            select_tiles(
                cam_pos - solar.moon_km,
                moon_radius,
                moon_lod_error,
                moon_focus,
            )
        } else {
            Vec::new()
        };
        let missing_moon: Vec<TileKey> = moon_keys
            .iter()
            .filter(|key| !self.moon_cache.contains_key(key))
            .copied()
            .collect();
        if !missing_moon.is_empty() {
            use rayon::prelude::*;
            let generator = Arc::clone(&self.moon_generator);
            let built: Vec<(TileKey, TileMesh)> = missing_moon
                .par_iter()
                .map(|key| (*key, build_moon_tile(&generator, *key, moon_radius)))
                .collect();
            for (key, mesh) in built {
                let gpu = self.upload(mesh, crate::weather::StructuralSeason::ANNUAL_BUCKET);
                // re-uploads of a live key (edits, refreshes) keep their
                // ease state - only genuinely new arrivals rise/dissolve
                let mut gpu = gpu;
                if let Some(old) = self.moon_cache.get(&key) {
                    gpu.shown_at = old.shown_at;
                }
                self.moon_cache.insert(key, gpu);
            }
        }
        let voxel_body: Option<Arc<dyn VoxelBody>> = match camera.body {
            BodyId::Neisor => {
                let body: Arc<dyn VoxelBody> =
                    Arc::new(SeasonalPlanet::new(Arc::clone(planet), structural_season));
                Some(body)
            }
            BodyId::Moon => Some(Arc::new(LunarBody::new(
                moon_radius,
                Arc::clone(&self.moon_generator),
            ))),
            BodyId::Sun => None,
        };
        let surface_altitude = crate::camera::nearest_surface_altitude_km(
            camera,
            solar,
            &self.solar_tuning,
            planet.radius_km,
        );
        let vp = camera.view_proj_for_surface_altitude(
            self.size.0 as f64 / self.size.1 as f64,
            surface_altitude,
        );
        let vp32 = Mat4::from_cols_array(&vp.to_cols_array().map(|x| x as f32));
        let solar_occlusion =
            crate::orbits::solar_occlusion_at(cam_pos, solar, &self.solar_tuning, planet.radius_km);
        let lunar_shadow =
            crate::orbits::lunar_shadow_fraction(solar, &self.solar_tuning, planet.radius_km);
        let solar_contact_possible =
            crate::orbits::solar_contact_possible(solar, &self.solar_tuning, planet.radius_km);
        self.last_solar_occlusion = solar_occlusion;
        self.last_lunar_shadow = lunar_shadow;
        let inv32 = Mat4::from_cols_array(&vp.inverse().to_cols_array().map(|x| x as f32));
        let up = camera.local_direction();
        let cam_h_km = (cam_local.length() - camera.radius_km).max(0.0);
        let neisor_cam_h_km = if camera.body == BodyId::Neisor {
            cam_h_km
        } else {
            (cam_pos.length() - planet.radius_km).max(0.0)
        };
        let lift = crate::voxel::lift_km(self.exaggeration);

        // placed torches: the nearest MAX_LIGHTS become point lights. Their
        // exact height needs a terrain sample, so rank cheaply by direction
        // first and only sample the winners.
        let mut lights = [[0.0f32; 4]; MAX_LIGHTS];
        let mut n_lights = 0usize;
        if camera.body == BodyId::Neisor
            && camera.altitude_km < VOXEL_MAX_ALT_KM
            && !self.torches.is_empty()
        {
            // Torches are Neisor-only and retain the original lattice.
            let nn = crate::voxel::COLUMNS_PER_FACE as f64;
            let mut ranked: Vec<(f64, (u8, u64, u64), DVec3)> = self
                .torches
                .iter()
                .map(|&(f, ci, cj)| {
                    let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nn;
                    let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nn;
                    let dir = crate::planet::face_dir(f as usize, u, v);
                    (
                        (dir * cam_pos.length() - cam_pos).length_squared(),
                        (f, ci, cj),
                        dir,
                    )
                })
                .collect();
            ranked.sort_by(|a, b| a.0.total_cmp(&b.0));
            for &(_, (f, ci, cj), dir) in ranked.iter().take(MAX_LIGHTS) {
                let top = crate::voxel::surface_height_km(
                    voxel_body
                        .as_ref()
                        .expect("Neisor has a voxel body")
                        .as_ref(),
                    edits,
                    dir,
                    self.exaggeration,
                );
                let pos = dir
                    * (planet.radius_km + top + 0.55 * crate::voxel::VOXEL_KM * self.exaggeration)
                    - cam_pos;
                // each flame breathes on its own phase
                let flicker = (0.88 + 0.18 * (t_s * 9.0 + torch_phase(f, ci, cj)).sin()) as f32;
                lights[n_lights] = [pos.x as f32, pos.y as f32, pos.z as f32, flicker];
                n_lights += 1;
            }
        }
        let exagg = self.exaggeration;
        // build missing tiles in parallel (rayon), then upload sequentially
        // accept tile builds that finished on background threads
        while let Ok((k, epoch, bucket, mesh)) = self.tile_rx.try_recv() {
            if self.tile_pending.get(&k) == Some(&epoch) {
                self.tile_pending.remove(&k);
                if bucket == structural_season.bucket {
                    let gpu = self.upload(mesh, bucket);
                    // re-uploads of a live key (edits, refreshes) keep their
                    // ease state - only genuinely new arrivals rise/dissolve
                    let mut gpu = gpu;
                    if let Some(old) = self.cache.get(&k) {
                        gpu.shown_at = old.shown_at;
                    }
                    // re-uploads of a live key (edits, refreshes) keep their
                    // ease state - only genuinely new arrivals rise/dissolve
                    let mut gpu = gpu;
                    if let Some(old) = self.cache.get(&k) {
                        gpu.shown_at = old.shown_at;
                    }
                    self.cache.insert(k, gpu);
                }
            }
        }
        // A season-bucket change never drops the visible tile set. Selected
        // stale tiles keep drawing while replacements stream in, avoiding a
        // synchronized world rebuild at mid-season.
        let stale_tiles: Vec<TileKey> = keys
            .iter()
            .filter(|key| {
                self.cache
                    .get(key)
                    .is_some_and(|tile| tile.season_bucket != structural_season.bucket)
            })
            .copied()
            .collect();
        if self.settle_visuals && !stale_tiles.is_empty() {
            // A capture is an instrument, not a live transition: build the
            // complete selected season snapshot as one deterministic batch.
            // Iterating the eight-per-frame live throttle can never converge
            // after a long teleport reel if the cache budget evicts an old
            // stand-in for each replacement.
            use rayon::prelude::*;
            let built: Vec<(TileKey, TileMesh)> = stale_tiles
                .par_iter()
                .map(|key| {
                    (
                        *key,
                        build_tile_at_season(planet, *key, exagg, structural_season),
                    )
                })
                .collect();
            for (key, mesh) in built {
                let mut gpu = self.upload(mesh, structural_season.bucket);
                if let Some(old) = self.cache.get(&key) {
                    gpu.shown_at = old.shown_at;
                }
                self.cache.insert(key, gpu);
                self.tile_pending.remove(&key);
            }
        } else {
            for key in stale_tiles.into_iter().take(8) {
                if self.tile_pending.contains_key(&key) {
                    continue;
                }
                let epoch = self.tile_epoch;
                self.tile_epoch = self.tile_epoch.wrapping_add(1);
                self.tile_pending.insert(key, epoch);
                let tx = self.tile_tx.clone();
                let planet = Arc::clone(planet);
                let season = structural_season;
                rayon::spawn(move || {
                    let mesh = build_tile_at_season(&planet, key, exagg, season);
                    let _ = tx.send((key, epoch, season.bucket, mesh));
                });
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
                if (0..4).all(|i| self_cache_has(cache, child(i % 2, i / 2))) {
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
            if !self.settle_visuals && !k.deep && covered(&self.cache, k) {
                if !self.tile_pending.contains_key(k) {
                    let epoch = self.tile_epoch;
                    self.tile_epoch = self.tile_epoch.wrapping_add(1);
                    self.tile_pending.insert(*k, epoch);
                    let tx = self.tile_tx.clone();
                    let planet = Arc::clone(planet);
                    let key = *k;
                    let season = structural_season;
                    rayon::spawn(move || {
                        let mesh = build_tile_at_season(&planet, key, exagg, season);
                        let _ = tx.send((key, epoch, season.bucket, mesh));
                    });
                }
            } else {
                urgent.push(*k);
            }
        }
        let built: Vec<(TileKey, TileMesh)> = {
            use rayon::prelude::*;
            urgent
                .par_iter()
                .map(|k| {
                    (
                        *k,
                        build_tile_at_season(planet, *k, exagg, structural_season),
                    )
                })
                .collect()
        };
        for (k, mesh) in built {
            let gpu = self.upload(mesh, structural_season.bucket);
            // re-uploads of a live key (edits, refreshes) keep their
            // ease state - only genuinely new arrivals rise/dissolve
            let mut gpu = gpu;
            if let Some(old) = self.cache.get(&k) {
                gpu.shown_at = old.shown_at;
            }
            self.cache.insert(k, gpu);
        }
        self.tiles_deferred = keys.iter().any(|key| {
            self.cache
                .get(key)
                .is_none_or(|tile| tile.season_bucket != structural_season.bucket)
        });
        // VERTICAL + parent PREFETCH: immediate parents cover slow
        // coarsening, but fast vertical motion reveals horizon/LOD nodes
        // that are not parents of the current selected set. Forecast the
        // deterministic future cover at a bounded altitude lookahead in
        // BOTH directions - the direction the camera is actually moving
        // first - and spend the same four-tile live budget there. Ascent
        // alone left descents hitting the fat fine-LOD ring synchronously
        // (B-4b: a 2.2 s frame at exactly 2 km, Austin's consistent
        // "2-3 km descent lag"). A hovering camera warms both covers once
        // and then this is all cache hits.
        let mut prefetched = 0usize;
        let mut live_budget = LIVE_TILE_PREFETCH_BUDGET;
        if !self.settle_visuals && camera.body == BodyId::Neisor {
            let altitude = camera.altitude_km;
            let vertical_delta = altitude - self.last_live_altitude_km;
            self.last_live_altitude_km = altitude;
            // Real vertical motion earns a doubled budget: the fine covers
            // are many expensive tiles, and this is exactly the moment the
            // background pool should saturate. Hovering keeps the steady
            // budget (its forecasts are cache hits anyway).
            if vertical_delta.abs() > 0.005 {
                live_budget = LIVE_TILE_PREFETCH_BUDGET * 2;
            }
            let ascent_altitude = (altitude + ASCENT_PREFETCH_MIN_LOOKAHEAD_KM)
                .max(altitude * ASCENT_PREFETCH_ALTITUDE_FACTOR);
            // Descent forecasts two bands: alt/2 is imminent, alt/4 gives
            // the slowest tiles (B-4a class, 300-900 ms builds) their lead
            // time. Nearest-term first while sinking.
            let descent_near = (altitude / 2.0).max(0.03);
            let descent_far = (altitude / ASCENT_PREFETCH_ALTITUDE_FACTOR).max(0.03);
            let forecasts = if vertical_delta < 0.0 {
                [descent_near, descent_far, ascent_altitude]
            } else {
                [ascent_altitude, descent_near, descent_far]
            };
            'forecast: for target in forecasts {
                let pos = camera.local_direction() * (planet.radius_km + target);
                for key in select_tiles(pos, planet.radius_km, ERR_TARGET, None) {
                    if prefetched >= live_budget {
                        break 'forecast;
                    }
                    if self.cache.contains_key(&key) || self.tile_pending.contains_key(&key) {
                        continue;
                    }
                    let epoch = self.tile_epoch;
                    self.tile_epoch = self.tile_epoch.wrapping_add(1);
                    self.tile_pending.insert(key, epoch);
                    let tx = self.tile_tx.clone();
                    let planet = Arc::clone(planet);
                    let season = structural_season;
                    rayon::spawn(move || {
                        let mesh = build_tile_at_season(&planet, key, exagg, season);
                        let _ = tx.send((key, epoch, season.bucket, mesh));
                    });
                    prefetched += 1;
                }
            }
        }
        // The original immediate-parent fallback remains useful once the
        // forecast set is warm, and for camera states where ascent lookahead
        // is not applicable.
        for k in &keys {
            if self.settle_visuals {
                break;
            }
            if prefetched >= live_budget {
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
            if !self.cache.contains_key(&parent) && !self.tile_pending.contains_key(&parent) {
                let epoch = self.tile_epoch;
                self.tile_epoch = self.tile_epoch.wrapping_add(1);
                self.tile_pending.insert(parent, epoch);
                let tx = self.tile_tx.clone();
                let planet = Arc::clone(planet);
                let season = structural_season;
                rayon::spawn(move || {
                    let mesh = build_tile_at_season(&planet, parent, exagg, season);
                    let _ = tx.send((parent, epoch, season.bucket, mesh));
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
                    .map(|k| {
                        (
                            *k,
                            build_tile_at_season(planet, *k, exagg, structural_season),
                        )
                    })
                    .collect();
                for (k, mesh) in built {
                    let gpu = self.upload(mesh, structural_season.bucket);
                    // re-uploads of a live key (edits, refreshes) keep their
                    // ease state - only genuinely new arrivals rise/dissolve
                    let mut gpu = gpu;
                    if let Some(old) = self.cache.get(&k) {
                        gpu.shown_at = old.shown_at;
                    }
                    // re-uploads of a live key (edits, refreshes) keep their
                    // ease state - only genuinely new arrivals rise/dissolve
                    let mut gpu = gpu;
                    if let Some(old) = self.cache.get(&k) {
                        gpu.shown_at = old.shown_at;
                    }
                    self.cache.insert(k, gpu);
                }
            }
            // ANCESTOR STAND-INS SUPPRESS THEIR DRAWN DESCENDANTS: when a
            // parent stands in for one missing child, its three cached
            // siblings still made the list - the parent's full quad then
            // overlapped them (z-fighting mesh, DOUBLE impostor trees) for
            // as long as the build stayed pending. One slow build used to
            // be a one-frame blink; with heavier tiles it became Andrew's
            // motion flicker (2026-07-12). Render the whole family at the
            // ancestor's level instead, coherently, until the child lands.
            let chosen: HashSet<TileKey> = resolved.iter().copied().collect();
            let has_live_ancestor = |k: &TileKey| -> bool {
                let mut cur = *k;
                while cur.level > 0 {
                    cur = TileKey {
                        face: cur.face,
                        level: cur.level - 1,
                        ix: cur.ix / 2,
                        iy: cur.iy / 2,
                        deep: false,
                    };
                    if chosen.contains(&cur) {
                        return true;
                    }
                }
                false
            };
            resolved.retain(|k| !has_live_ancestor(k));
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
        if self.voxels_on
            && camera.altitude_km < VOXEL_MAX_ALT_KM
            && let Some(body) = voxel_body.as_ref()
        {
            self.drain_chunks();
            chunk_keys = select_chunks(cam_local, body.as_ref(), voxel_radius_m);
            let center = camera.local_direction() * body.radius_km();
            let nn = body.columns_per_face() as f64;
            let mut seasonal_refreshes = 0usize;
            for k in &chunk_keys {
                let cached = self.chunk_cache.contains_key(k);
                let edit_stale = self.chunk_stale.contains(k);
                let season_stale = self
                    .chunk_cache
                    .get(k)
                    .is_some_and(|chunk| chunk.season_bucket != structural_season.bucket);
                let stale = edit_stale || season_stale;
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
                        unbuilt_min_km.min((cdir * body.radius_km() - center).length());
                }
                if !self.chunk_pending.contains_key(k) {
                    if season_stale && !edit_stale {
                        if seasonal_refreshes >= 16 {
                            continue;
                        }
                        seasonal_refreshes += 1;
                    }
                    let epoch = self.chunk_epoch;
                    self.chunk_epoch = self.chunk_epoch.wrapping_add(1);
                    self.chunk_pending.insert(*k, epoch);
                    self.chunk_stale.remove(k); // rebuilt with the current edits
                    let tx = self.chunk_tx.clone();
                    let body = Arc::clone(body);
                    let edits = Arc::clone(&self.edit_snapshot);
                    let key = *k;
                    let torches = if key.body == BodyId::Neisor {
                        Arc::clone(&self.torch_snapshot)
                    } else {
                        Arc::new(Torches::default())
                    };
                    let season_bucket = structural_season.bucket;
                    rayon::spawn(move || {
                        let mesh = build_chunk(
                            body.as_ref(),
                            edits.as_ref(),
                            torches.as_ref(),
                            key,
                            exagg,
                        );
                        let _ = tx.send((key, epoch, season_bucket, mesh));
                    });
                }
            }
            self.tiles_deferred |= chunk_keys.iter().any(|key| {
                self.chunk_pending.contains_key(key)
                    || self
                        .chunk_cache
                        .get(key)
                        .is_some_and(|chunk| chunk.season_bucket != structural_season.bucket)
            });
            chunk_keys.retain(|k| self.chunk_cache.contains_key(k));
        }

        // the heightfield hole: never larger than the classic safe radius,
        // and never reaching past the nearest chunk that isn't built yet
        // (minus a chunk's worth of margin). Fully built -> full hole;
        // freshly teleported -> no hole, mesh shows while blocks stream in.
        let mut hole = [0.0f32; 4];
        let mut hole_up = [0.0f32; 4];
        if voxel_body.is_some() && camera.altitude_km < VOXEL_MAX_ALT_KM && self.voxels_on {
            let r_km = crate::voxel::safe_hole_radius_km(voxel_radius_m)
                .min((unbuilt_min_km - 0.096).max(0.0));
            // center + up are set whenever the patch exists (the rim-sink
            // needs them even while the hole itself is still closed)
            let hup = camera.local_direction();
            let hc = -hup * camera.altitude_km; // camera ground point, camera-relative
            hole = [hc.x as f32, hc.y as f32, hc.z as f32, r_km.max(0.0) as f32];
            hole_up = [hup.x as f32, hup.y as f32, hup.z as f32, 0.0];
        }
        hole_up[3] = if self.underwater { 1.0 } else { 0.0 };

        // Resolve W-MOTION's immutable system bank once for this frame. The
        // camera sample, orbital deck mean, W2 probes, and GPU uniforms all
        // reuse these exact centers/intensities.
        let cyclones = if camera.body == BodyId::Neisor
            && self.weather_on
            && let Some(field) = &self.weather_field
        {
            crate::weather::cyclone_systems(
                field,
                planet.seed,
                planet.radius_km,
                crate::weather::season_frac(
                    weather_t_s,
                    self.effective_day_len_s(),
                    &self.solar_tuning,
                ),
                weather_t_s,
                &self.weather_tuning,
            )
        } else {
            crate::weather::CycloneSystems::default()
        };

        // living weather: ONE field sample per frame, at the camera. The
        // shaders add per-pixel detail from the same uniforms, so weather
        // stays a pure function of (seed, position, weather time) — the
        // determinism contract in WEATHER.md.
        let wx = if camera.body == BodyId::Neisor
            && self.weather_on
            && let Some(field) = &self.weather_field
        {
            let mut w = crate::weather::weather_at_with_cyclones(
                field,
                planet,
                cam_pos.normalize(),
                weather_t_s,
                self.effective_day_len_s(),
                &self.solar_tuning,
                &self.weather_tuning,
                &cyclones,
            );
            if let Some((c, p)) = self.weather_pin {
                w.cloud_cover = c as f64;
                w.precip = p as f64;
                // Pinning is a presentation override, so its visible
                // saturation must reach the presentation humidity proxy too.
                w.humidity = w
                    .humidity
                    .max(c as f64 * 0.75 + p as f64 * 0.35)
                    .clamp(0.0, 1.0);
            }
            w
        } else {
            // temp 999 = the shader sentinel for "weather off": no ground
            // dusting, no darkening (a real 999 C never happens)
            crate::weather::Weather {
                temp_c: 999.0,
                ..Default::default()
            }
        };
        self.last_weather = wx;
        // Live cover/precip shape now comes from the spatial raster. Retain
        // the established camera-to-planet-mean scalar handoff for formation
        // temperature and for the exact pinned compatibility path. A pin
        // overrides both ends, keeping established pinned captures unchanged.
        let deck = if camera.body == BodyId::Neisor
            && self.weather_on
            && let Some(field) = &self.weather_field
        {
            let a = self.weather_tuning.shell_alt_km;
            let b = self.weather_tuning.shell_fade_km;
            let m = {
                let x = ((cam_h_km - a) / (b - a).max(1e-6)).clamp(0.0, 1.0);
                x * x * (3.0 - 2.0 * x)
            };
            let mut cover = wx.cloud_cover;
            let mut precip = wx.precip;
            let mut temp = wx.temp_c;
            if m > 0.0 {
                let mut mc = 0.0f64;
                let mut mp = 0.0f64;
                let mut mt = 0.0f64;
                // The established 14-direction W3 temperature mean remains
                // in orbit. Inside W2's camera-weather range, retain its
                // centrally symmetric 10-direction subset while the eight
                // compass probes are active.
                let dirs: [(f64, f64, f64); 14] = [
                    (1.0, 0.0, 0.0),
                    (-1.0, 0.0, 0.0),
                    (0.0, 1.0, 0.0),
                    (0.0, -1.0, 0.0),
                    (0.0, 0.0, 1.0),
                    (0.0, 0.0, -1.0),
                    (1.0, 1.0, 1.0),
                    (1.0, 1.0, -1.0),
                    (1.0, -1.0, 1.0),
                    (1.0, -1.0, -1.0),
                    (-1.0, 1.0, 1.0),
                    (-1.0, 1.0, -1.0),
                    (-1.0, -1.0, 1.0),
                    (-1.0, -1.0, -1.0),
                ];
                let edge_probes_active = neisor_cam_h_km < 40.0;
                for (index, (x, y, z)) in dirs.into_iter().enumerate() {
                    // Drop two opposite corner pairs only while W2's eight
                    // probes are live. The retained ten directions are still
                    // centrally symmetric (6 axes + 2 corner pairs).
                    if edge_probes_active && (8..12).contains(&index) {
                        continue;
                    }
                    let w = crate::weather::weather_at_with_cyclones(
                        field,
                        planet,
                        DVec3::new(x, y, z).normalize(),
                        weather_t_s,
                        self.effective_day_len_s(),
                        &self.solar_tuning,
                        &self.weather_tuning,
                        &cyclones,
                    );
                    mc += w.cloud_cover;
                    mp += w.precip;
                    mt += w.temp_c;
                }
                let n = if edge_probes_active { 10.0 } else { 14.0 };
                cover = cover * (1.0 - m) + (mc / n) * m;
                precip = precip * (1.0 - m) + (mp / n) * m;
                temp = temp * (1.0 - m) + (mt / n) * m;
            }
            if let Some((c, p)) = self.weather_pin {
                cover = c as f64;
                precip = p as f64;
            }
            (cover as f32, precip as f32, temp as f32)
        } else {
            (0.0, 0.0, 999.0)
        };
        // the cloud deck scrolls with the same advection the field uses:
        // precompute the domain drift here so shader noise and CPU weather
        // agree on where the storm is. The instantaneous wind vector drives
        // particle slant.
        let (drift, wind_kms) = {
            let dirn = cam_pos.normalize();
            let east0 = glam::DVec3::Z.cross(dirn);
            let east = if east0.length_squared() < 1e-9 {
                glam::DVec3::Y
            } else {
                east0.normalize()
            };
            let north = dirn.cross(east);
            let wind = east * wx.wind_e + north * wx.wind_n;
            let scale =
                self.weather_tuning.synoptic_speed * weather_t_s / (1000.0 * planet.radius_km);
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
        // D-8's signed residual is also the W2 valley-pooling signal. Keep
        // one resident raster lookup shared by particles, ground mist, and
        // the sky veil; open water deliberately owns no concavity.
        let camera_concavity = if camera.body == BodyId::Neisor && self.weather_on {
            let dir = cam_pos.normalize();
            let (face, u, v) = crate::planet::face_from_dir(dir);
            if planet.ocean(face, u, v) > 0.5 {
                0.0
            } else {
                let coarse = planet.elevation(face, u, v) as f64;
                let local = camera.ground_km / self.exaggeration.max(1e-6);
                crate::terrain::rain_concavity_proxy(coarse, local) as f64
            }
        } else {
            0.0
        };
        // D-8 particle interpolation reuses the same signed sub-raster
        // elevation residual carried to ground fragments. This costs one
        // already-resident bilinear elevation lookup, not another full
        // terrain sample. Snow density stays unchanged; Andrew's verdict is
        // specifically about rain collecting in crevices.
        let particle_precip = if wx.precip > 0.0 && wx.snow_frac < 1.0 {
            let redistribute = 1.0
                + self.weather_tuning.rain_crevice_bias * camera_concavity * (1.0 - wx.snow_frac);
            (wx.precip * redistribute).clamp(0.0, 1.0)
        } else {
            wx.precip
        };
        let n_precip = if self.underwater || camera.body != BodyId::Neisor {
            0u32
        } else {
            (particle_precip * precip_fade * self.weather_tuning.particles_max as f64) as u32
        };

        // W2 valley mist: sunrise is derived from the planet-frame sun and
        // site direction, so seeks and pinned solar poses reproduce exactly.
        let dawn = if camera.body == BodyId::Neisor && self.weather_on {
            crate::weather::dawn_window(
                cam_pos.normalize(),
                solar.sun_km.normalize_or_zero(),
                self.weather_tuning.fog_dawn_window_h,
            )
        } else {
            0.0
        };
        let broad_fog = {
            let cover = ((wx.cloud_cover - 0.78) / 0.20).clamp(0.0, 1.0);
            let cover = cover * cover * (3.0 - 2.0 * cover);
            let precip = ((wx.precip - 0.05) / 0.50).clamp(0.0, 1.0);
            let precip = precip * precip * (3.0 - 2.0 * precip);
            cover * precip
        };

        // W2 storm edge: eight compass points on a fixed great-circle ring.
        // The fit is first-order only (base + tangent gradient), cheap and
        // deterministic. Above the camera-weather fade the response is zero,
        // so the probes are skipped and orbit retains the W3 deck budget.
        let storm_edge = if camera.body == BodyId::Neisor
            && self.weather_on
            && neisor_cam_h_km < 40.0
            && let Some(field) = &self.weather_field
        {
            let center = cam_pos.normalize();
            let east0 = DVec3::Z.cross(center);
            let east = if east0.length_squared() < 1e-9 {
                DVec3::Y
            } else {
                east0.normalize()
            };
            let north = center.cross(east);
            let arc = (self.weather_tuning.storm_edge_probe_km / planet.radius_km)
                .clamp(1e-6, std::f64::consts::PI * 0.45);
            let mut probes = Vec::with_capacity(8);
            for i in 0..8 {
                let bearing = std::f64::consts::TAU * i as f64 / 8.0;
                let tangent = east * bearing.cos() + north * bearing.sin();
                let dir = center * arc.cos() + tangent * arc.sin();
                let mut weather = crate::weather::weather_at_with_cyclones(
                    field,
                    planet,
                    dir,
                    weather_t_s,
                    self.effective_day_len_s(),
                    &self.solar_tuning,
                    &self.weather_tuning,
                    &cyclones,
                );
                if let Some((c, p)) = self.weather_pin {
                    weather.cloud_cover = c as f64;
                    weather.precip = p as f64;
                }
                probes.push((dir, weather));
            }
            crate::weather::storm_edge_fit(center, &probes)
        } else {
            crate::weather::StormEdge::default()
        };

        let globals = Globals {
            view_proj: vp32.to_cols_array_2d(),
            inv_view_proj: inv32.to_cols_array_2d(),
            sun_dir: [sun.x as f32, sun.y as f32, sun.z as f32, 0.0],
            moon_dir: [moon.x as f32, moon.y as f32, moon.z as f32, 0.0],
            sun_body: [
                sun_rel.x as f32,
                sun_rel.y as f32,
                sun_rel.z as f32,
                self.solar_tuning.radius_km(BodyId::Sun, planet.radius_km) as f32,
            ],
            moon_body: [
                moon_rel.x as f32,
                moon_rel.y as f32,
                moon_rel.z as f32,
                self.solar_tuning.radius_km(BodyId::Moon, planet.radius_km) as f32,
            ],
            eclipse: [
                solar_occlusion as f32,
                lunar_shadow as f32,
                solar_contact_possible as u8 as f32,
                self.solar_tuning.sun_halo_strength as f32,
            ],
            sun_tint: [
                self.solar_tuning.sun_tint[0] as f32,
                self.solar_tuning.sun_tint[1] as f32,
                self.solar_tuning.sun_tint[2] as f32,
                0.0,
            ],
            moon_tint: [
                self.solar_tuning.moon_tint[0] as f32,
                self.solar_tuning.moon_tint[1] as f32,
                self.solar_tuning.moon_tint[2] as f32,
                0.0,
            ],
            moon_copper_tint: [
                self.solar_tuning.moon_copper_tint[0] as f32,
                self.solar_tuning.moon_copper_tint[1] as f32,
                self.solar_tuning.moon_copper_tint[2] as f32,
                0.0,
            ],
            hole,
            hole_up,
            sky: [up.x as f32, up.y as f32, up.z as f32, cam_h_km as f32],
            center: [
                (-cam_pos.x) as f32,
                (-cam_pos.y) as f32,
                (-cam_pos.z) as f32,
                lift as f32,
            ],
            body_frame: {
                let center = solar.position_km(camera.body) - cam_pos;
                [
                    center.x as f32,
                    center.y as f32,
                    center.z as f32,
                    camera.body.numeric_id() as f32,
                ]
            },
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
                (self.weather_on && self.weather_field.is_some() && self.weather_pin.is_some())
                    as u8 as f32,
            ],
            // w is Neisor camera altitude even while the active sky frame is
            // lunar; distant Neisor terrain still needs its own terminator,
            // fog, and orbital-weather handoff coordinate.
            weather8: [deck.0, deck.1, deck.2, neisor_cam_h_km as f32],
            weather9: [
                self.weather_tuning.cloud_shadow_strength as f32,
                self.weather_tuning.orbit_cloud_shadow_strength as f32,
                self.weather_tuning.fog_density as f32,
                self.weather_tuning.fog_ceiling_km as f32,
            ],
            weather10: [
                dawn as f32,
                wx.humidity as f32,
                camera_concavity as f32,
                broad_fog as f32,
            ],
            weather11: [
                storm_edge.gradient.x as f32,
                storm_edge.gradient.y as f32,
                storm_edge.gradient.z as f32,
                storm_edge.base as f32,
            ],
            weather12: [
                self.weather_tuning.storm_edge_strength as f32,
                self.weather_tuning.storm_green_cast as f32,
                1.0 / self.size.0.max(1) as f32,
                1.0 / self.size.1.max(1) as f32,
            ],
            weather13: [
                (self.weather_tuning.differential_rotation_deg_h.to_radians() * weather_t_s
                    / 3600.0) as f32,
                crate::weather::zonal_shear_phase(weather_t_s, &self.weather_tuning) as f32,
                (self.weather_tuning.cyclone_radius_km / planet.radius_km) as f32,
                cyclones.count as f32,
            ],
            weather14: [
                0.0,
                self.weather_tuning.cyclone_cover_boost as f32,
                self.weather_tuning.cyclone_precip_boost as f32,
                self.weather_tuning.front_strength as f32,
            ],
            weather15: [
                (self.weather_tuning.front_leading_km / self.weather_tuning.cyclone_radius_km)
                    as f32,
                (self.weather_tuning.front_trailing_km / self.weather_tuning.cyclone_radius_km)
                    as f32,
                (self.weather_tuning.front_length_km / self.weather_tuning.cyclone_radius_km)
                    .clamp(1.2, 2.9) as f32,
                self.weather_tuning.cyclone_arm_strength as f32,
            ],
            weather16: [
                self.weather_tuning.cyclone_arm_count as f32,
                self.weather_tuning.cyclone_arm_twist as f32,
                0.0,
                0.0,
            ],
            cyclone_centers: std::array::from_fn(|index| {
                let system = cyclones.systems[index];
                [
                    system.center.x as f32,
                    system.center.y as f32,
                    system.center.z as f32,
                    system.intensity as f32,
                ]
            }),
            cyclone_fronts: std::array::from_fn(|index| {
                // xyz retired with the straight front (the comma tail works
                // in the storm's polar frame); w carries the bounded wrap.
                [0.0, 0.0, 0.0, cyclones.systems[index].wrap_angle as f32]
            }),
            cyclone_arms: std::array::from_fn(|index| {
                let system = cyclones.systems[index];
                [system.arm_phase as f32, system.front_angle as f32, 0.0, 0.0]
            }),
            karst: {
                let kseed = |k: i64| {
                    (planet.seed.wrapping_add(k).wrapping_mul(0x9E37_79B1) & 0xFFFF_FFFF) as u32
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
                // spare lane: the range-biome comparator's octave-zero
                // seedmul (karst.w already belongs to the cloud layout)
                boundary_shader_seedmul(planet.seed) as i32,
            ],
            lights,
        };
        self.queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));

        let mut draws: Vec<(DrawKey, u32)> = Vec::new();
        let lunar_chunk_keys: Vec<ChunkKey> = chunk_keys
            .iter()
            .copied()
            .filter(|key| key.body == BodyId::Moon)
            .collect();
        // near-field voxel chunks FIRST: if the slot pool ever fills, the
        // truncation victim must be a distant mesh tile, never a near chunk
        // (a dropped chunk leaves a see-through hole where the heightfield
        // was already cut away underneath it).
        let all_keys = chunk_keys
            .iter()
            .filter(|key| key.body == BodyId::Neisor)
            .map(|k| DrawKey::Chunk(*k))
            .chain(keys.iter().map(|k| DrawKey::Tile(*k)));
        // Reserve the adaptive moon set plus one compact Sun slot in the
        // shared dynamic-uniform buffer.
        let terrain_slots = MAX_TILES.saturating_sub(
            moon_keys
                .len()
                .saturating_add(lunar_chunk_keys.len())
                .saturating_add(1),
        );
        for (slot, key) in all_keys.enumerate().take(terrain_slots) {
            let tile = match key {
                DrawKey::Tile(k) => self.cache.get_mut(&k).unwrap(),
                DrawKey::Chunk(k) => self.chunk_cache.get_mut(&k).unwrap(),
            };
            // re-entering the drawn set after ANY absence (fresh build,
            // stand-in release, eviction return) restarts the ease window
            if tile.last_used + 1 < self.frame_counter {
                tile.shown_at = self.frame_counter;
            }
            tile.last_used = self.frame_counter;
            // Two precision regimes. NEAR (the body you stand on): the f64
            // difference cast once keeps sub-mm alignment - splitting into
            // two f32 terms here would displace ground tiles by up to a
            // meter (reel-caught). FAR (another body, 10^5 km away): each
            // tile's offset quantizes to f32 on a DIFFERENT 8 m step and
            // the body shimmers tile-by-tile as the camera moves (Andrew:
            // "celestial bodies jitter violently") - a SHARED anchor makes
            // the rounding rigid: the body wobbles whole (invisible),
            // tiles stay mutually coherent.
            let precise = tile.origin_km - cam_pos;
            let off = if precise.length_squared() > 50_000.0 * 50_000.0 {
                tile.origin_km.as_vec3().as_dvec3() + (-cam_pos).as_vec3().as_dvec3()
            } else {
                precise
            };
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
                DrawKey::Tile(k) if k.level > 0 && std::env::var_os("TRI_NO_MORPH").is_none() => {
                    if std::env::var_os("TRI_FORCE_MORPH").is_some() {
                        [0.0001, 0.0002, 0.0, 0.0]
                    } else {
                        let s = k.size_km(planet.radius_km);
                        let start = s * (1.0 / ERR_TARGET + 0.75);
                        let end = s * (2.0 / ERR_TARGET - 1.45);
                        // temporal ease: a just-landed tile starts at its
                        // parent's geometry and settles over ~18 frames.
                        // Deep tiles are exempt (they must match the voxel
                        // patch exactly, and they never defer anyway), as
                        // are settling captures (byte-determinism).
                        let ease = if k.deep || self.settle_visuals {
                            0.0
                        } else {
                            let age = self.frame_counter.saturating_sub(self.cache[&k].shown_at);
                            (1.0 - age as f32 / 18.0).clamp(0.0, 1.0)
                        };
                        [start as f32, end as f32, ease, 0.0]
                    }
                }
                DrawKey::Chunk(k) if !self.settle_visuals => {
                    // chunk twin of the temporal ease: fresh blocks rise
                    // from the mesh surface (full rim-sink) instead of
                    // popping in - blocks cannot geomorph, but they can
                    // emerge. morph.z rides the same shader lane.
                    let age = self
                        .frame_counter
                        .saturating_sub(self.chunk_cache[&k].shown_at);
                    let ease = (1.0 - age as f32 / 18.0).clamp(0.0, 1.0);
                    [0.0, 0.0, ease, 0.0]
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
        let mut moon_draws = Vec::with_capacity(moon_keys.len());
        for key in moon_keys
            .iter()
            .take(MAX_TILES.saturating_sub(draws.len() + 1))
        {
            let slot = (draws.len() + moon_draws.len()) as u32;
            let tile = self.moon_cache.get_mut(key).unwrap();
            tile.last_used = self.frame_counter;
            // Moon-local tile origin + body center - camera, all in f64.
            // same two-regime rule as terrain tiles: precise when landed
            // on / near the moon, rigid shared anchor from planet range
            let precise = moon_rel + tile.origin_km;
            let off = if precise.length_squared() > 50_000.0 * 50_000.0 {
                moon_rel.as_vec3().as_dvec3() + tile.origin_km.as_vec3().as_dvec3()
            } else {
                precise
            };
            let morph = if key.level > 0 && std::env::var_os("TRI_NO_MORPH").is_none() {
                if std::env::var_os("TRI_FORCE_MORPH").is_some() {
                    [0.0001, 0.0002, 0.0, 0.0]
                } else {
                    let s = key.size_km(moon_radius);
                    let tau = moon_lod_error;
                    [
                        (s * (1.0 / tau + 0.75)) as f32,
                        (s * (2.0 / tau - 1.45)) as f32,
                        0.0,
                        0.0,
                    ]
                }
            } else {
                [0.0; 4]
            };
            let data = [
                off.x as f32,
                off.y as f32,
                off.z as f32,
                3.0,
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
            moon_draws.push((*key, slot));
        }
        let mut moon_chunk_draws = Vec::with_capacity(lunar_chunk_keys.len());
        for key in lunar_chunk_keys
            .iter()
            .take(MAX_TILES.saturating_sub(draws.len() + moon_draws.len() + 1))
        {
            let slot = (draws.len() + moon_draws.len() + moon_chunk_draws.len()) as u32;
            let tile = self.chunk_cache.get_mut(key).unwrap();
            if tile.last_used + 1 < self.frame_counter {
                tile.shown_at = self.frame_counter;
            }
            tile.last_used = self.frame_counter;
            // same two-regime rule as terrain tiles: precise when landed
            // on / near the moon, rigid shared anchor from planet range
            let precise = moon_rel + tile.origin_km;
            let off = if precise.length_squared() > 50_000.0 * 50_000.0 {
                moon_rel.as_vec3().as_dvec3() + tile.origin_km.as_vec3().as_dvec3()
            } else {
                precise
            };
            let ease = if self.settle_visuals {
                0.0
            } else {
                let age = self.frame_counter.saturating_sub(tile.shown_at);
                (1.0 - age as f32 / 18.0).clamp(0.0, 1.0)
            };
            let data = [
                off.x as f32,
                off.y as f32,
                off.z as f32,
                4.0,
                0.0,
                0.0,
                ease,
                0.0,
            ];
            self.queue.write_buffer(
                &self.tiles_buf,
                slot as u64 * TILE_UNIFORM_STRIDE,
                bytemuck::bytes_of(&data),
            );
            moon_chunk_draws.push((*key, slot));
        }
        let body_specs = [(
            sun_rel,
            1.0f32,
            self.solar_tuning.radius_km(BodyId::Sun, planet.radius_km),
        )];
        let mut body_slots = Vec::with_capacity(1);
        for (center, kind, radius) in body_specs
            .into_iter()
            .filter(|(center, _, radius)| body_visible(*center, *radius))
        {
            let slot =
                (draws.len() + moon_draws.len() + moon_chunk_draws.len() + body_slots.len()) as u32;
            // Body center is already camera-relative in f64. Only this small
            // boundary conversion reaches the GPU; no planet/solar magnitude
            // is subtracted in f32.
            let data = [
                center.x as f32,
                center.y as f32,
                center.z as f32,
                kind,
                radius as f32,
                0.0,
                0.0,
                0.0,
            ];
            self.queue.write_buffer(
                &self.tiles_buf,
                slot as u64 * TILE_UNIFORM_STRIDE,
                bytemuck::bytes_of(&data),
            );
            body_slots.push(slot);
        }
        self.evict();

        let avatar_vertices = self.build_avatar_vertices(camera, solar, planet.radius_km);
        if !avatar_vertices.is_empty() {
            self.queue.write_buffer(
                &self.avatar_vertex_buf,
                0,
                bytemuck::cast_slice(&avatar_vertices),
            );
        }

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
            // Bodies draw over the non-depth-writing sky but remain behind
            // Neisor terrain. Sun first, adaptive moon second: reversed-Z
            // makes the foreground moon physically cover the Sun at contact.
            pass.set_pipeline(&self.body_pipeline);
            pass.set_vertex_buffer(0, self.body_vertex_buf.slice(..));
            pass.set_index_buffer(self.body_index_buf.slice(..), wgpu::IndexFormat::Uint32);
            for slot in &body_slots {
                pass.set_bind_group(0, &self.bind_group, &[*slot * TILE_UNIFORM_STRIDE as u32]);
                pass.draw_indexed(0..self.body_index_count, 0, 0..1);
            }
            pass.set_pipeline(&self.moon_pipeline);
            for (key, slot) in &moon_draws {
                let tile = &self.moon_cache[key];
                pass.set_bind_group(0, &self.bind_group, &[*slot * TILE_UNIFORM_STRIDE as u32]);
                pass.set_vertex_buffer(0, tile.vertex_buf.slice(..));
                pass.set_index_buffer(tile.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..tile.index_count, 0, 0..1);
            }
            for (key, slot) in &moon_chunk_draws {
                let tile = &self.chunk_cache[key];
                pass.set_bind_group(0, &self.bind_group, &[*slot * TILE_UNIFORM_STRIDE as u32]);
                pass.set_vertex_buffer(0, tile.vertex_buf.slice(..));
                pass.set_index_buffer(tile.index_buf.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..tile.index_count, 0, 0..1);
            }
            if !avatar_vertices.is_empty() {
                pass.set_pipeline(&self.avatar_pipeline);
                pass.set_bind_group(0, &self.bind_group, &[0]);
                pass.set_vertex_buffer(0, self.avatar_vertex_buf.slice(..));
                pass.draw(0..avatar_vertices.len() as u32, 0..1);
            }
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
        draws.len() + moon_draws.len() + moon_chunk_draws.len()
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

    fn upload(&self, mesh: TileMesh, season_bucket: u32) -> GpuTile {
        let bytes =
            (mesh.vertices.len() * std::mem::size_of::<Vertex>() + mesh.indices.len() * 4) as u64;
        GpuTile {
            origin_km: mesh.origin_km,
            vertex_buf: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tile vb"),
                    contents: bytemuck::cast_slice(&mesh.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            index_buf: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tile ib"),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            index_count: mesh.indices.len() as u32,
            last_used: self.frame_counter,
            bytes,
            shown_at: self.frame_counter,
            season_bucket,
        }
    }

    fn evict(&mut self) {
        let cutoff = self.frame_counter.saturating_sub(120);
        if self.cache.len() > 1500 {
            self.cache.retain(|_, t| t.last_used >= cutoff);
        }
        if self.moon_cache.len() > 2400 {
            self.moon_cache.retain(|_, t| t.last_used >= cutoff);
        }
        // sized for --patch 2.0 (a ~1 km disc is ~1600 chunks)
        if self.chunk_cache.len() > 4000 {
            self.chunk_cache.retain(|_, t| t.last_used >= cutoff);
        }
        // hard VRAM budget: newest chunks win, the LRU tail is dropped —
        // without this, fast flight at big patch sizes accumulates buffers
        // faster than the age-based eviction can retire them (OOM)
        self.enforce_chunk_budget();
        self.enforce_tile_budget();
        // Moon eviction runs only after this frame's selected tiles have
        // been marked used. Chunk streaming can finish earlier in draw; doing
        // lunar eviction there could drop a selected moon tile between
        // selection and uniform upload during a landing capture.
        self.enforce_moon_budget();
    }

    /// Tile twin of the chunk budget: age-based eviction alone lets a
    /// teleport sequence (the reel) accumulate rings faster than they expire.
    /// RECENCY IS SACRED here: dense-forest rings (impostor-fat tiles,
    /// ~2.2 MB each) can exceed the whole budget as a WORKING SET, and a
    /// current-frame-only exemption then evicts live tiles every frame -
    /// each re-entry pays the expensive jungle candidate loop and renders a
    /// coarse stand-in meanwhile (Andrew's giant flickering impostor trees,
    /// 2026-07-12). Tiles used within the last EVICT_PROTECT_FRAMES are
    /// untouchable; the budget may overshoot briefly instead of thrashing.
    fn enforce_tile_budget(&mut self) {
        let total: u64 = self.cache.values().map(|t| t.bytes).sum();
        if total <= TILE_VRAM_BUDGET {
            return;
        }
        // Captures drain to completion and cannot flicker - during them the
        // strict budget rules (teleport-heavy instruments like the 26-pose
        // reel would otherwise hold every prior pose alive and OOM small
        // adapters). Live frames get the full anti-thrash window, but the
        // HARD ceiling wins over protection: a crash is worse than a
        // one-frame rebuild (Andrew's in-game OOMs, incl. at photo press).
        let window = if self.settle_visuals { 0 } else { 180 };
        let protected = self.frame_counter.saturating_sub(window);
        let mut by_age: Vec<(u64, TileKey, u64)> = self
            .cache
            .iter()
            .map(|(k, t)| (t.last_used, *k, t.bytes))
            .collect();
        by_age.sort_unstable_by(|a, b| b.0.cmp(&a.0)); // newest first
        let mut acc = 0u64;
        let mut keep = HashSet::new();
        for (used, k, b) in by_age {
            let within_hard = acc + b <= TILE_VRAM_HARD;
            if (used >= protected && within_hard) || acc + b <= TILE_VRAM_RETAIN {
                acc += b;
                keep.insert(k);
            }
        }
        self.cache.retain(|k, _| keep.contains(k));
        // dropped tiles may have in-flight builds pending; those results
        // re-insert on arrival and age out normally, so no epoch fixup needed
    }

    /// Enforce the byte budget during streaming as well as before drawing;
    /// otherwise a teleport's completed batch can OOM before `evict` runs.
    fn enforce_chunk_budget(&mut self) {
        let total: u64 = self.chunk_cache.values().map(|t| t.bytes).sum();
        if total <= CHUNK_VRAM_BUDGET {
            return;
        }
        let mut by_age: Vec<(u64, ChunkKey, u64)> = self
            .chunk_cache
            .iter()
            .map(|(k, t)| (t.last_used, *k, t.bytes))
            .collect();
        by_age.sort_unstable_by(|a, b| b.0.cmp(&a.0)); // newest first
        let mut acc = 0u64;
        let mut keep = HashSet::new();
        // RECENCY IS SACRED (same law as tiles): a large patch's working
        // set exceeds the whole budget, and a current-frame-only exemption
        // then evicts live chunks every frame while moving - the patch
        // edge perpetually rebuilds and near-field blocks + voxel trees
        // pop in and out (Andrew's ground flicker, 2026-07-12). Overshoot
        // the budget briefly rather than thrash.
        let window = if self.settle_visuals { 0 } else { 180 };
        let protected = self.frame_counter.saturating_sub(window);
        for (used, k, b) in by_age {
            let within_hard = acc + b <= CHUNK_VRAM_HARD;
            if (used >= protected && within_hard) || acc + b <= CHUNK_VRAM_RETAIN {
                acc += b;
                keep.insert(k);
            }
        }
        self.chunk_cache.retain(|k, _| keep.contains(k));
    }

    fn enforce_moon_budget(&mut self) {
        let moon_total: u64 = self.moon_cache.values().map(|t| t.bytes).sum();
        if moon_total > MOON_VRAM_BUDGET {
            let mut by_age: Vec<(u64, TileKey, u64)> = self
                .moon_cache
                .iter()
                .map(|(k, t)| (t.last_used, *k, t.bytes))
                .collect();
            by_age.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            let mut bytes = 0u64;
            let mut keep = HashSet::new();
            for (used, key, tile_bytes) in by_age {
                if used == self.frame_counter || bytes + tile_bytes <= MOON_VRAM_BUDGET {
                    bytes += tile_bytes;
                    keep.insert(key);
                }
            }
            self.moon_cache.retain(|key, _| keep.contains(key));
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
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // TRI_RAW_CAPTURE: diagnostic mode - capture EXACTLY the single
        // live frame, no chunk wait, no tile drain. This is the only lens
        // that can see stand-in churn and motion flicker (the drains below
        // deliberately hide them for byte-determinism).
        let raw = std::env::var_os("TRI_RAW_CAPTURE").is_some();
        self.settle_visuals = !raw;
        // Capture settling must cover the INITIAL draw too. Previously it
        // was enabled only afterward, so a shot with no pending tiles kept
        // that first frame's temporal LOD ease. The ease age depended on
        // which asynchronous prefetch landed first and made consecutive
        // pinned orbital captures differ by a few thousand +/-1 pixels.
        let view = texture.create_view(&Default::default());
        let mut n_tiles = self.draw(&view, planet, camera, edits);
        if !raw {
            // free the anti-thrash overshoot BEFORE allocating the capture
            // texture + readback: photo-press was an OOM trigger in play
            self.evict();
        }
        // A screenshot is a complete frame for BOTH streaming systems.
        // Drain chunks and tiles as one convergence loop: a redraw after a
        // chunk drain can queue another boundary chunk, and `tiles_deferred`
        // deliberately includes that chunk state. The former two independent
        // loops could therefore wait 30 s on the TILE channel while only a
        // CHUNK was outstanding, then capture whichever builds happened to
        // land during 256 non-blocking spins. Time-stepped ground reels were
        // consequently scheduling-dependent even though weather was pure.
        let expected_bucket = self.structural_season(planet).bucket;
        let mut settled = raw || (self.chunk_pending.is_empty() && !self.tiles_deferred);
        let mut stream_error = None;
        // Structural-season swaps intentionally admit only eight stale tile
        // rebuilds per draw. MAX_TILES/8 therefore covers even the renderer's
        // theoretical full selected set while preserving a finite bound.
        for _round in 0..MAX_TILES.div_ceil(8) {
            if settled {
                break;
            }

            let mut chunk_landed = 0usize;
            for _ in 0..8192 {
                if self.chunk_pending.is_empty() {
                    break;
                }
                match self
                    .chunk_rx
                    .recv_timeout(std::time::Duration::from_secs(30))
                {
                    Ok((k, epoch, bucket, mesh)) => {
                        if self.chunk_pending.get(&k) == Some(&epoch) {
                            self.chunk_pending.remove(&k);
                            let mut gpu = self.upload(mesh, bucket);
                            if let Some(old) = self.chunk_cache.get(&k) {
                                gpu.shown_at = old.shown_at;
                            }
                            self.chunk_cache.insert(k, gpu);
                            chunk_landed += 1;
                            if chunk_landed.is_multiple_of(16) {
                                self.enforce_chunk_budget();
                            }
                        }
                    }
                    Err(err) => {
                        stream_error = Some(format!("chunk stream stalled: {err}"));
                        break;
                    }
                }
            }
            if chunk_landed > 0 {
                self.enforce_chunk_budget();
            }
            if stream_error.is_some() {
                break;
            }

            // Drain only when selected coverage is incomplete. Pending
            // parent-prefetch work is allowed to finish later because it is
            // not in this frame's resolved draw set.
            for _ in 0..8192 {
                if !self.tiles_deferred || self.tile_pending.is_empty() {
                    break;
                }
                match self
                    .tile_rx
                    .recv_timeout(std::time::Duration::from_secs(30))
                {
                    Ok((k, epoch, bucket, mesh)) => {
                        if self.tile_pending.get(&k) == Some(&epoch) {
                            self.tile_pending.remove(&k);
                            if bucket == expected_bucket {
                                let mut gpu = self.upload(mesh, bucket);
                                if let Some(old) = self.cache.get(&k) {
                                    gpu.shown_at = old.shown_at;
                                }
                                self.cache.insert(k, gpu);
                            }
                        }
                    }
                    Err(err) => {
                        stream_error = Some(format!("tile stream stalled: {err}"));
                        break;
                    }
                }
            }
            if stream_error.is_some() {
                break;
            }

            // Re-selection observes every landed mesh and may expose the
            // next refinement boundary. Iterate until that draw reports no
            // selected stand-ins or pending chunks.
            n_tiles = self.draw(&view, planet, camera, edits);
            settled = self.chunk_pending.is_empty() && !self.tiles_deferred;
        }
        if !settled {
            self.settle_visuals = false;
            anyhow::bail!(
                "capture streaming did not settle (chunks pending {}, tiles pending {}, deferred {}, chunk cache {}, tile cache {}){}",
                self.chunk_pending.len(),
                self.tile_pending.len(),
                self.tiles_deferred,
                self.chunk_cache.len(),
                self.cache.len(),
                stream_error
                    .as_deref()
                    .map(|err| format!(": {err}"))
                    .unwrap_or_default()
            );
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
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        self.settle_visuals = false;
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| anyhow::anyhow!("device poll: {e:?}"))?;
        let data = slice
            .get_mapped_range()
            .map_err(|e| anyhow::anyhow!("map readback: {e:?}"))?;

        // strip row padding; swizzle BGRA -> RGBA if needed
        let bgra = matches!(
            self.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
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

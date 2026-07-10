//! The LOD quadtree and tile meshing.
//!
//! One quadtree per cube face. A node at `level` covers 1/2^level of the face
//! per axis; every node can be meshed *by itself* from (face, level, ix, iy) —
//! the "query the hierarchy at any depth" property. Above the raster floor,
//! heights come from the baked planet data; below it, band-limited noise
//! octaves take over — each level adds exactly the octaves its vertex spacing
//! can carry, so descending never runs out of detail and coarser tiles are
//! consistent averages of finer ones.

use crate::noise::{fbm_band, ridged_band};
use crate::planet::{climate_surface, face_dir, MainBlock, Planet};
use glam::DVec3;

pub const TILE_QUADS: usize = 32; // 32x32 quads, 33x33 vertices per tile
pub const MAX_LEVEL: u8 = 14; // ~0.8 km tiles, ~26 m vertex spacing at the cap
// (was 12 / 103 m: a 15 m river cannot exist in a 103 m lattice — the wet
// paint widened it to vertex spacing and the carved valley vanished, so the
// mesh side of the patch rim read as a different planet. Austin's paired
// photos at 15.024 17.648 are the exhibit.)
const DETAIL_BASE_FREQ: f64 = 700.0; // first detail octave ~12 km at R=8660
const MAX_DETAIL_OCTAVES: u32 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TileKey {
    pub face: u8,
    pub level: u8,
    pub ix: u16,
    pub iy: u16,
    /// Deep tiles sit under/near the voxel patch and sample the full voxel
    /// octave stack, so mesh and blocks agree to within one block (otherwise
    /// their height fields differ by many meters and poke through each other).
    pub deep: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3], // relative to tile origin (km)
    pub normal: [f32; 3],
    pub color: [f32; 3],
    /// rgb = water color, a = wetness flag. The fragment shader steps on
    /// interpolated a, so painted water gets crisp per-pixel edges even on
    /// coarse tiles whose triangles span the whole river.
    pub water: [f32; 4],
    /// Geomorphing: radial height delta (km, exaggerated) from this vertex
    /// to the PARENT level's rendered triangle at this lattice point. The
    /// radial-only slide leaves at most a measured 0.13 m parent-chord
    /// residual in the V-6 level-9 probes, below the vec3 threshold.
    pub morph_dh: f32,
    /// Geomorphing for the river paint: the wetness the PARENT level's actual
    /// triangle interpolates here. The painted thread is widened to the
    /// vertex spacing, so a split halves its width — the dominant visible
    /// LOD pop. Morphing the wetness with the same factor retires it.
    pub morph_wet: f32,
    /// 1.0 on a sea/lake water *surface* vertex, else 0.0. The heightfield
    /// hole (which lets voxel blocks own the near disc) must NOT cut the
    /// mesh water plane: block water and mesh water are the same surface, so
    /// cutting it opens a see-through crack at the patch boundary that shows
    /// the sky (a black void underwater). Keeping the water plane under the
    /// patch backs any perimeter crack with water instead of void.
    pub wflag: f32,
    /// Signed water-minus-ground delta (km, clamped ±0.005) for sea/lake/river
    /// shorelines — the fragment shader steps its interpolated zero crossing
    /// with derivative AA, so the shoreline lives at PIXEL resolution
    /// instead of vertex resolution (TRANSITIONS.md B; V-5's angular lake
    /// polygons and orphan blue cells). -1.0 = no standing water nearby
    /// (also on voxel chunks and impostors, which are already exact).
    pub shore: f32,
}

pub struct TileMesh {
    pub origin_km: DVec3,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl TileKey {
    pub fn uv_range(&self) -> (f64, f64, f64) {
        let n = (1u32 << self.level) as f64;
        let size = 2.0 / n;
        (-1.0 + self.ix as f64 * size, -1.0 + self.iy as f64 * size, size)
    }

    pub fn center_dir(&self) -> DVec3 {
        let (u0, v0, size) = self.uv_range();
        face_dir(self.face as usize, u0 + size * 0.5, v0 + size * 0.5)
    }

    /// Approximate tile width in km at the planet surface.
    pub fn size_km(&self, radius_km: f64) -> f64 {
        (std::f64::consts::FRAC_PI_2 * radius_km) / (1u32 << self.level) as f64
    }
}

/// Select the tiles to render this frame (screen-space error driven).
/// focus: optional (direction, radius_km) of the voxel patch — nearby tiles
/// at fine levels are marked deep so their heights match the blocks.
pub fn select_tiles(
    cam_pos_km: DVec3,
    radius_km: f64,
    err_target: f64,
    focus: Option<(DVec3, f64)>,
) -> Vec<TileKey> {
    let mut out = Vec::new();
    let cam_dist = cam_pos_km.length();
    let horizon = (radius_km / cam_dist.max(radius_km + 1.0)).acos() + 0.35;
    for face in 0..6u8 {
        let mut stack = vec![TileKey { face, level: 0, ix: 0, iy: 0, deep: false }];
        while let Some(k) = stack.pop() {
            let center = k.center_dir();
            let size = k.size_km(radius_km);
            let ang = center.dot(cam_pos_km / cam_dist).clamp(-1.0, 1.0).acos();
            let node_ang = size * 1.5 / radius_km;
            if ang - node_ang > horizon {
                continue;
            }
            let dist = (cam_pos_km - center * radius_km).length().max(0.5);
            if size / dist > err_target && k.level < MAX_LEVEL {
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    stack.push(TileKey {
                        face: k.face,
                        level: k.level + 1,
                        ix: k.ix * 2 + dx,
                        iy: k.iy * 2 + dy,
                        deep: false,
                    });
                }
            } else {
                let deep = k.level >= 9
                    && focus.is_some_and(|(fdir, frad)| {
                        let a = center.dot(fdir).clamp(-1.0, 1.0).acos();
                        a * radius_km < frad + size
                    });
                out.push(TileKey { deep, ..k });
            }
        }
    }
    out
}

/// How many detail octaves a tile at `level` can carry: include octave o while
/// its wavelength stays comfortably above the vertex spacing.
fn octave_count(level: u8, radius_km: f64) -> u32 {
    let spacing = (std::f64::consts::FRAC_PI_2 * radius_km)
        / ((1u32 << level) as f64 * TILE_QUADS as f64);
    let mut count = 0u32;
    while count < MAX_DETAIL_OCTAVES {
        let wavelength = radius_km / (DETAIL_BASE_FREQ * 2f64.powi(count as i32));
        if wavelength < 2.5 * spacing {
            break;
        }
        count += 1;
    }
    count
}

/// Exaggerated ground height (km) under a direction — used to keep the
/// camera above the local surface rather than above sea level.
pub fn ground_height_km(planet: &Planet, dir: DVec3, exaggeration: f64) -> f64 {
    let (face, u, v) = crate::planet::face_from_dir(dir);
    let (h, _) = sample_height(planet, face, u, v, MAX_DETAIL_OCTAVES);
    h * exaggeration
}

/// The full octave depth used for voxel block heights (~3 m floor).
pub const VOXEL_OCTAVES: u32 = 12;
/// A lake needs this much water over continuous ground before it exists.
/// Shared with the mesh shore field so both renderers step the same class.
const LAKE_WATER_CLEARANCE_KM: f64 = 0.0005;
/// Octave depth of the river wet/dry reference surface: every LOD reads the
/// perch decision at this depth so mesh and voxels always agree about which
/// reaches carry water.
pub const RIVER_REF_OCTAVES: u32 = 8;

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One generated point of the world: terrain, water, and the map-scale
/// climate context everything downstream (colors, materials, flora) keys on.
#[derive(Clone, Copy)]
pub struct Sample {
    pub h_km: f64,     // terrain surface height (post river/pond carving)
    pub water_km: f64, // water surface height; f64::NEG_INFINITY = no water
    pub e_raw: f64,    // raw map elevation (negative = ocean)
    pub rough: f64,    // map-scale roughness (km between ~30 km cells)
    pub temp_c: f64,   // annual mean temperature
    pub precip: f64,   // annual precipitation (mm)
    /// Continuous 0..1 "wetness" for far-tile water tinting: feathered by
    /// channel proximity so coarse meshes show soft river threads instead
    /// of per-vertex chopped polygons.
    pub wet_soft: f64,
    /// Total river/pond carving depth (km) — how far below the undisturbed
    /// terrain this point was cut. Flora avoids carved ground.
    pub carve_km: f64,
    /// Point is flooded by a lake (water level from the drainage graph).
    /// Lakes render like the sea: real geometry at the surface, so a
    /// 100-km lake reads flat from orbit instead of a painted bowl.
    pub lake: bool,
    /// Candidate lake level for local shore material, even when this sample
    /// is dry ground just above the waterline.
    pub lake_level_km: f64,
    /// Octave-independent distance to the baked lake footprint edge. Derived
    /// inside the existing `lake_at` geometry query; no terrain resampling.
    pub lake_boundary_dist_km: f64,
    /// Distance to the nearest (meandered) river course and its channel
    /// half-width, for LOD-aware paint: a river narrower than a coarse
    /// tile's vertex spacing would only catch sporadic vertices and shatter
    /// into shards — build_tile widens the painted thread to one vertex
    /// spacing instead. `river_wet` is the perch fade (0 = dry wash).
    pub river_dist_km: f64,
    pub river_hw_km: f64,
    pub river_wet: f64,
    /// Graph water-surface level (km) of the nearest river course, kept even
    /// for dry banks above the waterline. The lateral water table that floods
    /// caves passing under a river bank (flooded-caves feature) reads this.
    /// f64::NEG_INFINITY when no river is near.
    pub river_level_km: f64,
    /// Flooding lake is a salt lake (mineral-pale water).
    pub salt: bool,
    /// True open ocean. NOT the same as e_raw <= 0: the map has genuine dry
    /// below-sea-level basins, and bilinear elevation dips a few meters under
    /// zero on land all along the coasts — painting those as sea put navy
    /// plates on beaches and flooded whole desert depressions.
    pub sea: bool,
}

impl Sample {
    /// Height the far-field mesh renders at. Only the SEA lifts geometry to
    /// its water surface: inland water is painted as color on the carved
    /// channel floor. Lifting per-vertex water at coarse LOD tents isolated
    /// vertices into floating shards; color can't.
    pub fn render_h_km(&self) -> f64 {
        if self.sea || self.lake {
            self.h_km.max(self.water_km)
        } else {
            self.h_km
        }
    }
    pub fn has_water(&self) -> bool {
        self.water_km > self.h_km
    }
}

/// The Phase-1 "generator", now with hydrology: planet data down to the
/// raster floor, banded noise below it, roughness-driven relief, and
/// carved rivers/ponds with explicit water surfaces. Shared by the mesh
/// tiles and the voxel columns so the two can never disagree.
pub fn sample(planet: &Planet, face: usize, u: f64, v: f64, octaves: u32) -> Sample {
    let e_raw = planet.elevation(face, u, v) as f64;
    let rough = planet.rough(face, u, v) as f64;
    let temp_c = planet.temp(face, u, v) as f64;
    let precip = planet.precip(face, u, v) as f64;
    let ofrac = planet.ocean(face, u, v) as f64;
    // sea = below sea level AND on the ocean side of the map's own
    // cell-resolution coastline (sharp mask). Raster elevation is an
    // inverse-distance blend of ~30 km cells, so deep ocean neighbors drag
    // land-side texels tens of meters below zero along coasts — those
    // shallow "tongues" reaching inland of the coastline painted triangular
    // navy lagoons on coastal deserts. The depth clause keeps genuinely deep
    // water wet where a ragged coast mislabels a texel land (straits,
    // fjords) — but only near the coast, so the map's deep DRY interior
    // basins stay dry.
    let wmask = planet.water_frac(face, u, v) as f64;
    let mut out = Sample {
        h_km: 0.0,
        water_km: f64::NEG_INFINITY,
        e_raw,
        rough,
        temp_c,
        precip,
        wet_soft: 0.0,
        carve_km: 0.0,
        lake: false,
        lake_level_km: f64::NEG_INFINITY,
        lake_boundary_dist_km: f64::INFINITY,
        river_dist_km: f64::INFINITY,
        river_hw_km: 0.0,
        river_wet: 0.0,
        river_level_km: f64::NEG_INFINITY,
        salt: false,
        sea: e_raw <= 0.0 && (wmask >= 0.5 || (e_raw <= -0.1 && ofrac > 0.35)),
    };

    if out.sea {
        // ocean: shallow near-field floor (blocks), true depth kept in e_raw
        out.h_km = e_raw.max(-0.006);
        out.water_km = 0.0;
        out.wet_soft = 1.0;
        return out;
    }

    // hold coastal land above the waterline; interior basins the map keeps
    // dry are allowed their true (below-sea) depth
    let h_floor = if ofrac > 0.02 { 0.002 } else { -9.0 };
    let mut h = e_raw;
    let dir = face_dir(face, u, v);
    let seed = planet.seed;

    // rivers: the course is DATA (nearest drainage-graph segment, exported
    // by scripts/bake_rivers.py), not noise. Noise only bends the perceived
    // course into meanders — a bounded displacement of the query point, so
    // the wiggled channel always stays inside its own damped floodplain.
    let mut riv_d = f64::INFINITY; // distance to (meandered) course, km
    let mut hw = 0.0; //  channel half-width, km
    let mut d_max = 0.0; // extra mid-channel depth, km
    let mut wl = f64::NEG_INFINITY; // water level from the graph, km
    if planet.rivers.maybe_river(face, u, v) {
        let ref_axis = if dir.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
        let t1 = (ref_axis - dir * ref_axis.dot(dir)).normalize();
        let t2 = dir.cross(t1);
        let m1 = fbm_band(dir, 0, 2, 9000.0, seed.wrapping_add(4111));
        let m2 = fbm_band(dir, 0, 2, 9000.0, seed.wrapping_add(4513));
        let dm = (dir + (t1 * m1 + t2 * m2) * (0.18 / planet.radius_km)).normalize();
        let (mf, mu, mv) = crate::planet::face_from_dir(dm);
        if let Some(hit) = planet.rivers.river_near(mf, mu, mv, dm) {
            // hydraulic geometry: width ~ 3 sqrt(Q) m, depth ~ Q^0.39,
            // tapering in from the headwater cutoff so creeks grow from
            // nothing instead of popping out at full width
            let q = hit.flow;
            let taper = smoothstep(120.0, 400.0, q);
            if taper > 0.0 {
                hw = 0.0015 * q.sqrt() * taper;
                d_max = (0.00027 * q.powf(0.39)).min(0.012) * taper;
                wl = hit.level_km;
                riv_d = hit.dist_km;
            }
        }
    }

    // lake candidate (level/salt/dist/radius) — queried BEFORE the octave
    // block so a lake can flatten the fine relief under its own footprint: a
    // lake bed is level sediment, and full-strength noise there both spikes
    // dry islands through the surface and digs pockets the flat fill drowns
    // into walls.
    let lake = planet.rivers.lake_at(face, u, v, dir);
    // Flood eligibility. The sim's rim ring is the dam (every rim cell's
    // elevation >= the spill level by construction), so the flood covers the
    // lake's own Voronoi territory plus a bounded shore band into rim
    // territory — the band is where fine noise dips below the level and the
    // shoreline legitimately wanders past the coarse footprint (the lake-414
    // walls). It must NOT extend to the raw 3-radius disc: that let a lake
    // pour over its dam wherever any distant terrain sat below its level —
    // e.g. a sunken outlet-river corridor 40 km out (BUGS.md W-1).
    let lake_flood = lake.as_ref().and_then(|hit| {
        // rim-cell TERRITORY is everything within ~half a cell spacing
        // (cells sit ~2.2 radii apart) of the rim's center: flood-through
        // there covers the below-level shore dips and drowned island rings
        // the sim's coarse footprint leaves out (lake 414's dry pit), while
        // anything past the rim ring — e.g. a below-level outlet corridor
        // 40 km on — stays governed by its own hydrology.
        // KNOWN RESIDUAL (BUGS.md W-5): a knife-ridge mountain lake's 31 km
        // cells overhang their outer flanks, so this territory test floods
        // terrain far below such a lake. Per-sample gates (level margin,
        // basin-floor comparison) were measured by the census to only move
        // or worsen those walls — the overhang needs a bake-level fix with
        // whole-lake context (shrink/flag steep-rim cells at export).
        // ...and only over TRUE DAMS: a rim whose own elevation is far below
        // the level (a peeled conduit cell down a flank) must not pass the
        // flood through its territory (W-5, lake 873).
        let d_any = hit.d_lake_km - hit.past_boundary_km;
        // ...and only within 2.6 radii of a true lake cell: a dam-height rim
        // left far up a peeled conduit chain otherwise floods its band with
        // water pinned at basin level 40+ km from any actual lake — phantom
        // pools that ended in census walls wherever coverage or terrain cut
        // them (166 m at 16.569 -32.262). Legit shore bands top out ~2.2 r
        // (voronoi edge + rim band), so 2.6 keeps every real shore wet. The
        // shore apron grades from this frontier too (rivers.rs apron_past).
        (hit.d_lake_km < hit.radius_km * 2.6
            && (hit.in_lake_voronoi || (d_any < hit.radius_km * 1.15 && hit.rim_is_dam)))
        .then_some((hit.level_km, hit.salt, hit.boundary_dist_km))
    });

    // the roughness raster spikes wherever a land cell borders deep ocean
    // (continental slope) — damp it near sea level so coasts get beaches,
    // not kilometer cliffs
    let rough_r = rough * smoothstep(0.02, 0.30, e_raw);
    // floodplain: fine relief flattens near a river course (rivers build
    // their own valley bottoms). Exact distance makes this surgical — the
    // old fuzzy flow gate would have flattened half the map.
    let valley = if riv_d.is_finite() {
        0.12 + 0.88 * smoothstep(hw + 0.10, hw * 5.0 + 0.9, riv_d)
    } else {
        1.0
    };
    // terrain height at the FIXED river-reference octave depth — the wet/dry
    // decision for rivers reads this at every LOD (see below). Falls back to
    // the caller's own h only on octave-0 tiles (orbital, paint subpixel).
    let mut h_river_ref = h;
    if octaves > 0 {
        // relief amplitude follows the map's own roughness metric: jagged
        // where the map is jagged, calm plains stay calm. Ridged noise
        // dominates in rough country, billowy fbm elsewhere.
        let mut env = (0.06 + rough_r * 0.85 + e_raw * 0.10).clamp(0.05, 1.7) * valley;
        // flatten the bed in proportion to how far the COARSE terrain sits
        // below the lake level: flat sediment deep in the basin (so the flat
        // fill can't spike islands or dig wall-making pockets), full relief on
        // dry land at/above the level — a lake can't flatten unrelated higher
        // ground that merely falls inside its cell-search disc (e.g. a pond).
        // Keyed to flood ELIGIBILITY: terrain outside the dam keeps its full
        // relief even when it happens to sit below some lake's level.
        if let Some((lvl, _, _)) = lake_flood {
            let submerge = smoothstep(-0.005, 0.012, lvl - e_raw);
            env *= 1.0 - 0.88 * submerge;
        }
        let rw = (0.30 + rough_r * 0.50).clamp(0.30, 0.72);
        let detail = rw * ridged_band(dir, 0, octaves, DETAIL_BASE_FREQ, seed.wrapping_add(1013))
            + (0.95 - rw) * fbm_band(dir, 0, octaves, DETAIL_BASE_FREQ, seed.wrapping_add(2027));
        h = (e_raw + env * detail).max(h_floor);
        // Octave-stable river reference: wet-or-dry must be the SAME
        // decision at every LOD, or a coarse background tile paints a blue
        // river through a valley the near voxels render as a dry wash
        // (reported 2026-07-08, lat 9.75 lon 30.23). The perch test
        // therefore always reads the terrain at a FIXED octave depth,
        // regardless of how many octaves this caller carries for geometry.
        if hw > 0.0 && octaves != RIVER_REF_OCTAVES {
            let dref = rw
                * ridged_band(dir, 0, RIVER_REF_OCTAVES, DETAIL_BASE_FREQ, seed.wrapping_add(1013))
                + (0.95 - rw)
                    * fbm_band(dir, 0, RIVER_REF_OCTAVES, DETAIL_BASE_FREQ, seed.wrapping_add(2027));
            h_river_ref = (e_raw + env * dref).max(h_floor);
        } else {
            h_river_ref = h;
        }
    }

    // shore apron (Sol review finding 1, generalized by census): fine relief
    // digs dips just OUTSIDE a lake's flood boundary that the bake's
    // blended-raster renderability cap cannot see, and the consequence is
    // planet-scale — 683 liquid lakes end somewhere in a standing water
    // cliff (median worst wall 21 m; 143 m at 4.377 39.078). No level cap
    // can fix it: capping levels to the rendered dips measured 628 of 678
    // lakes bone dry (the flood territory inevitably crosses relief gorges
    // deeper than any viable level). So the GROUND yields instead: where
    // terrain just past the flood boundary would dip below the waterline,
    // it floors at the level and falls away at a gentle bank grade — water
    // always meets a shore, and terrain that never dipped is untouched.
    // LIQUID lakes only: the frozen wall families (S-3/W-5b) are a
    // deliberately-open aesthetic call, and their 600 m summit cliffs would
    // demand 20 km aprons. Octave-independent by construction (pure
    // function of the lake hit + level), so every LOD agrees on the shore.
    let mut on_apron_band = false;
    if lake_flood.is_none()
        && temp_c >= -4.0
        && let Some(hit) = &lake
    {
        // distance past the flood's outer edge: rivers.rs measures it
        // against the UNION of flood frontiers (lake Voronoi + every dam
        // rim's 1.15 r band) — a single-frontier metric left the floor
        // metres-to-kilometres down its grade right beside the water
        // (measured at 4.377 39.078 and 16.569 -32.262).
        let past = hit.apron_past_km;
        const APRON_GRADE: f64 = 0.030; // 3% bank fall-away
        let floor = hit.level_km - 0.001 - APRON_GRADE * past;
        if h < floor {
            h = floor;
            h_river_ref = h_river_ref.max(floor);
            on_apron_band = true;
        }
    }

    // carve the channel: parabolic bed guaranteed below the water line,
    // banks blending back to natural ground (wider where the cut is deep —
    // a river routed through a ridge digs a canyon, not a slot with
    // vertical km walls)
    if hw > 0.0 {
        out.river_dist_km = riv_d;
        out.river_hw_km = hw;
        out.river_level_km = wl;
        out.river_wet = 1.0 - smoothstep(0.002, 0.006, wl - h_river_ref);
        let bank_w = (hw * 0.9).max(0.02) + (h - wl).max(0.0) * 1.2;
        if riv_d < hw + bank_w {
            // the graph level never exceeds its own cell's bed elevation,
            // so perching only happens where raster and graph disagree —
            // fade to a dry wash rather than a standing wall of water.
            // Perch reads the octave-stable reference, not this LOD's h.
            let perch = smoothstep(0.002, 0.006, wl - h_river_ref);
            if riv_d < hw {
                let x = riv_d / hw;
                let bed = wl - 0.0012 - d_max * (1.0 - x * x);
                let target = bed.min(h);
                out.carve_km += h - target;
                h = target;
                if perch < 0.5 && (wl * 1000.0).floor() > ((h + 1e-6) * 1000.0).floor() {
                    out.water_km = wl;
                }
            } else {
                let t = smoothstep(0.0, 1.0, (riv_d - hw) / bank_w);
                let edge = wl - 0.0012;
                let target = (edge + (h - edge) * t).min(h);
                out.carve_km += h - target;
                h = target;
                // carved below the waterline IS riverbed: the bank blend
                // digs up to ~1.2 m under the water level, and leaving those
                // columns dry ringed every river with a strip of land one
                // block below its own surface — the universal one-block
                // shoreline lip (Austin, shot at 15.650 28.794). Flooding
                // them puts the shoreline exactly where the carved profile
                // crosses the level, which also rounds flush.
                if perch < 0.5 && (wl * 1000.0).floor() > ((h + 1e-6) * 1000.0).floor() {
                    out.water_km = wl;
                }
            }
            out.wet_soft =
                (1.0 - smoothstep(hw, (hw * 1.8).max(0.28), riv_d)) * (1.0 - perch);
        } else if wl.is_finite() {
            // beside the carve zone, natural relief dips below the waterline
            // are the river's own bathymetry — a dry pit sunk under the
            // water surface right against the channel (photographed at
            // 0.630 69.024; under the census's 2 m wall threshold, caught
            // by eye). A bounded bay band floods them — which also gives
            // banks natural irregular bays — and past it the bank apron
            // floors the ground at the waterline, falling away at the same
            // 3% grade the lake aprons use, so the flood edge always meets
            // a shore. Perch-gated like all river water: dry washes stay dry.
            let perch = smoothstep(0.002, 0.006, wl - h_river_ref);
            if perch < 0.5 {
                let edge = hw + bank_w;
                let bay_reach = edge * 0.6 + 0.010;
                if riv_d < edge + bay_reach {
                    if (wl * 1000.0).floor() > ((h + 1e-6) * 1000.0).floor() {
                        out.water_km = out.water_km.max(wl);
                    }
                } else {
                    let floor = wl - 0.001 - 0.030 * (riv_d - edge - bay_reach);
                    if h < floor {
                        h = floor;
                        h_river_ref = h_river_ref.max(floor);
                    }
                }
            }
        }
    }

    // lakes: fill to the spill level from the drainage graph. The bed is
    // the natural terrain — noise poking above the level makes islands.
    if let Some((lvl, salt, boundary_dist)) = lake_flood {
        out.lake_level_km = lvl;
        out.lake_boundary_dist_km = boundary_dist;
        // the OUTLET channel owns its own water: past the spill the river's
        // fill level descends with the terrain, and letting the flood lay a
        // tongue of lake-level water in that descending channel would end in
        // a water step at the flood boundary. In-lake reaches carry the lake
        // level (bake pins them), so this only exempts genuinely downstream
        // water.
        // 8 m margin: export smoothing/re-anchoring wobbles outlet levels a
        // few metres below the fill just short of the dam; a channel merely
        // metres under the lake level is SUBMERGED, not downstream — only a
        // clearly-descended reach owns its water.
        // The carve test extends ownership to the WHOLE carved gorge: bank
        // width grows with cut depth (bank_w above), so a deep descending
        // gorge reaches far past the fixed 1.5 hw strip — flood filling its
        // outer banks to lake level stood as a 79 m wall against the exempt
        // channel (census, 27.037 -35.188). Carve tapers to zero at the
        // bank's outer edge, so the flood/exempt handoff is smooth by
        // construction. At lake-check time carve_km is river-only (ponds
        // dig later).
        let river_owns = hw > 0.0
            && wl.is_finite()
            && wl < lvl - 0.008
            && (riv_d < hw * 1.5 || out.carve_km > 0.0005);
        if lvl > h + LAKE_WATER_CLEARANCE_KM && !river_owns {
            out.water_km = out.water_km.max(lvl);
            out.lake = true;
            out.salt = salt;
            out.wet_soft = out
                .wet_soft
                .max(smoothstep(LAKE_WATER_CLEARANCE_KM, 0.0035, lvl - h));
        }
    }

    // ponds: noise depressions in wet, calm lowlands fill to just below
    // the original ground line. Near-field only — coarse tiles can't
    // resolve them and would smear water across the landscape. NOT inside a
    // river's floodplain: the pond level rides e_raw (the 30 km raster),
    // but valley-flattened/carved ground follows the river's own level —
    // a pond blob overlapping the valley hung its flat surface up to ~20 m
    // above the valley floor as a standing wall (census, 4.993 -29.392).
    // ...and never into a lake-shore apron: the bank is structural fill
    // holding the waterline (a pond dug through it would re-open the very
    // dip the apron exists to cover)
    // ...and only where fine relief stays within BANKING distance of the
    // pool: the pond's level rides e_raw, so in craggy or high country
    // (env0 >= 0.13 — detail digs 100+ m) every blob edge is an unbankable
    // cliff. The census measured 300 m pond walls on 2-5 km mountainsides
    // the moment interior water was allowed there; no pond beats a broken
    // pond. env0 mirrors the octave block's relief envelope, un-valleyed.
    let env0 = (0.06 + rough_r * 0.85 + e_raw * 0.10).clamp(0.05, 1.7);
    // ...and only on quasi-LEVEL coarse ground: a pool at e_raw-1.8m on a
    // sloping raster is a terraced sheet whose every downhill edge hangs
    // (rough and env0 are blind to sub-cell raster cliffs — the census
    // found 50-100 m pond walls and >60k water-to-water jumps on calm-
    // rough coastal escarpments at e.g. 9.27 -76.81). Two taps per axis
    // ~0.5 km apart, gated at a 2% grade; clamped taps at face edges only
    // make the gate more permissive there, never wrong-sided.
    let pond_flat = || {
        let d = 1e-4; // ~0.5 km in face uv
        let gx = planet.elevation(face, u + d, v) - planet.elevation(face, u - d, v);
        let gy = planet.elevation(face, u, v + d) - planet.elevation(face, u, v - d);
        ((gx * gx + gy * gy) as f64).sqrt() < 0.02
    };
    // ...and never inside a lake's flood-eligible territory: the raster
    // blends an escarpment over ~12 km, so e_raw at a basin's rim can read
    // 55 m above the actual lake level — a pond anchored to it is fiction
    // hanging over the lake basin (census: pond@556 over lake@503.8 at
    // 9.27 -76.81, pond@130 over dry@30 at -21.11 -108.41)
    if octaves >= 8
        && precip > 550.0
        && rough_r < 0.45
        && e_raw > 0.03
        && env0 < 0.13
        && lake_flood.is_none()
        && !on_apron_band
        && pond_flat()
    {
        let pn = fbm_band(dir, 0, 2, 16000.0, seed.wrapping_add(9241));
        if pn < -0.50 && valley >= 0.999 {
            let pd = (-0.50 - pn) * 0.030;
            h -= pd;
            out.carve_km += pd;
            // FLAT surface: the pool level is the coarse (detail-free) land
            // elevation, not this column's own detailed ground — a pond is a
            // level water table, so its top must be constant across the basin.
            // Tying it to per-column `h` made the surface step down the fine
            // terracing (and any slope), which real ponds never do. The `pn`
            // mask still confines the water to the depression; e_raw removes
            // only the sub-pond wobble, leaving fine dips flooded and bumps as
            // tiny shore.
            let wl = e_raw - 0.0018;
            // moderate interior dips fill with GROUND to just under the
            // pool — an underwater bench, i.e. bathymetry — so the pool
            // covers them instead of walling around a dry pit (18 m wall
            // photographed at -0.798 -67.941). The budget matches the
            // edge apron's reach so the mask edge can always hand off;
            // dips deeper than it stay honest dry pits. (Flooding interior
            // dips UNCONDITIONALLY was measured catastrophic: blobs on
            // mountainsides filled 300 m relief gorges and the planet
            // census exploded from 38k to 79k pond walls.)
            // budget scales with the local relief envelope so it always
            // covers what detail can dig inside an env0-gated pond — a
            // fixed 30 m budget left a razor edge where a 31 m dip beside
            // a 30 m dip became an instant benched-water-vs-pit cliff
            // (census, 4.992 -29.395). NEVER bench ground that is already
            // underwater: a lake-flooded dip benched up through its own
            // lake surface grew a +54 m pond terrace on top of the lake
            // (census, 9.270 -76.780 — the old guard made pond-over-lake
            // unreachable by construction, and this gate preserves that).
            let bench = wl - 0.0015;
            if out.water_km <= h && h < bench && bench - h <= env0 * 1.5 {
                h = bench;
            }
            // never perched: the pool may not stand above this column's
            // NATURAL (post-bench) ground — a blob lapping onto a slope or
            // a valley rim otherwise hangs its flat surface metres above
            // the downhill terrain as a standing wall (census: 20 m pond
            // walls at 4.999 -29.391). Water that would drain does not
            // exist.
            if wl > h && wl <= h + pd + 0.002 {
                out.water_km = out.water_km.max(wl);
                out.wet_soft = out.wet_soft.max(smoothstep(0.0, 0.004, pd));
            }
        } else if out.carve_km <= 0.0 && riv_d > hw * 1.5 {
            // pond shore apron, the lake apron's little sibling: where
            // ground just OUTSIDE the water mask dips below the local pool
            // level, it floors at the pool and falls away as the mask
            // fades, so pond water meets a bank instead of standing as a
            // wall over a relief dip (18 m pond wall photographed at
            // -0.798 -67.941; ~38k pond walls in the planet census). pn is
            // the only distance proxy out here, and it is ~10x steeper
            // than its wavelength suggests, so the profile is quadratic:
            // ~12 m/pn at the shoreline (a ~6% metric grade — under the
            // census's 2 m-per-pair wall threshold) steepening with
            // distance so the far field still sinks below all but the
            // deepest relief (at pn = 0 the floor sits ~32 m under the
            // pool — it only acts on dips deeper than that, and nothing
            // needs an outer bound). Unlike the dig, the apron is NOT
            // valley-gated: pond water stops hard at the valley gate while
            // the ground descends toward the river, and that seam stood as
            // the same wall (the in-blob x<0 clamp keeps the in-valley
            // floor at pool level, continuous with the pond edge). The
            // carve/ownership guards keep the bank out of river channels.
            let wl = e_raw - 0.0018;
            let x = (pn + 0.50).max(0.0);
            // profile scaled by the relief envelope (env0 ~0.10 in classic
            // pond country leaves it as written): the bank must be able to
            // descend as deep as detail digs before the profile runs out
            let scale = env0 * 10.0;
            let floor = wl - 0.001 - (0.012 * x + 0.090 * x * x) * scale;
            if h < floor {
                h = floor;
            }
        }
    }

    out.h_km = h;
    out
}

/// Back-compat shim: terrain surface height (km) and raw elevation.
pub fn sample_height(planet: &Planet, face: usize, u: f64, v: f64, octaves: u32) -> (f64, f64) {
    let s = sample(planet, face, u, v, octaves);
    (s.render_h_km(), s.e_raw)
}

/// Painted wetness for a (non-deep) tile vertex: continuous feathered
/// wetness, with the river thread widened to at least one vertex spacing so
/// distant threads stay continuous instead of shattering. Level-dependent
/// through `spacing_km` — which is why LOD swaps pop without wet morphing.
fn tile_wet(s: &Sample, spacing_km: f64) -> f64 {
    if s.sea || s.lake {
        return 0.0;
    }
    let wide = if s.river_dist_km.is_finite() {
        let paint_w = s.river_hw_km.max(spacing_km * 0.9);
        // coverage-correct opacity: the corridor is widened to the vertex
        // spacing for continuity, so paint it only as strongly as the real
        // channel fills it (sqrt for perceptual balance). A 30 m stream
        // across an 800 m corridor becomes a faint thread instead of a
        // full-strength band — which is what made every confluence bloom
        // into a blob: the union of two full-opacity widened corridors.
        // Big rivers (hw ~ spacing) still paint at full strength, and the
        // geomorph wet-lerp fades threads smoothly across LOD switches.
        let coverage = (s.river_hw_km / paint_w).min(1.0).sqrt();
        // hand-off to the per-pixel shore field (Vertex::shore): once the
        // channel out-resolves the vertex grid the field draws the bank at
        // pixel precision, and the paint's vertex-interpolated edge only
        // smears past it in triangle-shaped teeth (Austin's
        // river-zoom-2.png). Wide rivers mute the paint and let the field
        // own the water; the paint's remaining job is sub-vertex threads.
        let field_owns = smoothstep(0.9, 2.0, s.river_hw_km / spacing_km);
        (1.0 - smoothstep(s.river_hw_km, paint_w, s.river_dist_km))
            * s.river_wet
            * coverage
            * (1.0 - field_owns)
    } else {
        0.0
    };
    // ponds get the same treatment: their feather scale is ~0.28 km, so on
    // tiles whose vertices are further apart than that, lone vertices catch
    // a pond and paint whole angular triangles — fade them by coverage too
    let pond_cov = (0.28 / spacing_km).min(1.0).sqrt();
    (s.wet_soft * pond_cov).max(wide * 0.9)
}

const SHORE_CLAMP_KM: f64 = 0.005;

/// Whether this vertex's liquid lake/river classification can depend on the
/// procedural ground octave budget. Sea is deliberately absent: ocean
/// samples return before detail noise, while the dry side of a coast is
/// classified by the octave-independent raster coastline and elevation.
fn needs_voxel_shore_reference(s: &Sample, octaves: u32) -> bool {
    if s.temp_c < -4.0 {
        return false;
    }
    let river = s.river_level_km.is_finite()
        && s.river_wet > 0.5
        && s.river_dist_km < s.river_hw_km * 3.0;
    if river || !s.lake_level_km.is_finite() {
        return river;
    }

    // Bound the omitted lake-ground detail from the same local relief
    // envelope used by `sample`. Valley damping can only make it smaller.
    // Two is deliberately conservative for one band's roughly [-1, 1]
    // blended noise. Sampling outside this margin cannot change either the
    // lake predicate or the already-clamped shore value.
    let rough_r = s.rough * smoothstep(0.02, 0.30, s.e_raw);
    let mut envelope = (0.06 + rough_r * 0.85 + s.e_raw * 0.10).clamp(0.05, 1.7);
    let submerge = smoothstep(-0.005, 0.012, s.lake_level_km - s.e_raw);
    envelope *= 1.0 - 0.88 * submerge;
    let first = octaves.min(VOXEL_OCTAVES) as i32;
    let count = VOXEL_OCTAVES.saturating_sub(octaves) as i32;
    let missing_weight = 2f64.powi(-first) * 2.0 * (1.0 - 0.5f64.powi(count));
    let omitted_detail_bound = envelope * missing_weight * 2.0;
    let lake_delta = s.lake_level_km - s.h_km - LAKE_WATER_CLEARANCE_KM;
    lake_delta.abs() <= SHORE_CLAMP_KM + omitted_detail_bound
}

/// Complete a lake-only geometry sample to voxel octave depth without
/// repeating its raster, lake-index, and already-computed noise work. Within
/// flood territory and away from rivers, `sample` applies no later ground
/// transform (ponds are excluded), so adding bands [octaves, 12) is exact
/// apart from irrelevant floating-point reassociation. River overlap and a
/// newly-wet salt lake fall back to the authoritative full sampler.
fn voxel_shore_reference(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    octaves: u32,
    s: &Sample,
) -> Sample {
    if s.lake_level_km.is_finite() && !s.river_dist_km.is_finite() {
        let ofrac = planet.ocean(face, u, v) as f64;
        let h_floor = if ofrac > 0.02 { 0.002 } else { -9.0 };
        // A capped value pinned to the coastal floor has lost the underlying
        // sum, so it cannot be completed incrementally.
        if octaves == 0 || s.h_km > h_floor + f64::EPSILON {
            let dir = face_dir(face, u, v);
            let rough_r = s.rough * smoothstep(0.02, 0.30, s.e_raw);
            let mut envelope =
                (0.06 + rough_r * 0.85 + s.e_raw * 0.10).clamp(0.05, 1.7);
            let submerge = smoothstep(-0.005, 0.012, s.lake_level_km - s.e_raw);
            envelope *= 1.0 - 0.88 * submerge;
            let rw = (0.30 + rough_r * 0.50).clamp(0.30, 0.72);
            let missing = VOXEL_OCTAVES.saturating_sub(octaves);
            let detail = rw
                * ridged_band(
                    dir,
                    octaves,
                    missing,
                    DETAIL_BASE_FREQ,
                    planet.seed.wrapping_add(1013),
                )
                + (0.95 - rw)
                    * fbm_band(
                        dir,
                        octaves,
                        missing,
                        DETAIL_BASE_FREQ,
                        planet.seed.wrapping_add(2027),
                    );
            let mut out = *s;
            out.h_km = (s.h_km + envelope * detail).max(h_floor);
            let lake = s.lake_level_km > out.h_km + LAKE_WATER_CLEARANCE_KM;
            // A dry->wet transition needs the lake hit's salt bit, which a
            // dry Sample intentionally does not retain.
            if lake && !s.lake {
                return sample(planet, face, u, v, VOXEL_OCTAVES);
            }
            out.lake = lake;
            out.water_km = if lake { s.lake_level_km } else { f64::NEG_INFINITY };
            out.salt = lake && s.salt;
            return out;
        }
    }
    sample(planet, face, u, v, VOXEL_OCTAVES)
}

/// The color/class shoreline field. `s` is the voxel-octave reference for
/// liquid lakes and rivers, but may be the geometry sample when that sample
/// is already full-depth or when the reference cannot affect this vertex.
fn shore_field(planet: &Planet, face: usize, u: f64, v: f64, s: &Sample) -> f32 {
    // The raster/mask is the actual sea-side authority (interior dry basins
    // may sit below zero), and both values are octave-independent. Using it
    // also preserves the smooth, stable coastal crossing without re-running
    // the full sampler throughout open ocean.
    let shore_sea = if s.sea || planet.ocean(face, u, v) > 0.02 {
        -s.e_raw
    } else {
        f64::NEG_INFINITY
    };
    let shore_lake = if s.lake_level_km.is_finite() && s.temp_c >= -4.0 {
        // `sample` deliberately requires 0.5 m of clearance before a lake
        // exists (line 561). Use that same predicate boundary here: stepping
        // raw level-ground at zero paints broad sub-threshold shoals as water
        // even though voxel columns correctly materialize them as dry land.
        s.lake_level_km - s.h_km - LAKE_WATER_CLEARANCE_KM
    } else {
        f64::NEG_INFINITY
    };
    // The carve puts the bed below wl inside an active channel and the banks
    // above it outside, so wl - h crosses at the rendered river edge. The
    // fixed river_wet perch decision and 3 hw bound keep dry washes and
    // unrelated terrain dips out of the field.
    let shore_river = if s.river_level_km.is_finite()
        && s.river_wet > 0.5
        && s.temp_c >= -4.0
        && s.river_dist_km < s.river_hw_km * 3.0
    {
        s.river_level_km - s.h_km
    } else {
        f64::NEG_INFINITY
    };
    // Clamp TIGHT (±5 m): a vertex without water data must sit at a gentle
    // -5 m rather than a remote sentinel. A large negative jump skews the
    // interpolated crossing toward that vertex and cuts vertex-scale notches.
    shore_sea
        .max(shore_lake)
        .max(shore_river)
        .clamp(-SHORE_CLAMP_KM, SHORE_CLAMP_KM) as f32
}

/// Build the mesh for one tile. Positions are computed on a grid with one
/// ghost ring so normals use central differences everywhere — one-sided
/// normals at tile borders leave visible lighting seams between tiles.
pub fn build_tile(planet: &Planet, key: TileKey, exaggeration: f64) -> TileMesh {
    let n = TILE_QUADS + 1;
    let np2 = n + 2;
    let (u0, v0, size) = key.uv_range();
    let radius = planet.radius_km;
    let origin = key.center_dir() * radius;
    let octaves = if key.deep { VOXEL_OCTAVES } else { octave_count(key.level, radius) };
    let face = key.face as usize;

    let mut world = vec![DVec3::ZERO; np2 * np2];
    let mut samples = Vec::with_capacity(np2 * np2);
    for gj in 0..np2 {
        for gi in 0..np2 {
            let u = u0 + size * (gi as f64 - 1.0) / TILE_QUADS as f64;
            let v = v0 + size * (gj as f64 - 1.0) / TILE_QUADS as f64;
            let dir = face_dir(face, u, v);
            let s = sample(planet, face, u, v, octaves);
            world[gj * np2 + gi] = dir * (radius + s.render_h_km() * exaggeration);
            samples.push(s);
        }
    }

    // Geomorph targets: what the PARENT level renders here. Parent vertices
    // sit on this tile's even lattice (grids nest). Interpolate their height
    // and wetness across the same b-c diagonal as the index buffer below;
    // bilerp would put the odd/odd child vertex on a different surface by the
    // quad's twist term. When octave budgets match, reuse the child samples at
    // the even vertices rather than re-sampling them.
    let parent_oct = if key.level == 0 {
        octaves
    } else {
        octave_count(key.level - 1, radius)
    };
    let spacing = key.size_km(radius) / TILE_QUADS as f64;
    let half = TILE_QUADS / 2 + 1; // 17 parent-lattice points per axis
    let mut hp = vec![0.0f64; half * half];
    let mut wp = vec![0.0f64; half * half];
    if key.level > 0 {
        for pj in 0..half {
            for pi in 0..half {
                let k = pj * half + pi;
                if parent_oct == octaves {
                    let s = &samples[(2 * pj + 1) * np2 + 2 * pi + 1];
                    hp[k] = s.render_h_km();
                    wp[k] = tile_wet(s, spacing * 2.0);
                } else {
                    let u = u0 + size * (2 * pi) as f64 / TILE_QUADS as f64;
                    let v = v0 + size * (2 * pj) as f64 / TILE_QUADS as f64;
                    let s = sample(planet, face, u, v, parent_oct);
                    hp[k] = s.render_h_km();
                    wp[k] = tile_wet(&s, spacing * 2.0);
                }
            }
        }
    }
    let parent_value = |grid: &[f64], i: usize, j: usize| -> f64 {
        let (pi, fi) = (i / 2, (i % 2) as f64 * 0.5);
        let (pj, fj) = (j / 2, (j % 2) as f64 * 0.5);
        let (pi1, pj1) = ((pi + 1).min(half - 1), (pj + 1).min(half - 1));
        let (a, b, c, d) = (
            grid[pj * half + pi],
            grid[pj * half + pi1],
            grid[pj1 * half + pi],
            grid[pj1 * half + pi1],
        );
        if fi + fj <= 1.0 {
            a * (1.0 - fi - fj) + b * fi + c * fj
        } else {
            b * (1.0 - fj) + d * (fi + fj - 1.0) + c * (1.0 - fi)
        }
    };

    let mut vertices = Vec::with_capacity(n * n + 4 * n);
    for j in 0..n {
        for i in 0..n {
            let (gi, gj) = (i + 1, j + 1);
            let l = world[gj * np2 + gi - 1];
            let r = world[gj * np2 + gi + 1];
            let d = world[(gj - 1) * np2 + gi];
            let u_ = world[(gj + 1) * np2 + gi];
            let nrm = (r - l).cross(u_ - d).normalize_or_zero();
            let p = world[gj * np2 + gi] - origin;
            let uu = u0 + size * i as f64 / TILE_QUADS as f64;
            let vv = v0 + size * j as f64 / TILE_QUADS as f64;
            let s = &samples[gj * np2 + gi];
            // surface slope for rock exposure: radial up vs mesh normal
            let up = world[gj * np2 + gi].normalize();
            let slope = 1.0 - nrm.dot(up).clamp(0.0, 1.0);
            // Shore is a COLOR/CLASS channel, so liquid lake/river crossings
            // read the same full-octave ground as voxel columns even though
            // this tile's positions, normals, paint, water plane, and morph
            // data remain honestly spacing-capped. Restrict the extra sample
            // to vertices where those fields can participate; doing it for
            // every vertex would defeat the geometry octave cap's cost bound.
            let shore_reference = (octaves != VOXEL_OCTAVES
                && needs_voxel_shore_reference(s, octaves))
            .then(|| voxel_shore_reference(planet, face, uu, vv, octaves, s));
            let class_s = shore_reference.as_ref().unwrap_or(s);
            let wc = water_color(class_s);
            // deep tiles resolve water: binary flag, crisp step in the
            // shader. Far tiles get the continuous feathered wetness. The
            // sea carries NO wet paint: its geometry+ground color already
            // are the water, and paint only bleeds navy bands onto the
            // beach triangles next door.
            let wet = if key.deep && !(s.sea || s.lake) {
                s.has_water() as u32 as f64
            } else {
                tile_wet(s, spacing)
            };
            let wet_parent = if key.level > 0 {
                parent_value(&wp, i, j)
            } else {
                wet
            };
            // the sea is real geometry at its surface — its "ground" color
            // is the water color so the wetness blend is a no-op there and
            // it stays fully blue at every distance
            let ground = if class_s.sea || class_s.lake {
                // a frozen sheet is solid walkable ice — give it a snow-dusted,
                // LOD-stable surface so it reads as ground, not a flat plane
                if class_s.temp_c < -4.0 {
                    frost_color(world[gj * np2 + gi])
                } else {
                    wc
                }
            } else {
                shade_ground(planet, face, uu, vv, class_s, slope)
            };
            let dh = if key.level > 0 {
                ((parent_value(&hp, i, j) - s.render_h_km()) * exaggeration) as f32
            } else {
                0.0
            };
            let shore = shore_field(planet, face, uu, vv, class_s);
            vertices.push(Vertex {
                pos: [p.x as f32, p.y as f32, p.z as f32],
                normal: [nrm.x as f32, nrm.y as f32, nrm.z as f32],
                color: ground,
                water: [wc[0], wc[1], wc[2], wet as f32],
                morph_dh: dh,
                morph_wet: wet_parent as f32,
                wflag: if s.sea || s.lake { 1.0 } else { 0.0 },
                shore,
            });
        }
    }

    let mut indices = Vec::with_capacity(TILE_QUADS * TILE_QUADS * 6 + 8 * TILE_QUADS * 6);
    let idx = |i: usize, j: usize| (j * n + i) as u32;
    for j in 0..TILE_QUADS {
        for i in 0..TILE_QUADS {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i, j + 1), idx(i + 1, j + 1));
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }

    // skirts: border vertices pulled toward the planet center hide the
    // sub-meter cracks from per-tile f32 rounding and LOD T-junctions
    let drop = (key.size_km(radius) * 0.05).max(0.05);
    let border: Vec<u32> = (0..n).map(|i| idx(i, 0))
        .chain((0..n).map(|i| idx(i, n - 1)))
        .chain((0..n).map(|j| idx(0, j)))
        .chain((0..n).map(|j| idx(n - 1, j)))
        .collect();
    for &b in &border {
        let v = vertices[b as usize];
        let p = DVec3::new(v.pos[0] as f64, v.pos[1] as f64, v.pos[2] as f64) + origin;
        let pulled = p - p.normalize() * drop - origin;
        vertices.push(Vertex {
            pos: [pulled.x as f32, pulled.y as f32, pulled.z as f32],
            normal: v.normal,
            color: v.color,
            water: v.water,
            // skirts morph with their border vertex so no gap opens
            morph_dh: v.morph_dh,
            morph_wet: v.morph_wet,
            wflag: v.wflag,
            shore: v.shore,
        });
    }
    let skirt_base = (n * n) as u32;
    let seg = n as u32;
    for side in 0..4u32 {
        for t in 0..(n - 1) as u32 {
            let (t0, t1) = (side * seg + t, side * seg + t + 1);
            let (o0, o1) = (border[t0 as usize], border[t1 as usize]);
            let (s0, s1) = (skirt_base + t0, skirt_base + t1);
            // culling is off, winding doesn't matter — one quad per segment
            indices.extend_from_slice(&[o0, o1, s0, o1, s1, s0]);
        }
    }

    // ---- forest impostors (TRANSITIONS.md E, Andrew-greenlit) ----
    // The same trees the voxel patch grows, as crossed billboard quads on
    // the two finest mesh levels: the SAME placement lottery, species mix,
    // trunk heights, and leaf palette (voxel::tree_* / Mat::color), so a
    // forest keeps its density, color, and silhouettes from the patch rim
    // out to ~5-10 km, then hands off to the vegetation tint. Inside the
    // voxel patch the fragment shader's hole cut discards them — blocks own
    // the near trees, no double-draw, and the handoff line is the same
    // feathered rim the terrain already uses. Enumeration rides a strided
    // column lattice with density scaled by stride² (statistically the same
    // forest at a mesh vertex budget; per-tree identity only matters where
    // blocks take over, and there the hole owns the view). Two phases so
    // the expensive terrain sample runs only on budget survivors.
    let impostor_stride: u64 = match key.level {
        14 => 1,
        13 => 2,
        12 => 4,
        11 => 8,
        _ => 0,
    };
    if impostor_stride > 0 {
        // trees per tile: the vertex/fill budget knob. Austin measured the
        // frame rate sagging at mid distance on the RTX 2060 at 4000/4000;
        // level 11 (whose trees are 1-3 px) carries a lighter load
        let impostor_cap: usize = if key.level == 12 { 2600 } else { 1400 };
        let s = impostor_stride;
        let nn = crate::voxel::COLUMNS_PER_FACE;
        let nnf = nn as f64;
        let to_col = |x: f64| (((x + 1.0) * 0.5 * nnf).floor().clamp(0.0, nnf - 1.0)) as u64;
        let (ci0, ci1) = (to_col(u0), to_col(u0 + size));
        let (cj0, cj1) = (to_col(v0), to_col(v0 + size));
        let seed = planet.seed;
        let comp = (s * s) as f64; // stride density compensation
        let mut cands: Vec<(u64, u64, crate::voxel::TreeKind, f64)> = Vec::new();
        for ci in (ci0..=ci1).step_by(s as usize) {
            for cj in (cj0..=cj1).step_by(s as usize) {
                let lot = crate::voxel::tree_hash01(face as u8, ci, cj, seed);
                if lot >= 0.011 * comp {
                    continue; // cheapest gate: above every biome's density
                }
                let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nnf;
                let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nnf;
                let Some((kind, density)) =
                    crate::voxel::tree_kind_density(planet.koppen(face, u, v))
                else {
                    continue;
                };
                // shrubs are ground texture, not a silhouette at range
                if kind == crate::voxel::TreeKind::Shrub || lot >= density * comp {
                    continue;
                }
                cands.push((ci, cj, kind, lot));
            }
        }
        let keep_every = cands.len().div_ceil(impostor_cap).max(1);
        // decimation past the cap keeps visual mass by growing the kept
        // trees (area-conserving sqrt, capped before they read as blobs)
        let boost = (keep_every as f64).sqrt().min(2.2);
        for (ci, cj, kind, lot) in cands.into_iter().step_by(keep_every) {
            let u = -1.0 + 2.0 * (ci as f64 + 0.5) / nnf;
            let v = -1.0 + 2.0 * (cj as f64 + 0.5) / nnf;
            // one real sample per survivor: correct rooting on THIS tile's
            // surface (same octave budget) + the guards tree_at applies
            let smp = sample(planet, face, u, v, octaves);
            if smp.has_water() || smp.e_raw < 0.010 || smp.carve_km > 0.0005 {
                continue;
            }
            if smp.temp_c < -6.0 || smp.temp_c < -11.0 {
                continue;
            }
            let trunk = crate::voxel::tree_trunk(kind, face as u8, ci, cj);
            use crate::voxel::{Mat, TreeKind};
            // width/taper give each species its silhouette: conifers pinch
            // to spires, broadleaf/jungle round off, acacias flare into the
            // umbrella crown — flat rectangles read as a picket fence
            // sizes ~15% under the voxel canopy footprint: billboards fill
            // their whole quad while voxel canopies are airy block piles, so
            // equal dimensions read LARGER (Austin's field report)
            let (canopy_km, half_w_km, taper, leaf) = match kind {
                TreeKind::Jungle => (0.0052, 0.0023, 0.75, Mat::LeavesJungle),
                TreeKind::Conifer => (0.0043, 0.0016, 0.12, Mat::LeavesConifer),
                TreeKind::Broadleaf => (0.0038, 0.0018, 0.65, Mat::LeavesBroad),
                TreeKind::Acacia => (0.0030, 0.0017, 1.60, Mat::LeavesAcacia),
                TreeKind::Shrub => continue,
            };
            let dir = face_dir(face, u, v);
            // sink slightly so slopes don't leave floating root gaps
            let root = dir * (radius + smp.render_h_km() * exaggeration - 0.0008) - origin;
            // decimation boost conserves canopy AREA via width only —
            // heights stay true so the rim handoff keeps its skyline
            let hgt = (trunk as f64 * 0.001 + canopy_km) * exaggeration * boost.powf(0.25);
            let wid = half_w_km * boost;
            let ax = if dir.z.abs() < 0.9 { DVec3::Z } else { DVec3::Y };
            let e1 = (ax - dir * ax.dot(dir)).normalize();
            let e2 = dir.cross(e1);
            // per-tree brightness variation, like the voxel canopy speckle.
            // NO bark: a distant forest is a canopy sea (a bark-bottomed
            // gradient made whole rim bands read brown — overlapping
            // billboards stack far trees' bark above near trees' crowns).
            // The darkened base fakes the under-canopy shadow instead.
            let shade = 0.82 + 0.36 * (lot * 97.31).fract() as f32;
            let lc = leaf.color([0.0; 3]);
            let canopy = [lc[0] * shade * 1.08, lc[1] * shade * 1.08, lc[2] * shade * 1.08];
            let under = [lc[0] * shade * 0.45, lc[1] * shade * 0.45, lc[2] * shade * 0.45];
            let nrm = [dir.x as f32, dir.y as f32, dir.z as f32];
            for axis in [e1, e2] {
                let base_i = vertices.len() as u32;
                let wt = wid * taper;
                let quad = [
                    (root - axis * wid, under),
                    (root + axis * wid, under),
                    (root + axis * wt + dir * hgt, canopy),
                    (root - axis * wt + dir * hgt, canopy),
                ];
                for (p, col) in quad {
                    vertices.push(Vertex {
                        pos: [p.x as f32, p.y as f32, p.z as f32],
                        normal: nrm,
                        color: col,
                        water: [0.0; 4],
                        morph_dh: 0.0,
                        morph_wet: 0.0,
                        wflag: 0.0,
                        shore: -1.0,
                    });
                }
                indices.extend_from_slice(&[
                    base_i,
                    base_i + 1,
                    base_i + 2,
                    base_i,
                    base_i + 2,
                    base_i + 3,
                ]);
            }
        }
    }

    TileMesh { origin_km: origin, vertices, indices }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Water color by depth: true ocean depth for the sea, carved depth for
/// rivers and ponds.
/// Surface color for a FROZEN sea/lake vertex — a solid, walkable ice sheet.
/// A single flat color read as empty sky or liquid; this dusts it snow-white
/// with a smooth drift so it reads as ground. The drift is a function of world
/// position only (not tile level), so it is identical for a point at every LOD
/// and morphs without shimmer. Height morphing shifts `p` by sub-meter amounts
/// against km-scale arguments, so the pattern is effectively frozen in place.
fn frost_color(p: glam::DVec3) -> [f32; 3] {
    let d = ((p.x * 47.0).sin() * (p.y * 43.0).sin() * (p.z * 41.0).sin()
        + 0.5 * (p.x * 131.0 + p.y * 109.0).sin()) as f32;
    let t = (0.5 + 0.35 * d).clamp(0.0, 1.0);
    let ice = [0.60, 0.70, 0.82];
    let snow = [0.85, 0.88, 0.92];
    [
        ice[0] + (snow[0] - ice[0]) * t,
        ice[1] + (snow[1] - ice[1]) * t,
        ice[2] + (snow[2] - ice[2]) * t,
    ]
}

/// THE beach decision (TRANSITIONS.md): sand on low coastal ground, one
/// fraction for both renderers — the mesh mixes its tint by it, the blocks
/// dither their material on it — so the patch rim cannot disagree about
/// where the beach is. The old pair disagreed by construction: blocks
/// tested `e_raw < 12 m AND surface < 14 blocks` hard while the mesh
/// ramped on e_raw alone capped at 90% — a full-sand voxel disk on mostly
/// grass mesh at every low coast (the v1_color pose, 0.569 68.915).
/// Bands feather over ~2 m so the dithered edge reads as an ecotone.
/// Callers yield to `lake_shore_frac` wherever a finite lake level makes the
/// lake rule the material owner; otherwise this generic low coast could
/// repaint a bounded lake shore as a province.
pub fn beach_frac(e_raw_km: f64, h_km: f64) -> f64 {
    let by_raster = 1.0 - smoothstep(0.010, 0.012, e_raw_km);
    let by_surface = 1.0 - smoothstep(0.012, 0.014, h_km);
    by_raster * by_surface
}

const LAKE_SHORE_HEIGHT_KM: f64 = 0.0015;
// Keep a solid core through one ~100 m mesh-vertex spacing at the 2 km visual
// reference, then feather for another 200 m. Beyond that it is country, not
// beach, even if a coarse flood-eligible sample happens to hug lake level.
const LAKE_SHORE_SOLID_KM: f64 = 0.100;
const LAKE_SHORE_REACH_KM: f64 = 0.300;

/// THE lake-shore material decision (BUGS.md V-7): liquid-lake sand on dry
/// ground in a small height band above the local waterline AND near the baked
/// lake footprint's actual lake/rim edge. The height half preserves the old
/// voxel column semantics; the bounded edge half prevents coarse mesh samples
/// from painting a whole flood-eligible territory as sand. Mesh vertices mix
/// by this fraction and voxel columns dither on it, matching `beach_frac`.
/// Height is full at the waterline and zero 1.5 m above it; edge distance is
/// full through 100 m and zero by 300 m. Frozen lakes return zero so their
/// existing ice/snow shores stay untouched.
pub fn lake_shore_frac(
    temp_c: f64,
    h_km: f64,
    lake_level_km: f64,
    lake_boundary_dist_km: f64,
) -> f64 {
    if temp_c < -4.0
        || !lake_level_km.is_finite()
        || !lake_boundary_dist_km.is_finite()
        || h_km < lake_level_km
    {
        return 0.0;
    }
    let by_height = 1.0 - smoothstep(0.0, LAKE_SHORE_HEIGHT_KM, h_km - lake_level_km);
    let by_edge =
        1.0 - smoothstep(LAKE_SHORE_SOLID_KM, LAKE_SHORE_REACH_KM, lake_boundary_dist_km);
    by_height * by_edge
}

/// THE liquid water surface color (TRANSITIONS.md F): one ramp for both
/// renderers. The mesh and the blocks each kept a copy "in sync" by hand,
/// and they had already drifted (deep base 0.28 vs 0.30 blue — the exact
/// failure mode this function retires). Depth in km (true ocean depth for
/// the sea, carved/filled depth for rivers, lakes, ponds); `sea` widens
/// the shallow teal shoal; salt goes mineral-pale. Frozen sheets are NOT
/// this function's job (blocks use Mat::Ice + snow dusting, the mesh uses
/// frost_color — unify those when a real need shows).
pub fn water_surface_color(depth_km: f64, sea: bool, salt: bool) -> [f32; 3] {
    let t = (depth_km / 2.5).clamp(0.0, 1.0) as f32;
    let mut c = mix3([0.055, 0.17, 0.30], [0.004, 0.013, 0.055], t);
    if sea {
        // first ~20 m shoal to a lighter teal
        let sh = (1.0 - depth_km / 0.02).clamp(0.0, 1.0) as f32;
        c = mix3(c, [0.10, 0.32, 0.35], sh * 0.7);
    }
    if salt {
        // salt lakes read mineral-pale, almost milky
        c = mix3(c, [0.45, 0.55, 0.52], 0.55);
    }
    c
}

fn water_color(s: &Sample) -> [f32; 3] {
    if s.temp_c < -4.0 {
        return [0.60, 0.72, 0.85]; // frozen — matches Mat::Ice on the blocks
    }
    let depth = if s.sea { -s.e_raw } else { s.water_km - s.h_km };
    water_surface_color(depth, s.sea, s.salt)
}

/// Naturalistic ground shading for the far-field mesh: biome ground tint,
/// sand near sea level, bare rock on steep ground in rough country, snow
/// where it's cold. Kept in the same family as the voxel materials so the
/// block patch doesn't read as a different planet. (Water rides a separate
/// vertex channel — see Vertex::water.)
fn shade_ground(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    s: &Sample,
    slope: f64,
) -> [f32; 3] {
    let climate = climate_surface(planet, face, u, v, s.temp_c, s.precip);
    let mut c = climate.tint(climate.main_block);
    // forested biomes read darker from afar (canopy self-shadowing), so the
    // tree-covered voxel patch doesn't pop out of a flat bright lawn. The
    // shared context blends this weight so it cannot restore a class line.
    let forest = if climate.main_block == MainBlock::Grass {
        climate.forest
    } else {
        0.0
    };
    let dark = 1.0 - 0.22 * forest;
    c = [c[0] * dark, c[1] * dark, c[2] * dark];
    // beach sand on low coastal ground — the SAME fraction the blocks
    // dither their material on (beach_frac), mixed at full strength so a
    // frac-1 beach is exactly the blocks' Mat::Sand tint
    // A finite lake level means the lake-specific rule below owns sand here.
    // Letting the generic low-elevation beach fall through would immediately
    // repaint a near-sea-level lake's whole territory and make the bounded
    // V-7 rule a visual no-op (the 47.80, 14.42 lagoon repro).
    let sandy = if s.lake_level_km.is_finite() {
        0.0
    } else {
        beach_frac(s.e_raw, s.h_km) as f32
    };
    let sand = climate.tint(MainBlock::Sand);
    c = mix3(c, sand, sandy);
    // bare rock only where the ground is actually steep (like the blocks,
    // which rock by per-column steepness) — jagged-map areas rock sooner.
    // Blanket-graying whole rough biomes made the far mesh a different
    // planet from the vivid block patch.
    let rough_r = s.rough * smoothstep(0.02, 0.30, s.e_raw);
    let rocky = ((rough_r * 0.9 - 0.05).clamp(0.0, 0.65)
        * smoothstep(0.25, 0.60, slope)) as f32;
    c = mix3(c, [0.23, 0.22, 0.21], rocky);
    // Snow uses the same world-column hash and threshold as the voxels.
    // Re-apply after rock/beach because voxel surface_mat gives snow priority.
    if climate.main_block == MainBlock::Snow {
        c = climate.tint(MainBlock::Snow);
    }
    // Lake-shore sand uses the SAME fraction the blocks dither on. Apply
    // after rock/snow so barely-emergent liquid-lake shoals read as sandbars.
    let lake_shore = lake_shore_frac(
        s.temp_c,
        s.h_km,
        s.lake_level_km,
        s.lake_boundary_dist_km,
    ) as f32;
    c = mix3(c, sand, lake_shore);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_planet() -> &'static Planet {
        static PLANET: std::sync::OnceLock<Planet> = std::sync::OnceLock::new();
        PLANET.get_or_init(|| {
            let assets = if std::path::Path::new("assets/meta.json").exists() {
                "assets"
            } else {
                "viewer/assets"
            };
            Planet::load(assets).expect("terrain mesh gates require the baked viewer assets")
        })
    }

    fn mesh_world(mesh: &TileMesh, i: usize, j: usize) -> DVec3 {
        mesh_world_index(mesh, j * (TILE_QUADS + 1) + i)
    }

    fn mesh_world_index(mesh: &TileMesh, index: usize) -> DVec3 {
        let v = mesh.vertices[index];
        mesh.origin_km + DVec3::from_array(v.pos.map(f64::from))
    }

    /// Find the actual parent index-buffer triangle containing a child point.
    /// Reading the indices makes this gate fail if the rendered diagonal ever
    /// changes without the morph-target interpolation changing with it.
    fn parent_triangle_weights(
        parent: &TileMesh,
        child: TileKey,
        i: usize,
        j: usize,
    ) -> [(usize, f64); 3] {
        let n = TILE_QUADS + 1;
        let x = (child.ix as usize & 1) as f64 * (TILE_QUADS / 2) as f64
            + i as f64 * 0.5;
        let y = (child.iy as usize & 1) as f64 * (TILE_QUADS / 2) as f64
            + j as f64 * 0.5;
        let cell_i = (x.floor() as usize).min(TILE_QUADS - 1);
        let cell_j = (y.floor() as usize).min(TILE_QUADS - 1);
        let first = (cell_j * TILE_QUADS + cell_i) * 6;
        for tri in 0..2 {
            let indices = [
                parent.indices[first + tri * 3] as usize,
                parent.indices[first + tri * 3 + 1] as usize,
                parent.indices[first + tri * 3 + 2] as usize,
            ];
            let uv = indices.map(|index| ((index % n) as f64, (index / n) as f64));
            let ab = (uv[1].0 - uv[0].0, uv[1].1 - uv[0].1);
            let ac = (uv[2].0 - uv[0].0, uv[2].1 - uv[0].1);
            let ap = (x - uv[0].0, y - uv[0].1);
            let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
            let den = cross(ab, ac);
            let wb = cross(ap, ac) / den;
            let wc = cross(ab, ap) / den;
            let wa = 1.0 - wb - wc;
            if wa >= -1e-12 && wb >= -1e-12 && wc >= -1e-12 {
                return [(indices[0], wa), (indices[1], wb), (indices[2], wc)];
            }
        }
        panic!("child point ({i}, {j}) is outside both parent cell triangles");
    }

    fn full_morph_residuals(planet: &Planet, key: TileKey) -> (f64, f64) {
        let child = build_tile(planet, key, 1.0);
        let parent = build_tile(
            planet,
            TileKey {
                face: key.face,
                level: key.level - 1,
                ix: key.ix / 2,
                iy: key.iy / 2,
                deep: false,
            },
            1.0,
        );
        let mut max_position_km = 0.0f64;
        let mut max_wet = 0.0f64;
        for j in 0..=TILE_QUADS {
            for i in 0..=TILE_QUADS {
                let v = child.vertices[j * (TILE_QUADS + 1) + i];
                let p = mesh_world(&child, i, j);
                let morphed = p + p.normalize() * f64::from(v.morph_dh);
                let weights = parent_triangle_weights(&parent, key, i, j);
                let parent_position = weights
                    .iter()
                    .map(|&(index, w)| mesh_world_index(&parent, index) * w)
                    .sum::<DVec3>();
                let parent_wet = weights
                    .iter()
                    .map(|&(index, w)| f64::from(parent.vertices[index].water[3]) * w)
                    .sum::<f64>();
                max_position_km = max_position_km.max(morphed.distance(parent_position));
                max_wet = max_wet.max((f64::from(v.morph_wet) - parent_wet).abs());
            }
        }
        (max_position_km, max_wet)
    }

    #[test]
    fn lake_shore_fraction_requires_height_edge_and_liquid_water() {
        let level = 0.070;
        assert_eq!(lake_shore_frac(8.0, level, level, 0.0), 1.0);
        assert_eq!(lake_shore_frac(-4.0, level, level, 0.0), 1.0);
        assert_eq!(lake_shore_frac(8.0, level - 0.0001, level, 0.0), 0.0);
        assert_eq!(lake_shore_frac(-4.01, level, level, 0.0), 0.0);
        assert_eq!(lake_shore_frac(8.0, level, f64::NEG_INFINITY, 0.0), 0.0);
        assert_eq!(lake_shore_frac(8.0, level, level, f64::INFINITY), 0.0);
        assert_eq!(
            lake_shore_frac(8.0, level + LAKE_SHORE_HEIGHT_KM, level, 0.0),
            0.0
        );
        assert_eq!(
            lake_shore_frac(8.0, level, level, LAKE_SHORE_REACH_KM),
            0.0
        );

        // Both ramps are at their smoothstep midpoint, so they multiply.
        let f = lake_shore_frac(
            8.0,
            level + LAKE_SHORE_HEIGHT_KM * 0.5,
            level,
            (LAKE_SHORE_SOLID_KM + LAKE_SHORE_REACH_KM) * 0.5,
        );
        assert!((f - 0.25).abs() < 1e-12, "fraction={f}");
    }

    #[test]
    fn lake_shore_field_matches_sample_clearance_boundary() {
        let planet = test_planet();
        let lat = 13.346f64.to_radians();
        let lon = -4.806f64.to_radians();
        let dir = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        let (face, u, v) = crate::planet::face_from_dir(dir);
        let mut s = sample(planet, face, u, v, VOXEL_OCTAVES);
        assert!(s.lake_level_km.is_finite() && s.temp_c >= -4.0);

        s.h_km = s.lake_level_km - LAKE_WATER_CLEARANCE_KM;
        assert!(shore_field(planet, face, u, v, &s).abs() < 1e-9);
        s.h_km = s.lake_level_km;
        assert!(
            (f64::from(shore_field(planet, face, u, v, &s))
                + LAKE_WATER_CLEARANCE_KM)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn v5_shore_uses_voxel_ground_without_moving_capped_mesh() {
        let planet = test_planet();
        // A spacing-capped level-14 tile two kilometres from the measured
        // lake_shore camera. Its gentle shoals contain both 8<->12 octave
        // shore shifts and lake-class flips.
        let key = TileKey { face: 0, level: 14, ix: 7501, iy: 10141, deep: false };
        let octaves = octave_count(key.level, planet.radius_km);
        assert_eq!(octaves, MAX_DETAIL_OCTAVES);
        let mesh = build_tile(planet, key, 1.0);
        let n = TILE_QUADS + 1;
        let np2 = n + 2;
        let (u0, v0, size) = key.uv_range();
        let spacing = key.size_km(planet.radius_km) / TILE_QUADS as f64;

        // Reconstruct the geometry source independently at the capped budget,
        // including the ghost ring that owns vertex normals.
        let mut capped_world = vec![DVec3::ZERO; np2 * np2];
        for gj in 0..np2 {
            for gi in 0..np2 {
                let u = u0 + size * (gi as f64 - 1.0) / TILE_QUADS as f64;
                let v = v0 + size * (gj as f64 - 1.0) / TILE_QUADS as f64;
                let s = sample(planet, key.face as usize, u, v, octaves);
                capped_world[gj * np2 + gi] =
                    face_dir(key.face as usize, u, v) * (planet.radius_km + s.render_h_km());
            }
        }

        let mut shifted_fields = 0usize;
        let mut class_flips = 0usize;
        let mut distinct_heights = 0usize;
        for j in 0..n {
            for i in 0..n {
                let index = j * n + i;
                let (gi, gj) = (i + 1, j + 1);
                let u = u0 + size * i as f64 / TILE_QUADS as f64;
                let v = v0 + size * j as f64 / TILE_QUADS as f64;
                let capped = sample(planet, key.face as usize, u, v, octaves);
                let voxel = sample(planet, key.face as usize, u, v, VOXEL_OCTAVES);
                let vertex = mesh.vertices[index];

                let expected_shore = shore_field(planet, key.face as usize, u, v, &voxel);
                assert!(
                    (vertex.shore - expected_shore).abs() < 1e-7,
                    "({i},{j}) shore {} != voxel reference {expected_shore}",
                    vertex.shore
                );
                if (shore_field(planet, key.face as usize, u, v, &capped) - expected_shore).abs()
                    > 1e-7
                {
                    shifted_fields += 1;
                }
                class_flips += (capped.lake != voxel.lake) as usize;
                distinct_heights +=
                    ((capped.render_h_km() - voxel.render_h_km()).abs() > 1e-7) as usize;

                if needs_voxel_shore_reference(&capped, octaves) {
                    let optimized =
                        voxel_shore_reference(planet, key.face as usize, u, v, octaves, &capped);
                    assert!((optimized.h_km - voxel.h_km).abs() < 1e-12);
                    assert_eq!(optimized.lake, voxel.lake);
                    assert_eq!(optimized.salt, voxel.salt);
                }

                // Positions, normals, paint, and water-plane ownership must
                // remain tied to the capped geometry Sample.
                let world = mesh_world(&mesh, i, j);
                let expected_world = capped_world[gj * np2 + gi];
                assert!(world.distance(expected_world) < 0.000_001);
                let expected_normal = (capped_world[gj * np2 + gi + 1]
                    - capped_world[gj * np2 + gi - 1])
                    .cross(
                        capped_world[(gj + 1) * np2 + gi]
                            - capped_world[(gj - 1) * np2 + gi],
                    )
                    .normalize_or_zero();
                assert!(
                    DVec3::from_array(vertex.normal.map(f64::from)).distance(expected_normal)
                        < 0.000_001
                );
                assert!((f64::from(vertex.water[3]) - tile_wet(&capped, spacing)).abs() < 1e-6);
                assert_eq!(vertex.wflag, if capped.sea || capped.lake { 1.0 } else { 0.0 });
            }
        }
        assert!(shifted_fields > 0, "V-5 tile no longer exercises an octave shore shift");
        assert!(class_flips > 0, "V-5 tile no longer exercises a lake-class flip");
        assert!(distinct_heights > 0, "V-5 tile no longer distinguishes capped geometry");
    }

    #[test]
    fn full_morph_reproduces_parent_triangle_at_v6_sites() {
        let planet = test_planet();
        let sites = [
            (
                "peak",
                TileKey { face: 4, level: 9, ix: 339, iy: 308, deep: false },
            ),
            (
                "valley",
                TileKey { face: 0, level: 9, ix: 111, iy: 281, deep: false },
            ),
        ];
        for (name, key) in sites {
            let (position_km, wet) = full_morph_residuals(planet, key);
            assert!(
                position_km < 0.000_25,
                "{name} fully-morphed child misses its parent by {:.3} m",
                position_km * 1000.0
            );
            assert!(wet < 1e-6, "{name} fully-morphed wetness misses its parent by {wet}");
        }
    }
}

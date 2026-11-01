//! Generated water at rest: planetgen's lakes and rivers as water levels.
//!
//! `scripts/bake_rivers_v2.py` exports the drainage graph (`hydro.bin`
//! beside the world bake): river links with smooth course curves and
//! monotone fill-surface levels, and lakes as a smooth field of planetgen's
//! lake cells. This module turns that into a pure function of (position,
//! voxel size) per column: the terrain shaped to hold the water (valleys,
//! banks, lake beds) and the water surface W.
//!
//! # The water surface and the voxels
//!
//! A column has a water surface W (metres above sea) where some water body's
//! zone covers it: the sea on its side of the shoreline (`coast.rs`, W = 0), a
//! lake where its field exceeds `zone` (W = the lake's level), a river inside its
//! channel (W from the link's profile). Overlaps take the highest. At LOD 0
//! a cell is solid when the ground is above its centre, and otherwise holds
//! water level clamp(round((W - z) / s * 255), 0, 255) (z its bottom, s the
//! block size): the levels of a column are one integer T = round(255 W / s)
//! spread over its cells, so two columns whose surfaces differ by under one
//! level never differ by more than one level in any cell. Coarse LODs snap
//! the surface to the voxel grid (see `Generator::block_at`).
//!
//! # Containment (no water walls) and rest
//!
//! Generated water is at rest in Andrew's sense: horizontal neighbours differ
//! by at most one level (air counts as level 0, a solid cell as a wall that
//! holds any level), and no water cell sits over air or over a partly filled
//! water cell. That holds by construction:
//! - inside one water body W is flat (lakes, the sea) or leans at most
//!   `G_REST` (0.6 levels per block) along a river, never across it;
//! - every zone is ringed by ground at least 0.6 blocks above the highest
//!   W any water cell next to it can have: a lake's apron (flat across the
//!   field band 0.14..zone, then sloping away), a river's banks (`bank_w`:
//!   two voxels and more) which take the level of the most upstream course
//!   point within the channel's reach (`RiverAt::s_near`; the level never
//!   rises downstream, so no step or tight bend can spill sideways);
//! - waters meet only where their levels agree: river links are flat for
//!   four channel widths plus 32 m around every graph node (junctions,
//!   lake ends), a link holds its lake's level until its whole channel is
//!   clear of the lake (`Hydro::lake_ends`), a link with no room to fall
//!   hands its drop to its neighbour, and where two waters that are not
//!   connected there come close, the lower channel yields to the higher
//!   water's bank (`shape`, step 4), so they are always parted by ground.
//!
//! The exceptions, where the water drops on purpose and is *not* at rest,
//! are exactly these: a river cell A next to a cell B (lower by two levels
//! or more, air included, B holding water at or below that layer) where
//! both columns lie inside river channels and B is downstream of A: on the
//! same link further along (s_B > s_A), or on a link the water from A's link
//! reaches by following receivers. They are the river's steps, cascades and
//! waterfalls. The containment test (`hydro_tests.rs`, `water_is_contained`
//! and the wider `water_is_contained_wide`) checks the generated voxels
//! against exactly this rule; `Hydro::downstream` is the reachability.
//!
//! # Steep rivers: pools, steps and falls
//!
//! A link from node a to node b drops D = W_a - W_b over its length. Where
//! its middle part (between the flat ends) is no steeper than `G_REST` the
//! surface is a smooth slope at rest. Steeper links are pools leaning at
//! `G_REST` separated by steps: up to 3 blocks of excess drop become
//! cascades of at most one block each; more becomes waterfalls of up to 24
//! blocks, with a plunge pool and a one-voxel "curtain" (the water steps a
//! voxel after the bed, so the falling face is a column of water against
//! the cliff, standing on the pool, never on air). Below a fall only the
//! channel and its banks drop at once; the rest of the valley floor holds
//! the upper level for a gorge's length, so the fall sits in a notch. Steps
//! are at least four voxels apart at every LOD (merged into fewer, taller
//! ones at coarse LODs) and sit in the same places at LODs 0-3.
//! The flow direction at a river cell is the course tangent toward the
//! receiver (`RiverAt::tangent`); discharge is planetgen's.
//!
//! # Courses
//!
//! Links follow the bake's centripetal Catmull-Rom curves (tributaries arrive
//! straight; the main stem runs on smoothly), plus meanders laid on at load
//! time: a sine of a whole number of wavelengths (12 channel widths, at
//! least 40 m) per link, faded out near the nodes, with bends kept over five
//! half-widths in radius.

use super::boundary::Warp;
use super::noise::{gradient_noise, hash01};
use super::scale::Scale;
use crate::grid::FastMap;
use glam::DVec3;
use rayon::prelude::*;
use std::io::{self, Read};

// ---------------------------------------------------------------------------
// Raw data (hydro.bin)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RawNode {
    pub pos: DVec3,
    /// Fill-surface level (km, source scale), pinned to lakes and the sea.
    pub level_km: f64,
    pub flow: f64,
    /// Level of the lake or sea this node drains into (km).
    pub term_km: f64,
    /// Level of the lake its main stem last left (km; 1e6 for none).
    pub src_km: f64,
    /// Receiver node (always earlier in the list), -1 at a terminal.
    pub rcv: i32,
    /// The link starting here, -1 for none.
    pub seg: i32,
    /// Lake index, -1 for none.
    pub lake: i32,
    /// 0 land, 1 sea, 2 lake, 3 estuary (the last land node, at sea level).
    pub kind: u8,
}

#[derive(Clone, Debug)]
pub struct RawSeg {
    pub a: u32,
    pub b: u32,
    /// Course points strictly between the two nodes.
    pub interior: Vec<DVec3>,
}

#[derive(Clone, Copy, Debug)]
pub struct RawLake {
    pub level_km: f64,
    pub salt: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct RawField {
    pub pos: DVec3,
    pub lake: i32,
}

/// The contents of `hydro.bin`.
#[derive(Clone, Debug)]
pub struct HydroData {
    pub nodes: Vec<RawNode>,
    pub segs: Vec<RawSeg>,
    pub lakes: Vec<RawLake>,
    pub field: Vec<RawField>,
    /// Lake field support (chord on the unit sphere) and the zone threshold.
    pub lake_r_chord: f64,
    pub lake_zone: f64,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.at + n > self.b.len() {
            return Err(io::Error::other("hydro.bin: truncated"));
        }
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> io::Result<f64> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()) as f64)
    }
    fn f64(&mut self) -> io::Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn v3(&mut self) -> io::Result<DVec3> {
        Ok(DVec3::new(self.f64()?, self.f64()?, self.f64()?))
    }
}

impl HydroData {
    /// Read `hydro.bin`; Ok(None) when the file does not exist.
    pub fn load(path: &str) -> io::Result<Option<Self>> {
        let mut f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut b = Vec::new();
        f.read_to_end(&mut b)?;
        Self::parse(&b).map(Some)
    }

    pub fn parse(b: &[u8]) -> io::Result<Self> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != b"TRH1" {
            return Err(io::Error::other("hydro.bin: bad magic"));
        }
        let version = r.u32()?;
        if version != 1 {
            return Err(io::Error::other(format!("hydro.bin: version {version}, expected 1")));
        }
        let (nn, ns, nl, nf, sub) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        let lake_r_chord = r.f64()?;
        let lake_zone = r.f64()?;
        let mut nodes = Vec::with_capacity(nn);
        for _ in 0..nn {
            let pos = r.v3()?.normalize();
            let (level_km, flow, term_km, src_km) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
            let (rcv, seg, lake) = (r.i32()?, r.i32()?, r.i32()?);
            let kind = r.take(4)?[0];
            nodes.push(RawNode { pos, level_km, flow, term_km, src_km, rcv, seg, lake, kind });
        }
        let mut segs = Vec::with_capacity(ns);
        for _ in 0..ns {
            let (a, b) = (r.i32()? as u32, r.i32()? as u32);
            let mut interior = Vec::with_capacity(sub.saturating_sub(1));
            for _ in 1..sub {
                interior.push(r.v3()?.normalize());
            }
            segs.push(RawSeg { a, b, interior });
        }
        let mut lakes = Vec::with_capacity(nl);
        for _ in 0..nl {
            let level_km = r.f32()?;
            let salt = r.take(4)?[0] != 0;
            lakes.push(RawLake { level_km, salt });
        }
        let mut field = Vec::with_capacity(nf);
        for _ in 0..nf {
            let pos = r.v3()?.normalize();
            let lake = r.i32()?;
            r.take(4)?;
            field.push(RawField { pos, lake });
        }
        // Validate references (receivers come first: one forward pass suffices).
        for (k, n) in nodes.iter().enumerate() {
            if n.rcv >= k as i32 || n.seg >= ns as i32 || n.lake >= nl as i32 {
                return Err(io::Error::other(format!("hydro.bin: node {k} has bad references")));
            }
        }
        for (k, s) in segs.iter().enumerate() {
            if s.a as usize >= nn || s.b as usize >= nn {
                return Err(io::Error::other(format!("hydro.bin: link {k} has bad nodes")));
            }
        }
        for f in &field {
            if f.lake >= nl as i32 {
                return Err(io::Error::other("hydro.bin: field cell with a bad lake"));
            }
        }
        Ok(Self { nodes, segs, lakes, field, lake_r_chord, lake_zone })
    }
}

// ---------------------------------------------------------------------------
// Tuning (Andrew's dials are the widths, depths, incision and falls)
// ---------------------------------------------------------------------------

/// Steepest lean of resting water: 0.6 levels per block (Andrew's rule
/// allows 1; the margin covers bends, where the along-river coordinate
/// changes up to ~1.5x faster than the lattice step).
pub const G_REST: f64 = 0.6 / 255.0;
/// Banks stand this many blocks above the water they hold (> 0.5 so the
/// cell holding the surface is solid next door at LOD 0).
const FB_BANK_BLOCKS: f64 = 0.6;
/// Valley floor above the river surface (m).
const FB_FLOOR_M: f64 = 1.0;
/// Outer slope of embankments where the land around falls below a bank (rise over run).
const S_OUT: f64 = 0.25;
/// Longest outer slope (m); past it the natural ground takes over.
const S_OUT_REACH_M: f64 = 40.0;
/// River incision below the natural ground (m): base, per decade of
/// discharge above 100 m^3/s. A river's level is the lowest natural ground
/// along its course upstream, less this (`Hydro::new`), so it runs below the
/// land everywhere and never climbs.
const INC_BASE_M: f64 = 1.5;
const INC_PER_DECADE_M: f64 = 0.5;
/// Below the lake a river leaves, the incision grows from nothing (BETA x
/// the drop below the lake), so the outflow leaves at the lake's level.
const INC_BETA: f64 = 0.5;
/// Land falls towards water: the local relief ramps up from a river's bank
/// or a lake's shore over the valley's half-width, which is in proportion
/// to the relief it cuts through (`valley_half_width`): peaks stand about
/// 1.5 relief high, and the valley's wall slope runs from VALLEY_SLOPE_LOW
/// in lowlands to VALLEY_SLOPE_MOUNTAIN on the belts (V-shaped there,
/// wider and flat-floored in lowlands). Never under VALLEY_MIN_M; a lake's
/// basin never wider than its field reaches (LAKE_VALLEY_MAX_M).
const VALLEY_SLOPE_LOW: f64 = 0.35;
const VALLEY_SLOPE_MOUNTAIN: f64 = 0.6;
const VALLEY_MIN_M: f64 = 60.0;
const LAKE_VALLEY_MAX_M: f64 = 500.0;
/// A lake's basin: where the global surface stands above a lake, it comes
/// down to the shore at this slope (rise over run), never a step.
const LAKE_BASIN_SLOPE: f64 = 0.06;
/// Excess drop (blocks) at or above which a link gets waterfalls instead of cascades.
const FALL_MIN_BLOCKS: f64 = 3.0;
const FALL_MAX_BLOCKS: f64 = 24.0;
const STEP_MAX_BLOCKS: f64 = 1.0;
/// Lake field threshold of the apron top (the zone, where the water is, comes with the data).
const LAKE_APRON: f64 = 0.14;
/// A lake's bands are ground: metres, the same on any planet (the lake field
/// stretches with the radius, so its thresholds alone drew them 7 times wider
/// on the 832 planet): the apron, the sand round the shore and the shelf out
/// to full depth. Each stops at the nearer of these and its field threshold.
const LAKE_APRON_M: f64 = 150.0;
const LAKE_SAND_M: f64 = 50.0;
const LAKE_SHELF_M: f64 = 170.0;
/// Lake depth (m) reached inward of the shore, and the shallowest water at the shore (blocks).
const LAKE_DEPTH_M: f64 = 3.0;
const LAKE_SHORE_DEPTH_BLOCKS: f64 = 0.3;
/// `LakeField::grad` is the field's slope where it's read; the outline's warp
/// can squeeze that into a few times less ground, so a distance worked out
/// from it is only good to about this factor.
const SHORE_STRETCH: f64 = 6.0;
/// Rivers draw their channel up to this LOD, and beyond it only if at least
/// this fraction of a voxel wide (half-width).
const CHANNEL_MAX_LOD: u8 = 4;
const CHANNEL_MIN_HALF_VOXELS: f64 = 0.6;

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn smin(a: f64, b: f64, k: f64) -> f64 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

#[inline]
fn wendland(q: f64) -> f64 {
    if q >= 1.0 {
        return 0.0;
    }
    let u = 1.0 - q;
    let u2 = u * u;
    u2 * u2 * u2 * (35.0 * q * q + 18.0 * q + 3.0)
}

/// d wendland / dq.
#[inline]
fn wendland_d(q: f64) -> f64 {
    if q >= 1.0 {
        return 0.0;
    }
    let u = 1.0 - q;
    -56.0 * q * (5.0 * q + 1.0) * u * u * u * u * u
}

/// Channel half-width (m), depth (m) and valley floor half-width (m) for a discharge.
pub fn channel_shape(flow: f64) -> (f64, f64, f64) {
    let dec = (flow.max(1.0).log10() - 2.0).clamp(0.0, 3.0);
    let hw = (2.0 + 5.0 * dec).clamp(2.0, 24.0);
    let depth = 1.0 + 0.8 * dec;
    let hf = hw + 8.0 + 30.0 * dec;
    (hw, depth, hf)
}

// ---------------------------------------------------------------------------
// A uniform 3D hash grid over the sphere (no face seams)
// ---------------------------------------------------------------------------

struct SphereGrid {
    cell: f64,
    map: FastMap<u64, Vec<u32>>,
}

#[inline]
fn grid_key(x: i64, y: i64, z: i64) -> u64 {
    ((x as u64 & 0x1F_FFFF) << 42) | ((y as u64 & 0x1F_FFFF) << 21) | (z as u64 & 0x1F_FFFF)
}

impl SphereGrid {
    fn new(cell: f64) -> Self {
        Self { cell, map: FastMap::default() }
    }

    fn range(&self, p: DVec3, r: f64) -> [(i64, i64); 3] {
        let lo = (p - DVec3::splat(r)) / self.cell;
        let hi = (p + DVec3::splat(r)) / self.cell;
        [(lo.x.floor() as i64, hi.x.floor() as i64), (lo.y.floor() as i64, hi.y.floor() as i64), (lo.z.floor() as i64, hi.z.floor() as i64)]
    }

    /// Insert item `id` with a bounding sphere (centre and radius in metres).
    fn insert(&mut self, p: DVec3, r: f64, id: u32) {
        let [(x0, x1), (y0, y1), (z0, z1)] = self.range(p, r);
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    self.map.entry(grid_key(x, y, z)).or_default().push(id);
                }
            }
        }
    }

    /// Every item whose cells meet a sphere (centre and radius in metres), sorted, unique.
    fn query(&self, p: DVec3, r: f64, out: &mut Vec<u32>) {
        out.clear();
        let [(x0, x1), (y0, y1), (z0, z1)] = self.range(p, r);
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    if let Some(v) = self.map.get(&grid_key(x, y, z)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

// ---------------------------------------------------------------------------
// The per-world model
// ---------------------------------------------------------------------------

/// One river link in world units.
#[derive(Clone, Debug)]
pub struct Seg {
    pub a: u32,
    pub b: u32,
    /// Course points (unit vectors), nodes included, and arc length (m) at each.
    pub pts: Vec<DVec3>,
    pub cum: Vec<f64>,
    pub len: f64,
    /// Water level at the two nodes (m above sea).
    pub w_a: f64,
    pub w_b: f64,
    pub flow: f64,
    pub hw: f64,
    pub depth: f64,
    pub hf: f64,
    /// Lake at either end (-1 for none).
    pub lake_a: i32,
    pub lake_b: i32,
    /// The part of the link where its level may change (m along it): it
    /// holds the lake level until its course leaves the lake's band at the
    /// start (an outflow) and from where it enters the band at the end (an
    /// inflow), so a river meets a lake at the lake's level.
    pub s_lo: f64,
    pub s_hi: f64,
    /// Widest channel (m, LOD 0) of any link at each end node: a junction's
    /// channels overlap that far, so the water is flat there.
    pub hw_node_a: f64,
    pub hw_node_b: f64,
    /// Half-width of the valley the local relief leaves this link (m): the
    /// widest `valley_half_width` along its course.
    pub valley_w: f64,
    centre: DVec3,
    /// Bounding radius of the course (m).
    span: f64,
    /// Bounding spheres (centre, chord radius) of at most 128 course runs
    /// pieces, for the nearest-point search.
    runs: Vec<(DVec3, f64)>,
}

/// Course pieces per bounding sphere in the nearest-point search.
const SEARCH_RUN: usize = 8;

#[cfg(test)]
mod planet_scale_tests {
    use super::*;

    #[test]
    fn t1_large_river_search_covers_the_whole_course_with_bounded_index_work() {
        let data = HydroData::load(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/world/seed42/hydro.bin")).unwrap().unwrap();
        let p = crate::grid::Planet::new(1.0, 2048, 1024, 1_048_576);
        // A flat 10 m ground and 10 m of relief stand in for the generator:
        // this test is about the index, not the levels.
        let scale = Scale::new(p.radius / 6_371_000.0, p.radius, 1.0, 2048, 1024);
        let h = Hydro::new(&data, &scale, 42, &|_d: DVec3| 10.0, &|_d: DVec3| (10.0, 0.0));
        let (k, longest) = h.segs.iter().enumerate().max_by_key(|(_, s)| s.runs.len()).unwrap();
        let bins: i64 = h.seg_grid.range(longest.centre * p.radius, 26_000.0).iter().map(|(a,b)| b-a+1).product();
        println!("1024 half-heights: longest river {k}, {} pieces, {} search runs; coarse query visits {bins} bins; index has {} river / {} lake bins",
            longest.pts.len()-1, longest.runs.len(), h.seg_grid.map.len(), h.cell_grid.map.len());
        assert!(longest.runs.len() <= 128, "the u128 candidate mask loses the tail of long rivers");
        assert!(bins <= 250_000, "a coarse footprint must not probe a million empty index cells");
        let mut checked = 0;
        for (k, s) in h.segs.iter().enumerate().filter(|(_, s)| s.pts.len() > 1024) {
            for fraction in [0.1, 0.4, 0.75, 0.99] {
                let dir = s.pts[((s.pts.len()-1) as f64 * fraction) as usize];
                let c = h.cands(dir, 0.0);
                let mut rivers = Vec::new();
                h.rivers_at(dir, &c, 1.0, &mut rivers);
                assert!(rivers.iter().any(|r| r.seg == k as u32 && r.d < 1e-5), "river {k} at {fraction}: course omitted");
                checked += 1;
            }
        }
        assert!(checked > 0);
        println!("checked {checked} points on long river courses");
    }
}

impl Seg {
    /// Point on the course at arc length `s` (unit vector).
    pub fn point(&self, s: f64) -> DVec3 {
        let i = self.cum.partition_point(|&c| c <= s).clamp(1, self.pts.len() - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        let u = ((s - self.cum[i - 1]) / (self.cum[i] - self.cum[i - 1]).max(1e-12)).clamp(0.0, 1.0);
        (a * (1.0 - u) + b * u).normalize()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Lake {
    /// Surface level (m above sea).
    pub w: f64,
    pub salt: bool,
}

/// A link's water profile at one voxel size.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    /// Start of the part that may change level (m along the link).
    pub s0: f64,
    /// Flat lengths at the start and end of that part (m) and the middle part's length.
    pub r_a: f64,
    pub r_b: f64,
    pub lm: f64,
    /// Lean of the middle part (m per m).
    pub g: f64,
    /// Steps in the middle part and the drop of each (m).
    pub steps: u32,
    pub delta: f64,
    /// Steps are waterfalls (curtain, plunge pool, jittered positions).
    pub falls: bool,
    /// Voxel size (curtain thickness).
    pub voxel: f64,
}

/// A river link near a column.
#[derive(Clone, Copy, Debug)]
pub struct RiverAt {
    pub seg: u32,
    /// Distance from the course (m) and arc length along the link (m).
    pub d: f64,
    pub s: f64,
    /// The most upstream arc length of course within reach of a channel
    /// cell next to this column (the bank looks up the level there).
    pub s_near: f64,
    /// Downstream course direction (unit, tangent to the sphere).
    pub tangent: DVec3,
    /// The link's water profile at the voxel size asked for.
    pub prof: Profile,
}

/// One lake's field at a column.
#[derive(Clone, Copy, Debug, Default)]
pub struct LakeField {
    pub lake: i32,
    /// Field value (read through the outline's warp) and its gradient (per m).
    pub field: f64,
    pub grad: f64,
}

/// The lakes in reach of a column, each with its own field (the share of
/// the kernel weight on that lake's cells), strongest first. Each lake has
/// its own zone and apron, so where two lakes come close each one's apron
/// dams the other at its own level.
#[derive(Clone, Copy, Debug, Default)]
pub struct LakeAt {
    pub n: usize,
    pub items: [LakeField; MAX_LAKES],
}

const MAX_LAKES: usize = 4;

impl LakeAt {
    pub fn iter(&self) -> impl Iterator<Item = &LakeField> {
        self.items[..self.n].iter()
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// Field of lake `k` (0 when it is not in reach).
    pub fn field_of(&self, k: i32) -> f64 {
        self.iter().find(|l| l.lake == k).map_or(0.0, |l| l.field)
    }
    /// Strongest field.
    pub fn max_field(&self) -> f64 {
        self.iter().map(|l| l.field).fold(0.0, f64::max)
    }
}

impl LakeField {
    /// Distance from the shoreline (m), negative inside the water: the field's
    /// way to it at its slope here, good near the shore.
    #[inline]
    pub fn off_shore(&self, zone: f64) -> f64 {
        (zone - self.field) / self.grad.max(1e-9)
    }
}

/// What kind of water a column's surface belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WaterKind {
    #[default]
    None,
    Ocean,
    Lake,
    River,
}

/// Result of shaping one column.
#[derive(Clone, Copy, Debug)]
pub struct Shaped {
    /// Final ground height (m above sea).
    pub h: f64,
    /// Water surface (m above sea), NEG_INFINITY for none.
    pub water: f64,
    pub kind: WaterKind,
    /// 0..1: river channel bed (gravel).
    pub channel: f64,
    /// Lake bed or lake shore band (sand).
    pub lake_floor: bool,
    pub lake_shore: bool,
    /// A river's bank, right beside its channel (keeps its soil: the steep
    /// channel edge is not bare rock).
    pub bank: bool,
}

/// Candidate links and lake field cells for a region (a chunk or a column).
#[derive(Clone, Debug, Default)]
pub struct Cands {
    pub segs: Vec<u32>,
    /// Per link in `segs`: the runs of its course that can hold the nearest
    /// point of any column of the region (bit i = run i).
    pub runs: Vec<u128>,
    pub cells: Vec<u32>,
}

impl Cands {
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty() && self.cells.is_empty()
    }
}

pub struct Hydro {
    pub radius: f64,
    pub block: f64,
    pub seed: u64,
    /// Node water levels (m above sea), receivers, links, lakes, kinds.
    pub node_w: Vec<f64>,
    pub node_rcv: Vec<i32>,
    pub node_seg: Vec<i32>,
    pub node_lake: Vec<i32>,
    pub node_kind: Vec<u8>,
    pub node_pos: Vec<DVec3>,
    pub segs: Vec<Seg>,
    pub lakes: Vec<Lake>,
    pub field: Vec<RawField>,
    pub lake_r_chord: f64,
    pub lake_zone: f64,
    seg_grid: SphereGrid,
    cell_grid: SphereGrid,
    /// Largest influence radius of any link (m), for bounds.
    pub max_reach: f64,
    /// Bends lake outlines (`boundary.rs`), and how far it can reach (m).
    shore_warp: Warp,
    shore_reach: f64,
    /// Lakes lowered below the bake's level to the lowest river reaching
    /// them, and the largest drop (m).
    pub lakes_lowered: (usize, f64),
    /// How long each phase of `new` took (s): courses, the ground along
    /// them, levels, links, lake ends, short drops.
    pub build_phases: Vec<(&'static str, f64)>,
}

/// Meanders: wavelength in channel widths (D-12: meandering scales with
/// the river), shortest wavelength (m), amplitude as a fraction of the
/// wavelength (gentle enough that bends stay over five half-widths in radius).
const MEANDER_WIDTHS: f64 = 12.0;
const MEANDER_MIN_WL_M: f64 = 40.0;
const MEANDER_AMP: f64 = 0.12;

/// A link's course with meanders: the smooth base curve (nodes included)
/// resampled every few metres and swung sideways by a sine of a whole
/// number of wavelengths, faded in and out over half a wavelength beyond
/// the flat stretches at each node (`flat_a`, `flat_b`, m), so junctions
/// and lake ends keep the base course. `salt` (0..1) varies the amplitude
/// per link.
fn meander(base: &[DVec3], radius: f64, hw: f64, flat_a: f64, flat_b: f64, salt: f64) -> Vec<DVec3> {
    let mut cum = vec![0.0];
    for w in base.windows(2) {
        cum.push(cum.last().unwrap() + (w[1] - w[0]).length() * radius);
    }
    let len = *cum.last().unwrap();
    let at = |s: f64| {
        let i = cum.partition_point(|&c| c <= s).clamp(1, base.len() - 1);
        let u = ((s - cum[i - 1]) / (cum[i] - cum[i - 1]).max(1e-12)).clamp(0.0, 1.0);
        (base[i - 1] * (1.0 - u) + base[i] * u).normalize()
    };
    let wl0 = (MEANDER_WIDTHS * 2.0 * hw).max(MEANDER_MIN_WL_M);
    let active = len - flat_a - flat_b;
    // A link shorter than a wavelength still takes one (big rivers have
    // links of only a few widths), as long as the bend stays gentle.
    if active < 0.4 * wl0 || active < 8.0 * hw + 30.0 {
        return base.to_vec();
    }
    let waves = (active / wl0).round().max(1.0);
    let wl = active / waves;
    // Peak curvature amp (2 pi / wl)^2 keeps bends over five half-widths in radius.
    let amp_curv = wl * wl / (std::f64::consts::TAU * std::f64::consts::TAU * 5.0 * hw);
    let amp = (MEANDER_AMP * wl * (0.6 + 0.7 * salt)).min(amp_curv);
    let step = (wl / 16.0).clamp(2.0, 8.0);
    let n = (len / step).ceil() as usize;
    let tau = std::f64::consts::TAU;
    (0..=n)
        .map(|t| {
            let s = len * t as f64 / n as f64;
            let p = at(s);
            let x = s - flat_a;
            if x <= 0.0 || x >= active {
                return p;
            }
            let env = smoothstep(0.0, 0.5 * wl, x) * smoothstep(0.0, 0.5 * wl, active - x);
            let off = amp * env * (tau * x / wl).sin();
            let tan = at((s + 4.0).min(len)) - at((s - 4.0).max(0.0));
            let side = tan.cross(p).normalize_or_zero();
            (p + side * (off / radius)).normalize()
        })
        .collect()
}

/// Influence of a link beyond its course (m): its valley (the relief ramps
/// up over `valley_w`), and at least its floor, walls and embankments.
#[inline]
fn reach_of(hf: f64, valley_w: f64) -> f64 {
    (hf + 160.0).max(valley_w + 16.0)
}

#[inline]
fn seg_reach(s: &Seg) -> f64 {
    reach_of(s.hf, s.valley_w)
}

/// Half-width of the valley the local relief leaves a river or a lake (m),
/// in proportion to the relief it cuts through: the land's peaks stand
/// about 1.5 relief high, and the valley wall climbs that at a slope from
/// `VALLEY_SLOPE_LOW` (lowlands) to `VALLEY_SLOPE_MOUNTAIN` (the belts).
/// `mountain` is the belt weight (0..1).
#[inline]
pub fn valley_half_width(relief: f64, mountain: f64) -> f64 {
    let slope = VALLEY_SLOPE_LOW + (VALLEY_SLOPE_MOUNTAIN - VALLEY_SLOPE_LOW) * mountain;
    (1.5 * relief / slope).max(VALLEY_MIN_M)
}

/// The valley's cross-section as a share of the relief at `t` = distance
/// from the bank over the half-width: V-shaped (linear) on the belts, a
/// flat floor rising to the hills (quadratic) in lowlands.
#[inline]
fn valley_profile(t: f64, mountain: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t.powf(2.0 - mountain)
}

impl Hydro {
    /// The model from `hydro.bin`. `ground` is the natural ground (m above
    /// sea) with no local relief, as the generator makes it at a river's
    /// channel; `relief` the local relief scale and belt weight at a
    /// direction (`landform.rs`). Both are pure functions of position, so
    /// the model is too.
    pub fn new(data: &HydroData, scale: &Scale, seed: u64, ground: &(dyn Fn(DVec3) -> f64 + Sync), relief: &(dyn Fn(DVec3) -> (f64, f64) + Sync)) -> Self {
        let (radius, block) = (scale.radius, scale.block_m);
        let nn = data.nodes.len();
        let mut phases: Vec<(&'static str, f64)> = Vec::new();
        let mut t_phase = std::time::Instant::now();
        let mut phase = |name: &'static str, phases: &mut Vec<(&'static str, f64)>| {
            phases.push((name, t_phase.elapsed().as_secs_f64()));
            t_phase = std::time::Instant::now();
        };
        // Global heights, under the world's ceiling: lake levels and the
        // terminals rivers drain into.
        let level_m = |km: f64| scale.soft_ceiling(scale.global_m(km));
        let lakes: Vec<Lake> = data.lakes.iter().map(|l| Lake { w: level_m(l.level_km), salt: l.salt }).collect();
        // Widest channel at each node (LOD 0).
        let mut hw_node = vec![0.0f64; nn];
        for s in &data.segs {
            let hw = channel_shape(data.nodes[s.a as usize].flow).0;
            hw_node[s.a as usize] = hw_node[s.a as usize].max(hw);
            hw_node[s.b as usize] = hw_node[s.b as usize].max(hw);
        }
        // Courses with their meanders.
        let courses: Vec<Vec<DVec3>> = data
            .segs
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let (na, nb) = (&data.nodes[s.a as usize], &data.nodes[s.b as usize]);
                let mut base = Vec::with_capacity(s.interior.len() + 2);
                base.push(na.pos);
                base.extend_from_slice(&s.interior);
                base.push(nb.pos);
                let hw = channel_shape(na.flow).0;
                // Meanders stay out of the flat water around each node, and out
                // of lakes (a lake's own links run straight across it).
                let flat = |hw_n: f64| 4.0 * hw_n + 12.0;
                if na.lake >= 0 && nb.lake >= 0 { base } else { meander(&base, radius, hw, flat(hw_node[s.a as usize]), flat(hw_node[s.b as usize]), hash01(k as i64, 3, 11, seed ^ 0x3EA)) }
            })
            .collect();
        phase("courses", &mut phases);
        // The lake field cells, and a lake's share of the field at a point
        // (no wiggle): the ground inside a lake's apron band is the lake's
        // own (its bed), not a level a river can hold.
        // The network and lake field stretch with R. Keep the lookup grids
        // at that scale too (T1): fixed metre bins cost millions of empty
        // probes per coarse footprint on a large planet. Exact metre
        // distances still filter candidates, so this changes no channel or
        // shore.
        let index_scale = (radius / (131_072.0 * (6.0 / (4.0 * std::f64::consts::PI)).sqrt())).max(1.0);
        let r_field = data.lake_r_chord * radius;
        let shore_warp = Warp::shore(radius, block, r_field, seed);
        let shore_reach = shore_warp.reach_m(radius);
        let mut cell_grid = SphereGrid::new(2048.0 * index_scale);
        for (k, c) in data.field.iter().enumerate() {
            cell_grid.insert(c.pos * radius, r_field + shore_reach, k as u32);
        }
        let in_band = |dir: DVec3, lakes_here: [i32; 2]| -> bool {
            let field = |dir: DVec3| -> [f64; 2] {
                let mut cells = Vec::new();
                cell_grid.query(dir * radius, 0.0, &mut cells);
                let (mut den, mut num) = (0.0, [0.0f64; 2]);
                for &k in &cells {
                    let c = &data.field[k as usize];
                    let q = (dir - c.pos).length() / data.lake_r_chord;
                    if q >= 1.0 {
                        continue;
                    }
                    let w = wendland(q);
                    den += w;
                    for (i, l) in lakes_here.iter().enumerate() {
                        if *l >= 0 && c.lake == *l {
                            num[i] += w;
                        }
                    }
                }
                if den > 0.0 { [num[0] / den, num[1] / den] } else { [0.0; 2] }
            };
            let f = field(dir);
            let (a, b) = (dir.any_orthonormal_vector() * (LAKE_APRON_M / radius), dir.cross(dir.any_orthonormal_vector()) * (LAKE_APRON_M / radius));
            (0..2).any(|i| {
                if f[i] <= LAKE_APRON {
                    return false;
                }
                // Over the apron's width the field rises this much at most.
                let rise = [a, -a, b, -b].iter().map(|e| field((dir + *e).normalize())[i] - f[i]).fold(0.0f64, f64::max);
                f[i] + rise > data.lake_zone
            })
        };
        let incision = |flow: f64| INC_BASE_M + INC_PER_DECADE_M * (flow.max(1.0).log10() - 2.0).clamp(0.0, 3.0);
        // Along every course: the lowest natural ground less the incision
        // (the level the river can hold there), and the widest valley. Course
        // points inside the band of a lake the link starts or ends in are
        // skipped: the ground there is the lake's bed.
        let per_link: Vec<(f64, f64)> = courses
            .par_iter()
            .enumerate()
            .map(|(k, pts)| {
                let (na, nb) = (&data.nodes[data.segs[k].a as usize], &data.nodes[data.segs[k].b as usize]);
                let inc = incision(na.flow);
                let src = (na.src_km < 1.0e5).then(|| level_m(na.src_km));
                let (mut lo, mut vw) = (f64::MAX, 0.0f64);
                let lakes_here = [na.lake, nb.lake];
                // The ground is the bake's (smooth over 140 m texels) shaped
                // by the coast: every other course point (4-16 m) is enough;
                // the relief scale every eighth.
                for (i, p) in pts.iter().enumerate() {
                    let last = i + 1 == pts.len();
                    if i % 2 == 1 && !last {
                        continue;
                    }
                    if (na.lake >= 0 || nb.lake >= 0) && in_band(*p, lakes_here) {
                        continue;
                    }
                    let g = ground(*p);
                    let inc_here = match src {
                        Some(w) => inc.min(INC_BETA * (w - g).max(0.0)),
                        None => inc,
                    };
                    lo = lo.min(g - inc_here);
                    if i % 8 == 0 || last {
                        let (rl, m) = relief(*p);
                        vw = vw.max(valley_half_width(rl, m));
                    }
                }
                (lo, vw)
            })
            .collect();
        phase("ground along the courses", &mut phases);
        // Node levels (m above sea), upstream first (receivers come first in
        // the list, so the reverse order visits every link's start before its
        // end): the lowest of the natural ground less the incision over the
        // node and every course upstream of it, so the river lies below the
        // land everywhere and never climbs. A lake takes the lowest level any
        // river reaches it at (the bake's if that is lower), so no river is
        // held up above its own valley on the way in: the lake sits in its
        // basin and the rivers fall into it. Levels are then floored at the
        // lake or sea each node drains into (which can no longer raise a
        // river above its ground), and lake and sea nodes hold theirs exactly.
        let mut incoming: Vec<Vec<u32>> = vec![Vec::new(); nn];
        for (k, s) in data.segs.iter().enumerate() {
            incoming[s.b as usize].push(k as u32);
        }
        // A lake's level is settled at its lowest-indexed node, by when every
        // inflow (a higher index than the lake node it enters) is known.
        let mut lake_last = vec![usize::MAX; lakes.len()];
        for (k, n) in data.nodes.iter().enumerate() {
            if n.lake >= 0 {
                lake_last[n.lake as usize] = k;
            }
        }
        let mut inflows: Vec<Vec<u32>> = vec![Vec::new(); lakes.len()];
        for (i, s) in data.segs.iter().enumerate() {
            let (la, lb) = (data.nodes[s.a as usize].lake, data.nodes[s.b as usize].lake);
            if lb >= 0 && la != lb {
                inflows[lb as usize].push(i as u32);
            }
        }
        let mut lakes = lakes;
        let mut lowered = (0usize, 0.0f64);
        let mut free = vec![0.0f64; nn];
        for k in (0..nn).rev() {
            let n = &data.nodes[k];
            free[k] = match n.kind {
                1 | 3 => 0.0,
                2 => {
                    let l = n.lake as usize;
                    if lake_last[l] == k {
                        let mut w = lakes[l].w;
                        for &i in &inflows[l] {
                            w = w.min(free[data.segs[i as usize].a as usize]).min(per_link[i as usize].0);
                        }
                        if w < lakes[l].w {
                            lowered = (lowered.0 + 1, lowered.1.max(lakes[l].w - w));
                            lakes[l].w = w;
                        }
                    }
                    lakes[l].w
                }
                _ => {
                    // The node's own ground, unless it sits in the band of
                    // the lake it drains into (that ground is the lake's).
                    let rl = if n.rcv >= 0 { data.nodes[n.rcv as usize].lake } else { -1 };
                    let mut w = if rl >= 0 && in_band(n.pos, [rl, -1]) { f64::MAX } else { ground(n.pos) - incision(n.flow) };
                    for &i in &incoming[k] {
                        w = w.min(free[data.segs[i as usize].a as usize]).min(per_link[i as usize].0);
                    }
                    if w == f64::MAX {
                        w = ground(n.pos) - incision(n.flow);
                    }
                    w
                }
            };
        }
        // The terminal of every node: the lake it drains into, or the sea.
        let mut term: Vec<i32> = vec![-1; nn];
        for k in 0..nn {
            let n = &data.nodes[k];
            term[k] = if n.kind == 2 {
                n.lake
            } else if n.rcv >= 0 {
                let r = n.rcv as usize;
                if data.nodes[r].kind == 2 { data.nodes[r].lake } else { term[r] }
            } else {
                -1
            };
        }
        let node_w: Vec<f64> = (0..nn)
            .map(|k| match data.nodes[k].kind {
                1 | 3 => 0.0,
                2 => lakes[data.nodes[k].lake as usize].w,
                _ => free[k].max(if term[k] >= 0 { lakes[term[k] as usize].w } else { 0.0 }),
            })
            .collect();
        phase("levels", &mut phases);
        let mut segs = Vec::with_capacity(data.segs.len());
        let mut seg_grid = SphereGrid::new(512.0 * index_scale);
        let mut max_reach = 0.0f64;
        for (k, (s, pts)) in data.segs.iter().zip(courses).enumerate() {
            let (na, nb) = (&data.nodes[s.a as usize], &data.nodes[s.b as usize]);
            let (hw, depth, hf) = channel_shape(na.flow);
            let mut cum = Vec::with_capacity(pts.len());
            let mut acc = 0.0;
            cum.push(0.0);
            for w in pts.windows(2) {
                acc += (w[1] - w[0]).length() * radius;
                cum.push(acc);
            }
            let centre = pts.iter().fold(DVec3::ZERO, |a, p| a + *p).normalize();
            let span = pts.iter().map(|p| (*p - centre).length() * radius).fold(0.0, f64::max);
            // Cands holds a u128 mask. Longer courses need larger runs,
            // never a shift past bit 127 (which aliases bits in release).
            let search_run = (pts.len() - 1).div_ceil(128).max(SEARCH_RUN);
            let runs = pts
                .windows(2)
                .collect::<Vec<_>>()
                .chunks(search_run)
                .map(|run| {
                    let c = run.iter().fold(DVec3::ZERO, |a, w| a + w[0] + w[1]).normalize();
                    let r = run.iter().map(|w| (w[0] - c).length().max((w[1] - c).length())).fold(0.0, f64::max);
                    (c, r)
                })
                .collect();
            let valley_w = per_link[k].1;
            let reach = reach_of(hf, valley_w);
            max_reach = max_reach.max(reach);
            seg_grid.insert(centre * radius, span + reach, k as u32);
            segs.push(Seg {
                a: s.a,
                b: s.b,
                pts,
                cum,
                len: acc,
                w_a: node_w[s.a as usize],
                w_b: node_w[s.b as usize],
                flow: na.flow,
                hw,
                depth,
                hf,
                lake_a: na.lake,
                lake_b: nb.lake,
                s_lo: 0.0,
                s_hi: acc,
                hw_node_a: hw_node[s.a as usize],
                hw_node_b: hw_node[s.b as usize],
                valley_w,
                centre,
                span,
                runs,
            });
        }
        phase("links", &mut phases);
        let mut hydro = Self {
            radius,
            block,
            seed: seed ^ 0x48D_2A70,
            node_w,
            node_rcv: data.nodes.iter().map(|n| n.rcv).collect(),
            node_seg: data.nodes.iter().map(|n| n.seg).collect(),
            node_lake: data.nodes.iter().map(|n| n.lake).collect(),
            node_kind: data.nodes.iter().map(|n| n.kind).collect(),
            node_pos: data.nodes.iter().map(|n| n.pos).collect(),
            segs,
            lakes,
            field: data.field.clone(),
            lake_r_chord: data.lake_r_chord,
            lake_zone: data.lake_zone,
            seg_grid,
            cell_grid,
            max_reach,
            shore_warp,
            shore_reach,
            lakes_lowered: lowered,
            build_phases: Vec::new(),
        };
        phase("grids", &mut phases);
        hydro.lake_ends();
        phase("lake ends", &mut phases);
        hydro.pass_on_short_drops();
        phase("short drops", &mut phases);
        hydro.build_phases = phases;
        hydro
    }

    /// A link whose stretch between its lake ends is too short to hold the
    /// flat water its junctions need (a lake outlet that leaves the lake's
    /// band only metres before it joins another river) hands its drop on:
    /// an outflow's end node rises to the lake level, an inflow's start
    /// node sinks to it, so the neighbouring link takes the drop instead.
    /// Lake, sea and estuary nodes keep their levels.
    fn pass_on_short_drops(&mut self) {
        let nn = self.node_w.len();
        let block = self.block;
        let short = |s: &Seg| {
            let need = 2.0 * (4.0 * s.hw_node_a.max(s.hw_node_b) + 4.0 * block) + 16.0;
            s.s_hi - s.s_lo < need && s.w_a > s.w_b
        };
        let mut incoming: Vec<Vec<u32>> = vec![Vec::new(); nn];
        for (k, s) in self.segs.iter().enumerate() {
            incoming[s.b as usize].push(k as u32);
        }
        // Upstream first (receivers have lower indices): raise end nodes.
        for b in (0..nn).rev() {
            if self.node_kind[b] != 0 || incoming[b].is_empty() {
                continue;
            }
            let segs = &self.segs;
            if incoming[b].iter().any(|&k| short(&segs[k as usize])) {
                let w = incoming[b].iter().map(|&k| self.node_w[segs[k as usize].a as usize]).fold(f64::MAX, f64::min);
                if w > self.node_w[b] {
                    self.node_w[b] = w;
                    for &k in &incoming[b] {
                        self.segs[k as usize].w_b = w;
                    }
                    let out = self.node_seg[b];
                    if out >= 0 {
                        self.segs[out as usize].w_a = w;
                    }
                }
            }
        }
        // Downstream first: sink start nodes of links still too short.
        for a in 0..nn {
            let out = self.node_seg[a];
            if self.node_kind[a] != 0 || out < 0 || !short(&self.segs[out as usize]) {
                continue;
            }
            let w = self.segs[out as usize].w_b;
            self.node_w[a] = w;
            self.segs[out as usize].w_a = w;
            for &k in &incoming[a] {
                self.segs[k as usize].w_b = w;
            }
        }
    }

    /// Where each link meets a lake: the arc length at which its course
    /// leaves the lake's band for good (outflows) or enters it for good (inflows).
    /// The field is sampled every few metres along the course, with the
    /// outline's bends down to a few metres, so a river meets the lake where
    /// its shore really is.
    fn lake_ends(&mut self) {
        const STEP: f64 = 3.0;
        const MIN_ACTIVE: f64 = 40.0;
        let ends: Vec<(usize, f64, f64)> = self
            .segs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.lake_a >= 0 || s.lake_b >= 0)
            .map(|(k, s)| {
                let n = (s.len / STEP).ceil().max(1.0) as usize;
                // (inside lake a's band, inside lake b's band) along the course.
                let inside: Vec<(bool, bool)> = (0..=n)
                    .map(|t| {
                        let x = s.len * t as f64 / n as f64;
                        let d = s.point(x);
                        let reach = s.hw + 12.0;
                        let c = self.cands(d, reach);
                        let l = self.lake_at(d, &c.cells, 4.0);
                        // Inside: in the lake's band, or the channel's edge
                        // (and its banks) within reach of the lake's water. A
                        // bent shore can reach the channel's edge from a field
                        // that says otherwise here, so look there too.
                        let along = s.point((x + STEP).min(s.len)) - s.point((x - STEP).max(0.0));
                        let side = d.cross(along).normalize_or_zero() * (reach / self.radius);
                        let (l1, l2) = (self.lake_at((d + side).normalize(), &c.cells, 4.0), self.lake_at((d - side).normalize(), &c.cells, 4.0));
                        let near = |k: i32| {
                            k >= 0
                                && (l.iter().any(|lf| lf.lake == k && lf.off_shore(self.lake_zone) < if lf.field > LAKE_APRON { LAKE_APRON_M.max(reach) } else { reach })
                                    || [&l1, &l2].iter().any(|e| e.iter().any(|lf| lf.lake == k && lf.field > self.lake_zone)))
                        };
                        (near(s.lake_a), near(s.lake_b))
                    })
                    .collect();
                // An outflow holds the lake's level until its course is clear
                // of the band for good (the last point inside it), an inflow
                // from the first point inside the band on: a course that
                // weaves in and out of the band never carries water above
                // the lake's level beside the lake's own (T2: a lake lowered
                // to its inflow put such a weave's water a film above the
                // lake, 205 walls in the wide sweep).
                let mut lo = 0.0;
                if s.lake_a >= 0 {
                    // Leaves it for good: a bent shore can come back to the course.
                    let last_in = inside.iter().rposition(|b| b.0).map_or(0, |i| i + 1);
                    lo = s.len * last_in.min(n) as f64 / n as f64;
                }
                let mut hi = s.len;
                if s.lake_b >= 0 {
                    let first_in = inside.iter().position(|b| b.1).unwrap_or(n);
                    hi = s.len * first_in as f64 / n as f64;
                }
                // Keep a stretch to change level in (both ends in lakes: flat anyway).
                if hi - lo < MIN_ACTIVE.min(s.len) {
                    let mid = 0.5 * (lo + hi).clamp(0.0, 2.0 * s.len);
                    let half = 0.5 * MIN_ACTIVE.min(s.len);
                    let m = mid.clamp(half, s.len - half);
                    lo = m - half;
                    hi = m + half;
                }
                (k, lo, hi)
            })
            .collect();
        for (k, lo, hi) in ends {
            self.segs[k].s_lo = lo;
            self.segs[k].s_hi = hi;
        }
    }

    /// Candidates whose influence can reach a disc of radius `r` (m) around `dir`.
    pub fn cands(&self, dir: DVec3, r: f64) -> Cands {
        let p = dir * self.radius;
        let mut all = Vec::new();
        self.seg_grid.query(p, r, &mut all);
        let rho = r / self.radius;
        let mut segs = Vec::with_capacity(all.len());
        let mut runs = Vec::with_capacity(all.len());
        for k in all {
            let s = &self.segs[k as usize];
            if (s.centre - dir).length() * self.radius > s.span + seg_reach(s) + r {
                continue;
            }
            // Every column within rho of `dir` has its nearest course point
            // within (closest run's far side) + rho; runs that start farther
            // than that from every such column cannot hold it.
            let near: Vec<(f64, f64)> = s.runs.iter().map(|&(c, rr)| {
                let d = (dir - c).length();
                (d - rr, d + rr)
            }).collect();
            let m = near.iter().map(|x| x.1).fold(f64::MAX, f64::min);
            let lb = near.iter().map(|x| x.0).fold(f64::MAX, f64::min);
            if (lb - rho) * self.radius > seg_reach(s) {
                continue;
            }
            let mut mask = 0u128;
            for (i, x) in near.iter().enumerate() {
                if x.0 <= m + 2.0 * rho + 1e-12 {
                    mask |= 1u128 << i;
                }
            }
            segs.push(k);
            runs.push(mask);
        }
        let mut cells = Vec::new();
        self.cell_grid.query(p, r, &mut cells);
        let rf = self.lake_r_chord * self.radius + self.shore_reach;
        cells.retain(|&k| (self.field[k as usize].pos - dir).length() * self.radius <= rf + r);
        // Most field cells are the dry land around lakes: with no lake among
        // them there's nothing to find, and no outline to warp for.
        if !cells.iter().any(|&k| self.field[k as usize].lake >= 0) {
            cells.clear();
        }
        Cands { segs, runs, cells }
    }

    /// The water profile of link `k` for voxels of `voxel` metres.
    pub fn profile(&self, k: u32, voxel: f64) -> Profile {
        let s = &self.segs[k as usize];
        let active = s.s_hi - s.s_lo;
        let floor_hw = CHANNEL_MIN_HALF_VOXELS * voxel + 0.2 * voxel;
        // The same at LODs 0-3 (voxels up to 8 m), so steps and falls sit in
        // the same places there.
        let flat = |hw_node: f64| (4.0 * hw_node.max(floor_hw) + 4.0 * voxel.max(8.0)).min(0.3 * active);
        let (r_a, r_b) = (flat(s.hw_node_a), flat(s.hw_node_b));
        let lm = (active - r_a - r_b).max(1e-6);
        let s0 = s.s_lo;
        let drop = (s.w_a - s.w_b).max(0.0);
        let g_mid = drop / lm;
        if g_mid <= G_REST {
            return Profile { s0, r_a, r_b, lm, g: g_mid, steps: 0, delta: 0.0, falls: false, voxel };
        }
        let excess = drop - G_REST * lm;
        let b = self.block;
        let falls = excess >= FALL_MIN_BLOCKS * b;
        let want = if falls { (excess / (FALL_MAX_BLOCKS * b)).ceil() } else { (excess / (STEP_MAX_BLOCKS * b)).ceil() }.max(1.0);
        // Band limit: steps at least four voxels apart.
        let fit = (lm / (4.0 * voxel)).floor().max(1.0);
        let steps = want.min(fit) as u32;
        Profile { s0, r_a, r_b, lm, g: G_REST, steps, delta: excess / steps as f64, falls, voxel }
    }

    /// Position (m along the link) of step j (1-based) of a profile.
    #[inline]
    fn step_at(&self, k: u32, p: &Profile, j: u32) -> f64 {
        let jit = if p.falls { 0.3 * (hash01(k as i64, j as i64, 7, self.seed) - 0.5) } else { 0.0 };
        p.s0 + p.r_a + p.lm * ((j as f64 - 0.5 + jit) / p.steps as f64)
    }

    /// Steps at or before `s` (counting a step at `s_j` once `s >= s_j + shift`).
    #[inline]
    fn steps_before(&self, k: u32, p: &Profile, s: f64, shift: f64) -> u32 {
        if p.steps == 0 {
            return 0;
        }
        if !p.falls {
            let x = (s - shift - p.s0 - p.r_a) / p.lm * p.steps as f64 + 0.5;
            return (x.floor().max(0.0) as u32).min(p.steps);
        }
        (1..=p.steps).filter(|&j| self.step_at(k, p, j) + shift <= s).count() as u32
    }

    /// Water surface on link `k` at arc length `s` (the curtain: a fall's
    /// water steps one voxel after its bed).
    pub fn w_water(&self, k: u32, p: &Profile, s: f64) -> f64 {
        let seg = &self.segs[k as usize];
        let t = (s - p.s0 - p.r_a).clamp(0.0, p.lm);
        let shift = if p.falls { p.voxel } else { 0.0 };
        seg.w_a - p.g * t - p.delta * self.steps_before(k, p, s, shift) as f64
    }

    /// The surface the bed and valley floor follow (steps at their own positions).
    pub fn w_bed(&self, k: u32, p: &Profile, s: f64) -> f64 {
        let seg = &self.segs[k as usize];
        let t = (s - p.s0 - p.r_a).clamp(0.0, p.lm);
        seg.w_a - p.g * t - p.delta * self.steps_before(k, p, s, 0.0) as f64
    }

    /// Extra depth of a plunge pool below a fall at arc length `s` (m).
    fn plunge(&self, k: u32, p: &Profile, s: f64) -> f64 {
        if !p.falls || p.steps == 0 {
            return 0.0;
        }
        let pmax = (0.4 * p.delta).min(4.0);
        let lp = (2.0 * p.delta).max(3.0 * p.voxel).min(0.5 * p.lm / p.steps as f64);
        let mut out = 0.0f64;
        for j in 1..=p.steps {
            let x = s - self.step_at(k, p, j);
            if x >= 0.0 && x < lp {
                out = out.max(pmax * (1.0 - x / lp));
            }
        }
        out
    }

    /// Channel half-width at a voxel size (never under a fraction of a voxel,
    /// so a river stays continuous at coarse LODs).
    #[inline]
    fn half_width(&self, s: &Seg, voxel: f64) -> f64 {
        s.hw.max(CHANNEL_MIN_HALF_VOXELS * voxel + 0.2 * voxel)
    }

    /// Whether link `k` draws a channel at this LOD.
    #[inline]
    fn has_channel(&self, s: &Seg, lod: u8, voxel: f64) -> bool {
        lod <= CHANNEL_MAX_LOD || s.hw >= CHANNEL_MIN_HALF_VOXELS * voxel
    }

    /// Bank band beyond the channel (m): at least two voxels.
    #[inline]
    fn bank_w(&self, voxel: f64) -> f64 {
        2.0 * voxel + 0.5 * self.block + 1.0
    }

    /// Nearby river links: distance from the course, arc length, tangent.
    pub fn rivers_at(&self, dir: DVec3, cands: &Cands, voxel: f64, out: &mut Vec<RiverAt>) {
        out.clear();
        for (&k, &mask) in cands.segs.iter().zip(&cands.runs) {
            let s = &self.segs[k as usize];
            let search_run = (s.pts.len() - 1).div_ceil(128).max(SEARCH_RUN);
            if (s.centre - dir).length() * self.radius > s.span + seg_reach(s) {
                continue;
            }
            let mut best = (f64::MAX, 0.0, DVec3::ZERO);
            // Runs of pieces, skipping those whose bounding sphere is farther
            // than the best piece so far.
            let mut bits = mask;
            while bits != 0 {
                let ri = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                let (c, rr) = s.runs[ri];
                let lb = ((dir - c).length() - rr).max(0.0);
                if lb * lb >= best.0 {
                    continue;
                }
                let i0 = ri * search_run;
                for i in i0..(i0 + search_run).min(s.pts.len() - 1) {
                    let (p, q) = (s.pts[i], s.pts[i + 1]);
                    let e = q - p;
                    let ee = e.length_squared();
                    let t = if ee > 0.0 { ((dir - p).dot(e) / ee).clamp(0.0, 1.0) } else { 0.0 };
                    let d2 = (dir - (p + e * t)).length_squared();
                    if d2 < best.0 {
                        best = (d2, s.cum[i] + t * (s.cum[i + 1] - s.cum[i]), e);
                    }
                }
            }
            let d = best.0.sqrt() * self.radius;
            if d > seg_reach(s) {
                continue;
            }
            // Beside the channel: the most upstream course point within the
            // channel's half-width plus a voxel and a half (every channel
            // cell next door has its nearest course point there; the level
            // never rises downstream, so the water there is the highest
            // this column can be next to, however tight the bend).
            let reach = self.half_width(s, voxel) + 1.5 * voxel + 0.5 * self.block;
            let mut s_near = best.1;
            if d <= reach {
                let rc = reach / self.radius;
                for (ri, &(c, rr)) in s.runs.iter().enumerate() {
                    if (dir - c).length() - rr > rc {
                        continue;
                    }
                    let i0 = ri * search_run;
                    for i in i0..(i0 + search_run).min(s.pts.len() - 1) {
                        let (p, q) = (s.pts[i], s.pts[i + 1]);
                        if s.cum[i] >= s_near {
                            break;
                        }
                        let (e, w) = (q - p, p - dir);
                        let (a, b, cc) = (e.length_squared(), w.dot(e), w.length_squared() - rc * rc);
                        let disc = b * b - a * cc;
                        if a <= 0.0 || disc < 0.0 {
                            continue;
                        }
                        let (t0, t1) = ((-b - disc.sqrt()) / a, (-b + disc.sqrt()) / a);
                        if t1 < 0.0 || t0 > 1.0 {
                            continue;
                        }
                        s_near = s_near.min(s.cum[i] + t0.max(0.0) * (s.cum[i + 1] - s.cum[i]));
                    }
                }
            }
            let t = best.2 - dir * best.2.dot(dir);
            out.push(RiverAt { seg: k, d, s: best.1, s_near, tangent: t.normalize_or_zero(), prof: self.profile(k, voxel) });
        }
    }

    /// The lake field at a direction, read at the warped position
    /// (`boundary.rs`), band-limited: an outline has only the pieces the
    /// field has, bent at every scale.
    pub fn lake_at(&self, dir: DVec3, cells: &[u32], voxel: f64) -> LakeAt {
        let mut out = LakeAt::default();
        // The warp is the dearest thing here: a lake has to be within its reach.
        let far = self.lake_r_chord + self.shore_reach / self.radius;
        if !cells.iter().any(|&k| self.field[k as usize].lake >= 0 && (self.field[k as usize].pos - dir).length() < far) {
            return out;
        }
        let dir = self.shore_warp.at(dir, voxel);
        let r = self.lake_r_chord;
        let mut den = 0.0;
        let mut gden = DVec3::ZERO;
        // Per lake: (lake, weight, weight gradient).
        let mut per: [(i32, f64, DVec3); MAX_LAKES] = [(-1, 0.0, DVec3::ZERO); MAX_LAKES];
        let mut np = 0usize;
        for &k in cells {
            let c = &self.field[k as usize];
            let v = dir - c.pos;
            let dist = v.length();
            let q = dist / r;
            if q >= 1.0 {
                continue;
            }
            let w = wendland(q);
            let gw = if dist > 0.0 { v * (wendland_d(q) / (dist * r)) } else { DVec3::ZERO };
            den += w;
            gden += gw;
            if c.lake >= 0 {
                match per[..np].iter_mut().find(|e| e.0 == c.lake) {
                    Some(e) => {
                        e.1 += w;
                        e.2 += gw;
                    }
                    None if np < MAX_LAKES => {
                        per[np] = (c.lake, w, gw);
                        np += 1;
                    }
                    None => {}
                }
            }
        }
        if den <= 0.0 || np == 0 {
            return out;
        }
        for &(lake, num, gnum) in &per[..np] {
            let f = num / den;
            let g = (gnum - gden * f) / den;
            let g = (g - dir * g.dot(dir)).length() / self.radius;
            out.items[out.n] = LakeField { lake, field: f, grad: g };
            out.n += 1;
        }
        out.items[..out.n].sort_by(|a, b| b.field.total_cmp(&a.field).then(a.lake.cmp(&b.lake)));
        out
    }

    /// A lake bent over a river that stands above it stops short of the river
    /// and its bank. (The other way round, the river yields: `shape`, step 4.)
    /// Two waters at different levels are parted by ground either way.
    pub fn lake_yields(&self, lake: &mut LakeAt, rivers: &[RiverAt], voxel: f64, lod: u8) {
        let eps = 0.5 / 255.0 * self.block;
        let bank = self.bank_w(voxel);
        let zone = self.lake_zone;
        for lf in lake.items[..lake.n].iter_mut().filter(|l| l.field > zone) {
            let w = self.lakes[lf.lake as usize].w;
            let over = rivers.iter().any(|r| {
                let s = &self.segs[r.seg as usize];
                self.has_channel(s, lod, voxel) && r.d < self.half_width(s, voxel) + bank && self.w_water(r.seg, &r.prof, r.s_near) > w + eps
            });
            if over {
                lf.field = zone;
            }
        }
    }

    /// Land falls towards water: how much of the local relief stands at a
    /// column (0 at a river's bank or a lake's shore, 1 beyond the valley),
    /// for the column's relief scale and belt weight. A river's valley is
    /// the narrower of the column's own and the link's widest (so the ramp
    /// always ends inside the link's reach); a lake's basin is capped by how
    /// far its field is seen. Several waters: the lowest ramp.
    pub fn relief_ramp(&self, lake: &LakeAt, rivers: &[RiverAt], relief: f64, mountain: f64, voxel: f64) -> f64 {
        let dv = valley_half_width(relief, mountain);
        let mut ramp = 1.0f64;
        for r in rivers {
            let s = &self.segs[r.seg as usize];
            let d0 = self.half_width(s, voxel) + self.bank_w(voxel);
            ramp = ramp.min(valley_profile((r.d - d0) / dv.min(s.valley_w), mountain));
        }
        let zone = self.lake_zone;
        for lf in lake.iter() {
            if lf.field >= zone {
                return 0.0;
            }
            if lf.grad > 1e-9 {
                let d = (zone - lf.field) / lf.grad;
                ramp = ramp.min(valley_profile(d / dv.min(LAKE_VALLEY_MAX_M), mountain));
            }
        }
        ramp
    }

    /// Shape one column: valleys, lake beds, basins and aprons, banks,
    /// channels; and its water surface. `h` is the natural ground (m above
    /// sea), `h_floor` the same with no local relief (the global surface as
    /// the generator shapes it at a channel), `sea` whether the column lies
    /// on the sea's side of the shoreline, `rugged` the landform's
    /// ruggedness.
    #[allow(clippy::too_many_arguments)]
    pub fn shape(&self, dir: DVec3, h: f64, h_floor: f64, lake: &LakeAt, rivers: &[RiverAt], sea: bool, rugged: f64, voxel: f64, lod: u8) -> Shaped {
        let mut h = h;
        let fb = FB_BANK_BLOCKS * self.block;
        let zone = self.lake_zone;
        let in_lake = lake.iter().any(|l| l.field > zone);
        // Under the sea or inside a lake the river's course is drowned: no
        // valley or channel is cut into the sea floor or the lake bed.
        let drowned = in_lake || (sea && h < 0.0);
        let bank_w = self.bank_w(voxel);
        // 1. River valleys (lowering): a floor FB_FLOOR_M above the river,
        // walls rising at 0.3-0.7, blended into the land by a soft minimum.
        // Below a waterfall only the channel and its banks drop at once: the
        // rest of the floor holds the upper level for a gorge's length, then
        // ramps down, so the fall sits in a notch, not across the valley.
        let wall = 0.3 + 0.4 * rugged;
        for r in rivers {
            if drowned {
                break;
            }
            let s = &self.segs[r.seg as usize];
            let p = r.prof;
            let mut floor = self.w_bed(r.seg, &p, r.s) + FB_FLOOR_M;
            if p.falls {
                let hw = self.half_width(s, voxel);
                let b = smoothstep(hw + bank_w, hw + bank_w + 6.0 + 0.3 * p.delta, r.d);
                if b > 0.0 {
                    floor += b * self.gorge(r.seg, &p, r.s, dir);
                }
            }
            let x = r.d - s.hf;
            let k = 2.0;
            let rise = if x > 8.0 * k { x } else { k * (x / k).exp().ln_1p() };
            let target = floor + wall * rise;
            let reach = seg_reach(s);
            let wv = 1.0 - smoothstep(0.75 * reach, reach, r.d);
            h += wv * (smin(h, target, 3.0) - h);
        }
        // 2. Lake beds (lowering): inside the zone, at least the shore depth
        // below the surface, deepening inward. Outside it, the basin: where
        // the global surface stands above the lake (a lake lowered to the
        // river that reaches it), the ground comes down to the shore at
        // LAKE_BASIN_SLOPE, the local relief riding on it unchanged.
        let mut lake_floor = false;
        let mut lake_shore = false;
        for lf in lake.iter() {
            let l = self.lakes[lf.lake as usize];
            if lf.field > zone {
                let shelf = smoothstep(zone, zone + 0.3, lf.field).max(smoothstep(0.0, LAKE_SHELF_M, -lf.off_shore(zone)));
                let depth = LAKE_SHORE_DEPTH_BLOCKS * self.block + LAKE_DEPTH_M * shelf;
                h = h.min(l.w - depth);
                lake_floor = true;
            } else if lf.grad > 1e-9 {
                let d = (zone - lf.field) / lf.grad;
                let basin = l.w + fb + LAKE_BASIN_SLOPE * d;
                if h_floor > basin {
                    h -= h_floor - basin;
                }
            }
        }
        // 3. Raises: lake aprons (every lake in reach whose zone this column
        // is not in) and river banks.
        for lf in lake.iter().filter(|l| l.field <= zone) {
            let l = self.lakes[lf.lake as usize];
            let top = l.w + fb;
            let d = lf.off_shore(zone);
            // How far past the apron's outer edge, whichever edge is nearer.
            let dist = (d - LAKE_APRON_M).max((LAKE_APRON - lf.field) / lf.grad.max(1e-9));
            let bound = if dist <= 0.0 {
                top
            } else if dist < S_OUT_REACH_M {
                top - S_OUT * dist
            } else {
                f64::NEG_INFINITY
            };
            if h < bound {
                h = bound;
            }
            lake_shore |= lf.field >= zone - 0.07 && d < LAKE_SAND_M && h < l.w + fb + 0.6 * self.block;
        }
        let mut bank = false;
        // Inside a lake its water is the bank, unless the river stands above
        // the lake (a river crossing the zone of a lake it does not drain
        // into, lowered below it: T2): then the river keeps its banks
        // through the lake, so its water never stands over the lake's.
        let lake_top = lake.iter().filter(|l| l.field > zone).map(|l| self.lakes[l.lake as usize].w).fold(f64::NEG_INFINITY, f64::max);
        for r in rivers {
            let s = &self.segs[r.seg as usize];
            if !self.has_channel(s, lod, voxel) {
                continue;
            }
            let p = r.prof;
            let wb = self.w_water(r.seg, &p, r.s_near);
            if in_lake && wb <= lake_top + 0.5 / 255.0 * self.block {
                continue; // inside a lake: its water is the bank
            }
            if wb <= 0.0 && sea {
                continue; // at sea level, on the sea's side, the sea holds it
            }
            let hw = self.half_width(s, voxel);
            if r.d < hw {
                continue;
            }
            bank |= r.d < hw + 1.5 * voxel + self.block;
            let x = r.d - hw - bank_w;
            let bound = if x <= 0.0 {
                wb + fb
            } else if x < S_OUT_REACH_M {
                wb + fb - S_OUT * x
            } else {
                continue;
            };
            if h < bound {
                h = bound;
            }
        }
        // 4. Channels (lowering) and the water surface.
        let mut water = f64::NEG_INFINITY;
        let mut kind = WaterKind::None;
        let mut channel = 0.0f64;
        for r in rivers {
            let s = &self.segs[r.seg as usize];
            if !self.has_channel(s, lod, voxel) {
                continue;
            }
            let hw = self.half_width(s, voxel);
            if r.d >= hw {
                continue;
            }
            let p = r.prof;
            let (ww, wbed) = (self.w_water(r.seg, &p, r.s), self.w_bed(r.seg, &p, r.s));
            // Higher water close by that this channel does not meet at its
            // own level (another river beside it, a lake it skirts): here
            // the channel yields to that water's bank, so two waters at
            // different levels are always parted by ground. Where links
            // join, their levels agree (flat at the node), so nothing yields.
            let eps = 0.5 / 255.0 * self.block;
            let yields = rivers.iter().any(|o| {
                if o.seg == r.seg {
                    return false;
                }
                let so = &self.segs[o.seg as usize];
                if !self.has_channel(so, lod, voxel) {
                    return false;
                }
                let hwo = self.half_width(so, voxel);
                if o.d < hwo {
                    // Inside the other channel too: the higher water is the water here.
                    self.w_water(o.seg, &o.prof, o.s) > ww + eps
                } else {
                    // In the other channel's bank band: its bank stands here.
                    o.d < hwo + bank_w && self.w_water(o.seg, &o.prof, o.s_near) > ww + eps
                }
            }) || lake.iter().any(|lf| lf.field <= zone && (zone - lf.field) < SHORE_STRETCH * bank_w * lf.grad && self.lakes[lf.lake as usize].w > ww + eps);
            if yields {
                continue;
            }
            let q = r.d / hw;
            if !drowned {
                let bed = wbed - s.depth * (1.0 - q * q * q * q) - LAKE_SHORE_DEPTH_BLOCKS * self.block - self.plunge(r.seg, &p, r.s);
                h = h.min(bed);
            }
            if ww > water {
                water = ww;
                kind = WaterKind::River;
            }
            // Under a lake or the sea there is no channel, so no gravel
            // stripe across the shallows.
            if !drowned {
                channel = channel.max(1.0 - q * q);
            }
        }
        for lf in lake.iter().filter(|l| l.field > zone) {
            let l = self.lakes[lf.lake as usize];
            if l.w >= water {
                water = l.w;
                kind = WaterKind::Lake;
            }
        }
        if sea && 0.0 >= water {
            water = 0.0;
            kind = WaterKind::Ocean;
        }
        Shaped { h, water, kind, channel, lake_floor, lake_shore, bank: bank && h >= water }
    }

    /// How far a link's valley floor beyond its banks stands above the
    /// floor at the channel just below a waterfall (m): the full drop for a
    /// gorge length (two drops, jagged by noise), then ramping down at 45
    /// degrees. Zero away from falls.
    fn gorge(&self, k: u32, p: &Profile, s: f64, dir: DVec3) -> f64 {
        if !p.falls || p.steps == 0 {
            return 0.0;
        }
        let jag = 1.0 + 0.45 * gradient_noise(dir * (self.radius / 9.0), self.seed ^ 0x6029);
        let mut out = 0.0f64;
        for j in 1..=p.steps {
            let x = s - self.step_at(k, p, j);
            let g = (2.0 * p.delta).max(6.0 * p.voxel) * jag;
            if x >= 0.0 && x < g + p.delta {
                out = out.max(if x < g { p.delta } else { p.delta * (1.0 - (x - g) / p.delta) });
            }
        }
        out
    }

    /// Whether the water of link `from` reaches link `to` by following
    /// receivers (at most `max_links` links downstream; `from == to` counts).
    pub fn downstream(&self, from: u32, to: u32, max_links: usize) -> bool {
        let mut k = from;
        for _ in 0..=max_links {
            if k == to {
                return true;
            }
            let b = self.segs[k as usize].b as usize;
            let next = self.node_seg[b];
            if next < 0 {
                return false;
            }
            k = next as u32;
        }
        false
    }

    /// Conservative height range (m) the hydrology can move ground or water
    /// to near a set of candidates: (lowest, highest).
    pub fn height_range(&self, c: &Cands) -> Option<(f64, f64)> {
        if c.is_empty() {
            return None;
        }
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for &k in &c.segs {
            let s = &self.segs[k as usize];
            let (a, b) = (s.w_a.min(s.w_b), s.w_a.max(s.w_b));
            lo = lo.min(a - s.depth - 4.0 - 2.0 * self.block);
            hi = hi.max(b + FB_FLOOR_M + FB_BANK_BLOCKS * self.block + 1.0);
        }
        for &k in &c.cells {
            let l = self.field[k as usize].lake;
            if l >= 0 {
                let w = self.lakes[l as usize].w;
                lo = lo.min(w - LAKE_DEPTH_M - 2.0 * self.block);
                hi = hi.max(w + FB_BANK_BLOCKS * self.block + 1.0);
            }
        }
        // Lake damping can pull the landform toward zero anywhere a field cell reaches.
        Some((lo, hi))
    }

    /// The channels containing a column at a voxel size: (link, arc length m).
    pub fn channels_at(&self, rivers: &[RiverAt], voxel: f64, lod: u8) -> Vec<(u32, f64)> {
        rivers
            .iter()
            .filter(|r| {
                let s = &self.segs[r.seg as usize];
                self.has_channel(s, lod, voxel) && r.d < self.half_width(s, voxel)
            })
            .map(|r| (r.seg, r.s))
            .collect()
    }

    /// Flow direction and discharge where a column lies in a river channel.
    pub fn flow_at(&self, rivers: &[RiverAt], voxel: f64) -> Option<(DVec3, f64, u32, f64)> {
        rivers
            .iter()
            .filter(|r| r.d < self.half_width(&self.segs[r.seg as usize], voxel))
            .min_by(|a, b| a.d.total_cmp(&b.d))
            .map(|r| (r.tangent, self.segs[r.seg as usize].flow, r.seg, r.s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wendland_derivative_matches_finite_differences() {
        for i in 1..20 {
            let q = i as f64 / 20.0;
            let h = 1e-6;
            let fd = (wendland(q + h) - wendland(q - h)) / (2.0 * h);
            assert!((fd - wendland_d(q)).abs() < 1e-5, "q {q}: {fd} vs {}", wendland_d(q));
        }
    }

    #[test]
    fn channel_shape_grows_with_discharge() {
        let (a, b, c) = channel_shape(120.0);
        let (d, e, f) = channel_shape(10_000.0);
        assert!(a < d && b < e && c < f);
        assert!(a >= 2.0 && d <= 24.0);
    }
}

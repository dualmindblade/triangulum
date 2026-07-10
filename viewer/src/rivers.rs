//! River courses and lakes from the planetgen drainage graph.
//!
//! scripts/bake_rivers.py exports the cell->receiver polyline segments (with
//! flow and monotonic-downstream water levels) and lake cells (with spill
//! levels). The terrain generator measures exact distance to the nearest
//! segment, so channels follow the map's real valleys and reach the sea —
//! the course is data, only the sub-cell meander is noise.
//!
//! Spatial lookup is a per-face uv bucket grid. Segments are inserted into
//! every bucket within their influence radius (channel + floodplain +
//! meander), so a query only ever reads the single bucket containing the
//! query point — an empty bucket is a near-free "no river here".

use crate::planet::FACES;
use glam::DVec3;

pub struct Segment {
    pub a: DVec3, // unit vector, upstream end
    pub b: DVec3, // unit vector, downstream (receiver) end
    pub flow_log: f32,
    pub level_a_km: f32,
    pub level_b_km: f32,
}

pub struct LakeCell {
    pub center: DVec3, // unit vector
    pub radius_km: f32,
    pub level_km: f32,
    pub salt: bool,
    /// A rim cell: dry land bordering the lake. Water only exists where the
    /// nearest cell is a lake cell — the Voronoi footprint of the actual
    /// map lake, instead of blobby per-cell discs.
    pub rim: bool,
}

pub struct RiverHit {
    pub dist_km: f64,
    pub level_km: f64,
    pub flow: f64, // m3/s at the segment
}

/// Lake candidate at a query point — everything the terrain sampler needs to
/// decide whether this column floods. The decision itself lives in
/// terrain::sample (it needs the planet rasters); this is pure geometry.
pub struct LakeHit {
    pub level_km: f64,
    pub salt: bool,
    /// nearest true lake cell: distance, center, radius
    pub d_lake_km: f64,
    pub lake_center: DVec3,
    pub radius_km: f64,
    /// Distance to the baked lake footprint's actual edge: the bisector
    /// between the nearest true lake cell and nearest dry rim cell. Unlike
    /// `past_boundary_km`, this is symmetric, so it stays meaningful on dry
    /// islands and shoals inside the flood-eligible territory as well as in
    /// the rim territory outside it.
    pub boundary_dist_km: f64,
    /// whether the nearest cell overall (lake AND rim competing) is a lake
    /// cell — i.e. the query is inside the lake's own Voronoi territory
    pub in_lake_voronoi: bool,
    /// how far past the lake/rim Voronoi boundary the query sits (0 inside
    /// lake territory; grows toward the rim cell's far side)
    pub past_boundary_km: f64,
    /// distance past the NEAREST flood frontier (the flood is a union of
    /// the lake's Voronoi territory and every dam rim's 1.15 r shore band;
    /// the shore apron grades from whichever frontier is closest). The
    /// Voronoi metric alone left a 170 m wall where a non-dam rim's
    /// territory sits beside a dam rim's flooded band (16.569 -32.262).
    pub apron_past_km: f64,
    /// the nearest rim (when the nearest cell is one) is a TRUE DAM: its own
    /// elevation reaches the lake level. Shore-band flood-through is only
    /// sound over dam-height rims — a below-level rim (e.g. a peeled
    /// conduit cell down a mountain flank) must not pass the flood through
    /// its territory.
    pub rim_is_dam: bool,
}

const GRID: usize = 128;
/// Influence radius (km) a query may care about: channel half-width (<=0.4)
/// + floodplain damping (<=2.5) + meander (<=0.3), rounded up.
pub const SEG_INFLUENCE_KM: f64 = 3.5;

pub struct RiverIndex {
    pub segments: Vec<Segment>,
    pub lakes: Vec<LakeCell>,
    radius_km: f64,
    seg_buckets: Vec<Vec<u32>>,  // 6 * GRID * GRID
    lake_buckets: Vec<Vec<u32>>,
}

fn bucket_of(face: usize, u: f64, v: f64) -> usize {
    let x = (((u + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
    let y = (((v + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
    (face * GRID + y) * GRID + x
}

/// Project a unit direction onto one face's gnomonic plane (no clamping).
fn face_uv(face: usize, dir: DVec3) -> Option<(f64, f64)> {
    let (axis, right, up) = FACES[face];
    let d = dir.dot(axis);
    if d < 0.30 {
        return None; // far behind this face; projection blows up
    }
    let p = dir / d;
    Some((p.dot(right), p.dot(up)))
}

impl RiverIndex {
    pub fn empty(radius_km: f64) -> Self {
        Self {
            segments: Vec::new(),
            lakes: Vec::new(),
            radius_km,
            seg_buckets: vec![Vec::new(); 6 * GRID * GRID],
            lake_buckets: vec![Vec::new(); 6 * GRID * GRID],
        }
    }

    pub fn load(path: &str, radius_km: f64) -> anyhow::Result<Self> {
        let raw = std::fs::read(path)?;
        anyhow::ensure!(raw.len() >= 12 && &raw[0..4] == b"RIV1", "bad rivers.bin header");
        let n_seg = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let n_lake = u32::from_le_bytes(raw[8..12].try_into().unwrap()) as usize;
        anyhow::ensure!(
            raw.len() == 12 + n_seg * 36 + n_lake * 24,
            "rivers.bin size mismatch - rerun scripts/bake_rivers.py"
        );
        let f = |off: usize| f32::from_le_bytes(raw[off..off + 4].try_into().unwrap());
        let mut out = Self::empty(radius_km);
        for i in 0..n_seg {
            let o = 12 + i * 36;
            out.segments.push(Segment {
                a: DVec3::new(f(o) as f64, f(o + 4) as f64, f(o + 8) as f64),
                b: DVec3::new(f(o + 12) as f64, f(o + 16) as f64, f(o + 20) as f64),
                flow_log: f(o + 24),
                level_a_km: f(o + 28),
                level_b_km: f(o + 32),
            });
        }
        let lake_base = 12 + n_seg * 36;
        for i in 0..n_lake {
            let o = lake_base + i * 24;
            let flags = f(o + 20);
            out.lakes.push(LakeCell {
                center: DVec3::new(f(o) as f64, f(o + 4) as f64, f(o + 8) as f64),
                radius_km: f(o + 12),
                level_km: f(o + 16),
                salt: (flags - 1.0).abs() < 0.25,
                rim: flags > 1.5,
            });
        }
        out.build_buckets();
        Ok(out)
    }

    fn insert(buckets: &mut [Vec<u32>], face: usize, lo: (f64, f64), hi: (f64, f64), id: u32) {
        let clamp01 = |t: f64| t.clamp(-1.0, 1.0);
        let (u0, v0) = (clamp01(lo.0), clamp01(lo.1));
        let (u1, v1) = (clamp01(hi.0), clamp01(hi.1));
        let bx0 = ((u0 + 1.0) * 0.5 * GRID as f64) as usize;
        let bx1 = (((u1 + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
        let by0 = ((v0 + 1.0) * 0.5 * GRID as f64) as usize;
        let by1 = (((v1 + 1.0) * 0.5 * GRID as f64) as usize).min(GRID - 1);
        for y in by0..=by1 {
            for x in bx0..=bx1 {
                buckets[(face * GRID + y) * GRID + x].push(id);
            }
        }
    }

    fn build_buckets(&mut self) {
        // uv padding that covers the metric influence radius anywhere on a
        // face (worst-case gnomonic compression at corners is 3x)
        let pad_seg = SEG_INFLUENCE_KM / self.radius_km * 3.2;
        for (i, s) in self.segments.iter().enumerate() {
            for face in 0..6 {
                let (pa, pb) = match (face_uv(face, s.a), face_uv(face, s.b)) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                };
                let lo = (pa.0.min(pb.0) - pad_seg, pa.1.min(pb.1) - pad_seg);
                let hi = (pa.0.max(pb.0) + pad_seg, pa.1.max(pb.1) + pad_seg);
                if lo.0 > 1.0 || lo.1 > 1.0 || hi.0 < -1.0 || hi.1 < -1.0 {
                    continue;
                }
                Self::insert(&mut self.seg_buckets, face, lo, hi, i as u32);
            }
        }
        for (i, l) in self.lakes.iter().enumerate() {
            for face in 0..6 {
                let Some(c) = face_uv(face, l.center) else { continue };
                // pad past the cell spacing: the Voronoi test needs every
                // competitor (lake AND rim) visible from any query point
                // whose nearest cell might be this one (3.4 matches the
                // extended hit gate in lake_at)
                let pad = (l.radius_km as f64 * 3.4 + 2.0) / self.radius_km * 3.2;
                let lo = (c.0 - pad, c.1 - pad);
                let hi = (c.0 + pad, c.1 + pad);
                if lo.0 > 1.0 || lo.1 > 1.0 || hi.0 < -1.0 || hi.1 < -1.0 {
                    continue;
                }
                Self::insert(&mut self.lake_buckets, face, lo, hi, i as u32);
            }
        }
    }

    /// Any river segments near this bucket at all? Near-free pre-check so
    /// the caller can skip meander noise on the vast riverless majority.
    pub fn maybe_river(&self, face: usize, u: f64, v: f64) -> bool {
        !self.seg_buckets[bucket_of(face, u, v)].is_empty()
    }

    /// Nearest river segment within SEG_INFLUENCE_KM of `dir` — geometry
    /// (distance, flow) from the winner, but the water LEVEL blended across
    /// every in-influence segment by inverse closeness. Winner-take-all
    /// levels are discontinuous along the Voronoi bisector between two
    /// reaches (a hairpin bend's arms, a tributary beside the main stem),
    /// which rendered as a staircase water CLIFF running mid-channel
    /// (BUGS.md W-2). Levels along one course are continuous by export, so
    /// blending only acts where independent reaches meet — the cliff
    /// becomes a smooth ramp confined to the overlap zone, reading as
    /// rapids. Weights peak inside a channel (w -> 1 at dist 0) and die at
    /// the influence edge, so a lone segment's level is exactly its own.
    pub fn river_near(&self, face: usize, u: f64, v: f64, dir: DVec3) -> Option<RiverHit> {
        let ids = &self.seg_buckets[bucket_of(face, u, v)];
        if ids.is_empty() {
            return None;
        }
        let p = dir * self.radius_km;
        let mut best: Option<RiverHit> = None;
        let mut wsum = 0.0f64;
        let mut lsum = 0.0f64;
        for &id in ids {
            let s = &self.segments[id as usize];
            let a = s.a * self.radius_km;
            let ab = s.b * self.radius_km - a;
            let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            let d = (p - (a + ab * t)).length();
            if d >= SEG_INFLUENCE_KM {
                continue;
            }
            let level = s.level_a_km as f64 + (s.level_b_km - s.level_a_km) as f64 * t;
            // closeness^2 so the owning channel dominates its own water
            // level, but a lower neighbouring reach still bends the surface
            // down before the bisector instead of cliffing at it
            let w = ((1.0 - d / SEG_INFLUENCE_KM) * (1.0 - d / SEG_INFLUENCE_KM)).max(1e-6);
            wsum += w;
            lsum += w * level;
            if best.as_ref().is_none_or(|b| d < b.dist_km) {
                best = Some(RiverHit {
                    dist_km: d,
                    level_km: level,
                    flow: 10f64.powf(s.flow_log as f64) - 1.0,
                });
            }
        }
        best.map(|mut b| {
            b.level_km = lsum / wsum;
            b
        })
    }

    /// Lake candidate at `dir`. Pure geometry: the nearest lake cell (its
    /// level/salt/center/radius) plus where the query sits relative to the
    /// lake's Voronoi territory and its RIM ring (the sim's dry cells around
    /// the lake — by construction of the spill level, every rim cell's
    /// elevation is >= the lake level, so the rim is the dam).
    ///
    /// The flood decision belongs to terrain::sample, which combines this
    /// with the planet rasters. History, because this geometry has been
    /// wrong in two opposite ways: the original hard rule (flood only inside
    /// lake-cell Voronoi) left below-level noise dips just outside the
    /// footprint dry — vertical water walls at the shore (lake 414). The
    /// 586a828 fix flooded the whole 3-radius disc regardless of the rim,
    /// which let a +70 m coastal lake spill across its dam and stand against
    /// the open sea (BUGS.md W-1). The truth needs both distances, so both
    /// are returned.
    pub fn lake_at(&self, face: usize, u: f64, v: f64, dir: DVec3) -> Option<LakeHit> {
        let ids = &self.lake_buckets[bucket_of(face, u, v)];
        if ids.is_empty() {
            return None;
        }
        let p = dir * self.radius_km;
        let mut best_lake: Option<(f64, &LakeCell)> = None; // nearest true lake cell
        let mut best_rim: Option<(f64, &LakeCell)> = None; // nearest dry rim cell
        let mut d_any = f64::INFINITY; // nearest cell of either kind
        let mut any_is_lake = false;
        let mut any_rim_elev = f64::NEG_INFINITY; // rim rows carry elevation
        for &id in ids {
            let l = &self.lakes[id as usize];
            let d = (p - l.center * self.radius_km).length();
            if d < d_any {
                d_any = d;
                any_is_lake = !l.rim;
                any_rim_elev = if l.rim { l.level_km as f64 } else { f64::NEG_INFINITY };
            }
            if !l.rim && best_lake.as_ref().is_none_or(|b| d < b.0) {
                best_lake = Some((d, l));
            }
            if l.rim && best_rim.as_ref().is_none_or(|b| d < b.0) {
                best_rim = Some((d, l));
            }
        }
        match best_lake {
            // 3.4 r: hit coverage must OUTLAST the flood (bounded at 2.6 r in
            // terrain::sample) by the apron's grading reach, or the water and
            // its bank get clipped mid-basin by the search radius itself —
            // that truncation stood as a 170 m wall at 16.569 -32.262
            Some((d, l)) if d < l.radius_km as f64 * 3.4 => {
                // Exact distance to the perpendicular bisector of the two
                // cells that define the lake-vs-rim Voronoi decision. Chord
                // space is already lake_at's metric; at ~30 km cells on an
                // 8,660 km planet its difference from surface distance is
                // negligible. This is color/material metadata only.
                let boundary_dist = best_rim.map_or(f64::INFINITY, |(dr, r)| {
                    let centers_km = (l.center - r.center).length() * self.radius_km;
                    ((dr * dr - d * d).abs() / (2.0 * centers_km.max(1e-9))).max(0.0)
                });
                let past_boundary = if any_is_lake { 0.0 } else { (d - d_any).max(0.0) };
                // distance past every dam rim's shore band, minimized: each
                // dam rim floods its own 1.15 r disc, so the apron must cone
                // out from EVERY flooded frontier, not just the lake-Voronoi
                // one (rim rows carry their elevation in level_km; only rims
                // at dam height for THIS lake's level count — a lower
                // cascade neighbor's rims must not hoist its ground)
                let mut apron_past = past_boundary;
                if !any_is_lake && apron_past > 0.0 {
                    let band = l.radius_km as f64 * 1.15;
                    for &id in ids {
                        let r = &self.lakes[id as usize];
                        if !r.rim || (r.level_km as f64) + 0.03 < l.level_km as f64 {
                            continue;
                        }
                        let dr = (p - r.center * self.radius_km).length();
                        apron_past = apron_past.min((dr - band).max(0.0));
                        if apron_past == 0.0 {
                            break;
                        }
                    }
                }
                // the flood region is (voronoi ∪ dam bands) ∩ {d < 2.6 r},
                // so the apron distance is the MAX of the two violations:
                // past the voronoi/band frontiers (computed above) and past
                // the 2.6 r lake-distance bound. Min-ing the bound in
                // instead let a far rim's band zero the distance inside a
                // phantom pool the 2.6 r rule had just removed — the apron
                // then built a 170 m dirt mesa where the fiction water was.
                apron_past = apron_past.max((d - l.radius_km as f64 * 2.6).max(0.0));
                Some(LakeHit {
                    level_km: l.level_km as f64,
                    salt: l.salt,
                    d_lake_km: d,
                    lake_center: l.center,
                    radius_km: l.radius_km as f64,
                    boundary_dist_km: boundary_dist,
                    in_lake_voronoi: any_is_lake,
                    past_boundary_km: past_boundary,
                    apron_past_km: apron_past,
                    rim_is_dam: any_is_lake
                        || any_rim_elev + 0.03 >= l.level_km as f64,
                })
            }
            _ => None,
        }
    }
}

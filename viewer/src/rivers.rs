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
                // whose nearest cell might be this one
                let pad = (l.radius_km as f64 * 3.0 + 2.0) / self.radius_km * 3.2;
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

    /// Nearest river segment within SEG_INFLUENCE_KM of `dir`.
    pub fn river_near(&self, face: usize, u: f64, v: f64, dir: DVec3) -> Option<RiverHit> {
        let ids = &self.seg_buckets[bucket_of(face, u, v)];
        if ids.is_empty() {
            return None;
        }
        let p = dir * self.radius_km;
        let mut best: Option<RiverHit> = None;
        for &id in ids {
            let s = &self.segments[id as usize];
            let a = s.a * self.radius_km;
            let ab = s.b * self.radius_km - a;
            let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            let d = (p - (a + ab * t)).length();
            if d < SEG_INFLUENCE_KM && best.as_ref().is_none_or(|b| d < b.dist_km) {
                best = Some(RiverHit {
                    dist_km: d,
                    level_km: s.level_a_km as f64 + (s.level_b_km - s.level_a_km) as f64 * t,
                    flow: 10f64.powf(s.flow_log as f64) - 1.0,
                });
            }
        }
        best
    }

    /// Lake water level at `dir`: inside the lake's Voronoi footprint (the
    /// nearest map cell is a lake cell, not a rim cell).
    pub fn lake_at(&self, face: usize, u: f64, v: f64, dir: DVec3) -> Option<(f64, bool)> {
        let ids = &self.lake_buckets[bucket_of(face, u, v)];
        if ids.is_empty() {
            return None;
        }
        let p = dir * self.radius_km;
        let mut best: Option<(f64, &LakeCell)> = None;
        for &id in ids {
            let l = &self.lakes[id as usize];
            let d = (p - l.center * self.radius_km).length_squared();
            if best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, l));
            }
        }
        match best {
            Some((d2, l)) if !l.rim && d2.sqrt() < l.radius_km as f64 * 3.0 => {
                Some((l.level_km as f64, l.salt))
            }
            _ => None,
        }
    }
}

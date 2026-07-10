//! Planet data: baked cube-face rasters + cube-sphere math.
//!
//! Face convention must match scripts/bake_faces.py:
//!   direction(u, v) = normalize(axis + u*right + v*up),  u, v in [-1, 1]

use anyhow::{Context, Result};
use glam::DVec3;

pub const FACES: [(DVec3, DVec3, DVec3); 6] = [
    (DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(-1.0, 0.0, 0.0), DVec3::new(0.0, -1.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, -1.0, 0.0), DVec3::new(1.0, 0.0, 0.0), DVec3::new(0.0, 0.0, 1.0)),
    (DVec3::new(0.0, 0.0, 1.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(-1.0, 0.0, 0.0)),
    (DVec3::new(0.0, 0.0, -1.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(1.0, 0.0, 0.0)),
];

/// The canonical one-metre-ish column lattice. Biome dithering hashes this
/// identity in both renderers; changing it would move every ecotone column.
pub const COLUMNS_PER_FACE: u64 = 10_000_000;

/// Total width of a cross-material ecotone (half on either side of the baked
/// Koppen edge). Same-material classes never consult this band.
pub const CROSS_BLOCK_ECOTONE_KM: f64 = 0.300;
/// Koppen contributes a small, spatially blended hue memory; continuous
/// temperature and precipitation remain the dominant color coordinates.
pub const KOPPEN_HUE_NUDGE: f32 = 0.14;

const ECOTONE_HASH_SALT: u64 = 0xEC07_0AE;
const SNOW_HASH_SALT: u64 = 0x5A0E;
const SNOWLINE_CENTER_C: f64 = -9.0;
const SNOWLINE_HALF_RANGE_C: f64 = 1.5;

/// The categories Koppen actually selects in today's surface material code.
/// Rock and stone are local-slope overrides, not biome main blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MainBlock {
    Grass,
    Sand,
    Snow,
}

pub fn koppen_main_block(id: u8) -> MainBlock {
    match id {
        3 | 4 | 255 => MainBlock::Sand,
        29 => MainBlock::Snow,
        _ => MainBlock::Grass,
    }
}

/// The shared climate/material result evaluated once per mesh vertex or voxel
/// column. All three tints are carried because a local beach/cliff rule may
/// override the biome's main block without another raster pass.
#[derive(Clone, Copy, Debug)]
pub struct ClimateSurface {
    pub main_block: MainBlock,
    grass: [f32; 3],
    sand: [f32; 3],
    snow: [f32; 3],
    /// Existing far-canopy approximation, spatially blended so it cannot put
    /// a second hard line back across a smooth same-block transition.
    pub forest: f32,
}

impl ClimateSurface {
    pub fn tint(self, block: MainBlock) -> [f32; 3] {
        match block {
            MainBlock::Grass => self.grass,
            MainBlock::Sand => self.sand,
            MainBlock::Snow => self.snow,
        }
    }
}

/// Shared splitmix-style column hash. It intentionally preserves the exact
/// voxel hash that already owns snowline statistics.
pub fn hash_u64(face: u8, ci: u64, cj: u64, salt: u64) -> u64 {
    let mut x = (ci ^ ((face as u64) << 60))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(cj.wrapping_mul(0x85EB_CA77_C2B2_AE63))
        .wrapping_add(salt.wrapping_mul(0xC2B2_AE3D_27D4_EB4F));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

pub fn hash01(face: u8, ci: u64, cj: u64, salt: u64) -> f64 {
    ((hash_u64(face, ci, cj, salt) >> 11) & 0xFFFF_FFFF) as f64 / 4294967296.0
}

/// Unit direction for face-local coordinates (u, v) in [-1, 1].
pub fn face_dir(face: usize, u: f64, v: f64) -> DVec3 {
    let (axis, right, up) = FACES[face];
    (axis + u * right + v * up).normalize()
}

/// Inverse gnomonic projection: which face a direction hits, and where.
pub fn face_from_dir(dir: DVec3) -> (usize, f64, f64) {
    let mut best = (0usize, f64::MIN);
    for (i, (axis, _, _)) in FACES.iter().enumerate() {
        let d = dir.dot(*axis);
        if d > best.1 {
            best = (i, d);
        }
    }
    let (axis, right, up) = FACES[best.0];
    let p = dir / dir.dot(axis);
    (best.0, p.dot(right), p.dot(up))
}

pub struct FaceRaster {
    pub res: usize,
    pub elev_km: Vec<f32>,
    pub koppen: Vec<u8>, // 255 = ocean
    pub rough_km: Vec<f32>,     // mean |elevation delta| between map cells
    pub precip_mm_yr: Vec<f32>, // annual precipitation
    pub temp_c: Vec<f32>,       // annual mean temperature
    pub flow_log10: Vec<f32>,   // log10(1 + river flow accumulation m3/s)
    /// Blurred is-ocean mask (0 = interior land, 1 = open sea), derived from
    /// koppen==255 at load. "Below sea level" alone is NOT ocean: the map has
    /// genuine dry depressions, and elevation dips a few meters under zero
    /// all along the coasts. The blur (radius 2 texels) keeps one mislabeled
    /// texel from drying out a strait or flooding an inland dip.
    pub ocean: Vec<f32>,
    /// UNBLURRED is-ocean mask (koppen==255 as 0/1). Bilinear over this is
    /// the map's own cell-resolution coastline — the authority on which side
    /// of the shore a point is. Sub-sea-level interpolation tongues reaching
    /// inland of it are undershoot artifacts, not water.
    pub water: Vec<f32>,
}

pub struct Planet {
    pub radius_km: f64,
    pub seed: i64,
    pub faces: Vec<FaceRaster>,
    /// River courses + lakes from the drainage graph (empty if rivers.bin
    /// is missing — run scripts/bake_rivers.py).
    pub rivers: crate::rivers::RiverIndex,
}

impl Planet {
    pub fn load(dir: &str) -> Result<Self> {
        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(format!("{dir}/meta.json"))
                .context("missing viewer/assets/meta.json - run scripts/bake_faces.py")?,
        )?;
        let res = meta["resolution"].as_u64().unwrap() as usize;
        let radius_km = meta["radius_km"].as_f64().unwrap();
        let seed = meta["seed"].as_i64().unwrap_or(42);
        let mut faces = Vec::new();
        for fi in 0..6 {
            let raw = std::fs::read(format!("{dir}/face_{fi}.bin"))?;
            let n = res * res;
            anyhow::ensure!(
                raw.len() == n * 21,
                "face_{fi}.bin has unexpected size - rerun scripts/bake_faces.py (format now carries rough/precip/temp/flow layers)"
            );
            let f32s = |off: usize| -> Vec<f32> {
                raw[off..off + n * 4]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect()
            };
            let elev_km = f32s(0);
            let koppen = raw[n * 4..n * 5].to_vec();
            let rough_km = f32s(n * 5);
            let precip_mm_yr = f32s(n * 9);
            let temp_c = f32s(n * 13);
            let flow_log10 = f32s(n * 17);
            let ocean = blur_mask(&koppen, res, 2);
            let water = koppen.iter().map(|&k| (k == 255) as u8 as f32).collect();
            faces.push(FaceRaster {
                res,
                elev_km,
                koppen,
                rough_km,
                precip_mm_yr,
                temp_c,
                flow_log10,
                ocean,
                water,
            });
        }
        // The per-face blur above is edge-CLAMPED, so each face averages only
        // its own interior near a shared cube edge — the derived ocean value
        // then DISAGREES between two faces at the very same world direction
        // (BUGS.md: 0.32 via face 0 vs 0.56 via face 3 at lat -14.457 lon -45,
        // a half-screen navy/sand split at the seam). Re-derive the near-edge
        // band over true neighbor data and force shared texels bit-identical.
        seam_exact_ocean(&mut faces, 2);
        let rivers = match crate::rivers::RiverIndex::load(&format!("{dir}/rivers.bin"), radius_km)
        {
            Ok(r) => {
                println!("rivers: {} segments, {} lake cells", r.segments.len(), r.lakes.len());
                r
            }
            Err(e) => {
                eprintln!(
                    "no river data ({e}) - rivers/lakes disabled; run scripts/bake_rivers.py"
                );
                crate::rivers::RiverIndex::empty(radius_km)
            }
        };
        Ok(Self { radius_km, seed, faces, rivers })
    }

    /// Bilinear sample of a per-face f32 layer at (u, v) in [-1, 1].
    /// Rasters are edge-inclusive: texel 0 and res-1 lie exactly on the face
    /// edge, making adjacent faces agree along shared cube edges.
    fn bilinear(&self, face: usize, layer: impl Fn(&FaceRaster) -> &[f32], u: f64, v: f64) -> f32 {
        let r = &self.faces[face];
        let data = layer(r);
        let res = r.res as f64;
        let x = ((u * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let y = ((v * 0.5 + 0.5) * (res - 1.0)).clamp(0.0, res - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(r.res - 1), (y0 + 1).min(r.res - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let at = |xx: usize, yy: usize| data[yy * r.res + xx];
        let a = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let b = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        a * (1.0 - fy) + b * fy
    }

    /// Bilinear elevation (km).
    pub fn elevation(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.elev_km, u, v)
    }

    /// Map-scale roughness (km of elevation delta between ~30 km cells).
    pub fn rough(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.rough_km, u, v)
    }

    /// Annual precipitation (mm/yr).
    pub fn precip(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.precip_mm_yr, u, v)
    }

    /// Annual mean temperature (deg C).
    pub fn temp(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.temp_c, u, v)
    }

    /// log10(1 + river flow accumulation m3/s), dilated one map cell.
    pub fn flow(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.flow_log10, u, v)
    }

    /// Blurred open-ocean fraction at (u, v): ~1 on the sea, ~0 in interior
    /// land — including dry below-sea-level basins the map keeps as land.
    pub fn ocean(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.ocean, u, v)
    }

    /// Sharp (unblurred) ocean fraction at (u, v). >= 0.5 means this point
    /// is on the ocean side of the map's cell-resolution coastline.
    pub fn water_frac(&self, face: usize, u: f64, v: f64) -> f32 {
        self.bilinear(face, |r| &r.water, u, v)
    }

    /// Nearest-texel Koppen class id at (u, v); 255 = ocean.
    pub fn koppen(&self, face: usize, u: f64, v: f64) -> u8 {
        let r = &self.faces[face];
        let res = r.res as f64;
        let x = (((u * 0.5 + 0.5) * (res - 1.0)).round().max(0.0) as usize).min(r.res - 1);
        let y = (((v * 0.5 + 0.5) * (res - 1.0)).round().max(0.0) as usize).min(r.res - 1);
        r.koppen[y * r.res + x]
    }

    /// Categorical texel access with a real neighbor across cube-face edges.
    /// Interior transition checks are four direct byte reads; only the outer
    /// raster ring pays for a direction remap.
    fn koppen_texel(&self, face: usize, x: i64, y: i64) -> u8 {
        let r = &self.faces[face];
        if x >= 0 && x < r.res as i64 && y >= 0 && y < r.res as i64 {
            return r.koppen[y as usize * r.res + x as usize];
        }
        let d = (r.res - 1) as f64;
        let u = -1.0 + 2.0 * x as f64 / d;
        let v = -1.0 + 2.0 * y as f64 / d;
        let (f2, u2, v2) = face_from_dir(face_dir(face, u, v));
        self.koppen(f2, u2, v2)
    }

    /// Smooth the old palette anchor and far-forest weight over the categorical
    /// raster lattice. This signal is only a small nudge; unlike nearest-texel
    /// Koppen it is continuous at cell edges.
    fn koppen_signature(&self, face: usize, u: f64, v: f64) -> ([f32; 3], f32) {
        let r = &self.faces[face];
        let d = (r.res - 1) as f64;
        let x = ((u * 0.5 + 0.5) * d).clamp(0.0, d);
        let y = ((v * 0.5 + 0.5) * d).clamp(0.0, d);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(r.res - 1), (y0 + 1).min(r.res - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let sample = |xx: usize, yy: usize| {
            let k = r.koppen[yy * r.res + xx];
            (ground_tint(k), koppen_forest(k))
        };
        let (a, fa) = sample(x0, y0);
        let (b, fb) = sample(x1, y0);
        let (c, fc) = sample(x0, y1);
        let (d, fd) = sample(x1, y1);
        let top = mix3(a, b, fx);
        let bottom = mix3(c, d, fx);
        (
            mix3(top, bottom, fy),
            (fa + (fb - fa) * fx) * (1.0 - fy) + (fc + (fd - fc) * fx) * fy,
        )
    }

    /// Select the neighbor class only inside the short band around a
    /// DIFFERENT-main-block edge. The probability is 1/2 at the exact edge
    /// and eases to zero at either outer edge; same-block pairs return the
    /// nearest class without hashing at all.
    fn dithered_koppen(&self, face: usize, u: f64, v: f64, hash: f64) -> u8 {
        let r = &self.faces[face];
        let d = (r.res - 1) as f64;
        let x = ((u * 0.5 + 0.5) * d).clamp(0.0, d);
        let y = ((v * 0.5 + 0.5) * d).clamp(0.0, d);
        let (xi, yi) = (x.round() as i64, y.round() as i64);
        let here = self.koppen_texel(face, xi, yi);
        let here_block = koppen_main_block(here);

        // Gnomonic metric: angular derivative of normalize([u,v,1]). This
        // keeps the artist-facing width in kilometres across face centers,
        // edges, and corners instead of treating a raster texel as constant.
        let denom = 1.0 + u * u + v * v;
        let texel_uv = 2.0 / d;
        let km_x = self.radius_km * texel_uv * (1.0 + v * v).sqrt() / denom;
        let km_y = self.radius_km * texel_uv * (1.0 + u * u).sqrt() / denom;
        let candidates = [
            (xi - 1, yi, (x - (xi as f64 - 0.5)).max(0.0) * km_x),
            (xi + 1, yi, ((xi as f64 + 0.5) - x).max(0.0) * km_x),
            (xi, yi - 1, (y - (yi as f64 - 0.5)).max(0.0) * km_y),
            (xi, yi + 1, ((yi as f64 + 0.5) - y).max(0.0) * km_y),
        ];
        let mut nearest: Option<(f64, u8)> = None;
        for (nx, ny, dist_km) in candidates {
            let other = self.koppen_texel(face, nx, ny);
            if koppen_main_block(other) == here_block {
                continue;
            }
            if nearest.is_none_or(|(best, _)| dist_km < best) {
                nearest = Some((dist_km, other));
            }
        }
        let Some((dist_km, other)) = nearest else {
            return here;
        };
        let half = CROSS_BLOCK_ECOTONE_KM * 0.5;
        if dist_km >= half {
            return here;
        }
        let t = (dist_km / half).clamp(0.0, 1.0);
        let ease = 1.0 - t * t * (3.0 - 2.0 * t);
        if hash < 0.5 * ease { other } else { here }
    }
}

/// Separable box blur of the (koppen == 255) ocean mask, edge-clamped.
fn blur_mask(koppen: &[u8], res: usize, radius: i32) -> Vec<f32> {
    let mask: Vec<f32> = koppen.iter().map(|&k| (k == 255) as u8 as f32).collect();
    let span = (2 * radius + 1) as f32;
    let mut tmp = vec![0f32; mask.len()];
    for y in 0..res {
        for x in 0..res {
            let mut s = 0.0;
            for d in -radius..=radius {
                let xx = (x as i32 + d).clamp(0, res as i32 - 1) as usize;
                s += mask[y * res + xx];
            }
            tmp[y * res + x] = s / span;
        }
    }
    let mut out = vec![0f32; mask.len()];
    for y in 0..res {
        for x in 0..res {
            let mut s = 0.0;
            for d in -radius..=radius {
                let yy = (y as i32 + d).clamp(0, res as i32 - 1) as usize;
                s += tmp[yy * res + x];
            }
            out[y * res + x] = s / span;
        }
    }
    out
}

/// Make the derived ocean mask agree across shared cube edges.
///
/// `blur_mask` runs per face with EDGE-CLAMPED taps: near a shared edge a
/// face averages only its own interior, so face A and face B derive different
/// ocean fractions for the identical world direction. Downstream
/// `terrain::sample` classifies sea from that fraction, so the coastline can
/// flip across a seam (the reported navy/sand half-screen split). Two-step,
/// load-time, raster-level fix (no bake-format change):
///
///  1. INTERIOR texels (further than `radius` from every edge) keep the fast
///     separable blur already in `faces[*].ocean` — bit-identical to before,
///     so coastlines away from seams do not move.
///  2. BORDER texels (within `radius` of an edge) are re-blurred with a direct
///     (2r+1)² gather whose off-face taps resolve by DIRECTION onto the
///     neighbor face's raw mask (the ghost-ring trick `voxel::canonical_column`
///     uses at column level), so they average TRUE neighbor data instead of
///     clamped own-face data — this kills the 0.32-vs-0.56 magnitude error.
///  3. SHARED texels (on an edge/corner) are then overwritten with their owner
///     face's value. `face_from_dir` is the codebase's canonical tie-breaker,
///     so every face's copy of a shared texel converges on one owner value —
///     bit-identical, corners (three faces) included. Bilinear exactly on an
///     edge uses only edge texels, so this makes the sea classification
///     seam-free by construction.
///
/// All six border re-blurs run BEFORE the canonical copy so the copy reads
/// finished (ghost-ring) values.
fn seam_exact_ocean(faces: &mut [FaceRaster], radius: i32) {
    let n = faces.len();
    let res = faces[0].res;
    let resf = res as f64;
    let r = radius as usize;
    let span2 = ((2 * radius + 1) * (2 * radius + 1)) as f32;

    // raw 0/1 ocean masks (koppen == 255) for every face, for the gather.
    let masks: Vec<Vec<f32>> = faces
        .iter()
        .map(|f| f.koppen.iter().map(|&k| (k == 255) as u8 as f32).collect())
        .collect();
    // start from the existing per-face separable blur (interior already final).
    let mut blurred: Vec<Vec<f32>> = faces.iter().map(|f| f.ocean.clone()).collect();

    // Edge-inclusive raster index -> texel-center face coordinate: texel k
    // centers at u = -1 + 2k/(res-1), so texel 0 and res-1 sit ON the edge.
    let uv_of = |x: i64, y: i64| (-1.0 + 2.0 * x as f64 / (resf - 1.0), -1.0 + 2.0 * y as f64 / (resf - 1.0));
    // nearest texel index for a face coordinate (matches koppen()/bilinear).
    let texel = |u: f64, v: f64| -> usize {
        let x = (((u * 0.5 + 0.5) * (resf - 1.0)).round().max(0.0) as usize).min(res - 1);
        let y = (((v * 0.5 + 0.5) * (resf - 1.0)).round().max(0.0) as usize).min(res - 1);
        y * res + x
    };
    // raw mask at a (possibly off-face) integer texel: off-face indices are
    // resolved by direction onto the neighbor face's nearest texel.
    let raw_at = |face: usize, x: i64, y: i64| -> f32 {
        if x >= 0 && x < res as i64 && y >= 0 && y < res as i64 {
            masks[face][y as usize * res + x as usize]
        } else {
            let (u, v) = uv_of(x, y);
            let (f2, u2, v2) = face_from_dir(face_dir(face, u, v));
            masks[f2][texel(u2, v2)]
        }
    };

    // 2. re-blur the border band over direction-resolved neighbor data.
    for face in 0..n {
        for y in 0..res {
            for x in 0..res {
                let border = x < r || x + r >= res || y < r || y + r >= res;
                if !border {
                    continue;
                }
                let mut s = 0.0f32;
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        s += raw_at(face, x as i64 + dx as i64, y as i64 + dy as i64);
                    }
                }
                blurred[face][y * res + x] = s / span2;
            }
        }
    }

    // 3. canonicalize shared texels to their owner face's (finished) value.
    let mut out = blurred.clone();
    for face in 0..n {
        for y in 0..res {
            for x in 0..res {
                if !(x == 0 || x == res - 1 || y == 0 || y == res - 1) {
                    continue;
                }
                let (u, v) = uv_of(x as i64, y as i64);
                let (owner, u2, v2) = face_from_dir(face_dir(face, u, v));
                if owner == face {
                    continue;
                }
                out[face][y * res + x] = blurred[owner][texel(u2, v2)];
            }
        }
    }

    for (f, o) in faces.iter_mut().zip(out) {
        f.ocean = o;
    }
}

/// Legacy Koppen palette anchor (linear RGB). The minimap still uses it
/// directly; world surfaces only take a small, bilinearly-smoothed hue nudge
/// from it through `climate_surface`.
pub fn ground_tint(id: u8) -> [f32; 3] {
    match id {
        0 | 1 => [0.050, 0.230, 0.040],  // Af/Am tropical rainforest
        2 => [0.200, 0.240, 0.055],      // Aw savanna gold-green
        3 => [0.480, 0.360, 0.190],      // BWh hot desert sand
        4 => [0.380, 0.310, 0.210],      // BWk cold desert
        5 => [0.300, 0.260, 0.095],      // BSh hot steppe
        6 => [0.260, 0.240, 0.110],      // BSk cold steppe
        7 | 8 | 9 => [0.180, 0.230, 0.070],   // Cs* mediterranean olive
        10 | 11 | 12 => [0.090, 0.240, 0.055], // Cw* subtropical highland
        13 | 14 | 15 => [0.085, 0.250, 0.048], // Cf* temperate green
        16 | 17 | 18 | 19 => [0.150, 0.210, 0.085], // Ds* dry continental
        20 | 21 | 22 | 23 => [0.095, 0.210, 0.060], // Dw* monsoon continental
        24 | 25 => [0.085, 0.220, 0.052],      // Dfa/Dfb humid continental
        26 | 27 => [0.055, 0.150, 0.062],      // Dfc/Dfd taiga blue-green
        28 => [0.220, 0.215, 0.150],           // ET tundra grey-green
        29 => [0.780, 0.820, 0.880],           // EF ice cap
        // 255 (ocean) on LAND happens routinely: elevation interpolation
        // overshoots above zero across texels whose nearest map cell is
        // ocean — coastal strands. Blocks fall back to sand there; the old
        // navy "ocean floor" tint here painted the same strands as blue
        // plates on every desert coast.
        _ => [0.52, 0.45, 0.27],
    }
}

fn koppen_forest(id: u8) -> f32 {
    match id {
        0 | 1 => 0.85,
        10..=15 | 20 | 21 | 24 | 25 => 0.5,
        22 | 23 | 26 | 27 => 0.6,
        16..=19 => 0.4,
        7..=9 => 0.3,
        2 => 0.15,
        _ => 0.0,
    }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smoothstepf(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Four temperature anchors for provisional climate colors. Smooth segments
/// avoid putting a new contour at an anchor temperature.
fn temperature_ramp(temp_c: f32, colors: [[f32; 3]; 4]) -> [f32; 3] {
    const T: [f32; 4] = [-4.0, 4.0, 16.0, 26.0];
    if temp_c <= T[1] {
        return mix3(colors[0], colors[1], smoothstepf(T[0], T[1], temp_c));
    }
    if temp_c <= T[2] {
        return mix3(colors[1], colors[2], smoothstepf(T[1], T[2], temp_c));
    }
    mix3(colors[2], colors[3], smoothstepf(T[2], T[3], temp_c))
}

fn climate_grass(temp_c: f32, precip_mm: f32) -> [f32; 3] {
    // Annual rain must work harder in heat. This is deliberately a simple
    // smooth 2-D ramp, not a second climate classifier; seasonal color waits
    // for texture anchors as Andrew directed.
    let dry_limit = 100.0 + temp_c.max(0.0) * 35.0;
    let moisture = smoothstepf(dry_limit, dry_limit + 900.0, precip_mm.max(0.0));
    let dry = temperature_ramp(
        temp_c,
        [
            [0.220, 0.215, 0.150],
            [0.260, 0.240, 0.110],
            [0.180, 0.230, 0.070],
            [0.205, 0.240, 0.055],
        ],
    );
    let wet = temperature_ramp(
        temp_c,
        [
            [0.070, 0.160, 0.070],
            [0.075, 0.205, 0.060],
            [0.090, 0.250, 0.050],
            [0.050, 0.230, 0.040],
        ],
    );
    mix3(dry, wet, moisture)
}

fn climate_sand(temp_c: f32, precip_mm: f32) -> [f32; 3] {
    let heat = smoothstepf(-4.0, 26.0, temp_c);
    let mut sand = mix3([0.545, 0.470, 0.290], [0.585, 0.485, 0.255], heat);
    let damp = smoothstepf(250.0, 1400.0, precip_mm.max(0.0)) * 0.10;
    sand = mix3(sand, [0.485, 0.430, 0.270], damp);
    sand
}

fn climate_snow(temp_c: f32, precip_mm: f32) -> [f32; 3] {
    let warmth = smoothstepf(-30.0, -4.0, temp_c);
    let wet = smoothstepf(50.0, 900.0, precip_mm.max(0.0));
    let snow = mix3([0.795, 0.835, 0.900], [0.835, 0.865, 0.905], warmth);
    mix3(snow, [0.850, 0.875, 0.915], wet * 0.12)
}

fn hue_nudge(base: [f32; 3], anchor: [f32; 3]) -> [f32; 3] {
    // Match luminance before mixing: Koppen remembers hue/chroma without
    // regaining ownership of brightness (which belongs to climate + material).
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let scale = luma(base) / luma(anchor).max(1e-4);
    let same_value = [
        (anchor[0] * scale).clamp(0.0, 1.0),
        (anchor[1] * scale).clamp(0.0, 1.0),
        (anchor[2] * scale).clamp(0.0, 1.0),
    ];
    mix3(base, same_value, KOPPEN_HUE_NUDGE)
}

/// One truth for the two ground renderers: continuous climate supplies all
/// provisional material tints, smoothly sampled Koppen only nudges grass hue,
/// and the shared world-column hashes choose cross-block and snow categories.
/// This performs raster reads and hashes only; callers already own the terrain
/// `Sample`, so no extra `terrain::sample` enters either hot path.
pub fn climate_surface(
    planet: &Planet,
    face: usize,
    u: f64,
    v: f64,
    temp_c: f64,
    precip_mm: f64,
) -> ClimateSurface {
    // Only shared cube edges need canonical ownership. Voxel centers and tile
    // interiors keep the cheap direct path.
    let (face, u, v) = if u.abs() >= 1.0 || v.abs() >= 1.0 {
        face_from_dir(face_dir(face, u, v))
    } else {
        (face, u, v)
    };
    let n = COLUMNS_PER_FACE as f64;
    let ci = (((u + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let cj = (((v + 1.0) * 0.5 * n).clamp(0.0, n - 1.0)) as u64;
    let edge_hash = hash01(face as u8, ci, cj, ECOTONE_HASH_SALT);
    let snow_hash = hash01(face as u8, ci, cj, SNOW_HASH_SALT);

    let (koppen_anchor, forest) = planet.koppen_signature(face, u, v);
    let chosen = planet.dithered_koppen(face, u, v, edge_hash);
    let mut main_block = koppen_main_block(chosen);
    let snow_threshold =
        SNOWLINE_CENTER_C + (snow_hash - 0.5) * (SNOWLINE_HALF_RANGE_C * 2.0);
    if main_block == MainBlock::Snow || temp_c < snow_threshold {
        main_block = MainBlock::Snow;
    }

    let temp = temp_c as f32;
    let precip = precip_mm as f32;
    ClimateSurface {
        main_block,
        grass: hue_nudge(climate_grass(temp, precip), koppen_anchor),
        sand: climate_sand(temp, precip),
        snow: climate_snow(temp, precip),
        forest,
    }
}

/// Koppen class -> base color (matches planetgen/biomes.py palette, linearized-ish).
#[allow(dead_code)]
pub fn koppen_color(id: u8) -> [f32; 3] {
    const HEX: [u32; 30] = [
        0x0000fe, 0x0078ff, 0x46aafa, 0xff0000, 0xff9696, 0xf5a500, 0xffdc64, 0xffff00,
        0xc8c800, 0x969600, 0x96ff96, 0x64c864, 0x329632, 0xc8ff50, 0x64ff50, 0x32c800,
        0xff00fe, 0xc800c8, 0x963296, 0x966496, 0xabb1ff, 0x5a77db, 0x4b50b4, 0x320087,
        0x00ffff, 0x37c8ff, 0x007d7d, 0x00465f, 0xb2b2b2, 0x686868,
    ];
    if (id as usize) < HEX.len() {
        let h = HEX[id as usize];
        let s = |c: u32| (c as f32 / 255.0).powf(2.2); // rough sRGB -> linear
        [s((h >> 16) & 255), s((h >> 8) & 255), s(h & 255)]
    } else {
        [0.02, 0.09, 0.18] // ocean fallback (unused: water colored by depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain;

    fn climate_test_planet(right_class: u8) -> Planet {
        let res = 4usize;
        let n = res * res;
        let koppen: Vec<u8> = (0..res)
            .flat_map(|_| [6, 6, right_class, right_class])
            .collect();
        let face = || FaceRaster {
            res,
            elev_km: vec![0.2; n],
            koppen: koppen.clone(),
            rough_km: vec![0.0; n],
            precip_mm_yr: vec![500.0; n],
            temp_c: vec![8.0; n],
            flow_log10: vec![0.0; n],
            ocean: vec![0.0; n],
            water: vec![0.0; n],
        };
        Planet {
            radius_km: 1.0,
            seed: 42,
            faces: (0..6).map(|_| face()).collect(),
            rivers: crate::rivers::RiverIndex::empty(1.0),
        }
    }

    #[test]
    fn koppen_main_blocks_match_surface_materials() {
        for k in 0..=29 {
            let expected = match k {
                3 | 4 => MainBlock::Sand,
                29 => MainBlock::Snow,
                _ => MainBlock::Grass,
            };
            assert_eq!(koppen_main_block(k), expected, "Koppen class {k}");
        }
        assert_eq!(koppen_main_block(255), MainBlock::Sand);
    }

    #[test]
    fn category_dither_only_crosses_main_blocks() {
        // In a 4x4 raster the class edge is x=1.5 -> u=0. Just inside the
        // left cell, hash 0 selects the neighbor and hash 1 keeps the owner.
        let cross = climate_test_planet(4);
        let u_near = -0.006;
        assert_eq!(cross.dithered_koppen(0, u_near, 0.0, 0.0), 4);
        assert_eq!(cross.dithered_koppen(0, u_near, 0.0, 0.999), 6);
        // More than half the 300 m band away, no hash can cross the edge.
        assert_eq!(cross.dithered_koppen(0, -0.30, 0.0, 0.0), 6);

        // Classes 6 and 14 are both grass: even at the exact edge the
        // category remains nearest-texel and no random decision is made.
        let same = climate_test_planet(14);
        assert_eq!(same.dithered_koppen(0, u_near, 0.0, 0.0), 6);
        assert_eq!(same.dithered_koppen(0, u_near, 0.0, 0.999), 6);

        // The hue nudge itself is bilinear, so same-block color is continuous
        // across that nearest-class edge.
        let a = climate_surface(&same, 0, -1e-6, 0.0, 8.0, 900.0)
            .tint(MainBlock::Grass);
        let b = climate_surface(&same, 0, 1e-6, 0.0, 8.0, 900.0)
            .tint(MainBlock::Grass);
        assert!(a.into_iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5));
    }

    /// Project a world direction onto a specific face's gnomonic plane.
    /// `on` is true when the direction lies within this face's [-1,1]^2
    /// domain (a hair of slack so an exact edge direction registers on both
    /// of the faces that share it — three, at a corner).
    fn project(face: usize, dir: DVec3) -> (f64, f64, bool) {
        let (axis, right, up) = FACES[face];
        let d = dir.dot(axis);
        if d <= 0.0 {
            return (0.0, 0.0, false);
        }
        let p = dir / d;
        let (u, v) = (p.dot(right), p.dot(up));
        let slack = 1.0 + 1e-6;
        (u, v, u.abs() <= slack && v.abs() <= slack)
    }

    /// Snap a coordinate that should sit on the shared lattice to its exact
    /// value: an ULP-band around ±1 goes to exactly ±1 (the edge column),
    /// everything else to its nearest edge-inclusive texel center. This makes
    /// both faces read the SAME shared lattice point, where `face_dir` is
    /// bit-identical (each world axis gets exactly one signed-unit term, so
    /// the pre-normalize vector is order-independent).
    fn snap(x: f64, res: usize) -> f64 {
        if x.abs() > 1.0 - 1e-6 {
            x.signum()
        } else {
            let k = ((x * 0.5 + 0.5) * (res as f64 - 1.0)).round();
            -1.0 + 2.0 * k / (res as f64 - 1.0)
        }
    }

    /// The property the review asked for: on every shared cube edge and corner
    /// the derived ocean mask — and the sea classification `terrain::sample`
    /// draws from it — must be identical no matter which face samples the
    /// point. Loads the real baked planet; skips (does not fail) without
    /// assets so CI-less clones stay green.
    #[test]
    fn ocean_mask_seam_exact() {
        let planet = match Planet::load("assets") {
            Ok(p) => p,
            Err(e) => {
                eprintln!("skipping ocean_mask_seam_exact: no assets ({e})");
                return;
            }
        };
        let res = planet.faces[0].res;
        // (axis_kind, edge_value): 0 = u fixed, v marches; 1 = v fixed, u marches.
        let edges: [(u8, f64); 4] = [(0, -1.0), (0, 1.0), (1, -1.0), (1, 1.0)];
        let steps = 512usize;
        let mut checked = 0u64;
        for fa in 0..6usize {
            for &(kind, edge_val) in &edges {
                for s in 0..=steps {
                    let t = -1.0 + 2.0 * s as f64 / steps as f64;
                    let (ua, va) = if kind == 0 { (edge_val, t) } else { (t, edge_val) };
                    let (ua, va) = (snap(ua, res), snap(va, res));
                    let dir = face_dir(fa, ua, va);
                    let ocean_a = planet.ocean(fa, ua, va);
                    let samp_a = terrain::sample(&planet, fa, ua, va, 5);
                    for fb in 0..6usize {
                        if fb == fa {
                            continue;
                        }
                        let (ub, vb, on) = project(fb, dir);
                        if !on {
                            continue;
                        }
                        let (ubs, vbs) = (snap(ub, res), snap(vb, res));
                        // must genuinely be an edge/corner of fb, not interior
                        if ubs.abs() < 0.9999 && vbs.abs() < 0.9999 {
                            continue;
                        }
                        // sanity: the two representations must resolve the same
                        // world point. (They agree to the bit on identity edges;
                        // on reversal edges the tangential lattice index is
                        // reconstructed as -1+2(res-1-k)/(res-1), a real ULP off
                        // -(-1+2k/(res-1)), so dir can differ by 1 ULP — which
                        // the nearest-texel rounding downstream absorbs.)
                        let dir_b = face_dir(fb, ubs, vbs);
                        assert!(
                            dir.distance(dir_b) < 1e-9,
                            "seam faces disagree on the point: f{fa} {dir:?} vs f{fb} {dir_b:?}"
                        );
                        // (1) derived ocean fraction bit-identical
                        let ocean_b = planet.ocean(fb, ubs, vbs);
                        assert_eq!(
                            ocean_a.to_bits(),
                            ocean_b.to_bits(),
                            "ocean seam: f{fa}({ua},{va})={ocean_a} vs f{fb}({ubs},{vbs})={ocean_b}"
                        );
                        // (2) sea classification and water surface agree
                        let samp_b = terrain::sample(&planet, fb, ubs, vbs, 5);
                        assert_eq!(samp_a.sea, samp_b.sea, "sea seam f{fa} vs f{fb}");
                        let (wa, wb) = (samp_a.water_km, samp_b.water_km);
                        assert!(
                            wa.to_bits() == wb.to_bits() || (!wa.is_finite() && !wb.is_finite()),
                            "water_km seam f{fa}={wa} vs f{fb}={wb}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 0, "no seam pairs sampled");
        eprintln!("ocean_mask_seam_exact: {checked} seam pairs verified");
    }
}

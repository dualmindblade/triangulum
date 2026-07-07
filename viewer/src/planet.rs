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

/// Koppen class -> naturalistic ground tint (linear RGB). This is what the
/// landscape *looks like*, as opposed to koppen_color's atlas false-color:
/// rainforest greens, savanna golds, desert sands, taiga blue-greens,
/// tundra greys, ice-cap white. Shared by mesh shading and block materials.
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

//! The world bake: planetgen's output rasterised onto cube faces.
//!
//! `Bake` is the interface the generator samples through. `RasterBake`
//! holds edge-inclusive per-face rasters and reads either layout:
//! - the new world bake (`scripts/bake_world.py`, a `meta.json` next to
//!   `face_{0..5}.bin`): equal-angle layout, texels uniform in the lattice
//!   parameter p with a = tan(p pi/4), planar f32 layers then u8 layers at
//!   the byte offsets meta.json lists. Only the layers used are read.
//! - the old viewer bake (`viewer/assets/face_{0..5}.bin`): gnomonic
//!   layout, texels uniform in a; kept for comparison.
//! Both use the viewer's `FACES` frames. Smooth fields are sampled with
//! Catmull-Rom (C1); mask-like fields bilinearly.

use cubesphere::FACES;
use glam::DVec3;
use std::f64::consts::FRAC_PI_4;
use std::io::{self, Read, Seek, SeekFrom};

/// Climate, terrain and geology fields at one direction.
#[derive(Clone, Copy, Debug, Default)]
pub struct BakeSample {
    /// Elevation in km, Earth scale (negative = below sea level).
    pub elev_km: f64,
    pub temp_c: f64,
    pub precip_mm: f64,
    /// Local relief field in km (mean |elevation delta| to planetgen neighbours).
    pub rough_km: f64,
    /// Ocean fraction 0..1 (smooth in the new bake; koppen 255 in the old).
    pub ocean: f64,
    /// Soil/sediment depth in km (0 in the old bake).
    pub soil_km: f64,
    /// Geology hints 0..1 (0 in the old bake).
    pub continental: f64,
    pub volcanic: f64,
    /// Rock erodibility (1 = neutral; 0 in the old bake).
    pub erodibility: f64,
    /// Active tectonic uplift 0..1.2 (orogens, arcs; 0 in the old bake).
    pub uplift: f64,
    /// River discharge signal, log10(1 + m^3/s), regional (0 in the old bake).
    pub flow_log10: f64,
    /// Lake fraction 0..1 (0 in the old bake).
    pub lake: f64,
    /// Face whose raster was sampled (the face the direction points into).
    pub face: u8,
    /// Gradient of `elev_km` in that face's equal-angle chart: km per
    /// radian of u = atan(a) and v = atan(b) (a, b gnomonic). Divide by the
    /// radius to get km per metre along the chart axes.
    pub elev_du_km: f64,
    pub elev_dv_km: f64,
    /// The elevation smoothed by a uniform cubic B-spline (C2, so its
    /// curvature is continuous) and its first and second derivatives in the
    /// same chart (km per radian, km per radian^2): finds valley axes.
    pub smooth_du_km: f64,
    pub smooth_dv_km: f64,
    pub smooth_uu_km: f64,
    pub smooth_vv_km: f64,
    pub smooth_uv_km: f64,
    /// `flow_log10` smoothed by the same B-spline, and its derivatives
    /// (per radian, per radian^2): river courses are ridges of this field.
    pub flow_smooth: f64,
    pub flow_du: f64,
    pub flow_dv: f64,
    pub flow_uu: f64,
    pub flow_vv: f64,
    pub flow_uv: f64,
    /// Distance from the sample to the nearest edge of its face raster, in
    /// texels (derivatives are clamped within ~2 texels of an edge).
    pub edge_texels: f64,
}

pub trait Bake: Send + Sync {
    /// Smooth sample (C1 for elevation, temperature, precipitation).
    fn sample(&self, dir: DVec3) -> BakeSample;
    /// Conservative (min, max) elevation in km over a gnomonic rectangle on
    /// one face, padded so the smooth interpolant's overshoot stays inside.
    fn elev_range_km(&self, face: u8, a0: f64, b0: f64, a1: f64, b1: f64) -> (f32, f32);
    /// Largest roughness (km) over a gnomonic rectangle on one face.
    fn rough_max_range_km(&self, face: u8, a0: f64, b0: f64, a1: f64, b1: f64) -> f32;
    /// Lowest land elevation anywhere (km); the horizon occluder uses it.
    fn land_min_km(&self) -> f32;
    /// Largest roughness value anywhere (km); bounds the relief noise.
    fn rough_max_km(&self) -> f32;
    /// Visit every texel: (direction, elev_km, ocean, |gradient| km per texel).
    fn for_each_texel(&self, f: &mut dyn FnMut(DVec3, f32, bool, f32));
    fn describe(&self) -> String;
}

#[derive(Default)]
struct Face {
    elev_km: Vec<f32>,
    ocean: Vec<f32>,
    temp_c: Vec<f32>,
    precip: Vec<f32>,
    rough_km: Vec<f32>,
    soil_km: Vec<f32>,
    continental: Vec<f32>,
    volcanic: Vec<f32>,
    erodibility: Vec<f32>,
    uplift: Vec<f32>,
    flow_log10: Vec<f32>,
    lake: Vec<f32>,
    /// Min/max mip pyramids; level 0 is res x res.
    pyr_min: Vec<Vec<f32>>,
    pyr_max: Vec<Vec<f32>>,
    pyr_rough: Vec<Vec<f32>>,
}

pub struct RasterBake {
    res: usize,
    faces: Vec<Face>,
    /// Texels uniform in the equiangular parameter (new bake) or in the gnomonic coordinate (old).
    equiangular: bool,
    has_geology: bool,
    land_min_km: f32,
    rough_max_km: f32,
    dir: String,
}

/// Open whichever bake lives in `dir`: the new one if it has a meta.json
/// in the world-raster format, else the old viewer layout.
pub fn open(dir: &str) -> io::Result<RasterBake> {
    let meta = format!("{dir}/meta.json");
    if let Ok(text) = std::fs::read_to_string(&meta)
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
        && json.get("format").and_then(|f| f.as_str()) == Some("triangulum-world-raster")
    {
        return RasterBake::load_world(dir, &json);
    }
    RasterBake::load_old(dir)
}

fn read_f32_layer(file: &mut std::fs::File, offset: u64, n: usize) -> io::Result<Vec<f32>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut raw = vec![0u8; n * 4];
    file.read_exact(&mut raw)?;
    Ok(raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

impl RasterBake {
    /// The new world bake described by its meta.json.
    pub fn load_world(dir: &str, meta: &serde_json::Value) -> io::Result<Self> {
        let layout = &meta["layout"];
        let res = layout["resolution"].as_u64().ok_or_else(|| io::Error::other("meta.json: no resolution"))? as usize;
        if layout["projection"].as_str() != Some("equal_angle") || layout["edge_inclusive"].as_bool() != Some(true) {
            return Err(io::Error::other("meta.json: expected an edge-inclusive equal_angle layout"));
        }
        let pattern = layout["file_pattern"].as_str().unwrap_or("face_{face}.bin").to_string();
        let fields = meta["fields"].as_array().ok_or_else(|| io::Error::other("meta.json: no fields"))?;
        let offset = |name: &str| -> io::Result<u64> {
            let f = fields.iter().find(|f| f["name"].as_str() == Some(name)).ok_or_else(|| io::Error::other(format!("meta.json: no field {name}")))?;
            if f["type"].as_str() != Some("f32") {
                return Err(io::Error::other(format!("field {name} is not f32")));
            }
            f["offset"].as_u64().ok_or_else(|| io::Error::other(format!("field {name}: no offset")))
        };
        let n = res * res;
        let mut faces = Vec::with_capacity(6);
        for fi in 0..6 {
            let path = format!("{dir}/{}", pattern.replace("{face}", &fi.to_string()));
            let mut file = std::fs::File::open(&path)?;
            let mut rd = |name: &str| read_f32_layer(&mut file, offset(name)?, n);
            let mut face = Face {
                elev_km: rd("elevation_km")?,
                rough_km: rd("rough_km")?,
                temp_c: rd("temp_mean_c")?,
                precip: rd("precip_mm_yr")?,
                ocean: rd("ocean_frac")?,
                soil_km: rd("soil_km")?,
                continental: rd("continental_frac")?,
                volcanic: rd("volcanic_frac")?,
                erodibility: rd("erodibility")?,
                uplift: rd("uplift")?,
                flow_log10: rd("flow_log10")?,
                lake: rd("lake_frac")?,
                ..Default::default()
            };
            let (pyr_min, pyr_max) = build_pyramid(&face.elev_km, res);
            let (_, pyr_rough) = build_pyramid(&face.rough_km, res);
            face.pyr_min = pyr_min;
            face.pyr_max = pyr_max;
            face.pyr_rough = pyr_rough;
            faces.push(face);
        }
        Ok(Self::finish(res, faces, true, true, dir))
    }

    /// The old viewer bake. Layout per face (planar, little-endian,
    /// n = res*res): [0,4n) elev_km f32; [4n,5n) koppen u8 (255 = ocean);
    /// [5n,9n) rough_km; [9n,13n) precip_mm_yr; [13n,17n) temp_c;
    /// [17n,21n) flow_log10. Gnomonic, edge-inclusive: texel x = (a*0.5+0.5)*(res-1).
    pub fn load_old(dir: &str) -> io::Result<Self> {
        let mut faces = Vec::with_capacity(6);
        let mut res = 0usize;
        for fi in 0..6 {
            let raw = std::fs::read(format!("{dir}/face_{fi}.bin"))?;
            let n = raw.len() / 21;
            res = (n as f64).sqrt() as usize;
            if res * res * 21 != raw.len() {
                return Err(io::Error::other(format!("face_{fi}.bin: unexpected size {}", raw.len())));
            }
            let f32s = |off: usize| -> Vec<f32> { raw[off..off + n * 4].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect() };
            let elev_km = f32s(0);
            let ocean: Vec<f32> = raw[n * 4..n * 5].iter().map(|&k| if k == 255 { 1.0 } else { 0.0 }).collect();
            let rough_km = f32s(n * 5);
            let (pyr_min, pyr_max) = build_pyramid(&elev_km, res);
            let (_, pyr_rough) = build_pyramid(&rough_km, res);
            faces.push(Face {
                elev_km,
                ocean,
                temp_c: f32s(n * 13),
                precip: f32s(n * 9),
                rough_km,
                soil_km: Vec::new(),
                continental: Vec::new(),
                volcanic: Vec::new(),
                erodibility: Vec::new(),
                uplift: Vec::new(),
                flow_log10: Vec::new(),
                lake: Vec::new(),
                pyr_min,
                pyr_max,
                pyr_rough,
            });
        }
        Ok(Self::finish(res, faces, false, false, dir))
    }

    fn finish(res: usize, faces: Vec<Face>, equiangular: bool, has_geology: bool, dir: &str) -> Self {
        let mut land_min = f32::MAX;
        let mut rough_max = 0f32;
        for f in &faces {
            for (e, o) in f.elev_km.iter().zip(&f.ocean) {
                if *o < 0.5 {
                    land_min = land_min.min(*e);
                }
            }
            for r in &f.rough_km {
                rough_max = rough_max.max(*r);
            }
        }
        Self { res, faces, equiangular, has_geology, land_min_km: land_min, rough_max_km: rough_max, dir: dir.to_string() }
    }

    /// Texel parameter in [-1, 1] for a gnomonic coordinate.
    #[inline]
    fn param(&self, a: f64) -> f64 {
        if self.equiangular { a.atan() / FRAC_PI_4 } else { a }
    }

    #[inline]
    fn texel_coords(&self, a: f64, b: f64) -> (f64, f64) {
        let m = (self.res - 1) as f64;
        (((self.param(a) * 0.5 + 0.5) * m).clamp(0.0, m), ((self.param(b) * 0.5 + 0.5) * m).clamp(0.0, m))
    }

    /// Gnomonic face hit and (a, b) for a direction.
    #[inline]
    fn face_ab(dir: DVec3) -> (usize, f64, f64) {
        let mut best = (0usize, f64::MIN);
        for (i, (axis, _, _)) in FACES.iter().enumerate() {
            let d = dir.dot(*axis);
            if d > best.1 {
                best = (i, d);
            }
        }
        let (axis, right, up) = FACES[best.0];
        let g = dir / dir.dot(axis);
        (best.0, g.dot(right), g.dot(up))
    }

    /// (min, max) of a pyramid pair over a gnomonic rectangle, padded by two texels.
    fn range(&self, pyr_min: &[Vec<f32>], pyr_max: &[Vec<f32>], a0: f64, b0: f64, a1: f64, b1: f64) -> (f32, f32) {
        let m = (self.res - 1) as f64;
        let (p0, p1) = (self.param(a0.min(a1)), self.param(a0.max(a1)));
        let (q0, q1) = (self.param(b0.min(b1)), self.param(b0.max(b1)));
        let tx0 = (((p0 * 0.5 + 0.5) * m).floor() - 2.0).max(0.0) as usize;
        let ty0 = (((q0 * 0.5 + 0.5) * m).floor() - 2.0).max(0.0) as usize;
        let tx1 = (((p1 * 0.5 + 0.5) * m).ceil() + 2.0).min(m) as usize;
        let ty1 = (((q1 * 0.5 + 0.5) * m).ceil() + 2.0).min(m) as usize;
        let span = (tx1 - tx0).max(ty1 - ty0).max(1);
        let mut level = 0usize;
        while (span >> level) > 4 && level + 1 < pyr_min.len() {
            level += 1;
        }
        let lr = self.res.div_ceil(1 << level).max(1);
        let (lx0, ly0, lx1, ly1) = (tx0 >> level, ty0 >> level, tx1 >> level, ty1 >> level);
        let (mut mn, mut mx) = (f32::MAX, f32::MIN);
        for y in ly0..=ly1.min(lr - 1) {
            for x in lx0..=lx1.min(lr - 1) {
                mn = mn.min(pyr_min[level][y * lr + x]);
                mx = mx.max(pyr_max[level][y * lr + x]);
            }
        }
        (mn, mx)
    }
}

/// Within this many texels of a face edge the elevation gradient comes
/// from central differences across the edge (see `sample`).
const EDGE_FD_TEXELS: f64 = 3.0;

#[inline]
fn smooth01(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl RasterBake {
    /// Catmull-Rom elevation (km) at a direction, on whichever face it
    /// points into.
    fn elev_at(&self, dir: DVec3) -> f64 {
        let (f, a, b) = Self::face_ab(dir);
        let face = &self.faces[f];
        let (x, y) = self.texel_coords(a, b);
        let (x0, y0) = (x.floor(), y.floor());
        let (ix, iy) = (x0 as i64, y0 as i64);
        let res = self.res as i64;
        let wx = catmull_weights(x - x0);
        let wy = catmull_weights(y - y0);
        let mut e = 0.0;
        for dy in -1..=2 {
            let yy = (iy + dy).clamp(0, res - 1) as usize * self.res;
            let mut r = 0.0;
            for dx in -1..=2 {
                r += wx[(dx + 1) as usize] * face.elev_km[yy + (ix + dx).clamp(0, res - 1) as usize] as f64;
            }
            e += wy[(dy + 1) as usize] * r;
        }
        e
    }

    /// Elevation gradient (km per radian of u, v) of face `f`'s equal-angle
    /// chart at gnomonic (a, b), by central differences one texel wide; the
    /// stencil reaches across the face edge onto the neighbouring raster.
    fn elev_gradient_fd(&self, f: u8, a: f64, b: f64) -> (f64, f64) {
        let (axis, right, up) = FACES[f as usize];
        let (u, v) = (a.atan(), b.atan());
        let h = 0.5 * FRAC_PI_4 * 2.0 / (self.res - 1) as f64; // half a texel in angle
        let at = |u: f64, v: f64| self.elev_at((axis + u.tan() * right + v.tan() * up).normalize());
        ((at(u + h, v) - at(u - h, v)) / (2.0 * h), (at(u, v + h) - at(u, v - h)) / (2.0 * h))
    }
}

impl Bake for RasterBake {
    fn sample(&self, dir: DVec3) -> BakeSample {
        let (f, a, b) = Self::face_ab(dir);
        let face = &self.faces[f];
        let (x, y) = self.texel_coords(a, b);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (ix, iy) = (x0 as i64, y0 as i64);
        let res = self.res as i64;
        let wx = catmull_weights(fx);
        let wy = catmull_weights(fy);
        let dwx = catmull_dweights(fx);
        let dwy = catmull_dweights(fy);
        // Catmull-Rom bicubic for the smooth fields; taps edge-clamped per face.
        // The elevation's gradient comes from the same taps.
        let (bx, by) = (bspline_weights(fx), bspline_weights(fy));
        let (bdx, bdy) = (bspline_dweights(fx), bspline_dweights(fy));
        let (bddx, bddy) = (bspline_ddweights(fx), bspline_ddweights(fy));
        let (mut elev, mut temp, mut precip) = (0.0, 0.0, 0.0);
        let (mut ex, mut ey) = (0.0, 0.0);
        // B-spline derivatives: x, y, xx, yy, xy.
        let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        // The same for the flow field (when the bake has one).
        let has_flow = !face.flow_log10.is_empty();
        let (mut f0, mut fx_, mut fy_, mut fxx, mut fyy, mut fxy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for dy in -1..=2 {
            let yy = (iy + dy).clamp(0, res - 1) as usize * self.res;
            let (mut re, mut rex, mut rt, mut rp) = (0.0, 0.0, 0.0, 0.0);
            let (mut b0, mut b1, mut b2) = (0.0, 0.0, 0.0);
            let (mut c0, mut c1, mut c2) = (0.0, 0.0, 0.0);
            for dx in -1..=2 {
                let xx = (ix + dx).clamp(0, res - 1) as usize;
                let t = (dx + 1) as usize;
                let w = wx[t];
                let e = face.elev_km[yy + xx] as f64;
                re += w * e;
                rex += dwx[t] * e;
                rt += w * face.temp_c[yy + xx] as f64;
                rp += w * face.precip[yy + xx] as f64;
                b0 += bx[t] * e;
                b1 += bdx[t] * e;
                b2 += bddx[t] * e;
                if has_flow {
                    let q = face.flow_log10[yy + xx] as f64;
                    c0 += bx[t] * q;
                    c1 += bdx[t] * q;
                    c2 += bddx[t] * q;
                }
            }
            let t = (dy + 1) as usize;
            let w = wy[t];
            elev += w * re;
            ex += w * rex;
            ey += dwy[t] * re;
            temp += w * rt;
            precip += w * rp;
            sx += by[t] * b1;
            sy += bdy[t] * b0;
            sxx += by[t] * b2;
            syy += bddy[t] * b0;
            sxy += bdy[t] * b1;
            f0 += by[t] * c0;
            fx_ += by[t] * c1;
            fy_ += bdy[t] * c0;
            fxx += by[t] * c2;
            fyy += bddy[t] * c0;
            fxy += bdy[t] * c1;
        }
        // Texel units -> per radian of the equal-angle chart: x = (p/2 + 1/2)(res - 1)
        // with p = u / (pi/4) (new bake) or p = a = tan u (old bake).
        let half = 0.5 * (self.res - 1) as f64;
        let (du, dv) = if self.equiangular { (half / FRAC_PI_4, half / FRAC_PI_4) } else { (half * (1.0 + a * a), half * (1.0 + b * b)) };
        let m = (self.res - 1) as f64;
        let edge_texels = x.min(m - x).min(y).min(m - y);
        // Near a face edge the taps are clamped to this face, which bends
        // the derivative (at the edge texel it is half the one-sided slope),
        // and the face across the edge bends it differently: the gradient
        // would jump where the sample switches faces. Within EDGE_FD_TEXELS
        // take it from central differences of the elevation across the
        // edge instead (the elevation itself is continuous), blended in.
        let (mut ex_c, mut ey_c) = (ex * du, ey * dv);
        if edge_texels < EDGE_FD_TEXELS {
            let (fdu, fdv) = self.elev_gradient_fd(f as u8, a, b);
            let w = smooth01((edge_texels - 1.0) / (EDGE_FD_TEXELS - 1.0));
            ex_c = w * ex_c + (1.0 - w) * fdu;
            ey_c = w * ey_c + (1.0 - w) * fdv;
        }
        // Bilinear for the mask-like fields.
        let (x1, y1) = (((ix + 1).min(res - 1)) as usize, ((iy + 1).min(res - 1)) as usize);
        let (ix, iy) = (ix as usize, iy as usize);
        let bil = |d: &[f32]| {
            if d.is_empty() {
                return 0.0;
            }
            let s = |xx: usize, yy: usize| d[yy * self.res + xx] as f64;
            let t = s(ix, iy) * (1.0 - fx) + s(x1, iy) * fx;
            let u = s(ix, y1) * (1.0 - fx) + s(x1, y1) * fx;
            t * (1.0 - fy) + u * fy
        };
        BakeSample {
            elev_km: elev,
            temp_c: temp,
            precip_mm: precip,
            rough_km: bil(&face.rough_km),
            ocean: bil(&face.ocean),
            soil_km: bil(&face.soil_km),
            continental: bil(&face.continental),
            volcanic: bil(&face.volcanic),
            erodibility: bil(&face.erodibility),
            uplift: bil(&face.uplift),
            flow_log10: bil(&face.flow_log10),
            lake: bil(&face.lake),
            face: f as u8,
            elev_du_km: ex_c,
            elev_dv_km: ey_c,
            smooth_du_km: sx * du,
            smooth_dv_km: sy * dv,
            smooth_uu_km: sxx * du * du,
            smooth_vv_km: syy * dv * dv,
            smooth_uv_km: sxy * du * dv,
            flow_smooth: f0,
            flow_du: fx_ * du,
            flow_dv: fy_ * dv,
            flow_uu: fxx * du * du,
            flow_vv: fyy * dv * dv,
            flow_uv: fxy * du * dv,
            edge_texels,
        }
    }

    fn elev_range_km(&self, face: u8, a0: f64, b0: f64, a1: f64, b1: f64) -> (f32, f32) {
        let f = &self.faces[face as usize];
        let (mn, mx) = self.range(&f.pyr_min, &f.pyr_max, a0, b0, a1, b1);
        // Bicubic Catmull-Rom overshoots the taps' range by up to 0.28125 of
        // it: at t = (1/2, 1/2) the positive weights sum to 1.125^2 + 0.125^2.
        let pad = (mx - mn) * 0.28125;
        (mn - pad, mx + pad)
    }

    fn rough_max_range_km(&self, face: u8, a0: f64, b0: f64, a1: f64, b1: f64) -> f32 {
        let f = &self.faces[face as usize];
        self.range(&f.pyr_rough, &f.pyr_rough, a0, b0, a1, b1).1
    }

    fn land_min_km(&self) -> f32 {
        self.land_min_km
    }

    fn rough_max_km(&self) -> f32 {
        self.rough_max_km
    }

    fn for_each_texel(&self, f: &mut dyn FnMut(DVec3, f32, bool, f32)) {
        let m = (self.res - 1) as f64;
        let unparam = |p: f64| if self.equiangular { (p * FRAC_PI_4).tan() } else { p };
        for (fi, face) in self.faces.iter().enumerate() {
            let (axis, right, up) = FACES[fi];
            for y in 0..self.res {
                for x in 0..self.res {
                    let a = unparam(x as f64 / m * 2.0 - 1.0);
                    let b = unparam(y as f64 / m * 2.0 - 1.0);
                    let dir = (axis + a * right + b * up).normalize();
                    let i = y * self.res + x;
                    let e = face.elev_km[i];
                    let gx = face.elev_km[y * self.res + (x + 1).min(self.res - 1)] - face.elev_km[y * self.res + x.saturating_sub(1)];
                    let gy = face.elev_km[(y + 1).min(self.res - 1) * self.res + x] - face.elev_km[y.saturating_sub(1) * self.res + x];
                    f(dir, e, face.ocean[i] > 0.5, (gx * gx + gy * gy).sqrt() * 0.5);
                }
            }
        }
    }

    fn describe(&self) -> String {
        format!(
            "{} bake {} ({}x{} per face, land min {:.2} km, rough max {:.2} km{})",
            if self.equiangular { "equal-angle world" } else { "old gnomonic face" },
            self.dir,
            self.res,
            self.res,
            self.land_min_km,
            self.rough_max_km,
            if self.has_geology { ", with soil and geology fields" } else { "" }
        )
    }
}

fn build_pyramid(elev: &[f32], res: usize) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut pyr_min = vec![elev.to_vec()];
    let mut pyr_max = vec![elev.to_vec()];
    let mut r = res;
    while r > 1 {
        let nr = r.div_ceil(2);
        let (pmin, pmax) = (pyr_min.last().unwrap(), pyr_max.last().unwrap());
        let mut nmin = vec![f32::MAX; nr * nr];
        let mut nmax = vec![f32::MIN; nr * nr];
        for y in 0..r {
            for x in 0..r {
                let d = (y / 2) * nr + x / 2;
                nmin[d] = nmin[d].min(pmin[y * r + x]);
                nmax[d] = nmax[d].max(pmax[y * r + x]);
            }
        }
        pyr_min.push(nmin);
        pyr_max.push(nmax);
        r = nr;
    }
    (pyr_min, pyr_max)
}

#[inline]
fn catmull_weights(t: f64) -> [f64; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

/// Derivatives of `catmull_weights` with respect to t.
#[inline]
fn catmull_dweights(t: f64) -> [f64; 4] {
    let t2 = t * t;
    [0.5 * (-3.0 * t2 + 4.0 * t - 1.0), 0.5 * (9.0 * t2 - 10.0 * t), 0.5 * (-9.0 * t2 + 8.0 * t + 1.0), 0.5 * (3.0 * t2 - 2.0 * t)]
}

/// Uniform cubic B-spline weights (C2; smooths rather than interpolates)
/// and their first and second derivatives.
#[inline]
fn bspline_weights(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [s * s * s / 6.0, (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0, (-3.0 * t * t * t + 3.0 * t * t + 3.0 * t + 1.0) / 6.0, t * t * t / 6.0]
}

#[inline]
fn bspline_dweights(t: f64) -> [f64; 4] {
    let s = 1.0 - t;
    [-0.5 * s * s, 1.5 * t * t - 2.0 * t, -1.5 * t * t + t + 0.5, 0.5 * t * t]
}

#[inline]
fn bspline_ddweights(t: f64) -> [f64; 4] {
    [1.0 - t, 3.0 * t - 2.0, -3.0 * t + 1.0, t]
}

/// Equal-angle chart coordinates (u, v) in radians of a direction on `face`
/// (a = tan u, b = tan v along the face's right and up axes). Defined for
/// any direction with a positive component along the face axis, so a chart
/// extends smoothly past its face's edges.
#[inline]
pub fn chart_uv(face: u8, dir: DVec3) -> (f64, f64) {
    let (axis, right, up) = FACES[face as usize];
    let z = dir.dot(axis);
    (dir.dot(right).atan2(z), dir.dot(up).atan2(z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_gradient_matches_finite_differences() {
        let new = open("C:/code/triangulum/game/assets/world/seed42").expect("new bake");
        for (k, &(f, u, v)) in [(2u8, 0.31, -0.12), (4, -0.5, 0.2), (0, 0.05, 0.7)].iter().enumerate() {
            let (axis, right, up) = FACES[f as usize];
            let dir_of = |u: f64, v: f64| (axis + u.tan() * right + v.tan() * up).normalize();
            let s = new.sample(dir_of(u, v));
            assert_eq!(s.face, f);
            let h = 2e-5;
            let gu = (new.sample(dir_of(u + h, v)).elev_km - new.sample(dir_of(u - h, v)).elev_km) / (2.0 * h);
            let gv = (new.sample(dir_of(u, v + h)).elev_km - new.sample(dir_of(u, v - h)).elev_km) / (2.0 * h);
            let scale = gu.abs().max(gv.abs()).max(1e-3);
            assert!((s.elev_du_km - gu).abs() < 1e-3 * scale + 1e-6, "case {k}: du {} vs {gu}", s.elev_du_km);
            assert!((s.elev_dv_km - gv).abs() < 1e-3 * scale + 1e-6, "case {k}: dv {} vs {gv}", s.elev_dv_km);
            // B-spline curvature: finite differences of the B-spline gradient.
            let suu = (new.sample(dir_of(u + h, v)).smooth_du_km - new.sample(dir_of(u - h, v)).smooth_du_km) / (2.0 * h);
            let svv = (new.sample(dir_of(u, v + h)).smooth_dv_km - new.sample(dir_of(u, v - h)).smooth_dv_km) / (2.0 * h);
            let suv = (new.sample(dir_of(u, v + h)).smooth_du_km - new.sample(dir_of(u, v - h)).smooth_du_km) / (2.0 * h);
            let cs = suu.abs().max(svv.abs()).max(1e-2);
            assert!((s.smooth_uu_km - suu).abs() < 2e-3 * cs, "case {k}: uu {} vs {suu}", s.smooth_uu_km);
            assert!((s.smooth_vv_km - svv).abs() < 2e-3 * cs, "case {k}: vv {} vs {svv}", s.smooth_vv_km);
            assert!((s.smooth_uv_km - suv).abs() < 2e-3 * cs, "case {k}: uv {} vs {suv}", s.smooth_uv_km);
            let (cu, cv) = chart_uv(f, dir_of(u, v));
            assert!((cu - u).abs() < 1e-12 && (cv - v).abs() < 1e-12);
        }
    }

    #[test]
    fn new_bake_samples_smoothly_and_agrees_with_the_old_one_roughly() {
        let new = open("C:/code/triangulum/game/assets/world/seed42").expect("new bake");
        let old = open("C:/code/triangulum/viewer/assets").expect("old bake");
        assert!(new.equiangular && !old.equiangular);
        let dir = DVec3::new(0.3, 0.9, 0.2).normalize();
        let (a, b) = (new.sample(dir), old.sample(dir));
        assert!((a.elev_km - b.elev_km).abs() < 1.5, "new {} vs old {}", a.elev_km, b.elev_km);
        assert!((a.temp_c - b.temp_c).abs() < 8.0);
        // Smoothness: tiny steps change elevation by tiny amounts.
        let d2 = (dir + DVec3::new(1e-6, 0.0, 0.0)).normalize();
        assert!((new.sample(d2).elev_km - a.elev_km).abs() < 1e-3);
        // Range bounds contain the samples in the middle of a face.
        let (mn, mx) = new.elev_range_km(2, -0.1, -0.1, 0.1, 0.1);
        let c = new.sample(FACES[2].0);
        assert!(mn as f64 <= c.elev_km && c.elev_km <= mx as f64);
    }
}

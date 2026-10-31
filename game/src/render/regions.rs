//! The bake's region layers for the mesh view (bug round #6): the Koppen
//! class, the tectonic plate and the crust of every texel, read straight from
//! the face rasters. `bake_world.py` writes these as byte layers the game
//! otherwise has no use for, so `worldgen::bake` skips them; this reads just
//! those bytes (1 MB a layer), off the main thread, when a view first asks.
//! The shader looks them up by the same rule the bake samples by, nearest.

use std::io::{Read, Seek, SeekFrom};

/// The layers as one texture: RGBA8 per texel (Koppen, plate, crust, 0), six
/// faces of `res` squared, rows from the bottom, as the bake stores them.
pub struct RegionData {
    pub res: u32,
    pub rgba: Vec<u8>,
    /// Plate ids run below this.
    pub plates: u32,
}

/// Where a game's copy of the layers is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RegionsState {
    #[default]
    NotAsked,
    Loading,
    Ready {
        plates: u32,
    },
    Missing(String),
}

const LAYERS: [&str; 3] = ["koppen", "plate", "crust_type"];

/// Read the layers of the bake in `dir`.
pub fn load(dir: &str) -> Result<RegionData, String> {
    let meta_path = format!("{dir}/meta.json");
    let text = std::fs::read(&meta_path).map_err(|e| format!("read {meta_path}: {e}"))?;
    let meta: serde_json::Value = serde_json::from_slice(&text).map_err(|e| format!("{meta_path}: {e}"))?;
    let layout = &meta["layout"];
    let res = layout["resolution"].as_u64().ok_or("this bake has no region layers (no resolution in its meta.json)")? as usize;
    let pattern = layout["file_pattern"].as_str().unwrap_or("face_{face}.bin");
    let fields = meta["fields"].as_array().ok_or("this bake has no region layers (no field list in its meta.json)")?;
    let offset = |name: &str| -> Result<u64, String> {
        let f = fields.iter().find(|f| f["name"].as_str() == Some(name)).ok_or(format!("this bake has no {name} layer"))?;
        if f["type"].as_str() != Some("u8") {
            return Err(format!("the bake's {name} layer is not bytes"));
        }
        f["offset"].as_u64().ok_or(format!("the bake's {name} layer has no offset"))
    };
    let offsets = [offset(LAYERS[0])?, offset(LAYERS[1])?, offset(LAYERS[2])?];
    let n = res * res;
    let mut rgba = vec![0u8; 6 * n * 4];
    let mut plates = 0u32;
    let mut line = vec![0u8; n];
    for (face, out) in rgba.chunks_exact_mut(n * 4).enumerate() {
        let path = format!("{dir}/{}", pattern.replace("{face}", &face.to_string()));
        let mut file = std::fs::File::open(&path).map_err(|e| format!("open {path}: {e}"))?;
        for (channel, off) in offsets.iter().enumerate() {
            file.seek(SeekFrom::Start(*off)).map_err(|e| format!("{path}: {e}"))?;
            file.read_exact(&mut line).map_err(|e| format!("{path}: {e}"))?;
            for (px, v) in out.chunks_exact_mut(4).zip(&line) {
                px[channel] = *v;
            }
            if channel == 1 {
                plates = plates.max(line.iter().map(|&v| v as u32 + 1).max().unwrap_or(0));
            }
        }
    }
    Ok(RegionData { res: res as u32, rgba, plates })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::bake::{Bake, open};
    use cubesphere::FACES;
    use glam::DVec3;

    const BAKE: &str = "C:/code/triangulum/game/assets/world/seed42";

    /// The texel the shader's `mesh_region` picks for a direction.
    fn texel(d: &RegionData, dir: DVec3) -> [u8; 4] {
        let m = dir.abs();
        let f = if m.x >= m.y && m.x >= m.z {
            if dir.x > 0.0 { 0 } else { 1 }
        } else if m.y >= m.z {
            if dir.y > 0.0 { 2 } else { 3 }
        } else if dir.z > 0.0 {
            4
        } else {
            5
        };
        let (axis, right, up) = FACES[f];
        let z = dir.dot(axis);
        let (a, b) = (dir.dot(right) / z, dir.dot(up) / z);
        let top = d.res as f64 - 1.0;
        let at = |v: f64| (((v.atan() / std::f64::consts::FRAC_PI_4) * 0.5 + 0.5) * top).round() as usize;
        let n = d.res as usize;
        let i = f * n * n + at(b) * n + at(a);
        d.rgba[i * 4..i * 4 + 4].try_into().unwrap()
    }

    /// The layers line up with the rest of the bake: where the shader's
    /// lookup lands on open sea the Koppen layer says so (255), and over
    /// land it names a class; the crust layer has both kinds; several plates.
    #[test]
    fn the_layers_agree_with_the_bake() {
        let d = load(BAKE).unwrap();
        assert_eq!(d.rgba.len(), 6 * (d.res * d.res) as usize * 4);
        assert!(d.plates >= 2 && d.plates < 64, "{} plates", d.plates);
        let bake = open(BAKE).unwrap();
        let (mut sea, mut land, mut wrong) = (0, 0, 0);
        let mut crust = [0u32; 2];
        for n in 0..4000 {
            // A spiral of directions over the sphere.
            let z = 1.0 - 2.0 * (n as f64 + 0.5) / 4000.0;
            let phi = n as f64 * 2.399963;
            let r = (1.0 - z * z).sqrt();
            let dir = DVec3::new(r * phi.cos(), r * phi.sin(), z);
            let s = bake.sample(dir);
            let [k, p, c, _] = texel(&d, dir);
            crust[(c as usize).min(1)] += 1;
            assert!((p as u32) < d.plates);
            assert!(k == 255 || (k as usize) < crate::render::meshview::KOPPEN.len(), "class {k}");
            if s.ocean > 0.98 {
                sea += 1;
                wrong += (k != 255) as u32;
            } else if s.ocean < 0.02 {
                land += 1;
                wrong += (k == 255) as u32;
            }
        }
        assert!(sea > 500 && land > 500, "{sea} sea, {land} land");
        assert!((wrong as f64) < 0.02 * (sea + land) as f64, "{wrong} of {} disagree", sea + land);
        assert!(crust[0] > 100 && crust[1] > 100, "{crust:?}");
    }

    /// A bake without the layers says which one is missing.
    #[test]
    fn a_bake_without_the_layers_says_so() {
        let dir = std::env::temp_dir().join(format!("triangulum-regions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.json"), r#"{"layout":{"resolution":4},"fields":[{"name":"plate","type":"u8","offset":0}]}"#).unwrap();
        let e = load(dir.to_str().unwrap()).err().unwrap();
        assert!(e.contains("koppen"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

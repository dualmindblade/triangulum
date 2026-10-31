//! What a picture is made of, apart from the world and the camera (B6; review
//! 4 #8). A replay checks the simulation exactly, but the same simulation can
//! look quite different: another look.json, other block colours, a repainted
//! texture. So a recording also holds this, and a playback compares it with
//! what it is drawing with and says so separately: "the simulation reproduced"
//! and "the look was the same" are two claims, and only the second one says
//! anything about a rendering bug.
//!
//! `Visual` holds the look's values, and the identity of every block and
//! texture as they are live (a texture painted in the panel and not saved yet
//! counts), not as the files say. It lives in `Session::visual`, where the app
//! and the harness put a new one when something changes. A change during a
//! recording is written as `note visual {json}`, a line older builds read as a
//! plain note, naming what moved.
//!
//! This is identity, not content: the textures themselves are not in the
//! recording, so a playback can say a texture differs and cannot show it. The
//! look's values are there whole: `--replay` writes them out for `--look-file`.

use crate::look::Look;
use crate::render::textures::{Picture, TextureSet};
use crate::sha256::{hex, sha256};
use crate::world::Registry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The commit of this build.
pub const BUILD: &str = env!("TRIANGULUM_BUILD");

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Visual {
    /// Every look value, as look.json spells them (no comment).
    pub look: Value,
    /// Block name to a digest of its colours and the texture files it names.
    pub blocks: BTreeMap<String, String>,
    /// Texture file to a digest of its pixels.
    pub textures: BTreeMap<String, String>,
    pub texture_size: u32,
    /// The mesh view in force ("normal" is the game's own picture).
    pub mesh_view: String,
    /// How the mesher ran: `ao`, `split`, `skirt`.
    pub mesh: BTreeMap<String, String>,
}

fn short(bytes: &[u8]) -> String {
    hex(&sha256(bytes))[..16].to_string()
}

/// A picture's identity: its size and pixels.
pub fn picture_id(p: &Picture) -> String {
    let mut b = p.size.to_le_bytes().to_vec();
    b.extend_from_slice(&p.rgba);
    short(&b)
}

/// The identity of every picture in a set, as it is now.
pub fn texture_ids(set: &TextureSet) -> BTreeMap<String, String> {
    (0..set.len()).map(|i| (set.file_at(i).to_string(), picture_id(set.picture_at(i)))).collect()
}

static STARTUP_TEXTURES: OnceLock<BTreeMap<String, String>> = OnceLock::new();

/// The textures main loaded, for whoever has no set of its own (the harness and
/// `--replay`; the app reads the look panel's live one).
pub fn install_textures(ids: BTreeMap<String, String>) {
    let _ = STARTUP_TEXTURES.set(ids);
}

pub fn startup_textures() -> BTreeMap<String, String> {
    STARTUP_TEXTURES.get().cloned().unwrap_or_default()
}

impl Visual {
    /// What the picture is made of now. `textures`: `texture_ids` of the live
    /// set. Reads and changes nothing else.
    pub fn of(look: &Look, registry: &Registry, textures: BTreeMap<String, String>, mesh_view: &str, ao: bool, split: bool, skirt: u32) -> Self {
        // Through text, so f32 colours read 0.55 and not 0.550000011920929.
        let mut look: Value = serde_json::to_string(&look.tidy()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
        if let Some(m) = look.as_object_mut() {
            m.remove("_comment");
        }
        let colours = registry.colours();
        let files = registry.textures();
        let blocks = registry
            .defs()
            .iter()
            .zip(&colours)
            .zip(&files)
            .filter(|((d, _), _)| !d.name.starts_with("unused_") && d.name != "air")
            .map(|((d, c), t)| (d.name.clone(), short(format!("{:?} {:?}", c.faces(), t.faces()).as_bytes())))
            .collect();
        let mesh = [("ao", ao.to_string()), ("split", split.to_string()), ("skirt", skirt.to_string())].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        Self { look, blocks, textures, texture_size: registry.texture_size(), mesh_view: mesh_view.to_string(), mesh }
    }
}

fn look_diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let p = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                look_diff(&p, x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null), out);
            }
        }
        _ if a != b => out.push(format!("look {path}: {a} recorded, {b} here")),
        _ => {}
    }
}

fn map_diff(what: &str, a: &BTreeMap<String, String>, b: &BTreeMap<String, String>, out: &mut Vec<String>) {
    let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    for k in keys {
        match (a.get(k), b.get(k)) {
            (Some(x), Some(y)) if x != y => out.push(format!("{what} {k}: not the same")),
            (Some(_), None) => out.push(format!("{what} {k}: in the recording only")),
            (None, Some(_)) => out.push(format!("{what} {k}: here only")),
            _ => {}
        }
    }
}

/// Everything that differs between two pictures' ingredients, in words.
pub fn differences(rec: &Visual, here: &Visual) -> Vec<String> {
    let mut out = Vec::new();
    look_diff("", &rec.look, &here.look, &mut out);
    map_diff("block", &rec.blocks, &here.blocks, &mut out);
    map_diff("texture", &rec.textures, &here.textures, &mut out);
    if rec.texture_size != here.texture_size {
        out.push(format!("texture size: {} recorded, {} here", rec.texture_size, here.texture_size));
    }
    if rec.mesh_view != here.mesh_view {
        out.push(format!("mesh view: {} recorded, {} here", rec.mesh_view, here.mesh_view));
    }
    map_diff("mesher setting", &rec.mesh, &here.mesh, &mut out);
    out
}

/// Could this playback have looked like the recording?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reproduced {
    /// Every look value, block and texture recorded matches.
    Same,
    /// Something differs; the frames may too.
    Different,
    /// The recording holds no look: it is older than this check.
    Unknown,
}

impl Reproduced {
    pub fn name(self) -> &'static str {
        match self {
            Reproduced::Same => "same",
            Reproduced::Different => "different",
            Reproduced::Unknown => "unknown",
        }
    }
}

/// The visual half of a replay's verdict. The simulation's half is
/// `replay::Report`.
#[derive(Clone, Debug)]
pub struct Verdict {
    pub reproduced: Reproduced,
    pub differences: Vec<String>,
    /// What changed while it was recording, by step: a playback starts from
    /// the first look and does not follow these.
    pub changed_during: Vec<(u64, Vec<String>)>,
    pub recorded_build: String,
    pub this_build: String,
}

/// Compare a recording's look with the one playing it.
pub fn judge(recorded: Option<&Visual>, recorded_build: &str, changed_during: Vec<(u64, Vec<String>)>, here: &Visual) -> Verdict {
    let (reproduced, differences) = match recorded {
        None => (Reproduced::Unknown, Vec::new()),
        Some(r) => {
            let d = differences(r, here);
            (if d.is_empty() && changed_during.is_empty() { Reproduced::Same } else { Reproduced::Different }, d)
        }
    };
    Verdict { reproduced, differences, changed_during, recorded_build: recorded_build.to_string(), this_build: BUILD.to_string() }
}

impl Verdict {
    /// Same build, or nothing to tell?
    pub fn other_build(&self) -> bool {
        !self.recorded_build.is_empty() && self.recorded_build != self.this_build
    }

    pub fn text(&self) -> String {
        let mut t = match self.reproduced {
            Reproduced::Same => "look: the same as the recording (every look value, block and texture)".to_string(),
            Reproduced::Unknown => "look: UNKNOWN (this recording is older than the look check and holds no look to compare)".to_string(),
            Reproduced::Different => "look: NOT the same as the recording, so frames can differ even though the simulation matched:".to_string(),
        };
        const SHOWN: usize = 12;
        for d in self.differences.iter().take(SHOWN) {
            t.push_str(&format!("\n  {d}"));
        }
        if self.differences.len() > SHOWN {
            t.push_str(&format!("\n  and {} more", self.differences.len() - SHOWN));
        }
        for (step, what) in self.changed_during.iter().take(SHOWN) {
            t.push_str(&format!("\n  changed during the recording at step {step} ({:.1} s): {}", *step as f64 / 60.0, what.join("; ")));
        }
        if self.changed_during.len() > SHOWN {
            t.push_str(&format!("\n  and {} more changes", self.changed_during.len() - SHOWN));
        }
        if self.other_build() {
            t.push_str(&format!("\n  recorded by build {}, this is build {}: the renderer itself may differ", self.recorded_build, self.this_build));
        }
        t
    }

    pub fn json(&self) -> Value {
        serde_json::json!({
            "reproduced": self.reproduced.name(),
            "differences": self.differences,
            "changed_during_recording": self.changed_during.iter().map(|(s, w)| serde_json::json!({ "step": s, "what": w })).collect::<Vec<_>>(),
            "recorded_build": self.recorded_build,
            "this_build": self.this_build,
            "other_build": self.other_build(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::Settings;

    fn registry() -> Registry {
        Registry::load(&format!("{}/assets/blocks.json", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn base() -> (Look, Registry, BTreeMap<String, String>) {
        let look = Look::defaults(&Settings::default(), 90_000.0);
        let reg = registry();
        let tex = BTreeMap::from([("stone.png".to_string(), picture_id(&Picture::flat(16, [0.5, 0.5, 0.5])))]);
        (look, reg, tex)
    }

    fn of(look: &Look, reg: &Registry, tex: &BTreeMap<String, String>) -> Visual {
        Visual::of(look, reg, tex.clone(), "normal", true, true, 2)
    }

    /// Review 4 #8: record with altered fog and water, a changed block colour
    /// and a repainted texture, play with the shipped ones: the verdict names
    /// each, where nothing but the simulation was checked before.
    #[test]
    fn a_different_look_is_named_thing_by_thing() {
        let (look, reg, tex) = base();
        let here = of(&look, &reg, &tex);
        assert_eq!(judge(Some(&here), BUILD, vec![], &here).reproduced, Reproduced::Same);
        let mut changed = look.clone();
        changed.sky.fog_strength = 0.2;
        changed.water.opacity = 0.9;
        let mut colours = reg.colours();
        let stone = reg.id("stone").unwrap() as usize;
        colours[stone].color = [0.9, 0.1, 0.1];
        let rec_reg = registry();
        assert!(rec_reg.set_colours(&colours));
        let mut rec_tex = tex.clone();
        rec_tex.insert("stone.png".into(), picture_id(&Picture::flat(16, [0.1, 0.1, 0.9])));
        rec_tex.insert("painted.png".into(), picture_id(&Picture::flat(16, [0.2, 0.9, 0.1])));
        let rec = of(&changed, &rec_reg, &rec_tex);
        let v = judge(Some(&rec), BUILD, vec![], &here);
        assert_eq!(v.reproduced, Reproduced::Different);
        let t = v.text();
        for want in ["look sky.fog_strength: 0.2 recorded, 0.7 here", "look water.opacity: 0.9 recorded", "block stone: not the same", "texture stone.png: not the same", "texture painted.png: in the recording only"] {
            assert!(t.contains(want), "{want:?} missing from:\n{t}");
        }
        assert!(t.contains("NOT the same") && !t.contains("the renderer itself"), "{t}");
    }

    /// Every value of the look counts, even the ones nothing else checks.
    #[test]
    fn every_look_value_counts() {
        let (look, reg, tex) = base();
        let a = of(&look, &reg, &tex);
        let mut m = look.clone();
        m.light.moon_colour[2] += 0.25;
        assert_ne!(a, of(&m, &reg, &tex));
        let mut m = look.clone();
        m.detail.lod_factor += 1.0;
        assert_ne!(a, of(&m, &reg, &tex));
        assert_ne!(a, Visual { mesh_view: "lod".into(), ..a.clone() });
        // The comment of a look file says nothing about the picture.
        let mut m = look.clone();
        m.comment = "another comment".into();
        assert_eq!(a, of(&m, &reg, &tex));
    }

    /// A recording older than this check says "unknown", not "same"; a change
    /// during a recording never passes; another build is said out loud.
    #[test]
    fn the_verdict_says_what_it_does_not_know() {
        let (look, reg, tex) = base();
        let here = of(&look, &reg, &tex);
        let v = judge(None, "", vec![], &here);
        assert_eq!(v.reproduced, Reproduced::Unknown);
        assert!(v.text().contains("UNKNOWN") && !v.other_build());
        let v = judge(Some(&here), BUILD, vec![(600, vec!["look sky.fog_strength".to_string()])], &here);
        assert_eq!(v.reproduced, Reproduced::Different);
        assert!(v.text().contains("at step 600 (10.0 s)"), "{}", v.text());
        let v = judge(Some(&here), "abc1234", vec![], &here);
        assert_eq!(v.reproduced, Reproduced::Same);
        assert!(v.other_build() && v.text().contains("build abc1234"), "{}", v.text());
    }

    #[test]
    fn it_survives_the_header_as_json() {
        let (look, reg, tex) = base();
        let a = of(&look, &reg, &tex);
        let back: Visual = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, back);
        assert!(differences(&a, &back).is_empty());
    }
}

//! Rebindable key mappings: one table from game action to physical key.
//!
//! Every keyboard-driven game action in main.rs routes through
//! [`KeyBindings`] instead of matching `KeyCode` directly, so the Controls
//! window (F1, `ui::KeysWindow`) can remap any of them at runtime. The table
//! persists as `keybindings.json` next to the other viewer persistence
//! (edits/torches/tuning live in the assets dir); a missing or invalid file
//! falls back to the defaults wholesale, same policy as the tuning loaders.
//!
//! Deliberately out of scope: mouse buttons/wheel (capture, dig/place,
//! altitude), Esc (the universal back-out — it cancels a pending rebind, so
//! it can never be captured as a binding), and the Shift chord on the raw
//! screenshot (Shift+P), which stays a fixed modifier.
//!
//! Conflict policy: rebinding an action to a key another action holds SWAPS
//! the two keys. Every action always has exactly one key and no key serves
//! two actions — the invariant the movement/`action_for` lookups rely on.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

/// Every rebindable game action. Serialized by variant name in
/// `keybindings.json`, so renaming a variant orphans saved files (they fall
/// back to defaults) — prefer adding over renaming.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Action {
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    Sprint,
    Jump,
    Descend,
    RollLeft,
    RollRight,
    WalkMode,
    FlyMode,
    CycleCameraFocus,
    Torch,
    TeleportMap,
    Photo,
    SyncDelta,
    StreamCycle,
    SessionRecord,
    TimeSlower,
    TimeFaster,
    ControlsWindow,
}

impl Action {
    /// Display order of the Controls window; also the tie-break order for
    /// `action_for` (unreachable in practice — keys are always distinct).
    pub const ALL: [Action; 21] = [
        Action::MoveForward,
        Action::MoveBack,
        Action::MoveLeft,
        Action::MoveRight,
        Action::Sprint,
        Action::Jump,
        Action::Descend,
        Action::RollLeft,
        Action::RollRight,
        Action::WalkMode,
        Action::FlyMode,
        Action::CycleCameraFocus,
        Action::Torch,
        Action::TeleportMap,
        Action::Photo,
        Action::SyncDelta,
        Action::StreamCycle,
        Action::SessionRecord,
        Action::TimeSlower,
        Action::TimeFaster,
        Action::ControlsWindow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::MoveForward => "Move forward",
            Action::MoveBack => "Move back",
            Action::MoveLeft => "Strafe left",
            Action::MoveRight => "Strafe right",
            Action::Sprint => "Sprint",
            Action::Jump => "Jump / up",
            Action::Descend => "Down (freecam)",
            Action::RollLeft => "Roll left (freecam)",
            Action::RollRight => "Roll right (freecam)",
            Action::WalkMode => "Walk mode",
            Action::FlyMode => "Fly mode",
            Action::CycleCameraFocus => "Camera focus",
            Action::Torch => "Torch",
            Action::TeleportMap => "Map / teleport",
            Action::Photo => "Screenshot",
            Action::SyncDelta => "Sync delta",
            Action::StreamCycle => "Detail streaming",
            Action::SessionRecord => "Session recorder",
            Action::TimeSlower => "Time slower",
            Action::TimeFaster => "Time faster",
            Action::ControlsWindow => "Controls window",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Action::MoveForward => "Walk or fly forward (hold)",
            Action::MoveBack => "Walk or fly backward (hold)",
            Action::MoveLeft => "Strafe left (hold)",
            Action::MoveRight => "Strafe right (hold)",
            Action::Sprint => "4x speed while held",
            Action::Jump => "Jump when walking; swim up / fly up while held",
            Action::Descend => "Descend while held in the free camera",
            Action::RollLeft => "Roll the free camera left (hold)",
            Action::RollRight => "Roll the free camera right (hold)",
            Action::WalkMode => "Land and walk on the surface",
            Action::FlyMode => "Lift off and fly",
            Action::CycleCameraFocus => "Cycle the camera between bodies",
            Action::Torch => "Place or remove a torch",
            Action::TeleportMap => "Open the photo map: teleport, photos, time travel",
            Action::Photo => "Settled screenshot; hold Shift for the raw live frame",
            Action::SyncDelta => "Save an A/B sync comparison to interchange/sync",
            Action::StreamCycle => "Cycle detail streaming: strict, balanced, eager",
            Action::SessionRecord => "Start/stop the 10 Hz .play pose recording",
            Action::TimeSlower => "Step the unified clock down the speed ladder",
            Action::TimeFaster => "Step the unified clock up the speed ladder",
            Action::ControlsWindow => "Open this key-mapping window",
        }
    }
}

/// The action table: complete (every action bound) and injective (no key
/// bound twice) by construction — `rebind` swaps on conflict, and `load`
/// rejects files that would break either property.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyBindings {
    map: HashMap<Action, KeyCode>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            map: Action::ALL.iter().map(|&a| (a, default_key(a))).collect(),
        }
    }
}

/// Today's shipped layout — the exact keys main.rs matched on before the
/// bindings table existed (behavior-neutral until the user remaps).
pub fn default_key(action: Action) -> KeyCode {
    match action {
        Action::MoveForward => KeyCode::KeyW,
        Action::MoveBack => KeyCode::KeyS,
        Action::MoveLeft => KeyCode::KeyA,
        Action::MoveRight => KeyCode::KeyD,
        Action::Sprint => KeyCode::ShiftLeft,
        Action::Jump => KeyCode::Space,
        Action::Descend => KeyCode::ControlLeft,
        Action::RollLeft => KeyCode::KeyQ,
        Action::RollRight => KeyCode::KeyE,
        Action::WalkMode => KeyCode::KeyG,
        Action::FlyMode => KeyCode::KeyF,
        Action::CycleCameraFocus => KeyCode::KeyC,
        Action::Torch => KeyCode::KeyR,
        Action::TeleportMap => KeyCode::KeyT,
        Action::Photo => KeyCode::KeyP,
        Action::SyncDelta => KeyCode::KeyV,
        Action::StreamCycle => KeyCode::F9,
        Action::SessionRecord => KeyCode::F10,
        Action::TimeSlower => KeyCode::BracketLeft,
        Action::TimeFaster => KeyCode::BracketRight,
        Action::ControlsWindow => KeyCode::F1,
    }
}

/// What a rebind did, for the Controls window's status line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rebind {
    /// The key was already this action's binding.
    Unchanged,
    /// The key was free; plain reassignment.
    Bound,
    /// The key belonged to this other action, which took the old key.
    Swapped(Action),
}

impl KeyBindings {
    /// The bound key for an action. The table always holds every action;
    /// the default is a belt-and-braces fallback, not a reachable state.
    pub fn key(&self, action: Action) -> KeyCode {
        self.map
            .get(&action)
            .copied()
            .unwrap_or_else(|| default_key(action))
    }

    /// Reverse lookup for the pressed-key dispatch in main.rs.
    pub fn action_for(&self, code: KeyCode) -> Option<Action> {
        Action::ALL.iter().copied().find(|&a| self.key(a) == code)
    }

    /// Bind `action` to `code`. If another action holds `code`, the two swap
    /// keys (see module docs), keeping the table complete and conflict-free.
    /// `Escape` is reserved (it cancels a pending rebind) and refused.
    pub fn rebind(&mut self, action: Action, code: KeyCode) -> Rebind {
        if code == KeyCode::Escape {
            return Rebind::Unchanged;
        }
        if self.key(action) == code {
            return Rebind::Unchanged;
        }
        let other = self.action_for(code);
        let old = self.key(action);
        self.map.insert(action, code);
        if let Some(other) = other {
            self.map.insert(other, old);
            Rebind::Swapped(other)
        } else {
            Rebind::Bound
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Defaults overridden by `{dir}/keybindings.json` when present. The
    /// whole file falls back together on any problem (unknown action or key
    /// name, duplicate keys, Escape bound) — a partially-applied layout is
    /// worse than the defaults.
    pub fn load(dir: &str) -> Self {
        let path = format!("{dir}/keybindings.json");
        match std::fs::read_to_string(&path) {
            Ok(raw) => Self::from_json(&raw, &path),
            Err(_) => Self::default(),
        }
    }

    fn from_json(raw: &str, path: &str) -> Self {
        match serde_json::from_str::<Self>(raw) {
            Ok(mut kb) => {
                // A file from an older build may miss newly added actions:
                // fill those from the defaults, then hold the whole table to
                // the completeness + distinctness invariant.
                for &a in &Action::ALL {
                    kb.map.entry(a).or_insert_with(|| default_key(a));
                }
                let keys: HashSet<KeyCode> = kb.map.values().copied().collect();
                if keys.len() != Action::ALL.len() || keys.contains(&KeyCode::Escape) {
                    eprintln!(
                        "keybindings ignored ({path}: duplicate or reserved keys); using defaults"
                    );
                    return Self::default();
                }
                println!("keybindings: {path}");
                kb
            }
            Err(e) => {
                eprintln!("keybindings ignored ({path}: {e}); using defaults");
                Self::default()
            }
        }
    }

    /// Written on every change (rebind or reset) so a layout survives the
    /// session. Ordered by `Action::ALL` for a stable, diffable file.
    pub fn save(&self, dir: &str) -> std::io::Result<()> {
        let path = format!("{dir}/keybindings.json");
        std::fs::write(&path, self.to_json())
    }

    fn to_json(&self) -> String {
        let mut obj = serde_json::Map::new();
        for &a in &Action::ALL {
            let name = serde_json::to_value(a).expect("action name");
            let key = serde_json::to_value(self.key(a)).expect("key name");
            let serde_json::Value::String(name) = name else {
                unreachable!("unit variant serializes as a string")
            };
            obj.insert(name, key);
        }
        serde_json::to_string_pretty(&serde_json::Value::Object(obj))
            .expect("keybindings serialize")
            + "\n"
    }
}

/// Short display name for a key, for the Controls window and window title.
/// Letters/digits shed their W3C-code prefix ("KeyW" reads "W"); everything
/// without a nicer spelling keeps its variant name (still unambiguous).
pub fn key_name(code: KeyCode) -> String {
    let name = match code {
        KeyCode::Space => "Space",
        KeyCode::BracketLeft => "[",
        KeyCode::BracketRight => "]",
        KeyCode::ShiftLeft => "L-Shift",
        KeyCode::ShiftRight => "R-Shift",
        KeyCode::ControlLeft => "L-Ctrl",
        KeyCode::ControlRight => "R-Ctrl",
        KeyCode::AltLeft => "L-Alt",
        KeyCode::AltRight => "R-Alt",
        KeyCode::ArrowUp => "Up",
        KeyCode::ArrowDown => "Down",
        KeyCode::ArrowLeft => "Left",
        KeyCode::ArrowRight => "Right",
        KeyCode::Comma => ",",
        KeyCode::Period => ".",
        KeyCode::Slash => "/",
        KeyCode::Backslash => "\\",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::Backquote => "`",
        KeyCode::Minus => "-",
        KeyCode::Equal => "=",
        KeyCode::Enter => "Enter",
        KeyCode::Tab => "Tab",
        KeyCode::Backspace => "Backspace",
        _ => {
            let dbg = format!("{code:?}");
            // KeyA..KeyZ and Digit0..Digit9 per the W3C code tables; no
            // other variant carries these prefixes.
            return dbg
                .strip_prefix("Key")
                .or_else(|| dbg.strip_prefix("Digit"))
                .unwrap_or(&dbg)
                .to_string();
        }
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "triangulum-keybindings-{tag}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_complete_and_distinct() {
        let kb = KeyBindings::default();
        let mut seen = HashSet::new();
        for &a in &Action::ALL {
            let key = kb.key(a);
            assert!(
                seen.insert(key),
                "{a:?} shares its default key {key:?} with another action"
            );
            assert_ne!(key, KeyCode::Escape, "{a:?} may not bind Esc (reserved)");
        }
        assert_eq!(seen.len(), Action::ALL.len());
        // and the reverse lookup agrees with the forward table
        for &a in &Action::ALL {
            assert_eq!(kb.action_for(kb.key(a)), Some(a));
        }
    }

    #[test]
    fn serde_round_trip_preserves_all_bindings() {
        let kb = KeyBindings::default();
        let json = kb.to_json();
        let back = KeyBindings::from_json(&json, "round-trip.json");
        assert_eq!(kb, back);
    }

    #[test]
    fn remap_save_load_preserves_custom_binding() {
        let dir = temp_dir("remap");
        let dir_s = dir.to_str().unwrap();
        let mut kb = KeyBindings::load(dir_s); // no file yet: defaults
        assert_eq!(kb, KeyBindings::default());
        assert_eq!(kb.rebind(Action::Torch, KeyCode::KeyL), Rebind::Bound);
        kb.save(dir_s).unwrap();
        let back = KeyBindings::load(dir_s);
        assert_eq!(back.key(Action::Torch), KeyCode::KeyL);
        assert_eq!(back, kb);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rebind_conflict_swaps_keys() {
        let mut kb = KeyBindings::default();
        // P (Photo) onto Torch: Torch takes P, Photo takes Torch's old R
        assert_eq!(
            kb.rebind(Action::Torch, KeyCode::KeyP),
            Rebind::Swapped(Action::Photo)
        );
        assert_eq!(kb.key(Action::Torch), KeyCode::KeyP);
        assert_eq!(kb.key(Action::Photo), KeyCode::KeyR);
        // still complete and distinct after the swap
        let keys: HashSet<KeyCode> = Action::ALL.iter().map(|&a| kb.key(a)).collect();
        assert_eq!(keys.len(), Action::ALL.len());
        // same-key rebind and Esc are refused without change
        assert_eq!(kb.rebind(Action::Torch, KeyCode::KeyP), Rebind::Unchanged);
        assert_eq!(
            kb.rebind(Action::Torch, KeyCode::Escape),
            Rebind::Unchanged
        );
        assert_eq!(kb.key(Action::Torch), KeyCode::KeyP);
    }

    #[test]
    fn load_rejects_broken_files_wholesale() {
        // duplicate key: W on two actions
        let dup = r#"{"MoveForward":"KeyW","MoveBack":"KeyW"}"#;
        assert_eq!(
            KeyBindings::from_json(dup, "dup.json"),
            KeyBindings::default()
        );
        // reserved key
        let esc = r#"{"Torch":"Escape"}"#;
        assert_eq!(
            KeyBindings::from_json(esc, "esc.json"),
            KeyBindings::default()
        );
        // unknown action name
        let junk = r#"{"Dance":"KeyL"}"#;
        assert_eq!(
            KeyBindings::from_json(junk, "junk.json"),
            KeyBindings::default()
        );
        // not JSON at all
        assert_eq!(
            KeyBindings::from_json("nope", "nope.json"),
            KeyBindings::default()
        );
        // partial file from an older build: valid override + defaults fill
        let partial = r#"{"Torch":"KeyL"}"#;
        let kb = KeyBindings::from_json(partial, "partial.json");
        assert_eq!(kb.key(Action::Torch), KeyCode::KeyL);
        assert_eq!(kb.key(Action::Photo), KeyCode::KeyP);
    }
}

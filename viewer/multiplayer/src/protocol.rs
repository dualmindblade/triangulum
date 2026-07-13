use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Increment for every incompatible wire-schema or synchronization-law
/// change. A mismatch is always a hard refusal: different universes must
/// never be presented as one world.
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_PLAYER_NAME_BYTES: usize = 32;
pub const NEISOR_COLUMNS_PER_FACE: u64 = 10_000_000;
pub const MOON_COLUMNS_PER_FACE: u64 = 2_700_000;
pub const MAX_EDIT_BLOCKS: i64 = 4096;

pub type PlayerId = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldIdentity {
    pub seed: i64,
    pub build_hash: String,
    /// SHA-256 by immutable bake-relative path. Mutable saves are excluded.
    pub asset_hashes: BTreeMap<String, String>,
}

impl WorldIdentity {
    pub fn mismatch(&self, other: &Self) -> Option<String> {
        if self.seed != other.seed {
            return Some(format!(
                "seed mismatch: server={} client={}",
                self.seed, other.seed
            ));
        }
        if self.build_hash != other.build_hash {
            return Some(format!(
                "build hash mismatch: server={} client={}",
                self.build_hash, other.build_hash
            ));
        }
        if self.asset_hashes != other.asset_hashes {
            let mut names = self
                .asset_hashes
                .keys()
                .chain(other.asset_hashes.keys())
                .cloned()
                .collect::<Vec<_>>();
            names.sort();
            names.dedup();
            let details = names
                .into_iter()
                .filter(|name| self.asset_hashes.get(name) != other.asset_hashes.get(name))
                .map(|name| {
                    format!(
                        "{} (server={}, client={})",
                        name,
                        self.asset_hashes
                            .get(&name)
                            .map(String::as_str)
                            .unwrap_or("missing"),
                        other
                            .asset_hashes
                            .get(&name)
                            .map(String::as_str)
                            .unwrap_or("missing")
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Some(format!("asset hash mismatch: {details}"));
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyId {
    Neisor,
    Moon,
    Sun,
}

impl BodyId {
    pub const fn columns_per_face(self) -> Option<u64> {
        match self {
            Self::Neisor => Some(NEISOR_COLUMNS_PER_FACE),
            Self::Moon => Some(MOON_COLUMNS_PER_FACE),
            Self::Sun => None,
        }
    }

    pub const fn wire_id(self) -> u8 {
        match self {
            Self::Neisor => 0,
            Self::Moon => 1,
            Self::Sun => 2,
        }
    }

    pub fn from_wire_id(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Neisor),
            1 => Some(Self::Moon),
            2 => Some(Self::Sun),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerMode {
    Fly,
    Walk,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BodyPose {
    pub body: BodyId,
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// Radial height above the body's nominal radius, in kilometres. This
    /// includes terrain elevation and puts the pose at the player's eye.
    pub alt_km: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
    pub mode: PlayerMode,
}

impl BodyPose {
    pub fn is_valid(&self) -> bool {
        self.body != BodyId::Sun
            && self.lat_deg.is_finite()
            && (-90.0..=90.0).contains(&self.lat_deg)
            && self.lon_deg.is_finite()
            && self.alt_km.is_finite()
            && (-100.0..=100_000.0).contains(&self.alt_km)
            && self.yaw_deg.is_finite()
            && self.pitch_deg.is_finite()
            && self.roll_deg.is_finite()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub name: String,
    pub tint: [f32; 3],
    pub pose: Option<BodyPose>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClockState {
    /// Sequence of the last authoritative clock event.
    pub sequence: u64,
    /// Canonical absolute time sampled at `server_mono_ms`.
    pub absolute_time_s: f64,
    pub time_scale: f64,
    /// Milliseconds since this server process's monotonic epoch.
    pub server_mono_ms: u64,
}

impl ClockState {
    pub fn at_server_mono_ms(&self, mono_ms: u64) -> f64 {
        self.absolute_time_s
            + (mono_ms as f64 - self.server_mono_ms as f64) * 0.001 * self.time_scale
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockEventKind {
    Seek,
    TimeScale,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClockEvent {
    pub kind: ClockEventKind,
    pub state: ClockState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ClockCommand {
    Seek { absolute_time_s: f64 },
    SetTimeScale { time_scale: f64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRequest {
    pub body: BodyId,
    pub face: u8,
    pub ci: u64,
    pub cj: u64,
    /// Absolute per-column height delta. Server sequence makes this LWW.
    pub value: i64,
}

impl EditRequest {
    pub fn validate(&self) -> Result<(), String> {
        let n = self
            .body
            .columns_per_face()
            .ok_or_else(|| "the Sun is not editable".to_string())?;
        if self.face >= 6 {
            return Err(format!("face {} is outside 0..6", self.face));
        }
        if self.ci >= n || self.cj >= n {
            return Err(format!(
                "column ({},{}) is outside the {:?} lattice {}",
                self.ci, self.cj, self.body, n
            ));
        }
        if !(-MAX_EDIT_BLOCKS..=MAX_EDIT_BLOCKS).contains(&self.value) {
            return Err(format!(
                "edit value {} exceeds +/-{}",
                self.value, MAX_EDIT_BLOCKS
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRecord {
    pub sequence: u64,
    pub accepted_at_mono_ms: u64,
    pub edit: EditRequest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub token: String,
    pub protocol_version: u32,
    pub build_hash: String,
    pub seed: i64,
    pub asset_hashes: BTreeMap<String, String>,
    pub name: String,
}

impl Hello {
    pub fn identity(&self) -> WorldIdentity {
        WorldIdentity {
            seed: self.seed,
            build_hash: self.build_hash.clone(),
            asset_hashes: self.asset_hashes.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    pub protocol_version: u32,
    pub player_id: PlayerId,
    pub identity: WorldIdentity,
    pub clock: ClockState,
    /// Full ordered append-only history for MP1 join/replay.
    pub edit_journal: Vec<EditRecord>,
    pub players: Vec<PlayerInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub code: String,
    pub message: String,
}

/// One small, versioned JSON WebSocket enum. Direction-valid variants are
/// documented in MULTIPLAYER-PROTOCOL.md; invalid directions receive Error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Message {
    Hello(Hello),
    Welcome(Welcome),
    Refusal(Refusal),
    EditRequest(EditRequest),
    Edit(EditRecord),
    PresenceUpdate(BodyPose),
    Presence { player_id: PlayerId, pose: BodyPose },
    ClockCommand(ClockCommand),
    ClockEvent(ClockEvent),
    PlayerJoined(PlayerInfo),
    PlayerLeft { player_id: PlayerId },
    Ping { nonce: u64 },
    Pong { nonce: u64, clock: ClockState },
    Error { code: String, message: String },
}

pub fn clean_player_name(input: &str) -> String {
    let trimmed = input.trim();
    let mut out = String::new();
    for ch in trimmed.chars() {
        if ch.is_control() {
            continue;
        }
        if out.len() + ch.len_utf8() > MAX_PLAYER_NAME_BYTES {
            break;
        }
        out.push(ch);
    }
    if out.is_empty() {
        "Player".to_string()
    } else {
        out
    }
}

pub fn player_tint(id: PlayerId) -> [f32; 3] {
    let mut x = id.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    let channel = |shift: u32| 0.35 + (((x >> shift) & 0xff) as f32 / 255.0) * 0.55;
    [channel(0), channel(8), channel(16)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(message: Message) {
        let json = serde_json::to_string(&message).unwrap();
        let decoded: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn protocol_round_trips_representative_messages() {
        let identity = WorldIdentity {
            seed: 42,
            build_hash: "b67b973".into(),
            asset_hashes: BTreeMap::from([("meta.json".into(), "abc".into())]),
        };
        let pose = BodyPose {
            body: BodyId::Moon,
            lat_deg: 12.5,
            lon_deg: -33.25,
            alt_km: 0.004,
            yaw_deg: 90.0,
            pitch_deg: -4.0,
            roll_deg: 1.0,
            mode: PlayerMode::Walk,
        };
        let edit = EditRecord {
            sequence: 7,
            accepted_at_mono_ms: 123,
            edit: EditRequest {
                body: BodyId::Neisor,
                face: 2,
                ci: 3,
                cj: 4,
                value: -1,
            },
        };
        let clock = ClockState {
            sequence: 2,
            absolute_time_s: 99.5,
            time_scale: 10.0,
            server_mono_ms: 500,
        };
        round_trip(Message::Hello(Hello {
            token: "secret".into(),
            protocol_version: PROTOCOL_VERSION,
            build_hash: identity.build_hash.clone(),
            seed: identity.seed,
            asset_hashes: identity.asset_hashes.clone(),
            name: "Alice".into(),
        }));
        round_trip(Message::Welcome(Welcome {
            protocol_version: PROTOCOL_VERSION,
            player_id: 1,
            identity,
            clock: clock.clone(),
            edit_journal: vec![edit.clone()],
            players: vec![],
        }));
        round_trip(Message::EditRequest(edit.edit.clone()));
        round_trip(Message::Edit(edit));
        round_trip(Message::PresenceUpdate(pose.clone()));
        round_trip(Message::Presence {
            player_id: 9,
            pose: pose.clone(),
        });
        round_trip(Message::ClockCommand(ClockCommand::Seek {
            absolute_time_s: 12.0,
        }));
        round_trip(Message::ClockCommand(ClockCommand::SetTimeScale {
            time_scale: 60.0,
        }));
        round_trip(Message::ClockEvent(ClockEvent {
            kind: ClockEventKind::Seek,
            state: clock.clone(),
        }));
        let player = PlayerInfo {
            id: 9,
            name: "Bob".into(),
            tint: [0.2, 0.4, 0.8],
            pose: Some(pose),
        };
        round_trip(Message::PlayerJoined(player));
        round_trip(Message::PlayerLeft { player_id: 9 });
        round_trip(Message::Refusal(Refusal {
            code: "identity_mismatch".into(),
            message: "loud".into(),
        }));
        round_trip(Message::Ping { nonce: 44 });
        round_trip(Message::Pong { nonce: 44, clock });
        round_trip(Message::Error {
            code: "time_authority".into(),
            message: "D-17".into(),
        });
    }

    #[test]
    fn identity_mismatch_is_specific() {
        let a = WorldIdentity {
            seed: 42,
            build_hash: "a".into(),
            asset_hashes: BTreeMap::new(),
        };
        let b = WorldIdentity {
            build_hash: "b".into(),
            ..a.clone()
        };
        assert!(a.mismatch(&b).unwrap().contains("build hash mismatch"));
    }
}

//! Replays (M2c): a recorded session that plays back exactly. DESIGN.md:
//! "Replays are a snapshot plus the ordered player inputs." This is the
//! project's bug report: press F10, do the thing, press F10, send the file.
//!
//! The file (`.replay`) is one piece, written as it goes, so a crash still
//! leaves everything up to the last second:
//!
//!   triangulum-replay 1          the magic line
//!   {header JSON, one line}      the world's identity and the starting
//!                                state outside the save: the player, the
//!                                step counter, the sky clock, the
//!                                simulation's settings and region
//!   <the snapshot>               a whole save (header.json, commit.json,
//!                                every edited chunk, the simulation state)
//!                                as files back to back; names, sizes and
//!                                SHA-256s are in the header
//!   @STEP EVENT ...              the inputs and checks, one per line
//!
//! STEP counts fixed 60 Hz steps from the start of the recording. The
//! events at STEP happen, in file order, after STEP steps and before the
//! next one. Events (the text `Session::apply` and `Session::step` saw):
//!
//!   keys w,a,s,d,space,ctrl,shift | keys -   held keys from the next step
//!   look DX DY            mouse motion, raw counts
//!   press break|place     a mouse button down (acts, then repeats held)
//!   release               the button up
//!   click break|place     one click (the harness's tap)
//!   select N              hotbar slot 1-9
//!   mode walk|fly         G, as the mode it set
//!   ground                R
//!   preset N              P (preset index from 0)
//!   water on|off          F, as shown or hidden
//!   scroll LINES          the wheel (fly speed)
//!   sky SECONDS SPEED     the sky clock ([ and ], or a harness seek)
//!   sim on|off   simradius M   waterperiod N
//!   pose X,Y,Z,VX,VY,VZ,HEADING,PITCH,MODE,GROUND,WATER,HEAD,FLYSPEED,SLOT
//!                         the player put exactly there (harness teleports)
//!   reload                save, then rebuild from the save
//!   note TEXT             something the recorder could not capture
//!   check K=V ...         the state after STEP steps, every second: the
//!                         digests of the world, the simulation, the
//!                         player and the view, the counts behind them,
//!                         the exact pose and clock, and the digest of
//!                         every chunk that changed since the last check,
//!                         so a divergence names its chunks
//!   end sha=HEX steps=N   the end: `Sim::state_hash` (the harness's sim_hash)
//!
//! Floats are written in Rust's shortest round-trip form and read back bit
//! for bit. Playing back (`session_from`, `Runner`) builds a session from
//! the snapshot in a scratch save directory (never the real save), feeds
//! the events through the same `Session::apply` and `Session::step` the app
//! and the harness use, compares every check and the end, and reports the
//! first divergence.

use crate::grid::{ChunkKey, FastMap};
use crate::player::{Input, Mode, Player, STEP};
use crate::session::{Action, Button, Session};
use crate::sha256::{hex, sha256};
use crate::sim::SimConfig;
use crate::sky::Clock;
use crate::world::EditStore;
use crate::world::edits::Edited;
use crate::world::save::{self, SaveDir};
use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const MAGIC: &str = "triangulum-replay";
pub const VERSION: u32 = 1;
/// A check every second of play.
pub const CHECK_EVERY: u64 = 60;

// ---------------------------------------------------------------------------
// Digests
// ---------------------------------------------------------------------------

/// A fast 64-bit digest for the periodic checks (not cryptographic: it only
/// has to notice that two runs differ). Word at a time, multiply-rotate,
/// with a final avalanche.
pub struct Digest {
    h: u64,
    n: u64,
}

impl Digest {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;

    pub fn new(domain: &[u8]) -> Self {
        let mut d = Self { h: 0x243F_6A88_85A3_08D3, n: 0 };
        d.bytes(domain);
        d
    }

    #[inline]
    pub fn u64(&mut self, x: u64) {
        self.h = (self.h.rotate_left(23) ^ x).wrapping_mul(Self::K);
        self.n += 1;
    }

    pub fn bytes(&mut self, b: &[u8]) {
        let mut it = b.chunks_exact(8);
        for c in &mut it {
            self.u64(u64::from_le_bytes(c.try_into().unwrap()));
        }
        let r = it.remainder();
        let mut last = [0u8; 8];
        last[..r.len()].copy_from_slice(r);
        self.u64(u64::from_le_bytes(last));
        self.u64(b.len() as u64);
    }

    pub fn u16s(&mut self, v: &[u16]) {
        let mut it = v.chunks_exact(4);
        for c in &mut it {
            self.u64(c[0] as u64 | (c[1] as u64) << 16 | (c[2] as u64) << 32 | (c[3] as u64) << 48);
        }
        for &x in it.remainder() {
            self.u64(x as u64);
        }
        self.u64(v.len() as u64);
    }

    pub fn finish(&self) -> u64 {
        let mut z = self.h ^ self.n.wrapping_mul(Self::K);
        z ^= z >> 33;
        z = z.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
        z ^= z >> 33;
        z = z.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
        z ^ (z >> 33)
    }
}

fn key_text(k: ChunkKey) -> String {
    format!("{}.{}.{}.{}.{}", k.body, k.face, k.cx, k.cy, k.cz)
}

fn parse_key(s: &str) -> Result<ChunkKey, String> {
    let v: Vec<u32> = s.split('.').map(|x| x.parse::<u32>()).collect::<Result<_, _>>().map_err(|_| format!("bad chunk key {s:?}"))?;
    match v[..] {
        [body, face, cx, cy, cz] if face < 6 && body < 256 => Ok(ChunkKey { body: body as u8, face: face as u8, lod: 0, cx, cy, cz }),
        _ => Err(format!("bad chunk key {s:?}")),
    }
}

fn describe_key(k: ChunkKey) -> String {
    format!("face {} chunk ({}, {}, {}) (blocks i {}.., j {}.., k {}..)", k.face, k.cx, k.cy, k.cz, k.cx as usize * crate::grid::CHUNK, k.cy as usize * crate::grid::CHUNK, k.cz as usize * crate::grid::CHUNK)
}

/// The digest of one edited chunk: its ids and its water levels (no water
/// and all-zero levels are the same).
fn chunk_digest(key: ChunkKey, e: &Edited) -> u64 {
    let mut d = Digest::new(b"chunk");
    d.bytes(&[key.body, key.face, key.lod]);
    d.u64(key.cx as u64);
    d.u64(key.cy as u64);
    d.u64(key.cz as u64);
    d.u16s(&e.ids.to_dense());
    match &e.water {
        Some(l) if l.iter().any(|&x| x != 0) => {
            d.u64(1);
            d.bytes(l);
        }
        _ => d.u64(0),
    }
    d.finish()
}

/// Per-chunk digests, cached by revision so a check re-reads only the
/// chunks that changed (one edit store at a time: a reload starts over).
#[derive(Default)]
pub struct ChunkDigests {
    store: Option<Arc<EditStore>>,
    cache: FastMap<ChunkKey, (u64, u64)>,
}

impl ChunkDigests {
    /// Every edited chunk's digest, by key.
    pub fn all(&mut self, store: &Arc<EditStore>) -> BTreeMap<ChunkKey, u64> {
        if !self.store.as_ref().is_some_and(|s| Arc::ptr_eq(s, store)) {
            self.cache.clear();
            self.store = Some(store.clone());
        }
        let mut out = BTreeMap::new();
        for key in store.keys() {
            let cached = self.cache.get(&key).copied();
            let Some((rev, d)) = store.with_edited(key, |e| match cached {
                Some((rev, d)) if rev == e.rev => (rev, d),
                _ => (e.rev, chunk_digest(key, e)),
            }) else {
                continue;
            };
            self.cache.insert(key, (rev, d));
            out.insert(key, d);
        }
        out
    }
}

/// The hash of the world's edits and how many chunks hold them (a photo's
/// `edits`): the digest a replay's checks use, read and nothing changed.
pub fn edit_state(store: &Arc<EditStore>, digests: &mut ChunkDigests) -> (String, usize) {
    let map = digests.all(store);
    (format!("{:016x}", world_digest(&map)), map.len())
}

fn world_digest(map: &BTreeMap<ChunkKey, u64>) -> u64 {
    let mut d = Digest::new(b"world");
    d.u64(map.len() as u64);
    for (k, v) in map {
        d.bytes(&[k.body, k.face, k.lod]);
        d.u64(k.cx as u64);
        d.u64(k.cy as u64);
        d.u64(k.cz as u64);
        d.u64(*v);
    }
    d.finish()
}

// ---------------------------------------------------------------------------
// Text forms
// ---------------------------------------------------------------------------

/// Shortest round-trip form: `str::parse` gives back the same bits.
fn f(x: f64) -> String {
    format!("{x:?}")
}

fn pf(s: &str) -> Result<f64, String> {
    s.parse::<f64>().map_err(|_| format!("bad number {s:?}"))
}

fn mode_text(m: Mode) -> &'static str {
    if m == Mode::Walk { "walk" } else { "fly" }
}

fn parse_mode(s: &str) -> Result<Mode, String> {
    match s {
        "walk" => Ok(Mode::Walk),
        "fly" => Ok(Mode::Fly),
        _ => Err(format!("bad mode {s:?}")),
    }
}

/// The player's whole state, exactly.
pub fn pose_text(p: &Player) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        f(p.pos.x),
        f(p.pos.y),
        f(p.pos.z),
        f(p.vel.x),
        f(p.vel.y),
        f(p.vel.z),
        f(p.heading),
        f(p.pitch),
        mode_text(p.mode),
        p.on_ground as u8,
        p.in_water as u8,
        p.head_in_water as u8,
        f(p.fly_speed),
        p.selected
    )
}

pub fn parse_pose(s: &str, block_size: f64) -> Result<Player, String> {
    let v: Vec<&str> = s.split(',').collect();
    if v.len() != 14 {
        return Err(format!("a pose has 14 fields, not {}: {s:?}", v.len()));
    }
    let flag = |x: &str| match x {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("bad flag {x:?}")),
    };
    let mut p = Player::new(DVec3::new(pf(v[0])?, pf(v[1])?, pf(v[2])?), 0.0, 0.0, parse_mode(v[8])?, block_size);
    p.vel = DVec3::new(pf(v[3])?, pf(v[4])?, pf(v[5])?);
    p.heading = pf(v[6])?;
    p.pitch = pf(v[7])?;
    p.on_ground = flag(v[9])?;
    p.in_water = flag(v[10])?;
    p.head_in_water = flag(v[11])?;
    p.fly_speed = pf(v[12])?;
    p.selected = v[13].parse().map_err(|_| format!("bad hotbar slot {:?}", v[13]))?;
    Ok(p)
}

fn pose_digest(p: &Player) -> u64 {
    let mut d = Digest::new(b"player");
    for x in [p.pos.x, p.pos.y, p.pos.z, p.vel.x, p.vel.y, p.vel.z, p.heading, p.pitch, p.fly_speed, p.block_size] {
        d.u64(x.to_bits());
    }
    d.u64((p.mode == Mode::Walk) as u64);
    d.u64(p.on_ground as u64 | (p.in_water as u64) << 1 | (p.head_in_water as u64) << 2);
    d.u64(p.selected as u64);
    d.finish()
}

const KEY_NAMES: [&str; 7] = ["w", "s", "a", "d", "space", "ctrl", "shift"];

fn keys_bits(i: &Input) -> u8 {
    [i.forward, i.back, i.left, i.right, i.jump, i.down, i.sprint].iter().enumerate().fold(0, |b, (n, &on)| b | ((on as u8) << n))
}

fn keys_text(i: &Input) -> String {
    let b = keys_bits(i);
    let on: Vec<&str> = KEY_NAMES.iter().enumerate().filter(|(n, _)| b & (1 << n) != 0).map(|(_, k)| *k).collect();
    if on.is_empty() { "-".into() } else { on.join(",") }
}

fn parse_keys(s: &str) -> Result<Input, String> {
    let mut i = Input::default();
    if s == "-" {
        return Ok(i);
    }
    for k in s.split(',') {
        match k {
            "w" => i.forward = true,
            "s" => i.back = true,
            "a" => i.left = true,
            "d" => i.right = true,
            "space" => i.jump = true,
            "ctrl" => i.down = true,
            "shift" => i.sprint = true,
            _ => return Err(format!("unknown key {k:?}")),
        }
    }
    Ok(i)
}

fn button_text(b: Button) -> &'static str {
    match b {
        Button::Break => "break",
        Button::Place => "place",
    }
}

fn parse_button(s: Option<&&str>) -> Result<Button, String> {
    match s.copied() {
        Some("break") => Ok(Button::Break),
        Some("place") => Ok(Button::Place),
        other => Err(format!("bad button {other:?}")),
    }
}

/// How an action is written in a replay.
pub fn action_text(a: &Action) -> String {
    match a {
        Action::Look(dx, dy) => format!("look {} {}", f(*dx), f(*dy)),
        Action::Press(b) => format!("press {}", button_text(*b)),
        Action::Release => "release".into(),
        Action::Click(b) => format!("click {}", button_text(*b)),
        Action::Select(i) => format!("select {}", i + 1),
        Action::ToggleMode => "mode toggle".into(),
        Action::SetMode(m) => format!("mode {}", mode_text(*m)),
        Action::Ground => "ground".into(),
        Action::Preset(i) => format!("preset {i}"),
        Action::ToggleWater => "water toggle".into(),
        Action::HideWater(h) => format!("water {}", if *h { "off" } else { "on" }),
        Action::Scroll(l) => format!("scroll {}", f(*l)),
        Action::Time(faster) => format!("time {}", if *faster { "faster" } else { "slower" }),
        Action::Sky(c) => format!("sky {} {}", f(c.seconds), f(c.speed)),
        Action::Sim(on) => format!("sim {}", if *on { "on" } else { "off" }),
        Action::SimRadius(m) => format!("simradius {}", f(*m)),
        Action::WaterPeriod(n) => format!("waterperiod {n}"),
        Action::Pose(p) => format!("pose {}", pose_text(p)),
        Action::Reload => "reload".into(),
    }
}

fn parse_action(verb: &str, args: &[&str], block_size: f64) -> Result<Action, String> {
    let arg = |n: usize| args.get(n).copied().ok_or_else(|| format!("{verb}: missing argument {}", n + 1));
    Ok(match verb {
        "look" => Action::Look(pf(arg(0)?)?, pf(arg(1)?)?),
        "press" => Action::Press(parse_button(args.first())?),
        "release" => Action::Release,
        "click" => Action::Click(parse_button(args.first())?),
        "select" => match arg(0)?.parse::<usize>() {
            Ok(n) if (1..=9).contains(&n) => Action::Select(n - 1),
            _ => return Err(format!("bad hotbar slot {:?}", arg(0)?)),
        },
        "mode" => match arg(0)? {
            "toggle" => Action::ToggleMode,
            m => Action::SetMode(parse_mode(m)?),
        },
        "ground" => Action::Ground,
        "preset" => Action::Preset(arg(0)?.parse().map_err(|_| "bad preset index".to_string())?),
        "water" => match arg(0)? {
            "on" => Action::HideWater(false),
            "off" => Action::HideWater(true),
            "toggle" => Action::ToggleWater,
            w => return Err(format!("water {w:?}")),
        },
        "scroll" => Action::Scroll(pf(arg(0)?)?),
        "time" => Action::Time(arg(0)? == "faster"),
        "sky" => Action::Sky(Clock { seconds: pf(arg(0)?)?, speed: pf(arg(1)?)? }),
        "sim" => Action::Sim(arg(0)? == "on"),
        "simradius" => Action::SimRadius(pf(arg(0)?)?),
        "waterperiod" => Action::WaterPeriod(arg(0)?.parse().map_err(|_| "bad water period".to_string())?),
        "pose" => Action::Pose(Box::new(parse_pose(arg(0)?, block_size)?)),
        "reload" => Action::Reload,
        other => return Err(format!("unknown event {other:?}")),
    })
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// The state after a step, as a check records it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Check {
    pub tick: u64,
    pub world: u64,
    pub sim: u64,
    pub player: u64,
    pub view: u64,
    pub edits: usize,
    pub queued: usize,
    pub frozen: usize,
    pub falling: usize,
    /// Water counters: in, out, placed, removed, displaced, lost, moved.
    pub flow: [u64; 7],
    pub clock: String,
    pub hidden: bool,
    pub pose: String,
    /// Chunks whose digest changed since the previous check.
    pub chunks: Vec<(ChunkKey, u64)>,
}

/// The check for a session's state now, and every chunk's digest.
pub fn measure(s: &Session, digests: &mut ChunkDigests) -> (Check, BTreeMap<ChunkKey, u64>) {
    let map = digests.all(&s.engine.edits);
    let (sim, queued) = s.sim.state_digest(s.engine.world.as_ref());
    let mut view = Digest::new(b"view");
    view.u64(s.sky.seconds.to_bits());
    view.u64(s.sky.speed.to_bits());
    view.u64(s.engine.hide_water as u64);
    let st = &s.sim.stats;
    let c = Check {
        tick: s.sim.tick(),
        world: world_digest(&map),
        sim,
        player: pose_digest(&s.player),
        view: view.finish(),
        edits: map.len(),
        queued,
        frozen: s.sim.frozen_count(),
        falling: s.sim.falling().len(),
        flow: [st.water_in, st.water_out, st.water_placed, st.water_removed, st.water_displaced, st.water_lost, st.water_moved],
        clock: format!("{},{}", f(s.sky.seconds), f(s.sky.speed)),
        hidden: s.engine.hide_water,
        pose: pose_text(&s.player),
        chunks: Vec::new(),
    };
    (c, map)
}

fn check_text(c: &Check) -> String {
    let mut t = format!(
        "check tick={} world={:016x} sim={:016x} player={:016x} view={:016x} edits={} queued={} frozen={} falling={} flow={} clock={} hidden={} pose={}",
        c.tick,
        c.world,
        c.sim,
        c.player,
        c.view,
        c.edits,
        c.queued,
        c.frozen,
        c.falling,
        c.flow.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
        c.clock,
        c.hidden as u8,
        c.pose
    );
    if !c.chunks.is_empty() {
        t.push_str(" chunks=");
        t.push_str(&c.chunks.iter().map(|(k, d)| format!("{}:{d:016x}", key_text(*k))).collect::<Vec<_>>().join(","));
    }
    t
}

fn parse_check(args: &[&str]) -> Result<Check, String> {
    let mut c = Check::default();
    let hex64 = |v: &str| u64::from_str_radix(v, 16).map_err(|_| format!("bad digest {v:?}"));
    let num = |v: &str| v.parse::<usize>().map_err(|_| format!("bad count {v:?}"));
    for a in args {
        let (k, v) = a.split_once('=').ok_or_else(|| format!("check field {a:?} is not KEY=VALUE"))?;
        match k {
            "tick" => c.tick = v.parse().map_err(|_| format!("bad tick {v:?}"))?,
            "world" => c.world = hex64(v)?,
            "sim" => c.sim = hex64(v)?,
            "player" => c.player = hex64(v)?,
            "view" => c.view = hex64(v)?,
            "edits" => c.edits = num(v)?,
            "queued" => c.queued = num(v)?,
            "frozen" => c.frozen = num(v)?,
            "falling" => c.falling = num(v)?,
            "flow" => {
                let xs: Vec<u64> = v.split(',').map(|x| x.parse::<u64>()).collect::<Result<_, _>>().map_err(|_| format!("bad flow {v:?}"))?;
                c.flow = xs.try_into().map_err(|_| format!("flow has 7 counters: {v:?}"))?;
            }
            "clock" => c.clock = v.to_string(),
            "hidden" => c.hidden = v == "1",
            "pose" => c.pose = v.to_string(),
            "chunks" => {
                for e in v.split(',') {
                    let (key, d) = e.split_once(':').ok_or_else(|| format!("bad chunk entry {e:?}"))?;
                    c.chunks.push((parse_key(key)?, hex64(d)?));
                }
            }
            _ => {} // a newer recorder's field: ignored
        }
    }
    Ok(c)
}

/// What differs between a recorded check and the replay's, in words.
fn compare(rec: &Check, got: &Check, rec_map: &BTreeMap<ChunkKey, u64>, got_map: &BTreeMap<ChunkKey, u64>) -> Vec<String> {
    let mut w = Vec::new();
    if rec.player != got.player || rec.pose != got.pose {
        match (parse_pose(&rec.pose, 1.0), parse_pose(&got.pose, 1.0)) {
            (Ok(a), Ok(b)) => {
                let mut parts = Vec::new();
                let d = a.pos.distance(b.pos);
                if a.pos != b.pos {
                    parts.push(format!("position {d:.6} m apart (recorded {:.4}, {:.4}, {:.4}; replayed {:.4}, {:.4}, {:.4})", a.pos.x, a.pos.y, a.pos.z, b.pos.x, b.pos.y, b.pos.z));
                }
                if a.vel != b.vel {
                    parts.push(format!("velocity recorded {:.4} m/s, replayed {:.4} m/s", a.vel.length(), b.vel.length()));
                }
                if a.heading != b.heading || a.pitch != b.pitch {
                    parts.push(format!("look recorded {:.4} / {:.4} deg, replayed {:.4} / {:.4} deg", a.heading.to_degrees(), a.pitch.to_degrees(), b.heading.to_degrees(), b.pitch.to_degrees()));
                }
                if a.mode != b.mode {
                    parts.push(format!("mode recorded {}, replayed {}", mode_text(a.mode), mode_text(b.mode)));
                }
                if (a.on_ground, a.in_water, a.head_in_water) != (b.on_ground, b.in_water, b.head_in_water) {
                    parts.push(format!("on ground / in water / head in water recorded {} {} {}, replayed {} {} {}", a.on_ground, a.in_water, a.head_in_water, b.on_ground, b.in_water, b.head_in_water));
                }
                if a.fly_speed != b.fly_speed {
                    parts.push(format!("fly speed recorded {}, replayed {}", a.fly_speed, b.fly_speed));
                }
                if a.selected != b.selected {
                    parts.push(format!("hotbar slot recorded {}, replayed {}", a.selected + 1, b.selected + 1));
                }
                if parts.is_empty() {
                    parts.push("the player digest differs".into());
                }
                w.push(format!("player: {}", parts.join("; ")));
            }
            _ => w.push(format!("player: digest {:016x} recorded, {:016x} replayed", rec.player, got.player)),
        }
    }
    if rec.world != got.world || rec.edits != got.edits {
        let keys: BTreeSet<ChunkKey> = rec_map.keys().chain(got_map.keys()).copied().collect();
        let differ: Vec<String> = keys
            .iter()
            .filter(|k| rec_map.get(k) != got_map.get(k))
            .map(|k| match (rec_map.get(k), got_map.get(k)) {
                (Some(_), Some(_)) => format!("{} differs", describe_key(*k)),
                (Some(_), None) => format!("{} edited in the recording only", describe_key(*k)),
                _ => format!("{} edited in the replay only", describe_key(*k)),
            })
            .collect();
        let shown = differ.iter().take(8).cloned().collect::<Vec<_>>().join("; ");
        let more = if differ.len() > 8 { format!("; and {} more", differ.len() - 8) } else { String::new() };
        w.push(format!("world: digest {:016x} recorded, {:016x} replayed; {} edited chunks recorded, {} replayed; {} differ: {shown}{more}", rec.world, got.world, rec.edits, got.edits, differ.len()));
    }
    if rec.sim != got.sim || rec.tick != got.tick {
        let counts = (rec.tick, rec.queued, rec.frozen, rec.falling) != (got.tick, got.queued, got.frozen, got.falling);
        w.push(format!(
            "simulation: tick {} / {}, queued {} / {}, frozen {} / {}, falling {} / {} (recorded / replayed){}",
            rec.tick,
            got.tick,
            rec.queued,
            got.queued,
            rec.frozen,
            got.frozen,
            rec.falling,
            got.falling,
            if counts { "" } else { "; same counts, different contents (queue, region or falling blocks)" }
        ));
    }
    if rec.view != got.view || rec.clock != got.clock || rec.hidden != got.hidden {
        w.push(format!("view: sky clock (seconds, speed) {} recorded, {} replayed; water {} recorded, {} replayed", rec.clock, got.clock, if rec.hidden { "hidden" } else { "shown" }, if got.hidden { "hidden" } else { "shown" }));
    }
    if rec.flow != got.flow {
        w.push(format!("water counters (in, out, placed, removed, displaced, lost, moved) {:?} recorded, {:?} replayed", rec.flow, got.flow));
    }
    w
}

// ---------------------------------------------------------------------------
// The header
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Everything about the start that is not in the save. Exact floats are
/// strings (shortest round-trip form).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    pub engine: String,
    pub created_utc: String,
    pub note: String,
    pub world: save::Identity,
    pub generator_version: u32,
    /// `sim::RULES_VERSION` of the recording build (absent: 1). Other rules
    /// would not reproduce the session, so another version is refused.
    #[serde(default = "rules_v1")]
    pub sim_rules: u32,
    pub bake_sha256: String,
    pub hotbar: Vec<String>,
    pub hide_water: bool,
    /// The sky calendar and optics (`sky::Settings`), when main installed one.
    pub sky_settings: BTreeMap<String, String>,
    /// The sky clock: seconds, speed.
    pub clock: [String; 2],
    pub sim_radius_m: String,
    pub sim_enabled: bool,
    pub water_period: u64,
    /// The simulation's counters (the HUD's water balance and falls).
    pub counters: BTreeMap<String, u64>,
    /// The session's step counter (the tick is every third step).
    pub steps: u64,
    /// A held mouse button and the step it last acted on.
    pub held: Option<(String, u64)>,
    pub player: String,
    /// The world digest and edited chunk count at the start.
    pub start_world: String,
    pub start_edits: usize,
    /// The real save this was taken from, for reference (the snapshot below
    /// is self-contained).
    pub save_ref: Option<String>,
    pub files: Vec<FileEntry>,
}

fn rules_v1() -> u32 {
    1
}

fn counters_of(s: &crate::sim::SimStats) -> BTreeMap<String, u64> {
    [
        ("falls", s.falls),
        ("landings", s.landings),
        ("vanished", s.vanished),
        ("processed", s.processed),
        ("water_moved", s.water_moved),
        ("water_writes", s.water_writes),
        ("water_in", s.water_in),
        ("water_out", s.water_out),
        ("water_placed", s.water_placed),
        ("water_removed", s.water_removed),
        ("water_displaced", s.water_displaced),
        ("water_lost", s.water_lost),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

fn restore_counters(s: &mut crate::sim::SimStats, c: &BTreeMap<String, u64>) {
    let get = |k: &str| c.get(k).copied().unwrap_or(0);
    s.falls = get("falls");
    s.landings = get("landings");
    s.vanished = get("vanished");
    s.processed = get("processed");
    s.water_moved = get("water_moved");
    s.water_writes = get("water_writes");
    s.water_in = get("water_in");
    s.water_out = get("water_out");
    s.water_placed = get("water_placed");
    s.water_removed = get("water_removed");
    s.water_displaced = get("water_displaced");
    s.water_lost = get("water_lost");
}

fn sky_settings_map() -> BTreeMap<String, String> {
    let Some(st) = crate::sky::installed() else { return BTreeMap::new() };
    let s = st.settings;
    [
        ("day_length_s", f(s.day_length_s)),
        ("lunar_month_days", f(s.lunar_month_days)),
        ("months_per_season", s.months_per_season.to_string()),
        ("year_days", f(s.year_days)),
        ("atmosphere_height_radii", f(s.atmosphere_height_radii)),
        ("limb_height_radii", f(s.limb_height_radii)),
        ("fog_per_radius", f(s.fog_per_radius)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

impl Header {
    /// The sky settings recorded with this replay, over `base`.
    pub fn sky_settings(&self, base: &crate::sky::Settings) -> Result<crate::sky::Settings, String> {
        let mut s = base.clone();
        let m = &self.sky_settings;
        let get = |k: &str, cur: f64| m.get(k).map_or(Ok(cur), |v| pf(v));
        s.day_length_s = get("day_length_s", s.day_length_s)?;
        s.lunar_month_days = get("lunar_month_days", s.lunar_month_days)?;
        if let Some(v) = m.get("months_per_season") {
            s.months_per_season = v.parse().map_err(|_| format!("bad months_per_season {v:?}"))?;
        }
        s.year_days = get("year_days", s.year_days)?;
        s.atmosphere_height_radii = get("atmosphere_height_radii", s.atmosphere_height_radii)?;
        s.limb_height_radii = get("limb_height_radii", s.limb_height_radii)?;
        s.fog_per_radius = get("fog_per_radius", s.fog_per_radius)?;
        Ok(s)
    }

    pub fn clock(&self) -> Result<Clock, String> {
        Ok(Clock { seconds: pf(&self.clock[0])?, speed: pf(&self.clock[1])? })
    }
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

/// What the recorder last saw, so a change made outside `apply`/`step`
/// (a harness teleport, a seek of the sky) is noticed and written down.
#[derive(Clone, Copy, PartialEq)]
struct Expect {
    pose: u64,
    sim: (u64, bool, u64),
    sky: (u64, u64),
    hide: bool,
    edit_gen: u64,
    store: usize,
}

impl Expect {
    fn of(s: &Session) -> Self {
        let c = &s.sim.config;
        Self {
            pose: pose_digest(&s.player),
            sim: (c.radius_m.to_bits(), c.enabled, c.water_period),
            sky: (s.sky.seconds.to_bits(), s.sky.speed.to_bits()),
            hide: s.engine.hide_water,
            edit_gen: s.engine.edits.generation(),
            store: Arc::as_ptr(&s.engine.edits) as usize,
        }
    }
}

/// What a finished recording holds.
#[derive(Clone, Debug)]
pub struct Summary {
    pub path: PathBuf,
    pub steps: u64,
    pub events: u64,
    pub checks: u64,
    pub notes: u64,
    pub bytes: u64,
    pub snapshot_bytes: u64,
}

/// A replay being written (owned by `Session::recorder`).
pub struct Recorder {
    path: PathBuf,
    out: Option<std::io::BufWriter<std::fs::File>>,
    start_steps: u64,
    keys: Option<u8>,
    expect: Expect,
    digests: ChunkDigests,
    prev: BTreeMap<ChunkKey, u64>,
    events: u64,
    checks: u64,
    notes: u64,
    snapshot_bytes: u64,
    /// The first write error; recording stops writing after it.
    pub error: Option<String>,
}

/// `game/replays/<world>-<UTC date>-<time>.replay`.
pub fn default_path(world_name: &str) -> PathBuf {
    let name: String = world_name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let (date, time) = utc_now();
    PathBuf::from(format!("{}/replays/{name}-{date}-{time}.replay", env!("CARGO_MANIFEST_DIR")))
}

/// UTC now as "YYYYMMDD-HHMMSS".
pub fn utc_stamp() -> String {
    let (date, time) = utc_now();
    format!("{date}-{time}")
}

/// UTC now as ("YYYYMMDD", "HHMMSS").
fn utc_now() -> (String, String) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    (format!("{y:04}{m:02}{d:02}"), format!("{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60))
}

impl Recorder {
    /// Write the magic line, the header and the snapshot, and start the
    /// event stream. The session's state is not changed (the simulation
    /// absorbs pending changes for its snapshot, as a save does, which is
    /// the same as absorbing them at the next tick).
    pub fn start(s: &mut Session, path: &Path, note: &str) -> Result<Self, String> {
        let world = s.engine.world.clone();
        let sim_bytes = s.sim.snapshot(world.as_ref()).to_bytes();
        let files = save::snapshot_image(&s.engine.edits, &s.engine.config, &s.engine.registry, &s.bake_sha256, &sim_bytes)?;
        let mut digests = ChunkDigests::default();
        let map = digests.all(&s.engine.edits);
        let reg = &s.engine.registry;
        let (date, time) = utc_now();
        let header = Header {
            format: VERSION,
            engine: format!("triangulum {}", env!("CARGO_PKG_VERSION")),
            created_utc: format!("{date}-{time}"),
            note: note.to_string(),
            world: save::Identity::of(&s.engine.config),
            generator_version: save::GENERATOR_VERSION,
            sim_rules: crate::sim::RULES_VERSION,
            bake_sha256: s.bake_sha256.clone(),
            hotbar: s.engine.hotbar.iter().map(|&id| reg.def(id).name.clone()).collect(),
            hide_water: s.engine.hide_water,
            sky_settings: sky_settings_map(),
            clock: [f(s.sky.seconds), f(s.sky.speed)],
            sim_radius_m: f(s.sim.config.radius_m),
            sim_enabled: s.sim.config.enabled,
            water_period: s.sim.config.water_period,
            counters: counters_of(&s.sim.stats),
            steps: s.steps(),
            held: s.held().map(|(b, since)| (button_text(b).to_string(), since)),
            player: pose_text(&s.player),
            start_world: format!("{:016x}", world_digest(&map)),
            start_edits: map.len(),
            save_ref: s.save.as_ref().map(|d| format!("{} generation {}", d.path.display(), d.read_commit().ok().flatten().map_or(0, |c| c.generation))),
            files: files.iter().map(|(n, b)| FileEntry { name: n.clone(), bytes: b.len() as u64, sha256: hex(&sha256(b)) }).collect(),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        let file = std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
        let mut out = std::io::BufWriter::new(file);
        let head = serde_json::to_string(&header).map_err(|e| e.to_string())?;
        let mut snapshot_bytes = 0u64;
        let w = (|| -> std::io::Result<()> {
            writeln!(out, "{MAGIC} {VERSION}")?;
            writeln!(out, "{head}")?;
            for (_, b) in &files {
                out.write_all(b)?;
                snapshot_bytes += b.len() as u64;
            }
            out.flush()
        })();
        w.map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(Self { path: path.to_path_buf(), out: Some(out), start_steps: s.steps(), keys: None, expect: Expect::of(s), digests, prev: map, events: 0, checks: 0, notes: 0, snapshot_bytes, error: None })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Steps recorded so far.
    pub fn steps(&self, s: &Session) -> u64 {
        s.steps() - self.start_steps
    }

    fn line(&mut self, step: u64, text: &str) {
        self.events += 1;
        if let Some(out) = &mut self.out {
            if let Err(e) = writeln!(out, "@{step} {text}") {
                self.error = Some(format!("write {}: {e}", self.path.display()));
                self.out = None;
            }
        }
    }

    fn flush(&mut self) {
        if let Some(out) = &mut self.out {
            if let Err(e) = out.flush() {
                self.error = Some(format!("write {}: {e}", self.path.display()));
                self.out = None;
            }
        }
    }

    /// Write down whatever changed since the recorder last looked that did
    /// not come through `apply` or `step`.
    fn sync(&mut self, s: &Session) {
        let now = Expect::of(s);
        if now == self.expect {
            return;
        }
        let step = self.steps(s);
        let was = self.expect;
        if now.store != was.store {
            self.notes += 1;
            self.line(step, "note the edit store was replaced outside a recorded reload; the replay will diverge here");
        } else if now.edit_gen != was.edit_gen {
            self.notes += 1;
            self.line(step, "note the world changed outside the recorded inputs; the replay will diverge here");
        }
        if now.pose != was.pose {
            self.line(step, &format!("pose {}", pose_text(&s.player)));
        }
        if now.sky != was.sky {
            self.line(step, &action_text(&Action::Sky(s.sky)));
        }
        if now.sim.1 != was.sim.1 {
            self.line(step, &action_text(&Action::Sim(now.sim.1)));
        }
        if now.sim.0 != was.sim.0 {
            self.line(step, &action_text(&Action::SimRadius(s.sim.config.radius_m)));
        }
        if now.sim.2 != was.sim.2 {
            self.line(step, &action_text(&Action::WaterPeriod(now.sim.2)));
        }
        if now.hide != was.hide {
            self.line(step, &action_text(&Action::HideWater(now.hide)));
        }
        self.expect = now;
    }

    pub fn before_action(&mut self, s: &Session, a: &Action) {
        self.sync(s);
        let t = action_text(a);
        self.line(self.steps(s), &t);
    }

    pub fn after(&mut self, s: &Session) {
        self.expect = Expect::of(s);
    }

    pub fn before_step(&mut self, s: &Session, input: &Input, dt: f64) {
        self.sync(s);
        if dt != STEP {
            self.notes += 1;
            self.line(self.steps(s), &format!("note a step of {dt} s, not 1/60: the replay steps 1/60 and will diverge"));
        }
        let k = keys_bits(input);
        if self.keys != Some(k) {
            self.line(self.steps(s), &format!("keys {}", keys_text(input)));
            self.keys = Some(k);
        }
    }

    pub fn after_step(&mut self, s: &Session) {
        self.expect = Expect::of(s);
        if self.steps(s) % CHECK_EVERY == 0 {
            self.check(s);
            self.flush();
        }
    }

    fn check(&mut self, s: &Session) {
        let (mut c, map) = measure(s, &mut self.digests);
        c.chunks = map.iter().filter(|(k, d)| self.prev.get(k) != Some(d)).map(|(k, d)| (*k, *d)).collect();
        self.prev = map;
        self.checks += 1;
        self.line(self.steps(s), &check_text(&c));
    }

    /// A last check, the end line with the full state hash, and close.
    pub fn finish(mut self, s: &mut Session) -> Result<Summary, String> {
        self.sync(s);
        self.check(s);
        let sha = s.sim_hash();
        let steps = self.steps(s);
        self.line(steps, &format!("end sha={sha} steps={steps}"));
        self.flush();
        drop(self.out.take());
        if let Some(e) = self.error {
            return Err(e);
        }
        let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        Ok(Summary { path: self.path, steps, events: self.events, checks: self.checks, notes: self.notes, bytes, snapshot_bytes: self.snapshot_bytes })
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Event {
    Keys(Input),
    Act(Action),
    Check(Check),
    Note(String),
    End { sha: String, steps: u64 },
}

#[derive(Clone, Debug)]
pub struct Line {
    pub step: u64,
    pub event: Event,
    /// Line number in the event section (1 is the first event).
    pub line_no: usize,
    pub text: String,
}

pub struct Replay {
    pub path: PathBuf,
    pub header: Header,
    pub files: Vec<(String, Vec<u8>)>,
    pub lines: Vec<Line>,
    pub ended: bool,
    pub warnings: Vec<String>,
}

impl Replay {
    /// Steps the recording covers (to its end line, or its last event).
    pub fn steps(&self) -> u64 {
        self.lines.last().map_or(0, |l| l.step)
    }
}

pub fn read(path: &Path) -> Result<Replay, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse(&bytes, path)
}

pub fn parse(bytes: &[u8], path: &Path) -> Result<Replay, String> {
    let name = path.display().to_string();
    let mut at = 0usize;
    let next_line = |at: &mut usize| -> Result<String, String> {
        let end = bytes[*at..].iter().position(|&b| b == b'\n').ok_or_else(|| format!("{name}: cut off in its header"))?;
        let l = String::from_utf8_lossy(&bytes[*at..*at + end]).trim_end_matches('\r').to_string();
        *at += end + 1;
        Ok(l)
    };
    let magic = next_line(&mut at)?;
    match magic.split_once(' ') {
        Some((m, v)) if m == MAGIC => {
            if v != VERSION.to_string() {
                return Err(format!("{name}: replay format {v}, this build reads {VERSION}"));
            }
        }
        _ => return Err(format!("{name}: not a Triangulum replay")),
    }
    let head = next_line(&mut at)?;
    let header: Header = serde_json::from_str(&head).map_err(|e| format!("{name}: bad replay header: {e}"))?;
    let mut files = Vec::with_capacity(header.files.len());
    for fe in &header.files {
        let n = fe.bytes as usize;
        let b = bytes.get(at..at + n).ok_or_else(|| format!("{name}: cut off inside its snapshot ({})", fe.name))?;
        if hex(&sha256(b)) != fe.sha256 {
            return Err(format!("{name}: the snapshot file {} is damaged (its SHA-256 does not match)", fe.name));
        }
        files.push((fe.name.clone(), b.to_vec()));
        at += n;
    }
    let text = String::from_utf8_lossy(&bytes[at..]);
    let complete = text.is_empty() || text.ends_with('\n');
    let raw: Vec<&str> = text.split('\n').collect();
    let mut lines = Vec::new();
    let mut warnings = Vec::new();
    let mut ended = false;
    let mut prev = 0u64;
    let block_size = header.world.block_size_m;
    for (n, raw_line) in raw.iter().enumerate() {
        let t = raw_line.trim_end_matches('\r').trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let line_no = n + 1;
        if n == raw.len() - 1 && !complete {
            warnings.push(format!("event line {line_no} is cut off (the recording stopped mid-write); ignored"));
            break;
        }
        if ended {
            warnings.push(format!("event line {line_no} comes after the end line; ignored"));
            continue;
        }
        let (step, event) = parse_line(t, block_size).map_err(|e| format!("{name}: event line {line_no}: {e}: {t}"))?;
        if step < prev {
            return Err(format!("{name}: event line {line_no}: step {step} comes after step {prev}"));
        }
        prev = step;
        if matches!(event, Event::End { .. }) {
            ended = true;
        }
        lines.push(Line { step, event, line_no, text: t.to_string() });
    }
    if !ended {
        warnings.push("the recording has no end line (the game stopped without F10 or quit, or crashed): it plays to its last event".into());
    }
    Ok(Replay { path: path.to_path_buf(), header, files, lines, ended, warnings })
}

fn parse_line(t: &str, block_size: f64) -> Result<(u64, Event), String> {
    let rest = t.strip_prefix('@').ok_or("an event line starts with @STEP")?;
    let (step, body) = rest.split_once(' ').ok_or("nothing after the step")?;
    let step: u64 = step.parse().map_err(|_| format!("bad step {step:?}"))?;
    let mut parts = body.split_whitespace();
    let verb = parts.next().ok_or("empty event")?;
    let args: Vec<&str> = parts.collect();
    let ev = match verb {
        "keys" => Event::Keys(parse_keys(args.first().copied().unwrap_or("-"))?),
        "check" => Event::Check(parse_check(&args)?),
        "note" => Event::Note(body.trim_start_matches("note").trim().to_string()),
        "end" => {
            let mut sha = String::new();
            let mut steps = step;
            for a in &args {
                match a.split_once('=') {
                    Some(("sha", v)) => sha = v.to_string(),
                    Some(("steps", v)) => steps = v.parse().map_err(|_| format!("bad steps {v:?}"))?,
                    _ => {}
                }
            }
            Event::End { sha, steps }
        }
        _ => Event::Act(parse_action(verb, &args, block_size)?),
    };
    Ok((step, ev))
}

// ---------------------------------------------------------------------------
// Playing back
// ---------------------------------------------------------------------------

/// Set the world and the sky to the replay's (main does this before it
/// builds the engine). The bake is whatever `--bake` or world.json names;
/// its identity is checked when the session is built.
pub fn apply_world(h: &Header, config: &mut crate::grid::WorldConfig, sky: &mut crate::sky::Settings) -> Result<(), String> {
    let w = &h.world;
    config.name = w.name.clone();
    config.seed = w.seed;
    config.block_size_m = w.block_size_m;
    config.world_height = w.world_height;
    config.sea_level = w.sea_level;
    config.columns_per_face = w.columns_per_face;
    config.vertical_scale = w.vertical_scale;
    config.roughness = w.roughness;
    config.landform = w.landform;
    *sky = h.sky_settings(sky)?;
    Ok(())
}

/// `game/saves/_replay/<name>/`: where replays play. Wiped before every run.
pub fn scratch_dir(name: &str) -> PathBuf {
    let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    PathBuf::from(format!("{}/saves/_replay/{name}", env!("CARGO_MANIFEST_DIR")))
}

/// Why this replay cannot play in this world, if it cannot.
pub fn compatibility(template: &Session, h: &Header) -> Result<(), String> {
    let want = save::Identity::of(&template.engine.config);
    let have = &h.world;
    let mut bad = Vec::new();
    if have.seed != want.seed {
        bad.push(format!("seed {} vs {}", have.seed, want.seed));
    }
    if have.block_size_m != want.block_size_m {
        bad.push(format!("block size {} vs {}", have.block_size_m, want.block_size_m));
    }
    if have.world_height != want.world_height {
        bad.push(format!("H {} vs {}", have.world_height, want.world_height));
    }
    if have.sea_level != want.sea_level {
        bad.push(format!("sea level {} vs {}", have.sea_level, want.sea_level));
    }
    if have.columns_per_face != want.columns_per_face {
        bad.push(format!("N {} vs {}", have.columns_per_face, want.columns_per_face));
    }
    if have.vertical_scale != want.vertical_scale {
        bad.push(format!("vertical scale {:?} vs {:?}", have.vertical_scale, want.vertical_scale));
    }
    if have.roughness != want.roughness {
        bad.push(format!("roughness {} vs {}", have.roughness, want.roughness));
    }
    if have.landform != want.landform {
        bad.push(format!("landform {} vs {}", have.landform, want.landform));
    }
    if h.generator_version != save::GENERATOR_VERSION {
        bad.push(format!("generator version {} vs {}", h.generator_version, save::GENERATOR_VERSION));
    }
    if h.sim_rules != crate::sim::RULES_VERSION {
        bad.push(format!("simulation rules version {} vs {} (a build with other water and falling rules would not reproduce it; play it with the build that recorded it)", h.sim_rules, crate::sim::RULES_VERSION));
    }
    if h.bake_sha256 != template.bake_sha256 {
        bad.push(format!("bake {}.. vs {}.. (copy the bake it was recorded with, or re-bake)", &h.bake_sha256[..h.bake_sha256.len().min(12)], &template.bake_sha256[..template.bake_sha256.len().min(12)]));
    }
    if bad.is_empty() { Ok(()) } else { Err(format!("this replay was recorded in a different world or under other rules (recorded vs here: {})", bad.join("; "))) }
}

/// A session at the replay's start, playing in `scratch` (wiped first;
/// it must be a `_replay` scratch directory, so a replay can never write
/// into a real save). `template` supplies the engine: the bake, the
/// generator, the registry.
pub fn session_from(template: &Session, rep: &Replay, scratch: &Path) -> Result<Session, String> {
    let h = &rep.header;
    compatibility(template, h)?;
    if !scratch.components().any(|c| c.as_os_str() == "_replay") {
        return Err(format!("refusing to play a replay in {}: replays play only in a saves/_replay scratch directory", scratch.display()));
    }
    if template.save.as_ref().is_some_and(|d| d.path == scratch) {
        return Err(format!("refusing to play a replay in the running session's own save {}", scratch.display()));
    }
    match std::fs::remove_dir_all(scratch) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot clear the replay scratch save {}: {e}", scratch.display())),
    }
    let dir = SaveDir::new(scratch);
    dir.install_image(&rep.files)?;
    let engine = &template.engine;
    let edits = Arc::new(EditStore::new());
    dir.load_into(&engine.config, &engine.registry, &template.bake_sha256, &edits)?;
    let mut e = engine.with_edits(edits);
    e.hide_water = h.hide_water;
    e.hotbar = h.hotbar.iter().map(|n| engine.registry.id(n)).collect::<Result<Vec<_>, _>>().map_err(|err| format!("the replay's hotbar: {err}"))?;
    let engine = Arc::new(e);
    let cfg = SimConfig { radius_m: pf(&h.sim_radius_m)?, enabled: h.sim_enabled, water_period: h.water_period.max(1) };
    let mut sim = Session::load_sim(Some(&dir), cfg, &engine.registry)?;
    restore_counters(&mut sim.stats, &h.counters);
    let player = parse_pose(&h.player, engine.planet.block_size)?;
    let mut s = Session::new(engine.clone(), player, Some(dir), template.bake_sha256.clone(), sim);
    let held = match &h.held {
        Some((b, since)) => Some((parse_button(Some(&b.as_str()))?, *since)),
        None => None,
    };
    s.restore_steps(h.steps, held);
    s.sky = h.clock()?;
    Ok(s)
}

/// Where a replay first stopped matching its recording.
#[derive(Clone, Debug)]
pub struct Divergence {
    pub step: u64,
    pub line_no: usize,
    pub what: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub steps: u64,
    pub events: usize,
    pub checks: usize,
    pub matched: usize,
    pub first: Option<Divergence>,
    /// Checks after the first divergence that did not match either.
    pub later_mismatches: usize,
    pub ended: bool,
    /// (recorded, replayed) full state hashes at the end line.
    pub end_sha: Option<(String, String)>,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.first.is_none() && self.errors.is_empty()
    }

    pub fn text(&self) -> String {
        let mut t = format!("replayed {} steps ({:.1} s), {} events; {} of {} checks matched", self.steps, self.steps as f64 / 60.0, self.events, self.matched, self.checks);
        match &self.end_sha {
            Some((a, b)) if a == b => t.push_str(&format!("; the end state hash matches ({}..)", &a[..a.len().min(16)])),
            Some((a, b)) => t.push_str(&format!("; the end state hash DIFFERS (recorded {}.., replayed {}..)", &a[..a.len().min(16)], &b[..b.len().min(16)])),
            None => t.push_str("; no end line (cut-off recording)"),
        }
        match &self.first {
            None => t.push_str("\nreplay OK: the session reproduced exactly"),
            Some(d) => {
                t.push_str(&format!("\nFIRST DIVERGENCE at step {} ({:.2} s in, event line {}):", d.step, d.step as f64 / 60.0, d.line_no));
                for w in &d.what {
                    t.push_str(&format!("\n  {w}"));
                }
                if self.later_mismatches > 0 {
                    t.push_str(&format!("\n  ({} later checks did not match either)", self.later_mismatches));
                }
            }
        }
        for n in &self.notes {
            t.push_str(&format!("\nnote: {n}"));
        }
        for w in &self.warnings {
            t.push_str(&format!("\nwarning: {w}"));
        }
        for e in &self.errors {
            t.push_str(&format!("\nerror: {e}"));
        }
        t
    }
}

/// Plays a replay into a session one step at a time, checking as it goes.
pub struct Runner<'r> {
    rep: &'r Replay,
    next: usize,
    step: u64,
    keys: Input,
    digests: ChunkDigests,
    recorded: BTreeMap<ChunkKey, u64>,
    done: bool,
    pub report: Report,
}

impl<'r> Runner<'r> {
    /// Start at the replay's beginning; `s` comes from `session_from`.
    pub fn new(rep: &'r Replay, s: &Session) -> Self {
        let mut digests = ChunkDigests::default();
        let map = digests.all(&s.engine.edits);
        let mut report = Report { warnings: rep.warnings.clone(), ..Default::default() };
        let start = format!("{:016x}", world_digest(&map));
        if start != rep.header.start_world || map.len() != rep.header.start_edits {
            report.first = Some(Divergence { step: 0, line_no: 0, what: vec![format!("the snapshot loaded as world {start} with {} edited chunks; recorded {} with {}", map.len(), rep.header.start_world, rep.header.start_edits)] });
        }
        Self { rep, next: 0, step: 0, keys: Input::default(), digests, recorded: map, done: false, report }
    }

    /// Steps played so far.
    pub fn step(&self) -> u64 {
        self.step
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// Apply the events due now (inputs, checks, the end), then run one
    /// step. False when the replay is over (nothing was stepped).
    pub fn advance(&mut self, s: &mut Session) -> bool {
        if self.done {
            return false;
        }
        while let Some(l) = self.rep.lines.get(self.next) {
            if l.step > self.step {
                break;
            }
            self.next += 1;
            self.report.events += 1;
            match &l.event {
                Event::Keys(i) => self.keys = *i,
                Event::Act(a) => {
                    if let Err(e) = s.apply(a.clone()) {
                        self.report.errors.push(format!("step {}: {}: {e}", l.step, l.text));
                    }
                }
                Event::Check(c) => self.verify(s, c, l),
                Event::Note(t) => self.report.notes.push(format!("step {}: {t}", l.step)),
                Event::End { sha, .. } => {
                    let got = s.sim_hash();
                    if *sha != got && self.report.first.is_none() {
                        self.report.first = Some(Divergence { step: l.step, line_no: l.line_no, what: vec![format!("the full state hash (sim_hash) differs: recorded {sha}, replayed {got}")] });
                    }
                    self.report.end_sha = Some((sha.clone(), got));
                    self.report.ended = true;
                    self.finish();
                    return false;
                }
            }
        }
        if self.next >= self.rep.lines.len() {
            self.finish();
            return false;
        }
        s.step(&self.keys, STEP);
        self.step += 1;
        true
    }

    fn finish(&mut self) {
        self.done = true;
        self.report.steps = self.step;
    }

    fn verify(&mut self, s: &Session, rec: &Check, l: &Line) {
        let (got, map) = measure(s, &mut self.digests);
        for (k, d) in &rec.chunks {
            self.recorded.insert(*k, *d);
        }
        self.report.checks += 1;
        let what = compare(rec, &got, &self.recorded, &map);
        if what.is_empty() {
            self.report.matched += 1;
        } else if self.report.first.is_none() {
            self.report.first = Some(Divergence { step: l.step, line_no: l.line_no, what });
        } else {
            self.report.later_mismatches += 1;
        }
    }

    /// Play to the end.
    pub fn run(mut self, s: &mut Session) -> Report {
        while self.advance(s) {}
        self.report
    }
}

#[cfg(test)]
#[path = "replay_tests.rs"]
pub(crate) mod tests;

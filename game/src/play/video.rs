//! Video for the play harness: frames written at a fixed rate while a
//! script runs, smooth camera moves, and the sky clock. The command list is
//! in the header of `play.rs` (section "video"); this module only adds
//! commands and one hook per simulation step, so the simulation commands
//! and the recording stay out of each other's way.
//!
//! Recording pipes raw RGBA frames into ffmpeg (`ffmpeg` on PATH, or the
//! `TRIANGULUM_FFMPEG` environment variable), which writes an H.264 file:
//! nothing but the finished video touches the disk.
//!
//! Every simulation step (60 Hz) while recording is one video tick. At
//! `record FILE 30` every second tick writes a frame. How streaming meets a
//! moving camera is `recmode`:
//!   settle  every written frame is settled first (streaming drained, as a
//!           `shot` is): what the world converges to at that pose. Clean,
//!           slow to record, and it hides pop-in between frames.
//!   live    one streaming update and one render per tick, paced to real
//!           time, so the workers get what a player at 60 fps gives them:
//!           what you would see playing, pop-in included.

use super::Run;
use crate::camera::Camera;
use crate::player::{EYE_M, Frame, Input, Mode, STEP};
use crate::ui::{Hud, draw_hud, outline_segments};
use glam::DVec3;
use std::io::Write as _;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Video {
    enc: Option<Encoder>,
    fps: f64,
    settle: bool,
    hud: bool,
    /// Look-at point for `fly ... target`, `keyrel` and `pan ... target`.
    target: Option<DVec3>,
    keys: Vec<Key>,
    last_tick: Instant,
    /// Settle timeouts and the worst settle, for the transcript.
    settle_timeouts: usize,
    settle_worst_s: f64,
    /// The most blocks in flight any written frame drew (review 3 #13).
    falling_drawn_max: usize,
    /// Vertical field of view of recorded frames, degrees (the game's is 60).
    /// Level of detail follows distance only, so a narrow view at 1 km shows
    /// the coarse world as it is drawn there, magnified.
    fov: f64,
}

impl Default for Video {
    fn default() -> Self {
        Self { enc: None, fps: 30.0, settle: true, hud: false, target: None, keys: Vec::new(), last_tick: Instant::now(), settle_timeouts: 0, settle_worst_s: 0.0, falling_drawn_max: 0, fov: 60.0 }
    }
}

struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    path: String,
    ticks: u64,
    written: u64,
    t0: Instant,
    /// (video seconds, text) cues from `caption`; "" ends the current one.
    captions: Vec<(f64, String)>,
    write_failed: bool,
}

impl Encoder {
    fn finish(&mut self) -> Result<(), String> {
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
        if status.success() { Ok(()) } else { Err(format!("ffmpeg exited with {status} for {}", self.path)) }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

/// One camera key: eye direction, eye altitude above sea level, view angles (radians).
#[derive(Clone, Copy, Debug)]
struct Key {
    t: f64,
    dir: DVec3,
    alt: f64,
    heading: f64,
    pitch: f64,
}

/// Altitude is interpolated in log space, so a fall from orbit to the
/// ground spends comparable time on every decade of altitude.
const ALT_OFFSET: f64 = 200.0;
fn alt_to_s(a: f64) -> f64 {
    (a + ALT_OFFSET).max(1.0).ln()
}
fn s_to_alt(s: f64) -> f64 {
    s.exp() - ALT_OFFSET
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Shortest signed angle from a to b (radians).
fn wrap(d: f64) -> f64 {
    (d + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// Heading and pitch (radians) looking from `eye` toward `target`.
fn look_at(eye: DVec3, target: DVec3) -> (f64, f64) {
    let d = (target - eye).normalize();
    let (north, east, up) = Camera::local_frame(eye);
    (d.dot(east).atan2(d.dot(north)), d.dot(up).clamp(-1.0, 1.0).asin().clamp(-1.55, 1.55))
}

/// Cubic Hermite through (t_i, v_i) with weighted-slope (Catmull-Rom for
/// uneven knots) tangents; the end tangents are zero where `ease` says so.
fn hermite<T>(ts: &[f64], vs: &[T], t: f64, ease_in: bool, ease_out: bool) -> T
where
    T: Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<f64, Output = T>,
{
    let n = ts.len();
    if n == 1 || t <= ts[0] {
        return vs[0];
    }
    if t >= ts[n - 1] {
        return vs[n - 1];
    }
    let slope = |a: usize, b: usize| (vs[b] - vs[a]) * (1.0 / (ts[b] - ts[a]));
    let tangent = |i: usize| -> T {
        if i == 0 {
            if ease_in { vs[0] * 0.0 } else { slope(0, 1) }
        } else if i == n - 1 {
            if ease_out { vs[0] * 0.0 } else { slope(n - 2, n - 1) }
        } else {
            let (h0, h1) = (ts[i] - ts[i - 1], ts[i + 1] - ts[i]);
            (slope(i - 1, i) * h1 + slope(i, i + 1) * h0) * (1.0 / (h0 + h1))
        }
    };
    let i = (0..n - 1).find(|&i| t < ts[i + 1]).unwrap_or(n - 2);
    let h = ts[i + 1] - ts[i];
    let s = (t - ts[i]) / h;
    let (s2, s3) = (s * s, s * s * s);
    let (h00, h10, h01, h11) = (2.0 * s3 - 3.0 * s2 + 1.0, s3 - 2.0 * s2 + s, -2.0 * s3 + 3.0 * s2, s3 - s2);
    vs[i] * h00 + tangent(i) * (h10 * h) + vs[i + 1] * h01 + tangent(i + 1) * (h11 * h)
}

#[derive(Clone, Copy, PartialEq)]
enum LookMode {
    Keys,
    Path,
    Target,
}

impl Run {
    /// The video commands. None: not one of them.
    pub(super) fn video_command(&mut self, cmd: &str, args: &[&str]) -> Option<Result<(), String>> {
        let num = |i: usize| -> Option<f64> { args.get(i).and_then(|s| s.parse::<f64>().ok()) };
        let r = match cmd {
            "record" => match args.first() {
                Some(file) => self.record_start(file, num(1).unwrap_or(30.0)),
                None => Err("record FILE [FPS]".into()),
            },
            "stop" => self.record_stop(),
            "caption" => match &mut self.video.enc {
                Some(enc) => {
                    // "caption +S TEXT" starts S seconds of video from now (inside a long fly).
                    let delay = args.first().and_then(|a| a.strip_prefix('+')).and_then(|d| d.parse::<f64>().ok());
                    let words = if delay.is_some() { &args[1..] } else { args };
                    let text = words.join(" ");
                    let text = if text == "-" { String::new() } else { text };
                    enc.captions.push(((enc.written.max(1) - 1) as f64 / self.video.fps + delay.unwrap_or(0.0), text));
                    Ok(())
                }
                None => Err("caption: not recording".into()),
            },
            "recmode" => match args.first().copied() {
                Some("settle") => {
                    self.video.settle = true;
                    Ok(())
                }
                Some("live") => {
                    self.video.settle = false;
                    Ok(())
                }
                _ => Err("recmode settle|live".into()),
            },
            "hud" => match args.first().copied() {
                Some(v @ ("on" | "off")) => {
                    self.video.hud = v == "on";
                    Ok(())
                }
                _ => Err("hud on|off".into()),
            },
            "settle" => {
                let cam = self.video_camera();
                self.sync_falling();
                let (secs, frames, ok) = self.harness.settle(&cam, Duration::from_secs(240));
                self.say(format!("  settled in {secs:.1} s / {frames} frames{}; {} draws", if ok { "" } else { " TIMEOUT" }, self.harness.streamer.stats.drawn));
                Ok(())
            }
            "skytime" => match args.first().map(|s| crate::sky::parse_hour(s)) {
                Some(Ok(hour)) => {
                    let pos = self.session.player.eye();
                    self.session.sky.seconds = self.harness.gpu.sky.settings.starting_seconds(hour, pos.y.atan2(pos.x));
                    let (elev, az) = self.sun_here();
                    self.say(format!("  local solar time {hour:.3} h: sun {elev:.2} deg above the horizon, azimuth {az:.1}"));
                    Ok(())
                }
                Some(Err(e)) => Err(e),
                None => Err("skytime HH:MM".into()),
            },
            "sun" => match num(0) {
                Some(elev) => {
                    let pm = args.get(1).is_none_or(|s| *s != "am");
                    self.seek_sun(elev, pm)
                }
                None => Err("sun ELEVATION_DEG [am|pm]".into()),
            },
            "skyspeed" => match num(0) {
                Some(v) if (0.0..=100_000.0).contains(&v) => {
                    // The session's clock: it advances on every step (M2c).
                    self.session.sky.speed = v;
                    Ok(())
                }
                _ => Err("skyspeed F (0 holds the sky)".into()),
            },
            "target" => self.set_target(args),
            "key" => match (num(0), num(1), num(2), num(3)) {
                (Some(t), Some(lat), Some(lon), Some(alt)) => {
                    let dir = crate::grid::Planet::dir_of_lat_lon_deg(lat, lon);
                    self.push_key(t, dir, alt, args.get(4).copied(), args.get(5).copied())
                }
                _ => Err("key T LAT LON ALT [HEADING PITCH]".into()),
            },
            "keyrel" => match (num(0), args.get(2), num(3)) {
                (Some(t), Some(bearing), Some(height)) => self.push_key_rel(t, num(1), bearing, height, args.get(4).copied(), args.get(5).copied()),
                _ => Err("keyrel T DIST BEARING HEIGHT [HEADING PITCH]".into()),
            },
            "fly" => self.fly(args),
            "pan" => match (num(0), args.get(1)) {
                (Some(secs), Some(_)) => self.pan(secs, &args[1..]),
                _ => Err("pan SECONDS look H P | turn DH DP | lookaxis AXIS P | aim DI DJ DK | target".into()),
            },
            "fov" => match num(0) {
                Some(d) if (1.0..=120.0).contains(&d) => {
                    self.video.fov = d;
                    Ok(())
                }
                _ => Err("fov DEGREES (1..120; the game's is 60)".into()),
            },
            "zoom" => match (num(0), num(1)) {
                (Some(secs), Some(d)) if (1.0..=120.0).contains(&d) => {
                    // Even steps in log(fov): a zoom that feels like a constant rate.
                    let (f0, n) = (self.video.fov, (secs * 60.0).round().max(1.0) as usize);
                    for step in 1..=n {
                        let s = smooth(step as f64 / n as f64);
                        self.video.fov = (f0.ln() + (d.ln() - f0.ln()) * s).exp();
                        self.session.step(&Input::default(), STEP);
                        self.video_tick();
                    }
                    Ok(())
                }
                _ => Err("zoom SECONDS DEGREES".into()),
            },
            _ => return None,
        };
        Some(r)
    }

    /// Called once per simulation step (the session advanced the sky clock
    /// with it): while recording, renders and writes frames.
    pub(super) fn video_tick(&mut self) {
        let Some(enc) = &mut self.video.enc else { return };
        enc.ticks += 1;
        let due = (enc.ticks as f64 * self.video.fps / 60.0 + 1e-9).floor() as u64 >= enc.written;
        let cam = self.video_camera();
        self.sync_falling();
        if self.video.settle {
            if due {
                self.settle_for_video(&cam);
                self.write_frame(&cam);
            }
        } else {
            // Real-time pacing: a tick takes at least one 60 Hz step of wall time.
            let since = self.video.last_tick.elapsed().as_secs_f64();
            if since < STEP {
                std::thread::sleep(Duration::from_secs_f64(STEP - since));
            }
            self.video.last_tick = Instant::now();
            self.harness.frame(&cam);
            if due {
                self.write_frame(&cam);
            }
        }
    }

    /// The player's camera with the recording's field of view.
    fn video_camera(&self) -> Camera {
        let mut cam = self.session.player.camera();
        cam.fov_y = self.video.fov.to_radians();
        cam
    }

    fn settle_for_video(&mut self, cam: &Camera) {
        let (secs, _, ok) = self.harness.settle(cam, Duration::from_secs(120));
        if !ok {
            self.video.settle_timeouts += 1;
        }
        self.video.settle_worst_s = self.video.settle_worst_s.max(secs);
    }

    fn record_start(&mut self, file: &str, fps: f64) -> Result<(), String> {
        if !(1.0..=60.0).contains(&fps) {
            return Err("record: FPS must be 1..60".into());
        }
        if let Some(mut old) = self.video.enc.take() {
            old.finish()?;
        }
        let path = format!("{}/{file}", self.out);
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let (w, h) = (self.harness.gpu.width, self.harness.gpu.height);
        let exe = std::env::var("TRIANGULUM_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let mut child = Command::new(&exe)
            .args(["-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-framerate", &format!("{fps}"), "-i", "-"])
            .args(["-c:v", "libx264", "-preset", "slow", "-crf", "14", "-pix_fmt", "yuv420p", "-movflags", "+faststart", &path])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("record: cannot start {exe} ({e}); install ffmpeg or set TRIANGULUM_FFMPEG"))?;
        let stdin = child.stdin.take();
        self.video.fps = fps;
        self.video.settle_timeouts = 0;
        self.video.settle_worst_s = 0.0;
        self.video.falling_drawn_max = 0;
        self.video.last_tick = Instant::now();
        self.video.enc = Some(Encoder { child, stdin, path: path.clone(), ticks: 0, written: 0, t0: Instant::now(), captions: Vec::new(), write_failed: false });
        self.say(format!("  recording {w}x{h} at {fps} fps ({}) -> {path}", if self.video.settle { "settled frames" } else { "live, real-time paced" }));
        // The first frame is the pose the recording starts at.
        let cam = self.video_camera();
        self.sync_falling();
        if self.video.settle {
            self.settle_for_video(&cam);
        } else {
            self.harness.settle(&cam, Duration::from_secs(240));
        }
        self.write_frame(&cam);
        Ok(())
    }

    fn record_stop(&mut self) -> Result<(), String> {
        let Some(mut enc) = self.video.enc.take() else { return Err("stop: not recording".into()) };
        enc.finish()?;
        // Captions as [{start, end, text}] next to the video, for the editor.
        let end = enc.written as f64 / self.video.fps;
        enc.captions.sort_by(|a, b| a.0.total_cmp(&b.0));
        let cues: Vec<serde_json::Value> = enc
            .captions
            .iter()
            .enumerate()
            .filter(|(_, (_, text))| !text.is_empty())
            .map(|(n, (start, text))| serde_json::json!({ "start": start, "end": enc.captions.get(n + 1).map_or(end, |c| c.0), "text": text }))
            .collect();
        let cue_path = format!("{}.captions.json", enc.path.trim_end_matches(".mp4"));
        std::fs::write(&cue_path, serde_json::to_string_pretty(&cues).unwrap()).map_err(|e| format!("write {cue_path}: {e}"))?;
        let bytes = std::fs::metadata(&enc.path).map(|m| m.len()).unwrap_or(0);
        let msg = format!(
            "  recorded {} frames ({:.2} s of video) to {} ({:.1} MB) in {:.1} s wall; settle worst {:.2} s, timeouts {}",
            enc.written,
            enc.written as f64 / self.video.fps,
            enc.path,
            bytes as f64 / 1e6,
            enc.t0.elapsed().as_secs_f64(),
            self.video.settle_worst_s,
            self.video.settle_timeouts
        );
        self.say(msg);
        let mut m = match std::mem::take(&mut self.last_stream) {
            serde_json::Value::Object(m) => m,
            _ => Default::default(),
        };
        m.insert("video_frames".into(), enc.written.into());
        m.insert("video_falling_drawn_max".into(), self.video.falling_drawn_max.into());
        self.last_stream = serde_json::Value::Object(m);
        if enc.write_failed {
            return Err(format!("frames were lost: the pipe to ffmpeg broke while writing {}", enc.path));
        }
        if self.video.settle_timeouts > 0 {
            return Err(format!("{} frame(s) did not settle within 120 s", self.video.settle_timeouts));
        }
        Ok(())
    }

    /// Paint the HUD if asked, read the frame back and send it to ffmpeg.
    fn write_frame(&mut self, cam: &Camera) {
        if self.video.hud {
            let hud = self.video_hud(cam);
            self.harness.paint_ui(&|ctx| draw_hud(ctx, &hud, false));
        }
        let rgba = self.harness.gpu.read_rgba();
        let Some(enc) = &mut self.video.enc else { return };
        if let Some(stdin) = &mut enc.stdin {
            if let Err(e) = stdin.write_all(&rgba) {
                let msg = format!("  writing frame {} to ffmpeg failed: {e}", enc.written);
                enc.stdin = None;
                enc.write_failed = true;
                self.say(msg);
                return;
            }
        }
        enc.written += 1;
        self.video.falling_drawn_max = self.video.falling_drawn_max.max(self.harness.falling.len());
    }

    /// The game HUD without the stats panel: crosshair, block outline, hotbar.
    fn video_hud(&self, cam: &Camera) -> Hud {
        let engine = &self.session.engine;
        let (w, h) = (self.harness.gpu.width, self.harness.gpu.height);
        let pick = self.session.pick();
        let p = &self.session.player;
        Hud {
            block: pick.map(|hit| (hit.block, engine.registry.def(hit.id).name.clone(), hit.t)),
            mode: if p.mode == Mode::Walk { "walk".into() } else { "fly".into() },
            hotbar: engine.hotbar.iter().map(|&id| (engine.registry.def(id).name.clone(), engine.registry.def(id).color)).collect(),
            selected: p.selected,
            outline: pick.map_or(Vec::new(), |hit| outline_segments(&engine.planet, cam, w as f64 / h as f64, w, h, hit.face, hit.cell)),
            ..Default::default()
        }
    }

    /// The sun here now: (elevation, azimuth from north), degrees.
    fn sun_here(&self) -> (f64, f64) {
        let sky = &self.harness.gpu.sky;
        let eye = self.session.player.eye();
        let to_sun = (sky.settings.at(self.session.sky.seconds).sun - eye / self.session.engine.planet.radius).normalize();
        let (north, east, up) = Camera::local_frame(eye);
        (to_sun.dot(up).asin().to_degrees(), to_sun.dot(east).atan2(to_sun.dot(north)).to_degrees())
    }

    /// Seek the clock (on the first day) to when the sun stands `elev`
    /// degrees above the horizon here, in the afternoon or the morning.
    fn seek_sun(&mut self, elev: f64, pm: bool) -> Result<(), String> {
        let eye = self.session.player.eye();
        let lon = eye.y.atan2(eye.x);
        let radius = self.session.engine.planet.radius;
        let sky = &self.harness.gpu.sky;
        let at = |hour: f64| {
            let t = sky.settings.starting_seconds(hour, lon);
            let sun = sky.settings.at(t).sun;
            ((sun - eye / radius).normalize().dot(eye.normalize()).asin().to_degrees(), t)
        };
        // The sun falls through the afternoon and rises through the morning.
        let (mut lo, mut hi) = if pm { (12.0, 23.999) } else { (0.0, 12.0) };
        let (e_lo, e_hi) = (at(lo).0, at(hi).0);
        if (elev - e_lo) * (elev - e_hi) > 0.0 {
            return Err(format!("sun: never {elev} deg here in the {} (it runs {e_lo:.1} .. {e_hi:.1})", if pm { "afternoon" } else { "morning" }));
        }
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            let above = at(mid).0 > elev;
            if above == pm {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let hour = 0.5 * (lo + hi);
        let (_, t) = at(hour);
        self.session.sky.seconds = t;
        let (e, az) = self.sun_here();
        self.say(format!("  sun {e:.2} deg, azimuth {az:.1}, at local solar {hour:.3} h (clock {t:.2} s)"));
        Ok(())
    }

    fn set_target(&mut self, args: &[&str]) -> Result<(), String> {
        let num = |i: usize| -> Option<f64> { args.get(i).and_then(|s| s.parse::<f64>().ok()) };
        let planet = self.session.engine.planet;
        let target = match args.first().copied() {
            Some("off") => None,
            Some("rel") => match (num(1), num(2), num(3)) {
                (Some(di), Some(dj), Some(dk)) => {
                    let f = Frame::at(&planet, self.session.player.pos);
                    let (ci, cj, ck) = f.feet_cell();
                    Some(planet.pos(f.face, ci as f64 + 0.5 + di, cj as f64 + 0.5 + dj, ck as f64 + dk))
                }
                _ => return Err("target rel DI DJ DK".into()),
            },
            Some("ll") => match (num(1), num(2), num(3)) {
                (Some(lat), Some(lon), Some(alt)) => Some(crate::grid::Planet::dir_of_lat_lon_deg(lat, lon) * (planet.radius + alt)),
                _ => return Err("target ll LAT LON ALT".into()),
            },
            _ => match (num(0), num(1), num(2), num(3)) {
                (Some(f), Some(i), Some(j), Some(k)) => Some(planet.pos(f as u8, i + 0.5, j + 0.5, k + 0.5)),
                _ => return Err("target FACE I J K | rel DI DJ DK | ll LAT LON ALT | off".into()),
            },
        };
        if let Some(t) = target {
            let (lat, lon) = crate::grid::Planet::lat_lon_deg(t);
            self.say(format!("  target lat {lat:.6} lon {lon:.6} alt {:.2} m", t.length() - planet.radius));
        }
        self.video.target = target;
        Ok(())
    }

    fn parse_angles(&self, heading: Option<&str>, pitch: Option<&str>) -> Result<(f64, f64), String> {
        let prev = self.video.keys.last().map_or((self.session.player.heading, self.session.player.pitch), |k| (k.heading, k.pitch));
        let angle = |s: Option<&str>, prev: f64| -> Result<f64, String> {
            match s {
                None | Some("-") => Ok(prev),
                Some(v) => v.parse::<f64>().map(|d| d.to_radians()).map_err(|_| format!("bad angle {v}")),
            }
        };
        Ok((angle(heading, prev.0)?, angle(pitch, prev.1)?.clamp(-1.55, 1.55)))
    }

    fn push_key(&mut self, t: f64, dir: DVec3, alt: f64, heading: Option<&str>, pitch: Option<&str>) -> Result<(), String> {
        if self.video.keys.last().is_some_and(|k| t <= k.t) || t <= 0.0 {
            return Err("key times must increase from > 0 (0 is the current pose)".into());
        }
        let (heading, pitch) = self.parse_angles(heading, pitch)?;
        self.video.keys.push(Key { t, dir: dir.normalize(), alt, heading, pitch });
        Ok(())
    }

    /// A key DIST metres from the target along BEARING (degrees from north,
    /// or a lattice axis +i -i +j -j at the target), HEIGHT metres above it.
    fn push_key_rel(&mut self, t: f64, dist: Option<f64>, bearing: &str, height: f64, heading: Option<&str>, pitch: Option<&str>) -> Result<(), String> {
        let Some(target) = self.video.target else { return Err("keyrel needs a target".into()) };
        let Some(dist) = dist else { return Err("keyrel T DIST BEARING HEIGHT".into()) };
        let engine = self.session.engine.clone();
        let tdir = target.normalize();
        let bearing_deg = match bearing {
            "+i" | "-i" | "+j" | "-j" => {
                let f = Frame::at(&engine.planet, target);
                let v = match bearing {
                    "+i" => f.ei,
                    "-i" => -f.ei,
                    "+j" => f.ej,
                    _ => -f.ej,
                };
                let (north, east, _) = Camera::local_frame(target);
                v.dot(east).atan2(v.dot(north)).to_degrees()
            }
            b => b.parse::<f64>().map_err(|_| format!("bad bearing {b}"))?,
        };
        let dir = engine.offset_dir(tdir, bearing_deg, dist);
        let alt = target.length() - engine.planet.radius + height;
        self.push_key(t, dir, alt, heading, pitch)
    }

    /// Play the keys from the current pose: `fly [ease|in|out|none] [keys|path|target]`.
    fn fly(&mut self, args: &[&str]) -> Result<(), String> {
        let (mut ease_in, mut ease_out, mut mode) = (true, true, LookMode::Keys);
        for a in args {
            match *a {
                "ease" => (ease_in, ease_out) = (true, true),
                "in" => (ease_in, ease_out) = (true, false),
                "out" => (ease_in, ease_out) = (false, true),
                "none" => (ease_in, ease_out) = (false, false),
                "keys" => mode = LookMode::Keys,
                "path" => mode = LookMode::Path,
                "target" => mode = LookMode::Target,
                other => return Err(format!("fly: unknown option {other}")),
            }
        }
        if self.video.keys.is_empty() {
            return Err("fly: no keys".into());
        }
        if mode == LookMode::Target && self.video.target.is_none() {
            return Err("fly target: no target set".into());
        }
        let planet = self.session.engine.planet;
        let eye = self.session.player.eye();
        let p = &self.session.player;
        let mut keys = vec![Key { t: 0.0, dir: eye.normalize(), alt: eye.length() - planet.radius, heading: p.heading, pitch: p.pitch }];
        keys.append(&mut self.video.keys);
        // Unwrap headings so each key turns the short way from the last.
        for i in 1..keys.len() {
            keys[i].heading = keys[i - 1].heading + wrap(keys[i].heading - keys[i - 1].heading);
        }
        let ts: Vec<f64> = keys.iter().map(|k| k.t).collect();
        let dirs: Vec<DVec3> = keys.iter().map(|k| k.dir).collect();
        let alts: Vec<f64> = keys.iter().map(|k| alt_to_s(k.alt)).collect();
        let heads: Vec<f64> = keys.iter().map(|k| k.heading).collect();
        let pitches: Vec<f64> = keys.iter().map(|k| k.pitch).collect();
        let end = *ts.last().unwrap();
        let pos_at = |t: f64| -> DVec3 {
            let d = hermite(&ts, &dirs, t, ease_in, ease_out).normalize();
            d * (planet.radius + s_to_alt(hermite(&ts, &alts, t, ease_in, ease_out)))
        };
        let n = (end * 60.0).round() as usize;
        let engine = self.session.engine.clone();
        let _ = self.session.apply(crate::session::Action::SetMode(Mode::Fly));
        let mut min_agl = f64::MAX;
        let mut prev_path_heading = self.session.player.heading;
        for step in 1..=n {
            let t = step as f64 / 60.0;
            let eye = pos_at(t);
            let (heading, pitch) = match mode {
                LookMode::Keys => (hermite(&ts, &heads, t, ease_in, ease_out), hermite(&ts, &pitches, t, ease_in, ease_out)),
                LookMode::Target => look_at(eye, self.video.target.unwrap()),
                LookMode::Path => {
                    // Along the horizontal motion, looking a little ahead (or
                    // behind at the very end, where the motion stops).
                    let (a, b) = if t + 0.25 <= end { (eye, pos_at(t + 0.25)) } else { (pos_at(t - 0.25), eye) };
                    let (north, east, _) = Camera::local_frame(eye);
                    let d = b - a;
                    let h = if (d - eye.normalize() * d.dot(eye.normalize())).length() > 1e-3 { d.dot(east).atan2(d.dot(north)) } else { prev_path_heading };
                    prev_path_heading = h;
                    (h, hermite(&ts, &pitches, t, ease_in, ease_out))
                }
            };
            let up = eye.normalize();
            min_agl = min_agl.min(eye.length() - planet.radius - engine.ground(up));
            // The camera is keyed, but the world goes on: every frame is one
            // step of the session (the simulation ticks, the sky turns),
            // as for `hold`, `pan` and `zoom` (review 3 #13). The pose is
            // pinned again after the step so the path is exactly the keys'.
            let pin = |pl: &mut crate::player::Player| {
                pl.pos = eye - up * EYE_M;
                pl.vel = DVec3::ZERO;
                pl.heading = heading;
                pl.pitch = pitch.clamp(-1.55, 1.55);
            };
            pin(&mut self.session.player);
            self.session.step(&Input::default(), STEP);
            pin(&mut self.session.player);
            self.video_tick();
        }
        let (lat, lon) = crate::grid::Planet::lat_lon_deg(self.session.player.eye());
        self.say(format!("  flew {} keys over {end:.2} s; lowest eye {min_agl:.1} m above the ground; now at lat {lat:.6} lon {lon:.6} alt {:.1} m", ts.len() - 1, self.session.player.eye().length() - planet.radius));
        if min_agl < 1.0 {
            return Err(format!("fly: the eye came within {min_agl:.1} m of the ground (below 1 m)"));
        }
        Ok(())
    }

    /// Turn the view smoothly to where a look command would point it,
    /// stepping the simulation with no input meanwhile.
    fn pan(&mut self, secs: f64, cmd: &[&str]) -> Result<(), String> {
        let num = |i: usize| -> Option<f64> { cmd.get(i).and_then(|s| s.parse::<f64>().ok()) };
        let planet = self.session.engine.planet;
        let p = &self.session.player;
        let (h0, p0) = (p.heading, p.pitch);
        let (h1, p1) = match cmd[0] {
            "look" => match (num(1), num(2)) {
                (Some(h), Some(pt)) => (h.to_radians(), pt.to_radians()),
                _ => return Err("pan S look HEADING PITCH".into()),
            },
            "turn" => match (num(1), num(2)) {
                (Some(dh), Some(dp)) => (h0 + dh.to_radians(), p0 + dp.to_radians()),
                _ => return Err("pan S turn DH DP".into()),
            },
            "lookaxis" => match (cmd.get(1).copied(), num(2)) {
                (Some(axis), Some(pt)) => {
                    let f = Frame::at(&planet, p.pos);
                    let v = match axis {
                        "+i" => f.ei,
                        "-i" => -f.ei,
                        "+j" => f.ej,
                        "-j" => -f.ej,
                        _ => return Err("pan S lookaxis +i|-i|+j|-j PITCH".into()),
                    };
                    let (north, east, _) = Camera::local_frame(p.pos);
                    (v.dot(east).atan2(v.dot(north)), pt.to_radians())
                }
                _ => return Err("pan S lookaxis AXIS PITCH".into()),
            },
            "aim" => match (num(1), num(2), num(3)) {
                // As the `aim` command: the lattice point (feet cell centre + DI, + DJ, feet layer + DK).
                (Some(di), Some(dj), Some(dk)) => {
                    let f = Frame::at(&planet, p.pos);
                    let (ci, cj, ck) = f.feet_cell();
                    look_at(p.eye(), planet.pos(f.face, ci as f64 + 0.5 + di, cj as f64 + 0.5 + dj, ck as f64 + dk))
                }
                _ => return Err("pan S aim DI DJ DK".into()),
            },
            "target" => match self.video.target {
                Some(t) => look_at(p.eye(), t),
                None => return Err("pan S target: no target set".into()),
            },
            other => return Err(format!("pan: cannot pan with {other}")),
        };
        let dh = wrap(h1 - h0);
        let p1 = p1.clamp(-1.55, 1.55);
        let n = (secs * 60.0).round().max(1.0) as usize;
        for step in 1..=n {
            let s = smooth(step as f64 / n as f64);
            self.session.player.heading = h0 + dh * s;
            self.session.player.pitch = p0 + (p1 - p0) * s;
            self.session.step(&Input::default(), STEP);
            self.video_tick();
        }
        Ok(())
    }
}

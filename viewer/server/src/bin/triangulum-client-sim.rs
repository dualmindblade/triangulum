//! Scriptable headless MP1 client used for localhost integration evidence.
//! It exercises the real WebSocket/serde protocol and clock-slew law without
//! linking the viewer, wgpu, or winit.

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use triangulum_multiplayer::{
    BodyId, BodyPose, ClockSlew, ClockState, EditJournal, EditRequest, Hello, Message,
    PROTOCOL_VERSION, PlayerMode, clean_player_name, load_world_identity, parse_invite,
};

struct Args {
    url: String,
    name: String,
    assets: PathBuf,
    build_hash: String,
    protocol_version: u32,
    duration_ms: u64,
    sample_server_ms: Option<u64>,
    local_time_s: f64,
    pose: BodyPose,
    edit: Option<EditRequest>,
    edit_delay_ms: u64,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut out = Self {
            url: String::new(),
            name: "ClientSim".into(),
            assets: default_assets_dir(),
            build_hash: option_env!("TRI_BUILD").unwrap_or("unstamped").into(),
            protocol_version: PROTOCOL_VERSION,
            duration_ms: 5_000,
            sample_server_ms: None,
            local_time_s: 0.0,
            pose: BodyPose {
                body: BodyId::Neisor,
                lat_deg: 10.0,
                lon_deg: 30.0,
                alt_km: 0.01,
                yaw_deg: 0.0,
                pitch_deg: 0.0,
                roll_deg: 0.0,
                mode: PlayerMode::Fly,
            },
            edit: None,
            edit_delay_ms: 1_000,
        };
        let argv = std::env::args().collect::<Vec<_>>();
        let mut edit_parts: Option<(BodyId, u8, u64, u64, i64)> = None;
        let mut i = 1;
        while i < argv.len() {
            let next = || argv.get(i + 1).cloned().unwrap_or_default();
            match argv[i].as_str() {
                "--url" => {
                    out.url = next();
                    i += 1;
                }
                "--name" => {
                    out.name = clean_player_name(&next());
                    i += 1;
                }
                "--assets" => {
                    out.assets = next().into();
                    i += 1;
                }
                "--build-hash" => {
                    out.build_hash = next();
                    i += 1;
                }
                "--protocol-version" => {
                    out.protocol_version = next().parse()?;
                    i += 1;
                }
                "--duration-ms" => {
                    out.duration_ms = next().parse()?;
                    i += 1;
                }
                "--sample-server-ms" => {
                    out.sample_server_ms = Some(next().parse()?);
                    i += 1;
                }
                "--local-time" => {
                    out.local_time_s = next().parse()?;
                    i += 1;
                }
                "--body" => {
                    out.pose.body = parse_body(&next())?;
                    i += 1;
                }
                "--lat" => {
                    out.pose.lat_deg = next().parse()?;
                    i += 1;
                }
                "--lon" => {
                    out.pose.lon_deg = next().parse()?;
                    i += 1;
                }
                "--alt" => {
                    out.pose.alt_km = next().parse()?;
                    i += 1;
                }
                "--yaw" => {
                    out.pose.yaw_deg = next().parse()?;
                    i += 1;
                }
                "--pitch" => {
                    out.pose.pitch_deg = next().parse()?;
                    i += 1;
                }
                "--roll" => {
                    out.pose.roll_deg = next().parse()?;
                    i += 1;
                }
                "--mode" => {
                    out.pose.mode = match next().as_str() {
                        "fly" => PlayerMode::Fly,
                        "walk" => PlayerMode::Walk,
                        other => bail!("unknown mode {other}"),
                    };
                    i += 1;
                }
                "--edit" => {
                    let body = parse_body(&next())?;
                    let face: u8 = argv
                        .get(i + 2)
                        .context("--edit BODY FACE CI CJ VALUE")?
                        .parse()?;
                    let ci: u64 = argv
                        .get(i + 3)
                        .context("--edit BODY FACE CI CJ VALUE")?
                        .parse()?;
                    let cj: u64 = argv
                        .get(i + 4)
                        .context("--edit BODY FACE CI CJ VALUE")?
                        .parse()?;
                    let value: i64 = argv
                        .get(i + 5)
                        .context("--edit BODY FACE CI CJ VALUE")?
                        .parse()?;
                    edit_parts = Some((body, face, ci, cj, value));
                    i += 5;
                }
                "--edit-delay-ms" => {
                    out.edit_delay_ms = next().parse()?;
                    i += 1;
                }
                "--help" | "-h" => {
                    println!(
                        "triangulum-client-sim --url INVITE [--name NAME] [--assets DIR] [--build-hash HASH] [--protocol-version N] [--duration-ms N] [--sample-server-ms N] [--body neisor|moon --lat D --lon D --alt KM --yaw D --pitch D --roll D --mode fly|walk] [--edit BODY FACE CI CJ VALUE --edit-delay-ms N]"
                    );
                    std::process::exit(0);
                }
                unknown => bail!("unknown argument: {unknown}"),
            }
            i += 1;
        }
        if out.url.is_empty() {
            bail!("--url is required");
        }
        if !out.pose.is_valid() {
            bail!("presence pose is invalid");
        }
        out.edit = edit_parts.map(|(body, face, ci, cj, value)| EditRequest {
            body,
            face,
            ci,
            cj,
            value,
        });
        if let Some(edit) = &out.edit {
            edit.validate().map_err(anyhow::Error::msg)?;
        }
        Ok(out)
    }
}

fn default_assets_dir() -> PathBuf {
    if Path::new("viewer/assets/meta.json").exists() {
        "viewer/assets".into()
    } else {
        "assets".into()
    }
}

fn parse_body(value: &str) -> Result<BodyId> {
    match value.to_ascii_lowercase().as_str() {
        "neisor" => Ok(BodyId::Neisor),
        "moon" => Ok(BodyId::Moon),
        "sun" => Ok(BodyId::Sun),
        _ => bail!("unknown body {value}"),
    }
}

async fn send_wire<S>(sink: &mut S, message: &Message) -> Result<()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    sink.send(WsMessage::Text(serde_json::to_string(message)?.into()))
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse()?;
    let invite = parse_invite(&args.url)?;
    let identity = load_world_identity(&args.assets, args.build_hash.clone())?;
    // wss:// invites (tunneled servers) need a process-level rustls
    // provider; installing twice is a benign error.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (websocket, _) = tokio_tungstenite::connect_async(invite.websocket_url.as_str())
        .await
        .with_context(|| format!("connect {}", invite.websocket_url))?;
    let (mut sink, mut incoming) = websocket.split();
    send_wire(
        &mut sink,
        &Message::Hello(Hello {
            token: invite.token,
            protocol_version: args.protocol_version,
            build_hash: identity.build_hash.clone(),
            seed: identity.seed,
            asset_hashes: identity.asset_hashes.clone(),
            name: args.name.clone(),
        }),
    )
    .await?;

    let started = tokio::time::Instant::now();
    let deadline = started + Duration::from_millis(args.duration_ms);
    let mut presence_tick = tokio::time::interval(Duration::from_millis(67));
    presence_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut players = BTreeMap::new();
    let mut welcome_clock: Option<ClockState> = None;
    let mut welcome_at: Option<tokio::time::Instant> = None;
    let mut slew: Option<ClockSlew> = None;
    let mut edit_sent = false;
    let mut sampled = false;
    let mut own_id = None;
    let mut journal = EditJournal::default();

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            _ = presence_tick.tick(), if own_id.is_some() => {
                send_wire(&mut sink, &Message::PresenceUpdate(args.pose.clone())).await?;
                if !edit_sent && started.elapsed() >= Duration::from_millis(args.edit_delay_ms)
                    && let Some(edit) = &args.edit
                {
                    send_wire(&mut sink, &Message::EditRequest(edit.clone())).await?;
                    println!("EDIT_SENT name={} body={:?} face={} ci={} cj={} value={}", args.name, edit.body, edit.face, edit.ci, edit.cj, edit.value);
                    edit_sent = true;
                }
                if !sampled
                    && let (Some(target_mono), Some(clock), Some(at), Some(slew)) = (args.sample_server_ms, welcome_clock.as_ref(), welcome_at, slew.as_ref())
                {
                    let estimated_mono = clock.server_mono_ms.saturating_add(at.elapsed().as_millis() as u64);
                    if estimated_mono >= target_mono {
                        let elapsed_at_target = target_mono.saturating_sub(clock.server_mono_ms) as f64 * 0.001;
                        let display = slew.sample(elapsed_at_target);
                        let canonical = clock.at_server_mono_ms(target_mono);
                        println!("CLOCK_SAMPLE name={} server_mono_ms={} display_s={:.6} canonical_s={:.6} error_ms={:.3}", args.name, target_mono, display, canonical, (display-canonical).abs()*1000.0);
                        sampled = true;
                    }
                }
            }
            frame = incoming.next() => {
                let Some(frame) = frame else { break };
                let frame = frame?;
                let message: Message = match frame {
                    WsMessage::Text(text) => serde_json::from_str(&text)?,
                    WsMessage::Binary(bytes) => serde_json::from_slice(&bytes)?,
                    WsMessage::Ping(payload) => { sink.send(WsMessage::Pong(payload)).await?; continue; }
                    WsMessage::Pong(_) => continue,
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                match message {
                    Message::Welcome(welcome) => {
                        own_id = Some(welcome.player_id);
                        for player in welcome.players { players.insert(player.id, player.name); }
                        for record in welcome.edit_journal {
                            journal.apply_record(record)?;
                        }
                        let at = tokio::time::Instant::now();
                        println!("WELCOME name={} id={} server_time_s={:.6} server_mono_ms={} scale={} journal_records={} players={}", args.name, welcome.player_id, welcome.clock.absolute_time_s, welcome.clock.server_mono_ms, welcome.clock.time_scale, journal.records().len(), players.len());
                        slew = Some(ClockSlew::new(args.local_time_s, &welcome.clock, 2.0));
                        welcome_clock = Some(welcome.clock);
                        welcome_at = Some(at);
                    }
                    Message::Refusal(refusal) => {
                        eprintln!("REFUSED name={} code={} message={}", args.name, refusal.code, refusal.message);
                        bail!("server refused handshake: {}", refusal.message);
                    }
                    Message::PlayerJoined(player) => {
                        println!("PLAYER_JOINED observer={} id={} name={}", args.name, player.id, player.name);
                        players.insert(player.id, player.name);
                    }
                    Message::PlayerLeft { player_id } => {
                        println!("PLAYER_LEFT observer={} id={} name={}", args.name, player_id, players.remove(&player_id).unwrap_or_default());
                    }
                    Message::Presence { player_id, pose } => {
                        println!("PRESENCE observer={} from_id={} from_name={} body={:?} lat_deg={:.6} lon_deg={:.6} alt_km={:.6} yaw_deg={:.3} pitch_deg={:.3} roll_deg={:.3} mode={:?}", args.name, player_id, players.get(&player_id).map(String::as_str).unwrap_or("?"), pose.body, pose.lat_deg, pose.lon_deg, pose.alt_km, pose.yaw_deg, pose.pitch_deg, pose.roll_deg, pose.mode);
                    }
                    Message::Edit(record) => {
                        journal.apply_record(record.clone())?;
                        println!("EDIT_APPLIED observer={} seq={} body={:?} face={} ci={} cj={} value={} journal_records={}", args.name, record.sequence, record.edit.body, record.edit.face, record.edit.ci, record.edit.cj, record.edit.value, journal.records().len());
                    }
                    Message::Ping { nonce } => send_wire(&mut sink, &Message::Pong { nonce, clock: welcome_clock.clone().unwrap_or(triangulum_multiplayer::ClockState { sequence: 0, absolute_time_s: 0.0, time_scale: 1.0, server_mono_ms: 0 }) }).await?,
                    Message::Pong { nonce, clock } => println!("PONG name={} nonce={} server_time_s={:.6} server_mono_ms={}", args.name, nonce, clock.absolute_time_s, clock.server_mono_ms),
                    Message::ClockEvent(event) => {
                        let local = welcome_at.zip(slew.as_ref()).map_or(args.local_time_s, |(at, old)| old.sample(at.elapsed().as_secs_f64()));
                        println!("CLOCK_EVENT name={} kind={:?} seq={} absolute_s={:.6} scale={}", args.name, event.kind, event.state.sequence, event.state.absolute_time_s, event.state.time_scale);
                        welcome_at = Some(tokio::time::Instant::now());
                        slew = Some(ClockSlew::new(local, &event.state, 2.0));
                        welcome_clock = Some(event.state);
                    }
                    Message::Error { code, message } => eprintln!("SERVER_ERROR name={} code={} message={}", args.name, code, message),
                    Message::Hello(_) | Message::EditRequest(_) | Message::PresenceUpdate(_) | Message::ClockCommand(_) => {}
                }
            }
        }
    }
    if args.sample_server_ms.is_some() && !sampled {
        bail!("clock sample target was not reached");
    }
    println!(
        "CLIENT_DONE name={} id={} elapsed_ms={}",
        args.name,
        own_id.unwrap_or(0),
        started.elapsed().as_millis()
    );
    let _ = sink.send(WsMessage::Close(None)).await;
    Ok(())
}

//! Non-blocking viewer bridge for MP1. Tokio and WebSocket work stay on one
//! background thread; the winit/render thread sees only std channels and
//! drains them with `try_recv` once per frame.

use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use triangulum_multiplayer::{
    BodyPose, ClockState, Hello, Invite, Message, PROTOCOL_VERSION, PlayerId, PlayerInfo,
    WorldIdentity, parse_invite,
};

#[derive(Clone, Debug)]
pub enum ClientCommand {
    Connect {
        invite: String,
        name: String,
        identity: WorldIdentity,
    },
    Send(Message),
    Disconnect,
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum ClientEvent {
    Connecting(String),
    Message(Message),
    Refused { code: String, message: String },
    Disconnected(String),
}

pub struct NetworkClient {
    commands: mpsc::Sender<ClientCommand>,
    events: mpsc::Receiver<ClientEvent>,
}

impl NetworkClient {
    pub fn spawn() -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("triangulum-net".into())
            .spawn(move || network_thread(command_rx, event_tx))
            .expect("spawn multiplayer network thread");
        Self { commands, events }
    }

    pub fn connect(
        &self,
        invite: String,
        name: String,
        identity: WorldIdentity,
    ) -> Result<(), String> {
        self.commands
            .send(ClientCommand::Connect {
                invite,
                name,
                identity,
            })
            .map_err(|e| e.to_string())
    }

    pub fn send(&self, message: Message) -> Result<(), String> {
        self.commands
            .send(ClientCommand::Send(message))
            .map_err(|e| e.to_string())
    }

    pub fn disconnect(&self) {
        let _ = self.commands.send(ClientCommand::Disconnect);
    }

    pub fn try_recv(&self) -> Option<ClientEvent> {
        self.events.try_recv().ok()
    }
}

impl Drop for NetworkClient {
    fn drop(&mut self) {
        let _ = self.commands.send(ClientCommand::Shutdown);
    }
}

fn network_thread(commands: mpsc::Receiver<ClientCommand>, events: mpsc::Sender<ClientEvent>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = events.send(ClientEvent::Disconnected(format!(
                "could not start Tokio: {error}"
            )));
            return;
        }
    };
    while let Ok(command) = commands.recv() {
        match command {
            ClientCommand::Connect {
                invite,
                name,
                identity,
            } => {
                let _ = events.send(ClientEvent::Connecting(invite.clone()));
                let result =
                    runtime.block_on(run_connection(&commands, &events, &invite, &name, identity));
                if let Err(error) = result {
                    let _ = events.send(ClientEvent::Disconnected(error));
                }
            }
            ClientCommand::Shutdown => break,
            ClientCommand::Disconnect | ClientCommand::Send(_) => {}
        }
    }
}

async fn send_wire<S>(sink: &mut S, message: &Message) -> Result<(), String>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::fmt::Display,
{
    let json = serde_json::to_string(message).map_err(|e| e.to_string())?;
    sink.send(WsMessage::Text(json.into()))
        .await
        .map_err(|e| e.to_string())
}

async fn run_connection(
    commands: &mpsc::Receiver<ClientCommand>,
    events: &mpsc::Sender<ClientEvent>,
    raw_invite: &str,
    name: &str,
    identity: WorldIdentity,
) -> Result<(), String> {
    let Invite {
        websocket_url,
        token,
    } = parse_invite(raw_invite).map_err(|e| e.to_string())?;
    let (websocket, _) = tokio_tungstenite::connect_async(websocket_url.as_str())
        .await
        .map_err(|e| format!("connect {websocket_url}: {e}"))?;
    let (mut sink, mut incoming) = websocket.split();
    send_wire(
        &mut sink,
        &Message::Hello(Hello {
            token,
            protocol_version: PROTOCOL_VERSION,
            build_hash: identity.build_hash.clone(),
            seed: identity.seed,
            asset_hashes: identity.asset_hashes.clone(),
            name: name.to_string(),
        }),
    )
    .await?;
    let mut command_tick = tokio::time::interval(Duration::from_millis(8));
    command_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut keepalive = tokio::time::interval(Duration::from_secs(10));
    let mut last_clock = ClockState {
        sequence: 0,
        absolute_time_s: 0.0,
        time_scale: 1.0,
        server_mono_ms: 0,
    };
    let mut last_inbound = Instant::now();
    let mut nonce = 1u64;
    loop {
        tokio::select! {
            _ = command_tick.tick() => {
                loop {
                    match commands.try_recv() {
                        Ok(ClientCommand::Send(message)) => send_wire(&mut sink, &message).await?,
                        Ok(ClientCommand::Disconnect) => {
                            let _ = sink.send(WsMessage::Close(None)).await;
                            return Ok(());
                        }
                        Ok(ClientCommand::Shutdown) => {
                            let _ = sink.send(WsMessage::Close(None)).await;
                            return Err("network client shut down".into());
                        }
                        Ok(ClientCommand::Connect { .. }) => return Err("a second connect was requested while already connected".into()),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return Err("viewer network channel closed".into()),
                    }
                }
            }
            _ = keepalive.tick() => {
                if last_inbound.elapsed() > Duration::from_secs(35) { return Err("server keepalive timed out".into()); }
                nonce = nonce.wrapping_add(1);
                send_wire(&mut sink, &Message::Ping { nonce }).await?;
            }
            frame = incoming.next() => {
                let Some(frame) = frame else { return Err("server closed the WebSocket".into()); };
                let frame = frame.map_err(|e| e.to_string())?;
                last_inbound = Instant::now();
                let message: Message = match frame {
                    WsMessage::Text(text) => serde_json::from_str(&text).map_err(|e| format!("protocol JSON: {e}"))?,
                    WsMessage::Binary(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("protocol JSON: {e}"))?,
                    WsMessage::Ping(payload) => { sink.send(WsMessage::Pong(payload)).await.map_err(|e| e.to_string())?; continue; }
                    WsMessage::Pong(_) => continue,
                    WsMessage::Close(frame) => return Err(format!("server closed connection: {frame:?}")),
                    _ => continue,
                };
                match &message {
                    Message::Welcome(welcome) => last_clock = welcome.clock.clone(),
                    Message::ClockEvent(event) => last_clock = event.state.clone(),
                    Message::Pong { clock, .. } => last_clock = clock.clone(),
                    Message::Ping { nonce } => {
                        send_wire(&mut sink, &Message::Pong { nonce: *nonce, clock: last_clock.clone() }).await?;
                        continue;
                    }
                    Message::Refusal(refusal) => {
                        let _ = events.send(ClientEvent::Refused { code: refusal.code.clone(), message: refusal.message.clone() });
                        return Ok(());
                    }
                    _ => {}
                }
                if events.send(ClientEvent::Message(message)).is_err() { return Err("viewer event receiver closed".into()); }
            }
        }
    }
}

#[derive(Clone)]
struct PoseSample {
    pose: BodyPose,
    at: Instant,
}

pub struct RemotePlayer {
    pub info: PlayerInfo,
    from: Option<PoseSample>,
    to: Option<PoseSample>,
}

impl RemotePlayer {
    fn new(info: PlayerInfo) -> Self {
        let sample = info.pose.clone().map(|pose| PoseSample {
            pose,
            at: Instant::now(),
        });
        Self {
            info,
            from: sample.clone(),
            to: sample,
        }
    }

    fn update_pose(&mut self, pose: BodyPose) {
        let now = Instant::now();
        let current = self.sample(now).unwrap_or_else(|| pose.clone());
        self.info.pose = Some(pose.clone());
        self.from = Some(PoseSample {
            pose: current,
            at: now,
        });
        self.to = Some(PoseSample {
            pose,
            at: now + Duration::from_millis(100),
        });
    }

    pub fn sample(&self, now: Instant) -> Option<BodyPose> {
        let (Some(from), Some(to)) = (&self.from, &self.to) else {
            return self.info.pose.clone();
        };
        if from.pose.body != to.pose.body {
            return Some(to.pose.clone());
        }
        let span = to.at.saturating_duration_since(from.at).as_secs_f64();
        let t = if span <= 0.0 {
            1.0
        } else {
            now.saturating_duration_since(from.at).as_secs_f64() / span
        }
        .clamp(0.0, 1.0);
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        let angle = |a: f64, b: f64| {
            let delta = (b - a + 180.0).rem_euclid(360.0) - 180.0;
            a + delta * t
        };
        Some(BodyPose {
            body: to.pose.body,
            lat_deg: lerp(from.pose.lat_deg, to.pose.lat_deg),
            lon_deg: angle(from.pose.lon_deg, to.pose.lon_deg),
            alt_km: lerp(from.pose.alt_km, to.pose.alt_km),
            yaw_deg: angle(from.pose.yaw_deg, to.pose.yaw_deg),
            pitch_deg: angle(from.pose.pitch_deg, to.pose.pitch_deg),
            roll_deg: angle(from.pose.roll_deg, to.pose.roll_deg),
            mode: to.pose.mode,
        })
    }
}

#[derive(Default)]
pub struct RemotePlayers {
    pub own_id: Option<PlayerId>,
    players: HashMap<PlayerId, RemotePlayer>,
}

impl RemotePlayers {
    pub fn reset(&mut self, own_id: PlayerId, players: Vec<PlayerInfo>) {
        self.own_id = Some(own_id);
        self.players = players
            .into_iter()
            .filter(|player| player.id != own_id)
            .map(|player| (player.id, RemotePlayer::new(player)))
            .collect();
    }
    pub fn join(&mut self, player: PlayerInfo) {
        if Some(player.id) != self.own_id {
            self.players.insert(player.id, RemotePlayer::new(player));
        }
    }
    pub fn leave(&mut self, id: PlayerId) {
        self.players.remove(&id);
    }
    pub fn presence(&mut self, id: PlayerId, pose: BodyPose) {
        if Some(id) == self.own_id {
            return;
        }
        self.players
            .entry(id)
            .or_insert_with(|| {
                RemotePlayer::new(PlayerInfo {
                    id,
                    name: format!("Player {id}"),
                    tint: triangulum_multiplayer::player_tint(id),
                    pose: None,
                })
            })
            .update_pose(pose);
    }
    pub fn iter(&self) -> impl Iterator<Item = (&PlayerId, &RemotePlayer)> {
        self.players.iter()
    }
    pub fn clear(&mut self) {
        self.own_id = None;
        self.players.clear();
    }
}

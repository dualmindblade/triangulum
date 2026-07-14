use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use triangulum_multiplayer::{
    AuthoritativeClock, BodyId, ClockCommand, EditRequest, Hello, JournalStore, Message,
    PROTOCOL_VERSION, PlayerId, PlayerInfo, Refusal, Welcome, WorldIdentity, clean_player_name,
    load_legacy_edits, load_world_identity, player_tint, write_legacy_edits,
};

#[derive(Debug)]
struct Args {
    bind: String,
    public_host: Option<String>,
    public_url: Option<String>,
    token: Option<String>,
    assets: PathBuf,
    journal: Option<PathBuf>,
    build_hash: String,
    absolute_time_s: f64,
    time_scale: f64,
    no_console: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut out = Self {
            bind: "127.0.0.1:7777".into(),
            public_host: None,
            public_url: None,
            token: None,
            assets: default_assets_dir(),
            journal: None,
            build_hash: option_env!("TRI_BUILD").unwrap_or("unstamped").into(),
            absolute_time_s: 0.0,
            time_scale: 1.0,
            no_console: false,
        };
        let argv = std::env::args().collect::<Vec<_>>();
        let mut i = 1;
        while i < argv.len() {
            let next = || argv.get(i + 1).cloned().unwrap_or_default();
            match argv[i].as_str() {
                "--bind" => {
                    out.bind = next();
                    i += 1;
                }
                "--public-host" => {
                    out.public_host = Some(next());
                    i += 1;
                }
                "--public-url" => {
                    out.public_url = Some(next());
                    i += 1;
                }
                "--token" => {
                    out.token = Some(next());
                    i += 1;
                }
                "--assets" => {
                    out.assets = PathBuf::from(next());
                    i += 1;
                }
                "--journal" => {
                    out.journal = Some(PathBuf::from(next()));
                    i += 1;
                }
                "--build-hash" => {
                    out.build_hash = next();
                    i += 1;
                }
                "--time" => {
                    out.absolute_time_s = next().parse().context("--time expects seconds")?;
                    i += 1;
                }
                "--time-scale" => {
                    out.time_scale = next().parse().context("--time-scale expects a number")?;
                    i += 1;
                }
                "--no-console" => out.no_console = true,
                "--help" | "-h" => {
                    println!(
                        "triangulum-server [--bind HOST:PORT] [--public-host HOST] [--public-url wss://HOST] [--token TOKEN] [--assets DIR] [--journal FILE] [--build-hash HASH] [--time SECONDS] [--time-scale SCALE] [--no-console]"
                    );
                    std::process::exit(0);
                }
                unknown => bail!("unknown argument: {unknown}"),
            }
            i += 1;
        }
        if out.token.as_deref() == Some("") {
            bail!("--token may not be empty");
        }
        if let Some(token) = &out.token
            && !token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~'))
        {
            bail!("--token may contain only URL-safe letters, digits, '-', '_', '.', and '~'");
        }
        if let Some(url) = &out.public_url {
            let rest = url
                .strip_prefix("wss://")
                .or_else(|| url.strip_prefix("ws://"))
                .ok_or_else(|| anyhow::anyhow!("--public-url must start with ws:// or wss://"))?;
            // The invite appends /?token=..., so a query, fragment, or
            // empty host here can only produce invites parse_invite
            // rejects (Sol review 2026-07-14). Refuse at startup instead
            // of failing at the first join attempt.
            let host = rest.trim_end_matches('/');
            if host.is_empty() || host.contains('?') || host.contains('#') {
                bail!("--public-url must be ws[s]://host[:port] with no query or fragment");
            }
        }
        if !out.absolute_time_s.is_finite() || out.absolute_time_s < 0.0 {
            bail!("--time must be finite and non-negative");
        }
        if !out.time_scale.is_finite() || out.time_scale <= 0.0 || out.time_scale > 1_000_000.0 {
            bail!("--time-scale must be finite and in 0..=1000000");
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

struct ServerState {
    identity: WorldIdentity,
    token: String,
    clock: Mutex<AuthoritativeClock>,
    journal: Mutex<JournalStore>,
    players: Mutex<BTreeMap<PlayerId, PlayerInfo>>,
    clients: Mutex<HashMap<PlayerId, mpsc::UnboundedSender<Message>>>,
    next_player_id: AtomicU64,
    neisor_snapshot: PathBuf,
    moon_snapshot: PathBuf,
}

impl ServerState {
    async fn broadcast(&self, message: Message, except: Option<PlayerId>) {
        let mut dead = Vec::new();
        let clients = self.clients.lock().await;
        for (&id, tx) in clients.iter() {
            if Some(id) != except && tx.send(message.clone()).is_err() {
                dead.push(id);
            }
        }
        drop(clients);
        if !dead.is_empty() {
            let mut clients = self.clients.lock().await;
            for id in dead {
                clients.remove(&id);
            }
        }
    }

    async fn clock_state(&self) -> triangulum_multiplayer::ClockState {
        self.clock.lock().await.state()
    }

    async fn persist_snapshots(&self) -> Result<()> {
        let journal = self.journal.lock().await;
        write_legacy_edits(
            &self.neisor_snapshot,
            BodyId::Neisor,
            journal.journal().columns(),
        )?;
        write_legacy_edits(
            &self.moon_snapshot,
            BodyId::Moon,
            journal.journal().columns(),
        )?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse()?;
    let identity = load_world_identity(&args.assets, args.build_hash.clone())?;
    let token = args.token.unwrap_or_else(generate_token);
    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("bind {}", args.bind))?;
    let local = listener.local_addr()?;
    let public_host = args.public_host.unwrap_or_else(|| {
        if local.ip().is_unspecified() {
            if local.is_ipv4() { "127.0.0.1" } else { "::1" }.into()
        } else {
            local.ip().to_string()
        }
    });
    let public_authority = if public_host.contains(':') && !public_host.starts_with('[') {
        format!("[{public_host}]:{}", local.port())
    } else {
        format!("{public_host}:{}", local.port())
    };
    let journal_path = args.journal.unwrap_or_else(|| {
        args.assets
            .join(format!("multiplayer_seed{}.edj2", identity.seed))
    });
    let mut journal = JournalStore::open(&journal_path)?;
    let was_empty = journal.journal().records().is_empty();
    if was_empty {
        let neisor = args.assets.join(format!("edits_seed{}.bin", identity.seed));
        let moon = args.assets.join(format!(
            "edits_moon_lattice2700000_seed{}.bin",
            identity.seed
        ));
        let mut imported = 0usize;
        for (body, path) in [(BodyId::Neisor, neisor), (BodyId::Moon, moon)] {
            if path.exists() {
                for ((face, ci, cj), value) in load_legacy_edits(&path, body)? {
                    journal.append(
                        0,
                        EditRequest {
                            body,
                            face,
                            ci,
                            cj,
                            value,
                        },
                    )?;
                    imported += 1;
                }
            }
        }
        if imported > 0 {
            println!("IMPORTED {imported} existing EDT1 columns into the append-only journal");
        }
    }
    let neisor_snapshot = journal_path.with_extension("neisor.edt1");
    let moon_snapshot = journal_path.with_extension("moon.edt1");
    let state = Arc::new(ServerState {
        identity,
        token: token.clone(),
        clock: Mutex::new(AuthoritativeClock::new(
            args.absolute_time_s,
            args.time_scale,
        )),
        journal: Mutex::new(journal),
        players: Mutex::new(BTreeMap::new()),
        clients: Mutex::new(HashMap::new()),
        next_player_id: AtomicU64::new(1),
        neisor_snapshot,
        moon_snapshot,
    });
    state.persist_snapshots().await?;

    println!("TRIANGULUM SERVER READY");
    println!("  bind: {local}");
    println!(
        "  world: seed={} build={}",
        state.identity.seed, state.identity.build_hash
    );
    println!(
        "  assets: {} immutable hashes",
        state.identity.asset_hashes.len()
    );
    println!(
        "  journal: {} ({} records)",
        journal_path.display(),
        state.journal.lock().await.journal().records().len()
    );
    if let Some(url) = &args.public_url {
        let url = url.trim_end_matches('/');
        println!("  invite: {url}/?token={token}");
        println!("  local invite: triangulum://{public_authority}/#{token}");
    } else {
        println!("  invite: triangulum://{public_authority}/#{token}");
        println!("  websocket: ws://{public_authority}/?token={token}");
    }
    println!("  D-17: clock authority is SERVER OPERATOR ONLY");
    if !args.no_console {
        println!("  operator commands: seek SECONDS | scale FACTOR | status | help");
    }
    flush_stdout();

    let (operator_tx, mut operator_rx) = mpsc::unbounded_channel();
    if !args.no_console {
        spawn_console(operator_tx);
    }
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(state, stream, peer).await {
                        eprintln!("CLIENT {peer} ERROR: {error:#}");
                    }
                });
            }
            command = operator_rx.recv(), if !args.no_console => {
                let Some(command) = command else { continue };
                match operator_command(&state, &command).await {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err(error) => eprintln!("operator command rejected: {error:#}"),
                }
            }
        }
    }
    Ok(())
}

fn generate_token() -> String {
    let mut bytes = [0u8; 16];
    if !fill_random(&mut bytes) {
        // Last-resort fallback for unusual platforms without an OS random
        // source. It remains unique enough for LAN invites and is loud.
        eprintln!("WARNING: OS random source unavailable; invite token uses process/time entropy");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes.copy_from_slice(&now.wrapping_mul(0x9e37_79b9_7f4a_7c15).to_le_bytes());
        for (i, byte) in std::process::id().to_le_bytes().into_iter().enumerate() {
            bytes[i] ^= byte;
        }
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn flush_stdout() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

#[cfg(windows)]
fn fill_random(bytes: &mut [u8]) -> bool {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut core::ffi::c_void,
            buffer: *mut u8,
            len: u32,
            flags: u32,
        ) -> i32;
    }
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    unsafe {
        BCryptGenRandom(
            core::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        ) == 0
    }
}

#[cfg(not(windows))]
fn fill_random(bytes: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(bytes))
        .is_ok()
}

fn spawn_console(tx: mpsc::UnboundedSender<String>) {
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    eprintln!("operator console: {error}");
                    break;
                }
            }
        }
    });
}

async fn operator_command(state: &Arc<ServerState>, raw: &str) -> Result<bool> {
    let words = raw.split_whitespace().collect::<Vec<_>>();
    match words.as_slice() {
        [] => {}
        ["seek", value] => {
            let value: f64 = value.parse().context("seek expects seconds")?;
            let event = state
                .clock
                .lock()
                .await
                .seek(value)
                .map_err(anyhow::Error::msg)?;
            println!(
                "CLOCK EVENT seq={} seek absolute={:.3} effective_at={}ms",
                event.state.sequence, event.state.absolute_time_s, event.state.server_mono_ms
            );
            flush_stdout();
            state.broadcast(Message::ClockEvent(event), None).await;
        }
        ["scale", value] => {
            let value: f64 = value.parse().context("scale expects a number")?;
            let event = state
                .clock
                .lock()
                .await
                .set_time_scale(value)
                .map_err(anyhow::Error::msg)?;
            println!(
                "CLOCK EVENT seq={} scale={} effective_at={}ms",
                event.state.sequence, event.state.time_scale, event.state.server_mono_ms
            );
            flush_stdout();
            state.broadcast(Message::ClockEvent(event), None).await;
        }
        ["status"] => {
            let clock = state.clock_state().await;
            println!(
                "STATUS players={} edits={} time={:.3} scale={}x mono={}ms",
                state.players.lock().await.len(),
                state.journal.lock().await.journal().records().len(),
                clock.absolute_time_s,
                clock.time_scale,
                clock.server_mono_ms
            );
            flush_stdout();
        }
        ["help"] => println!("operator commands: seek SECONDS | scale FACTOR | status | quit"),
        ["quit"] => return Ok(true),
        _ => {
            eprintln!("unknown operator command; use: seek SECONDS | scale FACTOR | status | quit")
        }
    }
    Ok(false)
}

async fn send_wire<S>(sink: &mut S, message: &Message) -> Result<()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let json = serde_json::to_string(message)?;
    sink.send(WsMessage::Text(json.into())).await?;
    Ok(())
}

async fn read_wire<S>(stream: &mut S) -> Result<Option<Message>>
where
    S: futures_util::Stream<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
        + Unpin,
{
    while let Some(frame) = stream.next().await {
        match frame? {
            WsMessage::Text(text) => return Ok(Some(serde_json::from_str(&text)?)),
            WsMessage::Binary(bytes) => return Ok(Some(serde_json::from_slice(&bytes)?)),
            WsMessage::Close(_) => return Ok(None),
            WsMessage::Ping(_) | WsMessage::Pong(_) => {}
            _ => {}
        }
    }
    Ok(None)
}

async fn refuse<S>(
    sink: &mut S,
    peer: std::net::SocketAddr,
    code: &str,
    message: String,
) -> Result<()>
where
    S: futures_util::Sink<WsMessage> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    eprintln!("REFUSED {peer}: {message}");
    send_wire(
        sink,
        &Message::Refusal(Refusal {
            code: code.into(),
            message,
        }),
    )
    .await
}

async fn handle_connection(
    state: Arc<ServerState>,
    stream: TcpStream,
    peer: std::net::SocketAddr,
) -> Result<()> {
    let websocket = tokio_tungstenite::accept_async(stream).await?;
    let (mut sink, mut incoming) = websocket.split();
    let first = tokio::time::timeout(Duration::from_secs(10), read_wire(&mut incoming))
        .await
        .context("hello timeout")??;
    let Some(Message::Hello(hello)) = first else {
        refuse(
            &mut sink,
            peer,
            "hello_required",
            "first message must be hello".into(),
        )
        .await?;
        return Ok(());
    };
    if let Some((code, reason)) = validate_hello(&state, &hello) {
        refuse(&mut sink, peer, code, reason).await?;
        return Ok(());
    }

    let id = state.next_player_id.fetch_add(1, Ordering::Relaxed);
    let info = PlayerInfo {
        id,
        name: clean_player_name(&hello.name),
        tint: player_tint(id),
        pose: None,
    };
    let existing_players = state
        .players
        .lock()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let welcome = Welcome {
        protocol_version: PROTOCOL_VERSION,
        player_id: id,
        identity: state.identity.clone(),
        clock: state.clock_state().await,
        edit_journal: state.journal.lock().await.journal().records().to_vec(),
        players: existing_players,
    };
    send_wire(&mut sink, &Message::Welcome(welcome)).await?;
    // Do not publish the registry entry until the welcome is on the wire. A
    // peer that disappears during the handshake must not leave a ghost.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel();
    state.clients.lock().await.insert(id, out_tx.clone());
    state.players.lock().await.insert(id, info.clone());
    state
        .broadcast(Message::PlayerJoined(info.clone()), Some(id))
        .await;
    println!("JOIN id={id} name={:?} peer={peer}", info.name);
    flush_stdout();

    let mut keepalive = tokio::time::interval(Duration::from_secs(10));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen = tokio::time::Instant::now();
    let result: Result<()> = loop {
        tokio::select! {
            outbound = out_rx.recv() => {
                let Some(outbound) = outbound else { break Ok(()); };
                if let Err(error) = send_wire(&mut sink, &outbound).await { break Err(error); }
            }
            inbound = read_wire(&mut incoming) => {
                match inbound {
                    Ok(Some(message)) => {
                        last_seen = tokio::time::Instant::now();
                        if let Err(error) = handle_client_message(&state, id, &out_tx, message).await {
                            let _ = out_tx.send(Message::Error { code: "invalid_message".into(), message: error.to_string() });
                        }
                    }
                    Ok(None) => break Ok(()),
                    Err(error) => break Err(error),
                }
            }
            _ = keepalive.tick() => {
                if last_seen.elapsed() > Duration::from_secs(35) {
                    break Err(anyhow::anyhow!("keepalive timeout"));
                }
                let nonce = state.clock.lock().await.monotonic_ms();
                if out_tx.send(Message::Ping { nonce }).is_err() { break Ok(()); }
            }
        }
    };
    state.clients.lock().await.remove(&id);
    state.players.lock().await.remove(&id);
    state
        .broadcast(Message::PlayerLeft { player_id: id }, Some(id))
        .await;
    println!("LEAVE id={id} name={:?}", info.name);
    flush_stdout();
    result
}

fn validate_hello(state: &ServerState, hello: &Hello) -> Option<(&'static str, String)> {
    if hello.token != state.token {
        return Some(("bad_token", "invite token was rejected".into()));
    }
    if hello.protocol_version != PROTOCOL_VERSION {
        return Some((
            "protocol_mismatch",
            format!(
                "protocol mismatch: server={} client={}",
                PROTOCOL_VERSION, hello.protocol_version
            ),
        ));
    }
    state
        .identity
        .mismatch(&hello.identity())
        .map(|reason| ("identity_mismatch", reason))
}

async fn handle_client_message(
    state: &Arc<ServerState>,
    player_id: PlayerId,
    reply: &mpsc::UnboundedSender<Message>,
    message: Message,
) -> Result<()> {
    match message {
        Message::EditRequest(edit) => {
            edit.validate().map_err(anyhow::Error::msg)?;
            let record = {
                let mono = state.clock.lock().await.monotonic_ms();
                state.journal.lock().await.append(mono, edit)?
            };
            state.persist_snapshots().await?;
            println!(
                "EDIT PERSISTED seq={} player={} body={:?} face={} ci={} cj={} value={} journal={}",
                record.sequence,
                player_id,
                record.edit.body,
                record.edit.face,
                record.edit.ci,
                record.edit.cj,
                record.edit.value,
                state.journal.lock().await.path().display()
            );
            flush_stdout();
            state.broadcast(Message::Edit(record), None).await;
        }
        Message::PresenceUpdate(pose) => {
            if !pose.is_valid() {
                bail!("invalid body-local presence");
            }
            if let Some(player) = state.players.lock().await.get_mut(&player_id) {
                player.pose = Some(pose.clone());
            }
            state
                .broadcast(Message::Presence { player_id, pose }, Some(player_id))
                .await;
        }
        Message::Ping { nonce } => {
            let _ = reply.send(Message::Pong {
                nonce,
                clock: state.clock_state().await,
            });
        }
        Message::Pong { .. } => {}
        Message::ClockCommand(command) => {
            let action = match command {
                ClockCommand::Seek { .. } => "seek",
                ClockCommand::SetTimeScale { .. } => "time-scale change",
            };
            eprintln!("CLOCK REFUSED player={player_id}: D-17 server-operator-only {action}");
            let _ = reply.send(Message::Error {
                code: "time_authority".into(),
                message: "D-17: only the server operator may seek or change time scale in MP1"
                    .into(),
            });
        }
        Message::Hello(_)
        | Message::Welcome(_)
        | Message::Refusal(_)
        | Message::Edit(_)
        | Message::Presence { .. }
        | Message::ClockEvent(_)
        | Message::PlayerJoined(_)
        | Message::PlayerLeft { .. }
        | Message::Error { .. } => bail!("message is not valid client-to-server after hello"),
    }
    Ok(())
}

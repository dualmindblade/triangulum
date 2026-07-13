# Multiplayer (Austin + Andrew, 2026-07-13)

Ask: a server app, join via invite/URL, and a basic avatar so other
players' positions are visible. This doc is the engineering read:
what our architecture gives us for free, the phased roadmap, and the
honest impediments.

## The determinism dividend (why this is easier here than usual)

Every voxel, tree, cloud, season, and orbit is a pure function of
(seed, position, absolute time). Two machines with the same build,
the same baked assets, and the same clock render THE SAME UNIVERSE
without exchanging a single chunk. What actually needs to move over
the network:

1. THE CLOCK - one authoritative absolute time (plus time-scale
   anchors and any time-travel seeks). Tiny, and our clock is
   already seekable in O(1), so clients can join at any moment and
   snap to now.
2. THE EDIT JOURNAL - the per-body sparse column edits (place/break)
   as an ordered, append-only log. This IS the world's entire
   mutable state today.
3. PRESENCE - each player's body id + body-local pose (lat, lon,
   alt, yaw, pitch, roll, mode) at ~15 Hz, relayed and interpolated.

That is the whole synchronization surface. No terrain streaming, no
entity soup, no chunk serialization. A weekend-scale server, not a
月-scale one - because the hard 95% (a shared deterministic world)
is already built and gauntlet-enforced.

## Architecture

- SERVER: a headless Rust binary in this workspace (no wgpu/winit
  dependency), owning: world identity (seed + asset/build hashes),
  the canonical clock (monotonic; scale changes and seeks are
  authoritative timestamped events), the edit journal (persisted to
  disk in the existing per-body edits format), and the player
  registry. Transport: WebSocket (tokio + tungstenite) - it plays
  perfectly with invite URLs, works through most home routers'
  outbound rules for clients, and leaves a path to web tooling.
- INVITE/URL: triangulum://host:port/#token (and a plain
  ws://host:port?token=... form). The server prints/generates the
  invite; the client gets a "Join" field in the title/pause UI. A
  token is a shared secret, not an account system.
- JOIN HANDSHAKE: hello(token, build hash, seed, asset hashes) ->
  server verifies IDENTITY MATCH (see impediment 1), then sends
  (clock state, full edit journal, player list). Client applies
  edits, slews its clock (smooth over ~2 s to avoid a visible time
  jump), spawns avatars, done. A joining client needs NO world
  download.
- EDITS: client sends ops; the server assigns a sequence number and
  broadcasts. Last-write-wins per column - correct enough for
  family-scale play, and the journal makes replay/undo tooling
  possible later.
- PRESENCE + AVATAR v1: body-local poses so a friend on the moon is
  correctly placed while you stand on Neisor. Avatar: a two-box
  blocky figure, per-player tint, name tag, simple pose
  interpolation between updates. Deliberately placeholder pending
  the texture/model pass.
- PHYSICS: client-authoritative positions (we trust the family).
  Server-side validation is an internet-era problem, not a v1 one.

## Roadmap

- MP0 (analysis + contracts - this doc, plus protocol schema and
  the clock-authority semantics written before code).
- MP1 PLAYABLE LAN/DIRECT-IP: server binary, invite URL join, clock
  sync, shared edit journal with persistence, presence + placeholder
  avatars. Works across bodies. Playable by the two of you across
  the house (or the internet with a port forward).
- MP2 COMFORT: reconnect resume, join-time journal compaction,
  smarter interpolation/prediction, player list + join/leave toasts
  in the UI, time-control permissions surfaced properly (D-17).
- MP3 INTERNET-HARDENING: token rotation, optional relay mode (no
  port forwarding), server backups, protocol versioning discipline,
  and - if desired - server-validated movement.

## Impediments (the honest list)

1. GENERATION CHURN IS THE BIG ONE. We rewrite terrain generation
   weekly - any gen difference between two builds means two players
   see DIFFERENT worlds while believing they share one, and edits
   land at wrong heights. Non-negotiable mitigation: the join
   handshake hard-gates on build hash + baked-asset hashes and
   refuses mismatches LOUDLY. For the family workflow: update
   together. (Long-term: pin "world versions" per server.)
2. ASSET DISTRIBUTION: joiners need the same output/seed42_r8 and
   weather.bin bakes (~100 MB). MP1: copy them / same machine
   images. MP2+: the server can stream bakes on mismatch.
3. TIME AUTHORITY (needs Andrew - filed as D-17): time travel and
   fast-forward are now GLOBAL actions. Who may bend time on a
   shared server? MP1 default: the server operator only; clients'
   [ ] keys and Travel button grey out with a note. Votes/permissions
   are a design conversation.
4. NAT: direct internet play needs a port forward on the host. Fine
   for you two; a relay is the eventual answer for friends.
5. SEASONAL REBUILDS: bucket-refresh on time jumps means a big
   time-travel event triggers chunk rebuilds on every client -
   already bounded and async, but worth a matched "time is
   changing" toast so it reads as intended.
6. THE PLAY HARNESS stays single-player and deterministic: the
   net layer must be cleanly absent in harness runs so every
   instrument keeps its byte-exactness.

## Deployment: LIVE (2026-07-13)

wss://triangulum.dieorwrite.net is up - verified end to end with a
client joining through the public edge (WELCOME + journal sync).
Cloudflared named tunnel 86002e2b... with 4 HA connections; TLS
terminates at Cloudflare; no router changes; wslrelay untouched.

Pieces:
- Client + server speak wss: tokio-tungstenite rustls (webpki
  roots, ring provider installed at connect), parse_invite accepts
  wss:// invites, server prints the public invite via --public-url.
- Tokens (.cloudfare-creds, GITIGNORED): CLOUDFARE_API_TOKEN (zone
  DNS edit) + CLOUDFARE_TUNNEL_API_TOKEN (account token with
  Cloudflare Tunnel Edit; the /user/tokens/verify endpoint rejects
  account tokens - harmless). The connector run-token lives in
  .tunnel-token, the game token in .game-token (both gitignored).
- scripts/deploy_tunnel.ps1: idempotent create/repair of tunnel +
  ingress + CNAME. Ingress MUST be http://127.0.0.1:7777, never
  "localhost": on this machine localhost resolves to ::1 first,
  where wslrelay also listens on 7777 - the tunnel then delivers
  game traffic to Austin's app proxy (302 /login) instead of the
  game server. Diagnosed 2026-07-13; docker also holds :::7777.
- tools/cloudflared.exe 2026.7.1 (standalone; winget MSI needs an
  elevation prompt). tools/ is gitignored.

RUNBOOK (two terminals from the repo root):
  viewer\target\release\triangulum-server.exe --token GAMETOKEN ^
    --public-url wss://triangulum.dieorwrite.net --no-console
  tools\cloudflared.exe tunnel run --token-file .tunnel-token
The server prints the invite; paste it into the laptop's Join
field. Joiner needs the same build + baked assets (impediments 1/2).

Notes: plex.dieorwrite.net rides a separate pre-existing tunnel -
do not touch. Dead end tried: TryCloudflare quick tunnels register
but their edge 404s the hostname indefinitely (both quic and
http2); named tunnels have no such issue.

## What multiplayer does NOT threaten

The gauntlet, the reels, sync_diff, the censuses - all run on the
single-player pure world and remain the ground truth. The server
adds a thin authoritative shell (clock + journal + presence) around
an engine that was accidentally designed for this from day one.

# M3a: Andrew's look panel, look files, per-face block colours (Opus, 2026-10-25)

Summarised by the coordinator from the agent's report; merged at `db9b138`.
Evidence in `M3a-evidence/`.

## What it adds

- **The panel (F1):** an egui window on the right with plain-word labels
  and hover help. Opening it frees the mouse; keys still move you; typing
  in a field isn't game input. Sections:
  - **Sky and light:** time of day, time speed, atmosphere height, fog
    density and strength, sky colours, sunset strength, night sky, stars,
    sun, sky light, light in shadow, night brightness, moonlight and its
    colour, side and underside shade, corner shadows.
  - **Water:** colour, opacity, edge opacity, sparkle, underwater tint,
    "far ocean follows the water colour" (keeps the opaque `deep_ocean`
    seen from orbit matching).
  - **Blocks:** every block, one colour or top/sides/bottom.
  - **Distance detail:** the LOD factor.
  - **Terrain:** roughness, landforms, vertical scale (1/70, 1/20, 1/8).
    A preview world with an empty edit store and saving off; the real
    engine, save and simulation are set aside untouched. "Back to my world"
    returns; "Make this the default..." confirms, warns that old saves
    won't load, and writes only those three values to `world.json`.
  - Reset per section, Save look, Revert to saved, Screenshot.
- **Look files:** `game/assets/look.json` (new, `src/look.rs`; defaults
  are the M1 look). `blocks.json` is written back in its own style
  (rewriting the shipped file from its own colours is byte-identical). All
  three files hot-reload twice a second; a broken file is reported in the
  panel and not applied. World.json's sky optical fields still set the
  defaults; look.json overrides them.
- **Per-face colours:** grass and dry grass have dirt sides and bottoms;
  snow has a dirt bottom only. The GPU colour table holds three entries
  per block. Colour changes rewrite that table instead of remeshing:
  meshes carry block ids, not colours.
- **Screenshot** saves the next frame without the panel to
  `game/out/look/`, with a JSON sidecar (the look, every block's face
  colours, terrain, time, position). `--screenshot` writes the same sidecar.
- **Replays:** the time slider goes through `Session::apply(Action::Sky)`;
  starting a terrain preview stops a replay recording.
- **Options:** `--look-file FILE`, `--panel`; `--lod-factor` defaults to
  look.json's value. `TRIANGULUM_PANEL_SECTION=blocks|sky|water|detail|terrain`
  opens `--panel` on a section (for evidence screenshots).

## Evidence

- Panel screenshots `panel_01..04`; a live session in the real window
  driven by real mouse clicks (`live_01..04`): Screenshot button, a
  `blocks.json` colour edited in a text editor appearing live, a 1/20
  preview built in 1.6–3.5 s with the save's files hashing identical
  before and after, Make default writing `"vertical_scale": 0.05` (and the
  old save then refused, as warned), Back to my world, the time slider
  through the session after merging M2c.
- Before/after frames (`ba_01..10`): atmosphere 4,000 m keeps the sky blue
  at 1.5 km and over the 1/8 peak; brighter night; bluer water; a tunnel
  under grass with a dirt ceiling (`scripts/m3a-tunnel.play`, 20 asserts);
  grass terraces now with brown risers.
- With the shipped look and M2b's blocks.json, frames match M2b within
  2/255 on every shot of `scripts/m3a-look.play`.
- Bench flight, panel closed, idle machine: M3a 1,236–1,258 fps average,
  598–627 1% low; M2b 1,234 / 543. Unchanged within noise.
- Coordinator's check: 132 tests, all 14 play scripts pass; my own look
  files (atmosphere 4,000 m; night brightness 0.3) change the frames as
  expected at 1.5 km, noon and 23:30.

## Not verified

- Readability at 100% display scaling: this machine runs at 150%.
- The colour pickers, Save look and terrain slider drags were not clicked
  live (unit tests, hot reload and the preset button cover those paths).
- What a 1/8 preview draws at the deepest ocean in the 2,048-block world.

## Feedback

- **Driving the real window** works from PowerShell (the scripts were in
  the agent's scratchpad and are gone). Pitfalls: the game is a console
  program, so `Process.MainWindowHandle` is the console; find the window
  titled "Triangulum" by process id instead. Pass `[NullString]::Value`,
  not `$null`, as the window class. The window's DPI is 144: call
  `SetProcessDPIAware` first. Worth making a repo tool for app-only paths
  the play harness can't reach.
- **Timing runs** kept being contaminated by other agents' builds. A small
  "wait for one quiet minute, then run" loop made the before/after flights
  trustworthy; worth adding to the bench workflow.

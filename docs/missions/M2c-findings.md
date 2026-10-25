# M2c: replays that reproduce a session exactly (Fable, 2026-10-25)

Branch `worktree-agent-a79596cd21d5293e5`, crate `game/`, from main at
3f8672f (M2b merged). Also folds in review 3 #4, #6 and #13 (the
coordinator's mid-mission request). Evidence in `docs/missions/M2c-evidence/`;
full-size frames under this worktree's `game/out/` (gitignored, this
machine only).

## What was built

| where | what |
|---|---|
| `session.rs` (the driver) | Everything a player does reaches the game at a fixed 60 Hz step boundary: the held movement keys through `Session::step`, every other input as an `Action` through `Session::apply` (look deltas, press / release / click, hotbar, mode, R, P, water view, wheel, time speed, sky seek, sim on/off, radius, period, an exact pose, reload). Toggles are recorded as the state they set. The held-button repeat and the sky clock moved into the session (`Session::sky`, advanced on every step); renderers copy it. `start_recording` / `stop_recording`. `state()` gains `sky_seconds`, `sky_speed`, `steps`, `recording`. |
| `controls.rs` (new) | The windowed app's input: window events in, step-positioned actions out, in arrival order. A key pressed and released between two steps counts as held for one step (a tap shorter than a frame used to be lost). Mouse motion is drawn ahead of its step (`camera`) so looking stays smooth above 60 fps without moving the player early. Focus loss releases keys and the button. |
| `app.rs` | Uses `Controls`; F10 starts/stops a recording (title shows `REC replay`, the stats panel names the file); quitting stops it; `--record-replay FILE` starts one at launch (the same toggle, for headless checks); `--replay FILE --watch` plays one in the window. Preset logic moved to `Session::apply_preset`. |
| `replay.rs` (new) | The file: magic line, one-line JSON header (world identity, bake SHA-256, generator version, hotbar, sky calendar and clock, sim settings, counters, step counter, held button, exact player pose, start digest), the starting snapshot as a whole save image (header.json, commit.json, every edited chunk, the sim state; names, sizes, SHA-256s in the header), then append-only text events `@STEP verb args`. Floats in shortest round-trip form (bit exact). `Recorder` writes it, flushing every second, so a crash keeps everything up to the last second. A sentinel notices changes made outside `apply`/`step` (harness teleports, sky seeks, sim settings) and writes them as exact `pose`/`sky`/`sim...` events; a world change behind the driver's back becomes a `note` (the replay reports it). Checks every 60 steps: digests of the world (per chunk, cached by revision), the simulation, the player, the view, plus the counts, the exact pose and clock, and the digests of the chunks that changed since the last check, so a divergence names its chunks. `session_from` builds a session from the file in a scratch save `saves/_replay/<name>` (refuses any other directory and the running session's own save), `Runner` plays and verifies, `Report` says the first divergence. |
| `world/save.rs` | `snapshot_image` (a whole save as files, in memory), `install_image` (writes an image into an empty directory; every name checked against the save layout first, since replays come from other machines). |
| `sim/mod.rs` | `state_digest`: a non-mutating digest that counts pending changes as the updates they become, so a save's early absorb and a replay's late one hash the same. Review 3 #4: the snapshot (format v2; v1 still reads) carries the settings, the active region's focus and `rules_digest` (the block definitions' physics flags, not colours); a restore brings the region back where it was computed; `state_hash` covers settings, focus, rules. Review 3 #6: `state_hash` hashes content (edited chunks equal to the generator's are left out). |
| `world/edits.rs` | Review 3 #6: a commit that changes nothing in a chunk it materialised drops it again (and its index entries). |
| `play.rs`, `play/find.rs` (new), `play/video.rs` | `find flat|shore|river|lake|waterfall`; `replay record|stop|check`; `--replay FILE [--shot-at STEP|end]... [--watch]` (`run_replay`). Harness commands that change state go through `apply`. Review 3 #13: `fly` steps the session every frame; video ticks refresh the falling blocks the frames draw; the sky clock is the session's (`skyspeed`, `skytime`, `sun` set it). |
| scripts | `m2c-replay.play` (find, record, a sand collapse, digging and pouring at a river bank, sky, view, region, fly, save, sim_hash, reload, then play it back and check); `m2c-video-driver.play` (review 3 #13). |
| README | "Reporting a bug: press F10" for Austin and Andrew, the replay commands, `find`, the new scripts. `.gitignore`: `game/replays/`. |

### Decisions written down

- **A replay embeds its snapshot.** The file is the whole bug, so nothing
  has to travel with it. It costs about 4.6 KB per edited chunk (400
  chunks: 1.8 MB) plus a few KB per minute of events. The real save is
  only referenced (path and generation) and brought up to date at F10.
- **Inputs, not just state.** The file holds the inputs (so an agent can
  see what was pressed) and per-second checks (so a divergence is found
  within a second and named). Harness pokes become exact `pose` events.
- **The settings a replay plays under are its own**; the scratch save's
  are the file's. On an ordinary launch the command line's simulation
  settings win over the save's, with a printed note if they differ.
- **Same inputs means same steps.** "A session at 144 fps and one at 30 fps
  give the same world from the same inputs" holds when the inputs arrive at
  the same step positions. Physical key timing maps onto steps through the
  frame boundaries, which no engine can make frame-rate independent; what
  was fixable (taps lost between frames, clicks and looks outside the step
  driver, the harness sky clock, the fly path bypassing the driver) is
  fixed, and each recording reproduces itself whatever its frame rate.
- **The active region is restored, not re-derived** (review 3 #4 offered
  either). Re-deriving it from the chunk centre would change which cells
  `m2b-boundary` and `m2b-river` simulate; persisting the focus changes
  nothing for a continuous session and makes a reload exact.

## Evidence

All on the RTX 2060 machine, `--bake C:/code/triangulum/game/assets/world/seed42`.

**Tests:** `cargo test --release`: 121 passed, 0 failed, 11 ignored (was
111; `tests-release.txt`). New:

| test | shows |
|---|---|
| `replay::record_then_replay_gives_the_same_state` | a 400-step session driven through `Controls` with synthetic window events (walk, sprint, look, held dig, a sub-frame Space tap, water poured and flowing, sand placed, sky speed, water hidden and shown, fly, wheel) records with 0 notes and replays: 7 of 7 checks, end hash equal to the session's, final pose bit-identical |
| `replay::the_same_inputs_at_different_frame_rates_give_the_same_world` | the same timeline at 144 fps and 30 fps: equal end hashes and byte-identical event streams (inputs at the same steps, identical checks); a ragged pacing (frames 2-200 ms) replays exactly too |
| `replay::a_corrupted_replay_reports_its_first_divergence` | one look delta changed at step 68: first divergence at step 120, "player: position ... apart"; a held dig turned into a place: step 180, "world: ... face 2 chunk (...) differs"; a flipped check digest: reported at that check only; a damaged snapshot byte refused ("damaged"); a file cut off mid-line plays to its last event with two warnings |
| `replay::a_replay_never_touches_the_real_save` | a session on a real save records and replays; every file of the real save is byte-identical afterwards; the replay's save is under `_replay`; playing into the real save or the running session's own save is refused |
| `replay::harness_pokes_are_recorded_as_poses_and_replay_exactly` | teleport, heading change and sky seek outside the driver become `@30 pose` / `@30 sky` lines and replay bit-exactly; a world edit behind the driver's back becomes a note and the replay says it diverged |
| `replay::a_tap_shorter_than_a_frame_still_counts` | Space pressed and released between two frames jumps at 30 and 144 fps. **Fails with the latch removed** (`tap-without-latch.txt`: "30 fps: the tap jumped" panics) |
| `sim::a_reload_keeps_the_active_region_it_was_computed_for` (review 3 #4) | region computed at a chunk's edge, player walks to its middle, save, restore: same region, sand in the neighbour chunk falls in both, equal hashes after 10 ticks. **Failed before** (`review3-before.txt`: "the restored region is the saved one: left 3, right 2") |
| `sim::the_hash_covers_the_simulation_settings` (#4) | radius, enabled, water period, sand's gravity flag each change the hash; a colour does not. **Failed before** ("radius is in the hash": equal hashes) |
| `water_waits_for_its_period_and_the_period_is_in_the_hash` (#4) | now asserts on the hash. **Failed before** |
| `sim::water_tests::a_write_that_changes_nothing_leaves_no_chunk_and_no_trace_in_the_hash` (#6) | pouring into a full sea cell and writing air over air leave 0 edited chunks and the hash unchanged; a block dug and put back hashes as untouched. **Failed before** ("left: 2, right: 0") |
| `world::save::a_save_image_installs_and_loads_like_the_store_it_came_from` | the image loads as the store it came from; names like `../x.json`, `chunks/../../evil.chunk`, `C:/evil.chunk` are refused |
| `snapshot_bytes_round_trip_and_reject_damage` | v2 round trip with settings/focus/rules; a v1 file still reads |

`world::edits::set_tracks_change_and_dirt` asserted the old behaviour
("the chunk becomes authoritative on first touch") that review 3 #6 calls a
bug; it now asserts the new contract.

**Scripts** (`scripts-summary.txt`, logs beside it): m1-done 94/94,
tunnel-wall-reload 18/18, edge-corner-flight 17/17, m2a-sand 95/95,
m2b-water 97/97, m2b-boundary 44/44, m2b-river 41/41, water-lod-seam and
water-lod-seam-high run clean (frames only), **m2c-replay 29/29** (the
recorded session: 468 steps, 40 events, 8 checks, 0 notes, 7.9 KB; played
back in-process: 8 of 8 checks, end hash matches), **m2c-video-driver
21/21**.

**Review 3 #13 before/after:** `m2c-video-driver.play` on main's build
(3f8672f, exported and built in the scratchpad): 8 of 21 asserts fail:
`sim_tick == 21, actual 1` after the one-second flight, `landings == 1,
actual 0`, and the sand still hanging in the air afterwards
(`block_rel[1,0,1] == air, actual sand`), plus the new fields missing
(`m2c-video-driver-main.txt`). On this branch 21/21
(`m2c-video-driver-new.txt`): the block lands during the flight, the tick
advances 20, the sky clock exactly 60 s at 60x, and the recorded frames
draw the falling block (`video_falling_drawn_max >= 1`).

**The app path:** the windowed app recorded with `--record-replay` (the
function F10 calls) at vsync and uncapped with the sky at 600x; both
played back headless "reproduced exactly" (7 of 7 and 8 of 8 checks, end
hashes equal), and the uncapped one again in the window with `--watch`
(`app-window-replays.txt`, `replay-watch-window.jpg`).

**Shots from replays:** `--replay m2c-river.replay --shot-at 0 --shot-at
120 --shot-at end` wrote settled shots with the HUD and state sidecars;
step 120 shows the river bank with the river flowing into the dug cells
(`replay-shot-step120.jpg`), as in the recording (`m2c-river-replay-run.log`,
`m2c-river-replay-report.json`).

**find** from the default start (`find-*.jpg`): flat 3 m away (101 columns
sampled), shore 3.9 km (5,169), river 1.9 km (22), lake 11.1 km (3, after
walking from the nearest lake cell to its shore), waterfall 2.2 km (58; a
3-block fall). The script pins the flat and river sites by coordinates, so
a change in the search shows up as a failed assert.

**Cost of recording** (`replay_cost`, ignored test): with 400 edited
chunks, F10 takes 79 ms once (the snapshot); a per-second check takes
0.085 ms when nothing changed and 4.5 ms with all 400 chunks re-digested.
Not recording costs nothing (two `Option` checks per step). I did not run
the 60 s bench flight: nothing in rendering or streaming changed, and the
machine was shared.

## Not verified (what a human still needs to try)

- **Pressing F10 in the real window.** The key mapping (`KeyCode::F10` to
  the toggle) and real devices (raw mouse motion, a held button across
  frames, alt-tab during a recording) were never exercised; the code
  behind them was (synthetic events through `Controls`, and the window
  loop through `--record-replay`). Please: play a minute with F10 on, dig,
  pour water, toggle G, F, [ ], P, R, then run `--replay` on the file.
- **Another machine.** Replays are bit-exact on this machine. Player
  physics and the camera use `sin`, `cos`, `atan2`, `powf` from the
  Windows C runtime; a different Windows build could round one of them
  differently and a replay from Andrew's machine could diverge on Austin's.
  A divergence would be reported, not hidden. Worth one test: Andrew
  records, Austin replays.
- **A different build.** A replay is tied to the code that recorded it
  (any physics change diverges it). The header names the engine version
  but not a commit hash.
- Very long recordings (hours) and huge saves were not tried. By
  estimate (one ~20-byte `look` line per step while the mouse moves) the
  file grows about 1.2 KB a second of looking around; not measured.

## What I would change next

- Put the git commit in the header (a build script) so a replay names the
  build it needs.
- `--replay FILE --until STEP` then hand over to the keyboard: reproduce,
  then explore from the bug.
- If cross-machine divergence shows up, switch the player's trig to a
  portable `libm`.
- `find` frames a waterfall from beside its lip, looking steeply down; a
  `find ... view` variant could hover back and frame the fall.

## Feedback

- **New tooling (mandatory):** `find` saved real time already: the replay
  script needed a river bank and flat ground and got both in two lines,
  pinned by asserts. The `replay record/check` pair makes "does this
  session reproduce" a script assert, which is what made the recorder
  testable without a human. The `state_digest` design (count pending
  changes as their future updates) came from noticing that the app's
  autosave absorbs changes early; without it every check after a save
  would have been a false divergence.
- The worktree isolation refused any Bash command that ran the game
  through a shell variable or a loop (and a `git archive | tar` pipe),
  which cost a dozen retries; plain one-command calls and a script file
  in the scratchpad work. Worth a line in `docs/PROCESS.md`.
- The coordinator's mid-mission review items arrived with line numbers and
  a clear fix shape; writing the failing tests first against the
  unchanged code gave the before/after evidence directly.
- Building main from `git archive` in the scratchpad is a cheap way to get
  a "before" run for a harness script; it took 2 min and touched no shared
  tree.

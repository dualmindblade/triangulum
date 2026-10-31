# B6-reports findings

Austin: "now that it's recordings I suppose we should have a way to annotate
recordings as well as images for bug reports, but both screenshots and recordings
would be nice."

It is built, and every part ran headless through the real code with synthetic
events. **Nothing here has run in a real window, by the brief's rules**, so how the
pen feels is still to be found out. The list is under *Not verified*.

## What Andrew does now

1. Sees something wrong and presses **F9**. The game stops on that frame.
2. Draws on it (pen, arrow, box, text, five colours, undo) and types a sentence.
   Typing goes in the comment whatever he drew last.
3. Presses **Enter** and is back in the game, keys and all.
4. Later: **T**, *Recordings*, picks the recording, *Make report*, *Show folder*, and
   sends that folder.

With no recording running, F9 is a marked photo. *Annotate* in the T window does
the same to a photo already on the roll.

## What's there

**1. Annotate a photo** (`annotate.rs`, new). Marks are data in the picture's own
pixels: `annotations` in the sidecar, written through `photos::edit_sidecar`. The
PNG is only read. A flattened `<stem>_annotated.png` goes beside it, and it belongs
to its photo: it isn't a new row on the roll, and Delete takes it along.

One function paints a mark on screen and into the flattened copy. The copy goes
through egui's own tessellator into a small software rasteriser, so it is what was
on screen, on any thread, with no GPU and at the picture's own size.

**2. Mark a moment.** F9 draws the frame again clean (no HUD), reads it back, and
opens the annotator. Saving writes `note marker {JSON}` at that step, naming its
shot, and flushes the file. The shot and a whole sidecar go in
`<recording>.markers/` beside the recording. Cancelling leaves nothing, and takes
no marker number.

**3. Reading it back.** `--replay` lists the markers, takes a settled shot at each
looking where the player was looking, and puts them in `report.json`, with how
many pixels each differs from the frame that was marked. `--watch` shows a
marker's words as it passes. The T window has a *Recordings* tab beside *Photos*:
every recording with its length and marker count, a title and description kept in a
`.json` beside it, and its markers. A marker behaves as a photo there: preview,
Annotate, Repro, Teleport.

**4. One thing to send** (`report.rs`, new). *Make report* gathers a recording, or
the ticked photos, into `interchange/reports/<when>-<title>/` with a `report.md`.
The folder is built under another name and renamed when whole. The copying is on a
thread. Read the example: `B6-reports-evidence/example-report/report.md`.

**5. Review 4 #8** (`visual.rs`, new). A recording now holds every look value, the
identity of every block and texture as they are live (a painted, unsaved texture
counts), the mesh view and the mesher settings. A replay reports the simulation and
the look separately: *the same*, *NOT the same* with what differs, or *UNKNOWN* for a
file older than the check. A change during a recording is written down as
`note visual`. The exit code and `ok` are unchanged and mean the simulation.

## How F9 stays out of the replay

The mission's hard part. `Gate` (in `annotate.rs`) sits between the window events
and `Controls`, and the app and the tests go through it:

- **Steps stop.** A frame with the gate open runs none, and nothing accumulates, so
  there's no burst to catch up on afterwards.
- **Presses, clicks, motion and the wheel go no further.** egui has them already.
- **Every release gets through.** A key held at F9 and let go while drawing is let
  go at that step. This also protects F9 itself: it goes down with the gate shut and
  comes up with it open, and if that release were swallowed `Controls` would hold it
  down forever and F9 would never fire again.
- **A mouse button's release gets through only if the game saw it go down,** so a
  click in the annotator isn't a stray `release` in the file.
- **What's held all the way through is still held.** Nothing is cleared, so the game
  carries on exactly where it stopped, with the same accumulator and queue. This
  differs from the T window, which lets go of everything.

The marker note precedes that step's other events, and the pose in it is the one
the camera saw, which can be ahead of the step by mouse motion not yet applied.

The proof is that a timeline with three markers (one with W held, one with the left
button down and repeating, one in a quiet moment, annotators open for up to 2,000
frames) gives **the same event file, marker lines aside, the same player and the same
state hash as the timeline with none**. Then it replays exactly. Nine tests in
`marker_tests.rs`, through the same synthetic events as `replay_tests.rs`.

## What running it found

| Found by | Fault | Now |
|---|---|---|
| Looking at the first frames | A text mark lost what was typed: the click's release lands outside the field that just opened, and egui takes the keys away | Focus is held every pass; a test types through it |
| Looking at 1280 x 720 at 150% | The line about the moment ran over the colours and Undo | A row of its own; the fit test now checks that nothing overlaps, at six sizes and scales |
| Reading the code first | `shot_x_annotated.png` would have become a photo on the roll, and been left behind by Delete | Fixed, with tests |
| Re-reading the code | After a text mark closed, its field's rectangle stayed behind and swallowed presses there, so a stroke started on that spot did nothing | The test failed first (2 marks, wanted 3), then the fix |
| Re-reading the code | After *Esc again throws this away*, typing more and pressing Esc lost everything without asking | Typing takes the warning back; the test failed first |
| Reading `report.json` | Marks had tails: `669.5999755859375` | Stored as f64 to a tenth of a pixel |
| Looking at a `--shot-at` frame | The marker line sat under the stats panel | Beside it, as the mesh legend does |
| Tests | `Runner` would have listed markers among its warnings | They're filtered; `notes` stays "what the recorder could not capture" |

## Measured

From fresh processes on the final build (`d132cd8`): `scripts/b6-report.play`, then
what an agent does with what it made. The full transcript is
`B6-reports-evidence/scenarios.log`. **All timings are provisional:** other agents were
running, so each is the range over two runs.

| What | Result |
|---|---|
| The marked frame against `--replay`'s settled shot at that marker | **0 of 2,073,600 pixels differ, both markers.** The frame was settled before F9 here; see the note below |
| Playing it back | 4 of 4 checks, end hash matches, `look: the same` |
| The same recording under another look (fog, sun, water changed) | Simulation reproduced, exit 0. `look: NOT the same`, naming all three values. 1,208,020 and 1,288,624 pixels differ |
| With the `recorded.look.json` it wrote, via `--look-file` | `look: the same`, **0 pixels differ** |
| Under another `blocks.json` (sand's colour) | Simulation reproduced; `block sand: not the same`; 231,359 and 39,265 pixels differ |
| A header with `build` and `visual` removed (a file from before B6) | Plays exactly; `look: UNKNOWN` |
| A copy with its rules version changed, so it's refused | Exit 2; markers listed; `report.json` has their words, poses, shots and marks; `--repro` works from the report folder with no recording |
| Andrew's nine real recordings | All parse and are refused by name (generator 3 vs 4, since B2), exit 2, with a `report.json`. They list in the T window with their lengths |
| F9, frame grabbed (redrawn clean, read back, sidecar) | 6 to 15 ms |
| F9, annotator on screen with its picture | 16 to 31 ms |
| A pass of the annotator, 20 strokes of 300 points | 2.2 to 2.6 ms (worst 3.1) |
| The same, 200 strokes (60,000 points of ink) | 16 to 26 ms (worst 33): the far end, and over a 60 Hz frame there. Nobody should reach it, but it isn't free: the cost grows with what's drawn |
| Flattening those 200 strokes at 1080p, on its thread | 509 to 575 ms |

**About the 0 pixels.** A marked frame is live, so in play the distance can still be
loading when F9 is pressed, and the settled shot will differ there. The first run of
the script grabbed an unsettled frame and differed in 1,610,315 pixels for exactly that
reason. `report.md` says so on every live frame.

**About a repro with edits.** `--repro` of the first marker gave the frame at the
exact pose and clock, 356,077 pixels different, because the sand stack isn't in an
empty save, and its edit check failed and said so, as B5 designed. So when a recording
is refused, a repro of a moment in an edited world shows the land without the edits.
`report.md` says this on each such item.

## Evidence

`B6-reports-evidence/`, 3.1 MB, game frames from the runs above:

- **`example-report/`** is one complete report from `b6-report.play`, as it came out of
  Make report. Start at its `report.md`, and read it as an agent would. **The bug in it
  is invented:** the words and marks are the script's, and it says so. The marks went
  through the same pointer events a hand makes.
- `annotator-open.jpg`, `annotator-drawn.jpg`, `annotator-esc-warning.jpg`: the
  annotator on F9, with all four tools, and after one Esc.
- `annotator-1080p-at-150.jpg`, `annotator-720p-at-150.jpg`: 150%, the second being
  where the overlap was.
- `annotate-from-the-roll.jpg`, `t-window-photo-marked.jpg`: Annotate from the T window,
  and the roll afterwards.
- `t-window-recording.jpg`, `t-window-make-report.jpg`, `t-window-report-ready.jpg`:
  the Recordings tab, a marker selected, and Make report.
- `t-window-andrews-recordings.jpg`: the tab over Andrew's nine real files.
- `replay-shot-with-marker-line.jpg`: the line `--watch` shows, drawn by the same
  `draw_hud`, in a `--shot-at` frame.
- `replay-settled-shot-of-marker-1.jpg`, `replay-shot-in-another-look.jpg`: a marker's
  shot in the recorded look and in another.
- `replay-report.json`, `refused-report.json`: `report.json` from a replay that played
  and from one that was refused.

## Verified, and how

| What | How | Result |
|---|---|---|
| Everything | `cargo test --release` | 284 passed, 0 failed, 22 ignored |
| The feel of the pen: a mark on the pressed pixel in the pass that saw the press; 50 pointer moves between two frames all land | `annotate_tests.rs`, `Rig` | pass |
| Marks land on the right picture pixels, off the edge stays on it, nothing overlaps, at six sizes and scales | `annotate_tests.rs` | pass |
| Every tool and colour, undo, Enter, Shift+Enter, Esc's two steps, text marks | `annotate_tests.rs` | pass |
| The flattened copy: marks on their pixels, nothing elsewhere, input untouched, on a thread | `annotate_tests.rs` | pass |
| Original PNG unchanged by a byte; marks round trip; unknown keys survive | `annotate_tests.rs`, `menu_tests.rs` | pass |
| A recording with markers replays exactly; the gate, key by key | `marker_tests.rs`, 9 tests | pass |
| A look change during a recording | `marker_tests.rs`: `note visual` at its step, in words | pass |
| Old headers parse; unknown stays unknown | `marker_tests.rs`, `visual.rs` | pass |
| The report: every field; photos; missing sidecars; nothing half-made left | `report_tests.rs` | pass |
| A report with its replay gone: `--repro` from the copy alone | `report_tests.rs`, then in a fresh process | pass |
| The T window, by synthetic pointer: Annotate, Recordings, Make report, Esc layers | `menu_tests.rs` | pass |
| The whole thing end to end | `b6-report.play` | 29 asserts, 0 failed |
| Drawn at 100% and 150%, 1080p and 720p | `b6-scale.play` | 5 asserts each, 0 failed |
| Gates | `m1-done` 94, `m2c-replay` 29, `m2a-sand` 95, `m2b-water` 97, `b5-menu` 29, `b5-menu-scale` 4, `m3b-hotbar` 11 asserts; `m3a-look` ran | 0 failed |

`m3b-hotbar` used `--blocks scripts/m3b-look/blocks.json`, and `b5-menu` used
`--interchange`. Nothing was written to `game/interchange/` or `game/replays/`: every
run used its own folder, and Andrew's recordings were only read.

## Not verified

The brief bans a visible window and real input, so all of this needs real hands.

- **How the pen feels.** The pass cost is measured, the display isn't. It's the first
  thing to try. F9 skips drawing the world, and `set_frame_latency` lets only one frame
  queue while the annotator is open; neither has run on a real surface.
- **All of F9 in `App`:** the key reaching `mark_wanted`, `open_mark`, the mouse coming
  free and being taken back, `annotate_frame`, `finish_mark`. `Gate`, `Moment`, the
  annotator and `Menu` are tested; the wiring between them is not. By reading only.
- **Real pointer events:** egui_winit's `PointerMoved` rate, and whether a real press
  and release behave like `Rig`'s. A real 150% display scale.
- **`--watch`.** The ticker is tested, and the line was seen drawn by `draw_hud` in
  `--shot-at` frames, which show the same line. It hasn't run in the app.
- **`refresh_visual` in the app**, so a change in the look panel during a real
  recording. `Session::visual` and the recorder are tested; the 2 Hz check that feeds
  them isn't. It also means a change can land up to half a second late. At F9 it's
  exact.
- **The window closing with F9 open** keeps the marker (`finish_now`). By reading.
- **Show folder** (it would have opened Explorer on Austin's desktop) and **Copy path**.
- **Whether it looks right.** Pen weight, colours and text size are Andrew's.
- **Astra's work.** This branch is built on `3684653` and hasn't met it.

## For the merge

- `controls.rs` isn't touched. `Gate` wraps `Controls::key`, `button`, `motion`, `scroll`
  and `frame`. If review #10's fix changes those calls, `annotate.rs` follows.
- `replay.rs` is additive: `build` and `visual` in `Header`, `Expect::visual`,
  `Summary::markers`, `Recorder::marker` and `next_marker`, a block in `sync`,
  `Replay::markers` and `visual_changes`, and one match arm in `Runner::advance`.
  `Report::ok` and `Report::text` are unchanged, so it still prints *the session
  reproduced exactly* and then the `look:` line. When #9 rewrites that verdict, "the
  simulation" is the better word there.
- Small edits outside my list: `session.rs` gains `Session::visual`, `workshop.rs` gains
  a read-only `texture_set()`, and `replay_tests.rs` gains one line to include
  `marker_tests.rs`.

## For Andrew and Austin to decide

- **F9's frame has no HUD,** so it's the world alone. A HUD bug needs Shift+P and then
  Annotate. Is that right, or should F9 keep the HUD?
- **Enter saves** and Shift+Enter is a new line, because a sentence is the common case.
- **Esc asks once more** if anything would be lost.
- **Pen weight 5 px and text 32 px on a 1080p picture,** scaled with the picture.
- **A report is a folder,** not a zip. Windows zips one in two clicks; a real zip here
  would need a new dependency.
- **`--replay` reports a different look and doesn't apply the recorded one.** It writes
  `recorded.look.json` for `--look-file`, and that gave 0 pixels. It could apply it by
  default, so an agent sees Andrew's look without asking, at the cost of a replay no
  longer showing the player's own look files.
- **Textures are recorded by identity, as the brief asked,** so a replay can say one
  differs but can't bring it back. At 16 x 16 the pictures would be cheap to embed.
- **A marker doesn't hold the world's edits.** A small save beside each marker would
  make `--repro` exact in an edited world even after a recording is refused. Not built:
  it's more than the brief asked.

## Proposed wording for the lead

For `STATUS.md`: "F9 marks what's wrong: the game stops, draw on the frame and say why,
Enter goes back. During an F10 recording each one is a marker at that step and the
replay is unaffected; otherwise it's a marked photo. The T window annotates photos, lists
recordings with their markers, and Make report gathers one folder to send, with a
report.md an agent can act on alone. A replay reports the simulation and the look
separately (review 4 #8)."

For `DESIGN.md`, under *Saves and replays*: "A marker is a note in a replay and changes
nothing it reproduces. A replay holds the look's values and the identity of its blocks
and textures, and says separately whether the simulation and the look reproduced."

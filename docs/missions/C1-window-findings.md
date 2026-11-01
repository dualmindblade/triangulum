# C1: the window and great circles (Opus, 2026-10-31)

Rows #8 and #10 of round 2. Branch `worktree-agent-a60ba3785e1a85de9`. Evidence in
`docs/missions/C1-window-evidence/`. All timing here is provisional: T1, C2 and others
were running throughout.

## In short

- **Minimising killed the game, and that is fixed.** The real panic text is below. It
  wasn't Vulkan refusing a 0 x 0 surface: the game configured a 1 x 1 surface and kept
  a 640 x 360 depth buffer, and wgpu panics on a render pass with both.
- **Losing focus never crashed**, but the game went on believing it had the mouse. It
  lets go now, and the click that brings you back captures the mouse and breaks nothing.
- **The powder launch crash is not reproduced.** I tried everything I could think of
  in the real window (table below) and it never crashed. I found three ways a launch
  ends at once, or panics, that would look like a crash, and fixed those. I can't say
  any of them is what Andrew saw.
- **Fly speed has no ceiling, and the floor is 1 cm/s.** Holding the wheel found one
  real hang, one NaN and a way through the planet.
- **WASD follows great circles**, flying and walking, in every direction, straight
  over both poles. The way a body faces is now a vector carried along each move, not a
  compass heading.
- Old recordings refuse to play and say why. New ones reproduce exactly.

## 1. The crashes

### Clicking out

The panic, on 18d69c7 with only the launch flag added, from
`ShowWindow(SW_SHOWMINNOACTIVE)` (`before-minimise-panic.txt`):

```
thread 'main' panicked at ...\wgpu-30.0.1\src\backend\wgpu_core.rs:2678:18:
wgpu error: Validation Error

Caused by:
  In a CommandEncoder
    In a pass parameter
      Attachments have differing sizes: the depth attachment's texture view has extent
      (640, 360, 1) but is followed by the color attachment at index 0's texture view
      which has (1, 1, 1)
```

What happens: minimising sends `Resized(0, 0)`. `configure_surface` clamped that to
1 x 1 and configured the surface; `Renderer::resize(0, 0)` returned early and kept the
old depth buffer. The next frame's render pass had a 1 x 1 colour target and a 640 x 360
depth target. wgpu's default error handler panics on a validation error. So the table's
guess was close but not it: nothing is wrong with the Vulkan surface, the two sizes
just disagree. Sizing the window to nothing (`SetWindowPos` 1 x 1, client 0 x 0) gave
the same panic.

I can't see inside Andrew's words, but clicking the taskbar button of a window that has
focus minimises it, so "clicking out of the game tab" crashing every time fits exactly
this. Everything else I tried did not crash, on the old build or the new.

| Trigger, real window, `--captured` | Old build | Fixed build |
|---|---|---|
| Minimise (`SW_SHOWMINNOACTIVE`) | panic above | runs on; 0.02 s of CPU in 10 s |
| Size to 1 x 1, 120 x 1, 1 x 120 (client 0 x 0, 105 x 0, 0 x 82) | panic above on 1 x 1 in one run; in a later run no text, and it would not close in 20 s | runs on |
| Tiny but not nothing (client 122 x 2) | not tried | runs on, draws |
| Restore (`SW_SHOWNOACTIVATE`) | died before it got there | draws: `restored-after-minimise-and-tiny.png` |
| Minimise, restore, three times | | runs on |
| `WM_CLOSE` while minimised, real save behind it | | saves (generation 6 to 7) and exits |
| Minimised in the middle of a far flight, watched with `--replay --watch` | | runs on; the replay still reproduces |
| Lose focus with the mouse captured (posted messages) | no crash, keeps "mouse captured" | no crash, lets go |
| `WM_ACTIVATE` posted | no crash | no crash |
| Minimise, restore, 1 x 1, focus on the DX12 backend (`--backend dx12`) | not tried | runs on (`window-runs-dx12.txt`) |
| Covered by other windows | no crash | no crash |

`Occluded` is not a case on Windows: winit 0.30.13 never sends that event there
(nothing in `platform_impl/windows` mentions it), and wgpu only reports an occluded
surface on Metal. A covered window is a window drawing as usual, at the bottom of the
z-order in every one of these runs, and off the screen in most.

**The fix.** `app.rs` has a `hidden` flag: no area, no surface change, no drawing.
`frame()` still runs the steps, autosave and the photo writer, and returns before
anything touches the GPU. `about_to_wait` calls it itself and sleeps a sixtieth of a
second, so the event loop doesn't spin. The surface and depth buffer now agree from the
first frame too (`resumed` used the requested size for one and the window's for the
other). Three things waited for a frame that a hidden window never draws, and don't now:
quitting (closing it from the taskbar), `--exit-after`, and a pending photo or mark.

**The simulation behaves as it did.** A recording made through a minimise, a restore
and a 1 x 1 is 2,327 steps in 38.8 s, tick 775 on the last frame, and it reproduces
exactly (`window-runs.txt`). The same recording in the first pass was 2,154 steps in
35.9 s, with 11 s of it minimised. CPU while minimised: 0.02 s in 10 s, and nothing
measurable in 5 s; visible it was 95% of a core.

### The capture state

Before: after focus loss `captured` stayed true, so the first click back was game
input (it broke a block), and the pointer stayed hidden over the window. Evidence:
the frame at the end of a focus-lost, focus-regained run says "mouse captured" on the
old build (`capture-before-focus-lost.png`) and "click to capture mouse" on the new
one (`capture-after-focus-lost.png`). Held keys and the held mouse button were already
released, and still are. Going back doesn't take the mouse by itself: the first click
does, and that click does nothing else.

### Powder levels

**Not reproduced as a crash.** Each row is a real window on the build from before my
fixes, running 8 to 12 s with no error on stderr, unless it says otherwise.

| Launch | Result |
|---|---|
| `--powder levels`, first launch, default save, no folder there | fine; nothing saved because nothing changed |
| The same again, second launch | fine |
| The same with Austin's blocks.json (snow falls), first and second launch | fine |
| Near the north pole, standing on real snow, crosshair on it, with and without that file | fine |
| On a save holding a real pile, standing beside it (`b7-walk.play`'s save), with and without that file | fine |
| `--play` digging real snow under `--powder levels` with that file, save, reload | fine (headless) |
| **A save folder with a header and no commit** | **exits at once, window never opens** |
| `--save` naming a normal save | exits at once with the refusal |
| A normal launch on a powder save | exits at once with the refusal |

So the only launches that end at once are refusals, and they said so only on stderr.
What changed:

- `Session::load_sim` refused a header with no commit as "not a --powder levels save".
  That's a first save cut off by a crash, and with minimise crashing the game it could
  really happen, though the window is short. Nothing was ever played there, so it now
  loads as new. It launches, and a real save lands in that folder on quit.
- Refusals show a box as well as the console line (`crash::tell_the_player`), and exit
  cleanly. **I have never seen that box**: it would take focus. See "Not verified".
- `--powder-levels`, or `--powder` with nothing after it, was a Rust panic. Those are
  messages now. Andrew could have typed either.

Austin's blocks.json matters to something else: recording 210239 reproduces exactly on
the old build **only** with that file (with the committed one, the end hash differs).
So the build that recorded it had snow falling. It doesn't crash anything.

### A panic leaves a trace

`crash.rs`, installed first thing in `main`: every panic on any thread writes
`game/out/crash/panic-<UTC>.txt` with the message, the place, the thread, the command
line, the build and a backtrace, and then prints as ever. `--crash-test` panics on
purpose. `crash-file-sample.txt` is one from the real window, with symbols. (The panics
above have no symbols only because I had renamed that exe.)

### The tool

`game/tools/quiet-window.ps1`, documented in `game/README.md`. `--no-focus` opens the
window inactive and at the bottom of the z-order, and `--captured` starts as if clicked.
Each run read `GetForegroundWindow` and `GetClipCursor` before and after; the clip
never changed. Idle time was read before every run.

**One run went wrong and you should know.** In the sixth launch of the first powder
batch the game ended up as the foreground window, and my script threw and left it
running for under a minute before I killed it. Idle read 0 s just before, so someone was
at the machine. I can't say whether somebody clicked it, or Windows handed it the
foreground when another window closed. Any keys typed in those seconds went to the
game. Since then the tool sets `WS_EX_NOACTIVATE`, moves the window off the screen, and
always closes the game in a `finally`, and it didn't happen again in more than 30
launches.

It also waits now for the game to run frames: the window exists long before the game
listens to it, and on a busy machine that gap was 20 s. Two of my early runs were
invalid for that reason, and `window-runs.txt` is the whole set again with the wait.

## 2. Unbound fly speed

The wheel now runs from `MIN_FLY_SPEED`, **0.01 m/s** (a centimetre a second, for creeping
up on something; it was 1), to `MAX_FLY_SPEED`, 1e150, which isn't a limit anyone meets
(1,300 lines of the wheel from 20 m/s) but where squares would overflow. The harness's
`speed` takes the same range. Sprint is still four times the wheel.

What broke when holding the wheel:

| At | What happened | Now |
|---|---|---|
| A step longer than the planet is wide | `pos += vel * dt` came out on the far side (test: 1e7 m/s, second step). `min_r` only looks at where a step ends | A fly step is along the ground and up or down, kept apart from the start, and only the height meets `min_r`. Kept apart matters: taking the up part back out of a sum left a rounding error longer than the planet at 1e30, and straight down went anywhere (my first version did) |
| Level flight at any speed | It climbed: each tangent step is further from the centre. 3,066 m after starting at 3,000 m and 150 km; at 1e6 m/s 1.5 km a step | A step stays at its height |
| Far past the sun (my test goes to 1e25 m) | `pick` walks layers that aren't there, the rounding of a radius being a great many of them: **it hung** (the test never returned on the old code, 30 s and counting) | `pick` stops at once above the top of the world |
| Inside the sun | From reading the code, not seen: `asin` of more than 1, a NaN in the sky uniform | Clamped; the sun's disc is the whole sky |
| Past about 1e19 m | From reading the code, not seen: squares of camera-relative f32s overflow | A flight stops at `MAX_RADIUS_M`, 1e15 m: a tenth of a light year, 900,000 times the sun's distance |

`min_r` holds at any speed: `a_huge_step_down_stops_at_the_floor_and_does_not_tunnel`
goes from orbit and from 1e12 m at speeds up to 1e150 and ends on this side, 2 m above
the bottom. `scripts/c1-far-flight.play` goes past the moon (13.9 radii) and the sun
(12,190 radii) to the edge, comes back in one step at 1e18 m/s, then flies round the
planet at 1e6 m/s. No panic, no NaN (`far-flight.txt`). The frames are stars out there
(`far_3_past_the_sun.png`). The script's flight round the planet happens to dive to the
floor and stay there, 2 m above the bottom of the world, so `far_7_round_the_planet.png`
is the underside of the world, odd to look at and what that place has always looked
like. A far flight played in the real window at real time, with a minimise in the
middle of it, reproduced exactly (`window-runs-far-and-watched.txt`).

At that kind of speed a step along the ground is at most a quarter of the way round
(`atan(d/r)`), so "metres a second" stops being literal near the planet: at 1e6 m/s
it's 1% slow, and past about 1e7 it spins. There's nothing else for it to mean.

## 3. Great circles

**What it does now.** `Player::facing` is a unit vector, level with the ground. Every
step, however it moved, carries it along the move with `camera::carried`, which is the
rotation taking the old up to the new up: parallel transport, in plain arithmetic.
`step` does this after `step_fly`, `step_walk` and `step_on_slopes`, so all three are
covered. `Camera` carries the same vector, and `Controls::camera` draws the mouse
ahead by the same `look` the step will do. `heading()` and `set_heading()` remain, for
people and scripts; no step reads a heading.

**Why not a heading recomputed each step,** as the mission page suggests: that would
put `atan2` in every moving step, and a heading into and out of trig 60 times a second,
which is the new trig-dependent divergence the page asks to avoid. A vector has no pole
to be singular at either. Fewer trig calls happen per step than before: sin and cos of
the pitch in fly, and of the look when the mouse moves. A teleport keeps the compass
heading (`Player::place`) by moving the north and east components across, also without
trig. The cost is `facing` in `pose_text`: a pose is 16 fields now. A 14-field pose
from an old photo still reads, because a heading means the same as it did.

**Andrew's recording.** 210239 is 13.6 km up at -85.8, looking straight down and
sprinting at 20,000 m/s, strafing left and right. Its right vector came from the compass
heading, so it swung with the longitude and he went round the pole: lon -122 to 132 to
118 to 150 to 105. `scripts/c1-andrew-210239.play` is the same start and keys; on the old
build it gives the recording's path to the digit, and on the new one a straight line,
back and forth along one great circle, at a constant 13,605.91 m where the old one climbed
to 13,692 (`andrew-path-before.txt`, `andrew-path-after.txt`).

**What the old build did at a pole.** `great-circle-script-before.txt`: flying 150 km at
heading 45 from lat 40 ended at lat 89.96, spiralled in. Flying at the pole stuck there:
2.5 m from it after 4 s, the view flipped (`before-pole_n_4_beyond.png`; the same pose
from the fixed build is `after-`, 0.4 degrees past it and still looking the same way).
Walking north, you bounced at the pole and never got past it: the fixed build's walker
ends on the far side, on the real snow, with the same 7 s of W and jumping. South pole
likewise, on the sea ice.

**Tests** (`src/player/great_circle_tests.rs`, 22 tests). I ran them on the old code
with a shim that adds names only: **17 fail** (`before-great-circle-tests.txt`), one
hangs (the far `pick`), and 4 pass:

- `mouse_look_at_lat_89_99_turns_the_same_as_at_the_equator` **passes on the old code**.
  You asked for it to fail before, and it can't: standing still, a heading plus an angle
  always turned the right amount. What was wrong was on the move, so
  `mouse_look_on_the_move_near_a_pole_turns_by_what_the_mouse_said` is the test that
  fails before (the view swung 0.016 rad on its own in one step, 16 m from the pole).
- `a_teleport_faces...`, `set_heading_and_heading_agree`, `walking_from_far_out...`
  guard things that were right and could have broken.

Worst off-circle in the walking tests: under the 2 m they allow over a full circumference
of a 5.6 km planet, through face edges and corners. The fly tests hold to 1e-9 of a
radian. In the view-jump tests the view never turns by more than the move's own angle
(flying, to 1 part in a million; walking, 1 in a thousand), at 20 and 5000 m/s, at misses
of 0, 1e-7, 1e-4 and 0.01 degrees from both poles. The old code: 3.14 rad in one step.

## Old replays, new replays

A recording's header holds `move_rules` (missing: 1). `compatibility` refuses another
version and says what to do (`replay-210239-refused.txt`, exit code 2, markers still
listed):

> movement rules version 1 vs 2: it was recorded before W followed great circles (the
> same keys would take the player somewhere else, so it will not play here. Your world
> and saves are fine, and its markers and its report still stand; to watch this
> recording, use the build that made it, 8134c61db)

New recordings reproduce exactly: a flight over the pole with looks and scrolls at 30 and
144 fps (`a_flight_over_the_pole_replays_exactly`), the far flight, and the recordings
made through minimises in the real window. The fixed-step test passes.

No `.play` script depended on rhumb movement. The scripts did not need changing.

## Verified

- `cargo test --release`: see the numbers in my report to the lead; the run on the final
  commit is the one that counts. `great_circle_tests`, `window_tests` (the app's event
  path with synthetic events) and the new replay tests are in it.
- Every `.play` script bar the five that need an old build or are long renders:
  `plays-summary.txt` is the first pass, on a loaded machine. Of its six failures,
  `b5-menu`, `b5-menu-scale` and `b6-report` wanted `--interchange` and `--replays`, which
  my runner didn't pass, and pass with them. `m1-done` and `m2b-river` failed one timing
  assert each (`bench_fps > 60`, `tick_ms_avg < 2`) and pass on a quieter machine.
  `b7-close` fails the same way on the old build. `m1-done.play`: 94 asserts, none failed.
- The real window, every row in the first table, on both builds where the row says so.

## Not verified

- **A real focus change.** The game has never had real focus in my tests. "Lost focus"
  was posted messages, and with `--no-focus` the system clips nothing. So these need a
  person: click in, Alt-Tab away, and see that the pointer is free at once and visible;
  come back by Alt-Tab and by clicking the window, and see the first click capture the
  mouse and break nothing; click the taskbar button, and click it again. Especially
  whether `set_capture(false)` runs in time to free the clip, or Windows has done it
  first. I expect either is fine; I haven't seen it.
- **The box for a refused save.** The call compiles and is on the path, but it takes
  focus, so I have never run it. Try `--powder levels --save saves/Neisor`.
- **That any of this is Andrew's powder crash.** Worth asking him what he typed, and for
  `game/out/crash/` next time.
- The `Lost` surface and GPU device loss, sleep, the lock screen, Remote Desktop.
- A second machine. The transport is plain arithmetic, but I haven't compared replays
  across machines.
- Frame rate: no bench run, the machine was never free. Nothing in the drawing path
  changed, and a player step gains a few multiplies. Note that the bench flight's route
  now goes straight, not along a rhumb line: 3 km, too little to change a frame.
- Multi-monitor and DPI changes while running.

## For Andrew to decide

1. **Coming back to the window.** After clicking away, the mouse is yours until you click
   in the game. That one click does nothing else. The other way is that Alt-Tab back
   recaptures by itself, but then a click back on the window breaks a block.
2. **The wheel's floor.** 1 cm/s, 29 lines of the wheel down from 20 m/s. It was 1 m/s.
   One number to change.
3. **Level flight stays level now,** from any height. Flying at the horizon from orbit
   circles the planet rather than leaving it; to leave, look up or hold Space. That is
   what a great circle means, and I think it's right.

## For the lead

- **`DESIGN.md`, proposed wording:** "Holding a movement key follows a great circle,
  walking or flying. A body faces a direction carried along each move, not a compass
  heading; a heading is how people and scripts say a direction. The fly speed has no
  ceiling. Leaving the window never stops the game: minimised it draws nothing and keeps
  stepping; losing focus lets go of the mouse."
- **`b7-close.play` line 133 fails on the old build too** (`falling == 0`, 9 still in
  flight, in a column at a cube edge with 4,162 frozen updates). Not mine: same count on
  both. It's C2's side of the house.
- **A "Segmentation fault" from Git Bash**, exit 139, twice in about 70 harness runs:
  once on the old build, once on mine, both after "done in" on `c1-great-circle.play`,
  when everything had finished. A `bash.exe.stackdump` of msys frames turned up in
  `game/` on the same evening, so it may be the shell and not the game. I didn't settle
  it. Worth knowing because the harness's exit code means something.
- **`annotate::a_pass_stays_quick_with_a_lot_drawn` failed once** under load (189 ms), as
  did the two timing asserts above. They're timing, not logic.
- **`--exit-after` counts from before the renderer is made**, and a first frame has no
  stats panel (egui's sizing pass), so a shot on a busy machine can come out empty. It
  fooled me; it isn't new.
- **No new absolute layers.** The three `c1-*.play` scripts use lat, lon and metres. Two
  asserts in `c1-great-circle.play` say the ground is snow at the north pole and ice at
  the south, which T3 could change.
- **`Player::pos` is still public.** Anything that moves the player more than a little
  must use `place`, or `facing` will be stale (it's re-levelled, so wrong but not NaN).
  T1, if it moves the player: note that.

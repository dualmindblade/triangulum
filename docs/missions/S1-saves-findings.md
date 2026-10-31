S1 saves off the main thread — 2026-10-31

Worktree `E:/triangulum-work/s1-saves`, branch `s1-saves`, starting at
`42df82818619316dd700dc1678063b099f1602c4`. Changes are uncommitted for the
lead. The existing `game/.cargo/config.toml` is retained, unchanged and
ignored. All runs are headless; no real input or visible game window was used.

The design is one owned snapshot and one writer thread. The game thread
copies dirty chunks in their existing packed representation, their revisions,
the simulation including pending notifications, and a private copy of the
world/block identity. The worker expands and compresses chunks, serializes
the simulation, hashes files and runs the existing durable generation
protocol. The game thread acknowledges the captured revisions only after
joining a successful completion. Later edits stay dirty. Save format **3**,
simulation layout **4**, generator identity and simulation rules are unchanged.

There is no queue of snapshots. A busy request returns without copying;
autosave revisits the remaining dirt after completion. The snapshot payload
has a **256 MiB** bound, checked before copying chunks or simulation queues,
with a conservative allowance for registry metadata and acknowledgements.
An oversized snapshot reports an error and retains all dirt; it cannot
silently save a subset. This is a payload bound, not a process RSS bound:
the worker also needs its manifest, serialized simulation, and one chunk's
compression buffers. There is no whole-world dense copy.

`save_if_due` submits and polls. `save_now` is a barrier: join the current
generation, capture the latest state, then join that generation. The harness,
reload, recording start and terrain preview use that barrier. Recording no
longer logs a save failure and proceeds anyway. A preview with no intervening
changes leaves the saved files alone. Quit presents a waiting frame, pauses
simulation and input, drains the old write, submits the final state, then
exits on success. A failure cancels quit and stays visible, including with
stats hidden or the annotator open. Autosave retries after two seconds.

`Sim::snapshot_for_save` includes pending notifications without draining them
or changing the simulation's dirty flag. The older absorbing snapshot used
by existing simulation/recording callers delegates to the same snapshot
builder after its existing absorption. `sim/mod.rs` has only this save-copy
work and a read-only change-log read in `state_digest`; powder, water rules
and `mesh.rs` are untouched. B7 should retain the same fields in this shared
snapshot builder when merging its simulation changes.

Timing method and baseline

All timings are **provisional**, on the shared machine. This run did not
reproduce the historical one-second figure: durable writes in this worktree
took substantially longer. No idle-machine or frame-rate claim is made.
`session::save_tests::s1_main_thread_save_cost` uses the real Session autosave
entry point, exactly 400 or 2,000 dirty LOD-0 chunks, one changed voxel per
chunk, three passes (IDs 4, 5, 6), and actual durable I/O. The chunks have
a small palette and no water layer, and the simulation queue is empty;
this isolates the cost of dirty chunk count. The timer starts
after dirtying and stops when `save_if_due` returns. A separate timer includes
`save_now` as a completion barrier. The baseline used the original save path
with only the new tests added; its saves are retained under `game/out/s1-tests`.

| Dirty chunks | Before: main-thread samples (ms) | After: main-thread samples (ms) |
|---|---|---|
| 400 | 13006.051, 12667.343, 12419.644 | 3.020, 2.818, 1.883 |
| 2,000 | 62902.504, 64085.769, 63241.806 | 12.851, 12.629, 13.343 |

The observed save-request budgets are **4 ms for 400 chunks** and **16 ms for
2,000 chunks**, with worst samples of **3.020 ms** and **13.343 ms**. These
are three samples per size, not a p99 guarantee. The 2,000-chunk copy uses
most of a 16.7 ms frame by itself; rendering and other frame work are not
included. Large simulation queues, water layers and wider palettes can cost
more to copy and were not included in this timing fixture.

Afterwards, completion through the barrier still took 12.84–13.34 seconds
for 400 chunks and 65.41–67.30 seconds for 2,000. The durable I/O still
happens; it no longer occupies the requesting frame. The baseline log is
`game/out/s1-before-cost.log`, and the after log is
`game/out/s1-validation-20261031-052613/after-cost.log`.
Repeat with `cargo test --release -j 6 s1_main_thread_save_cost -- --ignored --nocapture --test-threads=1`.

Verification

Before the implementation, three new regressions failed against the original
path (`game/out/s1-red.log`): autosave returned a completed chunk count on its
requesting frame; a save drained pending work and cleared simulation dirt;
recording proceeded after a failed save. Two further regressions exposed
revision reuse on whole-chunk replacement and a reload incorrectly appearing
unsaved (`game/out/s1-edges-red.log`); both were fixed. The replacement test
uses the pre-existing EditStore API and also applies to the original code.

The added worker tests cover:

- A blocked writer while the live world and simulation change; loading the
  first generation recovers the captured moment, and the next save includes
  the later revisions. Repeated busy requests do not add generations.
- Independent water buffers and captured block identities, including adding
  a block after capture and before the worker starts.
- Worker write failure, worker panic, and failure of the directory flush
  after publication: no acknowledgement or premature “saved”; errors reach
  Session/UI state; retry succeeds. Simulation-only failures also retry.
- Real child-process kills at a partially written chunk and at a partially
  written `commit.tmp`, after the new generation's data files. Both leave
  generation 1's commit byte-identical and its chunks and simulation readable;
  a subsequent generation 2 succeeds. Only children started by the test are
  killed. These fixtures are retained, not player saves.
- Quit waits for a presented message, the old write and the last changes;
  a failed final write cancels quit. Egui output contains the waiting/error
  text without drawing the optional stats HUD.
- Save, reload, recording and the preview barrier finish the old and latest
  generations before returning. Exact per-step simulation digests and player
  positions match a session without saves, and a recording made while a save
  is blocked replays exactly.
- Rejection before copying when the payload budget is exceeded, including a
  large simulation queue with no dirty chunks, with all dirty work retained.

The existing M2 durability-order and every-operation-failure tests also pass
through the shared writer implementation. R4's manifest containment, damaged
save, block-identity, simulation-version and restore-radius checks remain
passing. The process-kill tests supplement that protocol coverage with a
real worker interrupted before publication.

The final full release run passed **324 tests, 0 failed, 24 ignored** in
465.08 seconds (`game/out/s1-final-release.log`). The stress run passed all **four** ignored stress
tests in 90.00 seconds. A read-only load of the actual pre-S1 baseline save
passed: 400 chunks, generation 6, including the simulation, from
`game/out/s1-tests/cost-400-30196-1793441866248247900`.
Logs are in `game/out/s1-validation-20261031-052613/`.
The final suite ran against the final source state. `git diff --check` passed.

All required headless scenarios passed, with **382 assertions, no failed
assertions and no errors**. Each used a fresh explicit `--save` destination
under that validation directory, `--bake C:/code/triangulum/game/assets/world/seed42`,
`--threads 4 --backend vulkan --size 1920x1080`.

| Scenario | Assertions passed |
|---|---|
| `m1-done` | 94 |
| `tunnel-wall-reload` | 18 |
| `m2a-sand` | 95 |
| `m2b-water` | 97 |
| `m2c-replay` | 29, including exact replay verification |
| `b4-slide` | 49 |

`m3a-look` also completed without errors (rendering evidence; the script
contains no assertions and does not exercise the live terrain dials).
`cargo build --release -j 6` passed. The suite commands were
`cargo test --release -j 6 -- --test-threads=4` and
`cargo test --release -j 6 -- --ignored stress --test-threads=1`.
Scripts `game/out/s1-baseline.ps1`, `s1-check.ps1`, `s1-validate.ps1` and
`s1-final.ps1` preserve the multi-step commands. The existing `unused_mut`
warning in `stream.rs` is unchanged.

Limits of this verification: no physical power cut; no visible-window Q or
window-close interaction; no idle-machine 60-fps measurement; no cross-machine
replay comparison. The quit state machine and actual egui draw output are
tested headlessly. Worlds needing a snapshot larger than 256 MiB currently
receive a save error instead of a partial or unbounded snapshot. Low-level
write/flush failures are injected; actual disk exhaustion was not induced.
Other ignored performance/survey tests were not run.

Changed files: `game/README.md`; `game/src/app.rs`, `session.rs`, `replay.rs`
(comment only), `save_tests.rs`, `workshop.rs`, `sim/mod.rs`, `world/edits.rs`, `world/save.rs`,
`world/save_worker.rs`; and this report. Scripts, logs, fixture saves and
harness output remain in the ignored `game/out/` directory.

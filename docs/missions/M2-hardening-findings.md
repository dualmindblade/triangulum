# M2-hardening: the moving edge, saves, memory (Fable, 2026-10-25)

Branch `worktree-agent-ada8dcccdde4731a1`, crate `game/`, from main at
28389f7 (M2c merged). Fixes review 3 (`docs/reviews/2026-10-25-engine-review3-astra.md`)
#3, #9, #7, #2, #1, #5, #10, #12 and review 2 #17; #8 and #11 skipped as
asked. Evidence in `docs/missions/M2-hardening-evidence/`. Every "before"
below is main's code: either the new test run against this worktree before
the fix, or the same test run in a copy of main exported to my scratchpad.

**Merged with main after the work** (4bd0f0b, M3a's look panel; no
conflicts): `cargo test --release` 150 passed, 0 failed, 14 ignored, and
all twelve play scripts pass, the nine below plus m3a-look, m3a-tunnel
(20/20) and m3a-look-vscale8 (`merged-tests-release.txt`,
`merged-scripts-summary.txt`). The numbers below were taken before that
merge.

**Timings are provisional.** Another agent's builds and two game
instances were running for the second half of the session (the baseline
numbers at its start were taken with nothing else running). Where a
before/after pair matters, both halves were run back to back.

## What changed

| finding | where | what |
|---|---|---|
| #3 invisible dam | `sim/mod.rs` `refocus`, `wake_new_pairs` | When chunks join the region, every face-neighbour pair across their sides with both cells now active is checked: if the pair could move water (both replaceable, levels differ, or the upper cell could fall into the lower) and at least one cell is not as the generator made it, **both** cells are scheduled now. That covers the donor in an already-active chunk, water held up over newly active empty space, and two chunks that rejoin together. Pairs exactly as generated are skipped, so an untouched river stays held at no cost. Sides between two unedited chunks are skipped without reading a cell; for a side with one edited chunk, the edited side is read first and the other side's natural chunk is generated only when a changed cell needs it, and the generator's footprint bounds (`classify`) answer "all rock" / "all air" without generating anything. The first region after `new`/`restore` is not compared with anything (a restored state gets back the region it was saved with), so a save and reload still simulate exactly as the continuous session. New stat `pairs_woken`. |
| #9 throughflow asleep | `sim/water.rs` `finish_cell`, `commit_water` | A cell that exchanged water with the boundary this tick and ended at its starting level is rescheduled for the next tick (`Overlay::exchanged`), so steady flow and its `water_in`/`water_out` accounting keep going. |
| #7 falling blocks | `sim/mod.rs` `advance_falling`, `land_into`; `world/mod.rs` `displace_writes_within` | A falling block never writes a chunk outside the region. Its sweep stops at the bottom of the lowest active cell; the search for a free cell above something built into it stops at the region's top; a landing whose displaced water would have to rise into an inactive cell does not happen. In each case the block waits, unchanged (no state change, no dirt), and carries on when the region reaches further, as a frozen update does. The player's own placements still displace anywhere (player edits always could). |
| #2 missing header | `world/save.rs` `SaveDir::present` | Absent save (no header, no commit): loads as the natural world. Header with or without a commit: a save. **Commit without header: refused** by `load_into`, `read_sim` (so its simulation state is not resumed) and `flush` (so a fresh header is never written over the old generation), with a message that says how to recover (and notices an interrupted `header.tmp`). `install_image` refuses such a directory too. |
| #1 durability | `world/save.rs` `SaveFs`, `OsFs`, `flush_with` | The protocol, in order: the new generation's chunk and sim files (names no committed generation uses) are written in place and flushed (`FlushFileBuffers`); then `chunks/` and the save directory are flushed; then `commit.tmp` is written and flushed and published with `MoveFileExW(MOVEFILE_REPLACE_EXISTING \| MOVEFILE_WRITE_THROUGH)` ("does not return until the file is actually moved on the disk"); then the save directory is flushed again; only then are revisions acknowledged and the previous generation's superseded files removed. Every flush failure is an error: nothing acknowledged, nothing removed, the save retried. Windows lets a directory be flushed through a read+write handle opened with `FILE_FLAG_BACKUP_SEMANTICS` (a read-only handle gets "access denied"; probed on this machine). Unix: `rename` then `fsync` of the directory, errors propagated (it used to ignore them). Header writes and save-image installs use the same durable replace. All file operations go through a small `SaveFs` trait so tests can record their order and fail any one. |
| #5 bake identity | `worldgen/bake.rs` `open`, `BakeIdentity`, `content_identity`; `main.rs`; `world/save.rs` `bake_identity` | The loader reads meta.json, each face raster and `hydro.bin` once, whole, and hashes the very bytes it parses (on a second thread while the face is parsed); `Bake::identity()` returns that digest and `main.rs` records it instead of hashing the files again afterwards. The digest definition is unchanged (meta, then name + length + contents per face, then hydro.bin), so existing saves still match: checked against the old cache's recorded value for Neisor's bake. The metadata cache (`content.sha256.json`) is no longer read or written. To pay for hashing 560 MB at every start, `sha256.rs` gained an x86-64 SHA-NI path (runtime-detected; 1.67 GB/s vs 252 MB/s portable here) and stopped copying its input through a buffer. |
| #10 PosKey | `grid.rs` `Planet::try_new`; `main.rs` | Worlds with N > 2^20 or H > 2^16 are refused with the limit named, and `main.rs` checks the world settings before it loads the bake. Body ownership: documented on `PosKey` (a `Sim` is one body's simulation; M4's vehicles get their own `Sim` and key space); the format is unchanged. |
| #12 rescans | `world/edits.rs` `Edited::wet` | A per-chunk count of nonzero levels; the layer is dropped when it reaches 0. |
| review 2 #17 memory | `world/edits.rs` `PackedIds`, `for_each_sorted`, `natural_eq`; `sim/mod.rs` `state_hash`, `refocus`; `world/mod.rs` `remember_natural` | Edited chunk ids are packed against a per-chunk palette at 4 bits (8 or 16 when a chunk holds more than 16 or 256 ids): 64 KiB to 16 KiB. The state hash visits chunks in place (it cloned every chunk) and keeps, per chunk revision and per world, whether the chunk equals the generator's (that comparison regenerates the natural chunk). Refocus wakes frozen updates by looking up the chunks that joined, not by scanning every frozen chunk. Fresh no-op chunks are no longer indexed and then unindexed. LOD-0 chunks the streamer generates are put in the natural-chunk cache, so the simulation and physics near the player rarely generate one themselves. |
| replays | `sim::RULES_VERSION`, `replay.rs` | The rules changed, so `RULES_VERSION = 2` is recorded in each replay header; a replay without it (every M2c file, version 1) or with another version is refused with "simulation rules version 1 vs 2". |
| docs | `game/README.md` | The saves section: durability, the damaged-save refusal, the bake identity and its start-up cost. |

## Evidence

### Each finding: fails on main, passes now

`cargo test --release`: **139 passed, 0 failed, 14 ignored** (main: 121 /
11; `after-tests-release.txt`, `before-tests-release.txt`). The new tests,
with what main said (`before-new-tests-on-main.txt`,
`before-vertical-dam-on-main.txt`, `before-water-tests-on-main.txt`):

| finding | test | main |
|---|---|---|
| #3 | `sim::water_tests::water_dammed_at_the_old_edge_flows_on_when_the_region_grows`: a trough half in each of two chunks, water at rest in the active half, then the region grows over the edge | fails: "the water flowed on across the old edge: 0 units in the far half". Now 1,515 units cross (half of 3,060) and it settles in 41 ticks, conserved, at rest by Andrew's rule |
| #3 | `water_dammed_at_an_old_edge_flows_on_when_both_sides_return_together`: the same, but the player leaves and both chunks rejoin in one recompute | fails: 0 units |
| #3 | `water_held_up_by_the_old_edge_falls_when_the_region_grows_below_it`: water on the region's floor over empty cells | fails: "left: 0, right: 2295" (none reached the floor) |
| #3 | `growing_the_region_over_untouched_water_wakes_nothing`: the region grows from one chunk to 112 m over the sea | passes on both (the guard: 0 updates run, 0 chunks edited) |
| #9 | `a_steady_throughflow_at_the_boundary_keeps_flowing`: the review's case, a level-100 cell between a reservoir at 200 and a drain at 50 | fails: "(50, 50) vs (1000, 1000)": one tick of flow, then asleep. Now 50 in and 50 out every tick |
| #7 | `sim::tests::a_falling_block_waits_at_the_edge_of_the_active_region`: sand at terminal speed 0.1 layer above an inactive chunk; and sand inside a block with the only free cell above the region | fails: "nothing landed in the inactive chunk: left 4 (sand), right 0". Now it waits at z = the edge, unchanged for 5 ticks, and lands on the shelf when the region reaches down; the second case waits, then settles once the cell above is active |
| #7 | `a_landing_whose_displaced_water_would_leave_the_region_waits`: a water column crossing into an inactive chunk | fails: "no water pushed into the inactive chunk: left (8, 255)". Now the landing waits, and goes through (displacing 255 units) when the region grows |
| #2 | `world::save::tests::a_committed_save_without_its_header_is_refused_not_loaded_empty` | fails: "must not load: Ok(0)" |
| #1 | `a_save_makes_its_new_generation_durable_before_it_lets_go_of_the_old_one`, `a_failure_at_any_step_of_a_save_keeps_the_previous_generation` | new: main has no seam to observe or fail its file operations. The order recorded: `write chunks/…g2.chunk` ×2, `write sim.g2.bin`, `sync_dir chunks`, `sync_dir .`, `write commit.tmp`, `replace commit.tmp -> commit.json`, `sync_dir .`, `remove …g1.chunk`, `remove sim.g1.bin`. Each of the 10 operations is failed in turn: the save reports it, acknowledges nothing, removes nothing, and the save on disk loads whole as generation 1 (or 2, only when the flush after the publication failed); a failed removal leaves garbage and the save stands; the next save then succeeds |
| #5 | `world::save::tests::bake_identity_follows_contents_even_when_size_and_time_match` | fails: identical digests `d49928a0…` after a raster was rewritten with other bytes and its time put back |
| #5 | `worldgen::bake::tests::the_identity_is_hashed_from_the_bytes_that_were_loaded`: the files change after the load; the loaded bake keeps its terrain and its identity, a new load gets both of the new bake; a damaged hydro.bin is refused | new API. On main the order itself was the bug (`main.rs`: `bake::open`, then `bake_identity` on the paths) |
| #5 | `the_real_bake_keeps_the_identity_saves_recorded` | Neisor's bake: `90735e72…`, equal to the old cache's digest combined with hydro.bin as before, so saves made before this change still load |
| #10 | `grid::tests::worlds_past_the_simulation_key_are_refused` | fails: "N = 2,097,152 was accepted". From the command line: main starts `--n 2097152` (player at i = 1,714,495, past the key's 20 bits; `before-n2097152.txt`); now "error: world settings: N = 2097152 columns per face edge is more than the simulation's position key holds: at most 1048576" before the bake loads (`after-n2097152.txt`) |
| #12 | `world::edits::tests::emptying_water_cell_by_cell_does_not_rescan_the_chunk`: draining a full chunk one cell at a time | fails: 222 ms (the check is < 60 ms). Now 6 ms |
| #17 | `packed_ids_hold_what_was_written_through_every_width` | new: 60,000 random writes against a dense model, through both widenings |
| replays | `replay::tests::a_replay_recorded_under_other_simulation_rules_is_refused` | new |
| SHA-NI | `sha256::tests::the_sha_instructions_agree_with_the_portable_code` | new: both paths, every length from 0 to 300 bytes and around 4 KiB, whole and in pieces |

The old M2c recording `docs/missions/M2c-evidence/m2c-river.replay` is
refused by this build: "simulation rules version 1 vs 2"
(`m2c-replay-refused.txt`). For the record, I forced a copy of it to claim
version 2 and it reproduced exactly under the new rules (8 of 8 checks,
same end hash as on main; `m2c-replay-forced-to-new-rules.txt`,
`m2c-replay-on-main.txt`): that short session never meets the changed
cases. The refusal is the conservative choice you asked for.

### Play scripts

All nine pass on this branch, as on main (`after-scripts-summary.txt`,
`before-scripts-summary.txt`, logs `after-*.log`): m1-done 94/94,
tunnel-wall-reload 18/18, edge-corner-flight 17/17, m2a-sand 95/95,
m2b-water 97/97, m2b-boundary 44/44, m2b-river 41/41, m2c-replay 29/29,
m2c-video-driver 21/21. m2b-river's flow through the one-chunk region is
unchanged: 47,556 units in, 44,896 out over 400 ticks, both builds.

### Worst ticks

| run | main | this branch |
|---|---|---|
| m2b-water, worst tick of the script (the shore pit filling from the sea, ticks 255-455) | 7.22 ms | 7.40 ms |
| m2b-water, the tick after teleporting to the shore (the whole region changes; the pair scan) | 0.67 ms | 2.1 ms (an early version of the scan took 42 ms here: it generated natural chunks for every neighbour of an edited chunk; see "decisions") |
| dam break, 10,000 cells, worst / average moving tick (main at the start of the session, quiet; this branch later) | 7.51 / 4.88 ms | 7.03 / 4.65 ms |
| the same, back to back while another agent was running | 16.2 / 5.21 ms | 23.9 / 5.22 ms |
| large pour, 2,000 cells, worst tick (back to back) | 4.32 ms | 15.7 ms |

The worst ticks of the back-to-back pair are interference (the averages
are the same, and the closed-water results are identical to the unit:
65,472,480 and 8,771,353 units moved on both builds). The dam break's
one-time notification of 10,000 placed cells: 16.3 ms on main, 15.8-15.9
ms on this branch in the quieter runs (26 ms in the loaded pair).

### Memory and the cost of edits piling up (review 2 #17)

`sim::tests::edit_memory_and_costs_by_edited_chunks` (ignored; the
process's private bytes, `before-edit-memory.txt`, `after-edit-memory.txt`):

| edited chunks | dry, main | dry, now | wet, main | wet, now |
|---|---|---|---|---|
| 100 | 57 KiB each (6 MB) | 11 KiB (1 MB) | 87 KiB (9 MB) | 40 KiB (4 MB) |
| **1,000** | **64 KiB (66 MB)** | **17 KiB (17 MB)** | **96 KiB (98 MB)** | **51 KiB (53 MB)** |
| 10,000 | 65 KiB (669 MB) | 18 KiB (182 MB) | 97 KiB (992 MB) | 52 KiB (527 MB) |

The store's own count is 16 KiB of ids per chunk plus 32 KiB of levels per
wet chunk. Other costs at the same sizes:

| | main | now |
|---|---|---|
| state hash, 1,000 chunks (dry / wet) | 334 / 480 ms | 148 / 176 ms, again 120 / 144 ms |
| state hash, 10,000 chunks | 22 s, again 25 s | 18 s, again 1.2-1.5 s |
| crossing a chunk border with 100 / 1,000 / 10,000 frozen chunks | 0.55 / 0.56 / 0.67 ms | 0.56 / 0.56 / 0.57 ms |
| the tick that freezes 70,000 updates for 10,000 chunks | 20 ms | 19 ms |
| making a chunk edited (its first write) | 1.6-2.2 ms | 1.6-1.7 ms |

The first hash after many edits still regenerates every natural chunk it
has not compared yet (1.7 ms each); after that it only re-compares chunks
that changed. Refocus is now flat in the number of frozen chunks; its 0.55
ms is the region itself. The first write to a chunk costs a natural-chunk
generation; near the player the streamer's chunks now feed that cache.

**What a real fix (paging edits to disk) would take** (not built):

1. A resident set: the chunks in the active region, the streamer's LOD-0
   ring and every dirty chunk stay in memory; clean chunks outside it are
   dropped, since the save already holds them. Without a save directory
   (`--no-save`, the harness) a spill file would hold them instead.
2. A store that knows a chunk is edited without holding it: keep the key
   set and the ancestor index; `block_at` and the water rule's reads page
   a chunk in from its committed file (digest-checked, as loading does),
   outside the store's write lock, with the active region prefetched so
   the tick never waits on the disk.
3. Coarse LODs: `overlay_coarse` reads every edited chunk under a coarse
   chunk, which would page in thousands for a distant view. Each chunk
   would need a small summary kept resident (per coarse voxel: the topmost
   edited solid, all-dug, water) so far LODs never touch the chunk itself.
4. The state hash and replay digests: use the per-file SHA-256 that
   `commit.json` already records for clean chunks, and the content
   comparison for resident ones.
5. Results stay deterministic (the content is the same on disk or in
   memory); only latency changes. Estimate: two or three days, in
   `edits.rs`, `save.rs`, `world/mod.rs` and the replay digests. A cheaper
   step first: hold cold chunks compressed (`Chunk`, a few KiB) and
   unpack on write.

### Saves (review 3 #1)

`world::save::tests::save_cost_by_dirty_chunks` (ignored), main
(`before-save-cost.txt`) against this branch (`after-save-cost.txt`):

| dirty chunks | main | now (two runs) |
|---|---|---|
| 1 | 16-19 ms | 15-18 ms |
| 100 | 294-320 ms | 197-254 ms |
| 400 | 1,058 ms | 768-1,072 ms |

The durable protocol costs no more than the old one, because chunk files
no longer go through a temporary file and a rename each (their names are
never in use). On this SSD a `FlushFileBuffers` is about 1.6 ms per small
file, a write-through rename 1.2 ms, a directory flush 1 ms. Note that a
save runs on the main thread, so 400 dirty chunks is about a second's
hitch, before and after.

### Bake identity and start-up (review 3 #5)

"bake loaded in 0.30 s" on main (the identity then came from the cache)
becomes "bake loaded and hashed in 0.54 s": about 0.25 s more per start
on this Ryzen 7 2700X, which has SHA instructions. On a CPU without them
(Intel desktops before 2021, for one) hashing runs at about 250 MB/s:
about 2 s more per start. Faster would need a different digest (one per
face, hashed in parallel), which would make every existing save's
recorded bake digest wrong; I kept the definition.

### Bench (provisional: another agent was running)

The fixed 60 s flight, main's binary and this branch's back to back, with
two of another agent's game instances running the whole time
(`bench-main-results.md`, `bench-after-results.md`):

| | avg fps | 1% low fps | worst frame | CPU avg / p99 | GPU avg / p99 |
|---|---|---|---|---|---|
| main | 992 | 289 | 6.97 ms | 0.41 / 0.85 ms | 0.37 / 0.41 ms |
| this branch | 978 | 286 | 10.9 ms | 0.42 / 0.85 ms | 0.37 / 0.41 ms |

Within noise of each other and far inside the 60 fps target (16.7 ms).
LOD-0 generation in the workers (which now also fills the natural-chunk
cache) has the same median, 4.0 ms. Nothing in rendering changed; the one
worse frame is not something I can attribute with the machine shared.

## Not verified

- **A power cut.** I tested the order of the save's operations and every
  failure point, and that the Windows calls work on this machine
  (write-through rename and directory flush both succeed on NTFS). That
  they make the save durable through a power cut is Microsoft's
  documented behaviour, not something I could test. In particular I rely
  on NTFS putting a new file's directory entry on the disk when the file
  and its directory are flushed before the commit is published.
- **The Unix path** (`rename` + `fsync` of the directory) was not compiled
  or run here.
- **The portable SHA-256** is tested against the SHA-NI path on this CPU,
  but no run here was on a CPU without SHA instructions, so the 2 s figure
  is the measured portable throughput applied to 560 MB, not a start-up I
  watched.
- **The pair scan while walking through a heavily edited area.** Measured
  only in the scripts (a teleport into an edited shore: 2.1 ms tick) and
  the tests. Each chunk that joins costs a look at the sides it shares
  with edited chunks, and at most one natural-chunk generation per edited
  chunk.
- **Replays across this change.** Every M2c replay is now refused, including
  ones that would reproduce (the one I tried did).
- **The bench and every timing above** were taken with another agent's
  builds and game instances running for the second half of the session.
- A save left with garbage files after a failed flush that came after its
  publication: harmless (loading ignores them) but never cleaned up.

## Feedback

- The review's line numbers and fix shapes made this fast: writing every
  new test first against the unchanged code gave the "fails on main"
  column directly, and exporting main into the scratchpad covered the
  tests that needed a second copy.
- The first version of the #3 fix was correct and passed every test and
  script, but turned a 0.7 ms teleport tick into 42 ms. Only the play
  scripts' tick cost showed it. Worth keeping "worst tick before and
  after" in every simulation mission.
- The worktree guard refused Python and Bash commands that wrote files
  through heredocs or ran tar with a drive path; script files in the
  scratchpad worked every time, as the last mission found.
- Two other game instances and a build were running for the second half
  of the session, so the timing comparisons are back-to-back pairs rather
  than clean numbers.

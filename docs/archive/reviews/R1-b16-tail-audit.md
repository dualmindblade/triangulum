# R-1: audit of the B-16 residual-tail conclusion

Date: 2026-07-26  
Audited revision: `4f727dd`  
HUNT 4 resolution: `3fe7564` / `120995b`

## Verdict

**Partially supported, with high confidence.**

HUNT 4 supported the narrower conclusion that the observed admission freeze
was broken: after HUNT 3, the ledger's missing set changed from a stable
`~445` to `447 -> 399`, and the eight formerly refusing L11 keys were finally
seen as admitted/pending. The source also contains the three intended changes:
coarsest-first `covered_missing` order, two extra gap admissions beyond both
the live and productive caps, and wanted-set protection in the gentle VRAM
shed (`viewer/src/renderer.rs:2487-2554`, `3830-3870`).

HUNT 4 did **not** support the exclusive conclusion that everything remaining
was intrinsic build cost rather than scheduling. That was an inference. In
particular:

- The HUNT 4 instrumentation logs the complete pending set, the wanted and
  uncached counts, and only the first four sorted `covered_missing` keys every
  300 frames (`renderer.rs:2330-2369`, `2503-2516`). It does not log admission
  time, Rayon worker start, build finish, renderer receipt, first exact draw,
  or eviction. No raw HUNT 4 transcript is archived in the repository.
- `tile_pending` is set **before** `rayon::spawn` (`renderer.rs:2543-2552`).
  Therefore "pending for seconds" conflates time waiting in Rayon's queue with
  time executing `build_tile_at_season`; it is not a build-duration
  measurement.
- In a same-season BALANCED hold after the teleport burst, at most three
  ordinary productive jobs plus two gap-reserve jobs can be pending
  (`stream_caps(1).productive == 3` at `renderer.rs:86-92`; the `+ 2` test is
  at `2531-2537`). A literal reading that all `~405` L11 gaps were
  simultaneously admitted/pending is impossible.
  A charitable reading is that the formerly refusing head/core keys rotated
  through pending over time. That proves admission progress, not the cause of
  their residence time.
- Forecast admission runs before gap admission and may retain three productive
  slots even at a held pose (`renderer.rs:2371-2480`). Thus only two slots are
  guaranteed to serve the current selected gaps. More cores beyond those
  admitted jobs cannot help. This is an intentional scheduling policy, but it
  is still part of the tail's throughput.
- The ledger's own requested verification was "uncached must drain to 0 ...
  and draws reach 1540." Neither HUNT 4's archived evidence nor V-1 demonstrates
  that endpoint.

Accordingly, these are the defensible dispositions:

| HUNT 4 proposition | Audit result |
|---|---|
| The original finest-first/budget/cap starvation was real | **Proved** by HUNT 2 and the HUNT 3 before/after drain |
| The formerly refusing keys can now enter pending | **Supported** for the observed replay |
| The three edited code paths changed as intended | **Supported**, except that wanted-set eviction protection is incomplete; see below |
| Every visually important gap is admitted promptly | **Not measured** |
| Time pending is time building | **Not measured** |
| The tail will monotonically reach zero in minutes | **Not measured** |
| Only B-4c build optimization can improve the tail | **Unsupported** |

The most accurate closure at HUNT 4 would have been: **"the observed admission
deadlock is removed; the remaining long tail is throughput-limited somewhere
between admission and exact draw, with build cost a leading hypothesis."**

The V-1 run is especially relevant because there is no production or harness
diff in `renderer.rs`, `terrain.rs`, `voxel.rs`, `planet.rs`, or
`examples/play.rs` between `120995b` and current HEAD. It measured the same
post-HUNT-4 mechanism, not a later scheduler.

## The more-powerful-machine report

Austin's report does not currently contradict HUNT 4, because that machine has
not run the latest fixes. It is evidence about an unidentified older binary.
The first controlled rerun must record the Git revision and executable hash;
an interactive screenshot should also retain its sidecar's `build` value.
`TRI_BUILD` is a compile-time stamp in the window app
(`viewer/src/main.rs:704-710`), not a runtime timing probe, and the play
harness's state sidecar does not currently include it.

Even on the latest revision, "more powerful" does not imply that this tail must
be faster:

- Tile construction is CPU work in one Rayon task per tile
  (`renderer.rs:2547-2552`; `terrain.rs:1726-2240`). A faster GPU does not
  execute it. More than five available cores do not increase the BALANCED
  productive-pending cap, and only two slots are reserved for current gaps.
  Per-core CPU speed and memory behavior matter more than total core count.
- The replay performs a fixed 60 draws per simulated second and waits for the
  GPU after each draw (`viewer/examples/play.rs:803-834`). A faster GPU advances
  the replay through simulated time faster, giving Rayon **less wall time per
  curve second**. V-1 measured `sim_per_wall = 0.73` at B-16, so its 30 seconds
  of simulated time actually gave builders about 40 seconds of wall time.
- Several protections are frame-count based: the count eviction cutoff is 120
  frames, gentle protection is 180 frames, and teleport burst is 150 frames
  (`renderer.rs:3792-3796`, `3837`, `2313`). At a higher render/monitor rate
  those windows are shorter in wall time. The interactive renderer uses
  `AutoVsync` (`viewer/src/main.rs:2037`), so a high-refresh display is a real
  confound, not just a benchmark detail.
- Revision, `TRI_STREAM`/F9 mode, exact recorded flight, weather time
  (`555.111`), world assets/seed, background CPU load, power/thermal state,
  and monitor refresh must all match. The original B-16 trigger also depends
  on accumulated flight state; V-1 starts the pose cold.

Two suggested confounds do **not** increase the B-16 wanted tile count in the
current implementation. `select_tiles` receives camera position, planet
radius, a fixed error target, and optional voxel focus; it receives no viewport
size, resolution, FOV, look direction, or draw-distance setting
(`viewer/src/terrain.rs:164-207`; call at `renderer.rs:1823-1825`). At the same
3.05 km pose, a larger or higher-resolution window therefore does not request
more terrain tiles. It can still change GPU frame time and hence wall time per
scheduler frame. Also, 3.05 km is above `VOXEL_MAX_ALT_KM = 2.5`, so the B-16
hold does not build voxel chunks (`renderer.rs:21`, `2727-2732`). Different
lower-altitude symptom sites can have a `--patch`/chunk-load confound.

**Conclusion on the new datum: indeterminate.** It is a useful reason to run
the controlled replay, but it is neither a refutation nor confirmation.

## What the V-1 curve says

The run-2 raw values are:

| Sample | Mean residual | Fraction with max-channel delta > 6 | Live draws |
|---:|---:|---:|---:|
| 0.5 s | 3.420 | 0.0791 | 1141 |
| 25 s | 3.367 | 0.0788 | 1252 |
| 30 s | 1.964 | 0.0461 | 1489 |
| settled | 0 | 0 | 1543 |

Source: `docs/missions/V1-evidence/run2-report.json`. The prose findings and
the V-1 commit message say the final live draw count is 1432; the archived raw
report says **1489**. The raw report should be treated as authoritative unless
the missing original run log establishes that it was copied from a different
render.

The shape is repeatable across the two archived curves. Through 25 seconds,
run 1 falls only `3.420 -> 3.362` and run 2 only `3.420 -> 3.367`; changed-pixel
fraction is likewise essentially fixed. Run 1's wall timestamps show that
0.5-to-25-second plateau lasted **33.51 seconds of wall time**. The final
25-to-30 interval spans 6.66 wall seconds, so the samples do not establish an
instantaneous event: the improvement occurred somewhere inside a sparsely
sampled five-second interval.

This is not the shape predicted by a simple homogeneous model in which
screen-relevant tiles complete continuously at a stable build rate. In run 2,
111 additional draws have essentially zero pixel effect before 25 seconds,
then 237 draws and most of the measured improvement arrive in the last
interval. It does not disprove expensive builds: a cohort of similarly costly,
visually dominant tiles can finish together. It does disprove using "draw count
is increasing" by itself as evidence that the visible symptom is progressively
draining.

There are three source-supported ways to obtain this shape without a worker
deadlock:

1. **The "visible-gap" list is not visibility filtered.** `select_tiles`
   selects a horizon-wide radial set. Within a level, `covered_missing` is
   ordered by distance from `camera.local_direction()`—the sub-camera radial
   point—not `camera.look_dir()` or projected screen error
   (`renderer.rs:2486-2501`; `camera.rs:49-50`, `70-73`). At the B-16 pitch of
   -68 degrees, those are different directions. The 405 gaps are also mostly
   the same L11 level, so "coarsest first" does not order them by visual damage.
   Early completions may be off-screen or already close to their ancestor.
2. **Ancestor-family release is a barrier.** If any selected child is missing,
   a cached ancestor is added and all selected descendants under it are
   suppressed (`renderer.rs:2601-2715`). Several child completions can have no
   draw/pixel effect; the last missing child releases the parent and exposes
   the family at once. This naturally makes draw count and pixels step.
3. **Completions are drained in batches between samples.** A stationary frame
   consumes every ready channel item before re-selection
   (`renderer.rs:2013-2040`), while V-1 has no samples between 25 and 30
   seconds.

V-1's hypothesis—early arrivals mostly affect places where the image was
already close while the worst gaps fill late—is therefore consistent with
both the pixels and the source. It is **not** evidence that the HUNT 3 priority
is working as intended. Rather, it exposes the gap between the intent
("visible blur first") and the implemented proxy (horizon-wide, coarsest, then
sub-camera radial distance). Confidence: high for the proxy mismatch, medium
for it causing this particular cliff until per-key screen coverage is traced.

One further comparison argues against calling the delay purely intrinsic tile
cost. In run 1, the entire reference process—including startup, the complete
settled build, and all 15 reference captures—finished in **16.1 wall seconds**.
The live process took 45.8 seconds and was still not exact. The ref phase's
first shot builds the selected missing set as an unrestricted Rayon
`par_iter`, while the live phase applies forecast/admission caps. This does not
make the builds cheap, but it proves that scheduling/concurrency and live
render contention materially determine observed convergence time.

## Decisive measurement

### Why the current probes cannot close the question

A current-binary run with `TRI_STREAM_DEBUG=1` is worthwhile and can falsify a
clean scheduler—for example, if the first sorted gap remains absent from the
pending list while a productive slot is free. It cannot prove build cost:
`pending` begins before worker start, completions have no timestamps, and the
1500-count eviction path is silent. `TRI_GPU_TIMERS` measures the GPU render
pass, not CPU tile construction. `TRI_LOD_RING_DEBUG` false-colors the frame
and therefore must not be enabled in the primary pixel run. A rebuild with
diagnostic-only lifecycle logging is unavoidable for a decisive attribution.

Extend `TRI_STREAM_DEBUG` to emit one JSONL event per key with one monotonic
wall-clock origin:

- `missing`: first frame missing, `covered`, current gap rank, `on_screen`, and
  approximate projected pixel area;
- `admit`: source (`forecast`, `covered_gap`, `parent`, or `stale`), pending
  totals, and productive totals;
- `worker_start` immediately inside the Rayon closure;
- `worker_done` after `build_tile_at_season`, including build milliseconds,
  vertex/index counts, and mesh bytes;
- `received` and `first_exact_draw`;
- `evict` from **all** paths, with reason (`count1500`, `gentle`, or `hard`),
  whether the key is in `live_wanted`, and its last-used age.

Also emit a one-second summary of `wanted`, `uncached`, `covered_missing`,
on-screen missing, gap-pending, gap-unadmitted, exact-selected draws, and
productive workers started but not done. The relevant insertion points are
`renderer.rs:2450-2554`, `2013-2040`, `2606-2717`, and `3792-3924`. This is
observability only; it must not change ordering or caps.

Then run the **accumulated-session** replay, not just V-1's cold pose. On the
other machine, from `viewer/`, use a current checkout and keep the primary run
at BALANCED/1280x720:

```powershell
$r1Rev = git rev-parse HEAD
cargo build --release --example play

$r1Dir = "interchange\runs\r1-b16-$((Get-Date).ToString('yyyyMMdd-HHmmss'))"
New-Item -ItemType Directory -Force -Path $r1Dir | Out-Null
$r1Script = Join-Path $r1Dir 'b16-live-long.play'
Copy-Item -LiteralPath 'scripts\b16-live.play' -Destination $r1Script
Add-Content -LiteralPath $r1Script -Value @(
  'log R1 long hold begins'
  'bench 18000'
  'shot r1_after_long_hold'
)

$env:TRI_STREAM = '1'
$env:TRI_STREAM_DEBUG = '1'
$env:TRI_POSE_LIVE = '1'
$env:TRI_RAW_CAPTURE = '1'
$env:TRI_GPU_TIMERS = '1'
Remove-Item Env:TRI_LOD_RING_DEBUG -ErrorAction SilentlyContinue

$r1Rev | Set-Content (Join-Path $r1Dir 'revision.txt')
git status --short | Out-File (Join-Path $r1Dir 'revision.txt') -Append
git diff -- 'src/renderer.rs' 'examples/play.rs' |
  Set-Content (Join-Path $r1Dir 'trace.patch')
Get-FileHash '.\target\release\examples\play.exe' |
  Out-File (Join-Path $r1Dir 'revision.txt') -Append
& '.\target\release\examples\play.exe' $r1Script `
  --out $r1Dir --size 1280x720 --exagg 1 --patch 1 2>&1 |
  ForEach-Object { '{0:o} {1}' -f (Get-Date), $_ } |
  Tee-Object -FilePath (Join-Path $r1Dir 'console-timed.log')
```

Record CPU model/core count, GPU/driver, display refresh, and whether the
interactive symptom was seen with F9 mode 0/1/2. Preserve the output directory,
especially the timed console log, `run.log`, raw shots, and JSON sidecars.

The result has a clear fork:

- **Build-cost dominated:** every high-projected-area gap is admitted within
  one available scheduler frame; `admit -> worker_start` and
  `worker_done -> first_exact_draw` are negligible relative to
  `worker_start -> worker_done`; productive slots remain occupied; no wanted
  key is evicted/re-admitted; and uncached/on-screen-missing falls as those
  builds finish. Then HUNT 4's mechanism is substantially correct. If only two
  screen-gap workers run while additional cores are idle, report the result as
  **build cost under the two-slot scheduler cap**, not "not scheduling."
- **Scheduling still involved:** a high-impact gap spends material time
  unadmitted, admitted-but-not-started, done-but-not-drawn, ranked behind
  off-screen work, or evicted/rebuilt; or productive workers are below the cap
  while an on-screen backlog exists. The dominant interval identifies the
  exact owner: admission policy, Rayon queueing/shared-pool contention,
  landing/swap, visibility ranking, or eviction.

For each on-screen key, compute
`T_total = first_exact_draw - first_missing` and partition it into
`T_unadmitted`, `T_queue`, `T_build`, and `T_land/swap`. That partition—not a
pending count or aggregate draw curve—is the measurement that settles the
claim.

## Additional findings

### 1. Wanted-set eviction protection is incomplete

Confidence: high for the code defect; medium that it contributes to B-16.

Before byte-budget enforcement, `evict()` still performs:

```rust
if self.cache.len() > 1500 {
    self.cache.retain(|_, t| t.last_used >= cutoff);
}
```

(`renderer.rs:3792-3796`). This path does not check `live_wanted`. The HUNT 2
change protects wanted keys only in the later gentle VRAM shed
(`3841-3857`). The B-16 settled draw count is 1543 and the wanted set is about
1540, so this threshold is directly in range.

The ancestor-family rule suppresses cached siblings from the drawn set while
one child is missing. Suppressed siblings do not refresh `last_used`; after 120
frames the count sweep can drop them even though they remain wanted. A slow
final child can therefore land after its siblings were discarded, perpetuating
the ancestor stand-in. The count sweep emits no eviction-storm log, so HUNT 4's
current probes would miss it. This is a concrete reason the decisive trace must
log every eviction path. It is not proved active in either archived V-1 run.

### 2. "Heavyweight frozen-water L11" is not yet an attribution

Confidence: high that it was not measured; low on the actual cost center.

At 3.05 km, only terrain tiles apply. Every terrain tile builds the same base
33-by-33 grid plus ghost ring and skirts (`terrain.rs:1726-2005`). The frozen
open-water branch changes color/payload (`1885-1908`); it does not visibly add
a special ice mesh. L11 can still be expensive through terrain/hydrology
sampling and impostor candidate enumeration, including candidates later
rejected by `smp.has_water()` (`2019-2108`), so an ice-region cost is plausible.
There is simply no per-key timing to establish it, nor evidence tying it
specifically to B-4c's dense-forest cold-build center. Redirecting all remaining
work to B-4c was premature until `worker_start -> worker_done` is attributed.

### 3. The convergence gate measures a window, not eventual convergence

The current tail-ratio logic gives archived run 2 about `0.915`, so it passes;
the same curve truncated before the final sample is approximately flat and
flags STUCK. That is an honest observation-window rule, but it cannot prove
"minutes, not never." V-1 also explicitly states that its cold-start lane does
not cover the original accumulated-session trigger. The gate is useful for
regression detection; it is not evidence that B-16's original end condition
has been reached.

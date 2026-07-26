# B-4a v2 findings — candidate cache corrected for first-touch cost

Date: 2026-07-14  
Base: `c2bf501`  
Reverted implementation diagnosed: `ecbd6fc`

## Verdict

B-4a v2 meets the mission gates. The headline productive-forest 2 km p95 is
**2,977.92 ms -> 55.54 ms** (median of three final cold-process runs), a
**98.1% reduction / 53.6x speedup**. The first three horizontal move p95s are
**730.87/53.25/142.33 ms -> 19.49/13.93/17.15 ms**. No step in either standing
probe is worse than the locally measured `c2bf501` baseline.

The v1 regression was not an inherent cost of caching. It was caused by doing
the wrong unit of work on a miss: a request for 374,544 lattice sites built all
four LOD tiers of four level-14 regions, walking 1,490,841 tier sites and
evaluating 53,460 expensive vegetation profiles. The direct loop evaluated
only 4,056 profiles. The v1 `OnceLock` then made every worker wait for that
83 ms monolith.

The replacement keeps the request's exact comp-scaled lottery as the first
gate, caches only sparse expensive profiles (including negative profiles),
computes misses outside locks, and merges them into sharded bounded storage.
The renderer warms exact future candidate slices without constructing
off-screen terrain, bounds live terrain and voxel work, uses cached parents as
the safe live fallback, and leaves settled capture behavior unchanged.

## Measurement method

- All timings are release builds on the same machine and populated asset set.
- The pre-change tables were measured at `c2bf501` before restoring v1.
- Step tables report p95 milliseconds. Final forest and lateral values are the
  median p95 from three fresh `play.exe` processes; their baseline is the
  mandated fresh baseline run. The fast descent summaries also use three
  final processes. Ascent/fly and perf-reel rows are single standing runs.
- The user's existing modified `weather_tuning.json` was left untouched and
  was present for both baseline and final runs.
- `perf_reel.py` was run without `--accept` and compared here to the locally
  captured pre-change table because this worktree has no accepted perf JSON.

## Mandatory first-touch diagnosis

The productive specimen is face 3, level-14 tile `(11534,8052)`, columns
`7039794..7040405 x 4914550..4915161`. Its inclusive lattice intersects four
level-14 cache regions.

### Work performed for the same request

| Metric | Old direct loop | v1 region-tier cache | v2 sparse request cache |
|---|---:|---:|---:|
| Requested lattice sites | 374,544 | 374,544 | 374,544 |
| Whole region-tier sites front-loaded | 0 | 1,490,841 (3.980x request) | 0 |
| Sites passing request lot gate / expensive profiles | 4,056 | 53,460 | 4,056 cold; 0 new on hit |
| Profile over-evaluation vs direct | 1.00x | 13.18x | 1.00x cold |
| Exact retained candidate stream | 2,998 | 2,998 | 2,998 |

The direct request rejects 370,488 of 374,544 sites (98.917%) with
`lot >= MAX_TREE_DENSITY * comp` before climate/profile work. V1 forfeited
that LOD-specific gate by materializing request-independent tiers and paid for
the most permissive populations of every stride.

### First-touch time and urgent-path share

| Candidate phase | Median / wall ms | Expensive work synchronous in urgent tile |
|---|---:|---:|
| Old direct request | 6.493 | 6.493 ms |
| v1 cold region tiers | 83.143 | 83.143 ms (100%) |
| v1 four concurrent callers | 84.842 wall | all callers convoyed at about 83–86 ms |
| v2 cold exact request | 6.768 | 6.768 ms if no forecast reaches it |
| v2 after cache-only forecast | 1.581 | 1.581 ms lookup/filter; **0 new profiles** |
| v2 four concurrent cold callers | 7.970 wall | 6.71–7.34 ms/caller; no monolithic waiter |

V2 cold cost is 4.2% above the old direct candidate phase, but it is bounded
to the requesting slice. The live 2 km forecast pays that 6.768 ms before the
urgent handoff; the later urgent tile retains only the measured 1.581 ms exact
lattice/filter lookup.

### Hypothesis verdicts

| Hypothesis | Verdict | Evidence |
|---|---|---|
| 1. LOD-independence loses the comp-scaled lot gate | **Verified; primary** | 4,056 direct profiles vs 53,460 v1 profiles (13.18x). |
| 2. Region-tier granularity front-loads later tiles | **Verified; primary** | 1,490,841 tier sites for a 374,544-site request (3.980x). |
| 3. Productive-region proof attempts dominate | **Refuted** | Tile proof 0.023–0.043 ms; all four region proofs together 0.065–0.074 ms, under 0.1% of the v1 miss. |
| 4. `OnceLock` convoy | **Verified; secondary** | Four v1 callers all completed at about 83 ms and wall time was about 84 ms. V2 wall time is 7.970 ms with work outside the profile lock. |

The end-to-end v1 run agrees with the microprobe: the forest 2 km step was
2,977.92 -> 4,032.43 ms (+35.4%), while the first lateral move was
730.87 -> 989.84 ms. This is the same class as the field 3,165 -> 4,663 ms
regression.

## Chosen design

### Sparse, request-scaled profile cache

- Each request walks only its exact lattice and applies its own
  `MAX_TREE_DENSITY * stride^2` lottery before profile lookup/evaluation.
- A cached `LeanImpostorProfile` is at most 16 bytes and contains only packed
  checked site identity, annual density, and optional tree kind. Negative
  profile results are cached too.
- Profile climate work happens outside the region mutex. The lock is held only
  to take a snapshot or sort/merge/deduplicate pure results. Concurrent duplicate
  computation cannot affect bytes.
- The only `OnceLock`s retain the measured sub-0.1 ms conservative proof and
  immutable proof cells; there is no tier-build `OnceLock`.
- Candidate streams are sorted by `(ci,cj)` before return, so cache state,
  concurrent merge order, eviction, and LOD request order are unobservable.
- The direct loop's unconditional-snow cheap gate, which v1 also accidentally
  dropped, is restored exactly.

### Bounds and eviction

- 16 shards, 4 MiB of accounted profile/proof payload per shard, 64 MiB total.
- 512 entries per shard (8,192 persistent entries total) also bounds empty and
  proof-only map growth.
- On overflow, the oldest unpinned entry in the touched shard is evicted.
  Entries held by workers are temporarily pinned; the request-end trim removes
  the overshoot after those fixed Rayon workers release their `Arc`s.
- Hash-map/entry metadata is additionally bounded by the entry cap. Request
  scratch is bounded by the selected render tile and fixed worker concurrency.

### Incremental live scheduling

- Near/far deep covers are forecast with the real voxel focus (the old forecast
  selected the wrong non-deep keys below 2.5 km). Fine/deep/near keys are first.
- At 2 km, exact future candidate streams are warmed cache-only. No off-screen
  productive terrain mesh or GPU buffer is constructed. A proven-barren near
  stream may build one deep mesh in the step's large baseline headroom.
- A productive descent gets at most one exact replacement at the 1.5 km slice;
  this is the bounded synchronous remainder and costs about 33 ms in the final
  forest fast descent, below its 591 ms baseline.
- Fresh horizontal covers synchronously build one unique geomorphed parent per
  uncovered family, then refine selected children through the existing live
  background path. This directly addresses horizon reveals without a new pass.
- Live terrain admission is four jobs/frame (eight on vertical motion), at most
  16 total and at most two productive/deep. Through the 2.5 km handoff, motion
  admits no speculative terrain work; after six quiet frames, two jobs may run.
- Live voxel chunks are nearest-first and capped at two in flight. The exact
  heightfield stays visible until chunks arrive. Settled captures retain their
  original drain-to-empty behavior.
- Completed CPU meshes from a cover just left are discarded instead of being
  uploaded during the next move.

The final voxel cap was necessary even after the cache fix. In an intermediate
build, speculative low-altitude work survived a teleport and the forest 0.8 km
median p95 was 188.30 ms. Bounding live chunks reduced it to 30.57 ms. The
problem was visible only after the 2 km tile path became fast enough to expose
the previously hidden background convoy.

No renderer pass was added or reordered. Live scheduling uses deterministic
camera/frame state, not wall clock or randomness. Settled scheduling does not
take any of the live-only branches.

## Rejected directions and cut lines

- **Keep v1 and tune proofs:** rejected. Productive proofs cost at most 0.074 ms
  against an 83.143 ms miss; the wrong work population remained.
- **Monolithic future deep-mesh prebuild:** rejected. A one-key trial left the
  main forest 0.8 km p95 at 217.40 ms because the future exact key was not the
  later spill source. Candidate-only prewarm plus bounded workers was both
  cheaper and effective.
- **Explicit lateral region ring:** not retained. Once fresh-current-cover
  parent fallback and chunk admission were bounded, first lateral p95 became
  19.49 ms versus 730.87 ms and all ten moves were 12.10–19.49 ms. A speculative
  ring would spend worker/memory budget without a measured gate deficit.
- **A second front/query cache and CPU-ready mesh queues:** prototypes moved
  work or upload cost into later frames rather than removing it and worsened
  the descent distribution. The final design has one bounded source of truth.
- **Bake-time candidate artifact:** rejected for this mission. Runtime cold
  evaluation is 6.768 ms and prewarmed urgent lookup is 1.581 ms, so an asset
  cannot justify a new bake step, generation-version coupling, asset hash,
  multiplayer identity surface, and distribution payload.
- **More than two voxel/productive jobs:** rejected at the measured memory/
  worker convoy cut line. Two maintains Rayon concurrency; larger populations
  caused late-frame spill.

Honest remaining costs: `tile_cost` is 0.192 s versus 0.177 s (+8.5%), and a
completely unforecast productive request is 6.768 ms versus 6.493 ms (+4.2%)
in its candidate phase. The standing live probes have enough margin that these
small local costs do not become frame regressions. Moon-orbit perf is close to
the accepted average threshold (+19.4% versus a +20% limit), so that row should
remain watched even though this change does not touch lunar generation.

## Gate results

### 1. Release library tests

| Build | Result |
|---|---|
| `c2bf501` | 102 passed, 1 ignored |
| Final | **109 passed, 1 ignored** (110 total) |

The seven B-4a tests comprise the four historical gates (ordered stream across
strides/concurrency, treeline preservation, proof-union exactness, and bounded
eviction) plus three new gates: all 16,384 partition/site identities and both
face edges; six faces/four corners/four strides against the direct stream; and
malformed entry points.

### 2. `tile_cost`

| Build | Median | vertices | geometry | other | shore |
|---|---:|---:|---:|---:|---:|
| `c2bf501` | 0.177 s | 746031 | 9233981608705220 | 18863829261183721 | 1514547866741203 |
| Final | 0.192 s | 746031 | 9233981608705220 | 18863829261183721 | 1514547866741203 |

All emitted geometry, non-geometry payload, and shore checksums are identical.

### 3. `b4a-forest-descent.play`

| Pose / altitude | `c2bf501` p95 ms | Final median p95 ms |
|---|---:|---:|
| forest 640 km | 103.45 | 46.51 |
| forest 40 km | 61.46 | 16.53 |
| forest 10 km | 43.48 | 17.15 |
| forest 4.9 km | 40.43 | 13.31 |
| forest 3.1 km | 37.51 | 13.39 |
| **forest 2.0 km** | **2,977.92** | **55.54** |
| forest 1.3 km | 66.94 | 15.06 |
| forest 0.8 km | 59.84 | 30.57 |
| forest 0.4 km | 222.41 | 15.05 |
| forest 0.1 km | 71.71 | 15.91 |
| plains 40 km | 250.95 | 77.14 |
| plains 5 km | 58.16 | 18.41 |
| plains 2 km | 1,240.05 | 48.54 |
| plains 0.5 km | 272.66 | 36.82 |
| taiga 20 km | 354.53 | 19.78 |
| taiga 3 km | 66.06 | 6.58 |
| taiga 0.8 km | 219.37 | 14.07 |

Every step passes; the final 2 km range across the three runs was
55.27–61.06 ms.

### 4. `b4a-lateral.play`

| Step | `c2bf501` p95 ms | Final median p95 ms |
|---|---:|---:|
| initial warm | 3,998.12 | 1,401.00 |
| move 1 | 730.87 | 19.49 |
| move 2 | 53.25 | 13.93 |
| move 3 | 142.33 | 17.15 |
| move 4 | 65.72 | 14.27 |
| move 5 | 62.07 | 12.92 |
| move 6 | 58.57 | 12.86 |
| move 7 | 55.27 | 12.94 |
| move 8 | 58.23 | 13.32 |
| move 9 | 58.43 | 12.10 |
| move 10 | 59.76 | 13.79 |

Every horizontal step passes.

### 5. B-4 vertical probes at both poses

Fast descent values are median p95 from three final runs.

| Altitude | Polar pre | Polar final | Forest pre | Forest final |
|---|---:|---:|---:|---:|
| warm p95 | 108.16 | 54.43 | 102.32 | 42.67 |
| 160 km | 18.03 | 14.51 | 16.24 | 12.29 |
| 40 km | 47.48 | 15.66 | 46.56 | 13.07 |
| 10 km | 95.26 | 17.69 | 50.37 | 17.02 |
| 5 km | 33.67 | 15.17 | 34.67 | 13.19 |
| 4 km | 37.92 | 15.46 | 34.89 | 13.24 |
| 3 km | 33.45 | 15.85 | 40.59 | 12.20 |
| 2.5 km | 60.78 | 15.77 | 47.51 | 13.75 |
| 2 km | 2,026.20 | 22.25 | 3,252.70 | 50.56 |
| 1.5 km | 313.08 | 16.28 | 591.09 | 33.29 |
| 0.8 km | 105.03 | 15.93 | 292.81 | 31.56 |
| 0.2 km | 126.57 | 20.15 | 123.22 | 14.89 |
| 0.05 km | 107.83 | 15.75 | 92.43 | 14.66 |

| Probe | Pose | Pre-change | Final |
|---|---|---:|---:|
| ascent-fast warm p95 | polar | 2,584.29 | 136.55 |
| ascent-fast max after warm | polar | 41.90 | 24.59 |
| ascent-fast warm p95 | forest | 3,651.89 | 1,373.05 |
| ascent-fast max after warm | forest | 34.92 | 22.38 |
| descent-fly warm p95 | polar | 305.39 | 98.46 |
| descent-fly max after warm | polar | 487.35 | 43.20 |
| descent-fly warm p95 | forest | 297.61 | 68.41 |
| descent-fly max after warm | forest | 1,686.67 | 57.14 |

No B-4 step regressed.

### 6. First-touch table

The mandatory instrumented table is in the diagnosis above. Headline:
83.143 ms v1 monolith -> 6.768 ms bounded cold request -> 1.581 ms synchronous
candidate lookup after live prewarm. Instrumentation was removed.

### 7. Visual, sync, suites, and determinism

| Gate | Pre-change / accepted | Final |
|---|---|---|
| `world_reel.py` | clean | **all 28 poses clean** |
| `sky_reel.py` | clean | **all 5 poses clean** |
| `sync_diff.py` `sea_calib` | 0.0% | **0.0%** |
| physics-regressions | green | green |
| water-regressions | green | green |
| lake-regressions | green | green |
| flooded-caves | green | green |
| invariant-survey | green | green |
| camera-controls | green | green |
| solar-system | green | green |
| auto-survey | mission-excluded | not run |

One settled forest capture was rendered twice. Both PNGs were 520,211 bytes
and SHA-256 `E0C44F118FB32A2BAE99D3FB94BA8A1CB9F82CE1E2CBD9ED0AD685D41B93D762`.

### 8. Perf reel (no `--accept`)

| Scenario | Pre avg / p95 ms | Final avg / p95 ms | Gate |
|---|---:|---:|---|
| forest_ground | 27.84 / 31.72 | 12.85 / 15.63 | pass |
| plains_walk | 23.70 / 29.47 | 7.72 / 11.77 | pass |
| mountain_mid | 25.05 / 26.36 | 8.29 / 9.93 | pass |
| ocean_storm | 21.39 / 22.94 | 10.48 / 11.77 | pass |
| orbit | 17.89 / 23.27 | 8.79 / 9.78 | pass |
| moon_surface | 16.68 / 17.68 | 16.74 / 19.21 | pass |
| moon_orbit | 2.22 / 2.51 | 2.65 / 3.11 | pass |
| ascent_sweep | 37.73 / 46.92 | 25.09 / 26.20 | pass |

The reel thresholds are +20% average and +35% p95. There are no regressions by
that contract.

## Exit-101 panic verdict

**No provable cause for the field exit-101 was found in v1, and it did not
reproduce.** In particular, the audited v1 production partition result was
0..16383 before its `u16` cast, region products fit `u64`, level-14 regions are
only 610/611 columns wide, and no recursive initialization of the same
`OnceLock` was present. It would be dishonest to claim that field crash is
identified.

The suspected class is nevertheless proven absent from the replacement cache:

- all 16,384 partition bounds are walked contiguously from column 0 through
  9,999,999 and their first/last packed sites are checked;
- site packing uses `checked_sub` and `u16::try_from`, never truncating casts;
- proof cells store absolute `u64` columns and dynamically avoid empty
  subdivisions even at one-column widths;
- public cache entry points reject invalid face, reversed/out-of-face bounds,
  and unsupported/zero strides before face access or arithmetic;
- profile lookup uses checked binary-search indices plus `.get()`;
- poisoned mutexes recover their contained data rather than panicking;
- six faces x four face corners x four strides match the uncached stream;
- malformed entry-point tests return empty/false without panicking;
- no probe, reel, suite, determinism run, or release test produced exit 101.

Specific verdict: **unreproduced and unattributed for v1; the index/OnceLock/
slice panic surface named in the mission is guarded and exhaustively gated in
v2.**

## Final scope

The deliverable changes only the planet candidate cache, its terrain/voxel
exactness integration, and existing live renderer scheduling. It adds no baked
asset, multiplayer identity input, renderer pass, wall-clock dependency, or
instrumentation. Existing unrelated worktree assets/tuning/logs were preserved.

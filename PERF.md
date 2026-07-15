# Performance campaign

The contract is the perf reel (viewer/scripts/perf_reel.py: 8 scenarios,
avg +20% / p95 +35% gates vs the blessed baseline, re-blessed only after
human/AI review). Reference GPU for "is it fast enough": Austin's RTX
2060 - acceptable on Neisor, borderline on the moon (2026-07-14).
Hitches and steady-state framerate are SEPARATE problems with separate
tools: hitches are CPU tile-build scheduling (measure with the b4-*
probes and TRI_RAW_CAPTURE), steady-state is GPU shading (needs
timestamp queries, item 2).

## Shipped (evidence in interchange/reviews/ + commit messages)

- B-4 ascent lagspikes (d0fbcab, Sol): exact region proofs skip impostor
  candidate enumeration where every candidate provably rejects (was
  98.6% of tile CPU at level 11); deterministic ascent lookahead. First
  frames 3 km 117.6 -> ~34 ms, 10 km 251.8 -> ~38 ms.
- B-4b descent lookahead (973957e): vertical forecast both directions,
  two descent bands, doubled budget under vertical motion. Worst
  continuous-descent step 887 -> 523 ms at a 5 km/s emulation.
- W2.5 raster bake moved off the frame (async rayon); moon lattice nav
  p95 80.8 -> 17.8 ms (earlier); moon scenarios improved ~20% as a B-4
  side effect (shared worker pool no longer starved by enumeration).

## Campaign (priority order)

1. RESOLVED by v2 (merge 1b13151, Sol; findings in interchange/
   reviews/sol-b4a2-findings-2026-07-14.md). Root cause of the v1
   regression quantified: it forfeited the per-LOD lottery gate
   (13.18x more expensive profiles) and front-loaded 3.98x sites
   behind an 83 ms OnceLock monolith. v2: request-scaled sparse
   profile cache (misses computed outside locks, negatives cached,
   16-shard 64 MiB LRU) + bounded live scheduling (cache-only
   forecast prewarm, synchronous geomorphed PARENT per fresh horizon
   family with background child refinement, voxel chunks capped at 2
   in flight). Forest 2 km descent 2978 -> 55 ms; lateral first move
   731 -> 19 ms; every standing probe improved; checksums identical;
   settled captures byte-identical. USER-VERIFICATION TRADE TO WATCH
   (Austin is the gate): live frames now converge to full detail
   over ~5-10 s at a new pose instead of hitching to instant
   sharpness - steady-state GPU cost at the forest pose climbs
   9 -> ~25 ms as refinement completes (b4a2-converge.play measures
   this). If the sharpening reads badly in play, the dials are
   LIVE_CHUNK_PENDING_MAX and the quiet-frame productive cap in
   renderer.rs - raising them trades convergence speed against the
   convoy spill Sol measured (188 ms class). Historical note kept
   below for the v1 lesson:
   v1 was shipped 37b3e51, REVERTED b372b70 the same day. Austin field-caught what every merge gate missed: over
   PRODUCTIVE FOREST the cache made cold region builds WORSE (forest
   2 km descent step 3,165 -> 4,663 ms; live descents >1 s "worse
   than before", plus one exit-101 crash), and the trigger was
   HORIZONTAL flight - new regions entering the horizon pay first-
   touch cost that no vertical forecast covers. Why the gates were
   blind: Sol's probes + tile_cost run barren/polar/lake poses, and
   the perf reel warms before measuring. The design (region-keyed
   tiers, union proofs, exactness tests) measured well where proofs
   fire; the first-touch cost of productive regions was never
   measured. RE-MISSION REQUIREMENTS: (a) cold-descent gate at the
   forest pose -0.906 -67.804 (b4a-forest-descent.play - the 3.2 s
   pre-cache wall at 2 km is the real B-4 target and still open);
   (b) a HORIZONTAL-flight probe (fixed altitude, lateral teleport
   steps across forest); (c) first-touch region build must be
   measured and bounded, not just amortized; (d) find the exit-101
   panic (unreproduced in probes; likely in the cache path).
   The banked never-block ancestor-draw decision (Andrew) remains the
   structural cure for ALL urgent-build hitches regardless.
2. SHIPPED 2026-07-14 (7a8bbcb): GPU timestamp queries behind
   TRI_GPU_TIMERS=1 - six stamps bracket the render pass's pipeline
   groups; bench prints rolling per-segment averages
   (gputimers-smoke.play). FIRST ATTRIBUTION on the dev GPU: forest
   ground = 25.5 of 25.65 GPU ms in the TERRAIN group (the mega
   fragment shader: weather/biome/shadow work) - that is the
   steady-state budget and the next shader-pass target; orbit = 11.9;
   moon surface = 2.9 ms GPU TOTAL. Moon "lag" is therefore CPU build
   cost, not shading.
2b. NEXT: the mega-shader pass. With attribution in hand, hunt the
   terrain fragment cost (suspects: per-pixel weather deck taps,
   biome comparator stack, karst probes at ground poses). Every
   change gates on the sky/world reels.
3. MOON PASS - now known to be CPU: profile crater-fold sampling in
   tile/chunk BUILDS (the 1.8 s first-visit spikes in the smoke run),
   not the shader.
4. Smaller banked items: R-7 structured cyclone shading (+4.6% avg on
   the cyclone orbit bench - fold into the shader pass); cold-load
   upload batching (needs terrain-vs-voxel accounting first, per Sol);
   sky output dither for the 1/255 quantization whisper (art call -
   touches the sky-appearance contract).

## Instruments

perf_reel.py (contract), b4-ascent-fast / b4-descent-fast /
b4-descent-fly .play (transition hitches; TRI_RAW_CAPTURE=1),
TRI_NO_IMPOSTORS=1 (isolate enumeration cost), tile_cost example
(fixed 611-tile CPU gate with exact output checksums), fpsbench/
fillbench/forestflight.play (older steady-state probes).

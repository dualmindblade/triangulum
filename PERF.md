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

1. SHIPPED 2026-07-14 (37b3e51, Sol): THE LEAN CANDIDATE CACHE.
   Per-Planet 16-shard LRU (64 MiB / 512 entries per shard) keyed by
   (face, level-14 region), lean 16-byte candidates in four disjoint
   stride tiers so every LOD/seasonal rebuild reuses one evaluation;
   adaptive whole-tile union rejection proof. Cold load 2714 -> 608 ms;
   the 2 km descent step 698 -> 86 ms (88 ms confirmed on main);
   tile_cost -4% with identical checksums; four exactness tests lock
   bit-for-bit stream equality including concurrent misses. Residuals
   ledgered as B-4c in BUGS.md: teleport-probe urgent builds (cure =
   the banked never-block ancestor-draw decision - Andrew), a
   non-impostor ~220 ms scheduling floor at pose B 2 km, and ~0.6 s
   dense cold loads.
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

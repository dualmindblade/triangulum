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

1. B-4a THE LEAN CANDIDATE CACHE - the remaining hitch. Austin
   (2026-07-14): hitches now ~1/4 of transitions, was ~9/10, descent-
   skewed (95% credence) - matches the measured residual: dense or
   climate-boundary level 11-14 tiles defeat both region proofs and
   enumerate 300k+ candidates (~300-900 ms builds); descents need the
   FAT fine-LOD ring, ascents only cheap coarse tiles, hence the skew.
   Whether a hitch fires = whether the flight path crosses such a tile
   before its background build lands. Direct attack: a reusable
   candidate stream/cache keyed independently of tile LOD (enumerate a
   region once, serve every LOD/rebuild from it). Complementary design
   option (needs Andrew): never-block scheduling - draw the cached
   ancestor instead of any synchronous urgent build; trades a moment of
   coarser terrain for zero hitches.
2. GPU TIMESTAMP QUERIES - steady-state attribution. We only measure
   CPU frame time; the 2060's budget needs a per-pass GPU breakdown
   (terrain / impostors / deck+sky / post) via wgpu timestamp queries
   behind a diagnostic flag, reported in the play harness sidecars.
   Suspect: fragment-heavy weather/deck shading. This feeds the shader
   work list; it does NOT help the hitches (already CPU-attributed).
3. MOON PASS - crater-fold sampling has never been profiled; moon
   remains clearly costlier than Neisor at parity poses.
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

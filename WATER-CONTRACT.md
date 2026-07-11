# The water contract

What the water system PROMISES, what it READS, and which gate enforces
each promise. Any mission that touches landscape generation, terrain
sampling, or water rules reads this first and runs the full gauntlet
(below) before claiming success. Written after the water2 regressions
(2026-07-11) proved that locally-principled changes compose into
planet-scale surprises that aggregate gates cannot see.

## Invariants (each with its enforcing gate)

W1. Water always meets a shore at grade: no standing water face taller
    than one block against dry ground at rest.
    GATE: census --lips totals (liquid cohorts must not rise);
    colcensus LIP/EDGE at Difficulty Lake (13.346 -4.807) and the pond
    site (12.199 -44.827).
W2. Water that would drain does not exist (never-perched, all families:
    lakes, ponds, rivers).
    GATE: census WALL/JUMP totals; water-regressions.play asserts.
W3. Structures serving water (benches, fills, aprons) exist ONLY while
    their water exists, and taper continuously to nothing - no residual
    ridges, dams, or dry bowls when the water is suppressed.
    GATE: the world reel (dossier_ponds pose caught both the fence and
    the dry-bed classes); terrain unit gates on fill functions.
W4. Both renderers agree on water class, position, and surface height:
    mesh plane, block tops, the shore field, and physics
    (water_surface_km) are one truth.
    GATE: sync_diff.py standing table (sea_calib MUST stay 0.0%);
    lake/karst pose means; suites' immersion asserts.
W5. No see-through: every water body is backed by geometry from every
    angle (the wflag exemption contract) - no voids, no missing faces.
    GATE: world reel VOID lint (near-black clusters in daytime shots).
W6. Open liquid water renders featureless at rest: no per-quad seams,
    grids, or checkerboards. (Frozen ice checker is BY DESIGN.)
    GATE: world reel accepted-baseline diff (grid lint is informational
    until calibrated - a live specimen is needed to set its threshold).
W7. Octave stability: wet-vs-dry class at any point is identical at
    every LOD (rivers: RIVER_REF_OCTAVES; lakes/ponds: the voxel shore
    reference completion).
    GATE: terrain unit gates (v5 oracle tests); sync_diff lake poses.
W8. Determinism: all water decisions are pure functions of
    (seed, position); repeat renders byte-identical.
    GATE: world reel re-render hash equality; weather-visual double run.

## What water READS from the landscape (the coupling surface)

- e_raw (raster elevation, octave-independent) - sea class + coastal
  rules. NOTE: bilinear e_raw dips below 0 on land near coasts; the
  ocean MASK is the sea authority, never e_raw alone.
- h at the tile's octave budget (geometry) and at VOXEL_OCTAVES (class
  truth); RIVER_REF_OCTAVES for river perch decisions.
- The drainage graph (rivers.bin): levels are the HYDRAULIC FILL
  SURFACE, never bed-anchored.
- Lake Voronoi territory + rim/dam rings; pond mask noise (pn) + env0 +
  raster flatness taps; cave noise for karst tables.
- If a landscape change alters ANY of these inputs' statistics (raster
  resolution, noise envelopes, graph export), every invariant above is
  in play: run the full gauntlet, not a subset.

## The gauntlet (run all, in this order - cheap first)

1. cargo test --release --lib          (unit gates incl. water oracles)
2. bash scripts/verify.sh              (7 asserting play suites)
3. python scripts/sync_diff.py         (mesh<->voxel; sea_calib 0.0%)
4. python scripts/world_reel.py        (24-pose change detector + void
                                        lint; explain EVERY flagged pose
                                        in FINDINGS.md or fix it)
5. colcensus at the ledger sites       (Difficulty Lake, pond country)
6. census --lips liquid cohorts        (planet aggregate, no rises)

A change is DONE when the gauntlet is green or every flag is explained
and accepted by a human. The reel baseline is re-blessed
(world_reel.py --accept) only after human sign-off on the changes.

## Known debt the contract inherits (ledgered)

- A-4/A-5 reopened (pond ramparts, rim water band) - redo must pass
  this contract, gates included.
- P-1 residual pond walls at 12.194 -44.858 (28 sites).
- W-7/W-5b mega-lake families (upstream planetgen).
- Wanted: colcensus --survey (planet-sampled block-truth invariant
  sweep, ~100 feature-driven sites) to replace the two fixed ledger
  sites in gate 5 with statistical coverage.

# One world, two renderers: the transitions design note

## VERDICTS (Andrew, 2026-07-09 evening — these are now design constraints)

- **Direction: seamless-first.** "Less visible is ideal unless it
  introduces more complications." The quaint-rim option (C) survives only
  as a KNOB that defaults to invisible; engineer every seam to disappear.
- **E is greenlit as described**: tree billboard impostors at mid
  distance, vegetation coloring at high distance. (Austin: we fly at
  medium altitude a lot — this is important.)
- **D gains a dual-range rule**: long-range climate blending for biome
  pairs sharing the same main block (grass↔grass: kilometers-long tint
  gradients), SHORT-range blending between different-block pairs
  (sand↔grass: a crisp dithered ecotone band). Category dither is for
  cross-block boundaries; same-block boundaries are pure tint.
- **Climate-tint ramp colors are provisional**: whatever the ramp picks is
  fine for now; final colors come AFTER texturing, by taking each
  texture's average color as its tint anchor. (So: build the ramp
  machinery, don't gold-plate the palette.)

**Landed since the verdicts (2026-07-10 night):**
- **E v1** — forest impostors (9781db5): billboard trees on levels 11-12
  from the voxel placement lottery; reach ~3-6 km, then tint. Known
  tells: X-cross within ~400 m, level-border density step, no far fade.
- **D v1** — climate tint + dual-range ecotones (a65591f, by GPT-5.6
  Sol): planet::climate_surface shared by both renderers, same-block =
  pure km-scale ramp, cross-block = 300 m per-column dither with a true
  gnomonic metric; snowline hash unified. Known cut: far-mesh dither is
  per-VERTEX (sawtooth tongues at ~100 m spacing); per-pixel needs the
  A-style shader pass. Palette provisional per the verdict.
- Remaining, resequenced: B (per-pixel shorelines) -> A+F (shared
  micro-texture + water color; A also upgrades D's dither to per-pixel)
  -> C (rim knob, default invisible) -> E/D polish (far fade, sway,
  per-pair band widths).

For the texturing/transitions summit (Austin + Andrew + AI crew), 2026-07-09.
Covers BUGS.md V-1 (voxel patch reads as a disk against the far mesh), V-5
(lake shores alias into polygons at coarse LOD), and biome transitions.
Design stance from Austin: the switch does NOT have to be invisible — "some
trace of the rendering switching might be kind of quaint and reassuring" —
but it must look good and be non-distracting. That stance is a gift: it
means we engineer for STATISTICAL continuity (same colors, same texture
energy, same shapes on both sides) and then choose, as an art decision, how
much of the seam to let show.

## The seams we actually have (inventory)

1. **The patch disk (V-1 escalated).** Over flat terrain from low flight
   the voxel patch is a circular region of high-frequency block variation
   sitting on a perfectly flat far-mesh field. The geometry seam was solved
   long ago (rim feathering); the TEXTURE seam was never addressed: blocks
   carry a per-block brightness hash + AO + per-face shading, the mesh
   carries none of that. Two different noise statistics = visible circle.
2. **Material parity drift.** Voxel materials (grass/sand/snow decisions,
   dirt strata, beach bands, lake-shore sand) and mesh coloring
   (shade_ground) are parallel implementations of the same intent. They
   agree today only because we keep hand-syncing them (the lake-shore sand
   went in twice, once per renderer).
3. **Lake shores at distance (V-5).** The far mesh lifts lake vertices to
   the water level and colors by vertex — so a shoreline is drawn at
   vertex resolution: angular polygons, orphan blue cells inland.
4. **Forests at distance.** Voxel trees end at the patch rim; beyond it
   forests are a darkening tint. Groves visibly pop in/out character at
   the rim.
5. **Biome edges (both renderers).** Köppen class is nearest-texel, so a
   steppe/desert boundary is a hard line — same line in both renderers (at
   least they agree!), but a hard line regardless.

## The principle everything below follows

**One truth, two renderers.** Every visual decision must be a pure,
world-anchored function — of column lattice position, of (face,u,v), of
climate fields — that BOTH pipelines evaluate. The renderers may differ in
RESOLUTION (blocks near, triangles far) but never in STATISTICS (same tint,
same texture energy at matching scale, same category boundaries). The whole
day of bug-hunting kept proving this: every time the two sides computed
something separately (wetness by octave count, materials, water color),
they eventually disagreed somewhere visible.

## Proposals, ordered by leverage

### A. Shared micro-texture: put the block hash in the shader (kills V-1)
The per-block brightness hash (`bright()` in voxel.rs — hash of face,
column i/j, height) is what makes the near field read "textured". The far
mesh can evaluate THE SAME hash per fragment: world position → column
lattice cell → same splitmix hash → same ±10% brightness. The mesh then
carries block-granular texture identical in distribution to the voxel
field, and the patch disk dissolves into a resolution change instead of a
material change. Both sides fade the variance with DISTANCE (the mesh
already fades to flat tint at range; make the voxels match) so the far
field stays clean. Notes:
- The fragment shader already receives world-ish position (rel_flag.xyz
  is camera-relative position — position reconstruction is available).
- Same trick extends to strata color on cliff faces and beach bands.
- This is the first true "texturing" step and it is procedural — no
  texture assets, no UV problems on a sphere, works at every LOD, and the
  voxel and mesh versions CANNOT drift because they are the same function.

### B. Per-pixel shorelines: step on the water level (kills V-5)
The river paint already uses the trick: "the fragment shader steps on
interpolated wetness, so painted water gets crisp per-pixel edges even on
coarse tiles." Do the same for lake/sea GEOMETRY color: pass the local
water level and the terrain height per vertex; the fragment shader decides
water-vs-land per PIXEL (h < level), not per vertex. Angular shorelines
become smooth contour lines at any LOD; orphan blue cells self-clip
(their pixels are above the level). Follow-up in the same spirit: distant
river RIBBONS as a per-pixel decision over the blended level field
(roadmap item, same machinery).

### C. The rim as a designed moment (the "quaint" seam)
With A + material parity, the rim's only remaining tell is geometry:
blocks vs smooth. Feathering already morphs heights. Add a DITHERED
EMERGENCE band: across the outer ~15% of the patch, blocks materialize by
per-column hash threshold (a column "condenses" from the mesh when the
rim crosses its hash value) instead of all at once at a radius. From the
air the rim becomes a soft speckled annulus rather than a circle — and
whether we keep a faint deliberate trace of it (a subtle shimmer, a
half-tone ring — the reassuring "the world is becoming real here" cue) is
a pure art knob for Andrew. Parameterize it; ship it subtle; let him turn
it up or off.

### D. Biome transitions: color from climate, category by dithered Köppen
Two-layer split:
- **TINT comes from continuous fields.** Temperature and precipitation are
  already per-column/per-vertex rasters. Derive ground tint from a smooth
  climate→color ramp (with Köppen only nudging hue), and biome color
  transitions become gradients automatically — in both renderers, since
  both already read the same rasters. A steppe fades into desert over
  kilometers the way it does from an airplane.
- **CATEGORY stays discrete but dithers.** Materials, tree species, and
  densities key on Köppen class; at boundaries, interleave classes by the
  shared per-column hash over a transition band (hash < blend fraction →
  neighbour class). Salt-and-pepper borders are quantization-native (this
  is what Minecraft does), read as ecotones, and the SAME hash rule colors
  far-mesh fragments, so the dither pattern is identical at range. The
  precedent already exists in-repo: the snowline dithers per column
  (surface_mat jitter), and it reads great. This proposal is "snowline
  treatment for every boundary".
- Trees participate: density/species blend across the band via the same
  hash → groves thin out rather than stopping at a line.

### E. Forests at range
Near-term (cheap): far-mesh forest darkening should be computed from the
SAME density function that places voxel trees (same hash, same thresholds)
so the rim doesn't change the forest's texture — with A, the darkening
also picks up block-scale mottling that matches canopy speckle.
Mid-term: instanced tree impostors (a few quads per tree, same placement
hash) between the patch rim and ~10 km, so groves have silhouettes.
This is the biggest remaining "two worlds" tell after A/B land.

### F. Water color unification (hygiene)
Voxel water color and mesh water color share ramps by copy-paste today.
Extract one shared function (depth → color, shallows → teal, salt → pale,
frozen → ice) so they cannot drift. Small, do alongside A.

## The meter: sync-diff (landed 2026-07-10, Austin's idea)

`viewer/scripts/sync_diff.py` renders every pose in a standing 14-pose
table twice — normal, and with the voxel near-field forced off (`voxels
off` play command / `--no-voxels`; toggle round-trip byte-identical) —
and image-diffs the pairs. Outside the patch the frames match exactly
(open-sea calibration pose: 0.0%), so the divergence region IS the
mesh->voxel handoff, with per-pose numbers (divergent fraction, mean/p95
delta, signed luminance bias) and red heatmaps in
interchange/runs/sync-diff/. First fruits: the V-5 octave shoreline
offset measured at mean 50.6 (4-6x anything else), and two systematic
shading biases ledgered as V-9 (polar ice +12, steep slopes -8).

RULES OF USE: this is a measuring instrument and a regression gate, not
an optimization target. A change that "improves the number" by muting
detail on both sides is a regression in disguise (Goodhart); a change
can also be a big win the meter barely sees (per-pixel normal detail
moved river poses ~0.1 — because it fixes SHADING CHARACTER, not class
flips). Eyes — Austin's and Andrew's — stay the judges of better;
the meter's job is worst-pose ranking, did-it-move-together checks
after material edits, and catching regressions (sea_calib must stay 0,
no pose may jump without a named cause).

## Suggested sequencing

1. **B** first: self-contained shader work, kills V-5 outright, no art
   decisions needed, high visibility from the air.
2. **A + F** together: the texture unification. After this the patch disk
   should be hard to find over uniform terrain. (Verify with the sidecar
   repro at 7.042 33.477 alt 0.8 — the flagship V-1 frame.)
3. **D**: biome tint-from-climate + dithered categories. Most visible from
   mid-altitude; touches materials, so verify across the scenic table.
4. **C**: rim emergence dither, with Andrew tuning the visibility knob.
5. **E**: forest impostors (largest scope; can trail).

Everything above is procedural and shared-function — no texture atlas, no
mesh/voxel special cases — which is also the answer to "everything we do
to the voxels will have to be matched in the other rendering style":
after this program, matching is automatic because there is nothing to
match; there is one function evaluated twice.

## Questions for Andrew

Moved to DECISIONS.md (repo root) - the dedicated living decision-points
document. This program's entries there: D-1 rim dial, D-2 texture
energy, D-3 per-pair ecotone widths, D-10 slope shading truth, D-15 the
texture/palette pass. The 2026-07-09 verdicts that shaped this program
are recorded in its Answered section.

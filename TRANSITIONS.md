# One world, two renderers: the transitions design note

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

1. How visible should the rim be? (C is a dial: invisible ↔ soft speckle
   annulus ↔ deliberate shimmer. Austin votes "trace of it = quaint".)
2. Texture energy: how strong should block-scale mottling be at range?
   (A's distance fade curve is an art knob.)
3. Biome edge width: crisp ecotones (~200 m) or long gradients (~2 km)?
   Per-pair overrides (desert/oasis crisp; steppe/savanna long)?
4. Foam/rapids palette (codex is prototyping W-3 white-water now) — how
   lively should steep water be?
5. Does he want a hand in the climate→tint ramp? (It is essentially the
   planet's color grade — very much a creative-director artifact.)

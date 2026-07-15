# Landscape enrichment campaign (Austin + Andrew, 2026-07-14)

Direction chosen over more gameplay: the features are near target;
the world itself gets richer. Everything here is a pure function of
(seed, position, time) riding the established pipelines - censuses
gate it, reels pin it, multiplayer syncs nothing new. Start:
immediately after the B-4a v2 candidate cache is USER-VERIFIED
(Austin's call), because passes 1 and 3 ride that machinery.

Standing discipline for every pass: intended visual changes mean
intended reel diffs - each feature lands with its own play probes,
reel review + re-bless, census deltas where water/terrain data moves,
and WATER-CONTRACT.md consultation for anything touching water.

## Pass 1 - GEOLOGY: rocks, boulders, outcrops (both bodies)

Deterministic hash-lattice placement (the proven tree pipeline) with
density from geology/biome fields; voxel block clusters near, impostors
far. Moon boulders satisfy the banked SOLAR.md r3 ask via block
clusters (no block models needed yet). DESIGN AGAINST THE B-4a CACHE
from day one: rocks are a second candidate species stream, not a
bolt-on. Andrew decisions: rock palette per biome/body, size
distribution, rarity of large glacial erratics.

## Pass 2 - RIVERS: banks, then islands, then waterfalls

One mission family on the river bake, in that order (banks reshape the
data the others sit on). Natural banks = the already-discussed
meander/bank-profile work. Islands = width field + braiding noise
exposing mid-channel bars. TRUE WATERFALLS = steep-gradient river
segments emit falling-water features (sheets, foam, plunge pools),
plus spray mist and a sun-angle rainbow arc - the trailer shot.
Slowest, most carefully gated pass: full census before/after, F-4
regression suite, WATER-CONTRACT review at design time. Deltas and
estuaries (braided mouths) ride this bake family as a follow-up.

## Pass 3 - FLORA: tree variety, grasses, ground cover

More tree species and silhouettes, grasses, forest-floor litter
(fallen logs, stumps - same lattice pipeline). Multiplies impostor and
candidate counts, so it REQUIRES the stable cache underneath. This is
the most art-directed pass: Andrew specs silhouettes/palettes the way
he specced the moon.

## Pass 4 - FORMATIONS: two stages

v1 heightfield-friendly: mesas, buttes, hoodoos, sea cliffs, sea
stacks, scree/talus slopes under steep faces. v2 needs the 3D carve
field (karst tech): true arches and overhangs.

## Cross-cutting cheap-but-huge (slot between passes as capacity allows)

- STRATA BANDING: sedimentary color layers in every exposed cut -
  pure shader field (elevation + tilt/fold noise). Zero geometry;
  transforms all rock including Pass 4 later. Shader-only track.
- COASTAL BATHYMETRY COLOR: shallow-water turquoise from existing
  depth data. Shader-only track.
- WIND-ALIGNED DUNES: arid regions grow dune fields aligned to the
  BAKED WIND FIELD - deserts causally coherent with the weather.
- SALT FLATS / PLAYAS: the hydrology bake knows endorheic basins;
  cracked-playa texture in the right places.
- NEISOR METEOR CRATERS: sparse, ancient, eroded reuse of the moon
  crater fold; quietly sets up Andrew's banked comet/meteor-shower
  astronomy idea.
- GLACIER TONGUES: high-mountain glaciers with crevasse texture
  feeding meltwater rivers.
- Later/banked: tidal flats (pairs with banked moon tides), reefs,
  fireflies/bioluminescence (Andrew art call), aurora (SOLAR.md).

## Parallel tracks (Austin: "easy to split into two parallel agents?")

Yes - by file-collision group, up to three concurrent missions:

- TRACK A (candidate/lattice: terrain.rs impostors, voxel.rs
  species tables): Pass 1 geology, then Pass 3 flora, then litter.
  SERIAL within the track; depends on the B-4a cache.
- TRACK B (river bake + water fields: bake_rivers.py, rivers.bin,
  shore/water code): Pass 2 in order. Serial within; independent of
  Track A's files.
- TRACK C (shader-only: shader.wgsl fragment work): strata banding,
  bathymetry color. Parallel to A and B; short missions.
- Heightfield work (Pass 4 v1, dunes, playas, Neisor craters) shares
  planet.rs with Track A's density fields - run it in Track A's slots
  between its passes, or accept merge coordination.

All merges land serially through the main tree with the full gauntlet
regardless of how many tracks run; the worktree-per-mission pattern
isolates the work, not the verification.

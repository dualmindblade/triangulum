# The solar system (Andrew's spec, 2026-07-12)

Source: viewer/interchange/solar-system-and-moon.txt. Sun and moon
become real physical bodies; more stars/planets may follow. This doc
is the engineering read of that spec: what extends cleanly, what is
genuinely hard, the phase plan, and the gates each phase must pass.

## Non-negotiables (Austin, verbatim intent)

- The current atmospheric lighting from sun/moon, and their APPEARANCE
  in the sky from Neisor, must not degrade. Gate: a sky-appearance
  suite pins today's discs/halo/phase look before any body work lands.
- Time travel stays quick: every body position must be a pure O(1)
  function of (t, seed/params). Kepler two-body orbits satisfy this
  exactly (solve Kepler's equation per body per frame - closed-form
  cost, no integration, no accumulated state). Eccentric orbits are
  the SAME math, so the future-use requirement is free.
- One truth, two renderers stays law on the moon: every visual
  decision a pure function of (position, seed), mesh and voxels agree,
  the gauntlet grows moon poses.

## What extends cleanly (green lights)

- ORBITS: parameterized Kepler elements per body (a, e, i, phase),
  hierarchical (sun <- Neisor <- moon; "orbitally locked" = fixed
  offset in a parent's rotating frame). Deterministic timeskip and
  camera realign follow directly.
- LIGHTING: terrain is already lit per-pixel by dot(normal, sun_dir).
  The "whole screen light/dark" is the camera-anchored ambient/day
  factor. Making ambient/day follow the PIXEL's planet position gives
  the orbital terminator Andrew wants, and converges to today's look
  at ground level by construction (near the ground every pixel shares
  the camera's day factor). Sky dome and stars stay camera-based -
  the sky is where the camera is. Sun tint/halo functions unchanged.
- DETERMINISM of moon terrain: crater sequence drawn from the seed
  stream (ordered oldest->newest), each crater composes over prior
  terrain (override at site, disturb at rim). A pure fold - no sim
  state. Maria/rays/ridges are noise fields in the established style.
- D-5 TIE-IN: Andrew's clock (30-min days, 7-day lunar cycle,
  3 cycles/season, 4 seasons/year) fixes the moon's period at 7 game
  days and the year at 84. The sun's current +-23 deg seasonal
  declination becomes axial tilt + orbit geometry - reconcile so the
  weather bake's season phase is EXACTLY the orbital season phase
  (one clock, not two).

## The honest hard parts (flag before starting)

1. TWO VOXEL WORLDS. Everything assumes one Planet (loading, tile
   cache, patch streaming, physics, census tooling). A moon you can
   LAND ON is the single biggest lift - phase it behind a mesh-only
   moon. When it comes: Planet becomes per-body, streaming/physics
   bind to the focused body, tools grow a --body flag.
2. PRECISION. Planet-magnitude f32 bit us three times this week
   (karst moire, dither plateaus, crawling cells). Body positions and
   camera math stay f64 CPU-side; everything GPU renders
   camera-relative (existing pattern); any new shader noise domain
   uses the anchor/kgnoise precedents. Moon distance ~110x Neisor
   radius makes this mandatory, not optional.
3. SKY APPEARANCE REGRESSION. Today's sun/moon are tuned shader
   discs (angular size, halo, maria mottle, phase). Physical bodies
   must reproduce that look from the ground within the reel's
   tolerance. Build the sky suite FIRST, then swap implementations
   under it.
4. CAMERA STATE MACHINE. focused(Neisor) / focused(moon|sun) /
   freecam(6DOF + Q/E roll), timeskip realignment, walk mode
   untouched. Needs its own play-suite coverage (camera-controls
   suite grows freecam/focus/roll asserts).
5. ECLIPSES exist whether we design them or not: a physical moon
   between sun and Neisor WILL occlude. Decide the art call early
   (D-16): geometric shadow, softened penumbra, frequency tuning via
   orbit inclination. (Opportunity, not just risk - deterministic
   eclipse calendar is a gift to gameplay.)
6. CONTROLS: Q/E roll freecam-only; LMB break / RMB place blocks -
   check current bindings for conflicts before rebinding.

## Phases (each independently gauntlet-green)

STATUS 2026-07-12: P1 SHIPPED (merge 732360e) and P2 SHIPPED (merge
5fc0612) - physical Kepler bodies on the D-5 clock, eclipses per
D-16, freecam/focus/roll, generated moon (crater fold, rays, maria),
moon teleport map tab, two moon reel poses. P1's pinned-sun path
originally rotated bodies about the observer (non-rigid vs Neisor at
the origin) and pinned skies could show phantom eclipses - fixed at
the P2 merge (rotation about Neisor's center). Next: P3 moon landing.

- P1 BODIES + FRAMES: Kepler elements; sun and moon as positioned
  bodies (moon = mesh sphere with large-octave noise at true scale
  ratio ~0.27 R_neisor; sun = emissive mesh); sky suite pins
  appearance; per-pixel terminator ambient; freecam/focus/roll/
  timeskip realign; LMB/RMB rebind. New solar-system.play suite.
- P2 MOON GENERATION (planetgen project): crater fold (ridged rims,
  central-peak dimple for large, non-circular incidentals rarer per
  gravity note), ray systems, maria, smoother base than Neisor.
  Mesh-only; reel gains moon poses (from Neisor AND from near-moon).
- P3 MOON LANDING: per-body Planet/streaming/physics; voxel moon
  columns; census/colcensus grow --body; the water/ice deposition
  rules (poles, crater shadows, underground) join the material stack.
- P4 RESOURCES + GAMEPLAY: underground reservoirs, ice mining, and
  whatever the eclipse/tide/astronomy decisions (below) turn into.

## Ideas banked for Andrew (decision points when he wants them)

- Eclipse art direction (D-16 candidate): solar eclipses sweeping a
  moon shadow across Neisor; lunar eclipses copper-tinting the moon.
  \[Eclipses should occur at a much greater rate than on Earth,
  but I think they should remain irregular as that's somewhat part
  of the charm in having them, you can use the orbital perameters to
  tune this factor. Copper-tinting is a nice feature, and solar
  eclipses should reduce the light coming from the sun proportional
  to the occluded area until the sky becomes night-like.\]
- Tides: moon position -> small deterministic sea-level function;
  shores already understand levels. Interacts with the WATER
  CONTRACT - needs its own design pass before any code.
  \[Defer until water levels and block updates are added (the kind
  that appear when reloading terrain, as will need to be added to
  introduce seasonal ice cap coverage.\]
- Astronomy gameplay: the D-5 calendar IS the moon - phases as the
  in-world clock, navigation by stars, meteor showers when Neisor
  crosses a comet's (eccentric!) orbit - which can deposit NEW craters
  into the moon's crater stream at deterministic times.
  \[All great features to add. Defer comet collision events until
  the necessary prerequisites are added.\]
- Sun surface (spec: "dynamic texturing later"): kgnoise granulation,
  limb darkening, flare events on the weather-style deterministic
  clock; space weather -> polar auroras on Neisor.
  \[All good. Dark spots, auroras, and solar flares are all
  awesome features, though these can be deferred for some time.\]
- Orbitally-locked trojan asteroid as the spec's locked-body case.
  \[Good test to add, proceed or defer as is necessary.\]
  
\[Need to have an updated teleport map once phase 2 is completed.\]
  
## Standing constraints for every phase

Gauntlet applies (tests, suites, sync_diff, reel, censuses) plus:
double-run byte determinism at a timeskipped pose; sky-appearance
suite; frame budget (bench 30 at ground/orbit before/after);
WATER-CONTRACT.md untouched by body work (no water rule reads body
state without a design pass).

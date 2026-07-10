# Decision points (Andrew)

Living list of creative/design decisions that belong to the creative
director. One entry per decision: context, options, the default that
ships until decided, and what (if anything) is waiting on it. Answered
entries move to the bottom with the verdict and date — they are the
design record, don't delete them. Engineering priority calls (planetgen
rebakes, bug ordering) live in BUGS.md, not here.

## Open

### D-1 The rim dial (TRANSITIONS C)
How visible should the mesh->voxel boundary be? The 07-09 verdict said
err seamless "unless it introduces more complications", and the program
since then (shared texture, per-pixel shorelines, normal detail, karst
hints) has made the rim genuinely hard to spot. Remaining call: build
the C dial anyway (invisible <-> soft speckle annulus <-> deliberate
shimmer, Austin votes "trace of it = quaint") or declare seamless final
and skip the knob. Default until decided: no dial, seamless.

### D-2 Texture energy at range (TRANSITIONS A)
How strong should block-scale mottling read at distance? The micro-
texture's ~2 km fade curve and +-10% amplitude are art knobs. Default:
current values (subtle).

### D-3 Biome edge width, per-pair overrides (TRANSITIONS D)
The broad rule is decided (07-09: long gradients within a main block,
shorter across blocks — shipped). Open detail: per-pair overrides, e.g.
desert/oasis crisp (~200 m) vs steppe/savanna long (~2 km). Default:
one global pair of widths.

### D-4 Steep water liveliness (rapids/foam palette)
Rapids foam shipped (F-14). How lively should steep water be — foam
strength, whitecap color, where it triggers? Default: current subtle
foam.

### D-5 Weather: year length and starting season
Spectacle (short years, seasons visibly move) vs realism (long years).
Default: 20-day years (day_len 1200 s x 20), epoch spring. Tunable live
in weather_tuning.json — good demo material for deciding together.

### D-6 Cloud art direction
Soft painterly masses (ships today) vs crisp cumulus cells — the shell
noise is swappable. Default: painterly-soft.

### D-7 Overcast gloom cap
How dark may a fully overcast day get? Default keeps ~35% direct light
so screenshots stay readable (overcast_sun_floor knob).

### D-8 Storm frequency
How often should a given valley see rain — weather as event vs
wallpaper? (storminess bias knob.) Default: event.

### D-9 W4 scope: seasonal ice and melt
The biggest gameplay consequence on the board: lakes that freeze into
winter paths, melt seasons, snow accumulation. Needs a design doc + a
suite time-model strategy before any code (WEATHER.md W4). Decide
whether this world wants it at all, and how deep.

### D-10 Renderer shading truth: steep slopes (BUGS.md V-9 residual)
Blocks self-shade steep ground ~8/255 darker than the mesh's diffuse
(the polar-ice half of V-9 turned out to be a bug and is fixed; slopes
and a small +3.4 ice residual remain). Which look is right — the
blocks' chiseled darkness or the mesh's softer diffuse? One shared rule
will follow the verdict. Default: they disagree slightly.

### D-11 Karst art direction (BUGS.md V-10)
Cave-mouth pools and pits are canon (07-10, Austin, "karst frigging
awesome" — confirm when you land). Open art calls: dry-pit mouth
styling (currently exposed dirt, lightly shadowed), pool color, and
whether breach density should vary more by climate (wetter = more
sinkholes?). Default: current look, density as the cave noise gives it.

### D-12 River-bank aesthetics (banked 07-10 for an Andrew session)
Austin's pick from the proposals: meander-curvature asymmetry (cut bank
steep, point bar shallow — proposal 4) composed with wet margins (2)
and bank-width noise (3). Waiting for a design session, then a mission.

### D-13 Ice-shelf edges (BUGS.md S-3)
Frozen lakes/rivers end in multi-block ice cliffs above dry ground. An
ice shelf arguably HAS an edge — is this a feature or a wall? (The
extreme 600 m cases are W-5b planetgen work regardless.) Default: keep.

### D-14 Mega-lake apron dirt steps (BUGS.md W-7 caveat)
The shore apron trades standing-water cliffs for tens-of-metres DIRT
steps at Voronoi radius flips. Natural-looking terracing or ugly?
Revisit alongside the planetgen mega-lake fix. Default: keep.

### D-15 The texture/palette pass (the big one)
Per the 07-09 verdict the current climate-tint colors are provisional;
final block textures come first, then the tint map takes each texture's
average color. This is Andrew's own project: texture style, resolution,
palette. Everything color-related upstream (D-2, D-10, D-11) partially
lands here.

## Answered (the design record)

### 2026-07-09 — TRANSITIONS verdicts (Andrew, verbatim via Austin)
- Rim: "less visible is ideal unless it introduces more complications"
  -> seamless-first, dial optional (see D-1 for the remaining half).
- Forests: tree billboards at mid distance + vegetation coloring at
  high — shipped as E (impostors) + far tint.
- Biome blending: long-range for same-main-block biomes, shorter-range
  across blocks (e.g. sand vs grass) — shipped in D (climate tint).
- Palette: "whatever colors it decides are fine, we can finalize later
  after we do textures by taking the average color" -> provisional
  tint now, final = texture averages (D-15).

### 2026-07-10 — karst breaches are canon (Austin, Andrew confirm pending)
"No way are we going to want to suppress near-surface breaches." The
mesh got a bit-exact cave-field twin instead (V-10 hint); Difficulty
Lake (13.346 -4.807) is the flagship field and is named in the scenic
table.

# Decision points (Andrew, decisions in brackets: \[\])

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

\[Existing behavior is very good, colors need a little fine tuning, we
can work on that later \]

### D-2 Texture energy at range (TRANSITIONS A)
How strong should block-scale mottling read at distance? The micro-
texture's ~2 km fade curve and +-10% amplitude are art knobs. Default:
current values (subtle).

\[Blocks will eventually have textures for variation so the mottling
will eventually be removed and can be ignored for now.\]

### D-3 Biome edge width, per-pair overrides (TRANSITIONS D)
The broad rule is decided (07-09: long gradients within a main block,
shorter across blocks — shipped). Open detail: per-pair overrides, e.g.
desert/oasis crisp (~200 m) vs steppe/savanna long (~2 km). Default:
one global pair of widths.

\[Leave as-is.\]

### D-4 Steep water liveliness (rapids/foam palette)
Rapids foam shipped (F-14). How lively should steep water be — foam
strength, whitecap color, where it triggers? Default: current subtle
foam.

\[Single-block increments are a result of rivers flowing downhill
and do not necessarily represent discrete jumps in water height.
These increments will probably disappear at some point id water
levels are added, allowing for smoother-looking rivers. In areas with
rougher terrain, waterfalls with up to several dozen-block jumps could
be implemented into generation and white particles could apppear along
the downwards trajectory and at the bottom of the waterfall.\]

### D-5 Weather: year length and starting season
Spectacle (short years, seasons visibly move) vs realism (long years).
Default: 20-day years (day_len 1200 s x 20), epoch spring. Tunable live
in weather_tuning.json — good demo material for deciding together.

\[My current thoughts: 30 minutes per day, 7 days in a lunar cycle,
3 lunar cycles per season, and 4 seasons per year. This should
definitely be adjustable, as I have thoughts of adding an idle travel
mechanic with time passage to curb the gameplay difficulties that
come with a planetary-scale game, and longer timescales would be much
more ideal for servers that run 24/7.\]

### D-6 Cloud art direction
Soft painterly masses (ships today) vs crisp cumulus cells — the shell
noise is swappable. Default: painterly-soft.

\[I would opt for clouds that lean on the realistic side. They don't
need to have realistic simulation or full-detail, but they should
change over time and correlate with weather, which should result in
different types of formations. They should be extended into the
third demension, with potenetially multiple layers of clouds,
however full 3d simulation isn't necessary and they can be partially
faked based on a 2d rendering of some sort.\]

### D-7 Overcast gloom cap
How dark may a fully overcast day get? Default keeps ~35% direct light
so screenshots stay readable (overcast_sun_floor knob).

\[This can be adjustable, but I'd opt for whatever is realistic.\]

### D-8 Storm frequency
How often should a given valley see rain — weather as event vs
wallpaper? (storminess bias knob.) Default: event.

\[I would use the overall percipitation statistic, however some more
interpolation can be done at a smaller scale to favor rain in crevices
over peaks.\]

### D-9 W4 scope: seasonal ice and melt
The biggest gameplay consequence on the board: lakes that freeze into
winter paths, melt seasons, snow accumulation. Needs a design doc + a
suite time-model strategy before any code (WEATHER.md W4). Decide
whether this world wants it at all, and how deep.

\[Water should generate variably as either frozen
or not based on the season according to temperature and weather
statistics, and should update when revisited. Manually placed ice
or water should only break this rule if tempature warming or
cooling systems exist, if they are even implemented.\]

### D-10 Renderer shading truth: steep slopes (BUGS.md V-9 residual)
Blocks self-shade steep ground ~8/255 darker than the mesh's diffuse
(the polar-ice half of V-9 turned out to be a bug and is fixed; slopes
and a small +3.4 ice residual remain). Which look is right — the
blocks' chiseled darkness or the mesh's softer diffuse? One shared rule
will follow the verdict. Default: they disagree slightly.

\[It's fine if they disagree, although the shading differences should
roughly cancel out at a large scale so there are no obvious differences
in shading at the boundary between voxels and mesh.\]

### D-11 Karst art direction (BUGS.md V-10)
Cave-mouth pools and pits are canon (07-10, Austin, "karst frigging
awesome" — confirm when you land). Open art calls: dry-pit mouth
styling (currently exposed dirt, lightly shadowed), pool color, and
whether breach density should vary more by climate (wetter = more
sinkholes?). Default: current look, density as the cave noise gives it.

\[I like the idea of cave generation being affected by biome
properties in a somewhat-realistic way. I'l leave the specifics up to
you.\]

### D-12 River-bank aesthetics (banked 07-10 for an Andrew session)
Austin's pick from the proposals: meander-curvature asymmetry (cut bank
steep, point bar shallow — proposal 4) composed with wet margins (2)
and bank-width noise (3). Waiting for a design session, then a mission.

\[I agree with Austin's proposal. Additionally, I think the extent to
which rivers meander should be a variable based on flow or width as it
is in real life. e.g. the Lower Mississippi vs. the Uppper Mississippi.
\]

### D-13 Ice-shelf edges (BUGS.md S-3)
Frozen lakes/rivers end in multi-block ice cliffs above dry ground. An
ice shelf arguably HAS an edge — is this a feature or a wall? (The
extreme 600 m cases are W-5b planetgen work regardless.) Default: keep.

\[I don't know where it is realistic for ice cliffs vs. tapered
glaciers to form, but both should definitely exist. For example, the
Antarctic coastline has large cliffs, while mountain valley glaciers
tend to taper off.\]

### D-14 Mega-lake apron dirt steps (BUGS.md W-7 caveat)
The shore apron trades standing-water cliffs for tens-of-metres DIRT
steps at Voronoi radius flips. Natural-looking terracing or ugly?
Revisit alongside the planetgen mega-lake fix. Default: keep.

\[Small and/or considerably slanted terrace-like formations can exist
but large cliffs shouldn't form around lakes in smooth-terrain areas.
I don't know if i've seen this generation before, can you point at
an image where it occurs? Is this the bug in that image where the
desert lake was cut off by a seam in the terrain?\]

### D-15 The texture/palette pass (the big one)
Per the 07-09 verdict the current climate-tint colors are provisional;
final block textures come first, then the tint map takes each texture's
average color. This is Andrew's own project: texture style, resolution,
palette. Everything color-related upstream (D-2, D-10, D-11) partially
lands here.

\[Same message as before, the current biome and block colors are
satisfactory and can be tweaked later when textures are added.\]

### D-17 Multiplayer time authority
Time travel and fast-forward are global actions on a shared server:
who may bend time? MP1 default until decided: the server operator
only (clients' [ ]/Travel controls disabled with a note). Options:
op-only, per-player permission, majority vote, "anyone" (chaos mode,
which honestly might be fun for you two).

## Answered (the design record)

### 2026-07-12 - D-16 Eclipses + solar-system idea verdicts (Andrew,
brackets in SOLAR.md)
Eclipses: ANSWERED at proposal time - "much greater rate than on
Earth, but... irregular, that's somewhat part of the charm; use the
orbital parameters to tune this factor. Copper-tinting [lunar] is a
nice feature, and solar eclipses should reduce the light coming from
the sun proportional to the occluded area until the sky becomes
night-like." Tides: DEFERRED behind water levels + revisit-time block
updates (the seasonal-ice prerequisite). Comet collision events:
DEFERRED behind prerequisites; astronomy/calendar features accepted.
Sun surface (spots, flares, auroras): accepted, deferrable. Trojan
asteroid: "good test to add, proceed or defer as is necessary."
NEW REQUIREMENT: the teleport map must cover the moon once P2
(moon generation) completes.

### 2026-07-11 - Andrew's bracket verdicts (in place above, summarized)
CLOSED as decided: D-1 (seamless stays; color fine-tuning later), D-2
(mottling ignorable - textures will replace it), D-3 (edge widths
as-is), D-10 (renderers may disagree if it cancels at scale - the rim
must not show), D-15 (colors satisfactory until the texture pass).
NEW BACKLOG from verdicts: D-4 waterfalls (multi-block drops + particle
spray - generation feature); D-5 time-scale system (30-min days, lunar
cycles, seasons - all tunable; idle-travel mechanic noted); D-6 clouds
v2 (realistic-leaning, weather-correlated formations, faked 3D layers);
D-8 crevice-biased rain interpolation; D-9 seasonal ice spec (variable
frozen state by season, updates on revisit - W4 design doc next); D-11
karst density by climate (specifics delegated to the AI crew); D-12
river meander scaling by flow + Austin's bank proposals (design session
ready); D-13 both ice cliffs AND tapered glaciers by context.
D-14: Andrew asked to see the formation - filed
shot_lat4.377_lon39.078_alt0.400km (titled 'D-14 example') with an
explanatory note in its sidecar; NOT the desert-lake shear he recalls
(that was the pond rampart, reopened as A-4). His bar: small/slanted
terraces fine, no big cliffs on smooth terrain - now the W-7 acceptance
criterion.


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

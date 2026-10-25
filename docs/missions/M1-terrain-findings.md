# M1-terrain: landforms v2 (Opus, 2026-10-25)

Summarised by the coordinator from the agent's report; merged at `604854c`.

## What it adds

Height is the bake × the vertical scale, plus a landform layer that is a
pure function of (seed, direction, voxel size), in
`game/src/worldgen/landform.rs`:

- **Massif:** a warped ridged-noise network. Relief comes from roughness,
  uplift × elevation, and a floor. The wavelength grows with relief, so
  mountain slopes approach about 0.75 while lowlands stay gentle.
- **Gullies:** slope-aligned stripe noise, deeper on steep ground.
  Finer gullies following coarser walls (`GULLY_BRANCH`) gave real
  branching but needle spikes, so it is off for now.
- **Mesas:** mean-preserving terraces that follow the strata, on hard,
  quiet, dry, raised rock (about 2% of land).
- **River valleys:** carved along ridges of planetgen's discharge field,
  with a flat floor, walls and a dry gravel channel. Water comes in M2.
- **Dials** (`world.json`, CLI): `roughness` (metre-scale texture) and
  `landform` (0 = the bake only, 2 = double relief).
- **Look changes for Andrew:** temperature drops with landform height, so
  ridges get snow first, and hard domes in arid places get terrace rings.

Review fixes in the same work:
- oceans kept at every LOD;
- a `classify` / `generate` split, with `generate` always correct;
- a bedrock floor at layer 0;
- conservative footprint bounds;
- a cube-edge seam fix (bake gradient taken across face edges), with a test
  that walks all 24 face sides.

## Numbers

| Measure | Before | After |
|---|---|---|
| Median land slope at 1/70 | 0.002 | 0.060 |
| Median land slope at 1/20 | 0.007 | 0.075 |
| Near-flat ground at 1/70 | 94% | 15% |
| Lattice-direction bias in the detail | 1.89–2.33 | 1.09–1.17 |
| Generation per chunk, median | 2.3–3.1 ms | 3.1–4.0 ms |
| 1/70 test flight (average / 1% low fps) | 836 / 292 | 591 / 173 |
| 1/8 test flight | 24 fps | 14 fps |

The bias row compares the strongest and weakest slope directions.
The generation numbers were measured while another engine was running.
The 1/8 flight was too slow to fly before and after; that is
M1c-stream's main-thread work.

Large-scale fidelity holds: the mean departure from the bake over 10–20 km
windows is 0.4–1.4 m RMS.

## Known limits

- Gullies can look combed: many parallel contour steps. Judge this under
  the new sky.
- Rivers fade out within 2–6 texels of face edges.
- Faint arcs on the sea floor at the steepest coast.
- Branching is weaker than hoped.

## Evidence

Before/after contact sheets in `M1-terrain-evidence/`. Rows are 1/70,
1/20 and 1/8 at 1 m blocks, plus 1/20 at 0.5 m blocks:
- the highest range from the ground and from 1 km;
- a coast, a river valley, plains and a mesa;
- plan-view hillshade maps.

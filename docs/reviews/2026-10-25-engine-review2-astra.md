# Engine review 2 (GPT-6 Astra, 2026-10-25)

Independent, read-only review of `game/` from `e3b84cc` to `f6c5bc1`
(M1b, M1-terrain, M1c-sky, M1c-stream). Opus verified #1, #2, #3 and #5 by
reading the code. #9 means the water-seam claim in the M1c-stream merge
was wrong: the patch moves vertices onto the sphere, but the merged quads'
straight edges still disagree by up to 23 m at L9/L8.

**Routing:**
- M2a (Fable, in progress): #1, #2, #4, #5, #11, #14, #15, #16.
- Harness / video task (Opus): #3.
- Worldgen hardening (Opus): #12, #13.
- After M2a, hardening: #6, #7, #8, #9, #10, #17.

---

Reviewed `e3b84cc..f6c5bc1` through source inspection and read-only CPU calculations against the shipped projection table. No builds, engine/test execution, GPU work, delegation, or file creation.

The save path still has data-loss bugs. Several streaming and geometry fixes also need follow-up before M2 relies on them.

1. **[P1] Save replacement is not atomic or durable.** [world/save.rs:333](C:/code/triangulum/game/src/world/save.rs:333)  
   `write_atomic` deletes the destination before renaming the temporary file. A crash between those operations loses the previous chunk; loading ignores the surviving `.tmp`. Neither file contents nor publication are explicitly synchronized. Furthermore, `flush` publishes chunks individually: an M2 fall across `k=32→31` could save the destination before the source, duplicating sand after a crash. **Fix:** use atomic replacement without unlinking, durable writes, and a journal or generation manifest that commits all chunks and simulation metadata for one snapshot together. **Confidence: certain.**

2. **[P1] A failed save permanently clears unsaved work.** [world/save.rs:300](C:/code/triangulum/game/src/world/save.rs:300), [world/edits.rs:193](C:/code/triangulum/game/src/world/edits.rs:193)  
   `take_dirty()` drains every dirty key before writing the header or chunks. Disk-full, permission, or rename failure returns early, but subsequent autosaves and exit saves no longer retry the unwritten chunks. **Fix:** snapshot dirty revisions without clearing them; acknowledge each committed revision only after successful persistence, retaining newer edits and all failed writes. **Confidence: certain.**

3. **[P1] The play harness can delete an ordinary save.** [main.rs:303](C:/code/triangulum/game/src/main.rs:303)  
   `--play SCRIPT --save saves/Neisor` recursively deletes that explicitly supplied directory unless `--keep-save` is present—even before the script is read. Cleanup is not restricted to the harness’s generated scratch directory. **Fix:** automatically reset only harness-owned scratch saves; require an explicit reset option for supplied destinations and report cleanup errors. **Confidence: certain.**

4. **[P1] The bake identity does not identify bake contents.** [world/save.rs:48](C:/code/triangulum/game/src/world/save.rs:48)  
   The digest hashes metadata, filenames, and lengths, but no raster bytes. Changing elevation or hydrology values without changing metadata or dimensions passes compatibility checks and changes untouched terrain around saved edits. The test at line 383 explicitly accepts different equal-sized contents. **Fix:** hash raster contents while loading, or validate a content-addressed bake manifest. Retain portable golden chunk fixtures for generator compatibility. **Confidence: certain.**

5. **[P1] Hiding water can permanently drain edited chunks.** [engine.rs:172](C:/code/triangulum/game/src/engine.rs:172), [world/mod.rs:225](C:/code/triangulum/game/src/world/mod.rs:225)  
   Water visibility changes the authoritative generator. Press F, edit one block in a previously untouched ocean chunk, and its complete *dry* generated contents become authoritative. Turning water back on or reloading preserves that artificial drainage. Previously edited chunks can conversely retain water while it is hidden. **Fix:** keep canonical generation independent of rendering flags; suppress water during meshing/drawing. **Confidence: certain.**

6. **[P1] Breaking across a face edge changes the wrong block.** [player.rs:163](C:/code/triangulum/game/src/player.rs:163)  
   Picking queries `vox.block`, which resolves out-of-face cells, but constructs `Hit.block` by clamping coordinates onto the original face. A ray from face 0 hitting face 2 therefore reports face 2’s material while `break_block` modifies face 0’s boundary column. **Fix:** construct the hit’s canonical position using `wrap_cell`; reject out-of-world radial hits rather than clamping them. **Confidence: certain.**

7. **[P1] Excavation can leave missing tunnel walls after reload.** [stream.rs:1074](C:/code/triangulum/game/src/stream.rs:1074)  
   Admission checks natural classification plus edits *inside* the candidate chunk. Digging a boundary block exposes a face belonging to its unedited, naturally enclosed neighbour. On reload or later refinement, that neighbour is excluded, so its wall never renders. The coverage checker cannot detect a node that was never admitted. **Fix:** classify the edited padded neighbourhood, or conservatively admit chunks whose halo intersects authoritative changes, including across faces. **Confidence: certain.**

8. **[P1] Allocation failure can permanently strand an outdated mesh.** [stream.rs:1138](C:/code/triangulum/game/src/stream.rs:1138), [stream.rs:1227](C:/code/triangulum/game/src/stream.rs:1227)  
   Start a replacement job, edit again, then let the stale result encounter allocation failure. It is parked before revision validation, subsequently dropped, and `unpark` sets `Job::None`. The old resident mesh remains, so the normal absent-mesh dispatch rules never request its replacement; streaming can report settled. Dropping a current replacement at the awaiting-queue limit has the same scheduling problem. **Fix:** track mesh revision separately from desired revision and keep stale residents explicitly queued; validate results before allocation. **Confidence: certain.**

9. **[P1] The exact-radius correction does not close actual water seams.** [render/shader.wgsl:117](C:/code/triangulum/game/src/render/shader.wgsl:117), [mesh.rs:253](C:/code/triangulum/game/src/mesh.rs:253)  
   Uniform water merges into eight-voxel quads. Their grid-corner vertices already have exact radii, but rasterization draws straight edges between them. A finer neighbour introduces intermediate vertices above that chord. CPU calculation using the existing test’s boundary geometry leaves **23.18 m** radial separation at L9/L8. The test evaluates a hypothetical coarse midpoint vertex that need not exist in the mesh. **Fix:** stitch or constrain actual boundary vertices to a shared representation, and test emitted triangle edges. **Confidence: certain.**

10. **[P2] Face transitions can move the collision box into an unchecked wall.** [player.rs:390](C:/code/triangulum/game/src/player.rs:390)  
    Collision sweeps use integer-wrapped old-face cells, but crossing movement uses a tangent-world step. Those continuations differ. With default parameters, feet at face 0 `(N−0.02, N−0.70, 600)` moving +i at walking speed query only row `N−1`; the next frame’s face-2 box overlaps row `N−2` by **0.0255 lattice units**. A wall there was never checked. **Fix:** split movement at chart boundaries and sweep the transformed remaining movement/body against neighbouring prisms; validate the complete resulting box. **Confidence: certain from CPU calculation.**

11. **[P2] Coarse halo invalidation uses a one-block displacement.** [world/mod.rs:253](C:/code/triangulum/game/src/world/mod.rs:253)  
    The border test operates in coarse voxels, but neighbour selection adds only ±1 fine block. At L1, editing `i=65` should invalidate chunk `cx=0`, whose padding spans `[64,66)`. Subtracting one gives 64, still selecting `cx=1`. Cached neighbouring faces remain stale; this affects all axes and face edges. **Fix:** step by `1<<lod` or compute adjacent chunk coordinates directly before wrapping. **Confidence: certain.**

12. **[P2] Coarsest oceans have no opaque backing.** [worldgen/mod.rs:490](C:/code/triangulum/game/src/worldgen/mod.rs:490), [render/mod.rs:476](C:/code/triangulum/game/src/render/mod.rs:476)  
    At default L9, the sea-preservation rule replaces the entire bottom voxel with water, before the bedrock rule. Warm ocean columns consequently have no opaque interior geometry. The sky renders behind them, then water blends over it: stars or the sun can show through the planet from orbit. At normal incidence, default alpha leaves approximately 38% background contribution. **Fix:** retain separate opaque terrain/depth backing and water-surface representations at coarse LODs. **Confidence: certain.**

13. **[P2] Terrain clipped at the world ceiling can disappear.** [worldgen/mod.rs:425](C:/code/triangulum/game/src/worldgen/mod.rs:425)  
    `Enclosed` classification ignores the world-top air boundary. For H=512, sea=256, L0 `cz=15`, and a footprint minimum of 600 m ASL, classification says enclosed even though generation makes layer 512 air, exposing the chunk’s top. High terrain in a shallow world therefore vanishes during refinement. **Fix:** prevent enclosed classification when the padded extent crosses the world ceiling and preserve clipped surface bounds. **Confidence: certain.**

14. **[P2] Structurally valid corrupted chunks can crash after loading.** [world/save.rs:327](C:/code/triangulum/game/src/world/save.rs:327), [world/blocks.rs:89](C:/code/triangulum/game/src/world/blocks.rs:89)  
    Chunk decoding validates palette indices, but loading never validates palette block IDs against the registry. A damaged palette containing ID 65535 decodes successfully and later panics during solidity lookup. Coordinate bounds and duplicate embedded chunk keys are also unchecked. **Fix:** validate IDs, keys, and duplicates before installing anything; add checksums and reject inconsistent save directories, including chunks without a header. **Confidence: certain.**

15. **[P2, M2 trap] Authoritative storage currently discards water state.** [world/save.rs:293](C:/code/triangulum/game/src/world/save.rs:293), [world/edits.rs:26](C:/code/triangulum/game/src/world/edits.rs:26)  
    Loading converts chunks to IDs only; saving reconstructs every water cell as level 255. An M2 level-17 cell would reload full. `insert_chunk(..., dirty=true)` also bypasses mesh invalidation.

    **Change now:** let World’s authoritative store hold IDs, water bytes, and revisions. Give Simulation ownership of the ordered update queue, active region, and sleeping state. Keep persistence dirt and mesh invalidation separate, fed by one batch-commit API. Derive activity from canonical player/world coordinates, independent of render residency. Snapshot queue/tick/input position with chunk state, and obtain boundary values through canonical L0 generation. **Confidence: certain as a design trap.**

16. **[P2, determinism] Input mutations and clocks still depend on frames.** [app.rs:170](C:/code/triangulum/game/src/app.rs:170), [app.rs:181](C:/code/triangulum/game/src/app.rs:181), [play.rs:113](C:/code/triangulum/game/src/play.rs:113)  
    Physics drops accumulated time after eight steps; held clicks repeat by wall clock at most once per rendered frame. The harness advances player steps but never advances the renderer-owned sky clock. Identical held inputs can therefore produce different movement, edits, and sky state under different frame schedules.

    **Fix:** assign inputs integer tick/sequence positions and execute mutations through the shared session/simulation driver. Keep the 20 Hz tick independent of sky speed; persist sky time separately. Do not drive update ordering from unsorted `EditStore::keys()` or worker arrivals. **Confidence: certain.**

17. **[P2, performance] Changed-world residency and coarse regeneration grow with edit history.** [world/edits.rs:26](C:/code/triangulum/game/src/world/edits.rs:26), [world/mod.rs:389](C:/code/triangulum/game/src/world/mod.rs:389)  
    Every touched chunk remains a dense 64 KiB array indefinitely: 10,000 chunks consume approximately **625 MiB** before indexing and snapshots. Coarse overlays scan all fine blocks in overlapping edited chunks and fetch their natural chunks again. Large changed regions exceeding the 1,024-entry natural cache can repeatedly regenerate them during LOD changes. **Fix:** page persisted inactive authoritative chunks, retain compact change/material summaries, and update coarse summaries incrementally from simulation commits. **Confidence: certain.**

Prior review status:

| # | Original finding | Status |
|---|---|---|
| 1 | Face-edge neighbour sampling | **Fixed** — integer transforms preserve spacing. |
| 2 | Coarsest oceans disappear | **Partly fixed** — water survives; opaque backing is lost. |
| 3 | Allocation failure becomes empty coverage | **Partly fixed** — parked replacements can lose retry scheduling. |
| 4 | Enclosed generation replaces real materials | **Fixed** — full contents generated. |
| 5 | Untiled planet strips | **Fixed** — unsupported configurations rejected. |
| 6 | Sea-level setting creates billions of roots | **Fixed** — divisibility guard added. |
| 7 | Missing bedrock floor | **Fixed** — L0 layer zero enforced. |
| 8 | Unsafe horizon occluder | **Fixed** — world-bottom core used. |
| 9 | Undersized corner bounds | **Fixed** — original corner fits new shared bounds. |
| 10 | Warped-footprint/bicubic bounds | **Fixed** — expanded footprint sampling and 0.28125 overshoot allowance. |
| 11 | Water LOD cracks | **Not fixed** — actual triangle edges still disagree. |
| 12 | Overlapping/incomplete fallback selection | **Fixed** — atomic coverage cuts; edit-admission defect is separate. |
| 13 | Renderer owns authoritative world state | **Partly fixed** — independent edits/revisions/body keys; M2 state contract incomplete. |
| 14 | Missing durable generator identity | **Partly fixed** — versions/table hash present; raster contents unverified. |
| 15 | Unbounded streaming caches/history | **Fixed** — bounded caches and benchmark-only timing history. |

Found clean, within static-review limits:

- Single-edge integer transforms and `extended_coords` inversion; body-scoped chunk and footprint keys.
- Full L0 strata generation and bedrock precedence.
- Original corner-bounds case: 33.805 m extent inside the new 34.373 m radius.
- Fixed-order generation and sorted overlay/save snapshots; no thread-dependent floating reduction found.
- Palette packing, structural byte decoding, and little-endian serialization.
- Sky ephemeris evaluation, uniform layout, reversed-Z sky ordering, and camera-relative subtraction in f64.
- Sneak’s ordinary single-face support checks and axis sweeps.

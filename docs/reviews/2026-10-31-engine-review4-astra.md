# Engine review 4 (Astra)

Independent read-only review of everything merged after review 3 (M2c replays,
M3a look panel, M2-hardening, M3b block editor), `gpt-6-astra`, about 25 minutes,
on `54d21eb`. Filed unedited below. **The lead has not re-verified these yet**;
each gets a test that fails first when it's fixed.

**Routing** (a hardening mission after the bug round's first wave, so it doesn't
collide with work in progress):

- #1, #2, #3: the P1s, all about keeping a player's save safe. First.
- #4, #5: simulation versioning and restore. After B4 lands, which changes the
  rules version. #4 also decides whether B4's rule change protects saves.
- #8, #9: what "reproduced exactly" promises. Before B6 builds bug reports on it.
- #6, #7, #11, #13: textures and block identity.
- #10: input during catch-up frames.
- #12: load cost on an older save.

---

**Engine review 4 — 2026-10-31**

Reviewed `6a590d5..54d21eb` read-only, including the mission claims and review 3. Excluded the bug-round items and review 3 #8/#11.

No builds, engine tests, GPU runs, or files written. Evidence below distinguishes code inspection from in-memory calculations. Those calculations used the committed replay fixture, current source algorithms, and installed dependency source; they are not engine reproductions.

1. **[P1] An embedded replay manifest can escape the scratch save and delete a real save’s simulation file.** [world/save.rs:518](game/src/world/save.rs:518), [replay.rs:1209](game/src/replay.rs:1209)  
   `install_image` validates the image’s filenames, but not file references **inside** `commit.json`. A manifest can reference `../../Neisor/sim.g7.bin` with its correct digest. Loading reads that external file; a recorded `reload` writes the next scratch generation, then deletes the external file as “superseded.” The real save’s commit still references the deleted file.

   A recording already contains the simulation bytes and their digest, so this does not require defeating SHA-256. Validate every manifest reference and require it to belong to the installed image before reading, saving, or removing anything.  
   **Confidence: certain.** Demonstrated the filename-guard bypass and escaping Windows path in memory; deletion was not executed.

2. **[P1] A failed “Add block” write can make an existing save unloadable.** [workshop.rs:479](game/src/workshop.rs:479), [world/save.rs:468](game/src/world/save.rs:468)  
   Make `blocks.json` unwritable while the save directory remains writable, then add a block. The registry publishes the new ID before `write_defs` runs. Failure produces a transient note but leaves the block live. The next save extends its header with **every** new registry ID—even if the player never placed the new block. Restart then refuses the existing save because its header requires a definition absent from disk.

   Restoring permissions and pressing **Save look** does not retry the missing definition: that operation updates existing entries’ colours and textures. Publish the definition successfully before exposing its ID, and retain an explicit retry state on failure.  
   **Confidence: certain; code inspection.**

3. **[P1] Saves can accept block IDs whose identities were never recorded, then reinterpret those blocks.** [world/save.rs:438](game/src/world/save.rs:438), [world/save.rs:345](game/src/world/save.rs:345)  
   Loading checks palette IDs against the current registry’s length, not membership in the save’s block table. Older builds could append block 19, use it in an existing save, and leave that save’s original header unchanged. Loading that save with a different block at 19 accepts the replacement identity.

   The new extension also misses gaps: start with IDs 0–18 and 20, save, then define and use 19. Because extension only considers IDs above the header’s maximum, 19 remains unrecorded. Changing its name later passes the header check. Refuse used IDs lacking saved identity; extend by missing IDs, not maximum ID.  
   **Confidence: certain.** Demonstrated the gap, successful identity check after substitution, and numeric palette check in memory; verified the older writer’s missing extension in the baseline source.

4. **[P2] Simulation rule-versioning protects replays but does not protect saves or `Sim::state_hash`.** [sim/mod.rs:199](game/src/sim/mod.rs:199), [sim/mod.rs:745](game/src/sim/mod.rs:745), [session.rs:299](game/src/session.rs:299)  
   `RULES_VERSION` is recorded and checked only by replay code. Neither save headers nor simulation snapshots carry it, and neither state hash includes it. Bumping it for a falling-rule change therefore leaves old saves loading and permits identical hashes for states governed by different algorithms.

   I verified every embedded-file digest in the committed M2c replay: its save format is **3**, simulation snapshot format **2**, and recorded block-rules digest equals the current registry’s digest. Those save checks cannot detect its older simulation algorithm. Persist, check, and hash the algorithm version independently of block flags.  
   **Confidence: certain.** Fixture bytes verified; loader and hash paths inspected.

5. **[P2] Restoring with a larger simulation radius can recreate the invisible dam.** [sim/mod.rs:908](game/src/sim/mod.rs:908), [sim/mod.rs:553](game/src/sim/mod.rs:553)  
   Save a settled trough with water against the active region’s edge, then load with a larger `--sim-radius`. Restoration recomputes membership using the new radius but sets `region_known = false`, so it skips `wake_new_pairs`. Frozen empty cells wake; the sleeping donor does not. The newly active half can remain dry.

   There is also a snapshot-equivalence failure: call `set_radius`, then snapshot before the next tick. It clears the serialized focus while retaining the live old region. The restored state loses that history; equal starting hashes can produce different next ticks. Preserve the previous region’s determining state or finish the transition before snapshotting.  
   **Confidence: high; code-derived from the boundary-trough fixture, not run.**

6. **[P2] PNG size validation occurs after the dangerous allocation and decoding work.** [render/textures.rs:53](game/src/render/textures.rs:53)  
   A PNG declaring 65,536 × 65,536 RGBA pixels requests a **16 GiB** output buffer before reaching the 8,192-pixel dimension guard. The installed PNG decoder’s allocation limit explicitly excludes this caller-owned buffer. An oversized or damaged image can therefore exhaust memory instead of producing the intended rejection.

   Accepted large images also cause substantial main-thread work: an 8,192² image downsampled to 16² visits **67,108,864 source pixels**, performing **201,326,592 channel conversions**. Import and hot reload perform this synchronously. Check header dimensions and decoded-byte budgets before allocation; move substantial decoding/resampling off the game thread.  
   **Confidence: certain about allocation order and operation counts; termination and latency not measured.**

7. **[P2] Texture-array growth ignores the device’s layer limit.** [render/mod.rs:418](game/src/render/mod.rs:418), [render/textures.rs:363](game/src/render/textures.rs:363)  
   On a device reporting 2,048 array layers, 2,049 different referenced PNGs request a 4,096-layer texture. Neither registry validation nor texture assignment checks the limit. Wgpu rejects the allocation through its uncaptured-error path, terminating the game instead of rejecting the asset change.

   Layers also accumulate after references are removed, so repeated texture replacements can reach the limit without that many currently textured faces. Bound capacity against device limits and reclaim unused layers.  
   **Confidence: high.** Capacity arithmetic demonstrated; installed wgpu validation/error paths inspected; GPU failure not run.

8. **[P2] A replay can report exact reproduction with a different look and different textures.** [replay.rs:427](game/src/replay.rs:427), [replay.rs:603](game/src/replay.rs:603)  
   Record with altered fog, water opacity, block colours, or painted textures, then play on a machine with different asset files. The replay stores none of those values or images. Its “view” digest checks only sky seconds, sky speed, and water visibility. Playback loads the receiving machine’s look and textures, and all checks can pass despite materially different frames.

   Workshop changes during recording are likewise absent. This undermines F10 reports of rendering defects. Capture the effective visual settings and asset identities/content, including changes during recording, and distinguish simulation reproduction from visual reproduction.  
   **Confidence: certain; code inspection.**

9. **[P2] An unchecked or partially checked recording receives a successful “reproduced exactly” verdict.** [replay.rs:1257](game/src/replay.rs:1257), [replay.rs:1269](game/src/replay.rs:1269)  
   A recording cut off before its first checkpoint can load its valid snapshot, execute available inputs, match **zero checks**, and return `ok = true` and exit status 0. A longer recording’s tail after its last checkpoint is similarly unverified. Missing `end` produces a warning but does not prevent the exact-reproduction verdict.

   Report the verified prefix and an incomplete/unverified result separately. Exact success must require coverage through the claimed endpoint.  
   **Confidence: certain; code inspection of parser, runner, report, and exit status.**

10. **[P2] Catch-up frames apply newly arrived input to earlier simulation steps.** [controls.rs:154](game/src/controls.rs:154), [replay_tests.rs:127](game/src/replay_tests.rs:127)  
    Deliver W-down at **22 ms**, W-up at **44 ms**, with the same starting accumulator. At 30 fps, the first frame consumes the press for both owed steps: movement steps **1 and 2**. At 144 fps, movement occurs only on step **2**. At the default fly speed, that is approximately **0.667 m versus 0.333 m** of travel.

    The cross-rate test avoids this case by delivering events at preselected even simulation steps. Timestamp event arrival and assign it to the corresponding elapsed step instead of applying the latest held state to the entire backlog.  
    **Confidence: certain.** Demonstrated with an in-memory implementation of the current input/accumulator rules; no window input generated.

11. **[P2] The first missing texture is never polled for recovery.** [workshop.rs:425](game/src/workshop.rs:425), [render/textures.rs:325](game/src/render/textures.rs:325)  
    Start with no loaded textures, then hot-reload `blocks.json` referencing a PNG that is missing or temporarily incomplete. Assignment records a missing-file watch, but `TextureSet::is_empty` checks only successfully loaded layers. The workshop consequently skips `poll` forever. Creating or completing that PNG does not make it appear until another assignment or restart.

    This catches a normal pull/write ordering where JSON arrives before the first PNG. Poll outstanding missing-file watches even when there are no loaded layers.  
    **Confidence: certain; code inspection.**

12. **[P2, performance] Loading an older save can perform quadratic registry cloning and hashing.** [sim/mod.rs:230](game/src/sim/mod.rs:230), [sim/mod.rs:217](game/src/sim/mod.rs:217)  
    `rules_extend` recomputes every candidate prefix, and each attempt clones the **entire** registry before taking that prefix. Loading a 19-block save with a current 4,096-block registry performs **4,078 full clones: 16,703,488 cloned definitions and 8,390,485 hashed entries**. This runs synchronously during session loading and gets much worse toward the accepted registry capacity.

    Clone/read the definitions once and calculate prefix digests incrementally, or record the saved registry length.  
    **Confidence: certain about complexity and counts; wall-clock latency not measured.**

13. **[P3] Renaming a block back to its original name changes its replay identity.** [world/blocks.rs:597](game/src/world/blocks.rs:597), [world/blocks.rs:105](game/src/world/blocks.rs:105)  
    Rename `brick → red_brick → brick`. The final `was` becomes `["red_brick"]`; `first_name()` therefore changes from `brick` to `red_brick`. For a world containing that block, the rules digest and state hash change despite identical IDs, physics, and final names. Earlier replays report divergence.

    Keep an immutable original identity independently of the alias list.  
    **Confidence: certain.** Demonstrated the current rename algorithm’s resulting names and identity in memory.

Routes checked and found clean, within these review limits:

- Normal replay startup disables the ordinary save and uses a sanitized scratch name. The manifest-reference escape in #1 is the remaining route identified.
- Terrain previews use separate edit stores, suspend the home simulation, remove the save destination, save dirty home state before switching, and stop recordings before world swaps.
- Straightforward deletion or renumbering of existing named blocks is rejected during hot reload. Ordinary append-only header extension occurs before publishing chunks that may use those IDs.
- Save publication orders file synchronization and directory synchronization before acknowledgement and cleanup. Failures propagate. A surviving commit without its header is refused by loading, simulation loading, and writing.
- Bake identity now hashes the same raster and hydrology bytes consumed by loading; the metadata-only digest cache is gone.
- Ordinary restored regions retain their focus. Live region expansion wakes both endpoints; steady boundary throughflow reschedules; falling sweeps and displacement enforce active-write boundaries. #5 concerns restoration across a radius transition.
- No-op materialization is removed; water-array emptiness uses a maintained count; packed-ID widening preserves the dense representation.
- A normal `texture_size` change resamples layers and recreates the GPU array. Colour-buffer growth recreates its binding. Ordinary malformed JSON/PNG decode failures retain the prior live state.
- Video flight now advances `Session::step`, and video rendering refreshes falling draws.

Cross-machine floating-point identity, physical power-loss durability, GPU behaviour, and runtime performance were not exercised. The worktree remains unchanged.

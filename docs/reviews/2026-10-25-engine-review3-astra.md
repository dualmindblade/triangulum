# Engine review 3 (GPT-6 Astra, 2026-10-25)

Independent, read-only review of `game/` from `f6c5bc1` to `e5dbdba` (M2a,
M1-hardening, M2-hydro, M2b, M1-video), with a check of review 2's fix
claims. Opus confirmed #2, #3, #4 and #6 by reading the code. The water
rule's proof holds: Φ falls and closed water conserves. The weak points are
where the loaded area's edge moves, and what the state hash leaves out.

**Routing:**
- M2c (Fable, in progress): #4, #6, #13; they decide whether a replay is trustworthy.
- M2-hardening, after M2c: #1, #2, #3, #5, #7, #9, #10, #12, plus review 2 #17 (memory), now worse.
- After M3a (it owns `mesh.rs`'s colour path): #8 (water corners).
- Low priority: #11 only bites at N ≥ 1,048,576.

---

Reviewed `f6c5bc1..e5dbdba` read-only, including the fix claims. No builds, engine tests, GPU work, delegation, or files created. CPU calculations used the shipped projection table and installed hydrology bake.

The closed-water settling argument holds. Boundary activation and the snapshot/hash contract need fixes before replays depend on them.

1. **[P1] Save publication still lacks a Windows durability barrier.** [world/save.rs:552](C:/code/triangulum/game/src/world/save.rs:552)  
   The temporary file is synchronized **before** rename; directory synchronization exists only on Unix, where errors are ignored. `flush` then acknowledges revisions and removes superseded files. A power loss immediately after successful publication can therefore lose an acknowledged save; durable recovery is not established. Windows distinguishes ordinary moves from write-through moves. [Microsoft documentation](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw).  
   **Fix:** use a platform-specific durable publication protocol, propagate synchronization failures, and retain the previous validated generation until publication is durable. The process-crash staging sequence is otherwise substantially improved. **Confidence: likely.**

2. **[P1] A missing header silently turns an existing save into an empty world.** [world/save.rs:425](C:/code/triangulum/game/src/world/save.rs:425)  
   If `header.json` is missing but `commit.json` and its chunks survive a damaged restore or filesystem failure, `load_into` reports success with zero chunks. Separately, `read_sim` can still restore that generation’s falling entities. Saved excavations disappear while their old simulation resumes against natural terrain. Subsequent edits can overwrite the hidden saved contents.  
   **Fix:** reject a committed save without its header; distinguish an absent save from an inconsistent one. **Confidence: certain.**

3. **[P1] Expanding the active region can leave a permanent invisible dam.** [sim/mod.rs:630](C:/code/triangulum/game/src/sim/mod.rs:630)  
   Settle water against inactive air at a chunk boundary. Its outside neighbour receives a frozen notification, while the inside water eventually sleeps. Move the region so both chunks become active: only the outside cell wakes. An empty active cell cannot pull sideways from active water, so it changes nothing and never wakes the donor. Water remains stopped at the former boundary until another disturbance. The same mechanism can strand water above newly active empty space.  
   **Fix:** when boundary membership changes, notify both endpoints of affected neighbour pairs, including already-active donors. Preserve untouched generated reaches elsewhere. **Confidence: certain.**

4. **[P1, M2c] Equal hashes do not imply equal future simulation.** [sim/mod.rs:532](C:/code/triangulum/game/src/sim/mod.rs:532), [sim/mod.rs:594](C:/code/triangulum/game/src/sim/mod.rs:594)  
   Neither snapshot nor hash includes `radius_m`, `enabled`, `water_period`, or active-region history. The region uses the exact position at the last chunk crossing, then remains cached. Walk within that chunk and reload: restoration clears the cache and recomputes around the new position, changing which boundary cells simulate despite an identical restored hash. Changing `water_period` also leaves the hash immediately unchanged; the test named “period_is_in_the_hash” contains no hash assertion.  
   **Fix:** derive membership from a canonical chunk anchor, or persist/hash its determining state. Record simulation settings and physics-definition identity; keep look-only colours separate. **Confidence: certain.**

5. **[P1] Bake identity is still separable from the bytes actually loaded.** [world/save.rs:126](C:/code/triangulum/game/src/world/save.rs:126), [main.rs:309](C:/code/triangulum/game/src/main.rs:309)  
   The digest cache trusts matching names, sizes and modification times. Replacing raster contents while preserving those attributes retains the old identity. Additionally, startup loads the bake first and hashes the paths afterward: swapping a bake between those operations associates the new digest with old in-memory terrain. A save can then pass compatibility checks against different terrain.  
   **Fix:** compute identity from the bytes consumed by the loader, including `hydro.bin`, or load an immutable, verified bake generation. Metadata alone cannot validate a cached content digest. **Confidence: certain.**

6. **[P2, M2c] No-op writes introduce unsaved state into the hash.** [world/edits.rs:237](C:/code/triangulum/game/src/world/edits.rs:237)  
   `commit` inserts a natural chunk before checking whether the write changes anything. Pouring into an already-full water cell can therefore create a clean, materialized chunk. `state_hash` includes it, but saving writes only dirty chunks. After reload that chunk disappears from the hash although every block and level is identical.  
   **Fix:** avoid retaining chunks created solely by no-op writes, and define the hash over canonical authoritative content rather than materialization history. **Confidence: certain.**

7. **[P2] Falling blocks can land in inactive chunks.** [sim/mod.rs:767](C:/code/triangulum/game/src/sim/mod.rs:767)  
   Activity is checked only at the starting cell. At one-metre scale, a block at `z=32.1` travelling near terminal velocity can cross into inactive chunk `cz=0` and land at layer 30 during that tick. `land_into` writes the frozen chunk and may displace its water. Otherwise it overshoots the boundary before freezing next tick.  
   **Fix:** split the sweep at active-region boundaries and suspend the entity there; apply a consistent boundary policy to landing displacement too. **Confidence: certain.**

8. **[P2] Sloped water surfaces are not consistently watertight.** [mesh.rs:238](C:/code/triangulum/game/src/mesh.rs:238), [mesh.rs:137](C:/code/triangulum/game/src/mesh.rs:137)  
   Two separate cases:
   - A partial cell elsewhere in chunk A enables corner averaging throughout A. Adjacent chunk B, with no partial cells in its padded grid, uses flat 255 corners. At a shared shoreline corner containing two full water cells and two air cells, A emits **232**, B **255**: approximately **9 cm** separation.
   - At a cube corner, four padded samples represent only three distinct cells. Different faces duplicate different cells. For levels **10, 50, 90** around the `+X/+Y/+Z` corner, the implemented wrapping and averaging produce **60 versus 40** at the same vertex: approximately **8 cm** separation.

   **Fix:** calculate each corner from its canonical incident cells, deduplicating the three-cell cube corner. Any flat shortcut must produce exactly the same result. **Confidence: certain.**

9. **[P2] Steady boundary throughflow can fall asleep and stop flux accounting.** [sim/water.rs:317](C:/code/triangulum/game/src/sim/water.rs:317)  
   Consider an active, supported level-100 cell beside an inactive level-200 reservoir and level-50 drain, with other sides blocked. It receives 50, becomes 150, then exports 50 and finishes at 100. No active-cell write is recorded, so nothing reschedules it. Subsequent ticks report no flow despite unchanged source/drain conditions.  
   **Fix:** track and reschedule cells that exchanged boundary flux even when their final level equals their initial level. **Confidence: certain.**

10. **[P2] Accepted world dimensions overflow `PosKey`.** [sim/mod.rs:117](C:/code/triangulum/game/src/sim/mod.rs:117)  
    The key permits only 20-bit `i/j` and 16-bit `k`, enforced solely by `debug_assert`. World configuration accepts larger values. With `N=2,097,152`, `(face=0,i=1,048,576,j=0,k=0)` aliases `(face=1,i=0,j=0,k=0)`. Release builds schedule updates at the wrong location; debug builds panic.  
    **Fix:** reject dimensions exceeding the packing limits at startup or widen the representation. Make body ownership explicit when extending this format for M4. **Confidence: certain.**

11. **[P2] Hydrology’s fixed run mask overflows on larger supported planets.** [worldgen/hydro.rs:915](C:/code/triangulum/game/src/worldgen/hydro.rs:915)  
    Meander sampling creates an unbounded number of search runs, but candidates store only a `u128`. A read-only calculation from the installed `hydro.bin`, using the implementation’s sampling formula, finds **243 links exceeding 128 runs at `N=1,048,576`, block size 1**; link 15897 needs 160. Shifts beyond bit 127 panic with overflow checks or alias bits in release, and nearest-point search cannot visit those later runs correctly.  
    **Fix:** use dynamically sized masks or sorted run-index lists; validate any retained fixed limit. **Confidence: certain.**

12. **[P2, performance] Emptying water can repeatedly scan whole chunks.** [world/edits.rs:113](C:/code/triangulum/game/src/world/edits.rs:113)  
    Every transition to level zero scans the level array to decide whether to free it. Clearing many cells while the remaining water lies near the array’s end costs roughly `cleared_cells × 32,768` comparisons under the edit-store write lock. This creates a drainage-dependent spike beyond the queue and commit costs already measured.  
    **Fix:** maintain a nonzero-water count, or normalize empty water arrays once per affected chunk after the batch. **Confidence: certain.**

13. **[P3, video] Recorded motion can freeze simulation or show stale falling entities.** [play/video.rs:615](C:/code/triangulum/game/src/play/video.rs:615), [play/video.rs:275](C:/code/triangulum/game/src/play/video.rs:275)  
    `fly` advances video/sky ticks without `Session::step`, freezing water and sand throughout the flight. Commands that do step the session still do not refresh `harness.falling` through `video_tick`; a recording can show absent or stationary falling cubes while the world advances.  
    **Fix:** route video timelines through one 60 Hz driver that advances the session and refreshes falling draws before rendering. **Confidence: certain.**

Status of review 2’s findings:

| # | Finding | Status |
|---|---|---|
| 1 | Atomic, durable saves | **Partly fixed** — generation commits work; durability gap above. |
| 2 | Failed saves clear dirt | **Fixed** — acknowledgement follows commit and checks revisions. |
| 3 | Harness deletes ordinary saves | **Fixed** — supplied destinations require explicit reset. |
| 4 | Bake content identity | **Partly fixed** — content hashing added; cache/loading gaps remain. |
| 5 | Hiding water drains chunks | **Fixed** — app visibility is a draw flag. |
| 6 | Wrong block picked across faces | **Fixed** — canonical hit position. |
| 7 | Missing tunnel walls after reload | **Fixed** — halo-aware admission. |
| 8 | Stranded stale meshes | **Fixed** — resident revision tracking and retry. |
| 9 | Actual water LOD seams | **Partly fixed** — greatly reduced; test still permits 3 m separation. |
| 10 | Collision at face transitions | **Fixed** — split sweep and destination-box validation address the reported case. |
| 11 | Coarse halo invalidation | **Fixed** — whole-voxel displacement. |
| 12 | Ocean lacks opaque backing | **Fixed** — opaque bottom-ocean representation. |
| 13 | Clipped world-ceiling terrain disappears | **Fixed** — ceiling-aware classification. |
| 14 | Corrupt/inconsistent saves accepted | **Partly fixed** — chunk validation improved; missing-header case remains. |
| 15 | Water state discarded | **Fixed** — authoritative levels persist through save/load. |
| 16 | Input/clock determinism | **Partly fixed** — held repeats improved; new video paths have the omissions in #13 above. |
| 17 | Edit-history memory growth | **Not fixed; worse** — wet chunks use 96 KiB, hashing clones all chunks, and refocusing scans accumulated frozen-chunk keys. |

Found clean, within static-review limits:

- **Closed-water conservation and Φ descent:** falls decrease Φ; integer averaging strictly decreases its square term. Later cells deliberately read earlier overlay writes. This preserves the proof. Reservoirs, placements and solid displacement are outside that proof.
- **Tick ordering:** ordered queue, sorted water commits and falling entities; no unordered HashMap or rayon traversal found feeding authoritative update order.
- **Streamer independence:** active membership and simulation reads use world coordinates and authoritative generation, not resident meshes.
- **Ordinary save interruption before publication:** staged generation files leave the previous manifest usable; failed writes retain chunk dirt; committed files have content digests.
- **Falling serialization:** position and velocity retain their exact floating-point bits. Falling motion and region geometry still use `f64`; this review does not establish cross-platform bitwise replay.
- **Hydrology format:** `hydro.bin` reader and bake writer agree; `hydro.json` is descriptive metadata. Candidate lists are sorted, and untouched generated water receives no automatic simulation updates.

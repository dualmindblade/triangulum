# Cross-review: fresh Claude context reviewing Sol-authored MP1 / W2.5 / W2 / Moon (2026-07-14)

Scope: 480de3f (MP1), f4b3dca (W2.5), 2f6da40 (W2), 9548999 + 1f9c950
(moon), as merged at 999ed70. MP1 prioritized (internet-exposed).
Method: code trace + 11/11 multiplayer tests (live server smoke test
skipped: production server holds the binary). All findings banked as
R-1..R-5 in BUGS.md.

## Findings

- [MAJOR] server main.rs ~136/549-551: unbounded per-client outbound
  queue + select loop parked on the TCP write = slow-client memory
  exhaustion. Keepalive watches inbound only and cannot fire while
  parked. (R-1)
- [MAJOR] server main.rs ~610-629 + persist_snapshots: no edit rate
  limit; every edit fsyncs the journal and rewrites BOTH complete EDT1
  snapshots (non-atomic) under the global journal mutex — O(n^2) flood
  cost, serializes all clients. (R-2)
- [MINOR] no explicit WebSocketConfig size cap; serde_json parses up to
  tungstenite's ~64 MiB default of untrusted input pre-validation. (R-3)
- [MINOR] journal.rs open(): torn final record ((len-4)%42 != 0) hard-
  fails startup instead of truncating the tail. Loud, non-corrupting. (R-4)
- [NOTE] token != compare (timing); token in URL query (edge logging);
  import_legacy_if_empty dead code vs the server's inline import. (R-5)
- [NOTE] shader.wgsl ~1585: per-cell hash jitter before the deck lookup
  means the sky sample is not the byte-exact twin of sample_face that a
  nearby comment claims. Cosmetic.

## Found clean

D-17 clock authority enforced server-side; identity/version gate
complete and symmetric both directions; journal sequence assignment
race-free under the mutex, client re-validates strict +1; handshake
timeout, ghost prevention, disconnect cleanup, dead-sender pruning — no
task/sender leaks; malformed messages disconnect rather than panic; no
panic surface in decode_record; player ids never reused in-run. W2.5
raster: bake deterministic (thread-order independent, quantized key
round-trips), UNORM8 rounding correct, cube-edge texels agree across
faces, pinned path uniform. W2: storm-edge SH-1 fit math correct
(factor-2 recovery), dawn_window pole-safe, no camera-anchored motion.
Moon: crater lattice deterministic, prefetch a faithful twin of the
direct path, depth ratio capped per the B-2 fix, bin edges overlap
deliberately.

## Verdict

MP1's auth/versioning/journal/clock core is soundly designed and well
tested; the exposure is resource safety (R-1, R-2) — fix before wider
than family exposure. W2.5/W2/moon clean on all focus areas.

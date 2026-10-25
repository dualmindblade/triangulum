# MP1 findings

Date: 2026-07-13  
Branch/base: `sol/mp1` / `b67b973`  
World: `output/seed42_r8` (seed 42)

## Status

**MP1 acceptance is complete.** The real Tokio/Tungstenite dependency graph
compiled in release mode, both required binaries built, all tests passed, the
scripted server/two-client integration passed every assertion, the live
two-client offscreen screenshot was captured and inspected, and the complete
offline gauntlet remained byte-clean. No baseline was blessed and no commit was
made.

The shared build identity exercised by the server and clients was
`b67b973-s2ff1423a71775a4e` (commit plus dirty-source/toolchain digest).

## Release builds against the real dependencies

Environment:

```text
rustc 1.94.1 (e408947bf 2026-03-25)
cargo 1.94.1 (29ea6fb6a 2026-03-24)
tokio 1.52.3
tokio-tungstenite 0.28.0
```

Commands completed successfully using the fetched local registry cache:

```powershell
cd viewer
cargo build --release --offline -p triangulum-server --bins
cargo build --release --offline --features multiplayer --bin triangulum-viewer
```

Release artifacts:

```text
triangulum-server.exe     1,742,336 bytes  sha256 86D3821C2A43A316B2882B443DFCF9DFF0FEB93A96096B6180905FF2F66D51A5
triangulum-client-sim.exe 1,580,544 bytes  sha256 CDF65E50D65B77F999BBE4AB173FC48A5FE85739875254795516648FF06B78BC
triangulum-viewer.exe    16,043,520 bytes  sha256 05E4F0D1EF6673295AF1664AFF5BD31A401360BE4B877EFA18953AF6BBBFA966
```

The first real transport compile found and fixed only localized Rust ownership
and inference issues: explicit simulator clock/slew types, retaining a cloned
client sender in the server registry, and moving console input outside a
pattern guard. No protocol behavior changed.

## Full test suite

The literal mission gate passed:

```powershell
cargo test --release --offline --lib
```

```text
running 94 tests
test result: ok. 93 passed; 0 failed; 1 ignored
```

The complete all-feature workspace suite then exercised the multiplayer crate
through the actual dependencies and compiled the network-enabled viewer and
both headless binaries:

```powershell
cargo test --release --offline --workspace --all-features
```

```text
triangulum-multiplayer: 10 passed; 0 failed
triangulum-viewer lib:  93 passed; 0 failed; 1 ignored
golden integration:      1 passed; 0 failed
server/client/viewer bins and doctests: 0 failed
```

The ten multiplayer-core tests cover serde round-trips for the protocol enum,
specific identity mismatch diagnostics, complete immutable-bake hashing,
invite parsing, timestamped seek and scale continuity, two-second clock slew,
EDJ2 append/reopen/LWW persistence, and EDT1 compatibility.

## Two-client localhost integration

Command:

```powershell
cd viewer
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\mp1-integration.ps1 -SkipBuild
```

Script summary:

```text
MP1 INTEGRATION PASS
clock: Alice/Bob common canonical sample delta = 0 ms; client slew errors = 0 / 0 ms
edit: Alice value 7 observed by Bob and server EDJ2 persisted
presence: Alice Moon (12.5, -33.25, 0.004 km) observed exactly by Bob
identity: mismatched build refused loudly by both ends
journal: ...\viewer\interchange\mp1-integration\world.edj2 (3658 bytes)
```

Key transcript excerpts:

```text
CLOCK_SAMPLE name=Alice server_mono_ms=4000 display_s=1238.500000 canonical_s=1238.500000 error_ms=0.000
CLOCK_SAMPLE name=Bob server_mono_ms=4000 display_s=1238.500000 canonical_s=1238.500000 error_ms=0.000
EDIT_APPLIED observer=Bob seq=87 body=Neisor face=2 ci=12345 cj=54321 value=7 journal_records=87
EDIT PERSISTED seq=87 player=2 body=Neisor face=2 ci=12345 cj=54321 value=7
PRESENCE observer=Bob from_name=Alice body=Moon lat_deg=12.500000 lon_deg=-33.250000 alt_km=0.004000 yaw_deg=91.000 pitch_deg=-7.000 roll_deg=2.000 mode=Walk
REFUSED ... build hash mismatch: server=b67b973-s2ff1423a71775a4e client=definitely-not-the-server-build
REFUSED name=MismatchProbe code=identity_mismatch message=build hash mismatch: server=b67b973-s2ff1423a71775a4e client=definitely-not-the-server-build
```

The server intentionally imported the existing world's 86 EDT1 columns, then
durably appended Alice's edit as sequence 87. An independent byte parse of the
persisted file found 87 complete EDJ2 records and the exact required record
(`Neisor`, face 2, column 12345/54321, value 7) at sequence 87.

Artifacts:

- `viewer/interchange/mp1-integration/transcript.txt` - 32,580 bytes,
  SHA-256 `41EAF00C5C3A01B73BDCCFBBD1C1B9A46A2191BF45A3E226142602B7F954FE32`
- `viewer/interchange/mp1-integration/world.edj2` - 3,658 bytes,
  SHA-256 `FBC143E5A47C5F5CEE1B56869A3E26AA0003FCB1C7A66874B2B82B7A9AA573B1`

The evidence script also needed two Windows PowerShell 5 portability fixes
found by this run: `$()` for inline `if` transcript sections, and retaining the
native process handle so redirected child exit codes remain observable.

## Live two-client avatar screenshot

Command:

```powershell
cd viewer
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\mp1-avatar-capture.ps1 -SkipBuild
```

This launched the real server, a headless client named Bob, and the
network-enabled offscreen viewer client named Alice. The viewer reported:

```text
captured multiplayer ...\mp1-two-clients.png (1134 tiles, remote avatar + name label visible)
```

`viewer/interchange/mp1-two-clients.png` is 1280x720, 209,923 bytes, SHA-256
`975169EC0F2253ED0AB12FEE31AAF8690A5F3186C8FF84466274F69EDD95FFCF`.
The final PNG was visually inspected: Bob's tinted two-box placeholder and the
floating `BOB` label are plainly visible in Alice's world view.

## Offline byte-clean gauntlet

The harness itself was rebuilt with the network feature selected, then all
offline instruments were freshly rendered/run:

```powershell
cargo build --release --offline --features multiplayer --example play
```

`play.exe` SHA-256:

```text
4A6D4E45B0A8EE5D2B59B61DE05EDAA86C562CB392A16EC551D7421F162773ED
```

All eight asserting suites passed:

```text
PASS physics-regressions
PASS water-regressions
PASS lake-regressions
PASS flooded-caves
PASS invariant-survey
PASS auto-survey
PASS camera-controls
PASS solar-system
```

The freshly rendered instruments reported:

```text
sync_diff sea_calib: divergent 0.0%, mean d 0.0, p95 0, lum bias +0.0
REEL: all 28 poses clean (lint ok, no unexplained change)
SKY REEL: all 5 poses clean
```

Neither reel command used `--accept`. Accepted-baseline manifest fingerprints
were identical before and after the run:

```text
world reel: 28 files, EC9F1F8436B6D2DF9915EFD936027C30CD71E04AB14F6D05B92BDBFCE1C7A520
sky reel:    5 files, 46C7C670DB2B3BBB84ECE20FFF91300592B61E1C08C14799C87C2AC24246FDC7
```

The rendered sync report is at
`viewer/interchange/runs/sync-diff/report.md`; reel evidence remains in the
normal `viewer/interchange/reel`, `viewer/interchange/skyreel`, and run
directories.

## Scope check

- The server dependency path contains no wgpu or winit dependency.
- The default offline play path never imports the network module.
- Weather, moon generation, and terrain generation source were not changed.
- No commit was created.

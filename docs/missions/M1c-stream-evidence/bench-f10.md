# bench: M1c-stream coverage tree, lod factor 10, 1/70 (PROVISIONAL: other missions shared the machine)

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 10, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 1.00 | 1.05 | 0.73 | 0.84 | 1.41M | 0.01M | 2466 | L0:4368 L1:3215 L2:2480 L3:2246 L4:2251 L5:1199 L6:1024 L7:182 | 17155 | 27.2 | 5.7 | out/bench-f10/pose_stand_hills.png |
| stand_mountains | 1.08 | 1.40 | 1.10 | 1.43 | 1.95M | 0.02M | 3103 | L0:4594 L1:2888 L2:2934 L3:2580 L4:2668 L5:1419 L6:982 L7:121 | 18682 | 37.6 | 6.2 | out/bench-f10/pose_stand_mountains.png |
| alt_100m | 1.22 | 1.39 | 1.46 | 2.26 | 1.43M | 0.01M | 2486 | L0:4270 L1:3195 L2:2485 L3:2235 L4:2678 L5:1420 L6:990 L7:130 | 17903 | 28.8 | 6.0 | out/bench-f10/pose_alt_100m.png |
| alt_1km | 0.77 | 0.98 | 1.12 | 1.59 | 1.17M | 0.01M | 2148 | L1:4118 L2:2523 L3:2235 L4:2654 L5:1419 L6:1107 L7:182 | 14603 | 24.7 | 0.4 | out/bench-f10/pose_alt_1km.png |
| alt_10km | 0.40 | 0.40 | 0.74 | 0.95 | 0.67M | 0.02M | 1634 | L4:1848 L5:1467 L6:1224 L7:616 L8:26 | 5319 | 11.7 | 0.9 | out/bench-f10/pose_alt_10km.png |
| alt_100km | 0.33 | 0.33 | 0.53 | 0.63 | 0.25M | 0.01M | 672 | L7:164 L8:495 L9:81 | 740 | 2.7 | 0.3 | out/bench-f10/pose_alt_100km.png |
| alt_500km | 0.42 | 0.43 | 0.48 | 0.57 | 0.03M | 0.01M | 398 | L9:273 | 273 | 0.4 | 0.1 | out/bench-f10/pose_alt_500km.png |

AO cost over 17325 loaded chunks: 2454273 opaque quads with AO vs 1632310 without (+50.4%), i.e. 4.91M vs 3.26M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 32221 | 540.5 | 238.7 | 11.35 | 1.82 / 3.28 | 1.11 / 1.93 | 0.59 / 1.06 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 28827 | 3122 | 3799 | 245 | 340 |
| 1 | 28913 | 3548 | 4448 | 243 | 346 |
| 2 | 22878 | 4013 | 5201 | 250 | 350 |
| 3 | 17399 | 3985 | 5430 | 254 | 394 |
| 4 | 12699 | 4222 | 5420 | 257 | 431 |
| 5 | 5021 | 3702 | 4588 | 308 | 411 |
| 6 | 3783 | 2881 | 4257 | 294 | 429 |
| 7 | 2260 | 2491 | 3584 | 295 | 433 |
| 8 | 1077 | 2482 | 3268 | 307 | 491 |
| 9 | 407 | 2730 | 3766 | 207 | 342 |

# bench: M2a

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.52 | 0.57 | 0.56 | 0.79 | 0.48M | 0.00M | 898 | L0:1359 L1:974 L2:693 L3:651 L4:670 L5:328 L6:301 L7:298 L8:132 | 5459 | 8.9 | 2.8 | out/bench-m2a/pose_stand_hills.png |
| stand_mountains | 0.34 | 0.38 | 0.62 | 0.96 | 0.71M | 0.01M | 1107 | L0:1249 L1:918 L2:939 L3:739 L4:677 L5:420 L6:370 L7:329 L8:114 | 5857 | 12.6 | 2.7 | out/bench-m2a/pose_stand_mountains.png |
| alt_100m | 0.42 | 0.61 | 0.66 | 0.95 | 0.49M | 0.00M | 928 | L0:1232 L1:970 L2:701 L3:651 L4:678 L5:404 L6:354 L7:335 L8:116 | 5535 | 9.3 | 2.6 | out/bench-m2a/pose_alt_100m.png |
| alt_1km | 0.22 | 0.23 | 0.71 | 1.08 | 0.37M | 0.00M | 756 | L2:831 L3:671 L4:678 L5:400 L6:355 L7:344 L8:133 L9:2 | 3489 | 7.1 | 0.2 | out/bench-m2a/pose_alt_1km.png |
| alt_10km | 0.17 | 0.17 | 0.62 | 0.86 | 0.22M | 0.01M | 568 | L5:338 L6:369 L7:339 L8:234 L9:22 | 1337 | 3.8 | 0.3 | out/bench-m2a/pose_alt_10km.png |
| alt_100km | 0.17 | 0.17 | 0.68 | 0.99 | 0.05M | 0.01M | 254 | L8:80 L9:195 | 275 | 0.6 | 0.2 | out/bench-m2a/pose_alt_100km.png |
| alt_500km | 0.17 | 0.17 | 0.62 | 0.91 | 0.03M | 0.01M | 398 | L9:273 | 273 | 0.4 | 0.0 | out/bench-m2a/pose_alt_500km.png |

AO cost over 5629 loaded chunks: 806287 opaque quads with AO vs 564935 without (+42.7%), i.e. 1.61M vs 1.13M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 24370 | 409.9 | 48.2 | 67.34 | 1.69 / 13.78 | 1.02 / 6.61 | 0.42 / 1.34 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 10165 | 3262 | 5097 | 250 | 343 |
| 1 | 9604 | 3763 | 6250 | 249 | 355 |
| 2 | 7033 | 4401 | 8072 | 267 | 353 |
| 3 | 5785 | 4804 | 9103 | 263 | 332 |
| 4 | 4838 | 4716 | 9417 | 256 | 415 |
| 5 | 1858 | 3758 | 7233 | 304 | 387 |
| 6 | 1293 | 2885 | 5360 | 279 | 386 |
| 7 | 1049 | 2499 | 5622 | 296 | 379 |
| 8 | 766 | 2470 | 4671 | 308 | 366 |
| 9 | 350 | 2761 | 5577 | 225 | 298 |

# bench: M1b on merged main f96a3ba

PROVISIONAL: other agents were using the CPU during this run.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.12 | 0.12 | 0.83 | 1.04 | 0.19M | 0.00M | 875 | L0:137 L1:93 L2:97 L3:95 L4:94 L5:96 L6:89 L7:92 L8:57 L9:9 | 3575 | 4.9 | 1.4 | out/bench-m1b/pose_stand_hills.png |
| stand_mountains | 0.13 | 0.13 | 1.42 | 2.86 | 0.20M | 0.00M | 995 | L0:132 L1:98 L2:91 L3:102 L4:104 L5:93 L6:88 L7:88 L8:56 L9:10 | 3966 | 5.1 | 0.9 | out/bench-m1b/pose_stand_mountains.png |
| alt_100m | 0.50 | 0.54 | 1.18 | 2.24 | 0.19M | 0.00M | 866 | L0:122 L1:98 L2:97 L3:94 L4:92 L5:96 L6:89 L7:92 L8:58 L9:12 | 3470 | 4.9 | 0.8 | out/bench-m1b/pose_alt_100m.png |
| alt_1km | 0.46 | 0.48 | 0.73 | 1.45 | 0.15M | 0.00M | 628 | L2:46 L3:114 L4:92 L5:100 L6:89 L7:92 L8:65 L9:14 | 2229 | 3.6 | 0.1 | out/bench-m1b/pose_alt_1km.png |
| alt_10km | 0.40 | 0.42 | 0.38 | 0.44 | 0.10M | 0.00M | 376 | L5:28 L6:117 L7:91 L8:97 L9:27 | 1103 | 2.1 | 0.1 | out/bench-m1b/pose_alt_10km.png |
| alt_100km | 0.24 | 0.25 | 0.35 | 0.58 | 0.03M | 0.00M | 150 | L8:14 L9:136 | 263 | 0.4 | 0.0 | out/bench-m1b/pose_alt_100km.png |
| alt_500km | 0.21 | 0.23 | 0.21 | 0.27 | 0.05M | 0.00M | 294 | L9:294 | 294 | 0.4 | 0.0 | out/bench-m1b/pose_alt_500km.png |

AO cost over 3575 loaded chunks: 309269 opaque quads with AO vs 265698 without (+16.4%), i.e. 0.62M vs 0.53M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 44630 | 747.1 | 188.3 | 24.06 | 1.29 / 3.80 | 0.79 / 2.35 | 0.34 / 1.12 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 7329 | 2338 | 3834 | 233 | 292 |
| 1 | 3738 | 2818 | 3771 | 210 | 309 |
| 2 | 2442 | 2997 | 3917 | 239 | 316 |
| 3 | 2194 | 2761 | 3730 | 230 | 301 |
| 4 | 2355 | 2609 | 4109 | 224 | 401 |
| 5 | 1461 | 3104 | 4227 | 276 | 373 |
| 6 | 1204 | 2728 | 3807 | 264 | 407 |
| 7 | 855 | 2594 | 3526 | 281 | 438 |
| 8 | 639 | 2423 | 3095 | 276 | 384 |
| 9 | 349 | 2364 | 3091 | 255 | 318 |

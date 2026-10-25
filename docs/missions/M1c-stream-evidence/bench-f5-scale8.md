# bench: M1c-stream coverage tree, lod factor 5, vertical scale 1/8 H 4096 sea 1536 (PROVISIONAL: other missions shared the machine)

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 4096 blocks, sea level 1536 blocks, N = 131072, R = 90.6 km, vertical scale 0.12500 (1/8.0), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.93 | 1.07 | 0.47 | 0.60 | 1.33M | 0.00M | 1133 | L0:3594 L1:1891 L2:1373 L3:1016 L4:894 L5:640 L6:555 L7:307 L8:173 L9:12 | 10498 | 19.3 | 3.3 | out/bench-f5-scale8/pose_stand_hills.png |
| stand_mountains | 0.78 | 1.10 | 0.57 | 0.67 | 1.57M | 0.01M | 1334 | L0:3534 L1:1923 L2:1548 L3:1226 L4:1114 L5:948 L6:665 L7:346 L8:162 L9:6 | 11548 | 24.2 | 3.9 | out/bench-f5-scale8/pose_stand_mountains.png |
| alt_100m | 0.79 | 0.95 | 0.54 | 0.63 | 1.32M | 0.00M | 1141 | L0:3294 L1:1907 L2:1372 L3:1018 L4:902 L5:814 L6:654 L7:342 L8:160 L9:5 | 10548 | 19.9 | 3.2 | out/bench-f5-scale8/pose_alt_100m.png |
| alt_1km | 0.40 | 0.40 | 0.47 | 0.55 | 0.95M | 0.01M | 874 | L1:120 L2:1629 L3:1043 L4:903 L5:802 L6:656 L7:342 L8:173 L9:6 | 5744 | 14.5 | 0.3 | out/bench-f5-scale8/pose_alt_1km.png |
| alt_10km | 0.29 | 0.29 | 0.37 | 0.42 | 0.50M | 0.01M | 661 | L5:593 L6:682 L7:336 L8:275 L9:26 | 1936 | 6.3 | 0.3 | out/bench-f5-scale8/pose_alt_10km.png |
| alt_100km | 0.23 | 0.23 | 0.31 | 0.36 | 0.14M | 0.01M | 289 | L8:80 L9:203 | 283 | 1.0 | 0.1 | out/bench-f5-scale8/pose_alt_100km.png |
| alt_500km | 0.24 | 0.24 | 0.37 | 0.61 | 0.16M | 0.01M | 542 | L9:278 | 278 | 0.9 | 0.0 | out/bench-f5-scale8/pose_alt_500km.png |

AO cost over 10661 loaded chunks: 2085233 opaque quads with AO vs 1278604 without (+63.1%), i.e. 4.17M vs 2.56M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 59622 | 1002.9 | 395.5 | 6.42 | 1.04 / 1.68 | 0.53 / 1.04 | 0.35 / 0.53 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 27615 | 2619 | 3853 | 171 | 353 |
| 1 | 19804 | 3032 | 4414 | 182 | 378 |
| 2 | 12977 | 3661 | 5067 | 239 | 389 |
| 3 | 9147 | 3811 | 5243 | 244 | 400 |
| 4 | 7014 | 4084 | 5336 | 249 | 411 |
| 5 | 3749 | 3824 | 5286 | 274 | 409 |
| 6 | 2361 | 3569 | 5061 | 310 | 483 |
| 7 | 1062 | 3007 | 4200 | 347 | 517 |
| 8 | 840 | 2687 | 3538 | 325 | 489 |
| 9 | 366 | 2812 | 4164 | 305 | 695 |

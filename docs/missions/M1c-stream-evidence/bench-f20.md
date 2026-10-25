# bench: M1c-stream coverage tree, lod factor 20, 1/70 (PROVISIONAL: other missions shared the machine)

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 20, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 2.51 | 2.79 | 3.58 | 3.78 | 4.59M | 0.01M | 6738 | L0:15814 L1:11099 L2:9275 L3:9073 L4:8227 L5:1933 L6:56 | 56457 | 85.8 | 21.1 | out/bench-f20/pose_stand_hills.png |
| stand_mountains | 3.42 | 3.90 | 5.22 | 5.88 | 5.62M | 0.05M | 8602 | L0:16687 L1:10346 L2:10475 L3:11545 L4:9584 L5:1686 L6:40 | 62461 | 116.6 | 33.7 | out/bench-f20/pose_stand_mountains.png |
| alt_100m | 3.17 | 3.97 | 5.30 | 5.56 | 4.68M | 0.01M | 6910 | L0:15670 L1:11073 L2:9270 L3:11091 L4:9682 L5:1744 L6:46 | 60604 | 93.0 | 31.2 | out/bench-f20/pose_alt_100m.png |
| alt_1km | 2.91 | 3.06 | 5.03 | 5.38 | 4.44M | 0.01M | 6816 | L0:15129 L1:11249 L2:9270 L3:11035 L4:9849 L5:2626 L6:187 | 60699 | 93.7 | 0.5 | out/bench-f20/pose_alt_1km.png |
| alt_10km | 2.26 | 3.10 | 4.19 | 4.38 | 2.53M | 0.05M | 5716 | L3:6822 L4:10150 L5:4777 L6:1827 | 23956 | 45.3 | 6.3 | out/bench-f20/pose_alt_10km.png |
| alt_100km | 1.30 | 1.86 | 3.14 | 3.34 | 0.65M | 0.03M | 1985 | L6:432 L7:1580 L8:307 | 2361 | 6.7 | 2.6 | out/bench-f20/pose_alt_100km.png |
| alt_500km | 0.55 | 0.56 | 2.80 | 2.91 | 0.03M | 0.01M | 398 | L9:273 | 273 | 0.4 | 0.3 | out/bench-f20/pose_alt_500km.png |

AO cost over 56627 loaded chunks: 7802777 opaque quads with AO vs 4957836 without (+57.4%), i.e. 15.61M vs 9.92M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 6348 | 106.0 | 68.1 | 18.93 | 9.40 / 13.41 | 5.79 / 8.23 | 2.97 / 5.19 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 88282 | 3218 | 3896 | 238 | 342 |
| 1 | 88559 | 3663 | 4480 | 239 | 345 |
| 2 | 69351 | 4221 | 5188 | 247 | 362 |
| 3 | 50604 | 4095 | 5272 | 248 | 406 |
| 4 | 34871 | 4277 | 5191 | 254 | 432 |
| 5 | 14389 | 3719 | 5098 | 302 | 393 |
| 6 | 7384 | 2865 | 4099 | 291 | 360 |
| 7 | 3311 | 2489 | 3427 | 296 | 359 |
| 8 | 1452 | 2509 | 3266 | 302 | 352 |
| 9 | 446 | 2832 | 3776 | 188 | 281 |

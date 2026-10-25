# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.54 | 0.57 | 0.35 | 0.41 | 0.48M | 0.00M | 898 | L0:1359 L1:974 L2:693 L3:651 L4:670 L5:328 L6:301 L7:298 L8:132 | 5459 | 8.9 | 2.3 | out/bench-nohydro2/pose_stand_hills.png |
| stand_mountains | 0.67 | 0.79 | 0.38 | 0.42 | 0.71M | 0.01M | 1107 | L0:1249 L1:918 L2:939 L3:739 L4:677 L5:420 L6:370 L7:329 L8:114 | 5857 | 12.6 | 2.4 | out/bench-nohydro2/pose_stand_mountains.png |
| alt_100m | 0.59 | 0.61 | 0.36 | 0.41 | 0.49M | 0.00M | 928 | L0:1232 L1:970 L2:701 L3:651 L4:678 L5:404 L6:354 L7:335 L8:116 | 5535 | 9.3 | 2.1 | out/bench-nohydro2/pose_alt_100m.png |
| alt_1km | 0.56 | 0.59 | 0.32 | 0.36 | 0.37M | 0.00M | 754 | L2:831 L3:671 L4:678 L5:400 L6:355 L7:344 L8:133 L9:2 | 3489 | 7.1 | 0.2 | out/bench-nohydro2/pose_alt_1km.png |
| alt_10km | 0.49 | 0.54 | 0.28 | 0.28 | 0.22M | 0.01M | 560 | L5:338 L6:369 L7:339 L8:234 L9:22 | 1337 | 3.8 | 0.2 | out/bench-nohydro2/pose_alt_10km.png |
| alt_100km | 0.38 | 0.40 | 0.27 | 0.65 | 0.05M | 0.00M | 182 | L8:80 L9:195 | 275 | 0.5 | 0.1 | out/bench-nohydro2/pose_alt_100km.png |
| alt_500km | 0.35 | 0.36 | 0.20 | 0.22 | 0.04M | 0.00M | 273 | L9:273 | 273 | 0.4 | 0.0 | out/bench-nohydro2/pose_alt_500km.png |

AO cost over 5629 loaded chunks: 809026 opaque quads with AO vs 568051 without (+42.4%), i.e. 1.62M vs 1.14M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 86530 | 1454.4 | 625.7 | 5.25 | 0.70 / 1.14 | 0.34 / 0.62 | 0.23 / 0.28 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 10169 | 3973 | 4891 | 202 | 332 |
| 1 | 9613 | 3943 | 5479 | 206 | 336 |
| 2 | 7038 | 5007 | 6241 | 250 | 346 |
| 3 | 5785 | 4839 | 6295 | 253 | 326 |
| 4 | 4838 | 4890 | 6203 | 254 | 407 |
| 5 | 1858 | 4629 | 5440 | 299 | 397 |
| 6 | 1293 | 3828 | 5029 | 277 | 387 |
| 7 | 1049 | 2991 | 4081 | 294 | 420 |
| 8 | 766 | 2742 | 3626 | 303 | 435 |
| 9 | 350 | 3012 | 4297 | 260 | 452 |

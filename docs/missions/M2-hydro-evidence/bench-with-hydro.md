# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.56 | 0.58 | 0.36 | 0.37 | 0.48M | 0.00M | 905 | L0:1318 L1:974 L2:688 L3:651 L4:670 L5:328 L6:301 L7:298 L8:132 | 5413 | 8.9 | 2.3 | out/bench-hydro/pose_stand_hills.png |
| stand_mountains | 0.68 | 0.70 | 0.37 | 0.42 | 0.71M | 0.01M | 1112 | L0:1249 L1:918 L2:939 L3:739 L4:677 L5:420 L6:370 L7:329 L8:114 | 5857 | 12.6 | 2.4 | out/bench-hydro/pose_stand_mountains.png |
| alt_100m | 0.60 | 0.64 | 0.37 | 0.41 | 0.49M | 0.00M | 935 | L0:1206 L1:969 L2:696 L3:651 L4:678 L5:404 L6:354 L7:335 L8:116 | 5503 | 9.3 | 2.2 | out/bench-hydro/pose_alt_100m.png |
| alt_1km | 0.55 | 0.59 | 0.34 | 0.37 | 0.37M | 0.00M | 761 | L2:825 L3:671 L4:678 L5:400 L6:355 L7:344 L8:133 L9:2 | 3483 | 7.2 | 0.2 | out/bench-hydro/pose_alt_1km.png |
| alt_10km | 0.49 | 0.60 | 0.29 | 0.30 | 0.22M | 0.01M | 564 | L5:338 L6:369 L7:339 L8:234 L9:22 | 1337 | 3.8 | 0.2 | out/bench-hydro/pose_alt_10km.png |
| alt_100km | 0.38 | 0.40 | 0.23 | 0.23 | 0.05M | 0.00M | 182 | L8:80 L9:195 | 275 | 0.6 | 0.2 | out/bench-hydro/pose_alt_100km.png |
| alt_500km | 0.38 | 0.39 | 0.20 | 0.21 | 0.04M | 0.00M | 273 | L9:273 | 273 | 0.4 | 0.1 | out/bench-hydro/pose_alt_500km.png |

AO cost over 5583 loaded chunks: 815768 opaque quads with AO vs 573204 without (+42.3%), i.e. 1.63M vs 1.15M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 86332 | 1452.2 | 603.1 | 5.13 | 0.71 / 1.15 | 0.34 / 0.63 | 0.23 / 0.27 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 9722 | 3987 | 4796 | 211 | 328 |
| 1 | 9511 | 3957 | 5431 | 206 | 336 |
| 2 | 6909 | 4978 | 6014 | 250 | 345 |
| 3 | 5785 | 4832 | 6733 | 254 | 326 |
| 4 | 4838 | 5476 | 7481 | 259 | 416 |
| 5 | 1858 | 4824 | 6888 | 300 | 389 |
| 6 | 1293 | 4197 | 6016 | 286 | 362 |
| 7 | 1049 | 3381 | 6812 | 300 | 353 |
| 8 | 766 | 3264 | 11034 | 303 | 360 |
| 9 | 350 | 5049 | 24278 | 262 | 308 |

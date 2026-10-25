# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.41 | 0.46 | 0.42 | 0.61 | 0.51M | 0.08M | 905 | L0:1318 L1:974 L2:688 L3:651 L4:670 L5:328 L6:301 L7:298 L8:132 | 5413 | 12.2 | 2.4 | C:/Users/Austin/AppData/Local/Temp/claude/C--code-triangulum/d342b492-b4e1-4ca0-aed0-a98693098744/scratchpad/bench-main/pose_stand_hills.png |
| stand_mountains | 0.49 | 0.50 | 0.42 | 0.57 | 0.72M | 0.12M | 1112 | L0:1249 L1:918 L2:939 L3:739 L4:677 L5:420 L6:370 L7:329 L8:114 | 5857 | 16.3 | 2.6 | C:/Users/Austin/AppData/Local/Temp/claude/C--code-triangulum/d342b492-b4e1-4ca0-aed0-a98693098744/scratchpad/bench-main/pose_stand_mountains.png |
| alt_100m | 0.50 | 0.51 | 0.41 | 0.56 | 0.52M | 0.08M | 935 | L0:1206 L1:969 L2:696 L3:651 L4:678 L5:404 L6:354 L7:335 L8:116 | 5503 | 13.0 | 2.3 | C:/Users/Austin/AppData/Local/Temp/claude/C--code-triangulum/d342b492-b4e1-4ca0-aed0-a98693098744/scratchpad/bench-main/pose_alt_100m.png |
| alt_1km | 0.38 | 0.38 | 0.41 | 0.66 | 0.41M | 0.10M | 761 | L2:825 L3:671 L4:678 L5:400 L6:355 L7:344 L8:133 L9:2 | 3483 | 11.0 | 0.2 | C:/Users/Austin/AppData/Local/Temp/claude/C--code-triangulum/d342b492-b4e1-4ca0-aed0-a98693098744/scratchpad/bench-main/pose_alt_1km.png |
| alt_10km | 0.44 | 0.44 | 0.36 | 0.55 | 0.31M | 0.15M | 564 | L5:338 L6:369 L7:339 L8:234 L9:22 | 1337 | 8.8 | 0.3 | C:/Users/Austin/AppData/Local/Temp/claude/C--code-triangulum/d342b492-b4e1-4ca0-aed0-a98693098744/scratchpad/bench-main/pose_alt_10km.png |
| alt_100km | 0.39 | 0.40 | 0.28 | 0.48 | 0.31M | 0.03M | 182 | L8:80 L9:195 | 275 | 3.0 | 0.2 | C:/Users/Austin/AppData/Local/Temp/claude/C--code-triangulum/d342b492-b4e1-4ca0-aed0-a98693098744/scratchpad/bench-main/pose_alt_100km.png |
| alt_500km | 0.58 | 0.58 | 0.24 | 0.37 | 0.57M | 0.00M | 273 | L9:273 | 273 | 2.5 | 0.1 | C:/Users/Austin/AppData/Local/Temp/claude/C--code-triangulum/d342b492-b4e1-4ca0-aed0-a98693098744/scratchpad/bench-main/pose_alt_500km.png |

AO cost over 5583 loaded chunks: 1088728 opaque quads with AO vs 846540 without (+28.6%), i.e. 2.18M vs 1.69M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 59045 | 992.1 | 288.9 | 6.97 | 0.86 / 3.12 | 0.41 / 0.85 | 0.37 / 0.41 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 9722 | 3981 | 5247 | 213 | 328 |
| 1 | 9511 | 4171 | 5934 | 207 | 335 |
| 2 | 6909 | 4966 | 7057 | 250 | 346 |
| 3 | 5785 | 4912 | 7748 | 251 | 324 |
| 4 | 4838 | 5503 | 8103 | 258 | 433 |
| 5 | 1858 | 4891 | 7949 | 300 | 392 |
| 6 | 1293 | 4298 | 7106 | 284 | 383 |
| 7 | 1049 | 3570 | 7937 | 304 | 364 |
| 8 | 766 | 3431 | 12389 | 316 | 399 |
| 9 | 350 | 5462 | 27065 | 275 | 386 |

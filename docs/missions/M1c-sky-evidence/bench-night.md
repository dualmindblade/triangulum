# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 4, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.56 | 0.96 | 1.62 | 1.90 | 0.19M | 0.00M | 875 | L0:137 L1:93 L2:97 L3:95 L4:94 L5:96 L6:89 L7:92 L8:57 L9:9 | 3575 | 4.9 | 3.8 | out/m1c-sky/bench-final-night/pose_stand_hills.png |
| stand_mountains | 1.06 | 1.67 | 1.90 | 2.09 | 0.20M | 0.00M | 995 | L0:132 L1:98 L2:91 L3:102 L4:104 L5:93 L6:88 L7:88 L8:56 L9:10 | 3966 | 5.1 | 2.2 | out/m1c-sky/bench-final-night/pose_stand_mountains.png |
| alt_10km | 0.58 | 1.15 | 0.83 | 0.96 | 0.10M | 0.00M | 376 | L5:28 L6:117 L7:91 L8:97 L9:27 | 1103 | 2.1 | 0.3 | out/m1c-sky/bench-final-night/pose_alt_10km.png |
| alt_500km | 0.38 | 0.41 | 0.42 | 0.52 | 0.05M | 0.00M | 294 | L9:294 | 294 | 0.4 | 0.1 | out/m1c-sky/bench-final-night/pose_alt_500km.png |

AO cost over 3575 loaded chunks: 309269 opaque quads with AO vs 265698 without (+16.4%), i.e. 0.62M vs 0.53M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 29650 | 496.5 | 231.6 | 7.36 | 1.89 / 3.76 | 1.25 / 2.18 | 0.55 / 1.28 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 6545 | 1841 | 2643 | 164 | 229 |
| 1 | 3360 | 2313 | 2801 | 174 | 238 |
| 2 | 1998 | 2193 | 2628 | 156 | 194 |
| 3 | 1809 | 1980 | 2348 | 145 | 170 |
| 4 | 1874 | 1847 | 2601 | 139 | 266 |
| 5 | 1198 | 2130 | 2605 | 176 | 228 |
| 6 | 1102 | 1863 | 2209 | 158 | 209 |
| 7 | 832 | 1803 | 2363 | 173 | 269 |
| 8 | 595 | 1678 | 1993 | 166 | 211 |
| 9 | 335 | 1663 | 2205 | 157 | 256 |

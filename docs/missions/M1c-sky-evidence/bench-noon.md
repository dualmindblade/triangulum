# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 4, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.46 | 0.53 | 0.83 | 1.45 | 0.19M | 0.00M | 875 | L0:137 L1:93 L2:97 L3:95 L4:94 L5:96 L6:89 L7:92 L8:57 L9:9 | 3575 | 4.9 | 4.0 | out/m1c-sky/bench-final-noon/pose_stand_hills.png |
| stand_mountains | 0.75 | 1.22 | 0.91 | 1.01 | 0.20M | 0.00M | 995 | L0:132 L1:98 L2:91 L3:102 L4:104 L5:93 L6:88 L7:88 L8:56 L9:10 | 3966 | 5.1 | 2.1 | out/m1c-sky/bench-final-noon/pose_stand_mountains.png |
| alt_10km | 0.48 | 0.51 | 0.49 | 0.96 | 0.10M | 0.00M | 376 | L5:28 L6:117 L7:91 L8:97 L9:27 | 1103 | 2.1 | 0.3 | out/m1c-sky/bench-final-noon/pose_alt_10km.png |
| alt_500km | 0.43 | 0.49 | 0.21 | 0.24 | 0.05M | 0.00M | 294 | L9:294 | 294 | 0.4 | 0.1 | out/m1c-sky/bench-final-noon/pose_alt_500km.png |

AO cost over 3575 loaded chunks: 309269 opaque quads with AO vs 265698 without (+16.4%), i.e. 0.62M vs 0.53M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)


## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 2978 | 1899 | 2711 | 165 | 255 |
| 1 | 1332 | 2249 | 2713 | 165 | 243 |
| 2 | 990 | 2246 | 2951 | 158 | 230 |
| 3 | 1020 | 1975 | 2410 | 141 | 171 |
| 4 | 1303 | 1867 | 2661 | 137 | 275 |
| 5 | 979 | 2140 | 2667 | 177 | 247 |
| 6 | 1015 | 1887 | 2568 | 163 | 263 |
| 7 | 795 | 1817 | 2239 | 171 | 234 |
| 8 | 523 | 1687 | 2066 | 167 | 246 |
| 9 | 314 | 1648 | 1922 | 157 | 180 |

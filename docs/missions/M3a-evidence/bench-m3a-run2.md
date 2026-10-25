# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.47 | 0.47 | 0.34 | 0.40 | 0.51M | 0.08M | 905 | L0:1318 L1:974 L2:688 L3:651 L4:670 L5:328 L6:301 L7:298 L8:132 | 5413 | 12.2 | 2.2 | out/m3a/bench-after3/pose_stand_hills.png |

AO cost over 5413 loaded chunks: 912048 opaque quads with AO vs 669969 without (+36.1%), i.e. 1.82M vs 1.34M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 74698 | 1254.8 | 626.5 | 5.10 | 0.79 / 1.24 | 0.31 / 0.59 | 0.37 / 0.41 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 5949 | 3175 | 4620 | 173 | 311 |
| 1 | 4896 | 3334 | 5167 | 174 | 328 |
| 2 | 3088 | 3996 | 5688 | 177 | 298 |
| 3 | 2402 | 4156 | 5935 | 183 | 290 |
| 4 | 1559 | 4501 | 6937 | 246 | 407 |
| 5 | 554 | 4588 | 6556 | 291 | 375 |
| 6 | 451 | 3990 | 5830 | 277 | 351 |
| 7 | 424 | 3314 | 6785 | 306 | 359 |
| 8 | 301 | 3133 | 9835 | 314 | 365 |
| 9 | 120 | 4898 | 25925 | 280 | 337 |

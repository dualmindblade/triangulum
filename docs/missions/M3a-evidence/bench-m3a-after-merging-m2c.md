# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.98 | 1.13 | 0.32 | 0.37 | 0.51M | 0.08M | 905 | L0:1318 L1:974 L2:688 L3:651 L4:670 L5:328 L6:301 L7:298 L8:132 | 5413 | 12.2 | 2.2 | out/m3a/bench-merged/pose_stand_hills.png |

AO cost over 5413 loaded chunks: 912048 opaque quads with AO vs 669969 without (+36.1%), i.e. 1.82M vs 1.34M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 74877 | 1257.6 | 598.2 | 7.53 | 0.78 / 1.25 | 0.31 / 0.58 | 0.37 / 0.41 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 5949 | 3182 | 4623 | 172 | 309 |
| 1 | 4896 | 3383 | 5157 | 178 | 324 |
| 2 | 3088 | 4031 | 5702 | 180 | 297 |
| 3 | 2402 | 4162 | 5981 | 179 | 289 |
| 4 | 1559 | 4550 | 7101 | 245 | 415 |
| 5 | 554 | 4585 | 6422 | 291 | 378 |
| 6 | 451 | 3985 | 5565 | 275 | 347 |
| 7 | 424 | 3322 | 6773 | 307 | 352 |
| 8 | 301 | 3133 | 10047 | 315 | 383 |
| 9 | 120 | 5008 | 26208 | 278 | 337 |

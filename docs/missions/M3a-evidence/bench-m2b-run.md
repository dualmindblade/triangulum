# bench: 

Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 512 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 9 (512 m voxels), lod factor 5, rayon threads 16, AO true, skirt 2

## static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s | png |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.49 | 0.77 | 0.33 | 0.38 | 0.51M | 0.08M | 905 | L0:1318 L1:974 L2:688 L3:651 L4:670 L5:328 L6:301 L7:298 L8:132 | 5413 | 12.2 | 2.2 | out/m3a/bench-before2/pose_stand_hills.png |

AO cost over 5413 loaded chunks: 912048 opaque quads with AO vs 669969 without (+36.1%), i.e. 1.82M vs 1.34M triangles

## fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 73400 | 1233.9 | 543.2 | 8.11 | 0.79 / 1.39 | 0.33 / 0.72 | 0.37 / 0.40 |

## worker costs

| LOD | chunks | generate median µs | p95 | mesh median µs | p95 |
|---|---|---|---|---|---|
| 0 | 5949 | 3405 | 4622 | 177 | 302 |
| 1 | 4896 | 3464 | 5164 | 181 | 319 |
| 2 | 3088 | 4095 | 5691 | 180 | 292 |
| 3 | 2402 | 4266 | 6065 | 186 | 289 |
| 4 | 1559 | 4528 | 7290 | 249 | 411 |
| 5 | 554 | 4661 | 6807 | 288 | 389 |
| 6 | 451 | 4091 | 5920 | 274 | 352 |
| 7 | 424 | 3259 | 6952 | 303 | 354 |
| 8 | 301 | 3095 | 10509 | 309 | 360 |
| 9 | 120 | 4851 | 27075 | 275 | 336 |

# T1 benches — provisional

All five used the built-in seven static poses (120 measured frames per pose), the same 60-second flight, 1920×1080, four Rayon workers, LOD factor 5, AO on and skirt 2. Other missions were active. These are **not** idle-machine performance acceptance numbers. No static pose timed out. Raw pose PNGs remain under `game/out/t1/after/SIZE/bench/`; the numeric reports are preserved below.

## 128 half-heights


Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 1024 blocks, N = 131072, R = 90.6 km, vertical scale 0.01422 (1/70.3), roughness 1, max LOD 10 (1024 m voxels), lod factor 5, rayon threads 4, AO true, skirt 2

### static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 2.21 | 2.86 | 1.23 | 1.38 | 0.59M | 0.09M | 974 | L0:1318 L1:974 L2:688 L3:651 L4:666 L5:643 L6:300 L7:298 L8:141 L9:1 | 5757 | 19.8 | 31.9 |
| stand_mountains | 0.73 | 0.74 | 1.35 | 2.04 | 0.76M | 0.13M | 1139 | L0:1249 L1:918 L2:939 L3:739 L4:665 L5:771 L6:368 L7:341 L8:118 L9:1 | 6225 | 24.1 | 12.3 |
| alt_100m | 0.56 | 0.56 | 0.81 | 1.19 | 0.60M | 0.09M | 1015 | L0:1206 L1:969 L2:696 L3:651 L4:678 L5:784 L6:362 L7:339 L8:127 L9:2 | 5922 | 20.9 | 15.1 |
| alt_1km | 0.49 | 0.50 | 0.87 | 1.24 | 0.45M | 0.10M | 835 | L2:825 L3:671 L4:678 L5:776 L6:355 L7:347 L8:138 L9:4 | 3890 | 17.4 | 0.7 |
| alt_10km | 1.33 | 1.92 | 1.58 | 2.80 | 0.28M | 0.17M | 652 | L5:616 L6:375 L7:339 L8:238 L9:25 | 1645 | 11.9 | 2.7 |
| alt_100km | 1.16 | 2.23 | 0.73 | 1.25 | 0.20M | 0.17M | 290 | L8:68 L9:185 L10:16 | 271 | 4.7 | 2.2 |
| alt_500km | 0.37 | 0.38 | 0.60 | 0.76 | 0.16M | 0.00M | 78 | L10:78 | 78 | 0.9 | 1.2 |

AO cost over 5788 loaded chunks: 1052434 opaque quads with AO vs 820993 without (+28.2%), i.e. 2.10M vs 1.64M triangles

### fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 4844 | 81.3 | 13.8 | 97.26 | 6.39 / 62.88 | 1.65 / 18.60 | 0.84 / 3.14 |

### worker costs

| LOD | chunks | generate median Âµs | p95 | mesh median Âµs | p95 |
|---|---|---|---|---|---|
| 0 | 9446 | 4481 | 6816 | 289 | 435 |
| 1 | 8793 | 5148 | 8325 | 293 | 464 |
| 2 | 6436 | 5818 | 10944 | 307 | 465 |
| 3 | 5485 | 5896 | 11342 | 304 | 471 |
| 4 | 4713 | 5979 | 11788 | 303 | 508 |
| 5 | 3551 | 5794 | 10946 | 299 | 564 |
| 6 | 1281 | 5288 | 12024 | 383 | 657 |
| 7 | 1041 | 4300 | 9886 | 371 | 557 |
| 8 | 775 | 3931 | 19530 | 361 | 558 |
| 9 | 400 | 6571 | 67145 | 349 | 575 |
| 10 | 127 | 45989 | 173483 | 333 | 563 |

## 256 half-heights


Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 1024 blocks, N = 262144, R = 181.1 km, vertical scale 0.02843 (1/35.2), roughness 1, max LOD 10 (1024 m voxels), lod factor 5, rayon threads 4, AO true, skirt 2

### static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 0.55 | 0.55 | 0.81 | 0.97 | 0.74M | 0.13M | 1006 | L0:1421 L1:904 L2:978 L3:679 L4:651 L5:670 L6:301 L7:305 L8:298 L9:122 | 6383 | 23.7 | 17.4 |
| stand_mountains | 0.84 | 0.84 | 1.36 | 1.58 | 1.00M | 0.26M | 1250 | L0:1532 L1:997 L2:945 L3:945 L4:701 L5:677 L6:360 L7:374 L8:317 L9:106 | 7064 | 30.3 | 19.7 |
| alt_100m | 0.61 | 0.61 | 0.75 | 1.05 | 0.75M | 0.15M | 1034 | L0:1298 L1:890 L2:975 L3:682 L4:651 L5:678 L6:360 L7:362 L8:322 L9:105 | 6429 | 25.5 | 12.7 |
| alt_1km | 0.58 | 0.58 | 1.28 | 1.94 | 0.58M | 0.16M | 842 | L1:39 L2:1213 L3:699 L4:651 L5:678 L6:360 L7:362 L8:330 L9:111 | 4538 | 21.6 | 0.8 |
| alt_10km | 0.62 | 0.68 | 1.38 | 2.95 | 0.38M | 0.19M | 642 | L5:640 L6:360 L7:354 L8:348 L9:166 L10:6 | 1923 | 15.5 | 1.8 |
| alt_100km | 1.41 | 1.52 | 0.97 | 1.42 | 0.37M | 0.20M | 437 | L8:88 L9:298 L10:100 | 486 | 7.4 | 1.9 |
| alt_500km | 0.61 | 0.61 | 0.74 | 1.08 | 0.53M | 0.00M | 251 | L10:251 | 251 | 2.8 | 0.5 |

AO cost over 6532 loaded chunks: 1448510 opaque quads with AO vs 1166811 without (+24.1%), i.e. 2.90M vs 2.33M triangles

### fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 4406 | 73.9 | 11.3 | 174.61 | 7.69 / 73.56 | 2.34 / 15.79 | 1.85 / 6.70 |

### worker costs

| LOD | chunks | generate median Âµs | p95 | mesh median Âµs | p95 |
|---|---|---|---|---|---|
| 0 | 11083 | 4484 | 6691 | 282 | 467 |
| 1 | 8792 | 4733 | 7547 | 280 | 434 |
| 2 | 8065 | 5175 | 8068 | 272 | 449 |
| 3 | 5979 | 5403 | 8125 | 290 | 439 |
| 4 | 4759 | 5322 | 8523 | 286 | 396 |
| 5 | 4352 | 4927 | 7644 | 274 | 404 |
| 6 | 1640 | 4533 | 6881 | 315 | 451 |
| 7 | 1251 | 3483 | 5541 | 304 | 446 |
| 8 | 1037 | 3228 | 6812 | 333 | 432 |
| 9 | 697 | 3399 | 12411 | 329 | 434 |
| 10 | 331 | 6676 | 33258 | 332 | 507 |

## 512 half-heights


Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 1024 blocks, N = 524288, R = 362.3 km, vertical scale 0.05686 (1/17.6), roughness 1, max LOD 10 (1024 m voxels), lod factor 5, rayon threads 4, AO true, skirt 2

### static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 1.75 | 1.85 | 1.29 | 1.62 | 0.98M | 0.08M | 1054 | L0:1993 L1:1102 L2:908 L3:979 L4:670 L5:651 L6:299 L7:302 L8:307 L9:297 L10:113 | 7622 | 26.7 | 15.7 |
| stand_mountains | 1.25 | 1.42 | 1.86 | 2.70 | 1.22M | 0.30M | 1342 | L0:2211 L1:1304 L2:963 L3:983 L4:935 L5:667 L6:314 L7:366 L8:370 L9:298 L10:97 | 8581 | 35.4 | 17.0 |
| alt_100m | 2.07 | 2.31 | 1.98 | 2.51 | 0.98M | 0.10M | 1065 | L0:1846 L1:1082 L2:900 L3:981 L4:671 L5:651 L6:303 L7:365 L8:363 L9:304 L10:96 | 7629 | 28.9 | 18.7 |
| alt_1km | 2.01 | 2.23 | 1.91 | 2.53 | 0.79M | 0.10M | 873 | L2:1178 L3:997 L4:671 L5:651 L6:303 L7:361 L8:364 L9:310 L10:98 | 4993 | 24.4 | 1.4 |
| alt_10km | 1.79 | 2.17 | 1.90 | 2.90 | 0.52M | 0.11M | 643 | L5:656 L6:326 L7:342 L8:361 L9:345 L10:133 | 2177 | 17.0 | 1.5 |
| alt_100km | 1.23 | 1.21 | 1.31 | 1.82 | 0.62M | 0.12M | 518 | L8:116 L9:367 L10:345 | 828 | 10.5 | 0.8 |
| alt_500km | 1.66 | 1.91 | 1.29 | 1.84 | 1.58M | 0.00M | 751 | L10:751 | 751 | 8.3 | 0.7 |

AO cost over 8160 loaded chunks: 2250621 opaque quads with AO vs 1863513 without (+20.8%), i.e. 4.50M vs 3.73M triangles

### fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 5632 | 94.4 | 13.6 | 150.23 | 4.76 / 59.73 | 2.09 / 17.62 | 1.58 / 5.49 |

### worker costs

| LOD | chunks | generate median Âµs | p95 | mesh median Âµs | p95 |
|---|---|---|---|---|---|
| 0 | 14422 | 4585 | 6310 | 284 | 479 |
| 1 | 10056 | 4924 | 6985 | 295 | 470 |
| 2 | 7488 | 5181 | 7399 | 301 | 445 |
| 3 | 7708 | 5506 | 7557 | 300 | 481 |
| 4 | 5237 | 5759 | 7551 | 317 | 478 |
| 5 | 4346 | 5409 | 7838 | 308 | 449 |
| 6 | 2015 | 4878 | 8002 | 370 | 523 |
| 7 | 1598 | 3534 | 5929 | 330 | 482 |
| 8 | 1244 | 3181 | 5319 | 323 | 455 |
| 9 | 1033 | 3089 | 6812 | 331 | 493 |
| 10 | 868 | 3390 | 11876 | 332 | 503 |

## 832 half-heights


Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 1024 blocks, N = 851968, R = 588.7 km, vertical scale 0.09240 (1/10.8), roughness 1, max LOD 10 (1024 m voxels), lod factor 5, rayon threads 4, AO true, skirt 2

### static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 1.40 | 1.93 | 1.68 | 2.30 | 1.25M | 0.02M | 1252 | L0:2630 L1:1393 L2:1181 L3:727 L4:796 L5:644 L6:313 L7:311 L8:316 L9:319 L10:247 | 8877 | 30.3 | 19.9 |
| stand_mountains | 3.21 | 4.99 | 4.24 | 13.94 | 1.51M | 0.23M | 1419 | L0:3019 L1:1553 L2:1229 L3:1125 L4:695 L5:663 L6:310 L7:347 L8:379 L9:369 L10:230 | 9966 | 40.5 | 19.7 |
| alt_100m | 2.42 | 4.22 | 2.27 | 5.51 | 1.24M | 0.04M | 1245 | L0:2436 L1:1361 L2:1177 L3:730 L4:796 L5:644 L6:313 L7:339 L8:385 L9:360 L10:232 | 8820 | 32.1 | 52.0 |
| alt_1km | 1.81 | 2.01 | 1.79 | 2.98 | 0.90M | 0.04M | 972 | L2:1508 L3:735 L4:796 L5:644 L6:313 L7:339 L8:385 L9:360 L10:235 | 5361 | 26.2 | 1.5 |
| alt_10km | 1.63 | 1.83 | 1.60 | 2.34 | 0.58M | 0.04M | 603 | L5:632 L6:341 L7:339 L8:369 L9:364 L10:291 | 2361 | 17.4 | 1.5 |
| alt_100km | 2.12 | 2.25 | 1.80 | 3.50 | 0.81M | 0.02M | 518 | L8:116 L9:395 L10:681 | 1192 | 13.3 | 1.3 |
| alt_500km | 3.49 | 3.54 | 2.83 | 6.70 | 2.91M | 0.00M | 1409 | L10:1535 | 1535 | 16.7 | 1.6 |

AO cost over 10059 loaded chunks: 3361700 opaque quads with AO vs 2849273 without (+18.0%), i.e. 6.72M vs 5.70M triangles

### fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 5024 | 84.3 | 18.5 | 90.39 | 9.86 / 40.83 | 3.58 / 17.57 | 2.45 / 7.63 |

### worker costs

| LOD | chunks | generate median Âµs | p95 | mesh median Âµs | p95 |
|---|---|---|---|---|---|
| 0 | 19762 | 4595 | 6897 | 282 | 476 |
| 1 | 13719 | 4941 | 7532 | 282 | 489 |
| 2 | 10587 | 5660 | 8359 | 290 | 504 |
| 3 | 6574 | 5766 | 8632 | 315 | 558 |
| 4 | 5818 | 5817 | 9305 | 325 | 531 |
| 5 | 4308 | 5537 | 8357 | 317 | 498 |
| 6 | 2037 | 5029 | 9702 | 377 | 549 |
| 7 | 1919 | 3759 | 8384 | 338 | 529 |
| 8 | 1517 | 3325 | 7046 | 325 | 567 |
| 9 | 1197 | 3231 | 6545 | 338 | 571 |
| 10 | 1678 | 2967 | 8175 | 330 | 495 |

## 1024 half-heights


Timing is only valid on an idle machine: a build or a render elsewhere during this run makes these numbers provisional.

planet: s = 1 m, H = 2048 blocks, sea level 1024 blocks, N = 1048576, R = 724.6 km, vertical scale 0.11373 (1/8.8), roughness 1, max LOD 10 (1024 m voxels), lod factor 5, rayon threads 4, AO true, skirt 2

### static poses

| pose | GPU avg ms | GPU p95 | CPU avg ms | CPU p95 | tris | water tris | draws | drawn chunks/LOD | loaded | mesh VRAM MB | settle s |
|---|---|---|---|---|---|---|---|---|---|---|---|
| stand_hills | 1.11 | 1.73 | 2.12 | 4.01 | 1.27M | 0.00M | 1114 | L0:3096 L1:1573 L2:1140 L3:921 L4:972 L5:671 L6:313 L7:302 L8:302 L9:307 L10:332 | 9929 | 30.0 | 25.2 |
| stand_mountains | 2.12 | 3.27 | 1.58 | 1.92 | 1.72M | 0.20M | 1336 | L0:3408 L1:1610 L2:1296 L3:970 L4:683 L5:636 L6:321 L7:313 L8:367 L9:370 L10:317 | 10361 | 40.9 | 24.1 |
| alt_100m | 2.18 | 3.30 | 2.30 | 3.56 | 1.26M | 0.01M | 1121 | L0:2844 L1:1590 L2:1125 L3:926 L4:973 L5:671 L6:313 L7:306 L8:361 L9:364 L10:315 | 9862 | 31.8 | 23.6 |
| alt_1km | 1.74 | 1.85 | 1.69 | 2.38 | 0.96M | 0.01M | 860 | L1:85 L2:1370 L3:940 L4:973 L5:671 L6:313 L7:306 L8:361 L9:364 L10:320 | 5773 | 26.2 | 1.6 |
| alt_10km | 1.66 | 1.75 | 1.94 | 2.91 | 0.65M | 0.01M | 619 | L5:640 L6:344 L7:306 L8:357 L9:361 L10:394 | 2442 | 17.7 | 1.7 |
| alt_100km | 2.13 | 2.63 | 1.75 | 2.73 | 0.83M | 0.01M | 546 | L8:116 L9:379 L10:895 | 1390 | 14.5 | 2.7 |
| alt_500km | 3.01 | 3.44 | 2.12 | 4.25 | 3.13M | 0.00M | 1528 | L10:2055 | 2055 | 22.3 | 2.2 |

AO cost over 11550 loaded chunks: 3957287 opaque quads with AO vs 3416397 without (+15.8%), i.e. 7.91M vs 6.83M triangles

### fixed flight (60 s: 50 m AGL at 50 m/s, climb to 5 km, dive back)

| frames | avg fps | 1% low fps | worst frame ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|
| 4715 | 79.5 | 9.4 | 174.37 | 7.13 / 81.79 | 3.77 / 22.41 | 2.41 / 9.37 |

### worker costs

| LOD | chunks | generate median Âµs | p95 | mesh median Âµs | p95 |
|---|---|---|---|---|---|
| 0 | 19080 | 4508 | 6424 | 275 | 456 |
| 1 | 13352 | 4961 | 6838 | 277 | 473 |
| 2 | 9404 | 5388 | 7386 | 288 | 461 |
| 3 | 7601 | 5201 | 7404 | 294 | 455 |
| 4 | 6425 | 5546 | 7682 | 303 | 476 |
| 5 | 4382 | 5463 | 7718 | 314 | 491 |
| 6 | 2056 | 4952 | 9496 | 371 | 546 |
| 7 | 1991 | 3563 | 8662 | 326 | 488 |
| 8 | 1589 | 3074 | 7031 | 316 | 473 |
| 9 | 1240 | 2972 | 6297 | 308 | 484 |
| 10 | 2209 | 2726 | 6925 | 315 | 467 |


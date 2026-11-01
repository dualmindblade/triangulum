# terrain inspection: T2 after

vertical scale 1/70.3, s = 1 m, landforms v3 (local relief in metres, ceiling 1024 m), roughness 1, landform 1

- peak: lat 69.1133 lon 122.1579 (bake: elev 8.579 km, rough 0.003 km, uplift 1.20, erodibility 0.76, flow 0.00, temp -58.0 C, precip 3 mm, ocean 0.00)
- coast: lat 39.9390 lon -95.5865 (bake: elev -0.211 km, rough 4.846 km, uplift 0.56, erodibility 1.11, flow 1.64, temp -6.3 C, precip 1770 mm, ocean 0.37)
- river: lat 1.4274 lon 118.7243 (bake: elev 1.148 km, rough 0.151 km, uplift 0.00, erodibility 0.86, flow 2.71, temp 5.0 C, precip 1450 mm, ocean 0.00)
- plain: lat -2.9480 lon 63.2991 (bake: elev 0.229 km, rough 0.004 km, uplift 0.00, erodibility 0.79, flow 1.36, temp 9.6 C, precip 1279 mm, ocean 0.00)
- mesa: lat -11.3510 lon -40.2493 (bake: elev 0.609 km, rough 0.165 km, uplift 0.05, erodibility 0.74, flow 2.20, temp 3.5 C, precip 184 mm, ocean 0.00)

## slope (rise over run)

| where | spacing | slope p10 | p50 | p90 | p99 | max | frac < 0.02 | frac > 0.7 | frac > 1.0 |
|---|---|---|---|---|---|---|---|---|---|
| peak | 1 m | 0.077 | 0.198 | 0.417 | 0.693 | 0.876 | 0.01 | 0.009 | 0.000 |
| peak | 16 m | 0.104 | 0.446 | 0.874 | 1.316 | 1.587 | 0.00 | 0.218 | 0.056 |
| coast | 1 m | 0.194 | 1.354 | 3.306 | 3.827 | 4.009 | 0.00 | 0.540 | 0.512 |
| coast | 16 m | 0.182 | 0.374 | 1.597 | 3.143 | 4.197 | 0.00 | 0.175 | 0.139 |
| river | 1 m | 0.004 | 0.011 | 0.236 | 1.095 | 1.965 | 0.55 | 0.031 | 0.020 |
| river | 16 m | 0.034 | 0.132 | 0.383 | 0.655 | 1.179 | 0.05 | 0.007 | 0.001 |
| plain | 1 m | 0.038 | 0.098 | 0.235 | 0.395 | 0.626 | 0.02 | 0.000 | 0.000 |
| plain | 16 m | 0.016 | 0.045 | 0.093 | 0.152 | 0.226 | 0.14 | 0.000 | 0.000 |
| mesa | 1 m | 0.096 | 0.114 | 1.832 | 3.902 | 6.450 | 0.03 | 0.239 | 0.193 |
| mesa | 16 m | 0.022 | 0.122 | 0.469 | 0.873 | 1.427 | 0.09 | 0.033 | 0.003 |
| 60 random land patches | 1 m | 0.036 | 0.163 | 0.829 | 2.233 | 3.923 | 0.06 | 0.123 | 0.078 |
| 60 random land patches | 16 m | 0.014 | 0.063 | 0.426 | 1.038 | 7.634 | 0.15 | 0.039 | 0.012 |

## large-scale fidelity

| window | windows | mean(h - bake) RMS m | p95 abs m | max abs m | bake std in window, median m | RMS / bake std | detail std, median m | LOD6 vs LOD0 window mean, max abs m |
|---|---|---|---|---|---|---|---|---|
| 10 km | 120 | 90.94 | 249.96 | 330.42 | 4.9 | 18.496 | 7.48 | 5.09 |
| 20 km | 120 | 67.88 | 167.96 | 267.95 | 12.2 | 5.568 | 8.38 | 5.25 |

## lattice artifacts (aspect anisotropy)

aspect of the detail gradient mod 90 deg (5 deg bins, / mean): 1.08 1.07 1.04 1.04 0.99 0.99 0.98 0.94 0.94 0.90 0.89 0.92 0.97 1.01 1.04 1.04 1.06 1.08
max/min bin ratio 1.215; 0 deg bin 1.084, 45 deg bin 0.898

## LOD consistency

| LOD | voxel m | mean abs(coarse - mean of fine) m | in voxels | p99 in voxels | max in voxels |
|---|---|---|---|---|---|
| 1 | 2 | 0.056 | 0.028 | 0.182 | 0.337 |
| 2 | 4 | 0.349 | 0.087 | 0.496 | 0.719 |
| 3 | 8 | 0.660 | 0.082 | 0.474 | 0.904 |
| 4 | 16 | 1.323 | 0.083 | 0.496 | 0.980 |
| 5 | 32 | 2.072 | 0.065 | 0.404 | 0.763 |
| 6 | 64 | 3.301 | 0.052 | 0.426 | 1.260 |
| 7 | 128 | 5.312 | 0.041 | 0.377 | 0.846 |

(stats took 14.2 s)

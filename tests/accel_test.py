"""Accelerator parity tests.

  python tests/accel_test.py

* numba hydrology cores must be BIT-IDENTICAL to the pure-Python fallbacks
  (same heap tie-breaking, same accumulation order).
* the CuPy climate backend (if cupy is installed) must match the CPU stage
  within float32 tolerance on a small grid; skipped otherwise.
"""
import os
import sys
import time

import numpy as np

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from planetgen import hydrology as H          # noqa: E402
from planetgen import climate, noise, tectonics  # noqa: E402
from planetgen.config import PlanetConfig     # noqa: E402
from planetgen.grid import Grid               # noqa: E402


def test_numba_bit_exact():
    cfg = PlanetConfig()
    g = Grid(5, cfg.radius_km, jitter=cfg.grid_jitter, seed=5, kind="fibonacci")
    elev = noise.fbm(g.xyz, octaves=5, freq=2.0, seed=11) * 2.0
    is_ocean = elev < 0.0

    saved = H._njit
    if saved is None:
        print("numba not installed - skipping (pure Python in use)")
        return
    H._njit = None
    fill_py = H._priority_flood(g, elev, is_ocean)
    H._njit = saved
    fill_nb = H._priority_flood(g, elev, is_ocean)
    assert np.array_equal(fill_py, fill_nb), "priority flood diverged"

    rcv = H._receivers(g, fill_py, is_ocean)
    order = np.argsort(-fill_py)
    order = order[~is_ocean[order]]
    rng = np.random.default_rng(7)
    runoff = rng.random(g.n) * 100.0
    eroded = rng.random(g.n) * 5.0
    capacity = rng.random(g.n) * 3.0
    H._njit = None
    ref = H._accumulate(order, rcv, runoff, eroded, capacity, 0.55)
    H._njit = saved
    out = H._accumulate(order, rcv, runoff, eroded, capacity, 0.55)
    for a, b, name in zip(ref, out, ("acc", "dep", "sink")):
        assert np.array_equal(a, b), f"accumulate {name} diverged"
    print("numba hydrology: bit-exact PASS")


def test_gpu_climate_parity():
    try:
        import cupy  # noqa: F401
        cupy.arange(4).sum()  # probe the device really works
    except Exception as e:
        print(f"cupy unavailable ({type(e).__name__}) - skipping GPU parity")
        return
    cfg = PlanetConfig()
    cfg.seed = 5
    g = Grid(5, cfg.radius_km, jitter=cfg.grid_jitter, seed=cfg.seed,
             kind="fibonacci")
    t = tectonics.generate(g, cfg)
    t0 = time.time()
    cfg.gpu = False
    ref = climate.simulate(g, cfg, t["elev"], t["is_ocean"])
    t_cpu = time.time() - t0
    t0 = time.time()
    cfg.gpu = True
    out = climate.simulate(g, cfg, t["elev"], t["is_ocean"])
    t_gpu = time.time() - t0
    for k, tol in (("T", 0.05), ("P", 1.0), ("wind_e", 0.02), ("T_ann", 0.05),
                   ("P_ann", 5.0), ("clouds", 0.01), ("maritime", 0.01)):
        a = np.nan_to_num(np.asarray(ref[k], np.float64), nan=0.0)
        b = np.nan_to_num(np.asarray(out[k], np.float64), nan=0.0)
        d = np.abs(a - b).max()
        assert d < tol, f"{k}: max|d| {d} exceeds tolerance {tol}"
    print(f"gpu climate: parity PASS (cpu {t_cpu:.1f}s, gpu {t_gpu:.1f}s)")


if __name__ == "__main__":
    test_numba_bit_exact()
    test_gpu_climate_parity()
    print("accel tests: ALL PASS")

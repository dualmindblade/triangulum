"""Fast end-to-end sanity check: python tests/smoke_test.py

Runs the full pipeline (minus rendering) at a tiny resolution and asserts
invariants that should survive any parameter tuning. Finishes in ~30s.
"""

import os
import sys
import tempfile
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import numpy as np

from planetgen.config import PlanetConfig
from planetgen.grid import Grid
from planetgen.pipeline import Run


def check_grid():
    for kind, jitter in (("icosphere", 0.0), ("fibonacci", 0.35)):
        g = Grid(5, jitter=jitter, seed=7, kind=kind)
        assert g.n == 10 * 4 ** 5 + 2
        deg = g.deg.astype(int)
        if kind == "icosphere":
            assert (deg == 5).sum() == 12 and set(np.unique(deg)) == {5, 6}
        else:
            assert deg.min() >= 4 and g.max_deg <= 12, (deg.min(), g.max_deg)
        assert abs(g.area_km2.sum() / (4 * np.pi * g.radius_km ** 2) - 1) < 1e-9
        # adjacency is symmetric
        for i in [0, 777, g.n - 1]:
            for j in g.nbr[i]:
                if j >= 0:
                    assert i in g.nbr[j]
        # conservative advection conserves mass
        q = np.exp(-((g.lat - 0.3) ** 2 + (g.lon - 1.0) ** 2) * 30)
        v = np.stack([np.full(g.n, 10.0), np.zeros(g.n)], 1)
        m0 = (q * g.area_km2).sum()
        for _ in range(30):
            q = g.advect(q, v)
        assert abs((q * g.area_km2).sum() / m0 - 1) < 0.01
        # bounded advection respects the max principle
        s = np.sin(g.lon) * 10
        hi, lo = s.max(), s.min()
        for _ in range(30):
            s = g.advect_interp(s, v)
        assert s.max() <= hi + 1e-9 and s.min() >= lo - 1e-9
        print(f"grid OK ({kind}: max_deg {g.max_deg}, "
              f"area spread {g.area_km2.min()/g.area_km2.max():.2f})")


def check_pipeline():
    cfg = PlanetConfig(seed=99, subdivisions=5)
    out = tempfile.mkdtemp(prefix="planetgen_smoke_")
    state = Run(cfg, out).run(until_stage="chronicle", quiet=True)

    t, h, c, k = state["tect"], state["hydro"], state["clim2"], state["koppen"]
    g = Grid(cfg.subdivisions, cfg.radius_km)
    area = g.area_km2

    for name in ("elev", "flow_acc", "soil"):
        assert np.isfinite(h[name]).all(), f"NaN in hydro[{name}]"
    for name in ("T", "P", "wind_e", "wind_n", "clouds"):
        assert np.isfinite(c[name]).all(), f"NaN in clim[{name}]"

    land_frac = area[~h["is_ocean"]].sum() / area.sum()
    assert abs(land_frac - (1 - cfg.ocean_fraction)) < 0.03, land_frac

    assert -12.5 < h["elev"].min() < -6 and 5 < h["elev"].max() < 10.5

    land = ~h["is_ocean"]
    p_land = (c["P_ann"][land] * area[land]).sum() / area[land].sum()
    assert abs(p_land / cfg.target_land_precip_mm - 1) < 0.05, p_land

    t_mean = c["T_ann"].mean()
    assert 5 < t_mean < 20, t_mean

    n_classes = len(np.unique(k[k >= 0]))
    assert n_classes >= 12, f"only {n_classes} Koppen classes"
    assert h["river"].sum() > 50
    assert state["features"]["planet"]
    print(f"pipeline OK (land {land_frac*100:.1f}%, P_land {p_land:.0f} mm, "
          f"T {t_mean:.1f} C, {n_classes} Koppen classes, "
          f"{len(h['lakes'])} lakes, planet '{state['features']['planet']}')")


if __name__ == "__main__":
    t0 = time.time()
    check_grid()
    check_pipeline()
    print(f"ALL SMOKE TESTS PASSED ({time.time() - t0:.0f}s)")

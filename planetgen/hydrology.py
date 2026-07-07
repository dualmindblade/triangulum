"""Stage 3: hydrology — rivers, erosion, deposition, lakes.

Runoff comes from the climate stage (precip minus evapotranspiration). The
erosion loop interleaves:

  * priority-flood depression filling (so every land cell drains somewhere)
  * steepest-descent flow routing and discharge accumulation
  * stream-power channel incision  E ~ K * Q^m * S
  * sediment transport with deposition where carrying capacity drops
    (floodplains, inland basins, coastal deltas)
  * hillslope diffusion, and continued tectonic uplift — active orogens keep
    rising while they erode, which is what keeps young ranges rugged and
    lets ancient ones decay into smooth stumps

Afterwards, remaining depressions become lakes: open (overflowing, fresh) if
inflow beats evaporation, else shrunken endorheic salt lakes.
"""

from __future__ import annotations

import heapq

import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components

from . import noise

# numba compiles the two per-iteration passes that are plain Python loops
# (the priority-flood heap and the downstream accumulation). The compiled
# versions replicate the Python semantics exactly — including heapq's
# (key, index) tie-breaking — so results are bit-identical; pure Python
# remains as the fallback when numba isn't installed.
try:
    from numba import njit as _njit
except ImportError:  # pragma: no cover - numba is an optional accelerator
    _njit = None

MM_YR_KM2_TO_M3S = 3.171e-5


def _priority_flood(grid, elev, is_ocean):
    """Depression-filled surface: every land cell can reach the ocean along a
    monotonically descending path on the returned surface."""
    coastal = np.flatnonzero(is_ocean & (grid.nbr_ok & ~is_ocean[grid.nbr_safe]).any(1))
    if _njit is not None:
        return _pf_core(elev, is_ocean, grid.nbr, coastal)
    fill = elev.copy()
    closed = is_ocean.copy()
    heap = []
    for i in coastal:
        heap.append((elev[i], i))
    heapq.heapify(heap)
    nbr = grid.nbr
    max_deg = nbr.shape[1]
    eps = 1e-6
    while heap:
        f, i = heapq.heappop(heap)
        for s in range(max_deg):
            j = nbr[i, s]
            if j < 0 or closed[j]:
                continue
            closed[j] = True
            fill[j] = max(elev[j], f + eps)
            heapq.heappush(heap, (fill[j], j))
    return fill


if _njit is not None:

    @_njit(cache=True)
    def _pf_core(elev, is_ocean, nbr, coastal):
        n = elev.shape[0]
        fill = elev.copy()
        closed = is_ocean.copy()
        # binary min-heap over (key, index), ordered exactly like heapq's
        # (float, int) tuples so the pop sequence — and therefore every
        # fill value — matches the Python implementation bit for bit
        cap = n + coastal.shape[0] + 1
        hk = np.empty(cap, np.float64)
        hi = np.empty(cap, np.int64)
        size = 0
        for c in range(coastal.shape[0]):
            i = coastal[c]
            k = elev[i]
            # push
            p = size
            hk[p] = k
            hi[p] = i
            size += 1
            while p > 0:
                q = (p - 1) >> 1
                if hk[p] < hk[q] or (hk[p] == hk[q] and hi[p] < hi[q]):
                    hk[p], hk[q] = hk[q], hk[p]
                    hi[p], hi[q] = hi[q], hi[p]
                    p = q
                else:
                    break
        max_deg = nbr.shape[1]
        eps = 1e-6
        while size > 0:
            f = hk[0]
            i = hi[0]
            size -= 1
            hk[0] = hk[size]
            hi[0] = hi[size]
            # sift down
            p = 0
            while True:
                l = 2 * p + 1
                r = l + 1
                m = p
                if l < size and (hk[l] < hk[m] or (hk[l] == hk[m] and hi[l] < hi[m])):
                    m = l
                if r < size and (hk[r] < hk[m] or (hk[r] == hk[m] and hi[r] < hi[m])):
                    m = r
                if m == p:
                    break
                hk[p], hk[m] = hk[m], hk[p]
                hi[p], hi[m] = hi[m], hi[p]
                p = m
            for s in range(max_deg):
                j = nbr[i, s]
                if j < 0 or closed[j]:
                    continue
                closed[j] = True
                fj = elev[j]
                if fj < f + eps:
                    fj = f + eps
                fill[j] = fj
                # push
                p = size
                hk[p] = fj
                hi[p] = j
                size += 1
                while p > 0:
                    q = (p - 1) >> 1
                    if hk[p] < hk[q] or (hk[p] == hk[q] and hi[p] < hi[q]):
                        hk[p], hk[q] = hk[q], hk[p]
                        hi[p], hi[q] = hi[q], hi[p]
                        p = q
                    else:
                        break
        return fill


def _receivers(grid, fill, is_ocean, acc=None, capture=0.0, meander_w=None):
    """Downhill receiver on the filled surface (self for ocean/pits).

    With `acc` from the previous iteration and capture > 0, cells prefer the
    downhill neighbor that already carries flow — a valley-capture model that
    merges adjacent streams into dendritic networks instead of letting them
    run in parallel down a regional slope.

    meander_w: fixed seeded per-edge weights. Pure steepest descent locks
    onto the grid's local lattice directions on smooth regional slopes
    (parallel rivers); the weights randomize *which* downhill neighbor wins
    so channels wind. Fixed across iterations, so the network reinforces
    rather than thrashing. Cycle-safe: only strictly downhill neighbors are
    ever candidates.
    """
    drop = (fill[:, None] - fill[grid.nbr_safe]) / grid.edge_km
    drop = np.where(grid.nbr_ok, drop, -np.inf)
    score = drop
    if meander_w is not None:
        score = np.where(drop > 0, score * meander_w, score)
    if acc is not None and capture > 0.0:
        pref = 1.0 + capture * np.log1p(np.maximum(acc[grid.nbr_safe], 0.0) / 100.0)
        score = np.where(drop > 0, score * pref, score)
    best = score.argmax(1)
    rcv = grid.nbr_safe[np.arange(grid.n), best]
    has_down = drop[np.arange(grid.n), best] > 0
    rcv = np.where(has_down & ~is_ocean, rcv, np.arange(grid.n))
    return rcv


def _accumulate(order, rcv, runoff_m3s, eroded_vol=None, capacity=None, deposit_fraction=0.55):
    """Downstream pass: discharge accumulation and (optionally) sediment flux.

    Returns (acc, deposit_vol, ocean_flux) — deposit per cell and the sediment
    volume delivered to each cell's receiver when that receiver is a sink.
    """
    if _njit is not None:
        with_flux = eroded_vol is not None
        ev = eroded_vol if with_flux else np.zeros(1)
        cap = capacity if with_flux else np.zeros(1)
        return _acc_core(order, rcv, runoff_m3s, ev, cap,
                         float(deposit_fraction), with_flux)
    acc = runoff_m3s.copy()
    n = len(acc)
    dep = np.zeros(n)
    sink_flux = np.zeros(n)
    flux = None if eroded_vol is None else eroded_vol.copy()
    for i in order:
        r = rcv[i]
        if r == i:
            if flux is not None:
                sink_flux[i] += flux[i]
            continue
        acc[r] += acc[i]
        if flux is not None:
            f = flux[i]
            if f > capacity[i]:
                d = (f - capacity[i]) * deposit_fraction
                dep[i] += d
                f -= d
            flux[r] += f
    return acc, dep, sink_flux


if _njit is not None:

    @_njit(cache=True)
    def _acc_core(order, rcv, runoff, eroded_vol, capacity, deposit_fraction, with_flux):
        acc = runoff.copy()
        n = acc.shape[0]
        dep = np.zeros(n)
        sink_flux = np.zeros(n)
        flux = eroded_vol.copy() if with_flux else np.zeros(1)
        for k in range(order.shape[0]):
            i = order[k]
            r = rcv[i]
            if r == i:
                if with_flux:
                    sink_flux[i] += flux[i]
                continue
            acc[r] += acc[i]
            if with_flux:
                f = flux[i]
                if f > capacity[i]:
                    d = (f - capacity[i]) * deposit_fraction
                    dep[i] += d
                    f -= d
                flux[r] += f
        return acc, dep, sink_flux


def simulate(grid, cfg, elev_in, climate_out, tect, rng=None, rec=None):
    if rng is None:
        rng = np.random.default_rng([cfg.seed, 3])
    elev = elev_in.copy()
    N = grid.n
    area = grid.area_km2
    # sub-grid grit: breaks up implausibly parallel flow on gentle regional
    # slopes; incision feedback then organizes it into convergent networks
    grit = 0.05 * noise.fbm(grid.xyz, octaves=3, freq=cfg.noise_base_freq * 8.0,
                            seed=cfg.seed * 89 + 47)
    elev += np.where(elev > 0.0, grit, 0.0)

    p_ann = climate_out["P_ann"]
    pet = climate_out["pet_ann"]
    t_ann = climate_out["T_ann"]
    runoff_mm = np.maximum(p_ann - 0.7 * np.minimum(pet, p_ann), 0.02 * p_ann)
    runoff_m3s = runoff_mm * area * MM_YR_KM2_TO_M3S

    erod = tect["erodibility"]
    uplift = tect["uplift"] * cfg.uplift_rate_km \
        - tect.get("subsidence", 0.0) * cfg.subsidence_rate_km
    is_ocean = elev < 0.0
    frozen = t_ann < -8.0                     # ice-locked ground erodes far less fluvially
    k_cell = cfg.erode_k * erod * np.where(frozen, 0.25, 1.0)

    def _roughness(e, land_mask):
        """Mean local relief: std of elevation over each cell's neighborhood (km)."""
        s = np.where(grid.nbr_ok, e[grid.nbr_safe], np.nan)
        st = np.nanstd(np.concatenate([s, e[:, None]], 1), axis=1)
        return float(st[land_mask].mean())

    rough_seed = _roughness(elev, ~is_ocean)
    total_dep = np.zeros(N)
    acc = None
    dz_hist = []
    elev_prev = elev.copy()
    meander_w = 1.0 + cfg.river_meander * 2.0 * (rng.random((N, grid.nbr.shape[1])) - 0.5)
    # weathered ridge grain, injected late in the run. Ridge-only (positive
    # crests, no hollows): signed texture mints closed basins faster than
    # rivers can breach them and pockmarks the world with tiny lakes.
    tex = np.maximum(noise.ridged(grid.xyz, octaves=4, freq=cfg.noise_base_freq * 10.0,
                                  seed=cfg.seed * 107 + 63) - 0.55, 0.0) / 0.45
    for it in range(cfg.erosion_iters):
        if it == max(cfg.erosion_iters - 25, 0):
            # exposed bedrock keeps fine relief; thick sediment lies flat
            rocky = np.clip(1.0 - total_dep * 2.0 / 0.05, 0.0, 1.0) \
                * np.clip(elev / 0.35, 0.0, 1.0)
            elev += np.where(~is_ocean, cfg.bedrock_texture_km * tex * rocky, 0.0)
        fill = _priority_flood(grid, elev, is_ocean)
        rcv = _receivers(grid, fill, is_ocean, acc=acc, capture=cfg.river_capture,
                         meander_w=meander_w)
        if rec is not None:
            rec.frame("hydrology-lakes",
                      np.where(~is_ocean, np.minimum(fill - elev, 0.6), 0.0),
                      label=f"depressions/lakes  step {it + 1}/{cfg.erosion_iters}  (water depth)",
                      vmin=0.0, vmax=0.6, cmap="Blues")
        order = np.argsort(-fill)
        order = order[~is_ocean[order]]
        slope = np.maximum((fill - fill[rcv]) / grid.mean_edge_km, 0.0)

        acc, _, _ = _accumulate(order, rcv, runoff_m3s)
        aq = (np.maximum(acc, 0.0) / 350.0) ** cfg.erode_m
        dz = np.minimum(k_cell * aq * slope, cfg.erode_cap_km)
        dz = np.where(is_ocean, 0.0, dz)

        capacity = cfg.sediment_capacity_k * aq * slope * area.mean()
        _, dep, sink_flux = _accumulate(order, rcv, runoff_m3s,
                                        eroded_vol=dz * area, capacity=capacity,
                                        deposit_fraction=cfg.deposit_fraction)
        dz_dep = np.minimum(dep / area, 0.08)
        # sediment reaching the sea builds deltas and shelf aprons
        coastal_sink = sink_flux * cfg.delta_fraction
        dz_delta = np.minimum(coastal_sink / area, 0.05)

        elev += -dz + dz_dep + np.where(~is_ocean, uplift, 0.0)
        elev += np.where(is_ocean, dz_delta, np.minimum(dz_delta, 0.02))
        # slope-gated hillslope diffusion: steep faces shed mass (talus),
        # gentle terrain keeps its texture instead of blurring away
        lap = grid.laplacian(elev)
        gate = np.clip(slope / 0.002, 0.15, 2.5)
        elev += np.where(~is_ocean, cfg.diffusion_k * gate * lap, 0.0)
        total_dep += dz_dep + dz_delta
        is_ocean = elev < 0.0
        land_now = ~is_ocean
        dz_hist.append(float(np.abs(elev - elev_prev)[land_now].mean() * 1000.0))
        elev_prev = elev.copy()
        if rec is not None:
            if it == 0:
                rec.set_coast(is_ocean)
            lbl = f"erosion step {it + 1}/{cfg.erosion_iters}"
            rec.frame("hydrology-elevation", elev, label=lbl, cmap="hypso")
            rec.frame("hydrology-discharge", np.log10(np.maximum(acc, 1.0)),
                      label=lbl + "  (log10 discharge)", vmin=0, vmax=4.7, cmap="PuBu")

    marks = [0, len(dz_hist) // 4, len(dz_hist) // 2, 3 * len(dz_hist) // 4, len(dz_hist) - 1]
    print("[hydrology] settling, mean |dz| per step (m): "
          + "  ".join(f"step {i + 1}: {dz_hist[i]:.2f}" for i in marks), flush=True)

    # re-solve sea level (isostasy hand-wave): keep the target ocean fraction
    order_e = np.argsort(elev)
    cum = np.cumsum(area[order_e])
    k_idx = int(np.searchsorted(cum, cfg.ocean_fraction * area.sum()))
    elev -= elev[order_e[min(k_idx, N - 1)]]
    hi = elev > 6.2
    elev[hi] = 6.2 + 2.4 * np.tanh((elev[hi] - 6.2) / 2.4)
    is_ocean = elev < 0.0

    rough_end = _roughness(elev, ~is_ocean)
    print(f"[hydrology] land roughness (mean local relief): seed {rough_seed * 1000:.0f} m "
          f"-> final {rough_end * 1000:.0f} m "
          f"({'+' if rough_end >= rough_seed else ''}{(rough_end / rough_seed - 1) * 100:.0f}%)",
          flush=True)

    # ---- final routing, lakes, rivers ------------------------------------------
    fill = _priority_flood(grid, elev, is_ocean)
    rcv = _receivers(grid, fill, is_ocean, acc=acc, capture=cfg.river_capture,
                     meander_w=meander_w)
    order = np.argsort(-fill)
    order = order[~is_ocean[order]]
    acc, _, _ = _accumulate(order, rcv, runoff_m3s)

    depression = ~is_ocean & (fill - elev > 1e-4)
    lake_id = np.full(N, -1, np.int64)
    lake_level = np.full(N, np.nan)
    lake_salt = np.zeros(N, bool)
    lakes = []
    if depression.any():
        cells = np.flatnonzero(depression)
        remap = np.full(N, -1)
        remap[cells] = np.arange(len(cells))
        src, dst = [], []
        for s in range(grid.nbr.shape[1]):
            j = grid.nbr[cells, s]
            good = (j >= 0) & depression[np.maximum(j, 0)]
            src.append(remap[cells[good]])
            dst.append(remap[j[good]])
        adj = coo_matrix((np.ones(sum(len(s) for s in src)),
                          (np.concatenate(src), np.concatenate(dst))),
                         shape=(len(cells), len(cells)))
        n_comp, labels = connected_components(adj, directed=False)
        for c in range(n_comp):
            comp = cells[labels == c]
            spill = fill[comp].min()
            inflow = acc[comp].max()
            pet_lake = max(pet[comp].mean(), 50.0)
            full_area = area[comp].sum()
            evap_full = pet_lake * full_area * MM_YR_KM2_TO_M3S
            if inflow >= evap_full or len(comp) == 1:
                level, salt = spill, False
            else:
                by_depth = comp[np.argsort(elev[comp])]
                cum_area = np.cumsum(area[by_depth])
                need = pet_lake * cum_area * MM_YR_KM2_TO_M3S
                k = int(np.searchsorted(need, inflow))
                k = max(k, 1)
                level = elev[by_depth[min(k, len(comp) - 1)]]
                salt = True
            in_lake = comp[elev[comp] <= level]
            if len(in_lake) == 0:
                in_lake = comp[[np.argmin(elev[comp])]]
            # skip shallow ponds barely bigger than a cell (texture hollows);
            # keep small-but-deep basins (crater/rift lakes)
            if len(in_lake) < 3 and (level - elev[in_lake]).max() < 0.03:
                continue
            lid = len(lakes)
            lake_id[in_lake] = lid
            lake_level[in_lake] = level
            lake_salt[in_lake] = salt
            lakes.append(dict(id=lid, cells=len(in_lake), area_km2=float(area[in_lake].sum()),
                              level_km=float(level), salt=bool(salt),
                              deepest=float((level - elev[in_lake]).max())))

    is_lake = lake_id >= 0
    river = ~is_ocean & ~is_lake & (acc >= cfg.river_min_m3s)

    # soil: deposited sediment + climate-driven weathering, thin on steep rock
    slope_f = np.maximum((fill - fill[rcv]) / grid.mean_edge_km, 0.0)
    weather = 0.03 * np.clip((t_ann + 5.0) / 30.0, 0.0, 1.0) \
        * np.clip(p_ann / 800.0, 0.0, 1.5) * np.clip(1.0 - slope_f * 60.0, 0.15, 1.0)
    soil = np.clip(total_dep * 2.0 + np.where(~is_ocean, weather, 0.0), 0.0, 0.2)

    return dict(elev=elev, is_ocean=is_ocean, flow_acc=acc, receiver=rcv,
                river=river, lake_id=lake_id, lake_level=lake_level,
                lake_salt=lake_salt, lakes=lakes, soil=soil, runoff_mm=runoff_mm,
                settle_history=np.array(dz_hist))

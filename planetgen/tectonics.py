"""Stage 1: plate tectonics and base terrain.

Plates are grown as noise-warped Voronoi regions and rotate about random Euler
poles (as real plates do), so every boundary gets a relative-motion vector
that classifies it as convergent / divergent / transform with a rate. Terrain
follows textbook geology:

  continent-continent convergence  -> broad high orogens (Himalaya)
  ocean-continent convergence      -> offshore trench + volcanic cordillera (Andes)
  ocean-ocean convergence          -> trench + volcanic island arc (Japan)
  divergent (oceanic)              -> mid-ocean ridge; seafloor deepens with crust age
  divergent (continental)          -> rift valley with shoulders (East Africa)
  transform                        -> shear roughness

Deep history: one or two *older* plate configurations are generated and their
collision zones are stamped onto today's continents as low, eroded ranges
(Appalachians / Urals analogs). Mantle hotspots leave island chains trailing
along plate motion (Hawaii). Sea level is solved so a target fraction of the
surface is flooded; continental shelves emerge from the crust-edge taper.

All distance transforms use a roughened metric (Grid.rough_metric) and fine
noise uses a gentle domain warp — exact distances and strongly-warped noise
both imprint directional striations that rivers then follow.
"""

from __future__ import annotations

import numpy as np

from . import noise

CONV, DIV, TRANS = 1, 2, 3


# ----------------------------------------------------------------------
def _smoothstep(x):
    x = np.clip(x, 0.0, 1.0)
    return x * x * (3.0 - 2.0 * x)


def _farthest_seeds(grid, k, rng, mask=None, first=None):
    """k well-separated cells (optionally within mask), farthest-point sampling."""
    if mask is None:
        mask = np.ones(grid.n, bool)
    idx = np.flatnonzero(mask)
    seeds = [first if first is not None else rng.choice(idx)]
    d = np.full(grid.n, np.inf)
    for _ in range(k - 1):
        d = np.minimum(d, ((grid.xyz - grid.xyz[seeds[-1]]) ** 2).sum(1))
        # pick randomly among the farthest 5% for variety between seeds
        cand = idx[np.argsort(-d[idx])[: max(1, len(idx) // 20)]]
        seeds.append(rng.choice(cand))
    return np.array(seeds)


def _euler_velocity(grid, pole, omega_rad_myr):
    """Surface velocity (km/Myr) of a rigid rotation, as (east, north) components."""
    v3 = omega_rad_myr * np.cross(pole, grid.xyz) * grid.radius_km
    return np.einsum("nd,nd->n", v3, grid.east), np.einsum("nd,nd->n", v3, grid.north)


def _rotate_points(p, axis, angle):
    """Rodrigues rotation of points p (n, 3) about a unit axis."""
    c, s = np.cos(angle), np.sin(angle)
    return (p * c + np.cross(axis, p) * s
            + axis * (p @ axis)[..., None] * (1 - c))


# ----------------------------------------------------------------------
def _plate_layout(grid, rng, n_plates, warp_amp, warp_freq, warp_seed):
    """Assign every cell to a plate: warped-Voronoi around well-spread seeds."""
    seeds = _farthest_seeds(grid, n_plates, rng)
    wpos = noise.warp(grid.xyz, amp=warp_amp, freq=warp_freq, seed=warp_seed)
    weight = rng.uniform(0.72, 1.38, n_plates)  # size variety
    d2 = ((wpos[:, None, :] - grid.xyz[seeds][None, :, :]) ** 2).sum(2) / weight[None, :]
    plate = d2.argmin(1)
    poles = rng.normal(size=(n_plates, 3))
    poles /= np.linalg.norm(poles, axis=1, keepdims=True)
    return plate, seeds, poles


def _classify_boundaries(grid, sub, ve, vn, conv_threshold=10.0):
    """Per-cell boundary regime from relative plate motion across cross-edges.

    Returns dict of per-cell arrays; meaningful only where mask is True.
    """
    sub_j = sub[grid.nbr_safe]
    cross = grid.nbr_ok & (sub_j != sub[:, None])
    rel_e = ve[:, None] - ve[grid.nbr_safe]
    rel_n = vn[:, None] - vn[grid.nbr_safe]
    conv_edge = rel_e * grid.dir_e + rel_n * grid.dir_n   # >0: closing toward that neighbor
    shear_edge = np.sqrt(np.maximum(rel_e**2 + rel_n**2 - conv_edge**2, 0.0))

    n_cross = np.maximum(cross.sum(1), 1)
    conv = np.where(cross, conv_edge, 0.0).sum(1) / n_cross
    shear = np.where(cross, shear_edge, 0.0).sum(1) / n_cross
    # the neighbor that defines this boundary segment (strongest relative motion)
    saliency = np.where(cross, np.abs(conv_edge) + shear_edge, -1.0)
    other = grid.nbr_safe[np.arange(grid.n), saliency.argmax(1)]

    mask = cross.any(1)
    btype = np.zeros(grid.n, np.int8)
    is_conv = mask & (conv > conv_threshold) & (conv > 0.5 * shear)
    is_div = mask & (conv < -conv_threshold) & (-conv > 0.5 * shear)
    btype[mask] = TRANS
    btype[is_conv] = CONV
    btype[is_div] = DIV
    strength = np.where(btype == CONV, conv, np.where(btype == DIV, -conv, shear))
    strength = np.where(mask, np.maximum(strength, 0.0), 0.0)
    return dict(mask=mask, btype=btype, strength=strength, other=other)


def _safe_lab(lab):
    return np.maximum(lab, 0)


# ----------------------------------------------------------------------
def generate(grid, cfg, rec=None):
    rng = np.random.default_rng([cfg.seed, 1])
    N = grid.n
    R = grid.radius_km

    def snap(field, label):
        if rec is not None:
            rec.frame("tectonics-construction", field, label=label, cmap="hypso")

    # ---- plates and motion ------------------------------------------------
    plate, seeds, poles = _plate_layout(
        grid, rng, cfg.n_plates, cfg.plate_warp_amp, cfg.plate_warp_freq,
        warp_seed=cfg.seed * 13 + 5)
    lo, hi = np.radians(cfg.plate_omega_deg_myr[0]), np.radians(cfg.plate_omega_deg_myr[1])
    omegas = rng.uniform(lo, hi, cfg.n_plates) * rng.choice([-1, 1], cfg.n_plates)

    # subplates: the largest plates fracture, children inherit perturbed motion
    plate_areas = np.bincount(plate, weights=grid.area_km2, minlength=cfg.n_plates)
    order = np.argsort(-plate_areas)
    sub = plate.copy()
    sub_parent = list(range(cfg.n_plates))
    sub_pole = list(poles)
    sub_omega = list(omegas)
    next_id = cfg.n_plates
    for p in order[: max(2, cfg.n_plates // 3)]:
        n_children = int(rng.integers(2, cfg.max_subplates + 1))
        cells = np.flatnonzero(plate == p)
        if len(cells) < 200 or n_children < 2:
            continue
        child_seeds = _farthest_seeds(grid, n_children, rng, mask=(plate == p))
        wpos = noise.warp(grid.xyz[cells], amp=cfg.plate_warp_amp * 0.8,
                          freq=cfg.plate_warp_freq * 1.6, seed=cfg.seed * 17 + p)
        d2 = ((wpos[:, None, :] - grid.xyz[child_seeds][None, :, :]) ** 2).sum(2)
        which = d2.argmin(1)
        for c in range(1, n_children):
            sel = cells[which == c]
            sub[sel] = next_id
            tilt = rng.normal(size=3) * 0.35
            cp = poles[p] + tilt
            sub_pole.append(cp / np.linalg.norm(cp))
            sub_omega.append(omegas[p] * rng.uniform(0.8, 1.2))
            sub_parent.append(p)
            next_id += 1
    sub_pole = np.array(sub_pole)
    sub_omega = np.array(sub_omega)
    sub_parent = np.array(sub_parent)

    ve = np.zeros(N)
    vn = np.zeros(N)
    for s in range(next_id):
        m = sub == s
        if m.any():
            e, n = _euler_velocity(grid, sub_pole[s], sub_omega[s])
            ve[m], vn[m] = e[m], n[m]

    # ---- continental crust -------------------------------------------------
    # mark plates continental until ~continental_fraction of surface is covered
    total_area = grid.area_km2.sum()
    shuffled = rng.permutation(cfg.n_plates)
    continental = np.zeros(cfg.n_plates, bool)
    acc = 0.0
    infill = {}
    for p in shuffled:
        if acc >= cfg.continental_fraction * total_area:
            break
        continental[p] = True
        infill[p] = rng.uniform(0.55, 0.9)     # fraction of the plate carrying crust
        acc += plate_areas[p] * infill[p]

    rough = grid.rough_metric(cfg.seed, amount=0.55)
    plate_edge = grid.nbr_ok & (plate[grid.nbr_safe] != plate[:, None])
    bdist_plate = grid.distance_to(plate_edge.any(1), edge_scale=rough)
    craton = np.zeros(N, bool)
    craton_score = (noise.fbm(noise.warp(grid.xyz, 0.4, 1.1, seed=cfg.seed * 29 + 3),
                              octaves=4, freq=1.4, seed=cfg.seed * 31 + 7))
    for p in range(cfg.n_plates):
        if not continental[p]:
            continue
        m = plate == p
        score = craton_score[m] + cfg.craton_center_bias * (
            bdist_plate[m] / max(bdist_plate[m].max(), 1.0))
        thr = np.quantile(score, 1.0 - infill[p])
        sel = np.flatnonzero(m)[score > thr]
        craton[sel] = True

    # ---- boundary classification -------------------------------------------
    b = _classify_boundaries(grid, sub, ve, vn)
    btype, strength, other = b["btype"], b["strength"], b["other"]
    me_c, oth_c = craton, craton[other]
    cls_cc = (btype == CONV) & me_c & oth_c
    cls_oc = (btype == CONV) & (me_c ^ oth_c)
    cls_oo = (btype == CONV) & ~me_c & ~oth_c
    # subduction polarity: ocean dives under continent; for ocean-ocean pick a
    # consistent pseudo-random winner per plate pair
    pair_lo = np.minimum(sub, sub[other]).astype(np.int64)
    pair_hi = np.maximum(sub, sub[other]).astype(np.int64)
    pair_bit = ((pair_lo * 73856093 + pair_hi * 19349663 + cfg.seed) // 7) % 2 == 0
    over_sub = np.where(pair_bit, pair_lo, pair_hi)
    over_is_me = np.where(cls_oc, me_c, sub == over_sub)
    other_sub = sub[other]

    # ---- influence fields ----------------------------------------------------
    d_conv, l_conv = grid.nearest_source(btype == CONV, edge_scale=rough)
    d_div, l_div = grid.nearest_source(btype == DIV, edge_scale=rough)
    d_trans, l_trans = grid.nearest_source(btype == TRANS, edge_scale=rough)
    sc = _safe_lab(l_conv)
    sd = _safe_lab(l_div)
    st = _safe_lab(l_trans)

    def side_of(src):
        """+True if a cell sits on the same side of the boundary as its source cell."""
        return (sub == sub[src]) | ((plate == plate[src]) & (sub != other_sub[src]))

    s_conv = np.clip(strength[sc] / cfg.conv_rate_norm, 0.0, 1.6)
    same_side = side_of(sc)
    on_over = same_side == over_is_me[sc]

    # ---- seafloor from crust age --------------------------------------------
    ridge_src = (btype == DIV) & ~me_c & ~oth_c
    same_plate_edge = grid.nbr_ok & (plate[grid.nbr_safe] == plate[:, None])
    d_ridge, l_ridge = grid.nearest_source(ridge_src, edge_ok=same_plate_edge,
                                           edge_scale=rough)
    half_rate = np.maximum(strength[_safe_lab(l_ridge)] * 0.5, 6.0)
    age = np.where(np.isfinite(d_ridge), d_ridge / half_rate, cfg.seafloor_age_max_myr)
    age = np.clip(age, 0.0, cfg.seafloor_age_max_myr)
    a_d, b_d = cfg.seafloor_depth_kms
    depth_age = -(a_d + b_d * np.sqrt(age))

    # ---- continental platform / shelf ----------------------------------------
    # distance from the craton edge, measured separately inside and outside
    craton_edge = craton & (grid.nbr_ok & ~craton[grid.nbr_safe]).any(1)
    d_edge = grid.distance_to(craton_edge, edge_scale=rough)
    d_in = np.where(craton, d_edge, 0.0)
    d_out = np.where(~craton, d_edge, 0.0)

    # Two-tier domain warp. The strong low-frequency warp (W) sculpts organic
    # continental-scale forms, but its Jacobian shears fine noise into long
    # parallel grooves that share one orientation across whole continents —
    # rivers then all follow the same diagonal. Valley-scale texture therefore
    # uses a much gentler warp (Wf) so it stays isotropic.
    W = noise.warp(grid.xyz, amp=cfg.noise_warp, freq=1.5, seed=cfg.seed * 37 + 11)
    Wf = noise.warp(grid.xyz, amp=0.06, freq=3.5, seed=cfg.seed * 97 + 53)
    base_int = 0.45 + 0.30 * noise.fbm(W, octaves=2, freq=0.9, seed=cfg.seed * 41 + 13) \
        + 0.16 * noise.fbm(Wf, octaves=3, freq=3.6, seed=cfg.seed * 41 + 14)
    taper = _smoothstep(d_in / cfg.shelf_taper_km)
    elev_cont = base_int - cfg.shelf_edge_drop_km * (1.0 - taper)
    slope = _smoothstep(d_out / 300.0)
    elev_ocean = (-0.9) * (1.0 - slope) + depth_age * slope
    elev = np.where(craton, elev_cont, elev_ocean)
    snap(elev, "1 crust platform + seafloor age  (sea level not yet solved)")
    if rec is not None:
        rec.frame("tectonics-plates", (sub % 20).astype(float), vmin=0, vmax=19,
                  cmap="tab20", label="plates and subplates")

    # ---- convergent features ---------------------------------------------------
    w_cc = cfg.orogen_width_km * (0.65 + 0.5 * np.clip(s_conv, 0, 1))
    oro = cfg.orogen_amp_km * s_conv * np.exp(-((d_conv / w_cc) ** 2)) * cls_cc[sc]
    plateau = 0.30 * cfg.orogen_amp_km * s_conv * np.exp(-((d_conv / (w_cc * 2.3)) ** 2)) * cls_cc[sc]

    trench = -cfg.trench_amp_km * s_conv * np.exp(-((d_conv / cfg.trench_width_km) ** 2)) \
        * (cls_oc[sc] | cls_oo[sc]) * (~on_over) * (~craton)
    arc_oc = cfg.arc_amp_km * s_conv \
        * np.exp(-(((d_conv - cfg.arc_offset_km) / cfg.arc_width_km) ** 2)) \
        * cls_oc[sc] * on_over * craton
    arc_oc = arc_oc + 0.25 * cfg.arc_amp_km * s_conv * np.exp(-((d_conv / (cfg.arc_width_km * 3.5)) ** 2)) \
        * cls_oc[sc] * on_over * craton
    chain_noise = 0.35 + 0.9 * noise.ridged(Wf, octaves=4, freq=6.0, seed=cfg.seed * 43 + 17)
    arc_oo = cfg.island_arc_amp_km * s_conv * chain_noise \
        * np.exp(-(((d_conv - 140.0) / cfg.island_arc_width_km) ** 2)) \
        * cls_oo[sc] * on_over * (~craton)

    # ---- divergent + transform features ------------------------------------------
    s_div = np.clip(strength[sd] / cfg.conv_rate_norm, 0.0, 1.5)
    rift_src = (btype == DIV) & (me_c | oth_c)
    rift = (-cfg.rift_amp_km * np.exp(-((d_div / cfg.rift_width_km) ** 2))
            + 0.35 * cfg.rift_amp_km * np.exp(-(((d_div - 2.2 * cfg.rift_width_km) / cfg.rift_width_km) ** 2))
            ) * s_div * rift_src[sd] * craton
    axial = -0.35 * np.exp(-((d_div / 30.0) ** 2)) * ridge_src[sd] * (~craton)

    s_tr = np.clip(strength[st] / cfg.conv_rate_norm, 0.0, 1.2)
    rough_tr = cfg.transform_amp_km * s_tr * np.exp(-((d_trans / 70.0) ** 2)) \
        * noise.fbm(Wf, octaves=4, freq=7.0, seed=cfg.seed * 47 + 19)

    # soft-saturate stacked convergent relief so collisions can't pile up
    # implausible peaks (tanh keeps small values linear, compresses the top)
    conv_pos = oro + plateau + arc_oc + arc_oo
    conv_pos = 5.6 * np.tanh(conv_pos / 5.6)
    trench = -6.2 * np.tanh(-trench / 6.2)
    elev = elev + conv_pos + trench + rift + axial + rough_tr
    snap(elev, "2 boundary features: orogens, trenches, arcs, rifts")

    # ---- deep history: eroded ancient orogens on today's continents ---------------
    era_fields = []
    era_meta = []
    for k in range(cfg.n_eras):
        rng_e = np.random.default_rng([cfg.seed, 100 + k])
        n_e = int(rng_e.integers(6, 10))
        e_plate, _, e_poles = _plate_layout(
            grid, rng_e, n_e, cfg.plate_warp_amp, cfg.plate_warp_freq * 0.9,
            warp_seed=cfg.seed * 53 + 900 + 71 * k)
        e_omega = rng_e.uniform(lo, hi, n_e) * rng_e.choice([-1, 1], n_e)
        eve = np.zeros(N)
        evn = np.zeros(N)
        for p in range(n_e):
            m = e_plate == p
            e_, n_ = _euler_velocity(grid, e_poles[p], e_omega[p])
            eve[m], evn[m] = e_[m], n_[m]
        eb = _classify_boundaries(grid, e_plate, eve, evn)
        d_e, l_e = grid.nearest_source(eb["btype"] == CONV, edge_scale=rough)
        s_e = np.clip(eb["strength"][_safe_lab(l_e)] / cfg.conv_rate_norm, 0.0, 1.3)
        w_e = cfg.orogen_width_km * cfg.era_width_factors[k] * (0.7 + 0.4 * s_e)
        O = s_e * np.exp(-((d_e / w_e) ** 2))
        mod = 0.55 + 0.5 * noise.fbm(Wf, octaves=4, freq=1.7, seed=cfg.seed * 59 + 900 + k)
        field = cfg.orogen_amp_km * cfg.era_amp_factors[k] * O * mod * craton
        era_fields.append(field)
        elev = elev + field
        era_meta.append(dict(index=k, n_plates=n_e))
        snap(elev, f"3 ancient orogens of era {k + 1}")

    # ---- hotspot chains ---------------------------------------------------------
    kd = grid.kdtree()
    hotspot_chains = []
    n_hot = cfg.n_hotspots
    hot_cells = rng.choice(N, size=n_hot * 3, replace=False)
    hot_cells = np.concatenate([hot_cells[~craton[hot_cells]][: int(n_hot * 0.75)],
                                hot_cells[craton[hot_cells]][: n_hot - int(n_hot * 0.75)]])
    for h in hot_cells[:n_hot]:
        p0 = grid.xyz[h]
        s_id = sub[h]
        pole, om = sub_pole[s_id], sub_omega[s_id]
        on_land = bool(craton[h])
        chain = []
        steps = 3 if on_land else cfg.hotspot_chain_steps
        for t in range(steps):
            pos = _rotate_points(p0[None, :], pole, om * t * cfg.hotspot_step_myr)[0]
            amp = (cfg.hotspot_amp_km if not on_land else 1.7) * (0.88 ** t)
            width = cfg.hotspot_width_km if not on_land else 210.0
            radius_chord = min(3.5 * width / R, 1.0)
            cells = kd.query_ball_point(pos, r=radius_chord)
            cells = np.asarray(cells, dtype=np.int64)
            if len(cells) == 0:
                continue
            d = R * np.arccos(np.clip(grid.xyz[cells] @ pos, -1, 1))
            bump_mod = 0.7 + 0.6 * rng.random()
            elev[cells] += amp * bump_mod * np.exp(-((d / width) ** 2))
            chain.append(dict(cell=int(cells[d.argmin()]), amp=float(amp * bump_mod)))
        hotspot_chains.append(dict(head=int(h), ocean=not on_land, chain=chain))
    snap(elev, "4 hotspot volcano chains")

    # ---- tectonic-context noise ---------------------------------------------------
    env_mountain = np.clip((oro + arc_oc + 0.6 * arc_oo) / 2.2, 0.0, 1.0) \
        + 0.35 * sum(era_fields) / max(cfg.orogen_amp_km * cfg.era_amp_factors[0], 0.1)
    env_mountain = np.clip(env_mountain, 0.0, 1.2)
    craton_soft = _smoothstep(np.where(craton, d_in, -d_out) / 150.0 + 0.5)

    nb = cfg.noise_base_freq
    rmount = noise.ridged(Wf, octaves=6, freq=nb * 2.0, seed=cfg.seed * 61 + 23)
    # fine ridge-and-valley structure (~100-500 km wavelengths): this is what
    # makes orogens look dissected instead of like smooth welts, and it seeds
    # the valleys erosion will deepen
    rfine = noise.ridged(Wf, octaves=4, freq=nb * 6.0, seed=cfg.seed * 101 + 59)
    elev += cfg.noise_mountain_amp * (rmount - 0.42) * env_mountain
    elev += 0.6 * cfg.noise_mountain_amp * (rfine - 0.5) * env_mountain
    elev += cfg.noise_hill_amp * noise.fbm(Wf, octaves=6, freq=nb, seed=cfg.seed * 67 + 29) * craton_soft
    elev += cfg.noise_abyssal_amp * noise.fbm(Wf, octaves=5, freq=nb * 1.5, seed=cfg.seed * 71 + 31) * (1.0 - craton_soft)
    elev += cfg.noise_detail_amp * noise.fbm(grid.xyz, octaves=4, freq=nb * 4.0, seed=cfg.seed * 73 + 37)
    snap(elev, "5 fractal terrain detail")

    # ---- sea level: flood the target fraction of surface area ----------------------
    order_e = np.argsort(elev)
    cum = np.cumsum(grid.area_km2[order_e])
    k_idx = np.searchsorted(cum, cfg.ocean_fraction * total_area)
    sea = elev[order_e[min(k_idx, N - 1)]]
    elev = elev - sea
    # gentle compression of the extremes toward Earth-like limits
    hi = elev > 6.0
    elev[hi] = 6.0 + 2.6 * np.tanh((elev[hi] - 6.0) / 2.6)
    lo_m = elev < -9.0
    elev[lo_m] = -9.0 - 2.2 * np.tanh((-elev[lo_m] - 9.0) / 2.2)
    is_ocean = elev < 0.0
    snap(elev, "6 sea level solved: the world takes shape")

    # ---- auxiliary outputs -----------------------------------------------------------
    # ridge-noise modulation so continued uplift builds ridge-and-valley
    # structure for erosion to carve, not a smooth welt (two scales: massif
    # and dissection-scale ridges)
    uplift = np.clip((oro + 1.2 * arc_oc + arc_oo) / cfg.orogen_amp_km, 0.0, 1.2) \
        * (0.30 + 0.7 * rmount + 0.5 * rfine)
    # sustained subsidence keeps rift basins and cratonic sags from silting up
    # during the erosion stage (this is where big lakes and inland seas live)
    rift_sub = s_div * np.exp(-((d_div / (cfg.rift_width_km * 1.4)) ** 2)) * rift_src[sd] * craton
    sag = _smoothstep((noise.fbm(Wf, octaves=4, freq=1.6, seed=cfg.seed * 83 + 43) - 0.42) / 0.18) \
        * _smoothstep(d_in / 500.0)
    subsidence = np.clip(rift_sub + 0.55 * sag, 0.0, 1.2)
    hardness = 0.8 + 0.45 * noise.fbm(Wf, octaves=4, freq=3.0, seed=cfg.seed * 79 + 41) \
        + 0.25 * noise.fbm(Wf, octaves=3, freq=12.0, seed=cfg.seed * 103 + 61) \
        + 0.3 * taper * craton
    erodibility = np.clip(1.0 / np.maximum(hardness, 0.3), 0.4, 2.5)
    volcanic = (arc_oc + arc_oo > 0.4) | (np.abs(rift) > 0.4)

    plate_table = []
    for p in range(cfg.n_plates):
        m = plate == p
        plate_table.append(dict(
            id=p, area_km2=float(plate_areas[p]), continental=bool(continental[p]),
            omega_deg_myr=float(np.degrees(omegas[p])),
            speed_km_myr=float(np.hypot(ve[m], vn[m]).mean()),
            centroid_cell=int(np.flatnonzero(m)[np.argmax(bdist_plate[m])])))

    return dict(
        elev=elev, plate=plate, sub=sub, craton=craton, is_ocean=is_ocean,
        v_e=ve, v_n=vn, btype=btype, bstrength=strength,
        seafloor_age=np.where(craton, 0.0, age), uplift=uplift,
        subsidence=subsidence, erodibility=erodibility, volcanic=volcanic,
        plate_table=plate_table, era_meta=era_meta, hotspots=hotspot_chains,
        orogen_field=oro + plateau, era_field=sum(era_fields))

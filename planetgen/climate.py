"""Stage 2: climate — insolation, temperature, winds, ocean currents, rainfall.

A stylized (not full-physics) atmosphere producing 12 monthly snapshots:

  * analytic daily-mean insolation from axial tilt
  * energy-balance temperatures: annual structure from annual insolation,
    seasonal swing damped by "maritime influence" that is advected inland by
    the prevailing winds (west coasts get mild, interiors get continental)
  * winds = seasonally migrating circulation bands (trades / westerlies /
    polar easterlies, shifting ITCZ) + a thermal component blowing toward
    warm anomalies with Coriolis-style deflection -> monsoons emerge
  * ocean currents relaxed from wind stress with coastline steering; sea
    surface temperature is advected by them (warm/cold boundary currents)
  * moisture: evaporation -> upwind advection -> rain from convection,
    orographic lift (rain shadows), mid-latitude fronts, and saturation
    capping; precipitation is calibrated to a target land-mean afterwards
"""

from __future__ import annotations

import numpy as np


def _q_sat(t_c):
    """Relative moisture-holding capacity (Clausius-Clapeyron-ish shape)."""
    return np.exp(0.062 * t_c)


def _rot2(ve, vn, ang):
    c, s = np.cos(ang), np.sin(ang)
    return c * ve - s * vn, s * ve + c * vn


def _band_winds(lat_deg, m, cfg, itcz_scale=1.0):
    """Analytic circulation bands for month m: trades, westerlies, polar
    easterlies, meridional flow toward the (seasonally shifted) ITCZ.

    itcz_scale (scalar or per-cell array) multiplies the seasonal migration —
    the rain belt wanders much farther over land, where the surface heats
    and cools quickly, than over the thermally sluggish ocean.
    """
    seas = np.sin(2 * np.pi * (m - 2.75) / 12.0)
    itcz = cfg.itcz_amplitude_deg * seas * itcz_scale
    phi = lat_deg - itcz * np.exp(-((lat_deg / 35.0) ** 2))
    a = np.abs(phi)
    u = (-cfg.trade_wind_ms * np.exp(-(((a - 12.0) / 13.0) ** 2))
         + cfg.westerlies_ms * np.exp(-(((a - 47.0) / 16.0) ** 2))
         - cfg.polar_easterlies_ms * np.exp(-(((a - 76.0) / 12.0) ** 2)))
    v = (-np.sign(phi) * 3.4 * np.exp(-(((a - 8.0) / 14.0) ** 2))
         + np.sign(phi) * 1.3 * np.exp(-(((a - 45.0) / 13.0) ** 2))
         - np.sign(phi) * 0.9 * np.exp(-(((a - 72.0) / 10.0) ** 2)))
    return u, v


def _band_convergence(lat_deg, m, cfg, radius_km, itcz_scale=1.0):
    """Horizontal convergence (1/s) of the band circulation, per cell.

    Zonal flow that only varies with latitude is divergence-free, so
    div = d(v cos(lat))/dlat / (R cos(lat)), evaluated by central difference
    in latitude with each cell's own ITCZ scale. Analytic evaluation avoids
    the lattice bias the graph divergence operator would imprint on rain.
    """
    d = 0.25
    _, vp = _band_winds(lat_deg + d, m, cfg, itcz_scale)
    _, vm = _band_winds(lat_deg - d, m, cfg, itcz_scale)
    cosp = np.cos(np.radians(lat_deg + d))
    cosm = np.cos(np.radians(lat_deg - d))
    cosl = np.maximum(np.cos(np.radians(lat_deg)), 0.05)
    div = (vp * cosp - vm * cosm) / 1000.0 / (2 * np.radians(d)) / (radius_km * cosl)
    return -div   # positive = convergence


def _itcz_land_scale(grid, cfg, is_ocean, n_bins=72, smooth=6):
    """Per-cell ITCZ migration multiplier from the tropical land fraction at
    each longitude (circularly smoothed): 1 over all-ocean longitudes, up to
    1 + itcz_land_boost where the tropics are solid land.

    Interpolated between bin centers — assigning bin values directly makes a
    5-degree longitude staircase that stripes the rain band vertically in
    the solstice months (when the seasonal displacement it scales is largest).
    """
    tropics = np.abs(np.degrees(grid.lat)) < 25.0
    lon_bin = np.clip(((grid.lon + np.pi) / (2 * np.pi) * n_bins).astype(int), 0, n_bins - 1)
    w = grid.area_km2 * tropics
    land_a = np.bincount(lon_bin, weights=w * ~is_ocean, minlength=n_bins)
    tot_a = np.bincount(lon_bin, weights=w, minlength=n_bins)
    frac = land_a / np.maximum(tot_a, 1e-9)
    for _ in range(smooth):
        frac = 0.25 * np.roll(frac, 1) + 0.5 * frac + 0.25 * np.roll(frac, -1)
    centers = (np.arange(n_bins) + 0.5) / n_bins * 2 * np.pi - np.pi
    c_pad = np.concatenate([centers[-1:] - 2 * np.pi, centers, centers[:1] + 2 * np.pi])
    f_pad = np.concatenate([frac[-1:], frac, frac[:1]])
    return 1.0 + cfg.itcz_land_boost * np.interp(grid.lon, c_pad, f_pad)


def _insolation(lat, tilt_deg, months):
    """Daily-mean top-of-atmosphere insolation, (months, n), arbitrary units."""
    out = np.zeros((months, len(lat)))
    for m in range(months):
        day = 15.2 + 30.44 * m
        dec = np.radians(-tilt_deg * np.cos(2 * np.pi * (day + 10) / 365.25))
        cos_h0 = np.clip(-np.tan(lat) * np.tan(dec), -1.0, 1.0)
        h0 = np.arccos(cos_h0)
        out[m] = (h0 * np.sin(lat) * np.sin(dec)
                  + np.cos(lat) * np.cos(dec) * np.sin(h0)) / np.pi
    return out


def simulate(grid, cfg, elev, is_ocean, lake_mask=None, rec=None, tag="climate"):
    N = grid.n
    months = cfg.months
    lat_deg = np.degrees(grid.lat)
    water = is_ocean if lake_mask is None else (is_ocean | lake_mask)
    land_elev = np.maximum(elev, 0.0)
    if rec is not None:
        rec.set_coast(is_ocean)

    # ---- insolation and radiative temperature structure ----------------------
    Q = _insolation(grid.lat, cfg.axial_tilt_deg, months)
    Q_ann = Q.mean(0)
    # annual structure: cos-power latitude profiles — insolation shape with the
    # flattening real meridional heat transport produces (Earth zonal fits).
    # The ocean gets its own, flatter curve: currents carry so much heat
    # poleward that open water hovers near freezing even at high latitude.
    t_land = cfg.solar_temp_pole_c + (cfg.solar_temp_eq_c - cfg.solar_temp_pole_c) \
        * np.cos(grid.lat) ** 1.35
    t_sea = cfg.sst_pole_c + (cfg.solar_temp_eq_c - cfg.sst_pole_c) \
        * np.cos(grid.lat) ** 1.3
    T_ann_sea = np.where(water, t_sea, t_land)
    dQ = Q - Q_ann[None, :]
    swing = 46.0 * dQ / max(np.abs(dQ).max(), 1e-9)     # instant "continental" seasonal response, degC

    # ---- seasonal circulation bands -------------------------------------------
    itcz_scale = _itcz_land_scale(grid, cfg, is_ocean)
    wind_e = np.zeros((months, N))
    wind_n = np.zeros((months, N))
    for m in range(months):
        wind_e[m], wind_n[m] = _band_winds(lat_deg, m, cfg, itcz_scale)

    # ---- maritime influence, advected inland by annual winds -------------------
    base_ann = np.stack([wind_e.mean(0), wind_n.mean(0)], 1)
    M = np.where(water, 1.0, 0.0)
    spd = np.hypot(base_ann[:, 0], base_ann[:, 1]) + 1e-6
    # per-edge decay: shorter reach against the wind
    to_me_e = -grid.dir_e  # direction j -> i expressed in i's frame (approx)
    to_me_n = -grid.dir_n
    align = (base_ann[grid.nbr_safe, 0] * to_me_e + base_ann[grid.nbr_safe, 1] * to_me_n) \
        / spd[grid.nbr_safe]
    reach = cfg.maritime_range_km * (0.30 + 0.70 * np.clip(align, 0.0, 1.0))
    decay = np.where(grid.nbr_ok, np.exp(-grid.edge_km / np.maximum(reach, 60.0)), 0.0)
    hops = int(3.5 * cfg.maritime_range_km / grid.mean_edge_km) + 10
    for _ in range(hops):
        cand = M[grid.nbr_safe] * decay
        best = cand.max(1)
        upd = ~water & (best > M)
        if not upd.any():
            break
        M[upd] = best[upd]

    # ---- provisional temperatures (no ocean-current anomalies yet) -------------
    resp = cfg.ocean_seasonal_damp + (1.0 - M) * (cfg.continental_seasonal_boost - cfg.ocean_seasonal_damp)
    T = np.zeros((months, N))
    for m in range(months):
        sw_ocean = swing[(m - 1) % months]           # ocean lags a month
        sw = (1.0 - M) * swing[m] + M * sw_ocean
        T[m] = T_ann_sea + resp * sw
        for _ in range(cfg.heat_diffusion_steps):
            T[m] += 0.5 * grid.laplacian(T[m])
        T[m] -= cfg.lapse_rate_c_km * land_elev

    # ---- thermal (monsoon) wind component ----------------------------------------
    grad_elev = grid.gradient(elev)
    base_e = wind_e.copy()
    base_n = wind_n.copy()
    for m in range(months):
        anom = T[m] - grid.zonal_mean(T[m])
        for _ in range(3):
            anom += 0.5 * grid.laplacian(anom)
        g = grid.gradient(anom)                       # points toward warm anomalies
        ang = np.radians(-58.0) * np.sign(grid.lat)
        ge, gn = _rot2(g[:, 0], g[:, 1], ang)
        mag = np.hypot(ge, gn)
        scale = cfg.thermal_wind_ms / max(np.percentile(mag, 92), 1e-9)
        cap = 2.0 * cfg.thermal_wind_ms
        norm = np.minimum(mag * scale, cap) / np.maximum(mag, 1e-12)
        wind_e[m] += ge * norm
        wind_n[m] += gn * norm
        damp = 1.0 / (1.0 + 0.22 * land_elev)         # high terrain blocks flow
        wind_e[m] *= damp
        wind_n[m] *= damp

    # ---- ocean currents (annual) ----------------------------------------------------
    tau = np.stack([wind_e.mean(0), wind_n.mean(0)], 1)
    ang0 = np.radians(-25.0) * np.sign(grid.lat)
    ce, cn = _rot2(tau[:, 0], tau[:, 1], ang0)
    cur = np.stack([ce, cn], 1) * 0.12 * cfg.current_speed
    cur[~is_ocean] = 0.0
    # coastal normal (points from ocean cell toward adjacent land)
    landn = grid.nbr_ok & ~is_ocean[grid.nbr_safe]
    cne = (grid.dir_e * landn).sum(1)
    cnn = (grid.dir_n * landn).sum(1)
    cmag = np.hypot(cne, cnn)
    coastal = is_ocean & (cmag > 1e-9)
    cne = np.where(coastal, cne / np.maximum(cmag, 1e-9), 0.0)
    cnn = np.where(coastal, cnn / np.maximum(cmag, 1e-9), 0.0)
    ocean_nbr = grid.nbr_ok & is_ocean[grid.nbr_safe]
    n_on = np.maximum(ocean_nbr.sum(1), 1)
    for it in range(cfg.current_iters):
        # smooth within the ocean, keep wind forcing, steer along coasts
        avg_e = np.where(ocean_nbr, cur[grid.nbr_safe, 0], 0.0).sum(1) / n_on
        avg_n = np.where(ocean_nbr, cur[grid.nbr_safe, 1], 0.0).sum(1) / n_on
        cur[:, 0] = 0.55 * cur[:, 0] + 0.45 * avg_e + 0.06 * cfg.current_speed * (ce - cur[:, 0])
        cur[:, 1] = 0.55 * cur[:, 1] + 0.45 * avg_n + 0.06 * cfg.current_speed * (cn - cur[:, 1])
        normal = cur[:, 0] * cne + cur[:, 1] * cnn
        cur[:, 0] -= normal * cne
        cur[:, 1] -= normal * cnn
        if it % 5 == 4:                               # mild divergence damping
            div = grid.divergence(cur)
            for _ in range(2):
                div += 0.5 * grid.laplacian(div)
            gd = grid.gradient(div)
            cur[:, 0] -= 900.0 * gd[:, 0]
            cur[:, 1] -= 900.0 * gd[:, 1]
        cur[~is_ocean] = 0.0
        if rec is not None and it % 5 == 0:
            rec.frame(f"{tag}-currents", np.where(is_ocean, np.hypot(cur[:, 0], cur[:, 1]), 0.0),
                      label=f"{tag}: current relaxation step {it + 1}", vmin=0, vmax=9, cmap="magma")
    spd_c = np.hypot(cur[:, 0], cur[:, 1])
    cap_c = np.percentile(spd_c[is_ocean], 98)
    over = spd_c > cap_c
    cur[over] *= (cap_c / spd_c[over])[:, None]

    # ---- SST: radiative structure + current advection -------------------------------
    sst = np.zeros((months, N))
    for m in range(months):
        s0 = T[m].copy()
        s = s0.copy()
        for _ in range(cfg.sst_advect_iters):
            s = grid.advect_interp(s, cur, frac=0.5)
            s = np.where(is_ocean, 0.96 * s + 0.04 * s0, s0)
        sst[m] = np.where(is_ocean, s, np.nan)

    # ---- final temperature: coastal cells inherit advected SST anomalies -------------
    V = np.zeros(N)
    for m in range(months):
        # propagate SST inland along the same maritime decay, then blend by M
        V[:] = np.where(is_ocean, sst[m], 0.0)
        Mv = np.where(water, 1.0, 0.0)
        for _ in range(min(hops, 45)):
            cand = Mv[grid.nbr_safe] * decay
            slot = cand.argmax(1)
            best = cand[np.arange(N), slot]
            upd = ~water & (best > Mv)
            if not upd.any():
                break
            Mv[upd] = best[upd]
            V[upd] = V[grid.nbr_safe[upd, slot[upd]]]
        blend = np.clip(M, 0.0, 1.0) * 0.82
        t_maritime = V - cfg.lapse_rate_c_km * land_elev
        T[m] = np.where(is_ocean, sst[m],
                        (1.0 - blend) * T[m] + blend * t_maritime)
        T[m] += 0.5 * grid.laplacian(T[m])
        if rec is not None:
            rec.frame(f"{tag}-temperature", T[m], label=f"{tag}: temperature month {m + 1:02d}",
                      vmin=-45, vmax=45, cmap="RdYlBu_r")

    # ---- moisture and precipitation ------------------------------------------------
    # The seasonal phase advances *continuously inside* each month's loop
    # (band winds are analytic, so they evaluate at fractional months) — a
    # rain belt parked at 12 discrete monthly positions would stripe the
    # annual precipitation map.
    P = np.zeros((months, N))
    clouds = np.zeros((months, N))
    q = np.where(water, 0.4 * _q_sat(T[0]), 0.05)
    wet = np.full(N, 0.5)
    lat_a = np.abs(lat_deg)
    front_band = np.exp(-(((lat_a - 48.0) / 20.0) ** 2))
    grad_land = grid.gradient(land_elev)               # orographic lift is a land effect
    damp = 1.0 / (1.0 + 0.22 * land_elev)
    # per-month damped thermal (monsoon) winds and their smoothed divergence;
    # the graph operator only ever touches this residual (units: 1/s)
    th_e = wind_e - base_e * damp[None, :]
    th_n = wind_n - base_n * damp[None, :]
    div_t = np.zeros((months, N))
    for m in range(months):
        d = grid.divergence(np.stack([th_e[m], th_n[m]], 1)) * 1e-3
        for _ in range(8):
            d += 0.5 * grid.laplacian(d)
        div_t[m] = d
    def _month_fields(mi):
        """Evaporation, saturation, convective-warmth, and SST-anomaly fields
        at month mi. These are interpolated per step inside the loop — holding
        them fixed within a month and jumping at boundaries pulses the rain."""
        qs_m = _q_sat(T[mi])
        sst_eff = np.where(is_ocean, sst[mi], T[mi])
        E_m = np.where(water, cfg.evap_ocean * _q_sat(sst_eff) / _q_sat(30.0),
                       cfg.evap_land_factor * wet * qs_m / _q_sat(30.0))
        E_m = np.minimum(E_m, 3.0)
        E_m = np.where(water & (T[mi] < -2.0), E_m * 0.15, E_m)  # frozen seas evaporate little
        warm_m = np.clip((T[mi] - 8.0) / 22.0, 0.0, 1.0) ** 2
        # ocean convection responds to how warm the sea is *for its latitude*:
        # the rain band flares over warm pools and breaks over cold tongues
        anom = np.where(is_ocean, np.nan_to_num(sst[mi], nan=0.0), 0.0)
        anom = anom - grid.zonal_mean(anom)
        sstf_m = np.where(is_ocean,
                          np.clip(1.0 + cfg.sst_itcz_coupling * anom / 3.5, 0.3, 2.2),
                          1.0)
        return qs_m, E_m, warm_m, sstf_m

    for m in range(months):
        m2 = (m + 1) % months
        qs, E0, warm0, sstf0 = _month_fields(m)
        qs2, E1, warm1, sstf1 = _month_fields(m2)
        p_sum = np.zeros(N)
        for i in range(cfg.moisture_iters):
            a = (i + 0.5) / cfg.moisture_iters
            qs_f = (1.0 - a) * qs + a * qs2
            E = (1.0 - a) * E0 + a * E1
            warm = (1.0 - a) * warm0 + a * warm1
            sstf = (1.0 - a) * sstf0 + a * sstf1
            ue, un = _band_winds(lat_deg, m + a, cfg, itcz_scale)
            we = ue * damp + (1.0 - a) * th_e[m] + a * th_e[m2]
            wn = un * damp + (1.0 - a) * th_n[m] + a * th_n[m2]
            W2 = np.stack([we, wn], 1)
            conv = _band_convergence(lat_deg, m + a, cfg, grid.radius_km, itcz_scale) * damp \
                - ((1.0 - a) * div_t[m] + a * div_t[m2])
            conv_lift = np.clip(conv / max(np.percentile(np.abs(conv), 90), 1e-12), 0.0, 2.5)
            orog = np.clip(we * grad_land[:, 0] + wn * grad_land[:, 1], 0.0, None)
            orog = np.where(is_ocean, 0.0, orog)
            rate = (cfg.rain_convective * warm * (0.35 + conv_lift) * sstf
                    + cfg.rain_orographic * orog
                    + cfg.rain_frontal * front_band * np.hypot(we, wn) / 10.0
                    + cfg.rain_base)
            p_now = q * np.clip(rate, 0.0, 0.4) + 0.12 * np.maximum(q - qs_f, 0.0)
            q = np.maximum(q + 0.15 * E - p_now, 0.0)
            q = grid.advect(grid.advect(q, W2, frac=0.9), W2, frac=0.9)
            if cfg.moisture_diffusion > 0.0:
                # isotropic smoothing counters the upwind scheme's directional
                # diffusion, which otherwise draws streaks along the trades
                # that meet in chevrons at the ITCZ
                q += cfg.moisture_diffusion * grid.laplacian(q)
            p_sum += p_now
            if rec is not None:
                lbl = f"{tag}: month {m + 1:02d} step {i + 1:03d}"
                rec.frame(f"{tag}-moisture", q, label=lbl + "  (column moisture)",
                          vmin=0.0, vmax=4.5, cmap="YlGnBu", every=rec.every)
                rec.frame(f"{tag}-rainfall", p_now, label=lbl + "  (rain rate)",
                          vmin=0.0, vmax=0.22, cmap="YlGnBu", every=rec.every)
        P[m] = p_sum / cfg.moisture_iters
        wet = np.clip(0.5 * wet + 0.5 * np.minimum(P[m] / np.percentile(P[m][~water], 75), 1.5), 0.1, 1.5)
        cl = np.clip(0.10 + 0.55 * q / np.maximum(qs, 1e-9) + 0.5 * P[m] / max(P[m].mean() * 4, 1e-9), 0.0, 1.0)
        for _ in range(3):                             # cosmetic: hide grid seams
            cl += 0.5 * grid.laplacian(cl)
        clouds[m] = np.clip(cl, 0.0, 1.0)

    # calibrate rainfall so land annual mean hits the target
    land = ~is_ocean
    ann_raw = P.sum(0)
    factor = cfg.target_land_precip_mm / max((ann_raw[land] * grid.area_km2[land]).sum()
                                             / grid.area_km2[land].sum(), 1e-9)
    P *= factor
    P = np.minimum(P, 1300.0)                          # winsorize pathological peaks (mm/month)

    snow = np.where(T < 0.5, P, 0.0)
    seaice = is_ocean[None, :] & (np.nan_to_num(sst, nan=99.0) < -1.8)

    return dict(
        T=T, P=P, wind_e=wind_e, wind_n=wind_n, clouds=clouds, snow=snow,
        sst=sst, seaice=seaice, cur_e=cur[:, 0], cur_n=cur[:, 1], maritime=M,
        T_ann=T.mean(0), P_ann=P.sum(0),
        pet_ann=np.sum(38.0 * np.exp(0.055 * np.maximum(T, -5.0)), axis=0))

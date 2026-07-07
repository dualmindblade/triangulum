"""Stage 5: biome classification.

Full Koppen-Geiger classification (Peel et al. 2007 rules) from the monthly
temperature and precipitation fields, plus game-facing layers: vegetation
density (Miami-model net primary productivity), soil fertility, and
permafrost. Ocean cells get shallow/deep/ice classes for rendering.
"""

from __future__ import annotations

import numpy as np

# class id -> (code, description, map color)
KOPPEN = [
    ("Af", "tropical rainforest", "#0000fe"),
    ("Am", "tropical monsoon", "#0078ff"),
    ("Aw", "tropical savanna", "#46aafa"),
    ("BWh", "hot desert", "#ff0000"),
    ("BWk", "cold desert", "#ff9696"),
    ("BSh", "hot steppe", "#f5a500"),
    ("BSk", "cold steppe", "#ffdc64"),
    ("Csa", "hot-summer mediterranean", "#ffff00"),
    ("Csb", "warm-summer mediterranean", "#c8c800"),
    ("Csc", "cold-summer mediterranean", "#969600"),
    ("Cwa", "monsoon humid subtropical", "#96ff96"),
    ("Cwb", "subtropical highland", "#64c864"),
    ("Cwc", "cold subtropical highland", "#329632"),
    ("Cfa", "humid subtropical", "#c8ff50"),
    ("Cfb", "oceanic", "#64ff50"),
    ("Cfc", "subpolar oceanic", "#32c800"),
    ("Dsa", "hot dry-summer continental", "#ff00fe"),
    ("Dsb", "warm dry-summer continental", "#c800c8"),
    ("Dsc", "subarctic dry-summer", "#963296"),
    ("Dsd", "frigid dry-summer", "#966496"),
    ("Dwa", "monsoon hot continental", "#abb1ff"),
    ("Dwb", "monsoon warm continental", "#5a77db"),
    ("Dwc", "monsoon subarctic", "#4b50b4"),
    ("Dwd", "monsoon frigid subarctic", "#320087"),
    ("Dfa", "hot-summer continental", "#00ffff"),
    ("Dfb", "warm-summer continental", "#37c8ff"),
    ("Dfc", "subarctic taiga", "#007d7d"),
    ("Dfd", "frigid subarctic", "#00465f"),
    ("ET", "tundra", "#b2b2b2"),
    ("EF", "ice cap", "#686868"),
]
CODE_INDEX = {code: i for i, (code, _, _) in enumerate(KOPPEN)}


def classify(grid, cfg, T, P, is_ocean):
    """Koppen class per land cell (-1 ocean). T (12, n) degC, P (12, n) mm/month."""
    n = grid.n
    north = grid.lat >= 0
    # summer = Apr..Sep in the north, Oct..Mar in the south
    summer_months = np.zeros((12, n), bool)
    for m in range(12):
        summer_months[m] = north if 3 <= m <= 8 else ~north

    T_ann = T.mean(0)
    P_ann = P.sum(0)
    T_hot = T.max(0)
    T_cold = T.min(0)
    P_dry = P.min(0)
    Ps = np.where(summer_months, P, 0.0).sum(0)          # summer precip
    Pw = P_ann - Ps
    P_sdry = np.where(summer_months, P, np.inf).min(0)
    P_swet = np.where(summer_months, P, -np.inf).max(0)
    P_wdry = np.where(~summer_months, P, np.inf).min(0)
    P_wwet = np.where(~summer_months, P, -np.inf).max(0)
    months_above_10 = (T >= 10.0).sum(0)

    # arid threshold
    frac_summer = Ps / np.maximum(P_ann, 1e-9)
    pth = 20.0 * T_ann + np.where(frac_summer >= 0.7, 280.0,
                                  np.where(frac_summer >= 0.3, 140.0, 0.0))
    pth = np.maximum(pth, 10.0)

    out = np.full(n, -1, np.int64)
    land = ~is_ocean

    is_B = land & (P_ann < pth) & (T_hot > 0)
    is_BW = is_B & (P_ann < 0.5 * pth)
    hot = T_ann >= 18.0
    out[is_BW & hot] = CODE_INDEX["BWh"]
    out[is_BW & ~hot] = CODE_INDEX["BWk"]
    is_BS = is_B & ~is_BW
    out[is_BS & hot] = CODE_INDEX["BSh"]
    out[is_BS & ~hot] = CODE_INDEX["BSk"]

    rest = land & ~is_B
    is_A = rest & (T_cold >= 18.0)
    af = is_A & (P_dry >= 60.0)
    am = is_A & ~af & (P_dry >= 100.0 - P_ann / 25.0)
    out[af] = CODE_INDEX["Af"]
    out[am] = CODE_INDEX["Am"]
    out[is_A & ~af & ~am] = CODE_INDEX["Aw"]

    is_C = rest & ~is_A & (T_hot > 10.0) & (T_cold > 0.0) & (T_cold < 18.0)
    is_D = rest & ~is_A & (T_hot > 10.0) & (T_cold <= 0.0)
    is_E = rest & ~is_A & (T_hot <= 10.0)

    def sw_flavor(mask):
        s = mask & (P_sdry < 40.0) & (P_sdry < P_wwet / 3.0)
        w = mask & ~s & (P_wdry < P_swet / 10.0)
        f = mask & ~s & ~w
        return s, w, f

    for group, prefix in ((is_C, "C"), (is_D, "D")):
        s, w, f = sw_flavor(group)
        for flav_mask, flav in ((s, "s"), (w, "w"), (f, "f")):
            a = flav_mask & (T_hot >= 22.0)
            b = flav_mask & ~a & (months_above_10 >= 4)
            if prefix == "D":
                d = flav_mask & ~a & ~b & (T_cold < -38.0)
                c = flav_mask & ~a & ~b & ~d
                for m_, suffix in ((a, "a"), (b, "b"), (c, "c"), (d, "d")):
                    key = prefix + flav + suffix
                    if key in CODE_INDEX:
                        out[m_] = CODE_INDEX[key]
            else:
                c = flav_mask & ~a & ~b
                for m_, suffix in ((a, "a"), (b, "b"), (c, "c")):
                    key = prefix + flav + suffix
                    if key in CODE_INDEX:
                        out[m_] = CODE_INDEX[key]

    out[is_E & (T_hot > 0.0)] = CODE_INDEX["ET"]
    out[is_E & (T_hot <= 0.0)] = CODE_INDEX["EF"]
    return out


def supplements(grid, cfg, T, P, koppen, soil, volcanic, river, flow_acc, is_ocean):
    """Game-facing layers derived from climate + hydrology."""
    T_ann = T.mean(0)
    P_ann = P.sum(0)
    npp_t = 3000.0 / (1.0 + np.exp(1.315 - 0.119 * T_ann))
    npp_p = 3000.0 * (1.0 - np.exp(-0.000664 * P_ann))
    npp = np.where(~is_ocean, np.minimum(npp_t, npp_p) / 3000.0, 0.0)

    flood_bonus = np.where(river, np.clip(np.log10(np.maximum(flow_acc, 1.0)) / 5.0, 0.0, 0.5), 0.0)
    fertility = np.clip(2.2 * soil + 0.5 * npp + 0.35 * volcanic + flood_bonus, 0.0, 1.0)
    fertility = np.where(is_ocean, 0.0, fertility)

    permafrost = ~is_ocean & (T_ann < -3.0)
    ice_cap = koppen == CODE_INDEX["EF"]
    return dict(vegetation=npp, fertility=fertility, permafrost=permafrost, ice_cap=ice_cap)

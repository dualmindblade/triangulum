"""Map rendering: equirectangular sheets, orthographic globes, HTML atlas.

Geographic conventions rule the color choices: hypsometric tints for relief,
a warm/cool diverging scale centered near freezing for temperature, a
single-ramp sequential for rainfall, standard Koppen-Geiger biome colors.
Categorical layers are drawn nearest-cell; continuous fields are IDW-smooth.
"""

from __future__ import annotations

import os

import numpy as np
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection
from matplotlib.colors import to_rgb
from matplotlib.patches import Patch

from .biomes import KOPPEN

OCEAN_STOPS = [(-11, (5, 15, 40)), (-6, (10, 30, 72)), (-4, (16, 48, 98)),
               (-2.5, (24, 65, 118)), (-1.2, (40, 88, 142)), (-0.4, (66, 116, 165)),
               (0.0, (118, 168, 200))]
LAND_STOPS = [(0.0, (76, 124, 72)), (0.35, (112, 152, 82)), (0.9, (174, 182, 102)),
              (1.6, (194, 164, 94)), (2.6, (162, 124, 78)), (3.6, (134, 98, 68)),
              (4.6, (144, 128, 118)), (5.6, (202, 202, 202)), (8.0, (250, 250, 250))]


def _ramp(vals, stops):
    xs = np.array([s[0] for s in stops], float)
    cols = np.array([s[1] for s in stops], float) / 255.0
    out = np.zeros(vals.shape + (3,))
    for c in range(3):
        out[..., c] = np.interp(vals, xs, cols[:, c])
    return out


def _hillshade(grid, elev, exaggeration=60.0):
    grad = grid.gradient(elev)
    nx, ny = -grad[:, 0] * exaggeration, -grad[:, 1] * exaggeration
    nrm = np.sqrt(nx ** 2 + ny ** 2 + 1.0)
    light = np.array([-0.55, 0.45, 0.65])
    light /= np.linalg.norm(light)
    return np.clip((nx * light[0] + ny * light[1] + light[2]) / nrm, 0.0, 1.0)


class Renderer:
    def __init__(self, grid, cfg, out_dir):
        self.grid = grid
        self.cfg = cfg
        self.dir = out_dir
        os.makedirs(out_dir, exist_ok=True)
        self.W = cfg.map_width
        self.H = cfg.map_width // 2
        self.ext = [-180, 180, -90, 90]
        self.sheets = []          # (filename, caption) for the atlas

    # ------------------------------------------------------------------
    def _relief_rgb(self, elev, is_ocean, lake_id=None, seaice_frac=None, land_ice=None):
        g = self.grid
        shade = _hillshade(g, elev)
        e_img = g.project(elev, self.W, self.H)
        o_img = g.project(is_ocean.astype(float), self.W, self.H) > 0.5
        sh = g.project(shade, self.W, self.H)
        rgb = np.where(o_img[..., None], _ramp(e_img, OCEAN_STOPS), _ramp(e_img, LAND_STOPS))
        light = np.where(o_img, 0.84 + 0.22 * sh, 0.46 + 0.72 * sh)
        rgb = np.clip(rgb * light[..., None], 0, 1)
        if land_ice is not None:
            icel = np.clip(g.project(land_ice, self.W, self.H), 0, 1)
            m = ~o_img & (icel > 0.25)
            w = np.clip((icel[m] - 0.25) / 0.5, 0, 1)[:, None] * 0.85
            rgb[m] = rgb[m] * (1 - w) + np.array([238, 242, 246]) / 255.0 * w
        if lake_id is not None:
            lk = self.grid.project((lake_id >= 0).astype(float), self.W, self.H, smooth=False) > 0.5
            rgb[lk] = np.array([92, 138, 178]) / 255.0
        if seaice_frac is not None:
            icy = np.clip(self.grid.project(seaice_frac, self.W, self.H), 0, 1)
            ice_col = np.array([225, 233, 240]) / 255.0
            m = o_img & (icy > 0.4)
            rgb[m] = rgb[m] * 0.25 + ice_col * 0.75
        return rgb, o_img

    def _river_segments(self, hydro):
        g = self.grid
        riv = hydro["river"]
        rcv = hydro["receiver"]
        acc = hydro["flow_acc"]
        donors = np.zeros(g.n, bool)
        donors[rcv[np.flatnonzero(riv)]] = True
        sources = np.flatnonzero(riv & ~donors)
        drawn = np.zeros(g.n, bool)
        segs, wid = [], []
        lon, lat = np.degrees(g.lon), np.degrees(g.lat)
        amax = max(acc.max(), 1.0)
        for s0 in sources[np.argsort(-acc[sources])]:
            i = s0
            while riv[i]:
                j = rcv[i]
                if j == i:
                    break
                if abs(lon[i] - lon[j]) < 90:
                    segs.append([(lon[i], lat[i]), (lon[j], lat[j])])
                    wid.append(0.25 + 1.8 * np.sqrt(acc[i] / amax))
                if drawn[j]:
                    break
                drawn[j] = True
                i = j
        return segs, wid

    def _fig(self, title):
        fig, ax = plt.subplots(figsize=(self.W / 150, self.H / 150), dpi=150)
        ax.set_xticks([])
        ax.set_yticks([])
        ax.set_title(title, fontsize=13)
        return fig, ax

    def _save(self, fig, name, caption):
        path = os.path.join(self.dir, name)
        fig.savefig(path, bbox_inches="tight", facecolor="white")
        plt.close(fig)
        self.sheets.append((name, caption))

    def _coast(self, ax, o_img, color="k", lw=0.5):
        ax.contour(np.linspace(-180, 180, self.W), np.linspace(90, -90, self.H),
                   o_img.astype(float), levels=[0.5], colors=color, linewidths=lw)

    def _scalar_sheet(self, field, cmap, title, name, caption, vmin=None, vmax=None,
                      o_img=None, cbar_label="", transform=None):
        img = self.grid.project(field, self.W, self.H)
        if transform is not None:
            img = transform(img)
        fig, ax = self._fig(title)
        im = ax.imshow(img, extent=self.ext, aspect="auto", cmap=cmap, vmin=vmin, vmax=vmax)
        if o_img is not None:
            self._coast(ax, o_img)
        cb = fig.colorbar(im, ax=ax, shrink=0.75, pad=0.015)
        cb.set_label(cbar_label, fontsize=9)
        fig.tight_layout()
        self._save(fig, name, caption)

    # ------------------------------------------------------------------
    def render_all(self, state):
        cfg, g = self.cfg, self.grid
        tect, clim, hydro = state["tect"], state["clim2"], state["hydro"]
        koppen = state["koppen"]
        elev = hydro["elev"]
        is_ocean = hydro["is_ocean"]
        seaice_frac = clim["seaice"].mean(0)
        # permanent land ice: ice-cap biome plus deep-frozen tundra
        land_ice = (state["supp"]["ice_cap"].astype(float)
                    + 0.6 * (state["supp"]["permafrost"] & (clim["T_ann"] < -9.0)))
        land_ice = np.clip(land_ice, 0, 1)

        # 1. hero relief with rivers + lakes + sea ice
        rgb, o_img = self._relief_rgb(elev, is_ocean, hydro["lake_id"], seaice_frac, land_ice)
        fig, ax = self._fig(f"{state['features']['planet']} — physical map")
        ax.imshow(rgb, extent=self.ext, aspect="auto")
        segs, wid = self._river_segments(hydro)
        ax.add_collection(LineCollection(segs, linewidths=wid, colors="#2660a4",
                                         capstyle="round", alpha=0.9))
        ax.set_xlim(-180, 180)
        ax.set_ylim(-90, 90)
        fig.tight_layout()
        self._save(fig, "01_relief.png", "Physical relief, rivers, lakes, sea ice")

        # 2. plates + boundaries + motion
        palette = np.array(plt.get_cmap("tab20").colors)
        pimg = palette[g.project(state["tect"]["plate"].astype(float), self.W, self.H,
                                 smooth=False).astype(int) % 20]
        pimg *= np.where(o_img[..., None], 0.55, 1.0)
        bt = g.project(tect["btype"].astype(float), self.W, self.H, smooth=False).astype(int)
        pimg[bt == 1] = to_rgb("#c62828")
        pimg[bt == 2] = to_rgb("#2e7d32")
        pimg[bt == 3] = to_rgb("#f9a825")
        fig, ax = self._fig("tectonic plates — red convergent, green divergent, yellow transform")
        ax.imshow(pimg, extent=self.ext, aspect="auto")
        step = self.W // 60
        idx = g.equirect_index(self.W, self.H)
        yy, xx = np.mgrid[step // 2:self.H:step, step // 2:self.W:step]
        cells = idx[yy, xx]
        ax.quiver(xx / self.W * 360 - 180, 90 - yy / self.H * 180,
                  tect["v_e"][cells], tect["v_n"][cells], color="white",
                  width=0.0014, scale=2800, alpha=0.9)
        fig.tight_layout()
        self._save(fig, "02_plates.png", "Plates, boundary types, motion vectors")

        # 3-4. temperature January / July
        for m, name in ((0, "January"), (6, "July")):
            self._scalar_sheet(clim["T"][m], "RdYlBu_r", f"{name} temperature",
                               f"0{3 + (m > 0)}_temp_{name.lower()}.png",
                               f"{name} surface temperature", vmin=-45, vmax=45,
                               o_img=o_img, cbar_label="deg C")

        # 5. annual precipitation
        self._scalar_sheet(clim["P_ann"], "YlGnBu", "annual precipitation",
                           "05_precip.png", "Annual precipitation (sqrt scale)",
                           o_img=o_img, cbar_label="sqrt(mm/yr)",
                           transform=lambda x: np.sqrt(np.maximum(x, 0)))

        # 6. winds (January & July side by side)
        fig, axes = plt.subplots(2, 1, figsize=(self.W / 150, self.H / 75), dpi=150)
        Ws, Hs = 180, 90
        X = np.linspace(-180, 180, Ws)
        Y = np.linspace(-90, 90, Hs)
        for ax, m, label in ((axes[0], 0, "January"), (axes[1], 6, "July")):
            ax.imshow(g.project(clim["T"][m], self.W, self.H), extent=self.ext,
                      aspect="auto", cmap="RdYlBu_r", vmin=-45, vmax=45, alpha=0.7)
            ue = g.project(clim["wind_e"][m], Ws, Hs)[::-1]
            un = g.project(clim["wind_n"][m], Ws, Hs)[::-1]
            ax.streamplot(X, Y, ue, un, color="k", density=2.2, linewidth=0.6, arrowsize=0.7)
            self._coast(ax, o_img, color="w", lw=0.7)
            ax.set_xticks([]); ax.set_yticks([])
            ax.set_title(f"{label} winds", fontsize=11)
        fig.tight_layout()
        self._save(fig, "06_winds.png", "Seasonal wind fields over temperature")

        # 7. ocean currents over SST
        fig, ax = self._fig("ocean currents over annual sea-surface temperature")
        sst = np.nan_to_num(clim["sst"].mean(0), nan=0.0)
        sst_img = g.project(sst, self.W, self.H)
        sst_img[~o_img] = np.nan
        ax.imshow(sst_img, extent=self.ext, aspect="auto", cmap="RdYlBu_r", vmin=-5, vmax=32)
        ce = g.project(np.where(is_ocean, clim["cur_e"], np.nan), Ws, Hs, smooth=False)[::-1]
        cn = g.project(np.where(is_ocean, clim["cur_n"], np.nan), Ws, Hs, smooth=False)[::-1]
        ax.streamplot(X, Y, ce, cn, color="k", density=2.4, linewidth=0.6, arrowsize=0.7)
        fig.tight_layout()
        self._save(fig, "07_currents.png", "Wind-driven gyres and boundary currents")

        # 8. Koppen biomes
        colors = np.array([to_rgb(c) for _, _, c in KOPPEN] + [(0.08, 0.15, 0.3)])
        kimg = g.project(koppen.astype(float), self.W, self.H, smooth=False).astype(int)
        rgbk = colors[np.where(kimg < 0, len(KOPPEN), kimg)]
        shelf = o_img & (g.project(elev, self.W, self.H) > -0.4)
        rgbk[shelf] = (0.16, 0.28, 0.45)
        fig, ax = self._fig("Koppen-Geiger climate classification")
        ax.imshow(rgbk, extent=self.ext, aspect="auto")
        present = np.unique(koppen[koppen >= 0])
        counts = np.bincount(koppen[koppen >= 0], minlength=len(KOPPEN))
        order = [i for i in np.argsort(-counts) if counts[i] > 0][:18]
        handles = [Patch(facecolor=KOPPEN[i][2], label=f"{KOPPEN[i][0]} {KOPPEN[i][1]}")
                   for i in order]
        ax.legend(handles=handles, loc="lower left", fontsize=6.5, ncols=3,
                  framealpha=0.85, borderpad=0.4)
        fig.tight_layout()
        self._save(fig, "08_biomes.png",
                   f"{len(present)} Koppen classes present")

        # 9. vegetation + fertility
        self._scalar_sheet(state["supp"]["vegetation"], "YlGn", "vegetation density (NPP)",
                           "09_vegetation.png", "Miami-model net primary productivity",
                           vmin=0, vmax=1, o_img=o_img, cbar_label="relative NPP")

        # 10. globes
        self._globes(state, elev, is_ocean, hydro["lake_id"], clim, land_ice, "10_globes.png")

        self._write_atlas(state)

    # ------------------------------------------------------------------
    def _globes(self, state, elev, is_ocean, lake_id, clim, land_ice, name):
        g = self.grid
        S = self.cfg.globe_size
        shade = _hillshade(g, elev, 55.0)
        cloud_ann = clim["clouds"].mean(0)
        seaice = clim["seaice"].mean(0)
        views = self.cfg.render_views
        fig, axes = plt.subplots(1, len(views), figsize=(5.2 * len(views), 5.6), dpi=130,
                                 facecolor="#05070d")
        ys, xs = np.mgrid[1:-1:S * 1j, -1:1:S * 1j]
        rr = xs ** 2 + ys ** 2
        vis = rr <= 1.0
        zz = np.sqrt(np.maximum(1.0 - rr, 0.0))
        for ax, lon0 in zip(np.atleast_1d(axes), views):
            lon0r = np.radians(lon0)
            lat0r = np.radians(12.0)
            # view space -> world: x east, y north, z toward viewer
            cx, sx = np.cos(lat0r), np.sin(lat0r)
            cl, sl = np.cos(lon0r), np.sin(lon0r)
            wx = zz * cx * cl - ys * sx * cl - xs * sl
            wy = zz * cx * sl - ys * sx * sl + xs * cl
            wz = zz * sx + ys * cx
            pts = np.stack([wx[vis], wy[vis], wz[vis]], 1)
            pts /= np.linalg.norm(pts, axis=1, keepdims=True)
            _, cells = g.kdtree().query(pts, workers=-1)
            e = elev[cells]
            oc = is_ocean[cells]
            rgb = np.where(oc[:, None], _ramp(e, OCEAN_STOPS), _ramp(e, LAND_STOPS))
            light = np.where(oc, 0.84 + 0.22 * shade[cells], 0.5 + 0.65 * shade[cells])
            rgb *= light[:, None]
            li = ~oc & (land_ice[cells] > 0.25)
            wgt = (np.clip((land_ice[cells][li] - 0.25) / 0.5, 0, 1) * 0.85)[:, None]
            rgb[li] = rgb[li] * (1 - wgt) + np.array([0.93, 0.95, 0.97]) * wgt
            lk = lake_id[cells] >= 0
            rgb[lk] = np.array([92, 138, 178]) / 255.0
            icy = oc & (seaice[cells] > 0.4)
            rgb[icy] = rgb[icy] * 0.25 + np.array([0.88, 0.91, 0.94]) * 0.75
            cl_a = np.clip(cloud_ann[cells], 0, 1) ** 1.6 * 0.75
            rgb = rgb * (1 - cl_a[:, None]) + cl_a[:, None] * 0.95
            # limb darkening + day-night hint
            limb = 0.62 + 0.38 * zz[vis]
            rgb *= limb[:, None]
            img = np.zeros((S, S, 3))
            img[vis] = np.clip(rgb, 0, 1)
            ax.imshow(img)
            ax.set_xticks([]); ax.set_yticks([])
            ax.set_facecolor("#05070d")
            for spine in ax.spines.values():
                spine.set_visible(False)
        fig.suptitle(f"{state['features']['planet']}", color="white", fontsize=16, y=0.98)
        fig.tight_layout()
        path = os.path.join(self.dir, name)
        fig.savefig(path, bbox_inches="tight", facecolor="#05070d")
        plt.close(fig)
        self.sheets.append((name, "Orthographic views with annual cloud cover"))

    # ------------------------------------------------------------------
    def _write_atlas(self, state):
        planet = state["features"]["planet"]
        items = "".join(
            f'<button class="tab" onclick="show({i})">{cap}</button>'
            for i, (_, cap) in enumerate(self.sheets))
        imgs = "".join(
            f'<img id="img{i}" src="{fn}" style="display:{"block" if i == 0 else "none"}">'
            for i, (fn, _) in enumerate(self.sheets))
        html = f"""<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{planet} — atlas</title>
<style>
 body {{ background:#10141c; color:#e8e8e8; font-family:Segoe UI,system-ui,sans-serif;
        margin:0; padding:16px; }}
 h1 {{ font-size:20px; margin:4px 0 12px; }}
 .tabs {{ display:flex; flex-wrap:wrap; gap:6px; margin-bottom:12px; }}
 .tab {{ background:#232b3a; color:#dce3ee; border:1px solid #3a465c; border-radius:6px;
        padding:6px 10px; cursor:pointer; font-size:13px; }}
 .tab:hover {{ background:#31405c; }}
 img {{ max-width:100%; border-radius:8px; }}
</style></head><body>
<h1>{planet} — generated atlas (seed {state['cfg'].seed})</h1>
<div class="tabs">{items}</div>
{imgs}
<script>
function show(k) {{
  for (let i = 0; ; i++) {{
    const el = document.getElementById('img' + i);
    if (!el) break;
    el.style.display = (i === k) ? 'block' : 'none';
  }}
}}
document.addEventListener('keydown', e => {{
  const vis = [...document.querySelectorAll('img')].findIndex(im => im.style.display !== 'none');
  const n = document.querySelectorAll('img').length;
  if (e.key === 'ArrowRight') show((vis + 1) % n);
  if (e.key === 'ArrowLeft') show((vis + n - 1) % n);
}});
</script></body></html>"""
        with open(os.path.join(self.dir, "atlas.html"), "w", encoding="utf-8") as f:
            f.write(html)

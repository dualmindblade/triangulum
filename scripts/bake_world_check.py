"""Evidence and gates for scripts/bake_world.py.

    python scripts/bake_world_check.py --world game/assets/world/seed42 \
        --old viewer/assets --run-dir output/seed42_r8

Reads the new bake through its meta.json only (as the engine will), and the
old gnomonic bake (bake_faces.py: elev f32 [0,4n), koppen u8 [4n,5n), rough
[5n,9n), precip [9n,13n), temp [13n,17n), flow [17n,21n)). Writes, under
<world>/evidence/:

  face_{f}_hillshade.png     elevation hillshade per face, native resolution
  crop_{name}.png            old | new over the same directions: a 300x300
                             texel crop, and below it a 60x60 texel zoom
                             sampled 8x the way each engine does it (old:
                             bilinear, as the old viewer; new: Catmull-Rom)
  spectrum.png               slope spectra along random great-circle lines
  report.json                every number below

Gates (exit code 1 if any fails):
  cell structure   share of slope power at wavelengths < 50 km along random
                   lines. A ~30 km hex lattice cannot carry those
                   wavelengths (its Nyquist limit is ~52-60 km), so any power
                   there is reconstruction artifact. Also: mean |slope| binned
                   by distance to the nearest cell must be flat.
  edges            max |difference| per field between faces along every
                   shared edge (expected: 0, the directions are bit-identical)
  coast            cell-centre land/sea sign agreement (raster, Catmull-Rom)

Informational: cell-centre reproduction, elevation vs ocean_frac coastline
offset, and uphill excursions of the terrain along planetgen river segments.
"""

import argparse
import json
import os
import sys

import numpy as np
from PIL import Image, ImageDraw
from scipy.spatial import cKDTree

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import world_interp as wi  # noqa: E402

R = wi.SOURCE_RADIUS_KM
# hillshade vertical exaggeration: 70/8, i.e. Earth slopes as they would
# look at the 1/8 vertical scale under review in DESIGN.md (worst case)
ZF = 70.0 / 8.0
SUPER_NYQUIST_KM = 50.0
GATE_SUPER_NYQUIST_FRAC = 0.02
GATE_PHASE_DEV = 0.05


# ----------------------------------------------------------------------------
# loading
# ----------------------------------------------------------------------------

class NewBake:
    def __init__(self, world):
        with open(os.path.join(world, "meta.json"), encoding="utf-8") as f:
            self.meta = json.load(f)
        lay = self.meta["layout"]
        assert lay["projection"] == "equal_angle" and lay["edge_inclusive"]
        self.res = lay["resolution"]
        self.fields = {f["name"]: f for f in self.meta["fields"]}
        self.world = world
        self.tan = wi.equal_angle_tangent(self.res)
        self._cache = {}

    def face(self, f):
        if f not in self._cache:
            path = os.path.join(self.world, self.meta["layout"]["file_pattern"].format(face=f))
            self._cache[f] = np.fromfile(path, np.uint8)
        return self._cache[f]

    def layer(self, f, name):
        e = self.fields[name]
        buf = self.face(f)
        dt = np.dtype("<f4") if e["type"] == "f32" else np.uint8
        n = self.res * self.res
        return np.frombuffer(buf, dt, count=n, offset=e["offset"]).reshape(self.res, self.res)

    def texcoords(self, xyz):
        face, a, b = wi.direction_to_face(xyz)
        col = (np.arctan(a) * 4 / np.pi + 1) * 0.5 * (self.res - 1)
        row = (np.arctan(b) * 4 / np.pi + 1) * 0.5 * (self.res - 1)
        return face, row, col

    def dirs(self, f, rows, cols):
        """Directions at fractional texel coordinates."""
        p = rows / (self.res - 1) * 2 - 1
        q = cols / (self.res - 1) * 2 - 1
        return _dirs(f, np.tan(q * np.pi / 4), np.tan(p * np.pi / 4))


class OldBake:
    OFFS = dict(elevation_km=(0, "f4"), koppen=(4, "u1"), rough_km=(5, "f4"),
                precip_mm_yr=(9, "f4"), temp_mean_c=(13, "f4"), flow_log10=(17, "f4"))

    def __init__(self, folder):
        with open(os.path.join(folder, "meta.json"), encoding="utf-8") as f:
            self.meta = json.load(f)
        self.res = self.meta["resolution"]
        self.folder = folder
        self._cache = {}

    def face(self, f):
        if f not in self._cache:
            self._cache[f] = np.fromfile(os.path.join(self.folder, f"face_{f}.bin"), np.uint8)
        return self._cache[f]

    def layer(self, f, name):
        k, dt = self.OFFS[name]
        n = self.res * self.res
        return np.frombuffer(self.face(f), np.dtype("<" + dt) if dt == "f4" else np.uint8,
                             count=n, offset=k * n).reshape(self.res, self.res)

    def texcoords(self, xyz):
        face, a, b = wi.direction_to_face(xyz)
        return face, (b + 1) * 0.5 * (self.res - 1), (a + 1) * 0.5 * (self.res - 1)


def _dirs(f, a, b):
    axis, right, up = (np.asarray(v, np.float64) for v in wi.FACES[f])
    d = axis + a[..., None] * right + b[..., None] * up
    return d / np.linalg.norm(d, axis=-1, keepdims=True)


# ----------------------------------------------------------------------------
# samplers
# ----------------------------------------------------------------------------

def bilinear(z, row, col):
    n = z.shape[0]
    r0 = np.clip(np.floor(row).astype(int), 0, n - 2)
    c0 = np.clip(np.floor(col).astype(int), 0, n - 2)
    fr = row - r0
    fc = col - c0
    z = z.astype(np.float64)
    return ((z[r0, c0] * (1 - fc) + z[r0, c0 + 1] * fc) * (1 - fr)
            + (z[r0 + 1, c0] * (1 - fc) + z[r0 + 1, c0 + 1] * fc) * fr)


def _cr_weights(t):
    t2 = t * t
    t3 = t2 * t
    return np.stack([(-t3 + 2 * t2 - t) * 0.5, (3 * t3 - 5 * t2 + 2) * 0.5,
                     (-3 * t3 + 4 * t2 + t) * 0.5, (t3 - t2) * 0.5])


def catmull_rom(z, row, col):
    """C1 interpolating bicubic; stencil clamped at the raster border."""
    n = z.shape[0]
    r0 = np.floor(row).astype(int)
    c0 = np.floor(col).astype(int)
    r0 = np.clip(r0, 0, n - 2)
    c0 = np.clip(c0, 0, n - 2)
    wr = _cr_weights(row - r0)
    wc = _cr_weights(col - c0)
    z = z.astype(np.float64)
    out = np.zeros(np.shape(row))
    for i in range(4):
        rr = np.clip(r0 + i - 1, 0, n - 1)
        acc = np.zeros(np.shape(row))
        for j in range(4):
            cc = np.clip(c0 + j - 1, 0, n - 1)
            acc += wc[j] * z[rr, cc]
        out += wr[i] * acc
    return out


def sample(bake, name, xyz, how):
    face, row, col = bake.texcoords(xyz)
    out = np.empty(len(xyz))
    for f in range(6):
        m = face == f
        if m.any():
            z = bake.layer(f, name)
            out[m] = (catmull_rom if how == "cr" else bilinear)(z, row[m], col[m])
    return out


# ----------------------------------------------------------------------------
# hillshade
# ----------------------------------------------------------------------------

def hillshade(z, dirs, zf=ZF, az=315.0, alt=35.0):
    """z (h, w) km over a grid of unit directions (h, w, 3); rows walk 'up'.

    Uses the true spacing between texels on the sphere, so equal-angle and
    gnomonic grids shade alike. Light from the upper left of the image.
    """
    P = dirs * R
    dcol = np.linalg.norm(np.gradient(P, axis=1), axis=2)
    drow = np.linalg.norm(np.gradient(P, axis=0), axis=2)
    gx = np.gradient(z, axis=1) / dcol * zf
    gy = np.gradient(z, axis=0) / drow * zf      # +row = up in the image
    # surface normal (-gx, -gy, 1) against a light at azimuth `az` (clockwise
    # from image-up) and elevation `alt`; az 315 = from the upper left
    a = np.radians(az)
    al = np.radians(alt)
    lx, ly, lz = np.cos(al) * np.sin(a), np.cos(al) * np.cos(a), np.sin(al)
    hs = (-gx * lx - gy * ly + lz) / np.sqrt(gx * gx + gy * gy + 1.0)
    return np.clip(hs, 0, 1)


def to_img(hs, z=None):
    """Hillshade (rows walk up) -> uint8 image with up at the top.

    With z given, the shade is tinted: sand for z >= 0, blue for z < 0 (sea at
    sea level, as the engine will draw it), so coastlines show.
    """
    hs = np.flipud(hs)
    if z is None:
        return (hs * 255 + 0.5).astype(np.uint8)
    sea = np.flipud(z) < 0
    tint = np.where(sea[..., None], [0.62, 0.76, 0.95], [0.97, 0.93, 0.84])
    shade = 0.25 + 0.75 * hs
    return (shade[..., None] * tint * 255 + 0.5).astype(np.uint8)


def label(img, text):
    im = Image.fromarray(img).convert("RGB")
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, 8 + 7 * len(text), 16], fill=(255, 255, 255))
    d.text((4, 2), text, fill=(0, 0, 0))
    return im


# ----------------------------------------------------------------------------
# crops
# ----------------------------------------------------------------------------

def window_sums(a, w, stride):
    """Sums of a over w x w windows at the given stride: (top-left rows, cols, sums)."""
    ii = np.pad(a.astype(np.float64).cumsum(0).cumsum(1), ((1, 0), (1, 0)))
    n = a.shape[0]
    starts = np.arange(2, n - w - 2, stride)
    r = starts[:, None]
    c = starts[None, :]
    s = ii[r + w, c + w] - ii[r, c + w] - ii[r + w, c] + ii[r, c]
    return starts, s


def pick_crops(new, size=300):
    best = {}
    for f in range(6):
        e = new.layer(f, "elevation_km")
        oc = new.layer(f, "ocean_frac") > 0.5
        rough = new.layer(f, "rough_km")
        coast = np.zeros_like(oc)
        coast[:, 1:] |= oc[:, 1:] != oc[:, :-1]
        coast[1:, :] |= oc[1:, :] != oc[:-1, :]
        starts, land = window_sums(~oc, size, 25)
        _, csum = window_sums(coast, size, 25)
        _, rsum = window_sums(rough, size, 25)
        _, esum = window_sums(e, size, 25)
        lf = land / size ** 2
        # coastline: about half land, most coastline texels
        score = np.where((lf > 0.3) & (lf < 0.7), csum, -1)
        i = np.unravel_index(np.argmax(score), score.shape)
        if score[i] > best.get("coastline", (-1,))[0]:
            best["coastline"] = (score[i], f, starts[i[0]], starts[i[1]])
        # plain: (nearly) all land, lowland, least rough
        score = np.where((lf > 0.95) & (esum / size ** 2 < 1.0), -rsum, -np.inf)
        i = np.unravel_index(np.argmax(score), score.shape)
        if score[i] > best.get("plain", (-np.inf,))[0]:
            best["plain"] = (score[i], f, starts[i[0]], starts[i[1]])
        # mountain: centred on the highest texel
        r, c = np.unravel_index(np.argmax(e), e.shape)
        if e[r, c] > best.get("mountain", (-np.inf,))[0]:
            r0 = int(np.clip(r - size // 2, 2, new.res - size - 2))
            c0 = int(np.clip(c - size // 2, 2, new.res - size - 2))
            best["mountain"] = (e[r, c], f, r0, c0)
    return {k: dict(face=int(v[1]), row0=int(v[2]), col0=int(v[3]), size=size)
            for k, v in best.items()}


def render_crop(new, old, spec, zoom=60, up=8):
    f, r0, c0, n = spec["face"], spec["row0"], spec["col0"], spec["size"]
    rows = np.arange(r0, r0 + n, dtype=float)
    cols = np.arange(c0, c0 + n, dtype=float)
    RR, CC = np.meshgrid(rows, cols, indexing="ij")
    dirs = new.dirs(f, RR, CC)
    z_new = new.layer(f, "elevation_km")[r0:r0 + n, c0:c0 + n].astype(np.float64)
    z_old = sample(old, "elevation_km", dirs.reshape(-1, 3), "bilinear").reshape(n, n)
    top = [to_img(hillshade(z_old, dirs), z_old), to_img(hillshade(z_new, dirs), z_new)]
    # zoom: the central zoom x zoom texels, sampled up x finer, each bake the
    # way its engine samples it
    zr0 = r0 + n // 2 - zoom // 2
    zc0 = c0 + n // 2 - zoom // 2
    fr = zr0 + np.arange(zoom * up) / up
    fc = zc0 + np.arange(zoom * up) / up
    FR, FC = np.meshgrid(fr, fc, indexing="ij")
    zd = new.dirs(f, FR, FC)
    zn = catmull_rom(new.layer(f, "elevation_km"), FR, FC)
    zo = sample(old, "elevation_km", zd.reshape(-1, 3), "bilinear").reshape(FR.shape)
    bot = [to_img(hillshade(zo, zd), zo), to_img(hillshade(zn, zd), zn)]
    km = (np.pi / 2) / (new.res - 1) * R
    scale = 2
    tops = [label(np.kron(t, np.ones((scale, scale, 1), np.uint8)), s) for t, s in
            zip(top, [f"OLD bake (IDW-3, gnomonic, bilinear) {n}x{n} texels",
                      f"NEW bake (MLS, equal-angle) {n}x{n} texels"])]
    bots = [label(b, s) for b, s in
            zip(bot, [f"OLD zoom {zoom}x{zoom} texels (~{zoom * km:.0f} km), bilinear",
                      f"NEW zoom {zoom}x{zoom} texels, Catmull-Rom"])]
    W = tops[0].width
    gap = 8
    canvas = Image.new("RGB", (2 * W + gap, tops[0].height + gap + bots[0].height), (255, 255, 255))
    canvas.paste(tops[0], (0, 0))
    canvas.paste(tops[1], (W + gap, 0))
    off = (W - bots[0].width) // 2
    canvas.paste(bots[0], (off, tops[0].height + gap))
    canvas.paste(bots[1], (W + gap + off, tops[0].height + gap))
    return canvas, top, bot


# ----------------------------------------------------------------------------
# cell-structure tests
# ----------------------------------------------------------------------------

def random_arcs(new, n_lines, n_samples, step_km, seed=11):
    """Great-circle arcs that stay inside one face (2-texel margin)."""
    rng = np.random.default_rng(seed)
    out = []
    while len(out) < n_lines:
        p = rng.normal(size=3)
        p /= np.linalg.norm(p)
        t = rng.normal(size=3)
        t -= t.dot(p) * p
        t /= np.linalg.norm(t)
        ang = (np.arange(n_samples) - n_samples / 2) * step_km / R
        pts = np.cos(ang)[:, None] * p + np.sin(ang)[:, None] * t
        face, row, col = new.texcoords(pts)
        if (face == face[0]).all() and row.min() > 2 and col.min() > 2 \
                and row.max() < new.res - 3 and col.max() < new.res - 3:
            out.append(pts)
    return np.stack(out)


def slope_psd(z, step_km):
    s = np.diff(z, axis=1) / step_km
    s = s - s.mean(1, keepdims=True)
    win = np.hanning(s.shape[1])
    S = np.abs(np.fft.rfft(s * win, axis=1)) ** 2
    return np.fft.rfftfreq(s.shape[1], step_km), S.mean(0)


def phase_profile(bake, name, tree, spacing, faces=range(6), stride=2):
    """Mean |gradient| (km/km) by distance-to-nearest-cell / spacing."""
    bins = np.linspace(0, 0.6, 7)
    sums = np.zeros(6)
    cnts = np.zeros(6)
    for f in faces:
        z = bake.layer(f, name).astype(np.float64)
        n = z.shape[0]
        idx = np.arange(1, n - 1, stride)
        RR, CC = np.meshgrid(idx, idx, indexing="ij")
        if isinstance(bake, NewBake):
            d0 = bake.dirs(f, RR.astype(float), CC.astype(float))
            dc = bake.dirs(f, RR.astype(float), CC + 1.0) - bake.dirs(f, RR.astype(float), CC - 1.0)
            dr = bake.dirs(f, RR + 1.0, CC.astype(float)) - bake.dirs(f, RR - 1.0, CC.astype(float))
        else:
            def od(r, c):
                return _dirs(f, c / (n - 1) * 2 - 1, r / (n - 1) * 2 - 1)
            d0 = od(RR, CC)
            dc = od(RR, CC + 1.0) - od(RR, CC - 1.0)
            dr = od(RR + 1.0, CC) - od(RR - 1.0, CC)
        gx = (z[RR, CC + 1] - z[RR, CC - 1]) / (np.linalg.norm(dc, axis=-1) * R)
        gy = (z[RR + 1, CC] - z[RR - 1, CC]) / (np.linalg.norm(dr, axis=-1) * R)
        g = np.hypot(gx, gy).ravel()
        dist, _ = tree.query(d0.reshape(-1, 3), k=1, workers=-1)
        ph = dist * R / spacing
        b = np.clip(np.digitize(ph, bins) - 1, 0, 5)
        sums += np.bincount(b, weights=g, minlength=6)
        cnts += np.bincount(b, minlength=6)
    prof = sums / np.maximum(cnts, 1)
    return (prof / (sums.sum() / cnts.sum())).tolist(), cnts.tolist()


# ----------------------------------------------------------------------------
# edges
# ----------------------------------------------------------------------------

def edge_agreement(new):
    n = new.res
    pts, ref = [], []
    for f in range(6):
        for r, c in ([(0, j) for j in range(n)] + [(n - 1, j) for j in range(n)]
                     + [(i, 0) for i in range(1, n - 1)] + [(i, n - 1) for i in range(1, n - 1)]):
            ref.append((f, r, c))
        rr = np.array([x[1] for x in ref[-(4 * n - 4):]], float)
        cc = np.array([x[2] for x in ref[-(4 * n - 4):]], float)
        pts.append(_dirs(f, new.tan[cc.astype(int)], new.tan[rr.astype(int)]))
    pts = np.concatenate(pts)
    ref = np.array(ref)
    tree = cKDTree(pts)
    pairs = tree.query_pairs(1e-9, output_type="ndarray")
    pairs = pairs[ref[pairs[:, 0], 0] != ref[pairs[:, 1], 0]]
    exact = int(sum(np.array_equal(pts[i], pts[j]) for i, j in pairs))
    out = {}
    for name, e in new.fields.items():
        vals = np.empty(len(ref), np.float64)
        for f in range(6):
            m = ref[:, 0] == f
            vals[m] = new.layer(f, name)[ref[m, 1], ref[m, 2]]
        diff = np.abs(vals[pairs[:, 0]] - vals[pairs[:, 1]])
        out[name] = float(diff.max())
    return dict(pairs=int(len(pairs)), bit_identical_direction_pairs=exact,
                max_abs_diff=out)


# ----------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--world", default="game/assets/world/seed42")
    ap.add_argument("--old", default="viewer/assets")
    ap.add_argument("--run-dir", default="output/seed42_r8")
    ap.add_argument("--evidence", default=None)
    ap.add_argument("--lines", type=int, default=2000)
    args = ap.parse_args()
    ev = args.evidence or os.path.join(args.world, "evidence")
    os.makedirs(ev, exist_ok=True)

    new = NewBake(args.world)
    old = OldBake(args.old)
    d = np.load(os.path.join(args.run_dir, "planet_data.npz"))
    xyz = d["xyz"].astype(np.float64)
    xyz /= np.linalg.norm(xyz, axis=1, keepdims=True)
    elev = d["elevation_km"].astype(np.float64)
    ocean = d["is_ocean"]
    tree = cKDTree(xyz)
    spacing = new.meta["source"]["mean_neighbour_spacing_km"]
    report = dict(world=os.path.abspath(args.world), old=os.path.abspath(args.old),
                  hillshade_vertical_exaggeration=ZF)
    gates = {}

    # 1. hillshade per face
    for f in range(6):
        idx = np.arange(new.res, dtype=float)
        RR, CC = np.meshgrid(idx, idx, indexing="ij")
        hs = hillshade(new.layer(f, "elevation_km").astype(np.float64), new.dirs(f, RR, CC))
        Image.fromarray(to_img(hs)).save(os.path.join(ev, f"face_{f}_hillshade.png"))
    print("hillshades written", flush=True)

    # 2. crops
    crops = pick_crops(new)
    report["crops"] = crops
    for name, spec in crops.items():
        canvas, top, bot = render_crop(new, old, spec)
        canvas.save(os.path.join(ev, f"crop_{name}.png"))
        Image.fromarray(top[0]).save(os.path.join(ev, f"crop_{name}_old.png"))
        Image.fromarray(top[1]).save(os.path.join(ev, f"crop_{name}_new.png"))
    print("crops written:", crops, flush=True)

    # 3a. line spectra
    step = 2.0
    ns = 512
    arcs = random_arcs(new, args.lines, ns, step)
    flat = arcs.reshape(-1, 3)
    series = {
        "old_bilinear": sample(old, "elevation_km", flat, "bilinear"),
        "old_catmull_rom": sample(old, "elevation_km", flat, "cr"),
        "new_catmull_rom": sample(new, "elevation_km", flat, "cr"),
    }
    spec = {}
    freqs = None
    for k, v in series.items():
        freqs, psd = slope_psd(v.reshape(len(arcs), ns), step)
        spec[k] = psd
    lam = np.where(freqs > 0, 1.0 / np.maximum(freqs, 1e-12), np.inf)
    sup = (lam < SUPER_NYQUIST_KM) & (freqs > 0)
    resolved = (lam >= 100) & (lam <= 400)
    lines = {}
    for k, psd in spec.items():
        lines[k] = dict(super_nyquist_slope_power_frac=float(psd[sup].sum() / psd[1:].sum()),
                        resolved_100_400km_power_vs_old_bilinear=float(
                            psd[resolved].sum() / spec["old_bilinear"][resolved].sum()))
    report["line_spectra"] = dict(lines=len(arcs), length_km=(ns - 1) * step, step_km=step,
                                  super_nyquist_below_km=SUPER_NYQUIST_KM, **lines)
    gates["super_nyquist"] = lines["new_catmull_rom"]["super_nyquist_slope_power_frac"] \
        < GATE_SUPER_NYQUIST_FRAC
    plot_spectrum(freqs, spec, os.path.join(ev, "spectrum.png"))
    print("line spectra:", json.dumps(lines, indent=1), flush=True)

    # 3b. phase profiles on the rasters themselves
    pn, cnt = phase_profile(new, "elevation_km", tree, spacing)
    po, _ = phase_profile(old, "elevation_km", tree, spacing)
    report["phase_profile"] = dict(
        bins_dist_over_spacing=[[round(a, 2), round(a + 0.1, 2)] for a in np.arange(0, 0.6, 0.1)],
        texels_per_bin=cnt, new=pn, old=po,
        note="mean |gradient| of elevation per bin / overall mean; 1.0 everywhere = no cell signature")
    gates["phase_flat"] = max(abs(x - 1) for x in pn) < GATE_PHASE_DEV
    print("phase profile new", np.round(pn, 3), "old", np.round(po, 3), flush=True)

    # 4. cell-centre reproduction through the raster
    cell = {}
    for nm, src in (("elevation_km", elev),
                    ("temp_mean_c", d["temp_c_monthly"].astype(np.float64).mean(0)),
                    ("precip_mm_yr", d["precip_mm_monthly"].astype(np.float64).mean(0) * 12)):
        v = sample(new, nm, xyz, "cr")
        r = v - src
        cell[nm] = dict(rms=float(np.sqrt((r ** 2).mean())), median_abs=float(np.median(np.abs(r))),
                        p90_abs=float(np.percentile(np.abs(r), 90)),
                        p99_abs=float(np.percentile(np.abs(r), 99)), max_abs=float(np.abs(r).max()))
        if nm == "elevation_km":
            vo = sample(old, nm, xyz, "bilinear") - src
            cell["elevation_km_old_bilinear"] = dict(
                rms=float(np.sqrt((vo ** 2).mean())), median_abs=float(np.median(np.abs(vo))),
                p99_abs=float(np.percentile(np.abs(vo), 99)))
            wrong = int(((~ocean) & (v < 0)).sum() + (ocean & (v >= 0)).sum())
            cell["elevation_sign_wrong_cells"] = wrong
            cell["elevation_sign_wrong_cells_old_bilinear"] = int(
                ((~ocean) & (vo + src < 0)).sum() + (ocean & (vo + src >= 0)).sum())
    report["cell_centre_through_raster"] = cell
    gates["coast_sign"] = cell["elevation_sign_wrong_cells"] <= 0.001 * len(xyz)
    print("cell-centre:", json.dumps(cell, indent=1), flush=True)

    # 5. coast consistency between elevation and ocean_frac
    mism, total, cross = 0, 0, 0
    for f in range(6):
        e = new.layer(f, "elevation_km")
        oc = new.layer(f, "ocean_frac")
        mism += int(((e < 0) != (oc > 0.5)).sum())
        total += e.size
        s = e < 0
        cross += int((s[:, 1:] != s[:, :-1]).sum() + (s[1:, :] != s[:-1, :]).sum())
    report["coast_consistency"] = dict(
        texels_elev_sign_vs_ocean_frac_disagree=mism, texels=total,
        coastline_texel_crossings=cross,
        mean_offset_texels=mism / max(cross, 1) * 1.0,
        note="disagreeing texels per coastline crossing ~ mean offset of the two "
             "coastlines in texels (~9.8 km each at 1024)")
    print("coast consistency:", report["coast_consistency"], flush=True)

    # 6. rivers (informational): water must run downhill, so count uphill
    # excursions of the sampled terrain along each cell -> receiver segment
    rcv = d["receiver"]
    lake = d["lake_id"] >= 0
    seg = np.flatnonzero(d["river"] & ~ocean[rcv] & ~lake[rcv] & (rcv != np.arange(len(rcv))))
    t = np.linspace(0.0, 1.0, 31)
    pts = xyz[seg][:, None, :] * (1 - t)[None, :, None] + xyz[rcv[seg]][:, None, :] * t[None, :, None]
    pts = (pts / np.linalg.norm(pts, axis=2, keepdims=True)).reshape(-1, 3)
    riv = dict(segments=int(len(seg)),
               source_drop_median_m=float(np.median(elev[seg] - elev[rcv[seg]]) * 1000))
    for k, bake, how in (("old_bilinear", old, "bilinear"), ("new_catmull_rom", new, "cr")):
        z = sample(bake, "elevation_km", pts, how).reshape(len(seg), len(t))
        climb = np.clip(np.diff(z, axis=1), 0, None).sum(1)
        riv[k] = dict(segments_climbing_over_1m_frac=float(np.mean(climb > 0.001)),
                      climb_p90_m=float(np.percentile(climb, 90) * 1000),
                      climb_max_m=float(climb.max() * 1000))
    report["rivers_downhill"] = riv
    print("rivers:", json.dumps(riv, indent=1), flush=True)

    # 7. edges
    edges = edge_agreement(new)
    report["edges"] = edges
    gates["edges"] = max(edges["max_abs_diff"].values()) == 0.0
    print("edges:", json.dumps(edges, indent=1), flush=True)

    report["gates"] = gates
    report["bake"] = new.meta.get("bake")
    with open(os.path.join(ev, "report.json"), "w", encoding="utf-8") as f:
        json.dump(report, f, indent=1)
    print("GATES:", gates)
    sys.exit(0 if all(gates.values()) else 1)


def plot_spectrum(freqs, spec, path):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    ink, muted, grid, surface = "#0b0b0b", "#898781", "#e1e0d9", "#fcfcfb"
    orange, blue = "#eb6834", "#2a78d6"
    m = freqs > 0
    lam = 1.0 / freqs[m]
    fig, ax = plt.subplots(figsize=(8.8, 4.6), dpi=150, facecolor=surface)
    ax.set_facecolor(surface)
    ax.axvspan(lam.min(), SUPER_NYQUIST_KM, color="#f2f1ec", lw=0)
    ax.text(SUPER_NYQUIST_KM * 0.93, 0.97, "shorter than the ~30 km cell lattice can carry:\nany power here is artifact",
            transform=ax.get_xaxis_transform(), ha="left", va="top", fontsize=8, color=muted)
    styles = {"old_bilinear": (orange, "-", "old bake, bilinear (as the old viewer)", "old, bilinear"),
              "old_catmull_rom": (orange, (0, (4, 2)), "old bake, Catmull-Rom", "old, Catmull-Rom"),
              "new_catmull_rom": (blue, "-", "new bake, Catmull-Rom", "new")}
    for k, (c, ls, lab, short) in styles.items():
        ax.plot(lam, spec[k][m], color=c, ls=ls, lw=2, label=lab)
        ax.annotate(short, (lam[-1], spec[k][m][-1]), xytext=(4, 0), textcoords="offset points",
                    va="center", ha="left", fontsize=7, color=ink, annotation_clip=False)
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.invert_xaxis()
    ax.set_xlabel("wavelength along the line (km)", color=ink)
    ax.set_ylabel("slope power (arbitrary units)", color=ink)
    ax.set_title("Elevation slope spectrum along 2,000 random 1,022 km great-circle lines",
                 color=ink, fontsize=10, loc="left")
    ax.grid(True, which="major", color=grid, lw=0.6)
    for s in ax.spines.values():
        s.set_color("#c3c2b7")
    ax.tick_params(colors=muted)
    ax.legend(frameon=False, fontsize=8, loc="lower left", labelcolor=ink)
    fig.tight_layout()
    fig.savefig(path, facecolor=surface)
    plt.close(fig)


if __name__ == "__main__":
    main()

"""Bake planetgen's planet into the new engine's world rasters.

    python scripts/bake_world.py --run-dir output/seed42_r8 \
        --out game/assets/world/seed42 [--res 1024]

Input:  <run-dir>/planet_data.npz (read only). If <run-dir>/cache/ holds the
        tectonics and hydrology stage pickles (and they match the npz), five
        extra fields come from them: rock erodibility, volcanic fraction,
        uplift and subsidence (tectonics), and lake surface level (hydrology).
Output: <out>/face_{0..5}.bin + <out>/meta.json. meta.json is the contract:
        the engine reads only it to find every field, its type, units and
        byte offset. Nothing about the layout is implied by this script.

Layout
------
Six cube faces with the frames of viewer/src/planet.rs FACES. Each face is a
res x res raster on an EQUAL-ANGLE layout: face parameter p in [-1, 1] maps to
tangent coordinate a = tan(p * pi/4), and the direction is
normalize(axis + a*right + b*up). Rasters are edge inclusive: texel 0 and
texel res-1 lie exactly on the face edges, so neighbouring faces sample the
same directions along every shared edge. Row r walks `up` (row 0 is b = -1),
column c walks `right`; layers are row-major.

Interpolation (see scripts/world_interp.py for the kernel)
----------------------------------------------------------
Continuous fields use quadratic moving least squares (MLS) with a Wendland
C4 weight of fixed support radius, `--support` x the grid's mean neighbour
spacing (2.75 x 30.8 km = 84.6 km on Neisor r8). The result is a C4 function
of direction, so no bullseyes at cell centres and no steps where a
neighbour set changes. The radius was chosen with the line-spectrum and
cell-phase tests in scripts/bake_world_check.py: it removes the cell
signature while keeping the slope power planetgen actually resolves.

MLS approximates, it does not interpolate: on Neisor, elevation at cell
centres comes back within a few metres for most cells (median 2.5 m), with an
RMS near 50 m driven by single-cell spikes and trenches. One correction keeps the
coastline honest: low coastal cells (median +38 m) beside deep water would
otherwise sink below sea level, so before baking, the elevation data is
pre-compensated at the few cells whose sign flips. Every land cell centre
stays >= 0 and every sea cell centre stays < 0, which keeps small islands and
narrow straits. The output is still exactly MLS of (slightly edited) cell
data, so it is just as smooth.

Bounded fields (fractions, precipitation, ...) are MLS'd like everything else
and clamped to their physical range intersected with the range the source
data spans: quadratic MLS overshoots a step by up to ~20%. Categorical fields
(Koppen, plate, crust type) take the nearest cell.
"""

import argparse
import json
import os
import pickle
import subprocess
import sys
import time

import numpy as np
from scipy.sparse import csr_matrix
from scipy.spatial import cKDTree

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import world_interp as wi  # noqa: E402

FORMAT = "triangulum-world-raster"
FORMAT_VERSION = 1
MONTHS = 12
COAST_MARGIN_KM = 0.010


# ----------------------------------------------------------------------------
# per-cell fields
# ----------------------------------------------------------------------------

def annual_harmonic(monthly):
    """Mean and first annual harmonic of a (12, n) monthly series.

    Month m is centred on year fraction t_m = (m + 0.5)/12 (planetgen's
    insolation uses day 15.2 + 30.44 m). Returns mean, c, s with
    x(t) ~ mean + c cos(2 pi t) + s sin(2 pi t).
    """
    m = np.arange(MONTHS)
    w = 2.0 * np.pi * (m + 0.5) / MONTHS
    mean = monthly.mean(0)
    c = (monthly * np.cos(w)[:, None]).sum(0) * 2.0 / MONTHS
    s = (monthly * np.sin(w)[:, None]).sum(0) * 2.0 / MONTHS
    fit = mean + np.cos(w)[:, None] * c + np.sin(w)[:, None] * s
    explained = 1.0 - ((monthly - fit) ** 2).sum() / ((monthly - mean) ** 2).sum()
    return mean, c, s, float(explained)


def load_cells(run_dir):
    """Per-cell inputs. Returns (xyz, continuous dict, extras dict, notes)."""
    d = np.load(os.path.join(run_dir, "planet_data.npz"))
    xyz = d["xyz"].astype(np.float64)
    xyz /= np.linalg.norm(xyz, axis=1, keepdims=True)
    elev = d["elevation_km"].astype(np.float64)
    ocean = d["is_ocean"].astype(bool)

    nb = d["neighbors"]
    valid = nb >= 0
    nb_safe = np.where(valid, nb, 0)
    # roughness exactly as bake_faces.py: mean |elevation delta| to neighbours
    rough = (np.abs(elev[nb_safe] - elev[:, None]) * valid).sum(1)
    rough /= np.maximum(valid.sum(1), 1)
    spacing_km = float((np.linalg.norm(xyz[nb_safe] - xyz[:, None], axis=2)[valid]).mean()
                       * wi.SOURCE_RADIUS_KM)

    temp = d["temp_c_monthly"].astype(np.float64)
    t_mean, t_cos, t_sin, t_expl = annual_harmonic(temp)
    precip_yr = d["precip_mm_monthly"].astype(np.float64).mean(0) * 12.0
    lake = d["lake_id"] >= 0

    cont = {
        "elevation_km": elev,
        "rough_km": rough,
        "temp_mean_c": t_mean,
        "temp_season_cos_c": t_cos,
        "temp_season_sin_c": t_sin,
        "precip_mm_yr": precip_yr,
        "flow_log10": np.log10(1.0 + d["flow_accum_m3s"].astype(np.float64)),
        "ocean_frac": ocean.astype(np.float64),
        "lake_frac": lake.astype(np.float64),
        "soil_km": d["soil_km"].astype(np.float64),
        "vegetation": d["vegetation"].astype(np.float64),
        "fertility": d["fertility"].astype(np.float64),
        "permafrost_frac": d["permafrost"].astype(np.float64),
        "continental_frac": d["craton"].astype(np.float64),
        "seafloor_age_myr": d["seafloor_age_myr"].astype(np.float64),
    }
    koppen = d["koppen"].astype(np.int16)
    extras = {
        "ocean": ocean,
        "lake": lake,
        "lake_salt": d["lake_salt"].astype(bool),
        "koppen": np.where(ocean | (koppen < 0), 255, koppen).astype(np.uint8),
        "plate": d["plate"].astype(np.uint8),
        "crust_type": d["craton"].astype(np.uint8),
        "spacing_km": spacing_km,
    }
    notes = {"temp_first_harmonic_variance_explained": t_expl}

    cache = os.path.join(run_dir, "cache")
    tect_p = os.path.join(cache, "tectonics.pkl")
    tect = None
    if os.path.exists(tect_p):
        with open(tect_p, "rb") as f:
            tect = pickle.load(f)["tect"]
    if tect is not None and not (np.array_equal(np.asarray(tect["plate"]), d["plate"])
                                 and np.array_equal(np.asarray(tect["craton"]), d["craton"])):
        print("tectonics cache does not match planet_data.npz (plates/craton differ); skipped")
        tect = None
    if tect is not None:
        cont["erodibility"] = np.asarray(tect["erodibility"], np.float64)
        cont["volcanic_frac"] = np.asarray(tect["volcanic"], np.float64)
        cont["uplift"] = np.asarray(tect["uplift"], np.float64)
        cont["subsidence"] = np.asarray(tect["subsidence"], np.float64)
        notes["tectonics_cache"] = tect_p
        notes["tectonics_cache_note"] = (
            "erodibility, volcanic_frac, uplift and subsidence come from the tectonics "
            "stage (before erosion). Rock hardness itself is not cached; erodibility = "
            "clip(1/max(hardness, 0.3), 0.4, 2.5), so hardness ~ 1/erodibility.")
    else:
        notes["tectonics_cache"] = None
        print("no tectonics cache: erodibility, volcanic_frac, uplift, subsidence skipped")
    hyd_p = os.path.join(cache, "hydrology.pkl")
    if os.path.exists(hyd_p):
        with open(hyd_p, "rb") as f:
            hyd = pickle.load(f)["hydro"]
        lvl = np.asarray(hyd["lake_level"], np.float64)
        hl = np.asarray(hyd["lake_id"]) >= 0
        if np.array_equal(hl, lake) and np.isfinite(lvl[lake]).all():
            extras["lake_level"] = lvl
            notes["hydrology_cache"] = hyd_p
        else:
            print("hydrology cache lakes do not match planet_data.npz; "
                  "lake_level_km skipped")
    return xyz, cont, extras, notes


# ----------------------------------------------------------------------------
# field catalogue: name -> (units, clamp, description)
# ----------------------------------------------------------------------------

F32_FIELDS = {
    "elevation_km": ("km", None,
                     "Surface elevation relative to sea level at the source radius. "
                     "Land cell centres >= 0, sea cell centres < 0 (coast-preserving)."),
    "rough_km": ("km", (0.0, None),
                 "Mean |elevation difference| from a planetgen cell to its graph "
                 "neighbours (~30 km), as in bake_faces.py; sub-grid relief dial."),
    "temp_mean_c": ("degC", None, "Annual mean 2 m air temperature."),
    "temp_season_cos_c": ("degC", None,
                          "Annual harmonic: T(t) ~ temp_mean_c + cos_c*cos(2 pi t) + "
                          "sin_c*sin(2 pi t), t = year fraction (0 = start of month 0)."),
    "temp_season_sin_c": ("degC", None, "See temp_season_cos_c."),
    "temp_season_amp_c": ("degC", None,
                          "hypot(cos_c, sin_c): half the warmest-coldest swing of the "
                          "annual harmonic."),
    "precip_mm_yr": ("mm/yr", (0.0, None), "Annual precipitation."),
    "flow_log10": ("log10(1 + m^3/s)", (0.0, None),
                   "River discharge accumulated through planetgen's drainage graph, "
                   "smoothed like every field: a regional 'how much river water is "
                   "near' signal, not channel geometry (that is the receiver graph)."),
    "ocean_frac": ("fraction", (0.0, 1.0),
                   "Smoothed ocean flag; 0.5 contour follows the cell coastline."),
    "lake_frac": ("fraction", (0.0, 1.0), "Smoothed lake flag."),
    "lake_level_km": ("km", None,
                      "Water surface level of the nearest lake cell's lake (flat per "
                      "lake). Defined everywhere; meaningful only where lake_frac > 0."),
    "soil_km": ("km", (0.0, None), "Soil/sediment depth."),
    "vegetation": ("fraction", (0.0, 1.0), "Vegetation density (Miami-model NPP)."),
    "fertility": ("fraction", (0.0, 1.0), "Soil fertility."),
    "permafrost_frac": ("fraction", (0.0, 1.0), "Smoothed permafrost flag."),
    "continental_frac": ("fraction", (0.0, 1.0),
                         "Smoothed continental-crust (craton) flag; 0 = oceanic crust."),
    "seafloor_age_myr": ("Myr", (0.0, None),
                         "Oceanic crust age (0 on continental crust)."),
    "erodibility": ("relative", (0.4, 2.5),
                    "Rock erodibility from tectonics (1/hardness; high = soft rock)."),
    "volcanic_frac": ("fraction", (0.0, 1.0),
                      "Smoothed volcanic-arc / rift mask from tectonics."),
    "uplift": ("relative (0..1.2)", (0.0, 1.2),
               "Active tectonic uplift from tectonics (orogens and arcs, ridge-noise "
               "modulated); drove re-uplift during erosion. Young, deformed rock."),
    "subsidence": ("relative (0..1.2)", (0.0, 1.2),
                   "Sustained subsidence from tectonics: rift floors and cratonic sags, "
                   "i.e. sedimentary basins."),
}

U8_FIELDS = {
    "koppen": ("class", "Koppen-Geiger class index (planet.json koppen_classes); "
                        "255 = ocean. Nearest cell."),
    "plate": ("id", "Tectonic plate id (planet.json features.plates). Nearest cell."),
    "crust_type": ("class", "0 = oceanic, 1 = continental. Nearest cell."),
    "lake_salt": ("flag", "1 if the nearest lake cell's lake is salt (endorheic), "
                          "else 0. Meaningful only where lake_frac > 0."),
}


# ----------------------------------------------------------------------------
# coast-preserving pre-compensation
# ----------------------------------------------------------------------------

def coast_precompensation(ip, xyz, elev, ocean, max_iter=200):
    """Edit elevation at the few cells whose land/sea sign MLS would flip.

    Finds delta (nonzero only near coasts) so that MLS(elev + delta) at each
    cell centre is >= min(elev/2, 10 m) on land and <= max(elev/2, -10 m) at
    sea: the sign survives with a small margin. Jacobi iterations on the
    violators, using each cell's own MLS weight.
    """
    n = len(elev)
    idx, phi, _ = ip.weights(xyz)
    ok = idx < n
    rows = np.nonzero(ok)[0]
    W = csr_matrix((phi[ok], (rows, idx[ok])), shape=(n, n))
    self_w = W.diagonal()
    land = ~ocean
    bound = np.where(land, np.minimum(0.5 * elev, COAST_MARGIN_KM),
                     np.maximum(0.5 * elev, -COAST_MARGIN_KM))
    tol = 1e-9
    delta = np.zeros(n)
    initial = None
    it = 0
    for it in range(max_iter):
        eh = W @ (elev + delta)
        viol = np.where(land, eh < bound - tol, eh > bound + tol)
        if initial is None:
            initial = int(((land & (eh < 0)) | (ocean & (eh >= 0))).sum())
        if not viol.any():
            break
        delta[viol] += (bound[viol] - eh[viol]) / self_w[viol]
    eh = W @ (elev + delta)
    wrong = int(((land & (eh < 0)) | (ocean & (eh >= 0))).sum())
    res = eh - elev
    stats = dict(iterations=it + 1, sign_flips_before=initial, sign_flips_after=wrong,
                 cells_adjusted=int((delta != 0).sum()),
                 max_abs_delta_km=float(np.abs(delta).max()),
                 cell_residual_rms_m=float(np.sqrt((res ** 2).mean()) * 1000),
                 cell_residual_median_abs_m=float(np.median(np.abs(res)) * 1000),
                 cell_residual_p99_abs_m=float(np.percentile(np.abs(res), 99) * 1000),
                 cell_residual_max_abs_m=float(np.abs(res).max() * 1000))
    return delta, stats, W


# ----------------------------------------------------------------------------
# bake
# ----------------------------------------------------------------------------

def git_head():
    try:
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            cwd=os.path.dirname(os.path.abspath(__file__)),
            stderr=subprocess.DEVNULL).decode().strip()
    except Exception:
        return None


def bake(args):
    t_start = time.time()
    run_dir = os.path.abspath(args.run_dir)
    out_dir = os.path.abspath(args.out)
    os.makedirs(out_dir, exist_ok=True)
    res = args.res

    xyz, cont, extras, notes = load_cells(run_dir)
    n_cells = len(xyz)
    spacing = extras["spacing_km"]
    radius_km = args.support * spacing
    print(f"{n_cells} cells, mean neighbour spacing {spacing:.2f} km; "
          f"MLS degree 2, Wendland C4 support {radius_km:.1f} km", flush=True)
    tree = cKDTree(xyz)
    ip = wi.Interpolator(xyz, radius_km, 2, kdtree=tree, kmax=args.kmax)

    # warm the numba kernels before timing anything else
    ip(xyz[:16], cont["elevation_km"])
    t_load = time.time() - t_start

    coast = None
    if not args.no_coast_fix:
        t0 = time.time()
        delta, coast, _ = coast_precompensation(ip, xyz, cont["elevation_km"],
                                                extras["ocean"])
        cont["elevation_km"] = cont["elevation_km"] + delta
        coast["seconds"] = time.time() - t0
        print(f"coast fix: {coast['sign_flips_before']} cell signs would flip; "
              f"adjusted {coast['cells_adjusted']} cells in {coast['iterations']} "
              f"iterations (max |delta| {coast['max_abs_delta_km']:.2f} km); "
              f"flips after {coast['sign_flips_after']}; elevation at cell centres: "
              f"median |err| {coast['cell_residual_median_abs_m']:.1f} m, "
              f"RMS {coast['cell_residual_rms_m']:.1f} m", flush=True)

    # field order: f32 first (4-byte aligned), then u8
    f32_names = [k for k in F32_FIELDS if k in cont or k == "temp_season_amp_c"
                 or (k == "lake_level_km" and "lake_level" in extras)]
    mls_names = [k for k in cont]           # everything MLS'd, in a fixed order
    u8_names = list(U8_FIELDS)
    values = np.ascontiguousarray(np.stack([cont[k] for k in mls_names], 1))
    # A bounded field is clamped to its physical range intersected with the
    # range the source data actually spans: quadratic MLS overshoots steps,
    # and a bounded field should never leave what planetgen produced.
    # Unbounded fields (elevation, temperature) are left alone.
    clamps = {}
    for j, k in enumerate(mls_names):
        decl = F32_FIELDS[k][1]
        if decl is None:
            continue
        smin, smax = float(values[:, j].min()), float(values[:, j].max())
        lo = smin if decl[0] is None else max(decl[0], smin)
        hi = smax if decl[1] is None else min(decl[1], smax)
        clamps[k] = (lo, hi)

    lake_cells = np.flatnonzero(extras["lake"])
    lake_tree = cKDTree(xyz[lake_cells]) if len(lake_cells) else None

    npx = res * res
    layout = []
    off = 0
    for k in f32_names:
        layout.append((k, "f32", off))
        off += 4 * npx
    for k in u8_names:
        layout.append((k, "u8", off))
        off += npx
    face_bytes = off

    tan = wi.equal_angle_tangent(res)
    stats = {k: dict(min=np.inf, max=-np.inf, clamped_texels=0, worst_excursion=0.0)
             for k in f32_names}
    max_support = 0
    min_support = 10 ** 9
    t_faces = time.time()
    chunk = max(1, args.chunk_rows)
    for face in args.faces:
        tf = time.time()
        f32 = {k: np.empty(npx, np.float32) for k in f32_names}
        u8 = {k: np.empty(npx, np.uint8) for k in u8_names}
        for r0 in range(0, res, chunk):
            rows = np.arange(r0, min(res, r0 + chunk))
            q = wi.face_directions(face, tan, rows=rows).reshape(-1, 3)
            sl = slice(r0 * res, r0 * res + len(q))
            idx, phi, count = ip.weights(q)
            if np.isnan(phi).any():
                raise RuntimeError(f"singular MLS fit on face {face} rows {r0}+")
            max_support = max(max_support, int(count.max()))
            min_support = min(min_support, int(count.min()))
            out = wi.apply_weights(phi, idx, values)
            for j, k in enumerate(mls_names):
                v = out[:, j]
                if k in clamps:
                    lo, hi = clamps[k]
                    below = v < lo
                    above = v > hi
                    st = stats[k]
                    # count real overshoot, not round-off around an exact 0 or 1
                    tol = 1e-6 * (hi - lo)
                    st["clamped_texels"] += int((v < lo - tol).sum() + (v > hi + tol).sum())
                    if below.any():
                        st["worst_excursion"] = max(st["worst_excursion"], float(lo - v[below].min()))
                    if above.any():
                        st["worst_excursion"] = max(st["worst_excursion"], float(v[above].max() - hi))
                    v = np.clip(v, lo, hi)
                f32[k][sl] = v
            f32["temp_season_amp_c"][sl] = np.hypot(f32["temp_season_cos_c"][sl],
                                                    f32["temp_season_sin_c"][sl])
            nearest = idx[:, 0]           # KD-tree results are sorted by distance
            u8["koppen"][sl] = extras["koppen"][nearest]
            u8["plate"][sl] = extras["plate"][nearest]
            u8["crust_type"][sl] = extras["crust_type"][nearest]
            if lake_tree is not None:
                _, li = lake_tree.query(q, k=1, workers=-1)
                lc = lake_cells[li]
                u8["lake_salt"][sl] = extras["lake_salt"][lc]
                if "lake_level_km" in f32:
                    f32["lake_level_km"][sl] = extras["lake_level"][lc]
            else:
                u8["lake_salt"][sl] = 0
                if "lake_level_km" in f32:
                    f32["lake_level_km"][sl] = 0.0
        for k in f32_names:
            stats[k]["min"] = min(stats[k]["min"], float(f32[k].min()))
            stats[k]["max"] = max(stats[k]["max"], float(f32[k].max()))
        path = os.path.join(out_dir, f"face_{face}.bin")
        with open(path, "wb") as fh:
            for k in f32_names:
                fh.write(f32[k].astype("<f4").tobytes())
            for k in u8_names:
                fh.write(u8[k].tobytes())
        assert os.path.getsize(path) == face_bytes
        print(f"face {face}: {time.time() - tf:.1f} s  elev "
              f"{f32['elevation_km'].min():.2f}..{f32['elevation_km'].max():.2f} km, "
              f"ocean {(f32['ocean_frac'] > 0.5).mean() * 100:.0f}%", flush=True)
    t_faces = time.time() - t_faces

    planet_json = os.path.join(run_dir, "planet.json")
    planet_name, seed = None, None
    if os.path.exists(planet_json):
        with open(planet_json, encoding="utf-8") as f:
            pj = json.load(f)
        planet_name, seed = pj.get("planet"), pj.get("seed")

    fields = []
    for name, dtype, offset in layout:
        if dtype == "f32":
            units, clamp, desc = F32_FIELDS[name]
            if name == "lake_level_km":
                method = "nearest lake cell (hydrology cache)"
            elif name == "temp_season_amp_c":
                method = "hypot of the interpolated harmonic components"
            else:
                method = "mls2"
            entry = dict(name=name, type="f32", units=units, offset=offset,
                         bytes=4 * npx, method=method, description=desc,
                         min=stats[name]["min"], max=stats[name]["max"])
            if name in clamps:
                entry["clamp"] = list(clamps[name])
                entry["clamped_texels"] = stats[name]["clamped_texels"]
                entry["worst_excursion_before_clamp"] = stats[name]["worst_excursion"]
        else:
            units, desc = U8_FIELDS[name]
            entry = dict(name=name, type="u8", units=units, offset=offset, bytes=npx,
                         method="nearest cell" if name != "lake_salt"
                         else "nearest lake cell", description=desc)
        fields.append(entry)

    meta = dict(
        format=FORMAT,
        format_version=FORMAT_VERSION,
        planet=planet_name,
        seed=seed,
        source=dict(path=os.path.join(run_dir, "planet_data.npz"), cells=n_cells,
                    radius_km=wi.SOURCE_RADIUS_KM,
                    mean_neighbour_spacing_km=spacing, **notes),
        layout=dict(
            projection="equal_angle",
            mapping="a = tan(p*pi/4), b = tan(q*pi/4); "
                    "direction = normalize(axis + a*right + b*up)",
            faces=[dict(face=i, axis=list(ax), right=list(rt), up=list(up))
                   for i, (ax, rt, up) in enumerate(wi.FACES)],
            faces_source="viewer/src/planet.rs FACES",
            resolution=res,
            edge_inclusive=True,
            texel_param="p = -1 + 2*col/(res-1) (col walks right), "
                        "q = -1 + 2*row/(res-1) (row walks up; row 0 = bottom edge)",
            order="row-major: index = row*res + col; planar layers",
            endianness="little",
            file_pattern="face_{face}.bin",
            face_bytes=face_bytes,
        ),
        sampling=dict(
            recommended="C1 or smoother: Catmull-Rom (interpolating, C1) or uniform "
                        "cubic B-spline (C2). Bilinear brings back texel creases.",
            categorical="nearest texel; never blend class ids",
        ),
        interpolation=dict(
            method="moving least squares, degree 2, local tangent plane",
            weight="Wendland C4: (1-q)^6 (35 q^2 + 18 q + 3), q = chord/support",
            support_km=radius_km,
            support_over_spacing=args.support,
            coast_precompensation=coast,
        ),
        fields=fields,
        bake=dict(script="scripts/bake_world.py", git_head=git_head(),
                  seconds_total=None, seconds_load_and_setup=t_load,
                  seconds_faces=t_faces, faces=list(args.faces),
                  mls_support_cells_min=min_support, mls_support_cells_max=max_support),
    )
    meta["bake"]["seconds_total"] = time.time() - t_start
    with open(os.path.join(out_dir, "meta.json"), "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=1)
    print(f"baked {len(args.faces)} faces at {res}^2 -> {out_dir} "
          f"({face_bytes / 2**20:.0f} MiB/face) in {meta['bake']['seconds_total']:.1f} s",
          flush=True)
    return meta


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--run-dir", default="output/seed42_r8",
                    help="planetgen output directory holding planet_data.npz")
    ap.add_argument("--out", default="game/assets/world/seed42")
    ap.add_argument("--res", type=int, default=1024)
    ap.add_argument("--support", type=float, default=2.75,
                    help="MLS support radius as a multiple of mean neighbour spacing")
    ap.add_argument("--kmax", type=int, default=64,
                    help="KD-tree candidates per texel (the bake refuses if too few)")
    ap.add_argument("--chunk-rows", type=int, default=128)
    ap.add_argument("--faces", type=int, nargs="+", default=list(range(6)))
    ap.add_argument("--no-coast-fix", action="store_true",
                    help="skip the coast-preserving elevation pre-compensation")
    args = ap.parse_args()
    if args.res < 2:
        ap.error("--res must be >= 2")
    bake(args)


if __name__ == "__main__":
    main()

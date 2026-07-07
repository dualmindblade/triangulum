"""Feature naming and the Chronicle of the World.

Identifies the planet's named geography — continents, oceans, mountain
ranges, rivers, lakes, deserts, island chains — and writes a markdown
creation story stitching the tectonic history, hotspots, climate, and
hydrology into a narrative. All names are deterministic per seed.
"""

from __future__ import annotations

import numpy as np
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components

from .biomes import KOPPEN

ONSETS = ["", "b", "br", "c", "d", "dr", "f", "g", "gr", "h", "k", "kh", "l",
          "m", "n", "p", "r", "s", "sh", "st", "t", "th", "v", "vr", "y", "z"]
VOWELS = ["a", "e", "i", "o", "u", "a", "e", "o", "ae", "ia", "ei", "ou"]
CODAS = ["", "", "l", "n", "r", "s", "th", "m", "nd", "sh", "rk", "st"]


class Namer:
    def __init__(self, seed):
        self.rng = np.random.default_rng([seed, 777])
        self.used = set()

    def word(self, syllables=None):
        for _ in range(64):
            n = syllables or self.rng.choice([2, 2, 2, 3, 3])
            parts = []
            for i in range(n):
                parts.append(str(self.rng.choice(ONSETS)))
                parts.append(str(self.rng.choice(VOWELS)))
            parts.append(str(self.rng.choice(CODAS)))
            w = "".join(parts).capitalize()
            if 4 <= len(w) <= 11 and w not in self.used:
                self.used.add(w)
                return w
        return w  # pragma: no cover

    def styled(self, kind):
        w = self.word()
        r = self.rng.random()
        forms = {
            "continent": [w, w, f"{w}a", f"Great {w}"],
            "ocean": [f"{w} Ocean", f"Sea of {w}", f"{w} Sea"],
            "range": [f"{w} Mountains", f"{w} Range", f"Mountains of {w}", f"{w} Peaks"],
            "old_range": [f"{w} Hills", f"{w} Highlands", f"Old {w} Range"],
            "river": [f"River {w}", f"{w} River", f"The {w}"],
            "lake": [f"Lake {w}", f"{w} Lake"],
            "salt_lake": [f"{w} Salt Sea", f"Dry Lake {w}", f"{w} Basin"],
            "desert": [f"{w} Desert", f"{w} Wastes", f"Sands of {w}"],
            "isles": [f"{w} Isles", f"{w} Islands", f"{w} Archipelago"],
            "peak": [f"Mount {w}", f"{w} Peak"],
            "plate": [f"{w} Plate"],
            "era": [f"the {w} Era", f"the Age of {w}"],
        }
        opts = forms[kind]
        return opts[int(r * len(opts)) % len(opts)]


def _components(grid, mask, min_cells=1):
    """Connected components of a cell mask -> list of index arrays, biggest first."""
    cells = np.flatnonzero(mask)
    if len(cells) == 0:
        return []
    remap = np.full(grid.n, -1)
    remap[cells] = np.arange(len(cells))
    src, dst = [], []
    for s in range(grid.nbr.shape[1]):
        j = grid.nbr[cells, s]
        good = (j >= 0) & mask[np.maximum(j, 0)]
        src.append(remap[cells[good]])
        dst.append(remap[j[good]])
    adj = coo_matrix((np.ones(sum(len(x) for x in src)),
                      (np.concatenate(src), np.concatenate(dst))),
                     shape=(len(cells), len(cells)))
    _, labels = connected_components(adj, directed=False)
    comps = [cells[labels == c] for c in range(labels.max() + 1)]
    comps = [c for c in comps if len(c) >= min_cells]
    comps.sort(key=lambda c: -grid.area_km2[c].sum())
    return comps


def _river_donors(rcv, acc, min_acc):
    donors = {}
    for i in np.flatnonzero(acc >= min_acc):
        if rcv[i] != i:
            donors.setdefault(rcv[i], []).append(i)
    return donors


def _trace_river(grid, donors, acc, mouth):
    """Walk upstream from a mouth along the highest-discharge donor; returns
    (path, length_km) measured along actual cell-to-cell arcs."""
    path = [mouth]
    length = 0.0
    cur = mouth
    while True:
        ds = donors.get(cur, [])
        ds = [d for d in ds if d != cur]
        if not ds:
            break
        nxt = max(ds, key=lambda d: acc[d])
        length += grid.radius_km * float(
            np.arccos(np.clip(grid.xyz[cur] @ grid.xyz[nxt], -1, 1)))
        cur = nxt
        path.append(cur)
    return path[::-1], length


def _latlon_str(grid, cell):
    la, lo = np.degrees(grid.lat[cell]), np.degrees(grid.lon[cell])
    return f"{abs(la):.0f}{'N' if la >= 0 else 'S'} {abs(lo):.0f}{'E' if lo >= 0 else 'W'}"


def build(grid, cfg, tect, hydro, clim, koppen):
    namer = Namer(cfg.seed)
    elev = hydro["elev"]
    is_ocean = hydro["is_ocean"]
    area = grid.area_km2
    features = dict(planet=cfg.name or namer.word(2), continents=[], oceans=[],
                    ranges=[], rivers=[], lakes=[], deserts=[], isles=[], plates=[])

    # continents & major islands
    land_comps = _components(grid, ~is_ocean, min_cells=5)
    total_land = area[~is_ocean].sum()
    for c in land_comps[:8]:
        a = area[c].sum()
        kind = "continent" if a > 0.04 * total_land else "isles"
        name = namer.styled("continent" if kind == "continent" else "isles")
        peak = c[np.argmax(elev[c])]
        features["continents" if kind == "continent" else "isles"].append(dict(
            name=name, area_km2=float(a), cells=c, peak_cell=int(peak),
            peak_km=float(elev[peak]), where=_latlon_str(grid, c[len(c) // 2])))

    # oceans
    for c in _components(grid, is_ocean, min_cells=50)[:5]:
        features["oceans"].append(dict(name=namer.styled("ocean"),
                                       area_km2=float(area[c].sum()), cells=c,
                                       deepest_km=float(elev[c].min())))

    # mountain ranges: young orogens vs ancient eroded ones
    oro = tect["orogen_field"]
    for c in _components(grid, (oro > 1.2) & ~is_ocean, min_cells=4)[:7]:
        peak = c[np.argmax(elev[c])]
        features["ranges"].append(dict(name=namer.styled("range"), young=True,
                                       cells=c, peak_cell=int(peak), peak_km=float(elev[peak]),
                                       peak_name=namer.styled("peak"),
                                       where=_latlon_str(grid, peak)))
    era = tect["era_field"]
    for c in _components(grid, (era > 0.35) & ~is_ocean & (oro < 0.6), min_cells=6)[:5]:
        peak = c[np.argmax(elev[c])]
        features["ranges"].append(dict(name=namer.styled("old_range"), young=False,
                                       cells=c, peak_cell=int(peak), peak_km=float(elev[peak]),
                                       peak_name=None, where=_latlon_str(grid, peak)))

    # rivers: biggest mouths
    rcv = hydro["receiver"]
    acc = hydro["flow_acc"]
    river = hydro["river"]
    mouth_mask = river & (elev[rcv] < 0)
    mouths = np.flatnonzero(mouth_mask)
    mouths = mouths[np.argsort(-acc[mouths])][:10]
    donors = _river_donors(rcv, acc, min_acc=min(cfg.river_min_m3s * 0.3, 100.0))
    for m in mouths:
        path, length = _trace_river(grid, donors, acc, m)
        features["rivers"].append(dict(name=namer.styled("river"), mouth=int(m),
                                       cells=np.array(path), length_km=float(length),
                                       discharge_m3s=float(acc[m]),
                                       where=_latlon_str(grid, m)))

    # lakes
    lakes_sorted = sorted(hydro["lakes"], key=lambda l: -l["area_km2"])[:8]
    lake_id = hydro["lake_id"]
    for l in lakes_sorted:
        cells = np.flatnonzero(lake_id == l["id"])
        features["lakes"].append(dict(
            name=namer.styled("salt_lake" if l["salt"] else "lake"),
            cells=cells, area_km2=l["area_km2"], salt=l["salt"],
            depth_m=l["deepest"] * 1000.0, where=_latlon_str(grid, cells[0])))

    # deserts (BW classes)
    desert_mask = np.isin(koppen, [3, 4])
    for c in _components(grid, desert_mask, min_cells=8)[:5]:
        features["deserts"].append(dict(name=namer.styled("desert"),
                                        area_km2=float(area[c].sum()), cells=c,
                                        where=_latlon_str(grid, c[len(c) // 2])))

    # plates & eras
    for p in tect["plate_table"]:
        p = dict(p)
        p["name"] = namer.styled("plate")
        features["plates"].append(p)
    features["eras"] = [dict(name=namer.styled("era"), **e) for e in tect["era_meta"]]
    features["hotspot_isles"] = [namer.styled("isles") for _ in tect["hotspots"]]

    return features


def write_markdown(grid, cfg, features, tect, hydro, clim, koppen, path):
    elev = hydro["elev"]
    is_ocean = hydro["is_ocean"]
    area = grid.area_km2
    land_frac = area[~is_ocean].sum() / area.sum()
    planet = features["planet"]
    conts = features["continents"]
    oceans = features["oceans"]
    young = [r for r in features["ranges"] if r["young"]]
    old = [r for r in features["ranges"] if not r["young"]]
    rivers = features["rivers"]
    lakes = features["lakes"]
    deserts = features["deserts"]
    T_ann = clim["T_ann"]
    P_ann = clim["P_ann"]

    hi_cell = int(np.argmax(elev))
    deep_cell = int(np.argmin(elev))
    hot_cell = int(np.argmax(np.where(is_ocean, -99, T_ann)))
    cold_cell = int(np.argmin(np.where(is_ocean, 99, T_ann)))
    wet_cell = int(np.argmax(np.where(is_ocean, -1, P_ann)))

    def w(cell):
        return _latlon_str(grid, cell)

    lines = []
    a = lines.append
    a(f"# The Chronicle of {planet}")
    a("")
    a(f"*Generated with seed {cfg.seed} — every name, mountain, and river below "
      f"follows deterministically from that number.*")
    a("")
    a("## I. The Forging")
    a("")
    a(f"In the beginning {planet} was a world of magma and steam. As the crust cooled "
      f"and the first oceans condensed, the mantle beneath kept churning, and the "
      f"young lithosphere shattered into drifting plates. Nothing of that first age "
      f"survives on the surface — but its consequences are everywhere.")
    a("")
    for i, era in enumerate(features["eras"]):
        which = "first" if i == 0 else "second"
        old_in_era = old[i::len(features['eras'])] if old else []
        a(f"### {era['name'].title()}")
        a("")
        s = (f"In {era['name']}, the {which} of the remembered ages, {planet} was divided among "
             f"{era['n_plates']} ancient plates whose names are lost. Where they collided, "
             f"mountains rose that have long since worn down to their roots.")
        if old_in_era:
            names = ", ".join(r["name"] for r in old_in_era)
            s += f" Their stumps endure today as {names} — low, rounded highlands far from any active margin."
        a(s)
        a("")
    a("## II. The Age of the Present Plates")
    a("")
    plates = sorted(features["plates"], key=lambda p: -p["area_km2"])
    cont_plates = [p for p in plates if p["continental"]]
    oce_plates = [p for p in plates if not p["continental"]]
    a(f"Today the lithosphere of {planet} is divided among {len(plates)} major plates. "
      f"{len(cont_plates)} of them — {', '.join(p['name'] for p in cont_plates[:4])}"
      f"{'…' if len(cont_plates) > 4 else ''} — carry continental crust; the rest, led by the vast "
      f"{oce_plates[0]['name'] if oce_plates else plates[0]['name']}, are cold ocean floor. "
      f"The fastest of them creeps along at {max(p['speed_km_myr'] for p in plates):.0f} km per "
      f"million years.")
    a("")
    if young:
        a("Where the plates drive together, the land answers:")
        a("")
        for r in young[:5]:
            pk = f", crowned by {r['peak_name']} at {r['peak_km']:.1f} km" if r["peak_name"] else ""
            a(f"- **{r['name']}** ({r['where']}) — a young, still-rising range{pk}.")
        a("")
    isles = features["hotspot_isles"]
    if isles:
        a(f"And far from any boundary, plumes of deep fire have punched chains of volcanic "
          f"islands through the moving plates — among them {', '.join(isles[:3])}"
          f"{'…' if len(isles) > 3 else ''} — each chain a trail of extinct volcanoes "
          f"recording its plate's drift.")
        a("")
    a("## III. The Lands and the Waters")
    a("")
    if conts:
        biggest = conts[0]
        a(f"The waters of {planet} gathered into great basins, flooding {100 * (1 - land_frac):.0f}% "
          f"of the world, and the dry land assembled into its continents. Mightiest is "
          f"**{biggest['name']}** ({biggest['area_km2'] / 1e6:.1f} million km², around {biggest['where']})"
          + (f", with {len(conts)} great landmasses in all." if len(conts) > 1 else "."))
        for cont in conts[1:]:
            a(f"- **{cont['name']}** — {cont['area_km2'] / 1e6:.1f} million km², around {cont['where']}.")
        a("")
    if oceans:
        a(f"Between them roll {len(oceans)} named oceans, greatest of them **{oceans[0]['name']}**, "
          f"whose deepest trench reaches {-oceans[0]['deepest_km']:.1f} km below the waves.")
        a("")
    if rivers:
        r0 = rivers[0]
        a(f"Rain that falls on the highlands finds its way back to the sea along the great rivers. "
          f"The mightiest, **{r0['name']}**, pours {r0['discharge_m3s']:,.0f} m³/s into the ocean at "
          f"{r0['where']} after a course of roughly {r0['length_km']:,.0f} km.")
        for r in rivers[1:5]:
            a(f"- **{r['name']}** — {r['discharge_m3s']:,.0f} m³/s at {r['where']}.")
        a("")
    if lakes:
        a("Where the land holds water back, lakes have pooled:")
        for l in lakes[:5]:
            kind = "a salt sea with no outlet" if l["salt"] else "a freshwater lake"
            a(f"- **{l['name']}** ({l['where']}) — {kind}, {l['area_km2']:,.0f} km², "
              f"up to {l['depth_m']:.0f} m deep.")
        a("")
    a("## IV. The Climates")
    a("")
    koppen_land = koppen[koppen >= 0]
    counts = np.bincount(koppen_land, minlength=len(KOPPEN))
    top = np.argsort(-counts)[:6]
    clim_desc = ", ".join(f"{KOPPEN[i][1]} ({KOPPEN[i][0]})" for i in top if counts[i] > 0)
    a(f"The winds and currents of {planet} deal out its climates unevenly. The most widespread "
      f"climate types are: {clim_desc}.")
    a("")
    if deserts:
        a(f"The largest desert, **{deserts[0]['name']}** ({deserts[0]['where']}), spans "
          f"{deserts[0]['area_km2'] / 1e6:.1f} million km² where the subtropical air sinks dry"
          + (f"; {len(deserts) - 1} other great wastes accompany it." if len(deserts) > 1 else "."))
        a("")
    a("### Vital statistics")
    a("")
    a(f"| | |")
    a(f"|---|---|")
    a(f"| Land fraction | {land_frac * 100:.1f}% |")
    a(f"| Highest point | {elev[hi_cell]:.2f} km ({w(hi_cell)}) |")
    a(f"| Deepest trench | {elev[deep_cell]:.2f} km ({w(deep_cell)}) |")
    a(f"| Hottest place (annual) | {T_ann[hot_cell]:.1f} C ({w(hot_cell)}) |")
    a(f"| Coldest place (annual) | {T_ann[cold_cell]:.1f} C ({w(cold_cell)}) |")
    a(f"| Wettest place | {P_ann[wet_cell]:,.0f} mm/yr ({w(wet_cell)}) |")
    a(f"| Number of mapped rivers | {int(hydro['river'].sum())} river cells |")
    a(f"| Number of lakes | {len(hydro['lakes'])} |")
    a("")
    a("*So runs the chronicle; the rest is for its inhabitants to write.*")

    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    return path

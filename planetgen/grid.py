"""Sphere grids: Fibonacci-Delaunay (default) and subdivided icosahedron.

The planet is discretized as ~41k cells (level 6, ~165 km spacing) up to
~655k (level 8). Two constructions:

* "fibonacci": points on a Fibonacci spiral lattice, jittered, triangulated
  by their convex hull (which on a sphere IS the Delaunay triangulation; its
  dual is the Voronoi diagram). No global structure at all — the grid cannot
  imprint faces, seams, or preferred directions on the simulations. Cell
  degree varies (mostly 6, some 5/7).
* "icosphere": subdivided icosahedron, hexagonal cells + 12 pentagons. More
  regular; its 20 faces and 6 lattice directions can ghost into operators
  (mitigated but not eliminated by jitter). Kept for comparison.

Provides the graph operators the simulations need: gradients, Laplacian,
divergence, upwind advection, multi-source geodesic distance / nearest-source
propagation, and samplers for rendering and for the future game to query.
"""

from __future__ import annotations

import numpy as np
from scipy.spatial import ConvexHull, cKDTree

PHI = (1.0 + np.sqrt(5.0)) / 2.0


def _fibonacci_points(n):
    """n points on a Fibonacci (golden-angle) spiral over the sphere."""
    i = np.arange(n, dtype=np.float64) + 0.5
    z = 1.0 - 2.0 * i / n
    r = np.sqrt(np.maximum(1.0 - z * z, 0.0))
    theta = np.pi * (3.0 - np.sqrt(5.0)) * i
    return np.stack([r * np.cos(theta), r * np.sin(theta), z], 1)


def _base_icosahedron():
    v = np.array(
        [[-1, PHI, 0], [1, PHI, 0], [-1, -PHI, 0], [1, -PHI, 0],
         [0, -1, PHI], [0, 1, PHI], [0, -1, -PHI], [0, 1, -PHI],
         [PHI, 0, -1], [PHI, 0, 1], [-PHI, 0, -1], [-PHI, 0, 1]], dtype=np.float64)
    v /= np.linalg.norm(v, axis=1, keepdims=True)
    f = np.array(
        [[0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
         [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
         [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
         [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1]], dtype=np.int64)
    return v, f


def _subdivide(verts, faces):
    """One 4-to-1 subdivision pass, fully vectorized."""
    e = np.concatenate([faces[:, [0, 1]], faces[:, [1, 2]], faces[:, [2, 0]]])
    e = np.sort(e, axis=1)
    uniq, inv = np.unique(e, axis=0, return_inverse=True)
    mid = verts[uniq[:, 0]] + verts[uniq[:, 1]]
    mid /= np.linalg.norm(mid, axis=1, keepdims=True)
    midx = len(verts) + np.arange(len(uniq))
    nf = len(faces)
    m01, m12, m20 = midx[inv[:nf]], midx[inv[nf:2 * nf]], midx[inv[2 * nf:]]
    v0, v1, v2 = faces[:, 0], faces[:, 1], faces[:, 2]
    new_faces = np.concatenate([
        np.stack([v0, m01, m20], 1),
        np.stack([v1, m12, m01], 1),
        np.stack([v2, m20, m12], 1),
        np.stack([m01, m12, m20], 1)])
    return np.vstack([verts, mid]), new_faces


class Grid:
    def __init__(self, level: int, radius_km: float = 6371.0, jitter: float = 0.0,
                 seed: int = 0, kind: str = "fibonacci"):
        """jitter: tangential vertex displacement as a fraction of the mean
        cell spacing (seeded). It randomizes local edge orientations so grid
        regularity can't bias the operators — essential for the icosphere,
        still useful for the Fibonacci lattice (breaks its spiral rhythm)."""
        self.level = level
        self.kind = kind
        self.radius_km = radius_km
        n_target = 10 * 4 ** level + 2
        if kind == "fibonacci":
            verts = _fibonacci_points(n_target)
            faces = None
        elif kind == "icosphere":
            verts, faces = _base_icosahedron()
            for _ in range(level):
                verts, faces = _subdivide(verts, faces)
        else:
            raise ValueError(f"unknown grid kind: {kind}")
        if jitter > 0.0:
            rng = np.random.default_rng([seed, 4242])
            tang = rng.normal(size=verts.shape)
            tang -= verts * np.einsum("nd,nd->n", tang, verts)[:, None]
            tang /= np.linalg.norm(tang, axis=1, keepdims=True)
            spacing = np.sqrt(4.0 * np.pi / len(verts))
            verts = verts + tang * (jitter * spacing * rng.random(len(verts)))[:, None]
            verts /= np.linalg.norm(verts, axis=1, keepdims=True)
        if faces is None:
            faces = ConvexHull(verts).simplices.astype(np.int64)
        self.xyz = verts
        self.faces = faces
        self.n = len(verts)
        self._build_adjacency()
        self._build_geometry()
        self._kdtree = None
        self._eq_cache = {}

    # ------------------------------------------------------------------
    def _build_adjacency(self):
        f = self.faces
        de = np.concatenate([f[:, [0, 1]], f[:, [1, 2]], f[:, [2, 0]],
                             f[:, [1, 0]], f[:, [2, 1]], f[:, [0, 2]]])
        de = np.unique(de, axis=0)  # sorted by (src, dst)
        counts = np.bincount(de[:, 0], minlength=self.n)
        offsets = np.concatenate([[0], np.cumsum(counts)])
        self.max_deg = int(counts.max())
        nbr = np.full((self.n, self.max_deg), -1, dtype=np.int64)
        for slot in range(self.max_deg):
            has = counts > slot
            nbr[has, slot] = de[offsets[:-1][has] + slot, 1]
        self.nbr = nbr
        self.nbr_ok = nbr >= 0
        self.deg = counts.astype(np.float64)
        self.nbr_safe = np.where(self.nbr_ok, nbr, np.arange(self.n)[:, None])
        # reciprocal slot: nbr[nbr[i,s], recip_slot[i,s]] == i (needed for
        # exactly antisymmetric edge fluxes in advection); built slot-by-slot
        # to keep memory linear in max_deg
        ar = np.arange(self.n)
        self.recip_slot = np.zeros((self.n, self.max_deg), dtype=np.int64)
        for s in range(self.max_deg):
            j = self.nbr_safe[:, s]
            self.recip_slot[:, s] = (self.nbr[j] == ar[:, None]).argmax(1)

    def _build_geometry(self):
        xyz, R = self.xyz, self.radius_km
        z = xyz[:, 2]
        self.lat = np.arcsin(np.clip(z, -1, 1))
        self.lon = np.arctan2(xyz[:, 1], xyz[:, 0])

        # tangent basis (east, north); safe even though no cell sits exactly on a pole
        east = np.stack([-xyz[:, 1], xyz[:, 0], np.zeros(self.n)], 1)
        norm = np.linalg.norm(east, axis=1, keepdims=True)
        east = np.where(norm > 1e-9, east / np.maximum(norm, 1e-12), [1.0, 0.0, 0.0])
        north = np.cross(xyz, east)
        self.east, self.north = east, north

        # per-edge arc lengths (km) and unit tangent directions toward each neighbor
        nxyz = xyz[self.nbr_safe]                       # (n, 6, 3)
        dots = np.clip(np.einsum("nkd,nd->nk", nxyz, xyz), -1, 1)
        self.edge_km = np.where(self.nbr_ok, R * np.arccos(dots), np.inf)
        delta = nxyz - xyz[:, None, :]
        d_e = np.einsum("nkd,nd->nk", delta, east)
        d_n = np.einsum("nkd,nd->nk", delta, north)
        mag = np.sqrt(d_e**2 + d_n**2)
        mag = np.where(mag > 1e-12, mag, 1.0)
        self.dir_e = np.where(self.nbr_ok, d_e / mag, 0.0)  # (n, 6)
        self.dir_n = np.where(self.nbr_ok, d_n / mag, 0.0)

        # cell areas: one third of each adjacent face's area, rescaled to the sphere
        fv = xyz[self.faces]
        fa = 0.5 * np.linalg.norm(np.cross(fv[:, 1] - fv[:, 0], fv[:, 2] - fv[:, 0]), axis=1)
        area = np.zeros(self.n)
        np.add.at(area, self.faces[:, 0], fa / 3)
        np.add.at(area, self.faces[:, 1], fa / 3)
        np.add.at(area, self.faces[:, 2], fa / 3)
        area *= 4 * np.pi * R * R / area.sum()
        self.area_km2 = area
        self.mean_edge_km = float(self.edge_km[self.nbr_ok].mean())
        # symmetric per-edge area weight (harmonic mean) for conservative fluxes
        a_j = area[self.nbr_safe]
        self.edge_harm_area = np.where(self.nbr_ok, 2.0 / (1.0 / area[:, None] + 1.0 / a_j), 0.0)

    # ------------------------------------------------------------------
    # differential operators (all fields are (n,) or (n, 2) as east/north)
    def gradient(self, f):
        """Least-squares-ish gradient, returns (n, 2) in units of f per km."""
        df = np.where(self.nbr_ok, f[self.nbr_safe] - f[:, None], 0.0)
        slope = np.where(self.nbr_ok, df / self.edge_km, 0.0)
        ge = (slope * self.dir_e).sum(1) * (2.0 / self.deg)
        gn = (slope * self.dir_n).sum(1) * (2.0 / self.deg)
        return np.stack([ge, gn], 1)

    def laplacian(self, f):
        """Neighborhood mean minus self (dimensionless smoothing operator)."""
        s = np.where(self.nbr_ok, f[self.nbr_safe], 0.0).sum(1)
        return s / self.deg - f

    def divergence(self, v):
        """Divergence of a tangent field v (n, 2), per km."""
        ve, vn = v[:, 0], v[:, 1]
        fe = 0.5 * (ve[:, None] + ve[self.nbr_safe])
        fn = 0.5 * (vn[:, None] + vn[self.nbr_safe])
        flux = np.where(self.nbr_ok, (fe * self.dir_e + fn * self.dir_n) / self.edge_km, 0.0)
        return flux.sum(1) * (2.0 / self.deg)

    def advect(self, q, v, frac=0.3):
        """One conservative upwind advection step of scalar q by tangent field v.

        frac is the fraction of a cell width transported per step at the
        field's maximum speed (CFL-like stability control). Mass (q * area)
        is conserved to rounding because edge fluxes are antisymmetric.
        """
        ve, vn = v[:, 0], v[:, 1]
        fe = 0.5 * (ve[:, None] + ve[self.nbr_safe])
        fn = 0.5 * (vn[:, None] + vn[self.nbr_safe])
        vel = fe * self.dir_e + fn * self.dir_n            # (n, 6), >0 = outflow i->j
        vel = np.where(self.nbr_ok, vel, 0.0)
        # symmetrize against the neighbor's view of the same edge so that
        # flux(i->j) == -flux(j->i) exactly
        vel = 0.5 * (vel - vel[self.nbr_safe, self.recip_slot])
        vmax = np.abs(vel).max()
        if vmax < 1e-12:
            return q
        dt = frac * self.mean_edge_km / vmax / 3.0         # /3: several outflow edges
        w = np.where(self.nbr_ok, vel / self.edge_km, 0.0)
        q_j = q[self.nbr_safe]
        mass = dt * (np.maximum(w, 0.0) * q[:, None] + np.minimum(w, 0.0) * q_j) * self.edge_harm_area
        return np.maximum(q - mass.sum(1) / self.area_km2, 0.0)

    def advect_interp(self, q, v, frac=0.3):
        """Bounded (max-principle) upwind advection step — a weighted average
        of upwind values. Not conservative; use for temperature-like fields
        that must never overshoot (conservative advection piles mass into
        flow-convergence zones, which is wrong for SST)."""
        ve, vn = v[:, 0], v[:, 1]
        fe = 0.5 * (ve[:, None] + ve[self.nbr_safe])
        fn = 0.5 * (vn[:, None] + vn[self.nbr_safe])
        vel = fe * self.dir_e + fn * self.dir_n
        vel = np.where(self.nbr_ok, vel, 0.0)
        vmax = np.abs(vel).max()
        if vmax < 1e-12:
            return q
        dt = frac * self.mean_edge_km / vmax
        w_in = dt * np.maximum(-vel, 0.0) / self.edge_km
        num = q + (w_in * q[self.nbr_safe]).sum(1)
        return num / (1.0 + w_in.sum(1))

    # ------------------------------------------------------------------
    def distance_to(self, src_mask, edge_ok=None, max_iter=None, edge_scale=None):
        """Geodesic distance (km) to nearest source cell, by iterative relaxation."""
        d, _ = self.nearest_source(src_mask, edge_ok=edge_ok, max_iter=max_iter,
                                   edge_scale=edge_scale)
        return d

    def rough_metric(self, seed, amount=0.5):
        """Symmetric random per-edge length multipliers for distance transforms.

        Exact graph distances propagate as straight cones along the local
        lattice directions; every kernel built on them (mountain falloffs,
        crust tapers, seafloor age) inherits faceted striations. Roughening
        the metric turns those fronts irregular, like real geology.
        """
        rng = np.random.default_rng([seed, 31337])
        s = 1.0 + amount * (rng.random(self.n) - 0.5)
        return 0.5 * (s[:, None] + s[self.nbr_safe])

    def nearest_source(self, src_mask, edge_ok=None, max_iter=None, edge_scale=None):
        """Distance to and index of the nearest source cell.

        edge_ok: optional (n, max_deg) bool restricting which edges propagation
        may cross (e.g. stay within one tectonic plate). edge_scale: optional
        per-edge length multipliers (see rough_metric).
        """
        d = np.where(src_mask, 0.0, np.inf)
        lab = np.where(src_mask, np.arange(self.n), -1)
        edge = self.edge_km if edge_scale is None else self.edge_km * edge_scale
        if edge_ok is not None:
            edge = np.where(edge_ok, edge, np.inf)
        if max_iter is None:
            max_iter = int(np.pi * self.radius_km / self.mean_edge_km * 1.3) + 20
        for _ in range(max_iter):
            cand = d[self.nbr_safe] + edge                 # (n, 6)
            best = cand.min(1)
            improved = best < d - 1e-9
            if not improved.any():
                break
            slot = cand[improved].argmin(1)
            src_cells = self.nbr_safe[improved, slot]
            d = np.where(improved, best, d)
            lab[improved] = lab[src_cells]
        return d, lab

    # ------------------------------------------------------------------
    def kdtree(self):
        if self._kdtree is None:
            self._kdtree = cKDTree(self.xyz)
        return self._kdtree

    def equirect_index(self, width, height):
        """(height, width) array of nearest cell indices for map projection."""
        key = (width, height)
        if key not in self._eq_cache:
            lon = (np.arange(width) + 0.5) / width * 2 * np.pi - np.pi
            lat = np.pi / 2 - (np.arange(height) + 0.5) / height * np.pi
            LON, LAT = np.meshgrid(lon, lat)
            pts = np.stack([np.cos(LAT) * np.cos(LON),
                            np.cos(LAT) * np.sin(LON),
                            np.sin(LAT)], -1).reshape(-1, 3)
            _, idx = self.kdtree().query(pts, workers=-1)
            self._eq_cache[key] = idx.reshape(height, width)
        return self._eq_cache[key]

    def equirect_weights(self, width, height, k=4):
        """Inverse-distance interpolation stencil for smooth map projection."""
        key = (width, height, k)
        if key not in self._eq_cache:
            lon = (np.arange(width) + 0.5) / width * 2 * np.pi - np.pi
            lat = np.pi / 2 - (np.arange(height) + 0.5) / height * np.pi
            LON, LAT = np.meshgrid(lon, lat)
            pts = np.stack([np.cos(LAT) * np.cos(LON),
                            np.cos(LAT) * np.sin(LON),
                            np.sin(LAT)], -1).reshape(-1, 3)
            dist, idx = self.kdtree().query(pts, k=k, workers=-1)
            w = 1.0 / np.maximum(dist, 1e-9) ** 2
            w /= w.sum(1, keepdims=True)
            self._eq_cache[key] = (idx.reshape(height, width, k),
                                   w.reshape(height, width, k))
        return self._eq_cache[key]

    def project(self, field, width, height, smooth=True):
        """Project a cell field to an equirectangular image. Smooth uses IDW
        interpolation (continuous fields); otherwise nearest cell (categorical)."""
        if not smooth:
            return field[self.equirect_index(width, height)]
        idx, w = self.equirect_weights(width, height)
        return (field[idx] * w).sum(-1)

    def sample_latlon(self, lat_deg, lon_deg, k=3):
        """Interpolation weights for arbitrary points: returns (cells, weights)."""
        lat = np.radians(np.asarray(lat_deg, dtype=np.float64))
        lon = np.radians(np.asarray(lon_deg, dtype=np.float64))
        pts = np.stack([np.cos(lat) * np.cos(lon), np.cos(lat) * np.sin(lon), np.sin(lat)], -1)
        dist, idx = self.kdtree().query(pts, k=k, workers=-1)
        w = 1.0 / np.maximum(dist, 1e-9)
        w /= w.sum(axis=-1, keepdims=True)
        return idx, w

    def zonal_mean(self, f, n_bins=90, smooth=3):
        """Mean of f by latitude band, mapped back onto cells.

        Interpolates between bin centers — assigning each cell its bin's value
        creates a 2-degree staircase whose steps ring through any gradient
        taken of the result (visible as parallel bands in downstream fields).
        """
        edges = np.linspace(-np.pi / 2, np.pi / 2, n_bins + 1)
        centers = 0.5 * (edges[:-1] + edges[1:])
        which = np.clip(np.digitize(self.lat, edges) - 1, 0, n_bins - 1)
        sums = np.bincount(which, weights=f * self.area_km2, minlength=n_bins)
        wts = np.bincount(which, weights=self.area_km2, minlength=n_bins)
        prof = sums / np.maximum(wts, 1e-9)
        for _ in range(smooth):
            prof = np.convolve(np.pad(prof, 1, mode="edge"), [0.25, 0.5, 0.25], mode="valid")
        return np.interp(self.lat, centers, prof)

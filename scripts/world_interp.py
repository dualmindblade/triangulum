"""Scattered-cell -> sphere interpolation shared by bake_world.py and its checks.

The planetgen grid is a jittered Fibonacci sphere: ~655k cells, ~30 km
between graph neighbours on Neisor's 6,371 km source sphere. This module turns
per-cell values into a smooth function of direction on the unit sphere.

Method: moving least squares (MLS). At every query direction x:

  1. take every cell within a fixed great-circle-ish radius r (chord |p - x|),
  2. weight each by a Wendland C4 kernel  w(q) = (1-q)^6 (35q^2 + 18q + 3),
     q = |p - x| / r  (exactly zero at and beyond r, so the neighbour set can
     change without the result jumping),
  3. fit a polynomial of the given degree in local tangent-plane coordinates
     by weighted least squares, and
  4. return the fit's value at x.

Because the polynomial space is rotation invariant, the arbitrary choice of
tangent frame does not leak into the result, and because the weights are C4
and compactly supported, the result is C4 in x. Degree 2 reproduces any
locally quadratic field exactly, so slopes and curvature survive (no flat
"bullseye" at a cell centre, which inverse-distance weighting produces).
Degree 0 is a normalized-kernel (Shepard) average: a convex combination, so it
can never overshoot -- that is the right choice for 0..1 fractions.

The fit is linear in the data, so it is done once per query as a weight
vector phi_j (sum phi_j = 1) and applied to every field at once.
"""

import numpy as np
from numba import njit, prange

SOURCE_RADIUS_KM = 6371.0

# Face frames, verbatim from viewer/src/planet.rs FACES: (axis, right, up).
# direction(a, b) = normalize(axis + a*right + b*up)
FACES = (
    ((1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)),
    ((-1.0, 0.0, 0.0), (0.0, -1.0, 0.0), (0.0, 0.0, 1.0)),
    ((0.0, 1.0, 0.0), (-1.0, 0.0, 0.0), (0.0, 0.0, 1.0)),
    ((0.0, -1.0, 0.0), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0)),
    ((0.0, 0.0, 1.0), (0.0, 1.0, 0.0), (-1.0, 0.0, 0.0)),
    ((0.0, 0.0, -1.0), (0.0, 1.0, 0.0), (1.0, 0.0, 0.0)),
)


def equal_angle_tangent(res):
    """Tangent coordinate a = tan(p*pi/4) for edge-inclusive p in [-1, 1].

    Texel 0 is p = -1 and texel res-1 is p = +1, exactly on the face edges.
    tan(pi/4) is 0.9999999999999999 in floating point, so the end points (and
    the centre, for odd res) are pinned to their exact values, and the table
    is made exactly antisymmetric (a[res-1-i] == -a[i]) because some shared
    edges run in opposite directions on the two faces. Neighbouring faces then
    evaluate bit-identical directions along every shared edge.
    """
    i = np.arange(res, dtype=np.float64)
    p = (2.0 * i - (res - 1)) / (res - 1)
    a = np.tan(p * np.pi / 4.0)
    half = res // 2
    a[:half] = -a[::-1][:half]
    a[0], a[-1] = -1.0, 1.0
    if res % 2 == 1:
        a[half] = 0.0
    return a


def gnomonic_tangent(res):
    """The old bake's layout: a = p, edge inclusive."""
    return np.linspace(-1.0, 1.0, res)


def face_directions(face, tangent, rows=None):
    """Unit directions for a face raster, shape (len(rows), res, 3).

    Row index walks the face's `up` axis (row 0 = b = -1 edge) and column
    index walks `right` (col 0 = a = -1), matching bake_faces.py.
    """
    axis, right, up = (np.asarray(v, np.float64) for v in FACES[face])
    t = np.asarray(tangent, np.float64)
    b = t if rows is None else t[rows]
    d = (axis[None, None, :]
         + t[None, :, None] * right[None, None, :]
         + b[:, None, None] * up[None, None, :])
    return d / np.linalg.norm(d, axis=2, keepdims=True)


def direction_to_face(xyz):
    """Inverse map: unit directions (n, 3) -> (face, a, b) tangent coords."""
    xyz = np.asarray(xyz, np.float64)
    face = np.empty(len(xyz), np.int64)
    a = np.empty(len(xyz))
    b = np.empty(len(xyz))
    best = np.full(len(xyz), -np.inf)
    for f, (axis, right, up) in enumerate(FACES):
        axis, right, up = (np.asarray(v) for v in (axis, right, up))
        dot = xyz @ axis
        take = dot > best
        best[take] = dot[take]
        face[take] = f
    for f, (axis, right, up) in enumerate(FACES):
        axis, right, up = (np.asarray(v) for v in (axis, right, up))
        m = face == f
        dot = xyz[m] @ axis
        a[m] = (xyz[m] @ right) / dot
        b[m] = (xyz[m] @ up) / dot
    return face, a, b


# ----------------------------------------------------------------------------
# MLS kernel
# ----------------------------------------------------------------------------

@njit(cache=True)
def _wendland_c4(q):
    if q >= 1.0:
        return 0.0
    u = 1.0 - q
    u2 = u * u
    return u2 * u2 * u2 * (35.0 * q * q + 18.0 * q + 3.0)


@njit(cache=True)
def _solve_first_column(A, m):
    """Solve A c = e0 for the m x m system in A (destroyed). Partial pivoting.

    Returns c (length m) or NaNs if the system is singular.
    """
    rhs = np.zeros(m)
    rhs[0] = 1.0
    for col in range(m):
        piv = col
        big = abs(A[col, col])
        for r in range(col + 1, m):
            if abs(A[r, col]) > big:
                big = abs(A[r, col])
                piv = r
        if big < 1e-13:
            out = np.empty(m)
            out[:] = np.nan
            return out
        if piv != col:
            for k in range(m):
                tmp = A[col, k]
                A[col, k] = A[piv, k]
                A[piv, k] = tmp
            tmp = rhs[col]
            rhs[col] = rhs[piv]
            rhs[piv] = tmp
        for r in range(col + 1, m):
            f = A[r, col] / A[col, col]
            if f != 0.0:
                for k in range(col, m):
                    A[r, k] -= f * A[col, k]
                rhs[r] -= f * rhs[col]
    c = np.zeros(m)
    for r in range(m - 1, -1, -1):
        s = rhs[r]
        for k in range(r + 1, m):
            s -= A[r, k] * c[k]
        c[r] = s / A[r, r]
    return c


@njit(cache=True)
def _basis(s, t, degree, P):
    P[0] = 1.0
    if degree >= 1:
        P[1] = s
        P[2] = t
    if degree >= 2:
        P[3] = s * s
        P[4] = s * t
        P[5] = t * t


@njit(parallel=True, cache=True)
def mls_weights(query, idx, cells, radius_chord, degree):
    """MLS weight vectors.

    query  (n, 3)  unit directions
    idx    (n, k)  candidate cell indices from a KD-tree (len(cells) = absent)
    cells  (m, 3)  unit cell directions
    Returns phi (n, k) with sum_j phi[n, j] = 1, and a per-query count of
    cells with nonzero weight. A NaN row marks a singular fit.
    """
    n, k = idx.shape
    mcells = cells.shape[0]
    nb = 1 if degree == 0 else (3 if degree == 1 else 6)
    phi = np.zeros((n, k))
    count = np.zeros(n, np.int32)
    inv_r = 1.0 / radius_chord
    for i in prange(n):
        x0 = query[i, 0]
        x1 = query[i, 1]
        x2 = query[i, 2]
        # any orthonormal tangent frame: the result is frame invariant
        if abs(x0) < 0.9:
            h0, h1, h2 = 1.0, 0.0, 0.0
        else:
            h0, h1, h2 = 0.0, 1.0, 0.0
        dh = h0 * x0 + h1 * x1 + h2 * x2
        e10 = h0 - dh * x0
        e11 = h1 - dh * x1
        e12 = h2 - dh * x2
        ne = np.sqrt(e10 * e10 + e11 * e11 + e12 * e12)
        e10 /= ne
        e11 /= ne
        e12 /= ne
        e20 = x1 * e12 - x2 * e11
        e21 = x2 * e10 - x0 * e12
        e22 = x0 * e11 - x1 * e10

        A = np.zeros((nb, nb))
        P = np.zeros(6)
        S = np.zeros(k)
        T = np.zeros(k)
        W = np.zeros(k)
        cnt = 0
        for j in range(k):
            c = idx[i, j]
            if c >= mcells:
                continue
            d0 = cells[c, 0] - x0
            d1 = cells[c, 1] - x1
            d2 = cells[c, 2] - x2
            q = np.sqrt(d0 * d0 + d1 * d1 + d2 * d2) * inv_r
            w = _wendland_c4(q)
            if w <= 0.0:
                continue
            cnt += 1
            s = (d0 * e10 + d1 * e11 + d2 * e12) * inv_r
            t = (d0 * e20 + d1 * e21 + d2 * e22) * inv_r
            S[j] = s
            T[j] = t
            W[j] = w
            _basis(s, t, degree, P)
            for a in range(nb):
                wa = w * P[a]
                for b in range(a, nb):
                    A[a, b] += wa * P[b]
        for a in range(nb):
            for b in range(a):
                A[a, b] = A[b, a]
        count[i] = cnt
        cvec = _solve_first_column(A, nb)
        for j in range(k):
            if W[j] == 0.0:
                continue
            _basis(S[j], T[j], degree, P)
            acc = 0.0
            for a in range(nb):
                acc += cvec[a] * P[a]
            phi[i, j] = W[j] * acc
    return phi, count


@njit(parallel=True, cache=True)
def apply_weights(phi, idx, values):
    """out[n, f] = sum_j phi[n, j] * values[idx[n, j], f]  (absent idx skipped)."""
    n, k = idx.shape
    m, nf = values.shape
    out = np.zeros((n, nf))
    for i in prange(n):
        for j in range(k):
            w = phi[i, j]
            if w == 0.0:
                continue
            c = idx[i, j]
            if c >= m:
                continue
            for f in range(nf):
                out[i, f] += w * values[c, f]
    return out


def km_to_chord(km, radius_km=SOURCE_RADIUS_KM):
    """Surface distance (km) -> chord length on the unit sphere."""
    return 2.0 * np.sin(km / (2.0 * radius_km))


class Interpolator:
    """MLS of degree `degree` and support radius `radius_km` over `cells`."""

    def __init__(self, cells_xyz, radius_km, degree, kdtree=None, kmax=96):
        from scipy.spatial import cKDTree
        self.cells = np.ascontiguousarray(cells_xyz, np.float64)
        self.tree = kdtree if kdtree is not None else cKDTree(self.cells)
        self.radius_km = float(radius_km)
        self.r = km_to_chord(radius_km)
        self.degree = int(degree)
        self.kmax = int(kmax)

    def neighbours(self, query):
        dist, idx = self.tree.query(query, k=self.kmax,
                                    distance_upper_bound=self.r, workers=-1)
        # if the k-th candidate is still inside the radius the support set was
        # truncated, which would make the result discontinuous: refuse
        if np.isfinite(dist[:, -1]).any():
            raise RuntimeError(f"kmax={self.kmax} truncates the "
                               f"{self.radius_km:.0f} km support; raise kmax")
        return idx.astype(np.int64)

    def weights(self, query):
        query = np.ascontiguousarray(query, np.float64)
        idx = self.neighbours(query)
        phi, count = mls_weights(query, idx, self.cells, self.r, self.degree)
        return idx, phi, count

    def __call__(self, query, values):
        """values: (m,) or (m, nf) per-cell; returns (n,) or (n, nf)."""
        v = np.asarray(values, np.float64)
        flat = v.ndim == 1
        v2 = np.ascontiguousarray(v[:, None] if flat else v)
        idx, phi, _ = self.weights(query)
        out = apply_weights(phi, idx, v2)
        return out[:, 0] if flat else out


def idw3(tree, query, values):
    """The old bake (bake_faces.py): inverse-distance-squared over 3 nearest."""
    dist, idx = tree.query(query, k=3, workers=-1)
    w = 1.0 / np.maximum(dist, 1e-12) ** 2
    w /= w.sum(1, keepdims=True)
    v = np.asarray(values, np.float64)
    if v.ndim == 1:
        return (v[idx] * w).sum(1)
    return (v[idx] * w[..., None]).sum(1)

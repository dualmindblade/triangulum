"""Check my COBE CSC (quadrilateralized spherical cube) polynomial against wcslib (via astropy).

CSC inverse: face coords (x, y) in [-1, 1] -> gnomonic coords (chi, psi):
    chi = x + x (1 - x^2) * sum_ij P_ij x^(2i) y^(2j)
    psi = same with x and y swapped
Then direction on the face = normalize(1, chi, psi).
"""
import numpy as np
from astropy.wcs import WCS

P = {  # P[i][j]: coefficient of x^(2i) y^(2j)
    (0, 0): -0.27292696, (1, 0): -0.07629969, (2, 0): -0.22797056, (3, 0): 0.54852384,
    (4, 0): -0.62930065, (5, 0): 0.25795794, (6, 0): 0.02584375,
    (0, 1): -0.02819452, (1, 1): -0.01471565, (2, 1): 0.48051509, (3, 1): -1.74114454,
    (4, 1): 1.71547508, (5, 1): -0.53022337,
    (0, 2): 0.27058160, (1, 2): -0.56800938, (2, 2): 0.30803317, (3, 2): 0.98938102,
    (4, 2): -0.83180469,
    (0, 3): -0.60441560, (1, 3): 1.50880086, (2, 3): -0.93678576, (3, 3): 0.08693841,
    (0, 4): 0.93412077, (1, 4): -1.41601920, (2, 4): 0.33887446,
    (0, 5): -0.63915306, (1, 5): 0.52032238,
    (0, 6): 0.14381585,
}


def csc_ab(x, y):
    def warp(u, v):
        uu, vv = u * u, v * v
        s = sum(c * uu ** i * vv ** j for (i, j), c in P.items())
        return u + u * (1 - uu) * s
    return warp(x, y), warp(y, x)


if __name__ == "__main__":
    w = WCS(naxis=2)
    w.wcs.ctype = ["RA---CSC", "DEC--CSC"]
    w.wcs.crval = [0, 0]
    w.wcs.crpix = [1, 1]  # FITS pixels are 1-based; origin-0 pixel 0 = reference
    w.wcs.cdelt = [1, 1]
    w.wcs.set()
    g = np.linspace(-0.999, 0.999, 41)
    X, Y = np.meshgrid(g, g)
    pix = np.column_stack([X.ravel() * 45, Y.ravel() * 45])
    res = w.wcs.p2s(pix, 0)
    phi, th = np.radians(res["phi"]), np.radians(res["theta"])
    l, m, n = np.cos(th) * np.cos(phi), np.cos(th) * np.sin(phi), np.sin(th)
    ref = np.column_stack([m / l, n / l])
    mine = np.array([csc_ab(x, y) for x, y in zip(X.ravel(), Y.ravel())])
    d_ref = ref / np.linalg.norm(np.column_stack([np.ones(len(ref)), ref]), axis=1)[:, None]
    err = np.abs(mine - ref).max()
    # angular error on the sphere, in metres at R = 90.6 km
    v_ref = np.column_stack([np.ones(len(ref)), ref]); v_ref /= np.linalg.norm(v_ref, axis=1)[:, None]
    v_me = np.column_stack([np.ones(len(mine)), mine]); v_me /= np.linalg.norm(v_me, axis=1)[:, None]
    ang = np.arccos(np.clip((v_ref * v_me).sum(1), -1, 1))
    print(f"max |gnomonic coord| difference vs wcslib: {err:.2e}")
    print(f"max position difference on a 90.6 km planet: {ang.max() * 90_570:.3f} m")

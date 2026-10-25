"""Generate the smooth equal-area map on the CPU; numpy and scipy only.

Moser transport from the uniform density c=pi/6 to gnomonic sphere density.
The divergence is integrated analytically, then averaged with its transpose.
This avoids a Poisson solve and its unnecessary corner regularity problems.
"""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import time

import numpy as np
from numpy.polynomial.chebyshev import chebvander
from scipy.fft import dctn
from scipy.interpolate import RectBivariateSpline

ROOT = Path(__file__).resolve().parents[1]
C = np.pi / 6


def velocity(p, q, t):
    """v_t=-V/rho_t, where div V=rho-c and V has zero boundary flux.

    H(p,q)=integral_0^p rho(s,q) ds, m(q)=H(1,q),
    I(p)=integral_0^p m(s) ds. V is the transpose-symmetrized field
    (H(p,q)-p*m(q), I(q)-c*q). All these integrals have closed forms.
    """
    r = np.sqrt(1 + p*p + q*q)
    hp = p / ((1 + q*q) * r)
    hq = q / ((1 + p*p) * r)
    mp = 1 / ((1 + p*p) * np.sqrt(2 + p*p))
    mq = 1 / ((1 + q*q) * np.sqrt(2 + q*q))
    ip = np.arctan(p / np.sqrt(2 + p*p))
    iq = np.arctan(q / np.sqrt(2 + q*q))
    rho = C * (1-t) + t / (r*r*r)
    return -(hp - p*mq + ip - C*p) / (2*rho), -(hq - q*mp + iq - C*q) / (2*rho)


def flow(p, q, steps):
    p, q = np.array(p, copy=True), np.array(q, copy=True)
    dt = 1 / steps
    for k in range(steps):
        t = k * dt
        a, b = velocity(p, q, t)
        c, d = velocity(p + dt/2*a, q + dt/2*b, t + dt/2)
        e, f = velocity(p + dt/2*c, q + dt/2*d, t + dt/2)
        g, h = velocity(p + dt*e, q + dt*f, t + dt)
        p += dt/6 * (a + 2*c + 2*e + g)
        q += dt/6 * (b + 2*d + 2*f + h)
    return p, q


def spectral_map(order, steps):
    # Root nodes avoid dividing by p*(1-p*p)=0 at either endpoint.
    z = np.cos(np.pi * (np.arange(order) + 0.5) / order)
    x, y = np.meshgrid((z+1)/2, (z+1)/2, indexing="ij")
    p, q = np.sqrt(x), np.sqrt(y)
    # Only one eighth of the face needs transport. Reflect the two vector
    # components across the diagonal, then use even squared coordinates.
    ii, jj = np.triu_indices(order)
    u, v = flow(p[ii, jj], q[ii, jj], steps)
    a = np.empty_like(p)
    a[ii, jj] = u
    a[jj, ii] = v
    residual = (a-p) / (p*(1-x))
    coeff = dctn(residual, type=2) / order**2
    coeff[0, :] *= 0.5
    coeff[:, 0] *= 0.5
    print(f"transported {len(ii)} points in one eighth of a face")
    print(f"spectral tail (last 8 rows/cols): {max(np.max(np.abs(coeff[-8:, :])), np.max(np.abs(coeff[:, -8:]))):.3e}")
    return coeff


def make_spline(order=64, steps=256, intervals=128):
    coeff = spectral_map(order, steps)
    x = np.linspace(0, 1, intervals+1)
    v = chebvander(2*x-1, order-1)
    values = v @ coeff @ v.T
    return RectBivariateSpline(x, x, values, kx=5, ky=5, s=0)


def evaluate(spline, p, q):
    x, y = p*p, q*q
    g = spline.ev(x, y)
    h = spline.ev(y, x)
    a, b = p + p*(1-x)*g, q + q*(1-y)*h
    ap = 1+(1-3*x)*g+2*x*(1-x)*spline.ev(x, y, dx=1)
    aq = 2*p*q*(1-x)*spline.ev(x, y, dy=1)
    bq = 1+(1-3*y)*h+2*y*(1-y)*spline.ev(y, x, dx=1)
    bp = 2*p*q*(1-y)*spline.ev(y, x, dy=1)
    area = (ap*bq-aq*bp) / (1+a*a+b*b)**1.5
    return a, b, area


def validate(spline, n=1024):
    g = np.linspace(0, 1, n+1)
    p, q = np.meshgrid(g, g, indexing="ij")
    a, b, area = evaluate(spline, p, q)
    error = np.max(np.abs(area/C-1))
    print(f"Jacobian max relative error, {n+1}^2 quadrant samples: {error:.3e}")
    assert error < 1e-8, "refusing to write an inaccurate area table"
    # A full face N=1024 has 512 cells per quadrant.
    v = np.stack([np.ones_like(a), a, b], axis=-1)
    v /= np.linalg.norm(v, axis=-1)[..., None]
    def tri(a, b, c):
        # Difference form avoids cancellation of the small determinant.
        det = np.sum(a*np.cross(b-a, c-a), axis=-1)
        den = 1+np.sum(a*b, axis=-1)+np.sum(b*c, axis=-1)+np.sum(c*a, axis=-1)
        return 2*np.arctan2(np.abs(det), den)
    cells = tri(v[:-1, :-1], v[1:, :-1], v[1:, 1:]) + tri(v[:-1, :-1], v[1:, 1:], v[:-1, 1:])
    mean = C/n**2
    error = np.max(np.abs(cells/mean-1))
    print(f"two-triangle area max relative error, full-face N={2*n}: {error:.3e}")
    if n == 512:
        assert error < 1e-6, "N=1024 cell-area acceptance check failed"


def convergence(spline, order, steps, intervals):
    """Independently refine time integration, spectral order and spline mesh."""
    rng = np.random.default_rng(20261024)
    p, q = rng.uniform(0, 1, (2, 10_000))
    coarse = evaluate(spline, p, q)
    for label, settings in [
        ("twice the RK4 steps", (order, 2*steps, intervals)),
        ("1.5 times spectral order", (order*3//2, steps, intervals)),
        ("twice the spline intervals", (order, steps, 2*intervals)),
    ]:
        refined = evaluate(make_spline(*settings), p, q)
        position = max(np.max(np.abs(coarse[k]-refined[k])) for k in (0, 1))
        density = np.max(np.abs(coarse[2]-refined[2]))/C
        print(f"convergence ({label}): coordinate {position:.3e}, relative Jacobian {density:.3e}")
        assert position < 1e-10 and density < 1e-8


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--order", type=int, default=64)
    parser.add_argument("--steps", type=int, default=256)
    parser.add_argument("--intervals", type=int, default=128)
    parser.add_argument("--check", type=int, default=512)
    parser.add_argument("--convergence", action="store_true")
    args = parser.parse_args()
    if args.order < 16 or args.steps < 1 or args.intervals < 8 or args.check < 1:
        parser.error("require order>=16, steps>=1, intervals>=8, check>=1")
    start = time.perf_counter()
    spline = make_spline(args.order, args.steps, args.intervals)
    validate(spline, args.check)
    if args.convergence:
        convergence(spline, args.order, args.steps, args.intervals)
    dest = ROOT / "data" / "equal_area.bin"
    dest.parent.mkdir(exist_ok=True)
    # Header: magic, intervals u32 LE, spline degree u32 LE; coefficients f64 LE.
    data = b"CSEQAR01" + np.array([args.intervals, 5], dtype="<u4").tobytes()
    data += spline.get_coeffs().astype("<f8").tobytes()
    dest.write_bytes(data)
    print(f"SHA-256: {hashlib.sha256(data).hexdigest()}")
    print(f"wrote {dest.name}: {len(data):,} bytes in {time.perf_counter()-start:.2f}s")


if __name__ == "__main__":
    main()

"""CPU-only Moser correction of COBE's grid (numpy, scipy and numba).

Numerically project COBE's spherical density onto a Chebyshev basis, integrate
that polynomial to get a symmetric no-flux field, and integrate its flow with
RK4. A small Hamiltonian relaxation also controls the straight-sided cell's
chord error. Compose with COBE before fitting the runtime quintic spline.
No changes to the original EqualArea generator or asset are needed.
"""
from __future__ import annotations

import argparse
import ast
import hashlib
from pathlib import Path
import time

import numpy as np
from numpy.polynomial import chebyshev as cheb
from numba import njit
from scipy.fft import dctn
from scipy.interpolate import RectBivariateSpline

from solve_equal_area import C, evaluate

ROOT = Path(__file__).resolve().parents[1]
tree = ast.parse((ROOT / "tools/reference/csc_check.py").read_text())
terms = next(ast.literal_eval(n.value) for n in tree.body
             if isinstance(n, ast.Assign)
             and any(isinstance(t, ast.Name) and t.id == "P" for t in n.targets))
CSC = np.zeros((7, 7))
for (i, j), value in terms.items():
    CSC[j, i] = value

# First acceptable result of tune_equal_area_cobe.py; freeze rather than
# rerunning the optimizer during regeneration. Rows are powers of w, columns
# powers of z in the Hamiltonian below. Nine decimal places suffice.
RELAXATION = np.array([
    [0.010000000, -0.044419759, -0.024909799, -0.001283083],
    [0.062159085, -0.000537519, 0.093213995, 0.029492224],
])


@njit(cache=True)
def residual(x, y):
    g, gx, gy = 0.0, 0.0, 0.0
    for j in range(6, -1, -1):
        row, dx = 0.0, 0.0
        for i in range(6-j, -1, -1):
            dx = dx*x + row
            row = row*x + CSC[j, i]
        gy = gy*y + g
        gx = gx*y + dx
        g = g*y + row
    return g, gx, gy


@njit(cache=True)
def cobe(p, q):
    x, y = p*p, q*q
    g, gx, gy = residual(x, y)
    h, hy, hx = residual(y, x)
    a, b = p+p*(1-x)*g, q+q*(1-y)*h
    ap = 1+(1-3*x)*g+2*x*(1-x)*gx
    aq = 2*p*q*(1-x)*gy
    bp = 2*p*q*(1-y)*hx
    bq = 1+(1-3*y)*h+2*y*(1-y)*hy
    rho = (ap*bq-aq*bp)/(1+a*a+b*b)**1.5
    return a, b, rho


@njit(cache=True)
def densities(p, q):
    out = np.empty_like(p)
    for i in range(p.size):
        out.flat[i] = cobe(p.flat[i], q.flat[i])[2]
    return out


def project(values):
    coefficients = dctn(values, type=2) / values.shape[0]**2
    coefficients[0, :] *= 0.5
    coefficients[:, 0] *= 0.5
    return coefficients


def roots(order):
    return (1+np.cos(np.pi*(np.arange(order)+0.5)/order))/2


def make_field(order):
    """Integrate a numerical density expansion, not an elementary formula.

    rho(p,q) is even in each coordinate, and T_k(2*p*p-1)=T_{2k}(p).
    Expand in the squared coordinates, unfold into the ordinary Chebyshev
    basis, integrate, then divide Vx by p*(1-p*p). The remaining polynomial
    is even and folds back into the compact squared-coordinate basis.
    """
    p, q = np.meshgrid(np.sqrt(roots(order)), np.sqrt(roots(order)), indexing="ij")
    density = project(densities(p, q))
    density = (density+density.T)/2
    rho = np.zeros((2*order-1, 2*order-1))
    rho[::2, ::2] = density
    # H(p,q)=integral_0^p rho(s,q) ds; m(q)=H(1,q).
    h = cheb.chebint(rho, axis=0)
    marginal = cheb.chebval(1.0, h)
    integral = cheb.chebint(marginal)
    mean = cheb.chebval(1.0, integral)
    print(f"density quadrature mean error: {mean-C:+.3e}", flush=True)
    assert abs(mean-C) < 1e-13
    # Remove only the numerical quadrature's constant-mode roundoff.
    rho[0, 0] += C-mean
    h = cheb.chebint(rho, axis=0)
    marginal = cheb.chebval(1.0, h)
    integral = cheb.chebint(marginal)
    # Vx = (H(p,q) - p*m(q) + I(p) - C*p)/2; Vy=Vx(q,p).
    v = h.copy()
    v[1, :] -= marginal
    v[:, 0] += integral
    v[1, 0] -= C
    v *= 0.5
    factor = np.zeros_like(rho)
    remainder = 0.0
    for j in range(rho.shape[1]):
        # p*(1-p*p)=(T_1(p)-T_3(p))/4.
        quotient, rem = cheb.chebdiv(v[:, j], [0.0, 0.25, 0.0, -0.25])
        factor[:len(quotient), j] = quotient
        remainder = max(remainder, np.max(np.abs(rem)))
    assert remainder < 1e-13
    field = np.ascontiguousarray(factor[::2, ::2])
    print(f"field division remainder: {remainder:.3e}; spectral tail: "
          f"{max(np.max(np.abs(field[-8:, :])), np.max(np.abs(field[:, -8:]))):.3e}", flush=True)
    return field


@njit(cache=True)
def series(coefficients, x, y):
    """Tensor Clenshaw evaluation, without allocating intermediate arrays."""
    b1, b2 = 0.0, 0.0
    for i in range(coefficients.shape[0]-1, -1, -1):
        c1, c2 = 0.0, 0.0
        for j in range(coefficients.shape[1]-1, 0, -1):
            c0 = coefficients[i, j]+2*y*c1-c2
            c2, c1 = c1, c0
        row = coefficients[i, 0]+y*c1-c2
        if i == 0:
            return row+x*b1-b2
        b0 = row+2*x*b1-b2
        b2, b1 = b1, b0
    return 0.0


@njit(cache=True)
def velocity(p, q, t, field):
    x, y = p*p, q*q
    rho = (1-t)*C+t*cobe(p, q)[2]
    return (-p*(1-x)*series(field, 2*x-1, 2*y-1)/rho,
            -q*(1-y)*series(field, 2*y-1, 2*x-1)/rho)


@njit(cache=True)
def relaxation_velocity(p, q, coefficients):
    # Hamiltonian psi = p*q*(p*p-q*q)*(1-p*p)*(1-q*q)*P(z,w),
    # where z=p*p*q*q, w=(1-p*p)*(1-q*q).
    # (psi_q, -psi_p) is divergence free, symmetric and tangent to every edge.
    x, y = p*p, q*q
    z, w = x*y, (1-x)*(1-y)
    factor, dz, dw = 0.0, 0.0, 0.0
    for j in range(coefficients.shape[0]-1, -1, -1):
        row, derivative = 0.0, 0.0
        for i in range(coefficients.shape[1]-1, -1, -1):
            derivative = derivative*z+row
            row = row*z+coefficients[j, i]
        dw = dw*w+factor
        dz = dz*w+derivative
        factor = factor*w+row
    a = p*(1-x)*(((x-3*y)*(1-y)-2*y*(x-y))*factor
                 +2*y*(x-y)*(1-y)*(x*dz-(1-x)*dw))
    b = q*(1-y)*(((y-3*x)*(1-x)-2*x*(y-x))*factor
                 +2*x*(y-x)*(1-x)*(y*dz-(1-y)*dw))
    return a, b


@njit(cache=True)
def relax(p, q, steps, coefficients):
    u, v = p.copy(), q.copy()
    dt = 1/steps
    for i in range(p.size):
        x, y = p[i], q[i]
        for _ in range(steps):
            a, b = relaxation_velocity(x, y, coefficients)
            c, d = relaxation_velocity(x+dt/2*a, y+dt/2*b, coefficients)
            e, f = relaxation_velocity(x+dt/2*c, y+dt/2*d, coefficients)
            g, h = relaxation_velocity(x+dt*e, y+dt*f, coefficients)
            x += dt/6*(a+2*c+2*e+g)
            y += dt/6*(b+2*d+2*f+h)
        u[i], v[i] = x, y
    return u, v


@njit(cache=True)
def flow(p, q, steps, field):
    u, v = p.copy(), q.copy()
    dt = 1/steps
    for i in range(p.size):
        x, y = p[i], q[i]
        for k in range(steps):
            t = k*dt
            a, b = velocity(x, y, t, field)
            c, d = velocity(x+dt/2*a, y+dt/2*b, t+dt/2, field)
            e, f = velocity(x+dt/2*c, y+dt/2*d, t+dt/2, field)
            g, h = velocity(x+dt*e, y+dt*f, t+dt, field)
            x += dt/6*(a+2*c+2*e+g)
            y += dt/6*(b+2*d+2*f+h)
        u[i], v[i] = x, y
    return u, v


def make_spline(field, order, steps, intervals, relaxation=RELAXATION):
    x, y = np.meshgrid(roots(order), roots(order), indexing="ij")
    p, q = np.sqrt(x), np.sqrt(y)
    ii, jj = np.triu_indices(order)
    u, v = p[ii, jj], q[ii, jj]
    relaxation = np.atleast_2d(relaxation)
    if np.any(relaxation):
        u, v = relax(u, v, steps, relaxation)
    u, v = flow(u, v, steps, field)
    print(f"transported {len(ii)} nodes; maximum sampled COBE grid displacement: "
          f"{max(np.max(np.abs(u-p[ii,jj])), np.max(np.abs(v-q[ii,jj]))):.6e}", flush=True)
    a = np.empty_like(p)
    for k in range(len(ii)):
        ca, cb, _ = cobe(u[k], v[k])
        a[ii[k], jj[k]], a[jj[k], ii[k]] = ca, cb
    coefficients = project((a-p)/(p*(1-x)))
    print(f"map spectral tail: {max(np.max(np.abs(coefficients[-8:, :])), np.max(np.abs(coefficients[:, -8:]))):.3e}", flush=True)
    grid = np.linspace(0, 1, intervals+1)
    basis = cheb.chebvander(2*grid-1, order-1)
    values = basis @ coefficients @ basis.T
    return RectBivariateSpline(grid, grid, values, kx=5, ky=5, s=0)


def validate(spline, n=512, require_chords=True):
    grid = np.linspace(0, 1, n+1)
    p, q = np.meshgrid(grid, grid, indexing="ij")
    a, b, area = evaluate(spline, p, q)
    error = np.max(np.abs(area/C-1))
    print(f"Jacobian max relative error, {n+1}^2 quadrant samples: {error:.3e}", flush=True)
    assert error < 1e-8, "refusing to write an inaccurate continuous-area table"
    v = np.stack((np.ones_like(a), a, b), axis=-1)
    v /= np.linalg.norm(v, axis=-1)[..., None]
    def tri(a, b, c):
        determinant = np.sum(a*np.cross(b-a, c-a), axis=-1)
        denominator = 1+np.sum(a*b, axis=-1)+np.sum(b*c, axis=-1)+np.sum(c*a, axis=-1)
        return 2*np.arctan2(np.abs(determinant), denominator)
    cells = tri(v[:-1, :-1], v[1:, :-1], v[1:, 1:])+tri(v[:-1, :-1], v[1:, 1:], v[:-1, 1:])
    ratios = cells/(C/n**2)
    error = np.max(np.abs(ratios-1))
    print(f"two-triangle areas, full-face N={2*n}: min/mean={ratios.min():.12f}, "
          f"max/mean={ratios.max():.12f}, max/min={ratios.max()/ratios.min():.12f}, "
          f"max relative error={error:.3e}", flush=True)
    if n == 512 and require_chords:
        assert error < 1e-6, "N=1024 cell-area acceptance check failed"


def shape_metrics(spline, n=400):
    """Same samples, diagonal offsets, differences and weights as Rust."""
    grid = -1+2*(np.arange(n)+0.5)/n
    p, q = np.meshgrid(grid, grid, indexing="ij")
    q = q.copy()
    q[np.abs(p) == np.abs(q)] += 1e-6
    def direction(p, q):
        a, b, _ = evaluate(spline, p, q)
        r = np.stack((np.ones_like(a), a, b), axis=-1)
        return r/np.linalg.norm(r, axis=-1)[..., None]
    h = 1e-7
    dp = (direction(p+h, q)-direction(p-h, q))/(2*h)
    dq = (direction(p, q+h)-direction(p, q-h))/(2*h)
    area = np.linalg.norm(np.cross(dp, dq), axis=-1)
    lp, lq = np.linalg.norm(dp, axis=-1), np.linalg.norm(dq, axis=-1)
    skew = np.degrees(np.abs(np.arcsin(np.clip(np.sum(dp*dq, axis=-1)/(lp*lq), -1, 1))))
    share = np.sum(area[skew > 10])/np.sum(area)
    print(f"shape: surface >10deg={100*share:.6f}%, worst skew={np.max(skew):.6f}deg, "
          f"worst edge-length ratio={np.max(np.maximum(lp/lq, lq/lp)):.6f}", flush=True)
    assert share <= 0.17, "COBE correction misses the shape target"


def convergence(spline, field, args, relaxation):
    rng = np.random.default_rng(20261025)
    p, q = rng.uniform(0, 1, (2, 10_000))
    coarse = evaluate(spline, p, q)
    settings = [
        ("twice the RK4 steps", field, args.order, 2*args.steps, args.intervals),
        ("1.5 times map spectral order", field, args.order*3//2, args.steps, args.intervals),
        ("twice the spline intervals", field, args.order, args.steps, 2*args.intervals),
        ("1.5 times density quadrature order", make_field(args.density_order*3//2), args.order, args.steps, args.intervals),
    ]
    for label, refined_field, order, steps, intervals in settings:
        refined = evaluate(make_spline(refined_field, order, steps, intervals, relaxation), p, q)
        position = max(np.max(np.abs(coarse[k]-refined[k])) for k in (0, 1))
        density = np.max(np.abs(coarse[2]-refined[2]))/C
        print(f"convergence ({label}): coordinate {position:.3e}, relative Jacobian {density:.3e}", flush=True)
        assert position < 1e-10 and density < 1e-8


def reference_fixture(field, steps, relaxation):
    """Direct, refined ODE values, independently of the fitted runtime table."""
    rng = np.random.default_rng(20261025)
    grid = np.linspace(-1, 1, 17)
    p, q = np.meshgrid(grid, grid, indexing="ij")
    points = np.r_[np.column_stack((p.ravel(), q.ravel())), rng.uniform(-1, 1, (1024, 2))]
    u, v = relax(points[:, 0].copy(), points[:, 1].copy(), 2*steps, relaxation)
    u, v = flow(u, v, 2*steps, field)
    values = np.array([cobe(x, y)[:2] for x, y in zip(u, v)])
    return np.column_stack((points, values)).astype("<f8").tobytes()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--density-order", type=int, default=48)
    parser.add_argument("--order", type=int, default=64)
    parser.add_argument("--steps", type=int, default=64)
    parser.add_argument("--intervals", type=int, default=256)
    parser.add_argument("--check", type=int, default=512)
    parser.add_argument("--plain", action="store_true", help="diagnose Moser without relaxation; do not write assets")
    parser.add_argument("--convergence", action="store_true")
    parser.add_argument("--no-write", action="store_true")
    args = parser.parse_args()
    if min(args.density_order, args.order) < 16 or args.steps < 1 or args.intervals < 8 or args.check < 1:
        parser.error("require orders>=16, steps>=1, intervals>=8, check>=1")
    start = time.perf_counter()
    field = make_field(args.density_order)
    relaxation = np.zeros_like(RELAXATION) if args.plain else RELAXATION
    spline = make_spline(field, args.order, args.steps, args.intervals, relaxation)
    shape_metrics(spline)
    validate(spline, args.check, require_chords=not args.plain)
    if args.convergence:
        convergence(spline, field, args, relaxation)
    if not args.no_write and not args.plain:
        if args.check != 512:
            validate(spline, 512)
        data = b"CSEQAR01"+np.array([args.intervals, 5], dtype="<u4").tobytes()
        data += spline.get_coeffs().astype("<f8").tobytes()
        dest = ROOT / "data/equal_area_cobe.bin"
        fixture = reference_fixture(field, args.steps, relaxation)
        dest.write_bytes(data)
        (ROOT / "data/equal_area_cobe_reference.bin").write_bytes(fixture)
        print(f"SHA-256: {hashlib.sha256(data).hexdigest()}")
        print(f"wrote {dest.name}: {len(data):,} bytes")
        print(f"wrote {len(fixture)//32} direct-flow reference pairs")
    print(f"elapsed: {time.perf_counter()-start:.2f}s")


if __name__ == "__main__":
    main()

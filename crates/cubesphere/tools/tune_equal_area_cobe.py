"""Optional CPU experiment: reduce the N=1024 chord-area truncation error.

The plain COBE-based Moser map already meets the shape target. This searches
small, exactly area-preserving Hamiltonian flows to also meet the original
EqualArea test's tolerance on straight-sided spherical quadrilaterals.
"""
import argparse

import numpy as np
from scipy.optimize import linprog

from solve_equal_area_cobe import C, evaluate, make_field, make_spline, relax, shape_metrics, validate


def chord_errors(spline, p, q, n, coefficients):
    u, v = relax(p.ravel(), q.ravel(), 64, coefficients)
    a, b, _ = evaluate(spline, u, v)
    directions = np.stack((np.ones_like(a), a, b), axis=-1).reshape(n+1, n+1, 3)
    directions /= np.linalg.norm(directions, axis=-1)[..., None]
    def tri(a, b, c):
        return 2*np.arctan2(np.sum(a*np.cross(b-a, c-a), axis=-1),
                           1+np.sum(a*b, axis=-1)+np.sum(b*c, axis=-1)+np.sum(c*a, axis=-1))
    v = directions
    areas = tri(v[:-1, :-1], v[1:, :-1], v[1:, 1:])+tri(v[:-1, :-1], v[1:, 1:], v[:-1, 1:])
    return (areas.ravel()/(C/n**2)-1)*n**2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--degree", type=int, default=3)
    parser.add_argument("--layers", type=int, default=2)
    parser.add_argument("--grid", type=int, default=64)
    parser.add_argument("--min-constant", type=float, default=0.01)
    args = parser.parse_args()
    n = args.grid
    field = make_field(32)
    base = make_spline(field, 64, 64, 128, relaxation=np.zeros((1, 1)))
    p, q = np.meshgrid(np.linspace(0, 1, n+1), np.linspace(0, 1, n+1), indexing="ij")
    coefficients = np.zeros((args.layers, args.degree+1))
    coefficients[0, 0] = args.min_constant
    for iteration in range(5):
        errors = chord_errors(base, p, q, n, coefficients)
        print(f"iteration {iteration}: scaled chord error {np.max(np.abs(errors)):.6f}; "
              f"coefficients {coefficients.tolist()}", flush=True)
        if np.max(np.abs(errors)) < 0.21:
            break
        derivative = []
        for k in range(coefficients.size):
            shifted = coefficients.copy()
            shifted.flat[k] += 1e-3
            derivative.append((chord_errors(base, p, q, n, shifted)-errors)/1e-3)
        jacobian = np.array(derivative).T
        bounds = np.column_stack((np.r_[jacobian, -jacobian], -np.ones(2*len(errors))))
        result = linprog(np.r_[np.zeros(coefficients.size), 1.0], A_ub=bounds,
                         b_ub=np.r_[-errors, errors],
                         bounds=[(args.min_constant-coefficients[0, 0], 0.03-coefficients[0, 0])]
                         + [(-0.5, 0.5)]*(coefficients.size-1)+[(0, None)], method="highs")
        assert result.success, result.message
        coefficients += result.x[:-1].reshape(coefficients.shape)
    final = make_spline(field, 64, 64, 128, relaxation=coefficients)
    shape_metrics(final)
    validate(final, 512)


if __name__ == "__main__":
    main()

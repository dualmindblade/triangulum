"""Equal-area cube-sphere vs gnomonic vs equal-angle: what does the correction buy?

Exact equal-area map built the QSC / Rosca-Plonka way, per face triangle
(|q| <= |p|): the face-param square of half-size s maps to the gnomonic square
of half-size A(s) with sphere area proportional to s^2, and along each ring the
area is distributed linearly in t = q/p. Derived and checked here:
  F(A) = 4 atan(A^2 / sqrt(1 + 2A^2))     solid angle of gnomonic square
  A(s) = sqrt(T (T + sqrt(T^2 + 1))),  T = tan(pi s^2 / 6)
  K = t A / sqrt(1 + 2A^2),  w = K sqrt(1 + A^2) / (A sqrt(1 - K^2)),  b = w a
"""
import numpy as np

FACE, RIGHT, UP = np.array([0., 0., 1.]), np.array([1., 0., 0.]), np.array([0., 1., 0.])


def gnomonic_ab(p, q, proj):
    if proj == "gnomonic":
        return p, q
    if proj == "equiangular":
        return np.tan(p * np.pi / 4), np.tan(q * np.pi / 4)
    # equal-area
    swap = abs(q) > abs(p)
    if swap:
        p, q = q, p
    s = abs(p)
    if s < 1e-15:
        return 0.0, 0.0
    t = q / p
    T = np.tan(np.pi * s * s / 6)
    A = np.sqrt(T * (T + np.sqrt(T * T + 1)))
    K = t * A / np.sqrt(1 + 2 * A * A)
    w = K * np.sqrt(1 + A * A) / (A * np.sqrt(1 - K * K))
    a = np.sign(p) * A
    b = w * a
    return (b, a) if swap else (a, b)


def direction(p, q, proj):
    a, b = gnomonic_ab(p, q, proj)
    v = FACE + a * RIGHT + b * UP
    return v / np.linalg.norm(v)


def jac(p, q, proj, N, R, h=1e-7):
    st = 2.0 / N
    d0 = direction(p, q, proj)
    di = (direction(p + h, q, proj) - direction(p - h, q, proj)) / (2 * h) * st * R
    dj = (direction(p, q + h, proj) - direction(p, q - h, proj)) / (2 * h) * st * R
    e1 = di - d0 * d0.dot(di); e1 /= np.linalg.norm(e1); e2 = np.cross(d0, e1)
    return np.array([[e1.dot(di), e1.dot(dj)], [e2.dot(di), e2.dot(dj)]])


def shape(J):
    li, lj = np.linalg.norm(J[:, 0]), np.linalg.norm(J[:, 1])
    ang = np.degrees(np.arccos(np.clip(J[:, 0].dot(J[:, 1]) / (li * lj), -1, 1)))
    sv = np.linalg.svd(J, compute_uv=False)
    return li, lj, abs(90 - ang), abs(np.linalg.det(J)), sv[0] / sv[1]


def survey(proj, N, R):
    # total area check -> mean cell area should be face area / N^2
    g = (np.arange(160) + 0.5) / 160 * 2 - 1
    rows = []
    for p in g:
        for q in g:
            if abs(abs(p) - abs(q)) < 1.5 / 160:   # skip cells straddling the diagonal kink
                continue
            rows.append(shape(jac(p, q, proj, N, R)))
    rows = np.array(rows)
    area, shear, stretch = rows[:, 3], rows[:, 2], rows[:, 4]
    w = area / area.sum()
    edges = np.concatenate([rows[:, 0], rows[:, 1]])
    print(f"\n{proj:12s} area max/min {area.max()/area.min():5.3f}  "
          f"edge length {edges.min():.2f}-{edges.max():.2f} m  "
          f"worst stretch {stretch.max():.2f}:1  mean stretch {np.average(stretch, weights=w):.3f}:1")
    print(f"{'':12s} worst shear {shear.max():4.1f} deg   surface with shear >5: {100*w[shear>5].sum():.0f}%  "
          f">10: {100*w[shear>10].sum():.0f}%  >20: {100*w[shear>20].sum():.0f}%")
    for name, (p, q) in {"centre": (0, 0), "edge mid (0.95,0)": (0.95, 0),
                         "half diag (0.5,0.49)": (0.5, 0.49), "near corner (0.9,0.89)": (0.9, 0.89)}.items():
        li, lj, sh, ar, st = shape(jac(p, q, proj, N, R))
        print(f"    {name:24s} {li:4.2f} x {lj:4.2f} m  shear {sh:4.1f}  area {ar:4.2f} m2")


def kink(N, R):
    """Equal-area only: how abruptly do cells change across the face diagonal?"""
    print("\nequal-area: cells on either side of the face diagonal")
    for s in (0.3, 0.6, 0.9):
        a = shape(jac(s, s * 0.999, "equalarea", N, R, h=1e-9))
        b = shape(jac(s * 0.999, s, "equalarea", N, R, h=1e-9))
        Ja = jac(s, s * 0.999, "equalarea", N, R, h=1e-9)
        Jb = jac(s * 0.999, s, "equalarea", N, R, h=1e-9)
        # direction of a grid row (i-direction) just below vs just above the diagonal
        ang = np.degrees(np.arccos(Ja[:, 0].dot(Jb[:, 0]) / np.linalg.norm(Ja[:, 0]) / np.linalg.norm(Jb[:, 0])))
        print(f"    at s={s}: below {a[0]:.3f} x {a[1]:.3f} m shear {a[2]:4.1f} | "
              f"above {b[0]:.3f} x {b[1]:.3f} m shear {b[2]:4.1f} | rows bend {ang:4.1f} deg")


if __name__ == "__main__":
    N = 131_072
    R = 2 * N / np.pi            # equal-angle: 1 m along the axes at face centre
    print(f"N = {N:,} columns per face edge, R = {R/1000:.1f} km; face area / N^2 = "
          f"{4*np.pi*R*R/6/N/N:.3f} m2 mean cell")
    for proj in ("gnomonic", "equiangular", "equalarea"):
        survey(proj, N, R)
    kink(N, R)

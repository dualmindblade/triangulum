"""How far does a cube-sphere voxel grid deviate from perfect cubes?

Answers the detached-grid question with numbers. For a face of N columns
on a sphere of radius R, the local horizontal cell at face coords (a, b)
is described by the 2x2 Jacobian J (metres per grid step along i and j).
A perfect cube grid has J = s * rotation.

Quantities reported:
  cell size     - edge lengths |J e_i|, |J e_j| in metres
  shear         - deviation of the angle between grid lines from 90 deg
  convert err   - detach a structure of span L into perfect cubes in place:
                  how far (in blocks) its far corner moves
  transplant err- keep a structure's world shape, carry it to another
                  location and snap it into that grid (best rotation):
                  how far (in blocks) its far corner misses a cell
"""
import numpy as np

FACE_AXIS = np.array([0.0, 0.0, 1.0])
RIGHT = np.array([1.0, 0.0, 0.0])
UP = np.array([0.0, 1.0, 0.0])


def direction(p, q, proj):
    """Face param p, q in [-1, 1] -> unit direction."""
    if proj == "gnomonic":
        a, b = p, q
    else:  # equiangular
        a, b = np.tan(p * np.pi / 4), np.tan(q * np.pi / 4)
    v = FACE_AXIS + a * RIGHT + b * UP
    return v / np.linalg.norm(v)


def jacobian(p, q, proj, N, R):
    """Metres on the sphere per one grid step in i and j, as a 3x2 matrix,
    expressed in a local tangent frame -> 2x2."""
    step = 2.0 / N
    h = step * 1e-3
    d0 = direction(p, q, proj)
    di = (direction(p + h, q, proj) - direction(p - h, q, proj)) / (2 * h) * step * R
    dj = (direction(p, q + h, proj) - direction(p, q - h, proj)) / (2 * h) * step * R
    # local tangent frame
    e1 = di - d0 * d0.dot(di)
    e1 /= np.linalg.norm(e1)
    e2 = np.cross(d0, e1)
    return np.array([[e1.dot(di), e1.dot(dj)], [e2.dot(di), e2.dot(dj)]])


def rot(t):
    c, s = np.cos(t), np.sin(t)
    return np.array([[c, -s], [s, c]])


def best_fit_err(M, L):
    """Max displacement (blocks) of the corners of an L x L footprint under
    M, after the best rotation and best uniform offset (structure centred)."""
    best = 1e9
    corners = np.array([[x, y] for x in (-L / 2, L / 2) for y in (-L / 2, L / 2)]).T
    for t in np.linspace(-0.6, 0.6, 2401):
        D = rot(t) @ M - np.eye(2)
        err = np.max(np.linalg.norm(D @ corners, axis=0))
        best = min(best, err)
    return best


def report(proj, N, R, label):
    print(f"\n=== {label}: {proj}, N={N:,} columns/face, R={R/1000:,.0f} km")
    pts = {
        "face centre": (0.0, 0.0),
        "edge midpoint (0.95,0)": (0.95, 0.0),
        "halfway on diagonal (0.5,0.5)": (0.5, 0.5),
        "near corner (0.9,0.9)": (0.9, 0.9),
    }
    Js = {}
    for name, (p, q) in pts.items():
        J = jacobian(p, q, proj, N, R)
        Js[name] = J
        li, lj = np.linalg.norm(J[:, 0]), np.linalg.norm(J[:, 1])
        ang = np.degrees(np.arccos(J[:, 0].dot(J[:, 1]) / (li * lj)))
        s = np.sqrt(abs(np.linalg.det(J)))  # equal-area cube side
        conv = [best_fit_err(J / s, L) for L in (16, 64)]
        print(f"  {name:32s} cell {li:5.2f} x {lj:5.2f} m, shear {abs(90-ang):5.1f} deg, "
              f"to-cubes err L=16: {conv[0]:4.2f}, L=64: {conv[1]:5.2f} blocks")
    # transplant: carry a structure from A to B, keep world shape, snap into B's grid
    print("  transplant (keep world shape, snap into destination grid), best rotation:")
    pairs = [("face centre", "halfway on diagonal (0.5,0.5)"),
             ("face centre", "edge midpoint (0.95,0)"),
             ("halfway on diagonal (0.5,0.5)", "near corner (0.9,0.9)")]
    for a, b in pairs:
        M = np.linalg.inv(Js[b]) @ Js[a]
        errs = [best_fit_err(M, L) for L in (8, 16, 64)]
        print(f"    {a} -> {b}: L=8 {errs[0]:.2f}, L=16 {errs[1]:.2f}, L=64 {errs[2]:.2f} blocks")
    # local drift: same structure, how much does the grid itself change across it?
    for L in (64, 512):
        J0 = jacobian(0.5, 0.5, proj, N, R)
        J1 = jacobian(0.5 + 2.0 * L / N, 0.5 + 2.0 * L / N, proj, N, R)
        M = np.linalg.inv(J1) @ J0
        print(f"  grid drift across a {L}-block span at (0.5,0.5): "
              f"{best_fit_err(M, L):.3f} blocks")
    # area-weighted shear distribution across the face
    g = np.linspace(-0.995, 0.995, 121)
    shears, areas = [], []
    for p in g:
        for q in g:
            J = jacobian(p, q, proj, N, R)
            li, lj = np.linalg.norm(J[:, 0]), np.linalg.norm(J[:, 1])
            ang = np.degrees(np.arccos(J[:, 0].dot(J[:, 1]) / (li * lj)))
            shears.append(abs(90 - ang))
            areas.append(abs(np.linalg.det(J)))
    shears, areas = np.array(shears), np.array(areas)
    w = areas / areas.sum()
    print(f"  area spread max/min {areas.max()/areas.min():.2f}x; "
          f"surface with shear >5 deg: {100*w[shears>5].sum():.0f}%, "
          f">10 deg: {100*w[shears>10].sum():.0f}%, >20 deg: {100*w[shears>20].sum():.0f}%")


if __name__ == "__main__":
    report("gnomonic", 10_000_000, 8_660_254.0, "TODAY (Neisor)")
    # son's proposal: 50x fewer columns; choose R so face-centre cells are ~1 m
    report("gnomonic", 200_000, 100_000.0, "60x SMALLER, same projection")
    report("equiangular", 200_000, 200_000 * 2 / np.pi, "60x SMALLER, equal-angle")

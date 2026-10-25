# cubesphere

Six deterministic planet-grid projections with accurate inverses. Rust 2024,
with only `glam = "0.33"` as a dependency. Runtime calls allocate nothing, use no
mutable state, and require no external files. The two smooth equal-area maps
embed numerical tables with `include_bytes!`: **133,144 bytes** for EqualArea
and **528,408 bytes** for EqualAreaCobe.

```rust
use cubesphere::{Projection, face_to_dir, dir_to_face};

let n = 131_072_u32;
let (i, j) = (1234, 5678);
let p = -1.0 + 2.0 * (i as f64 + 0.5) / n as f64;
let q = -1.0 + 2.0 * (j as f64 + 0.5) / n as f64;
let direction = face_to_dir(Projection::EqualAreaCobe, 4, p, q);
let (face, recovered_p, recovered_q) = dir_to_face(Projection::EqualAreaCobe, direction);
assert_eq!(face, 4);
assert!((p - recovered_p).abs() < 1e-12);
assert!((q - recovered_q).abs() < 1e-12);
```

`warp` / `unwarp` convert between face parameters `(p,q)` and gnomonic tangent
coordinates `(a,b)`. Both domains are `[-1,1]^2`. `face_to_dir` constructs
`normalize(axis + a*right + b*up)`; multiply by the planet radius for a position.
`FACES` matches the viewer exactly: indices are +X, -X, +Y, -Y, +Z, -Z, including
the viewer's right/up orientations.

Inputs to `warp` and `unwarp` must be finite and in their specified domains.
`face_to_dir` panics for an invalid face. `dir_to_face` accepts finite, nonzero
directions of any magnitude and panics for zero or non-finite directions.
It selects the largest absolute component, breaking exact ties **X before Y
before Z**, equivalently the lowest eligible face index. Consequently exact
edges and corners return a canonical face, which may differ from the input
face. Strictly interior round trips recover the original face.

## Projections and measured properties

- **Gnomonic:** direct radial cube projection. Straight face grid lines become
  great-circle arcs; area varies strongly.
- **Equiangular:** `a=tan(pi*p/4)`, independently for both coordinates. Reduces
  area variation but retains the gnomonic grid's geometric skew distribution.
- **EqualAreaCrease:** the supplied closed-form square-ring equal-area map.
  Continuous, with discontinuous first derivatives on the face diagonals.
- **Cobe:** the supplied NASA COBE CSC polynomial, all 28 coefficients. The
  inverse numerically inverts that same polynomial, without a second fit.
- **EqualArea:** a Moser transport of uniform area to the sphere, represented
  by a tensor quintic spline. It is C4 inside a face, including across the
  diagonals and axes, with no crease. The inverse uses the spline's analytic
  Jacobian in Newton iterations.
- **EqualAreaCobe:** Moser transport using COBE's density, followed by a small
  area-preserving adjustment of the input grid to control cell curvature.
  The composed map is stored as another C4 quintic spline, with the same
  exact symmetries, boundaries, and Newton inverse. It retains COBE's shape
  while meeting the 17% off-square target and the original area tolerance.

Measurements on 2026-10-25, Windows x86_64 MSVC, Rust 1.94.1, Ryzen 7 2700X:

| Projection | Cell area max/min, N=1024 | Surface >10 degrees off square | Worst sampled skew | Worst edge-length ratio (stretch) | Diagonal crease | `face_to_dir` ns | `dir_to_face` ns |
|---|---:|---:|---:|---:|---|---:|---:|
| Gnomonic | 5.185991062 | 31.05% | 29.917 degrees | 1.412442 | No | 14.2 | 12.7 |
| Equiangular | 1.413128875 | 31.05% | 29.870 degrees | 1.411442 | No | 41.0 | 50.0 |
| EqualAreaCrease | 1.000283343 | 15.73% | 20.620 degrees | 1.567693 | Yes | 61.2 | 59.5 |
| Cobe | 1.214120177 | 15.76% | 29.808 degrees | 1.538024 | No | 32.3 | 333.6 |
| EqualArea | 1.000000957 | 23.09% | 29.844 degrees | 1.848545 | No | 120.7 | 970.8 |
| **EqualAreaCobe** | **1.000001372** | **14.03%** | **29.718 degrees** | **1.588324** | **No** | **127.1** | **1015.6** |

Area measurements evaluate **every cell** of one face as the spherical excess
of two triangles; all faces are rotations of the same map. Skew is the absolute
deviation of the angle between Jacobian columns from 90 degrees. Shape statistics
use a 400 by 400 midpoint grid, weighted by spherical area. They are sampled
estimates, not certified global extrema; the smooth maps reach 30 degrees at
the exact cube corners. Edge-length ratio is the longer Jacobian column divided
by the shorter, the local ratio of the two grid-edge lengths. The test also
reports the distinct singular-value ratio: 1.848549 for EqualArea and 1.722261
for EqualAreaCobe. The original EqualArea construction does not minimize skew;
EqualAreaCobe reduces the off-square share by 9.06 percentage points.

Speed numbers are **provisional**, measured with the release timing test alone,
8 passes over 8,192 points and `black_box`. Other missions may share the machine;
these are not isolated-machine performance guarantees. No GPU was used.

The supplied COBE polynomial is less area-preserving near corners than the
brief's approximately +/-2% estimate: the N=1024 cell extrema are -4.988% and
+15.356% relative to the mean. Independently differentiating its reference
coefficients gives **+15.875% at the exact corner**. The implementation retains
the supplied coefficients and matches 1,313 reference pairs within `1.11e-16`.

## Area, smoothness, and boundaries

For EqualArea, the N=1024 triangle-area extrema relative to the mean are
`0.999999601027` and `1.000000557769`: worst error **5.578e-7**, below `1e-6`.
The sampled continuous Jacobian error is much smaller, **1.47e-11** relative to
`pi/6`, including the quadrant boundaries and corners.

For EqualAreaCobe, the corresponding N=1024 extrema are `0.999999494867`
and `1.000000866958`: worst relative error **8.670e-7**, also below `1e-6`.
The continuous Jacobian error on the same 513 by 513 quadrant grid is
**1.791e-11**. Both smooth maps pass the unchanged `1e-6` cell-area criterion;
the max/min column is a spread, rather than the maximum deviation from the mean.
Over 100,000 points across all faces, EqualAreaCobe's worst round-trip coordinate
error is **3.331e-16**. Direct refined-flow fixtures independently check its
forward coordinates to **1.917e-13**.

Equal area refers to the footprint bounded by the images of parameter-grid
lines. Those lines generally curve on the sphere. Replacing them with four
great-circle segments, as the two-triangle test does, introduces an additional
error of order `1/N^2` for the smooth map. The `1e-6` triangle-area result is
verified at N=1024; it is not a promise for arbitrarily coarse triangulations.
Continuous footprints remain equal-area at arbitrary resolutions to the
numerical maps' accuracy. Gaussian integration checks resolutions 1, 2, 3, 7,
16, 31, 64, 127, 1024, and 131072, with observed error at most `6.27e-12`
for EqualArea and `4.09e-12` for EqualAreaCobe.

Cell width is measured between the two side midpoints by spherical distance.
For each width component, the test compares both directions of adjacent cells
everywhere on the face. Transpose symmetry covers the other width component.

| N | EqualArea maximum adjacent width change | EqualAreaCobe | EqualAreaCrease |
|---:|---:|---:|---:|
| 128 | 1.594460% | 1.718426% | 6.354725% |
| 256 | 0.815712% | 0.876319% | 6.167088% |
| 512 | 0.412592% | 0.441351% | 6.073698% |
| 1024 | 0.207495% | 0.221294% | 6.027126% |

Smooth variation halves with grid spacing. The creased map retains a jump.
Its one-sided differential width jump near the corner is **11.9358%**; cells
centered on the diagonal average the two sides, splitting that into roughly
two 6% changes. The smooth map's corresponding finite-difference comparison is
0.000006% for EqualArea and 0.000018% for EqualAreaCobe, consistent with smooth
variation and finite-difference roundoff. First- and second-derivative continuity
checks cover every spline knot, both diagonals, and the mirrored axes.

The eight face symmetries hold by construction. Corners map to corners and
edges to edges, with identical parameterization on the adjoining face. Tests
check the actual component bits at 1,025 points on each oriented face edge,
including corners. The normalization adds tangent squares symmetrically, so
exchanging the local axes does not change the floating-point result.

The globe's eight cube vertices still join three patches. No corner is removed,
rounded, or replaced by a special inaccurate patch. Each closed face map is
smooth with a nonsingular Jacobian there; the three-patch grid topology is the
remaining corner singularity. Global smoothness of grid coordinates across
those vertices is not claimed.

## Numerical construction

The original EqualArea generator starts from **gnomonic**, whose density admits elementary
antiderivatives, and constructs a symmetric no-flux Moser field directly.
It needs no Poisson solve. On the tangent square define

```text
c        = pi/6
rho(x,y) = (1 + x*x + y*y)^(-3/2)
H(x,y)   = x / ((1 + y*y) * sqrt(1 + x*x + y*y))
m(y)     = H(1,y)
I(x)     = atan(x / sqrt(2 + x*x))
Vx(x,y)  = (H(x,y) - x*m(y) + I(x) - c*x) / 2
Vy(x,y)  = Vx(y,x)
rho_t    = (1-t)*c + t*rho
d phi/dt = -V(phi)/rho_t(phi),       phi(0) = (p,q)
```

`div V = rho-c`, and `V` has zero normal component at the square boundary.
Therefore `d/dt (phi_t^* rho_t) = 0`: the exact flow pulls spherical density
back to constant `pi/6`. All densities stay positive; the field is smooth on
the closed square and respects both mirrors and exchange of coordinates.
The corners are stationary. This provides smoothness without introducing a
diagonal patch boundary or a corner singularity into the numerical solve.

The offline generator transports 2,080 Chebyshev root nodes in one eighth of
the face, with 256 RK4 time steps. It reconstructs an order-64 spectral
representation and fits a quintic tensor spline on 129 by 129 squared-coordinate
nodes. The scalar spline `S` is stored in the factored form

```text
a = p + p*(1-p*p) * S(p*p, q*q)
b = q + q*(1-q*q) * S(q*q, p*p)
```

This enforces edge locations, mirrors, and transpose symmetry independently of
the interpolation error. Quintic splines have continuous derivatives through
order four at the internal knots. There is no interpolation triangle split.
Refining RK4 steps, spectral order, and spline resolution independently changed
coordinates by at most `6.08e-14` in the convergence check.

`data/equal_area.bin` has an eight-byte `CSEQAR01` magic, a little-endian u32
interval count (128), a little-endian u32 degree (5), then 129 by 129
little-endian f64 coefficients in row-major order. The knots repeat each
endpoint six times and have internal values `3/128, 4/128, ..., 125/128`.
Rust checks the header/length at compile time and decodes without unsafe casts.

### COBE-based construction (v2)

Let `B(p,q)` be the existing COBE polynomial returning gnomonic coordinates.
Its spherical density is

```text
rho(p,q) = det(DB(p,q)) / (1 + Bx(p,q)^2 + By(p,q)^2)^(3/2)
```

`tools/solve_equal_area_cobe.py` numerically samples this density on 48 by 48
Chebyshev root nodes in squared coordinates. It integrates the resulting
spectral polynomial to obtain `H`, `m`, and `I`, then constructs the same
transpose-symmetrized no-flux field `V`. There is no assumed elementary
antiderivative of COBE's density. The identity
`T_k(2*p*p-1) = T_(2*k)(p)` permits numerical integration in the ordinary
Chebyshev basis while retaining an even, compact representation. Factoring
`Vx = p*(1-p*p)*G(p*p,q*q)` makes zero boundary flux explicit.

Integrating `d phi/dt = -V(phi)/((1-t)*c+t*rho(phi))` gives the equal-area map
`B(phi(p,q))`. This plain correction already yields **15.86%** off square.
Its N=1024 chord-area error is `6.313e-6`, however, from the retained corner
curvature. A small area-preserving input flow brings that error within the
original tolerance as well. Define

```text
x = p*p; y = q*q; z = x*y; w = (1-x)*(1-y)
psi(p,q) = p*q*(x-y)*(1-x)*(1-y) * P(z,w)
d chi/dt = (d psi/dq, -d psi/dp),  chi(0) = (p,q)
final map = B(phi(chi(p,q)))
```

This Hamiltonian field has zero divergence and zero normal boundary flux;
its flow preserves uniform area, all eight symmetries, and the corners.
`P` is cubic in `z` and linear in `w`. Its eight fixed coefficients appear in
`RELAXATION` in the solver. The optional `tools/tune_equal_area_cobe.py`
experiment used two local linear minimax updates of chord error, then stopped
at the first map meeting both area and shape limits. The sampled total shift
of COBE's input coordinates is at most `0.007262` on the refined spectral grid.
The failed simpler trials are recorded in [FINDINGS.md](FINDINGS.md).

The generator uses 64 RK4 steps for each flow, composes with COBE at 2,080
spectral nodes, and fits the same factored scalar residual as EqualArea. The
final spline has 257 by 257 coefficients and is C4 within each face.
Independent refinements of time steps, map order, density quadrature order,
and spline resolution changed coordinates by at most `2.731e-13`, and relative
Jacobian values by at most `8.719e-12`. A smaller 129 by 129 table met the area
and shape limits but differed from the direct flow by about `1.5e-11`;
the shipped table also passes a `1e-12` direct-flow fixture check.

`data/equal_area_cobe.bin` uses the same checked binary format, with interval
count 256 and internal knots `3/256, 4/256, ..., 253/256`. Only the composed
map is embedded; neither flow runs during a build or runtime call. The original
projections retain their formulas, coefficient evaluation order, and source
assets. Their measured geometric properties are unchanged.

## Build, test, and regenerate

Run from this directory, entirely offline:

```powershell
cargo build --offline
cargo test --offline
cargo test --offline -- --nocapture --test-threads=1
cargo test --offline --release call_timings -- --nocapture --test-threads=1
cargo clippy --offline --all-targets -- -D warnings
```

The normal test profile is optimized so the exhaustive area and round-trip
checks take seconds. The suite includes 100,000 points per projection,
coordinates within `1e-12` to `1e-9` of edges/corners, exact edge ties, both
inverse compositions, reference fixtures, area quadrature, spline derivatives,
width convergence, shape statistics, and CPU timings. EqualAreaCobe participates
in every applicable check, including an assertion of at most 17% off square.

Regeneration requires Python with numpy and scipy, plus numba for the COBE-based
solver, and uses only the CPU:

```powershell
python tools/solve_equal_area.py --convergence
python tools/solve_equal_area_cobe.py --convergence
python tools/generate_reference.py
cargo test --offline -- --nocapture --test-threads=1
```

The solver validates area before replacing the binary and has no role at
runtime or build time. The reference fixture generator extracts the polynomial
coefficients directly from the supplied `tools/reference/csc_check.py`; it does
not require astropy. The original wcslib comparison additionally needs astropy,
which was unavailable in this sandbox.

`python tools/solve_equal_area_cobe.py --plain` reports the unrelaxed result
without writing assets. `--no-write` validates the final construction without
writing. `python tools/tune_equal_area_cobe.py` reproduces the optional small
Hamiltonian search without modifying the shipped coefficients or assets.
The COBE-based solver also generates 1,313 direct-flow reference pairs with
twice the time steps, independently of its spline interpolation.

The generated table is intentionally a source asset for version control, as
requested for this crate. Its SHA-256 in this build is
`6505df9e8158cc0755255d2ccc3a0d103e76eedc2228032ecfb024137ed7321a`.
That original asset is unchanged. The EqualAreaCobe table's SHA-256 is
`f99d69da39ad7cdcb55f7396d29c3544de0914c60d64ce1846a5b41aaba81ee5`.
Solver output may differ in the last bits across numpy/scipy versions or CPU
implementations; acceptance is based on the Rust accuracy tests. See
[FINDINGS.md](FINDINGS.md) for verification scope and limitations.


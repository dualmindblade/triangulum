# cubesphere

Five deterministic planet-grid projections with accurate inverses. Rust 2024,
with only `glam = "0.33"` as a dependency. Runtime calls allocate nothing, use no
mutable state, and require no external files. The smooth equal-area map embeds
a **133,144-byte** numerical table with `include_bytes!`.

```rust
use cubesphere::{Projection, face_to_dir, dir_to_face};

let n = 131_072_u32;
let (i, j) = (1234, 5678);
let p = -1.0 + 2.0 * (i as f64 + 0.5) / n as f64;
let q = -1.0 + 2.0 * (j as f64 + 0.5) / n as f64;
let direction = face_to_dir(Projection::EqualArea, 4, p, q);
let (face, recovered_p, recovered_q) = dir_to_face(Projection::EqualArea, direction);
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

Measurements on 2026-10-24, Windows x86_64 MSVC, Rust 1.94.1, Ryzen 7 2700X:

| Projection | Cell area max/min, N=1024 | Surface >10 degrees off square | Worst sampled skew | Diagonal crease | `face_to_dir` ns | `dir_to_face` ns |
|---|---:|---:|---:|---|---:|---:|
| Gnomonic | 5.185991062 | 31.05% | 29.917 degrees | No | 9.3 | 9.7 |
| Equiangular | 1.413128875 | 31.05% | 29.870 degrees | No | 25.2 | 49.1 |
| EqualAreaCrease | 1.000283343 | 15.73% | 20.620 degrees | Yes | 38.6 | 42.4 |
| Cobe | 1.214120177 | 15.76% | 29.808 degrees | No | 21.6 | 291.5 |
| EqualArea | **1.000000957** | 23.09% | 29.844 degrees | **No** | 121.1 | 995.6 |

Area measurements evaluate **every cell** of one face as the spherical excess
of two triangles; all faces are rotations of the same map. Skew is the absolute
deviation of the angle between Jacobian columns from 90 degrees. Shape statistics
use a 400 by 400 midpoint grid, weighted by spherical area. They are sampled
estimates, not certified global extrema; the smooth maps reach 30 degrees at
the exact cube corners. The largest sampled singular-value ratio for EqualArea
is 1.84855. This construction prioritizes smoothness and uniform area; it does
not minimize skew.

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

Equal area refers to the footprint bounded by the images of parameter-grid
lines. Those lines generally curve on the sphere. Replacing them with four
great-circle segments, as the two-triangle test does, introduces an additional
error of order `1/N^2` for the smooth map. The `1e-6` triangle-area result is
verified at N=1024; it is not a promise for arbitrarily coarse triangulations.
Continuous footprints remain equal-area at arbitrary resolutions to the
numerical map's accuracy. Gaussian integration checks resolutions 1, 2, 3, 7,
16, 31, 64, 127, 1024, and 131072, with observed error at most `6.27e-12`.

Cell width is measured between the two side midpoints by spherical distance.
For each width component, the test compares both directions of adjacent cells
everywhere on the face. Transpose symmetry covers the other width component.

| N | EqualArea maximum adjacent width change | EqualAreaCrease |
|---:|---:|---:|
| 128 | 1.594460% | 6.354725% |
| 256 | 0.815712% | 6.167088% |
| 512 | 0.412592% | 6.073698% |
| 1024 | 0.207495% | 6.027126% |

Smooth variation halves with grid spacing. The creased map retains a jump.
Its one-sided differential width jump near the corner is **11.9358%**; cells
centered on the diagonal average the two sides, splitting that into roughly
two 6% changes. The smooth map's corresponding finite-difference comparison is
0.000006%, consistent with smooth variation and finite-difference roundoff.

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

The generator starts from **gnomonic**, whose density admits elementary
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
width convergence, shape statistics, and CPU timings.

Regeneration requires Python with numpy and scipy, and uses only the CPU:

```powershell
python tools/solve_equal_area.py --convergence
python tools/generate_reference.py
cargo test --offline -- --nocapture --test-threads=1
```

The solver validates area before replacing the binary and has no role at
runtime or build time. The reference fixture generator extracts the polynomial
coefficients directly from the supplied `tools/reference/csc_check.py`; it does
not require astropy. The original wcslib comparison additionally needs astropy,
which was unavailable in this sandbox.

The generated table is intentionally a source asset for version control, as
requested for this crate. Its SHA-256 in this build is
`6505df9e8158cc0755255d2ccc3a0d103e76eedc2228032ecfb024137ed7321a`.
Solver output may differ in the last bits across numpy/scipy versions or CPU
implementations; acceptance is based on the Rust accuracy tests. See
[FINDINGS.md](FINDINGS.md) for verification scope and limitations.


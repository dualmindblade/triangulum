# M1-grid findings — 2026-10-24

Implemented the standalone Rust 2024 `cubesphere` crate with the requested API,
all five projections, exact viewer face frames, and the sole dependency glam
0.33 (offline lockfile selects 0.33.12). All runtime functions are pure,
allocation-free and thread-safe. Changes are confined to this crate; no agent
delegation, network, GPU execution, or git commit was used.

The smooth EqualArea map is a Moser flow with an analytically integrated,
symmetrized no-flux field. Starting from gnomonic makes this construction
particularly simple; it avoids a Poisson solver. The shipped quintic spline is
C4 within each face and has no diagonal crease. Only 133,144 bytes are embedded.
The solver operates on one eighth of a face, preserves the cube corners, and
never runs during a Rust build or at runtime. Full derivation, measurements,
binary format, and regeneration commands are in [README.md](README.md).

## Measured acceptance results

- **Round trips:** 100,000 points per projection, distributed across all six
  faces, with edge/corner distances between `1e-12` and `1e-9`. Worst parameter
  errors: Gnomonic `1.11e-16`, Equiangular `2.22e-16`, EqualAreaCrease `5.55e-16`,
  Cobe `3.33e-16`, EqualArea `3.33e-16`. Both inverse compositions pass.
- **EqualArea N=1024:** every one of 1,048,576 cells checked using two spherical
  triangles. Minimum/mean `0.999999601027`, maximum/mean `1.000000557769`,
  worst relative deviation **`5.578e-7`**. The face-area sum matches `2*pi/3`.
- **Continuous area:** maximum sampled analytic Jacobian error **`1.47e-11`**
  relative to `pi/6` on the generator's 513 by 513 quadrant grid, including
  corners. Rust random/boundary sampling gives `1.15e-11` or less. Integrated
  curved-cell checks from N=1 through N=131072 give errors at most `6.27e-12`.
- **Smoothness:** maximum adjacent EqualArea width changes at N=128, 256, 512,
  1024 are 1.594460%, 0.815712%, 0.412592%, 0.207495%. The creased reference
  instead retains a 6.027126% adjacent-cell jump at N=1024. Its one-sided
  differential jump is 11.935843%; the smooth map gives 0.000006% in the same
  finite-difference comparison. Derivative continuity checks cover all knots.
- **Edges and symmetries:** all eight symmetries checked; 1,025 samples on each
  oriented face edge, including exact corners, agree component-for-component.
  The test additionally checks the floating-point bits, not just a tolerance.
- **COBE:** 1,313 fixtures generated directly from the supplied Python
  coefficients agree within `1.11e-16`. Independent coefficient differentiation
  confirms that its exact corner density is **15.874967% above the mean**.
- **Convergence:** doubling time steps changes coordinates by `6.073e-14`;
  increasing spectral order from 64 to 96 changes them by `2.065e-14`;
  doubling spline intervals changes them by `3.375e-14`. Changes in sampled
  relative Jacobian are at most `1.66e-11`.
- **Table integrity:** SHA-256
  `6505df9e8158cc0755255d2ccc3a0d103e76eedc2228032ecfb024137ed7321a`.

Surface fractions more than 10 degrees off square are 31.05%, 31.05%, 15.73%,
15.76%, and 23.09%, respectively, for Gnomonic, Equiangular, EqualAreaCrease,
Cobe, and EqualArea. These are area-weighted 400 by 400 samples. The smooth
EqualArea map's worst sampled skew is 29.844 degrees, and its corner limit is
30 degrees. This map optimizes uniform area and regularity, not minimum skew.

Release CPU timings on the Ryzen 7 2700X, Rust 1.94.1 Windows MSVC, are
provisional because this machine is shared with other missions:

| Projection | `face_to_dir` ns | `dir_to_face` ns |
|---|---:|---:|
| Gnomonic | 9.3 | 9.7 |
| Equiangular | 25.2 | 49.1 |
| EqualAreaCrease | 38.6 | 42.4 |
| Cobe | 21.6 | 291.5 |
| EqualArea | 121.1 | 995.6 |

## Verification and limits

`cargo build --offline`, `cargo test --offline` (13 tests), and
`cargo clippy --offline --all-targets -- -D warnings` pass. The release timing
test also passes. The generator's area and independent convergence checks pass.
Formatting is checked with `cargo fmt --check`.

An exact boundary point belongs to multiple faces. Recovering every caller's
original face there is mathematically impossible; the documented deterministic
rule chooses X before Y before Z. Interior tests require the original face;
exact-boundary tests require the canonical face and identical reconstructed
direction.

The mathematical flow is exactly equal-area; the stored floating-point spline
is an approximation. Accuracy was established by exhaustive N=1024 cell tests,
Jacobian sampling, quadrature, and convergence tests, not a formal interval
bound over all real inputs. Large-N quadrature checks sample representative
cells rather than enumerate the whole planet. The four-vertex spherical
triangle approximation has additional curvature error at coarse N, so a
`1e-6` promise for that approximation at every N would be incorrect. The
continuous footprints, which define equal-area grid cells, do not have that
chord error.

The eight three-face corner junctions remain. Each individual closed face map
is smooth and nonsingular there; a globally regular square grid through a
three-face junction is not claimed. Cross-platform bitwise reproducibility
was not tested; shared edges in this Windows build agree exactly.

The supplied `csc_check.py` wcslib comparison could not be rerun because
`astropy` is not installed. Its polynomial was independently evaluated for
fixtures and differentiated for the area check; the brief's earlier wcslib
verification is not presented as a new sandbox result. No dependencies were
downloaded. The binary source asset is left in place for review and eventual
commit, as requested; this sandbox run did not commit or stage files.

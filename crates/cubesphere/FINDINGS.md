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

## v2 — 2026-10-25: EqualAreaCobe

Added `Projection::EqualAreaCobe`. It corrects COBE's grid with a numerically
integrated, symmetric no-flux Moser flow and a small area-preserving Hamiltonian
relaxation. The result is a C4 tensor quintic spline within each face. Its
**14.027500%** area-weighted share more than 10 degrees off square passes the
17% target. The original EqualArea, Cobe, EqualAreaCrease, Gnomonic and
Equiangular maps retain their formulas and geometric results.

The final table is **528,408 bytes**, embedded with `include_bytes!`.
Newton inversion uses the exact Jacobian of the stored spline, as for the
original EqualArea. There are no new Rust dependencies, allocations, mutable
globals, runtime data reads, or build-time solver steps.

### Comparison measured in v2

All area ratios below use all 1,048,576 two-triangle cells of a face at N=1024.
Shape uses the original `shape_metrics` 400 by 400 grid, diagonal offsets,
centered finite differences and spherical-area weighting without modification.
Stretch is now explicitly reported as the ratio of the two Jacobian column
lengths; the previous singular-value ratio is also retained in test output.

| Projection | Area max/min | Surface >10 degrees off square | Worst sampled skew | Worst edge-length ratio | Crease | Forward ns | Inverse ns |
|---|---:|---:|---:|---:|---|---:|---:|
| EqualAreaCobe | 1.000001372 | 14.027500% | 29.718348 degrees | 1.588324 | No | 127.1 | 1015.6 |
| EqualArea | 1.000000957 | 23.087500% | 29.843836 degrees | 1.848545 | No | 120.7 | 970.8 |
| Cobe | 1.214120177 | 15.763479% | 29.808361 degrees | 1.538024 | No | 32.3 | 333.6 |
| EqualAreaCrease | 1.000283343 | 15.730000% | 20.620195 degrees | 1.567693 | Yes | 61.2 | 59.5 |

Timings are one isolated invocation of the release timing test on the same
Ryzen 7 2700X / Windows MSVC / Rust 1.94.1 setup, eight passes over 8,192
points. They remain provisional wall-clock measurements on a shared machine.
The new map's largest sampled singular-value ratio is 1.722261, compared with
1.848549 for EqualArea, 1.725381 for Cobe and 1.567694 for EqualAreaCrease.
All smooth maps still have the cube-corner limiting skew of 30 degrees.

### Construction and experiments

COBE's exact polynomial Jacobian provides the density `rho`. A 48 by 48
Chebyshev projection in squared coordinates numerically integrates that density
to obtain `H(p,q)`, `m(q)` and `I(p)`. The field is
`Vx=(H-p*m+I-c*p)/2`, `Vy=Vx(q,p)`, with `c=pi/6`, as in v1 but using COBE's
density. Polynomial division extracts `p*(1-p*p)` so zero boundary flux is
explicit. The density quadrature mean differs from `pi/6` by `1.11e-16`, and
the division remainder is `1.43e-17`. The solver removes the constant-mode
quadrature roundoff and uses the original polynomial density in the Moser ODE.

Plain Moser-from-COBE already met the shape target: **15.860000%**, with
sampled continuous-area error `1.785e-10` using the initial 129 by 129 spline.
However, its N=1024 two-triangle area error was **6.313e-6**. This is chord
truncation error from the retained curvature, not failure of continuous equal
area. To preserve the original test's `1e-6` criterion, the search continued
only far enough to reduce that curvature while retaining the shape target.

The relaxation is the time-one flow of `(psi_q,-psi_p)`, with

```text
x=p*p, y=q*q, z=x*y, w=(1-x)*(1-y)
psi=p*q*(x-y)*(1-x)*(1-y)*P(z,w)
```

Its divergence is zero, its normal boundary velocity is zero, and the vector
field respects mirrors and coordinate exchange. Precomposing the Moser map
with this flow preserves equal area. No blending with the old gnomonic-based
map or corner patch is used.

Trials, in order:

1. Single terms `P=alpha*z^k` were tested with several strengths and powers
   0, 1, 2, 3, 4 and 8. They could improve either shape or curvature, but did
   not pass both checks. For example, `P=-0.05*z^2` retained 16.275% off
   square while its coarse N=256 chord error was still `4.043e-5`.
2. An unconstrained cubic `P(z)` minimax fit reduced the coarse chord error but
   increased the off-square share to **24.4275%**; rejected.
3. Constraining the constant term to keep shape, and allowing degree five in
   `z`, yielded **15.0375%** off square but **1.470e-6** N=1024 chord error;
   rejected.
4. Adding one power of `w` allows interior curvature to change independently
   of the edge parameterization. A cubic in `z`, linear in `w` has eight
   coefficients. Two linear minimax updates on a 65 by 65 quadrant grid
   produced the first map passing both checks; the search stopped. Its fixed
   coefficients are rounded to nine decimal places in `RELAXATION` in
   `tools/solve_equal_area_cobe.py`. The optional experiment is reproducible
   through `tools/tune_equal_area_cobe.py` and writes no assets.

The final map is `COBE(phi(chi(p,q)))`. The sampled total COBE input-coordinate
shift is `0.007250` on the production spectral nodes and `0.007262` on refined
nodes. Both flows use 64 RK4 steps; 2,080 root nodes in one eighth of a face
are then composed with COBE and fitted in the same factored residual format
as v1. A 129 by 129 spline passed area and shape but missed the new direct-flow
fixture's initial `1e-11` tolerance. Increasing to 257 by 257 coefficients
reduced that discrepancy below `2e-13`, and the final fixture test requires
`1e-12`. This changes interpolation precision only, not the selected map.

### Acceptance and convergence

- **N=1024 area:** minimum/mean `0.999999494867`, maximum/mean
  `1.000000866958`, worst relative error **8.670e-7**. The unchanged `1e-6`
  acceptance assertion passes. Max/min is a spread and can exceed
  `1+1e-6` while both extrema remain within `1e-6` of the mean.
- **Continuous area:** maximum relative Jacobian error **1.791e-11** on the
  513 by 513 closed quadrant grid; **1.076e-11** in Rust random and near-corner
  samples. Gaussian curved-cell quadrature from N=1 through N=131072 has
  worst error **4.083e-12**.
- **Inverse:** 100,000 points across all faces, including points `1e-12` to
  `1e-9` from edges and corners, pass both compositions. Worst parameter
  error **3.331e-16**. Exact boundaries use the established canonical face.
- **Independent forward fixtures:** 1,313 signed-grid and random points
  evaluated directly through both ODEs at 128 steps, without the spline,
  agree within **1.917e-13**.
- **Symmetries and boundaries:** all eight symmetries, corners, and 1,025
  points per oriented face edge pass, including bit-identical directions on
  adjoining faces. All original projections pass the same checks.
- **Smoothness:** C4 by spline construction; first- and second-derivative
  continuity checked at every knot, both diagonals, and mirrored axes.
  Adjacent width changes at N=128, 256, 512, 1024 are 1.718426%, 0.876319%,
  0.441351%, 0.221294%. The one-sided diagonal comparison is 0.000018%,
  versus EqualAreaCrease's persistent 11.935843% differential jump.

Independent solver refinements, on 10,000 fixed random quadrant points:

| Refinement | Maximum coordinate change | Maximum relative Jacobian change |
|---|---:|---:|
| RK4 steps 64 to 128, both flows | 1.155e-14 | 8.719e-12 |
| Map spectral order 64 to 96 | 2.121e-14 | 7.761e-12 |
| Spline intervals 256 to 512 | 2.731e-13 | 5.042e-12 |
| Density quadrature order 48 to 72 | 5.551e-16 | 2.031e-13 |

`cargo build --offline`, `cargo test --offline` (13 tests), the standalone
release timing test, `cargo clippy --offline --all-targets -- -D warnings`,
and `cargo fmt --check` pass. Every applicable test includes EqualAreaCobe;
the shape test now asserts its 17% target. The generator's area, shape and
four independent convergence checks pass. All work remained in
`crates/cubesphere/`, offline, CPU-only, with no delegation or commits.

Table SHA-256 values:

```text
equal_area.bin       6505df9e8158cc0755255d2ccc3a0d103e76eedc2228032ecfb024137ed7321a
equal_area_cobe.bin  f99d69da39ad7cdcb55f7396d29c3544de0914c60d64ce1846a5b41aaba81ee5
```

The original binary and reference assets were not regenerated or changed.
The same numerical limits as v1 apply: sampled/convergence evidence is not
a formal interval proof, great-circle chord error depends on resolution,
and the globe retains its eight three-face corner junctions.

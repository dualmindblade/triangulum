# Cross-review findings: `14aed27..999ed70`

The review covered all eight commits and used the committed `999ed70` values when evaluating source semantics. The visual play-harness runs used the worktree's pre-existing local weather tuning (`cyclone_arms.count = 2`, `strength = 0.75`) rather than the committed `3`/`0.88`; the targeted CPU diagnostic below explicitly substituted the committed values.

## [major] Inter-arm cloud clearing does not clear cyclone precipitation

**Location:** `viewer/src/weather.rs:1700`, `viewer/src/weather.rs:1709`; downstream manifestation at `viewer/src/renderer.rs:2378`

**Failure scenario:** In the negative lobe between spiral arms, the final cloud cover can be carved from an overcast cyclone deck down to a thin or nearly clear lane. Cyclone precipitation is then added independently from `cyclone_positive`, without applying the same arm-density modulation or the resulting cover. Rain-particle density therefore remains storm-core strength beneath a visually sparse cloud lane. This contradicts the adjacent invariant that rain should follow cover and makes the CPU weather result disagree internally even though the CPU/GPU arm geometry itself matches.

**Evidence:** A temporary diagnostic called the production weather functions with the real assets, two resolved cyclone systems at `t = 3500 s`, and the committed arm tuning (`count = 3`, `strength = 0.88`). It sampled 720 azimuths across normalized radii `0.30..2.60` and compared the result with an otherwise identical `arm_strength = 0` run. There were 1,653 samples where arm clearing exceeded `0.25` while precipitation exceeded final cover. The worst observed sample was system 1 at `r = 0.85`, azimuth `292.0°`: no-arm cover `1.000000`, arm-on cover `0.124714`, precipitation `0.508447`, humidity `0.720761`. The renderer consumes this precipitation directly for rain particles. The diagnostic source was removed after the run.

## [major] The documented dual-token tunnel deployment fails at account discovery

**Location:** `scripts/deploy_tunnel.ps1:29`, `scripts/deploy_tunnel.ps1:32`, `scripts/deploy_tunnel.ps1:36`

**Failure scenario:** Following the script's documented least-privilege setup, an operator supplies a Cloudflare Tunnel API token in `CLOUDFLARE_TUNNEL_API_TOKEN` and a separate zone/DNS token in `CLOUDFLARE_DNS_API_TOKEN`. The script nevertheless calls `GET /zones/{zone_id}` with the tunnel header (`$h`) to discover the account ID. A tunnel-only token lacks the zone permission required by that endpoint, so deployment stops with a 403 before tunnel lookup/creation, DNS reconciliation, or config installation. It works only if the tunnel token is silently granted an additional zone permission, defeating the advertised token split.

**Evidence:** Static request/header tracing shows `$h` is built from `CLOUDFLARE_TUNNEL_API_TOKEN`, `$hDns` is built from the DNS token, and line 36 uses `$h` for the zone-details request. Cloudflare's official [Zone Details API](https://developers.cloudflare.com/api/resources/zones/methods/get/) lists zone-scoped permissions for that request, whereas the official [tunnel setup documentation](https://developers.cloudflare.com/tunnel/setup/) identifies Cloudflare Tunnel Write as the permission for tunnel management. This was not live-executed because no Cloudflare credentials were present; the failure follows from the script's unambiguous credential selection and the endpoint permissions.

## [minor] `--public-url` accepts values that can only produce invalid invites

**Location:** `viewer/server/src/main.rs:107`, `viewer/server/src/main.rs:268`

**Failure scenario:** Starting the server with `--public-url 'wss://host?x=1'` passes validation because only the scheme prefix is checked. Invite construction then appends `/?token=...`, producing `wss://host?x=1/?token=TOKEN`. `parse_invite` sees only the `x` query key and rejects the printed invite as missing its token. Even `wss://` by itself passes the same validation and yields an unusable invite. The server continues running, so the first visible failure is a client being unable to join from the server's advertised URL.

**Evidence:** The validation is a literal `starts_with("ws://") || starts_with("wss://")`; construction only trims trailing slashes before concatenating `/?token=`. The invite tests cover a normal `wss://host/?token=...` value but no malformed authority or pre-existing query. Directly applying the production parser to the constructed query-bearing form returns the missing-token error.

## [minor] Very large finite absolute times break lifecycle and zonal-slosh arithmetic

**Location:** `viewer/src/weather.rs:1310`, `viewer/src/weather.rs:1372`; accepted and uploaded at `viewer/src/renderer.rs:1375`, `viewer/src/renderer.rs:2603`

**Failure scenario:** The public time setter accepts every finite non-negative `f64`. At sufficiently large values, `cycle_t - floor(cycle_t / life) * life` loses the lifecycle remainder to cancellation, pinning systems to an epoch boundary and making every cyclone intensity zero. Near `f64::MAX`, computing `TAU * t_s / period` overflows before division, so `zonal_shear_phase` becomes NaN and that NaN is uploaded to `weather13.y`. The GPU cloud-domain rotation then has non-finite input. Thus the implementation is not a pure finite O(1) weather result for every time the API accepts, and huge seeks can erase storms or make deck sampling non-portable.

**Evidence:** A temporary probe using the production code remained finite and populated at ordinary times, but at `t = 1e20 s` both resolved systems had exactly zero intensity, and the same was true at `1e100 s`. At `f64::MAX`, `zonal_shear_phase.is_finite()` was false. The probe source was removed. Negative times themselves use `floor`/`rem_euclid` correctly; this finding is specifically about large accepted finite times and floating-point operation order.

## [note] Structured cyclone shading adds a small measurable orbit cost

**Location:** `viewer/src/shader.wgsl:1451`

**Failure scenario:** Views that shade the cyclone structure for multiple cloud shells now perform a per-system tangent-frame build plus `atan2`, `log`, arm trigonometry, and comma-front calculations per sample. On the existing cyclone orbit benchmark this consumes a modest but repeatable amount of frame budget. This is separate from the ledgered synchronous weather-raster bake regression.

**Evidence:** I built a disposable archive of `14aed27` against the same assets and ran the existing warmed `cyclones-bench.play` workload. Two baseline orbit runs averaged `9.925 ms` with mean p95 `10.69 ms`; three current runs averaged `10.377 ms` with mean p95 `11.31 ms`, increases of about `4.6%` and `5.8%`. The warmed ground view did not regress (baseline mean `26.345 ms`, p95 `27.475 ms`; current mean `25.810 ms`, p95 `27.237 ms`). The worktree arm count/strength differs from committed tuning, but those scalar values do not change the shader instruction path or the two-system loop count. This is recorded as a budget note, not a correctness defect.

## Found clean

- **CPU/GPU one-truth parity:** Arm azimuth, handed logarithmic twist, phase sign, signed sharpening, radial window, comma-tail wrap/asymmetric widths/radial window, eye/eyewall profiles, core-wrap rotation, and zonal-slosh consumption match between `structured_loads` and `cloud_structure`. `weather15.xyz` is consistently interpreted in cyclone-radius units after `999ed70`. At the normal 6,300-second lifecycle, f32 uniform truncation of arm phase and wrap angle is well below raster/deck resolution.
- **Determinism in the operating range:** Epoch salting, negative-epoch arithmetic, phase-staggered rebirth, and the grow/decay envelope are deterministic; the envelope reaches exactly zero at both lifecycle boundaries. Repeated rotation captures were byte-identical. The large-time exception is reported above.
- **Cover pipeline range safety:** The planetary band enters before `smooth01`, post-clamp arm/eye multipliers are clamped back to `[0, 1]`, and humidity/storm outputs remain bounded. Eye suppression and positive cyclone precipitation agree at the core. The inter-arm precipitation exception is reported above.
- **Shader threshold composition:** Mid and low cloud smoothstep upper bounds remain fixed positive distances (`+0.12` and `+0.11`) above their shifted lower bounds; the cirrus band also retains a positive gap. Combined eye, wall, front, and arm shifts can saturate formation but cannot invert a smoothstep interval. The ledgered pinned-weather behavior was not re-reported.
- **Multiplayer/TLS normal path and secret hygiene:** Normal `ws://` and `wss://` invite parsing and token extraction pass; the rustls/ring feature combinations compile and the default-provider installation path is safe when another provider is already installed. No credential material was found in the reviewed history, and the new token/credential filenames are ignored. Exceptions in public-URL validation and the dual-token script are reported above.
- **Mesh/voxel synchronization:** `viewer/scripts/sync_diff.py` passed, including the required `sea_calib` divergence of `0.0%`.
- **Build and test surface:** `cargo test --release --workspace` passed (viewer: 99 passed, 1 ignored; multiplayer: 11 passed; golden: 1 passed), as did `cargo check --release --features multiplayer -p triangulum-viewer` and the release play-example build. `cyclone-arms-check.play`, `cyclone-rotation.play`, `cyclone-age.play`, and `timeevolve.play` completed successfully; the four repeated rotation frames had identical SHA-256 hashes.

## Overall verdict

The core CPU/GPU cyclone geometry, threshold composition, ordinary-time determinism, TLS client path, and test surface are sound, but I would not treat this range as deployment-clean yet: inter-arm clearing can visibly decouple rain from cloud cover, and the advertised least-privilege Cloudflare dual-token workflow fails before deployment. The malformed-public-URL acceptance and huge-time arithmetic are narrower robustness defects, while the structured-shading cost is measurable but modest.

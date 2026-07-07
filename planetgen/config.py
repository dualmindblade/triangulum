"""All tunable parameters for planet generation.

Everything interesting to tweak lives here. Each stage derives its own RNG
stream from (seed, stage), so changing e.g. a climate parameter never changes
the tectonics for the same seed.
"""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, fields


@dataclass
class PlanetConfig:
    # ---- identity ----
    seed: int = 42
    name: str = ""                      # generated from seed if empty

    # ---- grid ----
    subdivisions: int = 6               # resolution level: 6 = 40,962 cells (~165 km), 7 = 163,842 (~82 km), 8 = 655k (~41 km)
    radius_km: float = 6371.0
    grid_kind: str = "fibonacci"        # "fibonacci" (Delaunay of a spiral lattice, structure-free)
                                        # or "icosphere" (subdivided icosahedron, kept for comparison)
    grid_jitter: float = 0.12           # vertex jitter (fraction of cell spacing). 0.12 breaks the
                                        # fibonacci spiral rings; the icosphere needs ~0.35 for its faces
    axial_tilt_deg: float = 23.44

    # ---- tectonics ----
    n_plates: int = 11                  # major plates
    max_subplates: int = 3              # largest plates split into up to this many subplates
    plate_warp_amp: float = 0.55        # domain-warp strength for plate boundary wiggle (unit-sphere units)
    plate_warp_freq: float = 1.3
    plate_omega_deg_myr: tuple = (0.25, 0.9)   # angular speed range, degrees/Myr (~28-100 km/Myr at pole equator)
    continental_fraction: float = 0.40  # fraction of surface area carrying continental crust
    ocean_fraction: float = 0.71        # sea level is solved so this fraction is submerged
    craton_center_bias: float = 0.9     # pushes continental crust toward plate interiors (passive margins)
    shelf_taper_km: float = 260.0       # continental crust ramps down toward its edge over this distance
    shelf_edge_drop_km: float = 1.1     # how far below craton base the crust edge sits

    # boundary feature amplitudes (km) and widths (km)
    orogen_amp_km: float = 4.4          # continent-continent collision ranges
    orogen_width_km: float = 420.0
    arc_amp_km: float = 2.7             # ocean-continent volcanic cordillera (Andes style)
    arc_width_km: float = 150.0
    arc_offset_km: float = 190.0        # arc distance inland from the trench boundary
    trench_amp_km: float = 3.6
    trench_width_km: float = 110.0
    island_arc_amp_km: float = 5.2      # ocean-ocean arcs; must beat ~5 km seafloor to emerge as islands
    island_arc_width_km: float = 95.0
    rift_amp_km: float = 1.3            # continental rift valley depth
    rift_width_km: float = 95.0
    transform_amp_km: float = 0.35
    conv_rate_norm: float = 55.0        # km/Myr of convergence considered "full strength"
    seafloor_age_max_myr: float = 170.0
    seafloor_depth_kms: tuple = (2.55, 0.24)   # depth = -(a + b*sqrt(age_myr))

    # deep history (eroded ancient orogens on today's continents)
    n_eras: int = 2
    era_amp_factors: tuple = (0.38, 0.18)
    era_width_factors: tuple = (1.8, 2.7)

    # hotspots
    n_hotspots: int = 7
    hotspot_chain_steps: int = 11
    hotspot_step_myr: float = 9.0
    hotspot_amp_km: float = 4.6         # newest ocean seamount height
    hotspot_width_km: float = 75.0

    # terrain noise (km amplitudes)
    noise_mountain_amp: float = 1.5     # ridged noise inside active orogens
    noise_hill_amp: float = 0.55        # continental interiors
    noise_abyssal_amp: float = 0.25     # seafloor texture
    noise_detail_amp: float = 0.18      # fine global grit
    noise_base_freq: float = 2.2
    noise_warp: float = 0.35

    # ---- climate ----
    months: int = 12
    solar_temp_eq_c: float = 29.0       # sea-level radiative temp at the subsolar latitude band
    solar_temp_pole_c: float = -32.0    # ... at zero insolation (land)
    sst_pole_c: float = -6.0            # polar ocean annual mean (currents keep seas warmer)
    lapse_rate_c_km: float = 6.0
    maritime_range_km: float = 1400.0   # how far ocean influence penetrates downwind
    continental_seasonal_boost: float = 1.55  # seasonal swing multiplier deep inside continents
    ocean_seasonal_damp: float = 0.28   # fraction of radiative seasonal swing the ocean keeps
    heat_diffusion_steps: int = 8
    itcz_amplitude_deg: float = 9.0     # seasonal migration of the tropical rain belt (over ocean)
    itcz_land_boost: float = 1.3        # extra migration over land longitudes (land heats/cools fast)
    sst_itcz_coupling: float = 0.6      # ocean convection response to SST anomaly (breaks the rain
                                        # band over cold currents, flares it over warm pools)
    moisture_diffusion: float = 0.12    # isotropic moisture smoothing per step (counters advection streaks)
    trade_wind_ms: float = 7.0
    westerlies_ms: float = 9.5
    polar_easterlies_ms: float = 4.0
    thermal_wind_ms: float = 6.5        # strength of monsoon/thermal circulation response
    current_speed: float = 1.0          # ocean current scale (arbitrary units for advection)
    current_iters: int = 70
    sst_advect_iters: int = 60
    moisture_iters: int = 80            # advection relaxation steps per month
    moisture_carry: bool = True         # warm-start each month from the previous one
    evap_ocean: float = 1.0
    evap_land_factor: float = 0.50      # land evapotranspiration recycling vs ocean evap
    rain_convective: float = 0.06
    rain_orographic: float = 4.0
    rain_frontal: float = 0.08
    rain_base: float = 0.006
    target_land_precip_mm: float = 830.0  # global land annual mean, used to calibrate rainfall units

    # ---- hydrology / erosion ----
    erosion_iters: int = 120            # duration: more steps = closer to uplift/erosion steady state
    erode_k: float = 0.065              # stream-power constant (km units, per iteration)
    erode_m: float = 0.5                # discharge exponent
    erode_cap_km: float = 0.09          # max incision per cell per iteration
    diffusion_k: float = 0.010          # hillslope smoothing per iteration; too high refills carved
                                        # valleys as fast as rivers cut them (terrain just smooths)
    uplift_rate_km: float = 0.033       # active-orogen uplift per erosion step (rate, not total —
    subsidence_rate_km: float = 0.015   # so erosion_iters honestly controls how long the run is)
    sediment_capacity_k: float = 1.4
    deposit_fraction: float = 0.35
    river_capture: float = 0.8          # valley capture: preference for joining existing channels
    bedrock_texture_km: float = 0.30    # weathered ridge grain on thin-soil uplands (dissected look)
    river_meander: float = 0.5          # random per-edge routing weights: winding instead of
                                        # lattice-straight parallel flow on smooth regional slopes
    delta_fraction: float = 0.35        # fraction of river sediment flux dumped at the coast
    river_min_m3s: float = 350.0        # minimum discharge to count as a mapped river
    lake_evap_mm: float = 1100.0        # open-water evaporation at 20 C (scales with temperature)

    # ---- simulation recording / debugging ----
    record: bool = False                # capture frames of the sims as they run -> simviz/player.html
    watch: bool = False                 # also open a live window during the run
    record_width: int = 720
    record_every: int = 4               # capture every Nth moisture step (erosion records every step)
    record_video: bool = False          # additionally assemble mp4 clips (requires ffmpeg)

    # ---- rendering ----
    map_width: int = 2400
    globe_size: int = 1000
    render_views: tuple = (20.0, 110.0, 200.0, 290.0)  # orthographic view center longitudes

    # ------------------------------------------------------------------
    def save(self, path):
        with open(path, "w", encoding="utf-8") as f:
            json.dump(asdict(self), f, indent=2)

    @classmethod
    def load(cls, path) -> "PlanetConfig":
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        valid = {f.name for f in fields(cls)}
        kwargs = {k: (tuple(v) if isinstance(v, list) else v) for k, v in data.items() if k in valid}
        return cls(**kwargs)

    def apply_overrides(self, pairs):
        """Apply 'key=value' strings (CLI --set). Values parsed as JSON when possible."""
        for pair in pairs:
            key, _, raw = pair.partition("=")
            key = key.strip()
            if not hasattr(self, key):
                raise KeyError(f"unknown config key: {key}")
            try:
                val = json.loads(raw)
            except json.JSONDecodeError:
                val = raw
            if isinstance(val, list):
                val = tuple(val)
            setattr(self, key, val)

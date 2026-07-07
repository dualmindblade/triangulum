"""Pipeline orchestration: run stages in order, cache each stage's output.

Stage order:  grid -> tectonics -> climate1 -> hydrology -> climate2
              -> biomes -> chronicle -> render -> export

Every stage's arrays are cached under <out>/cache/, so during tuning you can
change (say) an erosion parameter and re-run with --from hydrology without
recomputing tectonics or the first climate pass.
"""

from __future__ import annotations

import json
import os
import pickle
import time

import numpy as np

from . import biomes, chronicle, climate, hydrology, tectonics
from .config import PlanetConfig
from .grid import Grid

STAGES = ["tectonics", "climate1", "hydrology", "climate2", "biomes", "chronicle", "render"]


class Run:
    def __init__(self, cfg: PlanetConfig, out_dir: str):
        self.cfg = cfg
        self.out = out_dir
        self.cache = os.path.join(out_dir, "cache")
        os.makedirs(self.cache, exist_ok=True)
        self.state = {"cfg": cfg}
        self.grid = None

    # ------------------------------------------------------------------
    def _cache_path(self, stage):
        return os.path.join(self.cache, f"{stage}.pkl")

    def _save_stage(self, stage, payload):
        with open(self._cache_path(stage), "wb") as f:
            pickle.dump(payload, f, protocol=4)

    def _load_stage(self, stage):
        with open(self._cache_path(stage), "rb") as f:
            return pickle.load(f)

    # ------------------------------------------------------------------
    def run(self, from_stage=None, until_stage=None, quiet=False):
        cfg = self.cfg
        t_all = time.time()

        def log(msg):
            if not quiet:
                print(msg, flush=True)

        log(f"[grid] building {cfg.grid_kind} grid, level {cfg.subdivisions} ...")
        t0 = time.time()
        self.grid = Grid(cfg.subdivisions, cfg.radius_km,
                         jitter=cfg.grid_jitter, seed=cfg.seed, kind=cfg.grid_kind)
        self.rec = None
        if cfg.record or cfg.watch:
            from .recorder import Recorder
            self.rec = Recorder(self.grid, os.path.join(self.out, "simviz"),
                                width=cfg.record_width, every=cfg.record_every,
                                live=cfg.watch)
        log(f"[grid] {self.grid.n:,} cells, ~{self.grid.mean_edge_km:.0f} km spacing "
            f"({time.time() - t0:.1f}s)")

        start = STAGES.index(from_stage) if from_stage else 0
        stop = STAGES.index(until_stage) + 1 if until_stage else len(STAGES)

        for si, stage in enumerate(STAGES[:stop]):
            if si < start:
                t0 = time.time()
                payload = self._load_stage(stage)
                self.state.update(payload)
                log(f"[{stage}] loaded from cache ({time.time() - t0:.1f}s)")
                continue
            t0 = time.time()
            payload = getattr(self, f"_stage_{stage}")()
            if payload is not None:
                self.state.update(payload)
                self._save_stage(stage, payload)
            log(f"[{stage}] done ({time.time() - t0:.1f}s)")

        if self.rec is not None:
            player = self.rec.finalize(video=cfg.record_video)
            log(f"[simviz] {sum(len(m['files']) for m in self.rec.meta.values())} frames "
                f"-> {player}")
        if stop == len(STAGES):
            self._export()
        log(f"[all] total {time.time() - t_all:.1f}s -> {self.out}")
        return self.state

    # ------------------------------------------------------------------
    def _stage_tectonics(self):
        return {"tect": tectonics.generate(self.grid, self.cfg, rec=self.rec)}

    def _stage_climate1(self):
        t = self.state["tect"]
        return {"clim1": climate.simulate(self.grid, self.cfg, t["elev"], t["is_ocean"],
                                          rec=self.rec, tag="climate1")}

    def _stage_hydrology(self):
        return {"hydro": hydrology.simulate(self.grid, self.cfg,
                                            self.state["tect"]["elev"],
                                            self.state["clim1"], self.state["tect"],
                                            rec=self.rec)}

    def _stage_climate2(self):
        h = self.state["hydro"]
        lake_mask = h["lake_id"] >= 0
        return {"clim2": climate.simulate(self.grid, self.cfg, h["elev"],
                                          h["is_ocean"], lake_mask=lake_mask,
                                          rec=self.rec, tag="climate2")}

    def _stage_biomes(self):
        h, c = self.state["hydro"], self.state["clim2"]
        koppen = biomes.classify(self.grid, self.cfg, c["T"], c["P"], h["is_ocean"])
        supp = biomes.supplements(self.grid, self.cfg, c["T"], c["P"], koppen,
                                  h["soil"], self.state["tect"]["volcanic"],
                                  h["river"], h["flow_acc"], h["is_ocean"])
        return {"koppen": koppen, "supp": supp}

    def _stage_chronicle(self):
        feats = chronicle.build(self.grid, self.cfg, self.state["tect"],
                                self.state["hydro"], self.state["clim2"],
                                self.state["koppen"])
        if not self.cfg.name:
            self.cfg.name = feats["planet"]
        path = os.path.join(self.out, "CHRONICLE.md")
        chronicle.write_markdown(self.grid, self.cfg, feats, self.state["tect"],
                                 self.state["hydro"], self.state["clim2"],
                                 self.state["koppen"], path)
        return {"features": feats}

    def _stage_render(self):
        from .render import Renderer   # matplotlib import stays optional until needed
        r = Renderer(self.grid, self.cfg, os.path.join(self.out, "maps"))
        r.render_all(self.state)
        return None

    # ------------------------------------------------------------------
    def _export(self):
        """Write the engine-agnostic planet dataset."""
        s = self.state
        h, c, t = s["hydro"], s["clim2"], s["tect"]
        g = self.grid
        arrays = dict(
            xyz=g.xyz.astype(np.float32),
            lat=g.lat.astype(np.float32), lon=g.lon.astype(np.float32),
            neighbors=g.nbr.astype(np.int32),
            area_km2=g.area_km2.astype(np.float32),
            elevation_km=h["elev"].astype(np.float32),
            is_ocean=h["is_ocean"],
            plate=t["plate"].astype(np.int16), craton=t["craton"],
            seafloor_age_myr=t["seafloor_age"].astype(np.float32),
            flow_accum_m3s=h["flow_acc"].astype(np.float32),
            river=h["river"], receiver=h["receiver"].astype(np.int32),
            lake_id=h["lake_id"].astype(np.int32), lake_salt=h["lake_salt"],
            soil_km=h["soil"].astype(np.float32),
            temp_c_monthly=c["T"].astype(np.float32),
            precip_mm_monthly=c["P"].astype(np.float32),
            wind_e_monthly=c["wind_e"].astype(np.float32),
            wind_n_monthly=c["wind_n"].astype(np.float32),
            clouds_monthly=c["clouds"].astype(np.float32),
            sst_monthly=np.nan_to_num(c["sst"], nan=0.0).astype(np.float32),
            seaice_monthly=c["seaice"],
            current_e=c["cur_e"].astype(np.float32),
            current_n=c["cur_n"].astype(np.float32),
            koppen=s["koppen"].astype(np.int8),
            vegetation=s["supp"]["vegetation"].astype(np.float32),
            fertility=s["supp"]["fertility"].astype(np.float32),
            permafrost=s["supp"]["permafrost"],
        )
        np.savez_compressed(os.path.join(self.out, "planet_data.npz"), **arrays)

        feats = s.get("features", {})
        meta = dict(
            planet=feats.get("planet", ""),
            seed=self.cfg.seed,
            cells=int(g.n),
            koppen_classes=[dict(code=c_, name=n) for c_, n, _ in biomes.KOPPEN],
            features={k: [({kk: vv for kk, vv in f.items() if not isinstance(vv, np.ndarray)}
                           if isinstance(f, dict) else f)
                          for f in v] for k, v in feats.items()
                      if isinstance(v, list)},
        )
        with open(os.path.join(self.out, "planet.json"), "w", encoding="utf-8") as f:
            json.dump(meta, f, indent=1, default=str)
        self.cfg.save(os.path.join(self.out, "config.json"))

"""CLI: python -m planetgen [options]

Examples
  python -m planetgen --seed 42 --res 6 --out output/run42
  python -m planetgen --seed 42 --out output/run42 --from hydrology
  python -m planetgen --seed 7 --set n_plates=14 --set ocean_fraction=0.65
"""

from __future__ import annotations

import argparse
import os

from .config import PlanetConfig
from .pipeline import STAGES, Run


def main():
    ap = argparse.ArgumentParser(prog="planetgen",
                                 description="Generate an earthlike planet dataset.")
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--res", type=int, default=6,
                    help="icosphere subdivisions: 6=41k cells (~165 km), 7=164k (~82 km)")
    ap.add_argument("--out", default=None, help="output directory (default output/seed<N>)")
    ap.add_argument("--config", default=None, help="JSON config file to start from")
    ap.add_argument("--set", action="append", default=[], metavar="KEY=VALUE",
                    help="override any config field (repeatable)")
    ap.add_argument("--from", dest="from_stage", choices=STAGES, default=None,
                    help="resume from this stage using cached earlier stages")
    ap.add_argument("--until", dest="until_stage", choices=STAGES, default=None,
                    help="stop after this stage")
    ap.add_argument("--record", action="store_true",
                    help="capture simulation frames -> <out>/simviz/player.html")
    ap.add_argument("--watch", action="store_true",
                    help="open a live window while the simulations run (implies --record)")
    ap.add_argument("--video", action="store_true",
                    help="also assemble mp4 clips per sequence (requires ffmpeg)")
    ap.add_argument("--gpu", action="store_true",
                    help="run the climate stage on the GPU (requires cupy; "
                         "float32 — results match CPU within tolerance)")
    args = ap.parse_args()

    cfg = PlanetConfig.load(args.config) if args.config else PlanetConfig()
    cfg.seed = args.seed
    cfg.subdivisions = args.res
    if args.record:
        cfg.record = True
    if args.watch:
        cfg.watch = True
        cfg.record = True
    if args.video:
        cfg.record_video = True
    if args.gpu:
        cfg.gpu = True
    cfg.apply_overrides(args.set)

    out = args.out or os.path.join("output", f"seed{cfg.seed}_r{cfg.subdivisions}")
    os.makedirs(out, exist_ok=True)
    run = Run(cfg, out)
    state = run.run(from_stage=args.from_stage, until_stage=args.until_stage)

    if "features" in state:
        print(f"\nPlanet '{state['features']['planet']}' complete.")
        print(f"  maps      : {os.path.join(out, 'maps', 'atlas.html')}")
        print(f"  chronicle : {os.path.join(out, 'CHRONICLE.md')}")
        print(f"  dataset   : {os.path.join(out, 'planet_data.npz')}")


if __name__ == "__main__":
    main()

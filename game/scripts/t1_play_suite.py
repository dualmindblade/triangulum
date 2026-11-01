"""Run the shipped .play suite headless, retaining per-script logs and exits.

From game/: python scripts/t1_play_suite.py [script-stem ...]
No stems means every script. Each invocation uses a fresh scratch save folder.
GPU/CPU timings are provisional while other missions are active.
"""
import argparse
import json
from pathlib import Path
import subprocess
import time


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("scripts", nargs="*")
    args = ap.parse_args()
    game = Path(__file__).resolve().parents[1]
    scripts = [game / "scripts" / (s.removesuffix(".play") + ".play") for s in args.scripts] if args.scripts else sorted((game / "scripts").glob("*.play"))
    # The compatibility pair intentionally shares a save.
    if not args.scripts:
        scripts = [p for p in scripts if p.stem not in ["b4-oldsave-make", "b4-oldsave-load", "t1-size"]]
        scripts = [game / "scripts/m1-done.play"] + [p for p in scripts if p.stem != "m1-done"]
        scripts += [game / "scripts/b4-oldsave-make.play", game / "scripts/b4-oldsave-load.play"]
    run = time.strftime("%Y%m%d-%H%M%S")
    root = game / "out/t1/plays" / run
    root.mkdir(parents=True, exist_ok=True)
    results = []
    for script in scripts:
        if (game / "out/t1/STOP").exists():
            raise SystemExit("T1 suite stopped before the next script (out/t1/STOP exists).")
        out = root / script.stem
        out.mkdir(parents=True, exist_ok=True)
        save = root / "saves" / ("oldsave-pair" if script.stem.startswith("b4-oldsave-") else script.stem)
        blocks = game / ("scripts/m3b-look/blocks.json" if script.stem == "m3b-hotbar" else "assets/blocks.json")
        command = [str(game / "target/release/triangulum.exe"), "--play", str(script), "--out", str(out),
                   "--world", str(game / "assets/world.json"), "--blocks", str(blocks), "--look-file", str(game / "assets/look.json"),
                   "--save", str(save), "--interchange", str(out), "--replays", str(out), "--threads", "4", "--size", "1280x720"]
        if script.stem.endswith("vscale8"):
            command += ["--vscale", "0.125", "--height", "4096", "--sea-level", "2048", "--n", "131072"]
        if script.stem == "water-lod-seam":
            command += ["--lod-factor", "0.2"]
        if script.stem == "b7-close":
            command += ["--powder-angle", "35"]
        if script.stem == "b4-oldsave-make":
            command += ["--no-slide"]
        start = time.monotonic()
        print(f"Starting {script.stem}", flush=True)
        with (out / "console.log").open("w") as log:
            try:
                rc = subprocess.run(command, cwd=game, stdout=log, stderr=subprocess.STDOUT, timeout=2400).returncode
            except subprocess.TimeoutExpired:
                rc = "timeout (2400 seconds)"
        results.append({"script": script.name, "exit_code": rc, "wall_s": time.monotonic()-start, "command": command})
        (root / "results.json").write_text(json.dumps(results, indent=2))
        print(f"Finished {script.stem}: {rc}, {time.monotonic()-start:.1f} s", flush=True)
    print(root, flush=True)
    raise SystemExit(any(r["exit_code"] != 0 for r in results))


if __name__ == "__main__":
    main()

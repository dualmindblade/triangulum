"""Repeatable, headless T1 evidence. Run from game/; all timings provisional.

python scripts/t1_planet_check.py --phase before --exe E:/triangulum-build/baseline-8134c61/triangulum.exe --sizes 128
python scripts/t1_planet_check.py --phase after --sizes 128 256 512 832 1024
Add --bench for the same built-in static poses and 60-second flight at each size.
Raw output stays in game/out/t1; selected evidence is copied separately.
"""
import argparse
import json
import math
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, default=Path("target/release/triangulum.exe"))
    parser.add_argument("--phase", choices=["before", "after"], default="after")
    parser.add_argument("--sizes", nargs="+", type=int, default=[128, 256, 512, 832, 1024])
    parser.add_argument("--bench", action="store_true")
    parser.add_argument("--exercise", action="store_true", help="walk, pick, dig, water, save/reload and replay at each size")
    args = parser.parse_args()
    game = Path(__file__).resolve().parents[1]
    exe = args.exe.resolve()
    for size in args.sizes:
        if (game / "out/t1/STOP").exists():
            raise SystemExit("T1 run stopped before the next size (out/t1/STOP exists).")
        out = game / "out" / "t1" / args.phase / str(size)
        if args.exercise:
            out = out / "exercise"
        out.mkdir(parents=True, exist_ok=True)
        world = json.loads((game / "assets/world.json").read_text())
        world.pop("planet_size_half_heights", None)
        if args.phase == "before":
            world["columns_per_face"] = size * 1024
        else:
            world.pop("columns_per_face", None)
            world["planet_size_half_heights"] = size
        world["sea_level"] = 512 if args.phase == "before" else 1024
        world["vertical_scale"] = None
        world["bake"] = str(game / "assets/world/seed42")
        (out / "world.json").write_text(json.dumps(world, indent=2))
        look = json.loads((game / "assets/look.json").read_text())
        if args.phase == "before":
            look["sky"].pop("reference_radius_m", None)
        (out / "look.json").write_text(json.dumps(look, indent=2))
        radius = size * 1024 * math.sqrt(6 / (4 * math.pi))
        script = f"""# Same latitude, longitude, view angles and local solar time at every size.
sim off
mode walk
at -4.5654 117.4744
look -60 -10
skytime 12:00
skyspeed 0
assert on_ground == true
photo ground
bench 120
mode fly
at -4.5654 117.4744 2000
look -60 -20
photo two_km
bench 120
at -4.5654 117.4744 {radius * 1.75:.9f}
look 0 -89.9
photo orbit
bench 120
menu open
menu wait
assert menu_build_ms > 0
menu shot map
menu close
"""
        play = out / "frames.play"
        play.write_text(script)
        command = [str(exe), "--world", str(out / "world.json"), "--blocks", str(game / "assets/blocks.json"),
                   "--look-file", str(out / "look.json"), "--threads", "4", "--no-save",
                   "--interchange", str(out / "interchange"), "--replays", str(out / "replays")]
        if args.bench:
            command += ["--bench", "--frames", "120", "--out", str(out / "bench"), "--label", "T1 provisional, other agents active"]
        elif args.exercise:
            command.remove("--no-save")
            command += ["--play", str(game / "scripts/t1-size.play"), "--out", str(out), "--size", "1280x720"]
        else:
            command += ["--play", str(play), "--out", str(out), "--size", "1920x1080"]
        log = out / ("bench-console.log" if args.bench else "console.log")
        print(f"Starting {args.phase} {size}: {log}", flush=True)
        start = time.monotonic()
        with log.open("w") as stream:
            result = subprocess.run(command, cwd=game, stdout=stream, stderr=subprocess.STDOUT)
        (out / ("bench-command.json" if args.bench else "command.json")).write_text(json.dumps({
            "command": command, "exit_code": result.returncode, "wall_s": time.monotonic() - start,
            "timing": "PROVISIONAL: other agents are active", "size_half_heights": size, "radius_m": radius,
        }, indent=2))
        print(f"Finished {size}: exit {result.returncode}, {time.monotonic() - start:.1f} s", flush=True)
        if result.returncode:
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()

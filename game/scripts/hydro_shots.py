"""M2-hydro evidence shots.

Finds the sites (an ignored test writes one play script per site: the
largest lake's shore, the largest river away from lakes and the sea, the
tallest waterfall at this scale, the largest river mouth, a river crossing a
cube-face edge), then runs each through the play harness starting at that
site, so it is lit by day (10:30 local solar time).

    python scripts/hydro_shots.py [BAKE_DIR]      (from game/)

Frames land in out/play/<site>/.
"""

import os
import subprocess
import sys

bake = sys.argv[1] if len(sys.argv) > 1 else "C:/code/triangulum/game/assets/world/seed42"
here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(here)
subprocess.run(["cargo", "test", "--release", "hydro_places", "--", "--ignored", "--nocapture"], check=True)
with open("out/hydro-shots/sites.txt") as f:
    sites = [line.split() for line in f if line.strip()]
for site, lat, lon in sites:
    subprocess.run(["cargo", "run", "--release", "--", "--play", f"out/hydro-shots/{site}.play",
                    "--bake", bake, "--at", lat, lon, "50", "--time", "10:30"], check=True)

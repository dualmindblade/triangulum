"""Assemble simviz frame sequences (output/*/simviz/*.jpg) into mobile-friendly
mp4 clips with ffmpeg. Always rebuilds, so re-run any time a sequence grows or
encoding settings change.

Usage: python scripts/make_videos.py [output_root]
"""

import glob
import os
import re
import shutil
import subprocess
import sys

output_root = sys.argv[1] if len(sys.argv) > 1 else "output"

if not shutil.which("ffmpeg"):
    sys.exit("ffmpeg not found on PATH")

FRAME_RE = re.compile(r"^(.*)_(\d+)\.(jpg|jpeg|png)$")

for simviz_dir in sorted(glob.glob(os.path.join(output_root, "*", "simviz"))):
    frames = {}
    for name in os.listdir(simviz_dir):
        m = FRAME_RE.match(name)
        if m:
            seq, _, ext = m.groups()
            frames[(seq, ext)] = frames.get((seq, ext), 0) + 1

    for (seq, ext), count in sorted(frames.items()):
        if count < 2:
            print(f"[skip] {simviz_dir}/{seq} - only {count} frame")
            continue
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-framerate", "18",
             "-i", f"{seq}_%04d.{ext}",
             "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2",
             "-c:v", "libx264", "-profile:v", "high", "-level", "4.0",
             "-preset", "medium", "-crf", "18",
             "-pix_fmt", "yuv420p", "-color_range", "tv",
             "-movflags", "+faststart",
             f"{seq}.mp4"],
            cwd=simviz_dir, check=True)
        print(f"[ok]   {simviz_dir}/{seq}.mp4 ({count} frames)")

#!/usr/bin/env python3
"""Render and assemble Solar P3's four landed lunar evidence poses.

Evidence only: there is deliberately no accept/bless path.
"""

import argparse
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


SHOTS = [
    ("crater_floor", "STANDING IN A CRATER FLOOR"),
    ("maria_neisor_horizon", "MARIA HORIZON - NEISOR IN THE SKY"),
    ("ray_field_close", "RAY-BRIGHT REGOLITH - CLOSE"),
    ("dug_regolith", "ONE-BLOCK DIG - SUBSURFACE REGOLITH"),
]


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-render", action="store_true")
    parser.add_argument("--run-dir", type=Path)
    parser.add_argument("--out-dir", type=Path)
    args = parser.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    run_dir = args.run_dir or viewer / "interchange" / "runs" / "moon-surface-contact"
    out_dir = args.out_dir or viewer / "interchange" / "solar-p3"
    if not args.skip_render:
        exe = viewer / "target" / "release" / "examples" / "play.exe"
        play = viewer / "scripts" / "moon-surface-contact.play"
        result = subprocess.run(
            [str(exe), str(play), "--out", str(run_dir)],
            cwd=viewer,
            capture_output=True,
            text=True,
        )
        if result.returncode:
            raise SystemExit(result.stdout[-3000:] + result.stderr[-3000:])

    tile_w, tile_h, label_h = 640, 360, 36
    sheet = Image.new("RGB", (tile_w * 2, (tile_h + label_h) * 2), (3, 4, 8))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=18)
    for index, (name, label) in enumerate(SHOTS):
        source = Image.open(run_dir / f"{name}.png").convert("RGB")
        source.thumbnail((tile_w, tile_h), Image.Resampling.LANCZOS)
        tile = Image.new("RGB", (tile_w, tile_h), (2, 3, 7))
        tile.paste(source, ((tile_w - source.width) // 2, (tile_h - source.height) // 2))
        x = index % 2 * tile_w
        y = index // 2 * (tile_h + label_h)
        sheet.paste(tile, (x, y))
        draw.rectangle((x, y + tile_h, x + tile_w, y + tile_h + label_h), fill=(14, 16, 23))
        draw.text((x + 12, y + tile_h + 8), label, font=font, fill=(230, 232, 238))

    out_dir.mkdir(parents=True, exist_ok=True)
    output = out_dir / "moon-surface-contact-sheet.png"
    sheet.save(output)
    print(output)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Build map and world-reel before/after sheets without blessing baselines."""

import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def labeled_sheet(panels, columns, tile_size, output):
    tile_w, tile_h = tile_size
    label_h = 34
    rows = (len(panels) + columns - 1) // columns
    sheet = Image.new("RGB", (tile_w * columns, (tile_h + label_h) * rows), (5, 6, 10))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=17)
    for index, (path, label) in enumerate(panels):
        image = Image.open(path).convert("RGB")
        image.thumbnail((tile_w, tile_h), Image.Resampling.LANCZOS)
        tile = Image.new("RGB", (tile_w, tile_h), (3, 4, 8))
        tile.paste(image, ((tile_w - image.width) // 2, (tile_h - image.height) // 2))
        x = index % columns * tile_w
        y = index // columns * (tile_h + label_h)
        sheet.paste(tile, (x, y))
        draw.rectangle((x, y + tile_h, x + tile_w, y + tile_h + label_h), fill=(16, 18, 25))
        draw.text((x + 10, y + tile_h + 8), label, font=font, fill=(228, 230, 236))
    output.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output)
    print(output)


def main():
    viewer = Path(__file__).resolve().parent.parent
    mission = viewer / "interchange" / "codex" / "moonscape-v2"
    final = mission / "final"
    final_map = final / "teleport-moon-map.png"
    subprocess.run(
        [
            str(viewer / "target" / "release" / "examples" / "moonmap.exe"),
            str(final_map),
            "42",
            "1024",
        ],
        cwd=viewer,
        check=True,
    )
    labeled_sheet(
        [
            (mission / "baseline" / "teleport-moon-map.png", "BEFORE - GLOBAL 378-CRATER FOLD"),
            (final_map, "AFTER - SCALE-BINNED SATURATION"),
        ],
        2,
        (640, 320),
        final / "teleport-map-before-after.png",
    )

    reel = viewer / "interchange" / "reel"
    labeled_sheet(
        [
            (reel / "accepted" / "moon_orbit_face.png", "BEFORE - ORBIT FACE"),
            (reel / "current" / "moon_orbit_face.png", "AFTER - ORBIT FACE"),
            (reel / "accepted" / "moon_low_flyby.png", "BEFORE - LOW FLYBY"),
            (reel / "current" / "moon_low_flyby.png", "AFTER - LOW FLYBY"),
        ],
        2,
        (640, 360),
        final / "world-reel-moon-before-after.png",
    )


if __name__ == "__main__":
    main()

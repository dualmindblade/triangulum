"""Before/after contact sheets for mission T3 (boundaries).

    python scripts/t3_sheets.py BEFORE_DIR AFTER_DIR OUT_DIR

BEFORE_DIR and AFTER_DIR hold the frames of `t3-poses.play` and `t3-ladder.play`
from two builds. Writes poses.jpg (Andrew's four photos, before beside after)
and ladder_<site>.jpg (one line from orbit to the ground, before above after).
"""

import os
import sys

from PIL import Image, ImageDraw, ImageFont

SITES = ["lake", "biome", "snow", "polar"]
LADDERS = SITES + ["coast"]
HEIGHTS = ["150km", "40km", "10km", "2500m", "600m", "150m", "40m", "foot"]


def font(size):
    try:
        return ImageFont.truetype("consola.ttf", size)
    except OSError:
        return ImageFont.load_default()


def tile(path, w, h):
    return Image.open(path).convert("RGB").resize((w, h), Image.LANCZOS)


def label(sheet, x, y, text, size=18):
    d = ImageDraw.Draw(sheet)
    box = d.textbbox((x, y), text, font=font(size))
    d.rectangle((box[0] - 4, box[1] - 3, box[2] + 4, box[3] + 3), fill=(20, 20, 20))
    d.text((x, y), text, fill=(235, 235, 235), font=font(size))


def poses(before, after, out):
    w, h, gap = 960, 540, 6
    sheet = Image.new("RGB", (2 * w + 3 * gap, len(SITES) * (h + gap) + gap), (20, 20, 20))
    for r, site in enumerate(SITES):
        for c, (d, name) in enumerate([(before, "before"), (after, "after")]):
            x, y = gap + c * (w + gap), gap + r * (h + gap)
            sheet.paste(tile(os.path.join(d, f"pose_{site}.png"), w, h), (x, y))
            label(sheet, x + 8, y + 8, f"{site}: {name}")
    sheet.save(os.path.join(out, "poses.jpg"), quality=88)


def ladder(before, after, out, site):
    w, h, gap = 480, 270, 4
    names = [n for n in HEIGHTS if os.path.exists(os.path.join(after, f"{site}_{n}.png"))]
    sheet = Image.new("RGB", (4 * (w + gap) + gap, 2 * ((h + gap) * ((len(names) + 3) // 4)) + 3 * gap), (20, 20, 20))
    rows = (len(names) + 3) // 4
    for b, (d, name) in enumerate([(before, "before"), (after, "after")]):
        for k, n in enumerate(names):
            x = gap + (k % 4) * (w + gap)
            y = gap + (k // 4) * (h + gap) + b * (rows * (h + gap) + gap)
            sheet.paste(tile(os.path.join(d, f"{site}_{n}.png"), w, h), (x, y))
            label(sheet, x + 6, y + 6, f"{site} {n}: {name}", 16)
    sheet.save(os.path.join(out, f"ladder_{site}.jpg"), quality=86)


def main():
    before, after, out = sys.argv[1:4]
    os.makedirs(out, exist_ok=True)
    poses(before, after, out)
    for site in LADDERS:
        ladder(before, after, out, site)


if __name__ == "__main__":
    main()

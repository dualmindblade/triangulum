"""Tile captioned renders into one contact sheet.

    python game/scripts/contact_sheet.py OUT.png [--cols N] [--scale F] IMG [IMG ...]

Each input keeps its own caption band; a title strip names the sheet.
"""

import argparse
import os
import sys

from PIL import Image, ImageDraw, ImageFont


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("images", nargs="+")
    ap.add_argument("--cols", type=int, default=2)
    ap.add_argument("--scale", type=float, default=0.75)
    ap.add_argument("--title", default="")
    a = ap.parse_args()
    ims = [Image.open(p).convert("RGB") for p in a.images]
    w = int(ims[0].width * a.scale)
    h = int(ims[0].height * a.scale)
    ims = [im.resize((w, h), Image.LANCZOS) for im in ims]
    cols = min(a.cols, len(ims))
    rows = (len(ims) + cols - 1) // cols
    gap = 8
    title_h = 44 if a.title else 0
    sheet = Image.new("RGB", (cols * w + (cols + 1) * gap, title_h + rows * h + (rows + 1) * gap), (24, 24, 24))
    draw = ImageDraw.Draw(sheet)
    try:
        font = ImageFont.truetype("consola.ttf", 22)
    except OSError:
        font = ImageFont.load_default()
    if a.title:
        draw.text((gap, 10), a.title, fill=(235, 235, 235), font=font)
    for i, im in enumerate(ims):
        r, c = divmod(i, cols)
        sheet.paste(im, (gap + c * (w + gap), title_h + gap + r * (h + gap)))
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    sheet.save(a.out)
    print(a.out, sheet.size)
    return 0


if __name__ == "__main__":
    sys.exit(main())

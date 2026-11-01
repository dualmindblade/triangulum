"""Before/after contact sheets for mission T3 (boundaries).

    python scripts/t3_sheets.py BEFORE_DIR AFTER_DIR OUT_DIR

BEFORE_DIR and AFTER_DIR hold the frames of `t3-poses.play`, `t3-ladder.play`,
`t3-rock.play` and `t3-snow.play` from two builds. Writes poses.jpg (Andrew's four
photos, before beside after), ladder_<site>.jpg (one line from orbit to the ground,
before above after), rock.jpg (the rock line, before beside after) and snow.jpg (the
snow line and the rock line on the ranges, after only). A sheet whose frames are not
there is left out.

    python scripts/t3_sheets.py --grid NAME COLUMNS FRAME_DIR OUT_DIR frame frame ...

writes NAME.jpg, the named frames of one build in a grid.
"""

import os
import sys

from PIL import Image, ImageDraw, ImageFont

SITES = ["lake", "biome", "snow", "polar"]
LADDERS = SITES + ["coast"]
HEIGHTS = ["150km", "40km", "10km", "2500m", "600m", "150m", "40m", "foot"]
ROCK = ["rock_valley_peak", "rock_range_2km", "rock_down_1300m", "rock_down_6000m", "rock_belt_orbit"]
SNOW = [f"snow_{k}_{n}" for n in range(3) for k in ("valley", "2km")] + ["rock_foot_belt", "rock_foot_range_a"]


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


def rock(before, after, out):
    names = [n for n in ROCK if os.path.exists(os.path.join(before, f"{n}.png")) and os.path.exists(os.path.join(after, f"{n}.png"))]
    if not names:
        return
    w, h, gap = 960, 540, 6
    sheet = Image.new("RGB", (2 * w + 3 * gap, len(names) * (h + gap) + gap), (20, 20, 20))
    for r, n in enumerate(names):
        for c, (d, name) in enumerate([(before, "before"), (after, "after")]):
            x, y = gap + c * (w + gap), gap + r * (h + gap)
            sheet.paste(tile(os.path.join(d, f"{n}.png"), w, h), (x, y))
            label(sheet, x + 8, y + 8, f"{n}: {name}")
    sheet.save(os.path.join(out, "rock.jpg"), quality=86)


def grid(name, cols, frames, out, names):
    names = [n for n in names if os.path.exists(os.path.join(frames, f"{n}.png"))]
    if not names:
        return
    w, h, gap = 960, 540, 6
    rows = (len(names) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * (w + gap) + gap, rows * (h + gap) + gap), (20, 20, 20))
    for k, n in enumerate(names):
        x, y = gap + (k % cols) * (w + gap), gap + (k // cols) * (h + gap)
        sheet.paste(tile(os.path.join(frames, f"{n}.png"), w, h), (x, y))
        label(sheet, x + 8, y + 8, n)
    sheet.save(os.path.join(out, f"{name}.jpg"), quality=86)


def main():
    if sys.argv[1] == "--grid":
        name, cols, frames, out = sys.argv[2:6]
        grid(name, int(cols), frames, out, sys.argv[6:])
        return
    before, after, out = sys.argv[1:4]
    os.makedirs(out, exist_ok=True)
    poses(before, after, out)
    for site in LADDERS:
        ladder(before, after, out, site)
    rock(before, after, out)
    grid("snow", 2, after, out, SNOW)


if __name__ == "__main__":
    main()

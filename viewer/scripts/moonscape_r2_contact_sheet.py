#!/usr/bin/env python3
"""Assemble the Moonscape round-2 art-direction evidence sheet."""

import argparse
import math
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def fit(image: Image.Image, width: int, height: int) -> Image.Image:
    image = image.convert("RGB")
    scale = max(width / image.width, height / image.height)
    size = (round(image.width * scale), round(image.height * scale))
    image = image.resize(size, Image.Resampling.LANCZOS)
    left = (image.width - width) // 2
    top = (image.height - height) // 2
    return image.crop((left, top, left + width, top + height))


def histogram_panel(viewer: Path, width: int, height: int, output_dir: Path) -> Image.Image:
    executable = viewer / "target" / "release" / "examples" / "moonfeatures.exe"
    result = subprocess.run(
        [str(executable), "42"], cwd=viewer, capture_output=True, text=True, check=True
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "moonfeatures.txt").write_text(result.stdout, encoding="utf-8")
    rows = []
    for line in result.stdout.splitlines():
        if not line.startswith("radius_histogram_csv ") or "lower_deg" in line:
            continue
        lower, upper, count = line.removeprefix("radius_histogram_csv ").split(",")
        radius = math.sqrt(float(lower) * float(upper))
        if 0.02 <= radius <= 1.5 and int(count) > 0:
            rows.append((radius, int(count)))

    panel = Image.new("RGB", (width, height), (10, 12, 17))
    draw = ImageDraw.Draw(panel)
    font = ImageFont.load_default(size=16)
    small = ImageFont.load_default(size=13)
    left, top, right, bottom = 78, 34, width - 24, height - 54
    draw.rectangle((left, top, right, bottom), outline=(96, 103, 116), width=1)
    x0, x1 = math.log10(min(r for r, _ in rows)), math.log10(max(r for r, _ in rows))
    y0, y1 = 0.0, math.log10(max(c for _, c in rows)) + 0.12

    def xy(radius: float, count: float) -> tuple[int, int]:
        x = left + (math.log10(radius) - x0) / (x1 - x0) * (right - left)
        y = bottom - (math.log10(count) - y0) / (y1 - y0) * (bottom - top)
        return round(x), round(y)

    points = [xy(radius, count) for radius, count in rows]
    draw.line(points, fill=(105, 190, 242), width=3, joint="curve")
    for point in points:
        draw.ellipse((point[0] - 3, point[1] - 3, point[0] + 3, point[1] + 3), fill=(190, 230, 255))

    # D^-2 guide, anchored to the middle measured bin.
    anchor_r, anchor_c = rows[len(rows) // 2]
    guide = []
    for radius, _ in rows:
        count = anchor_c * (radius / anchor_r) ** -2
        if 1.0 <= count <= 10**y1:
            guide.append(xy(radius, count))
    if len(guide) > 1:
        draw.line(guide, fill=(242, 173, 88), width=2)
    draw.text((left, 8), "FACE 0 CRATER RADII - LOG/LOG", font=font, fill=(232, 235, 242))
    draw.text((right - 138, top + 12), "orange: D^-2", font=small, fill=(242, 173, 88))
    draw.text((left, bottom + 17), "radius (degrees), continuous overlapping bands", font=small, fill=(185, 191, 202))
    draw.text((8, top + 6), "count", font=small, fill=(185, 191, 202))
    panel.save(output_dir / "crater-radius-histogram.png")
    return panel


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--before", type=Path)
    parser.add_argument("--after", type=Path)
    parser.add_argument("--rim-before", type=Path)
    parser.add_argument("--rim-after", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    runs = viewer / "interchange" / "runs"
    before = args.before or runs / "r2-before"
    after = args.after or runs / "r2-after"
    rim_before = args.rim_before or runs / "r2-rim-before"
    rim_after = args.rim_after or runs / "r2-rim-after"
    output = args.output or viewer / "interchange" / "codex" / "moonscape-r2" / "contact-sheet.png"
    output.parent.mkdir(parents=True, exist_ok=True)

    tile_w, tile_h, label_h = 640, 360, 38
    histogram = histogram_panel(viewer, tile_w, tile_h, output.parent)
    panels = [
        (viewer / "interchange" / "tycho-rays-and-moon-surface.JPG", "REFERENCE - TYCHO + SATURATED SURFACE"),
        (after / "full_face_orbit.png", "R2 - FULL FACE"),
        (after / "saturated_highlands.png", "R2 - SATURATED HIGHLANDS"),
        (rim_before / "rim_straddling_floor.png", "SAME IMPACT - SUBTRACTIVE FLOOR"),
        (rim_after / "rim_straddling_floor.png", "SAME IMPACT - LOCALLY LEVELED FLOOR"),
        (before / "full_face_orbit.png", "MAIN - MARE SILHOUETTES"),
        (after / "full_face_orbit.png", "R2 - DARKER, BOLDER MARE SILHOUETTES"),
        (histogram, "CRATER SIZE HISTOGRAM - NO OCTAVE COMB"),
    ]
    sheet = Image.new("RGB", (tile_w * 2, (tile_h + label_h) * 4), (4, 5, 9))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=17)
    for index, (source, label) in enumerate(panels):
        image = source if isinstance(source, Image.Image) else Image.open(source)
        x = index % 2 * tile_w
        y = index // 2 * (tile_h + label_h)
        sheet.paste(fit(image, tile_w, tile_h), (x, y))
        draw.rectangle((x, y + tile_h, x + tile_w, y + tile_h + label_h), fill=(15, 17, 23))
        draw.text((x + 12, y + tile_h + 9), label, font=font, fill=(230, 232, 238))
    sheet.save(output)
    print(output)


if __name__ == "__main__":
    main()

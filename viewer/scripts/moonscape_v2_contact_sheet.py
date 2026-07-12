#!/usr/bin/env python3
"""Assemble Andrew's six Moonscape-v2 visual acceptance panels.

Evidence only: there is intentionally no accept/bless path.
"""

import argparse
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


PANELS = [
    ("reference", "REFERENCE - REAL MOON"),
    ("full_face_orbit", "V2 - FULL FACE COMPOSITION"),
    ("tycho_dark_halo", "TYCHO-SIZE LINES + DARK HALO"),
    ("saturated_highlands", "SATURATED HIGHLANDS"),
    ("mare_edge_inheritance", "MARE EDGE - ONE FLOOR COLOR"),
    ("landed_crater_floor", "LANDED - CRATER FLOOR"),
]


def fit(image: Image.Image, width: int, height: int) -> Image.Image:
    image = image.convert("RGB")
    scale = max(width / image.width, height / image.height)
    size = (round(image.width * scale), round(image.height * scale))
    image = image.resize(size, Image.Resampling.LANCZOS)
    left = (image.width - width) // 2
    top = (image.height - height) // 2
    return image.crop((left, top, left + width, top + height))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-render", action="store_true")
    parser.add_argument("--run-dir", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    run_dir = args.run_dir or viewer / "interchange" / "runs" / "moonscape-v2-evidence"
    output = args.output or viewer / "interchange" / "codex" / "moonscape-v2" / "contact-sheet.png"
    if not args.skip_render:
        result = subprocess.run(
            [
                str(viewer / "target" / "release" / "examples" / "play.exe"),
                str(viewer / "scripts" / "moonscape-v2-evidence.play"),
                "--out",
                str(run_dir),
            ],
            cwd=viewer,
            capture_output=True,
            text=True,
        )
        if result.returncode:
            raise SystemExit(result.stdout[-2000:] + result.stderr[-2000:])

    sources = {
        "reference": viewer / "interchange" / "tycho-rays-and-moon-surface.JPG",
        **{name: run_dir / f"{name}.png" for name, _ in PANELS[1:]},
    }
    tile_w, tile_h, label_h = 640, 360, 38
    sheet = Image.new("RGB", (tile_w * 2, (tile_h + label_h) * 3), (4, 5, 9))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=18)
    for index, (name, label) in enumerate(PANELS):
        x = index % 2 * tile_w
        y = index // 2 * (tile_h + label_h)
        sheet.paste(fit(Image.open(sources[name]), tile_w, tile_h), (x, y))
        draw.rectangle((x, y + tile_h, x + tile_w, y + tile_h + label_h), fill=(15, 17, 23))
        draw.text((x + 12, y + tile_h + 9), label, font=font, fill=(230, 232, 238))

    output.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output)
    print(output)


if __name__ == "__main__":
    main()

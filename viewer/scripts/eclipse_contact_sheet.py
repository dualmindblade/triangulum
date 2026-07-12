#!/usr/bin/env python3
"""Compose the reproducible SOLAR P1 eclipse evidence frames.

Run `eclipse-evidence.play` first. This script never synthesizes imagery; it
only fits the three renderer outputs into a labeled review sheet.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageOps


PANELS = [
    ("partial_solar.png", "PARTIAL SOLAR  t=7621.82  weather=off  overlap=0.5001"),
    ("near_total_ground.png", "TOTAL SOLAR  t=7621.82  weather=off  overlap=1.0000"),
    ("lunar_copper.png", "LUNAR COPPER  t=13785.65  weather=off  shadow=1.0000"),
]


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    source = viewer / "interchange" / "runs" / "eclipse-evidence"
    output = viewer / "interchange" / "solar-p1" / "eclipse-contact-sheet.png"
    font = ImageFont.load_default()
    panel_w, image_h, label_h = 640, 360, 34
    sheet = Image.new("RGB", (panel_w * len(PANELS), image_h + label_h), (8, 10, 14))
    draw = ImageDraw.Draw(sheet)
    for index, (filename, label) in enumerate(PANELS):
        frame = Image.open(source / filename).convert("RGB")
        frame = ImageOps.fit(frame, (panel_w, image_h), method=Image.Resampling.LANCZOS)
        x = index * panel_w
        sheet.paste(frame, (x, 0))
        draw.text((x + 10, image_h + 10), label, fill=(235, 238, 244), font=font)
    output.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output)
    print(output)


if __name__ == "__main__":
    main()

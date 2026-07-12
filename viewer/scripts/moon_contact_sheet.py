#!/usr/bin/env python3
"""Render and assemble the four required P2 moon evidence poses.

This is evidence only: it has no accept/bless mode and never touches reel
baselines. Run from anywhere after building the release play example.
"""

import argparse
import subprocess
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


SHOTS = [
    ("from_neisor_full_face", "FULL FACE - FROM NEISOR", True),
    ("quarter_phase", "QUARTER PHASE - ORBIT", False),
    ("ray_crater_flyby", "LARGE RAY CRATER - FLYBY", False),
    ("maria_horizon", "MARIA - LOW HORIZON", False),
]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-render", action="store_true")
    ap.add_argument(
        "--run-dir",
        type=Path,
        help="read already-rendered source frames from this directory",
    )
    ap.add_argument(
        "--out-dir",
        type=Path,
        help="write the contact sheet here (defaults to viewer/interchange)",
    )
    args = ap.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    run_dir = args.run_dir or viewer / "interchange" / "runs" / "moon-contact"
    out_dir = args.out_dir or viewer / "interchange" / "moon-contact"
    if not args.skip_render:
        exe = viewer / "target" / "release" / "examples" / "play.exe"
        play = viewer / "scripts" / "moon-contact.play"
        result = subprocess.run(
            [str(exe), str(play)], cwd=viewer, capture_output=True, text=True
        )
        if result.returncode:
            raise SystemExit(result.stdout[-2000:] + result.stderr[-2000:])

    tile_w, tile_h, label_h = 640, 360, 34
    sheet = Image.new("RGB", (tile_w * 2, (tile_h + label_h) * 2), (4, 5, 10))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=18)
    out_dir.mkdir(parents=True, exist_ok=True)
    for index, (name, label, crop_face) in enumerate(SHOTS):
        src = run_dir / f"{name}.png"
        image = Image.open(src).convert("RGB")
        if crop_face:
            # The moon is genuinely viewed from Neisor and therefore small.
            # A centered square crop makes the generated face judgeable while
            # the untouched source frame remains beside the sheet in run_dir.
            cx, cy = image.width // 2, image.height // 2
            side = min(image.width, image.height) // 9
            image = image.crop((cx - side, cy - side, cx + side, cy + side))
        image.thumbnail((tile_w, tile_h), Image.Resampling.LANCZOS)
        tile = Image.new("RGB", (tile_w, tile_h), (2, 3, 8))
        tile.paste(image, ((tile_w - image.width) // 2, (tile_h - image.height) // 2))
        x = (index % 2) * tile_w
        y = (index // 2) * (tile_h + label_h)
        sheet.paste(tile, (x, y))
        draw.rectangle((x, y + tile_h, x + tile_w, y + tile_h + label_h), fill=(15, 17, 24))
        draw.text((x + 12, y + tile_h + 7), label, font=font, fill=(225, 228, 235))

    out = out_dir / "contact_sheet.png"
    sheet.save(out)
    print(out)


if __name__ == "__main__":
    main()

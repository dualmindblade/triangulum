#!/usr/bin/env python3
"""Write before/after crops for the sky reel's fixed moon_zenith pose."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    root = viewer / "interchange" / "skyreel"
    accepted = Image.open(root / "accepted" / "moon_zenith.png").convert("RGB")
    current = Image.open(root / "current" / "moon_zenith.png").convert("RGB")
    cx, cy, radius = accepted.width // 2, int(accepted.height * 0.286), 48
    box = (cx - radius, cy - radius, cx + radius, cy + radius)
    before = accepted.crop(box).resize((384, 384), Image.Resampling.NEAREST)
    after = current.crop(box).resize((384, 384), Image.Resampling.NEAREST)
    before.save(root / "moon_zenith_before_crop.png")
    after.save(root / "moon_zenith_after_crop.png")

    label_h = 30
    sheet = Image.new("RGB", (768, 384 + label_h), (8, 9, 14))
    sheet.paste(before, (0, 0))
    sheet.paste(after, (384, 0))
    draw = ImageDraw.Draw(sheet)
    draw.rectangle((0, 384, 768, 414), fill=(16, 18, 25))
    font = ImageFont.load_default(size=16)
    draw.text((12, 391), "BEFORE - P1 PLACEHOLDER", fill=(225, 228, 235), font=font)
    draw.text((396, 391), "AFTER - P2 GENERATED FACE", fill=(225, 228, 235), font=font)
    out = root / "moon_zenith_before_after.png"
    sheet.save(out)
    print(out)


if __name__ == "__main__":
    main()

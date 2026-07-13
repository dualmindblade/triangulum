#!/usr/bin/env python3
"""Build accepted/current/heat crops for W2's expected reel findings."""

import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageOps


POSES = ["river_wide", "river_mouth", "coast_beach", "desert"]


def best_ground_crop(delta: np.ndarray, width: int = 640, height: int = 360) -> tuple[int, int, int, int]:
    """Pick the ground window with the most W2 change, never a sky-only crop."""
    image_h, image_w = delta.shape
    best = (-1.0, (0, image_h - height, width, image_h))
    for y in range(max(180, image_h - height * 2), image_h - height + 1, 30):
        for x in range(0, image_w - width + 1, 80):
            score = float(delta[y : y + height, x : x + width].mean())
            if score > best[0]:
                best = (score, (x, y, x + width, y + height))
    return best[1]


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    accepted = viewer / "interchange" / "reel" / "accepted"
    current = viewer / "interchange" / "reel" / "current"
    output = viewer / "interchange" / "w2-evidence" / "reel-crops"
    output.mkdir(parents=True, exist_ok=True)
    font = ImageFont.load_default()
    panel_size = (420, 236)
    label_h = 28
    aggregate = Image.new(
        "RGB",
        (panel_size[0] * 3, (panel_size[1] + label_h) * len(POSES)),
        (8, 10, 14),
    )
    rows = {}
    for row, name in enumerate(POSES):
        before_image = Image.open(accepted / f"{name}.png").convert("RGB")
        after_image = Image.open(current / f"{name}.png").convert("RGB")
        before = np.asarray(before_image, dtype=np.int16)
        after = np.asarray(after_image, dtype=np.int16)
        delta = np.abs(after - before).max(axis=2)
        box = best_ground_crop(delta)
        before_crop = before_image.crop(box)
        after_crop = after_image.crop(box)
        crop_delta = delta[box[1] : box[3], box[0] : box[2]]
        crop_base = np.asarray(before_crop, dtype=np.float64) * 0.28
        heat = crop_base
        heat[..., 0] = np.maximum(heat[..., 0], np.minimum(crop_delta * 28.0, 255.0))
        heat_crop = Image.fromarray(heat.astype(np.uint8), "RGB")
        panels = [before_crop, after_crop, heat_crop]
        labels = [f"{name}: accepted", f"{name}: W2", f"{name}: |delta| x28"]
        strip = Image.new("RGB", (panel_size[0] * 3, panel_size[1] + label_h), (8, 10, 14))
        draw = ImageDraw.Draw(strip)
        for column, (panel, label) in enumerate(zip(panels, labels)):
            panel = ImageOps.fit(panel, panel_size, method=Image.Resampling.LANCZOS)
            x = column * panel_size[0]
            strip.paste(panel, (x, 0))
            draw.text((x + 9, panel_size[1] + 8), label, fill=(235, 240, 246), font=font)
        strip.save(output / f"{name}.png")
        aggregate.paste(strip, (0, row * (panel_size[1] + label_h)))
        signed = after.astype(np.float64) - before.astype(np.float64)
        rows[name] = {
            "crop_box": box,
            "frame_mean_abs_delta": round(float(delta.mean()), 3),
            "frame_mean_signed_rgb": round(float(signed.mean()), 3),
            "pixels_over_6": round(float((delta > 6).mean()), 6),
        }
    aggregate.save(output / "all-flagged.png")
    (output / "report.json").write_text(json.dumps(rows, indent=2), encoding="utf-8")
    print(json.dumps(rows, indent=2))
    print(output / "all-flagged.png")


if __name__ == "__main__":
    main()

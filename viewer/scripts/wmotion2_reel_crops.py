#!/usr/bin/env python3
"""Build accepted/current/heat crops for every W-MOTION reel finding.

This is diagnostic only: it never writes interchange/reel/accepted.
"""

import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageOps


def best_crop(delta: np.ndarray, _orbital: bool) -> tuple[int, int, int, int]:
    height, width = delta.shape
    crop_w, crop_h = min(640, width), min(360, height)
    y_start = 0
    best = (-1.0, (0, y_start, crop_w, y_start + crop_h))
    for y in range(y_start, height - crop_h + 1, 30):
        for x in range(0, width - crop_w + 1, 80):
            score = float(delta[y : y + crop_h, x : x + crop_w].mean())
            if score > best[0]:
                best = (score, (x, y, x + crop_w, y + crop_h))
    return best[1]


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    reel = viewer / "interchange" / "reel"
    accepted = reel / "accepted"
    current = reel / "current"
    report = json.loads((reel / "report.json").read_text(encoding="utf-8"))
    names = [
        name
        for name, row in report.items()
        if isinstance(row.get("mean_delta"), (int, float))
        and (row["mean_delta"] > 1.0 or row.get("changed_frac", 0.0) > 0.01)
    ]
    output = viewer / "interchange" / "wmotion2-evidence" / "reel-crops"
    output.mkdir(parents=True, exist_ok=True)
    panel_size, label_h = (360, 203), 28
    font = ImageFont.load_default()
    aggregate = Image.new(
        "RGB",
        (panel_size[0] * 3, (panel_size[1] + label_h) * len(names)),
        (7, 9, 13),
    )
    crop_report = {}
    for row_index, name in enumerate(names):
        before_image = Image.open(accepted / f"{name}.png").convert("RGB")
        after_image = Image.open(current / f"{name}.png").convert("RGB")
        before = np.asarray(before_image, dtype=np.int16)
        after = np.asarray(after_image, dtype=np.int16)
        delta = np.abs(after - before).max(axis=2)
        box = best_crop(delta, name.startswith("orbital_"))
        before_crop = before_image.crop(box)
        after_crop = after_image.crop(box)
        crop_delta = delta[box[1] : box[3], box[0] : box[2]]
        heat = np.asarray(before_crop, dtype=np.float64).copy() * 0.25
        heat[..., 0] = np.maximum(heat[..., 0], np.minimum(crop_delta * 24.0, 255.0))
        heat_crop = Image.fromarray(heat.astype(np.uint8), "RGB")
        strip = Image.new(
            "RGB", (panel_size[0] * 3, panel_size[1] + label_h), (7, 9, 13)
        )
        draw = ImageDraw.Draw(strip)
        for column, (panel, label) in enumerate(
            zip(
                (before_crop, after_crop, heat_crop),
                (f"{name}: accepted", f"{name}: W-MOTION 2", f"{name}: |delta| x24"),
            )
        ):
            panel = ImageOps.fit(panel, panel_size, method=Image.Resampling.LANCZOS)
            x = column * panel_size[0]
            strip.paste(panel, (x, 0))
            draw.text((x + 8, panel_size[1] + 8), label, fill=(230, 236, 244), font=font)
        strip.save(output / f"{name}.png")
        aggregate.paste(strip, (0, row_index * (panel_size[1] + label_h)))
        crop_report[name] = {
            "crop_box": box,
            "frame_mean_delta": report[name]["mean_delta"],
            "frame_changed_frac": report[name].get("changed_frac"),
            "crop_mean_delta": round(float(crop_delta.mean()), 3),
            "cause": (
                "direct orbital cloud fabric: two-tap evolving-warp budget plus local structured terms"
                if name.startswith("orbital_")
                else "visible deck and W2 projection both read the same changed planet-anchored fabric"
            ),
        }
    aggregate.save(output / "all-flagged.png")
    (output / "report.json").write_text(json.dumps(crop_report, indent=2), encoding="utf-8")
    print(json.dumps(crop_report, indent=2))
    print(output / "all-flagged.png")


if __name__ == "__main__":
    main()

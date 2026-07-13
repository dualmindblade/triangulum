#!/usr/bin/env python3
"""Assemble W-MOTION pass 2's deterministic time strips."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageOps


ROWS = [
    (
        "cyclone-orbit",
        "ANALYTIC CYCLONE / ORBIT",
        ["cyclone_orbit_t000", "cyclone_orbit_t060", "cyclone_orbit_t120", "cyclone_orbit_t180"],
        ["t + 0 s", "t + 60 s", "t + 120 s", "t + 180 s"],
    ),
    (
        "cyclone-shadow",
        "ROTATING W2 SHADOW FIELD",
        ["cyclone_shadow_t000", "cyclone_shadow_t060", "cyclone_shadow_t120", "cyclone_shadow_t180"],
        ["t + 0 s", "t + 60 s", "t + 120 s", "t + 180 s"],
    ),
    (
        "front-passage",
        "ASYMMETRIC FRONT PASSAGE / GROUND",
        ["front_ground_t0600", "front_ground_t1200", "front_ground_t1800", "front_ground_t2400"],
        ["t = 600 s", "t = 1200 s", "ridge: 1800 s", "t = 2400 s"],
    ),
    (
        "cyclone-gloom",
        "W2 GLOOM TRACKS CYCLONE APPROACH",
        ["cyclone_gloom_t0000", "cyclone_gloom_t1200", "cyclone_gloom_t2400", "cyclone_gloom_t3600"],
        ["1040 km west", "690 km west", "350 km west", "overhead"],
    ),
]

# One fixed crop for the subtle shadow series. It was selected from the
# highest four-frame temporal range over ground only, then frozen here so all
# times show the identical place (no per-frame crop chasing).
CROP_BOXES = {"cyclone-shadow": (640, 0, 1200, 315)}


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    run = viewer / "interchange" / "runs" / "weather-visual"
    output = viewer / "interchange" / "wmotion2-evidence" / "time-strips.png"
    panel_w, panel_h, label_h, row_title_h = 420, 236, 25, 25
    row_h = row_title_h + panel_h + label_h
    canvas = Image.new("RGB", (panel_w * 4, row_h * len(ROWS)), (7, 9, 13))
    draw = ImageDraw.Draw(canvas)
    font = ImageFont.load_default()
    for row_index, (_slug, title, names, labels) in enumerate(ROWS):
        row_y = row_index * row_h
        draw.text((9, row_y + 8), title, fill=(238, 242, 248), font=font)
        for column, (name, label) in enumerate(zip(names, labels)):
            frame = Image.open(run / f"{name}.png").convert("RGB")
            if _slug in CROP_BOXES:
                frame = frame.crop(CROP_BOXES[_slug])
            frame = ImageOps.fit(frame, (panel_w, panel_h), method=Image.Resampling.LANCZOS)
            x = column * panel_w
            y = row_y + row_title_h
            canvas.paste(frame, (x, y))
            draw.text((x + 9, y + panel_h + 7), label, fill=(225, 232, 240), font=font)
    output.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(output)
    for row_index, (slug, _title, _names, _labels) in enumerate(ROWS):
        row = canvas.crop((0, row_index * row_h, panel_w * 4, (row_index + 1) * row_h))
        row.save(output.with_name(f"{slug}.png"))
    print(output)


if __name__ == "__main__":
    main()

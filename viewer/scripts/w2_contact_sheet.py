#!/usr/bin/env python3
"""Assemble the deterministic W2 presentation evidence into one sheet."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageOps


ROWS = [
    [
        ("shadow_clear", "SHADOWS: clear 0.05"),
        ("shadow_broken", "SHADOWS: broken 0.65"),
        ("shadow_overcast", "SHADOWS: overcast 0.98"),
        None,
    ],
    [
        ("mist_before", "MIST: -2 h edge"),
        ("mist_sunrise", "MIST: sunrise"),
        ("mist_after", "MIST: +2 h edge"),
        None,
    ],
    [
        ("storm_north", "STORM PAN: north"),
        ("storm_east", "STORM PAN: east"),
        ("storm_south", "STORM PAN: south / front"),
        ("storm_west", "STORM PAN: west"),
    ],
]


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    run = viewer / "interchange" / "runs" / "w2-contact"
    output = viewer / "interchange" / "w2-evidence" / "contact-sheet.png"
    panel_w, panel_h, label_h = 480, 270, 30
    canvas = Image.new(
        "RGB",
        (panel_w * 4, (panel_h + label_h) * len(ROWS)),
        (8, 10, 14),
    )
    draw = ImageDraw.Draw(canvas)
    font = ImageFont.load_default()
    for row_index, row in enumerate(ROWS):
        for column, panel in enumerate(row):
            if panel is None:
                continue
            name, label = panel
            frame = Image.open(run / f"{name}.png").convert("RGB")
            frame = ImageOps.fit(
                frame,
                (panel_w, panel_h),
                method=Image.Resampling.LANCZOS,
            )
            x = column * panel_w
            y = row_index * (panel_h + label_h)
            canvas.paste(frame, (x, y))
            draw.text((x + 10, y + panel_h + 9), label, fill=(235, 240, 246), font=font)
    output.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(output)
    print(output)


if __name__ == "__main__":
    main()

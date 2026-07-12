#!/usr/bin/env python3
"""Assemble the W4 fixed-pose render evidence into labeled contact sheets."""

from pathlib import Path

from PIL import Image, ImageDraw


def sheet(run: Path, names: list[str], labels: list[str], columns: int, out: Path) -> None:
    images = [Image.open(run / f"{name}.png").convert("RGB") for name in names]
    tile_w, tile_h = images[0].size
    label_h = 34
    rows = (len(images) + columns - 1) // columns
    canvas = Image.new("RGB", (tile_w * columns, (tile_h + label_h) * rows), (8, 10, 14))
    draw = ImageDraw.Draw(canvas)
    for index, (image, label) in enumerate(zip(images, labels)):
        x = index % columns * tile_w
        y = index // columns * (tile_h + label_h)
        canvas.paste(image, (x, y + label_h))
        draw.text((x + 12, y + 10), label, fill=(235, 240, 246))
    out.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(out)


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    run = viewer / "interchange" / "runs" / "seasonal-evidence"
    evidence = viewer / "interchange" / "w4-evidence"
    sheet(
        run,
        ["frozen_lake_s00", "frozen_lake_s25", "frozen_lake_s50", "frozen_lake_s75"],
        ["season 0.00", "season 0.25", "season 0.50", "season 0.75"],
        2,
        evidence / "frozen_lake_four_seasons.png",
    )
    sheet(
        run,
        ["sea_edge_s00", "sea_edge_s25", "sea_edge_s50", "sea_edge_s54", "sea_edge_s75"],
        ["0.00", "0.25", "0.50", "0.5417 monthly minimum", "0.75"],
        5,
        evidence / "sea_ice_edge_sweep.png",
    )
    print(evidence / "frozen_lake_four_seasons.png")
    print(evidence / "sea_ice_edge_sweep.png")


if __name__ == "__main__":
    main()

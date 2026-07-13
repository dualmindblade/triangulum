#!/usr/bin/env python3
"""Assemble the W2.5 heterogeneous-deck acceptance contact sheets."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont, ImageOps


VIEWER = Path(__file__).resolve().parent.parent
RUN = VIEWER / "interchange" / "runs" / "w25-heterogeneous"
OUT = VIEWER / "interchange" / "w25"


def font(size: int):
    for name in ("arial.ttf", "DejaVuSans.ttf"):
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            pass
    return ImageFont.load_default()


LABEL_FONT = font(22)
NOTE_FONT = font(17)
BG = (13, 18, 27)
FG = (238, 243, 250)
NOTE = (167, 185, 207)


def tile(path: Path, title: str, note: str, width: int = 640, height: int = 400):
    if not path.is_file():
        raise SystemExit(f"missing W2.5 capture: {path}")
    canvas = Image.new("RGB", (width, height), BG)
    image = Image.open(path).convert("RGB")
    fitted = ImageOps.contain(image, (width, height - 54), Image.Resampling.LANCZOS)
    x = (width - fitted.width) // 2
    y = 54 + (height - 54 - fitted.height) // 2
    canvas.paste(fitted, (x, y))
    draw = ImageDraw.Draw(canvas)
    draw.text((12, 6), title, font=LABEL_FONT, fill=FG)
    draw.text((12, 31), note, font=NOTE_FONT, fill=NOTE)
    return canvas


def save_grid(items, path: Path, columns: int):
    tiles = [tile(*item) for item in items]
    rows = (len(tiles) + columns - 1) // columns
    sheet = Image.new("RGB", (640 * columns, 400 * rows), BG)
    for index, image in enumerate(tiles):
        sheet.paste(image, ((index % columns) * 640, (index // columns) * 400))
    path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(path, optimize=True)
    print(path.relative_to(VIEWER))


def main():
    map_match = OUT / "cover_match_t3500.png"
    sky_match = RUN / "match_orbit.png"
    items = [
        (RUN / "global_wide.png", "Global live deck", "t=3500 s · several synoptic scales"),
        (RUN / "mid_altitude.png", "Mid-altitude", "500 km · regional carrier + local fabric"),
        (map_match, "Map cover overlay", "20°N, 0°E · 3× · gray is thicker"),
        (sky_match, "Same geography from sky", "20°N, 0°E · 4000 km · north-up"),
        (RUN / "ground_thick.png", "Ground: thick region", "raster cover 1.000 at capture node"),
        (RUN / "ground_clear.png", "Ground: clear lane", "raster cover 0.000 at capture node"),
        (OUT / "wind_global_t3500.png", "Wind map: global", "1× · deterministic comet streamlines"),
        (OUT / "wind_zoom4_t3500.png", "Wind map: regional", "4× · path length scales with zoom"),
    ]
    save_grid(items, OUT / "contact_sheet.png", 2)
    save_grid(items[2:4], OUT / "map_sky_match_pair.png", 2)


if __name__ == "__main__":
    main()

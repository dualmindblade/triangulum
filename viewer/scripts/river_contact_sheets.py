#!/usr/bin/env python3
"""Build the labeled Track-B river review sheets from settled play captures."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


LABEL_HEIGHT = 42
FULL_TILE = (1280, 720)
SUMMARY_TILE = (640, 360)


def font(size: int = 24) -> ImageFont.ImageFont:
    try:
        return ImageFont.truetype("DejaVuSans.ttf", size)
    except OSError:
        return ImageFont.load_default()


def load_labeled(path: Path, label: str, size: tuple[int, int]) -> Image.Image:
    image = Image.open(path).convert("RGB")
    if image.size != size:
        image = image.resize(size, Image.Resampling.LANCZOS)
    draw = ImageDraw.Draw(image)
    draw.rectangle((0, 0, size[0], LABEL_HEIGHT), fill=(7, 10, 14))
    draw.text((14, 8), label, fill=(240, 244, 248), font=font())
    return image


def grid(
    tiles: list[tuple[Path, str]],
    columns: int,
    tile_size: tuple[int, int],
    output: Path,
) -> None:
    rows = (len(tiles) + columns - 1) // columns
    canvas = Image.new(
        "RGB", (tile_size[0] * columns, tile_size[1] * rows), (7, 10, 14)
    )
    for index, (path, label) in enumerate(tiles):
        image = load_labeled(path, label, tile_size)
        x = index % columns * tile_size[0]
        y = index // columns * tile_size[1]
        canvas.paste(image, (x, y))
    output.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(output, optimize=True)
    print(output)


def main() -> None:
    viewer = Path(__file__).resolve().parent.parent
    rivers = viewer / "interchange" / "rivers"
    before = rivers / "before"
    after = rivers / "after"
    sheets = rivers / "contact-sheets"

    poses = [
        ("river_wide", "wide reach"),
        ("river_valley", "valley reach"),
        ("river_mouth", "river mouth"),
        ("waterfall_site", "waterfall site"),
        ("b9_lod_border", "B-9 mesh-LOD border"),
    ]
    summary: list[tuple[Path, str]] = []
    for index, (name, description) in enumerate(poses, start=1):
        pair = [
            (before / f"{name}.png", f"BEFORE (pre-Track-B) - {description}"),
            (after / f"{name}.png", f"AFTER (Iteration 2) - {description}"),
        ]
        summary.extend(pair)
        grid(
            pair,
            2,
            FULL_TILE,
            sheets / f"{index:02d}_{name}_before-left_after-right.png",
        )
    grid(
        summary,
        2,
        SUMMARY_TILE,
        sheets / "00_rivers_before-left_after-right.png",
    )

    comparison = rivers / "meander-comparison"
    grid(
        [
            (comparison / "amp_018" / "meander_comparison.png", "0.18 km amplitude"),
            (comparison / "amp_028" / "meander_comparison.png", "0.28 km amplitude (default)"),
            (comparison / "amp_038" / "meander_comparison.png", "0.38 km amplitude"),
        ],
        3,
        SUMMARY_TILE,
        sheets / "06_meander_amplitude_018_028_038.png",
    )

    island = rivers / "probes" / "island"
    grid(
        [
            (island / "island_overhead.png", "island - overhead mechanics"),
            (island / "island_bar_walkheight.png", "island - bank-height scenery"),
        ],
        2,
        FULL_TILE,
        sheets / "07_island_overhead_and_walkheight.png",
    )

    grid(
        [
            (
                rivers
                / "iteration1-review-baseline"
                / "waterfall"
                / "fall_approach.png",
                "Iteration 1 reviewed - aqueduct / pale ramp",
            ),
            (
                rivers / "probes" / "waterfall" / "fall_approach.png",
                "Iteration 2 - gorge banks / aerated sheet",
            ),
        ],
        2,
        FULL_TILE,
        sheets / "08_waterfall_iteration1_to_iteration2.png",
    )

    grid(
        [
            (rivers / "probes" / "bank" / "bank_start.png", "coherent bank band"),
            (
                rivers / "probes" / "overhead" / "reach_overhead_start.png",
                "reach-scale meander",
            ),
            (
                rivers / "probes" / "island" / "island_bar_walkheight.png",
                "walkable vegetated bar",
            ),
            (
                rivers / "probes" / "waterfall" / "fall_approach.png",
                "waterfall approach",
            ),
        ],
        2,
        SUMMARY_TILE,
        sheets / "09_iteration2_probe_gallery.png",
    )


if __name__ == "__main__":
    main()

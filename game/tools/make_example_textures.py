"""Write the example block textures in game/assets/textures/examples/.

They are placeholders made by Claude for M3b, so Andrew can see what a
texture looks like on a block with one click in the look panel (F1,
Blocks). They are not enabled by default and they are not art direction:
Andrew paints the real ones.

Each picture is 16 x 16, tiles seamlessly (the noise wraps around), and is
scaled in linear light so its average colour is exactly its block's colour
in blocks.json: far away a textured block looks the same as the flat one.

    python game/tools/make_example_textures.py

Writes stone.png, dirt.png and grass_top.png (deterministic: the same files
every run). Needs numpy and Pillow.

    python game/tools/make_example_textures.py --every-block DIR

writes a test world's look instead: DIR/blocks.json (a copy of the shipped
file) with a texture on every face of every solid block, in DIR/textures/,
each a speckled picture whose average is that face's colour. The bench
uses it to measure what textures cost, and frames drawn with it should
match the flat look from far away (the average colour) while showing the
pictures up close.
"""

import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image

N = 16
OUT = Path(__file__).resolve().parent.parent / "assets" / "textures" / "examples"

# Linear RGB of the blocks in game/assets/blocks.json.
STONE = (0.300, 0.290, 0.280)
DIRT = (0.180, 0.110, 0.060)
GRASS = (0.090, 0.260, 0.050)


def to_srgb(x):
    x = np.clip(x, 0.0, 1.0)
    return np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - 0.055)


def to_linear(s):
    return np.where(s <= 0.04045, s / 12.92, np.power((s + 0.055) / 1.055, 2.4))


def wrap_noise(rng, cells):
    """Smooth value noise on an N x N torus with `cells` lattice cells."""
    g = rng.random((cells, cells))
    out = np.zeros((N, N))
    for y in range(N):
        for x in range(N):
            fx, fy = x * cells / N, y * cells / N
            x0, y0 = int(fx), int(fy)
            tx, ty = fx - x0, fy - y0
            tx, ty = tx * tx * (3 - 2 * tx), ty * ty * (3 - 2 * ty)
            a = g[y0 % cells, x0 % cells]
            b = g[y0 % cells, (x0 + 1) % cells]
            c = g[(y0 + 1) % cells, x0 % cells]
            d = g[(y0 + 1) % cells, (x0 + 1) % cells]
            out[y, x] = (a * (1 - tx) + b * tx) * (1 - ty) + (c * (1 - tx) + d * tx) * ty
    return out


def finish(shade, tint, mean):
    """Per-pixel brightness `shade` (around 1) times a tint, scaled so the
    linear mean is exactly `mean`; returns 8-bit sRGB."""
    lin = shade[..., None] * np.array(tint)[None, None, :]
    for _ in range(6):  # clipping moves the mean a little; settle it
        lin = np.clip(lin * (np.array(mean) / lin.reshape(-1, 3).mean(axis=0)), 0, 1)
        q = np.round(to_srgb(lin) * 255) / 255
        lin = lin * (np.array(mean) / to_linear(q).reshape(-1, 3).mean(axis=0))
    return np.round(to_srgb(lin) * 255).astype(np.uint8)


def stone():
    rng = np.random.default_rng(7)
    s = 0.75 + 0.35 * wrap_noise(rng, 4) + 0.25 * wrap_noise(rng, 8) + 0.12 * rng.random((N, N))
    # A few dark cracks that wander across the block and wrap at its edges.
    for x0, y0, steps in [(2, 3, 9), (9, 10, 8), (12, 1, 5)]:
        x, y = x0, y0
        for _ in range(steps):
            s[y % N, x % N] *= 0.55
            x += 1
            y += int(rng.integers(-1, 2))
    # Light flecks.
    for x, y in [(5, 12), (13, 6), (7, 1), (1, 9)]:
        s[y, x] *= 1.45
    return finish(s, (1.0, 0.97, 0.93), STONE)


def dirt():
    rng = np.random.default_rng(11)
    s = 0.8 + 0.3 * wrap_noise(rng, 4) + 0.2 * rng.random((N, N))
    # Dark clumps and a few pale pebbles.
    for x, y in [(3, 4), (4, 4), (3, 5), (11, 9), (12, 9), (8, 14), (14, 2)]:
        s[y, x] *= 0.6
    for x, y in [(6, 8), (13, 13), (1, 13), (9, 2)]:
        s[y, x] *= 1.6
    return finish(s, (1.0, 0.62, 0.34), DIRT)


def grass_top():
    rng = np.random.default_rng(23)
    s = 0.85 + 0.25 * wrap_noise(rng, 4) + 0.25 * rng.random((N, N))
    # Short bright blades and darker gaps between them.
    for x, y in [(2, 2), (2, 3), (7, 5), (7, 6), (12, 11), (12, 12), (4, 13), (10, 1), (14, 7), (0, 9)]:
        s[y, x] *= 1.4
    for x, y in [(5, 9), (9, 13), (13, 3), (1, 6)]:
        s[y, x] *= 0.65
    return finish(s, (0.35, 1.0, 0.2), GRASS)


def every_block(out):
    """A copy of blocks.json with every solid block textured (see the top)."""
    shipped = Path(__file__).resolve().parent.parent / "assets" / "blocks.json"
    doc = json.loads(shipped.read_text())
    tex = out / "textures"
    tex.mkdir(parents=True, exist_ok=True)
    n = 0
    for b in doc["blocks"]:
        if not b.get("solid"):
            continue
        faces = {f: tuple(b.get(f) or b["color"]) for f in ("top", "side", "bottom")}
        made = {}  # colour -> file
        names = {}
        for f, colour in faces.items():
            if colour not in made:
                rng = np.random.default_rng(100 + 3 * b["id"] + len(made))
                s = 0.75 + 0.35 * wrap_noise(rng, 4) + 0.3 * rng.random((N, N))
                rgb = finish(s, (1.0, 1.0, 1.0), [max(c, 1e-4) for c in colour])
                made[colour] = f"{b['name']}_{f}.png"
                Image.fromarray(rgb, "RGB").convert("RGBA").save(tex / made[colour])
                n += 1
            names[f] = made[colour]
        if len(set(names.values())) == 1:
            b["texture"] = names["top"]
        else:
            for f in faces:
                b[f"{f}_texture"] = names[f]
    (out / "blocks.json").write_text(json.dumps(doc, indent=1))
    print(f"{out / 'blocks.json'}: every solid block textured ({n} pictures in {tex})")


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--every-block":
        every_block(Path(sys.argv[2]))
        return
    OUT.mkdir(parents=True, exist_ok=True)
    for name, make, colour in [("stone", stone, STONE), ("dirt", dirt, DIRT), ("grass_top", grass_top, GRASS)]:
        rgb = make()
        Image.fromarray(rgb, "RGB").convert("RGBA").save(OUT / f"{name}.png")
        mean = to_linear(rgb / 255.0).reshape(-1, 3).mean(axis=0)
        print(f"{name}.png: linear mean {mean.round(4)} (block {colour})")


if __name__ == "__main__":
    main()

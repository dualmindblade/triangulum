"""Review 2 #12 evidence: does anything show through the coarsest ocean?

Renders come from game/scripts/ocean-orbit.play, run before and after the
fix at local midnight and noon:

    triangulum --bake ... --at -11 -170 150000 --time 00:00 \
        --play scripts/ocean-orbit.play --out ROOT/after-0000
    (and --time 12:00 -> after-1200; the same from the old build -> before-*)

    python review2_12_ocean.py ROOT    # prints the numbers, writes the sheet

Night, straight down over the open ocean at (-11, -170) with the sun near
the antipode: ocean pixels brighter than the ocean's median by more than 12
levels are background (stars, the sun's disc) showing through the sea.
75 km: mean colour in strips either side of an L8/L9 boundary.
"""
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageFont

root = sys.argv[1]
out = sys.argv[2] if len(sys.argv) > 2 else "review2_12_ocean_before_after.jpg"


def load(path):
    return np.asarray(Image.open(path).convert("RGB")).astype(np.float64)


def lum(a):
    return 0.2126 * a[..., 0] + 0.7152 * a[..., 1] + 0.0722 * a[..., 2]


h, w = 1080, 1920
yy, xx = np.mgrid[0:h, 0:w]
cross = (abs(xx - 960) < 20) & (abs(yy - 540) < 20)
hud = ((xx < 790) & (yy < 235)) | (yy > 990)


def leaks(noon, night, radius):
    disc = (xx - 960) ** 2 + (yy - 540) ** 2 < radius ** 2
    ocean = disc & ~cross & ~hud & (noon[..., 2] - noon[..., 0] > 25)
    ln = lum(night)
    med = np.median(ln[ocean])
    return ocean.sum(), med, (ocean & (ln > med + 12)).sum(), ln[ocean].max()


for tag in ("before", "after"):
    for alt, radius in (("orbit150", 360), ("ocean75", 590)):
        n, med, bright, mx = leaks(load(f"{root}/{tag}-1200/{alt}.png"), load(f"{root}/{tag}-0000/{alt}.png"), radius)
        print(f"{tag} night {alt}: {n} ocean px, median luminance {med:.1f}, {bright} px brighter by >12 (max {mx:.0f})")
    for time in ("1200", "0000"):
        im = load(f"{root}/{tag}-{time}/ocean75.png")
        m9 = im[420:700, 430:456].reshape(-1, 3).mean(0)
        m8 = im[420:700, 476:502].reshape(-1, 3).mean(0)
        print(f"{tag} {time} ocean75: L9 {np.round(m9, 1)}, L8 {np.round(m8, 1)}, L9 - L8 {np.round(m9 - m8, 1)}")

# Contact sheet: rows before / after; columns night and noon at 150 km, then 75 km.
tiles = [("orbit150", "0000", (580, 160, 1340, 920), "150 km, midnight"),
         ("orbit150", "1200", (580, 160, 1340, 920), "150 km, noon"),
         ("ocean75", "0000", (420, 0, 1500, 1080), "75 km, midnight (L8 centre, L9 outside)"),
         ("ocean75", "1200", (420, 0, 1500, 1080), "75 km, noon")]
T, pad, top = 480, 8, 34
try:
    font = ImageFont.truetype("C:/Windows/Fonts/arial.ttf", 18)
except OSError:
    font = ImageFont.load_default()
sheet = Image.new("RGB", (len(tiles) * (T + pad) + 90, 2 * (T + top) + pad), (24, 26, 30))
d = ImageDraw.Draw(sheet)
for r, tag in enumerate(("before", "after")):
    y0 = r * (T + top + pad)
    d.text((8, y0 + top + T // 2 - 10), tag, fill=(235, 235, 235), font=font)
    for c, (shot, time, box, label) in enumerate(tiles):
        x0 = 90 + c * (T + pad)
        im = Image.open(f"{root}/{tag}-{time}/{shot}.png").convert("RGB").crop(box).resize((T, T), Image.LANCZOS)
        sheet.paste(im, (x0, y0 + top))
        d.text((x0, y0 + 8), label, fill=(235, 235, 235), font=font)
sheet.save(out, quality=88)
print(f"wrote {out}")

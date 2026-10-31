"""Holes in a lake's surface: pixels no water was drawn on, in a thin gap
inside water that was (B1's "hit pixels"; game/scripts/b2-seam.play).

    python seam_holes.py DIR [DIR ...]      one column per run

A pixel is water-drawn when the frame with water differs from the frame
without. A hole is a pixel that isn't, with drawn water close by on both
sides along a row or a column, in open water: at least OPEN of the 81 x 81
pixels around it are drawn. Without that last test the count is mostly the
voxel staircase along shores and river banks, which painting the pixels over
a frame shows plainly; a seam runs through open water. The stats panel,
hotbar and crosshair are left out.
"""
import os
import sys

import numpy as np
from PIL import Image

POSES = ["a_w12", "b_w6", "c_w18", "d_w16", "e_w4", "f_w1"]
REACH = 12  # pixels, at 4K
OPEN = 0.9
WINDOW = 40  # half-width of the open-water test, pixels


def density(mask):
    """Share of True in the (2 WINDOW + 1) square around each pixel."""
    s = np.zeros((mask.shape[0] + 1, mask.shape[1] + 1), np.int64)
    s[1:, 1:] = mask.cumsum(axis=0).cumsum(axis=1)
    h, w = mask.shape
    y0, y1 = np.clip(np.arange(h) - WINDOW, 0, h), np.clip(np.arange(h) + WINDOW + 1, 0, h)
    x0, x1 = np.clip(np.arange(w) - WINDOW, 0, w), np.clip(np.arange(w) + WINDOW + 1, 0, w)
    area = np.outer(y1 - y0, x1 - x0)
    total = s[y1][:, x1] - s[y0][:, x1] - s[y1][:, x0] + s[y0][:, x0]
    return total / area


def load(path):
    return np.asarray(Image.open(path).convert("RGB")).astype(np.int16)


def holes(run, pose):
    w, n = os.path.join(run, f"w_{pose}.png"), os.path.join(run, f"n_{pose}.png")
    if not (os.path.exists(w) and os.path.exists(n)):
        return None
    a, b = load(w), load(n)
    drawn = (np.abs(a - b).max(axis=2) > 2)
    h, wd = drawn.shape
    keep = np.ones((h, wd), bool)
    keep[: int(0.27 * h), : int(0.42 * wd)] = False  # the stats panel
    keep[int(0.92 * h) :, :] = False  # the hotbar
    cy, cx = h // 2, wd // 2
    keep[cy - 24 : cy + 24, cx - 24 : cx + 24] = False  # the crosshair

    def near(axis, sign):
        out = np.zeros_like(drawn)
        for k in range(1, REACH + 1):
            out |= np.roll(drawn, sign * k, axis=axis)
        return out

    gap = (near(0, 1) & near(0, -1)) | (near(1, 1) & near(1, -1))
    return int((~drawn & gap & keep & (density(drawn) >= OPEN)).sum()), int((drawn & keep).sum())


def main():
    runs = sys.argv[1:]
    print("| pose | " + " | ".join(os.path.basename(r.rstrip("/\\")) for r in runs) + " |")
    print("|---|" + "---|" * len(runs))
    totals = [0] * len(runs)
    for pose in POSES:
        cells = []
        for i, r in enumerate(runs):
            v = holes(r, pose)
            cells.append("no frame" if v is None else f"{v[0]} of {v[1]}")
            totals[i] += 0 if v is None else v[0]
        print(f"| {pose} | " + " | ".join(cells) + " |")
    print("| total | " + " | ".join(str(t) for t in totals) + " |")


if __name__ == "__main__":
    main()

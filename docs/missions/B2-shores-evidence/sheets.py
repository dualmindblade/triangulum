"""Before/after contact sheets from two runs of game/scripts/b2-shores.play.

    python sheets.py BEFORE_DIR AFTER_DIR OUT_DIR [--mid MID_DIR]

Frames pair up by name, one row a pose, before on the left and after on the
right (with --mid, a middle column: after #2 and #4, before #5). The 400 x 90
pixel strip the stats panel leaves at the top left is kept: it names the LODs
drawn.
"""
import argparse
import json
import os

from PIL import Image, ImageDraw, ImageFont

SHEETS = {
    "sheet_1_lakes": ["rec_041049_start", "lake_6000", "lake_2500", "lake_1000", "lake_0400", "lake_0120"],
    "sheet_2_coast": ["rec_041021_start", "coast_6000", "coast_2500", "coast_1000", "coast_0400", "coast_0120"],
    "sheet_3_inland_sea": ["rec_041200", "sea_6000", "sea_2500", "sea_1000", "sea_0400", "sea_0120"],
    "sheet_4_sea_and_orbit": ["rec_041049_end", "rec_041021_end", "orbit_040km", "orbit_150km"],
    "sheet_5_water_line": ["foot_041021", "foot_041200", "foot_q10", "foot_q50", "foot_q90"],
}
W, H, GAP, CAP = 720, 405, 6, 24


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("before")
    ap.add_argument("after")
    ap.add_argument("out")
    ap.add_argument("--mid")
    a = ap.parse_args()
    cols = [("before", a.before)] + ([("after #2, #4", a.mid)] if a.mid else []) + [("after", a.after)]
    try:
        font = ImageFont.truetype("consola.ttf", 16)
    except OSError:
        font = ImageFont.load_default()
    os.makedirs(a.out, exist_ok=True)
    for name, poses in SHEETS.items():
        sheet = Image.new("RGB", (len(cols) * (W + GAP) + GAP, len(poses) * (H + CAP + GAP) + GAP), (24, 24, 24))
        d = ImageDraw.Draw(sheet)
        for r, pose in enumerate(poses):
            y = GAP + r * (H + CAP + GAP)
            for c, (label, folder) in enumerate(cols):
                x = GAP + c * (W + GAP)
                path = os.path.join(folder, pose + ".png")
                text = f"{label}: {pose}"
                if os.path.exists(path):
                    sheet.paste(Image.open(path).convert("RGB").resize((W, H), Image.LANCZOS), (x, y + CAP))
                    side = path[:-4] + ".json"
                    if os.path.exists(side):
                        s = json.load(open(side))
                        text += f"  lat {s['lat']:.3f} lon {s['lon']:.3f} alt {s['alt']:.0f} m"
                else:
                    text += "  (no frame)"
                d.text((x + 2, y + 3), text, fill=(235, 235, 235), font=font)
        path = os.path.join(a.out, name + ".jpg")
        sheet.save(path, quality=86)
        print(path, sheet.size, os.path.getsize(path) // 1024, "KB")


if __name__ == "__main__":
    main()

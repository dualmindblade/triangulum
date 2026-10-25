"""Assemble unaltered M1c screenshots; run from any working directory.

    python docs/missions/M1c-sky-evidence/contact_sheet.py

Only thumbnail resampling and labels are applied. Raw 1920x1080 frames
remain in game/out/m1c-sky/ next to the manifests and harness transcripts.
"""

import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "game/out/m1c-sky"
DEST = Path(__file__).resolve().parent
PHASES = ["noon", "golden", "dusk", "night"]
ROWS = [
    ("Engine | hills | 1.7 m above ground | default 1/70.3", "engine-final", "hills"),
    ("July viewer | hills | 50 m above ground to clear foliage", "viewer-clear", "hills"),
    ("Engine | snowy range | 1.7 m above ground | default 1/70.3", "engine-final", "snow"),
    ("Engine | snowy range | 1.7 m above ground | comparison 1/8, H4096 / sea1536", "engine-vscale8-final", "snow"),
    ("July viewer | snowy range | 50 m above ground", "viewer-clear", "snow"),
    ("Engine | 10 km above sea level", "engine-final", "10km"),
    ("July viewer | 956.2 km | matched altitude / planet radius", "viewer-clear", "10km"),
    ("Engine | orbit, 500 km above sea level", "engine-final", "orbit"),
    ("July viewer | orbit, 47,810 km | matched altitude / planet radius", "viewer-clear", "orbit"),
]


def main():
    manifest = {v["name"]: v for v in json.loads((OUT / "engine-final/manifest.json").read_text())}
    font = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 22)
    small = ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", 18)
    title = ImageFont.truetype("C:/Windows/Fonts/segoeuib.ttf", 30)
    width, height, gap, band, label = 640, 360, 10, 120, 62
    sheet = Image.new("RGB", (4 * width + 5 * gap, band + len(ROWS) * (height + label + gap) + gap), "#131922")
    draw = ImageDraw.Draw(sheet)
    draw.text((gap, 8), "M1c sky / atmosphere / lighting — engine and July viewer", font=title, fill="#edf3fa")
    draw.text((gap, 51), "Same solar directions. Clouds off in July. Engine has no moon disc. All captures 1920×1080; thumbnails only.", font=font, fill="#b9c7d9")
    for col, phase in enumerate(PHASES):
        text = {"noon": "NOON", "golden": "GOLDEN HOUR  /  sun +5°", "dusk": "DUSK  /  sun −3°", "night": "NIGHT  /  local solar midnight"}[phase]
        draw.text((gap + col * (width + gap), 85), text, font=font, fill="#e7c391")
    for row, (description, folder, pose) in enumerate(ROWS):
        top = band + row * (height + label + gap)
        draw.text((gap, top), description, font=font, fill="#edf3fa")
        for col, phase in enumerate(PHASES):
            name = f"{pose}_{phase}"
            v = manifest[name]
            x = gap + col * (width + gap)
            hour = v["local_solar_hour"]
            stamp = f"{int(hour):02}:{int((hour % 1) * 60):02} solar  |  sun {v['sun_altitude_deg']:+.1f}°"
            draw.text((x, top + 29), stamp, font=small, fill="#a8bbd2")
            with Image.open(OUT / folder / f"{name}.png") as raw:
                assert raw.size == (1920, 1080), (name, raw.size)
                sheet.paste(raw.convert("RGB").resize((width, height), Image.Resampling.LANCZOS), (x, top + label))
    target = DEST / "contact-sheet.png"
    sheet.save(target)
    print(target)
    print(f"{sheet.width} x {sheet.height}; 36 labeled screenshots")


if __name__ == "__main__":
    main()

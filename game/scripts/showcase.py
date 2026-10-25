"""Render the M1 showcase video with the engine's own play harness.

    python game/scripts/showcase.py                 # render, assemble, encode (about 4 min)
    python game/scripts/showcase.py --skip-render   # re-assemble existing clips only

1. builds the engine and runs scripts/m1-showcase.play (orbit, river, the
   hands-on build, the pull-back) and scripts/m1-showcase-vscale8.play
   (vertical scale 1/8) at 1920x1080; each `record ... stop` in them writes an
   H.264 clip plus its caption cues to out/showcase/clips/{main,vscale8}/;
2. adds a title and an end card, crossfades the clips in name order, burns
   in the captions and writes the full-quality master
   out/showcase/m1-showcase-1080p.mp4 (gitignored);
3. encodes docs/media/m1-showcase.mp4 (1280x720, 30 fps, H.264, kept under
   15 MB) and extracts review stills into docs/media/m1-showcase-stills/.

Needs ffmpeg and ffprobe on PATH (or --ffmpeg DIR) and the baked world:
game/assets/world/seed42, else C:/code/triangulum/game/assets/world/seed42
(--bake DIR). Fonts: Segoe UI from C:/Windows/Fonts (--font FILE).
"""

import argparse
import glob
import json
import os
import shutil
import subprocess
import sys

GAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = os.path.dirname(GAME)
FPS = 30
XFADE = 0.6  # seconds of crossfade between clips
TITLE = ("Triangulum", "M1: the new engine, recorded by the engine itself")
END = ("Triangulum M1", "Neisor, seed 42  |  rendered headless by the play harness (game/scripts/showcase.py)")
# Review stills: (caption prefix, fraction of the way through that caption).
STILLS = [
    ("Neisor from orbit", 0.5),
    ("Down through the atmosphere", 0.55),
    ("Over the highest range", 0.6),
    ("Down a river valley", 0.75),
    ("...and walk across it", 0.35),
    ("...zoomed in", 0.8),
    ("Dusk into night", 0.5),
    ("Vertical scale 1/8", 0.45),
]
MAX_MB = 15.0


def run(cmd, **kw):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    r = subprocess.run(cmd, **kw)
    if r.returncode != 0:
        sys.exit(f"failed ({r.returncode}): {cmd[0]}")
    return r


def probe_frames(ffprobe, path):
    out = subprocess.run([ffprobe, "-v", "error", "-select_streams", "v:0", "-count_packets", "-show_entries", "stream=nb_read_packets", "-of", "csv=p=0", path], capture_output=True, text=True, check=True).stdout
    return int(out.strip())


def fmt(t):
    return f"{t:.3f}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bake", default=None)
    ap.add_argument("--size", default="1920x1080")
    ap.add_argument("--out", default=os.path.join(GAME, "out", "showcase"))
    ap.add_argument("--skip-render", action="store_true", help="re-assemble the clips already in OUT/clips")
    ap.add_argument("--ffmpeg", default=None, help="directory holding ffmpeg and ffprobe")
    ap.add_argument("--font", default="C:/Windows/Fonts/segoeui.ttf")
    ap.add_argument("--font-bold", default="C:/Windows/Fonts/segoeuib.ttf")
    ap.add_argument("--video", default=os.path.join(REPO, "docs", "media", "m1-showcase.mp4"))
    ap.add_argument("--stills", default=os.path.join(REPO, "docs", "media", "m1-showcase-stills"))
    a = ap.parse_args()
    a.out, a.video, a.stills = (os.path.abspath(p) for p in (a.out, a.video, a.stills))

    ffmpeg = os.path.join(a.ffmpeg, "ffmpeg") if a.ffmpeg else "ffmpeg"
    ffprobe = os.path.join(a.ffmpeg, "ffprobe") if a.ffmpeg else "ffprobe"
    bake = a.bake
    if bake is None:
        local = os.path.join(GAME, "assets", "world", "seed42")
        bake = local if os.path.exists(os.path.join(local, "meta.json")) else "C:/code/triangulum/game/assets/world/seed42"
    w, h = (int(x) for x in a.size.split("x"))
    clips_dir = os.path.join(a.out, "clips")
    os.makedirs(a.out, exist_ok=True)

    # ---- 1. render the clips -------------------------------------------------
    if not a.skip_render:
        if os.path.isdir(clips_dir):
            shutil.rmtree(clips_dir)
        run(["cargo", "build", "--release"], cwd=GAME)
        exe = os.path.join(GAME, "target", "release", "triangulum")
        common = ["--bake", bake, "--size", a.size]
        # One directory per run, so each keeps its run.log transcript.
        run([exe, *common, "--out", os.path.join(clips_dir, "main"), "--play", os.path.join(GAME, "scripts", "m1-showcase.play")], cwd=GAME)
        run([exe, *common, "--out", os.path.join(clips_dir, "vscale8"), "--vscale", "0.125", "--height", "4096", "--sea-level", "1536",
             "--play", os.path.join(GAME, "scripts", "m1-showcase-vscale8.play")], cwd=GAME)
    # Clips play in file-name order (10_, 20_, ...) whichever run made them.
    clips = sorted(glob.glob(os.path.join(clips_dir, "*.mp4")) + glob.glob(os.path.join(clips_dir, "*", "*.mp4")), key=os.path.basename)
    if not clips:
        sys.exit(f"no clips in {clips_dir}")

    # ---- 2. assemble: title, clips, end card; captions ------------------------
    work = os.path.join(a.out, "work")
    os.makedirs(work, exist_ok=True)
    for src, name in ((a.font, "regular.ttf"), (a.font_bold, "bold.ttf")):
        shutil.copyfile(src, os.path.join(work, name))  # relative font paths: no ':' escaping in filters

    def text_file(name, text):
        with open(os.path.join(work, name), "w", encoding="utf-8") as f:
            f.write(text)
        return name

    card_s = 3.0
    segments = [("title", card_s, None)]
    for c in clips:
        cues_path = c[: -len(".mp4")] + ".captions.json"
        cues = json.load(open(cues_path, encoding="utf-8")) if os.path.exists(cues_path) else []
        segments.append((c, probe_frames(ffprobe, c) / FPS, cues))
    segments.append(("end", card_s, None))

    inputs, chains = [], []
    starts, t = [], 0.0
    for n, (src, dur, _) in enumerate(segments):
        starts.append(t)
        t += dur - (XFADE if n < len(segments) - 1 else 0.0)
        if src in ("title", "end"):
            big, small = TITLE if src == "title" else END
            inputs += ["-f", "lavfi", "-t", fmt(dur), "-i", f"color=c=0x0b0f14:s={w}x{h}:r={FPS}"]
            tb, ts = text_file(f"{src}_big.txt", big), text_file(f"{src}_small.txt", small)
            chains.append(
                f"[{n}:v]drawtext=fontfile=bold.ttf:textfile={tb}:expansion=none:fontsize={int(h * 0.085)}:fontcolor=white:x=(w-tw)/2:y=h*0.40,"
                f"drawtext=fontfile=regular.ttf:textfile={ts}:expansion=none:fontsize={int(h * 0.030)}:fontcolor=0xc8d0d8:x=(w-tw)/2:y=h*0.55,"
                f"format=yuv420p,settb=AVTB,setpts=PTS-STARTPTS[s{n}]"
            )
        else:
            inputs += ["-i", src]
            chains.append(f"[{n}:v]fps={FPS},scale={w}:{h},format=yuv420p,settb=AVTB,setpts=PTS-STARTPTS[s{n}]")
    total = t
    graph = list(chains)
    last = "s0"
    for n in range(1, len(segments)):
        offset = starts[n]
        graph.append(f"[{last}][s{n}]xfade=transition=fade:duration={XFADE}:offset={fmt(offset)}[x{n}]")
        last = f"x{n}"

    # Captions: each cue on the timeline, clear of the crossfades, faded in and out.
    caption_list = []
    for n, (src, dur, cues) in enumerate(segments):
        for cue in cues or []:
            a0 = starts[n] + max(cue["start"], XFADE if n > 0 else 0.0)
            a1 = starts[n] + min(cue["end"], dur - XFADE)
            if a1 - a0 < 0.8:
                continue
            caption_list.append((a0, a1, cue["text"]))
    draws = []
    for i, (a0, a1, text) in enumerate(caption_list):
        tf = text_file(f"cap{i:02d}.txt", text)
        fade = 0.35
        alpha = f"if(lt(t,{fmt(a0 + fade)}),(t-{fmt(a0)})/{fade},if(gt(t,{fmt(a1 - fade)}),({fmt(a1)}-t)/{fade},1))"
        draws.append(
            f"drawtext=fontfile=regular.ttf:textfile={tf}:expansion=none:fontsize={int(h * 0.038)}:fontcolor=white:"
            f"box=1:boxcolor=black@0.5:boxborderw={int(h * 0.014)}:x={int(w * 0.035)}:y={int(h * 0.06)}:"
            f"enable='between(t,{fmt(a0)},{fmt(a1)})':alpha='{alpha}'"
        )
    tail = ",".join(draws + [f"fade=t=in:st=0:d=0.5", f"fade=t=out:st={fmt(total - 0.7)}:d=0.7", "format=yuv420p"])
    graph.append(f"[{last}]{tail}[out]")
    graph_path = os.path.join(work, "graph.txt")
    with open(graph_path, "w", encoding="utf-8") as f:
        f.write(";\n".join(graph))
    master = os.path.join(a.out, "m1-showcase-1080p.mp4")
    run([ffmpeg, "-y", "-loglevel", "error", *inputs, "-/filter_complex", "graph.txt", "-map", "[out]", "-r", str(FPS),
         "-c:v", "libx264", "-preset", "slow", "-crf", "17", "-pix_fmt", "yuv420p", "-movflags", "+faststart", os.path.abspath(master)], cwd=work)
    with open(os.path.join(a.out, "timeline.json"), "w", encoding="utf-8") as f:
        json.dump({"total_s": total, "segments": [{"src": os.path.basename(s), "start": st, "dur": d} for (s, d, _), st in zip(segments, starts)],
                   "captions": [{"start": c0, "end": c1, "text": tx} for c0, c1, tx in caption_list]}, f, indent=1)
    print(f"master: {master} ({total:.1f} s)")

    # ---- 3. the 720p video for the repo, and the stills ---------------------------
    os.makedirs(os.path.dirname(a.video), exist_ok=True)
    enc = ["-vf", "scale=1280:720:flags=lanczos", "-c:v", "libx264", "-preset", "slow", "-profile:v", "high", "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an"]
    run([ffmpeg, "-y", "-loglevel", "error", "-i", master, *enc, "-crf", "23", a.video])
    mb = os.path.getsize(a.video) / 1e6
    if mb > MAX_MB:
        # Two-pass at a bitrate that lands at ~85% of the limit.
        kbps = int(MAX_MB * 0.85 * 8000 / total)
        print(f"{mb:.1f} MB at CRF 23 is over {MAX_MB} MB: two-pass at {kbps} kb/s")
        passlog = os.path.join(work, "pass")
        run([ffmpeg, "-y", "-loglevel", "error", "-i", master, *enc, "-b:v", f"{kbps}k", "-pass", "1", "-passlogfile", passlog, "-f", "mp4", os.devnull])
        run([ffmpeg, "-y", "-loglevel", "error", "-i", master, *enc, "-b:v", f"{kbps}k", "-pass", "2", "-passlogfile", passlog, a.video])
        mb = os.path.getsize(a.video) / 1e6
    print(f"720p: {a.video} ({mb:.1f} MB)")

    if os.path.isdir(a.stills):
        for old in glob.glob(os.path.join(a.stills, "*.jpg")):
            os.remove(old)
    os.makedirs(a.stills, exist_ok=True)
    n = 0
    for prefix, frac in STILLS:
        match = [c for c in caption_list if c[2].startswith(prefix)]
        if not match:
            print(f"warning: no caption starts with {prefix!r}; still skipped")
            continue
        c0, c1, text = match[0]
        at = c0 + (c1 - c0) * frac
        n += 1
        slug = "".join(ch if ch.isalnum() else "-" for ch in prefix.lower()).strip("-")
        while "--" in slug:
            slug = slug.replace("--", "-")
        out = os.path.join(a.stills, f"{n:02d}_{slug}_{at:05.1f}s.jpg")
        run([ffmpeg, "-y", "-loglevel", "error", "-ss", fmt(at), "-i", master, "-frames:v", "1", "-vf", "scale=1280:720:flags=lanczos", "-q:v", "3", out])
    print(f"stills: {n} in {a.stills}")


if __name__ == "__main__":
    main()

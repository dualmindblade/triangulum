"""Simulation recorder — watch the planet form.

Hooks into the stage loops and captures evolving fields as image frames:
tectonic construction steps, ocean-current relaxation, every Nth moisture
advection step (the rain belt sweeping with the seasons), every erosion step
(mountains wearing down, rivers organizing).

Outputs land in <out>/simviz/:
  player.html   scrubbable player — pick a sequence, play/pause/step/speed
  *.jpg         the frames themselves
  *.mp4         one clip per sequence (only with record_video, needs ffmpeg)

With live=True a window opens during the run and shows frames as they are
rendered (about the closest thing to watching the planet happen in real time).
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time

import numpy as np
from matplotlib import colormaps
from PIL import Image, ImageDraw

# hypsometric mini-ramp for elevation frames (km)
_HYPSO_X = np.array([-8, -4, -2, -0.5, -0.01, 0.0, 0.3, 0.9, 1.6, 2.6, 3.6, 4.6, 5.6, 8.0])
_HYPSO_C = np.array([[5, 15, 40], [16, 48, 98], [24, 65, 118], [66, 116, 165],
                     [118, 168, 200], [76, 124, 72], [112, 152, 82], [174, 182, 102],
                     [194, 164, 94], [162, 124, 78], [134, 98, 68], [144, 128, 118],
                     [202, 202, 202], [250, 250, 250]], dtype=float) / 255.0


class Recorder:
    def __init__(self, grid, out_dir, width=720, every=4, live=False):
        self.grid = grid
        self.dir = out_dir
        os.makedirs(out_dir, exist_ok=True)
        self.W = width
        self.H = width // 2
        self.idx = grid.equirect_index(self.W, self.H)
        self.every = max(int(every), 1)
        self.coast = None
        self.count = {}
        self.meta = {}
        self.live = live
        self._live_fig = None
        self._live_last = 0.0

    # ------------------------------------------------------------------
    def set_coast(self, is_ocean):
        o = is_ocean[self.idx]
        edge = np.zeros_like(o)
        edge[1:, :] |= o[1:, :] != o[:-1, :]
        edge[:, 1:] |= o[:, 1:] != o[:, :-1]
        self.coast = edge

    # ------------------------------------------------------------------
    def frame(self, seq, field, label="", vmin=None, vmax=None, cmap="viridis", every=1):
        """Record one frame of `field` into sequence `seq` (skipping unless the
        call counter hits `every`). Color scale is frozen at first frame so
        change over time reads truthfully."""
        n = self.count.get(seq, 0)
        self.count[seq] = n + 1
        if n % max(every, 1):
            return
        m = self.meta.get(seq)
        if m is None:
            f = np.asarray(field, dtype=float)
            if cmap != "hypso":
                if vmin is None:
                    vmin = float(np.nanpercentile(f, 2))
                if vmax is None:
                    vmax = float(np.nanpercentile(f, 98)) or 1.0
            m = self.meta[seq] = dict(files=[], labels=[], vmin=vmin, vmax=vmax, cmap=cmap)
        im = self._render(field, label, m["vmin"], m["vmax"], m["cmap"])
        fn = f"{seq}_{len(m['files']):04d}.jpg"
        im.save(os.path.join(self.dir, fn), quality=82)
        m["files"].append(fn)
        m["labels"].append(label)
        if self.live:
            self._show_live(im, seq)

    def _render(self, field, label, vmin, vmax, cmap):
        img = np.nan_to_num(np.asarray(field, dtype=float))[self.idx]
        if cmap == "hypso":
            rgb = np.stack([np.interp(img, _HYPSO_X, _HYPSO_C[:, c]) for c in range(3)], -1)
        else:
            v = np.clip((img - vmin) / max(vmax - vmin, 1e-12), 0.0, 1.0)
            rgb = colormaps[cmap](v)[..., :3]
        if self.coast is not None:
            rgb[self.coast] *= 0.35
        im = Image.fromarray((rgb * 255).astype(np.uint8))
        if label:
            d = ImageDraw.Draw(im)
            d.text((9, 7), label, fill=(0, 0, 0))
            d.text((8, 6), label, fill=(255, 255, 255))
        return im

    # ------------------------------------------------------------------
    def _show_live(self, im, title):
        now = time.time()
        if now - self._live_last < 0.15:
            return
        self._live_last = now
        try:
            import matplotlib
            if self._live_fig is None:
                matplotlib.use("TkAgg", force=True)
            import matplotlib.pyplot as plt
            if self._live_fig is None:
                plt.ion()
                self._live_fig, ax = plt.subplots(figsize=(9, 4.8), num="planetgen — live")
                ax.set_xticks([]); ax.set_yticks([])
                self._live_img = ax.imshow(np.asarray(im))
                self._live_fig.tight_layout()
            else:
                self._live_img.set_data(np.asarray(im))
            self._live_fig.canvas.draw_idle()
            plt.pause(0.001)
        except Exception:
            self.live = False   # headless or backend trouble: keep recording

    # ------------------------------------------------------------------
    def finalize(self, video=False):
        manifest = {seq: dict(files=m["files"], labels=m["labels"])
                    for seq, m in self.meta.items() if m["files"]}
        html = _PLAYER_HTML.replace("__MANIFEST__", json.dumps(manifest))
        player = os.path.join(self.dir, "player.html")
        with open(player, "w", encoding="utf-8") as f:
            f.write(html)
        if video:
            if shutil.which("ffmpeg"):
                for seq, m in manifest.items():
                    if len(m["files"]) < 8:
                        continue
                    subprocess.run(
                        ["ffmpeg", "-y", "-loglevel", "error", "-framerate", "18",
                         "-i", f"{seq}_%04d.jpg", "-c:v", "libx264", "-pix_fmt",
                         "yuv420p", f"{seq}.mp4"],
                        cwd=self.dir, check=False)
            else:
                print("[simviz] ffmpeg not found - skipped mp4 assembly")
        return player


_PLAYER_HTML = """<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>planetgen — simulation player</title>
<style>
 body { background:#10141c; color:#e8e8e8; font-family:Segoe UI,system-ui,sans-serif;
        margin:0; padding:14px; }
 h1 { font-size:17px; margin:2px 0 10px; }
 .bar { display:flex; gap:10px; align-items:center; flex-wrap:wrap; margin-bottom:10px; }
 select,button,input { background:#232b3a; color:#dce3ee; border:1px solid #3a465c;
        border-radius:6px; padding:5px 9px; font-size:13px; }
 button { cursor:pointer; min-width:64px; } button:hover { background:#31405c; }
 input[type=range] { flex:1; min-width:200px; }
 img { max-width:100%; border-radius:8px; image-rendering:auto; }
 .lbl { font-size:12px; color:#9fb0c8; min-width:110px; text-align:right; }
</style></head><body>
<h1>planetgen — simulation player <span class="lbl" id="which"></span></h1>
<div class="bar">
 <select id="seq"></select>
 <button id="play">Play</button>
 <label>fps <input id="fps" type="number" value="15" min="1" max="60" style="width:56px"></label>
 <label><input id="loop" type="checkbox" checked> loop</label>
 <input id="pos" type="range" min="0" max="0" value="0">
 <span class="lbl" id="count"></span>
</div>
<img id="view">
<p style="color:#9fb0c8;font-size:12px">space = play/pause &nbsp; &larr;/&rarr; = step &nbsp;
 home/end = first/last frame</p>
<script>
const M = __MANIFEST__;
const seqs = Object.keys(M);
const sel = document.getElementById('seq'), img = document.getElementById('view'),
      pos = document.getElementById('pos'), cnt = document.getElementById('count'),
      which = document.getElementById('which'), fps = document.getElementById('fps'),
      loop = document.getElementById('loop'), playBtn = document.getElementById('play');
seqs.forEach(s => { const o = document.createElement('option'); o.value = s;
  o.textContent = s + '  (' + M[s].files.length + ' frames)'; sel.appendChild(o); });
let cur = seqs[0], i = 0, timer = null;
function show(k) {
  const m = M[cur]; i = Math.max(0, Math.min(k, m.files.length - 1));
  img.src = m.files[i]; pos.value = i; pos.max = m.files.length - 1;
  cnt.textContent = (i + 1) + ' / ' + m.files.length;
  which.textContent = m.labels[i] || '';
}
function tick() { const n = M[cur].files.length;
  if (i + 1 >= n) { if (loop.checked) show(0); else stop(); } else show(i + 1); }
function play() { stop(); timer = setInterval(tick, 1000 / (+fps.value || 15));
  playBtn.textContent = 'Pause'; }
function stop() { if (timer) clearInterval(timer); timer = null; playBtn.textContent = 'Play'; }
playBtn.onclick = () => timer ? stop() : play();
sel.onchange = () => { stop(); cur = sel.value; show(0); };
pos.oninput = () => { stop(); show(+pos.value); };
fps.onchange = () => { if (timer) play(); };
document.addEventListener('keydown', e => {
  if (e.key === ' ') { e.preventDefault(); playBtn.onclick(); }
  if (e.key === 'ArrowRight') { stop(); show(i + 1); }
  if (e.key === 'ArrowLeft') { stop(); show(i - 1); }
  if (e.key === 'Home') { stop(); show(0); }
  if (e.key === 'End') { stop(); show(1e9); }
});
// preload current sequence lazily
show(0);
</script></body></html>"""

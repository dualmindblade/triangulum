#!/usr/bin/env python3
"""X-1 (B-19): measure the lattice pitch of a class boundary in a WINDOW of
the frame whose ground range is known to be mesh-owned.

The altitude-ladder lint reports one frame-wide blended number; a 55 deg
ladder frame spans ground ranges from ~0.04*alt to ~2.4*alt, so it mixes the
voxel near-disc, the fine mesh, and (high up) the range-blended mesh. This
restricts the scan to rows 100..300 of a 720-row frame, i.e. ground range
0.85..1.63 x altitude -- outside the voxel patch at every altitude >= 0.7 km,
inside the 4 km range-blend gate, and inside MAX_LEVEL's flat region.

The reported pitch is the per-profile median lateral jump in world metres.
For a lattice-quantised boundary that IS the lattice pitch.
"""
import sys, math, argparse
from pathlib import Path
import numpy as np
from PIL import Image
from scipy import ndimage
sys.path.insert(0, str(Path(__file__).resolve().parent))
import altitude_ladder as al

ROW0, ROW1 = 100, 300


def ground_range_km(alt_km, row, h=720):
    dep = math.radians(al.DEPRESSION_DEG + (row - h / 2) / h * al.FOV_Y_DEG)
    return alt_km / math.tan(dep)


def profiles(path, alt, kind, row0=ROW0, row1=ROW1, min_jumps=3):
    im = np.asarray(Image.open(path).convert('RGB')).astype(np.int16)
    h = im.shape[0]
    band = im[row0:row1]
    mask = al.class_mask(band, kind)
    lab, n = ndimage.label(mask)
    if n == 0:
        return []
    sizes = ndimage.sum(mask, lab, range(1, n + 1))
    comp = lab == (int(np.argmax(sizes)) + 1)
    _, cross_km, down_km = al.frame_scale(alt, h)
    out = []
    for axis, m in ((0, comp), (1, np.ascontiguousarray(comp.T))):
        hh, w = m.shape
        lat_m = (cross_km if axis == 0 else down_km) * 1000.0
        for side in (0, 1):
            edge = []
            for y in range(hh):
                idx = np.flatnonzero(m[y])
                edge.append(int(idx[0] if side == 0 else idx[-1]) if idx.size >= 8 else -1)
            edge = np.array(edge, float)
            ok = edge > 0
            if ok.sum() < 0.5 * hh:
                continue
            v = edge[ok]
            d = np.diff(v)
            slope = float(np.abs(d).mean())
            if slope > 6.0:
                continue
            j = np.abs(d)[np.abs(d) >= max(3.0, 3.0 * slope)]
            if j.size < min_jumps:
                continue
            out.append(dict(axis=axis, side=side, n=int(j.size),
                            slope_px=round(slope, 2), rows=int(ok.sum()),
                            pitch_m=round(float(np.median(j)) * lat_m, 2)))
    return out


def report(run, kinds, alts, label=''):
    print(f'== {run} {label}')
    print(f'  {"site":<12}' + ' '.join(f'{a:>13}' for a in alts))
    res = {}
    for s, k in kinds.items():
        row = []
        for alt in alts:
            p = f'interchange/runs/{run}/{s}_{al.rung_tag(alt)}.png'
            try:
                ps = profiles(p, alt, k)
            except FileNotFoundError:
                ps = []
            ps.sort(key=lambda x: -x['n'])
            if ps:
                row.append(f"{ps[0]['pitch_m']:>8}({ps[0]['n']:>3})")
                res[(s, alt)] = ps[0]['pitch_m']
            else:
                row.append(f'{"-":>13}')
        print(f'  {s:<12}' + ' '.join(row))
    return res


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('runs', nargs='+')
    ap.add_argument('--alts', default='0.7,1.0,1.5')
    a = ap.parse_args()
    kinds = {'lake_shore': 'water', 'river_bank': 'water',
             'snowline': 'snow', 'ecotone': 'generic'}
    alts = [float(x) for x in a.alts.split(',')]
    for r in a.runs:
        report(r, kinds, alts)
    print()
    for lvl in (12, 13, 14, 15, 16):
        print(f'  predicted vertex spacing L{lvl} = '
              f'{math.pi / 2 * al.R_KM / (2**lvl * 32) * 1000:.2f} m')

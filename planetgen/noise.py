"""Seamless 3D gradient noise, fully vectorized in numpy.

Noise is evaluated at 3D points on (or near) the unit sphere, so there are no
map seams and no polar distortion. Everything is a pure function of position
and seed — the future game can re-evaluate the exact same fields at any
resolution for consistent local detail.
"""

from __future__ import annotations

import numpy as np

_MASK = np.int64(0xFFFFFFFF)

# fixed table of 256 well-distributed unit gradients
_rs = np.random.RandomState(1234567)
_g = _rs.normal(size=(256, 3))
_GRAD = _g / np.linalg.norm(_g, axis=1, keepdims=True)


def _hash(ix, iy, iz, seed):
    """Integer lattice hash -> [0, 255]. int64 arithmetic with 32-bit wrap."""
    h = (ix * np.int64(0x8DA6B343) + iy * np.int64(0xD8163841)
         + iz * np.int64(0xCB1AB31F) + np.int64(seed) * np.int64(0x9E3779B1)) & _MASK
    h = (h ^ (h >> 13)) * np.int64(0xC2B2AE35) & _MASK
    h = h ^ (h >> 16)
    return (h & 255).astype(np.intp)


def _fade(t):
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0)


def gradient_noise(p, seed=0):
    """Perlin-style gradient noise for points p (n, 3). Output roughly [-1, 1]."""
    pi = np.floor(p).astype(np.int64)
    pf = p - pi
    fx, fy, fz = _fade(pf[:, 0]), _fade(pf[:, 1]), _fade(pf[:, 2])
    total = np.zeros(len(p))
    for dx in (0, 1):
        wx = fx if dx else 1.0 - fx
        for dy in (0, 1):
            wy = fy if dy else 1.0 - fy
            for dz in (0, 1):
                wz = fz if dz else 1.0 - fz
                g = _GRAD[_hash(pi[:, 0] + dx, pi[:, 1] + dy, pi[:, 2] + dz, seed)]
                d = pf - np.array([dx, dy, dz], dtype=np.float64)
                total += (wx * wy * wz) * np.einsum("nd,nd->n", g, d)
    return total * 1.9


def fbm(p, octaves=5, freq=1.0, lacunarity=2.0, gain=0.5, seed=0):
    """Fractal Brownian motion; output roughly [-1, 1]."""
    total = np.zeros(len(p))
    amp, f, norm = 1.0, freq, 0.0
    for o in range(octaves):
        total += amp * gradient_noise(p * f, seed=seed * 7919 + o * 131)
        norm += amp
        amp *= gain
        f *= lacunarity
    return total / norm


def ridged(p, octaves=5, freq=1.0, lacunarity=2.0, gain=0.5, seed=0):
    """Ridged multifractal — sharp crests, good for young mountain texture. [0, 1]."""
    total = np.zeros(len(p))
    amp, f, norm = 1.0, freq, 0.0
    for o in range(octaves):
        n = 1.0 - np.abs(gradient_noise(p * f, seed=seed * 104729 + o * 131))
        total += amp * n * n
        norm += amp
        amp *= gain
        f *= lacunarity
    return total / norm


def fbm_vec3(p, octaves=4, freq=1.0, seed=0):
    """Three independent fBm channels -> (n, 3), for domain warping."""
    return np.stack([fbm(p, octaves, freq, seed=seed + 11),
                     fbm(p, octaves, freq, seed=seed + 23),
                     fbm(p, octaves, freq, seed=seed + 37)], 1)


def warp(p, amp=0.4, freq=1.0, octaves=4, seed=0, renormalize=True):
    """Domain-warped positions; renormalize keeps points on the unit sphere."""
    q = p + amp * fbm_vec3(p, octaves=octaves, freq=freq, seed=seed)
    if renormalize:
        q = q / np.linalg.norm(q, axis=1, keepdims=True)
    return q

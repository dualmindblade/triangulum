"""Freeze values from the supplied CSC reference without requiring astropy."""
import ast
from pathlib import Path
import struct

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
source = (ROOT / "tools/reference/csc_check.py").read_text()
tree = ast.parse(source)
coefficients = next(ast.literal_eval(n.value) for n in tree.body
                    if isinstance(n, ast.Assign)
                    and any(isinstance(t, ast.Name) and t.id == "P" for t in n.targets))


def reference(u, v):
    return u + u*(1-u*u)*sum(c*(u*u)**i*(v*v)**j for (i, j), c in coefficients.items())


rng = np.random.default_rng(20261024)
grid = np.linspace(-1, 1, 17)
points = [(p, q) for p in grid for q in grid]
points.extend(rng.uniform(-1, 1, (1024, 2)))
dest = ROOT / "data/csc_reference.bin"
dest.write_bytes(b"".join(struct.pack("<dddd", p, q, reference(p, q), reference(q, p))
                          for p, q in points))
print(f"wrote {len(points)} reference pairs to {dest.name}")


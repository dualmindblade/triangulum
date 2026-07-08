#!/usr/bin/env python3
"""Turn a viewer screenshot name into reproducible capture/play commands.

New shots with a JSON sidecar use the recorded pose and effective sun. Legacy
`shot_lat..._lon...` PNGs without a sidecar still recover the pose from the
filename, but the original sun is unknowable.
"""
import argparse
import json
import math
import re
import subprocess
import sys
from pathlib import Path

DEFAULT_SUN = (30.0, 30.0)

SHOT_RE = re.compile(
    r"shot_lat(?P<lat>-?\d+(?:\.\d+)?)"
    r"_lon(?P<lon>-?\d+(?:\.\d+)?)"
    r"(?:_alt(?P<alt>-?\d+(?:\.\d+)?)km)?"
    r"(?:_yaw(?P<yaw>-?\d+(?:\.\d+)?))?"
    r"(?:_pitch(?P<pitch>-?\d+(?:\.\d+)?))?"
    r"(?:_\d+)?$"
)


def fmt(x):
    x = float(x)
    if abs(x) < 0.0000005:
        x = 0.0
    s = f"{x:.6f}".rstrip("0").rstrip(".")
    return s if s and s != "-0" else "0"


def parse_filename(path):
    m = SHOT_RE.match(path.stem)
    if not m:
        return {}
    out = {}
    for key, val in m.groupdict().items():
        if val is not None:
            out[key] = float(val)
    return out


def load_sidecar(path):
    sidecar = path.with_suffix(".json")
    if not sidecar.exists():
        return None, sidecar
    with sidecar.open("r", encoding="utf-8") as f:
        return json.load(f), sidecar


def sun_from_dir(sidecar):
    d = sidecar.get("sun_dir")
    if not isinstance(d, dict):
        return None
    try:
        x, y, z = float(d["x"]), float(d["y"]), float(d["z"])
    except (KeyError, TypeError, ValueError):
        return None
    n = math.sqrt(x * x + y * y + z * z)
    if n == 0.0:
        return None
    return (math.degrees(math.asin(z / n)), math.degrees(math.atan2(y, x)))


def field(sidecar, parsed, sidecar_key, parsed_key, default=None):
    if sidecar and sidecar.get(sidecar_key) is not None:
        return float(sidecar[sidecar_key])
    if parsed_key in parsed:
        return parsed[parsed_key]
    return default


def require_pose(pose):
    missing = [k for k in ("lat", "lon", "alt", "yaw", "pitch") if pose[k] is None]
    if missing:
        raise SystemExit(
            "could not recover "
            + ", ".join(missing)
            + " from sidecar or filename"
        )


def shot_name(path):
    name = re.sub(r"[^A-Za-z0-9_.-]+", "_", path.stem).strip("._")
    return name or "repro"


def command_line(path, pose):
    argv = [
        "cargo",
        "run",
        "--release",
        "--",
        "--capture",
        str(path),
        "--lat",
        fmt(pose["lat"]),
        "--lon",
        fmt(pose["lon"]),
        "--alt",
        fmt(pose["alt"]),
        "--yaw",
        fmt(pose["yaw"]),
        "--pitch",
        fmt(pose["pitch"]),
        "--exagg",
        fmt(pose["exagg"]),
        "--sun-lat",
        fmt(pose["sun_lat"]),
        "--sun-lon",
        fmt(pose["sun_lon"]),
    ]
    return subprocess.list2cmdline(argv)


def play_script(path, pose):
    lines = [
        f"# Generated from {path}",
        f"teleport {fmt(pose['lat'])} {fmt(pose['lon'])} {fmt(pose['alt'])}",
        f"look {fmt(pose['yaw'])} {fmt(pose['pitch'])}",
        f"mode {pose['mode']}",
        f"sun {fmt(pose['sun_lat'])} {fmt(pose['sun_lon'])}",
        f"shot {shot_name(path)}",
    ]
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("shot", help="screenshot PNG path")
    ap.add_argument("--out", help="write the generated .play script here")
    args = ap.parse_args()

    path = Path(args.shot)
    parsed = parse_filename(path)
    sidecar, sidecar_path = load_sidecar(path)

    pose = {
        "lat": field(sidecar, parsed, "lat_deg", "lat"),
        "lon": field(sidecar, parsed, "lon_deg", "lon"),
        "alt": field(sidecar, parsed, "alt_km", "alt"),
        "yaw": field(sidecar, parsed, "yaw_deg", "yaw", 0.0),
        "pitch": field(sidecar, parsed, "pitch_deg", "pitch", 0.0),
        "mode": (sidecar or {}).get("mode", "fly"),
        "exagg": float((sidecar or {}).get("exaggeration", 1.0)),
    }
    require_pose(pose)

    if sidecar and sidecar.get("sun_lat_deg") is not None and sidecar.get("sun_lon_deg") is not None:
        pose["sun_lat"] = float(sidecar["sun_lat_deg"])
        pose["sun_lon"] = float(sidecar["sun_lon_deg"])
    else:
        sun = sun_from_dir(sidecar or {})
        if sun is not None:
            pose["sun_lat"], pose["sun_lon"] = sun
        else:
            pose["sun_lat"], pose["sun_lon"] = DEFAULT_SUN
            if sidecar:
                print(
                    f"warning: {sidecar_path} has no sun state; using default sun "
                    f"{fmt(DEFAULT_SUN[0])} {fmt(DEFAULT_SUN[1])}",
                    file=sys.stderr,
                )
            else:
                print(
                    f"warning: no sidecar at {sidecar_path}; sun state is unknown, "
                    f"using default sun {fmt(DEFAULT_SUN[0])} {fmt(DEFAULT_SUN[1])}",
                    file=sys.stderr,
                )

    script = play_script(path, pose)
    print("# equivalent --capture command")
    print(command_line(path, pose))
    if args.out:
        Path(args.out).write_text(script, encoding="utf-8")
        print(f"# wrote play script: {args.out}")
    else:
        print()
        print("# play script")
        print(script, end="")


if __name__ == "__main__":
    main()

"""Shift absolute .play layers after a sea-level move (default: +512).

From game/: python scripts/shift_layers.py [--delta 512] [FILE_OR_DIR ...]
With no paths, visits scripts/ and src/. Run once, including on new scripts at
merge: this is intentionally not idempotent. Comments, world-space poses,
metre altitudes, relative offsets, levels, LODs and tolerances are untouched.
"""

import argparse
from pathlib import Path
import re


NUMBER = r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?"
ATBLOCK = re.compile(r"^(\s*(?:atblock|target)\s+\d+\s+[+-]?\d+\s+[+-]?\d+\s+)(" + NUMBER + r")(?=\s|$)")
CELL = re.compile(r"^(\s*assert\s+(?:block|level|cover)\[\s*\d+\s*,\s*[+-]?\d+\s*,\s*[+-]?\d+\s*,\s*)(" + NUMBER + r")(\s*\])")
LAYER_ASSERT = re.compile(r"^(\s*assert\s+(?:k|fk|crosshair_k|find_k|find_target_k)\s+(?:(?:==|!=|<=|>=|<|>|~)\s+)?)(" + NUMBER + r")(?=\s|$)")


def shift(text, delta):
    def add(match):
        value = match[2]
        new = str(int(value) + delta) if re.fullmatch(r"[+-]?\d+", value) else format(float(value) + delta, ".15g")
        return match[1] + new + (match[3] if match.lastindex == 3 else "")

    result = []
    for line in text.splitlines(keepends=True):
        command, marker, comment = line.partition("#")
        command = ATBLOCK.sub(add, command)
        command = CELL.sub(add, command)
        command = LAYER_ASSERT.sub(add, command)
        result.append(command + marker + comment)
    return "".join(result)


def self_test():
    source = """# atblock 1 2 3 4; block[1,2,3,4]
atblock 1 512 1024 513 # stay 512
target 1 512 1024 513 # a block centre, not a metre altitude
target rel 1 2 512
target ll 1 2 512
key 1 2 3 512
keyrel 1 2 +i 512
assert block[1,512,1024,511] == water
assert level[1,512,1024,511] == 255
assert cover[1,512,1024,511] == drawn:9
assert crosshair_k == 513
assert fk ~ 513.25 0.02
assert find_target_k 512
assert block_rel[0,0,-1] == stone
fill 0 0 512 1 1 514 water 128
at 1 2 512
aim 0 0 -0.5
assert level_rel[0,0,0] == 255
assert crosshair_k == none
menu type block[1,2,3,512]
log atblock 1 2 3 512
"""
    expected = source.replace("1024 513 #", "1024 1025 #").replace(",511]", ",1023]").replace("crosshair_k == 513", "crosshair_k == 1025").replace("fk ~ 513.25", "fk ~ 1025.25").replace("find_target_k 512", "find_target_k 1024")
    assert shift(source, 512) == expected
    assert shift(source.replace("\n", "\r\n"), 512) == expected.replace("\n", "\r\n")
    assert shift(expected, -512) == source
    assert shift("assert block[1,-1,+2,513] == air\nassert fk ~ 5.13e2 0.2\n", 512) == "assert block[1,-1,+2,1025] == air\nassert fk ~ 1025 0.2\n"
    print("shift_layers: 4 checks passed (syntax/prose, CRLF, inverse, signed coordinates/exponents)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", type=Path)
    parser.add_argument("--delta", type=int, default=512)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    game = Path(__file__).resolve().parents[1]
    paths = args.paths or [game / "scripts", game / "src"]
    files = sorted({f for path in paths for f in (path.rglob("*.play") if path.is_dir() else [path])})
    changed = 0
    for path in files:
        with path.open(encoding="utf-8", newline="") as stream:
            before = stream.read()
        after = shift(before, args.delta)
        if after != before:
            with path.open("w", encoding="utf-8", newline="") as stream:
                stream.write(after)
            changed += 1
            print(path)
    print(f"Shifted {changed}/{len(files)} files by {args.delta:+d} layers.")


if __name__ == "__main__":
    main()

#!/usr/bin/env bash
# Session-start gate: rebuild the play harness and run every asserting survey.
# Exits non-zero if the build fails or any suite has a failing assertion — so a
# regression is caught before hand-verification, not after.
#
#   viewer/scripts/verify.sh          # build + regenerate sweep + run all suites
#   SKIP_BUILD=1 viewer/scripts/verify.sh   # skip the rebuild (already current)
#
# Run from anywhere; it locates the repo root relative to itself.
set -u
cd "$(dirname "$0")/../.." || exit 2   # repo root
BIN=viewer/target/release/examples/play.exe

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  echo "building play harness (examples are NOT built by a bare cargo build)..."
  cargo build --release --example play --manifest-path viewer/Cargo.toml || exit 2
fi

# regenerate the data-driven sweep so it reflects the current planet + classes
if [ -f output/seed42_r8/planet_data.npz ]; then
  python viewer/scripts/gen_survey.py --per 16 || exit 2
fi

fail=0
for s in physics-regressions water-regressions lake-regressions flooded-caves invariant-survey auto-survey camera-controls; do
  printf "  %-22s " "$s"
  if "$BIN" "viewer/scripts/$s.play" >/dev/null 2>&1; then
    echo "PASS"
  else
    echo "FAIL (run: $BIN viewer/scripts/$s.play)"
    fail=1
  fi
done

if [ "$fail" = 0 ]; then
  echo "all suites green"
else
  echo "SURVEYS FAILED — a physics/geometry invariant regressed"
fi
exit "$fail"

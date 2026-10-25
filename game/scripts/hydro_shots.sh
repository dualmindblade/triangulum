#!/bin/sh
# M2-hydro evidence shots: find the sites (an ignored test writes one play
# script per site), then run each through the play harness starting at that
# site so it is lit by day (10:30 local solar time).
#
#   sh scripts/hydro_shots.sh [BAKE_DIR]      (from game/)
#
# Frames land in out/play/<site>/.
set -e
BAKE=${1:-C:/code/triangulum/game/assets/world/seed42}
cargo test --release hydro_places -- --ignored --nocapture
while read -r site lat lon; do
  cargo run --release -- --play "out/hydro-shots/$site.play" --bake "$BAKE" --at "$lat" "$lon" 50 --time 10:30
done < out/hydro-shots/sites.txt

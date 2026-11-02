#!/bin/bash
# In-VM, as root: generate 40 fixture JPEGs under <src>/2024/03 (owned by tester).
# Sizes vary from ~800 px to 4000 px; every 10th is a 4000x3000 noise image
# (well over 5 MB). Each image shows its number, and gets an EXIF
# DateTimeOriginal in March 2024.
#
#   make-fixtures.sh [/srv/photos]
set -euo pipefail
src=${1:-/srv/photos}
dir=$src/2024/03
mkdir -p "$dir"
colors=(steelblue firebrick seagreen darkorange purple teal goldenrod slategray)
for n in $(seq 1 40); do
  f=$dir/IMG_$((1000 + n)).jpg
  c1=${colors[$((n % 8))]}; c2=${colors[$(((n + 3) % 8))]}
  if (( n % 10 == 0 )); then
    w=4000; h=3000
    magick -size ${w}x${h} xc:"$c1" -attenuate 2 +noise Random \
      -gravity center -pointsize 900 -fill white -stroke black -strokewidth 20 -annotate 0 "$n" \
      -quality 95 "$f"
  else
    w=$(( 800 + (n * 397) % 3200 ))
    if (( n % 4 == 0 )); then h=$(( w * 4 / 3 )); else h=$(( w * 3 / 4 )); fi   # some portrait
    magick -size ${w}x${h} gradient:"$c1-$c2" \
      -gravity center -pointsize $(( (w < h ? w : h) / 3 )) -fill white -annotate 0 "$n" \
      -quality 88 "$f"
  fi
  exiftool -q -overwrite_original \
    -DateTimeOriginal="2024:03:$(printf %02d $(( (n - 1) % 28 + 1 ))) 12:$(printf %02d "$n"):00" "$f"
done
chown -R tester: "$src"
ls -l "$dir" | awk 'NR>1{n++; s+=$5; if($5>5*1024*1024) big++} END{printf "%d files, %.1f MB total, %d over 5 MB\n", n, s/1048576, big}'

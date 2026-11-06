#!/usr/bin/env bash
# The macOS app icon, from the project's own icon (packaging/icons/dev.hviid.ImmichMount.svg): the same drive and
# lens, unchanged, on the rounded square macOS 26 expects of an app icon (ARCHITECTURE.md, "The icon"). Writes every
# size the asset catalog needs into apple/App/Assets.xcassets/AppIcon.appiconset/, which is committed: the Mac has no
# SVG renderer. Runs where rsvg-convert is, such as the Fedora toolbox; run it again when the icon changes.
#
#   apple/make-icons.sh
set -euo pipefail
cd "$(dirname "$0")/.."
source_svg=packaging/icons/dev.hviid.ImmichMount.svg
set_dir=apple/App/Assets.xcassets/AppIcon.appiconset
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The drawing is 128 units, the drive 112 wide around (64, 66); on macOS's 1024 grid it sits in the 824 square at
# (100, 100), drawn 600 wide.
python3 - "$source_svg" "$work/macos.svg" <<'PYTHON'
import re, sys
source = open(sys.argv[1]).read()
inner = re.sub(r"^.*?<svg[^>]*>", "", source, count=1, flags=re.S)
inner = inner[: inner.rindex("</svg>")]
scale = 600 / 112
open(sys.argv[2], "w").write(f"""<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#eef3fa"/><stop offset="1" stop-color="#c3d1e6"/>
    </linearGradient>
    <filter id="lift" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="10" stdDeviation="14" flood-color="#000000" flood-opacity="0.28"/>
    </filter>
  </defs>
  <rect x="100" y="100" width="824" height="824" rx="185" fill="url(#tile)" filter="url(#lift)"/>
  <rect x="100.5" y="100.5" width="823" height="823" rx="184.5" fill="none" stroke="#000000" stroke-opacity="0.08"/>
  <g transform="translate({512 - 64 * scale:.2f} {512 - 66 * scale:.2f}) scale({scale:.4f})">{inner}</g>
</svg>
""")
PYTHON

mkdir -p "$set_dir"
images=""
for size in 16 32 128 256 512; do
  for scale in 1 2; do
    pixels=$((size * scale))
    suffix=""
    if [ "$scale" = 2 ]; then suffix=@2x; fi
    name="icon_${size}x${size}${suffix}.png"
    rsvg-convert -w "$pixels" -h "$pixels" "$work/macos.svg" -o "$set_dir/$name"
    images+="${images:+,}
    { \"idiom\" : \"mac\", \"scale\" : \"${scale}x\", \"size\" : \"${size}x${size}\", \"filename\" : \"$name\" }"
  done
done
printf '{\n  "images" : [%s\n  ],\n  "info" : { "author" : "xcode", "version" : 1 }\n}\n' "$images" >"$set_dir/Contents.json"
printf '{\n  "info" : { "author" : "xcode", "version" : 1 }\n}\n' >apple/App/Assets.xcassets/Contents.json
echo "wrote $set_dir"

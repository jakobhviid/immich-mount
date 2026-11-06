#!/usr/bin/env bash
# The macOS app icon, from the project's own icon (packaging/icons/dev.hviid.ImmichMount.svg): the same drive and
# lens, unchanged, on the rounded square macOS 26 expects of an app icon (ARCHITECTURE.md, "The icon"). Writes every
# size the asset catalog needs into apple/App/Assets.xcassets/AppIcon.appiconset/, which is committed: the Mac has no
# SVG renderer. Also writes Finder's sidebar symbol (apple/make-symbol.py) into SidebarSymbol.symbolset/. Runs where
# rsvg-convert is, such as the Fedora toolbox; run it again when the icon changes.
#
#   apple/make-icons.sh
set -euo pipefail
cd "$(dirname "$0")/.."
source_svg=packaging/icons/dev.hviid.ImmichMount.svg
set_dir=apple/App/Assets.xcassets/AppIcon.appiconset
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The drive is the whole icon on macOS: its face and base fill the rounded square on Apple's 1024 grid (824 square at
# 100, 100), the base along the bottom edge, and the lens, the light and the gradients are the drawing's own, scaled.
python3 - "$source_svg" "$work/macos.svg" <<'PYTHON'
import re, sys
source = open(sys.argv[1]).read()
defs = re.search(r"<defs>(.*?)</defs>", source, flags=re.S).group(1)
lens = source[source.index('<circle cx="64" cy="58" r="30"'): source.rindex("</svg>")]
light = re.search(r'<rect x="96"[^>]*fill="(#[0-9A-Fa-f]+)"', source).group(1)
left, top, size, radius = 100, 100, 824, 185
base = 168  # the base's height, a fifth of the square, as the drawing's base is of the drive
face_bottom = top + size - base
lip = 88  # the face's lower corners, rounded as the drawing's face is, so the base shows at the sides
lens_radius = 255
scale = lens_radius / 30
cx, cy = left + size / 2, top + (size - base) / 2
face = (
    f"M{left} {top + radius} A{radius} {radius} 0 0 1 {left + radius} {top} L{left + size - radius} {top} "
    f"A{radius} {radius} 0 0 1 {left + size} {top + radius} L{left + size} {face_bottom - lip} "
    f"A{lip} {lip} 0 0 1 {left + size - lip} {face_bottom} L{left + lip} {face_bottom} "
    f"A{lip} {lip} 0 0 1 {left} {face_bottom - lip} Z"
)
open(sys.argv[2], "w").write(f"""<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>{defs}
    <clipPath id="tile"><rect x="{left}" y="{top}" width="{size}" height="{size}" rx="{radius}"/></clipPath>
    <filter id="lift" x="-10%" y="-10%" width="120%" height="125%">
      <feDropShadow dx="0" dy="10" stdDeviation="14" flood-color="#000000" flood-opacity="0.3"/>
    </filter>
  </defs>
  <g filter="url(#lift)">
    <rect x="{left}" y="{top}" width="{size}" height="{size}" rx="{radius}" fill="url(#base)"/>
  </g>
  <g clip-path="url(#tile)">
    <path d="{face}" fill="url(#face)"/>
    <rect x="{left + size - 110 - 96}" y="{face_bottom + base / 2 - 12}" width="96" height="24" rx="12" fill="{light}"/>
  </g>
  <rect x="{left + 1}" y="{top + 1}" width="{size - 2}" height="{size - 2}" rx="{radius - 1}" fill="none" stroke="#000000" stroke-opacity="0.12" stroke-width="2"/>
  <g transform="translate({cx - 64 * scale:.2f} {cy - 58 * scale:.2f}) scale({scale:.4f})">{lens}</g>
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

# The drawing itself, as the README and the GNOME app's pages show it, for the app window's header: Xcode keeps an
# SVG in an image set as a vector.
drive_dir=apple/App/Assets.xcassets/Drive.imageset
mkdir -p "$drive_dir"
cp "$source_svg" "$drive_dir/Drive.svg"
cat >"$drive_dir/Contents.json" <<'JSON'
{
  "images" : [ { "filename" : "Drive.svg", "idiom" : "universal" } ],
  "info" : { "author" : "xcode", "version" : 1 },
  "properties" : { "preserves-vector-representation" : true }
}
JSON

symbol_dir=apple/App/Assets.xcassets/SidebarSymbol.symbolset
mkdir -p "$symbol_dir"
python3 apple/make-symbol.py "$symbol_dir/SidebarSymbol.svg"
cat >"$symbol_dir/Contents.json" <<'JSON'
{
  "info" : { "author" : "xcode", "version" : 1 },
  "properties" : { "symbol-rendering-intent" : "template" },
  "symbols" : [ { "filename" : "SidebarSymbol.svg", "idiom" : "universal" } ]
}
JSON
echo "wrote $set_dir, $drive_dir and $symbol_dir"

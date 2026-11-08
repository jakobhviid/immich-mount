#!/usr/bin/env bash
# The macOS app icon, from the project's own icon (packaging/icons/dev.hviid.ImmichMount.svg): the same drive and
# lens, unchanged, on the rounded square macOS 26 expects of an app icon (ARCHITECTURE.md, "The icon"). Writes every
# size the asset catalog needs into apple/App/Assets.xcassets/AppIcon.appiconset/, which is committed: the Mac has no
# SVG renderer. Also writes Finder's sidebar symbol (apple/make-symbol.py) and the menu bar icon
# (apple/make-menu-bar-icon.py). Runs where rsvg-convert is, such as the Fedora toolbox; run it again when the icon
# changes.
#
#   apple/make-icons.sh
#   apple/make-icons.sh dev    the development build's icon (blue, DEV on its base) into AppIconDev.appiconset only
set -euo pipefail
cd "$(dirname "$0")/.."
source_svg=packaging/icons/dev.hviid.ImmichMount.svg
set_dir=apple/App/Assets.xcassets/AppIcon.appiconset
variant=${1:-}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
if [ "$variant" = dev ]; then
  set_dir=apple/App/Assets.xcassets/AppIconDev.appiconset
fi

# The drive is the whole icon on macOS: its face and base fill the rounded square on Apple's 1024 grid (824 square at
# 100, 100), the base along the bottom edge, and the lens, the light and the gradients are the drawing's own, scaled.
python3 - "$source_svg" "$work/macos.svg" "$variant" <<'PYTHON'
import re, sys


def dev_badge(x, centre_y, height):
    """DEV in white strokes, letters drawn rather than set in a font, so every SVG renderer draws them alike."""
    scale = height / 100
    letters = "M0 0 V100 H38 A50 50 0 0 0 38 0 Z M150 0 H100 V100 H150 M100 50 H140 M175 0 L205 100 L235 0"
    return (f'<g transform="translate({x:.1f} {centre_y - height / 2:.1f}) scale({scale:.4f})" fill="none" '
            f'stroke="#ffffff" stroke-width="16" stroke-linejoin="round" stroke-linecap="round">'
            f'<path d="{letters}"/></g>')


source = open(sys.argv[1]).read()
dev = sys.argv[3] == "dev"
if dev:
    # The development build is blue, as Mac development builds tend to be, so the two can be told apart at a glance.
    source = re.sub(r'(<linearGradient id="face".*?</linearGradient>)',
                    lambda match: match.group(1).replace("#ffffff", "#8ec5ff").replace("#deddda", "#3f8cf0"),
                    source, flags=re.S)
    source = re.sub(r'(<linearGradient id="base".*?</linearGradient>)',
                    lambda match: match.group(1).replace("#c0bfbc", "#2563c9").replace("#9a9996", "#173f8a"),
                    source, flags=re.S)
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
{dev_badge(left + 92, face_bottom + base / 2, 74) if dev else ""}
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
if [ "$variant" = dev ]; then
  # Finder's sidebar symbol, the same shape asking for blue (apple/make-symbol.py --blue).
  dev_symbol_dir=apple/App/Assets.xcassets/SidebarSymbolDev.symbolset
  mkdir -p "$dev_symbol_dir"
  python3 apple/make-symbol.py "$dev_symbol_dir/SidebarSymbolDev.svg" --square --blue
  cat >"$dev_symbol_dir/Contents.json" <<'JSON'
{
  "info" : { "author" : "xcode", "version" : 1 },
  "symbols" : [ { "filename" : "SidebarSymbolDev.svg", "idiom" : "universal" } ]
}
JSON
  echo "wrote $set_dir and $dev_symbol_dir"
  exit 0
fi

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
python3 apple/make-symbol.py "$symbol_dir/SidebarSymbol.svg" --square
cat >"$symbol_dir/Contents.json" <<'JSON'
{
  "info" : { "author" : "xcode", "version" : 1 },
  "properties" : { "symbol-rendering-intent" : "template" },
  "symbols" : [ { "filename" : "SidebarSymbol.svg", "idiom" : "universal" } ]
}
JSON

python3 apple/make-menu-bar-icon.py
echo "wrote $set_dir, $drive_dir and $symbol_dir"

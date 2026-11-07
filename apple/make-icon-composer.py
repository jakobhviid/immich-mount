#!/usr/bin/env python3
"""The macOS app icon as an Icon Composer document (apple/App/AppIcon.icon), from the project's own icon
(packaging/icons/dev.hviid.ImmichMount.svg): the same drive and lens, drawn as layers the system renders with its
own shape, lighting and glass, in light, dark and tinted appearances (ARCHITECTURE.md, "The icon"). macOS 26 and
later show it; AppIcon.appiconset (apple/make-icons.sh) stays for macOS 14 and 15.

The drive is the whole icon: its face is the document's background, the base runs along the bottom with its light,
and the lens is a group of its own, so the system's glass and lighting play on it. The assets are SVG, which Icon
Composer reads as they are. Run it again when the icon changes:

    apple/make-icon-composer.py
"""

import json
import re
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "packaging" / "icons" / "dev.hviid.ImmichMount.svg"
TARGET = ROOT / "apple" / "App" / "AppIcon.icon"

CANVAS = 1024
# The base is a fifth of the drive's height, as in the drawing, and the face's lower corners are rounded, so the
# base shows at the sides.
BASE = 205
LIP = 110
LENS_RADIUS = 295


def svg(body, defs=""):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{CANVAS}" height="{CANVAS}" '
            f'viewBox="0 0 {CANVAS} {CANVAS}">\n<defs>{defs}</defs>\n{body}\n</svg>\n')


def base_layer(top, bottom):
    """The base along the bottom edge, and the corners the face's rounded lower edge leaves."""
    face_bottom = CANVAS - BASE
    # The corners are drawn on their own, so no edge of the base runs along the face (it would show as a seam), and
    # the band reaches a point under them, so none runs between them either.
    band = f"M0 {face_bottom - 1} H{CANVAS} V{CANVAS} H0 Z"
    left = f"M0 {face_bottom - LIP} L0 {face_bottom} L{LIP} {face_bottom} A{LIP} {LIP} 0 0 1 0 {face_bottom - LIP} Z"
    right = (f"M{CANVAS} {face_bottom - LIP} A{LIP} {LIP} 0 0 1 {CANVAS - LIP} {face_bottom} "
             f"L{CANVAS} {face_bottom} Z")
    defs = (f'<linearGradient id="base" x1="0" y1="0" x2="0" y2="1">'
            f'<stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient>')
    return svg(f'<path d="{band} {left} {right}" fill="url(#base)"/>', defs)


def light_layer(colour):
    """The drive's light, on the base at the right."""
    y = CANVAS - BASE / 2 - 14
    return svg(f'<rect x="{CANVAS - 130 - 120}" y="{y:.0f}" width="120" height="28" rx="14" fill="{colour}"/>')


def lens_layers(source):
    """The lens, from the drawing: its rim and ring, and its glass with the highlights, each scaled onto the canvas."""
    defs = re.search(r"<defs>(.*?)</defs>", source, flags=re.S).group(1)
    glass_gradient = re.search(r'<radialGradient id="glass".*?</radialGradient>', defs, flags=re.S).group(0)
    lens = source[source.index('<circle cx="64" cy="58" r="30"'): source.rindex("</svg>")].strip()
    glass_start = lens.index('<circle cx="64" cy="58" r="18.00"')
    ring, glass = lens[:glass_start], lens[glass_start:]
    scale = LENS_RADIUS / 30
    centre_x, centre_y = CANVAS / 2, (CANVAS - BASE) / 2
    transform = f'translate({centre_x - 64 * scale:.2f} {centre_y - 58 * scale:.2f}) scale({scale:.4f})'
    return (svg(f'<g transform="{transform}">{ring}</g>'),
            svg(f'<g transform="{transform}">{glass}</g>', glass_gradient))


def main():
    source = SOURCE.read_text()
    ring, glass = lens_layers(source)
    assets = {
        "lens-glass.svg": glass,
        "lens-ring.svg": ring,
        "light.svg": light_layer("#18C249"),
        "base.svg": base_layer("#c0bfbc", "#9a9996"),
        "base-dark.svg": base_layer("#3a393c", "#242326"),
    }
    document = {
        "fill-specializations": [
            {"value": {"linear-gradient": ["srgb:1.00000,1.00000,1.00000,1.00000",
                                           "srgb:0.87059,0.86667,0.85490,1.00000"],
                       "orientation": {"start": {"x": 0.5, "y": 0}, "stop": {"x": 0.5, "y": 0.8}}}},
            {"appearance": "dark",
             "value": {"linear-gradient": ["srgb:0.32157,0.31765,0.33333,1.00000",
                                           "srgb:0.20000,0.19608,0.21176,1.00000"],
                       "orientation": {"start": {"x": 0.5, "y": 0}, "stop": {"x": 0.5, "y": 0.8}}}},
        ],
        "groups": [
            {
                "name": "Lens",
                "layers": [
                    {"name": "Glass", "image-name": "lens-glass.svg", "glass": True},
                    {"name": "Ring", "image-name": "lens-ring.svg", "glass": False},
                ],
                "lighting": "individual",
                "shadow": {"kind": "neutral", "opacity": 0.5},
                "translucency": {"enabled": False, "value": 0.5},
            },
            {
                "name": "Base",
                "layers": [
                    {"name": "Light", "image-name": "light.svg", "glass": False},
                    {"name": "Base", "image-name-specializations": [
                        {"value": "base.svg"},
                        {"appearance": "dark", "value": "base-dark.svg"},
                    ], "glass": False},
                ],
                "shadow": {"kind": "none", "opacity": 0.5},
                "translucency": {"enabled": False, "value": 0.5},
            },
        ],
        "supported-platforms": {"squares": ["macOS"]},
    }
    if TARGET.exists():
        shutil.rmtree(TARGET)
    (TARGET / "Assets").mkdir(parents=True)
    for name, text in assets.items():
        (TARGET / "Assets" / name).write_text(text)
    (TARGET / "icon.json").write_text(json.dumps(document, indent=2) + "\n")
    print(f"wrote {TARGET.relative_to(ROOT)}")


if __name__ == "__main__":
    main()

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
import sys
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


def badge_layer():
    """DEV in white strokes on the base at the left, opposite the light: letters drawn rather than set in a font,
    so every SVG renderer draws them alike."""
    height = 84
    scale = height / 100
    y = CANVAS - BASE / 2 - height / 2
    letters = "M0 0 V100 H38 A50 50 0 0 0 38 0 Z M150 0 H100 V100 H150 M100 50 H140 M175 0 L205 100 L235 0"
    return svg(f'<g transform="translate(130 {y:.1f}) scale({scale:.4f})" fill="none" stroke="#ffffff" '
               f'stroke-width="16" stroke-linejoin="round" stroke-linecap="round"><path d="{letters}"/></g>')


def srgb(hex_colour):
    red, green, blue = (int(hex_colour[index:index + 2], 16) / 255 for index in (1, 3, 5))
    return f"srgb:{red:.5f},{green:.5f},{blue:.5f},1.00000"


def main():
    # `--dev`: the development build's icon, blue with DEV on its base, so the two can be told apart at a glance.
    dev = "--dev" in sys.argv[1:]
    target = TARGET.with_name("AppIconDev.icon") if dev else TARGET
    source = SOURCE.read_text()
    ring, glass = lens_layers(source)
    face, face_dark = (("#8ec5ff", "#3f8cf0"), ("#1f4f99", "#14356b")) if dev else \
        (("#ffffff", "#deddda"), ("#525155", "#333236"))
    assets = {
        "lens-glass.svg": glass,
        "lens-ring.svg": ring,
        "light.svg": light_layer("#18C249"),
        "base.svg": base_layer(*(("#2563c9", "#173f8a") if dev else ("#c0bfbc", "#9a9996"))),
        "base-dark.svg": base_layer(*(("#123068", "#0b1e45") if dev else ("#3a393c", "#242326"))),
    }
    if dev:
        assets["badge.svg"] = badge_layer()
    document = {
        "fill-specializations": [
            {"value": {"linear-gradient": [srgb(face[0]), srgb(face[1])],
                       "orientation": {"start": {"x": 0.5, "y": 0}, "stop": {"x": 0.5, "y": 0.8}}}},
            {"appearance": "dark",
             "value": {"linear-gradient": [srgb(face_dark[0]), srgb(face_dark[1])],
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
                "layers": ([{"name": "Badge", "image-name": "badge.svg", "glass": False}] if dev else []) + [
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
    if target.exists():
        shutil.rmtree(target)
    (target / "Assets").mkdir(parents=True)
    for name, text in assets.items():
        (target / "Assets" / name).write_text(text)
    (target / "icon.json").write_text(json.dumps(document, indent=2) + "\n")
    print(f"wrote {target.relative_to(ROOT)}")


if __name__ == "__main__":
    main()

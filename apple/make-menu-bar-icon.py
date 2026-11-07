#!/usr/bin/env python3
"""The menu bar icon: the macOS app icon as one solid shape, as menu bar icons are drawn (ARCHITECTURE.md, "The
icon"). The drive fills a rounded square, and its details are cut out of it: the lens ring in five segments, as the
icon's ring carries five colours, the lens's centre, the line where the face meets the base, and the base's light.

Writes a template image at the size menu bar icons have, 18 points with a 16-point glyph, as a vector the asset
catalog keeps; macOS tints it for the menu bar's look. Run it again when the icon changes:

    apple/make-menu-bar-icon.py
"""

import json
import math
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / "apple" / "App" / "Assets.xcassets" / "MenuBarIcon.imageset"

BOX = 18.0
MARGIN = 1.0
SIDE = BOX - 2 * MARGIN
RADIUS = 4.0  # the rounded square's corners, close to the app icon's at this size
BASE = 3.4  # the base's height, a fifth of the drive, as on the icon
GAP = 0.9  # the width of what is cut out: the base's line and the ring's gaps


def fmt(value):
    return f"{value:.3f}".rstrip("0").rstrip(".")


def rounded_square():
    left, top, right, bottom, r = MARGIN, MARGIN, MARGIN + SIDE, MARGIN + SIDE, RADIUS
    return (f"M{fmt(left + r)} {fmt(top)} H{fmt(right - r)} A{fmt(r)} {fmt(r)} 0 0 1 {fmt(right)} {fmt(top + r)} "
            f"V{fmt(bottom - r)} A{fmt(r)} {fmt(r)} 0 0 1 {fmt(right - r)} {fmt(bottom)} H{fmt(left + r)} "
            f"A{fmt(r)} {fmt(r)} 0 0 1 {fmt(left)} {fmt(bottom - r)} V{fmt(top + r)} "
            f"A{fmt(r)} {fmt(r)} 0 0 1 {fmt(left + r)} {fmt(top)} Z")


def point(cx, cy, r, angle):
    return cx + r * math.cos(math.radians(angle)), cy - r * math.sin(math.radians(angle))


def ring_segment(cx, cy, outer, inner, start, end):
    """One segment of the lens ring, from `start` down to `end` degrees (clockwise on screen), traced
    counter-clockwise on screen so it cuts a hole: back along the outer edge, then forward along the inner one."""
    (x0, y0), (x1, y1) = point(cx, cy, outer, end), point(cx, cy, outer, start)
    (x2, y2), (x3, y3) = point(cx, cy, inner, start), point(cx, cy, inner, end)
    return (f"M{fmt(x0)} {fmt(y0)} A{fmt(outer)} {fmt(outer)} 0 0 0 {fmt(x1)} {fmt(y1)} "
            f"L{fmt(x2)} {fmt(y2)} A{fmt(inner)} {fmt(inner)} 0 0 1 {fmt(x3)} {fmt(y3)} Z")


def circle(cx, cy, r):
    """A circle, counter-clockwise on screen so it cuts a hole."""
    return (f"M{fmt(cx + r)} {fmt(cy)} A{fmt(r)} {fmt(r)} 0 0 0 {fmt(cx - r)} {fmt(cy)} "
            f"A{fmt(r)} {fmt(r)} 0 0 0 {fmt(cx + r)} {fmt(cy)} Z")


def bar(left, top, width, height):
    """A rounded bar, counter-clockwise on screen so it cuts a hole."""
    r = height / 2
    right, bottom = left + width, top + height
    return (f"M{fmt(right - r)} {fmt(top)} H{fmt(left + r)} A{fmt(r)} {fmt(r)} 0 0 0 {fmt(left + r)} {fmt(bottom)} "
            f"H{fmt(right - r)} A{fmt(r)} {fmt(r)} 0 0 0 {fmt(right - r)} {fmt(top)} Z")


def icon():
    split = MARGIN + SIDE - BASE
    cuts = [bar(MARGIN + 0.9, split - GAP / 2, SIDE - 1.8, GAP)]
    # The light, on the base at the right, as on the icon.
    light_y = split + GAP / 2 + (BASE - GAP / 2) / 2 - 0.45
    cuts.append(bar(MARGIN + SIDE - 2.2 - 2.4, light_y, 2.4, 0.9))
    # The lens, centred in the face, its ring in five segments and its centre cut out.
    cx, cy = BOX / 2, (MARGIN + split - GAP / 2) / 2
    outer, inner = 5.1, 3.45
    gap_degrees = math.degrees(GAP / ((outer + inner) / 2))
    for index in range(5):
        start = 90 - index * 72 - gap_degrees / 2
        cuts.append(ring_segment(cx, cy, outer, inner, start, start - 72 + gap_degrees))
    cuts.append(circle(cx, cy, 1.35))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{fmt(BOX)}" height="{fmt(BOX)}" '
            f'viewBox="0 0 {fmt(BOX)} {fmt(BOX)}">\n'
            f'  <!-- Immich Mount\'s menu bar icon, written by apple/make-menu-bar-icon.py. -->\n'
            f'  <path fill="#000000" fill-rule="nonzero" d="{rounded_square()} {" ".join(cuts)}"/>\n</svg>\n')


def main():
    TARGET.mkdir(parents=True, exist_ok=True)
    (TARGET / "MenuBarIcon.svg").write_text(icon())
    contents = {
        "images": [{"filename": "MenuBarIcon.svg", "idiom": "universal"}],
        "info": {"author": "xcode", "version": 1},
        "properties": {"preserves-vector-representation": True, "template-rendering-intent": "template"},
    }
    (TARGET / "Contents.json").write_text(json.dumps(contents, indent=2) + "\n")
    print(f"wrote {TARGET.relative_to(ROOT)}")


if __name__ == "__main__":
    main()

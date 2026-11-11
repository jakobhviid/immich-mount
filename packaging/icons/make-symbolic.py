#!/usr/bin/env python3
"""The symbolic icon on Linux: the drive filling a rounded square as one solid shape, with the lens ring in five
segments, the lens's centre, the line where the face meets the base and the base's light cut out (DESIGN.md, "The
icon family"). The Mac's menu bar icon, on Linux: a glyph that fills its square as its neighbours in a top bar or a
tray do (the wide drive drew smaller than they did). It serves the tray, the notifications and Files' sidebar, and
GTK and KDE both recolour it to the text colour.

    packaging/icons/make-symbolic.py
"""

import math
from pathlib import Path

TARGET = Path(__file__).resolve().parent / "dev.hviid.ImmichMount-symbolic.svg"

# The drive on a 16-pixel grid, filling it as the Mac's menu bar icon fills its 16 points: 15 by 15, its corners
# rounded as the Mac's are at that size.
LEFT, TOP, RIGHT, BOTTOM, RADIUS = 0.5, 0.5, 15.5, 15.5, 3.75
BASE = 3.2  # the base's height, a fifth of the drive, as on the Mac's
GAP = 0.85  # what is cut out: the base's line and the ring's gaps
# The lens in the middle of the face, its ring as large on the face as the app icon's.
CENTRE = (8.0, (TOP + BOTTOM - BASE - GAP / 2) / 2)
RING_OUTER, RING_INNER, DOT = 4.6, 2.9, 1.15
LIGHT = (11.3, BOTTOM - 2.0, 2.4, 0.9)  # the base's light: left, top, width, height


def fmt(value):
    return f"{value:.3f}".rstrip("0").rstrip(".")


def rounded(left, top, right, bottom, r):
    return (f"M{fmt(left + r)} {fmt(top)} H{fmt(right - r)} A{fmt(r)} {fmt(r)} 0 0 1 {fmt(right)} {fmt(top + r)} "
            f"V{fmt(bottom - r)} A{fmt(r)} {fmt(r)} 0 0 1 {fmt(right - r)} {fmt(bottom)} H{fmt(left + r)} "
            f"A{fmt(r)} {fmt(r)} 0 0 1 {fmt(left)} {fmt(bottom - r)} V{fmt(top + r)} "
            f"A{fmt(r)} {fmt(r)} 0 0 1 {fmt(left + r)} {fmt(top)} Z")


def point(r, angle):
    return CENTRE[0] + r * math.cos(math.radians(angle)), CENTRE[1] + r * math.sin(math.radians(angle))


def segment(start, end):
    """One of the ring's five segments, between two angles (degrees, clockwise from three o'clock)."""
    (ox1, oy1), (ox2, oy2) = point(RING_OUTER, start), point(RING_OUTER, end)
    (ix2, iy2), (ix1, iy1) = point(RING_INNER, end), point(RING_INNER, start)
    return (f"M{fmt(ox1)} {fmt(oy1)} A{fmt(RING_OUTER)} {fmt(RING_OUTER)} 0 0 1 {fmt(ox2)} {fmt(oy2)} "
            f"L{fmt(ix2)} {fmt(iy2)} A{fmt(RING_INNER)} {fmt(RING_INNER)} 0 0 0 {fmt(ix1)} {fmt(iy1)} Z")


def main():
    # The gaps are as wide as the base's line, measured along the ring's middle.
    gap = math.degrees(GAP / ((RING_OUTER + RING_INNER) / 2))
    segments = [segment(-90 + gap / 2 + 72 * index, -90 - gap / 2 + 72 * (index + 1)) for index in range(5)]
    cx, cy = CENTRE
    dot = (f"M{fmt(cx - DOT)} {fmt(cy)} A{fmt(DOT)} {fmt(DOT)} 0 1 0 {fmt(cx + DOT)} {fmt(cy)} "
           f"A{fmt(DOT)} {fmt(DOT)} 0 1 0 {fmt(cx - DOT)} {fmt(cy)} Z")
    line_top = BOTTOM - BASE - GAP / 2
    line = f"M{fmt(LEFT)} {fmt(line_top)} H{fmt(RIGHT)} V{fmt(line_top + GAP)} H{fmt(LEFT)} Z"
    left, top, width, height = LIGHT
    light = rounded(left, top, left + width, top + height, height / 2)
    shape = " ".join([rounded(LEFT, TOP, RIGHT, BOTTOM, RADIUS), *segments, dot, line, light])
    TARGET.write_text(f"""<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">
  <!-- Immich Mount, symbolic, written by packaging/icons/make-symbolic.py: the drive in a rounded square, its lens
       ring in five segments. GTK and KDE both recolour it to the text colour. -->
  <style id="current-color-scheme">.ColorScheme-Text {{ color: #2e3436; }}</style>
  <path class="ColorScheme-Text" fill="currentColor" fill-rule="evenodd" d="{shape}"/>
</svg>
""")
    print(f"wrote {TARGET}")


if __name__ == "__main__":
    main()

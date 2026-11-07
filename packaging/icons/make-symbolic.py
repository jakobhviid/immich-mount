#!/usr/bin/env python3
"""The symbolic icon on Linux: the app icon's wide drive as one solid shape, with the lens ring in five segments,
the lens's centre, the line where the face meets the base and the base's light cut out (DESIGN.md, "The icon
family"). The same language as the Mac's menu bar icon, in Linux's own shape. It serves the notifications and the
tray, and GTK and KDE both recolour it to the text colour.

    packaging/icons/make-symbolic.py
"""

import math
from pathlib import Path

TARGET = Path(__file__).resolve().parent / "dev.hviid.ImmichMount-symbolic.svg"

# The drive on a 16-pixel grid, as wide as the app icon's (112 by 88 of 128): 15 by 11.8, its corners rounded.
LEFT, TOP, RIGHT, BOTTOM, RADIUS = 0.5, 2.1, 15.5, 13.9, 2.4
BASE = 2.7  # the base's height, under the face
GAP = 0.8  # what is cut out: the base's line and the ring's gaps
# The lens in the middle of the face, as on the app icon, its ring 0.42 of the face's height across.
CENTRE = (8.0, (TOP + BOTTOM - BASE - GAP / 2) / 2)
RING_OUTER, RING_INNER, DOT = 3.75, 2.35, 0.95
LIGHT = (11.6, BOTTOM - 1.45, 2.0, 0.8)  # the base's light: left, top, width, height


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
  <!-- Immich Mount, symbolic, written by packaging/icons/make-symbolic.py: the wide drive, its lens ring in five
       segments. GTK and KDE both recolour it to the text colour. -->
  <style id="current-color-scheme">.ColorScheme-Text {{ color: #2e3436; }}</style>
  <path class="ColorScheme-Text" fill="currentColor" fill-rule="evenodd" d="{shape}"/>
</svg>
""")
    print(f"wrote {TARGET}")


if __name__ == "__main__":
    main()

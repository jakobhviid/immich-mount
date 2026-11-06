#!/usr/bin/env python3
"""The Finder sidebar symbol: the app icon's drive and lens in outline (ARCHITECTURE.md, "The icon").

Writes an SF Symbols template (v6.0, "variable"): three source glyphs, Ultralight, Regular and Black at the small
scale, from which macOS interpolates every weight and scale. They are made from one geometry with three stroke
widths, so their paths match command for command, as interpolation needs; only lines and cubic curves are used.
Glyph coordinates are Apple's: x from the left margin, y up from the baseline (negative), cap height 70.459.

    apple/make-symbol.py <output.svg>
"""
import math
import sys

CAP = 70.459
SIDE = 4.0  # side bearing on each side of the glyph
# Template columns (the centres Apple's template gives each weight) and stroke widths.
WEIGHTS = {"Ultralight-S": (559.711, 2.2), "Regular-S": (1449.84, 6.6), "Black-S": (2933.4, 11.6)}
KAPPA = 0.5522847498


def fmt(value):
    return f"{value:.3f}".rstrip("0").rstrip(".")


def arc(cx, cy, r, start, end, move=False):
    """A circular arc as cubic curves, in pieces of at most 90 degrees (angles in degrees, y up)."""
    pieces = max(1, math.ceil(abs(end - start) / 90))
    step = (end - start) / pieces
    k = 4 / 3 * math.tan(math.radians(step) / 4)
    point = lambda angle: (cx + r * math.cos(math.radians(angle)), cy - r * math.sin(math.radians(angle)))
    commands = []
    x0, y0 = point(start)
    if move:
        commands.append(f"M{fmt(x0)} {fmt(y0)}")
    for index in range(pieces):
        a0, a1 = start + index * step, start + (index + 1) * step
        x0, y0 = point(a0)
        x3, y3 = point(a1)
        t0 = (-math.sin(math.radians(a0)), -math.cos(math.radians(a0)))
        t1 = (-math.sin(math.radians(a1)), -math.cos(math.radians(a1)))
        commands.append(
            f"C{fmt(x0 + k * r * t0[0])} {fmt(y0 + k * r * t0[1])} {fmt(x3 - k * r * t1[0])} {fmt(y3 - k * r * t1[1])} "
            f"{fmt(x3)} {fmt(y3)}"
        )
    return commands


def rounded_rect(left, top, right, bottom, radius, clockwise):
    """A closed rounded rectangle; `top` is the smaller y (higher up)."""
    corners = [
        (right - radius, top + radius, 90, 0),
        (right - radius, bottom - radius, 0, -90),
        (left + radius, bottom - radius, -90, -180),
        (left + radius, top + radius, 180, 90),
    ]
    commands = [f"M{fmt(left + radius)} {fmt(top)}"]
    for cx, cy, a0, a1 in corners:
        x0, y0 = cx + radius * math.cos(math.radians(a0)), cy - radius * math.sin(math.radians(a0))
        commands.append(f"L{fmt(x0)} {fmt(y0)}")
        commands += arc(cx, cy, radius, a0, a1)
    commands.append("Z")
    path = " ".join(commands)
    return path if clockwise else reverse(left, top, right, bottom, radius)


def reverse(left, top, right, bottom, radius):
    corners = [
        (left + radius, top + radius, 90, 180),
        (left + radius, bottom - radius, 180, 270),
        (right - radius, bottom - radius, 270, 360),
        (right - radius, top + radius, 0, 90),
    ]
    commands = [f"M{fmt(right - radius)} {fmt(top)}"]
    for cx, cy, a0, a1 in corners:
        x0, y0 = cx + radius * math.cos(math.radians(a0)), cy - radius * math.sin(math.radians(a0))
        commands.append(f"L{fmt(x0)} {fmt(y0)}")
        commands += arc(cx, cy, radius, a0, a1)
    commands.append("Z")
    return " ".join(commands)


def glyph(width):
    """The glyph at one stroke width, and its advance. Proportions follow the width, so the heavy weights keep a base
    and a ring with an inside; the commands stay the same at every width."""
    w = width
    body_w, body_h = 104.0 + w * 0.5, 70.0
    left, right = SIDE, SIDE + body_w
    top, bottom = -CAP / 2 - body_h / 2, -CAP / 2 + body_h / 2
    radius = 16.0
    paths = [rounded_rect(left, top, right, bottom, radius, True), rounded_rect(left + w, top + w, right - w, bottom - w, radius - w, False)]
    # Where the face meets the base, as on the icon's drive, and the base's light as a dot.
    line = w * 0.6
    split = bottom - (12.0 + w * 0.9)
    paths.append(rounded_rect(left + w / 2, split - line / 2, right - w / 2, split + line / 2, min(line / 2, 1.0), True))
    light = min(max(w * 0.42, 1.8), (bottom - w - split - line / 2) * 0.45)
    light_y = (split + line / 2 + bottom - w) / 2
    paths.append(" ".join(arc(right - w - 8 - light, light_y, light, 90, -270, move=True) + ["Z"]))
    # The lens: a ring in five segments, as the icon's ring carries five colours, and the glass as a dot.
    cx, cy = (left + right) / 2, (top + w + split - line / 2) / 2
    outer = (split - line / 2 - top - w) / 2 - 1.5
    inner = outer - min(w * 0.95, outer * 0.5)
    gap = 9.0 + w * 0.4
    for index in range(5):
        a0 = 90 - index * 72 - gap / 2
        a1 = a0 - 72 + gap
        ring = arc(cx, cy, outer, a0, a1, move=True)
        x, y = cx + inner * math.cos(math.radians(a1)), cy - inner * math.sin(math.radians(a1))
        ring.append(f"L{fmt(x)} {fmt(y)}")
        ring += arc(cx, cy, inner, a1, a0)
        ring.append("Z")
        paths.append(" ".join(ring))
    paths.append(" ".join(arc(cx, cy, inner * 0.38, 90, -270, move=True) + ["Z"]))
    return paths, body_w + 2 * SIDE


def template():
    lines = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<!-- Immich Mount\'s Finder sidebar symbol, written by apple/make-symbol.py: the drive and its lens in outline. -->',
        '<svg version="1.1" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 3300 2200">',
        ' <g id="Notes">',
        '  <text id="template-version" style="stroke:none;fill:black;font-family:sans-serif;font-size:13;text-anchor:end;" transform="matrix(1 0 0 1 3036 1933)">Template v.6.0</text>',
        '  <text id="descriptive-name" style="stroke:none;fill:black;font-family:sans-serif;font-size:13;text-anchor:end;" transform="matrix(1 0 0 1 3036 1969)">Immich Mount</text>',
        ' </g>',
        ' <g id="Guides">',
    ]
    for scale, baseline in (("S", 696), ("M", 1126), ("L", 1556)):
        guide = 'style="fill:none;stroke:#27AAE1;opacity:1;stroke-width:0.5;"'
        lines.append(f'  <line id="Baseline-{scale}" {guide} x1="263" x2="3036" y1="{baseline}" y2="{baseline}"/>')
        lines.append(f'  <line id="Capline-{scale}" {guide} x1="263" x2="3036" y1="{fmt(baseline - CAP)}" y2="{fmt(baseline - CAP)}"/>')
    glyphs = []
    for name, (centre, width) in WEIGHTS.items():
        paths, advance = glyph(width)
        margin_left = centre - advance / 2
        margin = 'style="fill:none;stroke:#00AEEF;stroke-width:0.5;opacity:1.0;"'
        lines.append(f'  <line id="left-margin-{name}" {margin} x1="{fmt(margin_left)}" x2="{fmt(margin_left)}" y1="600.785" y2="720.121"/>')
        lines.append(f'  <line id="right-margin-{name}" {margin} x1="{fmt(margin_left + advance)}" x2="{fmt(margin_left + advance)}" y1="600.785" y2="720.121"/>')
        glyphs.append(f'  <g id="{name}" transform="matrix(1 0 0 1 {fmt(margin_left)} 696)">')
        glyphs.append(f'   <path d="{" ".join(paths)}"/>')
        glyphs.append("  </g>")
    lines.append(" </g>")
    lines.append(' <g id="Symbols">')
    lines += glyphs
    lines.append(" </g>")
    lines.append("</svg>")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    open(sys.argv[1], "w").write(template())

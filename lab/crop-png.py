#!/usr/bin/env python3
"""Crop a PNG, standard library only: lab/crop-png.py <in.png> <out.png> <x0> <y0> <x1> <y1>.

For README screenshots: shot.sh takes the whole session, and this cuts a window out of it at a fixed box, so a
retaken picture keeps its size (the `lab-desktop-vm` skill lists the boxes).
"""
import struct, sys, zlib


def read_png(path):
    data = open(path, "rb").read(); pos = 8; chunks = []
    while pos < len(data):
        length = struct.unpack(">I", data[pos:pos + 4])[0]
        chunks.append((data[pos + 4:pos + 8], data[pos + 8:pos + 8 + length])); pos += 12 + length
    header = dict(chunks)[b"IHDR"]; width, height = struct.unpack(">II", header[:8])
    bpp = {6: 4, 2: 3}[header[9]]
    raw = zlib.decompress(b"".join(body for kind, body in chunks if kind == b"IDAT"))
    stride = width * bpp; rows = []; prev = bytearray(stride); i = 0
    for _ in range(height):
        kind = raw[i]; line = bytearray(raw[i + 1:i + 1 + stride]); i += 1 + stride
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0; b = prev[x]; c = prev[x - bpp] if x >= bpp else 0
            if kind == 1: line[x] = (line[x] + a) & 255
            elif kind == 2: line[x] = (line[x] + b) & 255
            elif kind == 3: line[x] = (line[x] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c; pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line)); prev = line
    return bpp, rows


def write_png(path, rows, width, bpp):
    def chunk(kind, body): return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    header = struct.pack(">IIBBBBB", width, len(rows), 8, 6 if bpp == 4 else 2, 0, 0, 0)
    pixels = zlib.compress(b"".join(b"\x00" + row for row in rows), 9)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", pixels) + chunk(b"IEND", b""))


source, target = sys.argv[1], sys.argv[2]
x0, y0, x1, y1 = map(int, sys.argv[3:7])
bpp, rows = read_png(source)
write_png(target, [row[x0 * bpp:x1 * bpp] for row in rows[y0:y1]], x1 - x0, bpp)

#!/usr/bin/python3
"""In-VM, as tester: pre-seed freedesktop thumbnails for every JPEG in a mounted dir.

The thumbnail is rendered from the SOURCE copy (so seeding itself never opens a
file through the mount), but named and tagged for the MOUNT path, as the spec
requires:
  ~/.cache/thumbnails/<size>/<md5(file URI)>.png
  tEXt Thumb::URI   = the file URI of the mounted file
  tEXt Thumb::MTime = its integer mtime (stat through the mount: no open)

  seed-thumbs.py <mount_dir> <source_dir> <size_dir>:<px> [<size_dir>:<px> …]
  e.g. seed-thumbs.py ~/Immich/2024/03 /srv/photos/2024/03 large:256 x-large:512

Each seeded thumbnail carries a yellow "SEED <px>" banner, so a screenshot shows
whether Nautilus displays the seeded PNG or one it generated itself.
"""
import hashlib, os, sys
from pathlib import Path
from urllib.parse import quote
from PIL import Image, ImageDraw, PngImagePlugin

mount_dir, source_dir = Path(sys.argv[1]), Path(sys.argv[2])
buckets = [(d, int(px)) for d, px in (a.split(":") for a in sys.argv[3:])]
cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "thumbnails"

for src in sorted(source_dir.glob("*.jpg")):
    target = mount_dir / src.name
    uri = "file://" + quote(str(target))               # same as g_filename_to_uri for ASCII names
    mtime = int(os.stat(target).st_mtime)
    name = hashlib.md5(uri.encode()).hexdigest() + ".png"
    with Image.open(src) as im:
        for bucket, px in buckets:
            thumb = im.copy()
            thumb.thumbnail((px, px))                     # longest side = px, aspect kept
            draw = ImageDraw.Draw(thumb)
            bar = max(12, thumb.height // 5)
            draw.rectangle([0, 0, thumb.width, bar], fill="yellow")
            draw.text((4, 2), f"SEED {px}", fill="black", font_size=bar - 4)
            info = PngImagePlugin.PngInfo()
            info.add_text("Thumb::URI", uri)
            info.add_text("Thumb::MTime", str(mtime))
            info.add_text("Software", "seed-thumbs.py")
            out = cache / bucket
            out.mkdir(parents=True, exist_ok=True)
            os.chmod(cache, 0o700)
            thumb.save(out / name, pnginfo=info)
    print(f"{src.name} {uri} mtime={mtime} -> {name}")

#!/usr/bin/env python3
"""Build the download page (jakob.hviid.phd/immich-mount) for one release: site/index.html with that release's file links,
site/style.css, the icon and the screenshots, written into an output folder for GitHub Pages (ARCHITECTURE.md,
"The download page"). Standard library only.

    gh release view [TAG] --json tagName,publishedAt,url,assets > release.json
    site/build.py release.json OUTPUT

A release without a macOS disk image shows the macOS edition as not released yet, so the page never offers a file
that is not there.
"""

import html
import json
import re
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def find(assets, pattern):
    """The download address of the one asset whose name matches `pattern`, or None."""
    for asset in assets:
        if re.fullmatch(pattern, asset["name"]):
            return asset["url"]
    return None


def link(url, text):
    if url is None:
        return f'<span class="missing">{html.escape(text)}</span>'
    return f'<a href="{html.escape(url)}">{html.escape(text)}</a>'


def main(release_file, output):
    release = json.loads(Path(release_file).read_text())
    assets = release["assets"]
    version = release["tagName"].lstrip("v")
    files = {
        "rpm": find(assets, r"immich-mount-[\d.]+-\d+\.x86_64\.rpm"),
        "rpm_arm": find(assets, r"immich-mount-[\d.]+-\d+\.aarch64\.rpm"),
        "rpm_gnome": find(assets, r"immich-mount-gnome-.*\.x86_64\.rpm"),
        "rpm_kde": find(assets, r"immich-mount-kde-.*\.x86_64\.rpm"),
        "deb": find(assets, r"immich-mount_[\d.]+_amd64\.deb"),
        "deb_arm": find(assets, r"immich-mount_[\d.]+_arm64\.deb"),
        "deb_gnome": find(assets, r"immich-mount-gnome_.*_amd64\.deb"),
        "deb_kde": find(assets, r"immich-mount-kde_.*_amd64\.deb"),
        "tar": find(assets, r"immich-mount-x86_64-unknown-linux-musl\.tar\.gz"),
        "tar_arm": find(assets, r"immich-mount-aarch64-unknown-linux-musl\.tar\.gz"),
        "dmg": find(assets, r"(?i)immich[-_ ]mount.*\.dmg"),
    }
    if files["dmg"]:
        macos = (
            f'<p>One app for Apple silicon and Intel Macs, on macOS 14 or later.</p>'
            f'<p><a class="button" href="{html.escape(files["dmg"])}">Download for macOS</a></p>'
            '<p>Or with <a href="https://brew.sh">Homebrew</a>:</p>'
            '<pre class="command"><code>brew install --cask jakobhviid/tap/immich-mount</code></pre>'
            '<p class="small">On first start, switch Immich Mount on in System Settings, under General, Login Items '
            '&amp; Extensions, File Providers. Immich then shows in Finder\'s sidebar.</p>'
        )
    else:
        macos = (
            '<p>The macOS edition, Immich in Finder\'s sidebar through Apple\'s File Provider, is being built and is '
            'not released yet. Follow it on the <a href="https://github.com/jakobhviid/immich-mount/blob/main/ROADMAP.md">'
            'roadmap</a>.</p>'
        )
    values = {
        "version": html.escape(version),
        "release_url": html.escape(release["url"]),
        "published": html.escape(release.get("publishedAt", "")[:10]),
        "macos": macos,
        "macos_ready": "ready" if files["dmg"] else "not-ready",
        "fedora": f'{link(files["rpm"], "immich-mount")}, plus {link(files["rpm_gnome"], "the GNOME app")} or '
        f'{link(files["rpm_kde"], "the KDE app")}',
        "ubuntu": f'{link(files["deb"], "immich-mount")}, plus {link(files["deb_gnome"], "the GNOME app")} or '
        f'{link(files["deb_kde"], "the KDE app")}',
        "daemon_only": f'{link(files["rpm"], ".rpm")} or {link(files["deb"], ".deb")} (x86_64); '
        f'{link(files["rpm_arm"], ".rpm")} or {link(files["deb_arm"], ".deb")} (ARM)',
        "static": f'{link(files["tar"], "x86_64")} or {link(files["tar_arm"], "ARM")}',
    }
    page = (ROOT / "site" / "index.html").read_text()
    for name, value in values.items():
        page = page.replace("{{" + name + "}}", value)
    left = re.findall(r"\{\{\w+\}\}", page)
    if left:
        sys.exit(f"index.html has values build.py does not fill: {', '.join(sorted(set(left)))}")

    output = Path(output)
    if output.exists():
        shutil.rmtree(output)
    (output / "screenshots").mkdir(parents=True)
    (output / "index.html").write_text(page)
    shutil.copy(ROOT / "site" / "style.css", output / "style.css")
    shutil.copy(ROOT / "packaging" / "icons" / "dev.hviid.ImmichMount.svg", output / "icon.svg")
    for shot in sorted((ROOT / "docs" / "screenshots").glob("*.png")):
        shutil.copy(shot, output / "screenshots" / shot.name)
    # A renamed screenshot or icon would leave a broken image rather than fail, so every local link is checked.
    broken = [path for path in re.findall(r'(?:src|href)="([^"#:]+)"', page) if not (output / path).is_file()]
    if broken:
        sys.exit(f"index.html links files the page does not have: {', '.join(sorted(set(broken)))}")
    print(f"built the page for {release['tagName']} in {output}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])

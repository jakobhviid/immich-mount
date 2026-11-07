#!/usr/bin/env python3
"""Build the download page (jakob.hviid.phd/immich-mount) for one release: site/index.html with that release's file links,
site/style.css, the icon and the screenshots, written into an output folder for GitHub Pages (ARCHITECTURE.md,
"The download page"). Standard library only.

    gh release view [TAG] --json tagName,publishedAt,url,assets > release.json
    site/build.py release.json OUTPUT [--macos MAC_RELEASE_JSON [--update DMG.update]]

site/build-latest.sh gathers these for the latest release. The Mac download comes from MAC_RELEASE_JSON, the newest
release with a disk image (ci/latest-mac-release.sh), while the Mac is released by hand and the releases after it
carry none; without one, the page shows the macOS edition as not released yet, so it never offers a file that is not
there. With the disk image's update file (ci/sign-mac-update.sh), the page also carries appcast.xml, the feed the
Mac app's updates read.
"""

import argparse
import email.utils
import html
import json
import re
import shutil
import sys
from datetime import datetime
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


def appcast(mac_release, dmg, update_file):
    """Sparkle's feed with the one update: the version Sparkle compares is the build number, the disk image's size
    and signature are what the app checks before installing it."""
    update = dict(line.split("=", 1) for line in Path(update_file).read_text().split() if "=" in line)
    asset = next(asset for asset in mac_release["assets"] if asset["url"] == dmg)
    version = mac_release["tagName"].lstrip("v")
    published = datetime.fromisoformat(mac_release["publishedAt"].replace("Z", "+00:00"))
    return f"""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Immich Mount</title>
    <link>https://jakob.hviid.phd/immich-mount/</link>
    <item>
      <title>Immich Mount {html.escape(version)}</title>
      <pubDate>{email.utils.format_datetime(published)}</pubDate>
      <sparkle:version>{html.escape(update["build"])}</sparkle:version>
      <sparkle:shortVersionString>{html.escape(version)}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <sparkle:releaseNotesLink>{html.escape(mac_release["url"])}</sparkle:releaseNotesLink>
      <enclosure url="{html.escape(dmg)}" length="{asset["size"]}" type="application/octet-stream"
                 sparkle:edSignature="{html.escape(update["signature"])}"/>
    </item>
  </channel>
</rss>
"""


def main(release_file, output, mac_release_file=None, update_file=None):
    release = json.loads(Path(release_file).read_text())
    assets = release["assets"]
    version = release["tagName"].lstrip("v")
    mac_release = json.loads(Path(mac_release_file).read_text()) if mac_release_file else release
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
        "dmg": find(mac_release["assets"], r"(?i)immich[-_ ]mount.*\.dmg"),
    }
    if files["dmg"]:
        macos = (
            '<p>One app for Apple silicon and Intel Macs, on macOS 14 or later, signed and notarized by Apple. It '
            'lives in the menu bar, and your library in Finder\'s sidebar.</p>'
            f'<p><a class="button" href="{html.escape(files["dmg"])}">Download for macOS</a></p>'
            f'<p class="small">Version {html.escape(mac_release["tagName"].lstrip("v"))}. Open the disk image, drag '
            'Immich Mount into Applications and open it from there. Sign in, then switch Immich Mount on in System '
            'Settings, under General, Login Items &amp; Extensions, File Providers: the app shows the way.</p>'
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
    if update_file and files["dmg"]:
        (output / "appcast.xml").write_text(appcast(mac_release, files["dmg"], update_file))
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
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("release")
    parser.add_argument("output")
    parser.add_argument("--macos", help="the newest release with a disk image, as gh release view writes it")
    parser.add_argument("--update", help="that disk image's update file, from ci/sign-mac-update.sh")
    arguments = parser.parse_args()
    if arguments.update and not arguments.macos:
        parser.error("--update goes with --macos")
    main(arguments.release, arguments.output, arguments.macos, arguments.update)

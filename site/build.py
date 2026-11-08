#!/usr/bin/env python3
"""Build the download page (jakob.hviid.phd/immich-mount) for one release: site/index.html with that release's file links,
site/style.css, the mark, the icons and the screenshots, written into an output folder for GitHub Pages (ARCHITECTURE.md,
"The download page"). Standard library only.

    gh release view [TAG] --json tagName,publishedAt,url,assets > release.json
    site/build.py release.json OUTPUT [--macos MAC_RELEASE_JSON [--update DMG.update] [--notes RELEASES_JSON]]

site/build-latest.sh gathers these for the latest release. The Mac download comes from MAC_RELEASE_JSON, the newest
release with a disk image and its update file (ci/latest-mac-release.sh), which is the latest release unless its Mac
job was switched off; without one, the page shows the macOS edition as not released yet, so it never offers a file
that is not there. With the disk image's update file (ci/sign-mac-update.sh), the page also carries appcast.xml, the
feed the Mac app's updates read.
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
PAGE = "https://jakob.hviid.phd/immich-mount/"


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


def version_key(tag):
    return tuple(int(part) for part in re.findall(r"\d+", tag)[:3])


def release_notes(releases, up_to, most=6):
    """The update window's notes: the newest releases' "New" and "Fixed" lists (ci/release-notes.sh) up to `up_to`,
    newest first, as one small HTML page for Sparkle to show as it is. Empty when none of them changed anything for
    users, and the window then shows no notes."""
    sections = []
    for release in sorted(releases, key=lambda release: version_key(release["tag_name"]), reverse=True):
        if release.get("draft") or version_key(release["tag_name"]) > version_key(up_to):
            continue
        parts, items = [], []
        for line in (release.get("body") or "").splitlines():
            line = line.strip()
            if line.startswith("### "):
                if items:
                    parts.append("<ul>" + "".join(items) + "</ul>")
                    items = []
                parts.append(f"<h3>{html.escape(line[4:])}</h3>")
            elif line.startswith("- ") and parts:
                items.append(f"<li>{html.escape(line[2:])}</li>")
        if items:
            parts.append("<ul>" + "".join(items) + "</ul>")
        if parts:
            sections.append(f"<h2>{html.escape(release['tag_name'].lstrip('v'))}</h2>" + "".join(parts))
        if len(sections) == most:
            break
    if not sections:
        return ""
    style = ("<style>:root{color-scheme:light dark}body{font:13px -apple-system,sans-serif;margin:0 4px}"
             "h2{font-size:13px;margin:14px 0 2px}h2:first-of-type{margin-top:4px}"
             "h3{font-size:12px;font-weight:600;opacity:.65;margin:6px 0 2px}ul{margin:0;padding-left:18px}"
             "li{margin:2px 0}</style>")
    return style + "".join(sections)


def appcast(mac_release, dmg, update_file, notes=""):
    """Sparkle's feed with the one update: the version Sparkle compares is the build number, the disk image's size
    and signature are what the app checks before installing it. The notes go in as they are, so the update window
    shows them and nothing else; without notes it shows none."""
    update = dict(line.split("=", 1) for line in Path(update_file).read_text().split() if "=" in line)
    asset = next(asset for asset in mac_release["assets"] if asset["url"] == dmg)
    version = mac_release["tagName"].lstrip("v")
    published = datetime.fromisoformat(mac_release["publishedAt"].replace("Z", "+00:00"))
    # CDATA keeps the notes' HTML as it is; their text is escaped, so "]]>" cannot occur in it.
    description = f"      <description><![CDATA[{notes}]]></description>\n" if notes else ""
    return f"""<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Immich Mount</title>
    <link>{PAGE}</link>
    <item>
      <title>Immich Mount {html.escape(version)}</title>
      <pubDate>{email.utils.format_datetime(published)}</pubDate>
      <sparkle:version>{html.escape(update["build"])}</sparkle:version>
      <sparkle:shortVersionString>{html.escape(version)}</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
{description}      <enclosure url="{html.escape(dmg)}" length="{asset["size"]}" type="application/octet-stream"
                 sparkle:edSignature="{html.escape(update["signature"])}"/>
    </item>
  </channel>
</rss>
"""


def main(release_file, output, mac_release_file=None, update_file=None, notes_file=None):
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
            '<p>Or with <a href="https://brew.sh">Homebrew</a>:</p>'
            '<pre class="command"><code>brew install --cask jakobhviid/tap/immich-mount</code></pre>'
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
    # What Linux packages from this page look at once a day, to tell of a newer release (ARCHITECTURE.md, "Updates
    # on Linux").
    (output / "latest.json").write_text(json.dumps({"version": version, "page": PAGE}) + "\n")
    if update_file and files["dmg"]:
        releases = json.loads(Path(notes_file).read_text()) if notes_file else []
        notes = release_notes(releases, mac_release["tagName"])
        (output / "appcast.xml").write_text(appcast(mac_release, files["dmg"], update_file, notes))
    shutil.copy(ROOT / "site" / "style.css", output / "style.css")
    # The mark for the project, and each platform's own icon for that platform (DESIGN.md, "The website").
    shutil.copy(ROOT / "site" / "mark.svg", output / "mark.svg")
    shutil.copy(ROOT / "site" / "llms.txt", output / "llms.txt")
    shutil.copy(ROOT / "packaging" / "icons" / "dev.hviid.ImmichMount.svg", output / "icon.svg")
    shutil.copy(ROOT / "apple" / "App" / "Assets.xcassets" / "AppIcon.appiconset" / "icon_128x128@2x.png",
                output / "icon-macos.png")
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
    parser.add_argument("--notes", help="the latest releases with their notes, as gh api repos/{owner}/{repo}/releases writes them")
    arguments = parser.parse_args()
    if arguments.update and not arguments.macos:
        parser.error("--update goes with --macos")
    main(arguments.release, arguments.output, arguments.macos, arguments.update, arguments.notes)

#!/usr/bin/env python3
"""Regenerate manifest.json and ATTRIBUTION.md from the Wikimedia Commons API.

Run only to change the fixture selection. seed.sh uses the committed, pinned
manifest.json and never calls this. Usage:  python3 -I fixtures/build-manifest.py

The selection below was found with Commons geosearch (namespace 6) around each city,
keeping JPEGs under 5 MB whose file metadata (iiprop=metadata, i.e. the real EXIF)
has DateTimeOriginal, and GPS for the "gps-*" ones, with CC0 / PD / CC BY / CC BY-SA.

Photos are fetched as standard-size Commons thumbnails, not originals: upload.wikimedia.org
answered original downloads with 429 + Retry-After: 600 after ~5 files and asked for
thumbnails instead. Thumbnails carry no EXIF, so the original's camera EXIF (from the
API, i.e. the real values) is pinned in "exif" and written back by seed-container.sh.
Videos are originals (only two files). The thumbnails are downloaded here once to pin
their sha1/sha256, straight into cache/ (git-ignored).
"""
import hashlib, html, json, os, re, sys, time, urllib.error, urllib.parse, urllib.request

UA = os.environ.get("WIKIMEDIA_UA",
     "ImmichDesktopNautilus-lab-fixtures/0.1 (test fixtures for a private dev lab; low volume)")
API = "https://commons.wikimedia.org/w/api.php"
HERE = os.path.dirname(os.path.abspath(__file__))

# (Commons title without "File:", roles). Roles drive seed.sh:
#   gps-<country>-<city>  EXIF GPS present (expect Places data)
#   date-no-gps           EXIF date, no GPS
#   name=<name>           upload under this filename instead of the Commons name
#   strip-metadata        exiftool -all= on a copy before upload (the "no date" asset)
#   album=<name>          add to that album
#   archive / trash       set visibility archive / move to trash after upload
#   video                 a video
SELECTION = [
    # France
    ("Sacrekör Cathedral.jpg", ["gps-france-paris", "album=Holiday Mix"]),
    ("Basilique du Sacré-Cœur - plaque 2.JPG", ["gps-france-paris"]),
    ("Basilica Sacre Coeur, Paris May 2014.jpg", ["gps-france-paris"]),
    ("Gargoyle water spouts 2, Paris May 2014.jpg", ["gps-france-paris", "archive"]),
    ("Pavé rouge de l'assassinat de Sadi Carnot.JPG", ["gps-france-lyon"]),
    ("Fête de la musique 2018 à Lyon - Chants place de la Bourse.jpg", ["gps-france-lyon", "name=IMG_0001.JPG", "duplicate-name-other-month"]),
    ("Festival de la soie 2018 - Flottes de soie.jpg", ["gps-france-lyon", "name=IMG_0001.JPG", "duplicate-name-same-month"]),
    ("Festival de la soie 2018 - Flottes et bobines.jpg", ["gps-france-lyon", "name=IMG_0001.JPG", "duplicate-name-same-month"]),
    ("Palais de la Bourse de Lyon - Verrière.jpg", ["gps-france-lyon", "album=Holiday Mix"]),
    # Denmark
    ("Cryptocoryne walkeri.jpg", ["gps-denmark-copenhagen"]),
    ("Kommunehospitalet (Copenhagen).jpg", ["gps-denmark-copenhagen"]),
    ("KU City Campus.jpg", ["gps-denmark-copenhagen", "album=Persons"]),
    ("København K, København, Denmark - panoramio (236).jpg", ["gps-denmark-copenhagen"]),
    ("Aarhus Teater 2019.jpg", ["gps-denmark-aarhus", "album=Persons"]),
    ("Køn - Gender Museum Denmark.jpg", ["gps-denmark-aarhus"]),
    ("Besættelsesmuseet - Dagligstue.jpg", ["gps-denmark-aarhus", "trash"]),
    ("3D projektion på Odense Rådhus.JPG", ["gps-denmark-odense"]),
    ("Dom van Odense 4.jpg", ["gps-denmark-odense"]),
    ("Odense Rådhus.jpg", ["gps-denmark-odense"]),
    # Japan
    ("Tokyo area bullet train cleaning ladies - Dec 29 2018 01-41PM.jpeg", ["gps-japan-tokyo"]),
    ("Shinkansen platform at Tokyo Station 14.jpg", ["gps-japan-tokyo", "album=Holiday Mix"]),
    ("タテ弁 01.jpg", ["gps-japan-tokyo"]),
    ("タテ弁 02.jpg", ["gps-japan-tokyo"]),
    ("東京駅 - panoramio (16).jpg", ["gps-japan-tokyo"]),
    # USA
    ("Times Square August 2021 011.jpg", ["gps-usa-newyork"]),
    ("Times Square - New York - USA - panoramio (1).jpg", ["gps-usa-newyork"]),
    ("Loew's State Theatre.jpg", ["gps-usa-newyork"]),
    # Iceland
    ("2016-08 Iceland Parliamentarians Meeting (29203692615).jpg", ["gps-iceland-reykjavik", "album=Persons"]),
    ("Iceland (20018350536).jpg", ["gps-iceland-reykjavik"]),
    ("Tjarnarbíó 2025.jpg", ["gps-iceland-reykjavik"]),
    ("池湖 Lake Tjörnin - panoramio.jpg", ["gps-iceland-reykjavik"]),
    # Australia
    ("Sydney Opera House, Sydney, Australia (Unsplash).jpg", ["gps-australia-sydney"]),
    ("Sydney Opera House concert hall October 2018.jpg", ["gps-australia-sydney"]),
    ("Sydney Opera House from Circular Quay, 2023, 14.jpg", ["gps-australia-sydney", "album=Holiday Mix"]),
    ("Sydney Opera House interior tour, 2023, 16.jpg", ["gps-australia-sydney"]),
    # Germany
    ("Mitte, Berlin, Germany - panoramio (181).jpg", ["gps-germany-berlin"]),
    ("Mitte, Berlin, Germany - panoramio (180).jpg", ["gps-germany-berlin"]),
    # EXIF date, no GPS (Vatican City, Rome, Bruges)
    ("Vatican City (Unsplash).jpg", ["date-no-gps", "big-image"]),
    ("교황청(바티칸), Città del Vaticano, Vatican City (Unsplash).jpg", ["date-no-gps"]),
    ("S.Pietro - panoramio - Emanuela Meme Giudic….jpg", ["date-no-gps"]),
    ("Blick vom Petersdom - panoramio.jpg", ["date-no-gps"]),
    ("Brugge - panoramio (194).jpg", ["date-no-gps"]),
    ("Lace (7402556828).jpg", ["date-no-gps"]),
    ("1982 Citroën HY (9502299641).jpg", ["date-no-gps"]),
    ("Bruges - panoramio (59).jpg", ["date-no-gps"]),
    ("Panorama di Roma - panoramio (23).jpg", ["date-no-gps"]),
    ("Vatikan, Petersdom, Orgel, Bild 1.JPG", ["date-no-gps"]),
    # the asset with no usable date: a copy with all metadata stripped at seed time
    ("Panorama di Roma - panoramio (14).jpg", ["strip-metadata", "name=no-date.jpg"]),
    # videos
    ("King's Peak summit.webm", ["video"]),
    ("Ocean waves at Lækjavik beach, Iceland.webm", ["video", "album=Holiday Mix"]),
]


def api(params):
    params.update(format="json", formatversion="2")
    req = urllib.request.Request(API + "?" + urllib.parse.urlencode(params), headers={"User-Agent": UA})
    for attempt in range(6):
        try:
            return json.load(urllib.request.urlopen(req, timeout=60))
        except urllib.error.HTTPError as e:
            if e.code != 429:
                raise
            time.sleep(int(e.headers.get("Retry-After") or 0) or 10 * (attempt + 1))
    sys.exit("Commons API keeps answering 429")


def fetch(url, dest):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    for attempt in range(4):
        try:
            data = urllib.request.urlopen(req, timeout=120).read()
            with open(dest, "wb") as f:
                f.write(data)
            return data
        except urllib.error.HTTPError as e:
            if e.code != 429:
                raise
            w = int(e.headers.get("Retry-After") or 0) or 30
            print(f"429 on {url}, waiting {w}s", file=sys.stderr)
            time.sleep(w)
    sys.exit("thumbnail host keeps answering 429")


STEPS = (1280, 1920, 3840)   # Wikimedia standard thumbnail widths that matter here


def text(v):
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", "", v or ""))).strip()


def main():
    info = {}
    titles = ["File:" + t for t, _ in SELECTION]
    for i in range(0, len(titles), 25):
        r = api({"action": "query", "titles": "|".join(titles[i:i + 25]), "prop": "imageinfo",
                 "iiprop": "url|size|sha1|mime|extmetadata|metadata",
                 "iiextmetadatafilter": "LicenseShortName|LicenseUrl|Artist|DateTimeOriginal"})
        norm = {n["to"]: n["from"] for n in r["query"].get("normalized", [])}
        for p in r["query"]["pages"]:
            if "imageinfo" not in p:
                sys.exit(f"missing on Commons: {p['title']}")
            info[norm.get(p["title"], p["title"])] = p
        time.sleep(2)
    thumbs = {}
    for w in STEPS:
        for i in range(0, len(titles), 25):
            r = api({"action": "query", "titles": "|".join(titles[i:i + 25]), "prop": "imageinfo",
                     "iiprop": "url", "iiurlwidth": str(w)})
            norm = {x["to"]: x["from"] for x in r["query"].get("normalized", [])}
            for p in r["query"]["pages"]:
                thumbs[(norm.get(p["title"], p["title"]), w)] = p["imageinfo"][0].get("thumburl")
            time.sleep(2)
    os.makedirs(os.path.join(HERE, "cache"), exist_ok=True)
    entries = []
    for n, (title, roles) in enumerate(SELECTION, 1):
        p = info["File:" + title]
        ii = p["imageinfo"][0]
        md = {m["name"]: m["value"] for m in (ii.get("metadata") or []) if isinstance(m, dict)}
        em = ii["extmetadata"]
        ext = os.path.splitext(title)[1].lower()
        if ii["mime"].startswith("image/"):
            # largest standard width below the original's (3840 for the big-image role)
            w = 3840 if "big-image" in roles else max(x for x in STEPS[:2] if x < ii["width"])
            url, variant = thumbs[("File:" + title, w)].split("?")[0], f"thumbnail-{w}px"
        else:
            url, variant = ii["url"].split("?")[0], "original"
        cache = os.path.join(HERE, "cache", f"{n:03d}{ext}")
        if variant == "original":
            # Originals are not downloaded here (upload.wikimedia.org rate-limits them hard);
            # Commons' own sha1 pins them. seed.sh treats them as optional.
            data, sha256 = None, None
        else:
            data = open(cache, "rb").read() if os.path.exists(cache) else fetch(url, cache)
            sha256 = hashlib.sha256(data).hexdigest()
            time.sleep(1)
        lat, lon = (md.get("GPSLatitude"), md.get("GPSLongitude"))
        exif = {k: md[k] for k in ("DateTimeOriginal", "OffsetTimeOriginal", "Make", "Model") if k in md}
        if lat is not None and lon is not None:
            exif.update(GPSLatitude=abs(lat), GPSLatitudeRef="N" if lat >= 0 else "S",
                        GPSLongitude=abs(lon), GPSLongitudeRef="E" if lon >= 0 else "W")
        entries.append({
            "id": f"{n:03d}",
            "roles": roles,
            "uploadName": next((r[5:] for r in roles if r.startswith("name=")), title),
            "title": p["title"],
            "pageUrl": ii["descriptionurl"],
            "url": url,
            "variant": variant,
            "originalUrl": ii["url"].split("?")[0],
            "originalSha1": ii["sha1"],
            "author": text(em.get("Artist", {}).get("value")) or "unknown",
            "license": em.get("LicenseShortName", {}).get("value", ""),
            "licenseUrl": em.get("LicenseUrl", {}).get("value", ""),
            "optional": variant == "original",
            "sha1": hashlib.sha1(data).hexdigest() if data else ii["sha1"],
            "sha256": sha256,
            "bytes": len(data) if data else ii["size"],
            "mime": ii["mime"],
            "width": ii["width"],
            "height": ii["height"],
            # EXIF DateTimeOriginal of the file ("YYYY:MM:DD hh:mm:ss"); for videos the
            # Commons page date. seed.sh sends it as fileCreatedAt.
            "dateTaken": md.get("DateTimeOriginal") or text(em.get("DateTimeOriginal", {}).get("value")),
            # camera EXIF of the original, re-applied to the thumbnail by seed-container.sh
            "exif": exif if ii["mime"].startswith("image/") and "strip-metadata" not in roles else {},
            "cacheName": f"{n:03d}{ext}",
        })
    with open(os.path.join(HERE, "manifest.json"), "w") as f:
        json.dump({"source": "Wikimedia Commons", "generatedBy": "fixtures/build-manifest.py",
                   "files": entries}, f, ensure_ascii=False, indent=1)
        f.write("\n")
    with open(os.path.join(HERE, "ATTRIBUTION.md"), "w") as f:
        f.write("# Fixture attribution\n\nGenerated from `manifest.json` by `build-manifest.py`; do not edit.\n"
                "The files are downloaded at seed time from Wikimedia Commons and are not in this repository.\n"
                "Photos are Commons' standard-size thumbnails (a scaled rendition of the work) with the original's\n"
                "camera EXIF written back; copies uploaded under another name, or with metadata stripped, are marked.\n\n"
                "| # | Work | Author | License | Use in the lab |\n|---|---|---|---|---|\n")
        for e in entries:
            mod = " (renamed copy)" if any(r.startswith("name=") for r in e["roles"]) else ""
            mod = " (metadata stripped)" if "strip-metadata" in e["roles"] else mod
            lic = f"[{e['license']}]({e['licenseUrl']})" if e["licenseUrl"] else e["license"]
            f.write(f"| {e['id']} | [{e['title'][5:]}]({e['pageUrl']}){mod} | {e['author'].replace('|', '/')} | {lic} | "
                    f"{', '.join(r for r in e['roles'] if not r.startswith('name='))} |\n")
    print(f"{len(entries)} entries, {sum(e['bytes'] for e in entries) / 1e6:.1f} MB")


main()

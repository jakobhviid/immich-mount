---
name: lab-immich-server
description: Use when a test needs a real Immich server - contract tests, mounting against real data, sign-in with OIDC, checking an API behaviour, or a version-floor check. Builds a disposable, seeded Immich server on the Incus lab host with lab/immich-server/, and explains what is in it and how to reach it.
---

# A disposable Immich server

Read AGENTS.md, "Testing on Erebus", first: its rules (the `ind-` prefix, the `lab` project, leave nothing behind,
throwaway credentials, no personal photos) apply to everything here.

## Build it

```sh
lab/immich-server/up.sh --lan     # or no flag for NAT; IMMICH_VERSION=v3.2.0 for the version floor
lab/immich-server/seed.sh         # admin, two API keys, 50 fixture photos and the edge cases below
lab/immich-server/oidc.sh         # optional: Dex as the OIDC provider, for browser sign-in
```

- `up.sh` takes about 90 s; `seed.sh` about 90 s with cached fixtures, about 160 s from an empty cache.
- Everything lands in the git-ignored `lab/immich-server/.env.local`: `IMMICH_URL`, a full-access and a read-only
  key, the throwaway admin login, and after `oidc.sh` the Dex user (`DEX_EMAIL`, `DEX_PASSWORD`).
- `seed.sh` expects a fresh server; on a seeded one it stops at the admin sign-up. Rebuild instead.
- Tear down with `lab/immich-server/down.sh`, always, and before anything else in a LAN-mode run: LAN mode exposes
  the server and its throwaway login to the whole LAN.

## Reach it

| From | NAT (default) | LAN (`--lan`) |
|---|---|---|
| this host | only through a tunnel: `apt-get install socat` in the container, then `socat TCP-LISTEN:2283,bind=127.0.0.1,reuseaddr,fork EXEC:"$EREBUS_DIR/bin/incus exec ind-immich-srv -- socat STDIO TCP\:127.0.0.1\:2283"` | its LAN address, which changes between runs (`.env.local` has it) |
| another lab instance on NAT | `http://ind-immich-srv.incus:2283` | its LAN address; the `.incus` name does not resolve |

The lab project forbids proxy devices, so NAT mode has no port forward. Prefer LAN mode when this host runs the
daemon; NAT mode when only lab VMs talk to the server.

## What is in it

Contract tests (`crates/immich-api/tests/contract.rs`) assert these numbers, so a change to the seed changes them:
- **52 uploads:** 51 not trashed (47 images, 4 videos), 50 on the timeline; 40 month buckets across 1970 and
  2008-2025; 7 countries and 11 cities; 10 photos with a date but no GPS.
- **One archived** ("Gargoyle water spouts 2") and **one trashed** ("Besættelsesmuseet - Dagligstue").
- **Albums:** "Holiday Mix" (5, one WebM) and "Persons" (3), named like a top-level folder on purpose.
- **One person,** made by hand (no machine learning): `POST /people`, then `POST /faces`.
- **Edge cases:** three different photos uploaded as `IMG_0001.JPG`; `no-date.jpg` with all metadata stripped
  (Immich puts it in 1970-01); Unicode and punctuation names (`タテ弁 01.jpg`, `Loew's State Theatre.jpg`, a `…`).
- **Cities are GeoNames' nearest place,** not the obvious one: Times Square is Weehawken, Tokyo Station Hatchōbori.

Machine learning is off (a compose override keeps its container out, and `seed.sh` turns it off in the system
config), so faces are never detected and smart search is unavailable.

## Fixtures

`fixtures/manifest.json` pins 50 Wikimedia Commons files (CC0, public domain, CC BY, CC BY-SA) with their URLs,
checksums, authors and licenses; `fixtures/ATTRIBUTION.md` is generated from it. The downloads go to the
git-ignored `fixtures/cache/`, which can be deleted at any time.
- **Commons' standard-size renditions, not originals:** `upload.wikimedia.org` answers bulk original downloads with
  429 and `Retry-After: 600`. Renditions carry no EXIF, so the manifest pins each original's camera metadata and
  `seed-container.sh` writes it back before uploading.
- **A checksum mismatch** means Wikimedia re-rendered a file: run `fixtures/build-manifest.py` to regenerate the
  manifest. Set `WIKIMEDIA_UA` to a descriptive User-Agent, as their policy asks.
- **Never add personal photos.** New fixtures are Creative Commons or public domain, added through the manifest.

## Sign-in with OIDC

`oidc.sh` runs Dex beside the server with one static user (`admin@example.com`, so OIDC lands on the seeded
admin) and points Immich's OAuth settings at it. Dex's issuer is plain http, which Immich refuses unless the OAuth
setting `allowInsecureRequests` is on; the script sets it. Real servers use https.

`lab/immich-server/oidc-sign-in.py [path/to/immich-mount]` plays the person at the browser: it runs
`immich-mount login` with a private HOME, fills in Dex's form, and hands the `app.immich:///oauth-callback`
redirect back. It fails unless the CLI signs in and `status` reaches the library.

## Traps

- Source `$EREBUS_DIR/lib/erebus.sh` from bash, never zsh (it needs `BASH_SOURCE`).
- Docker runs in the unprivileged container with `security.nesting=true` alone. Immich supports Docker Compose
  only, not Podman.
- The server's own OpenAPI spec is at `$IMMICH_URL/api/spec.json`; check a field there before guessing.

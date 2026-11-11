# ROADMAP

The single home for what is not built: milestones, open questions, and ideas parked on purpose. There is no
TODO.md and there are no stray TODO comments. Every milestone ends with an end-to-end check on Erebus (AGENTS.md,
"Testing on Erebus").

## M0. Design and spikes

- [x] The design: README, SPEC, ARCHITECTURE, WORKFLOWS, PRINCIPLES, with credits.
- [x] GNOME VM spike: the sidebar, thumbnail seeding, indexer defaults (ARCHITECTURE, "Thumbnails").
- [x] Immich server spike: range reads, thumbnails, buckets, change tracking, Creative Commons fixtures
      (ARCHITECTURE, "The Immich API layer").
- [x] The floor holds on Immich v3.2.0: search dialect, sync stream fields, timeline album and person filters
      (ARCHITECTURE, "Version policy").
- [x] Settled before M1:
  - Immich's section labels
  - album folders with a year prefix
  - date-time file names with remembered counters
  - day folders in busy months
  - the app ID `dev.hviid.ImmichMount`

  All are in SPEC.md.
- [x] A private GitHub repository: `jakobhviid/immich-mount`.

## M1. Read-only mount

The core (`immich-api`, `immich-mount-core`), the Linux adapter (`immich-mount-linux`) and `immich-mount`:
- the tree: sections, date groups, names and clashes, attributes, virtual files
- membership from Immich's timeline endpoints
- the metadata store (redb) with freshness windows, and ETag polling
- the block content cache with range reads
- an API key for development and tests
- the CLI surface:
  - `daemon`, `login --api-key`, `mount`, `unmount`, `status`, `doctor`, `configure`
  - `--json`, `--llm`, `--man`, `completions`
- the systemd user unit
- **mounted read-only** until uploads exist
- CI:
  - the clippy gate and the tests
  - the core compiles for `aarch64-apple-darwin`
  - the core's tests run through the in-memory adapter

Exit: in the GNOME VM, against the lab server, every section browses correctly, photos and videos open, and
copied-out files match the server's checksums.

**Status (2026-11-02): met.** In the GNOME 50 VM, every section browses, and all 50 timeline originals (photos and
videos) read back with the server's SHA-1. `immich-mount mount` starts the user service and waits for the mount;
changes are polled every 10 seconds, for every credential (ARCHITECTURE, "Freshness").

## M2. Thumbnails, sign-in and live changes

- Thumbnail seeding in both phases, the search-crawl rule, and the safety net. **Built.** In the VM, an album
  and a city show photo and video thumbnails while the content cache stays empty, and a Files search across the
  mount reads zero originals (verified in the GNOME VM).
- Read-ahead, eviction and offline behaviour of the block cache. **Eviction is built** (LRU over `cache.limit`, with
  the block map kept across restarts), and so is read-ahead. Offline, a folder visited before lists as it was, and
  uncached reads fail at once until a change poll reaches the server (up to 45 seconds), and the `offline` state
  comes with one notification.
- Browser sign-in (the `app.immich` scheme handler, and paste-back when headless) and password sign-in. The session
  goes into the keyring, and changes follow the sync stream. **The keyring and the sync stream are built** (verified
  in the GNOME VM; ARCHITECTURE, "Freshness"); the websocket for `all` keys is parked, since
  the 10-second poll already brings changes within seconds.
- An identity provider on Erebus next to the Immich lab: Dex for automated sign-in tests, Pocket ID (passkeys) for a
  manual check. **Dex is built** (`lab/immich-server/oidc.sh`), and the terminal's browser sign-in passes against it
  (`oidc-sign-in.py`); Pocket ID and the app's scheme handler are not.

Exit:
- Opening a 500-Asset folder in Files reads zero originals (bpftrace probe, server log). **Met at 600 files**
  (verified in the GNOME VM): zero originals read, the first listing with seeding in under half a second.
- Thumbnails show.
- A video seeks with only the blocks it needs fetched. **Met**: a seek to 75% of a 42 MB video fetched the
  header, the seek index and the block at the frame, 3 of 21.
- A change made in the Immich web UI reaches the mount within seconds. Whether Files redraws an open folder, or
  needs F5, is recorded. **Met**: 6 to 9 s with the 10-second poll; Files needs F5 or re-entry, since it
  watches with inotify.

## M3. Uploads

- Staging, upload on close, receipts, temporary names, duplicate and trashed-duplicate outcomes, notifications.
- Watch Files' real copy and move call sequence (close, `setattr`, rename) in the GNOME VM, and adjust receipts to it.
- An independent adversarial review before closing it (data-loss surface).

Exit:
- Copies and moves into Upload work.
- With the server stopped midway, a move keeps its source.
- Duplicates are reported correctly.

**Status (2026-11-02): built, verified on the host, not closed.** Staging, upload in `flush`, receipts, temporary
names uploaded on rename, duplicates and trashed duplicates, refused types at `create`, and the errors programs see
(SPEC, "Uploads"). On the host against the lab server: `cp`, a temp name then `mv`, and `gio copy` upload
byte-identical files; a duplicate counts as Already in library; a key without `asset.upload` makes `mv` and
`gio move` fail and keep their source. In the GNOME VM, Files' own copy and move upload, and with the server
stopped midway a Files move fails with "Error closing file" and keeps its source (verified in the GNOME VM).
The adversarial review found three ways to lose or corrupt data, all fixed and re-verified: a reader's or
second close starting or skipping the upload (fixed: per-handle, every close gets the result), hidden photo names
staged and dropped, and receipts reading back empty; plus rename under the folder lock, cut-short copies (now
refused by the file's own structure, and for killed writers), renames onto Assets' names, and leaked staging
files. Notifications for refused, failed and long uploads are built, and show on GNOME 50. M3's exit criteria are
met; what remains is watching a Cancel in Files' progress popover by hand.

**Folders copied into Upload (2026-11-09): built and tested.** A folder copied into Upload uploads everything in
it, and once the copy has settled its folders may become albums named by their paths, asked with two buttons or set
by `uploads.folders` (SPEC and ARCHITECTURE, "Folders copied into Upload"). Tested: the core's record and album
work, the staging area's folders, a real FUSE mount taking folders inside folders, refusals and `rmdir`, and the Mac
bridge's folder items, questions, answers, albums and departures. **Verified in a GNOME 50 VM** against the lab
server, with Files' own copy: a folder with a subfolder and a text file (Files stops at the text file; after 40
seconds on its question, Skip All copies the rest into the folders, which stayed); the urgent question stays on
screen; Make Albums made `Trip` and `Trip › Day 1`, a taken name became `Holiday Mix - 2026-11-09`, and Timeline
Only made none; the folders leave about 20 seconds after their copy, and a copy paused 40 seconds on Files'
question, its folder gone meanwhile, found the folder again and finished; a daemon restarted before the answer
closed its question and asked again. **Verified in a Plasma 6.8 VM** with KDE's own copy (`kioclient copy`, which
makes every folder first): every photo uploaded, the urgent question showed its buttons, and Make Albums made
`River › Day 1` and `River › Day 2`. **On the maintainer's Mac** (macOS 27.2), Finder's folder copy, the app's
dialog and the albums work; Upload takes a change of its times; macOS answers the extension's question about
its pending set, so a copied folder leaves soon after its copy; and a folder of photos that were in Immich's trash
comes back from the trash and into its albums (also verified against the lab server, from the GNOME VM). The adversarial review (2026-11-09)
found no way to lose a photo; it found a folder of a busy copy going too soon (Dolphin and rsync make every folder
first), a lost answer making a second album, album work stranded by a cancelled poll on macOS, the desktop looked
for only at login, a question lost to a daemon restart or to a click on its body, and Upload marked failing on the
Mac by a change of its times. All are fixed, and covered by tests where they can be. Open:
- What macOS does with a copied folder deleted by the extension while it still holds a file it has not handed over
  (the pending set is read first to avoid it).

## M4. The GNOME app

- First run, sign-in (all three methods), settings from the shared settings table, status, the About dialog with the
  notice, start at login.

Exit: a fresh GNOME VM goes from install to a working mount using only the app.

**Status (2026-11-02): exit met in the GNOME 50 VM.** Installed from the rpm, the app alone signs in (password,
creating the keyring), offers the sections and the location, and opens the mount in Files. `immich-mount-gnome`
(gtk4-rs, libadwaita, Blueprint) has the connect page, sign-in offering only what the server supports, first-run
setup, the status page (Immich's version, connection, cache and uploads in flight, read from the daemon over
D-Bus), "Mount" and "Unmount", "Sign In Again", "Disconnect", the preferences dialog from the settings table, a keyboard shortcuts dialog,
and the About dialog with the notice and credits. Verified in the GNOME 50 VM: single sign-on through Firefox and
Dex, handed back by the `app.immich` scheme handler, kept in the keyring, and the mount started. Packaged as an rpm
for Fedora 44 and a .deb for Ubuntu 26.04.

## M5. First release

**Status (2026-11-02): .deb and .rpm of the daemon and CLI are built** by the release workflow with the
distributions' own tools (`dpkg-deb`, `rpmbuild` on the prebuilt static binary) and attached to the GitHub
releases; tested by installing on Fedora 44 and Debian 13. The app is packaged as an rpm for Fedora 44, built in
the release workflow's `fedora:44` job, with AppStream metadata (validated) and the scheme handler. Homebrew is built
(below); COPR, the AUR and Flatpak are not yet. The app is also a .deb for Ubuntu 26.04 (GTK 4.22,
libadwaita 1.9), built in an `ubuntu:26.04` job and tested by installing in a container.

- Homebrew (the app's cask and the headless formula), RPM (COPR), DEB (GitHub Releases) and AUR. Each is tested on Erebus, with the
  install matrix plus a full GNOME VM run (`lab/packaging/`). **The install matrix is built** (`lab/packaging/matrix.sh`)
  and passes: Fedora 44 and Ubuntu 26.04 with both apps, Fedora 43, Ubuntu 24.04 and Debian 13 with the
  daemon; each installed, checked and removed.
- **README screenshots** of how it looks and works, taken in the GNOME VM: the Immich volume in the Files sidebar,
  a month with thumbnails, an album, the app's first run and status page. AppStream reuses them.
- AppStream metadata.
- **Public (2026-11-02):** the history is a single 1.0.0 commit, the repository is public, and GitHub's private
  vulnerability reporting is on (SECURITY.md points to it).
- **Homebrew is built:** `brew install jakobhviid/tap/immich-mount` installs the whole app, a cask with both
  interfaces (built on Fedora 44), the daemon and the command line, which the tap bumps daily from the releases. The
  formula `immich-mount-headless` is the daemon alone, for servers: the release workflow renders
  `ci/immich-mount-headless.rb.tmpl` with an x86_64 Linux bottle and pushes it to the maintainer's tap
  (`jakobhviid/homebrew-tap`), whose gate installs and tests it before it lands. Verified in a Fedora 44 GNOME VM
  with an atomic desktop's home (`/home` a link to `/var/home`): the one command, the app from the app grid,
  sign-in, the sidebar bookmark, Unmount and Mount, and the move from the 1.1 formula and cask. Next: COPR and the AUR, and Fedora 45 builds of the
  interfaces once Fedora 45 based systems need them (the Kirigami one is tied to its Fedora's Qt).
- An independent adversarial review before tagging.
- **The download page is built** (`site/`, ARCHITECTURE "The download page"): `jakob.hviid.phd/immich-mount`, with
  the latest release's files, deployed by the release workflow after each release. Once it is live, the README and
  the AppStream homepage link it.

## M6. KDE edition

- **Built:** the Kirigami app (`immich-mount-kde`) for sign-in, first-run setup, status and preferences, with "Show
  thumbnails" for Dolphin and an About page, packaged for Fedora 44 and Kubuntu 26.04. The mount works in Dolphin,
  and with previews allowed Dolphin shows the seeded thumbnails without reading originals; Dolphin's search crawl and
  KDE's thumbnail worker are recognised. KWallet works through the Secret Service (`ksecretd`): sign-in stored there
  and the daemon mounted from it.

Exit: a fresh Plasma VM on Erebus goes from install to a working mount in Dolphin.

**Status (2026-11-02): exit met in a fresh Plasma 6.8 VM.** Installed from the
rpms, the app alone signs in through the browser, keeps the session in a new KWallet, and opens the mount in
Dolphin, where it is in Places and shows the seeded thumbnails with no original read.

## M7. macOS edition

Finder through Apple's File Provider, on the same core (ARCHITECTURE, "The macOS edition"). Released with every
release since 2.14.2. Work that needs no Mac is done on Linux and checked by CI's macOS job; the maintainer's Mac is
needed only to run and debug it.

**Status (2026-11-08): closed,** tested and stable on the maintainer's Mac. Steps 2, 3 and 5 are met, 5 without the
install on a second Mac, which the maintainer dropped; what steps 1 and 4 left unseen is in "Open" below.

1. **The first slice.** The bridge crate (`immich-mount-apple`) and the item identifiers; the extension lists
   folders, fetches originals and answers thumbnails; the app signs in with an API key, reaches the server first
   (the Local Network prompt) and adds the Finder location; CI builds the whole Mac side on every branch push. A
   skill for driving the Mac over SSH, written in the first session on it.
   **Built, and nearly met (2026-11-06)** on the maintainer's Mac (macOS 27.0.1, Xcode 27.2), against the lab
   server through a loopback tunnel (`lab/mac/check.sh`; ARCHITECTURE, "The macOS edition", "Built so far"): every
   section browses in Finder, originals read back with the server's checksums, and thumbnails show with no
   original downloaded; videos too. Not yet: the lab server in LAN mode, which brings the Local Network prompt;
   with the maintainer's own server, whose address resolves outside the local network, macOS asks nothing.
   Exit: on the maintainer's Mac, against the lab server in LAN mode, every section browses in Finder, photos and
   videos open, copied-out files match the server's checksums, and thumbnails show with no original downloaded.
2. **Live changes.** The change journal in the store, the poll's state kept in the store, and the app as a login
   item that signals the working set.
   **Exit met (2026-11-06)** on the maintainer's Mac: a photo added or deleted on the server, or added to and taken
   out of an album, shows in Finder within about 10 seconds, also when the extension was stopped first.
   Exit: an Asset added, deleted, or moved between albums on the server shows in Finder within a poll interval, also
   after the extension was stopped.
3. **Sign-in and settings as on Linux.** Password and browser sign-in, the preferences from the core's settings
   table, status from the extension, the app's icon. The desktop crate's server check and sign-in move into the
   core, so both editions share them.
   **Built:** the shared sign-in, and the app's single sign-on, password and API key sign-in; on the maintainer's
   Mac, key and password sign-in add the location, and so does single sign-on (verified by the maintainer,
   2026-11-08). The Settings window, which the extension
   follows. Status from the extension, through a file. The icon, the same drive on macOS's rounded square
   (ARCHITECTURE, "The icon"). Finder's sidebar shows the location under the app's name, with the drive symbol
   (ARCHITECTURE, "The macOS edition", "Settings").
4. **Uploads** into the Upload folder, through the core's upload sequence.
   **Built and verified in Finder:** a photo copied or moved into Upload reaches the server byte for byte and leaves
   Upload; a moved photo's source is gone; a file Immich does not take stays in Upload, refused with its reason.
   Not yet seen: how Finder shows that refusal, and signing out with files waiting in Upload (removed with
   `preserveDirtyUserData`: where macOS keeps them, and that Finder opens that folder).
5. **Distribution.** A notarized universal DMG built by the release workflow; it needs the paid Apple Developer
   Program. The Homebrew cask `immich-mount` gains a macOS part beside its Linux one (the DMG, the app into
   Applications, macOS 14 or later, and a zap of the App Group folder and the sandbox containers), so one command
   installs on either system; the tap's pipeline bumps it from the releases. Gatekeeper refuses an unsigned app, so
   the cask waits for notarization. The download page (`jakob.hviid.phd/immich-mount`, built on `main`) offers the
   disk image and the cask once a release carries a `.dmg`.
   **Built:** `apple/release.sh` and the release workflow's `macos` job (ARCHITECTURE, "The macOS edition",
   "Distribution"); CI builds the unsigned disk image on every branch. **Verified on the maintainer's Mac
   (2026-11-07):** a disk image signed with Developer ID and notarized, which Gatekeeper accepts as "Notarized
   Developer ID", installed by dragging and run from Applications. With its secrets set and `MACOS_RELEASES` on
   (2026-11-07), the release workflow's `macos` job builds, signs and notarizes the disk image and attaches it, with
   its update file, to every release.
   **Released: 2.14.2 (2026-11-07),** the first Mac release, built on the maintainer's Mac and attached by hand.
   **2.15.1** is the first that updates itself (Sparkle); the feed is on the download page. **Verified on the
   maintainer's Mac:** 2.15.1 found 2.15.2 in the feed, checked its signature, replaced itself in Applications and
   started again. **Homebrew on the Mac:** the cask `immich-mount` has a macOS part beside its Linux one (the disk
   image, the app into Applications, `auto_updates`, a zap of its sandbox data), which the tap tests on a macOS
   runner, and its daily bump moves both systems' hashes (verified: 2.15.2 to 2.15.3). Verified on the maintainer's
   Mac: `brew install --cask` installed 2.15.2, and 2.15.3, the first signed
   and notarized in CI, updated it through Sparkle.
   Exit: a Mac that never had it installs from Homebrew, switches the extension on, and browses.

Open:
- **The Local Network prompt,** with a server on the LAN: macOS asks only before an app reaches a local address, so
  a server whose address is public never brings it (step 1).
- **How Finder shows a refused upload,** and **signing out with a file waiting in Upload** (removed with
  `preserveDirtyUserData`: where macOS keeps the file, and that Finder opens that folder) (step 4).
- **Three points the milestone's review could not check without a Mac:** that a location the system paused after
  "server unreachable" resumes once the app reports the server back (`signalErrorResolved`); what macOS does with
  files waiting in Upload when `uploads.enabled` is turned off; and that a reimport after a settings change, which
  matches items by name, never keeps an old file's bytes under a name that moved to another Asset.
- **A new item format without a reimport** (ARCHITECTURE, "The macOS edition", "Content"): the reimport leaves
  items unmatched until macOS runs a deferred import. Expiring the change anchor would have the system enumerate
  again without dropping its database; whether that refreshes the items it holds outside the working set is to be
  tried before the next format change.
- **Partial content** (`NSFileProviderPartialContentFetching`): a video that plays before it is fully downloaded,
  with the block cache back in use. Worth it once whole downloads are measured on a slow link.
- **What the poll cannot see** (a swap that keeps a month's count): on Linux the freshness windows catch it at the
  next visit, but the system revisits nothing it holds. Listing held folders again in the background, slowly?
- **A command line on macOS** (`status`, `doctor`), or none: the sandbox keeps the store and the credential out of
  reach of a plain program.

## The full Immich API

`immich-api` grows from the mount's endpoints to the API a full Immich client needs, so later apps (a gallery, or
more of the mount) build on one tested layer (ARCHITECTURE.md, "The Immich API layer"). Built: 190 endpoints, each
in the spec check, with contract tests that pass on the lab server on v3.3.6 and v3.2.0, and the machine-learning
tests on a server built with `up.sh --ml`.
1. **Albums, changing Assets, the trash, edits, metadata, tags, the map, video playback.** Built.
2. **People and faces, stacks, duplicates, memories, search (smart, explore, suggestions, places, random,
   statistics), folders, downloads.** Built. On the plain lab server faces and a duplicate group are made by hand
   and smart search is checked for its refusal; the server with machine learning checks the real results.
3. **Sharing:** shared links, partners, album comments and likes. Built, tested with a second user made for each
   test.
4. **The account:** the user and their preferences, profile picture, password, sessions, API keys, PIN code and
   the locked folder, notifications. Built.
5. **Administration:** users, queues and jobs, external libraries, server statistics, storage and version
   history. Built.
6. **Around it:** the lab server with machine learning (`up.sh --ml`) and the v3.2.0 runs are done. The CI job
   that runs the contract tests against Immich in Docker (`contract` in `.github/workflows/ci.yml`) runs on every
   branch push, and passes on v3.2.0 and v3.3.6.
7. **The mount on the grown crate:** the daemon built from the branch mounted the lab server (v3.3.6) on the
   development host: 50 photos, both albums, Lab Person and the seven countries listed as before, an album's five
   originals byte-identical to Immich's checksums, and a copy into Upload uploaded before `cp` returned. The release
   check on Fedora GNOME (`lab/check/run.sh gnome --binary`) passed every line with the branch's daemon and CLI.

## Open in Immich, and links between apps (Immich Crosslink)

Right-click items in Files, Dolphin and Finder that open what the file manager shows somewhere else: in Immich's own
web app, and in other Immich apps on the same computer. The other way round, any app can ask Immich Mount to show a
photo, album, person or place in the file manager. Apps find each other through
[Immich Crosslink](https://github.com/jakobhviid/immich-crosslink), a small open protocol published at
<https://jakob.hviid.phd/immich-crosslink/>: each app announces itself in the metadata it installs (the draft it
follows and the kinds of item it shows), registers the scheme `immich-crosslink:` that delivers links to it, and names
an item the way Immich's own web app does. Immich Mount implements draft 0.2. When this
section and the protocol differ, the protocol wins and this section is the bug. Neither app signs in for the other:
a link carries ids and the server's address, and works when both apps are signed in to that server.

Built in four steps, each usable on its own; an agent can build steps 1 to 3 from this section, SPEC and
ARCHITECTURE. Step 4 is in another project.

### The item map

An item's key (`NodeKey`, `crates/immich-mount-core/src/tree.rs`) is its whole path in keys: `storage_key` writes it
as `asset|<folder key>|<id>`, with the folder's own key inside, escaped. So a photo knows the album, person or place it
is listed in, and its link keeps that context. Each key maps to an **Immich path**, the path of Immich's web app for
it (`web/src/lib/route.ts` and `web/src/routes/(user)/` in immich-app/immich, and the paths Immich's mobile app takes
as deep links through `my.immich.app`, `mobile/lib/services/deep_link.service.dart`).

| Key | Immich path | Kind |
|---|---|---|
| `Asset` in `TimelineMonth` or `TimelineDay` | `/photos/<id>` | `asset` |
| `Asset` in `Album(a)`, directly or inside its `GroupYear`/`GroupMonth` | `/albums/<a>/photos/<id>` | `asset` |
| `Asset` in `Person(p)`, likewise | `/people/<p>/photos/<id>` | `asset` |
| `Asset` in `Section(Favorites)` or `Section(Archive)`, likewise | `/favorites/photos/<id>`, `/archive/photos/<id>` | `asset` |
| `Asset` in `City { country, city }`, likewise | `/search/photos/<id>?query=<q>` | `asset` |
| `Album(a)`, `Person(p)`, and a `GroupYear`/`GroupMonth` inside one | `/albums/<a>`, `/people/<p>` | `album`, `person` |
| `City { country, city }` | `/search?query=<q>`, `q` = `{"city":"<city>","country":"<country>"}` | `place` |
| `Country(country)` | `/search?query=<q>`, `q` = `{"country":"<country>"}` | `place` |
| `Section(Photos)`, `Albums`, `People`, `Places`, `Favorites`, `Archive` | `/photos`, `/albums`, `/people`, `/places`, `/favorites`, `/archive` | `page` |
| `Root`, `TimelineYear`, `TimelineMonth`, `TimelineDay`, `AlbumYear`, `Upload`, `UploadFolder`, an `Asset` in Upload | none: Immich has no page for it | |

The kind is the protocol's: other apps announce the kinds they show, and step 3 offers an app only on items of those
kinds.

- `q` is the JSON Immich's web app writes for a search (`Route.search`: `JSON.stringify` of the metadata search, keys
  in the order above, no spaces), encoded as `encodeURIComponent` does: every byte but `A-Z a-z 0-9 - _ . ! ~ * ' ( )`
  as `%XX` of its UTF-8. Immich's Places page links a city with `{"city":…}` alone; the country is added because a
  city's folder knows it, and it keeps two cities of one name apart.
- Ids are Immich's UUIDs, written in lower case; links read back accept either case. Names (cities, countries) are Immich's own strings from the key, never
  the folder's name on disk, which may be changed to fit a file system (SPEC, "Names").
- **A web address** is `server.url` without a trailing slash (and without `/api`), followed by the Immich path.
- **A crosslink** is `immich-crosslink://` with an empty host, the Immich path, and the server's address:
  `immich-crosslink:///albums/<a>/photos/<id>?server=<origin>`, where `<origin>` is `server.url` without a trailing
  slash, encoded as above; it joins a path's own query with `&`
  (`immich-crosslink:///search?query=<q>&server=<origin>`), and ends with `&user=<id>`, the signed-in user's Immich
  id, when the credential may read it.
- **Reading a link back** (step 2) is the map run backwards: an Immich path, from a crosslink or from a web address on
  the signed-in server, gives a key. `/photos/<id>` and the paths of a photo inside an album, person or city need the
  Asset's `localDateTime` (`GET /assets/{id}`) to find its month, its day where `grouping.photos` splits the month, and
  its date groups where the folder's mode makes them.

One module in `immich-mount-core` (`links.rs`, built, with the protocol's test links passing) holds the map both ways, with no platform code: a key to its Immich
path and kind, an Immich path to a key (given the Asset's date where needed), the web address and crosslink of a path,
and reading a crosslink into one of the protocol's three results (show, cannot show, invalid). Every platform uses it,
so the same item gives the same link everywhere. Unit tests cover every row of the table both ways, escaping (`|`,
`%`, spaces, non-ASCII city names), the JSON byte for byte against what Immich's web app writes, and paths that give
no key. They also run the protocol's test links: `tests/links.json` from Immich Crosslink, copied into
`crates/immich-mount-core/tests/crosslink-links.json` at the commit of the draft implemented (named in the file's
first test), whose `make` entries Immich Mount must write byte for byte and whose `read` entries it must read to the
given result.

### Step 1: Open in Immich, and Copy Immich Link

On one item at a time, inside the mount and nowhere else, two menu items: **Open in Immich** opens the item's web
address in the default browser, and **Copy Immich Link** puts it on the clipboard. Offered only for items with an
Immich path. **Built:** SPEC.md, "Links to Immich" and "In Finder", and ARCHITECTURE.md, "Links to Immich", have what
it is and why; the daemon's `WebLink` on D-Bus, `immich-mount web`, a Nautilus extension in C (and in Python for
Homebrew's cask), a Dolphin file item action plugin, and the File Provider extension's own actions in Finder. The
words are in every catalog.
- **Verified** (2026-11-10, the lab server on v3.3.6): in the Fedora 44 GNOME VM (Nautilus 50.4) and the Plasma VM,
  with the daemon built from the branch, the items showed on a photo in Photos, in an album, in a person and in a city,
  and on an album and a city folder; each opened the address `immich-mount web --json` gives, and Copy Immich Link
  left it on the clipboard (in Dolphin also after Dolphin quit). They did not show on a month folder, in Upload, in a
  folder copied into Upload, outside the mount, for two items, or (Files) with the daemon stopped, when the menu still
  opened at once. On Bazzite 44 GNOME the Python extension did the same through `/home`, in Danish from the cask's
  catalogs, and stayed quiet beside the C extension; on Bazzite 44 KDE the plugin built on Fedora 44 loaded from the
  home folder through `QT_PLUGIN_PATH`. On the maintainer's Mac (macOS 27.2) the maintainer opened albums and photos
  from Finder; over SSH, `fileproviderctl evaluate` offered the actions on the sections and refused them on Upload,
  a year and a month folder, and the extension put an address on the pasteboard.
- **Left:** Homebrew's cask installs the Python extension, the plugin and its Plasma login script (the tap's change
  waits for the maintainer); a release check line for the items (`lab/check/`).

### Step 2: Show in Immich Mount, from any app

Immich Mount takes crosslinks, and shows their item in the file manager, selected.

**Built on Linux:** SPEC.md, "Links from other apps", and ARCHITECTURE.md, "Links to Immich", have it: the launcher
entry announces draft 0.2 and takes the scheme, the daemon's `ShowLink` selects the item through
`org.freedesktop.FileManager1`, `immich-mount show`, the `crosslink.enabled` switch (Other Apps, in both apps'
settings) and every refusal's notification in all 20 languages. **Verified** (2026-11-10, lab server v3.3.6): in
the Fedora 44 GNOME and Plasma VMs, `xdg-open` of a crosslink selected the right file or folder in Files and Dolphin
for a photo in Photos, in an album, a person and a city (also inside date folders), an album, a person, a city, a
country and a section, without opening the app; another server, another user, an invalid link, `/tags`, an archived
photo with Archive off, the switch off and an unmounted drive each only notified. **Built on the Mac** too (SPEC.md,
"Links from other apps"): on macOS 27.2, links opened with `open -a` selected an album, a photo in Photos, in an
album and in a city, a person, a city and the Albums section in Finder, including folders Finder had never listed;
another server, another user, an invalid link and the switch off only notified, and a link that started the app
opened no window. A `/photos/<id>` landing in a day folder, which the seeded months are too small to make, is covered
by the tree's unit tests.

- **Announcing and registering:** on Linux, the launcher entry (`packaging/desktop/dev.hviid.ImmichMount.desktop.in`)
  announces `X-Immich-Crosslink-Version=0.2` and `X-Immich-Crosslink-Shows=asset;album;person;place;page;`, and
  `x-scheme-handler/immich-crosslink` joins `app.immich` in its `MimeType=`; its `Exec=immich-mount app %u` already
  hands a URL to the app, which passes a crosslink to the daemon. On the Mac, `apple/App/Info.plist` announces
  `ImmichCrosslink` (`Version` `0.2`, `Shows` the same five kinds) and declares the scheme in `CFBundleURLTypes`, and
  `AppDelegate` takes the URL (`application(_:open:)`). The Dev builds announce the same.
- **Reading it:** the core reads the link into the protocol's result: *invalid* links are dropped with a notification
  that the link is not valid, *cannot show* gets a notification saying Immich Mount cannot show that, and *show* goes
  on. The server must be the signed-in one (scheme, host and port compared, host in lower case, a default port
  written or not), and so must `user` when the link has it; the Immich path gives a key through the map run
  backwards. `immich-mount show <link>` does the
  same from a terminal, for a crosslink or a web address on the signed-in server, and prints the path with `--json`.
- **Showing it:** on Linux the daemon turns the key into its path (each folder's listing names its child) and calls
  `org.freedesktop.FileManager1.ShowItems`, which Files and Dolphin both answer; on the Mac the app gets the item's
  place from the system (`NSFileProviderManager.getUserVisibleURL(for:)`) and selects it with
  `NSWorkspace.activateFileViewerSelecting`.
- **Where a photo shows:** where the link says. `/photos/<id>` shows it in Photos, or in Archive when it is archived
  and that section is on; a photo in an album, person, city, Favorites or Archive shows in that folder.
- **Nothing to show** gets a notification saying why, and nothing else happens: another server or account, an id the
  account cannot see, a person the People section leaves out (hidden or unnamed), a section that is off, a path with
  no key.
- **Safety:** any web page can open a crosslink, so Immich Mount only ever shows a file or folder from one: never
  fetches an original, uploads, changes or deletes anything, and asks nothing.
- **Docs:** SPEC (the scheme, the command, what is shown where, the messages), ARCHITECTURE (receiving, and why only
  showing), WORKFLOWS.
- **Done when:** `xdg-open` (Linux) and `open` (Mac) of a crosslink for each row of the map selects the right file or
  folder, and a link for another server only notifies. Flatpak impact: a Flatpak exports its desktop entry with the
  scheme, so it can take crosslinks.

### Step 3: Show in other apps

**Built on Linux** (SPEC.md, "Show in other apps"; ARCHITECTURE.md, "Links to Immich"): Files (C and Python) and
Dolphin offer Show in *app* from the daemon's `Links`, and both apps' settings list the apps found with a switch each.
**Verified** (2026-11-10) in the Fedora 44 GNOME and Plasma VMs with three test apps (every kind, `album;` only, and
one announcing 0.1): an album folder offered both 0.2 apps, everything else only the every-kind one, the 0.1 app
never; each received exactly the item's crosslink with `&user=`; `crosslink.hidden` removed one app and
`crosslink.enabled` off removed every Show in item while Open in Immich stayed; against a daemon with only `WebLink`
the menus offered Open and Copy alone. The Python extension did the same on Bazzite 44. **Built on the Mac** too
(SPEC.md, "In Finder"): Immich Mount Dev on macOS 27.2 found Immich Gallery Dev (announcing) and two test apps,
`fileproviderctl evaluate` offered the action on Albums, an album and a photo in an album, and hiding an app or
switching crosslink off removed it (2026-11-10). **Left:** for a second or so after the daemon starts, an item in a
folder it has not listed yet gets no items; on the Mac, Show in Another App… sending straight to the one app and
asking Show in Which App? among several, and the Other Apps pane, wait for the maintainer's click and glance.

Immich Mount finds the other installed apps that take crosslinks, and offers **Show in *app*** for each, on the items
step 1 offers.

- **Finding them:** the apps registered for `immich-crosslink` that announce draft 0.2, less Immich Mount's own
  builds: on the Mac `NSWorkspace.urlsForApplications(toOpen:)` and each bundle's `ImmichCrosslink`, on Linux GIO's
  `g_app_info_get_all_for_type("x-scheme-handler/immich-crosslink")` and each entry's `X-Immich-Crosslink-*` keys
  (KDE's `KApplicationTrader` reads the same desktop entries). An app registered for the scheme without an
  announcement is not listed. An app is offered only on items of a kind it announces. An app's name and icon come
  from the system (its bundle, its desktop entry), never from the link or the announcement.
- **When:** Files' extension reads the apps for each menu, or keeps them until GIO's `GAppInfoMonitor` says apps may
  have changed; the Dolphin plugin likewise with `KSycoca::databaseChanged`. The Mac app reads them at launch, when it
  sees an app launch, and before Show in Another App… sends; it never asks an app anything at run time.
- **Opening one:** the crosslink, sent to that app alone: `NSWorkspace.open(_:withApplicationAt:configuration:)`,
  `g_app_info_launch_uris`. Never the scheme's default handler, which could be any of them.
- **Files** lists one item per app, and **Dolphin** too, through the file item action plugin for a list that changes
  (the protocol's first way to offer Show in). **Finder** declares its actions in the Info.plist, so their names are
  fixed: one action, **Show in Another App…** (the second way), sends to the one app there is, or asks which of
  several in a dialog. It is offered through a `userInfo` marker on the items of the kinds the apps announce; the app
  has the system read the items again when the set of apps or kinds changes (at launch, and when it sees an app
  launch).
- **No app set apart:** every app is found by its announcement and offered the same way. Immich Gallery gets Show in
  once it announces itself (step 4).
- **Settings:** `crosslink.enabled` (bool, default `true`): offer links to other apps. `crosslink.hidden` (list,
  default empty): apps left out, by bundle id or desktop entry id. In each settings window (the Mac's, GNOME's and
  Plasma's), a switch labelled **Immich Crosslink**, with a link to <https://jakob.hviid.phd/immich-crosslink/>, so it
  is clear what it is; under it the apps found, each with a switch, and Forget on one not found. The label and the link are the
  protocol's, the same in every app that takes part. With the switch off, a crosslink Immich Mount receives (step 2)
  only gets a notification saying so, with a way to turn it on. Open in Immich is not affected by either.
- **Docs:** SPEC (the items, the keys), ARCHITECTURE (finding apps, the fixed Finder action),
  DESIGN (the words), README.
- **Done when:** with test apps registered for the scheme (desktop entries, and small Mac apps), one announcing every
  kind is listed on every item with a path, one announcing only `album` only on albums, one with no announcement or
  another draft nowhere; each receives the right crosslink, and leaves the menus when switched off or removed; on the
  Mac, Show in Another App… sends to the one app offered, and asks among several. Flatpak impact: from inside a Flatpak the installed apps
  cannot be listed (the OpenURI portal only offers a chooser), so a Flatpak build would offer Show in… through it.

**Agreed with Immich Gallery, and built** (2026-11-10, in the protocol's clarified draft 0.2): a switch for
Open in Immich and Copy Immich Link apart from Immich Crosslink (`web.links`, under Immich Web App), and apps that
come and go: listed for 30 days after they were last found. Verified in the Fedora 44 GNOME and Plasma VMs (web links
off left only the Show in items; an uninstalled test app was listed, Forget removed it with its switch, and it came
back new and on), and the Python extension on Bazzite. On the Mac too: with web links off, `fileproviderctl evaluate`
refused Open and Copy and offered Show in; a removed test app was listed and Forget removed it. **Agreed next**
(2026-11-11, in the protocol): an app is found only when it can be read and started, one not found reads Not found
and keeps Forget, with a description of what the switches and Forget do; the link reads Learn more…; the Mac's settings get an Other Apps pane in Immich Gallery's
order; no app is set apart. Built; the desktop VMs' check of the new rows is left.
**Verified with Immich Gallery** on the maintainer's Mac: the Gallery's Show in Immich Mount
reached the installed release, which selected an album and a photo in an album in Finder.

### Step 4: Immich Gallery takes crosslinks (upstream)

Not Immich Mount's work; listed because steps 2 and 3 meet it. Proposed to IGOLz/ImmichGallery as an issue and a pull
request, as the maintainer contributes there ("Refs #N", never "Closes"): the gallery announces itself and registers
`immich-crosslink`,
opens what a link names (choosing the signed-in account for its server, or saying it has none), offers Show in *app*
for the apps it finds, with the protocol's **Immich Crosslink** switch and link in its settings (on by default, a
switch per app), and Copy Link. Immich Mount is
then one of the apps it finds, and Show in Immich Mount in the gallery is step 2.

### Open, to settle on each desktop
- **Mac (step 3):** how soon Finder offers or drops an action after an app is installed or removed: the menu bar app
  re-reads the items when it sees an app launch, which an install alone does not do.

## Package repositories

The real fix for updates on Linux (ARCHITECTURE.md, "Updates on Linux"): with a repository behind the packages, GNOME
Software and Discover offer each release like any other update, and the daemon's own look can stop for them.
1. **COPR for Fedora** (and Fedora's atomic desktops through `rpm-ostree`), built from the release's rpms or its
   sources. Needs the maintainer's Fedora account and a COPR project.
2. **An apt repository for Ubuntu,** signed, served beside the download page.
3. **Then:** packages from a repository carry a marker the daemon reads, and do not look for releases themselves.
4. **One package, `immich-mount`, on every distribution,** with both interfaces and both toolkits (GTK 4 and
   libadwaita, Qt 6 and Kirigami) as real dependencies, in place of `immich-mount-gnome` and `immich-mount-kde`;
   the launcher already picks the interface at startup, as the cask does. Compatibility comes before size: it
   must install and run the same on every distribution and version, never relying on what a desktop happens to
   ship, and about 100 MB more where the other toolkit is missing is accepted. The old package names become
   transitional (rpm `Obsoletes`, deb `Replaces`), so an upgrade moves to the one package.

## Watch upstream

- **Blueprint and GTK 5 interface definitions.** The GNOME app's screens are written in Blueprint, which still
  calls itself experimental. Follow its releases, and GTK's GTK 5 planning on how interfaces are defined (an
  intermediate format was floated in April 2026). Pin the Blueprint version in the build, and migrate deliberately
  when the direction settles.
- **zbus 6 and oo7 0.7.** zbus 6 folds `zvariant` and `zbus_names` into itself, and RustSec marks the standalone
  crates unmaintained (RUSTSEC-2026-0370, -0371, -0372; informational, no vulnerability). The keyring goes through
  oo7, whose zbus 6 release (0.7) is in beta; move both together once oo7 0.7 is released, so one zbus is linked.
- **fuser and `FUSE_REQUEST_TIMEOUT`.** Once fuser can set the request timeout in its init reply, the daemon asks
  the kernel for a long one instead of shortening its own waits (ARCHITECTURE.md, "Uploads").

## Open questions

- **Partner-shared Assets:** left out for now, because they belong to someone else's timeline. A `Partners/`
  section?
- **A `CACHEDIR.TAG` at the mount's root,** so restic, borg and tar with `--exclude-caches` skip it. It helps
  backups, but users would see an odd file in their Immich folder. Lean: yes.
- **Trusting a self-signed server certificate** from the app, for homelabs without a private CA in the system store.

## Performance

Where opening a folder stands, measured on Bazzite 44 GNOME against the lab server, a month of 450 photos the daemon
had not seen (`lab/bazzite-vm/freeze.sh`; the second figure with 20 ms added to every request):
- The folder appears in 0.5 s (1.3 s): its listing, then its small thumbnails seeded.
- Files' main thread is busy for 0.4 to 1 s, and responsive again 2 to 2.5 s after the click.
- rom-properties, which Bazzite's Files loads, gets through the folder in 1.5 s (1.9 s); with every file's edges
  already on disk it takes 0.7 s (0.9 s), the most any prefetch of originals could give.

These are the options for when that is not enough on a real server, with what each gains and costs
(ARCHITECTURE, "Content" and "Thumbnails", has the measurements behind them).

| Option | What it gains | What it costs |
|---|---|---|
| **Principle 2 and the edge prefetch** (decide) | Keeping the prefetch keeps rom-properties' pass at 1.5 s (1.9 s); without it, 3.4 s (12.9 s), growing with the files and the round trip | It fetches the start and end of files a program has not opened yet, though it reads them within seconds: the same bytes either way, unless the folder is left early. Amend the principle to allow that, or drop the prefetch |
| **Start the edge prefetch when a folder is listed,** once a program was seen peeking in this session | Most of what is left of rom-properties' pass (1.5 s towards 0.7 s), since the edges come while the folder's thumbnails are seeded; not measured | The same bytes as today, fetched earlier; a folder left at once still gets its edges, up to 80 MB |
| **Edges for counted subfolders** | The best case (0.7 s) on the next click | 80 KB for every file of every subfolder shown: opening a year of 3,000 photos fetches 240 MB, most of it for folders never opened. Against principle 2 |
| **Originals ahead,** a photo's neighbours when one is opened | Little: image viewers load the next and previous photo themselves | 2 to 10 MB a photo, gigabytes a folder: a mirror, against principle 2 |
| **Prefetching thumbnails beyond what file managers list** | Folders two levels down open as fast as the next click does | The folders one level below are seeded ahead already, since Files and Dolphin list them to count their items; going further fetches thumbnails for folders nobody shows, multiplied by People and Places |
| **Thumbhash placeholders** | A folder appears after a short wait whatever the server: Immich sends a thumbhash with every Asset, a blurred image in a few dozen bytes, and a seed made from it where the real one is late records no failure | A new dependency, and blurs instead of thumbnails on the first visit of a slow folder until the next. Seeding takes 0.5 s (1.1 s) for 450 photos |
| **More small-thumbnail fetches at once** (12) | The folder's 0.5 s (1.3 s) on a slow link: the lab server answers 500 small requests a second eight at a time, 145 one at a time | More load on the server, whose limits differ from the lab's |
| **Kept listings at once after a restart** | A folder's first visit after a login answers without waiting for its listing | It would need to know which kept listings changed while the daemon was down: the first sync stream of a run only sets the baseline, and a listing the change poll forgot must never come back from the store |
| **The daemon's copy of Immich's thumbnails in `cache.limit`** | Bounded disk: at 30,000 photos the copy holds 13 GB (17 KB thumbnail and 415 KB preview each), and nothing evicts it before signing out | An evicted preview is fetched again when its folder is seeded again |
| **Sharp thumbnails and GNOME's 512 MB thumbnail cache** | Revisits without seeding again | A sharp seed is 1.1 MB, written twice where home is a symlink: GNOME's housekeeping keeps about 200 photos' worth and deletes the rest, which the daemon writes again on the next visit (CPU, no download). Default PNG compression makes seeds 16% smaller for 30 times the CPU; sharp thumbnails off cuts a photo's seeds to 0.29 MB |

## Parked on purpose

| Idea | Why it waits |
|---|---|
| **Flatpak** | It needs host-command access for FUSE (the Pika Backup pattern), and Flathub acceptance is uncertain. When it comes: our own repository first, then Flathub. The design follows the Flatpak readiness rules in ARCHITECTURE until then |
| **A sealed headless credential** | Without a keyring the credential is a 0600 file, as SSH keys and most CLIs keep theirs. Encrypting it with a key kept beside it would add nothing; `systemd-creds encrypt --user` could seal it to the machine (and its TPM), so disk copies and backups never carry it readable |
| **Offline albums** | Pin chosen albums in the cache, showing the size first. Never the whole library |
| **A loopback sign-in redirect** | The RFC 8252 norm, but the admin must register it at the identity provider. An option for servers without mobile sign-in configured |
| **GNOME Shell search provider** | Immich smart search in the Activities overview: a Linux-only extra |
| **Nautilus extension, beyond its links** | More menu items in Files, once Open in Immich and Show in *app* (above) bring the extension |
| **Several servers or accounts** | The cache already separates accounts, so this is additive |
| **"Move to Trash" mapped to Immich's trash** | It makes a library-wide change one keystroke away in a file manager. Needs careful thought first |
| **Folder names in the user's language** | The interface is translated (ARCHITECTURE, "Translations"), the folders are not: a section's or a month's name is part of the mount's contract (SPEC, "Names"), and a path that changes with the desktop's language breaks scripts, links and bookmarks |
| **A Mac in the lab** | A Mac build server would run the Mac's checks unattended, as the Linux VMs do, and take its README pictures against the lab server, which come from the maintainer's own library until then (DESIGN.md, "Screenshots") |
| **More languages** | Next by reach: Persian, Swahili, Tagalog, Punjabi and Hausa, once someone asks or offers to check them |
| **The websocket for `all` API keys** | Every credential is polled every 10 seconds, so changes show within seconds already; the websocket would shave those seconds for one kind of key, at the price of a socket.io client and a second connection |
| **fanotify pre-content events** | Real placeholder files, the closest Linux match to macOS and Windows. They need root today |

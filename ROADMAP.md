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
release since 2.14.2; the milestone closes when the exits below are met. Work that needs no Mac is done on Linux and
checked by CI's macOS job; the maintainer's Mac is needed only to run and debug it.

1. **The first slice.** The bridge crate (`immich-mount-apple`) and the item identifiers; the extension lists
   folders, fetches originals and answers thumbnails; the app signs in with an API key, reaches the server first
   (the Local Network prompt) and adds the Finder location; CI builds the whole Mac side on every branch push. A
   skill for driving the Mac over SSH, written in the first session on it.
   **Built, and nearly met (2026-11-06)** on the maintainer's Mac (macOS 27.0.1, Xcode 27.2), against the lab
   server through a loopback tunnel (`lab/mac/check.sh`; ARCHITECTURE, "The macOS edition", "Built so far"): every
   section browses in Finder, originals read back with the server's checksums, and thumbnails show with no
   original downloaded; videos too. Not yet: the lab server in LAN mode, which brings the Local Network prompt.
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
   Mac, key and password sign-in add the location (single sign-on not yet tried there). The Settings window, which the extension
   follows. Status from the extension, through a file. The icon, the same drive on macOS's rounded square
   (ARCHITECTURE, "The icon"). Finder's sidebar shows the location under the app's name, with the drive symbol
   (ARCHITECTURE, "The macOS edition", "Settings"). Not yet: single sign-on on a Mac.
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

- **More sections:**
  - Partner-shared Assets: left out for now, because they belong to someone else's timeline. A `Partners/` section?
  - Favorites, which ImmichDrive (Windows) and immichfs both offer
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
| **Nautilus extension** | "Show in Immich" and similar menu items |
| **Several servers or accounts** | The cache already separates accounts, so this is additive |
| **"Move to Trash" mapped to Immich's trash** | It makes a library-wide change one keystroke away in a file manager. Needs careful thought first |
| **Tray icon** | Stock GNOME has none. Possible later as a StatusNotifierItem, for desktops with AppIndicator |
| **Folder names in the user's language** | The interface is translated (ARCHITECTURE, "Translations"), the folders are not: a section's or a month's name is part of the mount's contract (SPEC, "Names"), and a path that changes with the desktop's language breaks scripts, links and bookmarks |
| **More languages** | Next by reach: Persian, Swahili, Tagalog, Punjabi and Hausa, once someone asks or offers to check them |
| **The websocket for `all` API keys** | Every credential is polled every 10 seconds, so changes show within seconds already; the websocket would shave those seconds for one kind of key, at the price of a socket.io client and a second connection |
| **fanotify pre-content events** | Real placeholder files, the closest Linux match to macOS and Windows. They need root today |

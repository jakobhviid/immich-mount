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
  - Favorites and Archive, which ImmichDrive (Windows) and immichfs both offer
- **A `CACHEDIR.TAG` at the mount's root,** so restic, borg and tar with `--exclude-caches` skip it. It helps
  backups, but users would see an odd file in their Immich folder. Lean: yes.
- **Trusting a self-signed server certificate** from the app, for homelabs without a private CA in the system store.

## Parked on purpose

| Idea | Why it waits |
|---|---|
| **Flatpak** | It needs host-command access for FUSE (the Pika Backup pattern), and Flathub acceptance is uncertain. When it comes: our own repository first, then Flathub. The design follows the Flatpak readiness rules in ARCHITECTURE until then |
| **macOS edition** | A File Provider extension and a SwiftUI app on the core through `uniffi`. It needs the maintainer (a Mac, signing, notarization) |
| **A sealed headless credential** | Without a keyring the credential is a 0600 file, as SSH keys and most CLIs keep theirs. Encrypting it with a key kept beside it would add nothing; `systemd-creds encrypt --user` could seal it to the machine (and its TPM), so disk copies and backups never carry it readable |
| **Offline albums** | Pin chosen albums in the cache, showing the size first. Never the whole library |
| **A loopback sign-in redirect** | The RFC 8252 norm, but the admin must register it at the identity provider. An option for servers without mobile sign-in configured |
| **GNOME Shell search provider** | Immich smart search in the Activities overview: a Linux-only extra |
| **Nautilus extension** | "Show in Immich" and similar menu items |
| **Several servers or accounts** | The cache already separates accounts, so this is additive |
| **"Move to Trash" mapped to Immich's trash** | It makes a library-wide change one keystroke away in a file manager. Needs careful thought first |
| **Tray icon** | Stock GNOME has none. Possible later as a StatusNotifierItem, for desktops with AppIndicator |
| **Localisation** | Month folder names and the app's interface |
| **Prefetching the folders one level below** | Seeding before a listing answers already makes a first visit fast (600 files in under half a second); a prefetch would fetch thumbnails for folders never opened, multiplied by People and Places |
| **The websocket for `all` API keys** | Every credential is polled every 10 seconds, so changes show within seconds already; the websocket would shave those seconds for one kind of key, at the price of a socket.io client and a second connection |
| **fanotify pre-content events** | Real placeholder files, the closest Linux match to macOS and Windows. They need root today |

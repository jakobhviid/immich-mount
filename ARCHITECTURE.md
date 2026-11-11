# ARCHITECTURE

How Immich Mount is built and why. [SPEC.md](SPEC.md) is the contract (what it does), and
[PRINCIPLES.md](PRINCIPLES.md) holds the invariants. This file is the model and the reasoning: every design choice
is stated with its reason, and where a question tends to come back, the alternative that lost is named with the
reason it lost.

A claim marked *verified* was observed on a real system: Fedora 44 VMs with GNOME 50 (Files 50.4) or KDE Plasma 6.8
(Dolphin 26.08), against Immich v3.3.5, and v3.2.0 for the version floor. AGENTS.md, "Testing on Erebus", describes
that lab.

## What Immich Mount is

A feature for Immich, not a product of its own. It shows an Immich library as a folder in the file manager and in
every file dialog, with server thumbnails, and downloads originals only when something reads them. Files copied
into the timeline are uploaded.

It is inspired by [Kartax/immich-desktop-app](https://github.com/Kartax/immich-desktop-app), which does the same on
macOS through Apple's File Provider.

Order of platforms:
1. **GNOME** (Files, which is Nautilus) first.
2. **KDE** (Dolphin) next.
3. **macOS** third, through Apple's File Provider: being built (below, "The macOS edition"; ROADMAP, M7). Signing
   and notarization need the maintainer's Apple developer account.

There is no Windows edition; a separate project, ImmichDrive, already covers Windows with the Cloud Files API.

The library is never mirrored. Libraries run to 300 GB and more, and downloading them is never acceptable.

**Not its job:**
- syncing
- editing, renaming, moving or deleting Assets through the file manager
- managing albums
- a gallery, or anything else that duplicates Immich's own web and mobile apps: browsing photo by photo, search,
  faces, sharing and editing stay there

## Why FUSE

The library has to appear as real files to Files, to every file dialog, to sandboxed apps through the portals, and
to plain programs, with content arriving on demand. On Linux that is FUSE:
- **Every program sees real paths.** Flatpak apps reach files through the document portal, which is itself a FUSE
  filesystem.
- **No privileges, and no GNOME release cycle.** An unprivileged per-user daemon mounts through `fusermount3`, and
  the same mount works in KDE and elsewhere.
- **The kernel side keeps improving:** passthrough, io_uring transport and request timeouts arrived recently.
- **The cost:** we own what the OS owns on macOS: the content cache, thumbnails and freshness.

Not chosen (also in GNOME integration):
- a GNOME Online Accounts provider with a GVfs backend
- a Nautilus extension
- fanotify pre-content events
- a sync folder

## The name

**Immich Mount.** The file manager shows the volume as **Immich**, and our own interface (launcher, windows,
notifications, packages) says **Immich Mount**.
- **It is a feature for Immich, not a brand** (principle 11). "Mount" is the established word for making storage
  appear in the file manager, on Linux and on macOS.
- **The README and the repository's topics carry the search words** people will use: Immich drive, mount, FUSE,
  Nautilus, Files, Dolphin, on-demand, network drive. Web searches for "immich mount" are crowded with guides on
  mounting storage for the server, so "drive" must be there too.

**Not chosen:**
- **"Immich Desktop":** the name of Kartax's macOS app, and hdmain/immich-desktop already ships `immich-desktop`
  packages on Snap and AUR.
- **"Immich Drive":** the best consumer word, but a Windows project already uses it ("ImmichDrive", "Drive for
  Immich" in the Microsoft Store).
- **Plain "Immich" as the app name:** it would look official, and could collide with an official desktop app or
  package later.
- **A brand of our own** (photography or media words, plus "for Immich"): it would pull the spotlight from Immich.
- `immichfs` and `immich-fuse` were taken by small Linux projects too.

## The icon

**A drive with a camera lens whose ring carries Immich's five colours** (`packaging/icons/`): related to Immich by
its colours, never by its shapes. It is not Immich's flower logo, which would make the drive look official
(principle 11), and nothing of Kartax's app icon. It is drawn for this project as hand-written SVG, in GNOME's app
icon style, and is MIT like the rest.
- **One name for both apps,** `dev.hviid.ImmichMount`, with a symbolic variant for notifications. Both launchers,
  both About pages, the apps' welcome and status pages, and Dolphin's place for the mount use it.
- **Shipped in the `immich-mount` package,** which both app packages depend on: dpkg refuses one file in two
  packages, and both apps may be installed side by side.
- **Software centres** take the icon from the launcher entry, so the AppStream files name none (with one, the
  validator wants the categories repeated there too).
- **The Files sidebar** keeps its own drive symbol: it draws mounts with the theme's symbolic icons, whatever the
  mount supplies.
- **Flatpak impact:** a Flatpak exports icons named after its own app ID, `dev.hviid.ImmichMount`, so it ships the
  icon as it is.
- **On macOS** the drive is the whole icon, in the rounded square macOS 26 expects of an app icon (other shapes are
  shown inside a grey one): its face and base fill the square on Apple's 1024 grid, the base along the bottom edge,
  with the drawing's own lens, light and gradients. A tile behind the drive was tried first; its pale ground left the
  white drive without contrast. `apple/make-icons.sh` builds it from the SVG and renders the asset catalog's sizes
  with `rsvg-convert`; the PNGs are committed, since the Mac has no SVG
  renderer (QuickLook draws the SVG at its own size, not to a given one). The app and the extension both carry it:
  the Dock, Login Items, System Settings' list of file providers and the About panel show it. The app's window shows
  the drawing itself over its title, as the README and the GNOME app's welcome and status pages do: the SVG, kept as
  a vector in the asset catalog. The About panel has the GNOME About dialog's texts: the non-affiliation notice as
  its copyright line, then the website, the source code, the credits and the AI disclosure.
- **On macOS 26 and later** the icon is an Icon Composer document, `apple/App/AppIcon.icon`, as Apple's guidelines
  ask, so it follows the light, dark, tinted and clear appearances and takes the system's glass and lighting. The
  same drive in layers: the face is the document's background (a light and a dark gradient), the base and its light
  are one group, and the lens another, its glass a glass layer. `apple/make-icon-composer.py` writes it from the SVG,
  with SVG layers, which Icon Composer reads as they are; Xcode's `ictool` renders it to check each appearance. Apple
  publishes no schema of `icon.json`: the keys follow Xcode's own template and documents saved by Icon Composer, as
  reconstructed by community references. The asset catalog's AppIcon set, of the same name, stays for macOS 14 and
  15; actool compiles both (verified with Xcode 27.2: the catalog holds the layered stacks and the flat sizes).
- **Finder's sidebar** shows each location with a one-colour symbol, which the provider names in its extension's
  `CFBundleSymbolName` and ships in its asset catalog, as Nextcloud and OneDrive do (without one, Finder draws a
  generic drive or folder). Ours is the macOS app icon in outline, as Finder's sidebar draws its symbols: the drive
  filling a rounded square, the line where its face meets the base, the base's light, and the lens as a ring in five
  segments, as the icon's ring carries five colours. `apple/make-symbol.py --square` writes it as an SF Symbols
  template (v6.0) with its three source weights made from one geometry, so they interpolate; without `--square` it
  draws the drive's own wider shape, which the sidebar showed first.
- **The menu bar item** shows the macOS app icon as one solid shape, as menu bar icons are drawn (Finder's sidebar
  draws outlines): the drive filling a rounded square, with the five-segment lens ring, its centre, the line where
  the face meets the base and the light cut out. `apple/make-menu-bar-icon.py` writes it as a template image at
  menu bar icons' size, 18 points with a 16-point glyph, rather than a symbol that takes the text's size, which came
  out smaller than its neighbours. Together with the sidebar's outline and the Icon Composer icon, the rounded square
  gives the Mac edition a look of its own on the Mac.

## Two processes: a daemon and an app

```
 Files, file dialogs, players ──▶ kernel FUSE (~/Immich, shown as "Immich Mount")
                                         │ /dev/fuse
 ┌───────────────────────────────────────▼─────────────────────────────────────────┐
 │ immich-mount daemon      systemd user service, no display needed                │
 │   FUSE adapter ─▶ tree (names, inodes) ─▶ metadata store (redb)                  │
 │        ├─▶ content cache (blocks, range reads)    thumbnail seeder ─▶ ~/.cache/thumbnails
 │        └─▶ upload staging ─▶ uploader                                            │
 │   change feed (sync stream / polling)   D-Bus service   notifications            │
 │   immich-api (reqwest, tokio) ──────────────────────────────────────────────────┼─▶ Immich
 └───────────────────────────────────────▲─────────────────────────────────────────┘
                                         │ session D-Bus
                     immich-mount-gnome (GTK 4 + libadwaita): sign-in, settings, status
                     immich-mount-kde (Qt 6 + Kirigami): the same, for Plasma
```

**The daemon** (`immich-mount daemon`) owns the mount, the metadata store, the caches, thumbnail seeding and
uploads. **The app** (`immich-mount-gnome` on GNOME, `immich-mount-kde` on Plasma) is a normal desktop application:
it signs in, edits settings and shows status. Closing it never stops anything.

Why two processes:
- **A stalled FUSE process freezes everything** that touches the mount (Files, file dialogs, shells), and a crashed
  one leaves a dead mount ("Transport endpoint is not connected") until someone unmounts it.
- **The mount must not depend on a display or on GTK.** The daemon runs headless (test VMs, development
  hosts without a desktop, servers) and ships as a static musl binary.
- **A UI crash cannot take the mount down.**
- **GNOME has no menu bar or tray** to host a long-running app that outlives its windows, so a user service takes
  that role.

The cost is a D-Bus interface (SPEC.md), which also gives the CLI for free.

**Not chosen:** one process, where the GTK app keeps running in the background with the mount inside it. It would
put the mount at the mercy of a display session and a toolkit, in the one process that must never hang.

## Crates: a portable core with platform adapters

| Crate | Portable | Kind | Holds |
|---|---|---|---|
| `immich-api` | yes | lib | The Immich REST client and models |
| `immich-mount-core` | yes | lib | The library model, naming, membership, metadata store and its change journal, content cache, upload logic, settings table, config |
| `immich-mount-linux` | Linux | lib | FUSE adapter (inode table, kernel invalidation, staging), freedesktop thumbnail seeding, Secret Service, notifications, D-Bus service |
| `immich-mount` | Linux | bin | Thin CLI plus the `daemon` subcommand: resolve inputs, call one core function, render the result |
| `immich-mount-desktop` | Linux desktops | lib | What the desktop apps share, without a toolkit: checking a server, sign-in, the credential, first-run setup, status over D-Bus, the preferences' groups and labels |
| `immich-mount-gnome` | GNOME | bin | The GTK 4 / libadwaita app over `immich-mount-desktop`; talks to the daemon over D-Bus |
| `immich-mount-kde` | KDE Plasma | bin | The Qt 6 / Kirigami app over `immich-mount-desktop`: QML pages and one Rust object through cxx-qt |
| `immich-mount-apple` | yes (used on macOS) | lib | The bridge the macOS edition's Swift code calls through uniffi: items by identifier, listings, whole originals, thumbnails, the server check |

**Why a portable core:** each platform has its own native way to show a remote library as files.

| Platform | Mechanism | Who caches content | Identity |
|---|---|---|---|
| Linux | FUSE | We do | Inode numbers handed out per mount |
| macOS | File Provider | The OS | String item identifiers, sync anchors |

If the logic were written in FUSE terms (inodes, `readdir`, `flush`), a macOS edition would be a rewrite. So the
core speaks an identifier-based model:
- stable `NodeKey` identities, serialisable to strings
- listings with final names
- attributes
- byte-range reads
- thumbnail bytes
- uploads with duplicate semantics
- changes as the folders they touch (`LibraryChanges`): Linux forgets those listings, and the macOS edition turns
  them into per-item records with sequence numbers, its sync anchors ("The macOS edition")

OS services (secrets, notifications, directories, the clock) come in through traits.

**What keeps the boundary honest:**
- CI compiles the core for `aarch64-apple-darwin`.
- The core's integration tests run through a small in-memory adapter, so the core always has a consumer that is not
  FUSE.

**Key dependencies:**

| Area | Crates |
|---|---|
| Core | `tokio`, `reqwest` (rustls with the system's root certificates), `serde`, `redb` (pure Rust), `sha1`, `toml` + `toml_edit`, `tracing`, `thiserror` |
| Linux adapter | `fuser` (pure Rust, mounts through `fusermount3` with no libfuse), `zbus`, `oo7`, `image` + `png` |
| CLI | `clap` (with `clap_complete` and `clap_mangen`), `anyhow` |
| GNOME app | `gtk4`, `libadwaita` |
| macOS bridge | `uniffi`, `oslog` (macOS only, for the unified log) |

`fuser` calls the `Filesystem` trait synchronously; answers that need the network move the reply object into a
tokio task. Its async-native alternative, `fuse3`, is less used. Switch only if that reply pattern gets ugly.

**macOS:** a File Provider extension and a SwiftUI app on this core, through `uniffi` bindings ("The macOS
edition").

**Error types:** the core uses typed errors (`thiserror`), because the FUSE adapter must map each kind (offline,
signed out, gone, transient, protocol) to an error code. `anyhow` with context is used at the edges, in the CLI and
the daemon's wiring.

## The daemon's lifecycle

1. **Start:**
   - Read the config and the credential. A locked keyring brings up the system's unlock prompt.
   - A stale mount left by a crash is detached first (`fusermount3 -uz`). Staleness is asked with `statfs`, which
     always reaches the daemon: `stat` of the root is answered from the kernel's attribute cache for a minute, so a
     daemon restarted after a crash took the dead mount for another daemon's and stopped (seen on Bazzite).
   - The server's version is asked with its own limit of 10 s, not the client's retries (four minutes when the
     server drops requests, with the mount point an empty folder all along).
   - Mount, and answer from the metadata store at once. The server connection comes up in the background.
     *(Built so far: the daemon asks the server for its version and media types first, up to about ten seconds each
     when it does not answer, then mounts; listings kept in the metadata store answer offline.)*
2. **Server unreachable:** stay mounted. Serve cached listings and content, fail uncached reads fast, notify once.
3. **Eject from the file manager:** respected. The daemon notices within two seconds and exits cleanly. The
   user service restarts only on failure, so the next login (or `systemctl --user start immich-mount`) mounts again.
   The app's status page offers **Mount** then, and **Unmount** while mounted, which matters where Files shows a
   bookmark with no eject button; both run the command line's `mount` and `unmount`, so the app and a terminal take
   the same steps.
4. **Server or account change:** refused while uploads are staged or in flight. Otherwise unmount, switch to the
   new account's cache namespace, and remount. Old namespaces are deleted.
5. **Stop** (logout, `systemctl --user stop`): finish or cancel in-flight work, then unmount cleanly.
6. **Upgrade and uninstall.** Homebrew, rpm and dpkg replace a program by putting a new file in its place, which
   Linux allows while the old one runs: nothing blocks, and the running daemon keeps its old file until it exits.
   Left alone it would serve the old version against the new app until the next login, and after an uninstall keep
   the mount up and its service enabled, failing at every login. So the daemon looks every 30 seconds at the path it
   was started from (`argv[0]`, which systemd gives absolute):
   - **A different file there** is an upgrade. The daemon leaves once nothing on the mount is open or uploading
     (it counts open library files, and asks the staging area), with status 75; the unit's
     `RestartForceExitStatus=75` starts the new version, and `SuccessExitStatus=75` keeps it from counting as a
     failure (a unit written before these lines restarts it all the same, through `Restart=on-failure`).
   - **Nothing there for two minutes** is an uninstall; a Homebrew upgrade removes the old version before it adds
     the new one, hence the grace. The daemon leaves once idle and disables the unit it runs in, named by its own
     control group, so it touches no other unit. It also takes away what points at a mount that will not come back,
     as `logout` does: the place in Dolphin, the sidebar bookmark, the GVfs drop-in and the seeded thumbnails (small
     copies of private photos in a shared folder). The credential and the caches stay for `logout` or a zap: an
     uninstall may be a reinstall.
   - **Verified** on the development host with a transient user unit: a file held open on the mount held the
     restart until it closed, then the new binary mounted about 9 seconds later; with the binary removed, the daemon
     unmounted after the grace and was not restarted.
   - **Not chosen: restarting from the packages** (`%systemd_user_postun_with_restart`, a postinst, a cask
     postflight). It restarts at once, with a copy into Upload in flight or a video open, and needs three channels
     to get it right, Homebrew's steps running in a sandbox. **Flatpak impact:** none; a Flatpak would update the
     app, while the daemon and this check stay on the host.

**The user service** is the packages' `immich-mount.service` (the .rpm, the .deb and the tarball install it), or
Homebrew's `homebrew.immich-mount-headless.service`, which `brew services start immich-mount-headless` writes. The
apps' first-run setup, `immich-mount mount` and the `mount.at_login` switch use the packages' unit where it is
installed, else Homebrew's, so nobody starts a service by hand; `logout` turns off both. Where no service is
installed at all (Homebrew's cask and formula, a build from source), the first of them to need it writes the
packages' unit into `~/.config/systemd/user`, running the daemon it finds, and starts it. It looks first in the
`bin` of the Homebrew prefix it runs from, whose links survive an upgrade, never in one version's folder. A daemon
that finds the mount already served by another (both services on) leaves cleanly, so its service is not restarted
every few seconds. **Flatpak impact:** a Flatpak app could not write that unit; it would rely on the host's daemon
and its service.

## The tree

### Identity and inodes

```rust
enum NodeKey {
    Root, Timeline, TimelineYear(Year), TimelineMonth(YearMonth), TimelineDay(Date),
    AlbumsRoot, Album(AlbumId), PeopleRoot, Person(PersonId),
    PlacesRoot, Country(String), City { country: String, city: String },
    DateGroupYear { base: Box<NodeKey>, year: Year },
    DateGroupMonth { base: Box<NodeKey>, month: YearMonth },
    Asset { parent: Box<NodeKey>, asset: AssetId },   // one per Appearance
    Staging(StagingId), Virtual(VirtualFile),   // a receipt is a staged file after its upload
}
```

- **One node per Appearance.** The parent is part of the identity, so one Asset can live in its month, its albums,
  its persons and its city at once.
- **Shared content:** Appearances of one Asset share the content cache, keyed by asset id.
- **Inodes:** the Linux adapter keeps a two-way table between `NodeKey` and inode, hands out inode numbers in
  sequence for the life of a mount, and drops entries when the kernel's `forget` brings their lookup count to zero.
  Inode numbers are not stable across remounts; FUSE does not need them to be. Each mount uses a non-zero
  `generation`.
- **No hard links:** `st_nlink` is 1. Copy tools would otherwise deduplicate Appearances, and "delete one link" has
  no meaning here.

### Membership comes from Immich

Folder membership is whatever Immich's own endpoints say (the table in SPEC.md). The month folders use the same
timeline buckets the Immich web app uses, and albums, people and cities hold exactly the Assets Immich's searches
return for them.

Why: Immich groups the timeline by **local** capture time, while its search filters on **UTC** `fileCreatedAt`.
Recomputing months from search results puts a photo taken at 00:30 on 1 March in a UTC+2 zone into February,
while Immich's timeline shows it in March. This was verified on v3.3.5.
Any grouping logic we write ourselves is a chance to disagree with Immich; using its answers means following it
automatically.

Date groups inside albums, people and cities are computed from the folder's own Asset list, by each Asset's
`localDateTime`. That gives exactly what the timeline's `albumId` and `personId` buckets return (verified on v3.2.0:
the ids match), without a second round of requests, and cities have no timeline filter anyway. Day folders in busy months
work the same way: Immich's web timeline draws its day headers from `localDateTime`. That is Immich's rule, applied
to Immich's values.

**Not chosen:** months from the first seven characters of `fileCreatedAt` with UTC search ranges, which is wrong
at month boundaries.

### Favorites and the archive

Favorites and Archive are sections like People: one search each, fetched in pages of 1,000, whose year and month
folders reuse the one list. Each asks what Immich's own page asks for: Favorites `isFavorite` with no visibility
filter, so archived favorites are among them, and Archive `visibility: archive`. A read-only API key may ask for
both: only `visibility {ne: archive}` needs elevated permission (verified on v3.3.5).
- **Off by default,** because many libraries hardly use them, and a folder nobody opens is clutter.
- **Date groups always** (`grouping.favorites`, `grouping.archive`): both mostly grow, and with `atLeast500` every
  path in one would change the day it reached 500 Assets.
- **The cost** is that a first visit waits for the whole list, since a listing must be complete: 5,000 archived
  photos are five requests, 30,000 are thirty. A large album costs the same.
- **Not chosen: Photos' layout,** from Immich's buckets (`isFavorite=true`, `visibility=archive`), a month at a
  time. It keeps a first visit cheap at any size, but needs its own folder kinds and change tracking per month;
  worth it only for tens of thousands of favorites or archived photos.

### Names

The rules are in SPEC.md. FUSE needs unique, stable names, and Immich provides neither: shots in the same second,
`IMG_0001.JPG` from two cameras, two people with the same name, and free-text album names all clash.

**Files are named by date and time by default** (`2024-03-15 14.32.07.jpg`):
- **The reasons:** camera file names (`IMG_1234`) are counters that Immich hardly shows. Date-time names sort
  chronologically in every folder, and copies carry meaningful names.
- **The time** is the local capture time, the same value Immich's timeline uses.
- **Dots instead of colons,** because colons are invalid on exFAT, NTFS and SMB, and Finder shows them as slashes.
  It is the convention Dropbox's camera uploads use.
- **Original names** remain a setting.

**Clashes get a counter, assigned in upload order and remembered** (`-1`, `-2`).
- The Asset uploaded first keeps the plain name, so a newly uploaded duplicate never takes an existing name.
- Assigned names live in the metadata store, so a counter never moves to another Asset while its holder exists.
  Deleting one leaves a gap. That keeps principle 8.
- A fresh installation computes the same names from server data, except where deletions have left gaps.
- Comparison is case-insensitive after NFC normalisation. So
  copies to exFAT, NTFS or macOS volumes do not collide, and an NFD name from an iPhone matches its NFC twin.

**Not chosen:**
- **Milliseconds in the name.** Most cameras and all screenshots record only whole seconds, so bursts would still
  collide, and names would sometimes carry milliseconds and sometimes not.
- **Counters recomputed on every listing.** Deleting one Asset would renumber the rest, so a path could open a
  different photo than yesterday. Remembering the assignment is what makes counters safe.
- **An id suffix** (`IMG_0001 (3f9a2c1b).JPG`): stable, but noisy.
- **Leaving clashes to the platform.** On macOS the File Provider renames by arrival order, but
  FUSE has no such mechanism anyway.

### The metadata store

redb, a pure-Rust embedded database with crash-safe transactions, one database per account, holding:
- listings with their fetch times
- Assets and their Appearances, with the names assigned to them
- the change cursor

The seeded-thumbnail ledger is the exception: a plain list of URIs beside the store (`seeded-thumbnails`), appended
as seeds are written. `immich-mount logout` must read it to clean up, and a plain file can be read while the
daemon holds the database's lock, or after a crash left the database unreadable.

Why:
- **FUSE forgets everything at unmount.** On macOS, the File Provider framework keeps its own database of items,
  which is why Finder shows folders instantly after a reboot.
- **We have neither, so the store does both jobs.** Folders still open after a restart when the server cannot
  answer in time, listings work offline, assigned names stay stable, and writes are incremental. A folder's first
  visit after a restart still waits for the server when it answers (ROADMAP, "Performance": "Kept listings at once
  after a restart").

The store carries a format version and is migrated in code, with tests.

**Not chosen: SQLite** (`rusqlite` with its bundled C library). Everything stored is key-value shaped, and compiling
SQLite's C for every target would complicate the static builds and the macOS check of the portable core. SQLite's
advantage, inspecting the store with its command-line tool, matters less than a pure-Rust build.

### Freshness

The kernel caches whatever it is told to, for as long as it is told. inotify never sees changes that originate on
the server, so freshness is our job:

1. **Stale-while-revalidate.** A listing older than its window is answered at once, and fetched again in the
   background; the next visit shows the fresh one. A listing the change poll forgot, because Immich changed, is
   fetched before the folder answers, so a known change is never answered stale. Verified in the GNOME VM: with a
   1-second network delay, a month past its window listed in 5 ms.
   Nothing is pushed to the kernel (`notify_inval_entry` and the like): its timeouts are short (60 s), the daemon
   answers every `readdir` itself, and Files would not redraw on a push anyway, since it watches with inotify.

   | What | Window |
   |---|---|
   | Root, timeline, years | 5 min |
   | Current month | 1 min |
   | Older months | 1 h |
   | Album, person and place lists and contents | 10 min |
   | An Asset's attributes | 24 h |

   Kernel entry and attribute timeouts are 60 s for folders and files alike, so a name freed by a deletion and given
   to another Asset stops pointing at the old one quickly (principle 8).
   Files does not redraw an open folder when Immich changes: it watches folders with inotify, which sees nothing
   of a change made on the server, so F5 or re-entering shows it.
2. **The change feed** depends on the credential (verified on v3.3.5):
   - **A session** (browser or password sign-in) follows `/sync/stream` with acks. It gets exact, resumable deltas,
     including permanent deletes and album membership, so the mount is close to live.
   - **An API key with the `all` permission** can use the websocket (`/api/socket.io`, key in the `x-api-key`
     header), which sends `on_upload_success`, `on_asset_update`, `on_album_update`, `on_asset_trash`,
     `on_asset_restore` and `on_asset_delete`. Each event refreshes the folders it touches.
   - **A scoped API key** can only poll. ETags on `/timeline/buckets`, `/timeline/bucket` and `/albums` make that
     cheap: a 304 when nothing changed. `/sync/*` refuses every API key, and the websocket refuses scoped keys.
     Asset fields alone cannot track changes: adding an Asset to an album leaves its `updatedAt` alone, and a
     permanent delete leaves no trace but a missing id.

   The feed is chosen automatically. The freshness windows always stay on, as the safety net for connection gaps.

   **Built so far: ETag polling for every credential,** every 10 seconds: `/timeline/buckets` and `/albums` with
   `If-None-Match`, so an unchanged library costs two 304s. A month whose count changed, and an album whose count
   or update time changed, are forgotten, with the folders above them; the next visit (or F5 in Files) reads them
   again. Verified in the GNOME VM: an Asset added to an album, or archived out of a month, shows on the next poll.
   Swaps that keep a month's count are left to the freshness windows.

   **Shown Favorites and Archive are polled too:** `/timeline/buckets` with `isFavorite=true` or
   `visibility=archive`, and `If-None-Match`, one more 304 each while nothing changed, and only while the section is
   on. When a section's month counts change (something marked, archived, unmarked, unarchived, or trashed), its
   folders are forgotten. Without it, the only safe rule for the archive would be that any month change touches it,
   so every upload would read the whole archive again, at once on macOS, whose poll lists changed folders again
   itself; and marking a favorite changes no timeline count at all. A section switched on starts by taking note, as
   a first poll does. Verified against the lab server (the bridge's `tests/lab.rs`), and in a GNOME 50 VM through
   FUSE: a photo archived or marked in Immich showed in Archive or Favorites within 5 s, an archived favorite
   included, and left at the next poll when unmarked (a name looked up in the last minute stays in the kernel's
   cache for up to 60 s, as everywhere in the mount).

   **Sessions also follow the sync stream** in the same poll (`AssetsV2`, `AlbumToAssetsV1`): a changed Asset marks
   its month, album membership marks its album, and a permanent delete or a reset refreshes every folder, since its
   place is unknown. The first stream of a run only sets the baseline and is acked. On a large library that first
   stream is the whole library, so it is read over several polls: each reads it for at most 15 seconds, the baseline
   is acked every 5,000 records, and running out of time is not counted as being offline. Verified in the GNOME VM: a
   permanent delete arrived as `AssetDeleteV1` within one poll.

   **Not chosen, for now: the websocket for `all` keys.** With every credential polled every 10 seconds, a change
   already shows within seconds; the websocket would save those seconds for one kind of key, at the price of a
   socket.io client and a second connection to keep alive. Parked in ROADMAP.
3. **Our own uploads** invalidate the affected folders at once.

### Directory listings are complete

FUSE has no paging towards the kernel: a listing is complete or it is wrong. A large album is fetched in full
(about 1,000 per page) before its first `readdir` answers. Date groups keep the visible folders small, and the
metadata store makes later visits instant.

## Content: blocks fetched with range requests

Originals are fetched in 2 MiB blocks with HTTP range requests, and kept as one sparse file per Asset plus a block
map beside it (`<id>.blocks`, a line per block), which survives restarts:
- **Single flight:** each folder listing, content block and Asset's edges (below) is fetched once, however many FUSE
  calls wait for it, and an Asset's server thumbnail once, however many folders being seeded want it (then kept on
  disk by Asset id).
- **Read-ahead:** sequential reads (each starting where the last ended) widen a read-ahead window from 2 blocks up
  to 16 (32 MiB). Random access resets it. Read-ahead runs at lower priority than reads someone is waiting for
  (two fetches at once). It starts only after 4 MiB of sequential reading, so a program peeking at a header (a type
  sniffer, a thumbnailer) never pulls a block.
- **Peeks are served from the edges.** A peek is a read into bytes the cache does not hold, in a run of reads that
  stays within 64 KiB, of a file larger than that; past 64 KiB of run, reads fetch whole blocks as above. A peek
  within an original's first 64 KiB or last 16 KiB fetches both at once (two requests, 80 KB) and keeps them as the
  Asset's edges (`<id>.edges`, beside its blocks), which count towards `cache.limit`, are evicted with the Asset, and
  are dropped with its blocks when the original changes; any other peek fetches exactly its range and caches nothing.
  Bazzite's file managers load rom-properties (a plugin for game images; 2.9 on Bazzite 44), which reads every file of
  a folder they show from their own main thread, one after another: each file's first 8 KiB and its last partial page,
  which the kernel turns into three requests (0+16 KiB, the last page, and its own read-ahead of 32 KiB). Peeks
  fetching whole blocks fetched every original whole (5.7 MB for a month of 3 photos, 25 MB for a folder of 121,
  measured on Bazzite 44 with the kernel tracing reads). Fetched as they come, each a round trip on Files' main
  thread, a month of 450 photos made 1,350 requests and kept Files busy for 7 to 9 s (2.0.1, measured on Bazzite 44
  GNOME against the lab server, Files' main thread sampled over D-Bus; with rom-properties moved aside, 0.8 s). So
  once a program has peeked at three files of one folder within 10 s, the edges of the rest of the folder, up to 1,000
  files (80 MB), are fetched ahead of it, eight files at a time; it reads them from disk, and a single peek (a
  properties dialog) prefetches nothing. This is the one place the mount fetches bytes of files no program has opened
  yet, against the letter of principle 2: never an original, only the start and end of files in a folder that a
  program is reading one after another, which it reaches within seconds; whether the principle should say so is open
  (ROADMAP, "Performance"). Measured the same way: Files busy for 0.4 to 1 s and responsive again 2 to 2.5 s after the
  click, 900 requests. How long rom-properties takes over the month, traced in the kernel: with the edges fetched only
  as it reads, 3.4 s at the lab's latency and 12.9 s with 20 ms added; with the prefetch, 1.5 and 1.9 s; with every
  edge already on disk (the most any earlier prefetch could give), 0.7 and 0.9 s. rom-properties reads the files in an
  order of its own, not the listing's or the view's, so a peek at a file the prefetch has not reached fetches its
  edges itself. Edges survive restarts, so a folder seen before costs nothing the next time; they carry a checksum
  instead of being synced, since a torn one is merely fetched again. rom-properties after 2.9 counts unknown FUSE
  filesystems as remote and skips them; on KDE, `ShowDangerousPermissionsOverlayIcon=false` in its settings stops
  Dolphin's plugin already.
- **Crash safety:** a block counts as present only after it is written and synced, so a crash never serves zeros.
- **Validation:** the block map records the original's size and its weak ETag (`W/"<size>-<mtime>"`, sent with every
  206). Cached blocks are dropped, and fetched again from the new original, when the listing gives another size, when
  a fetched block carries another ETag, or when a range comes back shorter than the listed size allows; a read that
  overlapped the change reads again, and the folder it came from is listed again. One Asset's block writes take
  turns, so the check and the files change together, and the map is emptied and synced before the data file, so a
  crash never leaves old blocks listed over an emptied file. An original replaced at the same size whose blocks are
  all cached is not noticed until one is fetched again (after eviction), since nothing is asked of the server then.
  Verified against the lab server: the ETag is the same for every range of an original.
- **Page cache:** files are opened with `FOPEN_KEEP_CACHE`, so repeated reads never reach the daemon, unless the
  cache dropped the original's blocks since the last open: then the kernel's pages go too.
- **Eviction:** whole Assets, least recently used first, once the cache passes `cache.limit`, or once the disk is
  down to its last tenth, whichever comes first (the core asks the platform through a probe, `statvfs` on Linux, at
  most every 30 seconds: btrfs shows freed space only after a commit, and asking after every eviction would then
  evict ever more).
  An Asset being read is pinned and never evicted; an open file that is idle can be, and its next read fetches
  again. Offline albums, if built (ROADMAP), would be exempt. Server thumbnails (`server-thumbnails/`, a few
  hundred KB per Asset) are not counted.

**Trash checks.** `open` never waits. If an Asset's active state has not been checked recently, `open` starts a
background check, and the result applies to later reads.

**Concurrency** is bounded per kind of work: listings 4, thumbnails 8 to 16, content 4, uploads 2. Everything
shares one HTTP/2 connection pool. *(Built so far: thumbnails, 12 small for a folder being shown, 4 for folders
seeded ahead and 4 previews at once, each with its own budget so the background never holds up a listing;
listings started by probes, 4; edge prefetches, 8.)* The lab server answers 145 small range requests a second one
at a time and 500 eight at a time, so a burst of peeks is worth spreading; what held the first prefetches back was
the daemon's own CPU (Thumbnails, below), not the server.

Why blocks:
- **Many readers touch only part of a file:** content-type sniffing reads the first KB, EXIF readers read the
  header, and video players seek. A whole-file cache would download a multi-GB video because something wanted
  64 KB of it.
- **Range requests work on Immich v3.3.5**, byte-exact for images and videos, HEAD included.

Two quirks are handled:
- multi-range requests return the whole file, so only single ranges are sent
- a range past the end returns 404, not 416, which means our size is stale

`If-Range` works on the server, but HTTP forbids weak ETags in it, so it is not relied on. Whole-file download
remains only as the fallback for a server that refuses ranges.

**Never hang.** Every task that answers a FUSE request has a deadline:
- listings: about 10 s, then an older listing if there is one, else `EIO` (the fetch continues, and the next visit
  gets its result). A listing that waited that long and ran out marks the server silent for 30 s, and listings then
  wait at most 1 s: rom-properties (on Bazzite) looks for extracted game discs in every folder Files shows, from
  Files' main thread, and a server that drops requests froze all of Files for 3 minutes at 10 s a name. A probed
  name (SPEC, "Files the mount answers") waits for nothing: it is answered from the folder's listing at hand,
  however old, or else absent at once, not remembered, while the folder is listed in the background (four at a
  time, since a folder of hundreds of albums is probed album by album). Verified with the kernel-level mount tests,
  against a library whose listings never answer.
- reads: 30 s without progress, then `EIO`
- uploads on close: their own limit (Uploads)

The FUSE session thread never blocks. Handlers answer from memory, or move the reply object into a tokio task.

## Thumbnails

### The problem (verified)

On a FUSE mount, GNOME is aggressive (verified in a GNOME 50 VM):
- **GIO treats our mount as local.** It reports `filesystem::type fuse` (taken from the statfs magic, so a
  `fuse.immich` subtype is invisible) and `filesystem::remote FALSE`. Every `local-only` default applies.
- **Opening a folder makes `glycin-thumbnailer` read every file in it to the end, viewport or not:** 40 of 40 files
  and 99 MB while 16 were visible, and a 400-file folder within 7 seconds. Only files over the 50 MB
  `thumbnail-limit` are skipped.
- **A search typed in Files crawls the tree, and then thumbnails every hit.**

Without countermeasures, opening one month downloads all of it.

### Seeding the freedesktop cache

Nautilus never reads an original if a valid thumbnail exists, in any bucket. It uses the largest bucket present,
accepts PNGs smaller than the bucket as they are, and a valid thumbnail wins over a recorded failure. So:

1. **Before `readdir` answers** (budget 10 s; 8 to 16 fetches in parallel), every Appearance in the folder gets
   `x-large/` (512) from Immich's small `thumbnail`. That is a WebP with a short side of 250 px, so it fits without
   resampling, and costs 5 to 25 KB each. This alone stops every original from being read. A busy month of 1,000
   photos costs about 20 MB on its first visit, and nothing after.
2. **In the background,** with sharp thumbnails on by default, `xx-large/` (1024) is made from Immich's `preview`
   (40 to 520 KB each). Nautilus shows it the next time the folder is drawn.
   **On KDE** the same image also goes to `normal/` (128) and `large/` (256). Dolphin asks KIO for the bucket of its
   zoom level, `normal` at its default, and KIO starts its thumbnailer when that bucket is empty instead of taking
   a larger one (verified in a Plasma 6.8 VM). The daemon asks at each seeding: its environment, or Plasma's shell on
   the session bus, since at login a user service can start before Plasma hands over its environment.
3. **Server thumbnails are cached by asset id,** so writing them for more Appearances, or rewriting them after a
   name change, needs no network.
4. **A thumbnail is found by its URI,** so where home is reached through a symlink (Fedora's atomic desktops,
   Bazzite) each file is seeded under both paths a file manager reaches it by: the resolved one, which GVfs's drive
   opens (`/var/home/me/Immich`), and the configured one, which browsing from Home, the app's **Open in Files** and
   Dolphin's place use (`/home/me/Immich`). With only the first, every folder opened from Home showed generic icons
   (verified in a GNOME 50 VM with `/home` a link to `/var/home`); with both, its thumbnails show.
5. **Each server thumbnail is decoded once, and scaled once per bucket,** for every path it is written to; the PNG is
   compressed fast. At the default level the encoder took 100 of the 130 ms a 1024-pixel seed cost, and 2.0.1 spent
   about 40 s of CPU on a month of 450 photos with both paths, which slowed the file manager and the daemon's own
   answers on a 4-core VM (measured on Bazzite 44). Fast takes 3 ms, for a file 15 to 40 percent larger. Previews
   are rendered two at a time, so the sharp pass never takes every core while the file manager draws the folder.

**Counts are seeded ahead, not waited for.** Files and Dolphin count the items of every subfolder of a folder they
show, by listing each one, one after another, and Files' GIO workers then look up every file in it too: opening a
year listed and seeded all its months, previews included (verified on Bazzite 44 GNOME with the kernel tracing
listings and lookups). A listing is taken for a count when its parent's listing, or another count in it, ended less
than 3 s before, a sibling was listed since, and it was not listed itself in that time: a counter lists siblings in
a row, while a file dialog or a script opening one subfolder lists none. A count answers at once, and its small
seeds are written in the background, four fetches and one render at a time, behind the folders being shown, with no
previews. Its visit, often the next click, then finds them waiting (a month of 450 photos seeded in 0.2 s, where a
cold one takes 0.5 s). A subfolder opened within those 3 s is listed a second time, which counts as the visit; one
opened before the file manager counted it is listed once, as a count. That one is promoted: a file manager showing a
file asks its access rights right before it looks for its thumbnail (GLib: the stat, two `access()` calls, then the
thumbnail paths), and a count asks none, so the first `access()` on a file of a counted folder seeds it as a folder
shown and waits for that. This relies on the mount having no `default_permissions`, so that the kernel passes every
`access()` to the daemon (GNOME integration). Verified on Bazzite 44, GNOME and KDE: a month opened right after its
year was listed showed every thumbnail, and GNOME recorded no failure.

**Not chosen, for now: prefetching beyond what file managers list.** Seeding ahead covers the folders one level
below, since file managers list them anyway; prefetching further would fetch thumbnails for folders nobody shows,
multiplied by People and Places. ROADMAP, "Performance", weighs it with the other options.

The URI that keys the cache file must match `g_filename_to_uri` byte for byte. A unit test compares our encoder
with GLib's for awkward names (spaces, `#`, `%`, `?`, `[`, non-ASCII, combining characters). GLib is only a test
dependency.

**Indexer walks do not trigger seeding either.** Baloo (`baloo_file`) and GNOME's miners list every folder they
index; on a mount with People and Places, one walk would fetch a server thumbnail for every photo.

**Search crawls do not trigger seeding.** FUSE reports the calling thread, and Files' crawl runs on one named
`nautilus-search` (verified in a GNOME 50 VM). Their hits fall to the safety net and show generic icons until the
folder is opened normally.

**Not chosen:**
- **Small thumbnails only:** cheaper, but soft at larger zoom levels and on HiDPI screens.
- **Seeding for search crawls:** one search could fetch thumbnails for the whole library. Seeding them lazily,
  with a cap per minute, remains the fallback if crawl detection proves unreliable.

### The safety net

Seeding can lose a race or hit its budget. So when a known thumbnailer opens an original, it gets `EIO` at once:
- **Identifying the caller:** FUSE reports the calling thread. Its process (`Tgid` in `/proc/<id>/status`) and that
  process's ancestors are matched by `/proc/<pid>/comm`. The thread's own name is not enough: glycin reads from a
  worker thread named `blocking-1` (verified with the bpftrace probe, 2026-11-01).
  - On GNOME 50 that is `glycin-thumbnailer` under `bwrap`. `comm` is truncated to 15 characters, so the match is
    on `glycin-thumbnai`.
  - Videos go to `gst-video-thumbnailer` (`gst-video-thumb`), also under `bwrap`.
  - Also recognised: `gdk-pixbuf-thumbnailer`, `totem-video-thumbnailer`, `ffmpegthumbnailer`, and KDE's
    `thumbnail.so` worker.
- **What follows:** GNOME's thumbnail factory records a failure for the file (`thumbnails/fail/`), and Files keeps
  showing a generic icon for it, F5 included, until Files is started again; then the seed written meanwhile wins
  over the failure. In practice this happens only when seeding could not run, with the server
  unreachable, so WORKFLOWS tells users to reopen Files once it is back.

**Indexers are refused too, always.** Baloo indexes only Documents, Pictures, Videos and Music by default, but once
the mount is among its folders (added, or mounted inside Pictures), its extractor reads every original: 75 MB within
a minute on the lab's 50 photos. Its extractor and GNOME's get `EIO` at open, whatever the safety-net setting, and
their file-name indexing goes on. GNOME is also kept out by `.trackerignore`; KDE has no such file.

**Not chosen:** feeding the thumbnailer the preview's bytes instead of the original's. The same path would then return
different bytes depending on who asks, the size would not match, and a misidentified program (a backup tool, an
editor) would silently get a small JPEG instead of the photo (principle 3).

### Privacy

Seeds are small copies of private photos in a shared directory. Every seeded file is recorded, and deleted when
signing out, together with any `fail/` entries for our URIs. Signing out also sweeps
the thumbnail cache for any entry whose `Thumb::URI` lies under the mount, by either of its paths, including thumbnails a thumbnailer made
there and failure records for search hits that were never seeded, since those URIs carry album and person names.

## Uploads

**One folder takes new files: `Upload`, at the root.** Every other folder is a read-only view whose structure is
built from the library's metadata: Photos from each photo's date, Albums, People and Places from Immich's. What is
copied into Upload is uploaded, and leaves it once Immich has it, a few seconds after the upload; the photo then
shows in Photos under its own date, and wherever else Immich files it. With uploads off, there is no Upload folder.
- **Why not Photos and its date folders,** where new files went first: a date folder is computed from each photo's
  own date, so a photo dropped into `2024/03 March` lands wherever its date says, and the drop suggested a placement
  that could not be kept. And a copy pasted beside its source (Ctrl+C, Ctrl+V in Files) uploaded the same bytes,
  Immich added nothing, and the pasted name stood as a receipt that could not be deleted (seen on Bazzite). In
  Upload a paste of that kind adds nothing too, says so in a notification, and goes like any upload.
- **The lock** (`access()`) then reads simply: everything is read-only but Upload. And uploads do not depend on
  the Photos section being shown.
- **Flatpak impact:** none; it is the mount's own layout.

**The upload runs inside FUSE `FLUSH`, which is what `close()` waits on.**
- `close()` returns success only after the server has the file, and GIO checks the result of closing the
  destination before it deletes a moved file's source.
- So a move into Upload never loses its source early, in a program that checks its `close()` (principle 6).
  KIO, Dolphin's file layer, does not: in a Plasma 6.8 VM a move whose upload failed still deleted its source, since
  KIO counts the permissions it then cannot set as a mere warning. Hence the not-uploaded folder ("After a failed
  upload", below).
- The cost: Files' progress bar sits at 100% while the upload runs. A notification covers long uploads. And the
  kernel cannot interrupt a `flush`, so a program killed while its close waits (Ctrl+C on `cp`) stays until the
  upload ends, at most the limit below; the data stays safe.
- The wait is bounded: 30 minutes in total, and 2 minutes without progress. Temporary failures are retried with
  backoff inside that limit. *(Built so far: the 30-minute limit, and the client's own brief retries.)*

**Not chosen:** uploading after `release`, which returns at once. Release errors are discarded by the kernel, so
failures would be invisible and moves could lose files.

**What is accepted:** standalone image and video files, judged by extension against the server's media types (RAW
included), read from `/server/media-types` when the daemon starts, with a built-in list when the server cannot be
asked. A refused extension fails at `create`, before any bytes are copied.

**Not supported:** Live Photo pairing, and separate XMP sidecars. Folders are taken (below, "Folders copied into
Upload").

**The upload sequence:**
1. Stream SHA-1 over the staging file.
2. `POST /assets/bulk-upload-check`. A trashed duplicate is restored here (`POST /trash/restore/assets`), with no
   upload: copying a photo into Upload is the plainest way to say it is wanted, and it comes back as it was (its
   date, albums and faces), where the user would otherwise restore it in Immich and copy it again; a folder full of
   them was stuck in Upload on the Mac (the maintainer's call). Restoring needs `asset.delete`; without it the
   upload fails, saying so. **Not chosen:** refusing, the earlier rule, which kept a deletion standing; asking in
   the album question, one more choice in one dialog.
3. A streamed multipart `POST /assets` with `assetData`, `fileCreatedAt` and `fileModifiedAt`.
   - `fileCreatedAt` is the capture date from the file's EXIF (images) or container metadata (videos), falling back
     to the mtime, then to now. The server re-reads EXIF anyway. *(Built so far: the staged file's mtime at close;
     the client does not parse EXIF, since the server does. GIO may set the source's mtime only after closing, in
     which case a file without its own date is filed under the copy time.)*
4. 201 is a new Asset, and 200 `duplicate` is Already in library, or restored when it is in the trash.

A lost response is retried safely, because the checksum turns a second upload into a duplicate, not a second Asset.

**Details that need care:**
- **Upload once per round of writing, from a writer:** `flush` runs on every close of every descriptor, readers
  included (a thumbnailer). Every `open` gets its own handle that knows whether it may write; only a writer's close
  starts an upload, a close during an upload waits for its result, and a close after a failed upload gets the
  failure. A handle opened before an upload cannot write after it (`EIO`); a fresh open of the name is a new round.
- **Cut-short copies are refused:** a writer killed by a signal (`PF_SIGNALED` in its task flags) does not upload,
  and before any upload the file's own structure is checked: JPEG up to its end-of-image marker, PNG's `IEND`,
  GIF's trailer, the length RIFF (WebP, AVI) and BMP declare, Matroska's (WebM, MKV) Segment size, ISO media
  (HEIC, AVIF, MP4, MOV) whole up to its index and media data, and for TIFF and the RAW formats on it (DNG, CR2,
  NEF, ARW, ORF, RW2) every strip, tile and preview its directories point to. Data appended after a whole image
  (a Motion Photo's video, a phone's trailer) is allowed. A cancelled copy in Files closes the file before deleting
  it, so this check is what keeps it from becoming an Asset. What cannot be judged uploads: MPEG transport streams
  (MTS, M2TS), and Matroska written live, which declares no size.
- **Temporary names:** GIO's `.goutputstream-XXXXXX` and rsync's `.name.XXXXXX` are staged and upload under their
  real name. GIO renames before closing, so its rename returns at once and the close uploads (the folder is never
  locked for an upload); rsync renames after closing, so its rename uploads and returns the result, holding the
  folder's lock for that time. A hidden name with a media extension (`.IMG_1.JPG`) is a photo and uploads on close;
  any other hidden name is refused, so nothing is staged and then silently dropped.
- **Receipts:** the written name stays as a receipt for about 10 seconds, because GIO, KIO and rsync set the file's
  times right after closing it; deleting it before then is allowed and changes nothing in Immich. Writing to a
  receipt's name again counts as a new upload. Watched in
  the GNOME VM: GIO's copy into a new name sends `CREATE`, `WRITE`, `FLUSH` (the upload) and then two
  `SETATTR`s, the times and the mode, on the receipt, with no rename; Files' own copy and move behave the same, and
  a move deletes its source only after the close succeeded.
- **Free space:** `statfs` reports the staging filesystem's free space, so the file manager's space check is
  meaningful.
- **Kernel request timeouts:** since Linux 6.14 the kernel can abort a whole FUSE connection when one request
  outlasts `fs.fuse.default_request_timeout` (off by default; capped by `fs.fuse.max_request_timeout`). Verified in
  the GNOME VM: at 5 s, an upload in `flush` taking longer killed the mount ("Software caused connection
  abort"). So the daemon reads the limit at mount, warns once in its log, and ends every wait of its own a second
  before it: an upload that would take longer fails its `close()` and is stopped, a read or a listing fails with
  `EIO`, and the mount survives. `doctor` reports the setting. **Not chosen:** asking the kernel for a longer limit
  of our own (`FUSE_REQUEST_TIMEOUT` in the init reply), which fuser 0.18 cannot send; worth adopting once it can.
- **Unmounting during an upload:** an unmount, eject included, fails with "target is busy" while a writer still
  has the file open. Once `close()` has returned, the upload is finished.
- **After a failed upload,** the staged file disappears from Photos, and once its last handle closes it is moved to
  `~/.local/share/immich-mount/not-uploaded/` under its own name (numbered if taken), never deleted: a program that
  ignores `close()`'s error has deleted its source by then. The notification names the folder. Verified with `mv` and
  `gio move` and a key without `asset.upload` (both failed with "Permission denied" and kept the source), and with KIO's
  move in a Plasma VM (the source went; the kept copy matched it byte for byte).
- **After a crash:** staging files left behind by a dead daemon are not uploaded, since their writer's close never
  succeeded, but they are kept the same way at the next start, named from the name file each staged file has beside
  it.
- **What is deleted** is only what the user dropped: a copy cut short by Cancel (its source is still there), an
  empty file, a file deleted or replaced in Upload, an uploaded file's receipt.
- **Flatpak impact:** the folder follows `XDG_DATA_HOME`, which a Flatpak maps into its own data directory; the
  notification names the real path either way.

### Folders copied into Upload

A folder copied into Upload uploads every photo and video in it as it arrives, and once the copy has settled its
folders may become albums named by their paths (SPEC.md, "Folders copied into Upload").
- **Uploads first, the overview after.** No file manager says a copy is coming (Files, Dolphin and Finder send a
  folder one file at a time), and each upload runs at its file's close, so a move keeps its source until Immich has
  it (principle 6). So the files upload as they arrive, and the question about albums comes once the copy has
  settled. Immich adds an Asset to an album at any time, so the albums come out the same. **Not chosen:** the whole
  overview before any upload, which would hold every close until the last file arrived, or let a move delete its
  sources before Immich had them.
- **Settled** is nothing new for 5 seconds and nothing in the copy being written or uploaded: file managers hand a
  folder's files over one right after the other, and a long upload holds its file open, so a slow copy does not
  count as finished early. 10 seconds, with the Mac's question waiting for a poll and then for the app's change
  signal, kept it up to half a minute (the maintainer found it slow). On Linux the daemon looks every 2 seconds; on
  macOS the extension looks again just after the settling time following each upload in a copied folder, and the
  app reads the questions every 2 seconds, so the question comes within a few seconds of the copy. A copy that pauses longer (Files asking about a
  file Immich does not take) may be asked about early; what arrives after joins it: an unanswered question counts
  it, and albums take it in. A copy with nothing uploaded yet is not finished at all, since Files may ask before its
  first photo: it waits for that, and is forgotten after 10 minutes without one.
- **One portable record** (`immich_mount_core::folder_uploads`): each copy by its folder's name, each folder by its
  path with the Asset ids uploaded in it, the copy's state (copying, asked, albums, timeline) and the albums made.
  It is plain data, which macOS keeps in its store, since the system stops the extension between requests, and
  Linux in the account's cache, so a daemon restarted by a settings change asks again what it was asking (and
  closes its old notification, whose buttons would answer nobody). Linux keeps it at once after an answer and after
  each step of album work, so a daemon stopped midway asks nothing twice and makes no album twice.
- **Album work records each step as it happens:** the name chosen for a folder's album before the album is made,
  then the album. Work done again (an answer lost on the way back, a process stopped midway) finds the album it made
  by that name and the folder's first photo, so no second, dated album follows. Only a server that cannot be reached
  or is busy is tried again; any other failure ends the work, with a notification. On macOS the work runs in a task
  of its own, not in the poll, which the system cancels when it no longer waits: there it was left half done until
  the extension started again.
- **Names:** the path joined with ` › `, which reads the same in Immich and in the mount's Albums folder; `/`, which
  a file name cannot hold, would show as `_` there (SPEC.md, "Sanitising"). A taken name gets the day's date, then
  a counter, as the maintainer chose: it says when, where `Trip 2` says nothing. **Not chosen:** adding to an album
  of that name, which would mix a new trip into an old one; and refusing folders inside folders, which the path
  names make unnecessary.
- **One album per folder that holds photos,** with only its own: the timeline has them all, and a parent's album
  would repeat its children's.
- **The question needs an answer, so it is hard to miss.** A plain notification was not: in a GNOME 50 VM it went
  straight into the message list while another was on screen, and on the maintainer's Mac it was easy to ignore.
  - **On Linux** the daemon asks with an urgent notification, which GNOME and Plasma keep on screen until a button
    is pressed (verified in a GNOME 50 VM). It goes over one session connection that both shows the notification
    and hears the answer, since the desktop sends `ActionInvoked` to the connection that showed it, made when the
    first question is due: the user service starts at login, often before the desktop serves notifications, so a
    question waits for the desktop (only a machine without a session bus answers it Timeline Only at once). A
    click on the notification itself (its `default` action) answers nothing, and the question is asked again;
    without that action GNOME closed it as dismissed, which read as Timeline Only. GNOME shows the body on one line,
    so its parts are set apart with ` · `. **Not chosen:** a dialog, which the daemon has no window for, and the
    apps, which may not run (or be installed).
  - **On macOS** the menu bar app asks in a dialog, since the extension can show nothing. It floats above other
    apps until one of its two buttons is clicked, and neither answers to a key, so a dialog that comes up while
    someone types elsewhere is answered on purpose. Asking never waits on telling what came of albums, which first
    asks macOS whether the app may post notifications: an unanswered question about that held up every later
    question. Copy numbers start from the time in a new store, so after signing out and in an answer meant for
    a copy of before never reaches one of now. **Not chosen:** a notification with buttons, which Notification
    Center and Focus hide away; and a third button, Cancel (below).
  - Dismissing is an answer (the timeline only), and so is no answer within a day. Asking needs a desktop: a daemon
    without one, or with notifications that carry no buttons, leaves the copy in the timeline only, and
    `uploads.folders = albums` makes the albums without asking.
- **Timeline Only is the way out, and a change of mind is another copy.** By the time it asks, the photos are in
  Immich, so the only choice left is whether albums follow. A copy answered Timeline Only that takes an upload again
  is asked about again: copying the folder in once more costs nothing, since Immich recognises what it has, and Make
  Albums takes everything. **Not chosen,** as the maintainer settled: a Cancel that takes the uploads back. The
  mount never deletes from the library (principles 4 and 5); and it would take photos that were in Immich before the
  copy too.
- **On macOS the question passes through files** in the library's directory (the App Group's `Library/`). The
  extension writes the questions after each poll; the app writes each answer to a file of its own, which the
  extension takes and removes; and the extension leaves what came of the albums for the app to tell, one file
  each. Only the extension opens the store (redb allows one process), and with a file per answer neither side ever
  rewrites what the other wrote. The app reads the questions every 2 seconds, and wakes the extension at once
  after an answer.
- **Folders in the system's terms (macOS):** a copied folder is an item of its own (`upload-folder|Trip|Day 1`),
  kept in the store until it leaves, and listed in its parent and the working set, so the system keeps it however
  it enumerates; its uploaded files are answered for from the departures list, as files in Upload are. It leaves
  20 seconds after its last file, once it is empty and holds nothing the extension knows of (a file counts from the
  moment it arrives until it uploads, and a refused folder inside it counts too) and nothing in macOS's pending set:
  Finder may still be writing a large file into it, which the extension hears of only once it is whole, and taking
  the folder from under that copy would fail it (Finder deletes a moved file's source only once the whole move has
  succeeded, so the source stays). The extension reads the pending set before each poll, and a folder stays at
  least as long as macOS takes to bring that list up to date, and 10 seconds more; where macOS does not say (within
  5 seconds), or says its list is full, the folder stays 10 minutes. A folder that keeps something stays, and
  may be deleted in Finder, which forgets it. **Not chosen:** 10 minutes always, which left empty folders in Upload
  long enough to puzzle (the maintainer's call). Upload and the copied folders take changes to their times and such (the system passes
  on each change on disk as files come and go) and keep them as they are: refused, the system marked Upload as
  failing and tried again without end (seen on the maintainer's Mac, macOS 27.2). Finder's `.DS_Store` is no photo
  and keeps no folder.
- **Folders on Linux:** the staging area keeps each folder with the time something last arrived in it, and they go
  per copy: once no file is in any of the copy's folders and nothing arrived in any of them for 5 seconds, about as
  soon as the last file has left. Folder by folder, one waiting for its turn went while the copy was busy elsewhere,
  since Dolphin and rsync make every folder of a copy before its first file (found by the adversarial review). A gone
  folder is remembered for 10 minutes, unseen, and a path lookup brings it back: Files stops at a file Immich does not
  take until someone answers Skip All, and with the folder gone for good the rest of such a copy failed with "No
  such file or directory" (verified in a GNOME 50 VM). **Not chosen:** keeping the empty folders shown for those 10
  minutes, which left them in Upload long enough to puzzle (the maintainer's call). A file that did not upload
  is kept under its folder's path in `not-uploaded/`, so two `IMG_0001.JPG` from two folders stay apart, no name
  leads out of that folder, and where a folder of that path cannot be made the file is kept at the top. Times and
  modes set on a folder (`cp -a`, GIO) are taken and ignored, as a staged file's are, and no file may take a
  copied folder's name.
- **Permissions:** a credential that may not make albums (an API key made for uploads only) gets a notification
  saying so, and the copy stays in the timeline.
- **Settings:** `uploads.folders` (ask, timeline only, albums) is in every app: the Linux apps' preferences show it
  with the Upload switch in a group of their own, and the Mac's Folders pane under Uploads.
- **Flatpak impact:** the question needs notifications with buttons and their answers. The daemon calls
  `org.freedesktop.Notifications` and hears its `ActionInvoked`; a Flatpak would call the Notification portal
  instead (`AddNotification`, and the portal's own `ActionInvoked`), which carries buttons too. The record kept in
  the account's cache follows `XDG_CACHE_HOME`.

## Links to Immich

Open in Immich and Copy Immich Link take an item from the file manager to its page in Immich's web app (SPEC.md,
"Links to Immich").

- **The map is in the core** (`immich-mount-core/src/links.rs`): a key to its Immich path, the path to a web address
  and to a crosslink (ROADMAP.md, "Open in Immich, and links between apps"). Every platform asks it, so an item gives
  the same link in Files, Dolphin, Finder and the terminal, and the tests cover it once.
  - **The key is enough:** an Asset's key holds the folder it is listed in (`asset|<folder key>|<id>`), so a photo's
    link keeps its album, person or city without asking Immich anything.
  - **Immich's own paths, not ours:** they are what Immich's web app and its mobile app's deep links already take,
    and they are Immich Crosslink's, so one map serves the browser and the other apps.
  - **The search is Immich's JSON byte for byte** (`JSON.stringify`, then `encodeURIComponent`), tested against what
    a browser writes, so the address is the one Immich itself would make. The country is added to a city's search,
    which Immich's Places page leaves out, because the folder knows it and two cities of one name differ.
  - **Folders without a page have no link,** rather than the nearest page: Photos' year, month and day folders,
    the year folders above albums, Upload. A link that opened the timeline at large from a month folder would look
    like it had failed.
- **On Linux the daemon answers** (`Links`, and `WebLink` for older menus, on its D-Bus interface): it alone knows which key a
  path means, through the names it handed out. It walks the path from the mount point through the folder listings
  it has, however old (`Tree::key_at`), and reads a folder from Immich only when it has none: the file manager asking
  has just shown the folder, and names are kept per folder, so an old listing still gives the item a name means. A
  menu therefore costs no request to the server. A path that reaches the mount through a symlink in its parent
  folders (`/home` to `/var/home`) counts; the daemon resolves only the folders outside the mount, never the mount
  itself, which it serves.
  - **Not chosen: an extended attribute on each file** (`user.immich.url`) for the menus to read: every file
    manager would read it on every item it lists, and a folder's link would need one too, where the daemon is asked
    only for the one item right-clicked.
- **The command** (`immich-mount web`) asks the running daemon, as the menus do, and opens the address with
  `xdg-open`, which every desktop answers. **Copying** from a command that ends at once is the hard part: on Wayland,
  and on X11, the program that set the clipboard serves it, and the text is gone when it exits. So the command hands
  the text to a program that stays: Plasma's clipboard (Klipper, over D-Bus) where it runs, else `wl-copy`, `xclip`
  or `xsel`, which keep serving it in the background. A file manager extension running inside the file manager sets
  the clipboard itself.
- **Files: a Nautilus extension in C** (`crates/immich-mount-gnome/nautilus/`), about 140 lines on
  `libnautilus-extension-4` and GTK 4, which Nautilus has loaded already. It sends one selected `file://` item to
  the daemon's `Links` (250 ms limit, `NO_AUTO_START`) and offers the items when an address comes back, so the
  daemon alone decides what is in the mount; the extension never reads `MountPoint`. Files asks on its main thread at
  every selection, so after a call that timed out it does not ask again for 30 seconds: a stuck daemon costs one wait,
  not one per click. A call takes about 3 ms (GNOME
  50 VM). It opens through GIO with GDK's launch context, so the browser comes to the front on Wayland, and copies
  with GDK's clipboard, which it owns for as long as Files runs.
  - **Built in each package's own job,** for its own distribution: one build would load on Fedora 44 (Nautilus 50.4)
    and Ubuntu 26.04 (Nautilus 50.2) alike, but the extension folder differs, each job builds on its distribution
    anyway, and an API change then fails that build rather than a menu at run time.
  - **Homebrew's cask: the same in Python.** Nautilus loads C extensions only from its own folder in `/usr`, which is
    read-only on Bazzite and the atomic desktops, and reads no other (`nautilus-module.c`). nautilus-python, which
    Bazzite ships (4.1.0, with GSConnect's and Steam's extensions), loads from the home folder, so the cask installs
    `immich-mount-nautilus.py` there. It asks for Nautilus 4.1 (Bazzite has only that typelib) and falls back to 4.0,
    reads the catalogs beside the cask's programs, and stays quiet where the C extension is installed, so a machine
    with both shows one pair (verified on Bazzite 44 with both).
- **Dolphin: a file item action plugin** (`crates/immich-mount-kde/dolphin/`, `KAbstractFileItemActionPlugin`, C++ on
  KF6), asking `Links` the same way and copying with Qt's clipboard, inside Dolphin.
  - **Show in *app* runs the app's command itself:** KIO hands a URL to an app as `QUrl::toString()`, which decodes
    `%20` and UTF-8 escapes, where the protocol wants the link as `encodeURIComponent` wrote it. So the plugin makes
    the app's command from its desktop entry (`KIO::DesktopExecParser`) with a placeholder link, puts the crosslink
    in its place, and runs it as that app (`KIO::CommandLauncherJob`, argument by argument, no shell). An app started
    through D-Bus activation is then started through its `Exec` line instead (verified in the Plasma VM: an Århus
    link and a name with a space arrived byte for byte).
  - **Not chosen: a service menu.** It can only run a command, and its one way to show only on some items,
    `X-KDE-ShowIfDBusCall`, was deprecated in KF 5.76 and is gone from KF6 (absent at 6.0.0; a probe menu with it
    showed everywhere in Plasma 6). Shown on every file on the computer, Open in Immich would mostly say there is
    nothing to open. The plugin also lets Show in *app* list a changing set of apps (step 3 of the roadmap's
    section).
  - **Homebrew's cask:** KIO finds plugins only on Qt's plugin path, which a home folder is not on. The cask links the
    plugin under `~/.local/lib/immich-mount/plugins` (a folder that holds only it, as Qt advises against a
    broad `QT_PLUGIN_PATH`) and adds a Plasma login script (`~/.config/plasma-workspace/env/`), which
    startplasma sources, putting that folder on `QT_PLUGIN_PATH`; the items appear from the next login. Bazzite's
    Qt 6.11 and KF 6.31 match Fedora 44's, where the cask's programs are built (verified on Bazzite 44 KDE).
- **Finder: the File Provider extension's own actions** (SPEC.md, "In Finder"). The extension opens the address and
  sets the pasteboard itself (verified on macOS 27.2), so the actions work while the menu bar app is not running.
  Finder decides from each item's `userInfo` whether to offer them, so the bridge's items carry `hasWebLink`, and
  the item format went up to 5 for it (the app has the system read every item again once).
  - **Not chosen: handing the action to the menu bar app** (a file in the App Group and a Darwin notification), the
    plan while it was unknown whether a non-UI extension may open a URL: it would have failed whenever the app was
    not running.
- **Reading links back** (for showing an item from a crosslink, step 2 of the roadmap's section) is in the same
  module: a crosslink reads to one of Immich Crosslink's three results (show, cannot show, invalid) before anything
  is fetched, and a web address on the signed-in server to its item. The protocol's own test links
  (`crates/immich-mount-core/tests/crosslink-links.json`, copied at the commit its test names) are run as they are:
  the links made byte for byte, and every result reached.
- **Receiving a crosslink** (Linux): the launcher entry takes the scheme, so a link starts `immich-mount app %u`,
  which hands it to the daemon's `ShowLink` and ends; the app's window never opens. The daemon reads it with the
  core's `receive` (the protocol's checks, the server, the user), finds the item where the drive shows it
  (`Tree::locate`: a photo's date folders come from the Asset's details, an archived photo of the timeline is looked
  for in Archive), names its path (`Tree::path_of`) and asks the file manager to select it through
  `org.freedesktop.FileManager1.ShowItems`, which Files and Dolphin both answer (verified in Fedora 44's GNOME 50 and
  Plasma 6 VMs, and Plasma's `xdg-open` passes the link on without asking).
  - **The daemon, not the app:** it alone knows the names it handed out, and it already runs while the drive is
    mounted; starting a GUI to select a file would flash a window.
  - **Only showing:** any web page can open a registered scheme, so a link is a stranger's. The daemon fetches the
    folder listings and a photo's details and nothing else, and every way it ends is a selection or a sentence.
  - **The user** is read with the email at start (`GET /users/me`), so a link from another account's view is not
    shown as if it were this one's; an API key that may not read the account skips that check, as the protocol
    allows.
  - **The setting** is read at start, as every setting is; the apps restart the daemon when it changes.
- **Receiving on the Mac:** the menu bar app takes the URL (`application(_:open:)`) and asks the bridge's
  `show_link`, which runs the same checks and `Tree::locate` and `Tree::path_of` as the daemon, with a tree of its
  own over the app's connection: the library's store belongs to the extension, and keys do not depend on the names
  it keeps. The app then asks the system for the item's place (`NSFileProviderManager.getUserVisibleURL`) and has
  Finder select it. The system lists the folders above an item Finder never showed by itself, about a second for a
  photo three folders down (verified on macOS 27.2); right after every item was read again it can fail once with a
  locked database, so the app asks twice. The user's id is read only for a link that carries one.
- **Show in *app*:** the menus find the apps themselves, through their toolkit's own index of desktop entries
  (GIO's `g_app_info_get_all_for_type`, KDE's `KApplicationTrader`), and read the announcement from the entry, so an
  app's name and icon are the system's and the link goes to that app alone, as the protocol asks. The daemon gives
  what only it knows, in one call (`Links`): the item's web address, crosslink and kind, and the apps switched off.
  The settings list the apps from the same entries (`crosslink_apps.rs`, by the desktop entry specification's
  rules: the user's folder first, a subfolder's name in the id), so both apps' settings show what the menus offer.
  - **Not chosen: Immich Mount as the scheme's default handler.** Apps that take part never use the default, so it
    matters only to links opened in a browser or with `xdg-open`, and with two apps installed, which should get them
    is the user's choice. A package taking it over would also take it from the other app's users.
  - **Apps that come and go** stay listed in the settings for 30 days after they were last found, so one can be
    switched off before it comes back, and then age out with nothing to do (test apps, uninstalled ones). What was
    seen is kept as state, not a setting, as the protocol leaves to each app: it changes whenever the settings look,
    and is nothing a user writes. Immich Gallery keeps the same list by the same rules.
  - **Forget on every app, and off as the lasting choice.** It works like pairing devices, with pairing automatic:
    an app that announces itself is found, its switch is the user's choice, and Forget drops what was remembered of
    it, switch and all. One still found comes back at once, new and on; one not found leaves the list. So Forget is
    always safe, and clears an entry that went wrong without the user knowing why (an app moved, renamed, or no
    longer announcing), and an app the user does not want is switched off. The surprise, an app switched off coming
    back on after Forget, is why the list's description says what Forget does. An app away reads **Not found**,
    not "not installed": it may be installed and no longer announce itself, or be where a sandboxed app cannot read
    it.
    - **Not chosen: Forget only for apps away:** an entry for an installed app could never be cleared, and an app
      found by mistake stayed listed.
  - **No app set apart.** Immich Mount offers and names only apps that announce themselves, the same way whoever
    made them, so the protocol it publishes is the one it follows. Immich Gallery is one participant among them.
- **Web links can be switched off** (`web.links`), apart from Immich Crosslink, as the protocol asks of every
  participant: the daemon then leaves the web address out of `Links` and `WebLink`, and the menus, which offer each
  part only when it is there, offer Show in *app* alone. One switch for both items, since one opens the address the
  other copies.
  - **An older daemon** answers `Links` with an unknown method; the menus then ask `WebLink` and offer no Show in
    items, so a newer extension never breaks against a daemon not yet restarted after an upgrade.
- **Show in *app* on the Mac:** Finder's actions are declared in the extension's Info.plist, so their names are
  fixed; a list that changes cannot be one action per app. So the Mac takes the protocol's second way to offer Show
  in: one action, **Show in Another App…**, for every app that takes part. With one app offered for the item it
  sends straight to it, which shows where it went; with several it asks in a dialog, **Show in Which App?**, a list
  of the apps by name and icon with **Show** and **Cancel**, as Finder's own Open With > Other… asks. The app finds
  the apps (an extension may not ask Launch Services about other apps' bundles from its sandbox, and the app sees
  apps launch), writes what is offered to the App Group (`crosslink-apps.json`), and the extension marks the items
  Finder offers the action on (`userInfo`); the marker is part of each item's version, so the app has the system
  read the items again when the offers change. An extension shows no interface, so the action hands the request to
  the app: the extension writes it (the item) into the App Group's `actions/`, and opens the app's own URL scheme
  with that request's one-time token alone, which starts the app without a window if it is not running. The app
  takes a request once, only while it is a few seconds old, reads the apps again, makes the link itself from the
  item, and sends it only to an app offered for that item's kind; it asks which, and tells the user in an alert
  when sending fails, which an extension could not. Requests no one took are removed as the app starts.
  - **Not chosen: a menu at the pointer,** of the apps, popped up by the app once Finder's menu has closed (tried
    first): a menu opened from nothing, which macOS has no pattern for, taking focus from Finder with no window to
    show for it.
  - **Not chosen: an action per known app** (Show in Immich Gallery). Its name would be fixed in the extension, so
    only an app named in advance could have one: one participant set apart from the rest, with Immich Gallery's own
    link (`immichgallery://photo/<id>`) for its releases that do not announce Immich Crosslink (tried first). Those
    get Show in once they announce, like any other app.
  - **Not chosen: a Finder Sync extension,** which can build its menus as they open, with the apps' names and icons:
    Apple directs File Provider apps to their actions instead, and it would be one more extension for the user to
    switch on.
  - **Not chosen: the link in the URL.** Any web page can open an app's URL scheme, so a link carried there would let
    a page make Immich Mount send something to another app without a click; the App Group is the extension's and the
    app's alone.
- **Flatpak impact:** the D-Bus members and the command work from a Flatpak (the command needs the session bus and
  `xdg-open`, which the OpenURI portal answers); the file managers' menu items are files on the host, which a
  Flatpak cannot install. A Flatpak exports its launcher entry with the scheme, so it can take crosslinks; selecting
  the item needs `--talk-name=org.freedesktop.FileManager1`.

## Signing in

**Sign-in is a button that opens the user's own browser, like Immich's mobile app.**
1. The app reads `/server/features` (`oauth`, `oauthAutoLaunch`, `passwordLogin`) and `/server/config`
   (`oauthButtonText`) to know what to offer. Neither needs authentication.
2. `POST /oauth/authorize` with `{redirectUri, state, codeChallenge}` (PKCE S256) returns the identity provider's
   URL, opened through the OpenURI portal.
3. The user signs in there with passkeys, password managers, or whatever they normally use.
4. The provider redirects to `app.immich:///oauth-callback`. Our launcher entry is registered as the handler for that
   scheme. On servers with Immich's "mobile redirect override", the provider redirects to
   `https://<server>/api/oauth/mobile-redirect` instead, which bounces to the same `app.immich:` URL.
5. `POST /oauth/callback` with `{url, state, codeVerifier}` returns a session token.

This flow was read from the server source (`server/src/services/auth.service.ts`, `oauth.repository.ts`,
`dtos/auth.dto.ts`), and is verified on a running server: `immich-mount login` signs in through Dex in the lab
with Dex as the identity provider, and both apps complete it from a real browser redirect in the GNOME and
Plasma VMs.

**Why this design:**
- **Reusing the mobile app's redirect URI** means sign-in works with no server or identity-provider changes,
  wherever mobile sign-in already works.
- **The Immich server is the OIDC client:** it holds the secret and does the exchange, so we register nothing
  anywhere.
- **PKCE and state make an intercepted code useless.** If another program on the same machine claimed the
  `app.immich` scheme, the worst outcome is a failed sign-in, not a leak.

**Who runs it:** the app runs the interactive flow as a single instance, so the redirect reaches the process that
holds the PKCE verifier. It stores the credential in the keyring, and the daemon only reads it (`Reload` over
D-Bus). Headless, `immich-mount login` runs the same flow itself, with the redirect URL pasted back.

**The session is kept, in the keyring.**
- **Only a session gets the sync stream** (verified), which makes the mount near-live.
- **Revocable in the usual places:** the desktop shows up in Immich's "Authorized devices", and signing out there or
  at the identity provider (back-channel logout) cuts it off. That is what OIDC users expect.
- Sessions have no expiry by default.
- The cost: a session carries the user's full permissions, so least privilege rests on the app's own switches.

Requests send an honest User-Agent, `ImmichMount/<version> (Linux; <distro>)`, so the "Authorized devices" entry is
recognisable.

**Email and password** is offered when the server allows password login, both on the desktop and headless. It yields
a session too, so even a headless machine gets the sync stream. Immich's password login has no two-factor step:
`LoginCredentialDto` is email and password only, and the feature request for it (immich issue #880) was closed.
Two-factor comes through OIDC, and the form says so.

**API keys** are the advanced option: tests, CI, least-privilege setups, and machines without a browser.

**Not chosen:**
- **An embedded web view:** passkeys and password managers belong in the real browser (RFC 8252 says the same). The
  maintainer uses passkeys.
- **A loopback redirect (`127.0.0.1:port`):** the native-app norm, but the admin would have to register it at the
  identity provider. Parked in ROADMAP.
- **Turning the session into a scoped API key:** least privilege, but it loses the sync stream and identity-provider
  revocation.
- **No password form at all,** with API keys for password-only servers: no credentials typed into our app, but a
  clumsy setup and no live changes, for the main use case (desktops).

The core holds the Immich side of sign-in. Catching the redirect is per platform: a scheme handler on Linux, and
`ASWebAuthenticationSession` on macOS.

## The Immich API layer

`immich-api` holds the client and models for Immich's API: the endpoints the mount uses, and the rest of the API a
full client needs (albums and their members, changing Assets, the trash, edits, people and faces, tags, stacks,
duplicates, memories, the map, search, folders, downloads, sharing, the account, and administration). The mount
calls only its own part; the rest is there so other apps can be built on the same tested layer (the maintainer's
choice, 2026-11). The mount's endpoints are copied from [immich-rs](https://github.com/boul2gom/immich-rs) (MIT,
generated from the Immich v3.3.5 OpenAPI spec, commit `60d1eb3`) and adapted; the rest are written here from the
OpenAPI spec, with the choice of endpoints and their request shapes learned from two clients: ImmichGallery (MIT) and
Mimick (GPL-3.0, read, never copied). From then on all of it is our own code.

**How the whole API is kept honest:**
- **One table** (`immich_api::endpoints()`) lists every endpoint the crate calls with the API-key permission Immich
  requires, and each method's documentation names the same permission.
- **The spec check** (`tests/contract_spec.rs`) reads the server's own `/api/spec.json` and fails when an endpoint is
  missing or needs another permission. It runs against each supported version, so a renamed or removed endpoint
  shows up before a user meets it, and it prints the deprecated endpoints still in use.
- **A contract test per endpoint** against the lab server (`tests/contract*.rs`), except the OIDC callback, which
  `lab/immich-server/oidc-sign-in.py` checks with Dex. Tests that change the library put it back before they end,
  even when an assertion fails halfway, and the lab restores its seeded snapshot before each session.
- **The read-only key** is refused the changes of every area, aimed at ids no server holds, and the test checks
  that the server names the permission the table names. The tests first check that the key is the read-only one.
- **A lost answer is not a second change.** Creating, rotating a key, changing a password or PIN code, and shifting
  dates by an amount are resent only when the first attempt cannot have taken effect (`retry.rs`).
- **Secrets stay out of logs:** API key values, session tokens, passwords, PIN codes and shared link keys are
  `Secret`s, whose `Debug` output is redacted, and a transport error's message has its URL's query cut.
- **Deprecated endpoints** are used where v3.2 has nothing else (`PUT /assets/{id}`, `PUT /tags/{id}`, and others);
  the spec check lists them, and they move when the floor does.
- **New models** decode with a struct-level `serde(default)`, and `null` counts as missing, so a field absent on one
  version or `null` on another never breaks an answer. An enum keeps a value it does not know. A model's id may
  decode empty, so the client refuses an empty path segment before sending: `/albums/{id}` can never become
  `/albums/`.

**Left out on purpose,** until something needs them: cluster groups, workflows, plugins, database backups,
maintenance mode, the system configuration, licenses, onboarding, admin email templates, asset files, and the HLS
video stream (`/video/playback` serves players that read ranges). Each is a module and a contract test away.

**Not chosen: a client generated from the OpenAPI spec.** It would cover every endpoint at once, but the spec marks
fields required that older servers leave out, has no notion of the quirks below, and would give every caller
generated types instead of the error kinds and models the mount branches on.

**What is copied:**
- the models for the timeline buckets (with album and person filters), search, people, albums and suggestions
- the sync stream and ack types
- the OAuth authorize and callback types
- auth with API keys and bearer tokens
- multipart uploads with SHA-1
- the retry policy: 429 and 502 to 504 everywhere, 500 only on idempotent methods, honouring `Retry-After`

**Why copy instead of depending on it:** the crate was three days old with one author, and carries far more than we
use (the whole API, WASM, OpenTelemetry, Socket.IO). Our contract tests on Erebus catch API drift.

**Checks before relying on the copied code** (M0):
- the endpoints the mount uses, against the lab on v3.3.5 and v3.2.0
- **lenient decoding:** a field missing on the older server, or an unknown one, must not break decoding
- rustls only, no unused features, and it builds for the static musl target
- a reasonable compile time

**Version policy:** the current and previous Immich minor release, with the floor moving forward deliberately.
- Only the current search dialect is implemented: `filter`, `orderBy`, `cursor`. Smart, random and counting
  searches take the same `filter`. In this dialect smart search has no paging (`page` is deprecated and cannot sit
  beside `filter`, and there is no cursor), so it returns its best matches, up to 1000. The one exception is the
  deprecated `POST /search/large-assets`, which takes its few filters in the query string.
- Every search sets `filter.trashedAt: {eq: null}`, because that dialect includes trashed Assets by default.
- `/server/version` is read once per connection, and version-specific behaviour lives in one place.
- **Checked on v3.2.0:** the parts of the OpenAPI spec the client uses are identical to v3.3.5's, and the server
  answered the same. (One difference: a video's `fullsize` thumbnail redirects to its preview, and `fullsize` is
  never used.) The whole API's contract tests pass on v3.2.0 too, with three differences the code and tests know:
  its reverse-geocoding data names the United States "United States of America", merging people made by hand
  merges nothing (that version merges by person group), and a tag taken off an Asset while its sidecar job is
  pending comes back on it (`tag.rs`).
- **Not chosen:** a floor as old as v1.135.0, with the deprecated search dialect. It would mean two dialects, older
  upload fields, and a larger test matrix, for a new app whose users tend to upgrade Immich often.

**Names and sizes without a session.** With an API key there is no sync stream. The details for a month's ids
(names, sizes) then come from a search over the month's UTC range widened by 14 hours on each side, filtered to
the bucket's ids.

**The account namespace.**
- A session knows its user id from the sign-in response, so caches are keyed by the URL plus the user id.
- A scoped API key cannot read `/users/me`, and `/api-keys/me` returns no user id, so API-key connections are keyed
  by the URL plus a hash of the key.
- `status` shows the account's email only when the credential can read it.

What the API does, verified on v3.3.5, all of which the client encodes:
- **Sizes:** `withExif: true` on every search; without it there is no `exifInfo` at all, so no sizes.
- **Visibility:** searches and buckets outside albums, favorites and the archive restrict visibility to the
  timeline, since the defaults include archived Assets.
- **Albums:** album contents come from search by `albumIds`. The album response has no asset list.
- **The connection probe** is `/api-keys/me` for keys, which needs no scope. `/users/me` needs `user.read`, which
  scoped keys lack.
- **Thumbnail sizes:** `thumbnail` is a 250 px WebP and `preview` a 1440 px JPEG. `fullsize` redirects to the
  original, so it is never used.
- **Uploads:** they need `assetData`, `fileCreatedAt` and `fileModifiedAt`. The checksum is base64 SHA-1, and
  `bulk-upload-check` accepts hex or base64. A duplicate of a trashed Asset returns the trashed id, and the Asset
  stays trashed.
- **Error codes:** an unknown or deleted Asset id gives 400 `Not found or no asset.read access`, not 404. A missing
  or inaccessible Asset is not proof of deletion.
- **Keys stay out of URLs:** `?apiKey=` also authenticates, but is never used. An API key sent as `Bearer` is
  refused (401).
- **Ranges on originals:** byte-exact 206 answers with `Content-Range`, HEAD included. A multi-range request gets
  the whole file (200), and a range past the end gets 404, not 416. The weak ETag (size and mtime in hex) is the
  same for every range, and `Last-Modified` is the Asset's `fileModifiedAt`.
- **Timeline buckets** are columnar JSON: ids and a few fields, with no names or sizes. Their `fileCreatedAt` has
  no `Z`, and a malformed `timeBucket` gets a 500.
- **Value formats:** `duration` is in milliseconds (null for images), and `localDateTime` is wall-clock time with a
  misleading `Z`.
- **The search dialect** (`filter`, `orderBy`, `cursor`) cannot be mixed with the deprecated fields (400).
  `visibility {ne: archive}` needs elevated permission, since it would include locked Assets, so the client asks
  for `{eq: timeline}`.
- **Defaults that surprise:** a `size` parameter on buckets is ignored, not rejected; `GET /assets/{id}` has no
  `status` field; search defaults include archived (and, in the new dialect, trashed) Assets.

## GNOME integration

- **Modes are what copies get; `access()` is what the mount allows.** Files are `0644` and folders `0755`, and the
  mount has no `default_permissions`: the daemon refuses each change itself, and answers `access()`, from which GNOME
  Files draws its read-only emblem (the lock) and greys out Rename and Move to Trash. Only the mounting user reaches
  the mount either way (no `allow_other`).
  - **Why:** a copy takes its source's mode bits, with `cp` and in Files alike (Nautilus resets them only for a
    copy from a CD, `isofs`), so read-only modes made every photo copied out a read-only file, and every copied
    album a folder nothing could be added to or deleted from (seen on Bazzite).
  - **Not chosen: writable modes with `default_permissions`.** Copies would be right, but the kernel answers
    `access()` from the modes, so Files would offer Rename and Move to Trash everywhere and show no lock, and each
    would fail.

- **The mount lives at `~/Immich`** (verified in a GNOME 50 VM):
  - **In the sidebar:** a FUSE mount under home appears in the Files sidebar as its own volume, with a drive icon and
    an eject button.
  - **Except in a home reached through a symlink** (Fedora's atomic desktops and Bazzite: `HOME=/home/me`, `/home` a
    link to `/var/home`). GVfs's volume monitor (`should_include`) lists a FUSE mount by its path: under `$HOME`,
    `/media` or `/run/media/$USER`. It compares the kernel's path, always resolved (`/var/home/me/Immich`), with
    `$HOME` as written, so no mount in such a home is listed, sshfs and rclone's included (seen on Bazzite).
    - **The daemon gives the volume monitor the resolved home** as it starts: a drop-in for that one user service,
      `~/.config/systemd/user/gvfs-udisks2-volume-monitor.service.d/immich-mount.conf` with
      `Environment=HOME=/var/home/me`, then a restart of the monitor. The mount is then a drive with an eject
      button, as on any other Fedora, and other FUSE mounts in home are listed too; nothing else reads that
      service's `$HOME`. No root is needed, and it reaches every install and upgrade, the command line's included.
      `logout` removes the drop-in. Verified in a GNOME 50 VM with `/home` a link to `/var/home`: with Files
      already open, the drive appeared once the daemon started.
    - **A bookmark is the fallback** where the volume monitor is not a user service: the GNOME app's setup
      bookmarks the mount once, titled with the drive's name, in GTK's bookmarks file, and the daemon takes that
      bookmark away once the drive is listed (a bookmark the user made, untitled, stays). `doctor` applies GVfs's
      rule, with the home it was given, to say which case a machine is in.
    - **Flatpak impact:** a Flatpak app could not write the drop-in, but the daemon writes it, and the daemon stays
      on the host.
    - **Not chosen: the `x-gvfs-show` mount option,** which GVfs honours before the path. Only root may record a
      user-space option, so it reaches GVfs only through `fusermount3`: from fuse3 3.17, and only into a
      `/run/mount/utab` that already exists, which it does not on a Bazzite where nothing has mounted a drive since
      boot (seen there). An older `fusermount3` hands the option to the kernel, which refuses the mount.
    - **Not chosen: a mount point under `/run/media/$USER`,** which GVfs lists anywhere; only root may create one.
    - **The fix belongs in GVfs** (and GLib's `g_unix_mount_entry_guess_should_display`, which has the same
      comparison): resolving `$HOME` before comparing would list every such mount, and the drop-in would be needed
      no more.
  - **Its name** comes from `/.xdg-volume-info`. The icon set there only affects full-colour icons; the sidebar keeps
    the drive symbol.
  - **Elsewhere:** a mount under `/run/user` is invisible unless bookmarked.
  - **Eject** really unmounts. The daemon treats that as the user's choice and exits cleanly; the user service is
    not restarted, so the next login, **Mount** on the app's status page or `immich-mount mount` mounts again.
  - **Remounting:** gvfs ignores a quick unmount-and-remount, so the daemon waits for the old mount to disappear
    first.
  - **No `DCIM` folder at the root:** gvfs scans new mounts for camera layouts *(untested)*.
- **File dialogs:** GTK and portal file choosers show the mount, and should show the seeded thumbnails. Flatpak apps
  reach the files through the document portal. *(Both untested.)*
- **The indexer:** GNOME's indexer (localsearch) indexes all of `$HOME` recursively by default. In the GNOME VM it
  never entered the FUSE mount, for unknown reasons, so the hidden `.nomedia` and `.trackerignore` stay as
  protection, including if someone mounts inside `~/Pictures`.
- **Search:** a search typed in Files crawls the tree (listings only). People and Places multiply the tree, so a
  first search is slow. The GNOME Shell overview search did not touch the mount.
- **`updatedb` / `plocate`** run as root, and root cannot enter a mount without `allow_other`.
- **Backup tools** that walk `$HOME` (Déjà Dup, restic, borg) would see the mount. WORKFLOWS documents how to
  exclude it.
- **No tray icon:** stock GNOME has none. Status lives in the app and in notifications. The daemon is a user
  service, so it needs neither the Background portal nor a tray.
- **Notifications** cover only what the user cannot otherwise see:
  - a refused upload
  - a long upload
  - the server unreachable (once)
  - signed out
  - a locked keyring
- **Not chosen:**
  - **A GNOME Online Accounts provider plus a GVfs backend,** as GNOME did for OneDrive in GNOME 46. It is C code
    inside GNOME's own repositories, on GNOME's release cycle, and GNOME-only. GIO also marks such locations remote,
    which means no thumbnails by default. Non-GIO apps would only reach it through gvfs's own FUSE bridge.
  - **A Nautilus extension:** extensions can no longer add locations, only menus and columns.
  - **fanotify pre-content events** (Linux 6.14+): placeholder files on a local filesystem, filled on first access.
    They are the closest Linux match to macOS's placeholders, but need `CAP_SYS_ADMIN`.

## The desktop app

Each platform gets a native app that looks and reads like a first-party app (principle 9).

**How the GNOME app is written:** gtk4-rs and libadwaita, with screens in **Blueprint**.
- **Blueprint** is GNOME's compact markup for GTK interfaces. It compiles to GtkBuilder XML at build time, through
  gtk4-rs's `blueprint` feature (`#[template(file = "window.blp")]`).
- **Who uses it:** many GNOME Circle apps and some core apps, so it is the direction GNOME's own apps are taking.
- **The caveat:** Blueprint still calls itself experimental, and GTK 5's way of defining interfaces is undecided
  (the April 2026 GTK planning call kept Blueprint out of GTK's tree). The app is small (sign-in, settings,
  status), so a migration would be cheap. The build environment pins the Blueprint version, and ROADMAP tracks
  upstream changes.
- **Not chosen: Relm4.** Less boilerplate, but an extra layer, and not what GNOME's documentation or its own Rust
  apps (Loupe, Snapshot) use.
- On GNOME: GTK 4 and libadwaita, following the GNOME Human Interface Guidelines:
  - `AdwApplicationWindow`, `AdwNavigationView`, `AdwStatusPage`
  - `AdwPreferencesDialog`, `AdwToast`, `AdwAboutDialog` and `AdwShortcutsDialog` (Ctrl+?, which needs
    libadwaita 1.8, GNOME 49)
  - adaptive layout, the system's dark style and accent colour
  - symbolic icons, and GNOME's capitalisation rules
- **Names:** the app says "Immich Mount", and the file manager says "Immich". The interface says "Files", never
  "Nautilus".
- **First run:**
  1. the server URL, checked live
  2. sign-in, offering only the methods the server supports
  3. the sections and the mount location
  4. "Open in Files"
- **One settings table:** the settings dialog is generated from the core's settings table, the same one behind
  `immich-mount configure`. The two can never disagree about which settings exist or what values are valid.
- **Status:** connection, account, server version, mount point, cache usage and uploads in flight.
- **The About dialog** carries the non-affiliation notice, the links to the website (`jakob.hviid.phd/immich-mount`)
  and the source code, the credits, and the AI disclosure.
- **Accessibility:** keyboard navigation everywhere, a keyboard-shortcuts dialog, and accessible labels on every
  control.
- **One app, native on each desktop:** one launcher entry, `dev.hviid.ImmichMount`, runs `immich-mount app`, which
  opens the Kirigami interface on Plasma (`XDG_CURRENT_DESKTOP` names KDE) and the GTK one everywhere else, never
  the other's look. Bare `immich-mount` does the same where there is a display, so the app comes first and the
  command line is the fallback for a machine without one. The two interfaces stay separate binaries: one linking
  both GTK and Qt would not start where either is missing. Each answers `--probe` before it touches its toolkit,
  and the launcher passes over one that does not load; so on a Plasma whose Qt is not the one the Kirigami build
  links (cxx-qt's QML registration uses Qt's private API), the GTK interface stands in. Outside Plasma the Kirigami
  one never does. The launcher entry declares the `app.immich` scheme and hands the redirect on, so it reaches
  whichever interface is in use.
  - **Packaging:** the daemon's package carries the launcher entry and the AppStream data; `immich-mount-gnome`
    and `immich-mount-kde` carry the interfaces, and on Fedora the daemon's package recommends the one for the
    installed desktop. With Homebrew the whole app is one cask, `immich-mount`: both interfaces, built on Fedora 44,
    with the static daemon and command line, its man page and completions. `brew install jakobhviid/tap/immich-mount`
    finds it without `--cask`, since no formula has that name, and without `brew trust`, since it is named (verified
    with Homebrew 7.0.12 in a fresh Fedora 44 GNOME VM). The formula `immich-mount-headless` is the daemon alone,
    for x86_64 and ARM servers; the two hold the same binary, so they do not go together. The tap's
    `cask_renames.json` moves an installed `immich-mount-linux` to the cask at `brew update`.
  - **Not chosen for Homebrew: a formula for the daemon and a cask for the app that depends on it,** after
    Homebrew's own split (formulae for command-line tools and services, casks for apps). It had a desktop user
    install the cask under a second name to get the app, while the obvious name gave a command line with no app,
    and a cask's dependency from a third-party tap needs `brew trust --tap` first where naming the cask does not. A
    GUI-first app is installed by its own name, in one step.
  - **Notifications:** on Plasma the daemon's notifications name the app (`desktop-entry`), which heads them with its
    name and icon; Plasma is detected as for seeding (the environment, or Plasma's shell on the session bus). Elsewhere
    they carry the name and our symbolic icon and no `desktop-entry`: GNOME 50 drops a notification whose
    `desktop-entry` names an app it did not come from, and the daemon is not the app (verified in both VMs).
  - **Not chosen:** two launcher entries, one per desktop (`OnlyShowIn=KDE;` and `NotShowIn=KDE;`): two apps to explain,
    two scheme handlers that had to claim the scheme in turn, and two casks.
- **How the KDE app is written:** Kirigami pages in QML over one Rust object, bound with **cxx-qt** (KDAB's
  Rust/Qt bridge). The object is a thin layer over `immich-mount-desktop`, so the two apps share every step but
  the widgets.
  - **Its pages** follow GNOME's steps in Kirigami's terms: `PlaceholderMessage` for the start, waiting and status
    pages, `FormLayout` for sign-in, setup and preferences, Preferences and About as layers from the header's menu,
    and the `org.kde.desktop` style, so it looks like Plasma's own apps.
  - **Its preferences page** is generated from the same settings table as the GNOME dialog (one `FormLayout` row
    per setting: a switch, a list or a field), and shares its side effects through `immich-mount-desktop`: each
    change is written at once, "Mount when the session starts" enables or disables the user service, and the daemon
    restarts when the page closes if anything it reads changed. A refused value shows the table's message.
  - **The browser's redirect starts a second process,** because Qt has no single-instance service. The first process
    keeps the waiting sign-in (server, state, PKCE verifier) in `$XDG_RUNTIME_DIR/immich-mount-sign-in`, mode 0600;
    the second claims the attempt by renaming the file away (so a redirect delivered twice is finished once),
    finishes the sign-in, stores the credential, writes the outcome in its place, and exits; the first reads the
    outcome and moves on. It waits for that outcome, not for a stored credential, because "Sign In Again" starts
    with the old credential still in the keyring. A provider's refusal (`error` in the redirect) is reported the same
    way, and going back, or signing in another way, drops the attempt so a late redirect finds nothing.
  - **Not chosen: KDBusService** (KDE's single-instance helper). It would need C++ glue for one call, and the file
    is simpler and works the same for any toolkit.
  - **Dolphin's Places:** Solid lists only the FUSE mounts it knows (sshfs and rclone as network shares, encfs,
    cryfs and gocryptfs as vaults), so the mount is not among Dolphin's devices. The app's setup adds it to Places
    once, as "Add to Places" would, in `user-places.xbel`; a place the user removes stays removed, and `logout`
    takes it away. Without a Places file yet, the one written lets KIO add its own defaults around it.
  - **Dolphin's previews:** KIO counts the mount as remote, where Dolphin shows no previews. Setup and status
    offer "Show thumbnails", which sets `[PreviewSettings] MaximumRemoteSize` in `kdeglobals` to 100 MB through
    `kwriteconfig6`, or removes it to return to KDE's default. It is KDE's setting for every remote location, so
    the app changes it only when the user flips the switch, and says so beside it.
  - **Flatpak impact:** Qt opens URLs through the OpenURI portal inside a sandbox. Each sandboxed instance has a
    runtime directory of its own, so inside a Flatpak the hand-off file goes in the app's folder of it
    (`$XDG_RUNTIME_DIR/app/$FLATPAK_ID`), which every instance shares. The previews switch would need write access to
    `xdg-config/kdeglobals`, or would be left out of a Flatpak.

## The tray

The Mac's menu bar item on Linux (DESIGN.md, "The apps' interfaces"): a glance at the connection, and the way to the
drive and the app, without opening a window. **The daemon serves it,** as a StatusNotifierItem with a
`com.canonical.dbusmenu` menu (SPEC.md, "The daemon's D-Bus interface"):
- **D-Bus alone,** so the static daemon carries no toolkit, and the icon lives exactly as long as the mount, which
  is when there is something to show. The daemon already holds the session bus and knows the state.
- **KDE Plasma** shows it in its system tray, natively. **GNOME** shows it where the AppIndicator extension is on
  (Ubuntu and Bazzite ship it); stock GNOME has no tray, and its guidelines want none, so there the app and Files'
  sidebar stay the way in. Nothing is lost where no tray watches: the registration fails quietly, and is made again
  whenever a tray starts, after the daemon or again.
- **The menu is the Mac's:** the state on its first line, grey lines saying what each is, then Open in Files (or
  Dolphin) and Open Immich Mount. A click shows the menu, as on the Mac (`ItemIsMenu`). Its texts are the Mac's,
  translated once.
- **The icon** is the symbolic one, the wide drive with the five-segment ring (DESIGN.md, "The icon family"), which
  both trays recolour to the panel's text.
- **Verified** (Fedora 44 VMs, 2026-11-07 and 2026-11-08): in Plasma 6's system tray beside its own icons, the menu
  drawn with its five lines on a click (2.21.17), and both actions (Activate opens the app, Open in Dolphin opens
  Dolphin); in GNOME 50's top bar with the AppIndicator extension, the menu drawn with its five lines, connected to
  the lab server; and on Bazzite's GNOME as it ships (the extension on), installed from Homebrew's cask, with
  nothing set up (2.21.0).
- **Flatpak impact:** the sandbox needs `--talk-name=org.kde.StatusNotifierWatcher` to register; the item itself is
  served on the app's own connection, by object path.
- **Not chosen:** the GTK or Qt app holding the icon (the app would have to run at every login, a second process
  beside the daemon, and GTK 4 has no tray at all); `libayatana-appindicator` (a C library, and GTK 3, in a static
  Rust daemon).

## Updates on Linux

A package downloaded from the page is never updated: no repository stands behind it. Until one does (ROADMAP.md,
"Package repositories"), **the daemon tells of a new release:**
- **Once a day,** five minutes after it starts and then every 24 hours, it reads `latest.json` from the download page
  (the release's version and the page's address, written by `site/build.py`), the same host as the Mac's update
  feed. It sends nothing but the request and its User-Agent, `immich-mount/<version>`.
- **Told once per version:** a notification ("Immich Mount 2.21.0 is available") the first time, kept in
  `~/.local/state/immich-mount/update-told`, and a line on the tray's menu, "Get Immich Mount 2.21.0…", which opens
  the page, for as long as the daemon runs.
- **Who looks:** not Homebrew's installs (in its Cellar or a cask's folder, which `brew upgrade` keeps current), not a
  Flatpak (its remote updates it), not a development build (version 0.0.0), and not with `updates.check` off.
- **Verified** (a Fedora 44 GNOME VM, 2026-11-07): 2.21.0 from its rpms, five minutes after its daemon started, saw
  2.21.2 on the page, told of it in a notification, kept the version told of, and offered it on the tray's menu;
  started again, it offered it on the menu and sent no second notification.
- **Never replaced in place,** as Sparkle replaces the Mac app: the packages own `/usr`, and files changed under dpkg
  or rpm break their package database. The user updates the way they installed.
- **Flatpak impact:** none; a Flatpak does not look.
- **Not chosen:** GitHub's releases API (rate-limited without a token, and a second host to name in SECURITY.md); a
  check in the apps (they run only when opened).

## Translations

The apps (GNOME, KDE and macOS), the settings they show, the daemon's notifications and the Linux launcher entry are
in English and 20 other languages. Machine translation makes a language cheap, so the list follows reach: the most
spoken languages by total speakers (Ethnologue's ranking) that desktops commonly offer, with the Arabic and Chinese
varieties under their written standards and languages whose speakers mostly set another listed one (Javanese,
Nigerian Pidgin) left out, and Danish, the maintainer's: Arabic, Bengali, Chinese (Simplified), Danish, French,
German, Hindi, Indonesian, Italian, Japanese, Korean, Marathi, Portuguese, Russian, Spanish, Tamil, Telugu, Turkish,
Urdu and Vietnamese. Portuguese is Brazil's, which both platforms also offer to Portugal.

- **One source for every platform:** gettext catalogs, `po/<language>.po`, keyed by the English text and made from
  `po/immich-mount.pot`. `po/i18n.py extract` writes the template from the sources and brings every catalog in line
  with it; it finds `tr("...")` in Rust and Swift, `i18n("...")` in QML, `_("...")` in Blueprint, a SwiftUI view's
  literal title, the rows of a table of titles (`("grouping", "Date Folders")`), and the settings table's
  descriptions and choice labels, which the core keeps unmarked. Catalogs are what translation platforms and
  translators know, and GTK reads them itself.
- **Values are `{name}` placeholders,** filled in after the lookup, the same way on every platform, so a translation
  may move them. No text has plural forms: those with a count are phrased around it ("Files in Upload that have not
  reached Immich: {count}"). CI's `po/i18n.py check` fails when the template is stale or a translation drops a
  placeholder; an untranslated text shows in English.
- **The drive's folders keep their names** in every language (SPEC.md, "Names"; folder names in the user's language
  are parked in the ROADMAP), so a switch or a setting that names one (Photos, Albums, People, Places, Favorites,
  Archive, Upload) names it as the drive shows it, in a translated sentence. The extractor leaves the bare names
  out of the catalogs.
- **The core and the command line stay English.** The core holds no translation machinery, so it stays portable;
  the apps translate its texts where they show them. The command line's output is a contract (its JSON, these docs,
  `--llm`), and English there is what a search for an error finds. Errors from the server or the system, quoted in
  a message, stay as they come.
- **Linux:** `po/i18n.py mo` compiles the catalogs; the `immich-mount` package installs them in `/usr/share/locale`,
  since both app packages depend on it, and the cask keeps them in `share/locale` beside its programs. A program
  looks beside itself, in `share/locale` and then `../share/locale`, which also finds a Flatpak's `/app/share/locale`.
  `immich-mount-linux`'s `i18n::tr` reads them by gettext's rules (`LANGUAGE`, unless the locale is C, then
  `LC_ALL`, `LC_MESSAGES`, `LANG`) without the C library's gettext, so the static daemon translates its
  notifications too. GTK looks up the Blueprint's texts through the C library's gettext, which the GNOME app binds
  to the same folder. The KDE app's QML calls `i18n()`, which asks the backend; Kirigami's own texts come from KDE's
  catalogs. The launcher entry carries `Comment` and `Keywords` in every language (`po/i18n.py desktop`).
- **macOS:** `apple/build-rust.sh` runs `po/i18n.py apple`, which writes `<language>.lproj/Localizable.strings`,
  and `InfoPlist.strings` for the About panel's notice and the Local Network prompt, into the app's resources.
  SwiftUI looks up a view's literal title by itself; other texts go through `tr`. An English table is there too,
  empty, so a Mac whose first language is English and second Danish stays in English.
- **Verified** in GNOME 50 and Plasma 6 VMs (Fedora 44) under `LANG=da_DK.UTF-8`: both apps' pages in Danish,
  the GNOME app's Blueprint texts through GTK's gettext beside its other texts, and the daemon's notification of a
  refused file in Danish.
- **Made by an AI model,** and disclosed in the README; corrections from native speakers come as pull requests to
  the language's `.po` file.
- **Flatpak impact:** none. The catalogs travel in the app's own `share/locale`, which the lookup beside the program
  finds; a Flatpak's separate locale extensions are not needed for one small domain.
- **Not chosen:** Fluent (Rust-native, but foreign to GTK, Qt, Apple and the translation platforms); Qt Linguist's
  `.ts`/`.qm` for the KDE app, or an Xcode String Catalog as the source (one format per platform, kept in step by
  hand); the `gettext-rs` crate (it links libintl into the static daemon); KDE's KI18n (a C++ framework more to
  build for a few dozen texts).

## Security and privacy

- **Credentials** live in the keyring (`oo7`, which speaks both the Secret Service and the Secret portal).
  - **Why a keyring:** a credential opens a whole photo library, so it belongs in a keychain, not in a plain
    config file.
  - **KWallet** serves the Secret Service through `ksecretd`, which D-Bus can start only by its own name
    (`org.kde.secretservicecompat`); a Plasma login usually starts it through `pam_kwallet`. Where nothing has, the
    keyring code starts it before asking, so a Plasma install without `pam_kwallet` works the same. Its first use
    shows KWallet's "create a wallet" wizard. Not where the Secret Service's own name can be started (GNOME
    Keyring): D-Bus starts that one, and KWallet never takes its place on a machine that has both.
  - **The headless fallback** is a file that must be 0600, or the daemon refuses it. An environment variable is
    accepted only in tests.
  - **Never exposed:** credentials never appear in logs, URLs, D-Bus messages, cache paths or crash reports.
- **Mount options:** `nosuid`, `nodev`, `noexec`, `default_permissions`, and no `allow_other`, so only the user who
  mounted it can enter, not even root. The filesystem type is `fuse.immich`, with source `immich`.
- **Logs** (journald): endpoints and timings. No credentials and no request bodies; Asset ids only at debug level.
- **Cache directories** are 0700 and cleared on disconnect. They are not encrypted at rest; full-disk encryption is
  the user's layer.
- **TLS:** rustls with the system's root certificates (`rustls-native-certs`), so homelab private CAs added to the system trust store (for example
  with `update-ca-trust`) work. Plain `http://` is allowed for LAN servers, with a visible warning before any
  password is sent. Nothing silently downgrades from https to http.
- **D-Bus:** the session bus is the normal same-user trust boundary. The credential never crosses it.

## Packaging and releases

- **First release:** Homebrew (the cask `immich-mount`, the whole app, and the formula `immich-mount-headless` for
  servers), RPM (COPR), DEB (GitHub Releases) and AUR. All four package the same CI-built artifacts.
- **The daemon and CLI follow the house release pipeline:**
  - the version is derived from Conventional Commits: the first commit is 1.0.0, then a breaking change bumps the
    major version, a feature the minor, anything else the patch (AGENTS.md, "Versioning"). One script,
    `ci/version.sh`, gives every job and the Mac build the same number
  - the clippy gate and the tests run first; the build needs them
  - static musl builds for x86_64 and aarch64
  - every release kept, with its tag, files and notes ("Note for the house guidelines")
  - the headless Homebrew formula (`ci/immich-mount-headless.rb.tmpl`) with an x86_64 Linux bottle, pushed to a
    candidate branch of the maintainer's tap (`jakobhviid/homebrew-tap`), whose gate installs and tests it before it
    lands; `brew services` runs the daemon as `homebrew.immich-mount-headless.service`, which `logout` turns off like
    the packages' unit
  - the cask's archive, `immich-mount-app-x86_64.tar.gz`: the interfaces from the `fedora:44` job, with the static
    x86_64 daemon, its man page and completions; the tap bumps the cask daily from the releases
  - not built yet: `install.sh` for headless machines
- **The GNOME app** links the system's GTK 4 and libadwaita, so it ships through the native packages and the cask,
  not as a static bottle.
- **The KDE app** links the system's Qt 6 and loads Kirigami and the KDE style as QML modules, so its packages name
  them (`kf6-kirigami`, `kf6-qqc2-desktop-style`) besides the libraries rpm finds. cxx-qt's generated QML
  registration uses Qt's private API, which ties the package to its distribution's Qt minor version
  (`libQt6Qml.so.6(Qt_6.11_PRIVATE_API)` on Fedora 44): it is built in that release and rebuilt when Qt moves. The
  Ubuntu .deb says so with `qt6-declarative-private-abi (= 6.10.2)`, the virtual package Ubuntu's `libqt6qml6`
  provides for this, besides the libraries `dpkg-shlibdeps` finds.
- **Channel targets:**
  - **DEB:** Ubuntu 26.04 (GNOME 50). Debian 13 ships GNOME 48, below the GNOME floor, so it is not served until a
    Flatpak exists.
  - **DEB repository:** an apt repository can follow the GitHub Releases files later.
  - **Updates** come from the package managers where there is one (Homebrew today; a repository later), and are
    told of by the daemon for packages from the download page ("Updates on Linux"). The Mac app updates itself
    ("The macOS edition", "Distribution").
- **Installed files:**
  - the binaries
  - `immich-mount.service` (`Type=notify`, `ExecStop` unmounts, `Restart=on-failure`)
  - a D-Bus activation file
  - the one launcher entry (`dev.hviid.ImmichMount.desktop`, running `immich-mount app`), named "Immich Mount", with the
    keywords `Immich;photos;videos;drive;mount;` and the `x-scheme-handler/app.immich` handler, its comment and
    keywords in every language ("Translations")
  - the translations, `/usr/share/locale/<language>/LC_MESSAGES/immich-mount.mo`, in the `immich-mount` package
  - the icon, `dev.hviid.ImmichMount` and its symbolic variant, in the `immich-mount` package ("The icon")
  - AppStream metadata carrying the non-affiliation notice
  - the license notices: `LICENSE`, and `THIRD-PARTY-LICENSES.md`, written at release by
    `packaging/third-party-licenses.py`: `THIRD-PARTY-NOTICES.md` first, then every crate the package's binary links
    (from `cargo tree`) with the license files it ships, each distinct text printed once
- **Not chosen:**
  - **AppImage:** no service or autostart integration, the sign-in scheme handler needs extra desktop integration,
    and bundling GTK 4 and libadwaita drifts from the host look.

### The download page

`jakob.hviid.phd/immich-mount` is one static page: what Immich Mount is, its screenshots, and the files of the latest
release. GitHub Pages serves it, and the `pages` workflow deploys it: called by the release workflow after every
release, and run by hand to build the page again without releasing.
- **Its links come from the release itself.** `site/build.py` reads the release's file list (`gh release view
  --json`) and fills `site/index.html` with direct links to those files, so a link never points at an older version,
  and a file a release lacks shows as plain text, not a dead link. GitHub's `releases/latest/download/` links lost:
  they need file names without the version, and the packages carry it (`immich-mount-2.0.2-1.x86_64.rpm`).
- **The visitor's platform comes first.** A few lines of script read the browser's platform and put Linux or macOS
  first, marked "for your system". Both stay on the page, and nothing downloads by itself. It goes no further than
  the operating system: browsers do not reliably tell the CPU architecture, and never the distribution, so the
  Linux files are listed by distribution, x86_64 and ARM side by side. Without script, Linux comes first.
- **The supported platforms come first,** under the tagline: Linux on GNOME and KDE Plasma, and macOS 14 or later.
- **The Mac download comes from the newest release with a disk image and its update file**
  (`ci/latest-mac-release.sh`): every release the workflow makes carries both, so that is the latest one, unless its
  Mac job was switched off (`MACOS_RELEASES`). The appcast is made from the same release, so the page never offers a
  Mac version the installed apps cannot update to. Without one, the macOS block says the edition is not released
  yet, so the page never offers a file that is not there.
- **Nothing is loaded from elsewhere:** no analytics, cookies, web fonts or third-party scripts. The mark, both
  platforms' icons and the README's screenshots (`docs/screenshots/`) are copied in, and the build fails if a local
  link has no file.
- **Its look follows DESIGN.md:** the icon's palette, the lens as the hero, and each platform's own icon where that
  platform is meant.
- **Python's standard library, one template and one script,** rather than a static site generator: there is one
  page, and nothing to install in CI. CI builds it against the latest release on every branch, so a template change
  that breaks it fails before it reaches `main`.
- **No domain of its own.** GitHub serves a repository's site under its owner's user site (`jakobhviid.github.io`,
  whose domain is `jakob.hviid.phd`), so the page needs no DNS record and no domain setting. Its links are relative,
  so it works under any path.
- The page carries the non-affiliation notice word for word, the credits and the AI disclosure, as the README does.
- **Flatpak impact:** none; a Flatpak would add a Flathub link.

### Flatpak: deferred, but kept possible

A Flatpak can mount FUSE the way Pika Backup (a GNOME Circle app) and Vorta do on Flathub:
- it builds its own libfuse (`-Duseroot=false`)
- a `fusermount` wrapper runs the host's `fusermount3` through `flatpak-spawn --host`, forwarding the FUSE file
  descriptor
- it needs `--device=all` and `--talk-name=org.freedesktop.Flatpak`
- it mounts at a path both sides see (Pika uses `xdg-run/pika-backup:create`)

The mount is then a normal host mount, served from inside the sandbox.

For us it would also need `--filesystem=~/Immich:create` and `--filesystem=xdg-cache/thumbnails` (the host's cache).
The safety net would be lost: FUSE reports host process ids, which the sandbox's `/proc` cannot see.

The host-command permission is effectively no sandbox, so the Flatpak would be a delivery channel, not a security
boundary. Flathub's linter flags it as a warning (`finish-args-flatpak-spawn-access`), not an error. Acceptance is
at the reviewers' discretion; Pika Backup ships with it and needs no linter exception.

It is too much work for now, so it is parked. When it comes, it ships from our own Flatpak repository first, and goes
to Flathub only after that works.

**Comparison: Nextcloud.** Nextcloud's client is on Flathub with ordinary permissions (`--filesystem=host:rw`, the
keyring, a tray), because on Linux it **syncs real files** into a local folder: no FUSE, no host commands. Its Files
menu items come from a separate distribution package (`nautilus-nextcloud`), not from the Flatpak.

**Not chosen: a sandboxed Flatpak that syncs instead of mounting.** It would be Flathub-friendly, but:
- **every file would have to be the downloaded original.** An unprivileged app has no placeholder files that fill
  on read, and sparse files would read as zeros.
- **syncing everything** means downloading the library, which is never acceptable.
- **selective sync** breaks the "any photo from any file dialog" use case.
- **freeing space** could only delete local copies, so photos would vanish from the folder.

These rules keep a future Flatpak cheap:

| Rule | Why it matters inside a Flatpak |
|---|---|
| Mounting goes through one replaceable **mounter**: `fusermount3` natively, a host wrapper later | The sandbox cannot run the setuid helper |
| The daemon's logic never assumes **systemd**; startup lives in packaging, and `sd_notify` is optional | A Flatpak starts in the background through the Background portal |
| Every path goes through the **`Dirs` trait**, and the thumbnail cache is addressed as the host's `$HOME/.cache/thumbnails` | Flatpak remaps `XDG_CACHE_HOME` to a private directory that thumbnail lookups never read |
| Secrets go through **`oo7`** | It speaks both the Secret Service and the Secret portal |
| The browser opens through the **OpenURI portal**, not `xdg-open` | Spawning host programs is unavailable |
| **D-Bus names live under the app ID** | A Flatpak may only own names under its app ID |
| The **safety net is optional** and degrades silently | The sandbox cannot see host processes in `/proc` |
| No host programs besides `fusermount3` | Each would need another host call |

Every design change states its Flatpak impact against this table.

## The macOS edition

The same library in Finder's sidebar and in every file dialog, through Apple's File Provider, on the same core as
the Linux edition. **Released** with every release since 2.14.2, its milestone (ROADMAP, M7) not yet closed. A claim marked
*verified* was seen on the maintainer's Mac (macOS 27.0.1, Xcode 27.2) against the lab server, most of them by
`lab/mac/check.sh`; where a claim comes from Apple's documentation, it says so.

```
 Finder, file dialogs ──▶ fileproviderd (macOS: the item replica, downloaded originals, eviction)
                                  │ started on demand
            File Provider extension (Swift, sandboxed, NSFileProviderReplicatedExtension)
                                  │ uniffi
            immich-mount-apple (Rust) ─▶ core: tree, names, metadata store ─▶ immich-api ─▶ Immich
                                  ▲
                                  │ App Group container, Keychain group, signals
            Immich Mount.app (menu bar app, its window and Settings): sign-in, settings, status
```

**Why File Provider:** it is how macOS shows remote storage (iCloud Drive, OneDrive, Dropbox): files that download
when opened, eviction under storage pressure, the Finder sidebar and every file dialog, with no kernel extension.

**Not chosen:**
- **macFUSE or FUSE-T.** The Linux adapter would run nearly unchanged, but macFUSE is a kernel extension that Apple
  silicon Macs load only after lowering their boot security, and FUSE-T serves NFS from user space. Neither gives
  Finder's download progress or eviction, and both make users install a third-party filesystem first (principle 9).
- **The extension written in Rust** (through `objc2`'s File Provider bindings). One language, but every call into
  Apple's frameworks becomes unsafe foreign code, and Apple's documentation, samples and error messages speak Swift.
  The Swift layer stays a thin pass-through instead, so the logic stays in Rust, where Linux tests reach it.
- **The Mac App Store, for now.** File Provider extensions are sandboxed anyway, so the store's main rule is met,
  but an app named after Immich needs the owner's written permission there (guideline 5.2.1), and every release
  waits for review. A notarized download and Homebrew reach the same users.
- **A repository of its own.** The core is the product, and both editions consume it. In one repository a core
  change that breaks the Mac side fails in the same push, and SPEC.md stays one contract for Files, Dolphin and
  Finder.

**Two processes, as on Linux,** but macOS decides when the extension runs:
- **The extension** answers the system: listings, item details, originals, thumbnails, changes and uploads. The
  system starts it on demand and stops it when idle.
- **The app** signs in, edits settings and shows status, and keeps changes flowing while it runs (below). It is a
  menu bar app, as Mac apps that keep something in sync are (`LSUIElement`):
  - **Its menu** starts with what needs doing, when something does ("Sign In…", "Switch On in Finder…"),
    and otherwise with the connection ("Connected to <server>") and its details in grey lines below, as menu bar
    apps show them: Immich's version, the account, and when the connection was last checked, or why it failed. Then
    Open in Finder, Settings…, Sign Out… (asked first: it removes the location and what was downloaded), About and
    Quit.
  - **Its one window** is for signing in, laid out as Mail's and Internet Accounts' sign-in windows are: the drive,
    a title, and a form with its labels on the left (the server's address, then single sign-on as the main button,
    or an email and password, or an API key). After signing in it shows one step more while the location waits to
    be switched on, which nothing else on the Mac tells and without which Finder shows nothing: Open in Finder as the
    main button, and Use System Settings Instead beside it (below, "Switching the location on"). It closes by itself
    once the switch is on. It takes each step's height, keeping its top edge, as macOS's own setup windows do; the
    app resizes it after its layout pass, since a window that follows its SwiftUI content during layout is laid out
    again without end as the content changes, which stopped the app (macOS 27). Settings measures each pane once, as
    it is chosen, for the same reason.
  - **Opened by someone** (from Finder, Launchpad or Spotlight), it shows what is left to do, or Settings once
    nothing is; started at login, or for the first time after an update (its build differs from the one it last
    ran, which it keeps), it only takes its place in the menu bar. Settings is a window of its own, which
    the menu, the app menu's Command-comma and reopening all reach (below, "Settings").
  - **While a window is open** (the sign-in, Settings, About, an update's) it has a Dock icon and its menus, so
    Command-Q and pasting work as anywhere; with the last one closed it is in the menu bar only. During an update
    it keeps them until Sparkle's session ends: Sparkle closes each of its windows before it opens the next, and an
    app that leaves the Dock loses its place in front, so the next window opened behind other apps' ones.
  - Quitting it stops only the change signals; the extension serves Finder on its own.
- **Only the extension opens the metadata store.** redb allows one process, and the extension is the one that needs
  it.
- **Status goes through a file.** The extension writes what it last saw of the server (reachable or not, when, the
  server's version) to `status.json` in the App Group at each change check, and the app's status page reads it
  every few seconds (verified: the file is written, reachable and with the server's version). A file outlives the
  extension, which the system stops when idle, and needs no connection.
  **Not chosen:** the File Provider service channel (`NSFileProviderServiceSource`), an XPC connection the app opens
  through one of the location's items: it works only while the location is switched on and the extension answers,
  and asks more code of both sides for a few lines of status.
- **Shared through the App Group container:** `config.toml` (the settings table of Linux), the store and the
  extension's own files. **Shared through a Keychain access group:** the credential. The App Group is
  team-prefixed (`<team>.dev.hviid.ImmichMount`), which needs no provisioning entry; `group.` names do. A free
  Personal Team provisions the App IDs, the App Group and the Keychain group (verified, macOS 27.0.1, Xcode 27.2).

**Identity.** An item's identifier is its `NodeKey`'s storage key, the text form the store already keys names by;
`NodeKey::Root` is the root container. One item per Appearance, as on Linux. A parent is derived from the key itself
(an Asset's parent is part of its key, a month's is its year), except an album under year folders, whose year is
looked up in the album list.

**Content.** The system asks for an original when something opens the file. The extension downloads it whole into a
file the system then keeps, and evicts under storage pressure; the block cache is not used. Every item's content
policy is to download lazily, which allows evicting it (the policy replaces macOS 11's eviction capability), so Finder
offers "Remove Download" on a file or a folder, which frees the local copies and leaves the library alone (verified on
macOS 27.0.1: the system derives the eviction capability from the policy, and evicts a downloaded photo as Remove
Download does). The system keeps the details of items it holds (their capabilities and policy included), so the
bridge's item format is part of every item's version: a release that changes what items carry raises it, and the app,
seeing a new format at launch, has the system read every item again once.
- **A reimport** (after a new item format, or a settings change) drops the system's database: it then finds what is on
  disk again and offers each as a new item, marked "may already exist". The extension answers with the item the
  library has under that name in that folder; refusing it, as a read-only folder refuses new folders, left the
  system keeping its own copy beside the library's, renamed "People 2" (seen on macOS 27).
- **The import after a reimport waits on macOS,** which runs it at a priority it defers: on the maintainer's Mac, in
  use and on mains power, it had not run 10 minutes after the format changed from 2 to 3 (macOS 27.0.1). Until it
  runs, everything below the sections is the system's own local copy, not yet matched to the library: no "Remove
  Download" on it (ROADMAP, M7, has the open question).
- **Partial fetching** (`NSFileProviderPartialContentFetching`, macOS 12.3 and later) would bring range reads back
  for long videos (ROADMAP, M7).

**Thumbnails.** The system asks the extension for each item's thumbnail at a size, and the extension answers with
Immich's small thumbnail (250 pixels on its short side) up to 320 pixels, and its preview (1440) above. Nothing is
seeded, and there is no safety net: QuickLook's thumbnail service, which Finder uses, asks the extension and leaves
the original undownloaded (verified). `qlmanage -t` reads the file instead, so it is no test of this.

**Changes.** The system keeps a replica of every item it was shown, and learns of changes only by asking the working
set what changed since a sync anchor (Apple's documentation):
- **The change journal.** When the tree keeps a folder's new listing, the store compares it with the kept one and
  appends one record per item that appeared or changed, numbered in sequence. What went away departs: its deletion
  is settled once the poll's folders were all listed again, and recorded only if no kept listing holds it then, so
  an album whose year folder changed (the `year-folders` layout keys an album without its parent) is moved, not
  deleted. The anchor is the store's
  identity and a sequence number. Old records are trimmed past a limit, and an anchor older than the oldest record
  is expired, after which the system enumerates again. Linux does not use the journal.
- **Who polls.** The extension owns the poll (the same ETag and sync-stream poll as Linux) and lists again the
  touched folders the system holds, which writes the journal. The touched folders are marked in the store before the
  poll's memory is kept, and leave the mark only once listed from the server (an older listing does not count), so a
  folder the server cannot list now, or one past the poll's 20-second budget, is listed at a later poll; a listing
  shown while a change came is kept and marked the same way. The app signals the working set every 10 seconds, as
  often as Linux polls, which wakes the extension; it opens at login if the user turns that on. The poll's ETags and
  counts are kept in the store, so an extension started again still sees what changed while it was stopped; for a
  session the sync stream's next records are then changes, not a baseline. The server is asked only when the system
  has caught up with the journal.
- **A folder that went away** takes what the store kept below it: its deletion and its contents' are recorded, and
  its kept listing goes, so later polls stop asking for it.

**Settings.** The app's Settings window is laid out as Apple's Human Interface Guidelines ask of one, and as the
Mac's own apps and its sync apps (Dropbox, OneDrive) lay theirs out:
- **A toolbar of panes:** General, Folders, Other Apps and Account. The window is titled by the pane shown, opens on the pane last
  used, and takes each pane's height, so it neither resizes nor minimises. A grouped form scrolls, so it takes any
  height it is offered and has none of its own to ask; each pane tells the height of its laid-out content instead
  (its scroll geometry, macOS 15 and later; fixed heights on macOS 14), and the window follows once the height has
  settled, since a pane first laid out at a narrow width tells passing heights of thousands of points (macOS 27).
- **General:** opening at login and checking for updates.
- **Other Apps:** Immich Crosslink, the apps that take part, then Immich's web app, in the order and words of Immich
  Gallery's own Other Apps pane, so the two apps' settings read alike. The link to the protocol ends the switch's
  description (**Learn more…**), where System Settings puts such links; a link standing beside a switch is not a
  pattern the Mac's settings use.
- **Folders:** one row per section, its switch beside its date folders (greyed out while the section is hidden), then
  the album layout and file naming, so a section and its date folders are set in one place. Upload is a section too:
  its switch shows or hides the folder, and what is copied into it is uploaded; it has no date folders.
- **Account:** the connection, as the menu bar menu tells it, then Open in Finder (or Switch On in Finder…) and
  Sign Out…, which asks first; with nobody signed in, Sign In…. It repeats the menu, and is where Mac users look
  for signing out.
- **Run from elsewhere than Applications,** each pane says so, as the sign-in window does, and greys out what reaches
  Finder.
- **Grouped forms,** as System Settings has them; the centred columns of older Mac apps are the fallback should a
  form's height not settle.

The settings come from the bridge's `settings_panes`, generated from the core's settings table: the entries that
mean something on macOS, with Linux's labels. A change is written to `config.toml` in the App
Group, and the app asks the system to list everything again. The drive's name (`mount.name`) is not among them:
Finder's sidebar shows the extension's name, "Immich Mount", for a provider's only location, and a location's own
name only as a suffix once there are two (verified: renaming ours changed nothing in the sidebar; Nextcloud and
OneDrive show the same). Apple offers no way to set it. The location still carries the setting's name, as its
folder's: adding the domain again under a new name renames it in place, items and downloads kept, and the app does
so as it starts. The extension follows a changed file at the next
folder the system asks for, with a new tree on the same store; the mount's path, the cache and thumbnail seeding
are Linux's.

**Uploads.** The Upload folder lets items be added; every other folder stays a read-only view. The system hands the
extension a finished file, which goes through the core's admission (photos and videos only, AppleDouble `._` files
refused), its completeness check and its upload sequence, duplicates included; there is no staging, which Linux
needs because files arrive there in writes. Links and aliases are refused before anything is sent; folders are
taken, and the files in them go the same way ("Folders copied into Upload", above). The
uploaded file stays in Upload for about 10 seconds, as on Linux, then a journal record takes it away: the system
must have taken the item before it hears that it went. Pending departures are kept in the store, so a stopped
extension still records them, and the photo shows in its month once the poll sees it. Until it leaves, the extension
answers for the item (its name, size and date) from that list, and fetches it; the same bytes added twice under two
names are one Asset and one item, which the system merges. Until the upload succeeds,
the system keeps the file in the folder and retries; a refusal is shown with its reason.

**Signing in.** The app reaches the server before the extension ever runs, so macOS's Local Network prompt (macOS 15
and later, for a server on the LAN) names the app, and the extension shares the app's answer (Apple's TN3179). A
socket may be refused while that prompt is open, so the first check retries. Then the app offers what the server
does, as on Linux: single sign-on in a web authentication session that ends at the `app.immich` callback, a
password, or an API key. The steps live in the core's `sign_in`, which both editions call; a session is ended on the
server at sign-out. *Verified* on the maintainer's Mac (2026-11-08): single sign-on through the server's OIDC
provider adds the location. With that server's address, which resolves outside the local network, macOS asked
nothing: the prompt is for local addresses only, so it is still to be seen with a server on the LAN.

**Build.** Nothing generated is committed:
- `apple/build-rust.sh` builds the bridge as a static library for both Mac architectures, joins them with `lipo`,
  generates the Swift bindings with uniffi from the library, and wraps the result as an xcframework.
- XcodeGen writes the Xcode project from `apple/project.yml`, so the project is a text file that reviews well.
- The team ID stays out of the repository: a development build reads it from the git-ignored
  `apple/Config/Local.xcconfig`.
- **CI builds all of it on GitHub's macOS runner** on every branch push, unsigned, and runs the core's and the
  bridge's tests there. A Rust change is checked against the Swift side without a Mac at hand. It also builds the
  release's disk image there, unsigned, so the release script is checked without the signing secrets.

**The development build** is Immich Mount Dev (`apple/dev-app.sh`; AGENTS.md, "Development environment"): the
Debug configuration gives it an identity of its own (`dev.hviid.ImmichMount.Dev`, with its App Group, Keychain group,
settings and Finder location), a blue app icon, menu bar icon and sidebar symbol, and no updates. It runs beside the
installed release, so testing never signs the maintainer out, and a release never replaces it. Both answer the
browser sign-in's `app.immich://` address, which Immich fixes, so browser sign-in may reach the other one; the Dev
app signs in with a password or an API key.

**Distribution:** a notarized disk image on the GitHub release (`immich-mount-<version>.dmg`) and the Homebrew cask,
signed with the maintainer's paid Apple Developer Program team. `apple/release.sh` builds it, the same way on the maintainer's
Mac and in the release workflow's `macos` job:
- **The app is universal,** archived and signed with Developer ID, then notarized and stapled, so a copy taken out
  of the disk image passes Gatekeeper offline too. The disk image is then signed, notarized and stapled itself.
- **Two Developer ID provisioning profiles,** one for the app and one for the extension: the Keychain group they
  share is an entitlement only a profile grants (Apple's TN3125). The team-prefixed App Group needs none. On a Mac
  whose Xcode is signed in to the team, Xcode makes them as it signs; the release workflow names two made in the
  developer portal, because a manually signed build cannot use profiles Xcode manages.
- **Only the Account Holder can make the Developer ID certificate.** Xcode 27.2 shows an individual membership's
  role as Admin and does not offer it (seen on the maintainer's Mac), so the certificate is made in the developer
  portal, from a signing request made in Keychain Access.
- **Notarization uses an App Store Connect API key** (Developer role), not an Apple ID with an app-specific
  password: it is scoped to the team, and can be revoked on its own.
- **The release workflow's secrets:** `APPLE_TEAM_ID`; `MACOS_CERTIFICATE` (the Developer ID certificate with its
  key, a .p12 in base64) and `MACOS_CERTIFICATE_PASSWORD`; `MACOS_APP_PROFILE` and `MACOS_EXTENSION_PROFILE` (in
  base64, made in the developer portal and named after their App IDs); `NOTARY_KEY` (the .p8), `NOTARY_KEY_ID` and
  `NOTARY_ISSUER`; `SPARKLE_KEY`, which signs each disk image's update file. The job signs from a keychain of its own,
  deleted at the end. It runs only when the repository variable `MACOS_RELEASES` is `on` (a job's condition cannot
  read secrets), and the release goes out with whatever was built: without the job, the Linux packages alone.
- **Updates by Sparkle** (MIT), as most Mac apps outside the App Store have them: "Check for Updates…" in the menu
  and the app menu, a daily check once the user agrees (Sparkle asks; Settings has the switch), and the new version
  replaces the app in place in Applications, where Finder's location runs it from. The feed is `appcast.xml` on the
  download page, which `site/build-latest.sh` makes from the newest release with a disk image and that image's
  update file (`ci/sign-mac-update.sh`): an Ed25519 signature over the disk image, which the app checks against the
  public key in its Info.plist (`SUPublicEDKey`), and the build number Sparkle compares. OpenSSL makes the
  signature, so no Sparkle tool is needed to release. The private key is the maintainer's, kept outside the
  repository. In the sandbox Sparkle installs through its installer service (`SUEnableInstallerLauncherService`,
  with two mach-lookup exceptions); the app's network entitlement covers the download. A check sends a request for
  the feed to jakob.hviid.phd (GitHub Pages), with the app's version in its user agent, and no system profile.
- **The update window's notes** are in the feed itself: each release's "New" and "Fixed" lists, made from the
  `feat:` and `fix:` commit subjects since the release before (`ci/release-notes.sh`, also the GitHub release's
  text), the latest six releases that changed anything for users, newest first, as a small page Sparkle shows as it
  is. A release with nothing for users adds none, and with no notes at all the window shows none, only that a new
  version is out. **Not chosen:** a link to the GitHub release page, which Sparkle embeds whole, so the notes were
  a scroll away inside GitHub's page.
- **One version with Linux:** `apple/build-rust.sh` hands Xcode the release line's version (`ci/version.sh`) and the
  commit count as the build number, which grows with every release; development builds show the same.
- **Uninstalling with Homebrew signs out:** macOS has no uninstall step (dragging an app to the Bin runs nothing),
  and it keeps a provider's Finder location until the app removes it, so a deleted app left its location in the
  sidebar. The cask's uninstall script runs `Immich Mount --uninstall` once the app has quit: it signs out, as Sign
  Out does, and exits without a window. It runs only for an app whose Info.plist has `ImmichMountUninstall`, since
  an older app would ignore the argument and start, and `brew uninstall` would wait on it. A reinstall signs in
  again. **Not chosen:** keeping the sign-in through an uninstall, which leaves the credential and the session behind
  an app that is gone. The cask lives in the tap (`jakobhviid/homebrew-tap`), whose pipeline bumps only its version
  and hashes.

Development builds run under a free Personal Team on the maintainer's Mac. The location's folder is
`~/Library/CloudStorage/ImmichMount-<the drive's name without spaces>`, `ImmichMount-ImmichMount` by default.
Renaming the location moves the folder there, and keeps its items and downloads (verified, macOS 27.0.1).

**Switching the location on:** macOS keeps a new file provider off until someone switches it on, once per app and
user account; the locations the app adds after that come up on. The app adds the location switched off (verified:
`fileproviderctl dump` shows it `enabled: no` until then), and asks for the switch as the step after signing in.
Verified on the maintainer's Mac (macOS 27.2): with the Dev build's location switched off, Open in Finder showed
it with Finder's Enable button, and Enable switched it on and closed the app's window.
- **Finder's Enable button is the main way.** Open in Finder shows the location, and while it is off Finder offers
  Enable at the top of the window: one click, where the user is going anyway, with no list to search. Proton
  Drive's setup ends the same way.
- **System Settings is the fallback.** Use System Settings Instead, and Open in Finder when the system does not tell
  where a switched-off location is, open System Settings at its list of file providers
  (`x-apple.systempreferences:com.apple.ExtensionsPreferences?extensionPointIdentifier=com.apple.fileprovider-nonui`),
  where Immich Mount has a switch with its name and icon (the extension carries the app's). Login Items &
  Extensions alone (`x-apple.systempreferences:com.apple.LoginItems-Settings.extension`) opens above that list,
  and is the address left should System Settings not take the first. Verified on the maintainer's Mac (macOS
  27.2): Use System Settings Instead opens the list itself.
- **No program can switch it on,** so the install cannot either. The SDK has no call for it (`userEnabled` is
  read-only), and `pluginkit -e use`, adding the location again and starting the app through Launch Services
  change nothing (seen in a File Provider spike on macOS 27.0.1, valkyrianlabs/vaulthalla-desktop#2). The disk
  image runs no code, and the location exists only once signed in.
- **Not chosen:** System Settings as the main way, where a new app is one row of a long list; and testing mode
  (`NSFileProviderDomainTestingModeAlwaysEnabled`), which switches a location on with nobody asked but needs the
  `com.apple.developer.fileprovider.testing-mode` entitlement, made for testing. Device management can choose the
  apps whose file provider is on from the start (macOS 26.4 and later), on the Macs an organisation manages only.

**Flatpak impact:** none. The Linux edition is unchanged, and the core's additions are platform-free.

**Built so far** (ROADMAP, M7). Verified on the maintainer's Mac: the bridge's lab tests pass; the app and the
extension build and are signed by a free Personal Team; the sandboxed app signs in with a key or a password and
adds the location; and in Finder (`lab/mac/check.sh`, under a minute and a half) every section and timeline photo
lists with nothing downloaded, photos and videos read back with the server's SHA-1, thumbnails come without their
originals, a photo added to an album on the server shows in it and goes when taken out (within about 10 seconds),
a photo added while the extension was stopped shows once the app wakes it, a photo deleted on the server goes, a
photo moved into Upload loses its source, reaches the server byte for byte and leaves Upload, and a file Immich
does not take stays in Upload, refused with its reason.
- **The bridge** (`immich-mount-apple`): items by identifier, listings, whole originals and thumbnails, from the
  core's tree over the metadata store, on a tokio runtime of its own. uniffi's Swift does not pass a task's
  cancellation on to Rust, so each request takes a `Cancel`, which the extension calls from the request's progress
  and an enumerator's invalidation. A listing that takes more than 20 seconds is answered from the kept one, or as
  unreachable. Tested on Linux through a fake library, and against
  the lab server (Immich v3.3.5, `tests/lab.rs`): every section lists, all 50 timeline originals fetched match the
  server's SHA-1, an item asked for by identifier is the one listed, and the thumbnails are Immich's WebP and JPEG.
- **The core's part:** `NodeKey::from_storage_key`, the reverse of the storage key, which accepts only the form it
  writes; the tree's `parent_of` and `entry_of`; and the sign-in steps, moved from the Linux desktop crate.
  `immich-api` streams a whole original into a file. The bridge's lab tests sign in with a key and a password, and
  a session ended at sign-out is refused afterwards.
- **The change journal** in the metadata store (off on Linux), the change poll's memory kept in the store, and the
  bridge's poll, changes and working set, tested on Linux through a fake library whose server adds a photo.
- **The extension** lists, fetches and answers thumbnails through the bridge, reports changes through the working
  set from the journal, and uploads what is added to Upload. Other changes from Finder are refused.
- **Uploads** through the bridge, against the lab server: the original arrives byte for byte, and the same bytes
  again are the same Asset.
- **The app** signs in with single sign-on, a password or an API key (the server check runs in the app, for the
  Local Network prompt), keeps the credential in the Keychain group and the address in the App Group, adds the
  Finder location, wakes the extension for changes, opens at login when asked, shows the settings and the
  extension's status (the server's version, the account's email when known, the last check), and signs out.
  Its About panel carries the non-affiliation notice.
- **CI's `macos` job** runs the portable crates' and the bridge's clippy and tests on a Mac, builds the bridge for
  Xcode, and builds the app and the extension unsigned.

## Testing

| Layer | What | Where |
|---|---|---|
| Unit | Naming and clashes (stability as entries come and go, case and NFC clashes, 255-byte cuts), node keys, grouping thresholds, month keys from `localDateTime` around time zones, block arithmetic and read-ahead, the URI encoder against GLib, PNG chunks, error mapping, multipart encoding | `cargo test` in the toolbox |
| Core | The core driven through the in-memory adapter | `cargo test` |
| Contract | `immich-api` against a real server, with both the read-only and the full key, on the floor and the latest release. Run on demand and before milestones, never as part of a normal `cargo test` that would need the LAN | Erebus `lab/immich-server` (`IMMICH_VERSION=...`) |
| Sign-in | The OIDC flow against an identity provider on Erebus: Dex (static users, scriptable) for automated runs, and Pocket ID (passkeys, what the maintainer uses) for a manual check | `lab/immich-server/oidc.sh` (Dex); Pocket ID by hand |
| Mount | The daemon mounted for real: listing, checksums against the server, ranged reads (`dd` with offsets; bytes sent by the server), copies and moves into Upload with the server stopped midway, hang checks with the server down | The development host (`/dev/fuse` is there) or an Erebus container |
| Desktop | Files, the sidebar and eject, thumbnails with zero originals read (the bpftrace probe), copies and moves through Files' own operations (scripted via `org.gnome.Nautilus.FileOperations2`, or `gio copy` and `gio move`) and Dolphin's (`kioclient copy` and `move`), a video seeking in a player, both apps' first run, Dolphin, KWallet, screenshots | `lab/gnome-vm` and `lab/plasma-vm` |
| macOS | The bridge's answers through a fake library on Linux; on GitHub's macOS runner, the portable crates' and the bridge's tests and the app and extension built unsigned; Finder itself on the maintainer's Mac | `cargo test`; CI's `macos` job; the Mac |
| Packaging | Per channel, an install matrix in containers (`lab/packaging/matrix.sh`, a release's .rpm and .deb on several distributions at once: they install, `--version` matches, `doctor` runs, the apps find their libraries, register the sign-in scheme and start (GTK under headless Weston, Qt off screen), the daemon mounts the lab server as root and serves originals matching Immich's checksums, and they remove cleanly), and a full run in a fresh GNOME VM: sign in, mount, sidebar, zero originals read, checksum, remount after reboot, clean uninstall. VMs: Fedora 44 (Homebrew, RPM), Ubuntu 26.04 (Homebrew, DEB), Arch (AUR) | Erebus `lab/packaging/` |

Tests also hold the docs to the code:
- SPEC's config example parses
- WORKFLOWS names only real commands
- the D-Bus table in SPEC matches the interface
- every `--json` command writes exactly one JSON document

Before milestones, especially uploads (M3) and the first release, an independent agent runs an adversarial review
covering safety, correctness, and docs against behaviour.

## Prior art

| Project | Relation to this one |
|---|---|
| Kartax/immich-desktop-app (macOS) | The inspiration: the same idea on macOS, through Apple's File Provider. No code is shared |
| boul2gom/immich-rs (MIT) | The API layer started from it. Other Rust clients (`immich` 0.2.0, `immich_api_rs` generated for v1.137, `immich-lib` for duplicates) were stale or too narrow |
| Dynamic-Solutions-Canada/immich-drive (Windows) | The same idea on Windows, through the Cloud Files API. It also has Favorites and Partners sections (an open question in ROADMAP) |
| Katy248/immichfs (Go) and AlessandroLorenzi/immich-fuse (Python), both Linux FUSE | Read-only, and without thumbnail handling, so on GNOME both download every original of every folder opened. Thumbnails, sync, sign-in and uploads would be new code there too, so this is a project of its own |
| PersistentCloud/immich-webdav-wrapper | A different model: it runs next to the server, reads Immich's library volume directly, and covers albums only. GNOME treats WebDAV as a remote location, with no thumbnails by default and no volume |

Credits and licenses: [CREDITS.md](CREDITS.md).

## Note for the house guidelines

This project follows the maintainer's house style, `rust-cli-guidelines` (a private guide), which assumes
run-and-exit command-line tools. These are the deviations, and what they suggest for the guidelines:

- **Two binaries.** The daemon and CLI (`immich-mount`) are one binary with subcommands, as the guidelines prefer.
  The desktop app is a second binary, because it links GTK, which the daemon must not.
- **`CLAUDE.md` is a symlink** to `AGENTS.md` (the maintainer's choice), rather than the 6-line pointer.
- **Typed errors in the core,** with `anyhow` at the edges, because the FUSE adapter maps error kinds to error codes.
- **Long-running service and desktop app concerns** have no home in the guidelines:
  - a systemd user unit
  - a D-Bus interface
  - launcher entries, icons and AppStream metadata
  - packaging that links system libraries

  They are documented here, and could seed a "daemon + desktop app" chapter in the guidelines.
- **The macOS edition is mostly Swift and Xcode,** outside Cargo: a SwiftUI app, a File Provider extension, an
  XcodeGen project and a notarized DMG. Its Rust is a library the Swift code calls, so the guidelines' CLI rules apply
  to it only as far as Rust style goes.
- **The release pipeline** runs as specified for the CLI. The desktop app adds native packages (RPM, DEB, AUR) and
  a Linux cask beside the Homebrew formula.
- **Every release is kept,** where the guidelines prune to the last ten. A busy day cuts more than ten releases, so
  pruning would leave users no version to roll back to by evening, and would break any formula, cask or package
  still pointing at an older version's files. GitHub sets no limit on a repository's release storage (only 2 GiB per
  file), and nothing here reads old releases. The guidelines could drop the pruning step.

# SPEC

The contract: what Immich Mount does, as seen from outside. [ARCHITECTURE.md](ARCHITECTURE.md) explains how it does
it and why. When the code and this file disagree, the code wins and this file is the bug.

Where a section says something is not built yet, it describes the system being built. Behaviour marked *verified*
was observed on a real desktop: Fedora 44 VMs with GNOME 50 or KDE Plasma 6.8, against Immich v3.3.5.

## Vocabulary

Use these words in code, docs and issues.

- **Asset**: a photo or video in the Immich library. A file in the mount is one Appearance of an
  Asset. Avoid: item, image.
- **Mount**: the FUSE filesystem the daemon serves, by default at `~/Immich`. The file manager shows it under the
  drive's name, "Immich Mount" unless `mount.name` chooses "Immich Drive" or "Immich"; the name never changes the
  mount point. Avoid: sync folder (nothing is synchronised).
- **Daemon**: the per-user background process `immich-mount daemon`. It owns the mount, the caches and the uploads.
  It runs without a display.
- **App**: the desktop program for sign-in, settings and status, one launcher (`immich-mount app`) that opens the
  interface native to the desktop: `immich-mount-kde` (Kirigami) on KDE Plasma, `immich-mount-gnome` (GTK) elsewhere.
  It controls the daemon and can be closed at any time without stopping work.
- **Core**: the portable code (`immich-api`, `immich-mount-core`). It has no OS or desktop dependencies.
- **Adapter**: the platform code that presents the Core through one native mechanism (FUSE and freedesktop
  services on Linux now). Avoid: driver, plugin.
- **Section**: a top-level folder of the mount that mirrors an Immich section (the timeline, Albums, People,
  Places, Favorites, Archive). Each one can be switched on or off.
- **Appearance**: one path at which an Asset is visible. An Asset has one Appearance per folder it shows up in
  (its month, each album, each person, its city). Appearances share content, but each has its own path and its own
  thumbnail-cache entry. Avoid: copy, link, duplicate.
- **Date group**: an automatic `Year/MM Month` level inside an album, person or city folder, the favorites or the
  archive.
- **Content cache**: the daemon's bounded store of original bytes, filled on demand in blocks. Never the only
  copy of anything.
- **Thumbnail seeding**: writing Immich's own thumbnails into the freedesktop thumbnail cache, so the file manager
  never reads originals to draw previews.
- **Upload folder**: the mount's one writable folder, `Upload`: what is copied into it is uploaded, and leaves it
  once Immich has it. Every other folder is a read-only view, its structure built from the library's metadata.
- **Staging file**: a file being written into Upload. It lives in the daemon's staging area until its upload
  has finished.
- **Receipt**: the entry that stays in Upload for a few seconds after a file's upload, for the copy tools that set
  its times after closing it. It reads back the uploaded bytes, may be deleted (Immich keeps the photo), and is not
  renamed.
- **Asset upload**: transfer of a local photo or video into the existing library, unchanged, without creating
  anything for it in Immich (no album, no folder). Avoid: import, sync.
- **Already in library**: an upload whose content (by checksum) already exists as a non-trashed Asset.
  It counts as success.
- **Trashed duplicate**: an upload whose content matches an Asset in the Immich trash. It is restored from the
  trash and counts as success; where the credential may not restore (`asset.delete`), it fails.
- **Uncertain outcome**: a mutation whose request may or may not have reached the server. It is reported and
  checked, never blindly repeated.

## The mount

### Layout

The sections carry Immich's own labels: Photos, Albums, People, Places, Favorites, Archive. Beside them, `Upload`
takes new files.

```
~/Immich/                                  shown as "Immich Mount" (`mount.name`) in the file manager
├─ Photos/                                 the timeline
│  └─ <Year>/<MM Month>/[<DD>/]<file>
├─ Albums/
│  └─ <Year> - <Album>/[<Year>/<MM Month>/]<file>
├─ People/
│  └─ <Name>/[<Year>/<MM Month>/]<file>
├─ Places/
│  └─ <Country>/<City>/[<Year>/<MM Month>/]<file>
├─ Favorites/                              off until switched on (`sections.favorites`)
│  └─ [<Year>/<MM Month>/]<file>
├─ Archive/                                off until switched on (`sections.archive`)
│  └─ [<Year>/<MM Month>/]<file>
└─ Upload/                                 the only folder that takes new files and folders: copied in, uploaded, then gone
```

- Each section can be switched off; it then disappears from the root. Favorites and Archive are off until
  switched on: many libraries hardly use them, and a folder nobody opens is clutter.
- Month folders are named `MM Month` (`03 March`), and day folders `DD` (`15`), so sorting by name is chronological.
- **Busy months in Photos** are split into day folders by the setting `grouping.photos`:
  - `atLeast500` (the default): only months with 500 or more Assets
  - `always`: every month
  - `never`: no day folders
  Days come from the local capture time, as in Immich's timeline.
- **Album folders** follow the setting `albums.layout`:

  | Layout | Example |
  |---|---|
  | `year-prefix` (default) | `Albums/2023 - Summer trip/` |
  | `year-folders` | `Albums/2023/Summer trip/` |
  | `flat` | `Albums/Summer trip/` |

  - **The year** is the one Immich groups the album under: that of its most recent photo (`endDate`). An album
    with photos from 2018 to 2023 is a 2023 album, and moves to a later year when newer photos are added, as it
    does in Immich. (Immich uses `startDate` only when the user sorts albums by oldest photo, a browser setting
    the mount cannot see.)
  - **An empty album** has no year, and appears without one (`Albums/Summer trip/`, or in `Unknown Year/` with
    `year-folders`).
- **Date groups** inside albums, people, cities, the favorites and the archive follow a per-section mode: `never`,
  `atLeast500` or `always`. With `atLeast500`, a folder with 500 or more Assets is split into years and months. The
  default is `atLeast500`, and `always` for the favorites and the archive: they mostly grow, and with `atLeast500`
  their paths would all change the day they reached 500.

### Membership: Immich decides

A folder holds exactly what Immich puts there. The mount never recomputes membership from dates where Immich
has an answer.

| Folder | Source |
|---|---|
| Photos, its years | `GET /timeline/buckets?visibility=timeline` |
| A month | `GET /timeline/bucket?timeBucket=YYYY-MM-01` (the month's asset ids, grouped by local capture time) |
| A day, in a split month | The month's Assets, by the day of their `localDateTime` (the rule Immich's web timeline uses for its day headers) |
| Albums | `GET /albums`: owned albums, and albums shared with the user. `endDate` gives the year |
| An album | Search by `albumIds`; date groups by each Asset's `localDateTime` |
| People | `GET /people`, without hidden and unnamed persons |
| A person | Search by `personIds`; date groups by each Asset's `localDateTime` |
| Places | `GET /search/suggestions?type=country`, then `type=city&country=` |
| A city | Search by `city` + `country`; date groups by each Asset's `localDateTime` (Immich has no city timeline) |
| Favorites | Search by `isFavorite`, what Immich's Favorites page shows, archived favorites included; date groups by each Asset's `localDateTime` |
| Archive | Search by `visibility: archive`, what Immich's Archive page shows; date groups by each Asset's `localDateTime` |

**Undated Assets.** The API never returns an Asset without a date, because an upload must send one. A file with no
metadata sits wherever its uploader's date put it (the lab's ended up in 1970), and the mount shows it where Immich
does.

### Visibility

| Asset | Shown |
|---|---|
| On the timeline | Yes |
| Archived | Only inside albums, Favorites (when it is a favorite) and Archive |
| Hidden (the video half of a Live Photo) | No; the still is shown |
| In the locked folder | No |
| Trashed | No; it disappears at the next refresh |
| Shared by a partner | No (an open question in ROADMAP) |
| Part of a stack | Every Asset of the stack is shown |

### Names

Every name in a folder is unique, and a path keeps meaning the same Asset for as long as it exists.

**File names** follow the setting `files.naming`:

| Naming | Example | Built from |
|---|---|---|
| `date-time` (default) | `2024-03-15 14.32.07.jpg` | The local capture time (`localDateTime`), with dots instead of colons; the extension as in the original |
| `original` | `IMG_1234.JPG` | The original file name |

**Clashes.** Two Assets in one folder can share a name: shots in the same second, or `IMG_0001.JPG` from two
cameras. Clashes are detected case-insensitively, after Unicode NFC normalisation:
- The Asset uploaded first (earliest `createdAt`, then id) keeps the plain name.
- The others get a counter before the extension, in upload order: `2024-03-15 14.32.07-1.jpg`,
  `2024-03-15 14.32.07-2.jpg`, `IMG_0001-1.JPG`.
- **Assigned names are remembered.** A counter never moves to another Asset while the one holding it exists.
  Deleting one leaves a gap rather than renumbering the rest.
- Folders that clash (two albums or two people with the same name) get the same treatment: `Anna`, `Anna-1`.

**Sanitising**, for every name:
- `/` becomes `_`, and NUL is removed
- leading and trailing whitespace is trimmed
- `.` and `..` become `_`
- names over 244 bytes are cut on a UTF-8 boundary, keeping an extension of up to 16 bytes, so a clash counter
  still fits within the 255-byte limit
- a leading `.` is kept (it hides the entry, as its owner named it)

### Attributes

| Attribute | Files | Folders |
|---|---|---|
| `st_size` | The original's exact size (`exifInfo.fileSizeInByte`; if missing, a `HEAD` of the original). A size that stays unknown makes reads fail with `EIO`, never look empty | 0 |
| `st_mtime`, `st_atime`, `st_ctime` | The capture time (`fileCreatedAt`), so sorting by date follows shooting order and copies keep it | The mount time |
| `st_mode` | `0644` | `0755` |
| `access()` | Read; never write | Read and search; write only for Upload, with uploads |
| owner | The mounting user | The mounting user |
| `st_nlink` | 1 (Appearances are separate files, not hard links) | 2 |
| `st_blocks` | `ceil(size / 512)` | 0 |

### Files the mount answers itself

- `/.xdg-volume-info` (`[Volume Info]`, `Name=Immich Mount`, the drive's name from `mount.name`, and
  `Icon=dev.hviid.ImmichMount`): the display name in the file manager (verified in a GNOME 50 VM), and GVfs's
  full-colour icon.
- `/.directory` (`[Desktop Entry]`, `Icon=dev.hviid.ImmichMount`): the mount's folder icon in Dolphin, where previews
  of remote folders are on (ARCHITECTURE.md, "The drive's icon").
- `/.nomedia` and `/.trackerignore`: hidden; they keep GNOME's indexer out.
- Anything else that tools probe for (`.hidden`, `.directory` below the root, `.Trash-<uid>`, `autorun.inf`, `.DS_Store`, and
  rom-properties' `code`, `title.tik`, `default.xex` and `default.xbe` in their spellings): a fast `ENOENT`, which
  the kernel remembers for a minute (a negative entry), so a folder's next probes never reach the daemon. Such a
  name never waits for the server: without the folder's listing at hand it is answered absent at once, not
  remembered, and the folder is listed in the background. Only these known names are remembered: any other missing
  name, a dot-name of an album or a file included, may still arrive from Immich.

### What the mount refuses

- The modes are what a copy gets, so a photo copied out is an ordinary file (`0644`) and a copied folder an ordinary
  folder (`0755`). What the mount allows is `access()`'s answer, which file managers read for their read-only
  emblem (GNOME Files' lock), and each operation's own refusal.
- Every folder but Upload and the folders copied into it is a read-only view: write access, rename, delete and
  `setattr` on Assets, and new files or folders, fail with `EROFS`. "Move to Trash" and "Paste" in the file manager
  therefore fail there.
- In Upload, a hidden folder (`mkdir` of a name starting with `.`) fails with `EPERM`, and so do `mknod`, `symlink`
  and `link`. A folder copied into Upload takes the times and modes it is given and keeps none of them, and may be
  removed (`rmdir`) only while empty, else `ENOTEMPTY`; it cannot be renamed (`EPERM`).
- With uploads off (`uploads.enabled = false`), there is no Upload folder and the mount is read-only as a whole
  (`ro`).
- Desktop indexers get `EIO` when they open an original: KDE's Baloo extractor (`baloo_file_extractor`) and GNOME's
  (`localsearch-extractor-3`, `tracker-extract-3`). Always, not a setting: indexing the contents would download the
  library. Their file-name walks are answered as usual.

## Content

- Opening a file never waits for a download. Reads fetch the blocks they need, in 2 MiB blocks with HTTP range
  requests and sequential read-ahead.
- Cached blocks are served offline. An uncached read fails fast with `EIO` while the daemon is offline: once a
  request finds the server unreachable, every request fails at once for up to 45 seconds, and the change poll's next
  answer from the server ends that early.
- A read that makes no progress for 30 seconds fails with `EIO`. No filesystem call waits on the network without
  a deadline.
- A small read near a file's start or end that the cache cannot answer (a peek: a run of reads within 64 KiB)
  fetches the original's first 64 KiB and last 16 KiB together, and keeps them. Once a program has peeked at three
  files of a folder within 10 seconds, the start and end of the folder's other files, up to 1,000 of them, are
  fetched ahead of it, eight files at a time.
- The content cache has a size limit (default 10 GB). It evicts least-recently-used Assets that are not being
  read. It never takes the disk's last tenth: with less than 10% of the disk free, it evicts as if over its limit.
- Originals are byte-identical to the server's.
- The library is never mirrored.

## Thumbnails

- On the first listing of a folder with Assets, every Appearance gets a freedesktop thumbnail (verified in GNOME 50
  and Plasma 6.8 VMs):
  - **Where:** `$HOME/.cache/thumbnails/x-large/<md5 of the file URI>.png`, made from Immich's `thumbnail` size
    (a WebP with a short side of 250 px), re-encoded as PNG.
  - **On KDE** (Plasma's shell on the session bus, or `XDG_CURRENT_DESKTOP` naming KDE), also `normal/` (128) and
    `large/` (256) from the same image: the
    sizes Dolphin asks for at its smaller zoom levels.
  - **Format:** PNG `tEXt` chunks `Thumb::URI` and `Thumb::MTime`, file mode 0600, written atomically.
  - **Timing:** the listing waits up to 10 seconds for these. After that it answers anyway, and seeding continues
    in the background.
  - **Counts:** a listing of a folder whose parent's listing, or another count in it, ended less than 3 seconds
    before, after a sibling was listed, and which was not listed itself in that time, is taken for a file manager
    counting its items. It answers at once; its small thumbnails are written in the background, behind the folders
    being shown, and it gets no sharp thumbnails. Within a minute of such a listing, the first `access()` on one of
    its files (a file manager showing it after all) waits up to 10 seconds for its thumbnails, sharp ones following
    in the background.
- **Sharp thumbnails** (setting, on by default): each Appearance of a folder shown also gets an `xx-large/` (1024)
  thumbnail, downscaled from Immich's `preview`, in the background.
- **The safety net** (setting, on by default): when a known thumbnailer process opens an original (`glycin-thumbnailer`
  and others), it gets `EIO` instead of the bytes. It never gets substitute bytes.
- Listings made by the file manager's search, or by an indexer walking the tree (`baloo_file`, `localsearch-3`), do
  not trigger seeding.
- Every seeded file is recorded, and removed again when signing out (`immich-mount logout`, or Disconnect in the app).

## Uploads

- Only the Upload folder accepts new files, and only while `uploads.enabled` is on and the credential may upload.
  An uploaded file leaves Upload and shows in Photos under its own date (and in albums, people and places as Immich
  files it); where it was dropped never decides where it appears. With uploads off, there is no Upload folder.
- Only image and video files are uploaded (the server's media types, RAW included), on their own or in folders
  copied into Upload (below). Symlinks, other files, Live Photo pairs and XMP sidecars are not supported.
- **Upload on close:** the upload runs when the writing program closes the file. `close()` returns success only
  after the server has answered. If the upload fails, `close()` fails too, so the program reports the error and a
  move keeps its source, in programs that check `close()` (Files, `mv`, `cp`, rsync). Dolphin does not: its move
  deletes the source anyway.
- **Nothing that failed to upload is thrown away:** the file is kept in `~/.local/share/immich-mount/not-uploaded/`
  under its own name (numbered if the name is taken), and the notification says so. So is what a daemon that died
  mid-upload left behind, at its next start. Only what the user dropped is deleted: a cancelled copy that ends short, an
  empty file, a file deleted or replaced in Upload.
- **Where the kernel limits one file operation** (`fs.fuse.default_request_timeout`, Linux 6.14 and later), an
  upload, a read and a listing each end a second before that limit: a longer upload fails its `close()` with `EIO`
  and is stopped, with one notification naming the setting, and the mount stays up. Off by default.
- Files are sent unchanged: no resizing, transcoding, metadata stripping or date rewriting, and no albums created.
- **Outcomes:**
  - A new Asset or **Already in library** counts as success.
  - A **trashed duplicate** is restored from the trash, as it was (its date, albums and faces), and counts as
    success; on Linux a notification says so. Without `asset.delete` it fails, and stays in the trash.
  - Temporary server errors are retried, within a time limit.
- **Temporary names:**
  - A hidden photo (`.IMG_1.JPG`) uploads on close like any photo. Known temporary names (GIO's
    `.goutputstream-*`, rsync's `.<name>.<6 characters>` around a photo's name) are staged, and uploaded once
    renamed to the photo's name. Any other hidden name is refused, and so are macOS's `._` companion files.
  - An empty file is never uploaded: it disappears from Upload when its last handle closes.
  - After an upload, the written name stays in Upload as a **receipt** for about 10 seconds, then disappears; it
    may be deleted before then. The Asset appears in its date folder in Photos. **Already in library** (the same
    bytes are in Immich, a photo copied from the mount itself, say) adds nothing, and a notification says so.
- A desktop notification explains refused files, and reports long uploads and failed ones. Repeated refusals of
  one kind within 20 seconds give one notification.
- **What programs see:**

  | Situation | Error |
  |---|---|
  | A file type Immich does not take, at `create` or on a rename | `ENOTSUP` (Operation not supported) |
  | A name an Asset already has in that folder | `EEXIST` |
  | The same bytes are in the trash, and the credential may not restore them | `EEXIST` on `close()` |
  | The credential may not upload | `EACCES` on `close()` |
  | Server unreachable or busy past the limit | `EIO` on `close()` |
  | A hidden folder, a link or a device node in Upload; renaming a folder there | `EPERM` |
  | Removing a folder in Upload that is not empty | `ENOTEMPTY` |
  | Any change outside Upload | `EROFS` |

### Folders copied into Upload

- **A folder copied into Upload is taken whole,** with the folders inside it: every photo and video in it uploads to
  the timeline as it arrives, exactly as a single file does (on close, with the same outcomes and errors), and other
  files in it are refused as they would be in Upload.
- **Settling:** no program tells that a copy is coming or how big it is, so a copy counts as finished once nothing
  new has arrived in it for 5 seconds and nothing in it is being written or uploaded. It is asked about within a few
  seconds of that.
- **Albums,** once it has settled, as `uploads.folders` says:
  - `ask` (the default): Immich Mount asks, naming the folder, counting what uploaded and what did not, and
    listing the albums it would make, with two buttons: **Make Albums** and **Timeline Only**. Dismissing the
    question, or no answer within a day, is the timeline only. Where nobody can be asked (no desktop session, or
    notifications without buttons), it is the timeline only.
  - `albums`: the albums are made without asking. `timeline`: none, as for single files.
  - A copy with nothing uploaded (an empty folder, or only refused files) makes no album and asks nothing.
  - **Timeline Only is the way out:** nothing more happens, and the photos stay in the timeline. Nothing here takes
    an upload back; a photo uploaded by mistake is deleted in Immich, as any upload is.
  - **A change of mind is another copy:** a copy answered Timeline Only that is copied into Upload again is asked
    about again. Immich recognises the photos it has, so nothing is sent twice, and Make Albums takes them all.
- **Which albums:** one for each folder that directly holds photos or videos that uploaded, holding those (not those
  of the folders inside it). It is named by the folder's path below Upload, joined with ` › `: `Trip`,
  `Trip › Day 1`. A name an album has already (the account's own, or one shared with it, compared ignoring case and
  Unicode form) gets the day's date: `Trip - 2026-11-09`, then `Trip - 2026-11-09 (2)`. A photo Immich had already
  goes into the album too, and so does one restored from its trash.
- **After the albums:** what arrives later in the same copy, while it is remembered (10 minutes after its last
  file), joins its albums, and a new folder in it gets one of its own. A folder of the same name copied after that
  is a new copy. An album made by a copy and deleted since is not made again.
- **Failures:** making albums needs `album.read`, `album.create` and `albumAsset.create`. A server that cannot be
  reached, or is busy, is tried again a minute later, for up to a day; an album made whose answer was lost is found
  again by its name and first photo, never made twice. Any other failure (no permission, signed out, a request the
  server refused) is told in a notification, and the copy stays in the timeline only.
- **The folders leave Upload soon after their copy,** about as soon as their last file has (their files leave as
  single files do):
  - **On Linux** a copy's folders go together, once none of them holds a file and nothing arrived in any of them for
    5 seconds. For 10 minutes after, a gone folder looked up by its path comes back, so a copy that only paused
    (Files stops at a file Immich does not take until Skip or Skip All) goes on into it. A folder of the same name
    copied again within those 10 minutes is that folder: Files asks to merge (Dolphin, that the folder exists), and
    its photos join the earlier copy.
  - **On macOS** a folder goes 20 seconds after its last file (or as long as macOS takes to bring its list up to
    date, and 10 seconds more), once macOS says it holds nothing the extension has not been handed (Finder may still
    be writing a large file into it). Where macOS does not say, or says its list is full, it stays 10 minutes.
  - An empty one may be removed before then.
- **Files stops at a file it cannot copy** (one Immich does not take) and asks; Skip All goes on with the rest. A
  copy with nothing uploaded yet waits for its first upload before it is asked about.
- **Hidden folders** (a name starting with `.`: a thumbnail cache, a version-control folder) are refused, without a
  notification; a hidden photo still uploads.
- **On Linux** the daemon asks, as an urgent desktop notification, which stays on screen until a button is
  pressed or it is dismissed; a click on the notification itself answers nothing, and it asks again. It looks for
  the desktop's notifications when a question is due, and a question waits for them, so a daemon that started
  before the desktop still asks. A restarted daemon (a settings change) asks again what was being asked, in place of the notification it showed
  before. A file of a copied folder that did not upload is kept under its folder's path:
  `not-uploaded/Trip/Day 1/IMG_1.JPG`.
- **On macOS** the menu bar app asks, in a dialog that stays above other apps until one of its two buttons is
  clicked; neither answers to a key, so typing elsewhere cannot answer it. It asks while it runs: a copy that
  settles while it does not is asked about when it next starts. What came of the albums is told in notifications.
  A file that did not upload stays in its folder in Upload, with its reason, and the folder with it; deleting the
  folder in Finder lets both go.

## Links to Immich

Each item the mount shows that Immich's web app has a page for has a **web address**: the server's address
(`server.url` without a trailing slash and without `/api`) followed by the item's **Immich path**, the path of
Immich's web app for it (`web/src/lib/route.ts` and `web/src/routes/(user)/` in immich-app/immich, which Immich's
mobile app also takes as deep links). A photo keeps the folder it is listed in, so its page opens in that album,
person or place.

| Item | Immich path |
|---|---|
| A photo in Photos (in a month or a day folder) | `/photos/<id>` |
| A photo in an album, directly or in its year and month folders | `/albums/<album>/photos/<id>` |
| A photo in a person, likewise | `/people/<person>/photos/<id>` |
| A photo in Favorites or Archive, likewise | `/favorites/photos/<id>`, `/archive/photos/<id>` |
| A photo in a city, likewise | `/search/photos/<id>?query=<q>` |
| An album, a person, Favorites or Archive, and their year and month folders | `/albums/<album>`, `/people/<person>`, `/favorites`, `/archive` |
| A city, and its year and month folders | `/search?query=<q>`, `q` = `{"city":"<city>","country":"<country>"}` |
| A country | `/search?query=<q>`, `q` = `{"country":"<country>"}` |
| Photos, Albums, People, Places | `/photos`, `/albums`, `/people`, `/places` |
| The mount itself, Photos' year, month and day folders, the year folders above albums, Upload and everything in it | none: Immich has no page for it |

- `q` is the JSON Immich's web app writes for a search: its keys in the order shown, no spaces, encoded as
  JavaScript's `encodeURIComponent` does. City and country names are Immich's own, never the folder's name on disk.
  Immich's Places page searches for a city by its name alone; the country is added so two cities of one name stay
  apart.
- The same item gives the same address on every platform: the map is in the core (`links.rs`).

**Open in Immich** opens the web address in the default browser, and **Copy Immich Link** puts it on the clipboard.
Both are offered on one item at a time, inside the mount, for items with a web address, and nowhere else, while
the switch **Open in Immich and Copy Immich Link** (`web.links`, under Immich Web App in the settings, on by default)
is on:
- **In Files:** a Nautilus extension, in the `immich-mount-gnome` package (Nautilus's extension folder, such as
  `/usr/lib64/nautilus/extensions-4/libimmich-mount-nautilus.so`). Homebrew's cask, which cannot write there,
  installs the same items as a nautilus-python extension (`~/.local/share/nautilus-python/extensions/`), which
  Bazzite ships; it offers nothing where the packaged extension is installed too, so the items never show twice.
  Items whose address is not a file (Recent, Starred) have none.
- **In Dolphin:** a file item action plugin, in the `immich-mount-kde` package
  (`<Qt plugins>/kf6/kfileitemaction/immich-mount-dolphin.so`). With Homebrew's cask, Dolphin finds it from the
  next login, through `QT_PLUGIN_PATH`, which a Plasma login script sets (Files on disk).
- **In Finder:** two actions of the File Provider extension (In Finder). They work whether the menu bar app runs or
  not.
- **In a terminal:** `immich-mount web <path>`, `--copy`, and `--json` (The `immich-mount` command).
- **The daemon** answers `Links` on D-Bus (The daemon's D-Bus interface), which the items in Files and Dolphin
  ask with a 250 ms limit, never starting the daemon, falling back to `WebLink` against a daemon without it; after a
  call that timed out, Files does not ask again for 30 seconds. It finds the item from the folder listings it has, so
  a path the file manager shows needs no request to the server. With no daemon answering, the items are not offered.

### Links from other apps (Immich Crosslink)

Immich Mount takes part in [Immich Crosslink](https://jakob.hviid.phd/immich-crosslink/), draft 0.2: other Immich
apps on the computer can ask it to show a photo, album, person or place, and it shows the item in the file manager,
selected.
- **Announcing:** the launcher entry registers the scheme (`MimeType=…;x-scheme-handler/immich-crosslink;`) and
  announces `X-Immich-Crosslink-Version=0.2` and `X-Immich-Crosslink-Shows=asset;album;person;place;page;`. A link
  reaches `immich-mount app %u`, which hands it to the daemon (`ShowLink`) without opening the app.
- **Reading:** a link is *invalid* (another scheme, a host, no `server` or one that is not an `http` or `https`
  address, an id that is not a UUID, a malformed search), *cannot show* (a path or a search the protocol does not
  define), or names an item. The server must be the signed-in one (scheme, host without case, port, a default port
  the same as none), and the user too where the link names one and the credential may read the account's id.
- **Where it shows:** where the link says: a photo in its album, person, city, Favorites or Archive, in the date
  folders it falls in there; `/photos/<id>` in Photos, in its month (or day) folder, or in Archive when the photo is
  archived and that section is on. A photo whose folder the drive does not show, or whose folder does not hold it,
  shows in that place of its own instead, as the protocol asks. An album, a person, a city (also by its name alone, as Immich's Places page
  links it) and a country show as their folders, a section as its folder at the top.
- **Nothing to show** gets a notification, and nothing else happens:

  | Result | Notification |
  |---|---|
  | Immich Crosslink off | "Links from other apps are off": turn on Immich Crosslink in the settings |
  | Invalid | "A link from another app cannot be opened": not a valid Immich link |
  | Cannot show | "Nothing to show": no place for what it points to |
  | Another server | "Not signed in to *host*" |
  | Another user | "Another Immich account" |
  | Not in the drive (an id the account cannot see, a hidden person, a section that is off) | "Nothing to show": the drive does not show it |
  | Not mounted | "Immich Mount is not mounted": mount it, then open the link again |

- **On the Mac** the app's Info.plist announces the same (`ImmichCrosslink`: `Version` `0.2`, `Shows` the five
  kinds) and declares the scheme (`CFBundleURLTypes`). The menu bar app takes the link, without opening a window
  when a link started it, and Finder selects the item (`getUserVisibleURL`, then `activateFileViewerSelecting`),
  listing the folders above it first where Finder never has. The results and notifications are Linux's; signed out,
  a link gets "Not signed in", and clicking "Links from other apps are off" opens the app's settings.
- **The default handler:** several apps may take the scheme. Immich Mount never makes itself the default: apps that
  take part send a link to one app by name, and a link opened in a browser or with `xdg-open` or `open` goes to the
  app the system chose, which the user can change (`xdg-mime default dev.hviid.ImmichMount.desktop
  x-scheme-handler/immich-crosslink`).
- **Only showing:** any web page can open a link, so a link only ever selects a file or folder. Nothing is fetched
  but the item's folder listings and, for a photo, its details (`GET /assets/{id}`); nothing is downloaded,
  uploaded, changed or deleted, and nothing asks.

### Show in other apps

Beside Open in Immich and Copy Immich Link, **Show in *app*** sends the item's crosslink to another app that takes
part in Immich Crosslink, which shows it there.
- **Which apps:** those whose desktop entry takes the scheme and announces draft 0.2 (`X-Immich-Crosslink-Version`)
  and the item's kind (`X-Immich-Crosslink-Shows`), less Immich Mount itself and the ones switched off in the
  settings (`crosslink.hidden`). A photo's kind is `asset` wherever it is listed; an album's `album`, a person's
  `person`, a city's or a country's `place`, a section's `page`. Each is named and drawn by its desktop entry.
- **Sending:** the crosslink, with the signed-in user's id when known, goes to that app alone (GIO's
  `g_app_info_launch_uris` in Files; in Dolphin the app's own command, made from its desktop entry), never to the
  scheme's default, and byte for byte as the daemon wrote it.
  An app whose program is missing (its `TryExec`, where it has one, or the program of its `Exec`) is not offered. When sending fails, a notification titled "Immich Crosslink" says
  "*app* is no longer installed." (its desktop entry gone since the menu was made) or "Immich Mount could not open
  *app*: *the system's reason*", and the next menu reads the apps again.
- **Off:** with `crosslink.enabled` off, no Show in items; Open in Immich and Copy Immich Link stay.
- **The settings** (in both apps, and on the Mac): under Other Apps, the **Immich Crosslink** switch, described as
  showing items in other apps that take part and showing the items they send in the file manager (in Finder, on the
  Mac), with a link to the protocol whose address is not shown: **Learn More** beside the switch on Linux, **Learn
  more…** at the end of the description on the Mac. Then **Apps That Take Part**, each by its name and icon with a
  switch, described as "Apps that take part are found by themselves. Turn one off to leave it out of Show in. One not
  found stays listed for 30 days, so it stays off if it comes back; Forget removes it now." With none, "No other app that takes part is installed." and a link to the protocol's participants (**Find Apps
  That Take Part**, **Find apps that take part…** on the Mac). Then **Immich Web App** (Links to Immich). An app seen
  before but not found at the moment (not announcing, unreadable, or its program gone) stays listed for 30 days, as
  **Not found**, by its remembered name and a generic icon, with its switch and a **Forget** button. Forgetting it, by
  hand or after 30 days unseen, drops its switch too, so an app that comes back is new and on. An app that is found has
  no Forget: it would be found again at once. What was seen is the app's state
  (`~/.local/state/immich-mount/crosslink-seen.json` on Linux, the App Group on the Mac), not a setting. The menus
  offer only apps that are found. No app is set apart: Immich Mount names and offers no app of its own accord.
- **With web links off,** the menus offer Show in *app* alone.
- The menus ask the daemon's `Links`; against a daemon without it they offer Open in Immich and Copy Immich Link
  only.

## Signing in and credentials

| Method | Offered when | Desktop | Headless | Stored |
|---|---|---|---|---|
| Browser (OIDC) | The server has OAuth (`/server/features` `oauth`) | "Sign in with <server's button text>" opens the user's browser; the provider redirects to `app.immich:///oauth-callback` | `immich-mount login` prints the URL; the final redirect URL is pasted back | Session token |
| Email and password | The server allows password login (`passwordLogin`) | A small form | `immich-mount login --password`: a terminal prompt, or stdin | Session token; the password is never stored |
| API key | Always, as the advanced option | "Use an API key instead" | `immich-mount login --api-key` (stdin or a file) | The key |

- Credentials live in the keyring (Secret Service). Without a keyring they live in a 0600 file next to the config.
  - `immich-mount login` stores into the keyring inside a desktop session, and into the file otherwise (over SSH,
    on a server) or when `credential.file` is set.
  - Reading prefers a credential file when one exists, then the keyring.
- Signing in happens once. A credential persists across restarts and reboots until Immich or the identity provider
  revokes it, or the user disconnects.
- A locked keyring triggers the system's unlock prompt. If the user dismisses it, the daemon says so in its log and
  stops cleanly, so the service is not restarted into another prompt, and one notification says why. There is no
  repeated prompting, and no silent failure. (An **Unlock** action on it comes with the daemon's D-Bus control.)
- A revoked credential (401) leads to one notification, and the app reopens sign-in for the same server and account.
  Caches are kept.
- The redirect to `app.immich:///oauth-callback` opens the app that handles the scheme. A redirect carrying the
  provider's refusal (`error`, with our state) ends the waiting sign-in with that message. The app's launcher entry
  declares it, and the app makes it the scheme's default handler (in `~/.config/mimeapps.list`) as it starts a
  browser sign-in; the launcher hands the address to the interface for the desktop. The GTK interface is single
  instance and receives it in its open window. The Kirigami one is started a second time with the address, finishes
  the sign-in from the waiting attempt (Files on disk), and exits; its open window then continues to setup.
- Immich's password login has no two-factor step. Two-factor comes through the server's OIDC option, and the
  password form says so.

## Supported versions

- **Immich:** the current and previous minor release; today v3.3 and v3.2, so the floor is v3.2.0. Below the floor,
  sign-in stops with "Immich v3.2 or later is required".
- **GNOME:** the current and previous release; today GNOME 50 and 49.
- **Linux:** a kernel with FUSE protocol 7.31 or later, a setuid `fusermount3`, and access to `/dev/fuse`.

## Configuration

`~/.config/immich-mount/config.toml`. Unknown keys are ignored on read, and writes preserve comments. One settings
table in the Core drives validation, `immich-mount configure`, and the apps' settings pages, where each setting has
one place (ARCHITECTURE.md, "One layout for every settings window"): General, Folders, Other Apps, or the command
line only (the server, the apps left out of Show in, the headless credential file and `thumbnails.safety_net`).

| Key | Values | Default | Meaning |
|---|---|---|---|
| `server.url` | URL | none | The Immich server; `/api` is added |
| `mount.path` | path | `~/Immich` | Where the mount appears |
| `mount.name` | `immich-mount`, `immich-drive`, `immich` | `immich-mount` | The drive's name in the sidebar: "Immich Mount", "Immich Drive" or "Immich" in Files and Dolphin. The mount point stays `mount.path`; the apps apply it by restarting the daemon. Linux only: Finder's sidebar shows the app's name, "Immich Mount", for a provider's only location |
| `mount.at_login` | bool | `true` | Mount when the session starts: setting it enables or disables the user service |
| `sections.photos` / `.albums` / `.people` / `.places` | bool | `true` | Show the section |
| `sections.favorites` / `.archive` | bool | `false` | Show the section |
| `grouping.albums` / `.people` / `.places` | `never`, `atLeast500`, `always` | `atLeast500` | Date groups |
| `grouping.favorites` / `.archive` | `never`, `atLeast500`, `always` | `always` | Date groups |
| `grouping.photos` | `never`, `atLeast500`, `always` | `atLeast500` | Day folders inside months |
| `albums.layout` | `year-prefix`, `year-folders`, `flat` | `year-prefix` | How album folders are named and grouped |
| `files.naming` | `date-time`, `original` | `date-time` | How files are named |
| `cache.limit` | size (`10GB`, `500MB`, or binary units: `10GiB`) | `10GB` | Content cache limit. The apps offer 5, 10, 25, 50 and 100 GB, and show any other size as "Custom" |
| `uploads.enabled` | bool | `true` | Show the Upload folder, and upload what is copied into it (also needs the server's upload permission) |
| `uploads.folders` | `ask`, `timeline`, `albums` | `ask` | What a folder copied into Upload becomes besides its uploads: asked each time once the copy has settled, the timeline only, or an album of each folder too ("Folders copied into Upload") |
| `thumbnails.sharp` | bool | `true` | Also seed 1024 px thumbnails from `preview` |
| `thumbnails.safety_net` | bool | `true` | Refuse thumbnailer reads of originals. In `config.toml` and `configure` only: off, a thumbnailer may download every original it is shown |
| `web.links` | bool | `true` | Open in Immich and Copy Immich Link in the file managers' menus (Links to Immich); off, the daemon gives no web address and the menus offer Show in *app* alone |
| `crosslink.enabled` | bool | `true` | Immich Crosslink: show in the file manager what another app links to, and offer Show in *app* (Links to Immich). Off, a link from another app only gets a notification saying so. Labelled "Immich Crosslink" in the apps, in English in every language, with a link to the protocol |
| `crosslink.hidden` | ids separated by `;` | none | The apps left out of Show in *app*, by desktop entry id (`org.example.Photos.desktop`) or bundle id; the apps show a switch per app found instead |
| `updates.check` | bool | `true` | Tell, once per version, when a new release is out: a notification and a line on the tray's menu (ARCHITECTURE.md, "Updates on Linux"). Only packages from the download page look; Homebrew's installs are left to `brew upgrade`. Linux only: the Mac app has Sparkle's own setting |
| `credential.file` | path | none | Headless only: a 0600 file holding the credential |

The config file carries a format version; older files are migrated once on load.

## The `immich-mount` command

One binary: the daemon and the command line.

| Command | Does |
|---|---|
| `immich-mount` | Opens the app where there is a display (as `immich-mount app`); prints this help where there is none |
| `immich-mount app` | Opens the app in the interface native to the desktop: Kirigami on Plasma, GTK elsewhere (on Plasma GTK too, if the Kirigami one does not load) |
| `immich-mount daemon` | Runs the daemon in the foreground (what the user service starts). Once the program it was started from is replaced by an upgrade and nothing is open or uploading, it unmounts and exits with status 75, on which the service starts the new version; once the program has been gone for two minutes (uninstalled), it unmounts, turns off the service that started it, takes the mount out of Dolphin's Places and Files' sidebar, removes the GVfs drop-in and its seeded thumbnails, and exits 0 |
| `immich-mount login [--password \| --api-key] [--server URL]` | Signs in; prompts only on a terminal. A running mount restarts with the new sign-in |
| `immich-mount logout` | Unmounts, then removes the seeded thumbnails, the credential and the caches |
| `immich-mount mount` / `unmount` | Mounts (starts the user service, writing it first where no install brought one and enabling it when `mount.at_login` is on, and waits up to 15 s for the mount) or unmounts (which stops the daemon, as an eject does). When something keeps the mount busy, such as a file manager window showing it, `unmount` detaches it lazily (`fusermount3 -uz`), unless the daemon has a copy into Upload in flight: then it refuses, naming the file ("… is still uploading") |
| `immich-mount status` | Connection state, server and its version, account, credential, mount point, cache usage |
| `immich-mount doctor` | A read-only health check (see WORKFLOWS) |
| `immich-mount configure list \| keys \| get \| set \| unset` | Reads and writes settings |
| `immich-mount show <link>` | Shows in the file manager, selected, the item an `immich-crosslink:` link or an address of Immich's web app on the signed-in server names (Links to Immich), and prints its path. With `--json` it prints `{"link": …, "result": …, "path": …, "message": …}` and selects nothing. A link it cannot show exits non-zero with the reason. With `crosslink.enabled` off it shows nothing, web addresses included (`off`) |
| `immich-mount web [--copy] <path>` | Opens the item at `path` in Immich's web app, in the default browser (`xdg-open`), or with `--copy` puts its web address on the clipboard (Links to Immich). With `--json` it prints `{"path": …, "url": …}` and opens nothing. A path outside the mount, an item Immich has no page for, and a mount that is not running each exit non-zero, saying which. `--copy` hands the address to Plasma's clipboard (Klipper) where it runs, else to `wl-copy` on Wayland or `xclip` or `xsel` on X11, which keep it after the command ends. With `web.links` off it says so and exits non-zero |
| `immich-mount completions <shell>` | Shell completions |

Global flags:
- `--json` makes every command write one JSON document to stdout.
- `--llm` (root only) prints the command reference plus this repository's docs.
- `--man` prints the man page.

Streams and exit codes:
- Results go to stdout. Progress, warnings and errors go to stderr.
- Exit code 0 means the command completed. A non-zero exit means it could not.
- Problems with individual items do not change the exit code; they show in the output and the JSON.
- Without a terminal, no command prompts. It fails with guidance instead.
- `IMMICH_MOUNT_LOG` sets how much is logged: `error`, `warn`, `info` (the default), `debug` or `trace`. At `info`
  the daemon logs a folder that took longer than a second to open, with what took the time; at `debug`, every
  folder.

## The daemon's D-Bus interface

The app ID is `dev.hviid.ImmichMount`. The daemon owns the bus name `dev.hviid.ImmichMount.Daemon`, object
`/dev/hviid/ImmichMount/Daemon`, interface `dev.hviid.ImmichMount.Daemon1`.

| Member | Kind | Meaning |
|---|---|---|
| `State` | property `s` | `online`, `offline` or `signed-out`, from the change poll every 10 seconds. |
| `MountPoint`, `ServerUrl`, `ServerVersion` | properties `s` | What it is connected to |
| `Account` | property `s` | The signed-in account's email, read at start; empty when the credential may not read it (an API key without `user.read`) |
| `Features` | property `as` | What the server and credential allow *(not built)* |
| `CacheUsage` | property `(tt)` | Bytes used, bytes allowed |
| `Fetched` | property `(tt)` | Bytes of originals fetched from the server since the mount started, and the requests that fetched them |
| `Uploads` | property `a(sst)` | Uploads in flight: name, phase, bytes sent |
| `ProbeServer(url s) → (version s, oauth b, password_login b, button_text s)` | method | Used by sign-in before a credential exists *(not built: the app asks the server itself)* |
| `Reload()` | method | Re-read the config and the credential *(not built: the app restarts the user service)* |
| `Mount()`, `Unmount()` | methods | `Unmount()` unmounts, which stops the daemon (an eject). *(`Mount()` is not built: the daemon only lives while mounted, so the app and `immich-mount mount` start the user service.)* |
| `Refresh(path s)` | method | Read folders from Immich again on their next visit (the whole tree, whatever the path) |
| `ClearCache()` | method | Empty the content cache, except Assets being read |
| `Links(path s) → (web s, crosslink s, kind s, hidden s)` | method | The links of the item at `path` for the menus: its web address, its crosslink (empty while `crosslink.enabled` is off), its kind (`asset`, `album`, `person`, `place`, `page`) and the apps left out (`crosslink.hidden`); all empty where `WebLink` is |
| `ShowLink(link s, select b, notify b) → (result s, detail s)` | method | The item a crosslink or a web address names (Links to Immich): `show` with its path, or `invalid`, `cannot-show`, `other-server` with its host, `other-user`, `not-found`, `off` or `unreachable`. With `select` the file manager shows it selected (`org.freedesktop.FileManager1.ShowItems`); with `notify` a notification says why not |
| `WebLink(path s) → (url s)` | method | The web address in Immich of the item at `path` (Links to Immich); empty for a path outside the mount or an item Immich has no page for. A path through a symlink to the mount's folder (`/home` on Fedora's atomic desktops) counts |
| `StateChanged(state s, message s)` | signal | Sent when the connection state changes; a desktop notification says the same once |

The credential itself never crosses D-Bus.

**The tray icon** (ARCHITECTURE.md, "The tray"): on the same connection, the daemon serves an
`org.kde.StatusNotifierItem` at `/StatusNotifierItem`, named `dev.hviid.ImmichMount-symbolic`, and its
`com.canonical.dbusmenu` menu at `/MenuBar`, and registers the item with `org.kde.StatusNotifierWatcher` whenever one
appears. The menu, top to bottom:

| Line | Kind |
|---|---|
| "Connected to *host*", "*host* is not reachable", "Signed out of Immich" | grey: the state |
| "Server version: Immich *version*" | grey, when the server said |
| "Signed in as: *email*" | grey, when the credential may read it |
| "Open in Files" ("Open in Dolphin" on KDE Plasma) | the mount point, in the file manager (`xdg-open`) |
| "Open Immich Mount" | the app (`immich-mount app`); activating the icon does the same |

A click on the icon shows the menu (`ItemIsMenu`), as the Mac's menu bar item does.

## Files on disk

| What | Where |
|---|---|
| Settings | `~/.config/immich-mount/config.toml` |
| Credential (headless fallback) | `~/.config/immich-mount/credential`, mode 0600 |
| Metadata store | `~/.cache/immich-mount/<account>/metadata.redb` |
| Content cache | `~/.cache/immich-mount/<account>/content/`: `<id>.bin` (sparse) and its block map `<id>.blocks`, and `<id>.edges`, the original's first 64 KiB and last 16 KiB |
| Upload staging | `~/.cache/immich-mount/<account>/staging/` |
| The folders copied into Upload, followed for their albums | `~/.cache/immich-mount/<account>/folder-copies.json` |
| Files not uploaded | `~/.local/share/immich-mount/not-uploaded/`; `logout` keeps them |
| Server thumbnails | `~/.cache/immich-mount/<account>/server-thumbnails/` (Immich's own, by Asset id) |
| Seeded thumbnails | `$HOME/.cache/thumbnails/{x-large,xx-large}/`, and on KDE `{normal,large}/` too, each URI recorded in `~/.cache/immich-mount/<account>/seeded-thumbnails`; where home is reached through a symlink, under both the resolved and the configured path |
| A browser sign-in waiting for its redirect (KDE app) | `$XDG_RUNTIME_DIR/immich-mount-sign-in`, mode 0600: the server, the state and the PKCE verifier, replaced by the outcome once the redirect arrives, and removed when the app reads it |
| Dolphin's previews of remote files (KDE app, only when the user turns them on) | `[PreviewSettings] MaximumRemoteSize=104857600` in `~/.config/kdeglobals`; turning them off removes the key |
| The mount in Dolphin's Places (KDE) | A bookmark for the mount point, titled with the drive's name (`mount.name`), in `~/.local/share/user-places.xbel`: added once by the KDE app's setup, renamed by the daemon as it starts when the name changed (a place the user renamed otherwise keeps its name), removed by `logout` |
| GVfs's volume monitor's settings, where it is a user service | `~/.config/systemd/user/gvfs-udisks2-volume-monitor.service.d/immich-mount.conf`: `Environment=LIBMOUNT_FSTAB=<the volume monitor's fstab>`, and, where home is reached through a symlink, `Environment=HOME=<resolved home>`; written by the daemon as it starts, which then restarts the volume monitor, so Files lists the mount as a drive, with our icon; removed by `logout` |
| The volume monitor's fstab | `~/.local/state/immich-mount/gvfs-fstab`: `/etc/fstab` as it was when the daemon started, and, while the daemon runs, `immich <resolved mount point> fuse.immich noauto,x-gvfs-show,x-gvfs-name=<drive's name>,x-gvfs-icon=dev.hviid.ImmichMount,x-gvfs-symbolic-icon=dev.hviid.ImmichMount-symbolic 0 0`; written with that line as the daemon starts, before it mounts, and without it once it stops; removed by `logout` |
| The mount point's folder icon in Files | `metadata::custom-icon-name=dev.hviid.ImmichMount` on the mount point, in gvfsd-metadata's store (`~/.local/share/gvfs-metadata/`), set by the daemon as it starts and once mounted |
| The mount in Files' sidebar, where GVfs still does not list it (GNOME app) | A line `file://<mount point> <drive's name>` in `~/.config/gtk-3.0/bookmarks`: added once by the GNOME app's setup, renamed by the daemon as it starts when `mount.name` changed (a bookmark the user titled otherwise keeps its title), removed by the daemon once the drive is listed, and by `logout` (with a bookmark of the resolved path) |
| The sign-in redirect's handler | `x-scheme-handler/app.immich` in `~/.config/mimeapps.list`, set by the app that last started a browser sign-in |
| Open in Immich in Files, Homebrew's cask | `~/.local/share/nautilus-python/extensions/immich-mount-nautilus.py`, removed with the cask |
| Open in Immich in Dolphin, Homebrew's cask | The plugin linked as `~/.local/lib/immich-mount/plugins/kf6/kfileitemaction/immich-mount-dolphin.so`, found through `QT_PLUGIN_PATH`, which `~/.config/plasma-workspace/env/immich-mount.sh` adds to at login; both removed with the cask |
| The version last told of (`updates.check`) | `~/.local/state/immich-mount/update-told` |
| Logs | journald (`journalctl --user -u immich-mount`) |

`<account>` is the server's host and port (`photos.example.org-443`), readable in a file listing. Signing in again,
with any credential, keeps the caches. Two accounts on one server share the directory only if the user signs in as
another account without signing out first. Cached bytes stay right, because Asset ids are global, but the kept
listings show the first account's folders until each is refreshed (offline, until the server answers again).
`immich-mount logout` removes every cache, which is the way to switch accounts.
Directories are 0700 and files 0600.

## In Finder (the macOS edition)

The same layout, names, attributes and content as the mount, through Apple's File Provider (ARCHITECTURE.md, "The
macOS edition"). Where Finder's way differs:

- **Uploads:** the system hands over a finished file, so there is no staging. A file Immich refuses stays in Upload
  with its reason, and one that has not reached Immich yet stays there while the system tries again; there is no
  `not-uploaded/` folder. An uploaded file leaves Upload after about 10 seconds, as on Linux. Folders copied into
  Upload are taken as on Linux ("Folders copied into Upload").
- **Signing out** (the menu bar item's menu) ends the session on the server and removes the Finder location with
  what was downloaded. A file in Upload that has not reached Immich is kept: the confirmation counts them, the
  system preserves them, and Finder opens the folder they are kept in. When the location cannot be removed, the app
  stays signed in and says why.
- **Open in Immich and Copy Immich Link** (Links to Immich) are the File Provider extension's own actions
  (`dev.hviid.ImmichMount.open-in-immich` and `dev.hviid.ImmichMount.copy-link`), which Finder offers on one item
  with a web address (the item's `hasWebLink`). The extension opens the address with `NSWorkspace` or puts it on the
  pasteboard itself, so the menu bar app need not run. Their names are translated through the extension's
  `Localizable.strings`. There is no `web` command on the Mac.
- **Show in other apps** (Show in other apps): Finder's actions are fixed in the extension's Info.plist, so their
  names cannot be the apps', and the apps come in one action, **Show in Another App…** (the protocol's second way to
  offer Show in). Finder offers it on one item while at least one other app announces draft 0.2 and the item's
  kind. It has the menu bar app, started if it is not running and without a window, send the item's crosslink to
  that app straight away when there is one; with several, an alert, **Show in Which App?**, says "Choose an app to show
  “*item*” in." (the item's name as Finder shows it, when the system tells it at once) and has a button for each app,
  by the name and icon macOS gives it (a bundle's display name), in the settings' order, and **Cancel**. Return presses
  the app chosen last time (the first, before any), Escape cancels, and nothing is sent on Cancel. With more than six
  apps, the alert lists them, the first chosen, with **Show** and **Cancel**. The app finds the apps
  (`NSWorkspace.urlsForApplications(toOpen:)`) at launch, when any app launches, when a crosslink setting changes and
  before it sends, leaving out an app whose program is missing, and marks the items Finder offers the action on; a
  send that fails gets an alert titled "Immich Crosslink", as on Linux, and the list is read again. The settings list
  them, each with a switch (`crosslink.hidden`, by bundle id), and Forget on one not found. A sandboxed app reads another app's
  announcement only from a copy in /Applications.
- **Dates:** a file has its capture time, as on Linux; a folder the time the location was set up (signing in),
  where on Linux it has the mount's.
- **Settings:** the settings table less the Linux-only ones (`mount.*`, `cache.limit`, `thumbnails.*`), in the
  app's Settings window. A change has the system list every folder again.
- **Updates:** the app checks `https://jakob.hviid.phd/immich-mount/appcast.xml` once a day, unless switched off in
  Settings, and installs only a disk image signed with the project's key.
- **Files on disk:**

  | What | Where |
  |---|---|
  | Settings, the server's address, the account's email, the metadata store (`Library/`), the extension's status (`status.json`) | The App Group container, `~/Library/Group Containers/<team>.dev.hviid.ImmichMount/` |
  | The questions about copied folders, their answers and what came of the albums, passing between the extension and the app | `Library/folder-questions.json`, `folder-answer-<copy>.json` and `folder-notice-<n>.json` in the App Group container |
  | Credential | The Keychain, service `dev.hviid.ImmichMount`, in the app's access group |
  | The system's copy of the location: downloaded originals and what waits in Upload | `~/Library/CloudStorage/ImmichMount-<name>/`, kept by macOS |

  Signing out removes all of them but what waits in Upload. `brew uninstall` signs out first, by running the app as
  `Immich Mount --uninstall`, so the location leaves Finder with the app. Dragging the app to the Bin runs nothing:
  sign out first, or the location stays in Finder without its app.

## Errors seen by programs using the mount

| Condition | Error code | Daemon behaviour |
|---|---|---|
| Server unreachable | Cached answer if any (a folder visited before lists as it was, after a restart too, and also when the server stops answering mid-request), otherwise `EIO` at once | State `offline` and one notification; requests fail fast until the next change poll succeeds |
| Signed out (401) | `EACCES` | State `signed-out`; one notification |
| Missing permission (403) | `EACCES` | The app names the missing permission |
| Asset gone (404, or 400 `Not found`) | A read fails with `EIO` (the kernel reports any failed page read so, though the daemon answers `ENOENT`); looking it up again fails with `ENOENT` | The folder it was read from is listed again on its next visit, without it |
| Server busy (408, 429, 5xx) | Retried within the deadline, then the cached listing if any, else `EIO` | Backs off, honouring `Retry-After` |
| Size mismatch, or a range past the end | `EIO` for that read | The folder it was read from is listed again on its next visit, with the new size |

## The non-affiliation notice

Shown in the README, the app's About dialog, the AppStream metadata and every package description:

> Immich Mount is an independent, unofficial project. It is not affiliated with, endorsed by, or sponsored by the
> Immich project or FUTO. "Immich" is used only to describe the server this software works with, and remains the
> mark of its owner.

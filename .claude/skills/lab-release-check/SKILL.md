---
name: lab-release-check
description: Use at every release, and before calling a fix verified on a desktop, to check Immich Mount on a desktop VM in about a minute with few tokens - installed as its users install it (Homebrew's cask on Bazzite, the release's .rpm packages on Fedora), signed in, mounted, with one PASS or FAIL line per documented claim (launcher, service, sidebar, access, copies out, thumbnails, the cost of a peek, doctor, unmount and mount, an upload), and on Bazzite the cask's uninstall. Runs lab/check/ on the Incus lab host.
---

# The release check

Read AGENTS.md, "Testing on Erebus", first: its rules apply here too. This check drives the VMs of the
`lab-desktop-vm` and `lab-bazzite-vm` skills and the server of the `lab-immich-server` skill; their traps apply.

**When:** at every release, on each kind its users have, and before calling a fix verified on a desktop. It replaces
driving the GUI by screenshots for everything a command can state; what it cannot state is below.

```sh
lab/check/run.sh bazzite-gnome --release --uninstall   # Homebrew's cask; then its uninstall
lab/check/run.sh bazzite-kde --release
lab/check/run.sh gnome --release                       # Fedora 44 with the release's .rpm packages
lab/check/run.sh plasma --release
lab/check/run.sh gnome --binary target/release/immich-mount   # the release, with a local build of the daemon and CLI
```

- **What `run.sh` does:** brings up the server and the kind's VM (warm: each `up.sh` restores the `fresh` snapshot, so
  every run starts from a stock system and the seeded library), starts the session, installs the release as
  WORKFLOWS.md says for that system (`brew install jakobhviid/tap/immich-mount`, or `dnf install` of the daemon's
  package and the interface for the desktop), signs in from the command line, mounts, runs `desktop.sh` in the
  session, and prints its lines and the wall time. It leaves the instances up; each kind's `down.sh` deletes them.
- **`--release`** installs the latest release (`gh release view`), or `VERSION=1.4.0`. **`--binary <path>`** installs
  the release, then puts the local `immich-mount` in place of the installed one, so the launcher, icons and unit stay
  the release's; the version expected is the build's own.
- **`--uninstall`** (Bazzite): after the check, `brew uninstall --cask --zap jakobhviid/tap/immich-mount`, then
  `uninstall.sh`, which waits for the daemon's own cleanup (it leaves about 2.5 minutes after its program is gone) and
  reports what is left. Alone, without `--release`, it runs on the VM as the last run left it.
- **Alongside another agent:** give your own instances with `GNOME_VM`, `PLASMA_VM`, `BAZZITE_VM` (an
  `ind-bazzite-<edition>-*` name) and `IMMICH_INSTANCE`.
- **The exit code** is non-zero when a check failed. A failing check is a product bug until the check is shown wrong:
  find out which before changing either.

**Signing in without the GUI:** `immich-mount login --password` with the server's throwaway admin, after
`configure set credential.file`, so the credential goes to a 0600 file and no keyring prompt waits for an answer.
That is acceptable for a scripted check: it is the documented headless path, and the keyring is the GUI check's. On
KDE, `run.sh` then runs the line `immich-mount doctor` prints to turn Dolphin's previews on (what the KDE app's
Show thumbnails switch does), which also checks that doctor's advice works.

## What each check proves

`desktop.sh <version> [--app-setup]` runs inside the session as `tester`. Each line is one claim of the docs:

| Check | Proves | Claim |
|---|---|---|
| `version` | `immich-mount --version` on the PATH is the release (Homebrew's bin last on the PATH, as Bazzite's terminal has it) | |
| `launcher` | `dev.hviid.ImmichMount.desktop` is found in the XDG data dirs, and its `Exec` program runs that version | ARCHITECTURE "Packaging" |
| `scheme` | the launcher is registered for `x-scheme-handler/app.immich` | WORKFLOWS "Install" |
| `icons` | the icon and its symbolic variant, as SVG | ARCHITECTURE "Packaging" |
| `translations` | the Danish catalog where the programs look (`share/locale` beside them, or `../share/locale`), and `Comment[da]` in the launcher; it counts the languages | ARCHITECTURE "Translations" |
| `status` | `status --json`: online, mounted, not stale, an account | WORKFLOWS "Status" |
| `service` | `immich-mount.service` enabled and active | WORKFLOWS "The mount" |
| `mounted` | a FUSE mount at `~/Immich` (resolved), named Immich Mount (the default `mount.name`) by `.xdg-volume-info` | SPEC "Files the mount answers" |
| `sidebar` (GNOME) | `gio mount -li` lists it, unmountable, so Files shows it as a drive with an eject button; on Bazzite through the GVfs drop-in | ARCHITECTURE "GNOME integration" |
| `places` (KDE) | Dolphin's Places hold it; SKIP unless `--app-setup`, since only the KDE app's first run adds it | SPEC "Files on disk" |
| `access` | `access::can-write`: not the root, Albums or a photo; Photos, a year and a month | SPEC "Attributes" |
| `thumbnails` | a listed month's files have `x-large` (and `normal`, `large` on KDE) at once and `xx-large` within 40 s, keyed by the MD5 of `gio info`'s URI, mode 0600, `Thumb::URI` right; under `/home` and `/var/home` where home is a link | SPEC "Thumbnails" |
| `peek` | each file's first 8 KiB and last 4 KiB, page cache dropped first (`posix_fadvise`), cost under 100 KB per file in `status --json`'s `fetched.bytes`; nothing fetched fails as unmeasured | ARCHITECTURE "Peeks are served from the edges" |
| `copies` | a photo by `gio copy` and `cp`, and the smallest album by `cp -r`, come out 0644 and 0755, byte for byte the server's (SHA-1 asked of the server) | SPEC "What the mount refuses" |
| `upload` | a photo the server lacks (a copied one with a new JPEG comment), `cp` into Upload, returns once the server has its SHA-1; the receipt stays | SPEC "Uploads" |
| `upload-listed` | the asset shows in its date folder (its source's month) | WORKFLOWS "Uploading" |
| `upload-gone` | the file leaves Upload by itself within 30 s of the copy | SPEC "Uploads" |
| `read-only` | a paste beside a photo in its month is refused as `Read-only file system`, and leaves nothing | SPEC "What the mount refuses" |
| `doctor` | `doctor --json`: no problem, no warning | WORKFLOWS "Doctor" |
| `app` | the launcher's command, started in the user manager as a desktop starts it, opens `immich-mount-kde` on Plasma and `immich-mount-gnome` elsewhere, still running after 5 s | SPEC "The immich-mount command" |
| `unmount` | `immich-mount unmount` with a process inside Photos: exit 0, detached, the service stopped | SPEC "The immich-mount command" |
| `remount` | `immich-mount mount` mounts again, online | WORKFLOWS "The mount" |

`uninstall.sh [seconds]` reports, against WORKFLOWS "Disconnect and uninstall" and the cask's zap: no program, no
launcher, icons or scheme handler, the daemon gone (with how long it took), nothing mounted, the unit inactive with no
unit file or enable link, no GVfs drop-in, no bookmark or place, no thumbnail of a file on the mount, the cache and the
settings gone or in the Trash, and the credential kept (in the keyring, or with `credential.file` in the trashed
settings).

## Runtime

Measured with 1.4.1 (2026-11), warm: 41 s on `gnome`, 53 s on `bazzite-kde`, 56 s on `bazzite-gnome` (restoring the
server and the VM 17 to 25 s, installing 4 to 10 s, the checks about 20 s); the uninstall phase adds about 150 s,
nearly all of it the daemon's wait. The first run of a kind also builds its VM: 80 to 100 s for Bazzite, 115 s for
GNOME, 155 s for Plasma; the server takes about 3 minutes to build and seed.

## What it does not cover

One screenshot of the app's status page and one of the file manager showing the mount, per desktop, stay the rule
at a release (the `lab-desktop-vm` and `lab-bazzite-vm` skills), for what no command states:
- the GUI first run and sign-in: the app's pages, the keyring prompt, KWallet's wizard, and on KDE the place in
  Dolphin's Places (run `desktop.sh <version> --app-setup` after a first run to check that place)
- single sign-on through a browser and the `app.immich` hand-off (`lab/immich-server/oidc-sign-in.py` covers the
  CLI's side)
- what is drawn: the thumbnails Files and Dolphin show, the sidebar's eject button, notifications
- upgrades restarting into the new version, offline behaviour, and moves into Upload from the file managers

## Traps

- **`run-in-session.sh` passes no desktop variables.** A terminal in the session has `XDG_CURRENT_DESKTOP` and the
  rest, which the session gave the user manager; both scripts take them from `systemctl --user show-environment`.
  Without them `doctor` does not know it is on KDE and skips its Dolphin check.
- **Bazzite puts Homebrew's bin last on the PATH,** for interactive shells only (`/etc/profile.d/brew.sh`): the
  scripts append it, and `brew` itself runs through `bash -ic`.
- **An upload carries the upload's time until Immich has read its metadata,** so the server's first answer, and the
  mount for a moment, put it in the current month. `upload-listed` waits for its EXIF month.
- **Run `run.sh`, not `desktop.sh` alone, for a verdict:** a second `desktop.sh` on the same VM copies the last run's
  upload, and finds the thumbnails already there. Its peek fails as unmeasured: the edges the first run fetched
  are kept, so no read reaches the server.
- **The cask can lag a release by a few minutes:** `version` then fails with the cask's version. Wait, or check the
  previous release with `VERSION`.
- **The zap trashes the unit and the drop-in while the daemon still runs;** the daemon's cleanup then finds them
  gone. What `uninstall.sh` sees is the result of both.

## Recording the result

As in the `lab-desktop-vm` skill: a check's outcome goes where its claim lives (a release's in ROADMAP.md when it
closes a milestone), new lab traps go here, and the printed table stays out of git.

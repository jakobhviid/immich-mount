# WORKFLOWS

What you run. [SPEC.md](SPEC.md) is the contract behind each step, and [ARCHITECTURE.md](ARCHITECTURE.md) explains why
it works this way. Where a step is not built yet, it says so.

## 1. Install

Every [release](https://github.com/jakobhviid/immich-mount/releases/latest) carries the packages. Download the ones
for your system into a folder, and install them from there. The daemon and CLI package is also built for ARM
(`aarch64` in the .rpm's name, `arm64` in the .deb's); the apps are x86_64.

**Fedora 44:** the daemon and the app for your desktop, of the same version. The GNOME app is
`immich-mount-gnome-*.rpm`, the KDE Plasma app `immich-mount-kde-*.rpm` (it pulls in Kirigami and the KDE style):

```sh
sudo dnf install ./immich-mount-[0-9]*.x86_64.rpm ./immich-mount-gnome-*.rpm    # or ./immich-mount-kde-*.rpm
```

**Ubuntu 26.04 and Kubuntu 26.04:** the same pair as .debs:

```sh
sudo apt install ./immich-mount_*_amd64.deb ./immich-mount-gnome_*_amd64.deb    # or ./immich-mount-kde_*_amd64.deb
```

Install the app for your desktop. Both register the `app.immich` scheme, so single sign-on in the browser hands back
to the app. With both installed, Plasma lists only the KDE app and other desktops only the GNOME app.

**Fedora 43, Ubuntu 24.04, Debian 13, or a server:** the daemon and CLI alone, `immich-mount-*.rpm` or
`immich-mount_*.deb`, then sign in from a terminal (section 2). Both packages install `/usr/bin/immich-mount`, the
user unit and the icon, and pull in `fuse3`:

```sh
sudo dnf install ./immich-mount-[0-9]*.rpm     # or: sudo apt install ./immich-mount_*.deb
immich-mount login --server https://photos.example.org
systemctl --user enable --now immich-mount
```

**Any other Linux:** the static build, into your home directory (use `aarch64` in the name on ARM):

```sh
curl -fLO https://github.com/jakobhviid/immich-mount/releases/latest/download/immich-mount-x86_64-unknown-linux-musl.tar.gz
mkdir -p /tmp/immich-mount && tar xzf immich-mount-x86_64-unknown-linux-musl.tar.gz -C /tmp/immich-mount
install -Dm755 /tmp/immich-mount/immich-mount ~/.local/bin/immich-mount
install -Dm644 /tmp/immich-mount/immich-mount.service ~/.config/systemd/user/immich-mount.service
install -Dm644 /tmp/immich-mount/dev.hviid.ImmichMount.svg ~/.local/share/icons/hicolor/scalable/apps/dev.hviid.ImmichMount.svg
install -Dm644 /tmp/immich-mount/dev.hviid.ImmichMount-symbolic.svg \
  ~/.local/share/icons/hicolor/symbolic/apps/dev.hviid.ImmichMount-symbolic.svg      # the icon, for notifications
immich-mount login --server https://photos.example.org --password     # or --api-key
systemctl --user daemon-reload && systemctl --user enable --now immich-mount
immich-mount doctor
```

**The desktop app elsewhere, from source.** The GNOME app needs GTK 4.18, libadwaita 1.8 and `blueprint-compiler`;
the KDE app needs Qt 6 development files (`qt6-qtbase-devel`, `qt6-qtdeclarative-devel`) to build, and Kirigami
and `qqc2-desktop-style` to run. Build where those are installed, such as a toolbox, and run on the same release:

```sh
cargo build --release -p immich-mount-gnome
install -Dm755 target/release/immich-mount-gnome ~/.local/bin/immich-mount-gnome
sed "s|@BINDIR@|$HOME/.local/bin|" crates/immich-mount-gnome/data/dev.hviid.ImmichMount.desktop.in \
  > ~/.local/share/applications/dev.hviid.ImmichMount.desktop
gio mime x-scheme-handler/app.immich dev.hviid.ImmichMount.desktop   # the browser hands sign-in back to the app

cargo build --release -p immich-mount-kde                            # the same for KDE
install -Dm755 target/release/immich-mount-kde ~/.local/bin/immich-mount-kde
sed "s|@BINDIR@|$HOME/.local/bin|" crates/immich-mount-kde/data/dev.hviid.ImmichMount.Kde.desktop.in \
  > ~/.local/share/applications/dev.hviid.ImmichMount.Kde.desktop
xdg-mime default dev.hviid.ImmichMount.Kde.desktop x-scheme-handler/app.immich
```

**Homebrew** (any Linux, x86_64 or ARM): the daemon and CLI. FUSE comes from your distribution (`sudo dnf install
fuse3` or `sudo apt install fuse3`), and `brew services` runs the daemon in place of the packages' user unit:

```sh
brew install jakobhviid/tap/immich-mount
immich-mount login --server https://photos.example.org
brew services start immich-mount
```

**Coming:** COPR for Fedora, and the AUR.

Requirements: `fusermount3` (package `fuse3`) and a keyring (GNOME Keyring or KWallet) for normal use. The app needs
the current or previous GNOME release.

## 2. Sign in

**On the desktop:** open **Immich Mount** from the app grid.
1. Enter the server URL.
2. Choose a sign-in method:
   - **Sign in with <your server's OIDC button>** opens your browser. Sign in there as usual, with your passkey or
     password manager, and the browser hands you back to the app.
   - **Email and password**, if the server allows it.
   - **Advanced → Use an API key instead.**
3. Choose the sections and the mount location, then **Open in Files** (on KDE, **Open in Dolphin**; there,
   **Show thumbnails** lets Dolphin preview the mount, which KDE counts as a remote location).

You sign in once; the credential is kept in your keyring across reboots.

**Headless, or from a terminal:**

```sh
immich-mount login --server https://photos.example.org                # OIDC: prints a URL, then asks for the final URL
immich-mount login --server https://photos.example.org --password     # prompts for email and password
printf '%s' "$KEY" | immich-mount login --server https://photos.example.org --api-key   # key from stdin
```

Without a terminal, `login` never prompts. Browser sign-in still prints the address to open on stderr, and reads
the address the browser ends on from stdin; the other methods read their secrets from stdin. An API key needs
`album.read`, `asset.read`, `asset.view`, `asset.download` and `person.read`, plus `asset.upload` for uploads, and
`user.read` for the app and `status` to name the account.

## 3. The mount

- It appears at `~/Immich` and in the Files sidebar as **Immich**, with an eject button. On KDE it is in Dolphin's
  Places as **Immich**, added by the app's first run (remove it like any place; it is not added back).
- It starts at login, through the user service: `systemctl --user enable --now immich-mount`.
- **Eject** in Files unmounts it until the next login, or until `immich-mount mount` or **Mount again** in the app.
- `immich-mount unmount` and `immich-mount mount` do the same from a terminal.

Browse it like any folder: Photos by year and month (busy months by day), Albums (`2023 - Summer trip`),
People and Places. Files are named by when they were taken (`2024-03-15 14.32.07.jpg`). Thumbnails come from the
server, and an original is downloaded only when something opens it.

## 4. Status

```sh
immich-mount status
immich-mount status --json
```

```json
{
  "state": "online",
  "detail": null,
  "server": { "url": "https://photos.example.org", "version": "3.3.5" },
  "account": { "email": "me@example.org", "credential": "session" },
  "change_feed": "sync-stream",
  "mount": { "path": "/home/me/Immich", "mounted": true, "stale": false },
  "cache": { "used_bytes": 734003200, "limit_bytes": 10737418240 }
}
```

`state` is one of `unconfigured`, `online`, `offline`, `signed-out`, `missing-permission` or `unmounted`; `detail`
says why when it is not `online`.
`change_feed` is `sync-stream` (signed in with a session) or `polling` (an API key); either way, changes show
within about 10 seconds. `account.email` appears only when the credential may read it.

## 5. Doctor

`immich-mount doctor` is a read-only health check. It changes nothing. Each check reports `ok`, `warning` or
`problem`, with what to do:

| Check | Looks for |
|---|---|
| FUSE | `/dev/fuse` is accessible, and `fusermount3` exists and is setuid |
| Kernel timeout | `fs.fuse.default_request_timeout` is off (otherwise long uploads could abort the mount) |
| Mount | Mounted where configured; no stale mount ("Transport endpoint is not connected") |
| Server | Answers, and which Immich version it runs |
| Credential | Found (in the keyring or the credential file) and accepted by the server, with the permissions the mount needs. A locked or missing keyring shows here |
| Thumbnails | `$HOME/.cache/thumbnails` is writable |
| Location | The mount point is under `$HOME` and not hidden (otherwise it will not show in the sidebar) |
| Dolphin previews | On KDE: whether Dolphin may preview files on the mount (KIO counts it as remote, where previews are off until `MaximumRemoteSize` allows them), with the `kwriteconfig6` line that turns them on (the KDE app's **Show thumbnails** switch does the same) |

The loop for scripts: act, run `immich-mount doctor --json`, decide from the counts, act again.

## 6. Settings

```sh
immich-mount configure list                       # every setting and its effective value
immich-mount configure set albums.layout year-folders     # Albums/2023/Summer trip/
immich-mount configure set files.naming original          # IMG_1234.JPG instead of the date and time
immich-mount configure set cache.limit 20GiB
immich-mount configure unset thumbnails.sharp      # back to the default
```

The apps' preferences (GNOME's dialog, KDE's page) edit the same settings. The keys are listed in SPEC.md.

## 7. Uploading

- Copy or move photos and videos into **Photos** in Files. Each file uploads unchanged, and then appears in its
  date folder. The name you pasted stays for about two minutes as a receipt.
- **A move never loses a photo.** In Files, the source is deleted only after the server has the file: if an upload
  fails, Files reports an error and the source stays where it was. Dolphin deletes the source even then, so any file
  that could not be uploaded is kept in `~/.local/share/immich-mount/not-uploaded/`, and the notification says so.
- **Already in library:** a file whose content already exists counts as done. A file matching something in the Immich
  trash is refused; restore it in Immich if you want it.
- Folders and non-media files are refused, with a notification explaining why.
- **Cancelling a copy** keeps the part already copied out of Immich: each file's own structure shows whether it
  arrived whole. MTS and M2TS videos cannot tell, so a cut-short one does upload; delete it in Immich. Empty files
  and macOS's `._` companion files are never uploaded.

## 8. Troubleshooting

| Symptom | What to do |
|---|---|
| The keyring prompt appeared at login | Unlock it. If it was dismissed, the mount did not start (a notification says so): run `immich-mount mount`, or open Immich Mount |
| "Signed out of Immich" notification | Open the app and sign in again. Caches are kept |
| A change made in Immich does not show in an open folder | The mount has it within about 10 seconds, but Files and Dolphin do not redraw an open folder on their own: press F5, or open the folder again |
| KWallet asks to create a wallet at sign-in | First use of KWallet on this account (no `pam_kwallet`): create it; the credential is kept there |
| Dolphin shows generic icons on the whole mount | KDE keeps previews of remote locations off: turn on **Show thumbnails** in the KDE app, or run the line `immich-mount doctor` prints |
| Files shows generic icons in a folder opened while the server was unreachable | Once it is reachable again, close Files and open it again: Files remembers the failed thumbnails until then, F5 included |
| Files shows generic icons for some photos | They were search results from folders never opened. Open the folder once |
| "Transport endpoint is not connected" | The daemon died. `immich-mount doctor` confirms it; `systemctl --user restart immich-mount` cleans up and remounts |
| A large upload fails with "This computer lets a file operation take at most N seconds" | The system sets `fs.fuse.default_request_timeout`; the mount stops each upload just before that limit so as not to be aborted. Set it to 0 (`sudo sysctl fs.fuse.default_request_timeout=0`) for longer uploads |
| A backup tool wants to copy `~/Immich` | Exclude it: Déjà Dup "Folders to ignore"; restic and borg `--exclude ~/Immich`, or `--one-file-system` |

Logs: `journalctl --user -u immich-mount`.

## 9. Disconnect and uninstall

- `immich-mount logout`, or **Disconnect** in the app, removes:
  - the credential (from the keyring or KWallet, or the file)
  - the session on the server, so the device leaves Immich's "Authorized devices"
  - the metadata store and the content cache
  - every thumbnail the daemon seeded
  - the mount's place in Dolphin's Places, on KDE

  It also turns the user service off (Homebrew's too), which would otherwise fail at every login without a
  credential. After the
  next `immich-mount login`, `systemctl --user enable --now immich-mount` turns it on again (the apps do that at
  their first-run setup).
- Settings are kept, and so are files that could not be uploaded (`~/.local/share/immich-mount/not-uploaded/`).
  So is KDE's remote-preview limit if **Show thumbnails** turned it on (turn it off in the KDE app first, or with
  `kwriteconfig6 --file kdeglobals --group PreviewSettings --key MaximumRemoteSize --delete`).
- After that, remove the package as usual for your channel.

## Exit codes and JSON

- Every command accepts `--json` and writes exactly one JSON document to stdout. Progress and warnings go to
  stderr.
- Exit 0 means the command completed. Non-zero means it could not. Problems with individual items show in the
  output, not in the exit code.

---
name: lab-desktop-vm
description: Use to check Immich Mount on a real desktop - GNOME Files or KDE Dolphin, the GNOME or KDE app, thumbnails, sign-in in a browser, uploads by copy and move, keyrings - or to take README screenshots. Brings up a disposable Fedora 44 VM with a headless GNOME or Plasma session on the Incus lab host (lab/gnome-vm/, lab/plasma-vm/), and covers driving it, looking at it, tracing what reads the mount, and recording the result.
---

# Desktop VMs: GNOME and Plasma

Read AGENTS.md, "Testing on Erebus", first: its rules apply to everything here. Most checks also need the test
server (the `lab-immich-server` skill).

| | GNOME (`lab/gnome-vm/`) | Plasma (`lab/plasma-vm/`) |
|---|---|---|
| What you get | GNOME Shell 50 and Files, with the tracing tools | Plasma 6 (KWin, plasmashell, Dolphin), nothing of ours |
| Instance | `GNOME_VM` (default `ind-gnome-1`) | `PLASMA_VM` (default `ind-plasma-1`) |
| Session | `session.sh [WxH]`, 1280x800 | `session.sh [WxH]`, 1440x900 |
| Input | `exp/input.sh` (synthetic clicks and keys, by coordinates) | `act.py` (by the names on screen, through accessibility), `click.py` (a click by coordinates) |

## The loop

```sh
lab/gnome-vm/up.sh && lab/gnome-vm/session.sh              # warm: about 14 s, then 4 s; built: about 115 s
lab/gnome-vm/run-in-session.sh -d nautilus /home/tester/Immich   # -d: a GUI app, detached
lab/gnome-vm/shot.sh /tmp/files.png                         # then look at it with the Read tool
lab/gnome-vm/down.sh                                        # when the working session ends
```

**Warm VMs** (AGENTS.md, "Testing on Erebus"): `up.sh` restores a VM that has its `fresh` snapshot, taken once it
was built and stopped, so each check starts clean; without one, or with `--fresh`, it builds the VM (deleting one in
the way) and takes the snapshot. Each start renews `user.expires` and deletes the project's expired `ind-` instances
(`lab/warm.sh`). Run `up.sh` before every check, not only the first: it is what puts the VM back. Helpers (`exp/`,
`in-session`, `act.py`) are pushed on every start.

The Plasma scripts take the same shape. `run-in-session.sh` runs as `tester` with the session's bus and Wayland
display; single-quote `~` and `$HOME` so they expand inside the VM.

**Putting our build in.** Either install a release's packages (the honest test of what users get), or build in
the toolbox and push the binary:

```sh
"$EREBUS_DIR/bin/incus" file push --project lab target/release/immich-mount \
  ind-gnome-1/usr/local/bin/immich-mount --mode 0755 </dev/null
```

**An atomic desktop's home** (Bazzite, Silverblue: `HOME=/home/tester` while the folder is `/var/home/tester`) is
what a real Bazzite VM gives as it is (the `lab-bazzite-vm` skill). To imitate it in these Fedora VMs instead,
between `up.sh` and `session.sh`, as root in the VM:
`loginctl disable-linger tester; systemctl stop user@<uid>.service; mv /home /var/home; ln -s var/home /home;
loginctl enable-linger tester`. **Homebrew as a user installs it:** pre-create `/home/linuxbrew/.linuxbrew` owned
by `tester`, then run Homebrew's installer with `NONINTERACTIVE=1` as `tester` (it needs `git`, `procps-ng`,
`file`, `curl` and `gcc`). Source the lab helpers from bash: under zsh, `inc` looks for `bin/incus` in the wrong
place.

A stale binary earlier on the PATH (an old tarball install in `~/.local/bin`) silently wins over a package's; check
`command -v immich-mount` in the VM when a result looks old.

**Driving Plasma apps by name:** `in-session act.py tree immich-mount-kde` lists what is on screen;
`act.py press <app> <name>`, `act.py set <app> <name> <text>` and `act.py wait <app> <name>` do the rest, in our app
and in others (KWallet's wizard, Fedora's Welcome Center). Names beat coordinates: they survive layout changes.
`act.py` says "no application" until the window is on the accessibility bus; wait a few seconds after starting it.
Qt drops the name of password fields on the accessibility bus, so fill those by position:
`act.py set-nth immich-mount-kde 'password text' 1 <password>`. KWallet's first-use wizard is "Classic, blowfish
encrypted file", then Finish, then `set` on "Password:" and "Verify:" and OK.

**Signing in on GNOME without clicking:** `printf '%s\n%s\n' <email> <password> | immich-mount login --server <url>
--password` in the session, then `immich-mount mount`; the app then opens on its status
page. Storing the first credential brings up "Choose password for new keyring": answer it with `exp/input.sh type`
(twice, `key Tab` between, then `key Return`). `exp/input.sh` often drops the first event it sends, so check typed
text on a screenshot. The overview is hidden in the lab session; `org.gnome.Shell.Eval` with
`Main.overview.showApps()` shows the app grid.

## Looking at what reads the mount

This is the measurement that matters most: opening a folder must read no originals.
- **`fatrace` sees nothing** through a FUSE mount without `allow_other`. Use the bpftrace probe instead:
  `lab/gnome-vm/exp/fuse-opens.bt` prints every open and read on any FUSE mount, with the process and its parent.
- **To see FUSE request types** (CREATE, WRITE, FLUSH, SETATTR), put an `fentry` probe on the fuse module's
  `__fuse_simple_request`; a plain kprobe cannot resolve `struct fuse_args`.
- **The content cache** shows what the daemon fetched: an empty cache after browsing means no original was read.
- **Thumbnails** are in `~/.cache/thumbnails/{normal,large,x-large,xx-large}/`, failures in `fail/`.
- **How long Files is busy** after opening a folder: `/opt/exp/files-freeze.sh <folder>` in the session, with Files
  running, samples its main thread over D-Bus (`lab/bazzite-vm/freeze.sh` starts Files fresh around it on Bazzite,
  where rom-properties made this matter).
- **Failure cases:** throttle the VM (`tc qdisc add dev eth0 root tbf ...`, or `netem delay`) and stop the server
  mid-operation (`docker stop immich_server` in the server's container).

`lab/gnome-vm/experiment.sh` reproduces the original thumbnail experiments with a `bindfs` mount over generated
JPEGs: `fixtures`, then `open <name> <dir>` (cold cache, settle, screenshot, who opened what).

## Traps

- **Fedora cloud VMs hang after their first kernel update** unless the boot entry is fixed. The image has no
  `grubby`, so `up.sh` fixes `/etc/kernel/cmdline` with `sed`.
- **`incus file push` takes `<instance>/<path>`,** with `--project lab` as a flag.
- **The user manager's environment must be set,** because D-Bus-activated apps inherit it: the session scripts set
  `WAYLAND_DISPLAY`, `XDG_SESSION_TYPE`, `XDG_CURRENT_DESKTOP` and `XDG_SESSION_CLASS=user` (without the last,
  `localsearch` never starts).
- **Nothing a login starts runs** (no display manager, no PAM): no `pam_kwallet`, no XDG autostart. That is useful,
  since it shows what happens on installs without them, but remember it when something "should" be running.
- **GNOME's Eval** (`exp/input.sh`) has no `Clutter` global (use `imports.gi`), and drops clicks unless they use
  `Clutter.CURRENT_TIME`, one Eval per event, with hot corners off.
- **A key combination by Eval** (Ctrl+L, Alt+Left) takes one Eval per press and per release, and each Eval is slow
  enough for the key to repeat: turn repeat off first (`gsettings set org.gnome.desktop.peripherals.keyboard repeat
  false` in the session), or Files fills its search with the key. A key whose release was lost stays held; send its
  release alone. Files' grid view swallows Alt+Up; its bottom bar's back and forward buttons, clicked, always work.
- **Files' context menu:** `exp/input.sh` has no right click; with the item selected (`nautilus --select <path>`,
  which selects one file however many are given), `input.sh key Escape` then `key Menu` opens it (the Escape takes
  the dropped first event). Resize the window to the screen first, or a menu opened upward is cut off. A second
  item joins the selection by a Ctrl+click: one Eval each for `Clutter.KEY_Control_L` pressed, the button pressed
  and released, and the key released.
- **There is no browser:** to see the address something opens, make a `.desktop` entry whose `Exec` appends `%u` to
  a file, and `gio mime x-scheme-handler/http <entry>.desktop` (and `https`). The clipboard reads back through the
  shell: an Eval of `St.Clipboard.get_default().get_text(St.ClipboardType.CLIPBOARD, (c, text) => { global._clip =
  text; })`, then a second Eval of `global._clip`.
- **`gio info` hides `filesystem::*`** unless given `-f`.
- **gvfs coalesces a quick unmount and remount,** keeping the old volume and its name. Wait until the mount is gone
  from `gio mount -l` before mounting again.
- **Files keeps a recorded thumbnail failure** until it is started again, F5 included; restart Files between runs
  that test thumbnails.
- **Notifications:** GNOME drops a notification whose `desktop-entry` hint names an app it did not come from, and
  shows no banner for the app that has focus; its message list (click the clock) shows what arrived. A test
  notification from a script: `gdbus call ... Notify ... "int32 -1"` (a bare `-1` reads as an option).
- **`pkill -f immich-mount-gnome` inside `in-session sh -c`** kills its own shell; match the process name with
  `pkill -x immich-mount-gn` (15 characters).
- **The VMs run under `LANG=C.UTF-8`,** where gettext ignores `LANGUAGE`, so `LANGUAGE=da` shows English. To see
  an app in another language: `dnf install glibc-langpack-da` as root, then start it with `LANG=da_DK.UTF-8`; for the
  daemon, `systemctl --user set-environment LANG=da_DK.UTF-8` and restart its service. Without the langpack GTK
  falls back to C and shows its Blueprint texts in English while the rest is translated.
- **The tray** (ARCHITECTURE.md, "The tray"): on GNOME it needs the AppIndicator extension, as Ubuntu and Bazzite
  ship it: `dnf install -y gnome-shell-extension-appindicator` as root, then in the session `gsettings set
  org.gnome.shell enabled-extensions "['appindicatorsupport@rgcjonas.gmail.com']"` and `session.sh` again; the
  daemon registers once the watcher appears. `exp/input.sh click` on the top bar's icon opens its menu. Plasma's
  system tray shows the icon at once, and `in-session click.py 1247 869` opens its menu (the icon's place on the
  1440 by 900 screen, left of the clock). The accessibility bus cannot: it offers the item only "Press", which does
  not open it. The headless KWin reads no input device either (a uinput touchscreen's taps never arrive, and KWin
  6.8's `--virtual` has no `--libinput`), so `click.py`, pushed from `lab/plasma-vm/`, clicks through KWin's own
  fake input protocol, as KDE Connect does; any other click on the screen closes the menu. Its lines are also on
  D-Bus (`gdbus call --session --dest dev.hviid.ImmichMount.Daemon --object-path /MenuBar --method
  com.canonical.dbusmenu.GetLayout -- 0 -1 "[]"`), and `Event` clicks one (`-- 4 clicked "<int32 0>" 0`).
- **Dolphin's context menu** is not on the accessibility bus, and its file items offer no actions there.
  `in-session click.py <x> <y> right` opens it on the item at that place (the first item of a folder in icon view
  is at 567 270 on the 1440 by 900 screen, the next ones 149 px right and 119 px down), and a screenshot shows it.
  Selecting two items needs Ctrl held, which the fake input protocol also sends (`keyboard_key(29, ...)`).
- **The Plasma session is not started by `startplasma`,** so `~/.config/plasma-workspace/env/*.sh` is never
  sourced. To try such a script, do what `startplasma` does: `/bin/sh /usr/libexec/plasma-sourceenv.sh <script>`
  prints the environment it makes, and `dbus-update-activation-environment --systemd NAME=value` hands a variable to
  the apps started next; `systemctl --user unset-environment NAME` takes it back.
- **Harmless noise:** "Error registering session with GDM", and `xdg-desktop-portal` failing because
  `graphical-session.target` is inactive.

## README screenshots

`docs/screenshots/` holds the README's pictures, taken in these VMs against the seeded server (NAT mode, so the
apps show `http://ind-immich-srv.incus:2283`), so every photo in them is a credited fixture. Retake one at the same
size, check it for anything that is not test data, and keep the file name so the README needs no change. Take the
whole session with `shot.sh` and cut the window out with `lab/crop-png.py`. On GNOME, put the window in place first
with the shell's Eval, since the top bar moves where windows open (`gdbus call --session --dest org.gnome.Shell
--object-path /org/gnome/Shell --method org.gnome.Shell.Eval "global.display.focus_window.move_resize_frame(false,
x, y, width, height)"`); on Plasma they open where these boxes expect them:

| Picture | Window placed at (x y width height) | Box (x0 y0 x1 y1) |
|---|---|---|
| Files, `files-*` | 10 42 1260 748 | 0 32 1280 800 |
| GNOME app, `app-connect`, `app-sign-in`, `app-setup`, `app-preferences` (with 10 px of desktop) | 360 66 560 700 | 350 56 930 776 |
| GNOME app, `app-status` (the window only) | 360 66 560 700 | 360 66 920 766 |
| The tray's menu in GNOME's top bar, `gnome-tray` | | 840 0 1280 280 |
| The tray's menu in Plasma's system tray, `kde-tray` | | 1100 660 1440 900 |
| KDE app, `kde-*` | | 449 52 991 801 |
| Dolphin, `dolphin-album` | | 340 138 1101 717 |

The apps' first run gives the sign-in and setup pictures, and the keyring the status page names:
- **Signed out:** `immich-mount logout`, and `immich-mount configure unset server.url` for an empty address field.
- **GNOME:** create the login keyring without its prompt, `printf <password> | gnome-keyring-daemon --replace
  --unlock --components=secrets`, in the session. `act.py` (pushed from `lab/plasma-vm/`) fills GTK 4's fields by
  name and presses its plain buttons; a row that acts as a button ("Sign In") has no action on the bus, so click
  it with `exp/input.sh`, once (a second click lands on the next page). Typing with `exp/input.sh` does not reach
  the app's fields.
- **Plasma:** `act.py` all the way, KWallet's wizard included; its two password fields are `set-nth ksecretd
  'password text' 1` and `2` (counted from 1). Finishing the setup with Open in Dolphin puts the drive in Places.
- **Single sign-on** shows on the sign-in pages once `lab/immich-server/oidc.sh` has run; it restarts Immich, so the
  daemon may tell of a moment offline, a banner GNOME keeps until someone is active: dismiss it before a picture.

## Recording the result

A check's outcome goes where its claim lives: a design reason in ARCHITECTURE.md, observable behaviour in SPEC.md,
a milestone's exit in ROADMAP.md, a quirk in the code comment that handles it. Say what was verified and on what
(for example "verified in a GNOME 50 VM"). Lab notes, logs and screenshots of a run stay out of git: `evidence/`
and `screenshots/` under `lab/` are ignored. New Incus traps go to the Erebus repository's `docs/traps.md`, and new
lab traps into the skill that covers them.

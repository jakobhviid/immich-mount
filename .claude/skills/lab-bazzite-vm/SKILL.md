---
name: lab-bazzite-vm
description: Use to check Immich Mount on Bazzite, GNOME or KDE edition, as its users have it - installed with Homebrew, in a home reached through the /home link, with SELinux enforcing and Bazzite's own file-manager plugins - for example the release check of the Homebrew install. Boots a disposable Bazzite VM on the Incus lab host from a kept base disk (lab/bazzite-vm/), rebuilds that disk from the published bootc images when needed, and covers driving it and its traps.
---

# Bazzite VMs: GNOME and KDE

Read AGENTS.md, "Testing on Erebus", first, and the `lab-desktop-vm` skill: the session, the input helpers, the
screenshots and the measurements are the Fedora VMs', reused here. Most checks also need the test server (the
`lab-immich-server` skill; NAT mode, `http://ind-immich-srv.incus:2283` from the VM).

| | GNOME | KDE (Bazzite's default edition) |
|---|---|---|
| Published image | `ghcr.io/ublue-os/bazzite-gnome:stable` | `ghcr.io/ublue-os/bazzite:stable` |
| Base disk (kept) | `ind-bazzite-gnome-base` | `ind-bazzite-kde-base` |
| VM | `ind-bazzite-gnome-1` | `ind-bazzite-kde-1` |
| Session | GNOME Shell, 1280x800 (`lab/gnome-vm/session.sh`) | Plasma, 1440x900 (`lab/plasma-vm/session.sh`) |
| Input | `/opt/exp/input.sh` (`lab/gnome-vm/exp/`) | `act.py` (`lab/plasma-vm/act.py`) |

What a VM is: a stock Bazzite, nothing layered. The image adds only the Incus agent's units (so `incus exec` works).
- **Homebrew** is Bazzite's: `brew-setup.service` unpacks it to `/home/linuxbrew/.linuxbrew` at first boot, owned by
  uid 1000. It is on the PATH of interactive shells only (`/etc/profile.d/brew.sh`), so run it as a terminal would:
  `run-in-session.sh bash -ic 'brew ...'`.
- **The user** `tester` is uid 1000, in `wheel`, with `HOME=/home/tester`, the folder being `/var/home/tester`, as
  Bazzite's installer makes users.
- **SELinux enforces.** Commands through `incus exec` run as `unconfined_service_t`; apps started with
  `run-in-session.sh -d` run under the user manager, as a login's would.
- **Bazzite's update timers** (`uupd`, Homebrew's update and upgrade) are stopped by `up.sh`, so nothing upgrades
  during a check.

## The loop

```sh
lab/bazzite-vm/up.sh gnome && lab/bazzite-vm/session.sh        # warm: about 18 s, then 4 s; built: about 100 s
lab/bazzite-vm/run-in-session.sh bash -ic 'brew install jakobhviid/tap/immich-mount'
lab/bazzite-vm/shot.sh /tmp/bazzite.png                         # then look at it with the Read tool
lab/bazzite-vm/down.sh                                          # the VM and its disk; the base stays
```

**Warm VMs:** the first `up.sh` copies the base disk, boots through Bazzite's first-boot reboot, waits for Homebrew to
be unpacked, and snapshots the VM's disk (`fresh`); every later `up.sh` restores that snapshot, so each check starts
from a stock Bazzite again. `--fresh` copies the base again (a rebuilt base, say). Run `up.sh` before every check.

The scripts after `up.sh` find the VM by themselves; with both editions up, name one with
`BAZZITE_VM=ind-bazzite-kde-1`. `up.sh` prints the Bazzite version it boots.

## The base disks

- **Why volumes, not images:** the lab projects share the default project's image store, where the lab key may add
  an image but not remove it (the Erebus repository's `docs/traps.md`). So each edition is a custom block volume
  in the `lab` project, 50 GiB and sparse, with `user.keep=true`. `up.sh` boots a btrfs snapshot of it from an empty
  VM (`boot.priority`), with Secure Boot off: Bazzite's kernel is signed with Universal Blue's key.
- **Rebuild them** with `lab/bazzite-vm/build-image.sh [gnome] [kde]`, to check a newer Bazzite. A disposable
  Fedora 44 VM on Erebus (`ind-bazzite-build`) pulls the images (about 10 GB each), adds the agent's units, and runs
  Bazzite's own `bootc install to-disk` onto the volume (btrfs, as Bazzite's installer). Nothing of Bazzite runs on
  the development host. Both editions take about 15 minutes.
- **What a disk holds:** `"$EREBUS_DIR/bin/incus" storage volume show local ind-bazzite-kde-base --project lab`
  lists `user.bazzite.*`: the Bazzite version, the Fedora release, GTK, libadwaita, Qt, Kirigami, fuse3, GNOME
  Shell or Plasma, and the image digest.

## Driving it

- **GNOME:** as in the GNOME VM. `org.gnome.Shell.Eval` with `Main.overview.showApps()` shows the app grid;
  `/opt/exp/input.sh click` opens an icon. Bazzite's shell extensions (its logo menu, the dock in the overview) run
  in the lab session too.
- **KDE:** `act.py press plasmashell 'Application Launcher'`, then `act.py set plasmashell Search Immich`, then
  `act.py press plasmashell 'Immich Mount'` opens the app from the launcher. Dolphin's file items offer no
  accessibility actions; to show a folder, ask Dolphin as other apps do: `gdbus call --session --dest
  org.freedesktop.FileManager1 --object-path /org/freedesktop/FileManager1 --method
  org.freedesktop.FileManager1.ShowFolders "['file:///home/tester/Immich/Photos/2018']" ""`.
- **Signing in:** the keyring prompt (GNOME) and KWallet's first-use wizard (KDE) appear as in the Fedora VMs: no
  PAM login made the login keyring. Answer them as the `lab-desktop-vm` skill says.

## How long Files is busy

`lab/bazzite-vm/freeze.sh "Photos/1997/01 January"` closes Files, drops the page cache, starts Files on Home, opens
the folder, and samples Files' main thread over D-Bus (`lab/gnome-vm/exp/files-freeze.sh`): one line with the busy
time, the longest wait, when Files was last busy, and what the daemon fetched. Measure folders the daemon has not
seen since it started (a fresh month from `lab/immich-server/bulk.sh`, `1997-01:450`): edges and seeds of an earlier
visit answer the rest. To see what took the daemon's time, `systemctl --user set-environment IMMICH_MOUNT_LOG=debug`
and restart the service: each folder is logged with its listing and seeding times. Measured on Bazzite 44 GNOME
with the lab server, a cold month of 450 photos: 2.0.1 kept Files busy for 7 to 9 s; the current daemon, 0.4 to
1 s.

## Who reads the mount, without bpftrace

Bazzite ships neither bpftrace nor strace, and nothing can be installed. The kernel's tracefs names the reader of
every open and read on a FUSE mount, as root in the VM:

```sh
cd /sys/kernel/tracing
echo 'p:labfuse/open fuse_open' >> kprobe_events; echo 'p:labfuse/read fuse_file_read_iter' >> kprobe_events
echo 1 > events/labfuse/enable          # then browse; each line of `trace` starts with <thread name>-<thread id>
echo 0 > events/labfuse/enable; echo '-:labfuse/open' >> kprobe_events; echo '-:labfuse/read' >> kprobe_events
```

Probe arguments name the file and the range: `f=file:%pD` on `fuse_open`, and
`f=iocb->ki_filp:%pD pos=iocb->ki_pos:s64 len=to->count:u64` on `fuse_file_read_iter`. `fuse_readdir` with
`f=file:%pD` names the folders listed, and `fuse_lookup` with `n=entry->d_name.name:string` the names looked up;
`fuse_access` shows a program asking access rights, which Files does for each file it shows and not for a count.
- **The READs the daemon gets,** after the kernel's read-ahead: `fuse_read_args_fill` is inlined into the file read
  path (kernel 7.2), so it shows only directory reads. Probe where requests are sent instead, `fuse_simple_background`
  and `__fuse_simple_request`, with `op=args->opcode:u32 nid=args->nodeid:u64 off=+8(+40(args)):u64
  sz=+16(+40(args)):u32` and the filter `op == 15` (READ); `nid` is the file's inode (`stat -c %i`). Array
  subscripts such as `in_args[0]` are refused.
- **Which code read:** `echo 1 > options/record-tgid` adds the process, and `options/userstacktrace` with
  `options/sym-userobj` adds each event's user stack as library and offset; Bazzite has binutils, so `nm -DC` on
  the library names the function.

The content cache (`~/.cache/immich-mount/<server>/content/*.bin`) shows which originals were fetched. To try the
system without one of its files for a moment, `bootc usr-overlay` makes `/usr` writable until the next reboot.

## Traps

- **Bazzite's file managers read the files in every folder they show.** Bazzite ships rom-properties (a plugin for
  game images; 2.9 on Bazzite 44): its overlay-icon plugin in Dolphin and its extension in Files read the first 8 KiB
  and the last partial page of every file in the folder, shown on screen or not, from the file manager's main thread,
  one file after another, in an order of their own; the daemon fetches each file's edges once, and the rest of the
  folder's ahead of it after three (ARCHITECTURE.md, "Peeks are served from the edges"). That is about 80 KB per
  file on a folder's first visit and nothing after. `ShowDangerousPermissionsOverlayIcon=false` under `[Options]`
  in `~/.config/rom-properties/rom-properties.conf` turns Dolphin's off; Files' has no setting (version 2.9 reads the
  file before it looks at any). Know this before measuring that a folder opens without reading originals.
  Files' extension also probes inside every subfolder shown, from the main thread (libromdata's
  `RomDataFactory::create`): `code/app.xml`, `title.tik`, then `default.xex` and `default.xbe` in three spellings
  each, a statx and an openat apiece. Each is a lookup in a folder the daemon may have to list first; the daemon
  answers these names at once (ARCHITECTURE.md, "Never hang"), so a name rom-properties starts probing for that is
  missing from `is_probe_name` shows as a wait on Files' main thread: trace the lookups (below). A freeze of Files is
  therefore worth a look: its main thread's kernel stack shows who waits.
- **Files lists every subfolder of a folder it shows, to count its items,** one after another from its GIO workers
  (`pool-N`), and looks up every file in each: opening a year lists all its months. The daemon takes those for counts
  (ARCHITECTURE.md, "Counts are seeded ahead"); the debug log says `a count` for each. To move rom-properties aside
  for a control run, `bootc usr-overlay`, then move `/usr/lib64/nautilus/extensions-4/rom-properties-gtk4.so` away
  and back.
- **`/opt/exp/input.sh` runs as root,** through `incus exec`, not through `run-in-session.sh`: `runuser` refuses
  to start from the session user.
- **The first boot reboots once:** `bazzite-hardware-setup` adds kernel arguments and reboots. `up.sh` waits
  through it; `lib/erebus.sh`'s `wait_for` alone takes the moment the VM shows `STOPPED` for a failure.
- **The Incus agent needs a label** under SELinux; `build-image.sh` adds the drop-in. A disk built without it
  boots, but `incus exec` says "VM agent isn't currently running".
- **The builder runs with `selinux=0`,** set by `build-image.sh`: Incus' Fedora image has the SELinux module on
  with no policy, where `bootc install` fails.
- **No Flatpaks, so no Firefox:** Bazzite's ISO installs its Flatpaks, a bootc install does not. And
  `xdg-desktop-portal` cannot start in the headless session (`graphical-session.target` is inactive), so the browser
  sign-in and the `app.immich` hand-off through the portal cannot be checked here yet.
- **A cask's `zap delete:` runs through sudo** (`sudo -A rm -rf`, with Bazzite's `SUDO_ASKPASS`, which needs a
  display), and `tester` has no password: the zap stops after removing the artifacts, the cask stays listed, and a
  plain retry fails with "already a Generic Artifact". Our cask zaps with `trash:` only for that reason. If a zap
  asks anyway, give `tester` a throwaway password (`chpasswd` as root), point `SUDO_ASKPASS` at a script that
  prints it, and retry with `--force`.
- **Homebrew's broken pipe:** a first `brew install` on a fresh VM, its output not a terminal, sometimes stops
  right after tapping with `Error: Broken pipe` (seen in 3 of 7 tries); the same command then succeeds.
  `lab/check/run.sh` tries once more for exactly that. Not seen in a terminal.
- **No `tc` on Bazzite** to slow a copy into Upload. Copy slowly from one process instead (a short Python loop that
  writes and sleeps): a shell loop such as `for ...; do dd ...; done > file` closes the file from every child, and
  the daemon takes the first close for the end of the copy.
- **Measure each folder with its pages out of memory** (`echo 3 > /proc/sys/vm/drop_caches` as root, the file
  manager closed): reads the kernel answers from memory never reach the daemon, and the measurement shows nothing.
- **No right click in Plasma out of the box:** `up.sh` pushes `act.py` but not `click.py`, and Bazzite has neither
  pywayland nor the protocol XML, and its KWin (6.7) offers fake input only to programs whose desktop file asks for
  it ("KWin offers no fake input to this program"). For a session that needs it: `python3 -m pip install --user
  pywayland` as tester; fetch `wayland.xml` (freedesktop's wayland repository, `protocol/`) and `fake-input.xml`
  (KDE's plasma-wayland-protocols, `src/protocols/`) into the home and point a copy of `click.py` at them; add
  `~/.local/share/applications/lab-python-fakeinput.desktop` with `Exec=/usr/bin/python3.14` and
  `X-KDE-Wayland-Interfaces=org_kde_kwin_fake_input`, run `kbuildsycoca6`, and call `/usr/bin/python3.14 click.py
  x y right` (the real path, not the `python3` link). Dolphin's context menu items are not on the accessibility
  bus: click them by position from a screenshot.
- **A context menu in Files:** a right click through the shell's virtual pointer (`notify_button` with
  `BUTTON_SECONDARY`) selects the item but opens no menu; `/opt/exp/input.sh click` on the item, then `input.sh key
  Menu`, does.
- **`wl-copy` through `run-in-session.sh` never returns:** it forks a server that keeps the clipboard and holds the
  pipe open. Redirect its output (`wl-copy ... >/dev/null 2>&1`), or end it with `pkill -u tester wl-copy`.
- **Harmless:** `ublue-os-media-automount.service` fails on every boot (a bootc install writes no `/etc/fstab`
  entries); the VM's empty root disk shows as a blank device under Dolphin's Devices.
- **A dead guest's journal:** stop the VM, remove its `bazzite` disk device, attach its volume to another VM (the
  builder), mount the btrfs root, and read `journalctl -D <root>/ostree/deploy/default/var/log/journal`.

## Recording the result

As in the `lab-desktop-vm` skill: a check's outcome goes where its claim lives (say "verified on Bazzite 44 GNOME"
or "KDE"), lab traps go here, Incus traps to the Erebus repository's `docs/traps.md`, and logs and screenshots stay
in the ignored `lab/**/evidence/`.

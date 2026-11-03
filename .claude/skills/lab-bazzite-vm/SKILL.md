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
lab/bazzite-vm/up.sh gnome && lab/bazzite-vm/session.sh        # about 55 s, then 4 s; or up.sh kde
lab/bazzite-vm/run-in-session.sh bash -ic 'brew install jakobhviid/tap/immich-mount'
lab/bazzite-vm/shot.sh /tmp/bazzite.png                         # then look at it with the Read tool
lab/bazzite-vm/down.sh                                          # the VM and its disk; the base stays
```

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

## Who reads the mount, without bpftrace

Bazzite ships neither bpftrace nor strace, and nothing can be installed. The kernel's tracefs names the reader of
every open and read on a FUSE mount, as root in the VM:

```sh
cd /sys/kernel/tracing
echo 'p:labfuse/open fuse_open' >> kprobe_events; echo 'p:labfuse/read fuse_file_read_iter' >> kprobe_events
echo 1 > events/labfuse/enable          # then browse; each line of `trace` starts with <thread name>-<thread id>
echo 0 > events/labfuse/enable; echo '-:labfuse/open' >> kprobe_events; echo '-:labfuse/read' >> kprobe_events
```

The content cache (`~/.cache/immich-mount/<server>/content/*.bin`) shows which originals were fetched. To try the
system without one of its files for a moment, `bootc usr-overlay` makes `/usr` writable until the next reboot.

## Traps

- **Bazzite's file managers read the files in every folder they show.** Bazzite ships rom-properties (a plugin for
  game images): its overlay-icon plugin in Dolphin and its extension in Files read each file's start from the
  file manager's main thread, which fetches the original. `ShowDangerousPermissionsOverlayIcon=false` under
  `[Options]` in `~/.config/rom-properties/rom-properties.conf` turns Dolphin's off; Files' has no setting. Know
  this before measuring that a folder opens without reading originals.
- **The first boot reboots once:** `bazzite-hardware-setup` adds kernel arguments and reboots. `up.sh` waits
  through it; `lib/erebus.sh`'s `wait_for` alone takes the moment the VM shows `STOPPED` for a failure.
- **The Incus agent needs a label** under SELinux; `build-image.sh` adds the drop-in. A disk built without it
  boots, but `incus exec` says "VM agent isn't currently running".
- **The builder runs with `selinux=0`,** set by `build-image.sh`: Incus' Fedora image has the SELinux module on
  with no policy, where `bootc install` fails.
- **No Flatpaks, so no Firefox:** Bazzite's ISO installs its Flatpaks, a bootc install does not. And
  `xdg-desktop-portal` cannot start in the headless session (`graphical-session.target` is inactive), so the browser
  sign-in and the `app.immich` hand-off through the portal cannot be checked here yet.
- **Harmless:** `ublue-os-media-automount.service` fails on every boot (a bootc install writes no `/etc/fstab`
  entries); the VM's empty root disk shows as a blank device under Dolphin's Devices.
- **A dead guest's journal:** stop the VM, remove its `bazzite` disk device, attach its volume to another VM (the
  builder), mount the btrfs root, and read `journalctl -D <root>/ostree/deploy/default/var/log/journal`.

## Recording the result

As in the `lab-desktop-vm` skill: a check's outcome goes where its claim lives (say "verified on Bazzite 44 GNOME"
or "KDE"), lab traps go here, Incus traps to the Erebus repository's `docs/traps.md`, and logs and screenshots stay
in the ignored `lab/**/evidence/`.

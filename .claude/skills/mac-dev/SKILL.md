---
name: mac-dev
description: Use to build, run, test or debug the macOS edition (apple/ and the bridge, crates/immich-mount-apple) on the maintainer's Mac over SSH from Linux - building the bridge and the Xcode project, a signed build in the maintainer's session, reaching the lab server through tunnels, driving the app with launch arguments, reading the unified log and the system's File Provider state, and cleaning up. The Mac is the maintainer's everyday laptop; the rules here keep it that way.
---

# The Mac: building and testing the macOS edition

ARCHITECTURE.md, "The macOS edition", has the design; ROADMAP.md, M7, the steps. There are no macOS VMs (AGENTS.md,
"Development environment"): the Mac is the maintainer's laptop, available in sessions.

## Rules

The Mac is a personal laptop the maintainer carries and uses every day, not a lab machine:
- **Only build, test and debug there.** Never change its settings or state: no System Settings, no `sudo`, no
  `xcode-select -s`, no edits to shell profiles, no global installs or uninstalls, no `defaults write` outside our
  own app. Anything like that is the maintainer's to do, after asking.
- **Stay inside the checkout** (`~/Developer/Private/immich-mount` there) and its git-ignored `apple/build/`.
- **Do on Linux what Linux can do:** Rust builds and tests, the bridge's lab tests, bindings, docs. CI's `macos`
  job builds the Swift side on every branch push. The Mac is for Xcode builds, running the app and the extension,
  Finder and logs.
- **It may be asleep or away:** start with a short SSH command, and never assume it is there.
- **Clean up what a check leaves:** sign out with the launch argument below (it removes the Finder location, the
  account and the extension's store), unless the maintainer keeps using it. Close the tunnels.

## Reaching it

`MAC_HOST` (`user@address`; private, never written into the repository) with the SSH key the maintainer
authorized. Homebrew's tools are on the PATH only in a login shell, so every command goes through zsh:

```sh
ssh "$MAC_HOST" 'zsh -lc "cd ~/Developer/Private/immich-mount && git pull -q && ..."'
```

Traps (verified on macOS 27.0.1 with Xcode 27.2):
- **`xcode-select` may point at the Command Line Tools,** where `xcodebuild` refuses to run. Set
  `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` on our own commands; never change `xcode-select`.
- **Homebrew's `rust` formula beside rustup** puts a cargo without the x86_64 standard library first on the PATH.
  `apple/build-rust.sh` puts rustup's first for itself; other cargo commands need
  `PATH=$(brew --prefix rustup)/bin:$PATH`.
- **In zsh, `log` is a builtin:** call `/usr/bin/log`.

## Build

```sh
apple/build-rust.sh debug        # the bridge for both architectures, its Swift bindings, the xcframework (45 s)
cd apple && xcodegen             # ImmichMount.xcodeproj, git-ignored
```

**Unsigned, over SSH,** to see that it compiles. `-derivedDataPath` keeps the build inside the checkout, and a path
of its own keeps it from replacing the signed app in `build/DerivedData`:

```sh
xcodebuild -project ImmichMount.xcodeproj -scheme ImmichMount -configuration Debug -destination platform=macOS \
  -derivedDataPath build/UnsignedData CODE_SIGNING_ALLOWED=NO build
```

**Signed, in the maintainer's session.** Running needs a signed build (the extension's entitlements), and signing
cannot happen over SSH: there the login keychain is locked ("User interaction is not allowed"), so codesign cannot
reach the key. Write the build into a script in `apple/build/` and start it with `open -a Terminal
apple/build/<name>.command`: it runs in a Terminal window in the maintainer's session, and its log, written into
`apple/build/`, is read over SSH. The build passes `-allowProvisioningUpdates`; the free Personal Team then
provisions the App IDs, the App Group and the Keychain group (verified). The team comes from the git-ignored
`apple/Config/Local.xcconfig` (`DEVELOPMENT_TEAM = <team>`); the team ID is the OU of the signing certificate:
`security find-certificate -c "Apple Development" -p | openssl x509 -noout -subject`.

The first signed build stops at a keychain dialog ("codesign wants to sign using key ..."), which waits for the
maintainer's login password and **Always Allow**, once per key; later signed builds need nothing (verified). The
script's Terminal window stays open after the build; the maintainer closes it.

## The server

The lab server in NAT mode (the `lab-immich-server` skill; LAN mode is never left up unattended), forwarded to the
Mac's loopback through Linux:

```sh
socat TCP-LISTEN:2283,bind=127.0.0.1,reuseaddr,fork \
  EXEC:"$EREBUS_DIR/bin/incus exec ind-immich-srv -- socat STDIO TCP\:127.0.0.1\:2283" &
ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -R 127.0.0.1:2283:127.0.0.1:2283 "$MAC_HOST" &
```

On the Mac the server is then `http://127.0.0.1:2283` (verified). Background jobs end after two hours, the tunnels
with them; `lab/mac/check.sh` opens its own when nothing answers there, and closes them at its end. The tunnel starts an `incus exec` per connection,
which is slow when many open at once: a request that times out makes the extension take the server as offline for
45 seconds ("the server was unreachable moments ago"), as Linux does, so run checks one at a time. Loopback is not the local network, so macOS's
Local Network prompt never comes up; checking that prompt itself needs a LAN-mode server and the maintainer at the
Mac. The bridge's lab tests run on the Mac too, against the tunnel: `IMMICH_URL=http://127.0.0.1:2283
IMMICH_API_KEY_READONLY=<key> cargo test -p immich-mount-apple --test lab -- --ignored` (passed on macOS 27).

## Run and drive

A development build takes launch arguments (`AppModel.followLaunchArguments`, DEBUG only), so a script signs in
without typing into the window:

```sh
app="apple/build/DerivedData/Build/Products/Debug/Immich Mount.app"
open -n "$app" --args -signInServer http://127.0.0.1:2283 -signInAPIKey <key>
open -n "$app" --args -signOut YES       # the Finder location, the account and the store go
pkill -x "Immich Mount"                  # quit it; no Apple events, so no Automation prompt
```

`open` from SSH starts the app in the maintainer's session, where the Keychain and File Provider work (verified:
signed in to the lab server through the tunnel, and added the Finder location).

The extension serves Finder only once the maintainer has switched it on, once per Mac: System Settings, General,
Login Items & Extensions, File Providers. No command can do it for them.

## Check

`lab/mac/check.sh` (from Linux, with `MAC_HOST` and `MAC_SSH_OPTIONS`; pull the Mac's checkout first) starts fresh
(the app signs out and in again with the lab's full key) and prints one PASS or FAIL line per claim: the
sections, every timeline photo, listing downloads no original, originals read back as the server's (SHA-1), a
thumbnail without its original, videos byte for byte, a photo added to and taken out of an album, a photo added
while the extension was stopped and deleted again, a photo moved into Upload (its source gone, the server has it,
it leaves Upload), and a refused file staying in Upload. About 85 seconds. It needs a signed build, the tunnels and
the extension switched on; it works through the checkout's `apple/build/check/`, and cleans up after itself.

## Look

- **Our log** (the app, the extension and the bridge): `/usr/bin/log show --last 10m --predicate 'subsystem ==
  "dev.hviid.ImmichMount"'`; the system's side: `subsystem == "com.apple.FileProvider"`.
- **The system's state:** `fileproviderctl dump` (domains, whether ours is enabled, its items); whether the
  extension is switched on: `fileproviderctl dump | grep -A4 '^domain: dev.hviid' | grep enabled`. macOS 27's
  `fileproviderctl` also has `diagnose`, `evaluate` and `check`.
- **The location on disk:** `~/Library/CloudStorage/ImmichMount-Immich`, which SSH can list and read (verified).
- **Thumbnails as Finder asks for them:** `xcrun swift lab/mac/thumbnail.swift <file>` (QLThumbnailGenerator).
  `qlmanage -t` reads the file itself, which downloads the original: no test of the extension's thumbnails.
- **Info-level lines** (downloads, listings) show only with `/usr/bin/log show --info`.
- **A known system error:** the same photo copied into Upload twice under two names makes `fileproviderd` log
  `itemChangedDuringPropagation` for `asset|upload|...` while it merges the two into one item. It passes; the photo
  reaches the server once and leaves Upload.

## Record

Results go where their claim lives (AGENTS.md, "Testing on Erebus", "Record results in the docs"): ARCHITECTURE.md,
"The macOS edition", and ROADMAP.md, M7. New traps of the Mac go here.

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
- **Test with Immich Mount Dev, never the release:** the Debug build has an identity of its own (AGENTS.md,
  "Development environment"), so signing it out and in, its settings and its Finder location leave the maintainer's
  installed release and its sign-in alone. Quit only what runs from `Immich Mount Dev.app`, by its path (`pkill -f
  "Immich Mount Dev.app/Contents/"`): its extension's process has the release's name.
- **Clean up what a check leaves:** sign out with the launch argument below (it removes the Finder location, the
  account and the extension's store), unless the maintainer keeps using it. Close the tunnels.

## Reaching it

`MAC_HOST` (`user@address`; private, never written into the repository) with the SSH key the maintainer
authorized. Homebrew's tools are on the PATH only in a login shell, so every command goes through zsh:

```sh
ssh "$MAC_HOST" 'zsh -lc "cd ~/Developer/Private/immich-mount && git pull -q && ..."'
```

Traps (verified on macOS 27.0.1 with Xcode 27.2):
- **GitHub may refuse the Mac's SSH key** ("Permission denied (publickey)" on `git pull`, seen on 2026-11-09). The
  repository is public, so fetch over HTTPS, which needs no credentials: `git fetch -q
  https://github.com/jakobhviid/immich-mount.git <branch>:<branch> && git switch -q <branch>`, or for `main`,
  `git fetch -q https://github.com/jakobhviid/immich-mount.git main && git merge -q --ff-only FETCH_HEAD`.
- **`xcode-select` may point at the Command Line Tools,** where `xcodebuild` refuses to run. Set
  `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` on our own commands; never change `xcode-select`.
- **Homebrew's `rust` formula beside rustup** puts a cargo without the x86_64 standard library first on the PATH.
  `apple/build-rust.sh` puts rustup's first for itself; other cargo commands need
  `PATH=$(brew --prefix rustup)/bin:$PATH`.
- **In zsh, `log` is a builtin:** call `/usr/bin/log`.
- **macOS caches an app's icon by its path.** A build at a path macOS saw without an icon keeps showing the blank
  placeholder (also in the app's own window, through `applicationIconImage`). `touch` the built `.app` and start it
  again; a fresh copy elsewhere shows the icon at once (verified on macOS 27).
- **fileproviderd keeps the extension's launch job at the path it first ran from.** After that build is deleted, a
  copy elsewhere (a release build in `/Applications`) never starts: launchd logs "Attempt to re-bootstrap service
  from different path, will use existing" and "Missing executable detected", and Finder says the provider
  "encountered an error". `killall fileproviderd` (it runs as the user, and launchd starts it again at once) clears
  the job, and the next request starts the extension from the new path (verified on macOS 27.0.1). Before deleting a
  build, unregister it: `lsregister -u <app>` (in LaunchServices.framework's `Support/`).
- **Deleting the Dev app leaves its Finder location** in the sidebar: macOS keeps a provider's location until the app
  removes it (by signing out), and serves it from any copy it still knows of. Deleted from Applications, the Dev
  build ran on from `apple/build/DevData/` (seen on macOS 27.2). `apple/dev-app.sh --remove` signs it out first,
  quits it, unregisters both copies and deletes them.
- **notarytool's keychain profile is not found while the screen is locked** ("No Keychain password item found",
  with the maintainer away, two hours after it worked; seen on macOS 27.0.1), though the login keychain's signing key
  still is. An unattended release build notarizes with the API key file instead: `NOTARY_KEY=<.p8>
  NOTARY_KEY_ID=<ID> NOTARY_ISSUER=<issuer> apple/release.sh`.
- **A keychain dialog left open stops Xcode's package downloads:** Sparkle's framework archive waited for over an hour
  behind one (`waitForRemoteSourcePackagesToFinishLoading`, nothing downloading). `pgrep -x SecurityAgent` shows one
  is open; it is the maintainer's to answer, or, with their leave, stopping SecurityAgent cancels it.
- **No `osascript` over SSH, ever.** Apple events from an SSH session (to System Events, or `tell application … to
  quit`) make macOS ask the maintainer whether `sshd-keygen-wrapper`, the program that starts SSH sessions, may
  control that app, and accepting widens what every SSH session may do: a laptop setting this skill must not touch.
  Quit the Dev app with `pkill -f "Immich Mount Dev.app/Contents/MacOS"` (a signal needs no permission; macOS cuts
  process names at 15 characters, so `pkill -x` misses it), and ask the maintainer what is on screen.
- **zsh aborts a command whose glob matches nothing** ("no matches found"), and the rest of an SSH command line with
  it. Run cleanup scripts through `bash -s`.

## Build

```sh
apple/build-rust.sh debug        # the bridge for both architectures, its Swift bindings, the xcframework (45 s)
cd apple && xcodegen             # ImmichMount.xcodeproj, git-ignored
```

**Immich Mount Dev, the usual build:** `open -a Terminal apple/dev-app.command` from SSH builds it signed in the
maintainer's session, installs it as `/Applications/Immich Mount Dev.app` and starts it; `apple/build/dev-app.log`
has its output. Its sign-in, settings and Finder location stay from one install to the next.

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

**A release build** (`apple/release.sh`; ARCHITECTURE.md, "The macOS edition", "Distribution") runs the same way,
in a `.command` script, because it signs too. It needs, once per Mac:
- the Developer ID Application certificate with its key in the login keychain (ARCHITECTURE.md, "Distribution", on
  where it is made)
- a notarytool keychain profile, stored in a Terminal window from the API key:
  `xcrun notarytool store-credentials immich-mount-notary --key <.p8> --key-id <ID> --issuer <issuer>`

Then `TEAM_ID=<team> NOTARY_PROFILE=immich-mount-notary apple/release.sh`, without profile names:
Xcode makes the two Developer ID profiles itself. Notarizing takes a few minutes, twice (the app, then the disk
image); `--not-notarized` checks the signing alone. `apple/release.sh --unsigned` needs none of it and runs
over SSH. Development and release builds share the team, so they share the App Group and the Keychain group: a
release build finds the development build's sign-in. Remove the development build before opening a release build,
so the system has one extension with that identifier to run.

**Releasing by hand,** while the release workflow's `macos` job waits for its secrets: once `main`'s release
workflow has published `v<version>`, check out that commit on the Mac and build there; then from Linux, copy the disk
image over, sign it for Sparkle with `ci/sign-mac-update.sh <dmg> ~/.config/immich-mount-release/sparkle-ed25519.pem`,
attach both (`gh release upload v<version> <dmg> <dmg>.update`), and run the page (`gh workflow run pages.yml`),
which then offers the disk image and the update feed.

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
app="/Applications/Immich Mount Dev.app"
open -n "$app" --args -signInServer http://127.0.0.1:2283 -signInAPIKey <key>
open -n "$app" --args -signOut YES       # its Finder location, account and store go; the release's stay
open -n "$app" --args -settingsPane 1    # Settings, on a pane (0 General, 1 Folders, 2 Account)
pkill -f "Immich Mount Dev.app/Contents/MacOS"   # quit it; no Apple events, so no Automation prompt
```

`open` from SSH starts the app in the maintainer's session, where the Keychain and File Provider work (verified:
signed in to the lab server through the tunnel, and added the Finder location).

The extension serves Finder only once the maintainer has switched it on, once per Mac: Enable at the top of the
location's Finder window, or its switch in System Settings, General, Login Items & Extensions, File Providers. No
command can do it for them.

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
- **The location on disk:** `~/Library/CloudStorage/ImmichMount-ImmichMount` (the drive's name, `mount.name`,
  without spaces), which SSH can list and read (verified).
- **Thumbnails as Finder asks for them:** `xcrun swift lab/mac/thumbnail.swift <file>` (QLThumbnailGenerator).
  `qlmanage -t` reads the file itself, which downloads the original: no test of the extension's thumbnails.
- **Info-level lines** (downloads, listings) show only with `/usr/bin/log show --info`.
- **The app's windows,** when one "does not appear": `xcrun swift lab/mac/windows.swift` lists each window's layer,
  bounds and whether it is on screen, without Screen Recording permission (it reads no names or images). A window
  on screen that the maintainer cannot see is behind another app's; `sample <pid> 1` then shows whether the main
  thread waits in a modal alert (verified on macOS 27).
- **A known system error:** the same photo copied into Upload twice under two names makes `fileproviderd` log
  `itemChangedDuringPropagation` for `asset|upload|...` while it merges the two into one item. It passes; the photo
  reaches the server once and leaves Upload.

## Record

Results go where their claim lives (AGENTS.md, "Testing on Erebus", "Record results in the docs"): ARCHITECTURE.md,
"The macOS edition", and ROADMAP.md, M7. New traps of the Mac go here.

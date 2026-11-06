# AGENTS.md

Instructions for any AI coding agent (Claude Code, opencode, and others) and for humans. Read this first.
`CLAUDE.md` is a symlink to this file, so there is exactly one copy of the rules. Edit `AGENTS.md`, never the link.

This project follows the maintainer's house style for Rust command-line tools, `rust-cli-guidelines` (a private guide:
governance, code style, CLI patterns, release). Where those guidelines assume a pure CLI, the adaptation is written down
in ARCHITECTURE.md, "Note for the house guidelines".

## Hard rules

- **Never add AI attribution** to commits or pull requests: no `Co-Authored-By` trailer for an assistant, no
  "Generated with" line. Author every commit as the repository owner. AI use is disclosed once, in the README's
  "AI disclosure" section.
- **No em-dashes** in anything written here: docs, code comments, commit messages. Use a hyphen, comma, colon or
  parentheses, or rewrite the sentence.
- **Do not commit or push unless the maintainer asks.** Use Conventional Commit prefixes when you do (see
  Versioning).
- **Keep the credit.** Immich Mount is inspired by
  [Kartax/immich-desktop-app](https://github.com/Kartax/immich-desktop-app).
  - Every project we take ideas or code from is credited in the README's credits section and in
    [CREDITS.md](CREDITS.md), the way open-source projects usually credit: a line on what it is and what it gave
    us, not an inventory. No per-file headers.
  - MIT-licensed code may be copied and adapted freely; keep its notice once, in `THIRD-PARTY-NOTICES.md`.
  - Code, text or graphics without a license (Kartax's app, for one) are never copied, only learned from. Its app
    icon, logo and screenshots are never used, nor anything drawn after them.
  - Never remove or dilute the credit.
- **Say that we are unofficial.** The non-affiliation notice (SPEC.md) appears in the README, the About dialog,
  AppStream metadata and every package description. The Immich logo is never our icon or branding; our icon
  (`packaging/icons/`) takes Immich's colours, never its shapes.
- **Keep the core portable.** No FUSE, D-Bus, GTK or OS-specific code in `immich-api` or `immich-mount-core`.
  Platform specifics go in adapters, behind traits.
- **Keep a future Flatpak possible.** Every design change states its Flatpak impact against the readiness rules in
  ARCHITECTURE.md.

## The docs, and which one to change

The docs are load-bearing. They are embedded into `immich-mount --llm`, and a behaviour change ships with its doc
change in the same commit. When code and docs disagree, the code wins and the doc is the bug.

| File | Holds | Change it when |
|---|---|---|
| [README.md](README.md) | The landing page: what it is, keywords, install, credits, AI disclosure | The pitch, the install story or the credits change |
| [WORKFLOWS.md](WORKFLOWS.md) | What you run: install, sign in, status, doctor, settings, troubleshooting, JSON shapes | A command, a flag, an output or a recipe changes |
| [SPEC.md](SPEC.md) | The contract: vocabulary, the mount's layout, names and attributes, content, thumbnails, uploads, sign-in, config keys, CLI, D-Bus, files, errors | Observable behaviour changes |
| [ARCHITECTURE.md](ARCHITECTURE.md) | The model and every design decision with its reason, plus the alternatives that lost where a question tends to recur | A design decision is made or changed |
| [PRINCIPLES.md](PRINCIPLES.md) | The invariants, and what earned them | Rarely: only after discussing it with the maintainer |
| [ROADMAP.md](ROADMAP.md) | Milestones, open questions, ideas parked on purpose | Plans change, a question opens or closes, or something is deferred |
| [CREDITS.md](CREDITS.md) | Who we learned from or copied from, with licenses | We take anything from another project |
| `.claude/skills/` | Procedures for agents: the lab environments, how to drive them, their traps, and where a check's result is recorded | A lab procedure or trap changes |

Decisions live inline in ARCHITECTURE.md (or SPEC.md, for behaviour), never in separate decision-record files. Open
questions live in ROADMAP.md.

**Describe the current state, not the change.** A doc that says what *used to* be true misleads the next reader.
Before committing, grep your own doc diff for: "now", "no longer", "used to", "previously", "was changed",
"reopened", "superseded", "instead of the old". The before and after belong in the commit message.

## Versioning

CI derives the version from commit subjects on every push to `main`. Nobody edits `version` by hand.

The first commit is 1.0.0, and every later one bumps from there:

| Subject | Bump |
|---|---|
| `type!: ...` (breaking) | major |
| `feat: ...` | minor |
| anything else (`fix:`, `docs:`, `chore:`, ...) | patch |

A `BREAKING CHANGE` footer is not scanned. Mark breaking changes with `!` in the subject.

## The green gate

Before every push, run in the toolbox:

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p immich-api -p immich-mount-core --no-default-features --target aarch64-apple-darwin   # the core stays portable
```

CI runs the first two lines with `--exclude immich-mount-gnome --exclude immich-mount-kde` on Ubuntu, and checks
both desktop apps separately in a `fedora:44` container, because they need GTK 4.18, libadwaita 1.8 and
`blueprint-compiler` (GNOME) and Qt 6 (KDE). The toolbox has them, so the local gate covers the whole workspace.

The macOS side (`apple/`, and the bridge built for macOS) has no gate on Linux: CI's `macos` job builds it and runs
the tests on a Mac runner on every branch push, so pushing the branch is how a change is checked without a Mac.

There is deliberately no `cargo fmt` gate. Readability is a review concern; readable names are required (no
single-letter bindings), per the house style.

Before a milestone closes, especially uploads (M3) and every release, an independent agent runs an adversarial
review covering safety, correctness, and docs against behaviour.

## Reference material

- **Kartax's macOS app** (read-only reference, no license, so nothing is copied): `git clone --depth 1
  https://github.com/Kartax/immich-desktop-app /tmp/immich-ref/src` if missing. The most useful files are
  `Shared/ImmichClient.swift`, `Shared/AssetUpload.swift`, `FileProviderExt/ItemEnumerator.swift` and `Tests/`.
- **immich-rs** (MIT), the source of our API layer: https://github.com/boul2gom/immich-rs at commit `60d1eb3`.
- **The Immich API:** the server serves its OpenAPI spec at `/api/spec.json`; the immich repository has it at
  `open-api/immich-openapi-specs.json`.

## Development environment

Build on Fedora 44, or in a Fedora 44 toolbox (the easy way on an immutable host such as Silverblue or CoreOS:
`toolbox create -r 44 immich-mount`).
- **Packages:** `gcc`, `cargo`, `rust`, `clippy` and `rustfmt`. The GNOME app also needs `gtk4-devel`,
  `libadwaita-devel` and `blueprint-compiler`, and the KDE app `gcc-c++`, `qt6-qtbase-devel` and
  `qt6-qtdeclarative-devel`.
- **Mounting** needs `/dev/fuse` and a setuid `fusermount3` (package `fuse3`). The daemon can be mounted and tested
  on the development host directly.
- **A global cargo linker the toolbox lacks** (zig through `cargo-zigbuild`, say, on a host without a C toolchain)
  is overridden with the environment, rather than by editing the global config:

  ```sh
  toolbox run -c immich-mount bash -c 'export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc \
    CC_x86_64_unknown_linux_gnu=cc CXX_x86_64_unknown_linux_gnu=c++ AR_x86_64_unknown_linux_gnu=ar; \
    cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace'
  ```

- **The portability check** (`--target aarch64-apple-darwin`) runs in CI; Fedora's Rust has no `rustup` to add
  targets. It runs with `--no-default-features`, which leaves out TLS: ring's C code does not cross-compile for
  macOS on Linux.
- **Anything that needs a desktop session** is tested in a VM (below).
- **The macOS edition** (`apple/`; ARCHITECTURE.md, "The macOS edition") builds only on a Mac with Xcode. There:
  `brew install xcodegen`, `rustup target add aarch64-apple-darwin x86_64-apple-darwin`, then `apple/build-rust.sh`
  and `xcodegen` in `apple/`. A development build is signed with the developer's own team, set in the git-ignored
  `apple/Config/Local.xcconfig` (`DEVELOPMENT_TEAM = ...`); a free Personal Team is enough. On first run, switch the
  extension on in System Settings (General, Login Items & Extensions) and allow the Local Network prompt.
- **No macOS virtual machines on Erebus:** Apple's license allows macOS VMs only on Apple hardware. The
  maintainer's Mac is a laptop in daily use, so Mac work comes in sessions; between them, CI's macOS job is the Mac.
  The `mac-dev` skill covers building, running and debugging on it over SSH, and the rules that keep it a laptop.

## Testing on Erebus

**Erebus** is the maintainer's Incus host, for disposable VMs and containers. Its setup lives in a private
repository, `erebus`, checked out next to this one; `EREBUS_DIR` points elsewhere. On another Incus host, give
`EREBUS_DIR` a directory with what the `lab/` scripts use from it:
- `bin/incus`: the `incus` client, already pointed at the host
- `lib/erebus.sh`, sourced from bash, defining `inc` (runs `bin/incus`), `step` (prints a timed progress line), `die`
  (prints and exits 1), `state <project> <instance>` (the instance's status), `wait_for <project> <instance> <what>
  <command...>` (waits until the command succeeds in the instance) and `wait_cloud_init <project> <instance>`

On Erebus itself:
- Read the `erebus` repository's `AGENTS.md` and `docs/traps.md` before the first call.
- Always use its `bin/incus` wrapper, and give calls that should not read stdin `</dev/null`.
- Check access with `$EREBUS_DIR/bin/incus info </dev/null | grep auth:`; it should print `auth: trusted`.

The procedures are skills in `.claude/skills/`; agents that do not load skills read the `SKILL.md` directly.

| Environment | What it gives you | Skill |
|---|---|---|
| `lab/immich-server/` | A disposable Immich server (Docker CE in a nested Debian 13 container, no machine learning), seeded with 50 Creative Commons photos and edge cases, a full-access and a read-only API key, and optionally Dex for OIDC sign-in | `lab-immich-server` |
| `lab/gnome-vm/` | A Fedora 44 VM with a headless GNOME 50 session and Files, that you can drive, screenshot and trace | `lab-desktop-vm` |
| `lab/plasma-vm/` | A fresh Fedora 44 VM with a headless Plasma 6 session and Dolphin, driven by the names on screen (`act.py`); nothing of ours preinstalled | `lab-desktop-vm` |
| `lab/bazzite-vm/` | A stock Bazzite VM, GNOME or KDE edition, booted from a kept base disk built from the published bootc images: Bazzite's own Homebrew, `/home` a link to `/var/home`, SELinux enforcing, and the headless sessions and input helpers of the two VMs above | `lab-bazzite-vm` |
| `lab/packaging/` | A release's .rpm and .deb installed, checked and removed on several distributions at once; with the server up, each also mounts it and checks originals against Immich's checksums | `lab-packaging-matrix` |
| `lab/check/` | The release check: a desktop VM of one kind (Fedora GNOME or Plasma, Bazzite GNOME or KDE) and the server, warm, with the release installed as its users install it, signed in from the command line, and one PASS or FAIL line per documented claim, in about a minute; on Bazzite also the cask's uninstall | `lab-release-check` |

Rules:
- **Lab instances are disposable, and stay warm for a working session.** A VM or server may stay up between checks
  and is deleted when the session ends. Any `ind-` instance found at the start of a session may be destroyed and
  created again at will, unless it is kept (`user.keep=true`) or another agent of this session is using it. Warm is
  not dirty: snapshot an instance once it is set up, and restore that snapshot before each check, so no check passes
  on what an earlier one left. Give each instance `user.expires` (a UTC time, at most 12 hours ahead); a session
  starts by deleting the expired ones, so an abandoned session leaves nothing for long. Never run the Erebus
  teardown script with `--delete`: other projects' instances run there too.
- **Prefix instance names with `ind-`,** and work in the `lab` project. Agents that run at the same time use a name
  suffix of their own (`ind-gnome-2`), and leave each other's instances alone.
- **Kept things** need `user.keep=true` and a note to the maintainer: the Bazzite base disks are kept.
- **Fixtures are Creative Commons or public domain,** and never upload personal photos to a test server.
- **Test credentials are throwaway,** generated per deployment into a gitignored `.env.local`.
- **Wait on conditions, never on bare sleeps,** with `wait_for` and `wait_cloud_init` from `lib/erebus.sh`. Incus
  reports `RUNNING` even when a VM is stuck in boot. Source the helpers from **bash**, not zsh.
- **Record results in the docs, not in lab notes.** A check's outcome goes where its claim lives: ARCHITECTURE.md
  for a design reason, SPEC.md for behaviour, ROADMAP.md for a milestone, a code comment for a quirk. Logs and
  screenshots of a run stay out of git (`lab/**/evidence/` and `lab/**/screenshots/` are ignored); README pictures
  live in `docs/screenshots/`, and the download page (`site/`) shows them too.
- **New traps:** Incus ones go into the Erebus repository's `docs/traps.md`, lab ones into the skill that covers
  the environment.

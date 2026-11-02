---
name: lab-packaging-matrix
description: Use before or after a release, or after changing packaging (rpm specs, deb control files, the release workflow), to check that a release's .rpm and .deb packages install, work and remove cleanly on Fedora, Ubuntu and Debian at once, in throwaway containers on the Incus lab host (lab/packaging/).
---

# The packaging matrix

Read AGENTS.md, "Testing on Erebus", first: its rules apply here too.

```sh
lab/packaging/matrix.sh                    # the latest release, every distribution
lab/packaging/matrix.sh v1.0.0 fedora/44   # one tag, chosen distributions
```

- **One row per distribution,** each from an ephemeral container named `ind-pkg-<distribution>`, deleted at the
  end. The exit code fails when any row fails.
- **What is checked** is decided per distribution by `install-test.sh`, which says so in its header:
  - the daemon and CLI package everywhere: installs, `--version` matches the package, the user unit is in place, and
    it removes cleanly
  - the GNOME and KDE apps where their packages fit (Fedora 44 rpm, Ubuntu 26.04 deb): they install beside the
    daemon package and start
- **With the test server up** (`lab/immich-server/.env.local` present, LAN mode, see the `lab-immich-server` skill),
  each container also signs in with the read-only key, mounts as root, and checks originals read through the mount
  against Immich's own checksums. Bring the server up first when a change touches reading or mounting.
- **Releases come from the repository's GitHub releases** through `gh`, so the tag must be published. Downloads go
  to a temporary directory that is removed afterwards.

When a row fails, rerun just that distribution, then look inside a kept container by running `install-test.sh` by
hand in one launched without `--ephemeral` (and delete it after).

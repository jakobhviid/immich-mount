# Security

## Reporting a vulnerability

Please report security problems privately, not in a public issue: use **Report a vulnerability** on the
repository's Security tab, or email jakob@hviid.phd. Say what you found, how to reproduce it, and which version you
ran (`immich-mount --version`). Reports are answered as soon as possible, fixed in a new release, and credited in
it if you want.

Only the latest release gets fixes.

Problems in the Immich server itself belong to the Immich project:
[its security policy](https://github.com/immich-app/immich/security/policy).

## What Immich Mount holds

- **The credential** for your Immich server: a session token or an API key, never your password. On a desktop it
  is kept in the keyring (GNOME Keyring or KWallet). Without one it is a file readable only by you
  (`~/.config/immich-mount/credential`, mode 0600), which the daemon refuses if anyone else can read it. It never
  appears in logs.
- **Copies of your photos:** server thumbnails and the parts of originals that were read, in your cache directory,
  and thumbnails in the desktop's thumbnail cache. `immich-mount logout` removes them. Files that could not be
  uploaded are kept in `~/.local/share/immich-mount/not-uploaded/`; they are yours, and `logout` leaves them.
- **Its network use:** it talks only to the Immich server you sign in to.
- **What it changes on the server:** it uploads files you copy into its Upload folder, and signing out ends its own session.
  It never edits, moves or deletes what is already in your library.

SPEC.md, "Files on disk", lists every file it writes.

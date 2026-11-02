# PRINCIPLES

The invariants Immich Mount upholds, and where known, what earned them. A change that would break one is discussed
first, and ARCHITECTURE.md is updated with the reason.

1. **Never hang the desktop.** A stuck FUSE call freezes Files, every file dialog, and any shell that touches the
   mount. Every filesystem operation answers within a bounded time, from cache or with a fast error. Nothing waits on
   the network without a deadline. The upload on close is the one deliberate wait, and it has its own limit.

2. **Never download what nobody opened.** Listings, thumbnails, content sniffing and indexing never pull originals.
   The library is never mirrored or synced (libraries run to 300 GB and more). The only originals kept beyond the
   cache are albums the user explicitly pins.
   *Earned by:* the GNOME spike, where opening one folder made Nautilus read every original in it to the end,
   visible or not.

3. **Originals stay byte-identical, both ways.** No transcoding, resizing, metadata stripping or date rewriting.
   No program is ever handed different bytes for the same path.

4. **Immich is the source of truth.** The mount is a view with a cache, not a sync client. Folder membership is what
   Immich says, never recomputed from dates. Existing Assets cannot be edited, renamed, moved or deleted through the
   mount; additions go only through Photos.
   *Earned by:* the month-boundary case, where search (UTC) and the timeline (local time) disagree.

5. **Mutations are explicit and honest.** An outcome that cannot be confirmed is reported as uncertain and checked,
   never replayed blindly.

6. **A move never loses its source early.** A file moved into Photos is uploaded before `close()` returns
   success, so the source is removed only after the server has it.

7. **Least privilege where the user wants it.** An API key with read-only permissions is enough to browse and open
   files. Uploads need the server's permission, and the app's own switches gate them.

8. **A path means one thing.** As long as an Appearance exists, its path refers to the same Asset. Name clashes are
   never resolved by moving a name onto a different Asset.

9. **Native on every platform.** Each edition looks, reads and behaves like a first-party app of its platform, and
   integrates through the platform's own mechanisms. On GNOME:
   - the GNOME HIG and libadwaita
   - the Files sidebar, file dialogs and the thumbnail cache
   - the keyring, notifications, and the system browser for sign-in

   KDE and macOS follow their own guidelines the same way. Platform conventions win over consistency between
   editions. There is no cross-platform UI toolkit, no custom theming, and no second file manager.

10. **Measure, don't guess.** Behaviour that depends on Files, GLib, the kernel or the Immich server is verified on
    real machines (Erebus) before a design rests on it. Unverified claims are marked as such.

11. **Immich comes first.** This is a feature for Immich, not a brand. The file manager says "Immich", and our own
    interface says "Immich Mount". It never pretends to be official: the non-affiliation notice is always shown.

12. **Credit where credit is due.** Immich Mount is inspired by
    [Kartax/immich-desktop-app](https://github.com/Kartax/immich-desktop-app), and its API layer started from
    [immich-rs](https://github.com/boul2gom/immich-rs). The README's credits section and CREDITS.md stay true.

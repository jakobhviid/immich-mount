# DESIGN.md

How Immich Mount looks: one identity, drawn natively on each platform. This guide says how to use the icons, the
colours, the type, the website and the screenshots. ARCHITECTURE.md, "The icon", keeps the reasons behind each icon
and how it is built; the platforms' own guidelines (GNOME's, KDE's, Apple's) govern everything else.

## Principles

1. **Native first.** Each edition looks like its platform's own apps: libadwaita on GNOME, Kirigami and Breeze on
   KDE Plasma, AppKit and SwiftUI on the Mac. The identity comes from the icon, never from restyling the platform's
   controls, colours or fonts.
2. **The lens is the brand, the body is the platform.** Every icon shares the lens and its ring of five segments;
   the shape around it follows the platform: a wide drive on Linux, the drive filling Apple's rounded square on the
   Mac.
3. **Immich's colours, never its shapes** (AGENTS.md, "Hard rules"). The five colours stay accents: the drive and
   the lens carry the identity, so nothing looks like an official Immich page or app.
4. **Say we are unofficial** wherever the project presents itself (SPEC.md, "The non-affiliation notice").
5. **Measured, not guessed:** contrast is checked against WCAG AA, and every surface works in light and dark.

## The icon family

| Where | What | Source |
|---|---|---|
| Linux: launchers, About, the apps' pages, Dolphin's place | The app icon: the wide drive, white face and grey base, the lens with its five-colour ring, the green status light. GNOME's app icon style: an object, with no tile behind it | `packaging/icons/dev.hviid.ImmichMount.svg`, hand-written SVG |
| Linux: notifications and the tray | The symbolic icon: the wide drive as one solid shape, the ring in five segments, its centre, the face's edge and the light cut out, the Mac menu bar icon's language in Linux's shape. Recoloured to the text by GTK and KDE | `packaging/icons/dev.hviid.ImmichMount-symbolic.svg` (`packaging/icons/make-symbolic.py`) |
| Mac: Dock, Finder, System Settings, About | The app icon: the drive filling Apple's rounded square. On macOS 26 and later an Icon Composer document, with the light, dark, tinted and clear appearances and a glass lens | `apple/App/AppIcon.icon` (`apple/make-icon-composer.py`); `AppIcon.appiconset` for macOS 14 and 15 |
| Mac: the menu bar | One solid shape, the ring, its centre, the face's edge and the light cut out, as menu bar icons are drawn | `MenuBarIcon.imageset` (`apple/make-menu-bar-icon.py`) |
| Mac: Finder's sidebar | The app icon in outline | `SidebarSymbol.symbolset` (`apple/make-symbol.py --square`) |
| Mac: the sign-in window | The drawing itself, over the title | `Drive.imageset` |
| The project as a whole: the website's bar and footer, the favicon | The mark: the lens alone, cut from the Linux icon's drawing | `site/mark.svg` |
| A shared link to the repository: GitHub's social preview | The mark in its ring on the hero's glass, the name, the tagline, both platforms with their icons, and the non-affiliation line | `site/social-preview.html`, rendered at 1280 by 640 as `site/social-preview.png` and uploaded in the repository's Settings |

**Why the Finder sidebar's symbol looks different:** Finder draws every location in its sidebar as a one-colour
outline that follows the sidebar's text colour, as iCloud Drive's, OneDrive's and Nextcloud's are. A coloured icon
there would stand out from its neighbours as foreign, so ours is the app icon traced as a symbol: the same rounded
square, the face's edge, the light, and the lens as a ring of five segments.

**The Mac icons are final:** they are native to the Mac, and changes to them need the maintainer.

Rules:
- **A platform's icon only where that platform is meant:** its download, its screenshots, its entry in a list of
  platforms. **The mark** where the project as a whole is meant.
- **Both platforms' icons side by side** where the whole project stands for both platforms' apps, as at the top
  of the README: Linux's drive, then the Mac's (right-aligned in the README, which floats the first one rightmost,
  so the Mac's comes first in its source). The Mac's carries Apple's margin, so it is drawn a little larger.
- **Never one platform's icon on another's desktop:** no rounded square on Linux, no wide drive as the Mac's app
  icon.
- **Change an icon at its source** and run its generator (`apple/make-icons.sh`); never edit exported PNGs.
- **The small icons follow the Mac's lead on Linux too:** the five-segment ring in one colour, in the wide drive's
  shape, for the notifications and the tray.

## Colours

The palette is the icon's own, by part:

| Token | Hex | Part of the icon |
|---|---|---|
| Aluminium 0 to 4 | `#ffffff`, `#f6f5f4`, `#deddda`, `#c0bfbc`, `#9a9996` | The drive's face and base |
| Lens rim, lens, lens deep | `#3d3846`, `#241f31`, `#0f0d16` | The lens barrel |
| Glass, glass mid, glass light | `#2d5c9e`, `#18294a`, `#62a0ea` | The lens glass (glass light only as a link on dark) |
| Status light | `#18c249` | The drive's light |
| The ring, clockwise from the top | `#fa2921`, `#ed79b5`, `#ffb400`, `#1e83f7`, `#18c249` | Immich's five colours |

Rules:
- **One action colour:** the status light, for the one main action of a view (Download), with lens-deep text.
  White on it reaches only 2.4:1.
- **The ring's colours together and in order** (red, pink, amber, blue, green, clockwise from the top), or one at
  a time as a small accent. Never as a background or as text.
- **Warm, not cool:** text in lens, not black; surfaces in aluminium, not cool grey.
- **The apps use none of this.** They take their toolkit's accent and colours; only the icon brings ours in.

Measured contrast on the website (WCAG AA asks 4.5:1 for text):

| Pair | Ratio |
|---|---|
| Light: text (lens) on aluminium 1 | 14.7 |
| Light: muted (`#5e5c64`) on aluminium 1 | 6.0 |
| Light: link (glass) on white | 6.7 |
| The button: lens deep on the status light | 8.1 |
| Dark: text (aluminium 1) on `#15131c` | 16.9 |
| Dark: muted (aluminium 3) on lens | 8.7 |
| Dark: link (glass light) on lens | 5.9 |
| Hero: text (aluminium 2) on glass mid | 10.6 |
| Hero: links (white) on glass | 6.7 |

## Type and words

- **The website** uses the system's fonts, as the apps do: `-apple-system, BlinkMacSystemFont, "Segoe UI",
  Cantarell, "Noto Sans", Ubuntu, sans-serif`, and `ui-monospace` for commands. Body 17 px at 1.55; the title 3 rem
  (2.3 on a phone), weight 700, tracked in by 0.03 em; section titles 1.8 rem.
- **The apps** use their platform's fonts and sizes, untouched.
- **The name** is Immich Mount, two words, in prose and as the drive's default name. "Immich" alone names the
  server only.
- **Menu items in the file managers** are **Open in Immich** and **Copy Immich Link**, the same in Files, Dolphin
  and Finder: "Immich" there is the server's web app, where the link goes. "Immich Crosslink" is the protocol's name,
  in English in every language: its switch in the settings reads **Immich Crosslink**, with **Read more** beside it,
  a link to the protocol's page whose address is not shown. A notification about a link another app sent says "a link
  from another app", not the protocol's name.
- **Buttons and titles in title case** in English, as GNOME's, KDE's and Apple's guidelines ask; labels and
  descriptions in sentence case. Translations follow their language's own conventions.
- **No em-dashes** anywhere (AGENTS.md).

## The website

`site/` (ARCHITECTURE.md, "The download page"):
- **The hero is the lens:** its glass as the ground in both appearances, and the mark large in the ring, for every
  visitor, as on the social preview. A platform's app icon in the ring was tried; a tile in a circle looked cramped
  beside the mark's round lens.
- **A section for each platform,** right after the opening: GNOME, KDE Plasma and macOS, each with its icon, one
  line on what is native there and three of its pictures, the visitor's platform first. Being native on each is the
  point, so the pictures are never one mixed gallery.
- **Each platform's icon beside its name:** the supported platforms under the title, the platform sections and the
  install cards. The visitor's card comes first, outlined in the status light, with "for your system".
- **The mark** in the hero's ring, the bar, the footer and the favicon.
- **Cards** are white (lens in dark) with a soft shadow; each feature card carries one segment of the ring, in the
  ring's order.
- **Screenshots** are rounded with a soft border, in their platform's section; the Mac's keep macOS's own window
  shadow and margin.
- **Nothing is loaded from elsewhere:** no web fonts, analytics or third-party scripts.
- **Before a change ships,** look at it built (`site/build-latest.sh`) in a browser at 1280 and 390 pixels wide, in
  light and dark, and as a Mac and a Linux visitor.

## Screenshots

- **Lab data only:** the lab server's credited fixtures, never a personal library (AGENTS.md, "Testing on Erebus").
  One exception, until the lab has a Mac: the Mac's pictures come from the maintainer's own library, with photos
  chosen without people and everything else personal pixelated (the server, the account, other locations in the
  sidebar). They are taken again against the lab server once a Mac build server is there (ROADMAP.md).
- **Linux** in the lab VMs, cut to the boxes the `lab-desktop-vm` skill lists. **The Mac** on the maintainer's Mac
  against the lab server, each window taken with ⌘⇧4 and Space, which keeps macOS's own shadow.
- **Names** say what they show: `files-album.png`, `kde-status.png`, `gnome-tray.png`, and `mac-` for the Mac's (`mac-finder.png`,
  `mac-menu.png`, `mac-sign-in.png`, `mac-settings.png`).
- **Retaken** when the interface they show changes; keep the file name, so the README and the page need no change.

## The apps' interfaces

What the Mac edition taught, for every edition (ARCHITECTURE.md, "The desktop app" and "The macOS edition"):
- **A window for what needs doing,** not a dashboard: signing in, then switching the location on, then it closes.
- **Status where it can be glanced at:** on the Mac the menu bar menu, its first line the state and then grey
  lines, each saying what it is ("Server version: ...", "Signed in as: ..."). On Linux the same menu from the tray
  icon, where the desktop has a tray (ARCHITECTURE.md, "The tray"), and the app's status page.
- **Settings in the platform's own preferences window,** in panes where the platform's settings windows have them
  (the Mac: General, Folders, Account), with an account pane where the platform's sync apps have one, generated from
  the core's settings table, showing only what means something on that platform.
- **Destructive actions ask first,** in the platform's own alert, with its destructive button style.
- **Windows come forward the platform's way:** on the Mac, where macOS refuses a menu bar app's activation, they
  float above other apps' windows until used.
- **About** carries the non-affiliation notice, the credits, the AI disclosure, the website and the source.

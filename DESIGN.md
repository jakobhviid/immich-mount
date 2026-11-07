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
| Linux: notifications | The symbolic icon, one colour | `packaging/icons/dev.hviid.ImmichMount-symbolic.svg` |
| Mac: Dock, Finder, System Settings, About | The app icon: the drive filling Apple's rounded square. On macOS 26 and later an Icon Composer document, with the light, dark, tinted and clear appearances and a glass lens | `apple/App/AppIcon.icon` (`apple/make-icon-composer.py`); `AppIcon.appiconset` for macOS 14 and 15 |
| Mac: the menu bar | One solid shape, the ring, its centre, the face's edge and the light cut out, as menu bar icons are drawn | `MenuBarIcon.imageset` (`apple/make-menu-bar-icon.py`) |
| Mac: Finder's sidebar | The app icon in outline | `SidebarSymbol.symbolset` (`apple/make-symbol.py --square`) |
| Mac: the sign-in window | The drawing itself, over the title | `Drive.imageset` |
| The project as a whole: the website's bar and footer, the favicon | The mark: the lens alone, cut from the Linux icon's drawing | `site/mark.svg` |

**Why the Finder sidebar's symbol looks different:** Finder draws every location in its sidebar as a one-colour
outline that follows the sidebar's text colour, as iCloud Drive's, OneDrive's and Nextcloud's are. A coloured icon
there would stand out from its neighbours as foreign, so ours is the app icon traced as a symbol: the same rounded
square, the face's edge, the light, and the lens as a ring of five segments.

**The Mac icons are final:** they are native to the Mac, and changes to them need the maintainer.

Rules:
- **A platform's icon only where that platform is meant:** its download, its screenshots, its entry in a list of
  platforms. **The mark** where the project as a whole is meant.
- **Never one platform's icon on another's desktop:** no rounded square on Linux, no wide drive as the Mac's app
  icon.
- **Change an icon at its source** and run its generator (`apple/make-icons.sh`); never edit exported PNGs.
- **The small icons follow the Mac's lead on Linux too:** the five-segment ring in one colour, in the wide drive's
  shape, for the symbolic icon and a future tray icon (ROADMAP.md).

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
- **Buttons and titles in title case** in English, as GNOME's, KDE's and Apple's guidelines ask; labels and
  descriptions in sentence case. Translations follow their language's own conventions.
- **No em-dashes** anywhere (AGENTS.md).

## The website

`site/` (ARCHITECTURE.md, "The download page"):
- **The hero is the lens:** its glass as the ground in both appearances, the ring large around the app icon at the
  centre.
- **The visitor's platform decides the icon:** a Mac visitor sees the Mac's icon in the hero, everyone else the
  Linux drive. Both platforms show side by side only where both are listed: the supported platforms under the
  title, each with its own icon, and the install section, each card headed by its platform's icon. The visitor's
  card comes first, outlined in the status light, with "for your system".
- **The mark** in the bar, the footer and the favicon.
- **Cards** are white (lens in dark) with a soft shadow; each feature card carries one segment of the ring, in the
  ring's order.
- **Screenshots** are rounded with a soft shadow, each one beside the platform it shows.
- **Nothing is loaded from elsewhere:** no web fonts, analytics or third-party scripts.
- **Before a change ships,** look at it built (`site/build-latest.sh`) in a browser at 1280 and 390 pixels wide, in
  light and dark, and as a Mac and a Linux visitor.

## Screenshots

- **Test data only:** the lab server's credited fixtures, never a personal library (AGENTS.md, "Testing on
  Erebus").
- **Linux** in the lab VMs, cut to the boxes the `lab-desktop-vm` skill lists. **The Mac** on the maintainer's Mac
  against the lab server, each window taken with ⌘⇧4 and Space, which keeps macOS's own shadow.
- **Names** say what they show: `files-album.png`, `kde-status.png`, and `mac-` for the Mac's (`mac-finder.png`,
  `mac-menu.png`, `mac-sign-in.png`, `mac-settings.png`).
- **Retaken** when the interface they show changes; keep the file name, so the README and the page need no change.

## The apps' interfaces

What the Mac edition taught, for every edition (ARCHITECTURE.md, "The desktop app" and "The macOS edition"):
- **A window for what needs doing,** not a dashboard: signing in, then switching the location on, then it closes.
- **Status where it can be glanced at:** on the Mac the menu bar menu, its first line the state and then grey
  lines, each saying what it is ("Server version: ...", "Signed in as: ..."). On Linux the app's status page, and a
  tray icon where the desktop has one (ROADMAP.md).
- **Settings in the platform's own preferences window,** generated from the core's settings table, showing only
  what means something on that platform.
- **Destructive actions ask first,** in the platform's own alert, with its destructive button style.
- **Windows come forward the platform's way:** on the Mac, where macOS refuses a menu bar app's activation, they
  float above other apps' windows until used.
- **About** carries the non-affiliation notice, the credits, the AI disclosure, the website and the source.

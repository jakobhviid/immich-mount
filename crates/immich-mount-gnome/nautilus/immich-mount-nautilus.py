# Open in Immich and Copy Immich Link in Files' menu, for Homebrew's cask (ROADMAP.md, "Open in Immich").
#
# The same as immich-mount-nautilus.c, for systems whose /usr is read-only (Bazzite): nautilus-python loads it from
# ~/.local/share/nautilus-python/extensions. The daemon decides: it knows the mount point and every item's Immich page,
# so this asks it (WebLink on D-Bus) and offers the two items only when it gives an address.

import gettext
import os
import shutil

import gi

# nautilus-python 4.1 has only the 4.1 typelib; older releases have 4.0.
for nautilus_version in ("4.1", "4.0"):
    try:
        gi.require_version("Nautilus", nautilus_version)
        break
    except ValueError:
        continue
gi.require_version("Gdk", "4.0")

from gi.repository import Gdk, Gio, GLib, GObject, Nautilus  # noqa: E402

TEXT_DOMAIN = "immich-mount"
BUS_NAME = "dev.hviid.ImmichMount.Daemon"
OBJECT_PATH = "/dev/hviid/ImmichMount/Daemon"
INTERFACE = "dev.hviid.ImmichMount.Daemon1"
# Files asks on every change of selection and waits for the answer, so a daemon busy listing a folder on a slow server
# must not hold it up.
WEB_LINK_TIMEOUT_MS = 250
# Where a package installs the C extension; with it there, it offers the items, and this offers none.
C_EXTENSION_FOLDERS = (
    "/usr/lib64/nautilus/extensions-4",
    "/usr/lib/nautilus/extensions-4",
    "/usr/lib/x86_64-linux-gnu/nautilus/extensions-4",
    "/usr/lib/aarch64-linux-gnu/nautilus/extensions-4",
)


def c_extension_installed():
    return any(os.path.exists(os.path.join(folder, "libimmich-mount-nautilus.so")) for folder in C_EXTENSION_FOLDERS)


def locale_folder():
    """The cask's catalogs, in share/locale beside the program that Homebrew's bin links to; None for the system's."""
    search_path = os.pathsep.join(filter(None, [os.environ.get("PATH"), "/home/linuxbrew/.linuxbrew/bin"]))
    program = shutil.which("immich-mount", path=search_path)
    if program is None:
        return None
    folder = os.path.join(os.path.dirname(os.path.realpath(program)), "share", "locale")
    return folder if os.path.isdir(folder) else None


translate = gettext.translation(TEXT_DOMAIN, localedir=locale_folder(), fallback=True).gettext


def web_link_for_path(path):
    """The daemon's web address for a local path; None when it has none, the path is outside the mount, or no daemon
    answers in time."""
    try:
        connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        # No auto start: right-clicking a file elsewhere must never start the daemon.
        reply = connection.call_sync(BUS_NAME, OBJECT_PATH, INTERFACE, "WebLink", GLib.Variant("(s)", (path,)),
                                     GLib.VariantType.new("(s)"), Gio.DBusCallFlags.NO_AUTO_START,
                                     WEB_LINK_TIMEOUT_MS, None)
    except (GLib.Error, TypeError, UnicodeError):
        return None
    url = reply.unpack()[0]
    return url or None


def open_in_immich(item, url):
    # GDK's launch context passes an activation token, so Wayland lets the browser come to the front.
    context = Gdk.Display.get_default().get_app_launch_context()
    try:
        Gio.AppInfo.launch_default_for_uri(url, context)
    except GLib.Error as error:
        print(f"Immich Mount: could not open {url}: {error.message}")


def copy_immich_link(item, url):
    Gdk.Display.get_default().get_clipboard().set_content(Gdk.ContentProvider.new_for_value(url))


class ImmichMenuProvider(GObject.GObject, Nautilus.MenuProvider):
    def __init__(self):
        super().__init__()
        self.offers_items = not c_extension_installed()

    def get_file_items(self, *arguments):
        # Nautilus 4 passes the files alone; older bindings passed the window first.
        files = arguments[-1]
        if not self.offers_items or len(files) != 1 or files[0].get_uri_scheme() != "file":
            return []
        path = files[0].get_location().get_path()
        url = web_link_for_path(path) if path else None
        if url is None:
            return []
        open_item = Nautilus.MenuItem(name="ImmichMount::OpenInImmich", label=translate("Open in Immich"))
        open_item.connect("activate", open_in_immich, url)
        copy_item = Nautilus.MenuItem(name="ImmichMount::CopyImmichLink", label=translate("Copy Immich Link"))
        copy_item.connect("activate", copy_immich_link, url)
        return [open_item, copy_item]

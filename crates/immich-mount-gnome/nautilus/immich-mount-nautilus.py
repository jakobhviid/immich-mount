# Open in Immich, Copy Immich Link and Show in *app* in Files' menu, for Homebrew's cask (ROADMAP.md, "Open in Immich,
# and links between apps").
#
# The same as immich-mount-nautilus.c, for systems whose /usr is read-only (Bazzite): nautilus-python loads it from
# ~/.local/share/nautilus-python/extensions. The daemon decides: it knows the mount point and every item's Immich page,
# so this asks it (Links on D-Bus) and offers the items only when it gives an address. The other apps are found as
# immich-crosslink says: registered for its scheme, announcing draft 0.2 and the item's kind.

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
CROSSLINK_TYPE = "x-scheme-handler/immich-crosslink"
CROSSLINK_VERSION = "0.2"
# Immich Mount's own launcher entry takes the scheme too, and never offers itself.
OWN_DESKTOP_ID = "dev.hviid.ImmichMount.desktop"
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


def call_daemon(connection, method, path, reply_type):
    # No auto start: right-clicking a file elsewhere must never start the daemon.
    return connection.call_sync(BUS_NAME, OBJECT_PATH, INTERFACE, method, GLib.Variant("(s)", (path,)),
                                GLib.VariantType.new(reply_type), Gio.DBusCallFlags.NO_AUTO_START,
                                WEB_LINK_TIMEOUT_MS, None).unpack()


def links_for_path(path):
    """The daemon's (web address, crosslink, kind, hidden apps) for a local path; None when the item has neither a
    web address nor a crosslink, the path is outside the mount, or no daemon answers in time. The crosslink is empty while Immich Crosslink
    is off; a daemon from before Links gives the web address alone."""
    try:
        connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        try:
            web, crosslink, kind, hidden = call_daemon(connection, "Links", path, "(ssss)")
        except GLib.Error as error:
            if not error.matches(Gio.DBusError.quark(), Gio.DBusError.UNKNOWN_METHOD):
                return None
            (web,) = call_daemon(connection, "WebLink", path, "(s)")
            crosslink, kind, hidden = "", "", ""
    except (GLib.Error, TypeError, UnicodeError):
        return None
    # Outside the mount, or an item without a page, gives neither; with web.links off the web address alone is empty.
    return (web, crosslink, kind, hidden) if web or crosslink else None


def apps_showing(kind, hidden):
    """The other apps that show items of `kind` and that the user has not left out: registered for the scheme, and
    announcing exactly this draft in their desktop entry."""
    hidden_ids = set(hidden.split(";"))
    found = []
    for app in Gio.AppInfo.get_all_for_type(CROSSLINK_TYPE):
        app_id = app.get_id()
        if not isinstance(app, Gio.DesktopAppInfo) or not app_id or app_id == OWN_DESKTOP_ID or app_id in hidden_ids:
            continue
        version = app.get_string("X-Immich-Crosslink-Version")
        shows = app.get_string_list("X-Immich-Crosslink-Shows") or []
        if version == CROSSLINK_VERSION and kind in shows:
            found.append(app)
    return found


def open_in_immich(item, url):
    # GDK's launch context passes an activation token, so Wayland lets the browser come to the front.
    context = Gdk.Display.get_default().get_app_launch_context()
    try:
        Gio.AppInfo.launch_default_for_uri(url, context)
    except GLib.Error as error:
        print(f"Immich Mount: could not open {url}: {error.message}")


def copy_immich_link(item, url):
    Gdk.Display.get_default().get_clipboard().set_content(Gdk.ContentProvider.new_for_value(url))


def show_in_app(item, app, crosslink):
    context = Gdk.Display.get_default().get_app_launch_context()
    # To this app alone: the scheme's default handler could be any of them (immich-crosslink, "Sending").
    try:
        app.launch_uris([crosslink], context)
    except GLib.Error as error:
        print(f"Immich Mount: could not show {crosslink} in {app.get_name()}: {error.message}")


def show_in_item(app, crosslink):
    icon = app.get_icon()
    item = Nautilus.MenuItem(name=f"ImmichMount::ShowIn::{app.get_id()}",
                             label=translate("Show in {app}").replace("{app}", app.get_name()),
                             icon=icon.to_string() if icon else None)
    item.connect("activate", show_in_app, app, crosslink)
    return item


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
        links = links_for_path(path) if path else None
        if links is None:
            return []
        web, crosslink, kind, hidden = links
        items = []
        # The web address is empty while the setting Open in Immich and Copy Immich Link is off.
        if web:
            open_item = Nautilus.MenuItem(name="ImmichMount::OpenInImmich", label=translate("Open in Immich"))
            open_item.connect("activate", open_in_immich, web)
            copy_item = Nautilus.MenuItem(name="ImmichMount::CopyImmichLink", label=translate("Copy Immich Link"))
            copy_item.connect("activate", copy_immich_link, web)
            items += [open_item, copy_item]
        # The crosslink is empty while Immich Crosslink is off, and then no other app is offered.
        if crosslink:
            items += [show_in_item(app, crosslink) for app in apps_showing(kind, hidden)]
        return items

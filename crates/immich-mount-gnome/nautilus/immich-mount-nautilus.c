/* Open in Immich and Copy Immich Link in Files' menu, for one item in the mount (ROADMAP.md, "Open in Immich").
 *
 * The daemon decides: it knows the mount point and every item's Immich page, so this asks it (WebLink on D-Bus) and
 * offers the two items only when it gives an address. A daemon that is not running, or slow, gives no items. */

#include <libintl.h>

#include <gio/gio.h>
#include <gtk/gtk.h>
#include <nautilus-extension.h>

#ifndef LOCALEDIR
#define LOCALEDIR "/usr/share/locale"
#endif

#define TEXT_DOMAIN "immich-mount"
#define BUS_NAME "dev.hviid.ImmichMount.Daemon"
#define OBJECT_PATH "/dev/hviid/ImmichMount/Daemon"
#define INTERFACE "dev.hviid.ImmichMount.Daemon1"
/* The menu waits for the answer, so a daemon busy listing a folder on a slow server must not hold it up. */
#define WEB_LINK_TIMEOUT_MS 250

typedef struct {
    GObject parent;
} ImmichMenuProvider;

typedef struct {
    GObjectClass parent_class;
} ImmichMenuProviderClass;

static void immich_menu_provider_iface_init(NautilusMenuProviderInterface *iface);

G_DEFINE_DYNAMIC_TYPE_EXTENDED(ImmichMenuProvider, immich_menu_provider, G_TYPE_OBJECT, 0,
                               G_IMPLEMENT_INTERFACE_DYNAMIC(NAUTILUS_TYPE_MENU_PROVIDER,
                                                             immich_menu_provider_iface_init))

static GType provider_types[1];

/* The daemon's web address for a local path; NULL when it has none, the path is outside the mount, or no daemon
 * answers in time. */
static char *web_link_for_path(const char *path) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (connection == NULL) {
        return NULL;
    }
    /* No auto start: right-clicking a file elsewhere must never start the daemon. */
    g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
        connection, BUS_NAME, OBJECT_PATH, INTERFACE, "WebLink", g_variant_new("(s)", path), G_VARIANT_TYPE("(s)"),
        G_DBUS_CALL_FLAGS_NO_AUTO_START, WEB_LINK_TIMEOUT_MS, NULL, &error);
    if (reply == NULL) {
        return NULL;
    }
    const char *url = NULL;
    g_variant_get(reply, "(&s)", &url);
    return url[0] == '\0' ? NULL : g_strdup(url);
}

static void open_in_immich(G_GNUC_UNUSED NautilusMenuItem *item, gpointer user_data) {
    const char *url = user_data;
    g_autoptr(GError) error = NULL;
    /* GDK's launch context passes an activation token, so Wayland lets the browser come to the front. */
    g_autoptr(GdkAppLaunchContext) context = gdk_display_get_app_launch_context(gdk_display_get_default());
    if (!g_app_info_launch_default_for_uri(url, G_APP_LAUNCH_CONTEXT(context), &error)) {
        g_warning("Immich Mount: could not open %s: %s", url, error->message);
    }
}

static void copy_immich_link(G_GNUC_UNUSED NautilusMenuItem *item, gpointer user_data) {
    gdk_clipboard_set_text(gdk_display_get_clipboard(gdk_display_get_default()), user_data);
}

static NautilusMenuItem *menu_item(const char *name, const char *label, GCallback on_activate, const char *url) {
    NautilusMenuItem *item = nautilus_menu_item_new(name, label, NULL, NULL);
    /* Each item holds its own copy of the address, freed with the item. */
    g_signal_connect_data(item, "activate", on_activate, g_strdup(url), (GClosureNotify)(void (*)(void))g_free, 0);
    return item;
}

static GList *get_file_items(G_GNUC_UNUSED NautilusMenuProvider *provider, GList *files) {
    if (files == NULL || files->next != NULL) {
        return NULL;
    }
    NautilusFileInfo *file = files->data;
    g_autofree char *scheme = nautilus_file_info_get_uri_scheme(file);
    if (g_strcmp0(scheme, "file") != 0) {
        return NULL;
    }
    g_autoptr(GFile) location = nautilus_file_info_get_location(file);
    g_autofree char *path = g_file_get_path(location);
    /* A D-Bus string is UTF-8, and the mount only ever has UTF-8 names. */
    if (path == NULL || !g_utf8_validate(path, -1, NULL)) {
        return NULL;
    }
    g_autofree char *url = web_link_for_path(path);
    if (url == NULL) {
        return NULL;
    }
    GList *items = NULL;
    items = g_list_append(items, menu_item("ImmichMount::OpenInImmich", dgettext(TEXT_DOMAIN, "Open in Immich"),
                                           G_CALLBACK(open_in_immich), url));
    items = g_list_append(items, menu_item("ImmichMount::CopyImmichLink", dgettext(TEXT_DOMAIN, "Copy Immich Link"),
                                           G_CALLBACK(copy_immich_link), url));
    return items;
}

static GList *get_background_items(G_GNUC_UNUSED NautilusMenuProvider *provider,
                                   G_GNUC_UNUSED NautilusFileInfo *current_folder) {
    return NULL;
}

static void immich_menu_provider_iface_init(NautilusMenuProviderInterface *iface) {
    iface->get_file_items = get_file_items;
    iface->get_background_items = get_background_items;
}

static void immich_menu_provider_init(G_GNUC_UNUSED ImmichMenuProvider *provider) {}

static void immich_menu_provider_class_init(G_GNUC_UNUSED ImmichMenuProviderClass *class) {}

static void immich_menu_provider_class_finalize(G_GNUC_UNUSED ImmichMenuProviderClass *class) {}

G_MODULE_EXPORT void nautilus_module_initialize(GTypeModule *module) {
    /* The catalogs are the daemon's and the apps', installed with the immich-mount package. */
    bindtextdomain(TEXT_DOMAIN, LOCALEDIR);
    bind_textdomain_codeset(TEXT_DOMAIN, "UTF-8");
    immich_menu_provider_register_type(module);
    provider_types[0] = immich_menu_provider_get_type();
}

G_MODULE_EXPORT void nautilus_module_shutdown(void) {}

G_MODULE_EXPORT void nautilus_module_list_types(const GType **types, int *num_types) {
    *types = provider_types;
    *num_types = G_N_ELEMENTS(provider_types);
}

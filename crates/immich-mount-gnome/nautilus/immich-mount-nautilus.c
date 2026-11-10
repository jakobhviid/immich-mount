/* Open in Immich, Copy Immich Link and Show in *app* in Files' menu, for one item in the mount (ROADMAP.md, "Open in
 * Immich, and links between apps").
 *
 * The daemon decides: it knows the mount point and every item's Immich page, so this asks it (Links on D-Bus) and
 * offers the items only when it gives an address. A daemon that is not running, or slow, gives no items. The other
 * apps are found as immich-crosslink says: registered for its scheme, announcing draft 0.2 and the item's kind. */

#include <libintl.h>

#include <gio/gio.h>
#include <gio/gdesktopappinfo.h>
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
#define CROSSLINK_TYPE "x-scheme-handler/immich-crosslink"
#define CROSSLINK_VERSION "0.2"
/* Immich Mount's own launcher entry takes the scheme too, and never offers itself. */
#define OWN_DESKTOP_ID "dev.hviid.ImmichMount.desktop"
#define NOTIFICATION_ICON "dev.hviid.ImmichMount-symbolic"

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
/* The one provider Nautilus makes, to have it ask for the items again after a failed Show in. */
static NautilusMenuProvider *the_provider;

/* What the daemon says about an item: its web address, its crosslink (empty while Immich Crosslink is off), its kind,
 * and the apps the user left out, separated by `;`. */
typedef struct {
    char *web;
    char *crosslink;
    char *kind;
    char *hidden;
} ItemLinks;

static void item_links_free(ItemLinks *links) {
    g_free(links->web);
    g_free(links->crosslink);
    g_free(links->kind);
    g_free(links->hidden);
    g_free(links);
}

G_DEFINE_AUTOPTR_CLEANUP_FUNC(ItemLinks, item_links_free)

static GVariant *call_daemon(GDBusConnection *connection, const char *method, const char *path,
                             const char *reply_type, GError **error) {
    /* No auto start: right-clicking a file elsewhere must never start the daemon. */
    return g_dbus_connection_call_sync(connection, BUS_NAME, OBJECT_PATH, INTERFACE, method,
                                       g_variant_new("(s)", path), G_VARIANT_TYPE(reply_type),
                                       G_DBUS_CALL_FLAGS_NO_AUTO_START, WEB_LINK_TIMEOUT_MS, NULL, error);
}

/* The daemon's links for a local path; NULL when the item has neither a web address nor a crosslink, the path is
 * outside the mount, or no daemon answers in time. A daemon from before Links gives the web address alone. */
static ItemLinks *links_for_path(const char *path) {
    g_autoptr(GError) error = NULL;
    g_autoptr(GDBusConnection) connection = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, &error);
    if (connection == NULL) {
        return NULL;
    }
    ItemLinks *links = g_new0(ItemLinks, 1);
    g_autoptr(GVariant) reply = call_daemon(connection, "Links", path, "(ssss)", &error);
    if (reply != NULL) {
        g_variant_get(reply, "(ssss)", &links->web, &links->crosslink, &links->kind, &links->hidden);
    } else if (g_error_matches(error, G_DBUS_ERROR, G_DBUS_ERROR_UNKNOWN_METHOD)) {
        g_clear_error(&error);
        g_autoptr(GVariant) web_reply = call_daemon(connection, "WebLink", path, "(s)", &error);
        if (web_reply != NULL) {
            g_variant_get(web_reply, "(s)", &links->web);
        }
    }
    /* Outside the mount, or an item without a page, gives neither; with web.links off the web address alone is empty. */
    if ((links->web == NULL || links->web[0] == '\0') && (links->crosslink == NULL || links->crosslink[0] == '\0')) {
        item_links_free(links);
        return NULL;
    }
    return links;
}

/* Whether `list`, the entries of a `;`-separated value, holds `wanted`. */
static gboolean list_contains(char **list, const char *wanted) {
    return list != NULL && g_strv_contains((const char *const *)list, wanted);
}

/* Whether the app's program is there to start; one that is not is left out, silently (immich-crosslink, "Sending"). */
static gboolean can_start(GAppInfo *app) {
    const char *executable = g_app_info_get_executable(app);
    g_autofree char *program = executable != NULL ? g_find_program_in_path(executable) : NULL;
    return program != NULL;
}

/* The other apps that show items of `kind` and that the user has not left out: registered for the scheme, announcing
 * exactly this draft in their desktop entry, and able to start. */
static GList *apps_showing(const char *kind, const char *hidden) {
    g_auto(GStrv) hidden_ids = g_strsplit(hidden, ";", -1);
    GList *found = NULL;
    GList *candidates = g_app_info_get_all_for_type(CROSSLINK_TYPE);
    for (GList *entry = candidates; entry != NULL; entry = entry->next) {
        GAppInfo *app = entry->data;
        const char *id = g_app_info_get_id(app);
        if (!G_IS_DESKTOP_APP_INFO(app) || id == NULL || g_strcmp0(id, OWN_DESKTOP_ID) == 0 ||
            list_contains(hidden_ids, id)) {
            continue;
        }
        GDesktopAppInfo *desktop = G_DESKTOP_APP_INFO(app);
        g_autofree char *version = g_desktop_app_info_get_string(desktop, "X-Immich-Crosslink-Version");
        g_auto(GStrv) shows = g_desktop_app_info_get_string_list(desktop, "X-Immich-Crosslink-Shows", NULL);
        if (g_strcmp0(version, CROSSLINK_VERSION) == 0 && list_contains(shows, kind) && can_start(app)) {
            found = g_list_append(found, g_object_ref(app));
        }
    }
    g_list_free_full(candidates, g_object_unref);
    return found;
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

/* An app to show an item in, and the item's crosslink. */
typedef struct {
    GAppInfo *app;
    char *crosslink;
} ShowIn;

static void show_in_free(gpointer data, G_GNUC_UNUSED GClosure *closure) {
    ShowIn *show_in = data;
    g_object_unref(show_in->app);
    g_free(show_in->crosslink);
    g_free(show_in);
}

/* A desktop notification titled with the protocol's name, which stays in English, as Immich Mount's own. */
static void notify(const char *body) {
    g_autoptr(GDBusConnection) connection = g_bus_get_sync(G_BUS_TYPE_SESSION, NULL, NULL);
    if (connection == NULL) {
        return;
    }
    GVariant *parameters = g_variant_new("(susss@as@a{sv}i)", "Immich Mount", 0, NOTIFICATION_ICON, "Immich Crosslink",
                                         body, g_variant_new_strv(NULL, 0),
                                         g_variant_new_array(G_VARIANT_TYPE("{sv}"), NULL, 0), -1);
    g_dbus_connection_call(connection, "org.freedesktop.Notifications", "/org/freedesktop/Notifications",
                           "org.freedesktop.Notifications", "Notify", parameters, NULL, G_DBUS_CALL_FLAGS_NONE, -1,
                           NULL, NULL, NULL);
}

/* Say why Show in did nothing, and have Files read the apps again: it keeps a selection's items until it changes. */
static void show_in_failed(const char *body) {
    notify(body);
    if (the_provider != NULL) {
        nautilus_menu_provider_emit_items_updated_signal(the_provider);
    }
}

/* `msgid` in the user's language, with `{name}` replaced by `value` after the lookup. */
static char *translated(const char *msgid, const char *name, const char *value) {
    g_autoptr(GString) text = g_string_new(dgettext(TEXT_DOMAIN, msgid));
    g_string_replace(text, name, value, 0);
    return g_string_free(g_steal_pointer(&text), FALSE);
}

static void show_in_app(G_GNUC_UNUSED NautilusMenuItem *item, gpointer user_data) {
    ShowIn *show_in = user_data;
    const char *app_name = g_app_info_get_name(show_in->app);
    /* The menu may be older than the app's removal: an app gone since says so, rather than starting from memory. */
    const char *entry = g_desktop_app_info_get_filename(G_DESKTOP_APP_INFO(show_in->app));
    if (entry != NULL && !g_file_test(entry, G_FILE_TEST_EXISTS)) {
        g_autofree char *body = translated("{app} is no longer installed.", "{app}", app_name);
        show_in_failed(body);
        return;
    }
    g_autoptr(GError) error = NULL;
    g_autoptr(GdkAppLaunchContext) context = gdk_display_get_app_launch_context(gdk_display_get_default());
    GList uris = {.data = show_in->crosslink};
    /* To this app alone: the scheme's default handler could be any of them (immich-crosslink, "Sending"). */
    if (!g_app_info_launch_uris(show_in->app, &uris, G_APP_LAUNCH_CONTEXT(context), &error)) {
        g_autofree char *with_app = translated("Immich Mount could not open {app}: {error}", "{app}", app_name);
        g_autoptr(GString) body = g_string_new(with_app);
        g_string_replace(body, "{error}", error->message, 0);
        show_in_failed(body->str);
    }
}

static NautilusMenuItem *show_in_item(GAppInfo *app, const char *crosslink) {
    g_autofree char *name = g_strconcat("ImmichMount::ShowIn::", g_app_info_get_id(app), NULL);
    g_autoptr(GString) label = g_string_new(dgettext(TEXT_DOMAIN, "Show in {app}"));
    g_string_replace(label, "{app}", g_app_info_get_name(app), 0);
    GIcon *icon = g_app_info_get_icon(app);
    g_autofree char *icon_name = icon != NULL ? g_icon_to_string(icon) : NULL;
    NautilusMenuItem *item = nautilus_menu_item_new(name, label->str, NULL, icon_name);
    ShowIn *show_in = g_new0(ShowIn, 1);
    show_in->app = g_object_ref(app);
    show_in->crosslink = g_strdup(crosslink);
    g_signal_connect_data(item, "activate", G_CALLBACK(show_in_app), show_in, show_in_free, 0);
    return item;
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
    g_autoptr(ItemLinks) links = links_for_path(path);
    if (links == NULL) {
        return NULL;
    }
    GList *items = NULL;
    /* The web address is empty while the setting Open in Immich and Copy Immich Link is off. */
    if (links->web != NULL && links->web[0] != '\0') {
        items = g_list_append(items, menu_item("ImmichMount::OpenInImmich", dgettext(TEXT_DOMAIN, "Open in Immich"),
                                               G_CALLBACK(open_in_immich), links->web));
        items = g_list_append(items, menu_item("ImmichMount::CopyImmichLink",
                                               dgettext(TEXT_DOMAIN, "Copy Immich Link"), G_CALLBACK(copy_immich_link),
                                               links->web));
    }
    /* The crosslink is empty while Immich Crosslink is off, and then no other app is offered. */
    if (links->crosslink != NULL && links->crosslink[0] != '\0') {
        GList *apps = apps_showing(links->kind, links->hidden);
        for (GList *entry = apps; entry != NULL; entry = entry->next) {
            items = g_list_append(items, show_in_item(entry->data, links->crosslink));
        }
        g_list_free_full(apps, g_object_unref);
    }
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

static void immich_menu_provider_init(ImmichMenuProvider *provider) {
    the_provider = NAUTILUS_MENU_PROVIDER(provider);
}

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

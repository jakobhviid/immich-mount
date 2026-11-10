/* Open in Immich, Copy Immich Link and Show in *app* in Dolphin's menu, for one item in the mount (ROADMAP.md, "Open in
 * Immich" and "Show in other apps").
 *
 * A file item action plugin, since KF6 has no service menu key that asks anyone about the selection
 * (X-KDE-ShowIfDBusCall went with KF5). The daemon decides: it knows the mount point and every item's Immich page, so
 * this asks it (Links on D-Bus) and offers the items only when it gives an address. A daemon that is not running, or
 * slow, gives no items. The other apps come from the desktop entries registered for immich-crosslink, as the protocol
 * says (https://jakob.hviid.phd/immich-crosslink/, "Finding participants"). */

#include <KAbstractFileItemActionPlugin>
#include <KApplicationTrader>
#include <KDesktopFile>
#include <KConfigGroup>
#include <KFileItem>
#include <KFileItemListProperties>
#include <KIO/ApplicationLauncherJob>
#include <KIO/JobUiDelegateFactory>
#include <KIO/OpenUrlJob>
#include <KLocalizedString>
#include <KPluginFactory>
#include <KPluginMetaData>

#include <QAction>
#include <QClipboard>
#include <QDBusConnection>
#include <QDBusError>
#include <QDBusMessage>
#include <QDir>
#include <QFileInfo>
#include <QGuiApplication>
#include <QIcon>
#include <QUrl>
#include <QWidget>

namespace
{
const QString busName = QStringLiteral("dev.hviid.ImmichMount.Daemon");
const QString objectPath = QStringLiteral("/dev/hviid/ImmichMount/Daemon");
const QString interfaceName = QStringLiteral("dev.hviid.ImmichMount.Daemon1");
// The menu waits for the answer, so a daemon busy listing a folder on a slow server must not hold it up.
constexpr int linksTimeoutMs = 250;

// An item's links, as the daemon's Links gives them.
struct Links {
    QString web;
    // Empty while Immich crosslink is off.
    QString crosslink;
    QString kind;
    // The desktop entry ids of the apps left out of Show in, separated by `;`.
    QString hidden;
};

QDBusMessage callDaemon(const QString &method, const QString &path)
{
    QDBusMessage message = QDBusMessage::createMethodCall(busName, objectPath, interfaceName, method);
    message << path;
    // No auto start: right-clicking a file elsewhere must never start the daemon.
    message.setAutoStartService(false);
    return QDBusConnection::sessionBus().call(message, QDBus::Block, linksTimeoutMs);
}

// The links of the item at a local path; all empty when it has none, the path is outside the mount, or no daemon
// answers in time. A daemon from before Links gives the web address alone.
Links linksForPath(const QString &path)
{
    const QDBusMessage reply = callDaemon(QStringLiteral("Links"), path);
    if (reply.type() == QDBusMessage::ReplyMessage && reply.arguments().size() == 4) {
        const QVariantList values = reply.arguments();
        return {values.at(0).toString(), values.at(1).toString(), values.at(2).toString(), values.at(3).toString()};
    }
    if (reply.type() == QDBusMessage::ErrorMessage && reply.errorName() == QDBusError::errorString(QDBusError::UnknownMethod)) {
        const QDBusMessage web = callDaemon(QStringLiteral("WebLink"), path);
        if (web.type() == QDBusMessage::ReplyMessage && web.arguments().size() == 1) {
            return {web.arguments().constFirst().toString(), {}, {}, {}};
        }
    }
    return {};
}

// The other apps that show items of `kind`: registered for the scheme, announcing the draft this sends, listing the
// kind, and not left out by the user. Never Immich Mount itself.
KService::List appsShowing(const QString &kind, const QString &hidden)
{
    const QStringList hiddenIds = hidden.split(QLatin1Char(';'), Qt::SkipEmptyParts);
    return KApplicationTrader::queryByMimeType(QStringLiteral("x-scheme-handler/immich-crosslink"), [&](const KService::Ptr &app) {
        const QString id = app->storageId();
        if (id == QLatin1String("dev.hviid.ImmichMount.desktop") || hiddenIds.contains(id)) {
            return false;
        }
        // The announcement's keys are read from the entry itself, which every KF6 has.
        const KDesktopFile entry(app->entryPath());
        const KConfigGroup group = entry.desktopGroup();
        const QStringList shows = group.readEntry("X-Immich-Crosslink-Shows").split(QLatin1Char(';'), Qt::SkipEmptyParts);
        return group.readEntry("X-Immich-Crosslink-Version") == QLatin1String("0.2") && shows.contains(kind);
    });
}
}

class ImmichMountDolphin : public KAbstractFileItemActionPlugin
{
    Q_OBJECT

public:
    ImmichMountDolphin(QObject *parent, const KPluginMetaData &metaData, const QVariantList &)
        : KAbstractFileItemActionPlugin(parent)
    {
        // The translations are the app's catalogs: in the system's locale folder for a package, and beside the
        // plugin's own file, in share/locale, for Homebrew's cask, which keeps everything in one folder.
        const QDir folder = QFileInfo(QFileInfo(metaData.fileName()).canonicalFilePath()).dir();
        if (folder.exists(QStringLiteral("share/locale"))) {
            KLocalizedString::addDomainLocaleDir("immich-mount", folder.filePath(QStringLiteral("share/locale")));
        }
    }

    QList<QAction *> actions(const KFileItemListProperties &selection, QWidget *parentWidget) override
    {
        const KFileItemList items = selection.items();
        if (items.size() != 1) {
            return {};
        }
        const QString path = items.constFirst().localPath();
        if (path.isEmpty()) {
            return {};
        }
        const Links links = linksForPath(path);
        const QString address = links.web;
        if (address.isEmpty()) {
            return {};
        }
        // The plasma desktop's folder view gives no widget; the actions then live as long as this plugin.
        QObject *owner = parentWidget ? static_cast<QObject *>(parentWidget) : this;

        auto *open = new QAction(QIcon::fromTheme(QStringLiteral("internet-web-browser")),
                                 i18nd("immich-mount", "Open in Immich"), owner);
        connect(open, &QAction::triggered, this, [address, parentWidget]() {
            auto *job = new KIO::OpenUrlJob(QUrl(address));
            job->setUiDelegate(KIO::createDefaultJobUiDelegate(KJobUiDelegate::AutoHandlingEnabled, parentWidget));
            job->start();
        });

        auto *copy = new QAction(QIcon::fromTheme(QStringLiteral("edit-copy")), i18nd("immich-mount", "Copy Immich Link"),
                                 owner);
        connect(copy, &QAction::triggered, this, [address]() {
            QGuiApplication::clipboard()->setText(address);
        });
        QList<QAction *> actions{open, copy};

        if (links.crosslink.isEmpty()) {
            return actions;
        }
        // One item per app, by the name and icon its desktop entry gives, sending the link to that app alone.
        const QString crosslink = links.crosslink;
        for (const KService::Ptr &app : appsShowing(links.kind, links.hidden)) {
            QString label = i18nd("immich-mount", "Show in {app}");
            label.replace(QLatin1String("{app}"), app->name());
            auto *show = new QAction(QIcon::fromTheme(app->icon()), label, owner);
            connect(show, &QAction::triggered, this, [app, crosslink, parentWidget]() {
                auto *job = new KIO::ApplicationLauncherJob(app);
                job->setUrls({QUrl(crosslink)});
                job->setUiDelegate(KIO::createDefaultJobUiDelegate(KJobUiDelegate::AutoHandlingEnabled, parentWidget));
                job->start();
            });
            actions.append(show);
        }
        return actions;
    }
};

K_PLUGIN_CLASS_WITH_JSON(ImmichMountDolphin, "immich-mount-dolphin.json")

#include "immich-mount-dolphin.moc"

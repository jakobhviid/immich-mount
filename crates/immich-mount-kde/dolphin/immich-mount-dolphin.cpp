/* Open in Immich and Copy Immich Link in Dolphin's menu, for one item in the mount (ROADMAP.md, "Open in Immich").
 *
 * A file item action plugin, since KF6 has no service menu key that asks anyone about the selection
 * (X-KDE-ShowIfDBusCall went with KF5). The daemon decides: it knows the mount point and every item's Immich page, so
 * this asks it (WebLink on D-Bus) and offers the two items only when it gives an address. A daemon that is not
 * running, or slow, gives no items. */

#include <KAbstractFileItemActionPlugin>
#include <KFileItem>
#include <KFileItemListProperties>
#include <KIO/JobUiDelegateFactory>
#include <KIO/OpenUrlJob>
#include <KLocalizedString>
#include <KPluginFactory>
#include <KPluginMetaData>

#include <QAction>
#include <QClipboard>
#include <QDBusConnection>
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
constexpr int webLinkTimeoutMs = 250;

// The daemon's web address for a local path; empty when it has none, the path is outside the mount, or no daemon
// answers in time.
QString webLinkForPath(const QString &path)
{
    QDBusMessage message = QDBusMessage::createMethodCall(busName, objectPath, interfaceName, QStringLiteral("WebLink"));
    message << path;
    // No auto start: right-clicking a file elsewhere must never start the daemon.
    message.setAutoStartService(false);
    const QDBusMessage reply = QDBusConnection::sessionBus().call(message, QDBus::Block, webLinkTimeoutMs);
    if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().size() != 1) {
        return {};
    }
    return reply.arguments().constFirst().toString();
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
        const QString address = webLinkForPath(path);
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
        return {open, copy};
    }
};

K_PLUGIN_CLASS_WITH_JSON(ImmichMountDolphin, "immich-mount-dolphin.json")

#include "immich-mount-dolphin.moc"

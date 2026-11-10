// Links other apps send (ROADMAP.md, "Open in Immich, and links between apps", step 2): an immich-crosslink link, or
// the address of a page of Immich's web app, read and checked by the bridge, and its item selected in Finder; or a
// notification saying why nothing is shown. Any web page can open a crosslink, so nothing here does more than show:
// nothing is downloaded, changed or signed in to.

import AppKit
import FileProvider
import UserNotifications

@MainActor
enum ReceivedLinks {
    /// The notification's `userInfo` key that has a click on it open Settings, where Immich Crosslink is switched on.
    nonisolated static let opensSettings = "opensSettings"

    static func show(_ url: URL) async {
        guard let account = Account.load() else {
            logger.notice("a received link: nobody is signed in")
            await tell(title: tr("Not signed in"), body: "")
            return
        }
        let outcome = await showLink(server: account.server, credential: account.credential, config: AppGroup.configText,
                                     link: url.absoluteString)
        logger.notice("a received link: \(outcome.result, privacy: .public) \(outcome.identifier ?? "", privacy: .public)")
        guard outcome.result == "show", let identifier = outcome.identifier else {
            let (title, body) = refusal(outcome.result, detail: outcome.detail)
            await tell(title: title, body: body, opensSettings: outcome.result == "off")
            return
        }
        await select(NSFileProviderItemIdentifier(identifier))
    }

    /// Select the item in Finder, where the system says it is. The system finds an item it does not know yet itself,
    /// asking the extension for it and listing the folders above it (seen on macOS 27.2). It is asked twice: while it
    /// reads the location's items again after a change of settings it may answer "database is locked" (seen on macOS
    /// 27.2). Should the location not be found at all (switched off), Finder shows it as Open in Finder does.
    private static func select(_ item: NSFileProviderItemIdentifier) async {
        guard let manager = NSFileProviderManager(for: FinderLocation.domain) else {
            await AppDelegate.shared?.model.showInFinder()
            return
        }
        let url: URL
        do {
            url = try await manager.getUserVisibleURL(for: item)
        } catch {
            logger.info("the system did not tell where the item is, at first: \(String(describing: error), privacy: .public)")
            try? await Task.sleep(for: .seconds(2))
            do {
                url = try await manager.getUserVisibleURL(for: item)
            } catch {
                logger.error("the system did not tell where the item is: \(String(describing: error), privacy: .public)")
                await AppDelegate.shared?.model.showInFinder()
                return
            }
        }
        logger.notice("selected in Finder: \(url.path, privacy: .private)")
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    /// The notification for a link that shows nothing, in the user's language: Linux's daemon tells the same.
    private static func refusal(_ result: String, detail: String) -> (String, String) {
        switch result {
        case "off":
            return (tr("Links from other apps are off"), tr("Turn on Immich Crosslink in Immich Mount's settings to show what other apps link to."))
        case "invalid":
            return (tr("A link from another app cannot be opened"), tr("It is not a valid Immich link."))
        case "cannot-show":
            return (tr("Nothing to show"), tr("Immich Mount has no place for what this link points to."))
        case "other-server":
            return (tr("Not signed in to {host}", ["host": detail]), tr("Immich Mount is signed in to another Immich server."))
        case "other-user":
            return (tr("Another Immich account"), tr("The link is for another account than the one Immich Mount is signed in with."))
        case "not-found":
            return (tr("Nothing to show"),
                    tr("The drive does not show this. It may be hidden, in a section that is switched off, or not shared with you."))
        default:
            return (tr("Immich is unreachable"), tr("The Immich server cannot be reached. Try the link again later."))
        }
    }

    private static func tell(title: String, body: String, opensSettings: Bool = false) async {
        guard await FolderCopies.allowed() else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        if opensSettings {
            content.userInfo = [Self.opensSettings: true]
        }
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        try? await UNUserNotificationCenter.current().add(request)
    }
}

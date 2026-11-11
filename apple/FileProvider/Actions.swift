// Finder's actions on an item (Info.plist, NSExtensionFileProviderActions): Open in Immich opens its web address in
// the default browser, and Copy Immich Link puts the address on the pasteboard; Finder offers them on one item with a
// web address (FileProviderItem's `hasWebLink`). The extension does both itself: it runs in the user's session, and
// its sandbox lets it open a web address and write the general pasteboard, so the menu bar app need not run.
//
// Show in Another App… (ROADMAP.md, "Open in Immich, and links between apps", step 3) is offered by the items'
// `showInAnother`. It hands the item to the menu bar app (`ActionRequest`), which starts when it is not running: an
// extension can show no window to choose an app in, nor tell the user that an app is gone or did not open. The app
// sends the item's crosslink to the one app there is, or to the one the user chooses, alone.

import AppKit
import FileProvider

extension NSFileProviderExtensionActionIdentifier {
    static let openInImmich = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.open-in-immich")
    static let copyImmichLink = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.copy-link")
    static let showInAnotherApp = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.show-in-another-app")
}

extension FileProviderExtension: NSFileProviderCustomAction {
    func performAction(identifier actionIdentifier: NSFileProviderExtensionActionIdentifier,
                       onItemsWithIdentifiers itemIdentifiers: [NSFileProviderItemIdentifier],
                       completionHandler: @escaping (Error?) -> Void) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        let finish = { (error: Error?) in
            progress.completedUnitCount = 1
            completionHandler(error)
        }
        guard let library else {
            finish(NSFileProviderError(.notAuthenticated))
            return progress
        }
        guard itemIdentifiers.count == 1, let identifier = itemIdentifiers.first,
              let address = library.webLink(identifier: identifier.bridged), let url = URL(string: address) else {
            logger.error("Finder asked for a link of an item without one: \(itemIdentifiers.map(\.rawValue).joined(separator: ", "), privacy: .public)")
            finish(NSFileProviderError(.noSuchItem))
            return progress
        }
        switch actionIdentifier {
        case .openInImmich:
            NSWorkspace.shared.open(url, configuration: NSWorkspace.OpenConfiguration()) { _, error in
                if let error {
                    logger.error("opening \(address, privacy: .public) failed: \(String(describing: error), privacy: .public)")
                } else {
                    logger.notice("opened \(address, privacy: .public) in the browser")
                }
                finish(error)
            }
        case .copyImmichLink:
            DispatchQueue.main.async {
                NSPasteboard.general.clearContents()
                let copied = NSPasteboard.general.setString(address, forType: .string)
                logger.notice("copied \(address, privacy: .public) to the pasteboard: \(copied)")
                finish(copied ? nil : CocoaError(.featureUnsupported))
            }
        case .showInAnotherApp:
            Task { finish(await Self.showInAnotherApp(identifier.bridged)) }
        default:
            finish(CocoaError(.featureUnsupported))
        }
        return progress
    }

    /// Hand Show in Another App… to the app, which sends the item to the one app there is, or asks which.
    private static func showInAnotherApp(_ identifier: String) async -> Error? {
        await hand(ActionRequest(identifier: identifier))
    }

    /// Keep the request in the App Group, and open the app's own scheme (its bundle identifier) with its token alone.
    private static func hand(_ request: ActionRequest) async -> Error? {
        let token: String
        do {
            token = try request.write()
        } catch {
            logger.error("the request was not kept for the app: \(String(describing: error), privacy: .public)")
            return error
        }
        guard let appIdentifier = Bundle.main.bundleIdentifier?.replacingOccurrences(of: ".FileProvider", with: ""),
              let url = URL(string: "\(appIdentifier.lowercased())://action?token=\(token)") else {
            return NSFileProviderError(.noSuchItem)
        }
        // The extension is the app's PlugIns/<extension>.appex.
        let app = Bundle.main.bundleURL.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        return await send(url, to: app)
    }

    /// Open `url` with the app at `app` alone, never the scheme's default handler.
    private static func send(_ url: URL, to app: URL) async -> Error? {
        await withCheckedContinuation { continuation in
            NSWorkspace.shared.open([url], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration()) { _, error in
                if let error {
                    logger.error("\(app.lastPathComponent, privacy: .public) did not take \(url.absoluteString, privacy: .public): \(String(describing: error), privacy: .public)")
                } else {
                    logger.notice("sent \(url.absoluteString, privacy: .public) to \(app.lastPathComponent, privacy: .public)")
                }
                continuation.resume(returning: error)
            }
        }
    }
}

// Finder's actions on an item (Info.plist, NSExtensionFileProviderActions): Open in Immich opens its web address in
// the default browser, and Copy Immich Link puts the address on the pasteboard; Finder offers them on one item with a
// web address (FileProviderItem's `hasWebLink`). The extension does both itself: it runs in the user's session, and
// its sandbox lets it open a web address and write the general pasteboard, so the menu bar app need not run.
//
// Show in Immich Gallery and Show in Another App… (ROADMAP.md, "Open in Immich, and links between apps", step 3) are
// offered by the items' `showInGallery` and `showInAnother`. The first sends the item's crosslink (or, to a gallery
// that does not announce itself, its own link) to that app alone. The second needs a menu of the apps at the pointer,
// which an extension cannot show: it hands the crosslink to the menu bar app through the app's own scheme (its bundle
// identifier), which starts the app when it is not running.

import AppKit
import FileProvider

extension NSFileProviderExtensionActionIdentifier {
    static let openInImmich = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.open-in-immich")
    static let copyImmichLink = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.copy-link")
    static let showInGallery = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.show-in-gallery")
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
        case .showInGallery:
            Task { finish(await Self.showInGallery(identifier.bridged, library: library)) }
        case .showInAnotherApp:
            Task { finish(await Self.showInAnotherApp(identifier.bridged, library: library)) }
        default:
            finish(CocoaError(.featureUnsupported))
        }
        return progress
    }

    /// Send the item to Immich Gallery: its crosslink, or the gallery's own link to a photo in Photos.
    private static func showInGallery(_ identifier: String, library: Library) async -> Error? {
        CurrentOffers.reload()
        guard let gallery = CurrentOffers.value.gallery else { return CocoaError(.featureUnsupported) }
        let link: String?
        if let scheme = gallery.ownScheme {
            link = photoInPhotos(identifier: identifier).map { "\(scheme)://photo/\($0)" }
        } else {
            link = await library.crosslink(identifier: identifier)
        }
        guard let link, let url = URL(string: link) else { return NSFileProviderError(.noSuchItem) }
        return await send(url, to: URL(fileURLWithPath: gallery.path))
    }

    /// Hand the item's crosslink to the menu bar app, which shows the menu of the other apps at the pointer.
    private static func showInAnotherApp(_ identifier: String, library: Library) async -> Error? {
        guard let link = await library.crosslink(identifier: identifier),
              let encoded = link.addingPercentEncoding(withAllowedCharacters: .alphanumerics),
              let appIdentifier = Bundle.main.bundleIdentifier?.replacingOccurrences(of: ".FileProvider", with: ""),
              let url = URL(string: "\(appIdentifier.lowercased())://show-in-another-app?link=\(encoded)") else {
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

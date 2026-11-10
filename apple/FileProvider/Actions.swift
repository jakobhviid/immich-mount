// Finder's actions on an item (Info.plist, NSExtensionFileProviderActions): Open in Immich opens its web address in
// the default browser, and Copy Immich Link puts the address on the pasteboard. Finder offers them on one item with a
// web address (FileProviderItem's `hasWebLink`). The extension does both itself: it runs in the user's session, and
// its sandbox lets it open a web address and write the general pasteboard, so the menu bar app need not run.

import AppKit
import FileProvider

extension NSFileProviderExtensionActionIdentifier {
    static let openInImmich = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.open-in-immich")
    static let copyImmichLink = NSFileProviderExtensionActionIdentifier("dev.hviid.ImmichMount.copy-link")
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
        default:
            finish(CocoaError(.featureUnsupported))
        }
        return progress
    }
}

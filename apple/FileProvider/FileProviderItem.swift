// One item as the system sees it, made from the bridge's `Item`, and the translations between the two sides.

import FileProvider
import UniformTypeIdentifiers

final class FileProviderItem: NSObject, NSFileProviderItem {
    private let item: Item

    init(_ item: Item) {
        self.item = item
    }

    var itemIdentifier: NSFileProviderItemIdentifier { NSFileProviderItemIdentifier(bridged: item.identifier) }
    var parentItemIdentifier: NSFileProviderItemIdentifier { NSFileProviderItemIdentifier(bridged: item.parentIdentifier) }
    var filename: String { item.name }

    var contentType: UTType {
        if item.isFolder { return .folder }
        return UTType(filenameExtension: (item.name as NSString).pathExtension) ?? .data
    }

    /// Read-only, as on Linux; the Upload folder and the folders copied into it also take new files, and a copied
    /// folder may be deleted, with a file in it that did not upload.
    var capabilities: NSFileProviderItemCapabilities {
        guard item.isFolder else { return [.allowsReading] }
        guard item.acceptsNewFiles else { return [.allowsReading, .allowsContentEnumerating] }
        let adding: NSFileProviderItemCapabilities = [.allowsReading, .allowsContentEnumerating, .allowsAddingSubItems]
        return item.identifier.hasPrefix("upload-folder|") ? adding.union(.allowsDeleting) : adding
    }

    /// Downloaded when read, and evictable: Finder offers "Remove Download" on a file or a whole folder, which frees
    /// the downloaded copy, never the Asset. The policy replaces macOS 11's eviction capability.
    var contentPolicy: NSFileProviderContentPolicy { .downloadLazily }

    var documentSize: NSNumber? { item.size.map { NSNumber(value: $0) } }
    var creationDate: Date? { item.taken }
    var contentModificationDate: Date? { item.taken }

    /// What Finder's actions are offered by (the activation rules in Info.plist): `hasWebLink`, for Open in Immich and
    /// Copy Immich Link.
    var userInfo: [AnyHashable: Any]? { ["hasWebLink": item.hasWebLink] }

    var itemVersion: NSFileProviderItemVersion {
        NSFileProviderItemVersion(contentVersion: Data(item.contentVersion.utf8), metadataVersion: Data(item.version.utf8))
    }
}

extension NSFileProviderItemIdentifier {
    /// The identifier as the bridge knows it: the root container's is `root`.
    var bridged: String { self == .rootContainer ? "root" : rawValue }

    init(bridged: String) {
        self = bridged == "root" ? .rootContainer : NSFileProviderItemIdentifier(bridged)
    }
}

extension Error {
    /// The error in the system's terms, so Finder shows the right thing and tries again where it can.
    var forSystem: Error {
        if self is CancellationError { return CocoaError(.userCancelled) }
        guard let error = self as? BridgeError else { return self }
        switch error {
        case .NotAuthenticated:
            return NSFileProviderError(.notAuthenticated)
        case .ServerUnreachable:
            return NSFileProviderError(.serverUnreachable)
        case .NoSuchItem:
            return NSFileProviderError(.noSuchItem)
        case .AnchorExpired:
            return NSFileProviderError(.syncAnchorExpired)
        case .Cancelled:
            return CocoaError(.userCancelled)
        case .NotPermitted(let message):
            return CocoaError(.fileReadNoPermission, userInfo: [NSLocalizedDescriptionKey: message])
        case .Failed(let message):
            return NSError(domain: NSFileProviderErrorDomain, code: NSFileProviderError.Code.cannotSynchronize.rawValue,
                           userInfo: [NSLocalizedDescriptionKey: message])
        }
    }
}

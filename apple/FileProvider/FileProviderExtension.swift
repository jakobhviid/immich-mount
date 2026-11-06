// The File Provider extension (ARCHITECTURE.md, "The macOS edition"): a thin pass-through to the bridge, which
// answers from the portable core. The system starts it on demand and stops it when idle.

import FileProvider
import os

let logger = Logger(subsystem: "dev.hviid.ImmichMount", category: "extension")

final class FileProviderExtension: NSObject, NSFileProviderReplicatedExtension, NSFileProviderThumbnailing {
    /// How many thumbnails are fetched at once.
    private static let thumbnailsAtOnce = 8

    private let domain: NSFileProviderDomain
    /// `nil` while nobody is signed in: every request then says so, and the app asks for a sign-in.
    private let library: Library?

    required init(domain: NSFileProviderDomain) {
        self.domain = domain
        startLogging()
        library = FileProviderExtension.openLibrary()
        super.init()
        logger.notice("started for the domain \(domain.identifier.rawValue, privacy: .public), signed in: \(self.library != nil)")
    }

    private static func openLibrary() -> Library? {
        guard let account = Account.load() else {
            logger.notice("no account is signed in")
            return nil
        }
        let config = (try? String(contentsOf: AppGroup.configFile, encoding: .utf8)) ?? ""
        do {
            return try Library.open(server: account.server, credential: account.credential,
                                    directory: AppGroup.libraryDirectory.path, config: config)
        } catch {
            logger.error("the library did not open: \(String(describing: error), privacy: .public)")
            return nil
        }
    }

    func invalidate() {}

    // MARK: Items and folders

    func item(for identifier: NSFileProviderItemIdentifier, request: NSFileProviderRequest,
              completionHandler: @escaping (NSFileProviderItem?, Error?) -> Void) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        guard let library else {
            completionHandler(nil, NSFileProviderError(.notAuthenticated))
            return progress
        }
        let task = Task {
            do {
                let item = try await library.item(identifier: identifier.bridged)
                completionHandler(FileProviderItem(item), nil)
            } catch {
                logger.error("an item failed: \(String(describing: error), privacy: .public)")
                completionHandler(nil, error.forSystem)
            }
            progress.completedUnitCount = 1
        }
        progress.cancellationHandler = { task.cancel() }
        return progress
    }

    func enumerator(for containerItemIdentifier: NSFileProviderItemIdentifier,
                    request: NSFileProviderRequest) throws -> NSFileProviderEnumerator {
        guard let library else { throw NSFileProviderError(.notAuthenticated) }
        followConfig(library)
        return Enumerator(container: containerItemIdentifier, library: library)
    }

    /// Follow config.toml once the app's Settings window changed it; the app then asks the system to list everything
    /// again, which comes through here first.
    private func followConfig(_ library: Library) {
        let config = (try? String(contentsOf: AppGroup.configFile, encoding: .utf8)) ?? ""
        do {
            if try library.applyConfig(config: config) {
                logger.notice("the settings changed; folders follow them")
            }
        } catch {
            logger.error("the settings were not followed: \(String(describing: error), privacy: .public)")
        }
    }

    // MARK: Content

    func fetchContents(for itemIdentifier: NSFileProviderItemIdentifier, version requestedVersion: NSFileProviderItemVersion?,
                       request: NSFileProviderRequest,
                       completionHandler: @escaping (URL?, NSFileProviderItem?, Error?) -> Void) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        guard let library else {
            completionHandler(nil, nil, NSFileProviderError(.notAuthenticated))
            return progress
        }
        let domain = domain
        let task = Task {
            do {
                // The system's own temporary folder for the domain is on the right volume to take the file from.
                let directory = try NSFileProviderManager(for: domain)?.temporaryDirectoryURL()
                    ?? FileManager.default.temporaryDirectory
                let destination = directory.appendingPathComponent(UUID().uuidString)
                let item = try await library.fetch(identifier: itemIdentifier.bridged, destination: destination.path)
                logger.info("fetched an original of \(item.size ?? 0) bytes")
                completionHandler(destination, FileProviderItem(item), nil)
            } catch {
                logger.error("fetching an original failed: \(String(describing: error), privacy: .public)")
                completionHandler(nil, nil, error.forSystem)
            }
            progress.completedUnitCount = 1
        }
        progress.cancellationHandler = { task.cancel() }
        return progress
    }

    func fetchThumbnails(for itemIdentifiers: [NSFileProviderItemIdentifier], requestedSize size: CGSize,
                         perThumbnailCompletionHandler: @escaping (NSFileProviderItemIdentifier, Data?, Error?) -> Void,
                         completionHandler: @escaping (Error?) -> Void) -> Progress {
        let progress = Progress(totalUnitCount: Int64(itemIdentifiers.count))
        guard let library else {
            completionHandler(NSFileProviderError(.notAuthenticated))
            return progress
        }
        let pixels = UInt32(max(size.width, size.height).rounded(.up))
        let task = Task {
            await withTaskGroup(of: Void.self) { group in
                for (index, identifier) in itemIdentifiers.enumerated() {
                    if index >= FileProviderExtension.thumbnailsAtOnce {
                        await group.next()
                    }
                    group.addTask {
                        do {
                            let data = try await library.thumbnail(identifier: identifier.bridged, pixels: pixels)
                            perThumbnailCompletionHandler(identifier, data, nil)
                        } catch {
                            perThumbnailCompletionHandler(identifier, nil, error.forSystem)
                        }
                        progress.completedUnitCount += 1
                    }
                }
            }
            completionHandler(Task.isCancelled ? CocoaError(.userCancelled) : nil)
        }
        progress.cancellationHandler = { task.cancel() }
        return progress
    }

    // MARK: Changes from Finder

    // Nothing is added, changed or deleted from Finder yet: the Upload folder comes with uploads (ROADMAP, M7), and
    // every other folder is a read-only view, as on Linux. The items' capabilities already keep Finder from offering
    // these; a request that comes anyway is refused.

    func createItem(basedOn itemTemplate: NSFileProviderItem, fields: NSFileProviderItemFields, contents url: URL?,
                    options: NSFileProviderCreateItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        completionHandler(nil, [], false, CocoaError(.featureUnsupported))
        return Progress()
    }

    func modifyItem(_ item: NSFileProviderItem, baseVersion version: NSFileProviderItemVersion,
                    changedFields: NSFileProviderItemFields, contents newContents: URL?,
                    options: NSFileProviderModifyItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        completionHandler(nil, [], false, CocoaError(.featureUnsupported))
        return Progress()
    }

    func deleteItem(identifier: NSFileProviderItemIdentifier, baseVersion version: NSFileProviderItemVersion,
                    options: NSFileProviderDeleteItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (Error?) -> Void) -> Progress {
        completionHandler(CocoaError(.featureUnsupported))
        return Progress()
    }
}

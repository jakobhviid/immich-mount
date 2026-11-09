// The File Provider extension (ARCHITECTURE.md, "The macOS edition"): a thin pass-through to the bridge, which
// answers from the portable core. The system starts it on demand and stops it when idle.

import FileProvider
import os
import UniformTypeIdentifiers

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
        let cancel = Cancel()
        let task = Task {
            do {
                let item = try await library.item(identifier: identifier.bridged, cancel: cancel)
                completionHandler(FileProviderItem(item), nil)
            } catch {
                logger.error("an item failed: \(String(describing: error), privacy: .public)")
                completionHandler(nil, error.forSystem)
            }
            progress.completedUnitCount = 1
        }
        progress.cancellationHandler = { cancel.cancel(); task.cancel() }
        return progress
    }

    func enumerator(for containerItemIdentifier: NSFileProviderItemIdentifier,
                    request: NSFileProviderRequest) throws -> NSFileProviderEnumerator {
        guard let library else { throw NSFileProviderError(.notAuthenticated) }
        followConfig(library)
        return Enumerator(container: containerItemIdentifier, library: library, domain: domain)
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
        let cancel = Cancel()
        let task = Task {
            do {
                // The system's own temporary folder for the domain is on the right volume to take the file from.
                let directory = try NSFileProviderManager(for: domain)?.temporaryDirectoryURL()
                    ?? FileManager.default.temporaryDirectory
                let destination = directory.appendingPathComponent(UUID().uuidString)
                let item = try await library.fetch(identifier: itemIdentifier.bridged, destination: destination.path, cancel: cancel)
                logger.info("fetched an original of \(item.size ?? 0) bytes")
                completionHandler(destination, FileProviderItem(item), nil)
            } catch {
                logger.error("fetching an original failed: \(String(describing: error), privacy: .public)")
                completionHandler(nil, nil, error.forSystem)
            }
            progress.completedUnitCount = 1
        }
        progress.cancellationHandler = { cancel.cancel(); task.cancel() }
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
        let cancel = Cancel()
        let task = Task {
            // Only this task counts, as each thumbnail is done, so no count is lost between tasks.
            await withTaskGroup(of: Void.self) { group in
                for (index, identifier) in itemIdentifiers.enumerated() {
                    if index >= FileProviderExtension.thumbnailsAtOnce, await group.next() != nil {
                        progress.completedUnitCount += 1
                    }
                    group.addTask {
                        do {
                            let data = try await library.thumbnail(identifier: identifier.bridged, pixels: pixels, cancel: cancel)
                            perThumbnailCompletionHandler(identifier, data, nil)
                        } catch {
                            perThumbnailCompletionHandler(identifier, nil, error.forSystem)
                        }
                    }
                }
                for await _ in group {
                    progress.completedUnitCount += 1
                }
            }
            completionHandler(Task.isCancelled ? CocoaError(.userCancelled) : nil)
        }
        progress.cancellationHandler = { cancel.cancel(); task.cancel() }
        return progress
    }

    // MARK: Changes from Finder

    // Only the Upload folder takes anything: new files, which are uploaded and leave it a little later, and folders,
    // whose photos and videos are uploaded too and whose albums follow (ARCHITECTURE.md, "Folders copied into
    // Upload"). Every other folder is a read-only view, as on Linux (ARCHITECTURE.md, "The macOS edition", "Uploads").
    // The items' capabilities keep Finder from offering the rest; a request that comes anyway is refused.

    func createItem(basedOn itemTemplate: NSFileProviderItem, fields: NSFileProviderItemFields, contents url: URL?,
                    options: NSFileProviderCreateItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        let progress = Progress(totalUnitCount: 1)
        guard let library else {
            completionHandler(nil, [], false, NSFileProviderError(.notAuthenticated))
            return progress
        }
        let kind = itemTemplate.contentType ?? nil
        let isFolder = kind?.conforms(to: .folder) ?? false
        let isFile = kind.map { !$0.conforms(to: .folder) && !$0.conforms(to: .symbolicLink) && !$0.conforms(to: .aliasFile) } ?? true
        let modified = (itemTemplate.contentModificationDate ?? nil) ?? Date()
        let (folder, name) = (itemTemplate.parentItemIdentifier.bridged, itemTemplate.filename)
        // Finder's record of a folder's view is no photo: kept on this Mac only, and never pending, so it keeps no
        // copied folder in Upload.
        if name == ".DS_Store" {
            completionHandler(nil, [], false, NSFileProviderError(.excludedFromSync))
            return progress
        }
        let reimported = options.contains(.mayAlreadyExist)
        let cancel = Cancel()
        let task = Task {
            do {
                // A reimport drops the system's database, and it finds what is on disk again as new items. Each is
                // answered with the item the library has under that name, so the system matches them instead of
                // keeping both (it would rename the library's own folder "People 2").
                if reimported, let existing = try await library.list(identifier: folder, cancel: cancel).first(where: { $0.name == name }) {
                    completionHandler(FileProviderItem(existing), [], false, nil)
                    progress.completedUnitCount = 1
                    return
                }
                if isFolder {
                    let item = try library.createFolder(parent: folder, name: name)
                    logger.notice("a folder was copied into Upload")
                    completionHandler(FileProviderItem(item), [], false, nil)
                    progress.completedUnitCount = 1
                    return
                }
                guard let url, isFile else {
                    throw FileProviderExtension.refusal("Only photos, videos and folders of them can be added; links cannot.")
                }
                let item = try await library.upload(folder: folder, name: name, path: url.path, modified: modified, cancel: cancel)
                logger.notice("uploaded a file of \(item.size ?? 0) bytes")
                completionHandler(FileProviderItem(item), [], false, nil)
            } catch {
                logger.error("adding an item failed: \(String(describing: error), privacy: .public)")
                completionHandler(nil, [], false, error.forSystem)
            }
            progress.completedUnitCount = 1
        }
        progress.cancellationHandler = { cancel.cancel(); task.cancel() }
        return progress
    }

    /// A refusal Finder shows with its reason.
    private static func refusal(_ message: String) -> Error {
        NSError(domain: NSFileProviderErrorDomain, code: NSFileProviderError.Code.cannotSynchronize.rawValue,
                userInfo: [NSLocalizedDescriptionKey: message])
    }

    /// Upload and the folders copied into it change only their times and such on disk, as files come and go: those
    /// are taken and kept as they are, as Linux takes a copied folder's times. Refused, the system marked Upload as
    /// failing and tried again without end (seen on macOS 27.2, "diffs:mtime"). Anything else (a rename, a move, new
    /// contents) is refused: every other folder is a view of the library.
    func modifyItem(_ item: NSFileProviderItem, baseVersion version: NSFileProviderItemVersion,
                    changedFields: NSFileProviderItemFields, contents newContents: URL?,
                    options: NSFileProviderModifyItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (NSFileProviderItem?, NSFileProviderItemFields, Bool, Error?) -> Void) -> Progress {
        let metadata: NSFileProviderItemFields = [.contentModificationDate, .creationDate, .extendedAttributes, .tagData,
                                                  .lastUsedDate, .favoriteRank, .fileSystemFlags]
        let identifier = item.itemIdentifier.bridged
        let takesFiles = identifier == "upload" || identifier.hasPrefix("upload-folder|")
        guard let library, takesFiles, changedFields.subtracting(metadata).isEmpty else {
            completionHandler(nil, [], false, CocoaError(.featureUnsupported))
            return Progress()
        }
        let progress = Progress(totalUnitCount: 1)
        let task = Task {
            do {
                let current = try await library.item(identifier: identifier, cancel: nil)
                completionHandler(FileProviderItem(current), [], false, nil)
            } catch {
                completionHandler(nil, [], false, error.forSystem)
            }
            progress.completedUnitCount = 1
        }
        progress.cancellationHandler = { task.cancel() }
        return progress
    }

    /// Only a folder copied into Upload may be deleted (with a file in it that did not upload); everything else is a
    /// view of the library, which Finder never changes.
    func deleteItem(identifier: NSFileProviderItemIdentifier, baseVersion version: NSFileProviderItemVersion,
                    options: NSFileProviderDeleteItemOptions = [], request: NSFileProviderRequest,
                    completionHandler: @escaping (Error?) -> Void) -> Progress {
        do {
            guard let library else { throw NSFileProviderError(.notAuthenticated) }
            try library.deleteFolder(identifier: identifier.bridged)
            completionHandler(nil)
        } catch BridgeError.NoSuchItem {
            completionHandler(NSFileProviderError(.noSuchItem))
        } catch {
            completionHandler(CocoaError(.featureUnsupported))
        }
        return Progress()
    }
}

// Folder listings for the system, from the bridge.

import FileProvider

final class Enumerator: NSObject, NSFileProviderEnumerator {
    /// How many items go to the system per call when it does not suggest a number.
    private static let batchSize = 200
    /// The server's version, asked once per run of the extension, for the app's status.
    private static var version: String?

    private let container: NSFileProviderItemIdentifier
    private let library: Library
    private var task: Task<Void, Never>?

    init(container: NSFileProviderItemIdentifier, library: Library) {
        self.container = container
        self.library = library
    }

    func invalidate() {
        task?.cancel()
    }

    func enumerateItems(for observer: NSFileProviderEnumerationObserver, startingAt page: NSFileProviderPage) {
        // There is no trash. The working set is every item the system holds, asked for after an anchor expired.
        if container == .trashContainer {
            observer.finishEnumerating(upTo: nil)
            return
        }
        let (container, library) = (container, library)
        task = Task {
            do {
                let found: [Item]
                if container == .workingSet {
                    found = library.workingSet()
                } else {
                    found = try await library.list(identifier: container.bridged)
                }
                let items = found.map(FileProviderItem.init)
                let batch = max(observer.suggestedPageSize ?? Enumerator.batchSize, 1)
                for start in stride(from: 0, to: items.count, by: batch) {
                    observer.didEnumerate(Array(items[start..<min(start + batch, items.count)]))
                }
                observer.finishEnumerating(upTo: nil)
                logger.info("listed a folder of \(items.count) items")
            } catch {
                logger.error("listing a folder failed: \(String(describing: error), privacy: .public)")
                observer.finishEnumeratingWithError(error.forSystem)
            }
        }
    }

    /// Changes reach the system through the working set, from the change journal (ARCHITECTURE.md, "The macOS
    /// edition", "Changes"); a folder's own enumerator has none to report.
    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        guard container == .workingSet else {
            observer.finishEnumeratingChanges(upTo: anchor, moreComing: false)
            return
        }
        let library = library
        let limit = UInt32(clamping: max(observer.suggestedBatchSize ?? Enumerator.batchSize, 1))
        task = Task {
            do {
                let from = String(decoding: anchor.rawValue, as: UTF8.self)
                // The server is asked once the system has caught up; while changes wait, it comes back for them.
                if from == (try library.currentAnchor()) {
                    do {
                        _ = try await library.poll()
                        if Enumerator.version == nil {
                            Enumerator.version = try? await library.serverVersion()
                        }
                        Status(online: true, problem: nil, version: Enumerator.version, checked: Date()).save()
                    } catch {
                        logger.info("asking the server what changed failed: \(String(describing: error), privacy: .public)")
                        Status(online: false, problem: describe(error), version: Enumerator.version, checked: Date()).save()
                    }
                }
                let batch = try library.changesSince(anchor: from, limit: limit)
                if !batch.deleted.isEmpty {
                    observer.didDeleteItems(withIdentifiers: batch.deleted.map(NSFileProviderItemIdentifier.init(bridged:)))
                }
                if !batch.updated.isEmpty {
                    observer.didUpdate(batch.updated.map(FileProviderItem.init))
                }
                if !batch.deleted.isEmpty || !batch.updated.isEmpty {
                    logger.info("changes: \(batch.updated.count) updated, \(batch.deleted.count) gone")
                }
                observer.finishEnumeratingChanges(upTo: NSFileProviderSyncAnchor(Data(batch.anchor.utf8)), moreComing: batch.more)
            } catch {
                logger.error("reporting changes failed: \(String(describing: error), privacy: .public)")
                observer.finishEnumeratingWithError(error.forSystem)
            }
        }
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        completionHandler((try? library.currentAnchor()).map { NSFileProviderSyncAnchor(Data($0.utf8)) })
    }
}

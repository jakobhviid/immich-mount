// Folder listings for the system, from the bridge.

import FileProvider

final class Enumerator: NSObject, NSFileProviderEnumerator {
    /// The anchor while changes are not tracked yet: nothing ever changes (ROADMAP, M7, "Live changes").
    private static let unchanging = NSFileProviderSyncAnchor(Data("unchanging".utf8))
    /// How many items go to the system per call when it does not suggest a number.
    private static let batchSize = 200

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
        // The working set holds the items whose changes the system follows, which come with the change journal;
        // there is no trash.
        if container == .workingSet || container == .trashContainer {
            observer.finishEnumerating(upTo: nil)
            return
        }
        let (container, library) = (container, library)
        task = Task {
            do {
                let items = try await library.list(identifier: container.bridged).map(FileProviderItem.init)
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

    func enumerateChanges(for observer: NSFileProviderChangeObserver, from anchor: NSFileProviderSyncAnchor) {
        observer.finishEnumeratingChanges(upTo: Enumerator.unchanging, moreComing: false)
    }

    func currentSyncAnchor(completionHandler: @escaping (NSFileProviderSyncAnchor?) -> Void) {
        completionHandler(Enumerator.unchanging)
    }
}

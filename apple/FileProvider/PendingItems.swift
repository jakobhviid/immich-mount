// The folders holding items macOS has not handed over to the extension yet: its pending set. A folder copied into
// Upload leaves soon after its copy only when macOS says it holds none of them (ARCHITECTURE.md, "Folders copied into
// Upload"): Finder may still be writing a large file into it, which the extension hears of only once it is whole.

import FileProvider
import os

enum PendingItems {
    /// How long macOS may take to say; past it, nobody knows, and the folders stay their longer while.
    private static let patience: DispatchTimeInterval = .seconds(5)

    /// The folders that hold pending items, with how often macOS brings the list up to date; `nil` when macOS does not
    /// say, or says its list is full, and so may leave some out.
    static func folders(in domain: NSFileProviderDomain) async -> PendingFolders? {
        guard let manager = NSFileProviderManager(for: domain) else { return nil }
        let enumerator = manager.enumeratorForPendingItems()
        defer { enumerator.invalidate() }
        let folders: [String]? = await withCheckedContinuation { continuation in
            let answered = OSAllocatedUnfairLock(initialState: false)
            let answer: ([String]?) -> Void = { folders in
                let first = answered.withLock { done in
                    defer { done = true }
                    return !done
                }
                if first { continuation.resume(returning: folders) }
            }
            let collector = Collector(enumerator: enumerator, done: answer)
            collector.start()
            // Holding the collector until then: the system's enumerator may not keep its observer.
            DispatchQueue.global().asyncAfter(deadline: .now() + patience) { withExtendedLifetime(collector) { answer(nil) } }
        }
        guard let folders, !enumerator.isMaximumSizeReached else { return nil }
        let refresh = UInt32(clamping: Int(enumerator.refreshInterval.rounded(.up)))
        return PendingFolders(folders: folders, refreshSeconds: refresh)
    }
}

/// Reads the pending set page by page, keeping each item's folder.
private final class Collector: NSObject, NSFileProviderEnumerationObserver {
    private let enumerator: NSFileProviderEnumerator
    private let done: ([String]?) -> Void
    private let parents = OSAllocatedUnfairLock(initialState: Set<String>())

    init(enumerator: NSFileProviderEnumerator, done: @escaping ([String]?) -> Void) {
        self.enumerator = enumerator
        self.done = done
    }

    func start() {
        enumerator.enumerateItems(for: self, startingAt: NSFileProviderPage(NSFileProviderPage.initialPageSortedByName as Data))
    }

    func didEnumerate(_ updatedItems: [NSFileProviderItemProtocol]) {
        let folders = updatedItems.map { $0.parentItemIdentifier.bridged }
        parents.withLock { $0.formUnion(folders) }
    }

    func finishEnumerating(upTo nextPage: NSFileProviderPage?) {
        if let nextPage {
            enumerator.enumerateItems(for: self, startingAt: nextPage)
        } else {
            done(Array(parents.withLock { $0 }))
        }
    }

    func finishEnumeratingWithError(_ error: Error) {
        logger.info("macOS did not say what it holds back: \(String(describing: error), privacy: .public)")
        done(nil)
    }
}

// The apps Finder offers in Show in (ROADMAP.md, "Open in Immich, and links between apps", step 3). The app finds
// the apps that take crosslinks (`CrosslinkApps`) and keeps the offers here, in the App Group; the extension marks the
// items they are offered on, which Finder's actions are offered by.

import Foundation

struct OfferedApps: Codable, Equatable {
    struct App: Codable, Equatable {
        let bundleId: String
        let path: String
        /// The kinds of item it is offered on.
        let kinds: [String]
        /// For a known app that does not announce itself: the scheme of its own links, sent for a photo in Photos.
        let ownScheme: String?
    }

    /// Show in Immich Gallery.
    var gallery: App?
    /// The apps of Show in Another App….
    var others: [App] = []

    static var file: URL { AppGroup.container.appendingPathComponent("crosslink-apps.json") }

    /// The offers the app kept; none before it first looked.
    static func load() -> OfferedApps {
        guard let data = try? Data(contentsOf: file) else { return OfferedApps() }
        return (try? JSONDecoder().decode(OfferedApps.self, from: data)) ?? OfferedApps()
    }

    func save() throws {
        try FileManager.default.createDirectory(at: AppGroup.container, withIntermediateDirectories: true)
        try JSONEncoder().encode(self).write(to: Self.file, options: .atomic)
    }

    /// Whether Show in Immich Gallery is offered on an item of `kind` (nil: one Immich has no page for); a known app's
    /// own link only names a photo in Photos.
    func offersGallery(kind: String?, photoInPhotos: Bool) -> Bool {
        guard let gallery, let kind, gallery.kinds.contains(kind) else { return false }
        return gallery.ownScheme == nil || photoInPhotos
    }

    /// Whether Show in Another App… is offered on an item of `kind`.
    func offersAnother(kind: String?) -> Bool {
        guard let kind else { return false }
        return others.contains { $0.kinds.contains(kind) }
    }
}

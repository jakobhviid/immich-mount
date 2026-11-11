// The apps Finder's Show in Another App… offers (ROADMAP.md, "Open in Immich, and links between apps", step 3). The
// app finds the apps that take crosslinks (`CrosslinkApps`) and keeps the offers here, in the App Group; the extension
// marks the items they are offered on, which Finder's action is offered by.

import Foundation

struct OfferedApps: Codable, Equatable {
    struct App: Codable, Equatable {
        let bundleId: String
        let path: String
        /// The kinds of item it is offered on.
        let kinds: [String]
    }

    var apps: [App] = []

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

    /// The apps offered on an item of `kind` (nil: one Immich has no page for).
    func apps(for kind: String?) -> [App] {
        guard let kind else { return [] }
        return apps.filter { $0.kinds.contains(kind) }
    }
}

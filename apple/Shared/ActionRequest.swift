// Finder's Show in Another App…, which the extension hands to the menu bar app: the extension can show no window or
// message, so it writes the request into the App Group and opens the app's own scheme with the request's one-time
// token alone. Any web page can open that scheme, so the address carries nothing to act on: the app takes only a
// request the extension wrote moments ago, and makes the link itself from the item.

import Foundation

struct ActionRequest: Codable {
    /// How long a request waits for the app, which may have to start first.
    private static let freshFor: TimeInterval = 20

    /// The item's identifier.
    let identifier: String

    private static var folder: URL { AppGroup.container.appendingPathComponent("actions", isDirectory: true) }

    /// Keep the request, and return its token.
    func write() throws -> String {
        let token = UUID().uuidString
        try FileManager.default.createDirectory(at: Self.folder, withIntermediateDirectories: true)
        try JSONEncoder().encode(self).write(to: Self.folder.appendingPathComponent("\(token).json"), options: .atomic)
        return token
    }

    /// The request a token names, taken once: `nil` for a token that is not a request's, or one too old.
    static func take(_ token: String) -> ActionRequest? {
        guard let uuid = UUID(uuidString: token) else { return nil }
        let file = folder.appendingPathComponent("\(uuid.uuidString).json")
        defer { try? FileManager.default.removeItem(at: file) }
        guard let written = (try? file.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate,
              abs(written.timeIntervalSinceNow) < freshFor,
              let data = try? Data(contentsOf: file) else { return nil }
        return try? JSONDecoder().decode(ActionRequest.self, from: data)
    }

    /// Remove requests no one took (the app was not running and never started, say), which `take` would refuse as
    /// too old anyway; the app does it as it starts.
    static func removeStale() {
        let files = (try? FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        for file in files where file.pathExtension == "json" {
            let written = (try? file.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate ?? .distantPast
            if abs(written.timeIntervalSinceNow) >= freshFor {
                try? FileManager.default.removeItem(at: file)
            }
        }
    }
}

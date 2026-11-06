// What the extension last saw of the server, kept in the App Group for the app's status page (ARCHITECTURE.md, "The
// macOS edition", "Two processes"): a file outlives the extension, which the system stops when idle.

import Foundation

struct Status: Codable {
    var online: Bool
    /// Why the server could not be reached, when it could not.
    var problem: String?
    /// Immich's version, as `v3.3.5`, once known.
    var version: String?
    var checked: Date

    static var file: URL { AppGroup.container.appendingPathComponent("status.json") }

    static func load() -> Status? {
        guard let data = try? Data(contentsOf: file) else { return nil }
        return try? JSONDecoder().decode(Status.self, from: data)
    }

    func save() {
        guard let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: Status.file, options: .atomic)
    }
}

/// An error as a person reads it.
func describe(_ error: Error) -> String {
    switch error as? BridgeError {
    case .NotAuthenticated?:
        return "The server did not accept the sign-in; sign in again."
    case .ServerUnreachable(let message)?:
        return "The server could not be reached: \(message)"
    case .NoSuchItem?:
        return "No Immich server answers at this address."
    case .AnchorExpired?:
        return "Finder asked for changes the app no longer has; it reads the library again."
    case .Cancelled?:
        return "Cancelled."
    case .NotPermitted(let message)?, .Failed(let message)?:
        return message
    case nil:
        return error.localizedDescription
    }
}

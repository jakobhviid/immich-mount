// Where the app and the extension meet (ARCHITECTURE.md, "The macOS edition"): the App Group container holds the
// server's address, config.toml and the extension's metadata store; a Keychain access group holds the API key.

import Foundation
import Security

enum AppGroup {
    /// The team-prefixed group both targets carry in their entitlements, read from Info.plist, where Xcode writes it
    /// with the signing team's prefix.
    static let identifier = infoString("ImmichMountAppGroup")
    static let keychainGroup = infoString("ImmichMountKeychainGroup")

    static var container: URL {
        guard let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: identifier) else {
            fatalError("the App Group \(identifier) is missing from the entitlements")
        }
        return url
    }

    /// The server's address, as checked when signing in. Not secret, so a plain file.
    static var serverFile: URL { container.appendingPathComponent("server") }
    /// The same settings table as on Linux (SPEC.md, "Configuration"); absent means the defaults.
    static var configFile: URL { container.appendingPathComponent("config.toml") }
    /// The extension's own files: the metadata store. Only the extension opens it.
    static var libraryDirectory: URL { container.appendingPathComponent("Library", isDirectory: true) }

    private static func infoString(_ key: String) -> String {
        guard let value = Bundle.main.object(forInfoDictionaryKey: key) as? String, !value.isEmpty else {
            fatalError("\(key) is missing from Info.plist")
        }
        return value
    }
}

/// The signed-in account: the server's address and its API key.
struct Account {
    let server: String
    let apiKey: String

    private static let service = "dev.hviid.ImmichMount"

    /// The account the app saved, or `nil` when nobody signed in (or the Keychain cannot be read).
    static func load() -> Account? {
        guard let server = try? String(contentsOf: AppGroup.serverFile, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines), !server.isEmpty else { return nil }
        var query = keychainQuery(server: server)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var found: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &found) == errSecSuccess,
              let data = found as? Data, let apiKey = String(data: data, encoding: .utf8) else { return nil }
        return Account(server: server, apiKey: apiKey)
    }

    /// Keep the account: the key in the Keychain, readable by the extension after the first unlock, and the address
    /// beside it.
    func save() throws {
        Account.forget()
        var item = Account.keychainQuery(server: server)
        item[kSecValueData as String] = Data(apiKey.utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        try FileManager.default.createDirectory(at: AppGroup.container, withIntermediateDirectories: true)
        try server.write(to: AppGroup.serverFile, atomically: true, encoding: .utf8)
    }

    /// Forget the account: the key and the address. Nothing else is touched.
    static func forget() {
        if let server = try? String(contentsOf: AppGroup.serverFile, encoding: .utf8)
            .trimmingCharacters(in: .whitespacesAndNewlines) {
            SecItemDelete(keychainQuery(server: server) as CFDictionary)
        }
        try? FileManager.default.removeItem(at: AppGroup.serverFile)
    }

    private static func keychainQuery(server: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: server,
            kSecAttrAccessGroup as String: AppGroup.keychainGroup,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }
}

struct KeychainError: LocalizedError {
    let status: OSStatus

    var errorDescription: String? {
        let detail = (SecCopyErrorMessageString(status, nil) as String?) ?? "error \(status)"
        return "The Keychain refused the API key: \(detail)"
    }
}

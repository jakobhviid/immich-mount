// The app (ARCHITECTURE.md, "The macOS edition"): signs in, adds the Finder location and shows status. Closing it
// stops nothing; the extension serves Finder on its own.

import AppKit
import FileProvider
import os
import SwiftUI

let logger = Logger(subsystem: "dev.hviid.ImmichMount", category: "app")

@main
struct ImmichMountApp: App {
    @StateObject private var model = AppModel()

    init() {
        startLogging()
    }

    var body: some Scene {
        WindowGroup("Immich Mount") {
            ContentView()
                .environmentObject(model)
        }
        .windowResizability(.contentSize)
    }
}

/// The Finder location: one File Provider domain, shown as "Immich" in the sidebar, as the volume is on Linux.
enum FinderLocation {
    /// Never changes: the system keys its copy of the library by it.
    static let domain = NSFileProviderDomain(identifier: NSFileProviderDomainIdentifier("library"), displayName: "Immich")

    static func add() async throws {
        try await NSFileProviderManager.add(domain)
    }

    /// Remove the location and everything the system kept of it.
    static func remove() async throws {
        _ = try await NSFileProviderManager.remove(domain, mode: .removeAll)
    }

    /// Where Finder shows the location's root.
    static func rootURL() async throws -> URL? {
        try await NSFileProviderManager(for: domain)?.getUserVisibleURL(for: .rootContainer)
    }
}

@MainActor
final class AppModel: ObservableObject {
    /// The signed-in server, `nil` before signing in.
    @Published private(set) var server: String? = Account.load()?.server
    @Published private(set) var busy = false
    @Published var message: String?

    /// Check the server and the key from the app, before the extension ever runs, so macOS's Local Network prompt
    /// names the app; then keep them and add the Finder location.
    func signIn(address: String, apiKey: String) async {
        busy = true
        message = nil
        defer { busy = false }
        do {
            let key = apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
            let found = try await checkServer(address: address, apiKey: key)
            try Account(server: found.address, apiKey: key).save()
            try await FinderLocation.add()
            server = found.address
            logger.notice("signed in to Immich \(found.version, privacy: .public); the Finder location is added")
        } catch {
            message = AppModel.describe(error)
            logger.error("signing in failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Remove the Finder location, the account and the extension's store.
    func signOut() async {
        busy = true
        message = nil
        defer { busy = false }
        do {
            try await FinderLocation.remove()
        } catch {
            message = AppModel.describe(error)
        }
        Account.forget()
        try? FileManager.default.removeItem(at: AppGroup.libraryDirectory)
        server = nil
        logger.notice("signed out; the Finder location is removed")
    }

    #if DEBUG
    /// For driving a development build from a script (the `mac-dev` skill), with launch arguments: `-signOut YES`
    /// signs out, and `-signInServer <address> -signInAPIKey <key>` signs in when nobody is. Not in release builds.
    func followLaunchArguments() async {
        let arguments = UserDefaults.standard
        if arguments.bool(forKey: "signOut") {
            await signOut()
        }
        if server == nil, let address = arguments.string(forKey: "signInServer"),
           let apiKey = arguments.string(forKey: "signInAPIKey") {
            await signIn(address: address, apiKey: apiKey)
        }
    }
    #endif

    func showInFinder() async {
        do {
            guard let url = try await FinderLocation.rootURL() else { return }
            NSWorkspace.shared.open(url)
        } catch {
            message = "Finder does not show Immich yet. Is the extension switched on in System Settings? (\(error.localizedDescription))"
        }
    }

    /// System Settings at Login Items & Extensions, where the extension is switched on.
    func openExtensionSettings() {
        if let url = URL(string: "x-apple.systempreferences:com.apple.LoginItems-Settings.extension") {
            NSWorkspace.shared.open(url)
        }
    }

    static func describe(_ error: Error) -> String {
        switch error as? BridgeError {
        case .NotAuthenticated?:
            return "The server did not accept this API key."
        case .ServerUnreachable(let message)?:
            return "The server could not be reached: \(message)"
        case .NoSuchItem?:
            return "No Immich server answers at this address."
        case .NotPermitted(let message)?, .Failed(let message)?:
            return message
        case nil:
            return error.localizedDescription
        }
    }
}

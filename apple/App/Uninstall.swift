// `Immich Mount --uninstall`: what `brew uninstall` runs before it deletes the app (the cask's uninstall script), with
// the app quit. It signs out, as Sign Out does: the Finder location goes (macOS keeps it until its app removes it, and
// dragging the app to the Bin runs nothing), with the account, the Keychain entry and the extension's store. Files in
// Upload that had not reached Immich are kept by macOS, which says where. The cask runs it only for an app whose
// Info.plist has `ImmichMountUninstall`: an older app would ignore the argument and start, and the uninstall would wait.

import FileProvider
import Foundation

enum Uninstall {
    static var asked: Bool { CommandLine.arguments.contains("--uninstall") }

    /// Sign out, say what was done, and end the process.
    static func run() -> Never {
        let done = DispatchSemaphore(value: 0)
        Task.detached {
            await signOut()
            done.signal()
        }
        done.wait()
        exit(0)
    }

    private static func signOut() async {
        do {
            if let kept = try await NSFileProviderManager.remove(FinderLocation.domain, mode: .preserveDirtyUserData) {
                print("Immich Mount: files in Upload that had not reached Immich are kept in \(kept.path)")
            }
            print("Immich Mount: its location left Finder")
        } catch {
            // Never signed in, or signed out already: no location to remove.
            logger.notice("uninstalling: no Finder location removed (\(String(describing: error), privacy: .public))")
        }
        if let account = Account.load() {
            await endSession(server: account.server, credential: account.credential)
            print("Immich Mount: signed out of \(account.server)")
        }
        Account.forget()
        try? FileManager.default.removeItem(at: AppGroup.libraryDirectory)
        logger.notice("uninstalling: signed out")
    }
}

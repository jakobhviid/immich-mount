// A test participant of immich-crosslink (lab/mac/crosslink-app.sh): it writes each link it receives to
// crosslink-received.log in the user's temporary folder (`getconf DARWIN_USER_TEMP_DIR`), then quits.
import AppKit
final class Delegate: NSObject, NSApplicationDelegate {
    func application(_ application: NSApplication, open urls: [URL]) {
        let log = FileManager.default.temporaryDirectory.appendingPathComponent("crosslink-received.log")
        for url in urls {
            let line = "\(Bundle.main.bundleIdentifier ?? "?") \(url.absoluteString)\n"
            if let handle = try? FileHandle(forWritingTo: log) { handle.seekToEndOfFile(); handle.write(Data(line.utf8)); try? handle.close() }
            else { try? line.write(to: log, atomically: true, encoding: .utf8) }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { NSApp.terminate(nil) }
    }
}
let delegate = Delegate()
NSApplication.shared.delegate = delegate
NSApplication.shared.setActivationPolicy(.accessory)
NSApplication.shared.run()

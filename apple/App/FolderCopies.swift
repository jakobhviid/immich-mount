// Folders copied into Upload (ARCHITECTURE.md, "Folders copied into Upload"). The extension follows each copy and,
// once it has settled, leaves its question in the library's directory, since it cannot show anything itself; the app
// asks it in a dialog, which stays above other apps until one of its two buttons is clicked, and passes the answer
// back. A notification was easy to miss for a question that needs an answer. The app also tells what came of the
// albums, as notifications.

import AppKit
import FileProvider
import UserNotifications

@MainActor
enum FolderCopies {
    /// How many album names the dialog or a notification lists; the rest are an ellipsis.
    private static let namesShown = 4
    /// The copies asked about in this run of the app. One still waiting at the next start is asked again.
    private static var asked = Set<UInt64>()

    /// Ask what waits for an answer, and tell what came of albums. Called at each change signal.
    static func follow() async {
        let directory = AppGroup.libraryDirectory.path
        for notice in takeFolderNotices(directory: directory) {
            await tell(notice)
        }
        for question in folderQuestions(directory: directory) where !asked.contains(question.copy) {
            asked.insert(question.copy)
            ask(question)
        }
    }

    /// Pass an answer to the extension, and wake it, so the albums do not wait for the next change signal.
    static func answer(copy: UInt64, albums: Bool) {
        logger.notice("a folder copied into Upload: \(albums ? "albums wanted" : "the timeline only", privacy: .public)")
        answerFolderQuestion(directory: AppGroup.libraryDirectory.path, copy: copy, makeAlbums: albums)
        Task { try? await NSFileProviderManager(for: FinderLocation.domain)?.signalEnumerator(for: .workingSet) }
    }

    private static func names(_ names: [String]) -> String {
        (Array(names.prefix(namesShown)) + (names.count > namesShown ? ["…"] : [])).joined(separator: ", ")
    }

    /// Whether the app may post notifications, asking macOS the first time.
    private static func allowed() async -> Bool {
        let center = UNUserNotificationCenter.current()
        switch await center.notificationSettings().authorizationStatus {
        case .notDetermined:
            return (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
        case .denied:
            return false
        default:
            return true
        }
    }

    /// The question as a dialog, asked from the run loop, as the sign-out question is, so the menu keeps answering
    /// under it; a second one waits for the first. Neither button answers to Return: a dialog that comes up while
    /// someone types elsewhere is answered by a click.
    private static func ask(_ question: FolderQuestion) {
        var lines = [tr("Photos and videos in the timeline: {count}", ["count": String(question.uploaded)]),
                     tr("Albums it can make: {albums}", ["albums": names(question.albums)])]
        if question.notUploaded > 0 {
            lines.append(tr("Files not uploaded: {count}", ["count": String(question.notUploaded)]))
        }
        let (title, copy) = (tr("Uploaded to Immich: {folder}", ["folder": question.folder]), question.copy)
        RunLoop.main.perform {
            MainActor.assumeIsolated {
                let alert = NSAlert()
                alert.messageText = title
                alert.informativeText = lines.joined(separator: "\n")
                alert.addButton(withTitle: tr("Make Albums")).keyEquivalent = ""
                alert.addButton(withTitle: tr("Timeline Only")).keyEquivalent = ""
                AppDelegate.shared?.bringForward()
                AppDelegate.shared?.floatUntilUsed(alert.window)
                answer(copy: copy, albums: alert.runModal() == .alertFirstButtonReturn)
            }
        }
    }

    private static func tell(_ notice: FolderNotice) async {
        guard await allowed() else { return }
        let content = UNMutableNotificationContent()
        switch notice {
        case let .albumsMade(folder, albums):
            content.title = tr("Albums made from {folder}", ["folder": folder])
            content.body = names(albums)
        case let .albumsNotAllowed(folder, permission):
            content.title = tr("No albums made from {folder}", ["folder": folder])
            content.body = tr("Immich does not let this account make albums ({permission}). The photos and videos are in the timeline.",
                              ["permission": permission])
        }
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        try? await UNUserNotificationCenter.current().add(request)
    }
}

// Folders copied into Upload (ARCHITECTURE.md, "Folders copied into Upload"). The extension follows each copy and,
// once it has settled, leaves its question in the library's directory, since it cannot show anything itself; the app
// asks it as a notification with two buttons, or as an alert where the app's notifications are off, and passes the
// answer back. It also tells what came of the albums.

import AppKit
import FileProvider
import UserNotifications

@MainActor
enum FolderCopies {
    private static let category = "folder-copy"
    private static let makeAlbums = "make-albums"
    private static let timelineOnly = "timeline-only"
    /// How many album names a notification lists; the rest are an ellipsis.
    private static let namesShown = 4
    /// The copies asked about in this run of the app. One still waiting at the next start is asked again.
    private static var asked = Set<UInt64>()

    /// The question's two buttons; dismissing it answers too: the timeline only.
    static func register() {
        let albums = UNNotificationAction(identifier: makeAlbums, title: tr("Make Albums"))
        let timeline = UNNotificationAction(identifier: timelineOnly, title: tr("Timeline Only"))
        let question = UNNotificationCategory(identifier: category, actions: [albums, timeline], intentIdentifiers: [],
                                              options: [.customDismissAction])
        UNUserNotificationCenter.current().setNotificationCategories([question])
    }

    /// Ask what waits for an answer, and tell what came of albums. Called at each change signal.
    static func follow() async {
        let directory = AppGroup.libraryDirectory.path
        for notice in takeFolderNotices(directory: directory) {
            await tell(notice)
        }
        for question in folderQuestions(directory: directory) where !asked.contains(question.copy) {
            asked.insert(question.copy)
            await ask(question)
        }
    }

    /// The answer a notification's response gives: a button, or dismissing it; `nil` for a click on the notification
    /// itself, which answers nothing.
    nonisolated static func albums(for action: String) -> Bool? {
        switch action {
        case makeAlbums: return true
        case timelineOnly, UNNotificationDismissActionIdentifier: return false
        default: return nil
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

    private static func ask(_ question: FolderQuestion) async {
        let title = tr("Uploaded to Immich: {folder}", ["folder": question.folder])
        var lines = [tr("Photos and videos in the timeline: {count}", ["count": String(question.uploaded)]),
                     tr("Albums it can make: {albums}", ["albums": names(question.albums)])]
        if question.notUploaded > 0 {
            lines.append(tr("Files not uploaded: {count}", ["count": String(question.notUploaded)]))
        }
        guard await allowed() else {
            // The app's notifications are off: the same question as an alert, asked from the run loop, as the
            // sign-out question is, so the menu keeps answering under it.
            let copy = question.copy
            RunLoop.main.perform { MainActor.assumeIsolated { askInAlert(title: title, lines: lines, copy: copy) } }
            return
        }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = lines.joined(separator: "\n")
        content.categoryIdentifier = category
        content.userInfo = ["copy": String(question.copy)]
        let request = UNNotificationRequest(identifier: "folder-copy-\(question.copy)", content: content, trigger: nil)
        do {
            try await UNUserNotificationCenter.current().add(request)
        } catch {
            logger.error("the question about a copied folder was not shown: \(String(describing: error), privacy: .public)")
        }
    }

    private static func askInAlert(title: String, lines: [String], copy: UInt64) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = lines.joined(separator: "\n")
        alert.addButton(withTitle: tr("Make Albums"))
        alert.addButton(withTitle: tr("Timeline Only"))
        AppDelegate.shared?.bringForward()
        AppDelegate.shared?.floatUntilUsed(alert.window)
        answer(copy: copy, albums: alert.runModal() == .alertFirstButtonReturn)
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

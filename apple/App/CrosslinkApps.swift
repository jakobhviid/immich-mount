// The other apps that take part in immich-crosslink (ROADMAP.md, "Open in Immich, and links between apps", step 3):
// found through the system (`NSWorkspace.urlsForApplications(toOpen:)` and each bundle's `ImmichCrosslink`) at launch,
// when another app launches, when a setting of theirs changes, and before Show in Another App… sends. The bridge
// decides which Finder offers (`crosslinkTargets`), and no app is offered by name; the offers are kept in the App Group
// for the extension, which marks the items, and the system reads the items again when they change. An app is named and
// drawn by the system (its bundle's display name and icon), never by anything in a link or an announcement.

import AppKit
import FileProvider
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class CrosslinkApps: NSObject, ObservableObject {
    static let shared = CrosslinkApps()

    /// An app the Settings window lists, with its switch.
    struct Listed: Identifiable {
        let bundleId: String
        let name: String
        let icon: NSImage
        let shown: Bool
        /// Seen before and not found now: listed by its remembered name.
        let installed: Bool
        var id: String { bundleId }
    }

    /// The apps seen, kept per bundle id with their names and when last seen (the app's state, not a setting).
    static var seenFile: URL { AppGroup.container.appendingPathComponent("crosslink-seen.json") }

    @Published private(set) var listed: [Listed] = []
    private var targets: CrosslinkTargets?
    private var following = false
    private var pendingRefresh: DispatchWorkItem?

    /// Look now, and again whenever another app launches.
    func follow() {
        refresh()
        guard !following else { return }
        following = true
        NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.didLaunchApplicationNotification, object: nil,
                                                          queue: .main) { _ in
            MainActor.assumeIsolated { CrosslinkApps.shared.refreshSoon() }
        }
    }

    /// Several apps may launch at once, at login: one look after them.
    private func refreshSoon() {
        pendingRefresh?.cancel()
        let refresh = DispatchWorkItem { [weak self] in self?.refresh() }
        pendingRefresh = refresh
        DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: refresh)
    }

    /// Find the apps, and keep the offers for the extension; when they changed, have the system read the items again,
    /// so the actions follow.
    func refresh() {
        let targets = crosslinkTargets(found: Self.found(), config: AppGroup.configText)
        self.targets = targets
        let installed = targets.listed.map { app in
            Listed(bundleId: app.bundleId, name: Self.name(ofAppAt: app.path), icon: NSWorkspace.shared.icon(forFile: app.path),
                   shown: app.shown, installed: true)
        }
        // Remembered: an app away keeps its switch for 30 days (Immich Crosslink, "Participants that come and go").
        try? FileManager.default.createDirectory(at: AppGroup.container, withIntermediateDirectories: true)
        let remembered = rememberApps(file: Self.seenFile.path, installed: installed.map { SeenApp(bundleId: $0.bundleId, name: $0.name) },
                                      config: AppGroup.configText)
        let absent = remembered.absent.map { app in
            Listed(bundleId: app.bundleId, name: app.name, icon: NSWorkspace.shared.icon(for: .application), shown: app.shown, installed: false)
        }
        listed = (installed + absent).sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        logger.info("Settings lists \(installed.count) apps installed and \(absent.count) not installed: \(absent.map(\.bundleId).joined(separator: ", "), privacy: .public)")
        for bundleId in remembered.forgotten {
            Self.dropSwitch(bundleId)
        }
        let offers = OfferedApps(apps: targets.offered.map(Self.kept))
        guard offers != OfferedApps.load() else { return }
        do {
            try offers.save()
        } catch {
            logger.error("the apps of Show in were not kept: \(String(describing: error), privacy: .public)")
            return
        }
        logger.notice("Show in Another App… offers \(offers.apps.count) apps")
        Task {
            do {
                try await NSFileProviderManager(for: FinderLocation.domain)?.reimportItems(below: .rootContainer)
            } catch {
                logger.info("Finder was not asked to read the items again: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// Forget an app not found, and its switch with it: should it come back, it is new, and on (Immich Crosslink,
    /// "Participants that come and go").
    func forget(_ bundleId: String) {
        forgetApp(file: Self.seenFile.path, bundleId: bundleId)
        Self.dropSwitch(bundleId)
        refresh()
    }

    private static func dropSwitch(_ bundleId: String) {
        guard let updated = try? setAppShown(config: AppGroup.configText, bundleId: bundleId, shown: true),
              updated != AppGroup.configText else { return }
        try? updated.write(to: AppGroup.configFile, atomically: true, encoding: .utf8)
    }

    private static func kept(_ app: OfferedApp) -> OfferedApps.App {
        OfferedApps.App(bundleId: app.bundleId, path: app.path, kinds: app.kinds)
    }

    /// The apps registered for the scheme, with their announcements.
    private static func found() -> [FoundApp] {
        guard let probe = URL(string: "immich-crosslink:///photos") else { return [] }
        return NSWorkspace.shared.urlsForApplications(toOpen: probe).compactMap { url in
            // A sandboxed app reads only some places: an app outside Applications may not be read (immich-crosslink,
            // "Finding participants").
            guard let bundle = Bundle(url: url), let bundleId = bundle.bundleIdentifier, Self.canStart(url) else {
                logger.info("an app that takes crosslinks could not be read: \(url.path, privacy: .public)")
                return nil
            }
            let announcement = bundle.infoDictionary?["ImmichCrosslink"] as? [String: Any]
            logger.info("found \(bundleId, privacy: .public) at \(url.path, privacy: .public), announcing \(announcement == nil ? "nothing" : "itself", privacy: .public)")
            return FoundApp(bundleId: bundleId, path: url.path, takesCrosslinks: true,
                            version: announcement?["Version"] as? String, shows: announcement?["Shows"] as? [String] ?? [])
        }
    }

    /// An app's name as the system gives it: its bundle's display name, else its file's name.
    static func name(ofAppAt path: String) -> String {
        let bundle = Bundle(path: path)
        let display = bundle?.localizedInfoDictionary?["CFBundleDisplayName"] ?? bundle?.infoDictionary?["CFBundleDisplayName"]
        if let display = display as? String, !display.isEmpty {
            return display
        }
        return FileManager.default.displayName(atPath: path).replacingOccurrences(of: ".app", with: "")
    }

    // MARK: Show in Another App…

    /// Finder's Show in Another App…, which the extension handed over: `url` carries only the token of the request it
    /// kept in the App Group. Any page can open the app's scheme, so a token that names no fresh request does nothing;
    /// the link is made here from the item, and sent only to an app offered for that item's kind: the one there is
    /// straight away, or the one the user chooses of several (Immich Crosslink, "Sending").
    func perform(_ url: URL) {
        guard let token = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first(where: { $0.name == "token" })?.value,
              let request = ActionRequest.take(token) else {
            logger.notice("an address of the app's own scheme named no request of Finder's; nothing is sent")
            return
        }
        guard let kind = itemKind(identifier: request.identifier), let account = Account.load() else {
            logger.error("a Show in request named no item with a link")
            return
        }
        // Read again: an app removed since Finder showed the action is not offered.
        refresh()
        let apps = (targets?.offered ?? []).filter { $0.kinds.contains(kind) }
        guard !apps.isEmpty else {
            logger.notice("Show in Another App…: no app shows \(kind, privacy: .public)")
            Self.tell(tr("No other app that takes part is installed."))
            return
        }
        Task {
            guard let link = await itemCrosslink(server: account.server, credential: account.credential, identifier: request.identifier),
                  let url = URL(string: link) else { return }
            guard apps.count > 1 else {
                send(url, toAppAt: apps[0].path)
                return
            }
            // The item's name, as Finder shows it, for the question; none if the system cannot tell at once.
            let place = try? await NSFileProviderManager(for: FinderLocation.domain)?
                .getUserVisibleURL(for: NSFileProviderItemIdentifier(request.identifier))
            let item = place.map { FileManager.default.displayName(atPath: $0.path) }
            // A modal alert run inside a task holds the main queue (AppDelegate's `confirmSignOut`).
            RunLoop.main.perform { MainActor.assumeIsolated { self.choose(among: apps, for: url, item: item) } }
        }
    }

    /// The app last chosen in Show in Which App?, whose button Return presses next time (the app's own state).
    private static let lastChoiceKey = "ShowInLastApp"
    /// The most apps asked about as buttons; with more, an alert of buttons stops being one, and they are a list.
    private static let mostButtons = 6

    /// Ask which app shows the item: a button per app, by the name and icon the system gives it, in the settings'
    /// order, and Cancel; Return presses the app chosen last time. The link goes to the app clicked, alone.
    private func choose(among apps: [OfferedApp], for link: URL, item: String?) {
        let choices = apps.map { (bundleId: $0.bundleId, path: $0.path, name: Self.name(ofAppAt: $0.path)) }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        let alert = NSAlert()
        alert.messageText = tr("Show in Which App?")
        if let item {
            alert.informativeText = tr("Choose an app to show “{item}” in.", ["item": item])
        }
        let chosen: String?
        if choices.count > Self.mostButtons {
            let choice = AppChoice(apps: choices.map { AppChoice.Choice(path: $0.path, name: $0.name, icon: NSWorkspace.shared.icon(forFile: $0.path)) })
            alert.addButton(withTitle: tr("Show"))
            alert.addButton(withTitle: tr("Cancel"))
            let list = NSHostingView(rootView: AppChoiceList(choice: choice))
            list.frame = NSRect(x: 0, y: 0, width: 280, height: 200)
            alert.accessoryView = list
            chosen = run(alert) == .alertFirstButtonReturn ? choice.selection : nil
        } else {
            let last = UserDefaults.standard.string(forKey: Self.lastChoiceKey)
            for choice in choices {
                let button = alert.addButton(withTitle: choice.name)
                let icon = NSWorkspace.shared.icon(forFile: choice.path)
                icon.size = NSSize(width: 16, height: 16)
                button.image = icon
                button.imagePosition = .imageLeading
                button.keyEquivalent = ""
            }
            alert.addButton(withTitle: tr("Cancel")).keyEquivalent = "\u{1b}"
            let preferred = choices.firstIndex { $0.bundleId == last } ?? 0
            alert.buttons[preferred].keyEquivalent = "\r"
            let index = run(alert).rawValue - NSApplication.ModalResponse.alertFirstButtonReturn.rawValue
            chosen = choices.indices.contains(index) ? choices[index].path : nil
        }
        guard let path = chosen, let picked = choices.first(where: { $0.path == path }) else {
            logger.notice("Show in Which App?: cancelled")
            return
        }
        UserDefaults.standard.set(picked.bundleId, forKey: Self.lastChoiceKey)
        send(link, toAppAt: picked.path)
    }

    /// Run the question in front of Finder, which it was asked from.
    private func run(_ alert: NSAlert) -> NSApplication.ModalResponse {
        logger.notice("Show in Which App?: asking")
        AppDelegate.shared?.bringForward()
        AppDelegate.shared?.floatUntilUsed(alert.window)
        return alert.runModal()
    }

    /// Send `link` to the app at `path` alone. An app removed since the list was read, or one the system cannot open,
    /// is told about in an alert, and the list is read again (Immich Crosslink, "Sending").
    func send(_ link: URL, toAppAt path: String) {
        let app = URL(fileURLWithPath: path)
        let name = Self.name(ofAppAt: path)
        guard Self.canStart(app) else {
            logger.notice("\(app.lastPathComponent, privacy: .public) is no longer installed")
            Self.tell(tr("{app} is no longer installed.", ["app": name]))
            refresh()
            return
        }
        NSWorkspace.shared.open([link], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration()) { _, error in
            guard let error else {
                logger.notice("sent a link to \(app.lastPathComponent, privacy: .public)")
                return
            }
            logger.error("\(app.lastPathComponent, privacy: .public) did not take the link: \(String(describing: error), privacy: .public)")
            let message = tr("Immich Mount could not open {app}: {error}", ["app": name, "error": error.localizedDescription])
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    Self.tell(message)
                    CrosslinkApps.shared.refresh()
                }
            }
        }
    }

    /// Whether the app at `url` can start: its program is there, and executable. Read from the disk each time, since
    /// a bundle once read is kept by the system.
    private static func canStart(_ url: URL) -> Bool {
        guard let info = NSDictionary(contentsOf: url.appendingPathComponent("Contents/Info.plist")),
              let executable = info["CFBundleExecutable"] as? String else { return false }
        return FileManager.default.isExecutableFile(atPath: url.appendingPathComponent("Contents/MacOS/\(executable)").path)
    }

    /// An alert titled with the protocol's name, as Immich Gallery tells the same.
    private static func tell(_ message: String) {
        RunLoop.main.perform {
            MainActor.assumeIsolated {
                let alert = NSAlert()
                alert.messageText = "Immich Crosslink"
                alert.informativeText = message
                AppDelegate.shared?.bringForward()
                AppDelegate.shared?.floatUntilUsed(alert.window)
                alert.runModal()
            }
        }
    }
}

/// The apps Show in Which App? lists when there are more than its buttons, and the one chosen, by its path.
@MainActor
final class AppChoice: ObservableObject {
    struct Choice: Identifiable {
        let path: String
        let name: String
        let icon: NSImage
        var id: String { path }
    }

    let apps: [Choice]
    @Published var selection: String?

    init(apps: [Choice]) {
        self.apps = apps
        selection = apps.first?.path
    }
}

/// The list in Show in Which App?: each app by its icon and name. A double-click, or Return, shows the item in it.
private struct AppChoiceList: View {
    @ObservedObject var choice: AppChoice

    var body: some View {
        List(choice.apps, selection: $choice.selection) { app in
            Label {
                Text(verbatim: app.name)
            } icon: {
                Image(nsImage: app.icon).resizable().frame(width: 20, height: 20)
            }
            .tag(app.path)
        }
        .contextMenu(forSelectionType: String.self) { _ in } primaryAction: { paths in
            guard let path = paths.first else { return }
            choice.selection = path
            NSApp.stopModal(withCode: .alertFirstButtonReturn)
        }
    }
}

// The other apps that take part in immich-crosslink (ROADMAP.md, "Open in Immich, and links between apps", step 3):
// found through the system (`NSWorkspace.urlsForApplications(toOpen:)` and each bundle's `ImmichCrosslink`) at launch,
// when another app launches, when a setting of theirs changes, and before Show in Another App…'s menu. The bridge
// decides which Finder offers (`crosslinkTargets`); the offers are kept in the App Group for the extension, which marks
// the items, and the system reads the items again when they change. An app is named and drawn by the system, never by
// anything in a link or an announcement.

import AppKit
import FileProvider
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
        /// Seen before and not installed now: listed by its remembered name, with a Forget button.
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
        #if DEBUG
        let development = true
        #else
        let development = false
        #endif
        let targets = crosslinkTargets(found: Self.found(), config: AppGroup.configText, development: development)
        self.targets = targets
        let installed = targets.listed.map { app in
            Listed(bundleId: app.bundleId, name: FileManager.default.displayName(atPath: app.path).replacingOccurrences(of: ".app", with: ""),
                   icon: NSWorkspace.shared.icon(forFile: app.path), shown: app.shown, installed: true)
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
        let offers = OfferedApps(gallery: targets.gallery.map(Self.kept), others: targets.others.map(Self.kept))
        guard offers != OfferedApps.load() else { return }
        do {
            try offers.save()
        } catch {
            logger.error("the apps of Show in were not kept: \(String(describing: error), privacy: .public)")
            return
        }
        logger.notice("Show in offers \(offers.gallery?.bundleId ?? "no gallery", privacy: .public) and \(offers.others.count) other apps")
        Task {
            do {
                try await NSFileProviderManager(for: FinderLocation.domain)?.reimportItems(below: .rootContainer)
            } catch {
                logger.info("Finder was not asked to read the items again: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// Forget an app that is not installed, and its switch with it: should it come back, it is new again, and on.
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
        OfferedApps.App(bundleId: app.bundleId, path: app.path, kinds: app.kinds, ownScheme: app.ownScheme)
    }

    /// The apps registered for the scheme, and the known apps wherever they are, with their announcements.
    private static func found() -> [FoundApp] {
        guard let probe = URL(string: "immich-crosslink:///photos") else { return [] }
        let registered = NSWorkspace.shared.urlsForApplications(toOpen: probe)
        let takes = Set(registered.map(\.standardizedFileURL))
        let known = ["dev.igol.ImmichGallery", "dev.igol.ImmichGallery.Dev"].flatMap {
            NSWorkspace.shared.urlsForApplications(withBundleIdentifier: $0)
        }
        return (registered + known).compactMap { url in
            // A sandboxed app reads only some places: an app outside Applications may not be read (immich-crosslink,
            // "Finding participants").
            guard let bundle = Bundle(url: url), let bundleId = bundle.bundleIdentifier, Self.canStart(url) else {
                logger.info("an app that takes crosslinks could not be read: \(url.path, privacy: .public)")
                return nil
            }
            let announcement = bundle.infoDictionary?["ImmichCrosslink"] as? [String: Any]
            logger.info("found \(bundleId, privacy: .public) at \(url.path, privacy: .public), announcing \(announcement == nil ? "nothing" : "itself", privacy: .public)")
            return FoundApp(bundleId: bundleId, path: url.path, takesCrosslinks: takes.contains(url.standardizedFileURL),
                            version: announcement?["Version"] as? String, shows: announcement?["Shows"] as? [String] ?? [])
        }
    }

    // MARK: Show in Another App…

    /// A Finder Show in action the extension handed over: `url` carries only the token of the request it kept in the
    /// App Group. Any page can open the app's scheme, so a token that names no fresh request does nothing; the link is
    /// made here from the item, and sent only to an app offered for that item's kind.
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
        Task {
            switch request.action {
            case ActionRequest.showInApp:
                await showInApp(request.bundleId ?? "", identifier: request.identifier, kind: kind, account: account)
            case ActionRequest.showInAnotherApp:
                guard let link = await itemCrosslink(server: account.server, credential: account.credential, identifier: request.identifier) else { return }
                showMenu(link: link, kind: kind)
            default:
                logger.error("a Show in request of an unknown kind")
            }
        }
    }

    /// Show in Immich Gallery: to the app among those Finder was offering (as kept for the extension; one removed
    /// since is told about in `send`), when it is offered for the item.
    private func showInApp(_ bundleId: String, identifier: String, kind: String, account: Account) async {
        let offered = OfferedApps.load()
        guard let app = ([offered.gallery].compactMap { $0 } + offered.others).first(where: { $0.bundleId == bundleId }),
              offered.gallery?.bundleId != bundleId || offered.offersGallery(kind: kind, photoInPhotos: photoInPhotos(identifier: identifier) != nil),
              app.kinds.contains(kind) else {
            logger.error("Show in came for an app that is not offered on this item: \(bundleId, privacy: .public)")
            return
        }
        let link: String?
        if let scheme = app.ownScheme {
            link = photoInPhotos(identifier: identifier).map { "\(scheme)://photo/\($0)" }
        } else {
            link = await itemCrosslink(server: account.server, credential: account.credential, identifier: identifier)
        }
        guard let link, let url = URL(string: link) else { return }
        send(url, toAppAt: app.path)
    }

    /// The menu of the other apps that show the item's kind, at the pointer, as Finder's Show in Another App… asks.
    private func showMenu(link: String, kind: String) {
        refresh()
        let apps = (targets?.others ?? []).filter { $0.kinds.contains(kind) }
        guard !apps.isEmpty else {
            logger.notice("Show in Another App…: no app shows \(kind, privacy: .public)")
            return
        }
        let menu = NSMenu()
        for app in apps {
            let name = FileManager.default.displayName(atPath: app.path).replacingOccurrences(of: ".app", with: "")
            let item = NSMenuItem(title: tr("Show in {app}", ["app": name]), action: #selector(send(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = [link, app.path]
            let icon = NSWorkspace.shared.icon(forFile: app.path)
            icon.size = NSSize(width: 16, height: 16)
            item.image = icon
            menu.addItem(item)
        }
        logger.notice("Show in Another App…: \(apps.count) apps for \(kind, privacy: .public)")
        // A menu bar app's menu tracks only once the app is active.
        NSApp.activate()
        menu.popUp(positioning: nil, at: NSEvent.mouseLocation, in: nil)
    }

    /// Send the crosslink to the app chosen, and to it alone.
    @objc private func send(_ item: NSMenuItem) {
        guard let pair = item.representedObject as? [String], pair.count == 2, let url = URL(string: pair[0]) else { return }
        send(url, toAppAt: pair[1])
    }

    /// Send `link` to the app at `path` alone. An app removed since the list was read, or one the system cannot open,
    /// is told about in an alert, and the list is read again (Immich Crosslink, "Sending").
    func send(_ link: URL, toAppAt path: String) {
        let app = URL(fileURLWithPath: path)
        let name = FileManager.default.displayName(atPath: path).replacingOccurrences(of: ".app", with: "")
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

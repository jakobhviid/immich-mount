// The other apps that take part in immich-crosslink (ROADMAP.md, "Open in Immich, and links between apps", step 3):
// found through the system (`NSWorkspace.urlsForApplications(toOpen:)` and each bundle's `ImmichCrosslink`) at launch,
// when another app launches, when a setting of theirs changes, and before Show in Another App…'s menu. The bridge
// decides which Finder offers (`crosslinkTargets`); the offers are kept in the App Group for the extension, which marks
// the items, and the system reads the items again when they change. An app is named and drawn by the system, never by
// anything in a link or an announcement.

import AppKit
import FileProvider

@MainActor
final class CrosslinkApps: NSObject, ObservableObject {
    static let shared = CrosslinkApps()

    /// An app the Settings window lists, with its switch.
    struct Listed: Identifiable {
        let bundleId: String
        let name: String
        let icon: NSImage
        let shown: Bool
        var id: String { bundleId }
    }

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
        listed = targets.listed.map { app in
            Listed(bundleId: app.bundleId, name: FileManager.default.displayName(atPath: app.path).replacingOccurrences(of: ".app", with: ""),
                   icon: NSWorkspace.shared.icon(forFile: app.path), shown: app.shown)
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
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
            guard let bundle = Bundle(url: url), let bundleId = bundle.bundleIdentifier else {
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

    /// The menu of the other apps that show what `link` names, at the pointer, as Finder's Show in Another App… asks
    /// through the app's own scheme. Any page could open that address too: the menu only offers to send a crosslink,
    /// and only on a click.
    func showMenu(for url: URL) {
        guard let link = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first(where: { $0.name == "link" })?.value,
              let kind = crosslinkKind(link: link) else {
            logger.error("Show in Another App… came without a valid crosslink")
            return
        }
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
        let app = URL(fileURLWithPath: pair[1])
        NSWorkspace.shared.open([url], withApplicationAt: app, configuration: NSWorkspace.OpenConfiguration()) { _, error in
            if let error {
                logger.error("\(app.lastPathComponent, privacy: .public) did not take the crosslink: \(String(describing: error), privacy: .public)")
            } else {
                logger.notice("sent a crosslink to \(app.lastPathComponent, privacy: .public)")
            }
        }
    }
}

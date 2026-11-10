// The app (ARCHITECTURE.md, "The macOS edition"): a menu bar app that signs in, adds the Finder location and shows
// status, and keeps changes flowing while it runs. Quitting it stops nothing but that; the extension serves Finder on
// its own.

import AppKit
import AuthenticationServices
import Combine
import FileProvider
import os
import ServiceManagement
import Sparkle
import SwiftUI
import UserNotifications

let logger = Logger(subsystem: "dev.hviid.ImmichMount", category: "app")

@main
struct ImmichMountApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    /// The menu bar item, there once the app runs from Applications.
    @State private var inMenuBar = Installation.isInApplications

    init() {
        startLogging()
        // `brew uninstall` signs out before it deletes the app, and never sees a window (`Uninstall`).
        if Uninstall.asked {
            Uninstall.run()
        }
    }

    var body: some Scene {
        MenuBarExtra(isInserted: $inMenuBar) {
            MenuBarMenu()
                .environmentObject(delegate.model)
        } label: {
            // The app icon as one solid shape, its lens and base cut out, as menu bar icons are drawn; blue in the
            // Dev build, so it stands apart from the release's beside it.
            #if DEBUG
            Image(nsImage: devMenuBarIcon)
            #else
            Image("MenuBarIcon")
            #endif
        }
        .commands {
            CommandGroup(replacing: .appInfo) {
                Button("About Immich Mount") { showAbout() }
                if Installation.updates {
                    if Installation.updates {
            Button("Check for Updates…") { AppDelegate.shared?.updater.checkForUpdates(nil) }
        }
                }
            }
            CommandGroup(replacing: .appSettings) {
                Button("Settings…") { AppDelegate.shared?.showSettings() }
                    .keyboardShortcut(",")
            }
        }
    }
}

/// Opens the sign-in window while it is wanted and Settings when asked, and gives the app a Dock icon and its menus
/// while a window is open: at other times it lives in the menu bar only, as Mac apps that keep something in sync do.
/// Sparkle calls its user driver's delegate on the main thread, which its protocol does not declare: hence
/// `@preconcurrency`.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, SPUUpdaterDelegate,
                         @preconcurrency SPUStandardUserDriverDelegate, UNUserNotificationCenterDelegate {
    private(set) static weak var shared: AppDelegate?
    let model = AppModel()
    /// Sparkle, which reads the download page's appcast and replaces the app in place, so it stays in Applications,
    /// where Finder's location runs it from. It starts only there, and never in the Dev build (`Installation.updates`).
    private(set) lazy var updater = SPUStandardUpdaterController(startingUpdater: Installation.updates,
                                                                  updaterDelegate: self, userDriverDelegate: self)
    private var window: NSWindow?
    private var settingsWindow: NSWindow?
    private var setUp: AnyCancellable?
    /// Whether macOS started the app as a login item, read from the launch event before it is gone.
    private var launchedAtLogin = false
    /// Whether macOS started the app to open a link another app sent (`ReceivedLinks`): it shows the link's item, and
    /// no window of its own. Told by the launch event, or by the link arriving while the app still sets up.
    private var launchedForLink = false
    /// Whether the app has decided which window, if any, it opens at launch.
    private var finishedLaunching = false
    /// Whether this is the first run of this build: after an update (Sparkle's, or Homebrew's), the app comes back as
    /// it was, in the menu bar alone. Told by the build the app last ran, which it keeps; an update from a build that
    /// kept none counts too, as a first install is signed out and opens the sign-in window anyway.
    private var launchedAfterUpdate = false
    private static let lastBuildKey = "lastBuild"

    override init() {
        super.init()
        AppDelegate.shared = self
    }

    func applicationWillFinishLaunching(_ notification: Notification) {
        let launch = NSAppleEventManager.shared().currentAppleEvent
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? ""
        launchedAfterUpdate = UserDefaults.standard.string(forKey: Self.lastBuildKey) != build
        UserDefaults.standard.set(build, forKey: Self.lastBuildKey)
        launchedAtLogin = launch?.eventID == kAEOpenApplication
            && launch?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue == keyAELaunchedAsLogInItem
        launchedForLink = launch?.eventClass == AEEventClass(kInternetEventClass) && launch?.eventID == AEEventID(kAEGetURL)
    }

    /// Links another app sent, through the `immich-crosslink` scheme the Info.plist declares.
    func application(_ application: NSApplication, open urls: [URL]) {
        if !finishedLaunching {
            launchedForLink = true
        }
        let ownScheme = Bundle.main.bundleIdentifier?.lowercased()
        for url in urls {
            if url.scheme?.lowercased() == ownScheme {
                // Finder's Show in Another App…, from the extension.
                CrosslinkApps.shared.showMenu(for: url)
            } else {
                Task { await ReceivedLinks.show(url) }
            }
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        _ = updater
        followWindows()
        // The other apps that take crosslinks, for Finder's Show in actions.
        CrosslinkApps.shared.follow()
        // What came of a copied folder's albums is told in notifications, shown even while the app is in front.
        UNUserNotificationCenter.current().delegate = self
        // Back from Finder or System Settings, the menu and the window know at once whether the location is on.
        NotificationCenter.default.addObserver(forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated {
                // In front by right now, the windows need not float (`floatUntilUsed`).
                for window in NSApp.windows where window.level == .floating {
                    window.level = .normal
                }
            }
            Task { @MainActor in await AppDelegate.shared?.model.refreshState() }
        }
        // The sign-in window is done once there is an account and its location is on.
        setUp = model.$server.combineLatest(model.$locationSwitchedOff)
            .receive(on: RunLoop.main)
            .sink { [weak self] server, switchedOff in
                if server != nil && !switchedOff && Installation.isInApplications {
                    self?.window?.close()
                }
            }
        // At login, and back from an update, the app only takes its place in the menu bar. Opened by someone, it shows
        // what is left to do, or Settings.
        Task {
            await model.refreshState()
            defer { finishedLaunching = true }
            if launchedForLink && model.server != nil {
                return
            } else if !Installation.isInApplications || model.server == nil || model.locationSwitchedOff {
                if !launchedAtLogin || model.server == nil { showWindow() }
            } else if !launchedAtLogin && !launchedAfterUpdate {
                showSettings()
            }
        }
        #if DEBUG
        Task { await model.followLaunchArguments() }
        // `-settingsPane <0, 1 or 2>`: open Settings on that pane, to check its layout without clicking.
        if let pane = UserDefaults.standard.string(forKey: "settingsPane").flatMap(Int.init) {
            UserDefaults.standard.set(pane, forKey: "SettingsPane")
            showSettings()
        }
        #endif
    }

    /// A notification shows even while the app is in front (its Settings window open, say).
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                            withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .list])
    }

    /// A notification clicked: the one saying Immich Crosslink is off opens Settings where it is switched on.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                            withCompletionHandler completionHandler: @escaping () -> Void) {
        let opensSettings = response.notification.request.content.userInfo[ReceivedLinks.opensSettings] as? Bool == true
        Task { @MainActor in
            if opensSettings { AppDelegate.shared?.showSettings(on: .general) }
            completionHandler()
        }
    }

    /// Sparkle's scheduled update alerts come without a window of the app's in front: they bring the app forward, so
    /// they are not left behind other apps, as a menu bar app's would be.
    var supportsGentleScheduledUpdateReminders: Bool { true }

    /// The update's windows are gone: the menu bar alone, unless one of the app's own is open (`leaveDockIfAlone`).
    func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: Error?) {
        leaveDockIfAlone()
    }

    func standardUserDriverWillHandleShowingUpdate(_ handleShowingUpdate: Bool, forUpdate update: SUAppcastItem,
                                                   state: SPUUserUpdateState) {
        if handleShowingUpdate {
            bringForward()
        }
    }

    /// Signing out removes the location and what was downloaded, so it is asked first, saying which files in Upload
    /// have not reached Immich: those are kept (`FinderLocation.remove`).
    func confirmSignOut() {
        // Asked for at the click, while macOS still grants it: after the wait below it may refuse.
        bringForward()
        Task {
            let waiting = await FinderLocation.waitingUploads()
            // A modal alert run inside a task holds the main queue, where the second activation `bringForward` asks
            // for waits, so the alert stayed behind other apps' windows and the menu stopped answering (2.17.3 and
            // 2.17.4 on macOS 27). From the run loop, the queue keeps running under the alert.
            RunLoop.main.perform { MainActor.assumeIsolated { self.askToSignOut(waiting: waiting) } }
        }
    }

    private func askToSignOut(waiting: Int) {
        let alert = NSAlert()
        alert.messageText = tr("Sign out of Immich Mount?")
        var text = tr("Immich Mount leaves Finder, together with the photos and videos downloaded to this Mac. Your library on the server stays as it is.")
        if waiting > 0 {
            text += "\n\n" + tr("Files in Upload that have not reached Immich: {count}. They stay on this Mac, and Finder shows where once you have signed out.",
                                ["count": String(waiting)])
        }
        alert.informativeText = text
        alert.addButton(withTitle: tr("Sign Out")).hasDestructiveAction = true
        alert.addButton(withTitle: tr("Cancel"))
        bringForward()
        floatUntilUsed(alert.window)
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        Task {
            do {
                try await model.signOut()
            } catch {
                RunLoop.main.perform { MainActor.assumeIsolated { self.reportStillSignedIn(error) } }
            }
        }
    }

    private func reportStillSignedIn(_ error: Error) {
        let failure = NSAlert()
        failure.messageText = tr("Still signed in")
        failure.informativeText = tr("Finder's location could not be removed, so Immich Mount stays signed in: {error}",
                                     ["error": describe(error)])
        bringForward()
        floatUntilUsed(failure.window)
        failure.runModal()
    }

    /// Opened again from Finder, Launchpad or Spotlight while it runs: what is left to do, or Settings.
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if !Installation.isInApplications || model.server == nil || model.locationSwitchedOff {
            showWindow()
        } else {
            showSettings()
        }
        return false
    }

    /// The sign-in window: signing in, then switching the location on. It takes each step's height, as macOS's own
    /// setup windows do, keeping its top edge. The window is resized after its layout pass, not by the hosting
    /// view's constraints: a window that follows its SwiftUI content during layout is laid out again without end as
    /// the steps change, which stops the app (seen on macOS 27).
    func showWindow() {
        logger.notice("the sign-in window is shown (\(self.window == nil ? "new" : "again", privacy: .public))")
        if window == nil {
            let content = ContentView(heightChanged: { [weak self] height in
                DispatchQueue.main.async { self?.fitWindow(to: height) }
            })
            let hosting = NSHostingController(rootView: content.environmentObject(model))
            hosting.sizingOptions = []
            let window = NSWindow(contentViewController: hosting)
            window.title = "Immich Mount"
            window.styleMask = [.titled, .closable, .miniaturizable]
            // The hosting view's fitting size can be zero (seen on macOS 27.2, after signing out): the window was then
            // shown at 0 by 0 points, there and invisible. It takes the content's width, and a height until the step
            // tells its own.
            let fitting = hosting.view.fittingSize
            window.setContentSize(NSSize(width: max(fitting.width, ContentView.width), height: max(fitting.height, 320)))
            window.isReleasedWhenClosed = false
            window.isRestorable = false
            window.center()
            self.window = window
        } else if let window, window.contentRect(forFrameRect: window.frame).width < ContentView.width {
            // One made at no size before stays invisible: it takes its width again.
            let content = window.contentRect(forFrameRect: window.frame)
            window.setContentSize(NSSize(width: ContentView.width, height: max(content.height, 320)))
            window.center()
        }
        present(window)
    }

    private func fitWindow(to height: CGFloat) {
        guard let window, height > 0 else { return }
        let content = window.contentRect(forFrameRect: window.frame)
        guard abs(content.height - height) > 0.5 || content.width < ContentView.width else { return }
        let width = max(content.width, ContentView.width)
        var frame = window.frameRect(forContentRect: NSRect(x: content.minX, y: content.minY, width: width, height: height))
        frame.origin.y = window.frame.maxY - frame.height
        window.setFrame(frame, display: true, animate: window.isVisible)
    }

    /// Settings, in a window of the app's own, so the menu, the app menu and reopening all reach it: a toolbar of
    /// panes, titled by the pane shown, opening on the pane last used, and sized by each pane, so it neither
    /// resizes nor minimises, as Apple's Human Interface Guidelines ask for a settings window.
    func showSettings(on pane: SettingsPane? = nil) {
        if let pane {
            UserDefaults.standard.set(pane.rawValue, forKey: SettingsTabController.lastPaneKey)
            (settingsWindow?.contentViewController as? SettingsTabController)?.selectedTabViewItemIndex = pane.rawValue
        }
        if settingsWindow == nil {
            let panes = SettingsTabController(model: model)
            let window = NSWindow(contentViewController: panes)
            window.styleMask = [.titled, .closable]
            window.toolbarStyle = .preference
            window.isReleasedWhenClosed = false
            window.isRestorable = false
            window.center()
            settingsWindow = window
        }
        present(settingsWindow)
    }

    private func present(_ window: NSWindow?) {
        guard let window else { return }
        bringForward()
        window.orderFrontRegardless()
        window.makeKey()
        floatUntilUsed(window)
    }

    /// Keep a window above other apps' until the app is active. macOS may refuse a menu bar app's activation
    /// (cooperative activation, macOS 14 and later): the sign-out question then waited behind another app's
    /// full-screen window, and the menu, held by it, stopped answering (seen on macOS 27). Set once the window is up,
    /// as a modal alert sets its own level as it starts; back to normal when the app becomes active.
    func floatUntilUsed(_ window: NSWindow) {
        DispatchQueue.main.async {
            guard !NSApp.isActive else { return }
            window.level = .floating
            window.orderFrontRegardless()
        }
    }

    /// Make the app the active one, with its Dock icon and menus. The policy change takes effect on the next turn of
    /// the run loop, and macOS refuses an activation asked for before it (activation is cooperative since macOS 14),
    /// which left windows behind the other apps' ones; so it is asked for again once the change has landed.
    func bringForward() {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
        DispatchQueue.main.async {
            NSApp.activate()
            NSApp.keyWindow?.orderFrontRegardless()
        }
    }

    /// The Dock icon and the menus while any titled window (this one, Settings, About, Sparkle's) is open, none after
    /// the last.
    private func followWindows() {
        let center = NotificationCenter.default
        center.addObserver(forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main) { notification in
            guard let window = notification.object as? NSWindow, window.styleMask.contains(.titled) else { return }
            MainActor.assumeIsolated { _ = NSApp.setActivationPolicy(.regular) }
        }
        center.addObserver(forName: NSWindow.willCloseNotification, object: nil, queue: .main) { [weak self] notification in
            guard let closing = notification.object as? NSWindow, closing.styleMask.contains(.titled) else { return }
            MainActor.assumeIsolated { self?.leaveDockIfAlone(closing: closing) }
        }
    }

    /// The menu bar alone once no titled window is left, but not while Sparkle updates: it closes each of its windows
    /// before it opens the next, and the app, gone from the Dock in between, lost its place in front, so the next
    /// window (the download's, "Ready to Install") opened behind other apps' ones. The update's end asks again.
    private func leaveDockIfAlone(closing: NSWindow? = nil) {
        guard Installation.isInApplications, !updater.updater.sessionInProgress else { return }
        let others = NSApp.windows.filter { $0 !== closing && $0.isVisible && $0.styleMask.contains(.titled) }
        if others.isEmpty {
            NSApp.setActivationPolicy(.accessory)
        }
    }
}

/// The Settings window's panes, switched by its toolbar (`SettingsPane`). Each pane tells its content's height once
/// laid out, and again when it changes (signing in or out, a refusal); the window takes the height of the pane shown,
/// keeping its top edge. Resizing the window changes no pane's content, so this ends; a window that followed its
/// SwiftUI content's layout instead was laid out again without end on macOS 27.
final class SettingsTabController: NSTabViewController {
    /// The pane last chosen, in the app's own defaults.
    static let lastPaneKey = "SettingsPane"
    private static let width: CGFloat = 520
    /// The tallest the window grows for a pane; beyond it the pane scrolls.
    private static let tallest: CGFloat = 720

    /// Each pane's content height, as it last told it.
    private var heights: [SettingsPane: CGFloat] = [:]
    /// The window's resize, waiting for the pane's height to settle: as a pane is first laid out, at a narrow width,
    /// it tells passing heights of thousands of points before its own (seen on macOS 27).
    private var pendingResize: DispatchWorkItem?

    init(model: AppModel) {
        super.init(nibName: nil, bundle: nil)
        tabStyle = .toolbar
        let settings = SettingsModel()
        for pane in SettingsPane.allCases {
            let report: (CGFloat) -> Void = { [weak self] height in self?.pane(pane, measures: height) }
            let hosting = NSHostingController(rootView: pane.view(model: model, settings: settings, report: report))
            hosting.sizingOptions = []
            hosting.title = pane.title
            hosting.preferredContentSize = NSSize(width: Self.width, height: pane.defaultHeight)
            let item = NSTabViewItem(viewController: hosting)
            item.label = pane.title
            item.image = NSImage(systemSymbolName: pane.symbol, accessibilityDescription: pane.title)
            addTabViewItem(item)
        }
        let last = UserDefaults.standard.integer(forKey: Self.lastPaneKey)
        selectedTabViewItemIndex = SettingsPane(rawValue: last) == nil ? 0 : last
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not made from a nib")
    }

    override func tabView(_ tabView: NSTabView, didSelect tabViewItem: NSTabViewItem?) {
        super.tabView(tabView, didSelect: tabViewItem)
        UserDefaults.standard.set(selectedTabViewItemIndex, forKey: Self.lastPaneKey)
    }

    /// Keep a pane's height, and give the window that height when the pane is the one shown. After the layout pass
    /// that measured it, never during it.
    private func pane(_ pane: SettingsPane, measures measured: CGFloat) {
        logger.debug("settings pane \(pane.rawValue, privacy: .public) measures \(Double(measured), privacy: .public)")
        let height = min(max(measured.rounded(.up), 120), Self.tallest)
        guard heights[pane] != height else { return }
        heights[pane] = height
        tabViewItems[pane.rawValue].viewController?.preferredContentSize = NSSize(width: Self.width, height: height)
        pendingResize?.cancel()
        let resize = DispatchWorkItem { [weak self] in
            guard let self, selectedTabViewItemIndex == pane.rawValue, let window = view.window else { return }
            let content = window.contentRect(forFrameRect: window.frame)
            guard abs(content.height - height) > 0.5 else { return }
            var frame = window.frameRect(forContentRect: NSRect(x: content.minX, y: content.minY, width: content.width, height: height))
            frame.origin.y = window.frame.maxY - frame.height
            window.setFrame(frame, display: true, animate: window.isVisible)
        }
        pendingResize = resize
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05, execute: resize)
    }
}

/// The menu bar item's menu: what needs doing, or how the connection is with its details in grey, as menu bar apps
/// show them; then the way to everything else.
struct MenuBarMenu: View {
    @EnvironmentObject private var model: AppModel

    /// Made anew at each step. Kept in place, the first item, the status line while signed in, became the Sign In
    /// button on signing out, and clicking it did nothing until the app was started anew (2.17.3 on macOS 27).
    var body: some View {
        Group { items }
            .id(model.step)
    }

    @ViewBuilder
    private var items: some View {
        if model.server == nil {
            Button("Sign In…") { AppDelegate.shared?.showWindow() }
        } else if model.locationSwitchedOff {
            Button("Switch On in Finder…") { Task { await model.showInFinder() } }
        } else {
            Text(model.statusLine)
        }
        if model.server != nil {
            ForEach(model.details, id: \.self) { Text($0) }
        }
        Divider()
        if model.server != nil {
            Button("Open in Finder") { Task { await model.showInFinder() } }
        }
        Button("Settings…") { AppDelegate.shared?.showSettings() }
            .keyboardShortcut(",")
        Button("Check for Updates…") { AppDelegate.shared?.updater.checkForUpdates(nil) }
        if model.server != nil {
            Divider()
            Button("Sign Out…") { AppDelegate.shared?.confirmSignOut() }
                .disabled(model.busy)
        }
        Divider()
        Button("About Immich Mount") { showAbout() }
        Button("Quit Immich Mount") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}

/// The standard About panel, with the GNOME app's About texts: what it is, the credits and the AI disclosure. The
/// non-affiliation notice is its copyright line (Info.plist), which the panel always shows.
@MainActor
private func showAbout() {
    let credits = NSMutableAttributedString()
    let body: [NSAttributedString.Key: Any] = [.font: NSFont.systemFont(ofSize: NSFont.smallSystemFontSize),
                                               .foregroundColor: NSColor.labelColor]
    let heading: [NSAttributedString.Key: Any] = [.font: NSFont.boldSystemFont(ofSize: NSFont.smallSystemFontSize),
                                                  .foregroundColor: NSColor.labelColor]
    func add(_ text: String, _ attributes: [NSAttributedString.Key: Any], link: String? = nil) {
        var attributes = attributes
        if let link { attributes[.link] = URL(string: link) }
        credits.append(NSAttributedString(string: text, attributes: attributes))
    }
    add(tr("Your Immich photos and videos as a drive in Finder. By Jakob Hviid, under the MIT license.") + "\n\n", body)
    add(tr("Website") + "\n", heading)
    add("jakob.hviid.phd/immich-mount", body, link: "https://jakob.hviid.phd/immich-mount/")
    add("\n\n" + tr("Source code") + "\n", heading)
    add("github.com/jakobhviid/immich-mount", body, link: "https://github.com/jakobhviid/immich-mount")
    add("\n\n", body)
    add(tr("Inspired by") + "\n", heading)
    add("Kartax/immich-desktop-app", body, link: "https://github.com/Kartax/immich-desktop-app")
    add("\n\n" + tr("The Immich API layer") + "\n", heading)
    add("immich-rs", body, link: "https://github.com/boul2gom/immich-rs")
    add("\n\n" + tr("AI disclosure") + "\n", heading)
    add(tr("Parts of this project were written with the assistance of AI coding agents. All changes are reviewed by the maintainer."), body)
    AppDelegate.shared?.bringForward()
    NSApplication.shared.orderFrontStandardAboutPanel(options: [.credits: credits])
}

/// Where the app runs from. The system starts the extension from the path it first ran it at, for as long as its File
/// Provider service runs (verified on macOS 27): an app opened from its disk image, or moved out of Downloads by
/// Gatekeeper's translocation, would leave Finder a location whose extension is gone once the image is ejected. So a
/// release build serves Finder only from an Applications folder; a development build runs where Xcode put it.
#if DEBUG
/// The Dev build's menu bar icon: the release's shape in blue, drawn as it is rather than as a template, which macOS
/// would draw in the menu bar's own black or white.
@MainActor
let devMenuBarIcon: NSImage = {
    guard let shape = NSImage(named: "MenuBarIcon") else { return NSImage() }
    let blue = NSImage(size: shape.size, flipped: false) { rect in
        shape.draw(in: rect)
        NSColor.systemBlue.set()
        rect.fill(using: .sourceAtop)
        return true
    }
    blue.isTemplate = false
    return blue
}()
#endif

enum Installation {
    static var isInApplications: Bool {
        #if DEBUG
        return true
        #else
        let path = Bundle.main.bundleURL.resolvingSymlinksInPath().path
        return path.hasPrefix("/Applications/")
            || path.range(of: "^/Users/[^/]+/Applications/", options: .regularExpression) != nil
        #endif
    }

    /// Whether this copy updates itself: the release, run from Applications. Immich Mount Dev, the Debug build, never
    /// does, so no release replaces it.
    static var updates: Bool {
        #if DEBUG
        return false
        #else
        return isInApplications
        #endif
    }
}

/// The Finder location: one File Provider domain. Finder's sidebar shows the app's name for a provider's only location;
/// the domain's own name, the drive's (`mount.name`), names its folder in ~/Library/CloudStorage.
enum FinderLocation {
    /// Never changes: the system keys its copy of the library by it.
    static let identifier = NSFileProviderDomainIdentifier("library")

    static var domain: NSFileProviderDomain {
        NSFileProviderDomain(identifier: identifier, displayName: name)
    }

    static var name: String { driveName(config: AppGroup.configText) }

    static func add() async throws {
        try await NSFileProviderManager.add(domain)
    }

    /// Give the location the name the settings give it, where the system has another. Adding a domain again updates
    /// it in place: the items, downloads and the switch in System Settings stay.
    static func applyName() async {
        guard let current = try? await NSFileProviderManager.domains().first(where: { $0.identifier == identifier }),
              current.displayName != name else { return }
        do {
            try await NSFileProviderManager.add(domain)
            logger.notice("the Finder location is named \(name, privacy: .public)")
        } catch {
            logger.error("the Finder location was not renamed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Remove the location and what was downloaded, keeping what has not reached Immich: a file moved into Upload is
    /// its only copy until the upload succeeds (PRINCIPLES.md). The system preserves those files, and Finder shows
    /// where.
    static func remove() async throws {
        guard let preserved = try await NSFileProviderManager.remove(domain, mode: .preserveDirtyUserData) else { return }
        let kept = keptFiles(in: preserved)
        logger.notice("removed; \(kept.map { String($0) } ?? "unknown", privacy: .public) files not uploaded are kept at \(preserved.path, privacy: .public)")
        // The system keeps a folder even when nothing in it waited for Immich; Finder opens it only for real files.
        if kept != 0 {
            NSWorkspace.shared.activateFileViewerSelecting([preserved])
        }
    }

    /// The files a removal kept: regular files only, hidden ones (Finder's `.DS_Store`) left out. Nil when the folder
    /// cannot be read, so it is shown rather than missed.
    private static func keptFiles(in folder: URL) -> Int? {
        guard let walker = FileManager.default.enumerator(at: folder, includingPropertiesForKeys: [.isRegularFileKey],
                                                          options: [.skipsHiddenFiles]) else { return nil }
        return walker.compactMap { $0 as? URL }
            .filter { (try? $0.resourceValues(forKeys: [.isRegularFileKey]).isRegularFile) == true }
            .count
    }

    /// How many files wait in Upload: the system keeps a file there until its upload succeeds, and an uploaded one
    /// for the few seconds before it leaves.
    static func waitingUploads() async -> Int {
        guard let root = try? await rootURL() else { return 0 }
        let scoped = root.startAccessingSecurityScopedResource()
        defer { if scoped { root.stopAccessingSecurityScopedResource() } }
        let names = (try? FileManager.default.contentsOfDirectory(atPath: root.appendingPathComponent("Upload").path)) ?? []
        return names.filter { !$0.hasPrefix(".") }.count
    }

    /// Where Finder shows the location's root.
    static func rootURL() async throws -> URL? {
        try await NSFileProviderManager(for: domain)?.getUserVisibleURL(for: .rootContainer)
    }
}

@MainActor
final class AppModel: ObservableObject {
    /// How often the app wakes the extension to ask the server what changed, while signed in; Linux polls as often.
    private static let changeSignal: Duration = .seconds(10)

    /// The signed-in server, `nil` before signing in.
    @Published private(set) var server: String? = Account.load()?.server
    /// What the server being signed in to offers, once its address was checked.
    @Published var offer: ServerOffer?
    @Published private(set) var busy = false
    @Published var message: String?
    /// Whether the app opens at login, so changes keep reaching Finder; the user's choice, off until made.
    @Published private(set) var opensAtLogin = SMAppService.mainApp.status == .enabled
    /// What the extension last saw of the server, read again at each change signal.
    @Published private(set) var status = Status.load()
    /// Whether the location waits to be switched on, in Finder or System Settings, as it does after signing in.
    @Published private(set) var locationSwitchedOff = false

    /// Where the user is: signed out, waiting for the location to be switched on, or using it. The menu bar menu is
    /// made anew at each change.
    var step: String {
        server == nil ? "signed-out" : locationSwitchedOff ? "switched-off" : "on"
    }

    /// The menu bar menu's first line, while signed in.
    var statusLine: String {
        let host = server.flatMap { URL(string: $0)?.host() } ?? server ?? ""
        guard let status else { return tr("Signed in to {server}", ["server": host]) }
        return status.online ? tr("Connected to {server}", ["server": host]) : tr("{server} is not reachable", ["server": host])
    }

    /// The lines under the menu's first one, each saying what it is: the server's version, the account, and when
    /// the connection was last checked, or why it failed.
    var details: [String] {
        var lines: [String] = []
        if let version = status?.version {
            lines.append(tr("Server version: Immich {version}", ["version": version]))
        }
        if let email = Account.email {
            lines.append(tr("Signed in as: {account}", ["account": email]))
        }
        if let status {
            if status.online {
                let ago = RelativeDateTimeFormatter().localizedString(for: status.checked, relativeTo: Date())
                lines.append(tr("Last checked: {time}", ["time": ago]))
            } else {
                lines.append(tr("Reason: {reason}", ["reason": status.problem ?? tr("none given")]))
            }
        }
        return lines
    }

    /// Read the extension's status and whether the location is switched on.
    func refreshState() async {
        status = Status.load()
        guard server != nil else {
            locationSwitchedOff = false
            return
        }
        let location = try? await NSFileProviderManager.domains().first { $0.identifier == FinderLocation.identifier }
        locationSwitchedOff = location.map { !$0.userEnabled } ?? false
    }


    init() {
        guard Installation.isInApplications else {
            logger.notice("opened outside Applications, from \(Bundle.main.bundlePath, privacy: .public): waiting to be moved")
            return
        }
        Task { await refreshItemsAfterUpdate() }
        Task { await FinderLocation.applyName() }
        Task { await refreshState() }
        Task { await keepChangesFlowing() }
        Task { await followFolderCopies() }
    }

    /// Have the system read every item again when the bridge's item format changed since the last run. The format is
    /// part of every item's version, so the system takes the new details (capabilities and fields) it would otherwise
    /// keep.
    private func refreshItemsAfterUpdate() async {
        let kept = (try? String(contentsOf: Self.itemFormatFile, encoding: .utf8)).flatMap { UInt32($0) }
        let format = itemFormat()
        guard kept != format else { return }
        // Signed out, the system keeps no items to read again.
        guard server != nil, let manager = NSFileProviderManager(for: FinderLocation.domain) else {
            recordItemFormat()
            return
        }
        do {
            try await manager.reimportItems(below: .rootContainer)
            recordItemFormat()
            logger.notice("the items are read again for item format \(format)")
        } catch {
            logger.error("the items were not read again: \(String(describing: error), privacy: .public)")
        }
    }

    private static let itemFormatFile = AppGroup.container.appendingPathComponent("item-format")

    /// The system's items are in the bridge's item format: a new location's are, from the start.
    private func recordItemFormat() {
        try? String(itemFormat()).write(to: Self.itemFormatFile, atomically: true, encoding: .utf8)
    }

    /// Wake the extension on the poll interval: the system stops it when idle, and it has no clock of its own. It
    /// then asks the server what changed and reports it from the change journal (ARCHITECTURE.md, "The macOS
    /// edition", "Changes").
    private func keepChangesFlowing() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: AppModel.changeSignal)
            let wasOnline = status?.online
            await refreshState()
            guard server != nil, let manager = NSFileProviderManager(for: FinderLocation.domain) else { continue }
            // The extension answered "server unreachable" while it was; tell the system it may stop waiting.
            if wasOnline == false, status?.online == true {
                try? await manager.signalErrorResolved(NSFileProviderError(.serverUnreachable))
            }
            do {
                try await manager.signalEnumerator(for: .workingSet)
            } catch {
                logger.debug("the extension was not woken: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// Look for questions about copied folders every few seconds: the extension leaves one as soon as a copy settles,
    /// and a file read costs nothing, where waiting for the change signal kept it up to ten seconds.
    private func followFolderCopies() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(2))
            if server != nil {
                FolderCopies.follow()
            }
        }
    }

    func setOpensAtLogin(_ wanted: Bool) {
        do {
            if wanted {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            message = tr("The login item could not be changed: {error}", ["error": error.localizedDescription])
        }
        opensAtLogin = SMAppService.mainApp.status == .enabled
    }

    /// Check the server's address and what it offers. This runs in the app before the extension ever does, so
    /// macOS's Local Network prompt names the app (ARCHITECTURE.md, "The macOS edition", "Signing in").
    func check(address: String) async {
        busy = true
        message = nil
        defer { busy = false }
        do {
            offer = try await probe(address: address)
        } catch {
            message = describe(error)
        }
    }

    func useKey(_ key: String) async {
        guard let server = offer?.address else { return }
        await finishSignIn(on: server) { try await signInWithApiKey(server: server, key: key) }
    }

    func usePassword(email: String, password: String) async {
        guard let server = offer?.address else { return }
        await finishSignIn(on: server) { try await signInWithPassword(server: server, email: email, password: password) }
    }

    /// Sign in in a web authentication session, which ends at the address Immich's mobile app registers.
    func useBrowser(_ session: WebAuthenticationSession) async {
        guard let server = offer?.address else { return }
        await finishSignIn(on: server) {
            let attempt = try await beginBrowserSignIn(server: server)
            guard let address = URL(string: attempt.address()) else {
                throw BridgeError.Failed(message: tr("The server sent an address that cannot be opened."))
            }
            let redirect = try await session.authenticate(using: address, callbackURLScheme: attempt.callbackScheme(),
                                                          preferredBrowserSession: nil)
            return try await attempt.finish(redirect: redirect.absoluteString)
        }
    }

    /// Keep the credential and add the Finder location.
    private func finishSignIn(on server: String, _ signingIn: () async throws -> SignedIn) async {
        busy = true
        message = nil
        defer { busy = false }
        do {
            let signedIn = try await signingIn()
            try Account(server: server, credential: signedIn.credential).save()
            Account.keepEmail(signedIn.account)
            try await FinderLocation.add()
            recordItemFormat()
            // A new location waits to be switched on; known before the account is, so the window shows that step.
            locationSwitchedOff = true
            self.server = server
            offer = nil
            await refreshState()
            logger.notice("signed in; the Finder location is added")
        } catch {
            message = describe(error)
            logger.error("signing in failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// End the session on the server, and remove the Finder location, the account and the extension's store.
    /// A location that cannot be removed leaves the user signed in, so it is not left behind unseen: the error is
    /// thrown, for the caller to show.
    func signOut() async throws {
        busy = true
        message = nil
        defer { busy = false }
        do {
            try await FinderLocation.remove()
        } catch {
            logger.error("the Finder location was not removed, so the account stays: \(String(describing: error), privacy: .public)")
            throw error
        }
        if let account = Account.load() {
            await endSession(server: account.server, credential: account.credential)
        }
        Account.forget()
        try? FileManager.default.removeItem(at: AppGroup.libraryDirectory)
        FolderCopies.forget()
        server = nil
        logger.notice("signed out; the Finder location is removed")
    }

    #if DEBUG
    /// For driving a development build from a script (the `mac-dev` skill), with launch arguments: `-signOut YES`
    /// signs out, and `-signInServer <address>` with `-signInAPIKey <key>`, or `-signInEmail` and
    /// `-signInPassword`, signs in when nobody is; `-setPreference <key>=<value>` changes one setting, as the Settings
    /// window does; `-forgetApp <bundle id>` forgets an app not installed, as its Forget button does. Not in release
    /// builds.
    func followLaunchArguments() async {
        let arguments = UserDefaults.standard
        if arguments.bool(forKey: "signOut") {
            try? await signOut()
        }
        if let preference = arguments.string(forKey: "setPreference"), let equals = preference.firstIndex(of: "=") {
            let settings = SettingsModel()
            settings.change(String(preference[..<equals]), to: String(preference[preference.index(after: equals)...]))
            logger.notice("set \(preference, privacy: .public): \(settings.message ?? "done", privacy: .public)")
        }
        if let bundleId = arguments.string(forKey: "forgetApp") {
            CrosslinkApps.shared.forget(bundleId)
            logger.notice("forgot \(bundleId, privacy: .public)")
        }
        guard server == nil, let address = arguments.string(forKey: "signInServer") else { return }
        await check(address: address)
        if let key = arguments.string(forKey: "signInAPIKey") {
            await useKey(key)
        } else if let email = arguments.string(forKey: "signInEmail"), let password = arguments.string(forKey: "signInPassword") {
            await usePassword(email: email, password: password)
        }
    }
    #endif

    /// A Finder window on the location. The sandboxed app may not open the folder itself (macOS answers that it has
    /// no permission to open it), but it may ask Finder to show it. While the location is switched off, Finder shows
    /// its Enable button at the top of that window, the one click that switches it on; should the system not tell
    /// where the location is then, System Settings' list of file providers opens instead.
    func showInFinder() async {
        let url: URL
        do {
            guard let root = try await FinderLocation.rootURL() else { throw CocoaError(.fileNoSuchFile) }
            url = root
        } catch {
            logger.error("Finder was not shown the location: \(String(describing: error), privacy: .public)")
            if locationSwitchedOff {
                openExtensionSettings()
            } else {
                message = tr("Finder does not show Immich yet. Is the extension switched on in System Settings? ({error})", ["error": error.localizedDescription])
            }
            return
        }
        let scoped = url.startAccessingSecurityScopedResource()
        defer {
            if scoped { url.stopAccessingSecurityScopedResource() }
        }
        if !NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: url.path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
    }

    /// System Settings at its list of file providers, where the extension is switched on too. Naming the extension
    /// point opens the list itself; Login Items & Extensions alone leaves it to be found by scrolling, and is where
    /// the app goes should System Settings not take the first address.
    func openExtensionSettings() {
        let addresses = [
            "x-apple.systempreferences:com.apple.ExtensionsPreferences?extensionPointIdentifier=com.apple.fileprovider-nonui",
            "x-apple.systempreferences:com.apple.LoginItems-Settings.extension",
        ]
        for address in addresses {
            if let url = URL(string: address), NSWorkspace.shared.open(url) { return }
        }
    }

}

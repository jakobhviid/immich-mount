// The Settings window's panes (ARCHITECTURE.md, "The macOS edition", "Settings"): General, Folders and Account, each
// a grouped form, switched by the window's toolbar as Apple's Human Interface Guidelines ask. The settings of the
// core's table are kept in config.toml in the App Group, where the extension reads them.

import FileProvider
import SwiftUI

/// The panes, in the toolbar's order.
enum SettingsPane: Int, CaseIterable {
    case general, folders, account

    var title: String {
        switch self {
        case .general: tr("General")
        case .folders: tr("Folders")
        case .account: tr("Account")
        }
    }

    var symbol: String {
        switch self {
        case .general: "gearshape"
        case .folders: "folder"
        case .account: "person.crop.circle"
        }
    }

    /// A height for the pane until it has measured itself, and on macOS 14, which cannot tell a form's content
    /// height.
    var defaultHeight: CGFloat {
        switch self {
        case .general: 200
        case .folders: 640
        case .account: 330
        }
    }

    /// The pane, telling the window its content's height once laid out, and again whenever it changes.
    @MainActor
    func view(model: AppModel, settings: SettingsModel, report: @escaping (CGFloat) -> Void) -> AnyView {
        let pane: AnyView = switch self {
        case .general: AnyView(GeneralPane())
        case .folders: AnyView(FoldersPane())
        case .account: AnyView(AccountPane())
        }
        return AnyView(pane.reportingContentHeight(report).environmentObject(model).environmentObject(settings))
    }
}

extension View {
    /// The height of a grouped form's content, insets included, as laid out: a form scrolls, so it takes any height
    /// it is offered and has none of its own to ask, but its scroll geometry tells (macOS 15 and later).
    @ViewBuilder
    fileprivate func reportingContentHeight(_ report: @escaping (CGFloat) -> Void) -> some View {
        if #available(macOS 15, *) {
            onScrollGeometryChange(for: CGFloat.self) { geometry in
                geometry.contentSize.height + geometry.contentInsets.top + geometry.contentInsets.bottom
            } action: { _, height in
                report(height)
            }
        } else {
            self
        }
    }
}

/// The settings of the core's table the panes show, and the one way to change them.
@MainActor
final class SettingsModel: ObservableObject {
    @Published private(set) var panes: SettingsPanes?
    /// Why the last change was refused, shown in the pane it was made in.
    @Published var message: String?

    init() {
        reload()
    }

    func reload() {
        do {
            panes = try settingsPanes(config: AppGroup.configText)
        } catch {
            message = describe(error)
        }
    }

    func isOn(_ preference: Preference) -> Binding<Bool> {
        Binding(get: { preference.value == "true" }, set: { self.change(preference.key, to: $0 ? "true" : "false") })
    }

    func choice(_ preference: Preference) -> Binding<String> {
        Binding(get: { preference.value }, set: { self.change(preference.key, to: $0) })
    }

    /// Keep one value, then have the system list every folder again, by the new rules.
    func change(_ key: String, to value: String) {
        do {
            let updated = try setPreference(config: AppGroup.configText, key: key, value: value)
            try FileManager.default.createDirectory(at: AppGroup.container, withIntermediateDirectories: true)
            try updated.write(to: AppGroup.configFile, atomically: true, encoding: .utf8)
            panes = try settingsPanes(config: updated)
            message = nil
            Task {
                do {
                    try await NSFileProviderManager(for: FinderLocation.domain)?.reimportItems(below: .rootContainer)
                } catch {
                    logger.info("Finder was not asked to list again: \(String(describing: error), privacy: .public)")
                }
            }
        } catch {
            message = describe(error)
        }
    }
}

/// Opening at login, updates, and the Upload folder.
struct GeneralPane: View {
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var settings: SettingsModel
    @State private var checksForUpdates = AppDelegate.shared?.updater.updater.automaticallyChecksForUpdates ?? false

    var body: some View {
        Form {
            OutsideApplications()
            Section {
                Toggle("Open at login", isOn: Binding(get: { model.opensAtLogin }, set: { model.setOpensAtLogin($0) }))
                if Installation.updates {
                    Toggle("Check for updates automatically", isOn: $checksForUpdates)
                        .onChange(of: checksForUpdates) { _, checks in
                            AppDelegate.shared?.updater.updater.automaticallyChecksForUpdates = checks
                        }
                }
            } footer: {
                Text("While Immich Mount runs, in the menu bar, changes in Immich reach Finder within seconds. Updates come from jakob.hviid.phd, signed by the project and notarized by Apple.")
                    .foregroundStyle(.secondary)
            }
            Refusal()
        }
        .formStyle(.grouped)
    }
}

/// What Finder shows: the sections with their date folders, the album folders and the file names.
struct FoldersPane: View {
    @EnvironmentObject private var settings: SettingsModel

    var body: some View {
        Form {
            OutsideApplications()
            if let panes = settings.panes {
                Section {
                    ForEach(panes.sections, id: \.name) { section in
                        // The section's folder name, never translated (SPEC.md, "Names"); its date folders, which
                        // mean nothing while it is hidden (Upload has none); and whether Finder shows it.
                        LabeledContent(section.name) {
                            HStack(spacing: 12) {
                                if let dateFolders = section.dateFolders {
                                    choices(dateFolders)
                                        .disabled(section.shown.value != "true")
                                }
                                Toggle(tr(section.shown.title), isOn: settings.isOn(section.shown))
                                    .labelsHidden()
                                    .toggleStyle(.switch)
                            }
                        }
                    }
                } header: {
                    HStack {
                        Text(tr("Sections"))
                        Spacer()
                        Text(tr("Date Folders"))
                    }
                } footer: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Date folders are day folders inside each month in Photos, and year and month folders in the other sections.")
                        Text("Photos and videos copied into Upload are added to Immich.")
                    }
                    .foregroundStyle(.secondary)
                }
                Section(tr("Albums")) {
                    choices(panes.albumLayout, label: tr("Folder names"))
                }
                Section {
                    choices(panes.fileNaming, label: tr("File names"))
                } header: {
                    Text(tr("Files"))
                } footer: {
                    Text("Finder lists Immich again after each change.")
                        .foregroundStyle(.secondary)
                }
            }
            Refusal()
        }
        .formStyle(.grouped)
        // A change reimports the location's items, which only the copy in Applications may do.
        .disabled(!Installation.isInApplications)
    }

    /// A choice: with a short label of its own in a row, or with none beside a section's switch, where the
    /// setting's full title (Linux's) is still what VoiceOver reads.
    @ViewBuilder
    private func choices(_ preference: Preference, label: String? = nil) -> some View {
        if case .choice(let options) = preference.kind {
            let picker = Picker(label ?? tr(preference.title), selection: settings.choice(preference)) {
                ForEach(options, id: \.value) { option in
                    Text(tr(option.label)).tag(option.value)
                }
            }
            if label != nil {
                picker
            } else {
                picker.labelsHidden().fixedSize()
            }
        }
    }
}

/// The account: the connection as the menu bar menu tells it, the location, and signing out, or in.
struct AccountPane: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Form {
            OutsideApplications()
            if model.server == nil {
                Section {
                    LabeledContent(tr("Not signed in")) {
                        Button("Sign In…") { AppDelegate.shared?.showWindow() }
                    }
                }
            } else {
                Section {
                    LabeledContent {
                        Image(systemName: "circle.fill")
                            .foregroundStyle(model.status?.online == false ? .red : .green)
                            .imageScale(.small)
                            .accessibilityHidden(true)
                    } label: {
                        Text(model.statusLine)
                    }
                    ForEach(model.details, id: \.self) { line in
                        Text(line).foregroundStyle(.secondary)
                    }
                }
                Section {
                    HStack {
                        if model.locationSwitchedOff {
                            Button("Switch On in System Settings…") { model.openExtensionSettings() }
                        } else {
                            Button("Open in Finder") { Task { await model.showInFinder() } }
                        }
                        Spacer()
                        Button("Sign Out…", role: .destructive) { AppDelegate.shared?.confirmSignOut() }
                            .disabled(model.busy)
                    }
                }
            }
        }
        .formStyle(.grouped)
    }
}

/// While the app runs from elsewhere than Applications, what to do about it: the settings that reach Finder are
/// greyed out until then, as the sign-in window tells.
private struct OutsideApplications: View {
    var body: some View {
        if !Installation.isInApplications {
            Section {
                Title(text: tr("Move Immich Mount to Applications"),
                      line: tr("Finder's location runs from the Applications folder. Quit Immich Mount, drag it into Applications, then open it from there."))
                    .frame(maxWidth: .infinity)
            }
        }
    }
}

/// Why the last change was refused, in the pane it was made in.
private struct Refusal: View {
    @EnvironmentObject private var settings: SettingsModel

    var body: some View {
        if let message = settings.message {
            Section {
                Text(message).foregroundStyle(.red)
            }
        }
    }
}

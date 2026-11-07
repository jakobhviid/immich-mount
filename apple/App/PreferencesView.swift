// The app's Settings window: the settings of the core's table that mean something on macOS, kept in config.toml in
// the App Group, where the extension reads them (ARCHITECTURE.md, "The macOS edition", "Settings").

import FileProvider
import SwiftUI

struct PreferencesView: View {
    @State private var groups: [PreferenceGroup] = preferences(config: AppGroup.configText)
    @State private var message: String?

    var body: some View {
        Form {
            ForEach(groups, id: \.title) { group in
                Section(group.title) {
                    ForEach(group.preferences, id: \.key) { preference in
                        row(for: preference)
                    }
                }
            }
            if let message {
                Text(message).foregroundStyle(.red)
            }
        }
        .formStyle(.grouped)
        // A change reimports the location's items, which only the copy in Applications may do.
        .disabled(!Installation.isInApplications)
        .frame(width: 480)
    }

    @ViewBuilder
    private func row(for preference: Preference) -> some View {
        switch preference.kind {
        case .toggle:
            Toggle(preference.title, isOn: Binding(get: { preference.value == "true" },
                                                   set: { change(preference.key, to: $0 ? "true" : "false") }))
        case .choice(let options):
            Picker(preference.title, selection: Binding(get: { preference.value }, set: { change(preference.key, to: $0) })) {
                ForEach(options, id: \.value) { option in
                    Text(option.label).tag(option.value)
                }
            }
        }
    }

    /// Keep one value, then have the system list every folder again, by the new rules; a new name renames the location.
    private func change(_ key: String, to value: String) {
        do {
            let updated = try setPreference(config: AppGroup.configText, key: key, value: value)
            try FileManager.default.createDirectory(at: AppGroup.container, withIntermediateDirectories: true)
            try updated.write(to: AppGroup.configFile, atomically: true, encoding: .utf8)
            groups = preferences(config: updated)
            message = nil
            Task {
                if key == "mount.name" {
                    await FinderLocation.applyName()
                    return
                }
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

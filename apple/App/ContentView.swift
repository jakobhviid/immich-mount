// The app's one window: sign in, or the status of the Finder location.

import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Group {
            if let server = model.server {
                StatusView(server: server)
            } else {
                SignInView()
            }
        }
        .frame(width: 480)
        #if DEBUG
        .task { await model.followLaunchArguments() }
        #endif
    }
}

struct SignInView: View {
    @EnvironmentObject private var model: AppModel
    @State private var address = ""
    @State private var apiKey = ""

    var body: some View {
        Form {
            Section {
                TextField("Server", text: $address, prompt: Text("photos.example.org"))
                SecureField("API key", text: $apiKey)
            } header: {
                Text("Sign in to Immich")
            } footer: {
                Text("Create an API key in Immich under Account Settings, API Keys. A key that may only read is enough to browse.")
                    .foregroundStyle(.secondary)
            }
            if let message = model.message {
                Text(message).foregroundStyle(.red)
            }
            HStack {
                Spacer()
                if model.busy {
                    ProgressView().controlSize(.small)
                }
                Button("Sign In") {
                    Task { await model.signIn(address: address, apiKey: apiKey) }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(model.busy || address.isEmpty || apiKey.isEmpty)
            }
        }
        .formStyle(.grouped)
    }
}

struct StatusView: View {
    let server: String
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Form {
            Section {
                LabeledContent("Server", value: server)
                Text("Your library is in Finder's sidebar as Immich, and in every file dialog. Originals download when something opens them.")
            } header: {
                Text("Immich is in Finder")
            }
            Section {
                Text("If Immich is missing from Finder's sidebar, switch Immich Mount on in System Settings, under General, Login Items & Extensions, File Providers.")
                    .foregroundStyle(.secondary)
                HStack {
                    Button("Open System Settings") { model.openExtensionSettings() }
                    Button("Show in Finder") { Task { await model.showInFinder() } }
                }
            }
            if let message = model.message {
                Text(message).foregroundStyle(.red)
            }
            HStack {
                Spacer()
                Button("Sign Out") { Task { await model.signOut() } }
                    .disabled(model.busy)
            }
        }
        .formStyle(.grouped)
    }
}

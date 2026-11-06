// The app's one window: sign in, or the status of the Finder location.

import AuthenticationServices
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

/// Signing in as on Linux: the server's address first, then what it offers (ARCHITECTURE.md, "Signing in").
struct SignInView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.webAuthenticationSession) private var webAuthenticationSession
    @State private var address = ""
    @State private var email = ""
    @State private var password = ""
    @State private var apiKey = ""
    @State private var usesKey = false

    var body: some View {
        Form {
            Section {
                TextField("Server", text: $address, prompt: Text("photos.example.org"))
                    .disabled(model.offer != nil)
            } header: {
                Text("Sign in to Immich")
            }
            if let offer = model.offer {
                if let button = offer.browserButton, !usesKey {
                    Section {
                        Button(button) { Task { await model.useBrowser(webAuthenticationSession) } }
                            .disabled(model.busy)
                    }
                }
                if offer.password, !usesKey {
                    Section {
                        TextField("Email", text: $email)
                        SecureField("Password", text: $password)
                        HStack {
                            Spacer()
                            Button("Sign In") { Task { await model.usePassword(email: email, password: password) } }
                                .keyboardShortcut(.defaultAction)
                                .disabled(model.busy || email.isEmpty || password.isEmpty)
                        }
                    }
                }
                Section {
                    if usesKey {
                        SecureField("API key", text: $apiKey)
                        HStack {
                            Spacer()
                            Button("Sign In") { Task { await model.useKey(apiKey) } }
                                .keyboardShortcut(.defaultAction)
                                .disabled(model.busy || apiKey.isEmpty)
                        }
                    } else {
                        Button("Use an API Key Instead") { usesKey = true }
                    }
                } footer: {
                    Text("Immich \(offer.version). An API key is made in Immich under Account Settings, API Keys; one that may only read is enough to browse.")
                        .foregroundStyle(.secondary)
                }
            }
            if let message = model.message {
                Text(message).foregroundStyle(.red)
            }
            HStack {
                if model.offer != nil {
                    Button("Other Server") {
                        model.offer = nil
                        usesKey = false
                    }
                }
                Spacer()
                if model.busy {
                    ProgressView().controlSize(.small)
                }
                if model.offer == nil {
                    Button("Continue") { Task { await model.check(address: address) } }
                        .keyboardShortcut(.defaultAction)
                        .disabled(model.busy || address.isEmpty)
                }
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
            Section {
                Toggle("Open at login", isOn: Binding(get: { model.opensAtLogin }, set: { model.setOpensAtLogin($0) }))
            } footer: {
                Text("While Immich Mount runs, changes in Immich reach Finder within seconds.")
                    .foregroundStyle(.secondary)
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

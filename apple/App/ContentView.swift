// The app's one window, for signing in (ARCHITECTURE.md, "The macOS edition"): the server's address, then the way to
// sign in that the server offers, then, while the Finder location waits to be switched on, the step that does it.
// The window closes once the location is on; everything after that is in the menu bar item's menu.

import AuthenticationServices
import SwiftUI

struct ContentView: View {
    /// The window's width, whatever the step.
    static let width: CGFloat = 460
    @EnvironmentObject private var model: AppModel
    /// Told the content's height whenever a step changes it, so the window can take it.
    var heightChanged: (CGFloat) -> Void = { _ in }

    var body: some View {
        VStack(spacing: 18) {
            Image("Drive")
                .resizable()
                .frame(width: 80, height: 80)
            if !Installation.isInApplications {
                Title(text: tr("Move Immich Mount to Applications"),
                      line: tr("Finder's location runs from the Applications folder. Quit Immich Mount, drag it into Applications, then open it from there."))
            } else if model.server == nil {
                Title(text: tr("Sign In to Immich"), line: tr("Your photos and videos in Finder, as a drive."))
                SignInForm()
            } else {
                SwitchOnStep()
            }
        }
        .padding(.horizontal, 28)
        .padding(.top, 20)
        .padding(.bottom, 24)
        .frame(width: Self.width)
        .fixedSize(horizontal: false, vertical: true)
        .background(GeometryReader { proxy in
            Color.clear.preference(key: ContentHeight.self, value: proxy.size.height)
        })
        .onPreferenceChange(ContentHeight.self) { heightChanged($0) }
    }
}

private struct ContentHeight: PreferenceKey {
    static let defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}

/// A step's title over a line, centred, as macOS's own setup windows have them.
struct Title: View {
    let text: String
    let line: String

    var body: some View {
        VStack(spacing: 6) {
            Text(text).font(.title2.bold())
            Text(line)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Signing in as on Linux: the server's address first, then what it offers (ARCHITECTURE.md, "Signing in"). The
/// fields stand in a form with their labels on the left, as in Mail's and Internet Accounts' sign-in windows.
struct SignInForm: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.webAuthenticationSession) private var webAuthenticationSession
    @State private var address = ""
    @State private var email = ""
    @State private var password = ""
    @State private var apiKey = ""
    @State private var usesKey = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let offer = model.offer {
                Text(verbatim: "\(offer.address), Immich \(offer.version)")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity)
                if let button = offer.browserButton, !usesKey {
                    Button { Task { await model.useBrowser(webAuthenticationSession) } } label: {
                        Text(button).frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(model.busy)
                }
                if usesKey {
                    Form {
                        SecureField("API Key:", text: $apiKey)
                    }
                    Text("Make one in Immich under Account Settings, API Keys; one that may only read is enough to browse.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                } else if offer.password {
                    Form {
                        TextField("Email:", text: $email)
                        SecureField("Password:", text: $password)
                    }
                }
            } else {
                Form {
                    TextField("Server:", text: $address, prompt: Text(verbatim: "photos.example.org"))
                }
            }
            if let message = model.message {
                Text(message)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            buttons
        }
        .formStyle(.columns)
    }

    @ViewBuilder
    private var buttons: some View {
        HStack {
            if model.offer != nil {
                Button("Back") {
                    model.offer = nil
                    model.message = nil
                    usesKey = false
                }
                Button(usesKey ? tr("Use Email and Password") : tr("Use an API Key")) { usesKey.toggle() }
                    .buttonStyle(.link)
                    .disabled(usesKey && model.offer?.password != true)
            }
            Spacer()
            if model.busy {
                ProgressView().controlSize(.small)
            }
            if model.offer == nil {
                Button("Continue") { Task { await model.check(address: address) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.busy || address.isEmpty)
            } else if usesKey {
                Button("Sign In") { Task { await model.useKey(apiKey) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.busy || apiKey.isEmpty)
            } else if model.offer?.password == true {
                Button("Sign In") { Task { await model.usePassword(email: email, password: password) } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.busy || email.isEmpty || password.isEmpty)
            }
        }
    }
}

/// The step after signing in, while the location waits to be switched on: nothing else on the Mac says so, and
/// Finder shows nothing until it is. Finder's own Enable button, at the top of the location's window, is the one
/// click it takes; System Settings' list of file providers is the way when Finder offers none. The window closes by
/// itself once it is on.
struct SwitchOnStep: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        Title(text: tr("Switch On Immich Mount"),
              line: tr("macOS asks once before a new location shows in Finder: open it in Finder, then click Enable at the top of the window."))
        HStack {
            Button("Use System Settings Instead") { model.openExtensionSettings() }
                .buttonStyle(.link)
            Spacer()
            Button("Open in Finder") { Task { await model.showInFinder() } }
                .keyboardShortcut(.defaultAction)
        }
        .task {
            // Quicker than the change signal, so the window closes as soon as the switch is on.
            while !Task.isCancelled {
                await model.refreshState()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }
}

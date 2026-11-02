// Immich Mount for KDE Plasma: the same steps as the GNOME app (connect, sign in, choose, status), in Kirigami.
import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import org.kde.kirigami as Kirigami
import dev.hviid.ImmichMount

Kirigami.ApplicationWindow {
    id: root
    title: "Immich Mount"
    width: Kirigami.Units.gridUnit * 30
    height: Kirigami.Units.gridUnit * 40
    minimumWidth: Kirigami.Units.gridUnit * 20

    Backend {
        id: backend
    }

    // The usual keys of a KDE app: Ctrl+Q quits, Ctrl+, opens the preferences.
    Shortcut {
        sequences: [StandardKey.Quit]
        onActivated: Qt.quit()
    }
    Shortcut {
        sequence: "Ctrl+,"
        onActivated: root.pageStack.layers.push(preferencesPage)
    }

    property bool browserOpened: true
    function openBrowser() {
        browserOpened = Qt.openUrlExternally(backend.address)
    }

    // Keep the status current while it is shown, and notice a browser sign-in finished by the other instance.
    Timer {
        interval: 4000
        repeat: true
        running: backend.page === "status" || backend.page === "waiting"
        onTriggered: backend.refresh()
    }

    // KDE counts the mount as a remote location, where Dolphin shows no thumbnails until they are allowed.
    component PreviewsSwitch: ColumnLayout {
        Controls.Switch {
            text: "Show thumbnails"
            checked: backend.previews
            onToggled: backend.allowPreviews(checked)
        }
        Controls.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 16
            wrapMode: Text.WordWrap
            font: Kirigami.Theme.smallFont
            opacity: 0.7
            text: "Lets Dolphin preview files up to 100 MB on remote locations, this drive included. Thumbnails come from the server; no original is downloaded."
        }
    }

    // Generated from the settings table, as the GNOME app's dialog is: one row per setting, written at once.
    Component {
        id: preferencesPage
        Kirigami.ScrollablePage {
            id: preferences
            title: "Preferences"
            readonly property var rows: JSON.parse(backend.preferenceRows())
            function store(key, value) {
                refusal.text = backend.storeSetting(key, value)
            }
            Component.onDestruction: backend.preferencesClosed()
            header: Kirigami.InlineMessage {
                id: refusal
                type: Kirigami.MessageType.Error
                position: Kirigami.InlineMessage.Position.Header
                visible: text.length > 0
            }
            Kirigami.FormLayout {
                Repeater {
                    model: preferences.rows
                    // FormLayout lays out its direct children only, so each row is one Loader.
                    delegate: Loader {
                        id: row
                        required property var modelData
                        Kirigami.FormData.isSection: modelData.kind === "section"
                        Kirigami.FormData.label: modelData.kind === "section" ? modelData.title
                            : (modelData.kind === "bool" ? "" : modelData.title + ":")
                        sourceComponent: {
                            switch (modelData.kind) {
                            case "section": return sectionRow
                            case "bool": return switchRow
                            case "choice": return choiceRow
                            default: return textRow
                            }
                        }
                        Component { id: sectionRow; Kirigami.Separator {} }
                        Component {
                            id: switchRow
                            Controls.Switch {
                                text: row.modelData.title
                                checked: row.modelData.value === "true"
                                onToggled: preferences.store(row.modelData.key, checked ? "true" : "false")
                            }
                        }
                        Component {
                            id: choiceRow
                            Controls.ComboBox {
                                Accessible.name: row.modelData.title
                                model: row.modelData.choices
                                textRole: "label"
                                valueRole: "word"
                                Component.onCompleted: currentIndex = indexOfValue(row.modelData.value)
                                onActivated: preferences.store(row.modelData.key, currentValue)
                            }
                        }
                        Component {
                            id: textRow
                            Controls.TextField {
                                Accessible.name: row.modelData.title
                                text: row.modelData.value
                                // Also called when the field merely loses focus: only a changed value is stored.
                                onEditingFinished: {
                                    if (text.trim() !== row.modelData.value)
                                        preferences.store(row.modelData.key, text.trim())
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // The notice, the credits and the AI disclosure, as in the GNOME app's About dialog.
    Component {
        id: aboutPage
        Kirigami.ScrollablePage {
            title: "About"
            ColumnLayout {
                spacing: Kirigami.Units.largeSpacing
                Kirigami.Icon {
                    source: "dev.hviid.ImmichMount"
                    implicitWidth: Kirigami.Units.iconSizes.enormous
                    implicitHeight: Kirigami.Units.iconSizes.enormous
                }
                Kirigami.Heading { text: "Immich Mount " + backend.version }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Your Immich photos and videos as a drive in Dolphin. By Jakob Hviid, under the MIT license."
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Immich Mount is an independent, unofficial project. It is not affiliated with, endorsed by, or sponsored by the Immich project or FUTO. \u201cImmich\u201d is used only to describe the server this software works with, and remains the mark of its owner."
                }
                Kirigami.Heading { level: 3; text: "Credits" }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    textFormat: Text.StyledText
                    onLinkActivated: link => Qt.openUrlExternally(link)
                    text: "Inspired by <a href=\"https://github.com/Kartax/immich-desktop-app\">Kartax/immich-desktop-app</a><br>The Immich API layer: <a href=\"https://github.com/boul2gom/immich-rs\">immich-rs</a>"
                }
                Kirigami.Heading { level: 3; text: "AI disclosure" }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: "Parts of this project were written with the assistance of AI coding agents. All changes are reviewed by the maintainer."
                }
                Controls.Label {
                    textFormat: Text.StyledText
                    onLinkActivated: link => Qt.openUrlExternally(link)
                    text: "<a href=\"https://github.com/jakobhviid/immich-mount\">github.com/jakobhviid/immich-mount</a>"
                }
            }
        }
    }

    pageStack.initialPage: Kirigami.ScrollablePage {
        actions: [
            Kirigami.Action {
                icon.name: "go-previous"
                text: "Back"
                visible: backend.page === "signin" || backend.page === "waiting"
                enabled: !backend.busy
                onTriggered: backend.goBack()
            },
            Kirigami.Action {
                icon.name: "configure"
                text: "Preferences"
                displayHint: Kirigami.DisplayHint.AlwaysHide
                onTriggered: root.pageStack.layers.push(preferencesPage)
            },
            Kirigami.Action {
                icon.name: "help-about"
                text: "About"
                displayHint: Kirigami.DisplayHint.AlwaysHide
                onTriggered: root.pageStack.layers.push(aboutPage)
            }
        ]
        title: {
            switch (backend.page) {
            case "connect": return "Connect"
            case "signin": return "Sign In"
            case "waiting": return "Continue in Your Browser"
            case "setup": return "Your Library in Dolphin"
            default: return "Immich Mount"
            }
        }

        ColumnLayout {
            spacing: Kirigami.Units.largeSpacing

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                type: Kirigami.MessageType.Error
                text: backend.message
                visible: backend.message.length > 0 && backend.page !== "waiting"
            }

            // Connect: the server's address.
            ColumnLayout {
                visible: backend.page === "connect"
                Layout.fillWidth: true
                Kirigami.PlaceholderMessage {
                    Layout.fillWidth: true
                    icon.name: "dev.hviid.ImmichMount"
                    text: "Your Immich Library in Dolphin"
                    explanation: "Browse your photos and videos as a drive. Originals download only when you open them."
                }
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    Controls.TextField {
                        id: addressField
                        Kirigami.FormData.label: "Server address:"
                        Accessible.name: "Server address"
                        text: backend.server
                        placeholderText: "https://photos.example.org"
                        onAccepted: backend.checkServer(text)
                    }
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    text: "Continue"
                    icon.name: "go-next"
                    enabled: !backend.busy && addressField.text.trim().length > 0
                    onClicked: backend.checkServer(addressField.text)
                }
            }

            // Sign in: only the methods the server offers.
            ColumnLayout {
                visible: backend.page === "signin"
                Layout.fillWidth: true
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    visible: backend.sso.length > 0
                    text: backend.sso
                    icon.name: "internet-web-browser"
                    enabled: !backend.busy
                    onClicked: backend.signInBrowser()
                }
                Controls.Label {
                    Layout.fillWidth: true
                    visible: backend.sso.length > 0
                    wrapMode: Text.WordWrap
                    horizontalAlignment: Text.AlignHCenter
                    text: "Opens your browser, where you sign in as usual, with a passkey or a password manager."
                }
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    Kirigami.Separator {
                        Kirigami.FormData.isSection: true
                        Kirigami.FormData.label: "Email and password"
                        visible: backend.passwords
                    }
                    Controls.TextField { id: emailField; Kirigami.FormData.label: "Email:"; Accessible.name: "Email"; visible: backend.passwords }
                    Kirigami.PasswordField {
                        id: passwordField
                        Kirigami.FormData.label: "Password:"
                        Accessible.name: "Password"
                        placeholderText: ""
                        visible: backend.passwords
                        onAccepted: backend.signInPassword(emailField.text, text)
                    }
                    Controls.Button {
                        text: "Sign In"
                        visible: backend.passwords
                        enabled: !backend.busy
                        onClicked: backend.signInPassword(emailField.text, passwordField.text)
                    }
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "API key" }
                    Kirigami.PasswordField {
                        id: keyField
                        Kirigami.FormData.label: "Key:"
                        Accessible.name: "API key"
                        placeholderText: ""
                        onAccepted: backend.signInKey(text)
                    }
                    Controls.Button {
                        text: "Sign In With Key"
                        enabled: !backend.busy
                        onClicked: backend.signInKey(keyField.text)
                    }
                }
            }

            // Waiting for the browser: the redirect comes back to a second instance, which keeps the credential.
            Kirigami.PlaceholderMessage {
                visible: backend.page === "waiting"
                Layout.fillWidth: true
                icon.name: "internet-web-browser"
                text: "Continue in your browser"
                explanation: "Sign in there. When your browser asks to open Immich Mount, allow it."
                helpfulAction: Kirigami.Action {
                    icon.name: "internet-web-browser"
                    text: "Open the Sign-In Page Again"
                    onTriggered: root.openBrowser()
                }
            }
            ColumnLayout {
                visible: backend.page === "waiting"
                Layout.fillWidth: true
                Controls.Label {
                    Layout.fillWidth: true
                    visible: !root.browserOpened
                    wrapMode: Text.WordWrap
                    text: "No browser could be opened. Open this address in one:"
                }
                Controls.TextField {
                    Layout.fillWidth: true
                    visible: !root.browserOpened
                    readOnly: true
                    selectByMouse: true
                    Accessible.name: "Sign-in address"
                    text: backend.address
                }
            }
            Connections {
                target: backend
                function onAddressChanged() {
                    if (backend.address.length > 0)
                        root.openBrowser()
                }
                function onMountReady() {
                    Qt.openUrlExternally("file://" + backend.location)
                }
            }

            // First run: which sections show, and where the drive goes.
            ColumnLayout {
                visible: backend.page === "setup"
                Layout.fillWidth: true
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Sections" }
                    Controls.Switch { text: "Photos"; checked: backend.photos; onToggled: backend.photos = checked }
                    Controls.Switch { text: "Albums"; checked: backend.albums; onToggled: backend.albums = checked }
                    Controls.Switch { text: "People"; checked: backend.people; onToggled: backend.people = checked }
                    Controls.Switch { text: "Places"; checked: backend.places; onToggled: backend.places = checked }
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Location" }
                    Controls.TextField {
                        Kirigami.FormData.label: "Folder:"
                        Accessible.name: "Folder"
                        text: backend.location
                        onEditingFinished: backend.location = text
                    }
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: "Dolphin" }
                    PreviewsSwitch {}
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    text: "Open in Dolphin"
                    icon.name: "system-file-manager"
                    enabled: !backend.busy
                    onClicked: backend.finishSetup()
                }
                Controls.BusyIndicator { Layout.alignment: Qt.AlignHCenter; visible: backend.busy; running: visible }
            }

            // Status.
            ColumnLayout {
                visible: backend.page === "status"
                Layout.fillWidth: true
                Kirigami.PlaceholderMessage {
                    Layout.fillWidth: true
                    icon.name: "dev.hviid.ImmichMount"
                    text: backend.signedout ? "Signed Out of Immich" : (backend.mounted ? "Your Library Is in Dolphin" : "Not Mounted")
                    explanation: backend.signedout ? "Sign in again to keep browsing. Your caches are kept."
                        : (backend.mounted ? "Browse it like any folder. Originals download only when you open them."
                        : "The mount is not running. It starts again at your next login.")
                    helpfulAction: Kirigami.Action {
                        icon.name: backend.signedout ? "im-user" : "system-file-manager"
                        text: backend.signedout ? "Sign In Again" : "Open in Dolphin"
                        onTriggered: backend.signedout ? backend.signInAgain() : Qt.openUrlExternally("file://" + backend.location)
                    }
                }
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    Controls.Label { Kirigami.FormData.label: "Server:"; text: backend.serverline }
                    Controls.Label { Kirigami.FormData.label: "Account:"; text: backend.account; visible: text.length > 0 }
                    Controls.Label { Kirigami.FormData.label: "Credential:"; text: backend.credential }
                    Controls.Label { Kirigami.FormData.label: "Connection:"; text: backend.connection }
                    Controls.Label { Kirigami.FormData.label: "Cache:"; text: backend.cache }
                    Controls.Label { Kirigami.FormData.label: "Location:"; text: backend.location }
                    Controls.Label { Kirigami.FormData.label: "Uploading:"; text: backend.uploads; visible: text.length > 0 }
                    PreviewsSwitch { Kirigami.FormData.label: "Dolphin:"; Kirigami.FormData.labelAlignment: Qt.AlignTop }
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    text: "Disconnect"
                    icon.name: "network-disconnect"
                    enabled: !backend.busy
                    onClicked: backend.disconnect()
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    horizontalAlignment: Text.AlignHCenter
                    font: Kirigami.Theme.smallFont
                    opacity: 0.7
                    text: "Immich Mount is an independent, unofficial project. It is not affiliated with, endorsed by, or sponsored by the Immich project or FUTO."
                }
            }
        }
    }
}

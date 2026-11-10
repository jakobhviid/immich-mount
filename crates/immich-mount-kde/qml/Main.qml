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

    // A text in the user's language (ARCHITECTURE.md, "Translations"), its {name} placeholders filled from values.
    function i18n(text, values) {
        let translated = backend.translate(text)
        for (const name in values)
            translated = translated.replace("{" + name + "}", values[name])
        return translated
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
            text: root.i18n("Show thumbnails")
            checked: backend.previews
            onToggled: backend.allowPreviews(checked)
        }
        Controls.Label {
            Layout.maximumWidth: Kirigami.Units.gridUnit * 16
            wrapMode: Text.WordWrap
            font: Kirigami.Theme.smallFont
            opacity: 0.7
            text: root.i18n("Lets Dolphin preview files up to 100 MB on remote locations, this drive included. Thumbnails come from the server; no original is downloaded.")
        }
    }

    // Generated from the settings table, as the GNOME app's dialog is: one row per setting, written at once.
    Component {
        id: preferencesPage
        Kirigami.ScrollablePage {
            id: preferences
            title: i18n("Preferences")
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
                            : (modelData.kind === "bool" || modelData.kind === "app" ? "" : modelData.title + ":")
                        sourceComponent: {
                            switch (modelData.kind) {
                            case "section": return sectionRow
                            case "bool": return switchRow
                            case "app": return appRow
                            case "choice": return choiceRow
                            default: return textRow
                            }
                        }
                        Component { id: sectionRow; Kirigami.Separator {} }
                        Component {
                            id: switchRow
                            RowLayout {
                                Controls.Switch {
                                    text: row.modelData.title
                                    checked: row.modelData.value === "true"
                                    onToggled: preferences.store(row.modelData.key, checked ? "true" : "false")
                                }
                                // Immich crosslink's switch links to the protocol, so it is clear what it is.
                                Kirigami.UrlButton {
                                    visible: row.modelData.link.length > 0
                                    url: row.modelData.link
                                    text: i18n("What is this?")
                                }
                            }
                        }
                        Component {
                            id: appRow
                            // An app that takes crosslinks, by its desktop entry's name and icon, offered in Show in.
                            RowLayout {
                                Controls.Switch {
                                    Accessible.name: row.modelData.title
                                    checked: row.modelData.value === "true"
                                    onToggled: preferences.store(row.modelData.key, checked ? "true" : "false")
                                }
                                // The switch draws no icon, so the app's own is shown beside its name.
                                Kirigami.Icon {
                                    source: row.modelData.icon
                                    visible: row.modelData.icon.length > 0
                                    implicitWidth: Kirigami.Units.iconSizes.small
                                    implicitHeight: Kirigami.Units.iconSizes.small
                                }
                                Controls.Label { text: row.modelData.title }
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
            title: i18n("About")
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
                    text: i18n("Your Immich photos and videos as a drive in Dolphin. By Jakob Hviid, under the MIT license.")
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: i18n("Immich Mount is an independent, unofficial project. It is not affiliated with, endorsed by, or sponsored by the Immich project or FUTO. “Immich” is used only to describe the server this software works with, and remains the mark of its owner.")
                }
                Kirigami.Heading { level: 3; text: i18n("Credits") }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    textFormat: Text.StyledText
                    onLinkActivated: link => Qt.openUrlExternally(link)
                    text: i18n("Inspired by {project}", { project: "<a href=\"https://github.com/Kartax/immich-desktop-app\">Kartax/immich-desktop-app</a>" })
                        + "<br>" + i18n("The Immich API layer: {project}", { project: "<a href=\"https://github.com/boul2gom/immich-rs\">immich-rs</a>" })
                }
                Kirigami.Heading { level: 3; text: i18n("AI disclosure") }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: i18n("Parts of this project were written with the assistance of AI coding agents. All changes are reviewed by the maintainer.")
                }
                Controls.Label {
                    textFormat: Text.StyledText
                    onLinkActivated: link => Qt.openUrlExternally(link)
                    text: i18n("Website: {link}", { link: "<a href=\"https://jakob.hviid.phd/immich-mount/\">jakob.hviid.phd/immich-mount</a>" })
                        + "<br>" + i18n("Source code: {link}", { link: "<a href=\"https://github.com/jakobhviid/immich-mount\">github.com/jakobhviid/immich-mount</a>" })
                }
            }
        }
    }

    pageStack.initialPage: Kirigami.ScrollablePage {
        actions: [
            Kirigami.Action {
                icon.name: "go-previous"
                text: i18n("Back")
                visible: backend.page === "signin" || backend.page === "waiting"
                enabled: !backend.busy
                onTriggered: backend.goBack()
            },
            Kirigami.Action {
                icon.name: "configure"
                text: i18n("Preferences")
                displayHint: Kirigami.DisplayHint.AlwaysHide
                onTriggered: root.pageStack.layers.push(preferencesPage)
            },
            Kirigami.Action {
                icon.name: "help-about"
                text: i18n("About")
                displayHint: Kirigami.DisplayHint.AlwaysHide
                onTriggered: root.pageStack.layers.push(aboutPage)
            }
        ]
        title: {
            switch (backend.page) {
            case "connect": return i18n("Connect")
            case "signin": return i18n("Sign In")
            case "waiting": return i18n("Continue in Your Browser")
            case "setup": return i18n("Your Library in Dolphin")
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
                    text: i18n("Your Immich Library in Dolphin")
                    explanation: i18n("Browse your photos and videos as a drive. Originals download only when you open them.")
                }
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    Controls.TextField {
                        id: addressField
                        Kirigami.FormData.label: i18n("Server address:")
                        Accessible.name: i18n("Server address")
                        text: backend.server
                        placeholderText: "https://photos.example.org"
                        onAccepted: backend.checkServer(text)
                    }
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    text: i18n("Continue")
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
                    text: i18n("Opens your browser, where you sign in as usual, with a passkey or a password manager.")
                }
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    Kirigami.Separator {
                        Kirigami.FormData.isSection: true
                        Kirigami.FormData.label: i18n("Email and password")
                        visible: backend.passwords
                    }
                    Controls.TextField { id: emailField; Kirigami.FormData.label: i18n("Email:"); Accessible.name: i18n("Email"); visible: backend.passwords }
                    Kirigami.PasswordField {
                        id: passwordField
                        Kirigami.FormData.label: i18n("Password:")
                        Accessible.name: i18n("Password")
                        placeholderText: ""
                        visible: backend.passwords
                        onAccepted: backend.signInPassword(emailField.text, text)
                    }
                    Controls.Button {
                        text: i18n("Sign In")
                        visible: backend.passwords
                        enabled: !backend.busy
                        onClicked: backend.signInPassword(emailField.text, passwordField.text)
                    }
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: i18n("API key") }
                    Kirigami.PasswordField {
                        id: keyField
                        Kirigami.FormData.label: i18n("Key:")
                        Accessible.name: i18n("API key")
                        placeholderText: ""
                        onAccepted: backend.signInKey(text)
                    }
                    Controls.Button {
                        text: i18n("Sign In With Key")
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
                text: i18n("Continue in your browser")
                explanation: i18n("Sign in there. When your browser asks to open Immich Mount, allow it.")
                helpfulAction: Kirigami.Action {
                    icon.name: "internet-web-browser"
                    text: i18n("Open the Sign-In Page Again")
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
                    text: i18n("No browser could be opened. Open this address in one:")
                }
                Controls.TextField {
                    Layout.fillWidth: true
                    visible: !root.browserOpened
                    readOnly: true
                    selectByMouse: true
                    Accessible.name: i18n("Sign-in address")
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
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: i18n("Sections") }
                    Controls.Switch { text: i18n("Photos"); checked: backend.photos; onToggled: backend.photos = checked }
                    Controls.Switch { text: i18n("Albums"); checked: backend.albums; onToggled: backend.albums = checked }
                    Controls.Switch { text: i18n("People"); checked: backend.people; onToggled: backend.people = checked }
                    Controls.Switch { text: i18n("Places"); checked: backend.places; onToggled: backend.places = checked }
                    Controls.Switch { text: i18n("Favorites"); checked: backend.favorites; onToggled: backend.favorites = checked }
                    Controls.Switch { text: i18n("Archive"); checked: backend.archive; onToggled: backend.archive = checked }
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: i18n("Location") }
                    Controls.TextField {
                        Kirigami.FormData.label: i18n("Folder:")
                        Accessible.name: i18n("Folder")
                        text: backend.location
                        onEditingFinished: backend.location = text
                    }
                    Kirigami.Separator { Kirigami.FormData.isSection: true; Kirigami.FormData.label: i18n("Dolphin") }
                    PreviewsSwitch {}
                }
                Controls.Button {
                    Layout.alignment: Qt.AlignHCenter
                    text: i18n("Open in Dolphin")
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
                    text: backend.signedout ? i18n("Signed Out of Immich") : (backend.mounted ? i18n("Your Library Is in Dolphin") : i18n("Not Mounted"))
                    explanation: backend.signedout ? i18n("Sign in again to keep browsing. Your caches are kept.")
                        : (backend.mounted ? i18n("Browse it like any folder. Originals download only when you open them.")
                        : i18n("Mount it to browse your library in Dolphin."))
                    helpfulAction: Kirigami.Action {
                        icon.name: backend.signedout ? "im-user" : (backend.mounted ? "system-file-manager" : "media-mount")
                        text: backend.signedout ? i18n("Sign In Again") : (backend.mounted ? i18n("Open in Dolphin") : i18n("Mount"))
                        enabled: !backend.busy
                        onTriggered: backend.signedout ? backend.signInAgain()
                            : (backend.mounted ? Qt.openUrlExternally("file://" + backend.location) : backend.switchMount(true))
                    }
                }
                Kirigami.FormLayout {
                    Layout.fillWidth: true
                    Controls.Label { Kirigami.FormData.label: i18n("Server:"); text: backend.serverline }
                    Controls.Label { Kirigami.FormData.label: i18n("Account:"); text: backend.account; visible: text.length > 0 }
                    Controls.Label { Kirigami.FormData.label: i18n("Credential:"); text: backend.credential }
                    Controls.Label { Kirigami.FormData.label: i18n("Connection:"); text: backend.connection }
                    Controls.Label { Kirigami.FormData.label: i18n("Cache:"); text: backend.cache }
                    Controls.Label { Kirigami.FormData.label: i18n("Location:"); text: backend.location }
                    Controls.Label { Kirigami.FormData.label: i18n("Uploading:"); text: backend.uploads; visible: text.length > 0 }
                    PreviewsSwitch { Kirigami.FormData.label: i18n("Dolphin:"); Kirigami.FormData.labelAlignment: Qt.AlignTop }
                }
                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    Controls.Button {
                        text: i18n("Unmount")
                        icon.name: "media-eject"
                        visible: backend.mounted
                        enabled: !backend.busy
                        onClicked: backend.switchMount(false)
                    }
                    Controls.Button {
                        text: i18n("Disconnect")
                        icon.name: "network-disconnect"
                        enabled: !backend.busy
                        onClicked: backend.disconnect()
                    }
                }
                Controls.Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    horizontalAlignment: Text.AlignHCenter
                    font: Kirigami.Theme.smallFont
                    opacity: 0.7
                    text: i18n("Immich Mount is an independent, unofficial project. It is not affiliated with, endorsed by, or sponsored by the Immich project or FUTO.")
                }
            }
        }
    }
}

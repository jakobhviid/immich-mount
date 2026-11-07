// The user's language (ARCHITECTURE.md, "Translations"). The tables are made from the catalogs in po/, the same
// ones the Linux apps read, by `po/i18n.py apple` when the bridge is built; the English text is the key. SwiftUI
// looks up a view's literal title by itself; everything else comes through `tr`.

import Foundation

/// A text in the user's language, or as it is when the language has no translation of it, with its `{name}`
/// placeholders filled in from `values`.
func tr(_ text: String, _ values: [String: String] = [:]) -> String {
    var translated = Bundle.main.localizedString(forKey: text, value: text, table: nil)
    for (name, value) in values {
        translated = translated.replacingOccurrences(of: "{\(name)}", with: value)
    }
    return translated
}

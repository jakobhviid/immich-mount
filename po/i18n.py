#!/usr/bin/env python3
"""The translations' one tool (ARCHITECTURE.md, "Translations"). The catalogs in this folder are the single source:
one `<language>.po` per language, made from `immich-mount.pot`, which this tool extracts from the sources.

    po/i18n.py extract            write immich-mount.pot from the sources, and bring every .po in line with it
    po/i18n.py check              fail when the template is stale or a translation drops a placeholder (CI runs it)
    po/i18n.py mo DIRECTORY       compile DIRECTORY/<language>/LC_MESSAGES/immich-mount.mo, for Linux
    po/i18n.py apple DIRECTORY    write DIRECTORY/<language>.lproj/Localizable.strings, for the Mac app
    po/i18n.py desktop IN OUT     write the desktop entry IN with its Comment and Keywords in every language, as OUT

The text in English is the key (gettext's msgid) on every platform, and `{name}` marks a value filled in at run
time. Python's standard library only, so it runs on the Mac and in every build container as it is.
"""

import plistlib
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PO = ROOT / "po"
TEMPLATE = PO / "immich-mount.pot"
DOMAIN = "immich-mount"

# The languages, by gettext's code, with the Mac's where it differs. `pt` is Brazilian Portuguese, which both
# platforms also offer for Portugal; `en` is the source and has no catalog.
LANGUAGES = {
    "ar": "ar", "bn": "bn", "da": "da", "de": "de", "es": "es", "fr": "fr", "hi": "hi", "id": "id", "it": "it",
    "ja": "ja", "ko": "ko", "mr": "mr", "pt": "pt", "ru": "ru", "ta": "ta", "te": "te", "tr": "tr", "ur": "ur",
    "vi": "vi", "zh_CN": "zh-Hans",
}

LITERAL = r'"((?:[^"\\]|\\.)*)"'
SWIFT_VIEWS = r"Text|Button|Toggle|Section|Picker|Label|Link|TextField|SecureField|LabeledContent|Menu|GroupBox"

TR = r"\btr\(\s*" + LITERAL
# A row of a table of titles: a key in lower case, then its title in English, which the app translates where it
# shows it, as `("grouping", "Date Folders")`.
TITLE_ROW = r'\("[a-z.]+", ' + LITERAL

# Where translatable text is, and how it is marked: tr("...") in Rust and Swift, i18n("...") in QML, _("...") in
# Blueprint, a SwiftUI view's literal title, a row of a table of titles, and the texts of the settings table, which
# the core keeps unmarked.
SOURCES = [
    ("crates/immich-mount/src/*.rs", [TR]),
    ("crates/immich-mount-linux/src/*.rs", [TR]),
    ("crates/immich-mount-desktop/src/*.rs", [TR, TITLE_ROW]),
    ("crates/immich-mount-gnome/src/*.rs", [TR, TITLE_ROW]),
    ("crates/immich-mount-gnome/src/*.blp", [r"\b_\(\s*" + LITERAL]),
    ("crates/immich-mount-kde/src/*.rs", [TR, TITLE_ROW]),
    ("crates/immich-mount-kde/qml/*.qml", [r"\bi18n\(\s*" + LITERAL]),
    ("crates/immich-mount-core/src/settings.rs", [r"description: " + LITERAL, r'"[A-Za-z0-9-]+" => ' + LITERAL]),
    ("crates/immich-mount-apple/src/lib.rs", [TITLE_ROW]),
    ("apple/App/*.swift", [TR, r"\b(?:" + SWIFT_VIEWS + r")\(\s*" + LITERAL]),
]

# Names and examples, never translated: the drive's own folders among them, which keep their names in every
# language (SPEC.md, "Names"), so a switch or a setting that names one names it as the drive shows it.
UNTRANSLATED = {"Immich Mount", "Immich Drive", "Immich", "photos.example.org", "Photos", "Albums", "People", "Places"}

DESKTOP_ENTRY_KEYS = ("Comment", "Keywords")

# The Mac app's texts the system shows from its Info.plist: the About panel's notice, and the Local Network prompt.
INFO_PLIST = ROOT / "apple/App/Info.plist"
INFO_PLIST_KEYS = ("NSHumanReadableCopyright", "NSLocalNetworkUsageDescription")


def unescape(text):
    """A source literal's value: Rust's line continuation, Unicode escapes (Rust's and Swift's `\\u{201c}`,
    JavaScript's `\\u201c`), and the escapes every one of these languages shares."""
    text = re.sub(r"\\\n\s*", "", text)
    return re.sub(r'\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|.)', unescape_one, text)


def unescape_one(match):
    escape = match.group(1)
    if escape.startswith("u") and len(escape) > 1:
        return chr(int(escape.strip("u{}"), 16))
    return {"n": "\n", "t": "\t"}.get(escape, escape)


def extract():
    """Every translatable text, in the order first seen, with where it is used."""
    found = {}
    problems = []
    for pattern, expressions in SOURCES:
        for path in sorted(ROOT.glob(pattern)):
            source = path.read_text(encoding="utf-8")
            if path.suffix == ".rs":
                # Tests hold no texts anyone sees.
                source = source.split("\n#[cfg(test)]", 1)[0]
            relative = path.relative_to(ROOT)
            for expression in expressions:
                for match in re.finditer(expression, source):
                    text = unescape(match.group(1))
                    line = source.count("\n", 0, match.start()) + 1
                    if "\\(" in match.group(1):
                        problems.append(f"{relative}:{line}: an interpolated text cannot be looked up; use tr() with "
                                        f"{{name}} placeholders: {match.group(1)}")
                        continue
                    if not re.search(r"[A-Za-z]", text) or text in UNTRANSLATED:
                        continue
                    found.setdefault(text, []).append(f"{relative}:{line}")
    for line in desktop_entry_lines(ROOT / "packaging/desktop/dev.hviid.ImmichMount.desktop.in"):
        key, _, value = line.partition("=")
        found.setdefault(value, []).append(f"packaging/desktop/dev.hviid.ImmichMount.desktop.in ({key})")
    for key, value in info_plist_texts().items():
        found.setdefault(value, []).append(f"apple/App/Info.plist ({key})")
    return found, problems


def info_plist_texts():
    with INFO_PLIST.open("rb") as file:
        values = plistlib.load(file)
    return {key: values[key] for key in INFO_PLIST_KEYS if key in values}


def desktop_entry_lines(path):
    return [line for line in path.read_text(encoding="utf-8").splitlines() if line.split("=")[0] in DESKTOP_ENTRY_KEYS]


def quote(text):
    escaped = text.replace("\\", "\\\\").replace('"', '\\"').replace("\t", "\\t")
    if "\n" not in escaped:
        return '"' + escaped.replace("\n", "\\n") + '"'
    parts = escaped.split("\n")
    lines = [part + "\\n" for part in parts[:-1]] + ([parts[-1]] if parts[-1] else [])
    return '""\n' + "\n".join('"' + line + '"' for line in lines)


def header(language):
    lines = [
        "Project-Id-Version: immich-mount",
        "Report-Msgid-Bugs-To: https://github.com/jakobhviid/immich-mount/issues",
        f"Language: {language}" if language else "Language: ",
        "MIME-Version: 1.0",
        "Content-Type: text/plain; charset=UTF-8",
        "Content-Transfer-Encoding: 8bit",
    ]
    return "".join(line + "\n" for line in lines)


def write_catalog(path, entries, language, title):
    """Write a template or a catalog: (text, translation, places) in order, the header first."""
    out = [f"# {title}", "# This file is part of Immich Mount, under the MIT license. Made by po/i18n.py.", "#",
           'msgid ""', "msgstr " + quote(header(language)), ""]
    for text, translation, places in entries:
        out += [f"#: {place}" for place in places]
        out += ["msgid " + quote(text), "msgstr " + quote(translation), ""]
    path.write_text("\n".join(out), encoding="utf-8")


def read_catalog(path):
    """The translations of a .po file, by text, the header left out."""
    translations = {}
    field = None
    current = {}

    def flush():
        if "msgid" in current and current["msgid"]:
            translations[current["msgid"]] = current.get("msgstr", "")

    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line.startswith("msgid "):
            flush()
            current = {}
            field = "msgid"
            current[field] = unescape_po(line[6:])
        elif line.startswith("msgstr "):
            field = "msgstr"
            current[field] = unescape_po(line[7:])
        elif line.startswith('"') and field:
            current[field] += unescape_po(line)
        elif not line or line.startswith("#"):
            field = None if not line else field
    flush()
    return translations


def unescape_po(quoted):
    inner = quoted.strip()[1:-1]
    return re.sub(r'\\(.)', lambda match: {"n": "\n", "t": "\t"}.get(match.group(1), match.group(1)), inner)


def catalogs():
    return {language: read_catalog(PO / f"{language}.po") for language in LANGUAGES if (PO / f"{language}.po").exists()}


def placeholders(text):
    return sorted(re.findall(r"\{[a-z_]+\}", text))


def command_extract():
    found, problems = extract()
    for problem in problems:
        print(problem, file=sys.stderr)
    write_catalog(TEMPLATE, [(text, "", places) for text, places in found.items()], None,
                  "The texts of Immich Mount to translate.")
    for language in LANGUAGES:
        path = PO / f"{language}.po"
        existing = read_catalog(path) if path.exists() else {}
        dropped = [text for text in existing if text not in found]
        write_catalog(path, [(text, existing.get(text, ""), places) for text, places in found.items()], language,
                      f"Immich Mount in {language}.")
        missing = sum(1 for text in found if not existing.get(text))
        print(f"{language}: {len(found) - missing} of {len(found)} translated, {len(dropped)} dropped")
    return 1 if problems else 0


def command_check():
    found, problems = extract()
    stale = TEMPLATE.exists() and set(read_catalog(TEMPLATE)) != set(found) or not TEMPLATE.exists()
    if stale:
        problems.append("po/immich-mount.pot is out of date: run po/i18n.py extract")
    for language, translations in catalogs().items():
        if set(translations) != set(found):
            problems.append(f"po/{language}.po does not match the template: run po/i18n.py extract")
        for text, translation in translations.items():
            if translation and placeholders(translation) != placeholders(text):
                problems.append(f"po/{language}.po: the placeholders of {text!r} differ in {translation!r}")
            if translation and translation.endswith("\n") != text.endswith("\n"):
                problems.append(f"po/{language}.po: {text!r} and its translation end differently")
        untranslated = sum(1 for text in found if not translations.get(text))
        if untranslated:
            print(f"{language}: {untranslated} of {len(found)} texts untranslated, shown in English")
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


def command_mo(directory):
    for language, translations in catalogs().items():
        entries = {text: translation for text, translation in translations.items() if translation}
        entries[""] = header(language)
        keys = sorted(entries, key=lambda text: text.encode("utf-8"))
        ids = b""
        strings = b""
        offsets = []
        for key in keys:
            original = key.encode("utf-8")
            translated = entries[key].encode("utf-8")
            offsets.append((len(ids), len(original), len(strings), len(translated)))
            ids += original + b"\0"
            strings += translated + b"\0"
        # The layout of GNU gettext's .mo: a header, the table of originals, the table of translations, the texts.
        start_of_ids = 7 * 4 + 16 * len(keys)
        start_of_strings = start_of_ids + len(ids)
        original_table = b"".join(struct.pack("<II", length, start_of_ids + offset)
                                  for offset, length, _, _ in offsets)
        translation_table = b"".join(struct.pack("<II", length, start_of_strings + offset)
                                     for _, _, offset, length in offsets)
        out = struct.pack("<Iiiiiii", 0x950412DE, 0, len(keys), 7 * 4, 7 * 4 + 8 * len(keys), 0, 0)
        target = Path(directory) / language / "LC_MESSAGES" / f"{DOMAIN}.mo"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(out + original_table + translation_table + ids + strings)
    return 0


def apple_quote(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'


def command_apple(directory):
    # English is a localization of its own, so a Mac set to English, then another language, stays in English.
    tables = {"en": {}}
    for language, translations in catalogs().items():
        tables[LANGUAGES[language]] = translations
    for language, translations in tables.items():
        lines = [f"{apple_quote(text)} = {apple_quote(translation)};"
                 for text, translation in translations.items() if translation]
        target = Path(directory) / f"{language}.lproj" / "Localizable.strings"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("/* Made by po/i18n.py from po/. */\n" + "\n".join(lines) + "\n", encoding="utf-8")
        keys = [f"{apple_quote(key)} = {apple_quote(translations[value])};"
                for key, value in info_plist_texts().items() if translations.get(value)]
        (target.parent / "InfoPlist.strings").write_text("/* Made by po/i18n.py from po/. */\n" + "\n".join(keys) + "\n",
                                                          encoding="utf-8")
    return 0


def command_desktop(source, target):
    translated = catalogs()
    out = []
    for line in Path(source).read_text(encoding="utf-8").splitlines():
        out.append(line)
        key, _, value = line.partition("=")
        if key in DESKTOP_ENTRY_KEYS:
            out += [f"{key}[{language}]={translations[value]}"
                    for language, translations in sorted(translated.items()) if translations.get(value)]
    Path(target).write_text("\n".join(out) + "\n", encoding="utf-8")
    return 0


def main(arguments):
    commands = {"extract": (command_extract, 0), "check": (command_check, 0), "mo": (command_mo, 1),
                "apple": (command_apple, 1), "desktop": (command_desktop, 2)}
    if not arguments or arguments[0] not in commands or len(arguments) - 1 != commands[arguments[0]][1]:
        print(__doc__, file=sys.stderr)
        return 2
    command, _ = commands[arguments[0]]
    return command(*arguments[1:])


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

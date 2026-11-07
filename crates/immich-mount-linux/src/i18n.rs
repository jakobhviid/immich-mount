//! The user's language (ARCHITECTURE.md, "Translations"): the catalogs `po/i18n.py mo` compiles, read here without
//! the C library's gettext, so the static daemon reads them too. The language is chosen by gettext's own rules, so
//! the GNOME app's Blueprint texts, which GTK looks up through gettext, agree with the rest.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// The catalogs' name, `<locale directory>/<language>/LC_MESSAGES/immich-mount.mo`.
pub const DOMAIN: &str = "immich-mount";

/// A text in the user's language, or as it is when the language has no catalog or the catalog lacks the text.
/// `{name}` placeholders are left for the caller to fill in with `replace`.
pub fn tr(text: &str) -> &str {
    static CATALOG: OnceLock<HashMap<String, String>> = OnceLock::new();
    let catalog = CATALOG.get_or_init(|| {
        let Some(directory) = locale_directory() else { return HashMap::new() };
        languages(&|name| std::env::var(name).ok())
            .iter()
            .find_map(|language| std::fs::read(directory.join(language).join("LC_MESSAGES").join(format!("{DOMAIN}.mo"))).ok())
            .map(|bytes| read_mo(&bytes))
            .unwrap_or_default()
    });
    catalog.get(text).map_or(text, String::as_str)
}

/// Where the catalogs are, beside the program: `share/locale` next to it (Homebrew's cask keeps everything in one
/// folder), or in `../share/locale` (`/usr/bin` and `/usr/share/locale`, and a Flatpak's `/app`).
pub fn locale_directory() -> Option<PathBuf> {
    let program = std::env::current_exe().ok()?;
    let folder = program.parent()?;
    [folder.join("share/locale"), folder.join("../share/locale")].into_iter().find(|candidate| candidate.is_dir())
}

/// The catalogs to try, most wanted first, by gettext's rules: `LANGUAGE`'s list, unless the locale is C, then the
/// first of `LC_ALL`, `LC_MESSAGES` and `LANG` that is set. Each as `da_DK`, then `da`.
fn languages(variable: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|name| variable(name).filter(|value| !value.is_empty()));
    let Some(locale) = locale else { return Vec::new() };
    if locale == "C" || locale == "POSIX" || locale.starts_with("C.") {
        return Vec::new();
    }
    let wanted = variable("LANGUAGE").filter(|value| !value.is_empty()).unwrap_or_else(|| locale.clone());
    let mut out = Vec::new();
    for entry in wanted.split(':') {
        let name = entry.split(['.', '@']).next().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        for candidate in [Some(name), name.split('_').next()].into_iter().flatten() {
            if !out.iter().any(|known: &String| known == candidate) {
                out.push(candidate.to_owned());
            }
        }
    }
    out
}

/// The translations of a GNU `.mo` file, by text; empty when it is not one.
fn read_mo(bytes: &[u8]) -> HashMap<String, String> {
    let word = |at: usize, big_endian: bool| -> Option<usize> {
        let field: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(if big_endian { u32::from_be_bytes(field) } else { u32::from_le_bytes(field) } as usize)
    };
    let big_endian = match word(0, false) {
        Some(0x950412de) => false,
        Some(0xde120495) => true,
        _ => return HashMap::new(),
    };
    let text = |table: usize, index: usize| -> Option<String> {
        let length = word(table + 8 * index, big_endian)?;
        let offset = word(table + 8 * index + 4, big_endian)?;
        String::from_utf8(bytes.get(offset..offset + length)?.to_vec()).ok()
    };
    let (Some(count), Some(originals), Some(translations)) =
        (word(8, big_endian), word(12, big_endian), word(16, big_endian))
    else {
        return HashMap::new();
    };
    (0..count)
        .filter_map(|index| Some((text(originals, index)?, text(translations, index)?)))
        .filter(|(original, _)| !original.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
        move |name| pairs.iter().find(|(known, _)| known == name).map(|(_, value)| value.clone())
    }

    #[test]
    fn languages_follow_gettexts_rules() {
        assert_eq!(languages(&environment(&[("LANG", "da_DK.UTF-8")])), ["da_DK", "da"]);
        assert_eq!(languages(&environment(&[("LANG", "en_US.UTF-8"), ("LC_ALL", "de_DE.UTF-8")])), ["de_DE", "de"]);
        assert_eq!(languages(&environment(&[("LANG", "da_DK.UTF-8"), ("LANGUAGE", "pt_BR:fr")])), ["pt_BR", "pt", "fr"]);
        assert_eq!(languages(&environment(&[("LANG", "sr_RS@latin")])), ["sr_RS", "sr"]);
        // LANGUAGE is ignored under the C locale, as gettext ignores it.
        assert!(languages(&environment(&[("LANG", "C.UTF-8"), ("LANGUAGE", "da")])).is_empty());
        assert!(languages(&environment(&[])).is_empty());
    }

    #[test]
    fn reads_the_catalogs_the_tool_compiles() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let catalog = directory.path().join("da.po");
        std::fs::write(&catalog, "msgid \"\"\nmsgstr \"Content-Type: text/plain; charset=UTF-8\\n\"\n\nmsgid \"Sign In\"\nmsgstr \"Log ind\"\n\nmsgid \"Back\"\nmsgstr \"\"\n")
            .expect("the catalog is written");
        let compiled = compile(directory.path(), &catalog);
        let translations = read_mo(&std::fs::read(compiled).expect("the tool compiled it"));
        assert_eq!(translations.get("Sign In").map(String::as_str), Some("Log ind"));
        assert_eq!(translations.len(), 1, "untranslated texts and the header are left out: {translations:?}");
        assert!(read_mo(b"not a catalog").is_empty());
    }

    /// Compile one catalog with `po/i18n.py`'s own compiler, by loading the tool as a module.
    fn compile(directory: &Path, catalog: &Path) -> PathBuf {
        let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../po/i18n.py");
        let script = "import importlib.util, sys\nfrom pathlib import Path\nspec = importlib.util.spec_from_file_location('i18n', sys.argv[1])\ntool = importlib.util.module_from_spec(spec)\nspec.loader.exec_module(tool)\ntool.LANGUAGES = {'da': 'da'}\ntool.PO = Path(sys.argv[2]).parent\ntool.command_mo(sys.argv[3])\n";
        let status = std::process::Command::new("python3")
            .args(["-c", script])
            .arg(&tool)
            .arg(catalog)
            .arg(directory.join("locale"))
            .status()
            .expect("python3 runs");
        assert!(status.success());
        directory.join("locale/da/LC_MESSAGES").join(format!("{DOMAIN}.mo"))
    }
}

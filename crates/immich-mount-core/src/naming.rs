//! File and folder names in the mount: building them, sanitising them, and resolving clashes.
//!
//! The rules are in SPEC.md, "Names", and the reasons in ARCHITECTURE.md, "Names". In short: files are named by
//! their local capture time (`2024-03-15 14.32.07.jpg`) or by their original name; a clash keeps the plain name for
//! the entry uploaded first and gives the others a counter (`-1`, `-2`) in upload order. Names already handed out are
//! remembered by the caller and passed back in, so a name never moves to another entry while its holder exists.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, NaiveDateTime, Utc};
use unicode_normalization::UnicodeNormalization;

/// The longest name a Linux filesystem accepts, in bytes.
const MAXIMUM_NAME_BYTES: usize = 255;
/// The longest name before a clash counter, leaving room for the longest one (`-4294967295`), so a counted name is
/// never over the limit.
const BASE_NAME_BYTES: usize = MAXIMUM_NAME_BYTES - 11;
/// The longest extension kept apart when a name is cut; anything longer is just part of the name.
const MAXIMUM_EXTENSION_BYTES: usize = 16;

/// How files are named (`files.naming`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileNaming {
    /// `2024-03-15 14.32.07.jpg`, from the local capture time.
    DateTime,
    /// The original file name, as uploaded.
    Original,
}

/// The base name for a file, before clashes are resolved.
///
/// `local_capture_time` is Immich's `localDateTime`: the wall-clock time where the photo was taken. The extension is
/// kept exactly as in the original file name.
pub fn file_base_name(naming: FileNaming, original_file_name: &str, local_capture_time: NaiveDateTime) -> String {
    let base = match naming {
        FileNaming::Original => original_file_name.to_string(),
        FileNaming::DateTime => {
            let stamp = local_capture_time.format("%Y-%m-%d %H.%M.%S").to_string();
            match extension_of(original_file_name) {
                Some(extension) => format!("{stamp}.{extension}"),
                None => stamp,
            }
        }
    };
    sanitise(&base, true)
}

/// Make a name safe for the filesystem, without changing it more than necessary.
///
/// `/` becomes `_`, NUL is removed, surrounding whitespace is trimmed, `.` and `..` become `_`, and names over 244
/// bytes are cut on a UTF-8 boundary (leaving room for a clash counter), keeping an extension of up to 16 bytes when
/// `is_file` is set. A leading `.` is kept.
pub fn sanitise(name: &str, is_file: bool) -> String {
    let cleaned: String = name.chars().filter(|character| *character != '\0').map(|character| {
        if character == '/' { '_' } else { character }
    }).collect();
    let trimmed = cleaned.trim();
    let safe = match trimmed {
        "" | "." | ".." => "_".to_string(),
        other => other.to_string(),
    };
    truncate_to_limit(&safe, is_file)
}

fn truncate_to_limit(name: &str, is_file: bool) -> String {
    if name.len() <= BASE_NAME_BYTES {
        return name.to_string();
    }
    let extension = if is_file { extension_of(name).filter(|extension| extension.len() <= MAXIMUM_EXTENSION_BYTES) } else { None };
    let suffix = extension.map(|extension| format!(".{extension}")).unwrap_or_default();
    let stem_budget = BASE_NAME_BYTES.saturating_sub(suffix.len());
    let stem = if suffix.is_empty() { name } else { &name[..name.len() - suffix.len()] };
    format!("{}{suffix}", cut_on_char_boundary(stem, stem_budget))
}

fn cut_on_char_boundary(text: &str, budget: usize) -> &str {
    if text.len() <= budget {
        return text;
    }
    let mut end = budget;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The extension of a file name, without the dot. A leading dot alone (`.hidden`) is not an extension.
fn extension_of(file_name: &str) -> Option<&str> {
    let (stem, extension) = file_name.rsplit_once('.')?;
    if stem.is_empty() || extension.is_empty() || extension.contains(' ') {
        None
    } else {
        Some(extension)
    }
}

/// The key two names clash on: NFC-normalised and lower-cased, so copies to exFAT, NTFS or macOS never collide and
/// an NFD name from an iPhone matches its NFC twin.
pub fn clash_key(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// One entry that needs a name in a folder.
#[derive(Debug, Clone)]
pub struct NameRequest {
    /// A stable identity for the entry, such as an asset, album or person id.
    pub key: String,
    /// The name it would get without clashes (already sanitised).
    pub base: String,
    /// When the server first knew the entry (an Asset's upload time). Decides who keeps the plain name.
    pub created_at: DateTime<Utc>,
    /// Whether the counter goes before an extension (files) or at the end (folders).
    pub is_file: bool,
}

/// Assign every entry in one folder a unique name.
///
/// `remembered` maps entry keys to the names they were given before. A remembered name is kept as long as it was
/// derived from the same base name, so names never move to another entry while their holder exists. New entries
/// take the plain name if it is free, otherwise the next counter after the highest one in use for that base.
pub fn assign_names(requests: &[NameRequest], remembered: &HashMap<String, String>) -> HashMap<String, String> {
    let mut assigned: HashMap<String, String> = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();

    for request in requests {
        if let Some(previous) = remembered.get(&request.key) {
            let still_valid = derives_from(previous, &request.base, request.is_file);
            if still_valid && taken.insert(clash_key(previous)) {
                assigned.insert(request.key.clone(), previous.clone());
            }
        }
    }

    let mut newcomers: Vec<&NameRequest> = requests.iter().filter(|request| !assigned.contains_key(&request.key)).collect();
    newcomers.sort_by(|left, right| left.created_at.cmp(&right.created_at).then_with(|| left.key.cmp(&right.key)));

    for request in newcomers {
        let name = if taken.contains(&clash_key(&request.base)) {
            let next_counter = highest_counter_in_use(&taken, &request.base, request.is_file) + 1;
            with_counter(&request.base, next_counter, request.is_file)
        } else {
            request.base.clone()
        };
        taken.insert(clash_key(&name));
        assigned.insert(request.key.clone(), name);
    }
    assigned
}

/// `IMG_0001.JPG` with counter 2 becomes `IMG_0001-2.JPG`; a folder `Anna` becomes `Anna-2`.
pub fn with_counter(base: &str, counter: u32, is_file: bool) -> String {
    match (is_file, extension_of(base)) {
        (true, Some(extension)) => {
            let stem = &base[..base.len() - extension.len() - 1];
            format!("{stem}-{counter}.{extension}")
        }
        _ => format!("{base}-{counter}"),
    }
}

/// Whether `name` is `base` itself, or `base` with a counter.
fn derives_from(name: &str, base: &str, is_file: bool) -> bool {
    clash_key(name) == clash_key(base) || counter_of(name, base, is_file).is_some()
}

/// The counter in `name` if it is `base` with a counter.
fn counter_of(name: &str, base: &str, is_file: bool) -> Option<u32> {
    let name_key = clash_key(name);
    let (stem, extension) = match (is_file, extension_of(base)) {
        (true, Some(extension)) => (&base[..base.len() - extension.len() - 1], Some(extension)),
        _ => (base, None),
    };
    let prefix = format!("{}-", clash_key(stem));
    let rest = name_key.strip_prefix(&prefix)?;
    let digits = match extension {
        Some(extension) => rest.strip_suffix(&format!(".{}", clash_key(extension)))?,
        None => rest,
    };
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn highest_counter_in_use(taken: &HashSet<String>, base: &str, is_file: bool) -> u32 {
    taken.iter().filter_map(|name| counter_of(name, base, is_file)).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    fn capture(hour: u32, minute: u32, second: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2024, 3, 15).unwrap().and_hms_opt(hour, minute, second).unwrap()
    }

    fn uploaded(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 4, 1, 12, minute, 0).unwrap()
    }

    fn request(key: &str, base: &str, minute: u32) -> NameRequest {
        NameRequest { key: key.into(), base: base.into(), created_at: uploaded(minute), is_file: true }
    }

    #[test]
    fn date_time_names_use_dots_and_keep_the_extension() {
        assert_eq!(file_base_name(FileNaming::DateTime, "IMG_1234.JPG", capture(14, 32, 7)), "2024-03-15 14.32.07.JPG");
        assert_eq!(file_base_name(FileNaming::DateTime, "clip.mov", capture(9, 5, 0)), "2024-03-15 09.05.00.mov");
        assert_eq!(file_base_name(FileNaming::DateTime, "no-extension", capture(9, 5, 0)), "2024-03-15 09.05.00");
    }

    #[test]
    fn original_names_are_kept() {
        assert_eq!(file_base_name(FileNaming::Original, "IMG_1234.JPG", capture(0, 0, 0)), "IMG_1234.JPG");
    }

    #[test]
    fn sanitising_fixes_only_what_is_unsafe() {
        assert_eq!(sanitise("a/b", false), "a_b");
        assert_eq!(sanitise("  spaced  ", false), "spaced");
        assert_eq!(sanitise("..", false), "_");
        assert_eq!(sanitise("nul\0byte", false), "nulbyte");
        assert_eq!(sanitise(".hidden", false), ".hidden");
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary_keeping_the_extension() {
        let long = format!("{}.jpg", "æ".repeat(200));
        let cut = sanitise(&long, true);
        assert!(cut.len() <= BASE_NAME_BYTES);
        assert!(cut.ends_with(".jpg"));
        assert!(cut.is_char_boundary(cut.len() - 4));
        assert!(with_counter(&cut, u32::MAX, true).len() <= MAXIMUM_NAME_BYTES, "a counter still fits");
        let hostile = format!("photo.{}", "x".repeat(1100));
        assert!(sanitise(&hostile, true).len() <= BASE_NAME_BYTES, "an absurd extension is cut like the rest");
    }

    #[test]
    fn first_uploaded_keeps_the_plain_name_and_others_count_up() {
        let requests = vec![request("b", "2024-03-15 14.32.07.jpg", 2), request("a", "2024-03-15 14.32.07.jpg", 1), request("c", "2024-03-15 14.32.07.jpg", 3)];
        let names = assign_names(&requests, &HashMap::new());
        assert_eq!(names["a"], "2024-03-15 14.32.07.jpg");
        assert_eq!(names["b"], "2024-03-15 14.32.07-1.jpg");
        assert_eq!(names["c"], "2024-03-15 14.32.07-2.jpg");
    }

    #[test]
    fn remembered_names_never_move_when_an_earlier_entry_disappears() {
        let remembered: HashMap<String, String> = [("b", "IMG_0001-1.JPG"), ("c", "IMG_0001-2.JPG")]
            .into_iter().map(|(key, name)| (key.to_string(), name.to_string())).collect();
        let requests = vec![request("b", "IMG_0001.JPG", 2), request("c", "IMG_0001.JPG", 3)];
        let names = assign_names(&requests, &remembered);
        assert_eq!(names["b"], "IMG_0001-1.JPG");
        assert_eq!(names["c"], "IMG_0001-2.JPG");
    }

    #[test]
    fn a_new_duplicate_never_takes_an_existing_name() {
        let remembered: HashMap<String, String> = [("a", "IMG_0001.JPG"), ("b", "IMG_0001-1.JPG")]
            .into_iter().map(|(key, name)| (key.to_string(), name.to_string())).collect();
        let requests = vec![request("a", "IMG_0001.JPG", 1), request("b", "IMG_0001.JPG", 2), request("new", "IMG_0001.JPG", 0)];
        let names = assign_names(&requests, &remembered);
        assert_eq!(names["a"], "IMG_0001.JPG");
        assert_eq!(names["new"], "IMG_0001-2.JPG");
    }

    #[test]
    fn clashes_ignore_case_and_unicode_form() {
        let decomposed = "Cafe\u{301}.jpg";
        let composed = "CAFÉ.JPG";
        let requests = vec![request("a", composed, 1), request("b", decomposed, 2)];
        let names = assign_names(&requests, &HashMap::new());
        assert_eq!(names["a"], composed);
        assert_eq!(names["b"], "Cafe\u{301}-1.jpg");
    }

    #[test]
    fn a_name_whose_base_changed_is_reassigned() {
        let remembered: HashMap<String, String> = [("a", "2024-03-15 14.32.07.jpg")]
            .into_iter().map(|(key, name)| (key.to_string(), name.to_string())).collect();
        let requests = vec![request("a", "2024-03-16 08.00.00.jpg", 1)];
        assert_eq!(assign_names(&requests, &remembered)["a"], "2024-03-16 08.00.00.jpg");
    }

    #[test]
    fn folders_count_at_the_end() {
        let folder = |key: &str, minute| NameRequest { key: key.into(), base: "v1.2 Trip".into(), created_at: uploaded(minute), is_file: false };
        let names = assign_names(&[folder("a", 1), folder("b", 2)], &HashMap::new());
        assert_eq!(names["b"], "v1.2 Trip-1");
    }
}

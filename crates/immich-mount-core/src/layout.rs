//! The shape of the mount: sections, year, month and day folders, album folders, and when folders are split.
//!
//! The rules are in SPEC.md, "Layout". Folder membership itself comes from Immich; this module only decides how
//! Immich's answers are laid out as folders.

use crate::naming::sanitise;

/// A top-level folder of the mount, mirroring an Immich section and carrying Immich's own label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    Photos,
    Albums,
    People,
    Places,
    Archive,
}

impl Section {
    /// Every section, in the order the root lists them.
    pub const ALL: [Section; 5] = [Section::Photos, Section::Albums, Section::People, Section::Places, Section::Archive];

    /// The folder name, using Immich's label.
    pub fn folder_name(self) -> &'static str {
        match self {
            Section::Photos => "Photos",
            Section::Albums => "Albums",
            Section::People => "People",
            Section::Places => "Places",
            Section::Archive => "Archive",
        }
    }
}

/// The one folder that takes new files: what is copied into it is uploaded, and leaves it once Immich has it
/// (SPEC.md, "Uploads"). Every other folder is a read-only view of the library.
pub const UPLOAD_FOLDER: &str = "Upload";

/// When a folder is split into date folders (`grouping.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupingMode {
    Never,
    AtLeast500,
    Always,
}

/// The size at which `AtLeast500` starts splitting.
pub const GROUPING_THRESHOLD: usize = 500;

impl GroupingMode {
    /// Whether a folder holding `asset_count` Assets is split.
    pub fn splits(self, asset_count: usize) -> bool {
        match self {
            GroupingMode::Never => false,
            GroupingMode::AtLeast500 => asset_count >= GROUPING_THRESHOLD,
            GroupingMode::Always => asset_count > 0,
        }
    }
}

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

/// `2024` for a year folder.
pub fn year_folder_name(year: i32) -> String {
    year.to_string()
}

/// `03 March` for a month folder: the number first, so name order is chronological. Returns `None` for an invalid
/// month.
pub fn month_folder_name(month: u32) -> Option<String> {
    let name = MONTH_NAMES.get(month.checked_sub(1)? as usize)?;
    Some(format!("{month:02} {name}"))
}

/// `15` for a day folder inside a split month.
pub fn day_folder_name(day: u32) -> String {
    format!("{day:02}")
}

/// How album folders are named and grouped (`albums.layout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumLayout {
    /// `Albums/2023 - Summer trip/`
    YearPrefix,
    /// `Albums/2023/Summer trip/`
    YearFolders,
    /// `Albums/Summer trip/`
    Flat,
}

/// The folder for albums without photos, with `YearFolders`.
pub const UNKNOWN_YEAR_FOLDER: &str = "Unknown Year";

/// An album's folder name, before clashes are resolved. `year` is the year Immich groups the album under (its most
/// recent photo, `endDate`); an empty album has none and is shown without a year.
pub fn album_folder_base(layout: AlbumLayout, album_name: &str, year: Option<i32>) -> String {
    match (layout, year) {
        (AlbumLayout::YearPrefix, Some(year)) => sanitise(&format!("{year} - {album_name}"), false),
        _ => sanitise(album_name, false),
    }
}

/// The year folder an album sits in, with `YearFolders`.
pub fn album_year_folder(year: Option<i32>) -> String {
    year.map(year_folder_name).unwrap_or_else(|| UNKNOWN_YEAR_FOLDER.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_use_immichs_labels_in_order() {
        let names: Vec<&str> = Section::ALL.iter().map(|section| section.folder_name()).collect();
        assert_eq!(names, ["Photos", "Albums", "People", "Places", "Archive"]);
    }

    #[test]
    fn grouping_splits_at_the_threshold() {
        assert!(!GroupingMode::AtLeast500.splits(499));
        assert!(GroupingMode::AtLeast500.splits(500));
        assert!(!GroupingMode::Never.splits(10_000));
        assert!(GroupingMode::Always.splits(1));
        assert!(!GroupingMode::Always.splits(0));
    }

    #[test]
    fn month_and_day_folders_sort_chronologically_by_name() {
        assert_eq!(month_folder_name(3).as_deref(), Some("03 March"));
        assert_eq!(month_folder_name(12).as_deref(), Some("12 December"));
        assert_eq!(month_folder_name(0), None);
        assert_eq!(month_folder_name(13), None);
        assert_eq!(day_folder_name(5), "05");
    }

    #[test]
    fn album_folders_follow_the_layout() {
        assert_eq!(album_folder_base(AlbumLayout::YearPrefix, "Summer trip", Some(2023)), "2023 - Summer trip");
        assert_eq!(album_folder_base(AlbumLayout::YearPrefix, "Empty", None), "Empty");
        assert_eq!(album_folder_base(AlbumLayout::YearFolders, "Summer trip", Some(2023)), "Summer trip");
        assert_eq!(album_folder_base(AlbumLayout::Flat, "a/b", Some(2023)), "a_b");
        assert_eq!(album_year_folder(Some(2023)), "2023");
        assert_eq!(album_year_folder(None), "Unknown Year");
    }
}

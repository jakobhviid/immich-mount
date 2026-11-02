//! The library as the core sees it: Assets, albums, people and timeline months, independent of the HTTP client.
//!
//! The Immich client's types are mapped onto these, so the core's logic never depends on wire formats.

use chrono::{DateTime, Datelike, NaiveDateTime, Utc};

/// A photo or video in the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub id: String,
    pub original_file_name: String,
    /// The wall-clock time where it was taken (Immich's `localDateTime`). Decides date folders and date-time names.
    pub local_capture_time: NaiveDateTime,
    /// The capture instant (Immich's `fileCreatedAt`). Becomes the file's mtime.
    pub captured_at: DateTime<Utc>,
    /// When the server first knew it (Immich's `createdAt`). Decides who keeps a clashing plain name.
    pub uploaded_at: DateTime<Utc>,
    /// The original's size in bytes, when Immich reports it.
    pub size: Option<u64>,
    pub city: Option<String>,
    pub country: Option<String>,
}

impl Asset {
    pub fn year(&self) -> i32 {
        self.local_capture_time.year()
    }
    pub fn month(&self) -> u32 {
        self.local_capture_time.month()
    }
    pub fn day(&self) -> u32 {
        self.local_capture_time.day()
    }
}

/// An album.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    pub id: String,
    pub name: String,
    /// The year Immich files it under: that of its most recent photo (`endDate`), as the web app groups albums by
    /// year. `None` for an empty album.
    pub year: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// A named person.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub is_hidden: bool,
    pub created_at: DateTime<Utc>,
}

/// A calendar month with Assets in Immich's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

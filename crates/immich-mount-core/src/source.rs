//! Where the library comes from: the trait the tree reads through.
//!
//! The Immich client implements it in production; tests use an in-memory library. Membership always comes from the
//! source (Immich decides, ARCHITECTURE.md, "Membership comes from Immich").

use std::future::Future;

use crate::model::{Album, Asset, Person, YearMonth};

/// Why the library could not be read. The FUSE adapter maps each kind to an error code (SPEC.md, "Errors").
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum SourceError {
    #[error("the Immich server cannot be reached: {0}")]
    Offline(String),
    #[error("signed out of Immich")]
    SignedOut,
    #[error("the credential lacks a permission{}", .0.as_deref().map(|name| format!(": {name}")).unwrap_or_default())]
    MissingPermission(Option<String>),
    #[error("not found")]
    NotFound,
    #[error("the server is busy; try again")]
    Busy,
    #[error("unexpected answer from Immich: {0}")]
    Protocol(String),
    /// The original no longer matches what the listing said (its size changed on the server).
    #[error("the original changed on the server")]
    Changed,
}

/// What changed in the library since the last look, as far as the source can tell. Empty when nothing did, or when
/// the source cannot say (the freshness windows still apply).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryChanges {
    /// The set of timeline months, or their counts, changed.
    pub timeline_months_changed: bool,
    /// Months whose Assets changed.
    pub months: Vec<YearMonth>,
    /// The list of albums changed: one was added, removed or renamed.
    pub album_list_changed: bool,
    /// Albums whose Assets changed.
    pub albums: Vec<String>,
    /// Something changed whose place is unknown (an Asset deleted for good, or the server asking for a full resync):
    /// every listing is read again.
    pub everything: bool,
}

impl LibraryChanges {
    pub fn is_empty(&self) -> bool {
        *self == LibraryChanges::default()
    }
}

/// The library, as Immich presents it.
pub trait LibrarySource: Send + Sync {
    /// The timeline's months that hold Assets, with counts.
    fn timeline_months(&self) -> impl Future<Output = Result<Vec<(YearMonth, u32)>, SourceError>> + Send;
    /// The Assets of one timeline month, with their details.
    fn month_assets(&self, month: YearMonth) -> impl Future<Output = Result<Vec<Asset>, SourceError>> + Send;
    fn albums(&self) -> impl Future<Output = Result<Vec<Album>, SourceError>> + Send;
    fn album_assets(&self, album_id: &str) -> impl Future<Output = Result<Vec<Asset>, SourceError>> + Send;
    fn people(&self) -> impl Future<Output = Result<Vec<Person>, SourceError>> + Send;
    fn person_assets(&self, person_id: &str) -> impl Future<Output = Result<Vec<Asset>, SourceError>> + Send;
    fn countries(&self) -> impl Future<Output = Result<Vec<String>, SourceError>> + Send;
    fn cities(&self, country: &str) -> impl Future<Output = Result<Vec<String>, SourceError>> + Send;
    fn city_assets(&self, country: &str, city: &str) -> impl Future<Output = Result<Vec<Asset>, SourceError>> + Send;
    /// What changed since the last call. The first call only takes note of the current state.
    fn changes(&self) -> impl Future<Output = Result<LibraryChanges, SourceError>> + Send {
        async { Ok(LibraryChanges::default()) }
    }
}

/// One source can serve several users (the tree and the content cache) through an `Arc`.
impl<Shared: LibrarySource> LibrarySource for std::sync::Arc<Shared> {
    async fn timeline_months(&self) -> Result<Vec<(YearMonth, u32)>, SourceError> {
        self.as_ref().timeline_months().await
    }
    async fn month_assets(&self, month: YearMonth) -> Result<Vec<Asset>, SourceError> {
        self.as_ref().month_assets(month).await
    }
    async fn albums(&self) -> Result<Vec<Album>, SourceError> {
        self.as_ref().albums().await
    }
    async fn album_assets(&self, album_id: &str) -> Result<Vec<Asset>, SourceError> {
        self.as_ref().album_assets(album_id).await
    }
    async fn people(&self) -> Result<Vec<Person>, SourceError> {
        self.as_ref().people().await
    }
    async fn person_assets(&self, person_id: &str) -> Result<Vec<Asset>, SourceError> {
        self.as_ref().person_assets(person_id).await
    }
    async fn countries(&self) -> Result<Vec<String>, SourceError> {
        self.as_ref().countries().await
    }
    async fn cities(&self, country: &str) -> Result<Vec<String>, SourceError> {
        self.as_ref().cities(country).await
    }
    async fn city_assets(&self, country: &str, city: &str) -> Result<Vec<Asset>, SourceError> {
        self.as_ref().city_assets(country, city).await
    }
    async fn changes(&self) -> Result<LibraryChanges, SourceError> {
        self.as_ref().changes().await
    }
}

/// Which of Immich's own thumbnails to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailKind {
    /// A small WebP, 250 px on the short side.
    Small,
    /// A JPEG, 1440 px on the short side.
    Preview,
}

/// Where the thumbnails that stand in for originals come from (ARCHITECTURE.md, "Seeding the freedesktop cache").
pub trait ThumbnailSource: Send + Sync {
    /// The encoded image, exactly as the server made it.
    fn thumbnail(&self, asset_id: &str, kind: ThumbnailKind) -> impl Future<Output = Result<Vec<u8>, SourceError>> + Send;
}

impl<Shared: ThumbnailSource> ThumbnailSource for std::sync::Arc<Shared> {
    async fn thumbnail(&self, asset_id: &str, kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
        self.as_ref().thumbnail(asset_id, kind).await
    }
}

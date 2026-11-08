//! The Immich REST client and models: the Immich API for Immich Mount and for other apps built on this crate.
//!
//! Portable: no FUSE, D-Bus, GTK or OS-specific code (ARCHITECTURE.md, "Crates"). The models and request plumbing
//! started from immich-rs (MIT; THIRD-PARTY-NOTICES.md). The endpoints beyond the mount's are written from the
//! OpenAPI spec, learning which ones a full client needs from ImmichGallery and Mimick (CREDITS.md).
//!
//! Design rules that hold across the crate:
//! - Immich v3.2 and later (`version::MINIMUM_SUPPORTED`). Every endpoint is listed in [`endpoints`] with the
//!   permission it needs, and each is checked by a contract test against the lab server
//!   (`tests/contract*.rs`), including that the server's own `/api/spec.json` still has it.
//! - Decoding is lenient: a field missing on an older server, a new one, or `null` never breaks decoding
//!   (`wire`). The conversion to our types insists only on what a caller cannot do without.
//! - Every failure is one [`ApiError`], whose variants are the kinds callers branch on.
//! - Dates are `chrono`: `DateTime<Utc>` for instants, `NaiveDateTime` for wall-clock times.
//! - Concurrency limits and deadlines are the caller's; the client only retries briefly (`retry`), and resends a
//!   call whose second sending would do something else (a create, a password change) only when its first attempt
//!   cannot have taken effect.
//! - Secrets (key values, tokens, passwords, PIN codes, link keys) are [`Secret`]s, redacted in `Debug` output.
//! - A method's documentation names the API-key permission it needs.
//!
//! Every endpoint is a method on [`Client`]; the modules hold its types, one module per area of the API.


pub mod account; // the signed-in account: details, preferences, profile picture, password, PIN code, locked folder
pub mod activity; // comments and likes in shared albums
pub mod admin; // administration: users, job queues, external libraries
pub mod album; // albums: listing, editing, Assets and members
pub mod asset; // the Asset model, `GET /assets/{id}`
pub mod asset_change; // changing Assets: dates, place, rating, visibility, trash, edits, metadata, jobs
pub mod asset_info; // one Asset read whole, OCR, key-value metadata, counts
pub mod auth; // password and OIDC sign-in, sign-out
pub mod client; // the client: URL, credential, request plumbing
pub mod content; // ranged originals, HEAD, thumbnails
mod dates; // lenient date parsing (fake `Z`, no zone) and the format we send
pub mod download; // ZIP archives of many Assets: the plan, then each archive
pub mod duplicate; // groups of look-alike Assets, dismissing and resolving them
pub mod error; // `ApiError` and how HTTP outcomes map onto it
pub mod face; // the faces on an Asset, drawn by hand or detected
pub mod folder; // Assets by the server folder their original is in
pub mod map; // map markers and reverse geocoding
pub mod memory; // on-this-day memories and their Assets
pub mod notification; // the web app's notifications
pub mod partner; // partners who share their libraries
pub mod person; // people, hidden ones included: naming, merging, faces moved between them
pub mod place; // countries and cities from search suggestions
pub mod retry; // bounded retries honouring `Retry-After`
pub mod search; // metadata search in the v3.2 dialect, with cursor paging
pub mod search_more; // smart, random and counting search, explore, suggestions, place names
pub mod server; // version, features, public config, API key permissions
pub mod server_info; // ping, build, version history, statistics and storage
pub mod session; // sessions on devices, and API keys
pub mod shared_link; // public links to albums and Assets
pub mod stack; // stacks of Assets behind a primary
pub mod sync; // the session-only sync stream and acks
pub mod tag; // tags and tagging Assets
pub mod timeline; // timeline buckets: month counts and one month's ids
pub mod trash; // restoring from and emptying the trash
pub mod upload; // SHA-1, the duplicate check, streamed multipart uploads
pub mod user; // users as other endpoints show them, the account, preferences
pub mod version; // supported server versions
mod wire; // lenient decoding, enums that keep unknown values, bulk results, the endpoint table

#[cfg(test)]
mod fixtures; // JSON captured from the lab server, for unit tests
#[cfg(test)]
mod test_server; // a canned-response HTTP server for unit tests

pub use account::{
    AccountChanges, AlbumPreferences, AuthStatus, DownloadPreferences, EmailPreferences, FeaturePreference, MemoryPreferences,
    PeoplePreferences, Preferences, ProfileImage, Proof,
};
pub use activity::{Activity, ActivityFilter, ActivityKind, ActivityStatistics};
pub use admin::{
    ImportPathCheck, Library, LibrarySettings, LibraryStatistics, ManualJob, NewUser, Queue, QueueCommand, QueueName,
    QueueStatistics,
    QueuedJob, UserChanges,
};
pub use album::{Album, AlbumChanges, AlbumFilter, AlbumMember, AlbumRole, AlbumStatistics, AssetOrder, BulkAlbumOutcome, NewAlbum, NewMember};
pub use asset::{Asset, AssetKind, Visibility};
pub use asset_change::{AssetChanges, AssetEdit, AssetJob, CopyOptions, MirrorAxis};
pub use asset_info::{AssetDetails, AssetStatistics, Exif, MetadataEntry, OcrText, StackSummary, StatisticsFilter};
pub use auth::LoginResponse;
pub use client::{Client, Credential, Fetched, Media};
pub use content::{RangeResponse, ThumbnailSize};
pub use download::{DownloadPlan, DownloadSource, PlannedArchive};
pub use duplicate::{DuplicateGroup, DuplicateResolution};
pub use error::ApiError;
pub use face::{Face, FaceSource, NewFace};
pub use map::{MapMarker, MapQuery, Place};
pub use memory::{Memory, MemoryChanges, MemoryData, MemoryFilter, MemoryKind, MemoryOrder, NewMemory};
pub use notification::{Notification, NotificationFilter, NotificationKind, NotificationLevel};
pub use partner::{Partner, PartnerDirection};
pub use person::{NewPerson, Person, PersonChanges, PersonOnAsset};
pub use retry::RetryPolicy;
pub use search::{AssetPage, AssetQuery};
pub use search_more::{ExploreGroup, ExploreItem, PlaceMatch, SmartQuery, SuggestionKind, SuggestionQuery};
pub use server::{ServerConfig, ServerFeatures};
pub use server_info::{ServerAbout, ServerStatistics, ServerStorage, UserUsage, VersionHistoryEntry};
pub use session::{ApiKey, NewApiKey, NewSession, Session};
pub use shared_link::{AssetIdResult, LinkAddress, NewSharedLink, SharedLink, SharedLinkChanges, SharedLinkKind};
pub use stack::Stack;
pub use sync::{SyncAsset, SyncAssetExif, SyncLine, SyncRecord, SyncStream};
pub use tag::{NewTag, Tag, TagChanges};
pub use timeline::{BucketAsset, TimeBucket, TimelineScope, YearMonth};
pub use upload::{Sha1Checksum, UploadCheck, UploadOutcome};
pub use user::{AvatarColor, User, UserDetails, UserStatus};
pub use version::ServerVersion;
pub use wire::{BulkIdError, BulkIdResult, Endpoint, Secret};

/// Every endpoint this crate calls, with the permission Immich requires for it. The spec-conformance contract test
/// checks the list against the server's own `/api/spec.json`.
pub fn endpoints() -> Vec<Endpoint> {
    [
        account::ENDPOINTS,
        activity::ENDPOINTS,
        admin::ENDPOINTS,
        album::ENDPOINTS,
        asset::ENDPOINTS,
        asset_change::ENDPOINTS,
        asset_info::ENDPOINTS,
        auth::ENDPOINTS,
        content::ENDPOINTS,
        download::ENDPOINTS,
        duplicate::ENDPOINTS,
        face::ENDPOINTS,
        folder::ENDPOINTS,
        map::ENDPOINTS,
        memory::ENDPOINTS,
        notification::ENDPOINTS,
        partner::ENDPOINTS,
        person::ENDPOINTS,
        place::ENDPOINTS,
        search::ENDPOINTS,
        search_more::ENDPOINTS,
        server::ENDPOINTS,
        server_info::ENDPOINTS,
        session::ENDPOINTS,
        shared_link::ENDPOINTS,
        stack::ENDPOINTS,
        sync::ENDPOINTS,
        tag::ENDPOINTS,
        timeline::ENDPOINTS,
        trash::ENDPOINTS,
        upload::ENDPOINTS,
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_endpoint_table_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for endpoint in super::endpoints() {
            assert!(seen.insert((endpoint.method, endpoint.path)), "{} {} twice", endpoint.method, endpoint.path);
            assert!(endpoint.path.starts_with('/'), "{}", endpoint.path);
        }
    }
}

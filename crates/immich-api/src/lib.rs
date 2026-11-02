//! The Immich REST client and models that Immich Mount uses.
//!
//! Portable: no FUSE, D-Bus, GTK or OS-specific code (ARCHITECTURE.md, "Crates"). The models and request plumbing
//! are copied from immich-rs (MIT) and adapted; see CREDITS.md and THIRD-PARTY-NOTICES.md.
//!
//! Design rules that hold across the crate:
//! - Only the endpoints the mount uses, against Immich v3.2 and later (`version::MINIMUM_SUPPORTED`).
//! - Decoding is lenient: a field missing on an older server, or a new one, never breaks decoding. Wire structs
//!   are all-optional; the conversion to our types insists only on what the mount cannot do without.
//! - Every failure is one [`ApiError`], whose variants are the kinds the mount branches on.
//! - Dates are `chrono`: `DateTime<Utc>` for instants, `NaiveDateTime` for wall-clock times.
//! - Concurrency limits and deadlines are the caller's; the client only retries briefly (`retry`).
//!
//! Every endpoint is a method on [`Client`]; the modules hold its types.

pub mod album; // albums, owned and shared
pub mod asset; // the Asset model, `GET /assets/{id}`
pub mod auth; // password and OIDC sign-in, sign-out
pub mod client; // the client: URL, credential, request plumbing
pub mod content; // ranged originals, HEAD, thumbnails
mod dates; // lenient date parsing (fake `Z`, no zone) and the format we send
pub mod error; // `ApiError` and how HTTP outcomes map onto it
pub mod person; // people, hidden ones included
pub mod place; // countries and cities from search suggestions
pub mod retry; // bounded retries honouring `Retry-After`
pub mod search; // metadata search in the v3.2 dialect, with cursor paging
pub mod server; // version, features, public config, API key permissions
pub mod sync; // the session-only sync stream and acks
pub mod timeline; // timeline buckets: month counts and one month's ids
pub mod upload; // SHA-1, the duplicate check, streamed multipart uploads
pub mod version; // supported server versions

#[cfg(test)]
mod fixtures; // JSON captured from the lab server, for unit tests
#[cfg(test)]
mod test_server; // a canned-response HTTP server for unit tests

pub use album::Album;
pub use asset::{Asset, AssetKind, Visibility};
pub use auth::LoginResponse;
pub use client::{Client, Credential, Fetched};
pub use content::{RangeResponse, ThumbnailSize};
pub use error::ApiError;
pub use person::Person;
pub use retry::RetryPolicy;
pub use search::{AssetPage, AssetQuery};
pub use server::{ServerConfig, ServerFeatures};
pub use sync::{SyncAsset, SyncAssetExif, SyncLine, SyncRecord, SyncStream};
pub use timeline::{BucketAsset, TimeBucket, TimelineScope, YearMonth};
pub use upload::{Sha1Checksum, UploadCheck, UploadOutcome};
pub use version::ServerVersion;

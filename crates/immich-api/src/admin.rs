//! Administration: the server's accounts, its job queues, its external libraries, and what it stores. Everything
//! here needs an admin account (or a key with the admin permissions named on each method).
//!
//! Quirks, seen on v3.3.6:
//! - deleting a user marks it deleted (`deletedAt`, status `deleted`) and keeps its library for the server's grace
//!   period, during which [`Client::restore_user`] brings it back; `force` removes it at once (in a background job)
//! - `PUT /admin/users/{id}` and `PUT /libraries/{id}` are deprecated with no replacement yet
//! - the queues' names are camelCase (`thumbnailGeneration`), the manual jobs' kebab-case (`person-cleanup`)

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::account::Preferences;
use crate::asset::Visibility;
use crate::asset_info::AssetStatistics;
use crate::client::Client;
use crate::dates::serde_dates;
use crate::error::ApiError;
use crate::session::Session;
use crate::user::{AvatarColor, UserDetails};
use crate::wire::{endpoints, wire_enum, Endpoint, Secret};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/admin/users" "adminUser.read",
    GET "/admin/users/{id}" "adminUser.read",
    POST "/admin/users" "adminUser.create",
    PUT "/admin/users/{id}" "adminUser.update",
    DELETE "/admin/users/{id}" "adminUser.delete",
    POST "/admin/users/{id}/restore" "adminUser.delete",
    GET "/admin/users/{id}/statistics" "adminUser.read",
    GET "/admin/users/{id}/preferences" "adminUser.read",
    GET "/admin/users/{id}/sessions" "adminSession.read",
    GET "/queues" "queue.read",
    GET "/queues/{name}" "queue.read",
    PUT "/queues/{name}" "queue.update",
    GET "/queues/{name}/jobs" "queueJob.read",
    DELETE "/queues/{name}/jobs" "queueJob.delete",
    POST "/jobs" "job.create",
    PUT "/jobs/{name}" "job.create",
    GET "/libraries" "library.read",
    GET "/libraries/{id}" "library.read",
    POST "/libraries" "library.create",
    PUT "/libraries/{id}" "library.update",
    DELETE "/libraries/{id}" "library.delete",
    POST "/libraries/{id}/scan" "library.update",
    POST "/libraries/{id}/validate" -,
    GET "/libraries/{id}/statistics" "library.statistics",
];

/// A new account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewUser {
    pub email: String,
    pub name: String,
    pub password: Secret,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_admin: bool,
    /// Ask for a new password at the first sign-in.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub should_change_password: bool,
    /// Send the new user an email, when the server can.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub notify: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_size_in_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<AvatarColor>,
}

/// Changes to an account; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<Secret>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_admin: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub should_change_password: Option<bool>,
    /// `Some(None)` removes the quota.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_size_in_bytes: Option<Option<u64>>,
    /// `Some(None)` removes the storage label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_label: Option<Option<String>>,
    /// `Some(None)` removes the user's PIN code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_code: Option<Option<Secret>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<AvatarColor>,
}

wire_enum! {
    /// A queue of the server's background jobs.
    pub enum QueueName {
        ThumbnailGeneration = "thumbnailGeneration",
        MetadataExtraction = "metadataExtraction",
        VideoConversion = "videoConversion",
        FaceDetection = "faceDetection",
        FacialRecognition = "facialRecognition",
        SmartSearch = "smartSearch",
        DuplicateDetection = "duplicateDetection",
        BackgroundTask = "backgroundTask",
        StorageTemplateMigration = "storageTemplateMigration",
        Migration = "migration",
        Search = "search",
        Sidecar = "sidecar",
        Library = "library",
        Notifications = "notifications",
        BackupDatabase = "backupDatabase",
        Ocr = "ocr",
    }
}

/// A queue and its counts (`QueueResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Queue {
    pub name: QueueName,
    pub is_paused: bool,
    pub statistics: QueueStatistics,
}

impl Queue {
    /// Nothing running, waiting or delayed.
    pub fn is_idle(&self) -> bool {
        self.statistics.active + self.statistics.waiting + self.statistics.delayed == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct QueueStatistics {
    pub active: u64,
    pub waiting: u64,
    pub delayed: u64,
    pub paused: u64,
    pub completed: u64,
    pub failed: u64,
}

wire_enum! {
    /// What [`Client::command_queue`] tells a queue to do.
    pub enum QueueCommand {
        /// Queue the work for the Assets that lack it, or with `force` for every Asset.
        Start = "start",
        Pause = "pause",
        Resume = "resume",
        /// Drop the waiting jobs.
        Empty = "empty",
        ClearFailed = "clear-failed",
    }
}

/// A job in a queue (`QueueJobResponseDto`).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct QueuedJob {
    pub id: Option<String>,
    /// Such as `AssetGenerateThumbnails`.
    pub name: String,
    /// What the job works on, such as `{"id": <asset id>}`.
    pub data: serde_json::Value,
    /// When it was queued, in milliseconds since the Unix epoch.
    pub timestamp: i64,
}

wire_enum! {
    /// A maintenance job an admin can start by hand.
    pub enum ManualJob {
        PersonCleanup = "person-cleanup",
        TagCleanup = "tag-cleanup",
        UserCleanup = "user-cleanup",
        MemoryCleanup = "memory-cleanup",
        MemoryCreate = "memory-create",
        BackupDatabase = "backup-database",
    }
}

/// An external library: folders on the server that Immich indexes in place (`LibraryResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Library {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub import_paths: Vec<String>,
    /// Glob patterns of files to leave out, such as `**/@eaDir/**`.
    pub exclusion_patterns: Vec<String>,
    pub asset_count: u64,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub updated_at: DateTime<Utc>,
    /// The last scan; `None` before the first.
    #[serde(deserialize_with = "serde_dates::optional_utc")]
    pub refreshed_at: Option<DateTime<Utc>>,
}

/// Paths and patterns of an external library, to create, change or validate one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusion_patterns: Option<Vec<String>>,
}

/// Whether an import path can be used (`ValidateLibraryImportPathResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportPathCheck {
    pub import_path: String,
    pub is_valid: bool,
    /// Why not, when it cannot.
    pub message: Option<String>,
}

/// What an external library holds (`LibraryStatsResponseDto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct LibraryStatistics {
    pub photos: u64,
    pub videos: u64,
    pub total: u64,
    /// Bytes.
    pub usage: u64,
}

impl Client {
    /// Every account, optionally with the deleted ones (`GET /admin/users`). Needs `adminUser.read`.
    pub async fn admin_users(&self, with_deleted: bool) -> Result<Vec<UserDetails>, ApiError> {
        let query: &[(&str, &str)] = if with_deleted { &[("withDeleted", "true")] } else { &[] };
        self.call(Method::GET, &["admin", "users"], query).await
    }

    /// One account (`GET /admin/users/{id}`). Needs `adminUser.read`.
    pub async fn admin_user(&self, id: &str) -> Result<UserDetails, ApiError> {
        self.call(Method::GET, &["admin", "users", id], &[]).await
    }

    /// Create an account (`POST /admin/users`). Needs `adminUser.create`.
    pub async fn create_user(&self, user: &NewUser) -> Result<UserDetails, ApiError> {
        self.call_with(Method::POST, &["admin", "users"], &[], user).await
    }

    /// Change an account (`PUT /admin/users/{id}`). Needs `adminUser.update`.
    pub async fn update_user(&self, id: &str, changes: &UserChanges) -> Result<UserDetails, ApiError> {
        self.call_with(Method::PUT, &["admin", "users", id], &[], changes).await
    }

    /// Delete an account: kept for the grace period, or with `force` removed at once (`DELETE /admin/users/{id}`).
    /// Needs `adminUser.delete`.
    pub async fn delete_user(&self, id: &str, force: bool) -> Result<UserDetails, ApiError> {
        self.call_with(Method::DELETE, &["admin", "users", id], &[], &serde_json::json!({ "force": force }))
            .await
    }

    /// Bring back a deleted account within its grace period (`POST /admin/users/{id}/restore`). Needs
    /// `adminUser.delete`.
    pub async fn restore_user(&self, id: &str) -> Result<UserDetails, ApiError> {
        self.call(Method::POST, &["admin", "users", id, "restore"], &[]).await
    }

    /// How many images and videos an account has (`GET /admin/users/{id}/statistics`). Needs `adminUser.read`.
    pub async fn user_statistics(&self, id: &str, visibility: Option<&Visibility>) -> Result<AssetStatistics, ApiError> {
        let query: Vec<(&str, &str)> = visibility.map(|visibility| ("visibility", visibility.as_str())).into_iter().collect();
        self.call(Method::GET, &["admin", "users", id, "statistics"], &query).await
    }

    /// An account's preferences (`GET /admin/users/{id}/preferences`). Needs `adminUser.read`.
    pub async fn user_preferences(&self, id: &str) -> Result<Preferences, ApiError> {
        let raw: serde_json::Value = self.call(Method::GET, &["admin", "users", id, "preferences"], &[]).await?;
        crate::account::preferences_from(raw)
    }

    /// An account's sessions (`GET /admin/users/{id}/sessions`). Needs `adminSession.read`.
    pub async fn user_sessions(&self, id: &str) -> Result<Vec<Session>, ApiError> {
        self.call(Method::GET, &["admin", "users", id, "sessions"], &[]).await
    }

    /// Every job queue with its counts (`GET /queues`). Needs `queue.read`.
    pub async fn queues(&self) -> Result<Vec<Queue>, ApiError> {
        self.call(Method::GET, &["queues"], &[]).await
    }

    /// One queue (`GET /queues/{name}`). Needs `queue.read`.
    pub async fn queue(&self, name: &QueueName) -> Result<Queue, ApiError> {
        self.call(Method::GET, &["queues", name.as_str()], &[]).await
    }

    /// Pause or resume a queue (`PUT /queues/{name}`). Needs `queue.update`.
    pub async fn set_queue_paused(&self, name: &QueueName, paused: bool) -> Result<Queue, ApiError> {
        self.call_with(Method::PUT, &["queues", name.as_str()], &[], &serde_json::json!({ "isPaused": paused }))
            .await
    }

    /// The jobs in a queue (`GET /queues/{name}/jobs`). Needs `queueJob.read`.
    pub async fn queued_jobs(&self, name: &QueueName) -> Result<Vec<QueuedJob>, ApiError> {
        self.call(Method::GET, &["queues", name.as_str(), "jobs"], &[]).await
    }

    /// Empty a queue of its waiting jobs, and with `failed` of its failed ones too (`DELETE /queues/{name}/jobs`).
    /// Needs `queueJob.delete`.
    pub async fn clear_queue(&self, name: &QueueName, failed: bool) -> Result<(), ApiError> {
        self.call_unit_with(Method::DELETE, &["queues", name.as_str(), "jobs"], &[], &serde_json::json!({ "failed": failed }))
            .await
    }

    /// Tell a queue to start its work for every Asset that lacks it (with `force`, for every Asset), or to pause,
    /// resume, empty or clear its failed jobs (`PUT /jobs/{name}`), as the web app's job page does. Deprecated in
    /// v3.3.6 with no replacement for starting yet; answers the queue's counts. Needs `job.create`.
    pub async fn command_queue(&self, name: &QueueName, command: &QueueCommand, force: bool) -> Result<QueueStatistics, ApiError> {
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct Answer {
            job_counts: QueueStatistics,
        }
        let body = serde_json::json!({ "command": command, "force": force });
        // Starting twice would queue the work twice: sent once.
        let answer: Answer = self.call_with_once(Method::PUT, &["jobs", name.as_str()], &[], &body).await?;
        Ok(answer.job_counts)
    }

    /// Start a maintenance job (`POST /jobs`). Needs `job.create`.
    pub async fn start_job(&self, job: &ManualJob) -> Result<(), ApiError> {
        self.call_unit_with(Method::POST, &["jobs"], &[], &serde_json::json!({ "name": job })).await
    }

    /// Every external library (`GET /libraries`). Needs `library.read`.
    pub async fn libraries(&self) -> Result<Vec<Library>, ApiError> {
        self.call(Method::GET, &["libraries"], &[]).await
    }

    /// One external library (`GET /libraries/{id}`). Needs `library.read`.
    pub async fn library(&self, id: &str) -> Result<Library, ApiError> {
        self.call(Method::GET, &["libraries", id], &[]).await
    }

    /// Create an external library for a user (`POST /libraries`). Needs `library.create`.
    pub async fn create_library(&self, owner_id: &str, settings: &LibrarySettings) -> Result<Library, ApiError> {
        let mut body = serde_json::to_value(settings)
            .map_err(|error| ApiError::Protocol(format!("internal: cannot encode library settings: {error}")))?;
        body["ownerId"] = owner_id.into();
        self.call_with(Method::POST, &["libraries"], &[], &body).await
    }

    /// Change an external library's name, paths or patterns (`PUT /libraries/{id}`). Needs `library.update`.
    pub async fn update_library(&self, id: &str, settings: &LibrarySettings) -> Result<Library, ApiError> {
        self.call_with(Method::PUT, &["libraries", id], &[], settings).await
    }

    /// Delete an external library and its Assets from Immich, not from disk (`DELETE /libraries/{id}`). Needs
    /// `library.delete`.
    pub async fn delete_library(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["libraries", id], &[]).await
    }

    /// Scan an external library for new, changed and missing files (`POST /libraries/{id}/scan`). Needs
    /// `library.update`.
    pub async fn scan_library(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::POST, &["libraries", id, "scan"], &[]).await
    }

    /// Check import paths before saving them (`POST /libraries/{id}/validate`). Admin only.
    pub async fn validate_library(&self, id: &str, settings: &LibrarySettings) -> Result<Vec<ImportPathCheck>, ApiError> {
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct Answer {
            import_paths: Vec<ImportPathCheck>,
        }
        let answer: Answer = self.call_with(Method::POST, &["libraries", id, "validate"], &[], settings).await?;
        Ok(answer.import_paths)
    }

    /// What an external library holds (`GET /libraries/{id}/statistics`). Needs `library.statistics`.
    pub async fn library_statistics(&self, id: &str) -> Result<LibraryStatistics, ApiError> {
        self.call(Method::GET, &["libraries", id, "statistics"], &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_user_sends_only_what_is_set() {
        let user = NewUser {
            email: "a@example.com".to_owned(),
            name: "A".to_owned(),
            password: "secret".into(),
            ..NewUser::default()
        };
        assert_eq!(
            serde_json::to_value(&user).unwrap(),
            serde_json::json!({"email": "a@example.com", "name": "A", "password": "secret"})
        );
    }

    #[test]
    fn a_queue_is_idle_when_nothing_waits() {
        let queue: Queue = crate::client::decode_json(
            br#"{"name":"thumbnailGeneration","isPaused":false,"statistics":{"active":0,"waiting":0,"delayed":0,"completed":9,"failed":1,"paused":0}}"#,
            "queues",
        )
        .unwrap();
        assert!(queue.is_idle());
        assert_eq!(queue.name, QueueName::ThumbnailGeneration);
    }
}

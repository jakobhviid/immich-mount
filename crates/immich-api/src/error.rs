//! The one error type every call returns, and how HTTP outcomes map onto it.
//!
//! The variants are the kinds the mount must branch on (SPEC.md, "Errors seen by programs using the mount"):
//! offline, signed out, missing permission, gone, a stale size, busy, and everything else. The mapping encodes
//! what the lab server answered on Immich v3.3.5:
//! - an unknown or deleted Asset answers 400 `Not found or no asset.read access`, not 404, on most endpoints, and
//!   other kinds of thing answer 400 `<Kind> not found` (seen on v3.3.6)
//! - `/assets/{id}/original` answers 404 for an unknown Asset, and also 404 (not 416) for a range past the end;
//!   only the second carries `Content-Range: bytes */<size>`, which is how the two are told apart
//! - a missing scope answers 403 `Missing required permission: <permission>`

use std::time::Duration;

use reqwest::StatusCode;

/// What went wrong talking to Immich.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// The server could not be reached: connection refused, DNS, TLS, a timeout, or a dropped connection.
    #[error("cannot reach the Immich server: {0}")]
    Offline(String),

    /// The server refused the credential (401): signed out, revoked, or a wrong password.
    #[error("the Immich server refused the credential")]
    Unauthorized,

    /// The credential is valid but lacks a permission (403).
    #[error("{}", forbidden_message(missing_permission.as_deref()))]
    Forbidden {
        /// The permission named by `Missing required permission: <permission>`, when the server said which.
        missing_permission: Option<String>,
    },

    /// The thing asked for does not exist or is not visible to this credential (404, or 400 `Not found`).
    /// Not proof of deletion: a narrower credential sees the same answer.
    #[error("not found on the Immich server")]
    NotFound,

    /// A byte range starts past the end of the original, so the size we hold is stale.
    #[error("the requested range is past the end of the file")]
    RangeNotSatisfiable,

    /// The server is busy or failing for now (408, 429, 5xx), after the client's own bounded retries.
    #[error("the Immich server is busy; try again later")]
    Transient {
        /// How long the server asked us to wait (`Retry-After`), when it said.
        retry_after: Option<Duration>,
    },

    /// The server answered something this client cannot use: bad JSON, an impossible value, an unexpected
    /// status, or a server URL that cannot be used at all.
    #[error("unexpected answer from the Immich server: {0}")]
    Protocol(String),

    /// Reading or writing a local file failed: a file being uploaded, or an original being downloaded.
    #[error("cannot use the local file: {0}")]
    LocalFile(String),
}

fn forbidden_message(missing_permission: Option<&str>) -> String {
    match missing_permission {
        Some(permission) => format!("the credential lacks the Immich permission `{permission}`"),
        None => "the Immich server denied access to this credential".to_owned(),
    }
}

/// Whether the request asked for a byte range, which changes what a 404 means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestShape {
    Plain,
    Ranged,
}

/// The parts of an error response the mapping looks at.
#[derive(Debug, Clone)]
pub(crate) struct ErrorResponse<'a> {
    pub status: StatusCode,
    pub retry_after: Option<Duration>,
    /// Whether the response carried a `Content-Range` header (`bytes */<size>` on an unsatisfiable range).
    pub has_content_range: bool,
    pub body: &'a str,
}

/// Map a non-success HTTP response to an [`ApiError`].
pub(crate) fn classify(response: &ErrorResponse<'_>, shape: RequestShape) -> ApiError {
    let message = server_message(response.body);
    match response.status.as_u16() {
        401 => ApiError::Unauthorized,
        403 => ApiError::Forbidden {
            missing_permission: missing_permission(&message),
        },
        404 if shape == RequestShape::Ranged && response.has_content_range => ApiError::RangeNotSatisfiable,
        416 => ApiError::RangeNotSatisfiable,
        404 => ApiError::NotFound,
        400 if is_not_found(&message) => ApiError::NotFound,
        408 | 429 | 500..=599 => ApiError::Transient {
            retry_after: response.retry_after,
        },
        status => {
            let detail = if message.is_empty() {
                format!("HTTP {status}")
            } else {
                format!("HTTP {status}: {message}")
            };
            ApiError::Protocol(detail)
        }
    }
}

/// The human-readable `message` of an Immich error body. Immich sends a string, or a list of validation messages.
pub(crate) fn server_message(body: &str) -> String {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Message {
        One(String),
        Many(Vec<String>),
    }
    #[derive(serde::Deserialize)]
    struct ErrorBody {
        message: Option<Message>,
    }

    match serde_json::from_str::<ErrorBody>(body) {
        Ok(ErrorBody {
            message: Some(Message::One(text)),
        }) => text,
        Ok(ErrorBody {
            message: Some(Message::Many(lines)),
        }) => lines.join("; "),
        _ => body.trim().chars().take(200).collect(),
    }
}

/// A 400 that means "not found": `Not found or no asset.read access`, or `Notification not found`.
fn is_not_found(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.starts_with("not found") || lower.ends_with(" not found")
}

fn missing_permission(message: &str) -> Option<String> {
    let permission = message.strip_prefix("Missing required permission:")?.trim();
    (!permission.is_empty()).then(|| permission.to_owned())
}

/// Map a transport failure (no HTTP response at all) to an [`ApiError`].
pub(crate) fn from_transport(error: &reqwest::Error) -> ApiError {
    if error.is_builder() || error.is_decode() {
        return ApiError::Protocol(describe(error));
    }
    // Connect, timeout, request and body errors all mean the conversation with the server broke off.
    ApiError::Offline(describe(error))
}

/// An error with its whole source chain, because reqwest's own message alone ("error sending request") hides
/// the useful part (connection refused, certificate unknown, ...).
pub(crate) fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> ErrorResponse<'_> {
        ErrorResponse {
            status: StatusCode::from_u16(status).unwrap(),
            retry_after: None,
            has_content_range: false,
            body,
        }
    }

    #[test]
    fn unauthorized_is_401() {
        let error = classify(&response(401, r#"{"message":"Invalid API key"}"#), RequestShape::Plain);
        assert_eq!(error, ApiError::Unauthorized);
    }

    #[test]
    fn forbidden_names_the_missing_permission() {
        let body = r#"{"message":"Missing required permission: server.about","error":"Forbidden","statusCode":403}"#;
        let error = classify(&response(403, body), RequestShape::Plain);
        assert_eq!(
            error,
            ApiError::Forbidden {
                missing_permission: Some("server.about".to_owned())
            }
        );
    }

    #[test]
    fn forbidden_without_a_permission_name() {
        let body = r#"{"message":"Sync endpoints cannot be used with API keys"}"#;
        let error = classify(&response(403, body), RequestShape::Plain);
        assert_eq!(error, ApiError::Forbidden { missing_permission: None });
    }

    #[test]
    fn bad_request_not_found_means_not_found() {
        let body = r#"{"message":"Not found or no asset.read access","error":"Bad Request","statusCode":400}"#;
        assert_eq!(classify(&response(400, body), RequestShape::Plain), ApiError::NotFound);
    }

    #[test]
    fn other_bad_requests_are_protocol_errors() {
        let body = r#"{"message":["size must not be greater than 1000"],"error":"Bad Request"}"#;
        let error = classify(&response(400, body), RequestShape::Plain);
        assert_eq!(
            error,
            ApiError::Protocol("HTTP 400: size must not be greater than 1000".to_owned())
        );
    }

    #[test]
    fn range_past_the_end_is_404_with_content_range() {
        let past_the_end = ErrorResponse {
            has_content_range: true,
            ..response(404, r#"{"message":"Not Found"}"#)
        };
        assert_eq!(
            classify(&past_the_end, RequestShape::Ranged),
            ApiError::RangeNotSatisfiable
        );
    }

    #[test]
    fn unknown_asset_on_a_range_request_is_not_found() {
        // Immich answers 404 for an unknown Asset on /original too, but without Content-Range.
        let unknown = response(404, r#"{"message":"Not Found"}"#);
        assert_eq!(classify(&unknown, RequestShape::Ranged), ApiError::NotFound);
    }

    #[test]
    fn a_real_416_is_range_not_satisfiable() {
        assert_eq!(
            classify(&response(416, ""), RequestShape::Ranged),
            ApiError::RangeNotSatisfiable
        );
    }

    #[test]
    fn busy_statuses_are_transient_with_retry_after() {
        for status in [408, 429, 500, 502, 503, 504] {
            let busy = ErrorResponse {
                retry_after: Some(Duration::from_secs(7)),
                ..response(status, "")
            };
            assert_eq!(
                classify(&busy, RequestShape::Plain),
                ApiError::Transient {
                    retry_after: Some(Duration::from_secs(7))
                },
                "status {status}"
            );
        }
    }

    #[test]
    fn unexpected_status_keeps_the_message() {
        let error = classify(&response(409, r#"{"message":"conflict"}"#), RequestShape::Plain);
        assert_eq!(error, ApiError::Protocol("HTTP 409: conflict".to_owned()));
    }

    #[test]
    fn non_json_bodies_are_quoted_briefly() {
        assert_eq!(server_message("  <html>bad gateway</html> "), "<html>bad gateway</html>");
    }
}

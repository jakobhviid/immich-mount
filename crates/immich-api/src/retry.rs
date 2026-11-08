//! The retry policy: bounded attempts, exponential backoff with jitter, honouring `Retry-After`.
//!
//! Adapted from immich-rs (`core/retry.rs`). A call that may be sent twice (`Resend::Freely`) is retried on:
//! - 429, 502, 503 and 504 on every method
//! - 408 and 500 only on idempotent methods (GET, HEAD, PUT, DELETE), because a POST may have taken effect
//! - connection failures and timeouts on every method: the mount's POSTs are searches, sign-in, sync acks and
//!   uploads, and an upload repeated after a lost response becomes a duplicate, not a second Asset
//!
//! A call whose second sending would do something else (`Resend::OnlyUnsent`: creating something, rotating a
//! key, changing a password or PIN code, shifting dates by an amount) is retried only when its first attempt
//! cannot have taken effect: the connection failed before anything was sent, or the server answered 429.
//! Otherwise a lost answer would turn into a second album, or into a wrong-password error for a change that
//! succeeded. The client's generic calls use it for POST and PATCH, and the methods that need it for PUT or
//! DELETE ask for it.
//!
//! The client waits at most [`RetryPolicy::max_wait`] before one retry. A longer `Retry-After` is not slept on:
//! the call returns [`ApiError::Transient`](crate::ApiError::Transient) at once with the requested delay, because
//! callers answering a file read have deadlines of their own (ARCHITECTURE.md, "Never hang").

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{Method, StatusCode};

/// How often and how long the client retries a failing request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt. Zero turns retrying off.
    pub max_retries: u32,
    /// The backoff before the first retry; it doubles for each further retry.
    pub base_delay: Duration,
    /// The longest the client sleeps before one retry, whether from backoff or from `Retry-After`.
    pub max_wait: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_retries: 3,
            base_delay: Duration::from_millis(250),
            max_wait: Duration::from_secs(5),
        }
    }
}

impl RetryPolicy {
    /// The backoff before retry number `retry` (1-based): `base_delay * 2^(retry-1)`, with 20% jitter either way.
    pub(crate) fn backoff(&self, retry: u32) -> Duration {
        let exponent = retry.saturating_sub(1).min(10);
        let base = self.base_delay.saturating_mul(1 << exponent);
        base.mul_f64(jitter_factor()).min(self.max_wait)
    }
}

/// A factor in [0.8, 1.2), from the clock's nanoseconds; good enough to spread retries without a `rand` dependency.
fn jitter_factor() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    0.8 + f64::from(nanos % 1000) / 1000.0 * 0.4
}

/// Whether `method` may safely be sent twice.
pub(crate) fn is_idempotent(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::PUT | Method::DELETE | Method::OPTIONS | Method::TRACE
    )
}

/// Whether a response with `status` is worth retrying for a request with `method`.
pub(crate) fn is_retryable_status(status: StatusCode, method: &Method) -> bool {
    match status.as_u16() {
        429 | 502 | 503 | 504 => true,
        408 | 500 => is_idempotent(method),
        _ => false,
    }
}

/// Whether a transport failure (no response at all) is worth retrying.
pub(crate) fn is_retryable_transport(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout()
}

/// Whether a request may be sent again after an attempt that may have reached the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resend {
    /// Sending it twice does no harm: the policy for statuses and transport failures above.
    Freely,
    /// Sending it twice would do something else: only when the first attempt cannot have taken effect.
    OnlyUnsent,
}

impl Resend {
    /// What the generic calls use: freely for idempotent methods, only unsent for POST and PATCH.
    pub(crate) fn by_method(method: &Method) -> Resend {
        if is_idempotent(method) {
            Resend::Freely
        } else {
            Resend::OnlyUnsent
        }
    }

    pub(crate) fn retries_status(self, status: StatusCode, method: &Method) -> bool {
        match self {
            Resend::Freely => is_retryable_status(status, method),
            Resend::OnlyUnsent => status == StatusCode::TOO_MANY_REQUESTS,
        }
    }

    pub(crate) fn retries_transport(self, error: &reqwest::Error) -> bool {
        match self {
            Resend::Freely => is_retryable_transport(error),
            Resend::OnlyUnsent => error.is_connect(),
        }
    }
}

/// Parse `Retry-After`, given either as delta-seconds or as an HTTP date (RFC 9110).
pub(crate) fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let raw = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    parse_retry_after_value(raw, SystemTime::now())
}

fn parse_retry_after_value(raw: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    // IMF-fixdate ("Sun, 06 Nov 1994 08:49:37 GMT") is a subset of RFC 2822.
    let target = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
    let target = SystemTime::from(target.with_timezone(&chrono::Utc));
    Some(target.duration_since(now).unwrap_or(Duration::ZERO))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_within_jitter_bounds() {
        let policy = RetryPolicy::default();
        for _ in 0..20 {
            let first = policy.backoff(1);
            assert!(first >= Duration::from_millis(200) && first < Duration::from_millis(300));
            let second = policy.backoff(2);
            assert!(second >= Duration::from_millis(400) && second < Duration::from_millis(600));
        }
    }

    #[test]
    fn backoff_is_capped_by_max_wait() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.backoff(12), policy.max_wait);
    }

    #[test]
    fn server_errors_retry_only_where_safe() {
        for status in [429, 502, 503, 504] {
            let status = StatusCode::from_u16(status).unwrap();
            assert!(is_retryable_status(status, &Method::GET));
            assert!(is_retryable_status(status, &Method::POST));
        }
        for status in [408, 500] {
            let status = StatusCode::from_u16(status).unwrap();
            assert!(is_retryable_status(status, &Method::GET));
            assert!(is_retryable_status(status, &Method::HEAD));
            assert!(!is_retryable_status(status, &Method::POST));
        }
        assert!(!is_retryable_status(StatusCode::NOT_FOUND, &Method::GET));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED, &Method::GET));
    }

    #[test]
    fn retry_after_in_seconds() {
        assert_eq!(
            parse_retry_after_value("120", SystemTime::now()),
            Some(Duration::from_secs(120))
        );
    }

    #[test]
    fn retry_after_as_http_date() {
        let now = SystemTime::from(chrono::DateTime::parse_from_rfc3339("2026-11-01T12:00:00Z").unwrap());
        assert_eq!(
            parse_retry_after_value("Sun, 01 Nov 2026 12:00:30 GMT", now),
            Some(Duration::from_secs(30))
        );
        // A date in the past means "now".
        assert_eq!(
            parse_retry_after_value("Sun, 01 Nov 2026 11:00:00 GMT", now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn retry_after_garbage_is_ignored() {
        assert_eq!(parse_retry_after_value("soon", SystemTime::now()), None);
    }
}

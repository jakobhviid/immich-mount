//! Content: byte ranges of originals, their size and ETag, and server-made thumbnails.
//!
//! Range requests work on v3.3.5, byte-exact, HEAD included. Quirks handled here:
//! - only single ranges are sent: a multi-range request falls back to 200 with the whole file
//! - a 200 answer to a range request means something on the way ignored `Range`; it is refused (and its body
//!   left unread) rather than downloading a whole video for one block
//! - a range past the end answers **404** with `Content-Range: bytes */<size>`, not 416; that is
//!   [`ApiError::RangeNotSatisfiable`] and means our size is stale. A 404 **without** `Content-Range` is an
//!   unknown Asset ([`ApiError::NotFound`]): `/original` answers 404, not 400, for those
//! - the ETag is weak (`W/"<size hex>-<mtime hex>"`) and is returned verbatim; HTTP forbids weak ETags in
//!   `If-Range`, so callers compare it themselves
//!
//! Thumbnails: `thumbnail` is a WebP with a 250 px short side, `preview` a JPEG with a 1440 px short side.
//! `fullsize` redirects to the original for JPEG sources, so it is not offered.

use bytes::Bytes;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, RANGE};
use reqwest::{Method, StatusCode};

use crate::client::{check, header_text, Client, REQUEST_TIMEOUT};
use crate::error::{from_transport, ApiError, RequestShape};

/// Some bytes of an original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeResponse {
    /// The bytes from the requested start. Shorter than asked when the range runs past the end of the file.
    pub bytes: Bytes,
    /// The size of the whole original, from `Content-Range`.
    pub total_size: u64,
    /// The original's (weak) ETag, verbatim.
    pub etag: Option<String>,
}

/// Which server-made thumbnail to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbnailSize {
    /// WebP, 250 px on the short side.
    Thumbnail,
    /// JPEG, 1440 px on the short side, never upscaled.
    Preview,
}

impl ThumbnailSize {
    fn as_str(self) -> &'static str {
        match self {
            ThumbnailSize::Thumbnail => "thumbnail",
            ThumbnailSize::Preview => "preview",
        }
    }
}

/// `bytes <first>-<last>/<total>` as numbers.
fn parse_content_range(text: &str) -> Option<(u64, u64, u64)> {
    let range = text.trim().strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    let (first, last) = span.split_once('-')?;
    let (first, last, total) = (first.trim().parse().ok()?, last.trim().parse().ok()?, total.trim().parse().ok()?);
    (first <= last && last < total).then_some((first, last, total))
}

/// Check a 206 answer against what was asked.
fn validate_partial(
    start: u64,
    end_inclusive: u64,
    content_range: Option<&str>,
    body_length: usize,
) -> Result<u64, ApiError> {
    let text = content_range.ok_or_else(|| ApiError::Protocol("a 206 answer without Content-Range".to_owned()))?;
    let (first, last, total) = parse_content_range(text)
        .ok_or_else(|| ApiError::Protocol(format!("an unreadable Content-Range `{text}`")))?;
    if first != start || last > end_inclusive {
        return Err(ApiError::Protocol(format!(
            "asked for bytes {start}-{end_inclusive}, got `{text}`"
        )));
    }
    if body_length as u64 != last - first + 1 {
        return Err(ApiError::Protocol(format!(
            "`{text}` announced, but {body_length} bytes arrived"
        )));
    }
    Ok(total)
}

impl Client {
    /// Bytes `start..=end_inclusive` of an Asset's original (`GET /assets/{id}/original` with `Range`).
    ///
    /// Errors: [`ApiError::RangeNotSatisfiable`] when `start` is past the end (refetch the size), and
    /// [`ApiError::NotFound`] when the Asset is gone.
    pub async fn original_range(&self, id: &str, start: u64, end_inclusive: u64) -> Result<RangeResponse, ApiError> {
        if start > end_inclusive {
            return Err(ApiError::Protocol(format!(
                "an empty range was asked for: {start}-{end_inclusive}"
            )));
        }
        let builder = self
            .request(Method::GET, &["assets", id, "original"])
            .header(RANGE, format!("bytes={start}-{end_inclusive}"))
            .timeout(REQUEST_TIMEOUT);
        let response = check(self.execute(builder).await?, RequestShape::Ranged).await?;
        if response.status() != StatusCode::PARTIAL_CONTENT {
            // Dropping the response closes the connection without downloading the whole original.
            return Err(ApiError::Protocol(format!(
                "the server ignored the range request (HTTP {})",
                response.status().as_u16()
            )));
        }
        let content_range = header_text(&response, CONTENT_RANGE);
        let etag = header_text(&response, ETAG);
        let bytes = response.bytes().await.map_err(|error| from_transport(&error))?;
        let total_size = validate_partial(start, end_inclusive, content_range.as_deref(), bytes.len())?;
        Ok(RangeResponse {
            bytes,
            total_size,
            etag,
        })
    }

    /// The size and (weak) ETag of an Asset's original, without downloading it (`HEAD /assets/{id}/original`).
    pub async fn original_head(&self, id: &str) -> Result<(u64, Option<String>), ApiError> {
        let builder = self
            .request(Method::HEAD, &["assets", id, "original"])
            .timeout(REQUEST_TIMEOUT);
        let response = check(self.execute(builder).await?, RequestShape::Plain).await?;
        let total_size = header_text(&response, CONTENT_LENGTH)
            .and_then(|text| text.trim().parse::<u64>().ok())
            .ok_or_else(|| ApiError::Protocol("HEAD of an original without a Content-Length".to_owned()))?;
        Ok((total_size, header_text(&response, ETAG)))
    }

    /// A server-made thumbnail and its content type (`GET /assets/{id}/thumbnail?size=...`).
    pub async fn thumbnail(&self, id: &str, size: ThumbnailSize) -> Result<(Bytes, String), ApiError> {
        let builder = self
            .request(Method::GET, &["assets", id, "thumbnail"])
            .query(&[("size", size.as_str())])
            .timeout(REQUEST_TIMEOUT);
        let response = check(self.execute(builder).await?, RequestShape::Plain).await?;
        let content_type = header_text(&response, CONTENT_TYPE).unwrap_or_else(|| "application/octet-stream".to_owned());
        let bytes = response.bytes().await.map_err(|error| from_transport(&error))?;
        Ok((bytes, content_type))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn content_range_parses() {
        assert_eq!(parse_content_range("bytes 0-4095/1376749"), Some((0, 4095, 1_376_749)));
        assert_eq!(parse_content_range("bytes */1376749"), None);
        assert_eq!(parse_content_range("bytes 10-5/100"), None);
        assert_eq!(parse_content_range("bytes 0-100/100"), None);
    }

    #[test]
    fn partial_answers_are_checked() {
        assert_eq!(validate_partial(0, 4095, Some("bytes 0-4095/1376749"), 4096), Ok(1_376_749));
        // A range straddling the end comes back short, which is fine.
        assert_eq!(validate_partial(1_000_000, 1_999_999, Some("bytes 1000000-1012519/1012520"), 12_520), Ok(1_012_520));
        assert!(validate_partial(0, 4095, Some("bytes 1-4095/1376749"), 4095).is_err());
        assert!(validate_partial(0, 4095, Some("bytes 0-4095/1376749"), 100).is_err());
        assert!(validate_partial(0, 4095, None, 4096).is_err());
    }

    #[tokio::test]
    async fn range_request_returns_bytes_size_and_etag() {
        let server = FakeServer::start(vec![CannedResponse::bytes(206, "image/jpeg", b"\xff\xd8\xff\xe0")
            .with_header("content-range", "bytes 0-3/1012520")
            .with_header("etag", "W/\"f7328-196689abeb0\"")])
        .await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let range = client.original_range("asset-1", 0, 3).await.unwrap();
        assert_eq!(&range.bytes[..], b"\xff\xd8\xff\xe0");
        assert_eq!(range.total_size, 1_012_520);
        assert_eq!(range.etag.as_deref(), Some("W/\"f7328-196689abeb0\""));
        let request = &server.requests()[0];
        assert_eq!(request.target, "/api/assets/asset-1/original");
        assert_eq!(request.header("range").as_deref(), Some("bytes=0-3"));
    }

    #[tokio::test]
    async fn range_past_the_end_is_range_not_satisfiable() {
        let server = FakeServer::start(vec![
            CannedResponse::json(404, r#"{"message":"Not Found"}"#).with_header("content-range", "bytes */1012520")
        ])
        .await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let error = client.original_range("asset-1", 99_999_999, 100_000_000).await.unwrap_err();
        assert_eq!(error, ApiError::RangeNotSatisfiable);
    }

    #[tokio::test]
    async fn unknown_asset_on_a_range_is_not_found() {
        let server = FakeServer::start(vec![CannedResponse::json(404, r#"{"message":"Not Found"}"#)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let error = client.original_range("gone", 0, 10).await.unwrap_err();
        assert_eq!(error, ApiError::NotFound);
    }

    #[tokio::test]
    async fn an_ignored_range_is_refused() {
        let server = FakeServer::start(vec![CannedResponse::bytes(200, "image/jpeg", b"whole file")]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let error = client.original_range("asset-1", 0, 3).await.unwrap_err();
        assert!(matches!(error, ApiError::Protocol(_)), "{error:?}");
    }

    #[tokio::test]
    async fn thumbnails_return_their_content_type() {
        let server = FakeServer::start(vec![CannedResponse::bytes(200, "image/webp", b"RIFF")]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let (bytes, content_type) = client.thumbnail("asset-1", ThumbnailSize::Thumbnail).await.unwrap();
        assert_eq!(&bytes[..], b"RIFF");
        assert_eq!(content_type, "image/webp");
        assert_eq!(server.requests()[0].target, "/api/assets/asset-1/thumbnail?size=thumbnail");
    }
}

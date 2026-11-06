//! The secrets of one browser sign-in (SPEC.md, "Signing in and credentials").
//!
//! The Immich server is the OIDC client. We give it a PKCE challenge (S256) and a state, the user signs in in their
//! own browser, and the provider sends the browser to Immich's mobile redirect address with a code. Handing that
//! address back with the verifier proves it was our request.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use immich_api::Client;
use sha2::{Digest, Sha256};

/// Where the identity provider sends the browser: the address Immich's mobile app registers, which providers set up
/// for Immich already allow.
pub const REDIRECT_URI: &str = "app.immich:///oauth-callback";

/// A server address as typed, with `https://` added when no scheme was given; every edition's sign-in uses it.
pub fn normalised_address(typed: &str) -> String {
    let trimmed = typed.trim().trim_end_matches('/');
    if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    }
}

/// What a server offers, read before anyone signs in.
#[derive(Debug, Clone)]
pub struct ServerOffer {
    pub url: String,
    pub version: String,
    /// The text of the server's single sign-on button, when it has one.
    pub browser_button: Option<String>,
    pub password: bool,
}

/// A session from a password or browser sign-in: its token, and the account's email.
pub struct Session {
    pub token: String,
    pub account: String,
}

// Every edition's sign-in, over a client it made with its own user agent (`anonymous` has no credential). Errors are
// messages for people.

/// Check the server at `url` and what sign-in methods it offers. Needs no credential.
pub async fn probe(anonymous: &Client, url: &str) -> Result<ServerOffer, String> {
    let version = anonymous.server_version().await.map_err(|error| format!("Could not reach {url}: {error}"))?;
    if !version.is_supported() {
        return Err(format!(
            "This server runs Immich {version}. Immich Mount needs {} or later.",
            immich_api::version::MINIMUM_SUPPORTED
        ));
    }
    let features = anonymous.server_features().await.map_err(|error| error.to_string())?;
    let browser_button = if features.oauth {
        let config = anonymous.server_config().await.map_err(|error| error.to_string())?;
        Some(if config.oauth_button_text.trim().is_empty() { "Sign in with SSO".to_owned() } else { config.oauth_button_text })
    } else {
        None
    };
    Ok(ServerOffer { url: url.to_owned(), version: version.to_string(), browser_button, password: features.password_login })
}

/// Start a browser sign-in: the identity provider's address to open.
pub async fn begin_browser(anonymous: &Client, state: &str, code_challenge: &str) -> Result<String, String> {
    anonymous
        .oauth_authorize(REDIRECT_URI, state, code_challenge)
        .await
        .map_err(|error| format!("The server could not start single sign-on: {error}"))
}

/// Finish a browser sign-in with the address the browser was sent to.
pub async fn finish_browser(anonymous: &Client, redirect: &str, attempt: &BrowserSignIn) -> Result<Session, String> {
    if !is_redirect_for(redirect, &attempt.state) {
        return Err("That sign-in belongs to another attempt. Please sign in again.".to_owned());
    }
    let answer = anonymous
        .oauth_callback(redirect, &attempt.state, &attempt.code_verifier)
        .await
        .map_err(|error| format!("Signing in failed: {error}"))?;
    Ok(Session { token: answer.access_token, account: answer.user_email })
}

pub async fn password_session(anonymous: &Client, email: &str, password: &str) -> Result<Session, String> {
    let answer = anonymous.login_password(email, password).await.map_err(|error| match error {
        immich_api::ApiError::Unauthorized => "Wrong email or password.".to_owned(),
        other => format!("Signing in failed: {other}"),
    })?;
    Ok(Session { token: answer.access_token, account: answer.user_email })
}

/// Check that the server takes the API key `with_key` carries.
pub async fn check_api_key(with_key: &Client) -> Result<(), String> {
    with_key.api_key_permissions().await.map(|_| ()).map_err(|error| match error {
        immich_api::ApiError::Unauthorized => "The server did not accept this API key.".to_owned(),
        other => format!("Checking the key failed: {other}"),
    })
}

/// One sign-in attempt's state and PKCE pair. Never logged.
#[derive(Clone)]
pub struct BrowserSignIn {
    pub state: String,
    pub code_verifier: String,
    pub code_challenge: String,
}

impl BrowserSignIn {
    /// Fresh random secrets: a 256-bit verifier and a 128-bit state.
    pub fn new() -> Result<BrowserSignIn, getrandom::Error> {
        let code_verifier = random_text(32)?;
        Ok(BrowserSignIn { state: random_text(16)?, code_challenge: challenge_for(&code_verifier), code_verifier })
    }
}

fn random_text(byte_count: usize) -> Result<String, getrandom::Error> {
    let mut bytes = vec![0u8; byte_count];
    getrandom::fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The S256 challenge for a verifier (RFC 7636, section 4.2).
fn challenge_for(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
}

/// Whether a pasted address looks like the end of a browser sign-in: it carries a code and our state.
pub fn is_redirect_for(address: &str, state: &str) -> bool {
    let Some((_, query)) = address.split_once('?') else {
        return false;
    };
    let mut has_code = false;
    let mut has_state = false;
    for pair in query.split(['&', '#']) {
        match pair.split_once('=') {
            Some(("code", value)) => has_code = !value.is_empty(),
            Some(("state", value)) => has_state = value == state,
            _ => {}
        }
    }
    has_code && has_state
}

/// The identity provider's refusal in a redirect for `state` (RFC 6749 4.1.2.1: `error`, and `error_description`
/// when given), such as a sign-in the user cancelled. `None` when the redirect is for another attempt or is no error.
pub fn provider_error(address: &str, state: &str) -> Option<String> {
    let (_, query) = address.split_once('?')?;
    let (mut error, mut description, mut ours) = (None, None, false);
    for pair in query.split(['&', '#']) {
        match pair.split_once('=') {
            Some(("error", value)) => error = Some(decoded(value)),
            Some(("error_description", value)) => description = Some(decoded(value)),
            Some(("state", value)) => ours = value == state,
            _ => {}
        }
    }
    let error = error.filter(|_| ours)?;
    Some(match description {
        Some(description) if !description.is_empty() => format!("the sign-in was refused: {description} ({error})"),
        _ => format!("the sign-in was refused ({error})"),
    })
}

/// A query value with its `%XX` escapes and `+` spaces undone; anything malformed is kept as it is.
fn decoded(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
                match hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        index += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            other => out.push(other),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_host_becomes_an_https_address() {
        assert_eq!(normalised_address(" photos.example.org/ "), "https://photos.example.org");
        assert_eq!(normalised_address("http://192.168.1.20:2283"), "http://192.168.1.20:2283");
    }

    #[test]
    fn the_challenge_matches_the_rfc_example() {
        assert_eq!(challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn every_attempt_gets_new_secrets_of_the_right_length() {
        let first = BrowserSignIn::new().unwrap();
        let second = BrowserSignIn::new().unwrap();
        assert_ne!(first.code_verifier, second.code_verifier);
        assert_ne!(first.state, second.state);
        assert_eq!(first.code_verifier.len(), 43, "RFC 7636 wants 43 to 128 characters");
        assert_eq!(first.code_challenge, challenge_for(&first.code_verifier));
    }

    #[test]
    fn only_a_redirect_with_a_code_and_our_state_is_accepted() {
        assert!(is_redirect_for("app.immich:///oauth-callback?code=abc&state=s1", "s1"));
        assert!(is_redirect_for("https://photos.example.org/api/oauth/mobile-redirect?state=s1&code=abc", "s1"));
        assert!(!is_redirect_for("app.immich:///oauth-callback?code=abc&state=other", "s1"));
        assert!(!is_redirect_for("app.immich:///oauth-callback?error=access_denied&state=s1", "s1"));
        assert!(!is_redirect_for("not a url", "s1"));
    }

    #[test]
    fn a_refusal_for_our_attempt_is_read_and_decoded() {
        assert_eq!(
            provider_error("app.immich:///oauth-callback?error=access_denied&error_description=User+cancelled%21&state=s1", "s1").as_deref(),
            Some("the sign-in was refused: User cancelled! (access_denied)")
        );
        assert_eq!(provider_error("app.immich:///oauth-callback?error=access_denied&state=s1", "s1").as_deref(), Some("the sign-in was refused (access_denied)"));
        assert_eq!(provider_error("app.immich:///oauth-callback?error=access_denied&state=other", "s1"), None);
        assert_eq!(provider_error("app.immich:///oauth-callback?code=abc&state=s1", "s1"), None);
        assert_eq!(decoded("100%"), "100%");
        assert_eq!(decoded("%zzé%41"), "%zzéA");
    }
}

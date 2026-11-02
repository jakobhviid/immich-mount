//! Signing in: email and password, the server-side OIDC flow, and signing out.
//!
//! The Immich server is the OIDC client (ARCHITECTURE.md, "Signing in"): we ask it for the identity provider's
//! URL with our PKCE challenge and state, the user signs in in their own browser, and we hand the redirect URL
//! back with the verifier. Making the PKCE pair and state, and catching the redirect, are the caller's job.
//! Every path ends in a session token; use it as [`Credential::Session`](crate::Credential::Session).
//!
//! Errors: a wrong password is [`ApiError::Unauthorized`]; OIDC on a server without it is a
//! [`ApiError::Protocol`] carrying the server's "OAuth is not enabled".

use serde::Deserialize;
use serde_json::json;

use crate::asset::non_blank;
use crate::client::Client;
use crate::error::ApiError;

/// A new session.
#[derive(Clone, PartialEq, Eq)]
pub struct LoginResponse {
    /// The session token, for `Authorization: Bearer`.
    pub access_token: String,
    pub user_id: String,
    pub user_email: String,
    pub name: String,
    pub is_admin: bool,
}

impl std::fmt::Debug for LoginResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoginResponse")
            .field("access_token", &"<redacted>")
            .field("user_id", &self.user_id)
            .field("user_email", &self.user_email)
            .field("name", &self.name)
            .field("is_admin", &self.is_admin)
            .finish()
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct LoginWire {
    access_token: Option<String>,
    user_id: Option<String>,
    user_email: Option<String>,
    name: Option<String>,
    is_admin: Option<bool>,
}

impl LoginWire {
    fn into_login(self) -> Result<LoginResponse, ApiError> {
        let access_token = non_blank(self.access_token)
            .ok_or_else(|| ApiError::Protocol("sign-in answered without an access token".to_owned()))?;
        let user_id = non_blank(self.user_id)
            .ok_or_else(|| ApiError::Protocol("sign-in answered without a user id".to_owned()))?;
        Ok(LoginResponse {
            access_token,
            user_id,
            user_email: self.user_email.unwrap_or_default(),
            name: self.name.unwrap_or_default(),
            is_admin: self.is_admin.unwrap_or(false),
        })
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct AuthorizeWire {
    url: Option<String>,
}

/// The part of `GET /users/me` we read.
#[derive(Deserialize)]
struct MeWire {
    email: String,
}

impl Client {
    /// Sign in with email and password (`POST /auth/login`). Immich has no second factor on this path.
    pub async fn login_password(&self, email: &str, password: &str) -> Result<LoginResponse, ApiError> {
        let wire: LoginWire = self
            .post_json(&["auth", "login"], &json!({ "email": email, "password": password }))
            .await?;
        wire.into_login()
    }

    /// Start browser sign-in (`POST /oauth/authorize`): the identity provider URL to open.
    ///
    /// `redirect_uri` is usually `app.immich:///oauth-callback`; `code_challenge` is the PKCE S256 challenge.
    pub async fn oauth_authorize(&self, redirect_uri: &str, state: &str, code_challenge: &str) -> Result<String, ApiError> {
        let body = json!({ "redirectUri": redirect_uri, "state": state, "codeChallenge": code_challenge });
        let wire: AuthorizeWire = self.post_json(&["oauth", "authorize"], &body).await?;
        non_blank(wire.url).ok_or_else(|| ApiError::Protocol("OAuth authorize answered without a URL".to_owned()))
    }

    /// Finish browser sign-in (`POST /oauth/callback`) with the full redirect URL the browser was sent to.
    pub async fn oauth_callback(&self, url: &str, state: &str, code_verifier: &str) -> Result<LoginResponse, ApiError> {
        let body = json!({ "url": url, "state": state, "codeVerifier": code_verifier });
        let wire: LoginWire = self.post_json(&["oauth", "callback"], &body).await?;
        wire.into_login()
    }

    /// The signed-in account's email (`GET /users/me`). An API key needs `user.read` for it, and is refused without.
    pub async fn my_email(&self) -> Result<String, ApiError> {
        let me: MeWire = self.get_json(&["users", "me"], &[]).await?;
        Ok(me.email)
    }

    /// End the session this client holds (`POST /auth/logout`), so it disappears from "Authorized devices".
    pub async fn logout(&self) -> Result<(), ApiError> {
        self.post_without_answer::<()>(&["auth", "logout"], None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    const LOGIN: &str = r#"{"accessToken":"session-token","userId":"201f18cd-48ec-4220-8e10-1466650212ee",
        "userEmail":"admin@example.com","name":"Lab Admin","isAdmin":true,"profileImagePath":"",
        "shouldChangePassword":false,"isOnboarded":false}"#;

    #[test]
    fn login_decodes_and_hides_the_token_in_debug_output() {
        let login = serde_json::from_str::<LoginWire>(LOGIN).unwrap().into_login().unwrap();
        assert_eq!(login.access_token, "session-token");
        assert_eq!(login.user_email, "admin@example.com");
        assert!(login.is_admin);
        assert!(!format!("{login:?}").contains("session-token"));
    }

    #[test]
    fn login_without_a_token_is_a_protocol_error() {
        let wire: LoginWire = serde_json::from_str(r#"{"userId":"u"}"#).unwrap();
        assert!(matches!(wire.into_login(), Err(ApiError::Protocol(_))));
    }

    #[tokio::test]
    async fn wrong_password_is_unauthorized() {
        let server = FakeServer::start(vec![CannedResponse::json(401, r#"{"message":"Incorrect email or password"}"#)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let error = client.login_password("admin@example.com", "wrong").await.unwrap_err();
        assert_eq!(error, ApiError::Unauthorized);
        let request = &server.requests()[0];
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/api/auth/login");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request.body_text()).unwrap(),
            json!({ "email": "admin@example.com", "password": "wrong" })
        );
    }

    #[tokio::test]
    async fn oauth_sends_pkce_and_state() {
        let server = FakeServer::start(vec![
            CannedResponse::json(201, r#"{"url":"https://idp.example.com/authorize?x=1"}"#),
            CannedResponse::json(201, LOGIN),
        ])
        .await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let url = client
            .oauth_authorize("app.immich:///oauth-callback", "state-1", "challenge-1")
            .await
            .unwrap();
        assert_eq!(url, "https://idp.example.com/authorize?x=1");
        let login = client
            .oauth_callback("app.immich:///oauth-callback?code=c&state=state-1", "state-1", "verifier-1")
            .await
            .unwrap();
        assert_eq!(login.user_id, "201f18cd-48ec-4220-8e10-1466650212ee");
        let requests = server.requests();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&requests[0].body_text()).unwrap(),
            json!({ "redirectUri": "app.immich:///oauth-callback", "state": "state-1", "codeChallenge": "challenge-1" })
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&requests[1].body_text()).unwrap(),
            json!({ "url": "app.immich:///oauth-callback?code=c&state=state-1", "state": "state-1", "codeVerifier": "verifier-1" })
        );
    }
}

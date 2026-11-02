//! The credential the daemon signs in with: a session token or an API key (SPEC.md, "Signing in and credentials").
//!
//! On desktops it lives in the keyring. This module also provides the headless fallback: a file that must be readable
//! by its owner only (0600), or it is refused. The credential never appears in logs.

use std::fmt;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// What Immich accepts.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    ApiKey(String),
    Session(String),
}

impl fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credential::ApiKey(_) => formatter.write_str("ApiKey(<hidden>)"),
            Credential::Session(_) => formatter.write_str("Session(<hidden>)"),
        }
    }
}

impl Credential {
    pub fn kind(&self) -> &'static str {
        match self {
            Credential::ApiKey(_) => "api-key",
            Credential::Session(_) => "session",
        }
    }

    pub fn secret(&self) -> &str {
        match self {
            Credential::ApiKey(secret) | Credential::Session(secret) => secret,
        }
    }
}

/// Why a credential file could not be used.
#[derive(Debug, thiserror::Error)]
pub enum CredentialFileError {
    #[error("no credential at {0}; sign in with `immich-mount login`")]
    Missing(String),
    #[error("{path} is readable by others (mode {mode:o}); it must be 0600")]
    TooOpen { path: String, mode: u32 },
    #[error("{0} is not a credential file written by immich-mount")]
    Malformed(String),
    #[error("{path}: {message}")]
    Io { path: String, message: String },
}

/// Read a credential file: its first line is the kind (`api-key` or `session`), the second the secret.
pub fn read_credential_file(path: &Path) -> Result<Credential, CredentialFileError> {
    let shown = path.display().to_string();
    let metadata = std::fs::metadata(path).map_err(|_| CredentialFileError::Missing(shown.clone()))?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(CredentialFileError::TooOpen { path: shown, mode });
    }
    let text = std::fs::read_to_string(path).map_err(|error| CredentialFileError::Io { path: shown.clone(), message: error.to_string() })?;
    parse_credential(&text).ok_or(CredentialFileError::Malformed(shown))
}

/// The stored form of a credential, in the file and in the keyring alike: the kind on one line, the secret on the next.
pub fn format_credential(credential: &Credential) -> String {
    format!("{}\n{}\n", credential.kind(), credential.secret())
}

/// A credential from its stored form.
pub fn parse_credential(text: &str) -> Option<Credential> {
    let mut lines = text.lines();
    let kind = lines.next().unwrap_or_default().trim();
    let secret = lines.next().unwrap_or_default().trim().to_string();
    match (kind, secret.is_empty()) {
        ("api-key", false) => Some(Credential::ApiKey(secret)),
        ("session", false) => Some(Credential::Session(secret)),
        _ => None,
    }
}

/// Write a credential file with mode 0600, replacing any previous one.
pub fn write_credential_file(path: &Path, credential: &Credential) -> Result<(), CredentialFileError> {
    let shown = path.display().to_string();
    let io_error = |error: std::io::Error| CredentialFileError::Io { path: shown.clone(), message: error.to_string() };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&temporary).map_err(io_error)?;
    file.write_all(format_credential(credential).as_bytes()).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    std::fs::rename(&temporary, path).map_err(io_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_credential_reads_back_and_is_private() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credential");
        write_credential_file(&path, &Credential::ApiKey("secret-key".into())).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(read_credential_file(&path).unwrap(), Credential::ApiKey("secret-key".into()));
    }

    #[test]
    fn a_readable_file_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credential");
        write_credential_file(&path, &Credential::Session("token".into())).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(read_credential_file(&path), Err(CredentialFileError::TooOpen { .. })));
    }

    #[test]
    fn debug_output_never_shows_the_secret() {
        assert_eq!(format!("{:?}", Credential::ApiKey("abc".into())), "ApiKey(<hidden>)");
    }
}

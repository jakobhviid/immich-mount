//! The credential in the desktop keyring: the Secret Service, through `oo7` (SPEC.md, "Signing in and credentials").
//!
//! One item per server, labelled "Immich Mount: <server>", holding the credential in the same two-line form as the
//! headless file. A locked keyring brings up the system's unlock prompt. A dismissed prompt is an error of its own,
//! never a silent fallback to the file.

use crate::credentials::{format_credential, parse_credential, Credential};

/// The `application` attribute every item carries.
const APPLICATION: &str = "dev.hviid.ImmichMount";

#[derive(Debug, thiserror::Error)]
pub enum KeyringError {
    #[error("no keyring is available: {0}")]
    Unavailable(String),
    #[error("the keyring stayed locked (the unlock prompt was dismissed)")]
    Dismissed,
    #[error("the keyring failed: {0}")]
    Failed(String),
}

fn classify(error: oo7::Error) -> KeyringError {
    match error {
        oo7::Error::DBus(oo7::dbus::Error::Dismissed) => KeyringError::Dismissed,
        oo7::Error::DBus(oo7::dbus::Error::ZBus(error)) => KeyringError::Unavailable(error.to_string()),
        other => KeyringError::Failed(other.to_string()),
    }
}

/// The Secret Service's bus name, and the name KDE's `ksecretd` can be started by.
const SECRET_SERVICE: &str = "org.freedesktop.secrets";
const KWALLET_SECRET_SERVICE: &str = "org.kde.secretservicecompat";

/// Start KWallet's Secret Service where nothing has. `ksecretd` takes `org.freedesktop.secrets` once it runs, but D-Bus
/// can start it only by its own name; a Plasma login usually starts it through `pam_kwallet`, and without that every
/// Secret Service client finds no keyring (seen in a Plasma VM). Elsewhere, or when a keyring already answers,
/// this does nothing.
async fn wake_kwallet() {
    let Ok(connection) = zbus::Connection::session().await else { return };
    let Ok(bus) = zbus::fdo::DBusProxy::new(&connection).await else { return };
    let (Ok(secrets), Ok(kwallet)) = (zbus::names::BusName::try_from(SECRET_SERVICE), zbus::names::BusName::try_from(KWALLET_SECRET_SERVICE)) else {
        return;
    };
    if bus.name_has_owner(secrets.clone()).await.unwrap_or(true) {
        return;
    }
    // Where a keyring can be started by the Secret Service's own name (GNOME Keyring), D-Bus starts that one on the
    // first request; KWallet must not take its place on a machine that has both.
    let activatable = bus.list_activatable_names().await.unwrap_or_default();
    if activatable.iter().any(|name| name.as_str() == SECRET_SERVICE) || !activatable.iter().any(|name| name.as_str() == KWALLET_SECRET_SERVICE) {
        return;
    }
    let zbus::names::BusName::WellKnown(kwallet) = kwallet else { return };
    if bus.start_service_by_name(kwallet, 0).await.is_err() {
        return;
    }
    for _ in 0..50 {
        if bus.name_has_owner(secrets.clone()).await.unwrap_or(false) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn attributes(server: &str) -> [(&'static str, &str); 2] {
    [("application", APPLICATION), ("server", server)]
}

/// Store the credential for `server`, replacing any earlier one. Unlocks the keyring first, with a prompt if needed.
pub async fn store(server: &str, credential: &Credential) -> Result<(), KeyringError> {
    wake_kwallet().await;
    let keyring = oo7::Keyring::new().await.map_err(classify)?;
    keyring.unlock().await.map_err(classify)?;
    keyring
        .create_item(&format!("Immich Mount: {server}"), &attributes(server), format_credential(credential), true)
        .await
        .map_err(classify)
}

/// The items Immich Mount stored that match `attributes`, in the keyrings that already exist. Never the default
/// collection by alias: asking for it creates a keyring when there is none, with a prompt, which only storing a
/// credential should ever do.
async fn existing_items(attributes: &[(&str, &str)]) -> Result<Vec<oo7::dbus::Item>, KeyringError> {
    let dbus_error = |error: oo7::dbus::Error| classify(oo7::Error::DBus(error));
    wake_kwallet().await;
    let service = oo7::dbus::Service::new().await.map_err(dbus_error)?;
    let mut found = Vec::new();
    for collection in service.collections().await.map_err(dbus_error)? {
        found.extend(collection.search_items(&attributes).await.map_err(dbus_error)?);
    }
    Ok(found)
}

/// The credential stored for `server`, if any. A locked item is unlocked first, with a prompt if needed.
pub async fn load(server: &str) -> Result<Option<Credential>, KeyringError> {
    let dbus_error = |error: oo7::dbus::Error| classify(oo7::Error::DBus(error));
    let items = existing_items(&attributes(server)).await?;
    let Some(item) = items.first() else {
        return Ok(None);
    };
    if item.is_locked().await.map_err(dbus_error)? {
        item.unlock(None).await.map_err(dbus_error)?;
    }
    let secret = item.secret().await.map_err(dbus_error)?;
    let text = String::from_utf8_lossy(secret.as_bytes());
    parse_credential(&text).map(Some).ok_or_else(|| KeyringError::Failed("the stored credential is not one immich-mount wrote".to_owned()))
}

/// Remove every credential Immich Mount stored, for any server. Nothing stored is not an error, and no keyring is
/// created or unlocked unless there is something to remove.
pub async fn remove_all() -> Result<(), KeyringError> {
    for item in existing_items(&[("application", APPLICATION)]).await? {
        item.delete(None).await.map_err(|error| classify(oo7::Error::DBus(error)))?;
    }
    Ok(())
}

/// Remove the credential stored for `server`. Nothing stored is not an error.
pub async fn remove(server: &str) -> Result<(), KeyringError> {
    for item in existing_items(&attributes(server)).await? {
        item.delete(None).await.map_err(|error| classify(oo7::Error::DBus(error)))?;
    }
    Ok(())
}

/// Whether a credential is stored for `server`. Looks without unlocking or creating anything, so it never brings up
/// a prompt.
pub async fn is_stored(server: &str) -> bool {
    existing_items(&attributes(server)).await.is_ok_and(|items| !items.is_empty())
}

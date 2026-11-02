//! What Immich Mount's desktop apps share (ARCHITECTURE.md, "The desktop app"): checking a server, signing in,
//! keeping the credential, first-run setup, the daemon's status and disconnecting. No toolkit lives here, so the
//! GNOME app (GTK) and a KDE app (Qt) are thin layers over the same behaviour.

pub mod preferences;
pub mod session;

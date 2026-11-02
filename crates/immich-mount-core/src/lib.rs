//! The portable core of Immich Mount.
//!
//! Holds every piece of logic that a platform adapter needs: the identifier-based library model, naming and
//! clashes, membership from Immich, the metadata store and change journal, the content cache, upload logic and the
//! settings table (ARCHITECTURE.md, "Crates"). No FUSE, D-Bus, GTK or OS-specific code lives here; platform services
//! come in through traits.

pub mod content;
pub mod immich;
pub mod layout;
pub mod model;
pub mod naming;
pub mod settings;
pub mod sign_in;
pub mod source;
pub mod store;
pub mod tree;
pub mod uploads;

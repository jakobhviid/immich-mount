//! The Linux adapter of Immich Mount.
//!
//! Presents the core through FUSE (the inode table, kernel invalidation, staging and upload on close), and provides
//! the freedesktop services: thumbnail seeding, the Secret Service, notifications, translations and the daemon's D-Bus
//! interface (ARCHITECTURE.md, "Crates").

pub mod caller;
pub mod credentials;
pub mod daemon;
pub mod dbus;
pub mod doctor;
pub mod keyring;
pub mod fuse;
pub mod gnome;
pub mod i18n;
pub mod kde;
pub mod mounts;
pub mod notify;
pub mod paths;
pub mod programs;
pub mod seeding;
pub mod service;
pub mod staging;
pub mod thumbnails;

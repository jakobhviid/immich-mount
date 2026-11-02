//! Which Immich server versions are supported.
//!
//! Policy (SPEC.md, "Supported versions"): the current and previous minor release. The floor moves forward
//! deliberately, so it is a single constant here.

/// An Immich server version, as reported by `GET /server/version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ServerVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// The oldest server Immich Mount supports.
pub const MINIMUM_SUPPORTED: ServerVersion = ServerVersion { major: 3, minor: 2, patch: 0 };

impl ServerVersion {
    /// Whether this server is new enough to use.
    pub fn is_supported(&self) -> bool {
        *self >= MINIMUM_SUPPORTED
    }
}

impl std::fmt::Display for ServerVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "v{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u32, minor: u32, patch: u32) -> ServerVersion {
        ServerVersion { major, minor, patch }
    }

    #[test]
    fn floor_and_newer_are_supported() {
        assert!(version(3, 2, 0).is_supported());
        assert!(version(3, 3, 5).is_supported());
        assert!(version(4, 0, 0).is_supported());
    }

    #[test]
    fn older_than_floor_is_not_supported() {
        assert!(!version(3, 1, 9).is_supported());
        assert!(!version(1, 135, 0).is_supported());
    }

    #[test]
    fn displays_like_immich_tags() {
        assert_eq!(version(3, 3, 5).to_string(), "v3.3.5");
    }
}

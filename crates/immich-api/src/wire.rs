//! What every model shares: lenient decoding, enums that keep unknown values, bulk results, and the table of
//! endpoints the crate calls.
//!
//! The rules for models (the modules added for the full API follow them; the mount's original modules keep their
//! separate wire structs, which decode the same way):
//! - a model derives `Deserialize` with a struct-level `#[serde(default)]`, so a field missing on an older server
//!   takes its default, and unknown fields are ignored
//! - `null` counts as missing: [`strip_nulls`] removes null object members before decoding, so a field the server
//!   sends as `null` also takes its default, whatever its type
//! - an enum keeps a value it does not know as `Other(text)` ([`wire_enum!`]), so a new server value never breaks
//!   an answer
//! - dates go through [`crate::dates::serde_dates`], which accepts every spelling Immich uses
//! - a model's id may decode empty; [`crate::client::Client`] refuses an empty path segment before sending, so a
//!   model without an id can never reach a list endpoint by accident

use serde::Deserialize;

/// Remove `null` members from every object in a JSON value, recursively. Array elements stay, so columnar answers
/// (the timeline bucket) keep their positions.
pub(crate) fn strip_nulls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(members) => {
            members.retain(|_, member| !member.is_null());
            members.values_mut().for_each(strip_nulls);
        }
        serde_json::Value::Array(elements) => elements.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}

/// An enum of wire strings that keeps a value it does not know.
///
/// ```ignore
/// wire_enum! {
///     /// A doc comment.
///     pub enum AlbumUserRole { Owner = "owner", Editor = "editor", Viewer = "viewer" }
/// }
/// ```
///
/// Generates the variants plus `Other(String)`, `as_str`, `Display`, `From<String>`, and serde both ways (as the
/// string). `Default` is the first variant.
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(#[$first_meta:meta])* $first:ident = $first_wire:literal
            $(, $(#[$variant_meta:meta])* $variant:ident = $wire:literal)* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
        pub enum $name {
            $(#[$first_meta])*
            #[default]
            $first,
            $($(#[$variant_meta])* $variant,)*
            /// A value this client does not know, verbatim.
            Other(String),
        }

        impl $name {
            /// The wire value.
            pub fn as_str(&self) -> &str {
                match self {
                    $name::$first => $first_wire,
                    $($name::$variant => $wire,)*
                    $name::Other(value) => value,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<String> for $name {
            fn from(wire: String) -> Self {
                match wire.as_str() {
                    $first_wire => $name::$first,
                    $($wire => $name::$variant,)*
                    _ => $name::Other(wire),
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map($name::from)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }
    };
}
pub(crate) use wire_enum;

wire_enum! {
    /// Why one id of a bulk request failed.
    pub enum BulkIdError {
        Duplicate = "duplicate",
        NoPermission = "no_permission",
        NotFound = "not_found",
        Unknown = "unknown",
        Validation = "validation",
    }
}

/// The outcome for one id of a bulk request (adding Assets to an album, tagging, restoring).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BulkIdResult {
    pub id: String,
    pub success: bool,
    /// Why it failed; `None` on success.
    pub error: Option<BulkIdError>,
    pub error_message: Option<String>,
}

/// The body most bulk endpoints take: `{"ids": [...]}`.
#[derive(Debug, serde::Serialize)]
pub(crate) struct BulkIds<'a> {
    pub ids: &'a [String],
}

/// A secret the server sends or is sent: an API key's value, a session token, a password, a PIN code, a shared
/// link's key. Its `Debug` output is redacted, so a model printed in a log never shows it; read it with
/// [`Secret::expose`].
#[derive(Clone, PartialEq, Eq, Default, Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Secret {
        Secret(text.into())
    }

    /// The secret itself.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl From<String> for Secret {
    fn from(text: String) -> Secret {
        Secret(text)
    }
}

impl From<&str> for Secret {
    fn from(text: &str) -> Secret {
        Secret(text.to_owned())
    }
}

/// One endpoint the crate calls, with the API-key permission Immich requires for it.
///
/// The table ([`crate::endpoints`]) is what the spec-conformance contract test checks against the server's
/// `/api/spec.json`: every entry must exist on every supported version, with this permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    /// `GET`, `POST`, `PUT`, `PATCH` or `DELETE`.
    pub method: &'static str,
    /// The path below `/api`, as in the spec: `/albums/{id}/assets`.
    pub path: &'static str,
    /// The `x-immich-permission` of the operation; `None` where the spec names none. Such an endpoint is public
    /// (`/server/ping`), or for an API key needs `all` (`/auth/status`).
    pub permission: Option<&'static str>,
}

/// Shorthand for the endpoint tables in each module.
macro_rules! endpoints {
    ($($method:ident $path:literal $permission:tt),* $(,)?) => {
        &[$($crate::wire::Endpoint {
            method: stringify!($method),
            path: $path,
            permission: $crate::wire::endpoints!(@permission $permission),
        }),*]
    };
    (@permission -) => { None };
    (@permission $permission:literal) => { Some($permission) };
}
pub(crate) use endpoints;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nulls_in_objects_are_removed_but_array_slots_stay() {
        let mut value = serde_json::json!({"a": null, "b": {"c": null, "d": 1}, "e": [null, {"f": null}]});
        strip_nulls(&mut value);
        assert_eq!(value, serde_json::json!({"b": {"d": 1}, "e": [null, {}]}));
    }

    #[test]
    fn a_secret_is_redacted_in_debug_output_and_sent_as_is() {
        let secret = Secret::from("s3cret");
        assert_eq!(format!("{secret:?}"), "<redacted>");
        assert_eq!(serde_json::to_string(&secret).unwrap(), "\"s3cret\"");
        assert_eq!(secret.expose(), "s3cret");
    }

    #[test]
    fn unknown_enum_values_are_kept() {
        let known: BulkIdError = serde_json::from_str("\"duplicate\"").unwrap();
        assert_eq!(known, BulkIdError::Duplicate);
        let unknown: BulkIdError = serde_json::from_str("\"quota\"").unwrap();
        assert_eq!(unknown, BulkIdError::Other("quota".to_owned()));
        assert_eq!(serde_json::to_string(&unknown).unwrap(), "\"quota\"");
    }

    #[test]
    fn a_null_field_takes_its_default() {
        let mut value = serde_json::json!({"id": "a", "success": null, "error": null});
        strip_nulls(&mut value);
        let result: BulkIdResult = serde_json::from_value(value).unwrap();
        assert_eq!(result, BulkIdResult { id: "a".to_owned(), ..BulkIdResult::default() });
    }
}

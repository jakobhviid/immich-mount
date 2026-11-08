//! Changing Assets: their date, time zone, place, description, rating, favourite and visibility; moving them to
//! the trash or deleting them; non-destructive edits; key-value metadata; copying properties from one Asset to
//! another; and asking the server to redo its work on them.
//!
//! Quirks, seen on v3.3.6:
//! - `PUT /assets` (bulk) is the one endpoint that shifts dates (`dateTimeRelative`, in minutes) and sets the
//!   time zone; `PUT /assets/{id}` refuses those fields, so [`Client::update_asset`] leaves them out
//! - both update endpoints are marked deprecated in v3.3.6 with no replacement yet; they are the only way on v3.2
//! - `rating: null` clears a rating; leaving the field out keeps it
//! - `DELETE /assets` moves to the trash unless `force` is set; with the trash switched off on the server it deletes
//!   at once
//! - an edit list replaces the whole list: `PUT /assets/{id}/edits` with one rotation removes an earlier crop
//! - a crop must be the first step of the list (400 `Crop action must be the first edit action` otherwise)

use chrono::{DateTime, FixedOffset, SecondsFormat};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::asset::Visibility;
use crate::asset_info::{AssetDetails, MetadataEntry};
use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, wire_enum, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    PUT "/assets/{id}" "asset.update",
    PUT "/assets" "asset.update",
    DELETE "/assets" "asset.delete",
    GET "/assets/{id}/edits" "asset.edit.get",
    PUT "/assets/{id}/edits" "asset.edit.create",
    DELETE "/assets/{id}/edits" "asset.edit.delete",
    PUT "/assets/{id}/metadata" "asset.update",
    DELETE "/assets/{id}/metadata/{key}" "asset.update",
    PUT "/assets/metadata" "asset.update",
    DELETE "/assets/metadata" "asset.update",
    PUT "/assets/copy" "asset.copy",
    POST "/assets/jobs" "job.create",
];

/// Changes to one or more Assets; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetChanges {
    /// When it was taken, with the offset of the place it was taken.
    #[serde(serialize_with = "send_offset_date", skip_serializing_if = "Option::is_none")]
    pub date_time_original: Option<DateTime<FixedOffset>>,
    /// Shift the capture date by this many minutes. Bulk updates only ([`Client::update_assets`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_time_relative: Option<i64>,
    /// An IANA zone such as `Europe/Copenhagen`. Bulk updates only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// Set together with `longitude`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    /// An empty description removes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `Some(None)` removes the rating; -1 marks it rejected, 1 to 5 are stars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Option<i8>>,
    #[serde(rename = "isFavorite", skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    /// Archive, unarchive, hide, or move to the locked folder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    /// Pair a still with its video half, or `Some(None)` to unpair. Single updates only ([`Client::update_asset`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_photo_video_id: Option<Option<String>>,
    /// Put into a duplicate group, or `Some(None)` to take out of one. Bulk updates only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_id: Option<Option<String>>,
}

fn send_offset_date<S: serde::Serializer>(value: &Option<DateTime<FixedOffset>>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(date) => serializer.serialize_str(&date.to_rfc3339_opts(SecondsFormat::Millis, true)),
        None => serializer.serialize_none(),
    }
}

/// The fields only the bulk endpoint takes, and the one only the single endpoint takes.
const BULK_ONLY: [&str; 3] = ["dateTimeRelative", "timeZone", "duplicateId"];
const SINGLE_ONLY: [&str; 1] = ["livePhotoVideoId"];

fn changes_body(changes: &AssetChanges, leave_out: &[&str]) -> Result<serde_json::Map<String, serde_json::Value>, ApiError> {
    let value = serde_json::to_value(changes)
        .map_err(|error| ApiError::Protocol(format!("internal: cannot encode asset changes: {error}")))?;
    let serde_json::Value::Object(mut members) = value else {
        return Err(ApiError::Protocol("internal: asset changes did not encode as an object".to_owned()));
    };
    members.retain(|name, _| !leave_out.contains(&name.as_str()));
    Ok(members)
}

/// One step of a non-destructive edit. The list of steps is applied in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetEdit {
    /// Keep a rectangle, in pixels of the original.
    Crop { x: u32, y: u32, width: u32, height: u32 },
    /// Turn clockwise by a multiple of 90 degrees.
    Rotate { angle: i32 },
    /// Flip along an axis.
    Mirror { axis: MirrorAxis },
    /// A step this client does not know, kept verbatim so it can be sent back unchanged.
    Other { action: String, parameters: serde_json::Value },
}

wire_enum! {
    /// The axis a mirror step flips along.
    pub enum MirrorAxis {
        Horizontal = "horizontal",
        Vertical = "vertical",
    }
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct EditWire {
    action: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CropWire {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl AssetEdit {
    fn from_wire(wire: EditWire) -> AssetEdit {
        let parameters = wire.parameters;
        match wire.action.as_str() {
            "crop" => match serde_json::from_value::<CropWire>(parameters.clone()) {
                Ok(crop) => AssetEdit::Crop { x: crop.x, y: crop.y, width: crop.width, height: crop.height },
                Err(_) => AssetEdit::Other { action: wire.action, parameters },
            },
            "rotate" => match parameters.get("angle").and_then(serde_json::Value::as_i64) {
                Some(angle) => AssetEdit::Rotate { angle: angle as i32 },
                None => AssetEdit::Other { action: wire.action, parameters },
            },
            "mirror" => match parameters.get("axis").and_then(serde_json::Value::as_str) {
                Some(axis) => AssetEdit::Mirror { axis: MirrorAxis::from(axis.to_owned()) },
                None => AssetEdit::Other { action: wire.action, parameters },
            },
            _ => AssetEdit::Other { action: wire.action, parameters },
        }
    }

    fn to_wire(&self) -> EditWire {
        match self {
            AssetEdit::Crop { x, y, width, height } => EditWire {
                action: "crop".to_owned(),
                parameters: serde_json::json!({ "x": x, "y": y, "width": width, "height": height }),
            },
            AssetEdit::Rotate { angle } => EditWire {
                action: "rotate".to_owned(),
                parameters: serde_json::json!({ "angle": angle }),
            },
            AssetEdit::Mirror { axis } => EditWire {
                action: "mirror".to_owned(),
                parameters: serde_json::json!({ "axis": axis.as_str() }),
            },
            AssetEdit::Other { action, parameters } => EditWire {
                action: action.clone(),
                parameters: parameters.clone(),
            },
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct EditsWire {
    edits: Vec<EditWire>,
}

impl EditsWire {
    fn into_edits(self) -> Vec<AssetEdit> {
        self.edits.into_iter().map(AssetEdit::from_wire).collect()
    }
}

/// What to copy from one Asset to another with [`Client::copy_asset_properties`]. Each is copied when `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyOptions {
    /// Album membership.
    pub albums: bool,
    pub favorite: bool,
    pub shared_links: bool,
    /// The XMP sidecar.
    pub sidecar: bool,
    /// Stack membership.
    pub stack: bool,
}

wire_enum! {
    /// Work the server can redo for chosen Assets.
    pub enum AssetJob {
        RefreshMetadata = "refresh-metadata",
        RefreshFaces = "refresh-faces",
        RegenerateThumbnail = "regenerate-thumbnail",
        TranscodeVideo = "transcode-video",
    }
}

impl Client {
    /// Change one Asset, and get it back (`PUT /assets/{id}`). The bulk-only fields of `changes` are left out.
    /// Needs `asset.update`.
    pub async fn update_asset(&self, id: &str, changes: &AssetChanges) -> Result<AssetDetails, ApiError> {
        let body = changes_body(changes, &BULK_ONLY)?;
        self.call_with(Method::PUT, &["assets", id], &[], &body).await
    }

    /// Change many Assets the same way (`PUT /assets`). The single-only field of `changes` is left out. Needs
    /// `asset.update`.
    pub async fn update_assets(&self, ids: &[String], changes: &AssetChanges) -> Result<(), ApiError> {
        let mut body = changes_body(changes, &SINGLE_ONLY)?;
        body.insert("ids".to_owned(), serde_json::json!(ids));
        self.call_unit_with(Method::PUT, &["assets"], &[], &body).await
    }

    /// Move Assets to the trash, or with `force` delete them for good (`DELETE /assets`). Needs `asset.delete`.
    pub async fn delete_assets(&self, ids: &[String], force: bool) -> Result<(), ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            ids: &'a [String],
            force: bool,
        }
        self.call_unit_with(Method::DELETE, &["assets"], &[], &Body { ids, force }).await
    }

    /// An Asset's edit steps, in order (`GET /assets/{id}/edits`). Needs `asset.edit.get`.
    pub async fn asset_edits(&self, id: &str) -> Result<Vec<AssetEdit>, ApiError> {
        let wire: EditsWire = self.call(Method::GET, &["assets", id, "edits"], &[]).await?;
        Ok(wire.into_edits())
    }

    /// Replace an Asset's edit steps (`PUT /assets/{id}/edits`); returns them as stored. A crop must come first.
    /// Needs `asset.edit.create`.
    pub async fn set_asset_edits(&self, id: &str, edits: &[AssetEdit]) -> Result<Vec<AssetEdit>, ApiError> {
        #[derive(Serialize)]
        struct Body {
            edits: Vec<EditWire>,
        }
        let body = Body { edits: edits.iter().map(AssetEdit::to_wire).collect() };
        let wire: EditsWire = self.call_with(Method::PUT, &["assets", id, "edits"], &[], &body).await?;
        Ok(wire.into_edits())
    }

    /// Remove every edit step, back to the original (`DELETE /assets/{id}/edits`). Needs `asset.edit.delete`.
    pub async fn delete_asset_edits(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["assets", id, "edits"], &[]).await
    }

    /// Set key-value entries on an Asset (`PUT /assets/{id}/metadata`); each value must be a JSON object. Needs
    /// `asset.update`.
    pub async fn set_asset_metadata(
        &self,
        id: &str,
        entries: &[(String, serde_json::Value)],
    ) -> Result<Vec<MetadataEntry>, ApiError> {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|(key, value)| serde_json::json!({ "key": key, "value": value }))
            .collect();
        self.call_with(Method::PUT, &["assets", id, "metadata"], &[], &serde_json::json!({ "items": items }))
            .await
    }

    /// Remove one key-value entry from an Asset (`DELETE /assets/{id}/metadata/{key}`). Needs `asset.update`.
    pub async fn delete_asset_metadata(&self, id: &str, key: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["assets", id, "metadata", key], &[]).await
    }

    /// Set key-value entries across Assets, `(asset id, key, value)` each (`PUT /assets/metadata`). Needs
    /// `asset.update`.
    pub async fn set_assets_metadata(
        &self,
        entries: &[(String, String, serde_json::Value)],
    ) -> Result<Vec<MetadataEntry>, ApiError> {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|(asset_id, key, value)| serde_json::json!({ "assetId": asset_id, "key": key, "value": value }))
            .collect();
        self.call_with(Method::PUT, &["assets", "metadata"], &[], &serde_json::json!({ "items": items }))
            .await
    }

    /// Remove key-value entries across Assets, `(asset id, key)` each (`DELETE /assets/metadata`). Needs
    /// `asset.update`.
    pub async fn delete_assets_metadata(&self, entries: &[(String, String)]) -> Result<(), ApiError> {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|(asset_id, key)| serde_json::json!({ "assetId": asset_id, "key": key }))
            .collect();
        self.call_unit_with(Method::DELETE, &["assets", "metadata"], &[], &serde_json::json!({ "items": items }))
            .await
    }

    /// Copy albums, favourite, shared links, sidecar or stack from one Asset to another (`PUT /assets/copy`), as
    /// when replacing a photo with an edited version. Needs `asset.copy`.
    pub async fn copy_asset_properties(&self, source_id: &str, target_id: &str, options: &CopyOptions) -> Result<(), ApiError> {
        let mut body = serde_json::to_value(options)
            .map_err(|error| ApiError::Protocol(format!("internal: cannot encode copy options: {error}")))?;
        body["sourceId"] = serde_json::json!(source_id);
        body["targetId"] = serde_json::json!(target_id);
        self.call_unit_with(Method::PUT, &["assets", "copy"], &[], &body).await
    }

    /// Ask the server to redo some work for Assets (`POST /assets/jobs`). Needs `job.create`.
    pub async fn run_asset_job(&self, ids: &[String], job: &AssetJob) -> Result<(), ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            asset_ids: &'a [String],
            name: &'a AssetJob,
        }
        self.call_unit_with(Method::POST, &["assets", "jobs"], &[], &Body { asset_ids: ids, name: job })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn single_and_bulk_updates_send_their_own_fields() {
        let changes = AssetChanges {
            date_time_original: DateTime::parse_from_rfc3339("2024-06-01T12:00:00+02:00").ok(),
            time_zone: Some("Europe/Copenhagen".to_owned()),
            rating: Some(None),
            live_photo_video_id: Some(None),
            ..AssetChanges::default()
        };
        let single = changes_body(&changes, &BULK_ONLY).unwrap();
        assert_eq!(
            serde_json::Value::Object(single),
            serde_json::json!({"dateTimeOriginal": "2024-06-01T12:00:00.000+02:00", "rating": null, "livePhotoVideoId": null})
        );
        let bulk = changes_body(&changes, &SINGLE_ONLY).unwrap();
        assert_eq!(
            serde_json::Value::Object(bulk),
            serde_json::json!({"dateTimeOriginal": "2024-06-01T12:00:00.000+02:00", "timeZone": "Europe/Copenhagen", "rating": null})
        );
    }

    #[test]
    fn edits_round_trip_and_keep_unknown_steps() {
        let json = r#"{"assetId":"a","edits":[
            {"id":"1","action":"crop","parameters":{"x":1,"y":2,"width":30,"height":40}},
            {"id":"2","action":"rotate","parameters":{"angle":90}},
            {"id":"3","action":"mirror","parameters":{"axis":"horizontal"}},
            {"id":"4","action":"blur","parameters":{"radius":3}}]}"#;
        let edits = crate::client::decode_json::<EditsWire>(json.as_bytes(), "edits").unwrap().into_edits();
        assert_eq!(
            edits,
            [
                AssetEdit::Crop { x: 1, y: 2, width: 30, height: 40 },
                AssetEdit::Rotate { angle: 90 },
                AssetEdit::Mirror { axis: MirrorAxis::Horizontal },
                AssetEdit::Other { action: "blur".to_owned(), parameters: serde_json::json!({"radius": 3}) },
            ]
        );
        assert_eq!(
            serde_json::to_value(edits[3].to_wire()).unwrap(),
            serde_json::json!({"action": "blur", "parameters": {"radius": 3}})
        );
    }

    #[tokio::test]
    async fn trash_is_a_delete_without_force() {
        let server = FakeServer::start(vec![CannedResponse::json(204, "")]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        client.delete_assets(&["a".to_owned()], false).await.unwrap();
        let request = &server.requests()[0];
        assert_eq!((request.method.as_str(), request.target.as_str()), ("DELETE", "/api/assets"));
        assert_eq!(request.body_text(), r#"{"ids":["a"],"force":false}"#);
    }
}

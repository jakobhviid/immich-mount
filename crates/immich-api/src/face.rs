//! Faces: the boxes on an Asset that show a person, found by face detection or drawn by hand.
//!
//! A box is in pixels of the picture the face was found on (`image_width` by `image_height`, the preview for
//! detected faces, whatever the caller said for drawn ones), so scale it to the size shown.
//!
//! Quirks, seen on v3.3.6:
//! - detection needs machine learning; without it only faces drawn by hand ([`Client::create_face`]) exist
//! - `POST /faces` answers 201 with no body, so the new face's id comes from [`Client::faces`]
//! - `PUT /faces/{id}` takes the **person** in the path and the face in the body (`{"id": <face>}`), the reverse of
//!   what its path suggests; [`Client::reassign_face`] takes them by name
//! - deleting a person leaves their faces on the Assets with no person
//! - `DELETE /faces/{id}` without `force` marks the face deleted (it disappears from [`Client::faces`]), and with
//!   `force` removes it for good; a face marked deleted can still be removed with `force`

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::error::ApiError;
use crate::person::Person;
use crate::wire::{endpoints, wire_enum, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/faces" "face.read",
    POST "/faces" "face.create",
    PUT "/faces/{id}" "face.update",
    DELETE "/faces/{id}" "face.delete",
];

wire_enum! {
    /// Where a face came from.
    pub enum FaceSource {
        MachineLearning = "machine-learning",
        Exif = "exif",
        Manual = "manual",
    }
}

/// A face on an Asset (`AssetFaceResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Face {
    pub id: String,
    /// Whose face it is; `None` for a face nobody has been given (or whose person was deleted).
    pub person: Option<Person>,
    #[serde(rename = "boundingBoxX1")]
    pub left: i32,
    #[serde(rename = "boundingBoxY1")]
    pub top: i32,
    #[serde(rename = "boundingBoxX2")]
    pub right: i32,
    #[serde(rename = "boundingBoxY2")]
    pub bottom: i32,
    /// The size of the picture the box is measured on.
    pub image_width: u32,
    pub image_height: u32,
    /// Not sent by every version.
    pub source_type: Option<FaceSource>,
}

/// A face drawn by hand: a box on an Asset, given to a person.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFace {
    pub asset_id: String,
    pub person_id: String,
    /// The box's left edge, in pixels of a picture `image_width` wide.
    pub x: u32,
    /// The box's top edge, in pixels of a picture `image_height` high.
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub image_width: u32,
    pub image_height: u32,
}

impl Client {
    /// The faces on an Asset (`GET /faces?id=<assetId>`). Needs `face.read`.
    pub async fn faces(&self, asset_id: &str) -> Result<Vec<Face>, ApiError> {
        if asset_id.trim().is_empty() {
            return Err(ApiError::Protocol("internal: faces of an Asset without an id".to_owned()));
        }
        self.call(Method::GET, &["faces"], &[("id", asset_id)]).await
    }

    /// Draw a face on an Asset and give it to a person (`POST /faces`). Needs `face.create`.
    pub async fn create_face(&self, face: &NewFace) -> Result<(), ApiError> {
        self.call_unit_with(Method::POST, &["faces"], &[], face).await
    }

    /// Give one face to another person (`PUT /faces/{id}`, the path naming the person); answers that person.
    /// Needs `face.update`.
    pub async fn reassign_face(&self, face_id: &str, person_id: &str) -> Result<Person, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            id: &'a str,
        }
        if face_id.trim().is_empty() {
            return Err(ApiError::Protocol("internal: reassigning a face without an id".to_owned()));
        }
        self.call_with(Method::PUT, &["faces", person_id], &[], &Body { id: face_id }).await
    }

    /// Delete a face: marked deleted, or with `force` removed for good (`DELETE /faces/{id}`). Needs
    /// `face.delete`.
    pub async fn delete_face(&self, face_id: &str, force: bool) -> Result<(), ApiError> {
        #[derive(Serialize)]
        struct Body {
            force: bool,
        }
        self.call_unit_with(Method::DELETE, &["faces", face_id], &[], &Body { force }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn a_face_without_a_person_decodes() {
        let json = r#"[{"id":"f1","boundingBoxX1":10,"boundingBoxY1":20,"boundingBoxX2":60,"boundingBoxY2":80,
            "imageWidth":2448,"imageHeight":3264,"sourceType":"manual","person":null},
            {"id":"f2","sourceType":"something-new","person":{"id":"p1","name":"Ada","isHidden":false}}]"#;
        let faces: Vec<Face> = crate::client::decode_json(json.as_bytes(), "faces").unwrap();
        assert_eq!(faces[0].person, None);
        assert_eq!((faces[0].left, faces[0].top, faces[0].right, faces[0].bottom), (10, 20, 60, 80));
        assert_eq!(faces[0].source_type, Some(FaceSource::Manual));
        assert_eq!(faces[1].person.as_ref().map(|person| person.name.as_str()), Some("Ada"));
        assert_eq!(faces[1].source_type, Some(FaceSource::Other("something-new".to_owned())));
    }

    #[tokio::test]
    async fn reassigning_a_face_puts_the_person_in_the_path() {
        let answer = r#"{"id":"person-2","name":"","isHidden":false}"#;
        let server = FakeServer::start(vec![CannedResponse::json(200, answer)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let person = client.reassign_face("face-1", "person-2").await.unwrap();
        assert_eq!(person.id, "person-2");
        let request = &server.requests()[0];
        assert_eq!((request.method.as_str(), request.target.as_str()), ("PUT", "/api/faces/person-2"));
        assert_eq!(request.body_text(), r#"{"id":"face-1"}"#);
        assert!(matches!(client.reassign_face("", "person-2").await, Err(ApiError::Protocol(_))));
        assert!(matches!(client.faces(" ").await, Err(ApiError::Protocol(_))));
    }
}

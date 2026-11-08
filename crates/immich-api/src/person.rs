//! People: every person Immich knows, hidden and unnamed ones included; reading, naming, merging and deleting them,
//! and moving faces between them. A face itself is [`crate::face`].
//!
//! `GET /people` pages (`page`, `size` up to 1000, `hasNextPage`) and leaves hidden people out unless asked, so
//! [`Client::people`] asks with `withHidden=true` and pages to the end. Which people the mount shows (named, not
//! hidden, with enough Assets) is the caller's decision; this returns the raw list.
//!
//! v3.3.5 sends `updatedAt` but no `createdAt` for a person; `created_at` is kept for servers that send it.
//!
//! Quirks, seen on v3.3.6:
//! - people come from face recognition, which needs machine learning; a person made by hand
//!   ([`Client::create_person`]) has no face and no thumbnail (`GET /people/{id}/thumbnail` answers 404) until a
//!   face is given to them ([`Client::create_face`] or a reassignment)
//! - `PUT /people/{id}` is marked deprecated with no replacement yet; it is the only single update on v3.2
//! - merging answers one outcome per id **plus one for the person merged into**, first; an unknown id fails with
//!   `no_permission`, and merging a person into themselves is a 400 for the whole call. `POST /people/{id}/merge` is
//!   used because v3.2.0 has no `POST /people/merge`
//! - on v3.2.0 merging people made by hand answers `[]` and merges nothing: that version looks the merged people up
//!   by their person group, which such people do not match (seen on the lab; v3.3.6 merges them)
//! - `PUT /people/{id}/reassign` moves the face of one person on one Asset (each `(person, Asset)` pair) to the
//!   person in the path, and answers with that person
//! - a bulk update answers `unknown`, not `not_found`, for an id that does not exist
//! - `GET /search/person` matches the start of any word of a name, case-insensitively (`lab` and `Person` both find
//!   "Lab Person")

use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::asset::non_blank;
use crate::client::{Client, Media};
use crate::dates::{optional_utc, parse_date};
use crate::error::ApiError;
use crate::wire::{endpoints, BulkIdResult, BulkIds, Endpoint};

/// People per request; the server's maximum.
const PEOPLE_PAGE_SIZE: &str = "1000";

/// A guard against a server that always says there is another page.
const MAX_PEOPLE_PAGES: u32 = 1000;

/// A person, as recognised by face detection or created by hand.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "PersonWire")]
pub struct Person {
    pub id: String,
    /// Empty for a person nobody has named.
    pub name: String,
    pub is_hidden: bool,
    pub is_favorite: bool,
    pub birth_date: Option<NaiveDate>,
    /// The face crop's path on the server; fetch the picture with [`Client::person_thumbnail`].
    pub thumbnail_path: String,
    /// A colour chosen for the person, as CSS text (`#d3a4f7`), when one was.
    pub color: Option<String>,
    /// Not sent by v3.3.5.
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct PeoplePageWire {
    people: Vec<PersonWire>,
    has_next_page: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct PersonWire {
    id: Option<String>,
    name: Option<String>,
    is_hidden: Option<bool>,
    is_favorite: Option<bool>,
    birth_date: Option<String>,
    thumbnail_path: Option<String>,
    color: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

impl TryFrom<PersonWire> for Person {
    type Error = ApiError;

    fn try_from(wire: PersonWire) -> Result<Person, ApiError> {
        wire.into_person()
    }
}

impl PersonWire {
    fn into_person(self) -> Result<Person, ApiError> {
        let id = non_blank(self.id).ok_or_else(|| ApiError::Protocol("a person without an id".to_owned()))?;
        Ok(Person {
            id,
            name: self.name.unwrap_or_default(),
            is_hidden: self.is_hidden.unwrap_or(false),
            is_favorite: self.is_favorite.unwrap_or(false),
            birth_date: self.birth_date.as_deref().and_then(parse_date),
            thumbnail_path: self.thumbnail_path.unwrap_or_default(),
            color: non_blank(self.color),
            created_at: optional_utc(self.created_at.as_deref()),
            updated_at: optional_utc(self.updated_at.as_deref()),
        })
    }
}

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/people" "person.read",
    GET "/people/{id}" "person.read",
    POST "/people" "person.create",
    PUT "/people/{id}" "person.update",
    PUT "/people" "person.update",
    DELETE "/people/{id}" "person.delete",
    DELETE "/people" "person.delete",
    POST "/people/{id}/merge" "person.merge",
    PUT "/people/{id}/reassign" "person.reassign",
    GET "/people/{id}/statistics" "person.statistics",
    GET "/people/{id}/thumbnail" "person.read",
    GET "/search/person" "person.read",
];

/// A person made by hand; every field may be left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPerson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(serialize_with = "crate::dates::serde_dates::send_date", skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    /// CSS colour text, such as `#d3a4f7`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Changes to a person; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonChanges {
    /// An empty name makes the person unnamed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `Some(None)` removes the birth date.
    #[serde(serialize_with = "crate::dates::serde_dates::send_clearable_date", skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Option<NaiveDate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_favorite: Option<bool>,
    /// The Asset whose face of this person becomes their thumbnail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_face_asset_id: Option<String>,
    /// `Some(None)` removes the colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Option<String>>,
}

/// One face to move, named by whose face it is now and the Asset it is on ([`Client::reassign_faces`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonOnAsset {
    pub person_id: String,
    pub asset_id: String,
}

impl Client {
    /// Every person, hidden ones included (`GET /people?withHidden=true`, all pages).
    /// Needs `person.read`.
    pub async fn people(&self) -> Result<Vec<Person>, ApiError> {
        let mut people = Vec::new();
        for page_number in 1..=MAX_PEOPLE_PAGES {
            let page_text = page_number.to_string();
            let query = [("withHidden", "true"), ("size", PEOPLE_PAGE_SIZE), ("page", page_text.as_str())];
            let page: PeoplePageWire = self.get_json(&["people"], &query).await?;
            let page_was_empty = page.people.is_empty();
            for person in page.people {
                people.push(person.into_person()?);
            }
            if page.has_next_page != Some(true) || page_was_empty {
                return Ok(people);
            }
        }
        Err(ApiError::Protocol(format!(
            "the people list did not end after {MAX_PEOPLE_PAGES} pages"
        )))
    }

    /// One person (`GET /people/{id}`). Needs `person.read`.
    pub async fn person(&self, id: &str) -> Result<Person, ApiError> {
        self.call(Method::GET, &["people", id], &[]).await
    }

    /// The people whose name has a word starting with `name` (`GET /search/person`), hidden ones only when asked.
    /// Needs `person.read`.
    pub async fn search_people(&self, name: &str, with_hidden: bool) -> Result<Vec<Person>, ApiError> {
        let with_hidden = if with_hidden { "true" } else { "false" };
        self.call(Method::GET, &["search", "person"], &[("name", name), ("withHidden", with_hidden)])
            .await
    }

    /// Make a person by hand (`POST /people`); they have no face until one is given to them. Needs
    /// `person.create`.
    pub async fn create_person(&self, person: &NewPerson) -> Result<Person, ApiError> {
        self.call_with(Method::POST, &["people"], &[], person).await
    }

    /// Name, date, hide, favourite, recolour a person or choose their thumbnail (`PUT /people/{id}`). Needs
    /// `person.update`.
    pub async fn update_person(&self, id: &str, changes: &PersonChanges) -> Result<Person, ApiError> {
        self.call_with(Method::PUT, &["people", id], &[], changes).await
    }

    /// Change several people at once, one outcome per person (`PUT /people`). Needs `person.update`.
    pub async fn update_people(&self, changes: &[(String, PersonChanges)]) -> Result<Vec<BulkIdResult>, ApiError> {
        #[derive(Serialize)]
        struct Item<'a> {
            id: &'a str,
            #[serde(flatten)]
            changes: &'a PersonChanges,
        }
        #[derive(Serialize)]
        struct Body<'a> {
            people: Vec<Item<'a>>,
        }
        let people = changes.iter().map(|(id, changes)| Item { id, changes }).collect();
        self.call_with(Method::PUT, &["people"], &[], &Body { people }).await
    }

    /// Delete a person; their faces stay on the Assets, unassigned (`DELETE /people/{id}`). Needs `person.delete`.
    pub async fn delete_person(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["people", id], &[]).await
    }

    /// Delete several people (`DELETE /people`). Needs `person.delete`.
    pub async fn delete_people(&self, ids: &[String]) -> Result<(), ApiError> {
        self.call_unit_with(Method::DELETE, &["people"], &[], &BulkIds { ids }).await
    }

    /// Merge people into the person `id`: their faces become theirs and they are deleted
    /// (`POST /people/{id}/merge`). The answer has one outcome per id, after one for `id` itself. Needs
    /// `person.merge`.
    pub async fn merge_people(&self, id: &str, merged_ids: &[String]) -> Result<Vec<BulkIdResult>, ApiError> {
        self.call_with(Method::POST, &["people", id, "merge"], &[], &BulkIds { ids: merged_ids })
            .await
    }

    /// Give faces to the person `id`, each named by its current person and its Asset
    /// (`PUT /people/{id}/reassign`). Answers the people the faces went to. Needs `person.reassign`.
    pub async fn reassign_faces(&self, id: &str, faces: &[PersonOnAsset]) -> Result<Vec<Person>, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            data: &'a [PersonOnAsset],
        }
        self.call_with(Method::PUT, &["people", id, "reassign"], &[], &Body { data: faces })
            .await
    }

    /// How many Assets show a person (`GET /people/{id}/statistics`). Needs `person.statistics`.
    pub async fn person_statistics(&self, id: &str) -> Result<u64, ApiError> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Statistics {
            assets: u64,
        }
        let answer: Statistics = self.call(Method::GET, &["people", id, "statistics"], &[]).await?;
        Ok(answer.assets)
    }

    /// A person's face crop, a JPEG (`GET /people/{id}/thumbnail`); [`ApiError::NotFound`] for a person without a
    /// face. Needs `person.read`.
    pub async fn person_thumbnail(&self, id: &str) -> Result<Media, ApiError> {
        self.get_media(&["people", id, "thumbnail"], &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::parse_utc;
    use crate::fixtures::PEOPLE;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn people_decode() {
        let page: PeoplePageWire = serde_json::from_str(PEOPLE).unwrap();
        assert_eq!(page.has_next_page, Some(false));
        let person = page.people.into_iter().next().unwrap().into_person().unwrap();
        assert_eq!(
            person,
            Person {
                id: "1ac08f61-5c8a-4361-8139-a5dc1860137a".to_owned(),
                name: "Lab Person".to_owned(),
                is_hidden: false,
                is_favorite: false,
                birth_date: None,
                thumbnail_path: person.thumbnail_path.clone(),
                color: person.color.clone(),
                created_at: None,
                updated_at: parse_utc("2026-11-01T21:02:04.237574+00:00"),
            }
        );
    }

    #[test]
    fn person_changes_send_only_what_changes_and_can_clear() {
        let changes = PersonChanges {
            name: Some("Ada".to_owned()),
            birth_date: Some(None),
            color: Some(None),
            ..PersonChanges::default()
        };
        assert_eq!(
            serde_json::to_value(&changes).unwrap(),
            serde_json::json!({"name": "Ada", "birthDate": null, "color": null})
        );
        let person = NewPerson {
            birth_date: NaiveDate::from_ymd_opt(1990, 5, 17),
            is_favorite: Some(true),
            ..NewPerson::default()
        };
        assert_eq!(
            serde_json::to_value(&person).unwrap(),
            serde_json::json!({"birthDate": "1990-05-17", "isFavorite": true})
        );
    }

    #[tokio::test]
    async fn a_bulk_update_puts_each_id_beside_its_changes() {
        let server = FakeServer::start(vec![CannedResponse::json(200, r#"[{"id":"p1","success":true}]"#)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let changes = PersonChanges { is_hidden: Some(true), ..PersonChanges::default() };
        let results = client.update_people(&[("p1".to_owned(), changes)]).await.unwrap();
        assert!(results[0].success);
        let request = &server.requests()[0];
        assert_eq!((request.method.as_str(), request.target.as_str()), ("PUT", "/api/people"));
        assert_eq!(request.body_text(), r#"{"people":[{"id":"p1","isHidden":true}]}"#);
    }

    #[tokio::test]
    async fn a_person_thumbnail_comes_with_its_content_type() {
        let server = FakeServer::start(vec![CannedResponse::bytes(200, "image/jpeg", b"\xff\xd8")]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let media = client.person_thumbnail("p1").await.unwrap();
        assert_eq!(media.content_type, "image/jpeg");
        assert_eq!(&media.bytes[..], b"\xff\xd8");
        assert_eq!(server.requests()[0].target, "/api/people/p1/thumbnail");
        assert!(matches!(client.person_thumbnail("").await, Err(ApiError::Protocol(_))));
    }

    #[tokio::test]
    async fn people_pages_until_has_next_page_is_false() {
        let first = r#"{"people":[{"id":"p1","name":"","isHidden":true}],"hasNextPage":true,"total":2,"hidden":1}"#;
        let second = r#"{"people":[{"id":"p2","name":"Ada"}],"hasNextPage":false,"total":2,"hidden":1}"#;
        let server = FakeServer::start(vec![CannedResponse::json(200, first), CannedResponse::json(200, second)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let people = client.people().await.unwrap();
        assert_eq!(people.len(), 2);
        assert!(people[0].is_hidden);
        assert_eq!(people[1].name, "Ada");
        let targets: Vec<String> = server.requests().into_iter().map(|request| request.target).collect();
        assert_eq!(
            targets,
            [
                "/api/people?withHidden=true&size=1000&page=1",
                "/api/people?withHidden=true&size=1000&page=2"
            ]
        );
    }
}

//! People: every person Immich knows, hidden and unnamed ones included.
//!
//! `GET /people` pages (`page`, `size` up to 1000, `hasNextPage`) and leaves hidden people out unless asked, so
//! this asks with `withHidden=true` and pages to the end. Which people the mount shows (named, not hidden, with
//! enough Assets) is the caller's decision; this returns the raw list.
//!
//! v3.3.5 sends `updatedAt` but no `createdAt` for a person; `created_at` is kept for servers that send it.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;

use crate::asset::non_blank;
use crate::client::Client;
use crate::dates::optional_utc;
use crate::error::ApiError;

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
            birth_date: self
                .birth_date
                .as_deref()
                .and_then(|text| NaiveDate::parse_from_str(text.get(..10).unwrap_or(text), "%Y-%m-%d").ok()),
            thumbnail_path: self.thumbnail_path.unwrap_or_default(),
            color: non_blank(self.color),
            created_at: optional_utc(self.created_at.as_deref()),
            updated_at: optional_utc(self.updated_at.as_deref()),
        })
    }
}

pub(crate) const ENDPOINTS: &[crate::wire::Endpoint] = crate::wire::endpoints![
    GET "/people" "person.read",
];

impl Client {
    /// Every person, hidden ones included (`GET /people?withHidden=true`, all pages).
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

//! immich-crosslink's own test links (`tests/links.json` in jakobhviid/immich-crosslink), copied as
//! `crosslink-links.json`: the links Immich Mount must make byte for byte, and the result it must reach reading each.

use immich_mount_core::links::{crosslink, read_crosslink, Crosslink, Reading};
use immich_mount_core::model::YearMonth;
use immich_mount_core::tree::{NodeKey, SectionKey};
use serde_json::Value;

/// The protocol's commit the copy was taken at, and its draft.
const PROTOCOL_COMMIT: &str = "8a3c16e";
const DRAFT: &str = "0.2";

fn test_links() -> Value {
    serde_json::from_str(include_str!("crosslink-links.json")).expect("the test links are JSON")
}

#[test]
fn the_copy_is_of_draft_0_2_at_8a3c16e() {
    assert_eq!(test_links()["draft"], DRAFT, "copied at immich-crosslink {PROTOCOL_COMMIT}");
}

/// The key the mount gives the item a `make` entry describes. A photo of the timeline sits in some month folder; the
/// link does not depend on which.
fn key_of(item: &Value) -> NodeKey {
    let text = |name: &str| item[name].as_str().map(str::to_owned);
    let place = || match (text("country"), text("city")) {
        (Some(country), Some(city)) => Some(NodeKey::City { country, city }),
        (Some(country), None) => Some(NodeKey::Country(country)),
        _ => None,
    };
    let folder = text("album")
        .map(NodeKey::Album)
        .or_else(|| text("person").map(NodeKey::Person))
        .or_else(place);
    match item["kind"].as_str().expect("a kind") {
        "asset" => NodeKey::Asset {
            parent: Box::new(folder.unwrap_or(NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 }))),
            asset_id: text("asset").expect("an asset id"),
        },
        "page" => NodeKey::Section(match item["page"].as_str().expect("a page") {
            "photos" => SectionKey::Photos,
            "albums" => SectionKey::Albums,
            "people" => SectionKey::People,
            "places" => SectionKey::Places,
            "favorites" => SectionKey::Favorites,
            "archive" => SectionKey::Archive,
            other => panic!("unknown page {other}"),
        }),
        _ => folder.expect("an album, person or place"),
    }
}

#[test]
fn links_are_made_byte_for_byte() {
    for entry in test_links()["make"].as_array().expect("make entries") {
        let node = key_of(&entry["item"]);
        let made = crosslink(entry["server"].as_str().unwrap(), entry["user"].as_str(), &node);
        assert_eq!(made.as_deref(), entry["link"].as_str(), "{}", entry["name"]);
        assert_eq!(immich_mount_core::links::immich_path(&node).as_deref(), entry["path"].as_str(), "{}", entry["name"]);
        assert_eq!(immich_mount_core::links::kind(&node).map(|kind| kind.as_str()), entry["item"]["kind"].as_str(), "{}", entry["name"]);
    }
}

#[test]
fn links_are_read_to_the_given_result() {
    for entry in test_links()["read"].as_array().expect("read entries") {
        let name = entry["name"].as_str().unwrap();
        let reading = read_crosslink(entry["link"].as_str().unwrap());
        match entry["result"].as_str().unwrap() {
            "show" => {
                let Reading::Show(Crosslink { server, user_id, item }) = reading else { panic!("{name}: {reading:?}") };
                assert_eq!(item.kind().as_str(), entry["kind"], "{name}");
                assert_eq!(item.immich_path(), entry["path"].as_str().unwrap(), "{name}");
                assert_eq!(server, entry["server"].as_str().unwrap(), "{name}");
                assert_eq!(user_id.as_deref(), entry["user"].as_str(), "{name}");
            }
            "cannot-show" => assert_eq!(reading, Reading::CannotShow, "{name}"),
            "invalid" => assert_eq!(reading, Reading::Invalid, "{name}"),
            other => panic!("{name}: unknown result {other}"),
        }
    }
}

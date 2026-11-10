//! Links to what the mount shows: each key's Immich path, the path Immich's own web app uses for the item, and from it
//! the item's web address and its crosslink (ROADMAP.md, "Open in Immich, and links between apps"; the protocol is
//! immich-crosslink, draft 0.1). Every platform asks here, so one item gives the same link everywhere.

use crate::tree::{NodeKey, SectionKey};

/// The scheme of immich-crosslink, which apps that take part register.
pub const CROSSLINK_SCHEME: &str = "immich-crosslink";

/// The Immich path of `node`: the path of Immich's web app for it, with a search's query where it has one. `None`
/// where Immich has no page for the item: the root, the timeline's year, month and day folders, the year folders above
/// albums, and Upload with everything in it.
pub fn immich_path(node: &NodeKey) -> Option<String> {
    match node {
        NodeKey::Asset { parent, asset_id } => {
            let asset = encode_id(asset_id);
            match folder_base(parent)? {
                Base::Timeline => Some(format!("/photos/{asset}")),
                Base::Folder(path) => Some(format!("{path}/photos/{asset}")),
                Base::Search(query) => Some(format!("/search/photos/{asset}?query={query}")),
            }
        }
        NodeKey::Section(section) => Some(section_path(*section).to_owned()),
        NodeKey::Country(country) => Some(format!("/search?query={}", encode_component(&country_search(country)))),
        NodeKey::City { .. } | NodeKey::Album(_) | NodeKey::Person(_) | NodeKey::GroupYear { .. } | NodeKey::GroupMonth { .. } => {
            match folder_base(node)? {
                Base::Timeline => None,
                Base::Folder(path) => Some(path),
                Base::Search(query) => Some(format!("/search?query={query}")),
            }
        }
        NodeKey::Root
        | NodeKey::Upload
        | NodeKey::UploadFolder(_)
        | NodeKey::TimelineYear(_)
        | NodeKey::TimelineMonth(_)
        | NodeKey::TimelineDay(..)
        | NodeKey::AlbumYear(_) => None,
    }
}

/// The server's address as links name it: `server.url` without a trailing slash and without `/api`.
pub fn server_address(server_url: &str) -> String {
    let trimmed = server_url.trim().trim_end_matches('/');
    trimmed.strip_suffix("/api").unwrap_or(trimmed).trim_end_matches('/').to_owned()
}

/// The item's address in Immich's web app; `None` where it has no Immich path.
pub fn web_address(server_url: &str, node: &NodeKey) -> Option<String> {
    Some(format!("{}{}", server_address(server_url), immich_path(node)?))
}

/// The item's crosslink: `immich-crosslink://` with an empty host, the Immich path, the server's address and, when
/// known, the signed-in user's Immich id. `None` where it has no Immich path.
pub fn crosslink(server_url: &str, user_id: Option<&str>, node: &NodeKey) -> Option<String> {
    let path = immich_path(node)?;
    let joiner = if path.contains('?') { '&' } else { '?' };
    let mut link = format!("{CROSSLINK_SCHEME}://{path}{joiner}server={}", encode_component(&server_address(server_url)));
    if let Some(user_id) = user_id.filter(|user_id| !user_id.is_empty()) {
        link.push_str("&user=");
        link.push_str(&encode_id(user_id));
    }
    Some(link)
}

/// What sort of item a link names, as immich-crosslink calls it: other apps announce the kinds they show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Asset,
    Album,
    Person,
    Place,
    Page,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Asset => "asset",
            Kind::Album => "album",
            Kind::Person => "person",
            Kind::Place => "place",
            Kind::Page => "page",
        }
    }
}

/// The kind of an item with an Immich path; a date folder takes its base's.
pub fn kind(node: &NodeKey) -> Option<Kind> {
    immich_path(node)?;
    Some(match node {
        NodeKey::Asset { .. } => Kind::Asset,
        NodeKey::Album(_) => Kind::Album,
        NodeKey::Person(_) => Kind::Person,
        NodeKey::City { .. } | NodeKey::Country(_) => Kind::Place,
        NodeKey::GroupYear { base, .. } | NodeKey::GroupMonth { base, .. } => return kind(base),
        _ => Kind::Page,
    })
}

/// What an Immich path names, read back from a link (step 2 of the roadmap's section). Names are Immich's own; ids are
/// UUIDs in lower case. A photo's folder in the mount also depends on its date, which the link does not carry, so the
/// tree finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImmichItem {
    Section(SectionKey),
    Album(String),
    Person(String),
    Country(String),
    /// Immich's Places page searches for a city by its name alone, so the country may be missing.
    City { country: Option<String>, city: String },
    /// A photo, and where the link saw it: `None` for the timeline (`/photos/<id>`).
    Asset { within: Option<Box<ImmichItem>>, asset_id: String },
}

impl ImmichItem {
    /// The key of a folder the item names, where the link says enough for one: not for a photo, which needs its
    /// date, nor for a city without its country.
    pub fn folder_key(&self) -> Option<NodeKey> {
        match self {
            ImmichItem::Section(section) => Some(NodeKey::Section(*section)),
            ImmichItem::Album(album_id) => Some(NodeKey::Album(album_id.clone())),
            ImmichItem::Person(person_id) => Some(NodeKey::Person(person_id.clone())),
            ImmichItem::Country(country) => Some(NodeKey::Country(country.clone())),
            ImmichItem::City { country: Some(country), city } => Some(NodeKey::City { country: country.clone(), city: city.clone() }),
            ImmichItem::City { country: None, .. } | ImmichItem::Asset { .. } => None,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            ImmichItem::Section(_) => Kind::Page,
            ImmichItem::Album(_) => Kind::Album,
            ImmichItem::Person(_) => Kind::Person,
            ImmichItem::Country(_) | ImmichItem::City { .. } => Kind::Place,
            ImmichItem::Asset { .. } => Kind::Asset,
        }
    }

    /// The item's Immich path, written as a sender writes it.
    pub fn immich_path(&self) -> String {
        let place_search = |item: &ImmichItem| match item {
            ImmichItem::City { country: Some(country), city } => Some(city_search(country, city)),
            ImmichItem::City { country: None, city } => Some(format!("{{\"city\":{}}}", json_string(city))),
            ImmichItem::Country(country) => Some(country_search(country)),
            _ => None,
        };
        match self {
            ImmichItem::Section(section) => section_path(*section).to_owned(),
            ImmichItem::Album(album_id) => format!("/albums/{}", encode_id(album_id)),
            ImmichItem::Person(person_id) => format!("/people/{}", encode_id(person_id)),
            ImmichItem::Country(_) | ImmichItem::City { .. } => {
                format!("/search?query={}", encode_component(&place_search(self).unwrap_or_default()))
            }
            ImmichItem::Asset { within, asset_id } => {
                let asset = encode_id(asset_id);
                match within.as_deref() {
                    None => format!("/photos/{asset}"),
                    Some(place @ (ImmichItem::Country(_) | ImmichItem::City { .. })) => {
                        format!("/search/photos/{asset}?query={}", encode_component(&place_search(place).unwrap_or_default()))
                    }
                    Some(folder) => format!("{}/photos/{asset}", folder.immich_path()),
                }
            }
        }
    }
}

/// A crosslink read back, as immich-crosslink's receivers must read it: before anything is fetched it ends in one of
/// three results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// A valid link with a path the protocol defines.
    Show(Crosslink),
    /// A valid link whose path the protocol does not define (a search by anything but a place included).
    CannotShow,
    /// Anything else: another scheme, a host, a missing or bad `server`, an id that is not a UUID, a malformed search.
    Invalid,
}

/// A valid crosslink: the server it names (normalised, `normalise_server`), the user whose view it was, and the item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crosslink {
    pub server: String,
    pub user_id: Option<String>,
    pub item: ImmichItem,
}

/// An Immich path read back.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PathReading {
    Item(ImmichItem),
    Undefined,
    Invalid,
}

/// Read an Immich path, matched exactly (no trailing slash), with a search's still encoded `query`.
fn read_immich_path(path: &str, query: Option<&str>) -> PathReading {
    use PathReading::{Invalid, Item, Undefined};
    let parts: Vec<&str> = path.split('/').collect();
    let asset = |within: Option<ImmichItem>, asset_id: &str| match read_id(asset_id) {
        Some(asset_id) => Item(ImmichItem::Asset { within: within.map(Box::new), asset_id }),
        None => Invalid,
    };
    let with_id = |id: &str, make: fn(String) -> ImmichItem| read_id(id).map_or(Invalid, |id| Item(make(id)));
    let with_place = |then: &dyn Fn(ImmichItem) -> PathReading| match query.map(read_place_search) {
        Some(Item(place)) => then(place),
        Some(other) => other,
        None => Undefined,
    };
    match parts.as_slice() {
        ["", "photos"] => Item(ImmichItem::Section(SectionKey::Photos)),
        ["", "albums"] => Item(ImmichItem::Section(SectionKey::Albums)),
        ["", "people"] => Item(ImmichItem::Section(SectionKey::People)),
        ["", "places"] => Item(ImmichItem::Section(SectionKey::Places)),
        ["", "favorites"] => Item(ImmichItem::Section(SectionKey::Favorites)),
        ["", "archive"] => Item(ImmichItem::Section(SectionKey::Archive)),
        ["", "photos", asset_id] => asset(None, asset_id),
        ["", "albums", album_id] => with_id(album_id, ImmichItem::Album),
        ["", "people", person_id] => with_id(person_id, ImmichItem::Person),
        ["", "albums", album_id, "photos", asset_id] => match read_id(album_id) {
            Some(album_id) => asset(Some(ImmichItem::Album(album_id)), asset_id),
            None => Invalid,
        },
        ["", "people", person_id, "photos", asset_id] => match read_id(person_id) {
            Some(person_id) => asset(Some(ImmichItem::Person(person_id)), asset_id),
            None => Invalid,
        },
        ["", "favorites", "photos", asset_id] => asset(Some(ImmichItem::Section(SectionKey::Favorites)), asset_id),
        ["", "archive", "photos", asset_id] => asset(Some(ImmichItem::Section(SectionKey::Archive)), asset_id),
        ["", "search"] => with_place(&Item),
        ["", "search", "photos", asset_id] => with_place(&|place| asset(Some(place), asset_id)),
        _ => Undefined,
    }
}

/// An id from a link: a UUID, in lower case; `None` for anything else.
fn read_id(encoded: &str) -> Option<String> {
    decode_percent(encoded).filter(|id| is_uuid(id)).map(|id| id.to_ascii_lowercase())
}

/// The place a search's encoded JSON names: a country, or a city with or without its country. Any other search
/// (words, people, dates) is valid but not a place; text that is not a JSON object of strings is malformed.
fn read_place_search(encoded: &str) -> PathReading {
    let Some(search) = decode_percent(encoded).and_then(|json| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&json).ok())
    else {
        return PathReading::Invalid;
    };
    if search.keys().any(|key| key != "city" && key != "country") {
        return PathReading::Undefined;
    }
    let text = |key: &str| match search.get(key) {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(()),
    };
    match (text("country"), text("city")) {
        (Ok(country), Ok(Some(city))) => PathReading::Item(ImmichItem::City { country, city }),
        (Ok(Some(country)), Ok(None)) => PathReading::Item(ImmichItem::Country(country)),
        (Ok(None), Ok(None)) => PathReading::Undefined,
        _ => PathReading::Invalid,
    }
}

/// Read a crosslink as the protocol's receivers must. Parameters it does not know are ignored.
pub fn read_crosslink(link: &str) -> Reading {
    let Some((scheme, rest)) = link.split_once(':') else { return Reading::Invalid };
    if !scheme.eq_ignore_ascii_case(CROSSLINK_SCHEME) {
        return Reading::Invalid;
    }
    let Some(rest) = rest.strip_prefix("///") else { return Reading::Invalid };
    let (path, query) = rest.split_once('?').map_or((rest, None), |(path, query)| (path, Some(query)));
    let parameters = query.map(query_parameters).unwrap_or_default();
    let parameter = |name: &str| parameters.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str());
    let Some(server) = parameter("server").and_then(decode_percent).and_then(|server| normalise_server(&server)) else {
        return Reading::Invalid;
    };
    let user_id = match parameter("user") {
        None => None,
        Some(user) => match read_id(user) {
            Some(user_id) => Some(user_id),
            None => return Reading::Invalid,
        },
    };
    match read_immich_path(&format!("/{path}"), parameter("query")) {
        PathReading::Item(item) => Reading::Show(Crosslink { server, user_id, item }),
        PathReading::Undefined => Reading::CannotShow,
        PathReading::Invalid => Reading::Invalid,
    }
}

/// The item a web address on the signed-in server names; `None` for another server, or a path the protocol does not
/// define or that is not valid.
pub fn read_web_address(address: &str, server_url: &str) -> Option<ImmichItem> {
    let (address, _fragment) = address.split_once('#').unwrap_or((address, ""));
    let (without_query, query) = address.split_once('?').map_or((address, None), |(path, query)| (path, Some(query)));
    let (origin, path) = split_origin(without_query)?;
    let server = server_address(server_url);
    let (server_origin, server_path) = split_origin(&server)?;
    if !same_server(origin, server_origin) {
        return None;
    }
    let path = path.strip_prefix(server_path)?;
    let parameters = query.map(query_parameters).unwrap_or_default();
    let search = parameters.iter().find(|(key, _)| key == "query").map(|(_, value)| value.as_str());
    match read_immich_path(path, search) {
        PathReading::Item(item) => Some(item),
        PathReading::Undefined | PathReading::Invalid => None,
    }
}

/// A server address as the protocol compares them: an absolute `http` or `https` URL, scheme and host in lower case,
/// a scheme's default port dropped, no trailing slash. `None` for anything else.
pub fn normalise_server(address: &str) -> Option<String> {
    let (scheme, rest) = address.trim().split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => "443",
        "http" => "80",
        _ => return None,
    };
    if rest.contains(['?', '#']) {
        return None;
    }
    let (authority, path) = rest.find('/').map_or((rest, ""), |index| rest.split_at(index));
    // A bracketed IPv6 host keeps its colons; the port follows the bracket.
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.contains(']') => (host, Some(port)),
        _ => (authority, None),
    };
    if host.is_empty() || host.contains('@') || port.is_some_and(|port| port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())) {
        return None;
    }
    let port = port.filter(|port| port.trim_start_matches('0') != default_port).map(|port| format!(":{port}")).unwrap_or_default();
    Some(format!("{scheme}://{}{port}{}", host.to_ascii_lowercase(), path.trim_end_matches('/')))
}

/// Whether two server addresses name the same server, as `normalise_server` compares them.
pub fn same_server(first: &str, second: &str) -> bool {
    matches!((normalise_server(first), normalise_server(second)), (Some(first), Some(second)) if first == second)
}

/// `scheme://authority` and the path after it, of an absolute address.
fn split_origin(address: &str) -> Option<(&str, &str)> {
    let scheme_end = address.find("://")? + 3;
    let path_start = address[scheme_end..].find('/').map_or(address.len(), |index| scheme_end + index);
    Some((&address[..path_start], &address[path_start..]))
}

/// A query's `name=value` pairs, the values left encoded.
fn query_parameters(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (name.to_owned(), value.to_owned())
        })
        .collect()
}

/// Whether `text` is a UUID: 8-4-4-4-12 hexadecimal digits, in either case.
fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, length)| group.len() == length && group.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// An id as a link writes it: in lower case, as Immich does.
fn encode_id(id: &str) -> String {
    encode_component(&id.to_ascii_lowercase())
}

/// What a folder's photos are shown in by Immich: the timeline, a page of its own, or a search.
enum Base {
    Timeline,
    /// A page whose photos are at `<path>/photos/<id>`.
    Folder(String),
    /// A metadata search, its JSON already encoded.
    Search(String),
}

/// The Immich view of a folder that lists photos, its year and month groups taking their base's; `None` for a folder
/// without one.
fn folder_base(folder: &NodeKey) -> Option<Base> {
    match folder {
        NodeKey::TimelineMonth(_) | NodeKey::TimelineDay(..) => Some(Base::Timeline),
        NodeKey::Album(album_id) => Some(Base::Folder(format!("/albums/{}", encode_id(album_id)))),
        NodeKey::Person(person_id) => Some(Base::Folder(format!("/people/{}", encode_id(person_id)))),
        NodeKey::Section(section @ (SectionKey::Favorites | SectionKey::Archive)) => Some(Base::Folder(section_path(*section).to_owned())),
        NodeKey::City { country, city } => Some(Base::Search(encode_component(&city_search(country, city)))),
        NodeKey::GroupYear { base, .. } | NodeKey::GroupMonth { base, .. } => folder_base(base),
        _ => None,
    }
}

fn section_path(section: SectionKey) -> &'static str {
    match section {
        SectionKey::Photos => "/photos",
        SectionKey::Albums => "/albums",
        SectionKey::People => "/people",
        SectionKey::Places => "/places",
        SectionKey::Favorites => "/favorites",
        SectionKey::Archive => "/archive",
    }
}

/// The metadata search Immich's web app writes for a city (`Route.search`, `JSON.stringify`): keys in this order, no
/// spaces. The country is added to Immich's own `{"city":…}`, so two cities of one name stay apart.
fn city_search(country: &str, city: &str) -> String {
    format!("{{\"city\":{},\"country\":{}}}", json_string(city), json_string(country))
}

fn country_search(country: &str) -> String {
    format!("{{\"country\":{}}}", json_string(country))
}

/// A string as `JSON.stringify` writes it, which serde_json matches: only `"`, `\` and control characters escaped.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string always serialises")
}

/// `text` as JavaScript's `encodeURIComponent` encodes it: every byte but `A-Z a-z 0-9 - _ . ! ~ * ' ( )` as `%XX` of
/// its UTF-8.
pub fn encode_component(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// `text` with every `%XX` decoded; `None` when an escape is malformed or the bytes are not UTF-8.
pub fn decode_percent(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            // `from_str_radix` takes a sign as well, so the two digits are checked first.
            let hex = bytes.get(index + 1..index + 3).filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))?;
            decoded.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::YearMonth;

    const ASSET: &str = "9a1e5c3e-0b1d-4c8e-9f6a-2d7b8c9e0f1a";
    const ALBUM: &str = "0f8c2b4a-7d6e-4f1a-8b9c-3e2d1a0b9c8d";
    const PERSON: &str = "5b7d9f1e-3a2c-4e6b-8d0f-1c3e5a7b9d2f";
    const SERVER: &str = "https://photos.example.com";

    fn asset_in(parent: NodeKey) -> NodeKey {
        NodeKey::Asset { parent: Box::new(parent), asset_id: ASSET.to_owned() }
    }

    fn month() -> YearMonth {
        YearMonth { year: 2024, month: 3 }
    }

    fn city() -> NodeKey {
        NodeKey::City { country: "Denmark".to_owned(), city: "Aarhus".to_owned() }
    }

    fn grouped(base: NodeKey) -> [NodeKey; 2] {
        [
            NodeKey::GroupYear { base: Box::new(base.clone()), year: 2024 },
            NodeKey::GroupMonth { base: Box::new(base), month: month() },
        ]
    }

    const AARHUS: &str = "%7B%22city%22%3A%22Aarhus%22%2C%22country%22%3A%22Denmark%22%7D";

    #[test]
    fn a_photo_in_the_timeline_is_its_photo_page() {
        for parent in [NodeKey::TimelineMonth(month()), NodeKey::TimelineDay(month(), 15)] {
            assert_eq!(immich_path(&asset_in(parent)).as_deref(), Some(format!("/photos/{ASSET}").as_str()));
        }
    }

    #[test]
    fn a_photo_keeps_the_album_person_or_section_it_is_listed_in() {
        let cases = [
            (NodeKey::Album(ALBUM.to_owned()), format!("/albums/{ALBUM}/photos/{ASSET}")),
            (NodeKey::Person(PERSON.to_owned()), format!("/people/{PERSON}/photos/{ASSET}")),
            (NodeKey::Section(SectionKey::Favorites), format!("/favorites/photos/{ASSET}")),
            (NodeKey::Section(SectionKey::Archive), format!("/archive/photos/{ASSET}")),
            (city(), format!("/search/photos/{ASSET}?query={AARHUS}")),
        ];
        for (base, expected) in cases {
            assert_eq!(immich_path(&asset_in(base.clone())).as_deref(), Some(expected.as_str()), "{base:?}");
            for group in grouped(base) {
                assert_eq!(immich_path(&asset_in(group.clone())).as_deref(), Some(expected.as_str()), "{group:?}");
            }
        }
    }

    #[test]
    fn folders_with_a_page_link_to_it_and_their_date_groups_too() {
        let cases = [
            (NodeKey::Album(ALBUM.to_owned()), format!("/albums/{ALBUM}")),
            (NodeKey::Person(PERSON.to_owned()), format!("/people/{PERSON}")),
            (NodeKey::Section(SectionKey::Favorites), "/favorites".to_owned()),
            (NodeKey::Section(SectionKey::Archive), "/archive".to_owned()),
            (city(), format!("/search?query={AARHUS}")),
        ];
        for (folder, expected) in cases {
            assert_eq!(immich_path(&folder).as_deref(), Some(expected.as_str()), "{folder:?}");
            for group in grouped(folder) {
                assert_eq!(immich_path(&group).as_deref(), Some(expected.as_str()), "{group:?}");
            }
        }
    }

    #[test]
    fn a_country_is_a_search_for_it() {
        assert_eq!(
            immich_path(&NodeKey::Country("Denmark".to_owned())).as_deref(),
            Some("/search?query=%7B%22country%22%3A%22Denmark%22%7D")
        );
    }

    #[test]
    fn sections_are_their_pages() {
        let cases = [
            (SectionKey::Photos, "/photos"),
            (SectionKey::Albums, "/albums"),
            (SectionKey::People, "/people"),
            (SectionKey::Places, "/places"),
            (SectionKey::Favorites, "/favorites"),
            (SectionKey::Archive, "/archive"),
        ];
        for (section, expected) in cases {
            assert_eq!(immich_path(&NodeKey::Section(section)).as_deref(), Some(expected));
        }
    }

    #[test]
    fn what_immich_has_no_page_for_has_no_path() {
        let none = [
            NodeKey::Root,
            NodeKey::Upload,
            NodeKey::UploadFolder(vec!["Trip".to_owned()]),
            NodeKey::TimelineYear(2024),
            NodeKey::TimelineMonth(month()),
            NodeKey::TimelineDay(month(), 15),
            NodeKey::AlbumYear(Some(2024)),
            NodeKey::AlbumYear(None),
            asset_in(NodeKey::Upload),
            asset_in(NodeKey::UploadFolder(vec!["Trip".to_owned()])),
            asset_in(NodeKey::Root),
        ];
        for node in none {
            assert_eq!(immich_path(&node), None, "{node:?}");
            assert_eq!(web_address(SERVER, &node), None);
            assert_eq!(crosslink(SERVER, None, &node), None);
        }
    }

    #[test]
    fn the_search_is_the_json_immich_writes_byte_for_byte() {
        // What `encodeURIComponent(JSON.stringify({city, country}))` gives in a browser for these names.
        assert_eq!(
            encode_component(&city_search("Danmark", "Århus")),
            "%7B%22city%22%3A%22%C3%85rhus%22%2C%22country%22%3A%22Danmark%22%7D"
        );
        assert_eq!(
            encode_component(&city_search("United States of America", "St. Paul's \"Twin\" City")),
            "%7B%22city%22%3A%22St.%20Paul's%20%5C%22Twin%5C%22%20City%22%2C%22country%22%3A%22United%20States%20of%20America%22%7D"
        );
        assert_eq!(city_search("日本", "東京"), "{\"city\":\"東京\",\"country\":\"日本\"}");
        assert_eq!(country_search("A\\B\u{1}"), "{\"country\":\"A\\\\B\\u0001\"}");
    }

    #[test]
    fn names_with_the_key_separator_and_percent_signs_are_encoded() {
        let odd = NodeKey::City { country: "A|B".to_owned(), city: "100% Town".to_owned() };
        assert_eq!(
            immich_path(&odd).as_deref(),
            Some("/search?query=%7B%22city%22%3A%22100%25%20Town%22%2C%22country%22%3A%22A%7CB%22%7D")
        );
        // Through the key's storage form and back, as the Mac's item identifiers carry it.
        let round_trip = NodeKey::from_storage_key(&asset_in(odd.clone()).storage_key()).unwrap();
        assert_eq!(immich_path(&round_trip), immich_path(&asset_in(odd)));
    }

    #[test]
    fn encoding_keeps_only_what_encode_uri_component_keeps() {
        assert_eq!(encode_component("AZaz09-_.!~*'()"), "AZaz09-_.!~*'()");
        assert_eq!(encode_component(" /?#&=+:,;@$%|"), "%20%2F%3F%23%26%3D%2B%3A%2C%3B%40%24%25%7C");
        assert_eq!(encode_component("é😀"), "%C3%A9%F0%9F%98%80");
    }

    #[test]
    fn decoding_undoes_encoding_and_refuses_what_is_malformed() {
        for text in ["2024-03-15 14.32.07.JPG", "Århus | 100%", "東京/😀"] {
            assert_eq!(decode_percent(&encode_component(text)).as_deref(), Some(text));
        }
        assert_eq!(decode_percent("/home/me/Immich/My%20Album").as_deref(), Some("/home/me/Immich/My Album"));
        assert_eq!(decode_percent("%2"), None);
        assert_eq!(decode_percent("%zz"), None);
        assert_eq!(decode_percent("%+1"), None, "a sign is not a hex digit");
        assert_eq!(decode_percent("%FF"), None, "not UTF-8");
    }

    #[test]
    fn the_server_address_drops_api_and_trailing_slashes() {
        for typed in ["https://photos.example.com", "https://photos.example.com/", "https://photos.example.com/api", "https://photos.example.com/api/"] {
            assert_eq!(server_address(typed), SERVER);
        }
        assert_eq!(server_address("http://192.168.1.5:2283"), "http://192.168.1.5:2283");
    }

    #[test]
    fn a_web_address_is_the_server_and_the_path() {
        let photo = asset_in(NodeKey::Album(ALBUM.to_owned()));
        assert_eq!(web_address("https://photos.example.com/api/", &photo), Some(format!("{SERVER}/albums/{ALBUM}/photos/{ASSET}")));
        assert_eq!(web_address(SERVER, &city()), Some(format!("{SERVER}/search?query={AARHUS}")));
    }

    /// Every key with a path, and what reading its link back gives.
    fn round_trips() -> Vec<(NodeKey, ImmichItem)> {
        let album = || ImmichItem::Album(ALBUM.to_owned());
        let person = || ImmichItem::Person(PERSON.to_owned());
        let aarhus = || ImmichItem::City { country: Some("Denmark".to_owned()), city: "Aarhus".to_owned() };
        let photo = |within: Option<ImmichItem>| ImmichItem::Asset { within: within.map(Box::new), asset_id: ASSET.to_owned() };
        let mut cases = vec![
            (asset_in(NodeKey::TimelineMonth(month())), photo(None)),
            (asset_in(NodeKey::TimelineDay(month(), 15)), photo(None)),
            (NodeKey::Country("Denmark".to_owned()), ImmichItem::Country("Denmark".to_owned())),
        ];
        let bases = [
            (NodeKey::Album(ALBUM.to_owned()), album()),
            (NodeKey::Person(PERSON.to_owned()), person()),
            (NodeKey::Section(SectionKey::Favorites), ImmichItem::Section(SectionKey::Favorites)),
            (NodeKey::Section(SectionKey::Archive), ImmichItem::Section(SectionKey::Archive)),
            (city(), aarhus()),
        ];
        for (base, item) in bases {
            cases.push((base.clone(), item.clone()));
            cases.push((asset_in(base.clone()), photo(Some(item.clone()))));
            for group in grouped(base) {
                cases.push((group.clone(), item.clone()));
                cases.push((asset_in(group), photo(Some(item.clone()))));
            }
        }
        for section in [SectionKey::Photos, SectionKey::Albums, SectionKey::People, SectionKey::Places] {
            cases.push((NodeKey::Section(section), ImmichItem::Section(section)));
        }
        cases
    }

    #[test]
    fn every_link_reads_back_as_its_item() {
        for (node, item) in round_trips() {
            let link = crosslink(SERVER, Some(PERSON), &node).unwrap();
            assert_eq!(read_crosslink(&link), Reading::Show(Crosslink { server: SERVER.to_owned(), user_id: Some(PERSON.to_owned()), item: item.clone() }), "{link}");
            let address = web_address(SERVER, &node).unwrap();
            assert_eq!(read_web_address(&address, "https://photos.example.com/api/"), Some(item.clone()), "{address}");
            if !matches!(node, NodeKey::GroupYear { .. } | NodeKey::GroupMonth { .. } | NodeKey::Asset { .. }) {
                assert_eq!(item.folder_key(), Some(node), "a folder's own key comes back");
            }
        }
    }

    #[test]
    fn odd_place_names_read_back() {
        for (country, city) in [("A|B", "100% Town"), ("日本", "東京"), ("Danmark", "Århus"), ("US", "St. Paul's \"Twin\" City")] {
            let node = NodeKey::City { country: country.to_owned(), city: city.to_owned() };
            let item = read_web_address(&web_address(SERVER, &node).unwrap(), SERVER).unwrap();
            assert_eq!(item.folder_key(), Some(node));
        }
    }

    #[test]
    fn immichs_own_city_search_has_no_country() {
        let places_page = format!("{SERVER}/search?query=%7B%22city%22%3A%22Aarhus%22%7D");
        assert_eq!(read_web_address(&places_page, SERVER), Some(ImmichItem::City { country: None, city: "Aarhus".to_owned() }));
        assert_eq!(read_web_address(&places_page, SERVER).unwrap().folder_key(), None, "the tree must find its country");
    }

    #[test]
    fn links_the_protocol_does_not_define_give_nothing() {
        let refused = [
            format!("{SERVER}/photos/not-a-uuid"),
            format!("{SERVER}/albums/{ALBUM}/edit"),
            format!("{SERVER}/albums/../people/{PERSON}"),
            format!("{SERVER}/search?query=%7B%22query%22%3A%22cat%22%7D"),
            format!("{SERVER}/search?query=%7B%22city%22%3A%22A%22%2C%22takenAfter%22%3A%222020%22%7D"),
            format!("{SERVER}/search?query=%7Bnot%20json"),
            format!("{SERVER}/search"),
            format!("{SERVER}/admin"),
            format!("{SERVER}/photos/{ALBUM}x"),
        ];
        for address in refused {
            assert_eq!(read_web_address(&address, SERVER), None, "{address}");
        }
    }

    #[test]
    fn another_server_is_not_this_one() {
        let path = format!("/albums/{ALBUM}");
        assert_eq!(read_web_address(&format!("https://other.example.com{path}"), SERVER), None);
        assert_eq!(read_web_address(&format!("http://photos.example.com{path}"), SERVER), None, "another scheme");
        assert_eq!(read_web_address(&format!("https://photos.example.com:8443{path}"), SERVER), None, "another port");
        assert_eq!(read_web_address(&format!("HTTPS://Photos.Example.com:443{path}#top"), SERVER), Some(ImmichItem::Album(ALBUM.to_owned())));
        assert!(same_server("http://192.168.1.5:80/", "http://192.168.1.5"));
        assert!(same_server("http://[::1]:2283", "http://[::1]:2283/"));
        assert!(!same_server("http://[::1]:2283", "http://[::1]"));
        assert!(!same_server("ftp://photos.example.com", "ftp://photos.example.com"), "only http and https");
    }

    #[test]
    fn crosslinks_are_read_as_the_protocol_says() {
        let unencoded = format!("immich-crosslink:///albums/{ALBUM}?server=https://photos.example.com/&dev.example.view=grid");
        let Reading::Show(read) = read_crosslink(&unencoded) else { panic!("{unencoded}") };
        assert_eq!((read.server.as_str(), read.user_id, read.item), (SERVER, None, ImmichItem::Album(ALBUM.to_owned())));
        let server = "server=https%3A%2F%2Fphotos.example.com";
        assert!(matches!(read_crosslink(&format!("IMMICH-CROSSLINK:///albums/{ALBUM}?{server}")), Reading::Show(_)), "a scheme has no case");
        assert_eq!(read_crosslink(&format!("immich-crosslink:///albums/{ALBUM}/?{server}")), Reading::CannotShow, "paths match exactly");
        assert_eq!(read_crosslink(&format!("immich-crosslink:///albums/{ALBUM}/edit?{server}")), Reading::CannotShow);
        let invalid = [
            format!("immich-crosslink:///albums/{ALBUM}"),
            format!("immich-crosslink:///albums/{ALBUM}?server=photos.example.com"),
            format!("immich-crosslink:///albums/{ALBUM}?server=ftp%3A%2F%2Fphotos.example.com"),
            format!("immich-crosslink://host/albums/{ALBUM}?{server}"),
            format!("immich-crosslink:///albums/{ALBUM}?{server}&user=me"),
            format!("immich-crosslink:///search?query=%7B%22city%22%3A1%7D&{server}"),
            format!("immich:///albums/{ALBUM}?{server}"),
        ];
        for link in invalid {
            assert_eq!(read_crosslink(&link), Reading::Invalid, "{link}");
        }
    }

    #[test]
    fn an_item_read_back_writes_the_same_path_and_kind() {
        for (node, item) in round_trips() {
            assert_eq!(Some(item.immich_path()), immich_path(&node), "{node:?}");
            assert_eq!(Some(item.kind()), kind(&node), "{node:?}");
        }
        assert_eq!(kind(&NodeKey::GroupMonth { base: Box::new(city()), month: month() }), Some(Kind::Place));
        assert_eq!(kind(&NodeKey::GroupYear { base: Box::new(NodeKey::Section(SectionKey::Favorites)), year: 2024 }), Some(Kind::Page));
        assert_eq!(kind(&NodeKey::TimelineMonth(month())), None);
        let upper = asset_in(NodeKey::Album(ALBUM.to_uppercase()));
        assert_eq!(immich_path(&upper), Some(format!("/albums/{ALBUM}/photos/{ASSET}")), "ids are written in lower case");
    }

    #[test]
    fn servers_are_normalised_as_the_protocol_compares_them() {
        assert_eq!(normalise_server("HTTPS://Photos.Example.com:443/").as_deref(), Some(SERVER));
        assert_eq!(normalise_server("http://nas.local:2283/immich/").as_deref(), Some("http://nas.local:2283/immich"));
        assert_eq!(normalise_server("http://[::1]:80").as_deref(), Some("http://[::1]"));
        for refused in ["photos.example.com", "ftp://photos.example.com", "https://", "https://host:", "https://host:x", "https://u@host", "https://host/?a"] {
            assert_eq!(normalise_server(refused), None, "{refused}");
        }
    }

    #[test]
    fn uuids_are_checked() {
        assert!(is_uuid(ASSET));
        assert!(is_uuid(&ASSET.to_uppercase()));
        for text in ["", "9a1e5c3e0b1d4c8e9f6a2d7b8c9e0f1a", "9a1e5c3e-0b1d-4c8e-9f6a-2d7b8c9e0f1", "9a1e5c3e-0b1d-4c8e-9f6a-2d7b8c9e0f1g"] {
            assert!(!is_uuid(text), "{text}");
        }
    }

    #[test]
    fn a_crosslink_has_an_empty_host_the_server_and_the_user() {
        let photo = asset_in(NodeKey::Album(ALBUM.to_owned()));
        assert_eq!(
            crosslink(SERVER, None, &photo),
            Some(format!("immich-crosslink:///albums/{ALBUM}/photos/{ASSET}?server=https%3A%2F%2Fphotos.example.com"))
        );
        assert_eq!(
            crosslink("https://photos.example.com/", Some("u-1"), &city()),
            Some(format!("immich-crosslink:///search?query={AARHUS}&server=https%3A%2F%2Fphotos.example.com&user=u-1"))
        );
        assert_eq!(crosslink(SERVER, Some(""), &city()), crosslink(SERVER, None, &city()));
    }
}

//! JSON answers captured from the lab server (Immich v3.3.5, `lab/immich-server/`), shared by the unit
//! tests. Long ids and paths are kept as the server sent them, so the fixtures stay recognisable.

/// `GET /timeline/buckets?visibility=timeline`, shortened. Newest first; `1970-01-01` holds `no-date.jpg`.
pub(crate) const TIME_BUCKETS: &str =
    r#"[{"timeBucket":"2025-04-01","count":1},{"timeBucket":"2024-06-01","count":2},{"timeBucket":"1970-01-01","count":1}]"#;

/// `GET /timeline/bucket?timeBucket=2018-11-01&visibility=timeline`: columns, `fileCreatedAt` without `Z`.
pub(crate) const TIME_BUCKET_COLUMNS: &str = r#"{"duration":[null,null,null],
    "id":["2d6e3655-50dd-40f5-b8cf-f7d3f56aa95e","518e39e4-0053-45e3-9bb8-39aae3834243","6f06f3de-c4eb-4d86-a780-a676e3fcc4b3"],
    "visibility":["timeline","timeline","timeline"],"isFavorite":[false,false,false],"isImage":[true,true,true],
    "isTrashed":[false,false,false],"livePhotoVideoId":[null,null,null],
    "fileCreatedAt":["2018-11-17T12:04:36","2018-11-17T11:31:54","2018-11-17T11:31:11"],"localOffsetHours":[1,1,1],
    "createdAt":["2026-11-01T21:01:50.577537","2026-11-01T21:01:50.365203","2026-11-01T21:01:50.15736"],
    "ownerId":["201f18cd-48ec-4220-8e10-1466650212ee","201f18cd-48ec-4220-8e10-1466650212ee","201f18cd-48ec-4220-8e10-1466650212ee"],
    "projectionType":[null,null,null],"ratio":[0.750,0.750,0.750],"status":["active","active","active"],
    "thumbhash":["HQgKDQIDSW2VuGdnh3p3h/+a0a8p","FwgKFQS/eoOIKIeIZneYh4pfpPkl","WAgKDQIYp4B3CYdnhnaXhjBsB9BE"],
    "city":["Lyon","Lyon","Lyon"],"country":["France","France","France"]}"#;

/// One item of `POST /search/metadata` with `withExif: true`.
pub(crate) const SEARCH_ITEM: &str = r#"{"id":"12ecb329-c96f-41b1-8406-e5b8fe134454",
    "createdAt":"2026-11-01T21:01:55.220Z","ownerId":"201f18cd-48ec-4220-8e10-1466650212ee","libraryId":null,
    "type":"IMAGE","originalPath":"/data/upload/x.jpg","originalFileName":"Tjarnarbíó 2025.jpg",
    "originalMimeType":"image/jpeg","thumbhash":"3PcNDgCFZ/h1d7h2d4qGiYaJdICXBKg=",
    "fileCreatedAt":"2025-04-24T16:21:34.000Z","fileModifiedAt":"2025-04-24T16:21:34.000Z",
    "localDateTime":"2025-04-24T16:21:34.000Z","updatedAt":"2026-11-01T21:01:55.436Z","isFavorite":false,
    "isArchived":false,"isTrashed":false,"visibility":"timeline","duration":null,
    "exifInfo":{"make":"samsung","model":"SM-G960F","exifImageWidth":1920,"exifImageHeight":2179,
      "fileSizeInByte":1012520,"orientation":null,"dateTimeOriginal":"2025-04-24T16:21:34+00:00",
      "timeZone":"Atlantic/Reykjavik","latitude":64.145849,"longitude":-21.943388,"city":"Reykjavík",
      "state":"Capital Region","country":"Iceland","description":"","projectionType":null,"rating":null},
    "livePhotoVideoId":null,"people":[],"checksum":"JpqXUgvoX45cHzen+eI2fGVg89U=","isOffline":false,
    "hasMetadata":true,"duplicateId":null,"resized":true,"width":1920,"height":2179,"isEdited":false}"#;

/// `GET /albums`, one album (the one named like a top-level folder).
pub(crate) const ALBUMS: &str = r#"[{"albumName":"Persons","description":"",
    "albumThumbnailAssetId":"e8b4a739-4120-4af2-b22f-6c337a14119a","createdAt":"2026-11-01T21:02:04.055Z",
    "updatedAt":"2026-11-01T21:02:04.055Z","id":"81d8ee12-2679-4d47-b903-a1b1ebc1d533",
    "albumUsers":[{"user":{"id":"201f18cd-48ec-4220-8e10-1466650212ee","email":"admin@example.com",
      "name":"Lab Admin","profileImagePath":"","avatarColor":"gray",
      "profileChangedAt":"2026-11-01T21:01:48.301924+00:00"},"role":"owner"}],
    "shared":false,"hasSharedLink":false,"startDate":"2014-08-15T00:00:00.000Z",
    "endDate":"2019-12-12T00:00:00.000Z","assetCount":3,"isActivityEnabled":true,"order":"desc",
    "lastModifiedAssetTimestamp":"2026-11-01T21:01:55.024Z"}]"#;

/// `GET /people?withHidden=true`.
pub(crate) const PEOPLE: &str = r#"{"people":[{"id":"1ac08f61-5c8a-4361-8139-a5dc1860137a","name":"Lab Person",
    "birthDate":null,"thumbnailPath":"/data/thumbs/x.jpeg","isHidden":false,"isFavorite":false,
    "updatedAt":"2026-11-01T21:02:04.237574+00:00","otherPeople":[],"sharedBy":[],"sharedWith":[]}],
    "hasNextPage":false,"total":1,"hidden":0}"#;

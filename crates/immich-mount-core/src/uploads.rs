//! Uploads into Photos, independent of the platform (SPEC.md, "Uploads"; ARCHITECTURE.md, "Uploads").
//!
//! Which names are accepted, and the trait the upload goes through. The platform adapter stages the bytes and calls
//! the upload when the writing program closes the file.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;

use chrono::{DateTime, Utc};

use crate::source::SourceError;

/// What happens to a file created in Photos under a given name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// An image or video: uploaded when the program closes it.
    Upload,
    /// A known temporary name (GIO's `.goutputstream-XXXXXX`, rsync's `.name.jpg.XXXXXX`): staged, and uploaded
    /// when renamed to its real name.
    Staged,
    /// Not something Immich takes. Refused at `create`, before any bytes are copied.
    Refused,
}

/// The extensions the server accepts, lower case with the dot (`.jpg`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTypes {
    extensions: HashSet<String>,
}

/// Immich's image and video extensions (RAW included), for when the server cannot be asked. The server's own list
/// (`/server/media-types`) wins whenever it answers.
const KNOWN_MEDIA: &[&str] = &[
    ".3fr", ".ari", ".arw", ".avif", ".bmp", ".cap", ".cin", ".cr2", ".cr3", ".crw", ".dcr", ".dng", ".erf", ".fff",
    ".gif", ".heic", ".heif", ".hif", ".iiq", ".insp", ".jp2", ".jpe", ".jpeg", ".jpg", ".jxl", ".k25", ".kdc", ".mrw",
    ".nef", ".nrw", ".orf", ".ori", ".pef", ".png", ".psd", ".raf", ".raw", ".rw2", ".rwl", ".sr2", ".srf", ".srw",
    ".svg", ".tif", ".tiff", ".webp", ".x3f", ".3gp", ".3gpp", ".avi", ".flv", ".insv", ".m2t", ".m2ts", ".m4v",
    ".mkv", ".mov", ".mp4", ".mpe", ".mpeg", ".mpg", ".mts", ".vob", ".webm", ".wmv",
];

impl Default for MediaTypes {
    fn default() -> Self {
        MediaTypes::new(KNOWN_MEDIA.iter().map(|extension| extension.to_string()))
    }
}

impl MediaTypes {
    pub fn new(extensions: impl IntoIterator<Item = String>) -> MediaTypes {
        MediaTypes { extensions: extensions.into_iter().map(|extension| extension.to_ascii_lowercase()).collect() }
    }

    /// Whether a file name has one of the accepted extensions.
    pub fn accepts(&self, name: &str) -> bool {
        match name.rfind('.') {
            Some(dot) if dot > 0 => self.extensions.contains(&name[dot..].to_ascii_lowercase()),
            _ => false,
        }
    }

    /// What happens to a file created under `name`. A hidden photo (`.IMG_1.JPG`) is still a photo and uploads on
    /// close; other hidden names are accepted only as known temporary names, so nothing is ever staged and then
    /// silently dropped (principle 6).
    pub fn admission(&self, name: &str) -> Admission {
        // macOS's AppleDouble companions (`._IMG_0001.JPG`, on cards and sticks used on a Mac) only look like photos.
        if name.starts_with("._") {
            Admission::Refused
        } else if self.accepts(name) {
            Admission::Upload
        } else if self.is_temporary_name(name) {
            Admission::Staged
        } else {
            Admission::Refused
        }
    }

    /// GIO's `.goutputstream-XXXXXX`, and rsync's `.<name>.<6 characters>` around an accepted name.
    fn is_temporary_name(&self, name: &str) -> bool {
        if name.starts_with(".goutputstream-") {
            return true;
        }
        let Some(inner) = name.strip_prefix('.') else { return false };
        match inner.rsplit_once('.') {
            Some((base, suffix)) => {
                suffix.len() == 6 && suffix.chars().all(|character| character.is_ascii_alphanumeric()) && self.accepts(base) && !base.starts_with("._")
            }
            None => false,
        }
    }
}

/// A finished upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Uploaded {
    /// A new Asset, with its id.
    Created(String),
    /// The library already had these bytes ("Already in library"): success, with the existing Asset's id.
    AlreadyInLibrary(String),
}

/// Why an upload failed. Each kind becomes an error the writing program sees on `close()`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UploadError {
    #[error(transparent)]
    Source(#[from] SourceError),
    /// The same bytes are in the trash. Not restored: the user decides (SPEC.md, "Uploads").
    #[error("the library has this file in its trash; restore it there")]
    TrashedDuplicate,
    /// The server refused the file, with its reason (`unsupported-format`).
    #[error("the server refused the file: {0}")]
    Rejected(String),
    /// The staged copy could not be read.
    #[error("cannot read the staged file: {0}")]
    LocalFile(String),
}

/// Where uploads go.
pub trait UploadTarget: Send + Sync {
    /// The extensions the server accepts.
    fn media_types(&self) -> impl Future<Output = Result<MediaTypes, SourceError>> + Send;

    /// Upload the file at `path` as `file_name`, unchanged. `modified` is its modification time, which also stands in
    /// for the capture time until the server reads the file's own metadata.
    fn upload(&self, path: &Path, file_name: &str, modified: DateTime<Utc>) -> impl Future<Output = Result<Uploaded, UploadError>> + Send;
}

impl<Shared: UploadTarget> UploadTarget for std::sync::Arc<Shared> {
    async fn media_types(&self) -> Result<MediaTypes, SourceError> {
        self.as_ref().media_types().await
    }

    async fn upload(&self, path: &Path, file_name: &str, modified: DateTime<Utc>) -> Result<Uploaded, UploadError> {
        self.as_ref().upload(path, file_name, modified).await
    }
}

/// Whether a file's own structure says it is whole, judged before upload, so a copy that was cancelled or killed
/// midway never becomes an Asset (principle 3). Formats it cannot judge (MPEG transport streams such as MTS, a
/// Matroska file written live) are `Unknown` and upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    Truncated,
    Unknown,
}

/// Judge a file by its structure: JPEG needs its end-of-image marker after its scans, PNG its `IEND` chunk, GIF its
/// trailer, RIFF (WebP, AVI) and BMP the length they declare, Matroska (WebM, MKV) its Segment's size, ISO media
/// (HEIC, AVIF, MP4, MOV) complete top-level boxes, and TIFF with the RAW formats built on it (DNG, CR2, NEF, ARW,
/// ORF, RW2) every image strip, tile and preview inside the file. Data appended after a whole image is allowed.
pub fn completeness(path: &Path) -> std::io::Result<Completeness> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let mut head = [0u8; 12];
    let read = file.read(&mut head)?;
    let head = &head[..read];
    if head.starts_with(&[0xFF, 0xD8]) {
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(&mut bytes)?;
        return Ok(jpeg_completeness(&bytes));
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        // The end chunk, allowing data appended after it.
        let end = tail(&mut file, length, length.min(64 * 1024))?;
        let whole = end.windows(12).any(|window| window == b"\0\0\0\0IEND\xaeB`\x82");
        return Ok(if whole { Completeness::Complete } else { Completeness::Truncated });
    }
    if head.starts_with(b"GIF8") {
        return Ok(if tail(&mut file, length, 1)? == [0x3B] { Completeness::Complete } else { Completeness::Truncated });
    }
    if head.len() == 12 && head.starts_with(b"RIFF") {
        let declared = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]])) + 8;
        return Ok(if length >= declared { Completeness::Complete } else { Completeness::Truncated });
    }
    if head.len() >= 8 && &head[4..8] == b"ftyp" {
        return iso_media_completeness(&mut file, length);
    }
    if head.len() >= 6 && head.starts_with(b"BM") {
        let declared = u64::from(u32::from_le_bytes([head[2], head[3], head[4], head[5]]));
        return Ok(match declared {
            0 => Completeness::Unknown,
            declared if length >= declared => Completeness::Complete,
            _ => Completeness::Truncated,
        });
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return matroska_completeness(&mut file, length);
    }
    if head.len() >= 8 && is_tiff(head) {
        return tiff_completeness(&mut file, length, head);
    }
    Ok(Completeness::Unknown)
}

fn tail(file: &mut std::fs::File, length: u64, count: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    if length < count {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(length - count))?;
    let mut bytes = vec![0u8; count as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Walk the marker segments; inside entropy-coded data a real `0xFF` is followed by `0x00` or a restart marker, so
/// the first other marker ends the scan. Complete only once the end-of-image marker (`FF D9`) is reached, which
/// also allows data after it (Motion Photos append a video there).
fn jpeg_completeness(bytes: &[u8]) -> Completeness {
    let mut position = 2;
    while position + 4 <= bytes.len() {
        if bytes[position] != 0xFF {
            return Completeness::Truncated;
        }
        let marker = bytes[position + 1];
        match marker {
            0xD9 => return Completeness::Complete,
            0xFF => position += 1,
            0x01 | 0xD0..=0xD7 => position += 2,
            _ => {
                let segment = usize::from(u16::from_be_bytes([bytes[position + 2], bytes[position + 3]]));
                position += 2 + segment;
                if marker == 0xDA {
                    // Entropy-coded data until the next marker that is not stuffing or a restart.
                    while position + 1 < bytes.len() {
                        if bytes[position] == 0xFF && !matches!(bytes[position + 1], 0x00 | 0xD0..=0xD7) {
                            break;
                        }
                        position += 1;
                    }
                }
            }
        }
    }
    if bytes.len() >= 2 && position + 2 == bytes.len() && bytes[position..] == [0xFF, 0xD9] {
        return Completeness::Complete;
    }
    Completeness::Truncated
}

/// Top-level boxes walked to the end of the file; or, once the index (`moov`, or `meta` in HEIC and AVIF) and the
/// media data (`mdat`) have both arrived whole, anything after them is appended data (a phone's trailer).
fn iso_media_completeness(file: &mut std::fs::File, length: u64) -> std::io::Result<Completeness> {
    use std::io::{Read, Seek, SeekFrom};
    let mut position = 0u64;
    let (mut index_whole, mut media_whole) = (false, false);
    while position < length {
        if length - position < 8 {
            return Ok(Completeness::Truncated);
        }
        file.seek(SeekFrom::Start(position))?;
        let mut header = [0u8; 16];
        let read = file.read(&mut header)?;
        let size = u64::from(u32::from_be_bytes([header[0], header[1], header[2], header[3]]));
        let box_length = match size {
            0 => return Ok(Completeness::Complete), // the last box runs to the end of the file
            1 if read >= 16 => u64::from_be_bytes(header[8..16].try_into().expect("eight bytes")),
            1 => return Ok(Completeness::Truncated),
            size => size,
        };
        if box_length < 8 {
            return Ok(Completeness::Unknown);
        }
        position = match position.checked_add(box_length) {
            Some(next) if next <= length => next,
            _ => return Ok(Completeness::Truncated),
        };
        match &header[4..8] {
            b"moov" | b"meta" => index_whole = true,
            b"mdat" => media_whole = true,
            _ => {}
        }
        if index_whole && media_whole {
            return Ok(Completeness::Complete);
        }
    }
    Ok(Completeness::Complete)
}

/// `count` bytes at `offset`, or `None` when they would run past the end of the file.
fn read_at(file: &mut std::fs::File, length: u64, offset: u64, count: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::{Read, Seek, SeekFrom};
    if offset.checked_add(count).is_none_or(|end| end > length) {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0u8; count as usize];
    file.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

/// An EBML variable-length integer at `at`: its value, where the next field starts, and whether it is the reserved
/// "unknown" value (every value bit set).
fn ebml_number(bytes: &[u8], at: usize) -> Option<(u64, usize, bool)> {
    let first = *bytes.get(at)?;
    let width = first.leading_zeros() as usize + 1;
    if width > 8 || bytes.len() < at + width {
        return None;
    }
    let mut value = u64::from(first) & (0xFF >> width);
    for byte in &bytes[at + 1..at + width] {
        value = (value << 8) | u64::from(*byte);
    }
    let unknown = value == (1u64 << (7 * width)) - 1;
    Some((value, at + width, unknown))
}

/// Matroska and WebM: after the EBML header, the Segment declares its size, unless it was written live.
fn matroska_completeness(file: &mut std::fs::File, length: u64) -> std::io::Result<Completeness> {
    let Some(head) = read_at(file, length, 0, length.min(4096))? else { return Ok(Completeness::Unknown) };
    // Too short to hold the fields: cut short if that is the whole file, else a layout this cannot read.
    let unreadable = if length <= 4096 { Completeness::Truncated } else { Completeness::Unknown };
    let Some((header_size, header_body, _)) = ebml_number(&head, 4) else { return Ok(unreadable) };
    let Some(segment) = usize::try_from(header_size).ok().and_then(|size| header_body.checked_add(size)) else { return Ok(unreadable) };
    if head.len() < segment + 4 {
        return Ok(unreadable);
    }
    if head[segment..segment + 4] != [0x18, 0x53, 0x80, 0x67] {
        return Ok(Completeness::Unknown);
    }
    let Some((segment_size, segment_body, unknown)) = ebml_number(&head, segment + 4) else { return Ok(unreadable) };
    if unknown {
        return Ok(Completeness::Unknown);
    }
    Ok(if (segment_body as u64).saturating_add(segment_size) <= length { Completeness::Complete } else { Completeness::Truncated })
}

/// TIFF's byte order and magic number, including the RAW variants: Olympus (`RO`, `RS`) and Panasonic (`U`).
fn is_tiff(head: &[u8]) -> bool {
    let magic = match &head[..2] {
        b"II" => u16::from_le_bytes([head[2], head[3]]),
        b"MM" => u16::from_be_bytes([head[2], head[3]]),
        _ => return false,
    };
    matches!(magic, 42 | 0x4F52 | 0x5352 | 0x0055)
}

/// TIFF and the RAW formats on it: the directories of the main chain and their SubIFDs, and every strip, tile and
/// embedded preview they point to, must lie inside the file. A sequential copy cut short loses the image data at the
/// end while the directories at the start still point to it. `Unknown` when no directory points to image data.
fn tiff_completeness(file: &mut std::fs::File, length: u64, head: &[u8]) -> std::io::Result<Completeness> {
    let little = head[0] == b'I';
    let short = |bytes: &[u8], at: usize| {
        let pair = [bytes[at], bytes[at + 1]];
        if little { u16::from_le_bytes(pair) } else { u16::from_be_bytes(pair) }
    };
    let long = |bytes: &[u8], at: usize| {
        let quad = [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
        if little { u32::from_le_bytes(quad) } else { u32::from_be_bytes(quad) }
    };
    const DIRECTORY_LIMIT: usize = 64;
    let mut pending = vec![u64::from(long(head, 4))];
    let mut visited = HashSet::new();
    let mut points_to_data = false;
    while let Some(offset) = pending.pop() {
        if offset == 0 || visited.len() >= DIRECTORY_LIMIT || !visited.insert(offset) {
            continue;
        }
        let Some(count) = read_at(file, length, offset, 2)? else { return Ok(Completeness::Truncated) };
        let entries = usize::from(short(&count, 0));
        let Some(table) = read_at(file, length, offset + 2, entries as u64 * 12 + 4)? else { return Ok(Completeness::Truncated) };
        let mut values: HashMap<u16, Vec<u64>> = HashMap::new();
        for entry in table[..entries * 12].as_chunks::<12>().0 {
            let tag = short(entry, 0);
            // Strip offsets and byte counts, tile offsets and byte counts, SubIFDs, the embedded JPEG preview.
            if !matches!(tag, 273 | 279 | 324 | 325 | 330 | 513 | 514) {
                continue;
            }
            let width: u64 = match short(entry, 2) {
                3 => 2,
                4 | 13 => 4,
                _ => continue,
            };
            let count = u64::from(long(entry, 4));
            if count == 0 || count > 1 << 20 {
                continue;
            }
            let data = if count * width <= 4 {
                entry[8..12].to_vec()
            } else {
                match read_at(file, length, u64::from(long(entry, 8)), count * width)? {
                    Some(data) => data,
                    None => return Ok(Completeness::Truncated),
                }
            };
            let read = |index: usize| if width == 2 { u64::from(short(&data, index * 2)) } else { u64::from(long(&data, index * 4)) };
            values.insert(tag, (0..count as usize).map(read).collect());
        }
        for (starts, sizes) in [(273, 279), (324, 325), (513, 514)] {
            let (Some(starts), Some(sizes)) = (values.get(&starts), values.get(&sizes)) else { continue };
            for (start, size) in starts.iter().zip(sizes) {
                points_to_data = true;
                if start.saturating_add(*size) > length {
                    return Ok(Completeness::Truncated);
                }
            }
        }
        pending.extend(values.remove(&330).unwrap_or_default());
        pending.push(u64::from(long(&table, entries * 12)));
    }
    Ok(if points_to_data { Completeness::Complete } else { Completeness::Unknown })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_and_videos_upload_hidden_names_wait_and_the_rest_is_refused() {
        let media = MediaTypes::default();
        assert_eq!(media.admission("IMG_0001.JPG"), Admission::Upload);
        assert_eq!(media.admission("clip.MOV"), Admission::Upload);
        assert_eq!(media.admission("RAW_0001.dng"), Admission::Upload);
        assert_eq!(media.admission(".goutputstream-ABC123"), Admission::Staged);
        assert_eq!(media.admission(".IMG_0001.JPG.Xy12Za"), Admission::Staged, "rsync's temporary name");
        assert_eq!(media.admission(".IMG_0001.JPG"), Admission::Upload, "a hidden photo is still a photo");
        assert_eq!(media.admission("._IMG_0001.JPG"), Admission::Refused, "a Mac's AppleDouble companion");
        assert_eq!(media.admission(".._IMG_0001.JPG.Xy12Za"), Admission::Refused, "nor rsync's name for one");
        assert_eq!(media.admission(".hidden-notes"), Admission::Refused, "never staged and then dropped");
        assert_eq!(media.admission(".notes.txt.Ab12Cd"), Admission::Refused);
        assert_eq!(media.admission("notes.txt"), Admission::Refused);
        assert_eq!(media.admission("IMG_0001.JPG.xmp"), Admission::Refused, "sidecars are not supported");
        assert_eq!(media.admission("jpg"), Admission::Refused);
    }

    #[test]
    fn the_servers_list_replaces_the_built_in_one() {
        let media = MediaTypes::new([".JPG".to_string()]);
        assert!(media.accepts("a.jpg"));
        assert!(!media.accepts("a.png"));
    }

    fn judged(bytes: &[u8]) -> Completeness {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, bytes).unwrap();
        completeness(&path).unwrap()
    }

    /// A minimal JPEG: SOI, an APP0 segment, SOS with entropy data (stuffed FF 00 and a restart), EOI.
    fn jpeg() -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46];
        bytes.extend([0xFF, 0xDA, 0x00, 0x03, 0x01, 0x12, 0x34, 0xFF, 0x00, 0x56, 0xFF, 0xD0, 0x78, 0xFF, 0xD9]);
        bytes
    }

    #[test]
    fn a_whole_jpeg_is_complete_and_a_cut_one_is_not() {
        let whole = jpeg();
        assert_eq!(judged(&whole), Completeness::Complete);
        assert_eq!(judged(&whole[..whole.len() - 2]), Completeness::Truncated);
        assert_eq!(judged(&whole[..11]), Completeness::Truncated);
        let mut motion_photo = whole.clone();
        motion_photo.extend(b"\0\0\0\x18ftypmp42 a video after the image");
        assert_eq!(judged(&motion_photo), Completeness::Complete, "data after EOI is allowed");
    }

    #[test]
    fn iso_media_needs_boxes_that_end_at_the_end() {
        let mut movie = Vec::new();
        movie.extend([0, 0, 0, 16]);
        movie.extend(b"ftypisom");
        movie.extend([0, 0, 0, 0]);
        movie.extend([0, 0, 0, 12]);
        movie.extend(b"mdat");
        movie.extend([1, 2, 3, 4]);
        assert_eq!(judged(&movie), Completeness::Complete);
        assert_eq!(judged(&movie[..movie.len() - 1]), Completeness::Truncated);
    }

    #[test]
    fn png_gif_and_riff_are_judged_by_their_ends() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(b"\0\0\0\0IEND\xaeB`\x82");
        assert_eq!(judged(&png), Completeness::Complete);
        assert_eq!(judged(&png[..png.len() - 3]), Completeness::Truncated);
        assert_eq!(judged(b"GIF89a....;"), Completeness::Complete);
        assert_eq!(judged(b"GIF89a...."), Completeness::Truncated);
        let mut webp = b"RIFF".to_vec();
        webp.extend(8u32.to_le_bytes());
        webp.extend(b"WEBPVP8 ");
        assert_eq!(judged(&webp), Completeness::Complete);
        assert_eq!(judged(&webp[..14]), Completeness::Truncated);
        let mut padded = png.clone();
        padded.extend(b"appended");
        assert_eq!(judged(&padded), Completeness::Complete, "data after IEND is allowed");
        let mut bitmap = b"BM".to_vec();
        bitmap.extend(10u32.to_le_bytes());
        bitmap.extend([0, 0, 0, 0]);
        assert_eq!(judged(&bitmap), Completeness::Complete);
        assert_eq!(judged(&bitmap[..9]), Completeness::Truncated);
    }

    #[test]
    fn iso_media_may_carry_a_trailer_once_index_and_media_are_whole() {
        let mut movie = Vec::new();
        movie.extend([0, 0, 0, 16]);
        movie.extend(b"ftypisom");
        movie.extend([0, 0, 0, 0]);
        movie.extend([0, 0, 0, 8]);
        movie.extend(b"moov");
        movie.extend([0, 0, 0, 12]);
        movie.extend(b"mdat");
        movie.extend([1, 2, 3, 4]);
        movie.extend(b"SEFHa phone's trailer");
        assert_eq!(judged(&movie), Completeness::Complete);
        assert_eq!(judged(&movie[..30]), Completeness::Truncated, "cut inside mdat");
    }

    #[test]
    fn matroska_is_judged_by_its_segment_size() {
        let mut video = vec![0x1A, 0x45, 0xDF, 0xA3, 0x84, 1, 2, 3, 4];
        video.extend([0x18, 0x53, 0x80, 0x67, 0x84, 5, 6, 7, 8]);
        assert_eq!(judged(&video), Completeness::Complete);
        assert_eq!(judged(&video[..video.len() - 1]), Completeness::Truncated);
        let live = [&video[..9], &[0x18, 0x53, 0x80, 0x67, 0xFF][..], &[5, 6][..]].concat();
        assert_eq!(judged(&live), Completeness::Unknown, "a live recording declares no size");
    }

    /// A little-endian TIFF: one directory with one strip of four bytes, stored after it.
    fn tiff() -> Vec<u8> {
        let mut bytes = b"II*\0".to_vec();
        bytes.extend(8u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        for (tag, value) in [(273u16, 38u32), (279, 4)] {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(4u16.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(0u32.to_le_bytes());
        bytes.extend([1, 2, 3, 4]);
        bytes
    }

    #[test]
    fn tiff_and_raw_need_their_image_data_inside_the_file() {
        let whole = tiff();
        assert_eq!(whole.len(), 42);
        assert_eq!(judged(&whole), Completeness::Complete);
        assert_eq!(judged(&whole[..40]), Completeness::Truncated, "the strip runs past the end");
        assert_eq!(judged(&whole[..20]), Completeness::Truncated, "the directory itself is cut");
        assert_eq!(judged(b"II*\0 not a TIFF at all"), Completeness::Truncated);
    }
}


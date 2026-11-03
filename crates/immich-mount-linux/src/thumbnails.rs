//! Seeds for the freedesktop thumbnail cache (ARCHITECTURE.md, "Thumbnails").
//!
//! GNOME Files never reads an original if a valid thumbnail for it exists in any bucket of `~/.cache/thumbnails`, so
//! the daemon writes those thumbnails itself, from the small images Immich already has, before a folder is listed.
//! This module is the pure part of that: naming, rendering and writing the PNGs. It makes no network calls and knows
//! nothing of FUSE; the caller supplies the image bytes and the mount path.
//!
//! What makes a thumbnail valid to GNOME (seen in a GNOME 50 VM):
//! - **The file name** is the lowercase hex MD5 of the original's URI, plus `.png`.
//! - **The URI** must be exactly what GLib's `g_filename_to_uri(path, NULL)` makes of the path, byte for byte, or the
//!   name will not match. [`file_uri`] reproduces it, and the tests compare it with vectors taken from real GLib.
//! - **The `tEXt` chunks** `Thumb::URI` and `Thumb::MTime` must equal the original's URI and modification time (whole
//!   seconds), or the thumbnail is treated as stale.
//! - **Undersized PNGs are accepted as they are,** so an image smaller than its bucket is never scaled up.
//!
//! Immich's thumbnails are already rotated to their EXIF orientation when the server makes them, so no orientation
//! is applied here.
//!
//! Seeds are small copies of private photos in a shared directory. The caller records every URI it seeds, and calls
//! [`remove_thumbnails`] for each on disconnect, account change or cache clearing.

use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::imageops::FilterType;
use image::DynamicImage;
use md5::{Digest, Md5};

/// The `Software` text chunk in every seed, which tells our thumbnails apart from GNOME's own.
const SOFTWARE: &str = "Immich Mount";

/// Where GNOME's thumbnail factory records a failed thumbnail, below the thumbnail root.
const GNOME_FAILURE_DIRECTORY: &str = "fail/gnome-thumbnail-factory";

/// Every ASCII byte GLib leaves as it is in the path of a `file://` URI, read off GLib 2.88 by passing each byte
/// through `g_filename_to_uri`. Every other byte, including `;`, space and all non-ASCII bytes, is percent-encoded.
const KEPT_IN_PATH: &[u8] = b"!$&'()*+,-./0123456789:=@ABCDEFGHIJKLMNOPQRSTUVWXYZ_abcdefghijklmnopqrstuvwxyz~";

/// Tells apart the temporary files of concurrent writes within this process.
static TEMPORARY_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Why a thumbnail could not be made, written or removed.
#[derive(Debug, thiserror::Error)]
pub enum ThumbnailError {
    #[error("could not decode the source image: {0}")]
    Decode(String),
    #[error("could not encode the thumbnail PNG: {0}")]
    Encode(String),
    #[error("{0:?} is not a file URI from `file_uri`: it must be ASCII")]
    UriNotAscii(String),
    #[error("{path}: {error}")]
    Io { path: PathBuf, error: std::io::Error },
}

/// The size buckets of the freedesktop thumbnail cache, one directory each below the thumbnail root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bucket {
    Normal,
    Large,
    XLarge,
    XxLarge,
}

impl Bucket {
    /// Every bucket, smallest first.
    pub const ALL: [Bucket; 4] = [Bucket::Normal, Bucket::Large, Bucket::XLarge, Bucket::XxLarge];

    /// The bucket's directory below the thumbnail root.
    pub fn directory_name(self) -> &'static str {
        match self {
            Bucket::Normal => "normal",
            Bucket::Large => "large",
            Bucket::XLarge => "x-large",
            Bucket::XxLarge => "xx-large",
        }
    }

    /// The longest side, in pixels, a thumbnail in this bucket may have.
    pub fn max_pixels(self) -> u32 {
        match self {
            Bucket::Normal => 128,
            Bucket::Large => 256,
            Bucket::XLarge => 512,
            Bucket::XxLarge => 1024,
        }
    }
}

/// The `file://` URI of an absolute path, exactly as GLib's `g_filename_to_uri(path, NULL)` makes it.
///
/// The path's bytes are taken as they are (Linux file names need not be UTF-8), and every byte outside GLib's set
/// for paths is written as `%XX` with uppercase hex. GLib refuses relative paths; so must the caller.
pub fn file_uri(path: &Path) -> String {
    let path_bytes = path.as_os_str().as_bytes();
    let mut uri = String::with_capacity("file://".len() + path_bytes.len() * 3);
    uri.push_str("file://");
    for &byte in path_bytes {
        if KEPT_IN_PATH.contains(&byte) {
            uri.push(char::from(byte));
        } else {
            // Writing to a String cannot fail.
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    uri
}

/// The cache file name for a URI: the lowercase hex MD5 of the URI string, plus `.png`.
pub fn thumbnail_file_name(uri: &str) -> String {
    let digest = Md5::digest(uri.as_bytes());
    let mut name = String::with_capacity(digest.len() * 2 + ".png".len());
    for byte in digest.iter() {
        // Writing to a String cannot fail.
        let _ = write!(name, "{byte:02x}");
    }
    name.push_str(".png");
    name
}

/// Where the thumbnail for `uri` lives in `bucket`, below `thumbnail_root` (normally `~/.cache/thumbnails`).
pub fn thumbnail_path(thumbnail_root: &Path, bucket: Bucket, uri: &str) -> PathBuf {
    thumbnail_root.join(bucket.directory_name()).join(thumbnail_file_name(uri))
}

/// Render a thumbnail PNG for `bucket` from a WebP or JPEG (Immich's `thumbnail` and `preview` sizes).
///
/// The image is scaled down to fit the bucket's longest side, keeping its aspect ratio, and never scaled up. The PNG
/// carries the `tEXt` chunks `Thumb::URI` (`uri`, which must come from [`file_uri`]), `Thumb::MTime`
/// (`mtime_seconds`, in decimal) and `Software`.
pub fn render_png(source_image_bytes: &[u8], bucket: Bucket, uri: &str, mtime_seconds: i64) -> Result<Vec<u8>, ThumbnailError> {
    // tEXt is Latin-1, and GLib's URIs are ASCII. Anything else would be stored in a form GNOME never matches.
    if !uri.is_ascii() {
        return Err(ThumbnailError::UriNotAscii(uri.to_string()));
    }
    let source = image::load_from_memory(source_image_bytes).map_err(|error| ThumbnailError::Decode(error.to_string()))?;
    let (width, height) = fitted_dimensions(source.width(), source.height(), bucket.max_pixels());
    let scaled = if (width, height) == (source.width(), source.height()) {
        source
    } else {
        source.resize_exact(width, height, FilterType::CatmullRom)
    };
    encode_png(&scaled, uri, mtime_seconds)
}

/// The size an image of `width` x `height` is given to fit `max_pixels` on its longest side: scaled down, keeping
/// the aspect ratio (rounded to the nearest pixel, at least one), or left as it is if it already fits.
fn fitted_dimensions(width: u32, height: u32, max_pixels: u32) -> (u32, u32) {
    let longest_side = width.max(height);
    if longest_side <= max_pixels {
        return (width, height);
    }
    let scale = f64::from(max_pixels) / f64::from(longest_side);
    let scale_side = |side: u32| ((f64::from(side) * scale).round() as u32).clamp(1, max_pixels);
    (scale_side(width), scale_side(height))
}

/// Encode 8-bit RGB (or RGBA, if the image has alpha) as a PNG with the thumbnail's text chunks.
fn encode_png(image: &DynamicImage, uri: &str, mtime_seconds: i64) -> Result<Vec<u8>, ThumbnailError> {
    let (color_type, pixels) = if image.color().has_alpha() {
        (png::ColorType::Rgba, image.to_rgba8().into_raw())
    } else {
        (png::ColorType::Rgb, image.to_rgb8().into_raw())
    };
    let encode_error = |error: png::EncodingError| ThumbnailError::Encode(error.to_string());
    let mut png_bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut png_bytes, image.width(), image.height());
    encoder.set_color(color_type);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.add_text_chunk("Thumb::URI".to_string(), uri.to_string()).map_err(encode_error)?;
    encoder.add_text_chunk("Thumb::MTime".to_string(), mtime_seconds.to_string()).map_err(encode_error)?;
    encoder.add_text_chunk("Software".to_string(), SOFTWARE.to_string()).map_err(encode_error)?;
    let mut writer = encoder.write_header().map_err(encode_error)?;
    writer.write_image_data(&pixels).map_err(encode_error)?;
    writer.finish().map_err(encode_error)?;
    Ok(png_bytes)
}

/// Write a rendered thumbnail for `uri` into `bucket` below `thumbnail_root`, and return its path.
///
/// The bucket directory is created (mode 0700) if it is missing. The PNG goes to a temporary file in the same
/// directory with mode 0600 and is then renamed over the final name, so a reader never sees half a thumbnail. There
/// is no fsync: after a power loss a torn file is merely an invalid thumbnail, which the next seed replaces.
pub fn write_thumbnail(thumbnail_root: &Path, bucket: Bucket, uri: &str, png_bytes: &[u8]) -> Result<PathBuf, ThumbnailError> {
    let directory = thumbnail_root.join(bucket.directory_name());
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .map_err(|error| ThumbnailError::Io { path: directory.clone(), error })?;
    let file_name = thumbnail_file_name(uri);
    let final_path = directory.join(&file_name);
    let (temporary_path, mut temporary_file) = create_temporary_file(&directory, &file_name)?;
    let written = temporary_file
        .write_all(png_bytes)
        .and_then(|()| std::fs::rename(&temporary_path, &final_path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ThumbnailError::Io { path: final_path, error });
    }
    Ok(final_path)
}

/// Create a new, empty file with mode 0600 next to the thumbnail `file_name` in `directory`, under a name no other
/// writer uses: the process id and a counter, retried past any leftover from an earlier process with the same id.
fn create_temporary_file(directory: &Path, file_name: &str) -> Result<(PathBuf, std::fs::File), ThumbnailError> {
    let process_id = std::process::id();
    loop {
        let sequence = TEMPORARY_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary_path = directory.join(format!(".{file_name}.{process_id}.{sequence}.tmp"));
        let opened = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary_path);
        match opened {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ThumbnailError::Io { path: temporary_path, error }),
        }
    }
}

/// Remove the thumbnails for `uri` from every bucket below `thumbnail_root`, and GNOME's recorded failure for it
/// (`fail/gnome-thumbnail-factory/`). Files that do not exist are fine. Every file is tried; the first error that
/// is not a missing file is returned.
pub fn remove_thumbnails(thumbnail_root: &Path, uri: &str) -> Result<(), ThumbnailError> {
    let file_name = thumbnail_file_name(uri);
    let bucket_paths = Bucket::ALL.iter().map(|bucket| thumbnail_root.join(bucket.directory_name()).join(&file_name));
    let failure_path = thumbnail_root.join(GNOME_FAILURE_DIRECTORY).join(&file_name);
    let mut first_error = None;
    for path in bucket_paths.chain(std::iter::once(failure_path)) {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(ThumbnailError::Io { path, error });
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The `Thumb::URI` a thumbnail PNG carries, read from its text chunks without decoding the image.
pub fn uri_of_thumbnail(png_bytes: &[u8]) -> Option<String> {
    thumbnail_text(png_bytes, "Thumb::URI")
}

/// A text value of a thumbnail PNG, such as `Thumb::MTime`: plain (`tEXt`, ours and GNOME's), compressed (`zTXt`,
/// KDE's) or international (`iTXt`). Ours come before the image data, so the first few kilobytes of a seed are enough.
pub fn thumbnail_text(png_bytes: &[u8], keyword: &str) -> Option<String> {
    let prefix = [keyword.as_bytes(), b"\0"].concat();
    let mut position = 8; // the PNG signature
    while position + 8 <= png_bytes.len() {
        let length = u32::from_be_bytes(png_bytes[position..position + 4].try_into().ok()?) as usize;
        let kind = &png_bytes[position + 4..position + 8];
        let data = png_bytes.get(position + 8..position + 8 + length)?;
        if let Some(text) = data.strip_prefix(prefix.as_slice()) {
            match kind {
                // Ours, and GNOME's: the text as it is.
                b"tEXt" => return String::from_utf8(text.to_vec()).ok(),
                // KDE's thumbnailer: a compression method (0, zlib), then the text compressed.
                b"zTXt" => return inflated(text.strip_prefix(&[0])?),
                // International text: a compression flag and method, a language and a translated keyword, then the text.
                b"iTXt" => {
                    let (&compressed, rest) = text.split_first()?;
                    let rest = rest.get(1..)?;
                    let after_language = &rest[rest.iter().position(|&byte| byte == 0)? + 1..];
                    let body = &after_language[after_language.iter().position(|&byte| byte == 0)? + 1..];
                    return if compressed == 1 { inflated(body) } else { String::from_utf8(body.to_vec()).ok() };
                }
                _ => {}
            }
        }
        if kind == b"IEND" {
            return None;
        }
        position += 12 + length;
    }
    None
}

fn inflated(compressed: &[u8]) -> Option<String> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut flate2::read::ZlibDecoder::new(compressed), &mut text).ok()?;
    Some(text)
}

/// Remove every thumbnail, in every bucket and in GNOME's failure records, whose URI lies under `uri_prefix`
/// (`file:///home/me/Immich/`): ours, and any a thumbnailer made or failed on. Returns how many were removed.
pub fn remove_thumbnails_under(thumbnail_root: &Path, uri_prefix: &str) -> usize {
    let directories = Bucket::ALL
        .iter()
        .map(|bucket| thumbnail_root.join(bucket.directory_name()))
        .chain(std::iter::once(thumbnail_root.join(GNOME_FAILURE_DIRECTORY)));
    let mut removed = 0;
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(&directory) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "png") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            if uri_of_thumbnail(&bytes).is_some_and(|uri| uri.starts_with(uri_prefix)) && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;

    use image::{ImageFormat, RgbImage, RgbaImage};

    const FOLDER: &str = "/home/tester/Immich/Photos/2024/03 March/";

    // These URIs come from GLib's `g_filename_to_uri(path, NULL)` (GLib 2.88.4, through PyGObject's
    // `GLib.filename_to_uri`), for each name inside FOLDER.
    const GLIB_VECTORS: &[(&str, &str)] = &[
        ("2024-03-15 14.32.07.jpg", "file:///home/tester/Immich/Photos/2024/03%20March/2024-03-15%2014.32.07.jpg"),
        ("a#b%c?d[e]f.jpg", "file:///home/tester/Immich/Photos/2024/03%20March/a%23b%25c%3Fd%5Be%5Df.jpg"),
        ("\u{c5}rhus caf\u{e9}.jpg", "file:///home/tester/Immich/Photos/2024/03%20March/%C3%85rhus%20caf%C3%A9.jpg"),
        // "Café" with a combining acute accent (NFD), not the precomposed é.
        ("Cafe\u{301}.jpg", "file:///home/tester/Immich/Photos/2024/03%20March/Cafe%CC%81.jpg"),
        ("100% & more (1).JPG", "file:///home/tester/Immich/Photos/2024/03%20March/100%25%20&%20more%20(1).JPG"),
        ("semi;colon=eq@at.png", "file:///home/tester/Immich/Photos/2024/03%20March/semi%3Bcolon=eq@at.png"),
        ("tab\tname.jpg", "file:///home/tester/Immich/Photos/2024/03%20March/tab%09name.jpg"),
        ("\u{30bf}\u{30c6}\u{5f01} 01.jpg", "file:///home/tester/Immich/Photos/2024/03%20March/%E3%82%BF%E3%83%86%E5%BC%81%2001.jpg"),
    ];

    #[test]
    fn file_uris_match_glib() {
        for (name, glib_uri) in GLIB_VECTORS {
            let path = PathBuf::from(format!("{FOLDER}{name}"));
            assert_eq!(file_uri(&path), *glib_uri, "for {name:?}");
        }
    }

    #[test]
    fn non_utf8_bytes_are_encoded_as_they_are() {
        // Also from GLib: g_filename_to_uri(b"/tmp/caf\xe9 \xff.jpg", NULL).
        let path = Path::new(OsStr::from_bytes(b"/tmp/caf\xe9 \xff.jpg"));
        assert_eq!(file_uri(path), "file:///tmp/caf%E9%20%FF.jpg");
    }

    #[test]
    fn every_other_byte_is_percent_encoded() {
        // GLib encodes each byte 1..=255 that is not in KEPT_IN_PATH as %XX (checked byte by byte against GLib).
        for byte in 1..=255u8 {
            let path_bytes = [b'/', byte];
            let path = Path::new(OsStr::from_bytes(&path_bytes));
            let expected = if KEPT_IN_PATH.contains(&byte) { format!("file:///{}", char::from(byte)) } else { format!("file:///%{byte:02X}") };
            assert_eq!(file_uri(path), expected, "for byte {byte:#04x}");
        }
    }

    #[test]
    fn the_file_name_is_the_md5_of_the_uri() {
        // hashlib.md5(b"file:///home/tester/Immich/2024/03/IMG_1022.jpg").hexdigest()
        assert_eq!(thumbnail_file_name("file:///home/tester/Immich/2024/03/IMG_1022.jpg"), "09136648df6f3a764f19a57d09232796.png");
    }

    #[test]
    fn buckets_name_their_directories_and_sizes() {
        let described: Vec<(&str, u32)> = Bucket::ALL.iter().map(|bucket| (bucket.directory_name(), bucket.max_pixels())).collect();
        assert_eq!(described, [("normal", 128), ("large", 256), ("x-large", 512), ("xx-large", 1024)]);
    }

    /// A gradient, so the encoders have real content to compress.
    fn gradient(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |column, row| image::Rgb([(column % 256) as u8, (row % 256) as u8, ((column + row) % 256) as u8]))
    }

    fn encode(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    /// The PNG's dimensions, and its tEXt chunks as (keyword, text) pairs.
    fn read_png(png_bytes: &[u8]) -> ((u32, u32), Vec<(String, String)>) {
        let reader = png::Decoder::new(Cursor::new(png_bytes)).read_info().unwrap();
        let info = reader.info();
        let text_chunks = info.uncompressed_latin1_text.iter().map(|chunk| (chunk.keyword.clone(), chunk.text.clone())).collect();
        ((info.width, info.height), text_chunks)
    }

    #[test]
    fn a_large_jpeg_is_scaled_down_to_each_bucket() {
        // 3:2 like a camera photo; 1536 x 1024 rather than 3840 x 2560 keeps a debug-build test fast, with the same results.
        let jpeg = encode(DynamicImage::ImageRgb8(gradient(1536, 1024)), ImageFormat::Jpeg);
        let uri = "file:///home/tester/Immich/2024/03/IMG_1022.jpg";
        let x_large = render_png(&jpeg, Bucket::XLarge, uri, 1_710_513_127).unwrap();
        let xx_large = render_png(&jpeg, Bucket::XxLarge, uri, 1_710_513_127).unwrap();
        assert_eq!(read_png(&x_large).0, (512, 341));
        assert_eq!(read_png(&xx_large).0, (1024, 683));
    }

    #[test]
    fn a_small_webp_is_never_scaled_up() {
        let webp = encode(DynamicImage::ImageRgb8(gradient(300, 200)), ImageFormat::WebP);
        let rendered = render_png(&webp, Bucket::XxLarge, "file:///small.webp", 0).unwrap();
        assert_eq!(read_png(&rendered).0, (300, 200));
    }

    #[test]
    fn a_portrait_webp_with_alpha_fits_on_its_long_side() {
        let portrait = RgbaImage::from_pixel(400, 900, image::Rgba([10, 20, 30, 128]));
        let webp = encode(DynamicImage::ImageRgba8(portrait), ImageFormat::WebP);
        let rendered = render_png(&webp, Bucket::Large, "file:///portrait.webp", 0).unwrap();
        let reader = png::Decoder::new(Cursor::new(&rendered)).read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (114, 256));
        assert_eq!(reader.info().color_type, png::ColorType::Rgba);
    }

    #[test]
    fn compressed_and_international_text_is_read_too() {
        fn chunk(kind: &[u8], data: &[u8]) -> Vec<u8> {
            let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
            bytes.extend_from_slice(kind);
            bytes.extend_from_slice(data);
            bytes.extend_from_slice(&[0, 0, 0, 0]); // the CRC, which the reader does not check
            bytes
        }
        let uri = "file:///home/me/Immich/Photos/slow%20copy.jpg";
        let mut compressed = Vec::new();
        std::io::Write::write_all(&mut flate2::write::ZlibEncoder::new(&mut compressed, flate2::Compression::default()), uri.as_bytes()).unwrap();
        let mut ztxt = b"Thumb::URI\0\0".to_vec();
        ztxt.extend_from_slice(&compressed);
        let png = [b"\x89PNG\r\n\x1a\n".to_vec(), chunk(b"zTXt", &ztxt), chunk(b"IEND", b"")].concat();
        assert_eq!(thumbnail_text(&png, "Thumb::URI").as_deref(), Some(uri));
        let itxt = [b"Thumb::URI\0\0\0en\0\0".as_slice(), uri.as_bytes()].concat();
        let png = [b"\x89PNG\r\n\x1a\n".to_vec(), chunk(b"iTXt", &itxt), chunk(b"IEND", b"")].concat();
        assert_eq!(thumbnail_text(&png, "Thumb::URI").as_deref(), Some(uri));
    }

    #[test]
    fn the_png_carries_the_thumbnail_text_chunks() {
        let webp = encode(DynamicImage::ImageRgb8(gradient(300, 200)), ImageFormat::WebP);
        let uri = "file:///home/tester/Immich/Photos/2024/03%20March/a%23b.jpg";
        let rendered = render_png(&webp, Bucket::XLarge, uri, 1_710_513_127).unwrap();
        let (_, text_chunks) = read_png(&rendered);
        let expected = [("Thumb::URI", uri), ("Thumb::MTime", "1710513127"), ("Software", "Immich Mount")];
        for (keyword, text) in expected {
            assert!(text_chunks.contains(&(keyword.to_string(), text.to_string())), "missing {keyword}={text} in {text_chunks:?}");
        }
    }

    #[test]
    fn bytes_that_are_no_image_are_a_decode_error() {
        let rendered = render_png(b"not an image at all", Bucket::Normal, "file:///broken.jpg", 0);
        assert!(matches!(rendered, Err(ThumbnailError::Decode(_))));
    }

    #[test]
    fn a_non_ascii_uri_is_refused() {
        let webp = encode(DynamicImage::ImageRgb8(gradient(16, 16)), ImageFormat::WebP);
        let rendered = render_png(&webp, Bucket::Normal, "file:///caf\u{e9}.jpg", 0);
        assert!(matches!(rendered, Err(ThumbnailError::UriNotAscii(_))));
    }

    #[test]
    fn a_written_thumbnail_is_private_named_by_md5_and_leaves_no_temporary_file() {
        let thumbnail_root = tempfile::tempdir().unwrap();
        let uri = "file:///home/tester/Immich/2024/03/IMG_1022.jpg";
        let written = write_thumbnail(thumbnail_root.path(), Bucket::XLarge, uri, b"png bytes").unwrap();

        assert_eq!(written, thumbnail_root.path().join("x-large/09136648df6f3a764f19a57d09232796.png"));
        assert_eq!(written, thumbnail_path(thumbnail_root.path(), Bucket::XLarge, uri));
        assert_eq!(std::fs::read(&written).unwrap(), b"png bytes");
        assert_eq!(std::fs::metadata(&written).unwrap().permissions().mode() & 0o777, 0o600);
        let entries: Vec<_> = std::fs::read_dir(thumbnail_root.path().join("x-large")).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(entries, ["09136648df6f3a764f19a57d09232796.png"]);

        // Writing again replaces the file.
        write_thumbnail(thumbnail_root.path(), Bucket::XLarge, uri, b"newer png").unwrap();
        assert_eq!(std::fs::read(&written).unwrap(), b"newer png");
        assert_eq!(std::fs::read_dir(thumbnail_root.path().join("x-large")).unwrap().count(), 1);
    }

    #[test]
    fn removing_clears_every_bucket_and_the_recorded_failure() {
        let thumbnail_root = tempfile::tempdir().unwrap();
        let uri = "file:///home/tester/Immich/2024/03/IMG_1022.jpg";
        let other_uri = "file:///home/tester/Immich/2024/03/IMG_1023.jpg";
        let x_large = write_thumbnail(thumbnail_root.path(), Bucket::XLarge, uri, b"png").unwrap();
        let xx_large = write_thumbnail(thumbnail_root.path(), Bucket::XxLarge, uri, b"png").unwrap();
        let unrelated = write_thumbnail(thumbnail_root.path(), Bucket::XLarge, other_uri, b"png").unwrap();
        let failure_directory = thumbnail_root.path().join("fail/gnome-thumbnail-factory");
        std::fs::create_dir_all(&failure_directory).unwrap();
        let failure = failure_directory.join(thumbnail_file_name(uri));
        std::fs::write(&failure, b"1x1 png").unwrap();

        remove_thumbnails(thumbnail_root.path(), uri).unwrap();

        assert!(!x_large.exists());
        assert!(!xx_large.exists());
        assert!(!failure.exists());
        assert!(unrelated.exists());
        // Nothing left to remove is fine, as is a root that was never created.
        remove_thumbnails(thumbnail_root.path(), uri).unwrap();
        remove_thumbnails(&thumbnail_root.path().join("missing"), uri).unwrap();
    }

    #[test]
    fn thumbnails_under_the_mount_are_found_by_their_uri_and_removed() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let image = {
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4))
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .unwrap();
            bytes
        };
        let ours = "file:///home/me/Immich/Albums/2023%20-%20Trip/a.jpg";
        let theirs = "file:///home/me/Pictures/b.jpg";
        for uri in [ours, theirs] {
            let png = render_png(&image, Bucket::Large, uri, 1).unwrap();
            assert_eq!(uri_of_thumbnail(&png).as_deref(), Some(uri));
            write_thumbnail(root, Bucket::Large, uri, &png).unwrap();
        }
        assert_eq!(remove_thumbnails_under(root, "file:///home/me/Immich/"), 1);
        assert!(!thumbnail_path(root, Bucket::Large, ours).exists());
        assert!(thumbnail_path(root, Bucket::Large, theirs).exists(), "thumbnails outside the mount stay");
    }
}


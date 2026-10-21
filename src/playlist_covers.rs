//! Covers the user gives their playlists from their own computer. Each picture is cut square, made
//! at most `SIZE` pixels across and kept as a JPEG beside the saved state, under a random name; the
//! state only holds that name. A cover that no playlist names any more is removed.

use std::collections::HashSet;
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// The side of a cover kept on disk, in pixels.
const SIZE: u32 = 600;
/// A picture larger than this is not read.
const MAX_SOURCE_BYTES: u64 = 40 * 1024 * 1024;
/// The largest picture side and memory a decode may use.
const MAX_SIDE: u32 = 12_000;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
/// The quality of the JPEG kept.
const QUALITY: u8 = 88;

/// The folder the covers are kept in.
pub fn dir() -> Option<PathBuf> {
    Some(
        crate::app_dirs::config_dir()?
            .join("crabytube")
            .join("covers"),
    )
}

/// Whether `name` is the name of a cover this module made: a plain file name, never a path.
pub fn valid_name(name: &str) -> bool {
    name.len() <= 64
        && name.ends_with(".jpg")
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Where the cover of this name is, when the name is one this module made.
pub fn path(name: &str) -> Option<PathBuf> {
    valid_name(name).then(|| dir().map(|dir| dir.join(name)))?
}

/// Makes a cover from the picture at `source`: cut square from its middle, made at most `SIZE`
/// across, and saved as a new JPEG. Returns its name. Only PNG and JPEG pictures are read.
pub fn import(source: &Path) -> Result<String, String> {
    let size = std::fs::metadata(source)
        .map_err(|error| error.to_string())?
        .len();
    if size > MAX_SOURCE_BYTES {
        return Err("the picture is too large".to_owned());
    }
    let bytes = std::fs::read(source).map_err(|error| error.to_string())?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let picture = reader.decode().map_err(|error| error.to_string())?;
    let side = picture.width().min(picture.height());
    if side == 0 {
        return Err("the picture is empty".to_owned());
    }
    let square = picture.crop_imm(
        (picture.width() - side) / 2,
        (picture.height() - side) / 2,
        side,
        side,
    );
    let target = side.min(SIZE);
    let cover = square
        .resize_exact(target, target, image::imageops::FilterType::Lanczos3)
        .to_rgb8();
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, QUALITY)
        .encode_image(&cover)
        .map_err(|error| error.to_string())?;
    let dir = dir().ok_or_else(|| "there is no folder for covers".to_owned())?;
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let name = format!("{:016x}.jpg", rand::random::<u64>());
    // Written beside and moved in place, so a cover is never seen half written.
    let partial = dir.join(format!("{name}.part"));
    std::fs::write(&partial, &encoded).map_err(|error| error.to_string())?;
    std::fs::rename(&partial, dir.join(&name)).map_err(|error| {
        let _ = std::fs::remove_file(&partial);
        error.to_string()
    })?;
    Ok(name)
}

/// Removes the cover of this name, if there is one.
pub fn remove(name: &str) {
    if let Some(path) = path(name) {
        let _ = std::fs::remove_file(path);
    }
}

/// Whether the cover of this name is on disk.
pub fn exists(name: &str) -> bool {
    path(name).is_some_and(|path| path.is_file())
}

/// Removes every cover no playlist names, and pieces left by an import that did not finish.
pub fn remove_unused(kept: &HashSet<&str>) {
    let Some(dir) = dir() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let partial = name.ends_with(".jpg.part");
        if (valid_name(name) && !kept.contains(name)) || partial {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The tests share one covers folder: a test that removes unused covers must not see another's.
/// Every test that makes or removes covers holds this.
#[cfg(test)]
pub(crate) static TEST_FOLDER: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_test_folder() -> std::sync::MutexGuard<'static, ()> {
    TEST_FOLDER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: u32, height: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("crabytube-cover-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{width}x{height}.png"));
        image::RgbImage::from_pixel(width, height, image::Rgb([200, 30, 40]))
            .save(&path)
            .unwrap();
        path
    }

    #[test]
    fn a_picture_becomes_a_square_cover_no_larger_than_the_size() {
        let _folder = lock_test_folder();
        let name = import(&picture(1200, 800)).expect("a cover");
        assert!(valid_name(&name));
        let cover = image::open(path(&name).unwrap()).unwrap();
        assert_eq!((cover.width(), cover.height()), (SIZE, SIZE));
        // A small picture keeps its size.
        let small = import(&picture(90, 120)).unwrap();
        let cover = image::open(path(&small).unwrap()).unwrap();
        assert_eq!((cover.width(), cover.height()), (90, 90));
        remove(&name);
        remove(&small);
        assert!(!exists(&name));
    }

    #[test]
    fn what_is_not_a_picture_is_refused() {
        let _folder = lock_test_folder();
        let dir = std::env::temp_dir().join(format!("crabytube-cover-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = dir.join("notes.png");
        std::fs::write(&text, b"not a picture").unwrap();
        assert!(import(&text).is_err());
        assert!(import(&dir.join("missing.png")).is_err());
    }

    #[test]
    fn only_names_this_module_makes_are_paths() {
        assert!(valid_name("0123456789abcdef.jpg"));
        assert!(!valid_name("../state.json"));
        assert!(!valid_name("/etc/passwd.jpg"));
        assert!(!valid_name("a/b.jpg"));
        assert!(!valid_name(".hidden.jpg"));
        assert!(!valid_name("cover.png"));
        assert!(path("../x.jpg").is_none());
    }

    #[test]
    fn covers_no_playlist_names_are_removed() {
        let _folder = lock_test_folder();
        let kept = import(&picture(50, 50)).unwrap();
        let unused = import(&picture(60, 60)).unwrap();
        remove_unused(&HashSet::from([kept.as_str()]));
        assert!(exists(&kept));
        assert!(!exists(&unused));
        remove(&kept);
    }
}

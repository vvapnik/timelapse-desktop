//! Finding photos in a folder and determining when each one was taken.

use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::{FixedOffset, Local, NaiveDate, TimeZone};
use walkdir::WalkDir;

use crate::is_supported_image;

/// Where a frame's capture time came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSource {
    /// EXIF `DateTimeOriginal` (or `DateTime` when the former is missing).
    Exif,
    /// File modification time.
    FileModified,
}

/// A single photo that will become one video frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub path: PathBuf,
    /// Capture time in milliseconds since the Unix epoch.
    pub time_ms: i64,
    pub time_source: TimeSource,
}

/// Result of scanning a folder.
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Frames ordered by capture time, ties broken by path.
    pub frames: Vec<Frame>,
    /// Entries that could not be read (permission errors, broken links, …).
    pub skipped: Vec<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("folder '{0}' does not exist")]
    NotFound(PathBuf),
    #[error("'{0}' is not a folder")]
    NotADirectory(PathBuf),
    #[error("no photos found in '{0}' (supported: JPG, JPEG, PNG, BMP, TIFF)")]
    NoImages(PathBuf),
    #[error("cannot read folder '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Recursively finds supported photos in `folder` and orders them by capture time.
///
/// Capture time is EXIF `DateTimeOriginal` when present and valid, otherwise the
/// file modification time. Hidden files (names starting with `.`, e.g. macOS
/// `._IMG_0001.JPG` metadata files on FAT-formatted SD cards) are ignored.
pub fn scan_folder(folder: &Path) -> Result<ScanResult, ScanError> {
    let meta = fs::metadata(folder).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ScanError::NotFound(folder.to_path_buf()),
        _ => ScanError::Io {
            path: folder.to_path_buf(),
            source: e,
        },
    })?;
    if !meta.is_dir() {
        return Err(ScanError::NotADirectory(folder.to_path_buf()));
    }

    let mut result = ScanResult::default();
    for entry in WalkDir::new(folder).follow_links(true) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                if let Some(path) = err.path() {
                    if path == folder {
                        return Err(ScanError::Io {
                            path: folder.to_path_buf(),
                            source: err.into(),
                        });
                    }
                    result.skipped.push(path.to_path_buf());
                }
                continue;
            }
        };
        let path = entry.path();
        if !entry.file_type().is_file() || is_hidden(path) || !is_supported_image(path) {
            continue;
        }
        match frame_time(path) {
            Some((time_ms, time_source)) => result.frames.push(Frame {
                path: path.to_path_buf(),
                time_ms,
                time_source,
            }),
            None => result.skipped.push(path.to_path_buf()),
        }
    }

    if result.frames.is_empty() {
        return Err(ScanError::NoImages(folder.to_path_buf()));
    }
    sort_frames(&mut result.frames);
    Ok(result)
}

/// Orders frames by capture time; frames taken at the same time are ordered by path.
pub fn sort_frames(frames: &mut [Frame]) {
    frames.sort_by(|a, b| a.time_ms.cmp(&b.time_ms).then_with(|| a.path.cmp(&b.path)));
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

/// Capture time of a photo: EXIF if available, else the file modification time.
/// Returns `None` only if the file metadata cannot be read at all.
pub fn frame_time(path: &Path) -> Option<(i64, TimeSource)> {
    if let Some(ms) = exif_time_ms(path) {
        return Some((ms, TimeSource::Exif));
    }
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let ms = match modified.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).ok()?,
        Err(e) => -i64::try_from(e.duration().as_millis()).ok()?,
    };
    Some((ms, TimeSource::FileModified))
}

/// Reads EXIF `DateTimeOriginal` (falling back to `DateTime`) with optional
/// sub-seconds and UTC offset. Without an offset the time is taken as local time
/// of this computer, which is what cameras normally record.
fn exif_time_ms(path: &Path) -> Option<i64> {
    use exif::{In, Tag, Value};

    let file = File::open(path).ok()?;
    let exif = exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok()?;

    let ascii = |tag: Tag| -> Option<Vec<u8>> {
        match &exif.get_field(tag, In::PRIMARY)?.value {
            Value::Ascii(v) => v.first().cloned(),
            _ => None,
        }
    };

    let (raw, subsec_tag, offset_tag) = if let Some(raw) = ascii(Tag::DateTimeOriginal) {
        (raw, Tag::SubSecTimeOriginal, Tag::OffsetTimeOriginal)
    } else {
        (ascii(Tag::DateTime)?, Tag::SubSecTime, Tag::OffsetTime)
    };

    let mut dt = exif::DateTime::from_ascii(&raw).ok()?;
    if let Some(subsec) = ascii(subsec_tag) {
        let _ = dt.parse_subsec(&subsec);
    }
    if let Some(offset) = ascii(offset_tag) {
        let _ = dt.parse_offset(&offset);
    }

    let naive = NaiveDate::from_ymd_opt(dt.year.into(), dt.month.into(), dt.day.into())?
        .and_hms_nano_opt(
            dt.hour.into(),
            dt.minute.into(),
            dt.second.into(),
            dt.nanosecond.unwrap_or(0),
        )?;
    let time = match dt.offset {
        Some(minutes) => FixedOffset::east_opt(i32::from(minutes) * 60)?
            .from_local_datetime(&naive)
            .single()?
            .timestamp_millis(),
        None => Local
            .from_local_datetime(&naive)
            .earliest()?
            .timestamp_millis(),
    };
    Some(time)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn set_mtime(path: &Path, secs: u64) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    fn touch(dir: &Path, name: &str, mtime_secs: u64) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, b"not really an image").unwrap();
        set_mtime(&path, mtime_secs);
        path
    }

    #[test]
    fn missing_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = scan_folder(&dir.path().join("nope")).unwrap_err();
        assert!(matches!(err, ScanError::NotFound(_)), "{err}");
    }

    #[test]
    fn file_instead_of_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = touch(dir.path(), "a.jpg", 1_000);
        let err = scan_folder(&file).unwrap_err();
        assert!(matches!(err, ScanError::NotADirectory(_)), "{err}");
    }

    #[test]
    fn empty_folder_has_no_images() {
        let dir = tempfile::tempdir().unwrap();
        let err = scan_folder(dir.path()).unwrap_err();
        assert!(matches!(err, ScanError::NoImages(_)), "{err}");
    }

    #[test]
    fn folder_with_only_other_files_has_no_images() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "clip.mp4", 1_000);
        touch(dir.path(), "notes.txt", 1_000);
        touch(dir.path(), "._IMG_0001.JPG", 1_000);
        let err = scan_folder(dir.path()).unwrap_err();
        assert!(matches!(err, ScanError::NoImages(_)), "{err}");
    }

    #[test]
    fn finds_images_recursively_and_skips_hidden_and_other_files() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "100CANON/IMG_0001.JPG", 1_000);
        touch(dir.path(), "101CANON/img_0002.tiff", 1_010);
        touch(dir.path(), "a.Png", 1_020);
        touch(dir.path(), "b.bmp", 1_030);
        touch(dir.path(), "c.jpeg", 1_040);
        touch(dir.path(), "d.TIF", 1_050);
        touch(dir.path(), "100CANON/._IMG_0001.JPG", 1_000);
        touch(dir.path(), "video.mp4", 1_000);

        let result = scan_folder(dir.path()).unwrap();
        let names: Vec<_> = result
            .frames
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            names,
            [
                "IMG_0001.JPG",
                "img_0002.tiff",
                "a.Png",
                "b.bmp",
                "c.jpeg",
                "d.TIF"
            ]
        );
        assert!(result.skipped.is_empty());
    }

    #[test]
    fn broken_image_falls_back_to_file_time() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "broken.jpg", 1_234);
        let result = scan_folder(dir.path()).unwrap();
        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.frames[0].time_ms, 1_234_000);
        assert_eq!(result.frames[0].time_source, TimeSource::FileModified);
    }

    #[test]
    fn orders_by_time_then_by_path() {
        let dir = tempfile::tempdir().unwrap();
        // Names deliberately out of time order.
        touch(dir.path(), "c.jpg", 1_000);
        touch(dir.path(), "a.jpg", 2_000);
        touch(dir.path(), "b2.jpg", 1_500);
        touch(dir.path(), "b1.jpg", 1_500);
        let result = scan_folder(dir.path()).unwrap();
        let names: Vec<_> = result
            .frames
            .iter()
            .map(|f| f.path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["c.jpg", "b1.jpg", "b2.jpg", "a.jpg"]);
    }

    #[test]
    fn reads_exif_capture_time_with_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exif.jpg");
        fs::write(
            &path,
            jpeg_with_exif(b"2026:05:01 12:00:00\0", Some(b"+02:00\0")),
        )
        .unwrap();
        // Make mtime very different to prove EXIF wins.
        set_mtime(&path, 0);

        let (ms, source) = frame_time(&path).unwrap();
        assert_eq!(source, TimeSource::Exif);
        // 2026-05-01 10:00:00 UTC
        assert_eq!(ms, 1_777_629_600_000);
    }

    #[test]
    fn invalid_exif_date_falls_back_to_file_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exif.jpg");
        fs::write(&path, jpeg_with_exif(b"0000:00:00 00:00:00\0", None)).unwrap();
        set_mtime(&path, 5_000);

        assert_eq!(
            frame_time(&path).unwrap(),
            (5_000_000, TimeSource::FileModified)
        );
    }

    /// Builds a minimal JPEG (SOI + APP1 Exif + EOI) whose Exif IFD holds
    /// `DateTimeOriginal` and optionally `OffsetTimeOriginal`.
    fn jpeg_with_exif(datetime: &[u8; 20], offset: Option<&[u8; 7]>) -> Vec<u8> {
        // Little-endian TIFF: header, IFD0 with one ExifIFD pointer, Exif IFD.
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II*\0");
        tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset

        // IFD0 at 8: 1 entry (12 bytes) + next IFD (4) = 2 + 12 + 4 = 18 bytes.
        let exif_ifd_offset: u32 = 8 + 18;
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x8769u16.to_le_bytes()); // ExifIFDPointer
        tiff.extend_from_slice(&4u16.to_le_bytes()); // LONG
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&exif_ifd_offset.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());

        let entries: u16 = if offset.is_some() { 2 } else { 1 };
        let ifd_len = 2 + 12 * u32::from(entries) + 4;
        let data_offset = exif_ifd_offset + ifd_len;
        tiff.extend_from_slice(&entries.to_le_bytes());
        // DateTimeOriginal: ASCII, 20 bytes, stored at data_offset.
        tiff.extend_from_slice(&0x9003u16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&20u32.to_le_bytes());
        tiff.extend_from_slice(&data_offset.to_le_bytes());
        if offset.is_some() {
            // OffsetTimeOriginal: ASCII, 7 bytes, stored after the date.
            tiff.extend_from_slice(&0x9011u16.to_le_bytes());
            tiff.extend_from_slice(&2u16.to_le_bytes());
            tiff.extend_from_slice(&7u32.to_le_bytes());
            tiff.extend_from_slice(&(data_offset + 20).to_le_bytes());
        }
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(datetime);
        if let Some(offset) = offset {
            tiff.extend_from_slice(offset);
        }

        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend_from_slice(&u16::try_from(app1.len() + 2).unwrap().to_be_bytes());
        jpeg.extend_from_slice(&app1);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }
}

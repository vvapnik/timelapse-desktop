//! Core timelapse logic, independent of the GUI.
//!
//! Ports the behaviour of `create_timelapse.sh` from
//! [timelapse_from_photos](https://github.com/vvapnik/timelapse_from_photos):
//! find photos, order them by capture time, split them into sequences at long
//! pauses, and name the output files. It must not depend on Tauri so that a CLI
//! can reuse it later.

pub mod ffmpeg;
pub mod naming;
pub mod render;
pub mod scan;
pub mod sequence;

use std::path::Path;

pub use ffmpeg::{FfmpegError, Quality, VideoSettings, check_ffmpeg};
pub use naming::{OutputFormat, default_output_paths};
pub use render::{CancelToken, RenderError, RenderEvent, RenderTask, render};
pub use scan::{Frame, ScanError, ScanResult, TimeSource, scan_folder};
pub use sequence::{DEFAULT_GAP_SECONDS, Sequence, split_into_sequences};

/// Image file extensions accepted as timelapse frames (lowercase, without dot).
/// Matches `create_timelapse.sh`.
pub const SUPPORTED_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "bmp", "tif", "tiff"];

/// Version of the core crate.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns `true` if the path has a supported image extension (case-insensitive).
pub fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| ext.eq_ignore_ascii_case(supported))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_supported_extensions_case_insensitively() {
        for name in ["a.jpg", "b.JPEG", "c.Png", "d.bmp", "e.TIF", "f.tiff"] {
            assert!(is_supported_image(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn rejects_other_files() {
        for name in ["a.mp4", "b.txt", "noext", ".jpg", "dir/"] {
            assert!(!is_supported_image(Path::new(name)), "{name}");
        }
    }
}

//! End-to-end render tests against a real FFmpeg.
//!
//! FFmpeg is taken from the `FFMPEG` environment variable, else `ffmpeg` on
//! PATH. Tests are skipped (pass with a note) when no usable FFmpeg is found.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use timelapse_core::{
    CancelToken, OutputFormat, Quality, RenderError, RenderEvent, RenderTask, VideoSettings,
    check_ffmpeg, default_output_paths, render, scan_folder, split_into_sequences,
};

fn ffmpeg() -> Option<PathBuf> {
    let path = std::env::var_os("FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    match check_ffmpeg(&path) {
        Ok(_) => Some(path),
        Err(err) => {
            eprintln!("skipping: {err}");
            None
        }
    }
}

/// Writes `count` JPEG test frames named IMG_0001.JPG… into `dir`, with
/// modification times `start + offsets[i]` seconds.
fn make_photos(ffmpeg: &Path, dir: &Path, offsets: &[u64], size: &str) {
    let status = Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!("testsrc=size={size}:rate=1"))
        .args(["-frames:v", &offsets.len().to_string(), "-q:v", "5"])
        .arg(dir.join("IMG_%04d.JPG"))
        .status()
        .unwrap();
    assert!(status.success());
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_767_225_600);
    for (i, offset) in offsets.iter().enumerate() {
        let path = dir.join(format!("IMG_{:04}.JPG", i + 1));
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(start + Duration::from_secs(*offset))
            .unwrap();
    }
}

/// Counts video frames in a file using FFmpeg itself.
fn count_frames(ffmpeg: &Path, video: &Path) -> u64 {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostats", "-progress", "pipe:1", "-i"])
        .arg(video)
        .args(["-map", "0:v", "-c", "copy", "-f", "null", "-"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("frame="))
        .filter_map(|v| v.trim().parse().ok())
        .next_back()
        .unwrap_or(0)
}

fn tasks_for(folder: &Path, gap: u64, format: OutputFormat) -> Vec<RenderTask> {
    let scan = scan_folder(folder).unwrap();
    let sequences = split_into_sequences(scan.frames, gap);
    let outputs = default_output_paths(folder, folder, format, sequences.len());
    sequences
        .into_iter()
        .zip(outputs)
        .map(|(sequence, output)| RenderTask { sequence, output })
        .collect()
}

fn hidden_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect()
}

#[test]
fn renders_each_sequence_with_one_frame_per_photo() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("garden");
    std::fs::create_dir(&folder).unwrap();
    // 7 photos, then a 10-minute pause, then 4 photos; odd size on purpose.
    make_photos(
        &ffmpeg,
        &folder,
        &[0, 10, 20, 30, 40, 50, 60, 660, 670, 680, 690],
        "161x121",
    );

    let tasks = tasks_for(&folder, 120, OutputFormat::Mp4);
    assert_eq!(tasks.len(), 2);
    let settings = VideoSettings {
        fps: 12,
        quality: Quality::Low,
        format: OutputFormat::Mp4,
    };

    let mut events = Vec::new();
    let outputs = render(&ffmpeg, &tasks, &settings, &CancelToken::new(), |e| {
        events.push(e)
    })
    .unwrap();

    assert_eq!(
        outputs,
        [
            folder.join("garden_timelapse_part1.mp4"),
            folder.join("garden_timelapse_part2.mp4")
        ]
    );
    assert_eq!(count_frames(&ffmpeg, &outputs[0]), 7);
    assert_eq!(count_frames(&ffmpeg, &outputs[1]), 4);
    assert!(hidden_files(&folder).is_empty(), "no partial files left");

    // Overall progress never goes backwards and ends at 100.
    let overall: Vec<f64> = events
        .iter()
        .filter_map(|e| match e {
            RenderEvent::Progress {
                overall_percent, ..
            } => Some(*overall_percent),
            _ => None,
        })
        .collect();
    assert!(overall.windows(2).all(|w| w[0] <= w[1]), "{overall:?}");
    assert_eq!(overall.last(), Some(&100.0));
    let finished = events
        .iter()
        .filter(|e| matches!(e, RenderEvent::SequenceFinished { .. }))
        .count();
    assert_eq!(finished, 2);
}

#[test]
fn every_format_renders() {
    let Some(ffmpeg) = ffmpeg() else { return };
    for format in [OutputFormat::Mp4, OutputFormat::Mov, OutputFormat::Avi] {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("photos");
        std::fs::create_dir(&folder).unwrap();
        make_photos(&ffmpeg, &folder, &[0, 5, 10], "64x48");
        let tasks = tasks_for(&folder, 120, format);
        let settings = VideoSettings {
            format,
            ..VideoSettings::default()
        };
        let outputs = render(&ffmpeg, &tasks, &settings, &CancelToken::new(), |_| {}).unwrap();
        assert_eq!(
            outputs,
            [folder.join(format!("photos_timelapse.{}", format.extension()))]
        );
        assert_eq!(count_frames(&ffmpeg, &outputs[0]), 3, "{format:?}");
    }
}

#[test]
fn cancel_stops_ffmpeg_and_removes_partial_file() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("long");
    std::fs::create_dir(&folder).unwrap();
    let offsets: Vec<u64> = (0..400).map(|i| i * 5).collect();
    make_photos(&ffmpeg, &folder, &offsets, "1280x720");

    let tasks = tasks_for(&folder, 120, OutputFormat::Mp4);
    let settings = VideoSettings {
        quality: Quality::High,
        ..VideoSettings::default()
    };
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let mut started = false;
    let err = render(&ffmpeg, &tasks, &settings, &cancel, |e| {
        if matches!(e, RenderEvent::SequenceStarted { .. }) {
            started = true;
            // Cancel from another thread shortly after FFmpeg starts.
            let token = token.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                token.cancel();
            });
        }
    })
    .unwrap_err();

    assert!(started);
    assert!(matches!(err, RenderError::Cancelled), "{err}");
    assert!(!folder.join("long_timelapse.mp4").exists());
    assert!(hidden_files(&folder).is_empty(), "partial file removed");
}

#[test]
fn existing_output_survives_a_cancelled_render() {
    let Some(ffmpeg) = ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("keep");
    std::fs::create_dir(&folder).unwrap();
    make_photos(&ffmpeg, &folder, &[0, 5, 10], "64x48");
    let existing = folder.join("keep_timelapse.mp4");
    std::fs::write(&existing, b"previous video").unwrap();

    let cancel = CancelToken::new();
    cancel.cancel();
    let tasks = tasks_for(&folder, 120, OutputFormat::Mp4);
    let err = render(&ffmpeg, &tasks, &VideoSettings::default(), &cancel, |_| {}).unwrap_err();
    assert!(matches!(err, RenderError::Cancelled));
    assert_eq!(std::fs::read(&existing).unwrap(), b"previous video");
}

#[test]
fn missing_ffmpeg_fails_with_instructions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
    let tasks = tasks_for(dir.path(), 120, OutputFormat::Mp4);
    let err = render(
        Path::new("/nonexistent/ffmpeg"),
        &tasks,
        &VideoSettings::default(),
        &CancelToken::new(),
        |_| {},
    )
    .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("was not found"), "{message}");
    assert!(message.contains("Settings"), "{message}");
}

//! Talking to the FFmpeg executable: checking it, building arguments and
//! parsing its progress output.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::naming::OutputFormat;
use crate::scan::Frame;

/// Hint shown with every "FFmpeg is unavailable" error.
pub const FFMPEG_HELP: &str = "Reinstall Timelapse Desktop to restore the bundled FFmpeg, \
or set the path to your own FFmpeg in Settings (macOS: `brew install ffmpeg`, \
Windows: `winget install ffmpeg`, Linux: `sudo apt install ffmpeg`).";

#[derive(Debug, thiserror::Error)]
pub enum FfmpegError {
    #[error("FFmpeg was not found at '{path}'. {FFMPEG_HELP}")]
    NotFound { path: PathBuf },
    #[error("FFmpeg at '{path}' could not be started: {source}. {FFMPEG_HELP}")]
    CannotRun {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("'{path}' does not look like a working FFmpeg. {FFMPEG_HELP}")]
    NotFfmpeg { path: PathBuf },
    #[error("FFmpeg at '{path}' has no H.264 encoder (libx264). {FFMPEG_HELP}")]
    NoX264 { path: PathBuf },
}

/// Checks that `path` is a runnable FFmpeg with libx264 and returns its version line.
pub fn check_ffmpeg(path: &Path) -> Result<String, FfmpegError> {
    let run = |args: &[&str]| {
        Command::new(path)
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|source| match source.kind() {
                ErrorKind::NotFound => FfmpegError::NotFound {
                    path: path.to_path_buf(),
                },
                _ => FfmpegError::CannotRun {
                    path: path.to_path_buf(),
                    source,
                },
            })
    };

    let version = run(&["-hide_banner", "-version"])?;
    let version_line = String::from_utf8_lossy(&version.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if !version.status.success() || !version_line.starts_with("ffmpeg version") {
        return Err(FfmpegError::NotFfmpeg {
            path: path.to_path_buf(),
        });
    }

    let encoders = run(&["-hide_banner", "-encoders"])?;
    if !String::from_utf8_lossy(&encoders.stdout).contains("libx264") {
        return Err(FfmpegError::NoX264 {
            path: path.to_path_buf(),
        });
    }
    Ok(version_line)
}

/// Video quality presets, same as the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    #[default]
    Medium,
    High,
}

impl Quality {
    /// x264 constant rate factor.
    pub fn crf(self) -> u8 {
        match self {
            Quality::Low => 28,
            Quality::Medium => 23,
            Quality::High => 18,
        }
    }

    /// x264 speed preset.
    pub fn preset(self) -> &'static str {
        match self {
            Quality::Low => "fast",
            Quality::Medium => "medium",
            Quality::High => "slow",
        }
    }
}

pub const MIN_FPS: u32 = 1;
pub const MAX_FPS: u32 = 120;
pub const DEFAULT_FPS: u32 = 30;

/// Encoding settings for every sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSettings {
    pub fps: u32,
    pub quality: Quality,
    pub format: OutputFormat,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            fps: DEFAULT_FPS,
            quality: Quality::default(),
            format: OutputFormat::default(),
        }
    }
}

impl VideoSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(MIN_FPS..=MAX_FPS).contains(&self.fps) {
            return Err(format!(
                "FPS must be between {MIN_FPS} and {MAX_FPS}, got {}",
                self.fps
            ));
        }
        Ok(())
    }
}

/// Builds an FFmpeg concat-demuxer list for the given frames.
pub fn concat_list(frames: &[Frame]) -> String {
    let mut list = String::from("ffconcat version 1.0\n");
    for frame in frames {
        // Inside single quotes everything is literal except `'`, which is
        // written as `'\''` (close quote, escaped quote, reopen).
        let path = frame.path.to_string_lossy().replace('\'', r"'\''");
        list.push_str("file '");
        list.push_str(&path);
        list.push_str("'\n");
    }
    list
}

/// FFmpeg arguments that encode the frames listed in `list_file` into `output`.
///
/// Unlike the script, each photo becomes exactly one video frame: timestamps
/// are rewritten to `N / fps`, so no frames are duplicated or dropped. Odd
/// image dimensions are rounded down to even, which `yuv420p` requires.
pub fn render_args(list_file: &Path, output: &Path, settings: &VideoSettings) -> Vec<OsString> {
    let fps = settings.fps.to_string();
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-loglevel",
        "error",
        "-nostats",
        "-progress",
        "pipe:1",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(list_file.into());
    args.extend(
        [
            "-vf".to_owned(),
            format!("setpts=N/({fps}*TB),scale=trunc(iw/2)*2:trunc(ih/2)*2"),
            "-r".to_owned(),
            fps,
            "-c:v".to_owned(),
            "libx264".to_owned(),
            "-pix_fmt".to_owned(),
            "yuv420p".to_owned(),
            "-crf".to_owned(),
            settings.quality.crf().to_string(),
            "-preset".to_owned(),
            settings.quality.preset().to_owned(),
        ]
        .map(OsString::from),
    );
    if matches!(settings.format, OutputFormat::Mp4 | OutputFormat::Mov) {
        args.extend(["-movflags", "+faststart"].map(OsString::from));
    }
    args.push(output.into());
    args
}

/// One parsed line of `-progress` output that matters to us.
#[derive(Debug, PartialEq, Eq)]
pub enum ProgressLine {
    /// Frames encoded so far.
    Frame(u64),
    /// Encoding finished.
    End,
}

/// Parses one `key=value` line of FFmpeg `-progress` output.
pub fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    let (key, value) = line.trim().split_once('=')?;
    match key {
        "frame" => value.trim().parse().ok().map(ProgressLine::Frame),
        "progress" if value.trim() == "end" => Some(ProgressLine::End),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::TimeSource;

    fn args_to_strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn quality_matches_script() {
        assert_eq!((Quality::Low.crf(), Quality::Low.preset()), (28, "fast"));
        assert_eq!(
            (Quality::Medium.crf(), Quality::Medium.preset()),
            (23, "medium")
        );
        assert_eq!((Quality::High.crf(), Quality::High.preset()), (18, "slow"));
    }

    #[test]
    fn fps_is_validated() {
        let mut s = VideoSettings::default();
        assert_eq!(s.fps, 30);
        assert!(s.validate().is_ok());
        s.fps = 0;
        assert!(s.validate().is_err());
        s.fps = 121;
        assert!(s.validate().is_err());
        s.fps = 120;
        assert!(s.validate().is_ok());
    }

    #[test]
    fn concat_list_quotes_paths() {
        let frames = [
            Frame {
                path: PathBuf::from("/photos/a.jpg"),
                time_ms: 0,
                time_source: TimeSource::FileModified,
            },
            Frame {
                path: PathBuf::from("/photos/it's here.jpg"),
                time_ms: 1,
                time_source: TimeSource::FileModified,
            },
        ];
        assert_eq!(
            concat_list(&frames),
            "ffconcat version 1.0\nfile '/photos/a.jpg'\nfile '/photos/it'\\''s here.jpg'\n"
        );
    }

    #[test]
    fn render_args_use_script_codec_settings() {
        let settings = VideoSettings {
            fps: 24,
            quality: Quality::High,
            format: OutputFormat::Mov,
        };
        let args = args_to_strings(&render_args(
            Path::new("/tmp/list.txt"),
            Path::new("/out/v.mov"),
            &settings,
        ));
        let joined = args.join(" ");
        assert!(joined.contains("-f concat -safe 0 -i /tmp/list.txt"));
        assert!(joined.contains("setpts=N/(24*TB)"));
        assert!(joined.contains("-r 24 -c:v libx264 -pix_fmt yuv420p -crf 18 -preset slow"));
        assert!(joined.contains("-movflags +faststart"));
        assert!(joined.contains("-progress pipe:1"));
        assert_eq!(args.last().unwrap(), "/out/v.mov");
    }

    #[test]
    fn avi_has_no_movflags() {
        let settings = VideoSettings {
            format: OutputFormat::Avi,
            ..VideoSettings::default()
        };
        let args = args_to_strings(&render_args(Path::new("l"), Path::new("o.avi"), &settings));
        assert!(!args.iter().any(|a| a == "-movflags"));
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(
            parse_progress_line("frame=42"),
            Some(ProgressLine::Frame(42))
        );
        assert_eq!(
            parse_progress_line("frame= 7\n"),
            Some(ProgressLine::Frame(7))
        );
        assert_eq!(parse_progress_line("progress=end"), Some(ProgressLine::End));
        assert_eq!(parse_progress_line("progress=continue"), None);
        assert_eq!(parse_progress_line("fps=30.0"), None);
        assert_eq!(parse_progress_line("garbage"), None);
    }

    #[test]
    fn missing_ffmpeg_is_reported_with_help() {
        let err = check_ffmpeg(Path::new("/definitely/not/here/ffmpeg")).unwrap_err();
        assert!(matches!(err, FfmpegError::NotFound { .. }), "{err}");
        assert!(err.to_string().contains("Settings"));
    }
}

//! Rendering sequences into video files with progress reporting and cancel.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Serialize;

use crate::ffmpeg::{
    FfmpegError, ProgressLine, VideoSettings, check_ffmpeg, concat_list, parse_progress_line,
    render_args,
};
use crate::sequence::Sequence;

/// Shared flag that stops a running render. Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One sequence to render and where to put it.
#[derive(Debug, Clone)]
pub struct RenderTask {
    pub sequence: Sequence,
    pub output: PathBuf,
}

/// Progress notifications sent while rendering.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RenderEvent {
    #[serde(rename_all = "camelCase")]
    SequenceStarted {
        index: usize,
        count: usize,
        frames: usize,
        output: PathBuf,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        index: usize,
        /// 0–100 for the current sequence.
        sequence_percent: f64,
        /// 0–100 for the whole render.
        overall_percent: f64,
    },
    #[serde(rename_all = "camelCase")]
    SequenceFinished { index: usize, output: PathBuf },
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error(transparent)]
    Ffmpeg(#[from] FfmpegError),
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
    #[error("nothing to render")]
    Empty,
    #[error("rendering was cancelled")]
    Cancelled,
    #[error("FFmpeg failed on sequence {index}: {message}")]
    EncodeFailed { index: usize, message: String },
    #[error("cannot write '{path}': {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Renders each task into its own file, one after another.
///
/// The video is first written to a hidden temporary file next to the target
/// and renamed when FFmpeg succeeds, so a cancelled or failed render never
/// leaves a half-written file (or destroys an existing one).
pub fn render(
    ffmpeg: &Path,
    tasks: &[RenderTask],
    settings: &VideoSettings,
    cancel: &CancelToken,
    mut on_event: impl FnMut(RenderEvent),
) -> Result<Vec<PathBuf>, RenderError> {
    settings.validate().map_err(RenderError::InvalidSettings)?;
    let total_frames: usize = tasks.iter().map(|t| t.sequence.len()).sum();
    if total_frames == 0 {
        return Err(RenderError::Empty);
    }
    check_ffmpeg(ffmpeg)?;

    let mut done_frames = 0usize;
    let mut outputs = Vec::with_capacity(tasks.len());
    for (index, task) in tasks.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(RenderError::Cancelled);
        }
        let frames = task.sequence.len();
        on_event(RenderEvent::SequenceStarted {
            index,
            count: tasks.len(),
            frames,
            output: task.output.clone(),
        });
        render_one(ffmpeg, index, task, settings, cancel, |encoded| {
            let encoded = encoded.min(frames as u64) as usize;
            on_event(RenderEvent::Progress {
                index,
                sequence_percent: percent(encoded, frames),
                overall_percent: percent(done_frames + encoded, total_frames),
            });
        })?;
        done_frames += frames;
        on_event(RenderEvent::Progress {
            index,
            sequence_percent: 100.0,
            overall_percent: percent(done_frames, total_frames),
        });
        on_event(RenderEvent::SequenceFinished {
            index,
            output: task.output.clone(),
        });
        outputs.push(task.output.clone());
    }
    Ok(outputs)
}

fn percent(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 100.0;
    }
    (part as f64 * 100.0 / whole as f64).clamp(0.0, 100.0)
}

/// Hidden temporary path next to `output`, keeping its extension so FFmpeg
/// picks the right container.
fn partial_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match output.extension() {
        Some(ext) => format!(".{stem}.partial.{}", ext.to_string_lossy()),
        None => format!(".{stem}.partial"),
    };
    output.with_file_name(name)
}

fn render_one(
    ffmpeg: &Path,
    index: usize,
    task: &RenderTask,
    settings: &VideoSettings,
    cancel: &CancelToken,
    mut on_frames: impl FnMut(u64),
) -> Result<(), RenderError> {
    let io_err = |path: &Path| {
        let path = path.to_path_buf();
        move |source| RenderError::Io { path, source }
    };

    let mut list = tempfile::Builder::new()
        .prefix("timelapse-")
        .suffix(".ffconcat")
        .tempfile()
        .map_err(io_err(Path::new("temporary file")))?;
    list.write_all(concat_list(&task.sequence.frames).as_bytes())
        .and_then(|()| list.flush())
        .map_err(io_err(list.path()))?;

    if let Some(dir) = task.output.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
    }
    let partial = partial_path(&task.output);

    let mut child = Command::new(ffmpeg)
        .args(render_args(list.path(), &partial, settings))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| FfmpegError::CannotRun {
            path: ffmpeg.to_path_buf(),
            source,
        })?;

    let result = drive(&mut child, cancel, &mut on_frames);
    match result {
        Ok(stderr) => {
            let status = child.wait().map_err(io_err(ffmpeg))?;
            if !status.success() {
                let _ = fs::remove_file(&partial);
                return Err(RenderError::EncodeFailed {
                    index,
                    message: last_lines(&stderr, 5)
                        .unwrap_or_else(|| format!("exit status {status}")),
                });
            }
            fs::rename(&partial, &task.output).map_err(io_err(&task.output))?;
            Ok(())
        }
        Err(err) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(&partial);
            Err(err)
        }
    }
}

/// Reads progress until FFmpeg closes stdout, checking for cancel in between.
/// Returns collected stderr.
fn drive(
    child: &mut Child,
    cancel: &CancelToken,
    on_frames: &mut impl FnMut(u64),
) -> Result<String, RenderError> {
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");

    let stderr_reader = thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf);
        buf
    });

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Some(parsed) = parse_progress_line(&line)
                && tx.send(parsed).is_err()
            {
                break;
            }
        }
    });

    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            return Err(RenderError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ProgressLine::Frame(n)) => on_frames(n),
            Ok(ProgressLine::End) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(stderr_reader.join().unwrap_or_default())
}

fn last_lines(text: &str, n: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    Some(lines[lines.len().saturating_sub(n)..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_file_is_hidden_and_keeps_extension() {
        assert_eq!(
            partial_path(Path::new("/out/garden_timelapse_part2.mp4")),
            PathBuf::from("/out/.garden_timelapse_part2.partial.mp4")
        );
    }

    #[test]
    fn percent_is_clamped() {
        assert_eq!(percent(0, 10), 0.0);
        assert_eq!(percent(5, 10), 50.0);
        assert_eq!(percent(12, 10), 100.0);
        assert_eq!(percent(0, 0), 100.0);
    }

    #[test]
    fn last_lines_skips_blank_lines() {
        assert_eq!(last_lines("a\n\nb\nc\n", 2).as_deref(), Some("b\nc"));
        assert_eq!(last_lines("\n \n", 2), None);
    }

    #[test]
    fn empty_render_is_an_error() {
        let err = render(
            Path::new("ffmpeg"),
            &[],
            &VideoSettings::default(),
            &CancelToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(err, RenderError::Empty));
    }
}

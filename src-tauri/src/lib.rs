use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use timelapse_core::{
    CancelToken, OutputFormat, RenderError, RenderEvent, RenderTask, VideoSettings, check_ffmpeg,
    default_output_paths, render, scan_folder, split_into_sequences,
};

/// Event carrying a [`RenderEvent`] while a render runs.
const RENDER_PROGRESS_EVENT: &str = "render-progress";
/// Event sent once when a render ends (success, failure or cancel).
const RENDER_FINISHED_EVENT: &str = "render-finished";

/// Cancel handle of the render in progress, if any.
#[derive(Default)]
struct RenderState(Mutex<Option<CancelToken>>);

/// Basic information about the core, shown on the placeholder screen.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreInfo {
    version: &'static str,
    supported_extensions: &'static [&'static str],
}

#[tauri::command]
fn core_info() -> CoreInfo {
    CoreInfo {
        version: timelapse_core::VERSION,
        supported_extensions: timelapse_core::SUPPORTED_EXTENSIONS,
    }
}

/// The bundled sidecar sits next to the app executable (Tauri strips the
/// target-triple suffix when bundling and in `tauri dev`).
fn bundled_ffmpeg() -> PathBuf {
    let name = format!("ffmpeg{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// A custom path from settings wins over the bundled FFmpeg.
fn resolve_ffmpeg(custom_path: Option<String>) -> PathBuf {
    match custom_path.map(|p| p.trim().to_owned()) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => bundled_ffmpeg(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FfmpegStatus {
    path: PathBuf,
    version: String,
}

#[tauri::command]
async fn ffmpeg_status(custom_path: Option<String>) -> Result<FfmpegStatus, String> {
    let path = resolve_ffmpeg(custom_path);
    let version = check_ffmpeg(&path).map_err(|e| e.to_string())?;
    Ok(FfmpegStatus { path, version })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SequenceSummary {
    frames: usize,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    output: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanSummary {
    folder: PathBuf,
    total_frames: usize,
    exif_frames: usize,
    skipped: Vec<PathBuf>,
    sequences: Vec<SequenceSummary>,
}

/// Scans `folder` and splits it into render tasks with default output names.
fn plan_render(
    folder: &Path,
    gap_seconds: u64,
    format: OutputFormat,
) -> Result<(Vec<RenderTask>, Vec<PathBuf>), String> {
    let scan = scan_folder(folder).map_err(|e| e.to_string())?;
    let sequences = split_into_sequences(scan.frames, gap_seconds);
    let outputs = default_output_paths(folder, folder, format, sequences.len());
    let tasks = sequences
        .into_iter()
        .zip(outputs)
        .map(|(sequence, output)| RenderTask { sequence, output })
        .collect();
    Ok((tasks, scan.skipped))
}

#[tauri::command]
async fn scan(
    folder: PathBuf,
    gap_seconds: u64,
    format: OutputFormat,
) -> Result<ScanSummary, String> {
    let (tasks, skipped) = plan_render(&folder, gap_seconds, format)?;
    let frames = tasks.iter().flat_map(|t| &t.sequence.frames);
    let total_frames = frames.clone().count();
    let exif_frames = frames
        .filter(|f| f.time_source == timelapse_core::TimeSource::Exif)
        .count();
    Ok(ScanSummary {
        folder,
        total_frames,
        exif_frames,
        skipped,
        sequences: tasks
            .into_iter()
            .map(|t| SequenceSummary {
                frames: t.sequence.len(),
                start_ms: t.sequence.start_ms(),
                end_ms: t.sequence.end_ms(),
                output: t.output,
            })
            .collect(),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderFinished {
    outputs: Vec<PathBuf>,
    cancelled: bool,
    error: Option<String>,
}

/// Starts rendering in the background. Progress arrives as `render-progress`
/// events and the outcome as one `render-finished` event.
#[tauri::command]
fn start_render(
    app: AppHandle,
    state: State<'_, RenderState>,
    folder: PathBuf,
    gap_seconds: u64,
    settings: VideoSettings,
    custom_ffmpeg: Option<String>,
) -> Result<(), String> {
    settings.validate()?;
    let cancel = {
        let mut current = state.0.lock().map_err(|e| e.to_string())?;
        if current.is_some() {
            return Err("A render is already running".into());
        }
        let token = CancelToken::new();
        *current = Some(token.clone());
        token
    };
    let ffmpeg = resolve_ffmpeg(custom_ffmpeg);

    std::thread::spawn(move || {
        let result = plan_render(&folder, gap_seconds, settings.format).and_then(|(tasks, _)| {
            render(&ffmpeg, &tasks, &settings, &cancel, |event: RenderEvent| {
                let _ = app.emit(RENDER_PROGRESS_EVENT, event);
            })
            .map_err(|e| match e {
                RenderError::Cancelled => String::new(),
                other => other.to_string(),
            })
        });
        let finished = match result {
            Ok(outputs) => RenderFinished {
                outputs,
                cancelled: false,
                error: None,
            },
            Err(message) => RenderFinished {
                outputs: Vec::new(),
                cancelled: message.is_empty(),
                error: (!message.is_empty()).then_some(message),
            },
        };
        if let Some(state) = app.try_state::<RenderState>()
            && let Ok(mut current) = state.0.lock()
        {
            *current = None;
        }
        let _ = app.emit(RENDER_FINISHED_EVENT, finished);
    });
    Ok(())
}

/// Asks the running render to stop; the partial file is removed by the core.
#[tauri::command]
fn cancel_render(state: State<'_, RenderState>) -> Result<bool, String> {
    let current = state.0.lock().map_err(|e| e.to_string())?;
    Ok(match current.as_ref() {
        Some(token) => {
            token.cancel();
            true
        }
        None => false,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(RenderState::default())
        .invoke_handler(tauri::generate_handler![
            core_info,
            ffmpeg_status,
            scan,
            start_render,
            cancel_render
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

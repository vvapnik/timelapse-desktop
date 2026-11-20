use serde::Serialize;

/// Basic information about the core, shown on the placeholder screen.
#[derive(Serialize)]
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![core_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

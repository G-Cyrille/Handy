use crate::managers::{
    history::{HistoryEntry, HistoryManager, PaginatedHistory},
    transcription_jobs::TranscriptionJobManager,
};
use crate::screenshot::CapturedScreenshot;
use log::warn;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State};

/// Load the screenshot persisted with an entry so a retry can reuse its visual
/// context. Missing or corrupt files degrade to text-only.
pub(crate) fn load_history_screenshot(
    history_manager: &HistoryManager,
    entry: &HistoryEntry,
) -> Option<CapturedScreenshot> {
    let path = history_manager.get_screenshot_file_path(entry)?;
    match std::fs::read(&path) {
        Ok(bytes) => {
            let mut screenshot = CapturedScreenshot::from_jpeg(bytes)?;
            screenshot.load_monitor_images(&path);
            Some(screenshot)
        }
        Err(err) => {
            warn!(
                "Screenshot for history entry {} is unavailable: {}",
                entry.id, err
            );
            None
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    history_manager
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_history_entry_saved(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_audio_file_path(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    file_name: String,
) -> Result<String, String> {
    let path = history_manager.get_audio_file_path(&file_name);
    path.to_str()
        .ok_or_else(|| "Invalid file path".to_string())
        .map(|s| s.to_string())
}

/// Stored panorama and individual monitors as JPEG data URLs.
/// Looked up by entry id so the frontend never names a file.
#[tauri::command]
#[specta::specta]
pub async fn get_history_screenshot(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<Vec<String>, String> {
    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {} not found", id))?;

    load_history_screenshot(&history_manager, &entry)
        .map(|screenshot| screenshot.history_data_urls())
        .ok_or_else(|| format!("History entry {} has no readable screenshot", id))
}

/// Copy decoded pixels, so other applications receive an image rather than a URL.
#[tauri::command]
#[specta::specta]
pub async fn copy_history_screenshot(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
    index: usize,
) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|err| err.to_string())?
        .ok_or("History entry not found")?;
    let screenshot =
        load_history_screenshot(&history_manager, &entry).ok_or("Screenshot unavailable")?;
    let jpeg = screenshot
        .history_image(index)
        .ok_or("Screenshot index out of range")?;
    let rgba = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
        .map_err(|err| err.to_string())?
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    app.clipboard()
        .write_image(&tauri::image::Image::new_owned(
            rgba.into_raw(),
            width,
            height,
        ))
        .map_err(|err| err.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_history_entry(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    jobs: State<'_, Arc<TranscriptionJobManager>>,
    id: i64,
) -> Result<(), String> {
    jobs.cancel(id);
    history_manager
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

/// Queues the entry to be transcribed again with the current model. Its
/// progress and outcome arrive as `TranscriptionJob` events.
#[tauri::command]
#[specta::specta]
pub fn retry_history_entry_transcription(jobs: State<'_, Arc<TranscriptionJobManager>>, id: i64) {
    jobs.retry(id);
}

/// Stops the entry's background transcription: a queued one is dropped, a
/// running one stops after the chunk it is on. The entry keeps the text it
/// had before.
#[tauri::command]
#[specta::specta]
pub fn cancel_history_entry_transcription(jobs: State<'_, Arc<TranscriptionJobManager>>, id: i64) {
    jobs.cancel(id);
}

/// Adds an audio or video file to History and queues its transcription.
/// Fails without adding anything if the file has no audio Handy can read.
#[tauri::command]
#[specta::specta]
pub async fn transcribe_file(
    jobs: State<'_, Arc<TranscriptionJobManager>>,
    path: String,
) -> Result<HistoryEntry, String> {
    let jobs = Arc::clone(&jobs);
    tauri::async_runtime::spawn_blocking(move || jobs.transcribe_file(PathBuf::from(path)))
        .await
        .map_err(|e| format!("Failed to read the file: {}", e))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn update_history_limit(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    limit: usize,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.history_limit = limit;
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_period(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: String,
) -> Result<(), String> {
    use crate::settings::RecordingRetentionPeriod;

    let retention_period = match period.as_str() {
        "never" => RecordingRetentionPeriod::Never,
        "preserve_limit" => RecordingRetentionPeriod::PreserveLimit,
        "days3" => RecordingRetentionPeriod::Days3,
        "weeks2" => RecordingRetentionPeriod::Weeks2,
        "months3" => RecordingRetentionPeriod::Months3,
        _ => return Err(format!("Invalid retention period: {}", period)),
    };

    let mut settings = crate::settings::get_settings(&app);
    settings.recording_retention_period = retention_period;
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}

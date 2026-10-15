use crate::meetings::capture::{self, CaptureSelection, CaptureSources};
use crate::meetings::{archive::Session, MeetingDetail, MeetingManager, MeetingSnapshot};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

fn manager(app: &AppHandle) -> Result<Arc<MeetingManager>, String> {
    app.try_state::<Arc<MeetingManager>>()
        .map(|state| state.inner().clone())
        .ok_or_else(|| "The meeting archive could not initialize; see the application log".into())
}

#[tauri::command]
#[specta::specta]
pub async fn list_meeting_sources() -> Result<CaptureSources, String> {
    tauri::async_runtime::spawn_blocking(capture::sources)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn get_meeting_snapshot(app: AppHandle) -> Result<MeetingSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.snapshot(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn start_meeting(
    app: AppHandle,
    selection: CaptureSelection,
    ai_enabled: bool,
) -> Result<Session, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.start(selection, ai_enabled))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn stop_meeting(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.stop())
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn get_meeting_detail(
    app: AppHandle,
    id: String,
    offset: u64,
    limit: u32,
) -> Result<MeetingDetail, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.detail(&id, offset, limit))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn set_meeting_ai(app: AppHandle, enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.set_ai(enabled))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn retry_meeting_asr(app: AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.store.retry_asr(&id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn export_meeting_audio(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.export(&id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub fn open_meeting_folder(app: AppHandle, id: String) -> Result<(), String> {
    let path = manager(&app)?.store.session_dir(&id)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

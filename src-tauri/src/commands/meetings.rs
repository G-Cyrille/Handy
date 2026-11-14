use crate::meetings::archive::{TranscriptFormat, TranscriptLabels};
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
pub async fn get_meeting_workspace(
    app: AppHandle,
    id: String,
) -> Result<crate::meetings::workspace::MeetingWorkspace, String> {
    manager(&app)?.workspace(&id)
}

#[derive(serde::Serialize, specta::Type)]
pub struct MeetingLibraryItem {
    pub session: Session,
    pub title: String,
    pub folder: String,
    pub summary: String,
}

#[tauri::command]
#[specta::specta]
pub async fn get_meeting_library(app: AppHandle) -> Result<Vec<MeetingLibraryItem>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let manager = manager(&app)?;
        manager
            .store
            .list_sessions()?
            .into_iter()
            .map(|session| {
                let data = manager.workspace(&session.id)?;
                let summary = manager.detail(&session.id, 0, 1)?.insights.summary;
                Ok(MeetingLibraryItem {
                    session,
                    title: data.title,
                    folder: data.folder,
                    summary,
                })
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn edit_meeting_workspace(
    app: AppHandle,
    id: String,
    edit: crate::meetings::workspace::WorkspaceEdit,
) -> Result<crate::meetings::workspace::MeetingWorkspace, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.edit_workspace(&id, edit))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn create_meeting_preparation(
    app: AppHandle,
    title: String,
    folder: String,
) -> Result<Session, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.create_preparation(title, folder))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn start_prepared_meeting(
    app: AppHandle,
    id: String,
    selection: CaptureSelection,
    ai_enabled: bool,
) -> Result<Session, String> {
    tauri::async_runtime::spawn_blocking(move || {
        manager(&app)?.start_prepared(Some(&id), selection, ai_enabled)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub fn meeting_providers(app: AppHandle) -> Vec<crate::meetings::agents::MeetingProvider> {
    crate::meetings::agents::providers(&app)
}

#[tauri::command]
#[specta::specta]
pub async fn send_meeting_message(
    app: AppHandle,
    id: String,
    request: crate::meetings::agents::MeetingRequest,
) -> Result<crate::meetings::workspace::MeetingWorkspace, String> {
    manager(&app)?.send_message(app, id, request)
}

#[tauri::command]
#[specta::specta]
pub async fn stop_meeting_agent(
    app: AppHandle,
    id: String,
    chat_id: String,
) -> Result<crate::meetings::workspace::MeetingWorkspace, String> {
    manager(&app)?.cancel_reply(&id, &chat_id)
}

#[tauri::command]
#[specta::specta]
pub async fn capture_meeting_image(
    app: AppHandle,
    id: String,
    scope: String,
    at_ms: u64,
) -> Result<crate::meetings::workspace::MeetingWorkspace, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.capture_image(&id, &scope, at_ms))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn import_meeting_recording(
    app: AppHandle,
    path: String,
    title: String,
    folder: String,
) -> Result<Session, String> {
    tauri::async_runtime::spawn_blocking(move || {
        manager(&app)?.import_recording(std::path::Path::new(&path), title, folder)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn extract_meeting_frame(
    app: AppHandle,
    id: String,
    at_ms: u64,
) -> Result<crate::meetings::workspace::MeetingWorkspace, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.extract_frame(&id, at_ms))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn export_meeting_bundle(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || manager(&app)?.export_bundle(&id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub fn open_meeting_window(app: AppHandle, id: String) -> Result<(), String> {
    let manager = manager(&app)?;
    manager.store.session(&id)?;
    let label = format!("meeting-{id}");
    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|e| e.to_string())?;
        window.unminimize().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    let title = manager.workspace(&id)?.title;
    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::App(format!("index.html?meeting={id}").into()),
    )
    .title(if title.is_empty() { "Handy" } else { &title })
    .inner_size(1200.0, 850.0)
    .min_inner_size(400.0, 500.0)
    .center()
    .build()
    .and_then(|window| {
        window.show()?;
        window.set_focus()
    })
    .map_err(|e| e.to_string())
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
pub async fn export_meeting_transcript(
    app: AppHandle,
    id: String,
    format: TranscriptFormat,
    labels: TranscriptLabels,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        manager(&app)?
            .store
            .export_transcript(&id, format, &labels)
            .map(|path| path.to_string_lossy().into_owned())
    })
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

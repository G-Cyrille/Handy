//! Debug-only IPC for reproducible tests of the separately signed test bundle.
//! Responses stay in its own data directory; this endpoint is absent in release.
use super::{
    archive::{TranscriptFormat, TranscriptLabels},
    capture::CaptureSelection,
    insights::write_json,
    MeetingManager,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

#[derive(Deserialize)]
struct Request {
    request_id: String,
    #[serde(flatten)]
    operation: Operation,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum Operation {
    Snapshot,
    Sources,
    Start {
        selection: CaptureSelection,
        #[serde(default)]
        ai_enabled: bool,
    },
    Stop,
    RefreshCapture,
    Detail {
        id: String,
        #[serde(default)]
        offset: u64,
    },
    Export {
        id: String,
    },
    ExportTranscript {
        id: String,
        format: TranscriptFormat,
        labels: TranscriptLabels,
    },
    Ai {
        enabled: bool,
    },
    Shortcut {
        pressed: bool,
    },
    Dictation,
    CancelDictation,
    ImportFixture {
        path: String,
    },
    AiFixture,
    Quit,
}

pub fn dispatch(app: &AppHandle, request: &str) {
    if !crate::test_build::is_meetings_dev(app) || request.len() > 4096 {
        return;
    }
    let request: Request = match serde_json::from_str(request) {
        Ok(request) => request,
        Err(error) => {
            log::error!("Meeting test request: {error}");
            return;
        }
    };
    if request.request_id.is_empty()
        || request.request_id.len() > 64
        || !request
            .request_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let result = execute(&app, request.operation);
        let response = match result {
            Ok(data) => json!({"status":"ok","data":data}),
            Err(error) => json!({"status":"error","error":error}),
        };
        let result = crate::portable::app_data_dir(&app)
            .map_err(|e| e.to_string())
            .and_then(|root| {
                let dir = root.join("meeting-test-control");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                write_json(&dir.join(format!("{}.json", request.request_id)), &response)
            });
        if let Err(error) = result {
            log::error!("Meeting test response: {error}");
        }
    });
}

fn execute(app: &AppHandle, operation: Operation) -> Result<Value, String> {
    let manager = app
        .try_state::<Arc<MeetingManager>>()
        .ok_or("Meeting manager is unavailable")?;
    match operation {
        Operation::AiFixture => {
            serde_json::to_value(super::insights::test_fixture(app)?).map_err(|e| e.to_string())
        }
        Operation::Snapshot => {
            serde_json::to_value(manager.snapshot(app)?).map_err(|e| e.to_string())
        }
        Operation::Sources => {
            serde_json::to_value(super::capture::sources()?).map_err(|e| e.to_string())
        }
        Operation::Start {
            selection,
            ai_enabled,
        } => serde_json::to_value(manager.start(selection, ai_enabled)?).map_err(|e| e.to_string()),
        Operation::Stop => {
            manager.stop()?;
            Ok(Value::Null)
        }
        Operation::RefreshCapture => {
            manager.refresh_capture()?;
            Ok(Value::Null)
        }
        Operation::Detail { id, offset } => {
            serde_json::to_value(manager.detail(&id, offset, 100)?).map_err(|e| e.to_string())
        }
        Operation::Export { id } => Ok(json!(manager.export(&id)?)),
        Operation::ExportTranscript { id, format, labels } => Ok(json!(manager
            .store
            .export_transcript(&id, format, &labels)?
            .to_string_lossy())),
        Operation::Ai { enabled } => {
            manager.set_ai(enabled)?;
            Ok(Value::Null)
        }
        Operation::Shortcut { pressed } => {
            crate::shortcut::handler::handle_shortcut_event(app, "meeting", "TEST", pressed);
            Ok(Value::Null)
        }
        Operation::Dictation => {
            crate::signal_handle::send_transcription_input(app, "transcribe", "Meeting test");
            Ok(Value::Null)
        }
        Operation::CancelDictation => {
            crate::utils::cancel_current_operation(app);
            Ok(Value::Null)
        }
        Operation::Quit => {
            let app = app.clone();
            std::thread::spawn(move || app.exit(0));
            Ok(Value::Null)
        }
        Operation::ImportFixture { path } => {
            let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
            if !path.starts_with("/private/tmp/handy-meetings-fixtures") {
                return Err("Fixtures must be under /private/tmp/handy-meetings-fixtures".into());
            }
            let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
            let spec = reader.spec();
            if spec.sample_rate != 16_000
                || spec.channels != 1
                || spec.bits_per_sample != 16
                || spec.sample_format != hound::SampleFormat::Int
            {
                return Err("Fixture must be 16 kHz mono int16 WAV".into());
            }
            let session = manager.store.create_session(
                "Synthetic microphone fixture".into(),
                "Synthetic system fixture".into(),
                false,
                0,
            )?;
            let mut writer = manager.store.writer(&session, 0)?;
            let mut block = Vec::with_capacity(16_000);
            let mut cursor = 0u64;
            for sample in reader.samples::<i16>() {
                block.push(f32::from(sample.map_err(|e| e.to_string())?) / 32768.0);
                if block.len() == 16_000 {
                    writer.push(0, cursor * 62_500, &block)?;
                    writer.push(1, cursor * 62_500, &block)?;
                    cursor += block.len() as u64;
                    block.clear();
                }
            }
            if !block.is_empty() {
                writer.push(0, cursor * 62_500, &block)?;
                writer.push(1, cursor * 62_500, &block)?;
                cursor += block.len() as u64;
            }
            writer.finish(cursor / 16, "completed")?;
            Ok(json!(session.id))
        }
    }
}

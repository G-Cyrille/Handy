//! Capture writes a durable archive; ASR and suggestions consume finalized audio.
pub mod archive;
pub mod capture;
mod insights;
#[cfg(debug_assertions)]
pub mod test_control;
mod vad;

use archive::{ArchiveStore, Segment, Session};
use capture::{CaptureEvent, CaptureHandle, CaptureSelection};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub use insights::MeetingInsights;

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
pub struct MeetingPreferences {
    pub selection: Option<CaptureSelection>,
    pub ai_enabled: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct MeetingTrackStatus {
    pub track: u8,
    pub received_samples: u64,
    pub last_received_ms: u64,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct PendingTranscript {
    pub session_id: String,
    pub segment_id: i64,
    pub track: u8,
    pub start_sample: u64,
    pub committed: String,
    pub tentative: String,
}

#[derive(Serialize, Type)]
pub struct MeetingSnapshot {
    pub sessions: Vec<Session>,
    pub active_id: Option<String>,
    pub elapsed_ms: u64,
    pub tracks: Vec<MeetingTrackStatus>,
    pub capture_error: Option<String>,
    pub pending: Option<PendingTranscript>,
    pub storage_path: String,
    pub provider: String,
    pub model: String,
    pub backlog_ms: u64,
    pub buffered_samples: u64,
    pub asr_last_duration_ms: u64,
    pub asr_last_lag_ms: u64,
    pub preferences: MeetingPreferences,
}

#[derive(Serialize, Type)]
pub struct MeetingDetail {
    pub session: Session,
    pub segments: Vec<Segment>,
    pub has_more: bool,
    pub total_segments: u64,
    pub track_stats: Vec<archive::TrackStats>,
    pub insights: MeetingInsights,
}

struct CaptureRuntime {
    stop: AtomicBool,
    done: AtomicBool,
    ai_enabled: AtomicBool,
    samples: [AtomicU64; 2],
    buffered_samples: AtomicU64,
    last_ms: [AtomicU64; 2],
    error: Mutex<Option<String>>,
    warning: Mutex<Option<String>>,
}

struct ActiveMeeting {
    id: String,
    origin_ns: u64,
    capture: CaptureHandle,
    writer: Option<thread::JoinHandle<()>>,
    runtime: Arc<CaptureRuntime>,
}

pub struct MeetingManager {
    pub store: ArchiveStore,
    root: PathBuf,
    active: Mutex<Option<ActiveMeeting>>,
    preferences: Mutex<MeetingPreferences>,
    pending: Arc<Mutex<Option<PendingTranscript>>>,
    vad: std::sync::OnceLock<Result<Mutex<crate::audio_toolkit::vad::SileroVad>, String>>,
    ai_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    shutdown: AtomicBool,
    asr_last_duration_ms: AtomicU64,
    asr_last_lag_ms: AtomicU64,
    last_error: Mutex<Option<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl MeetingManager {
    pub fn new(app: &AppHandle) -> Result<Arc<Self>, String> {
        let root = crate::portable::app_data_dir(app)
            .map_err(|e| e.to_string())?
            .join("meetings");
        let store = ArchiveStore::open(root.clone())?;
        let preferences = match std::fs::read(root.join("preferences.json")) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("Meeting preferences: {e}"))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                MeetingPreferences::default()
            }
            Err(error) => return Err(error.to_string()),
        };
        let manager = Arc::new(Self {
            store,
            root,
            active: Mutex::new(None),
            preferences: Mutex::new(preferences),
            pending: Arc::new(Mutex::new(None)),
            vad: std::sync::OnceLock::new(),
            ai_task: Mutex::new(None),
            shutdown: AtomicBool::new(false),
            asr_last_duration_ms: AtomicU64::new(0),
            asr_last_lag_ms: AtomicU64::new(0),
            last_error: Mutex::new(None),
        });
        let weak = Arc::downgrade(&manager);
        let app = app.clone();
        thread::Builder::new()
            .name("meeting-asr".into())
            .spawn(move || {
                while let Some(manager) = weak.upgrade() {
                    if manager.shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    manager.reap_finished();
                    manager.process_one_segment(&app);
                    manager.maybe_suggest(&app);
                    drop(manager);
                    thread::sleep(Duration::from_millis(250));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(manager)
    }

    fn save_preferences(&self) -> Result<(), String> {
        insights::write_json(
            &self.root.join("preferences.json"),
            &*lock(&self.preferences),
        )
    }

    pub fn start(&self, selection: CaptureSelection, ai_enabled: bool) -> Result<Session, String> {
        self.reap_finished();
        let mut active = lock(&self.active);
        if active.is_some() {
            return Err("A meeting is already recording".into());
        }
        let sources = capture::sources()?;
        let microphone = sources
            .microphones
            .iter()
            .find(|device| device.id == selection.microphone_id)
            .ok_or("The selected microphone is no longer available")?
            .name
            .clone();
        let system = match selection.application_pid {
            Some(pid) => sources
                .applications
                .iter()
                .find(|app| app.pid == pid)
                .ok_or("The selected application is no longer running")?
                .name
                .clone(),
            None => "System audio".into(),
        };
        let (tx, rx) = mpsc::sync_channel(1024);
        let capture = capture::start(&selection, tx).inspect_err(|error| {
            *lock(&self.last_error) = Some(error.clone());
        })?;
        // Permission dialogs precede the archive clock. The earliest callback
        // anchors both tracks, including audio queued during native startup.
        let first = rx.recv_timeout(Duration::from_millis(250)).ok();
        let origin_ns = match &first {
            Some(CaptureEvent::Audio(frame)) => frame.timestamp_ns.saturating_sub(1_000_000_000),
            Some(CaptureEvent::Error(error)) => return Err(error.clone()),
            None => capture::host_time_ns().saturating_sub(1_000_000_000),
        };
        let session = self
            .store
            .create_session(microphone, system, ai_enabled, origin_ns)?;
        let session = self.store.align_started_at(
            &session.id,
            chrono::Utc::now().timestamp_millis() - elapsed_ms(origin_ns) as i64,
        )?;
        let mut writer = self.store.writer(&session, origin_ns)?;

        let runtime = Arc::new(CaptureRuntime {
            stop: AtomicBool::new(false),
            done: AtomicBool::new(false),
            ai_enabled: AtomicBool::new(ai_enabled),
            samples: std::array::from_fn(|_| AtomicU64::new(0)),
            buffered_samples: AtomicU64::new(0),
            last_ms: std::array::from_fn(|_| AtomicU64::new(0)),
            error: Mutex::new(None),
            warning: Mutex::new(None),
        });
        let worker_runtime = runtime.clone();
        let store = self.store.clone();
        let id = session.id.clone();
        let join = thread::Builder::new()
            .name("meeting-archive".into())
            .spawn(move || {
                let outcome = (|| -> Result<(), String> {
                    let mut first = first;
                    let mut latest_ns = origin_ns;
                    loop {
                        let event = match first.take() {
                            Some(event) => Ok(event),
                            None => rx.recv_timeout(Duration::from_millis(200)),
                        };
                        let mut disconnected = false;
                        match event {
                            Ok(event) => {
                                latest_ns = latest_ns.max(record_event(
                                    &mut writer,
                                    &worker_runtime,
                                    event,
                                    origin_ns,
                                )?)
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => disconnected = true,
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                        // Drain captured frames before advancing the watermark.
                        // A slow fsync must never turn queued audio into silence.
                        for _ in 0..1024 {
                            match rx.try_recv() {
                                Ok(event) => {
                                    latest_ns = latest_ns.max(record_event(
                                        &mut writer,
                                        &worker_runtime,
                                        event,
                                        origin_ns,
                                    )?)
                                }
                                Err(mpsc::TryRecvError::Empty) => break,
                                Err(mpsc::TryRecvError::Disconnected) => {
                                    disconnected = true;
                                    break;
                                }
                            }
                        }
                        let captured_ms = latest_ns.saturating_sub(origin_ns) / 1_000_000;
                        writer.flush_until(captured_ms.saturating_sub(1_000))?;
                        worker_runtime
                            .buffered_samples
                            .store(writer.buffered_samples() as u64, Ordering::Relaxed);
                        if disconnected {
                            break;
                        }
                    }
                    writer.finish(elapsed_ms(origin_ns), "completed")?;
                    if let Some(warning) = lock(&worker_runtime.warning).as_deref() {
                        store.note_warning(&id, warning)?;
                    }
                    Ok(())
                })();
                if let Err(error) = outcome {
                    // Best-effort finalize of buffered source audio. An ASR or UI
                    // failure never enters this capture path.
                    let _ = writer.finish(elapsed_ms(origin_ns), "failed");
                    let _ = store.fail_session(&id, &error);
                    *lock(&worker_runtime.error) = Some(error);
                }
                worker_runtime.done.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        *lock(&self.preferences) = MeetingPreferences {
            selection: Some(selection),
            ai_enabled,
        };
        *lock(&self.last_error) = None;
        *active = Some(ActiveMeeting {
            id: session.id.clone(),
            origin_ns,
            capture,
            writer: Some(join),
            runtime,
        });
        self.save_preferences()?;
        Ok(session)
    }

    pub fn is_active(&self) -> bool {
        lock(&self.active).is_some()
    }

    pub fn stop(&self) -> Result<(), String> {
        let Some(mut active) = lock(&self.active).take() else {
            return Ok(());
        };
        if let Some(task) = lock(&self.ai_task).take() {
            task.abort();
        }
        let capture_result = active.capture.stop();
        active.runtime.stop.store(true, Ordering::Release);
        if let Some(join) = active.writer.take() {
            if join.join().is_err() {
                self.store
                    .fail_session(&active.id, "Archive worker panicked")?;
                return Err("Archive worker panicked".into());
            }
        }
        if let Some(error) = lock(&active.runtime.error).clone() {
            *lock(&self.last_error) = Some(error.clone());
            return Err(error);
        }
        if let Some(warning) = lock(&active.runtime.warning).clone() {
            *lock(&self.last_error) = Some(warning);
        }
        capture_result
    }

    fn reap_finished(&self) {
        let finished = lock(&self.active)
            .as_ref()
            .is_some_and(|a| a.runtime.done.load(Ordering::Acquire));
        if finished {
            if let Err(error) = self.stop() {
                log::error!("Meeting capture stopped: {error}");
            }
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        if let Err(error) = self.stop() {
            log::error!("Meeting finalization: {error}");
        }
    }

    pub fn toggle(&self, app: &AppHandle) -> Result<(), String> {
        crate::show_main_window(app);
        let _ = app.emit("meeting-open", ());
        let result = if lock(&self.active).is_some() {
            self.stop()
        } else {
            let preferences = lock(&self.preferences).clone();
            let Some(selection) = preferences.selection else {
                return Ok(());
            };
            self.start(selection, preferences.ai_enabled).map(|_| ())
        };
        if let Err(error) = &result {
            *lock(&self.last_error) = Some(error.clone());
        }
        result
    }

    pub fn snapshot(&self, app: &AppHandle) -> Result<MeetingSnapshot, String> {
        self.reap_finished();
        let active = lock(&self.active);
        let active_id = active.as_ref().map(|a| a.id.clone());
        let tracks = active
            .as_ref()
            .map(|a| {
                (0..2)
                    .map(|track| MeetingTrackStatus {
                        track: track as u8,
                        received_samples: a.runtime.samples[track].load(Ordering::Relaxed),
                        last_received_ms: a.runtime.last_ms[track].load(Ordering::Relaxed),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (provider, model) = insights::provider_label(app);
        Ok(MeetingSnapshot {
            sessions: self.store.list_sessions()?,
            elapsed_ms: active
                .as_ref()
                .map(|a| elapsed_ms(a.origin_ns))
                .unwrap_or(0),
            backlog_ms: match active_id.as_deref() {
                Some(id) => self.store.backlog_ms(id)?,
                None => 0,
            },
            buffered_samples: active
                .as_ref()
                .map(|active| active.runtime.buffered_samples.load(Ordering::Relaxed))
                .unwrap_or(0),
            asr_last_duration_ms: self.asr_last_duration_ms.load(Ordering::Relaxed),
            asr_last_lag_ms: self.asr_last_lag_ms.load(Ordering::Relaxed),
            active_id,
            tracks,
            capture_error: active
                .as_ref()
                .and_then(|active| lock(&active.runtime.warning).clone())
                .or_else(|| lock(&self.last_error).clone()),
            pending: lock(&self.pending).clone(),
            storage_path: self.root.to_string_lossy().into(),
            provider,
            model,
            preferences: lock(&self.preferences).clone(),
        })
    }

    pub fn detail(&self, id: &str, offset: u64, limit: u32) -> Result<MeetingDetail, String> {
        let limit = limit.clamp(1, 200);
        let mut segments = self.store.segments(id, offset, limit + 1)?;
        let has_more = segments.len() > limit as usize;
        segments.truncate(limit as usize);
        Ok(MeetingDetail {
            session: self.store.session(id)?,
            segments,
            has_more,
            total_segments: self.store.segment_count(id)?,
            track_stats: self.store.track_stats(id)?,
            insights: insights::read(&self.store.session_dir(id)?)?,
        })
    }

    fn process_one_segment(&self, app: &AppHandle) {
        let preferred = lock(&self.active).as_ref().map(|active| active.id.clone());
        let segment = match self.store.next_pending_for(preferred.as_deref()) {
            Ok(Some(segment)) => segment,
            Ok(None) => return,
            Err(error) => {
                *lock(&self.last_error) = Some(error);
                return;
            }
        };
        let outcome = (|| -> Result<(), String> {
            let audio = self.store.read_segment(&segment)?;
            if segment.asr_status != "force" {
                let detector = self.vad.get_or_init(|| {
                    let path = app
                        .path()
                        .resolve(
                            "resources/models/silero-vad-v6.2-F32.gguf",
                            tauri::path::BaseDirectory::Resource,
                        )
                        .map_err(|e| e.to_string())?;
                    crate::audio_toolkit::vad::SileroVad::new(path, 0.1)
                        .map(Mutex::new)
                        .map_err(|e| {
                            log::warn!(
                                "Meeting VAD unavailable; ASR will process all archived audio: {e}"
                            );
                            e.to_string()
                        })
                });
                if let Ok(detector) = detector {
                    match vad::contains_speech(&mut *lock(detector), &audio) {
                        Ok(false) => return self.store.set_no_speech(segment.id),
                        Ok(true) => {}
                        Err(error) => {
                            log::warn!("Meeting VAD failed; retaining the full ASR window: {error}")
                        }
                    }
                }
            }
            let pending = self.pending.clone();
            let seed = PendingTranscript {
                session_id: segment.session_id.clone(),
                segment_id: segment.id,
                track: segment.track,
                start_sample: segment.start_sample,
                committed: String::new(),
                tentative: String::new(),
            };
            *lock(&pending) = Some(seed.clone());
            let tm = app.state::<Arc<crate::managers::transcription::TranscriptionManager>>();
            let started = std::time::Instant::now();
            let result = tm
                .transcribe_meeting(audio, move |progress| {
                    if let Some(text) = progress.text {
                        *lock(&pending) = Some(PendingTranscript {
                            committed: text.committed,
                            tentative: text.tentative,
                            ..seed.clone()
                        });
                    }
                })
                .map_err(|e| e.to_string())?;
            if let Some(text) = result {
                self.store.set_transcript(segment.id, &text)?;
                let duration_ms = started.elapsed().as_millis() as u64;
                let session = self.store.session(&segment.session_id)?;
                let end_ms = (segment.start_sample + segment.sample_count) / 16;
                let lag_ms =
                    (chrono::Utc::now().timestamp_millis() - session.started_at_ms - end_ms as i64)
                        .max(0) as u64;
                self.asr_last_duration_ms
                    .store(duration_ms, Ordering::Relaxed);
                self.asr_last_lag_ms.store(lag_ms, Ordering::Relaxed);
                log::info!(
                    "Meeting ASR session={} segment={} track={} compute_ms={} lag_ms={}",
                    segment.session_id,
                    segment.id,
                    segment.track,
                    duration_ms,
                    lag_ms
                );
            }
            Ok(())
        })();
        *lock(&self.pending) = None;
        if let Err(error) = outcome {
            if let Err(storage_error) = self.store.set_asr_error(segment.id, &error) {
                *lock(&self.last_error) = Some(storage_error);
            }
        }
    }

    pub fn set_ai(&self, enabled: bool) -> Result<(), String> {
        if let Some(active) = lock(&self.active).as_ref() {
            active.runtime.ai_enabled.store(enabled, Ordering::Release);
            self.store.set_ai_enabled(&active.id, enabled)?;
            if enabled {
                insights::reset_failures(&self.store.session_dir(&active.id)?)?;
            }
        }
        if !enabled {
            if let Some(task) = lock(&self.ai_task).take() {
                task.abort();
            }
        }
        lock(&self.preferences).ai_enabled = enabled;
        self.save_preferences()
    }

    pub fn export(&self, id: &str) -> Result<String, String> {
        let session = self.store.session(id)?;
        if session.status == "recording" {
            return Err("Stop the meeting before exporting".into());
        }
        let dir = self.store.session_dir(id)?;
        let wav = dir.join("export-stereo.wav");
        self.store.export_wav(id, &wav)?;
        #[cfg(target_os = "macos")]
        {
            let path = dir.join("meeting.m4a");
            let temporary = dir.join("meeting-export.m4a");
            let output = std::process::Command::new("/usr/bin/afconvert")
                .args(["-f", "m4af", "-d", "aac@48000", "-b", "128000"])
                .arg(&wav)
                .arg(&temporary)
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "AAC export: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
            std::fs::remove_file(&wav).map_err(|e| e.to_string())?;
            Ok(path.to_string_lossy().into())
        }
        #[cfg(not(target_os = "macos"))]
        Ok(wav.to_string_lossy().into())
    }
}

fn record_event(
    writer: &mut archive::ArchiveWriter,
    runtime: &CaptureRuntime,
    event: CaptureEvent,
    origin_ns: u64,
) -> Result<u64, String> {
    match event {
        CaptureEvent::Error(error) => Err(error),
        CaptureEvent::Audio(frame) => {
            let track = usize::from(frame.track);
            if frame.track != archive::TRACK_MICROPHONE && frame.track != archive::TRACK_SYSTEM {
                return Err("Unknown capture track".into());
            }
            writer.push(frame.track, frame.timestamp_ns, &frame.samples)?;
            let stats = writer.stats(frame.track)?;
            if stats.dropped_samples > 0 {
                *lock(&runtime.warning) = Some(format!("Track {} has {} dropped samples; capture continues and the archive contains gaps", frame.track, stats.dropped_samples));
            }
            runtime.samples[track].fetch_add(frame.samples.len() as u64, Ordering::Relaxed);
            let end_ns = frame.timestamp_ns + frame.samples.len() as u64 * 62_500;
            runtime.last_ms[track].store(
                end_ns.saturating_sub(origin_ns) / 1_000_000,
                Ordering::Relaxed,
            );
            Ok(end_ns)
        }
    }
}

fn elapsed_ms(origin_ns: u64) -> u64 {
    capture::host_time_ns().saturating_sub(origin_ns) / 1_000_000
}

#[derive(Default)]
pub struct MeetingShortcut {
    pressed: bool,
}

impl MeetingShortcut {
    pub const fn new() -> Self {
        Self { pressed: false }
    }
    pub fn edge(&mut self, pressed: bool) -> bool {
        let trigger = pressed && !self.pressed;
        self.pressed = pressed;
        trigger
    }
}

#[cfg(test)]
mod tests {
    use super::MeetingShortcut;
    #[test]
    fn shortcut_toggles_on_distinct_presses_only() {
        let mut shortcut = MeetingShortcut::default();
        assert!(shortcut.edge(true));
        for _ in 0..20 {
            assert!(!shortcut.edge(true));
        }
        assert!(!shortcut.edge(false));
        assert!(shortcut.edge(true));
        assert!(!shortcut.edge(false));
    }
}

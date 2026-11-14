use super::{
    agents::binary,
    archive::{ArchiveStore, STATUS_COMPLETED, WINDOW_SAMPLES},
    lock,
    workspace::{self, TimelineEntry},
    MeetingManager,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

impl MeetingManager {
    pub fn capture_image(
        &self,
        id: &str,
        scope: &str,
        at_ms: u64,
    ) -> Result<workspace::MeetingWorkspace, String> {
        let dir = self.store.session_dir(id)?;
        let entry_id = workspace::identifier();
        let filename = format!("capture-{entry_id}.jpg");
        let path = dir.join(&filename);
        match scope {
            "all" | "screen" => {
                let context = crate::screenshot::ScreenshotContext::new();
                context.begin_capture(
                    if scope == "all" {
                        crate::settings::ScreenshotCaptureScope::AllMonitors
                    } else {
                        crate::settings::ScreenshotCaptureScope::CurrentMonitor
                    },
                    false,
                    false,
                );
                let image = context
                    .take_pending()
                    .and_then(|p| p.wait())
                    .ok_or("Capture unavailable; check Screen Recording permission")?;
                std::fs::write(&path, image.jpeg()).map_err(|e| e.to_string())?;
                image.save_monitor_images(&path)?;
            }
            #[cfg(target_os = "macos")]
            "application" | "region" => {
                let status = Command::new("/usr/sbin/screencapture")
                    .args([
                        "-x",
                        "-t",
                        "jpg",
                        "-i",
                        if scope == "application" { "-w" } else { "-s" },
                    ])
                    .arg(&path)
                    .status()
                    .map_err(|e| e.to_string())?;
                if !status.success() || !path.exists() {
                    return Err("Capture cancelled".into());
                }
            }
            _ => return Err("Unsupported capture scope".into()),
        }
        let _guard = lock(&self.workspace_lock);
        let mut data = workspace::read(&dir)?;
        data.entries.push(TimelineEntry {
            id: entry_id,
            at_ms,
            kind: "capture".into(),
            text: scope.into(),
            image: Some(filename),
        });
        workspace::write(&dir, &data)?;
        Ok(data)
    }

    pub fn import_recording(
        &self,
        path: &Path,
        title: String,
        folder: String,
    ) -> Result<super::archive::Session, String> {
        if !path.is_file() {
            return Err("Recording file does not exist".into());
        }
        let ffmpeg = binary("ffmpeg").ok_or("Install FFmpeg to import audio or video")?;
        let session =
            self.store
                .create_session(String::new(), "Imported recording".into(), false, 1)?;
        let id = &session.id;
        let result = (|| {
            let dir = self.store.session_dir(id)?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("bin")
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(10)
                .collect::<String>();
            let source = dir.join(format!("source.{ext}"));
            std::fs::copy(path, &source).map_err(|e| e.to_string())?;
            let wav = dir.join("import.wav");
            let output = Command::new(ffmpeg)
                .args(["-nostdin", "-v", "error", "-y", "-i"])
                .arg(&source)
                .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
                .arg(&wav)
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "FFmpeg import: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            let mut reader = hound::WavReader::open(&wav).map_err(|e| e.to_string())?;
            let mut samples = reader.samples::<i16>();
            let mut cursor = 0u64;
            let mut writer = self.store.writer(&session, 1)?;
            loop {
                let chunk = samples
                    .by_ref()
                    .take(WINDOW_SAMPLES as usize)
                    .map(|s| s.map(|s| s as f32 / 32768.0).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()?;
                if chunk.is_empty() {
                    break;
                }
                writer.push(1, 1 + cursor * 1_000_000_000 / 16_000, &chunk)?;
                cursor += chunk.len() as u64;
                writer.flush_until(cursor / 16)?;
            }
            writer.finish(cursor / 16, STATUS_COMPLETED)?;
            std::fs::remove_file(&wav).map_err(|e| e.to_string())?;
            let _guard = lock(&self.workspace_lock);
            let mut data = workspace::read(&dir)?;
            data.title = title;
            data.folder = folder;
            if ["mp4", "mkv", "mov", "webm"].contains(&ext.to_lowercase().as_str()) {
                data.imported_video = Some(source.to_string_lossy().into());
            }
            workspace::write(&dir, &data)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.store.fail_session(id, &error)?;
            return Err(error);
        }
        self.store.session(id)
    }

    pub fn extract_frame(
        &self,
        id: &str,
        at_ms: u64,
    ) -> Result<workspace::MeetingWorkspace, String> {
        let dir = self.store.session_dir(id)?;
        let video = self
            .workspace(id)?
            .imported_video
            .ok_or("This meeting has no imported video")?;
        let ffmpeg = binary("ffmpeg").ok_or("Install FFmpeg to extract video frames")?;
        let entry_id = workspace::identifier();
        let filename = format!("frame-{entry_id}.jpg");
        let output = Command::new(ffmpeg)
            .args([
                "-nostdin",
                "-v",
                "error",
                "-y",
                "-ss",
                &format!("{:.3}", at_ms as f64 / 1000.0),
                "-i",
                &video,
                "-frames:v",
                "1",
            ])
            .arg(dir.join(&filename))
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() || !dir.join(&filename).is_file() {
            return Err(format!(
                "Video frame unavailable: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let _guard = lock(&self.workspace_lock);
        let mut data = workspace::read(&dir)?;
        data.entries.push(TimelineEntry {
            id: entry_id,
            at_ms,
            kind: "capture".into(),
            text: "video".into(),
            image: Some(filename),
        });
        workspace::write(&dir, &data)?;
        Ok(data)
    }

    pub fn export_bundle(&self, id: &str) -> Result<String, String> {
        let session = self.store.session(id)?;
        if session.status == "recording" {
            return Err("Stop the meeting before exporting audio".into());
        }
        let dir = self.store.session_dir(id)?;
        {
            let _guard = lock(&self.workspace_lock);
            let data = workspace::read(&dir)?;
            write_transcripts(&self.store, id, &data)?;
        }
        if session.duration_ms > 0 && !dir.join("meeting.mp3").exists() {
            let wav = dir.join("export-stereo.wav");
            self.store.export_wav(id, &wav)?;
            let ffmpeg =
                binary("ffmpeg").ok_or("Audio preserved as WAV; install FFmpeg for MP3")?;
            let temporary = dir.join("meeting-export.mp3");
            let output = Command::new(ffmpeg)
                .args(["-nostdin", "-v", "error", "-y", "-i"])
                .arg(&wav)
                .args(["-codec:a", "libmp3lame", "-q:a", "3"])
                .arg(&temporary)
                .output()
                .map_err(|e| e.to_string())?;
            if !output.status.success() {
                return Err(format!(
                    "MP3 export: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            std::fs::rename(temporary, dir.join("meeting.mp3")).map_err(|e| e.to_string())?;
            std::fs::remove_file(wav).map_err(|e| e.to_string())?;
        }
        Ok(dir.to_string_lossy().into())
    }
}

pub fn write_transcripts(
    store: &ArchiveStore,
    id: &str,
    data: &workspace::MeetingWorkspace,
) -> Result<(), String> {
    let dir = store.session_dir(id)?;
    let mut offset = 0;
    let mut original = Vec::new();
    let mut corrected = Vec::new();
    loop {
        let page = store.segments(id, offset, 500)?;
        if page.is_empty() {
            break;
        }
        offset += page.len() as u64;
        for segment in page {
            let key = segment.id.to_string();
            let item = serde_json::json!({"id":segment.id,"start_ms":segment.start_sample / 16,"end_ms":(segment.start_sample + segment.sample_count) / 16,"track":segment.track,"speaker":data.speakers.get(&key),"text":segment.text,"status":segment.asr_status,"error":segment.asr_error});
            let mut edited = item.clone();
            if let Some(text) = data.corrections.get(&key) {
                edited["text"] = text.clone().into();
            }
            original.push(item);
            corrected.push(edited);
        }
    }
    super::insights::write_json(&dir.join("transcript-original.json"), &original)?;
    super::insights::write_json(&dir.join("transcript-corrected.json"), &corrected)?;
    let readable = |items: &[serde_json::Value]| {
        items
            .iter()
            .filter_map(|s| {
                s["text"].as_str().filter(|t| !t.is_empty()).map(|text| {
                    format!(
                        "[{} ms] {}: {}",
                        s["start_ms"],
                        s["speaker"].as_str().unwrap_or(if s["track"] == 0 {
                            "Microphone"
                        } else {
                            "System"
                        }),
                        text
                    )
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    for (name, items) in [
        ("transcript-original.txt", &original),
        ("transcript-corrected.txt", &corrected),
    ] {
        let temporary: PathBuf = dir.join(format!(".{name}.tmp"));
        std::fs::write(&temporary, readable(items)).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, dir.join(name)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_and_corrected_exports_remain_distinct_after_asr_catches_up() {
        let temp = tempfile::tempdir().unwrap();
        let manager = MeetingManager::test_manager(temp.path().into());
        let session = manager
            .store
            .create_session("mic".into(), "system".into(), false, 0)
            .unwrap();
        let mut writer = manager.store.writer(&session, 0).unwrap();
        writer.push(0, 0, &vec![0.2; 16000]).unwrap();
        writer.finish(1000, STATUS_COMPLETED).unwrap();
        let segment = manager.store.segments(&session.id, 0, 1).unwrap().remove(0);
        manager
            .store
            .set_transcript(segment.id, "Je ne vais pas envoyer le contrat dem1")
            .unwrap();
        let mut data = manager.workspace(&session.id).unwrap();
        data.corrections.insert(
            segment.id.to_string(),
            "Je ne vais pas envoyer le contrat demain".into(),
        );
        write_transcripts(&manager.store, &session.id, &data).unwrap();
        let dir = manager.store.session_dir(&session.id).unwrap();
        assert!(std::fs::read_to_string(dir.join("transcript-original.txt"))
            .unwrap()
            .contains("dem1"));
        assert!(
            std::fs::read_to_string(dir.join("transcript-corrected.txt"))
                .unwrap()
                .contains("ne vais pas envoyer le contrat demain")
        );
        assert!(manager.store.segments(&session.id, 0, 1).unwrap()[0]
            .text
            .as_ref()
            .unwrap()
            .contains("dem1"));
    }

    #[test]
    fn imported_audio_is_local_archived_and_exportable_as_mp3() {
        if binary("ffmpeg").is_none() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("input.wav");
        let mut wav = hound::WavWriter::create(
            &source,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..8000 {
            wav.write_sample((1000.0 * (i as f32 * 0.1).sin()) as i16)
                .unwrap();
        }
        wav.finalize().unwrap();
        let manager = MeetingManager::test_manager(temp.path().join("archive"));
        let session = manager
            .import_recording(&source, "Import".into(), "Tibria".into())
            .unwrap();
        assert_eq!(session.status, STATUS_COMPLETED);
        assert_eq!(session.duration_ms, 500);
        let dir = manager.store.session_dir(&session.id).unwrap();
        assert!(dir.join("source.wav").is_file());
        manager.export_bundle(&session.id).unwrap();
        assert!(dir.join("meeting.mp3").metadata().unwrap().len() > 100);
    }
}

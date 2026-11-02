//! Transcript snapshots, independent from audio export and UI pagination.
use super::*;
use std::io::Write;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptFormat {
    Markdown,
    Text,
    Json,
}

/// Labels come from the UI's translations; transcript text is never rewritten.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct TranscriptLabels {
    pub title: String,
    pub microphone: String,
    pub system: String,
    pub pending: String,
    pub failed: String,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Track {
    Microphone,
    System,
}

#[derive(Serialize)]
struct TranscriptSegment {
    start_ms: f64,
    end_ms: f64,
    track: Track,
    status: String,
    text: Option<String>,
    error: Option<String>,
}

fn clock(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1_000 % 60,
        ms % 1_000
    )
}

impl ArchiveStore {
    /// Streams all archived segments from one SQLite snapshot, including pending
    /// and failed ASR states. Audio files and provisional ASR are not read.
    pub fn export_transcript(
        &self,
        id: &str,
        format: TranscriptFormat,
        labels: &TranscriptLabels,
    ) -> Result<PathBuf, String> {
        let mut conn = self.conn()?;
        let tx = conn.transaction().ctx("snapshot transcript")?;
        let session = query_session(&tx, id)?;
        if session.status == STATUS_RECORDING {
            return Err("Stop the meeting before exporting".into());
        }
        let dir = self.session_dir(id)?;
        let extension = match format {
            TranscriptFormat::Markdown => "md",
            TranscriptFormat::Text => "txt",
            TranscriptFormat::Json => "json",
        };
        let destination = dir.join(format!("transcript.{extension}"));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ctx("timestamp export")?
            .as_nanos();
        let temporary = dir.join(format!(".transcript-{nonce}.tmp"));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .ctx("create transcript export")?;
        let result = (|| -> Result<(), String> {
            let mut out = BufWriter::new(file);
            match format {
                TranscriptFormat::Json => {
                    write!(out, "{{\"schema_version\":1,\"session\":").ctx("write transcript")?;
                    serde_json::to_writer(&mut out, &session).ctx("write transcript metadata")?;
                    writeln!(out, ",\"segments\":[").ctx("write transcript")?;
                }
                TranscriptFormat::Markdown | TranscriptFormat::Text => {
                    let heading = if matches!(format, TranscriptFormat::Markdown) {
                        "# "
                    } else {
                        ""
                    };
                    writeln!(
                        out,
                        "{heading}{}\n{}\n",
                        labels.title,
                        clock(session.duration_ms)
                    )
                    .ctx("write transcript header")?;
                }
            }
            let mut statement = tx
                .prepare("SELECT track, start_sample, sample_count, asr_status, text, asr_error FROM segments WHERE session_id = ?1 ORDER BY start_sample, track")
                .ctx("read transcript")?;
            let segments = statement
                .query_map([id], |row| {
                    Ok((
                        row.get::<_, u8>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                })
                .ctx("read transcript")?;
            let mut first = true;
            for segment in segments {
                let (track, start, count, status, text, error) =
                    segment.ctx("read transcript segment")?;
                let (track, label) = match track {
                    TRACK_MICROPHONE => (Track::Microphone, &labels.microphone),
                    TRACK_SYSTEM => (Track::System, &labels.system),
                    _ => return Err("Unknown transcript track".into()),
                };
                if matches!(format, TranscriptFormat::Json) {
                    if !first {
                        writeln!(out, ",").ctx("write transcript")?;
                    }
                    let segment = TranscriptSegment {
                        start_ms: start as f64 / 16.0,
                        end_ms: (start + count) as f64 / 16.0,
                        track,
                        status,
                        text,
                        error,
                    };
                    serde_json::to_writer(&mut out, &segment).ctx("write transcript segment")?;
                    first = false;
                    continue;
                }
                let text = if let Some(error) = error {
                    format!("{}: {error}", labels.failed)
                } else if let Some(text) = text {
                    if text.trim().is_empty() {
                        continue;
                    }
                    text
                } else {
                    labels.pending.clone()
                };
                let range = format!(
                    "[{} - {}] {label}",
                    clock(start / 16),
                    clock((start + count) / 16)
                );
                if matches!(format, TranscriptFormat::Markdown) {
                    writeln!(out, "**{range}**\n\n{text}\n").ctx("write transcript segment")?;
                } else {
                    writeln!(out, "{range}\n{text}\n").ctx("write transcript segment")?;
                }
            }
            if matches!(format, TranscriptFormat::Json) {
                writeln!(out, "\n]}}").ctx("finish transcript")?;
            }
            out.flush().ctx("flush transcript")?;
            out.get_ref().sync_all().ctx("sync transcript")?;
            fs::rename(&temporary, &destination).ctx("publish transcript")
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> TranscriptLabels {
        TranscriptLabels {
            title: "Transcription · réunion de test".into(),
            microphone: "Micro".into(),
            system: "Système".into(),
            pending: "En attente de transcription".into(),
            failed: "Échec de transcription".into(),
        }
    }

    fn fixture() -> (tempfile::TempDir, ArchiveStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = ArchiveStore::open(dir.path().to_owned()).unwrap();
        let session = store
            .create_session("BY-V".into(), "Browser".into(), false, 0)
            .unwrap();
        let conn = store.conn().unwrap();
        conn.execute(
            "UPDATE sessions SET status = 'completed', duration_ms = 1260000 WHERE id = ?1",
            [&session.id],
        )
        .unwrap();
        let mut insert = conn.prepare("INSERT INTO segments (session_id, track, start_sample, sample_count, file, text, asr_status, asr_error) VALUES (?1, ?2, ?3, 80000, 'unused.wav', ?4, ?5, ?6)").unwrap();
        for index in 0..503 {
            let (text, status, error) = match index {
                500 => (None, "pending", None),
                501 => (None, "failed", Some("worker unavailable")),
                502 => (Some(""), "no_speech", None),
                _ => (
                    Some("Je ne vais pas envoyer le contrat demain.\nD'accord, déjà noté."),
                    "final",
                    None,
                ),
            };
            insert
                .execute(params![
                    session.id,
                    index % 2,
                    (index / 2) * 80000,
                    text,
                    status,
                    error
                ])
                .unwrap();
        }
        (dir, store, session.id)
    }

    #[test]
    fn json_exports_every_segment_with_overlaps_unicode_and_asr_states() {
        let (_dir, store, id) = fixture();
        let path = store
            .export_transcript(&id, TranscriptFormat::Json, &labels())
            .unwrap();
        let data: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let segments = data["segments"].as_array().unwrap();
        assert_eq!(
            segments.len(),
            503,
            "Export must exceed both UI and archive page limits"
        );
        assert_eq!(segments[0]["start_ms"], segments[1]["start_ms"]);
        assert_eq!(segments[0]["track"], "microphone");
        assert_eq!(segments[1]["track"], "system");
        assert_eq!(
            segments[0]["text"], segments[1]["text"],
            "Simultaneous identical text must not be deduplicated"
        );
        assert!(segments[0]["text"]
            .as_str()
            .unwrap()
            .contains("ne vais pas"));
        assert!(segments[0]["text"]
            .as_str()
            .unwrap()
            .contains("\nD'accord, déjà noté."));
        assert_eq!(segments[500]["status"], "pending");
        assert_eq!(segments[501]["error"], "worker unavailable");
        assert_eq!(segments[502]["status"], "no_speech");
        assert!(segments[0].get("path").is_none());
    }

    #[test]
    fn readable_exports_keep_timestamps_tracks_and_incomplete_transcripts_visible() {
        let (_dir, store, id) = fixture();
        for format in [TranscriptFormat::Text, TranscriptFormat::Markdown] {
            let path = store.export_transcript(&id, format, &labels()).unwrap();
            let text = fs::read_to_string(path).unwrap();
            assert!(text.contains("[00:00:00.000 - 00:00:05.000] Micro"));
            assert!(text.contains("[00:00:00.000 - 00:00:05.000] Système"));
            assert_eq!(
                text.matches("Je ne vais pas envoyer le contrat demain.")
                    .count(),
                500
            );
            assert!(text.contains("En attente de transcription"));
            assert!(text.contains("Échec de transcription: worker unavailable"));
            assert!(text.contains("[00:20:50.000 - 00:20:55.000]"));
        }
    }

    #[test]
    fn failed_export_preserves_the_previous_file() {
        let (_dir, store, id) = fixture();
        let path = store
            .export_transcript(&id, TranscriptFormat::Json, &labels())
            .unwrap();
        let before = fs::read(&path).unwrap();
        store.conn().unwrap().execute("UPDATE segments SET track = 99 WHERE session_id = ?1 AND track = 0 AND start_sample = 0", [&id]).unwrap();
        assert!(store
            .export_transcript(&id, TranscriptFormat::Json, &labels())
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(!list_files(path.parent().unwrap())
            .unwrap()
            .iter()
            .any(|name| name.ends_with(".tmp")));
    }
}

//! Durable, disk-first archive for meeting recordings.
//!
//! A session records two mono 16 kHz tracks (0 = microphone, 1 = system audio) on
//! one timeline anchored at the session's `origin_ns` (monotonic uptime). Each
//! track is cut into fixed 5-second windows; every window becomes one immutable
//! 16-bit PCM WAV segment indexed in SQLite. Silence, gaps and missing callbacks
//! are written as zero padding so both tracks always share the same timeline.
//!
//! ```text
//! <root>/archive.sqlite3                     sessions, segments, track counters (WAL)
//! <root>/sessions/<id>/t0-000000.wav         microphone, samples [0, 80000)
//! <root>/sessions/<id>/t1-000000.wav         system audio, same window
//! <root>/sessions/<id>/.t0-000001.wav.tmp    segment being written
//! ```
//!
//! Durability: a segment is written to a temp file, finalized, fsynced, renamed into
//! place, then indexed in a SQLite transaction. A crash leaves at most a temp file
//! (verified, then recovered or discarded at startup) or an unindexed WAV (verified,
//! then indexed at startup). Sessions still `recording` at startup become
//! `interrupted`. Meeting sessions are independent from dictation history retention.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use specta::Type;

pub const SAMPLE_RATE: u32 = 16_000;
pub const TRACK_MICROPHONE: u8 = 0;
pub const TRACK_SYSTEM: u8 = 1;
/// Samples per segment window (5 seconds).
pub const WINDOW_SAMPLES: u64 = 5 * SAMPLE_RATE as u64;

pub const STATUS_RECORDING: &str = "recording";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_INTERRUPTED: &str = "interrupted";
pub const STATUS_FAILED: &str = "failed";

const TRACKS: usize = 2;
const WINDOW: usize = WINDOW_SAMPLES as usize;
/// Timestamp jitter absorbed without padding or dropping samples (20 ms).
const JITTER_SAMPLES: u64 = 320;
/// A chunk starting further than this (60 s) ahead of both the track cursor and the
/// flush watermark carries a bogus timestamp; it is dropped instead of padding to it.
const MAX_LEAD_SAMPLES: u64 = 60 * SAMPLE_RATE as u64;
const MAX_SESSIONS: u32 = 200;
const MAX_PAGE: u32 = 500;
const DB_FILE: &str = "archive.sqlite3";
const SESSIONS_DIR: &str = "sessions";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    started_at_ms INTEGER NOT NULL,
    origin_ns INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,
    microphone TEXT NOT NULL,
    system TEXT NOT NULL,
    ai_enabled INTEGER NOT NULL,
    error TEXT
);
CREATE TABLE IF NOT EXISTS segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    track INTEGER NOT NULL,
    start_sample INTEGER NOT NULL,
    sample_count INTEGER NOT NULL,
    -- Samples that came from capture callbacks; NULL when recovered from disk.
    delivered_samples INTEGER,
    file TEXT NOT NULL,
    text TEXT,
    asr_error TEXT,
    asr_status TEXT NOT NULL DEFAULT 'pending',
    UNIQUE (session_id, track, start_sample)
);
CREATE INDEX IF NOT EXISTS segments_timeline ON segments (session_id, start_sample, track);
CREATE INDEX IF NOT EXISTS segments_pending ON segments (session_id, start_sample, track)
    WHERE text IS NULL AND asr_error IS NULL;
CREATE TABLE IF NOT EXISTS tracks (
    session_id TEXT NOT NULL REFERENCES sessions(id),
    track INTEGER NOT NULL,
    delivered_samples INTEGER NOT NULL,
    padded_samples INTEGER NOT NULL,
    overlapped_samples INTEGER NOT NULL,
    dropped_samples INTEGER NOT NULL,
    PRIMARY KEY (session_id, track)
);
PRAGMA user_version = 1;
";

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Session {
    pub id: String,
    pub started_at_ms: i64,
    pub duration_ms: u64,
    /// `recording`, `completed`, `interrupted` or `failed`.
    pub status: String,
    pub microphone: String,
    pub system: String,
    pub ai_enabled: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Segment {
    pub id: i64,
    pub session_id: String,
    pub track: u8,
    pub start_sample: u64,
    pub sample_count: u64,
    /// Absolute path inside the archive root.
    pub path: String,
    pub text: Option<String>,
    pub asr_error: Option<String>,
    pub asr_status: String,
}

/// Per-track sample accounting. `delivered + padded` is the track's written length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TrackStats {
    pub track: u8,
    /// Samples received from capture callbacks and written.
    pub delivered_samples: u64,
    /// Zero samples inserted for gaps, missing callbacks and the tail.
    pub padded_samples: u64,
    /// Same-track samples discarded because they overlap audio already written.
    pub overlapped_samples: u64,
    /// Samples discarded because their window was sealed, they precede the origin,
    /// their timestamp was out of range, or a disk error prevented buffering them.
    pub dropped_samples: u64,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Handle to the archive. Cheap to clone; every call opens a short-lived connection.
#[derive(Clone, Debug)]
pub struct ArchiveStore {
    root: PathBuf,
}

impl ArchiveStore {
    /// Opens (or creates) the archive at `root` and recovers what a crash left behind.
    pub fn open(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(root.join(SESSIONS_DIR))
            .map_err(|e| format!("create archive dir {}: {e}", root.display()))?;
        let store = Self { root };
        let mut conn = store.conn()?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .ctx("enable WAL")?;
        conn.execute_batch(SCHEMA).ctx("create archive schema")?;
        let has_status: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('segments') WHERE name = 'asr_status')", [], |row| row.get(0)).ctx("inspect ASR schema")?;
        if !has_status {
            conn.execute_batch(
                "ALTER TABLE segments ADD COLUMN asr_status TEXT NOT NULL DEFAULT 'pending'",
            )
            .ctx("migrate ASR status")?;
        }
        store.recover(&mut conn)?;
        Ok(store)
    }

    pub fn create_session(
        &self,
        microphone: String,
        system: String,
        ai_enabled: bool,
        origin_ns: u64,
    ) -> Result<Session, String> {
        // Ids are the start time in ms; `create_dir` failing on an existing directory
        // bumps it, so ids stay unique, sortable and safe as directory names.
        let mut started_at_ms = now_ms();
        let id = loop {
            let id = started_at_ms.to_string();
            match fs::create_dir(self.dir_path(&id)) {
                Ok(()) => break id,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => started_at_ms += 1,
                Err(e) => return Err(format!("create session dir: {e}")),
            }
        };
        let inserted = sync_dir(&self.root.join(SESSIONS_DIR)).and_then(|()| {
            self.conn()?
                .execute(
                    "INSERT INTO sessions (id, started_at_ms, origin_ns, status, microphone, system, ai_enabled)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, started_at_ms, to_i64(origin_ns)?, STATUS_RECORDING, microphone, system, ai_enabled],
                )
                .ctx("insert session")
        });
        if let Err(e) = inserted {
            let _ = fs::remove_dir(self.dir_path(&id));
            return Err(e);
        }
        self.session(&id)
    }

    /// Newest sessions first, at most 200.
    pub fn list_sessions(&self) -> Result<Vec<Session>, String> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions ORDER BY started_at_ms DESC LIMIT ?1"
            ))
            .ctx("list sessions")?;
        let rows = stmt
            .query_map([MAX_SESSIONS], map_session)
            .ctx("list sessions")?;
        rows.collect::<Result<_, _>>().ctx("list sessions")
    }

    pub fn session(&self, id: &str) -> Result<Session, String> {
        query_session(&self.conn()?, id)
    }

    pub fn align_started_at(&self, id: &str, started_at_ms: i64) -> Result<Session, String> {
        self.conn()?
            .execute(
                "UPDATE sessions SET started_at_ms = ?2 WHERE id = ?1",
                params![id, started_at_ms],
            )
            .ctx("align meeting clock")?;
        self.session(id)
    }

    pub fn note_warning(&self, id: &str, warning: &str) -> Result<(), String> {
        self.conn()?
            .execute(
                "UPDATE sessions SET error = ?2 WHERE id = ?1",
                params![id, warning],
            )
            .ctx("save meeting warning")?;
        Ok(())
    }

    /// Segments of a session in timeline order (start sample, then track). `limit` is capped at 500.
    pub fn segment_count(&self, id: &str) -> Result<u64, String> {
        self.conn()?
            .query_row(
                "SELECT COUNT(*) FROM segments WHERE session_id = ?1",
                [id],
                |row| row.get::<_, u64>(0),
            )
            .ctx("count meeting segments")
    }

    pub fn segments(&self, id: &str, offset: u64, limit: u32) -> Result<Vec<Segment>, String> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {SEGMENT_COLUMNS} FROM segments seg WHERE seg.session_id = ?1
                 ORDER BY seg.start_sample, seg.track LIMIT ?2 OFFSET ?3"
            ))
            .ctx("list segments")?;
        let rows = stmt
            .query_map(params![id, limit.min(MAX_PAGE), to_i64(offset)?], |row| {
                self.map_segment(row)
            })
            .ctx("list segments")?;
        rows.collect::<Result<_, _>>().ctx("list segments")
    }

    /// Earliest segment, across all sessions, with neither a transcript nor an ASR error.
    #[cfg(test)]
    pub fn next_pending(&self) -> Result<Option<Segment>, String> {
        self.next_pending_for(None)
    }

    pub fn next_pending_for(&self, preferred: Option<&str>) -> Result<Option<Segment>, String> {
        self.conn()?
            .query_row(
                &format!(
                    "SELECT {SEGMENT_COLUMNS} FROM segments seg JOIN sessions s ON s.id = seg.session_id
                     WHERE seg.text IS NULL AND seg.asr_error IS NULL
                     ORDER BY CASE WHEN seg.session_id = ?1 THEN 0 ELSE 1 END,
                     s.started_at_ms, seg.start_sample, seg.track LIMIT 1"
                ),
                [preferred],
                |row| self.map_segment(row),
            )
            .optional()
            .ctx("query pending segment")
    }

    pub fn set_transcript(&self, segment_id: i64, text: &str) -> Result<(), String> {
        let changed = self
            .conn()?
            .execute(
                "UPDATE segments SET text = ?2, asr_error = NULL, asr_status = 'final' WHERE id = ?1",
                params![segment_id, text],
            )
            .ctx("store transcript")?;
        expect_one(changed, "segment")
    }

    pub fn set_no_speech(&self, segment_id: i64) -> Result<(), String> {
        self.conn()?
            .execute(
                "UPDATE segments SET text = '', asr_status = 'no_speech' WHERE id = ?1",
                [segment_id],
            )
            .ctx("store VAD result")?;
        Ok(())
    }

    pub fn set_asr_error(&self, segment_id: i64, error: &str) -> Result<(), String> {
        let changed = self
            .conn()?
            .execute(
                "UPDATE segments SET asr_error = ?2, asr_status = 'error' WHERE id = ?1",
                params![segment_id, error],
            )
            .ctx("store ASR error")?;
        expect_one(changed, "segment")
    }

    /// Clears ASR errors of a session so its failed segments become pending again.
    pub fn retry_asr(&self, id: &str) -> Result<(), String> {
        let conn = self.conn()?;
        query_session(&conn, id)?;
        conn.execute(
            "UPDATE segments SET asr_error = NULL, text = NULL, asr_status = 'force' WHERE session_id = ?1 AND (delivered_samples IS NULL OR delivered_samples != 0) AND (asr_error IS NOT NULL OR asr_status = 'no_speech')",
            [id],
        )
        .ctx("retry ASR")?;
        Ok(())
    }

    pub fn set_ai_enabled(&self, id: &str, enabled: bool) -> Result<(), String> {
        let changed = self
            .conn()?
            .execute(
                "UPDATE sessions SET ai_enabled = ?2 WHERE id = ?1",
                params![id, enabled],
            )
            .ctx("update session")?;
        expect_one(changed, "session")
    }

    pub fn fail_session(&self, id: &str, error: &str) -> Result<(), String> {
        let changed = self
            .conn()?
            .execute(
                "UPDATE sessions SET status = ?2, error = ?3 WHERE id = ?1",
                params![id, STATUS_FAILED, error],
            )
            .ctx("fail session")?;
        expect_one(changed, "session")
    }

    /// Audio still waiting for ASR in a session, summed over both tracks.
    pub fn backlog_ms(&self, id: &str) -> Result<u64, String> {
        let samples: i64 = self
            .conn()?
            .query_row(
                "SELECT COALESCE(SUM(sample_count), 0) FROM segments
                 WHERE session_id = ?1 AND text IS NULL AND asr_error IS NULL",
                [id],
                |row| row.get(0),
            )
            .ctx("compute backlog")?;
        Ok(samples_to_ms(samples as u64))
    }

    pub fn track_stats(&self, id: &str) -> Result<Vec<TrackStats>, String> {
        let conn = self.conn()?;
        query_session(&conn, id)?;
        (0..TRACKS as u8)
            .map(|track| query_track_stats(&conn, id, track))
            .collect()
    }

    /// Decodes a segment to f32 samples. The path is resolved from the index, not
    /// taken from `segment.path`, so only archive files can be read.
    pub fn read_segment(&self, segment: &Segment) -> Result<Vec<f32>, String> {
        let (session_id, file): (String, String) = self
            .conn()?
            .query_row(
                "SELECT session_id, file FROM segments WHERE id = ?1",
                [segment.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ctx("look up segment")?
            .ok_or_else(|| format!("unknown segment {}", segment.id))?;
        read_samples(&self.dir_path(&session_id).join(file))
    }

    pub fn session_dir(&self, id: &str) -> Result<PathBuf, String> {
        query_session(&self.conn()?, id)?;
        Ok(self.dir_path(id))
    }

    /// Opens the writer of a `recording` session, resuming after its last indexed window.
    pub fn writer(&self, session: &Session, origin_ns: u64) -> Result<ArchiveWriter, String> {
        let conn = self.conn()?;
        let (status, stored_origin): (String, i64) = conn
            .query_row(
                "SELECT status, origin_ns FROM sessions WHERE id = ?1",
                [&session.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ctx("look up session")?
            .ok_or_else(|| format!("unknown session {}", session.id))?;
        if status != STATUS_RECORDING {
            return Err(format!("session {} is {status}, not recording", session.id));
        }
        if stored_origin != to_i64(origin_ns)? {
            return Err(format!(
                "session {} has a different time origin",
                session.id
            ));
        }
        let mut tracks: [TrackBuffer; TRACKS] = Default::default();
        for (track, buf) in (0u8..).zip(tracks.iter_mut()) {
            let end: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(start_sample + sample_count), 0) FROM segments
                     WHERE session_id = ?1 AND track = ?2",
                    params![session.id, track],
                    |row| row.get(0),
                )
                .ctx("resume track")?;
            let end = end as u64;
            if !end.is_multiple_of(WINDOW_SAMPLES) {
                return Err(format!("session {} ends on a partial window", session.id));
            }
            buf.sealed = end;
            buf.stats = query_track_stats(&conn, &session.id, track)?;
            buf.samples.reserve_exact(2 * WINDOW);
        }
        Ok(ArchiveWriter {
            session_id: session.id.clone(),
            dir: self.dir_path(&session.id),
            origin_ns,
            conn,
            tracks,
            watermark: 0,
            finished: false,
        })
    }

    /// Streams a session into a stereo 16 kHz WAV (left = microphone, right = system),
    /// one window at a time. Missing segments are exported as silence.
    pub fn export_wav(&self, id: &str, dest: &Path) -> Result<(), String> {
        let conn = self.conn()?;
        query_session(&conn, id)?;
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(start_sample + sample_count), 0) FROM segments WHERE session_id = ?1",
                [id],
                |row| row.get(0),
            )
            .ctx("measure session")?;
        let mut stmt = conn
            .prepare(
                "SELECT file FROM segments WHERE session_id = ?1 AND track = ?2 AND start_sample = ?3",
            )
            .ctx("export session")?;
        let dir = self.dir_path(id);
        let spec = hound::WavSpec {
            channels: 2,
            ..wav_spec()
        };
        let result = (|| {
            let mut out = hound::WavWriter::create(dest, spec).ctx("create export")?;
            for start in (0..total as u64).step_by(WINDOW) {
                let len = (total as u64 - start).min(WINDOW_SAMPLES) as usize;
                let mut channels = [vec![0.0f32; len], vec![0.0f32; len]];
                for (track, channel) in (0u8..).zip(channels.iter_mut()) {
                    let file: Option<String> = stmt
                        .query_row(params![id, track, start as i64], |row| row.get(0))
                        .optional()
                        .ctx("look up segment")?;
                    if let Some(file) = file {
                        let samples = read_samples(&dir.join(file))?;
                        let n = samples.len().min(len);
                        channel[..n].copy_from_slice(&samples[..n]);
                    }
                }
                for (left, right) in channels[0].iter().zip(&channels[1]) {
                    out.write_sample(to_i16(*left)).ctx("write export")?;
                    out.write_sample(to_i16(*right)).ctx("write export")?;
                }
            }
            out.finalize().ctx("finalize export")
        })();
        if result.is_err() {
            let _ = fs::remove_file(dest);
        }
        result
    }

    // --- internals ---

    fn conn(&self) -> Result<Connection, String> {
        let conn = Connection::open(self.root.join(DB_FILE)).ctx("open archive db")?;
        conn.busy_timeout(Duration::from_secs(5))
            .ctx("configure archive db")?;
        conn.pragma_update(None, "synchronous", "FULL")
            .ctx("configure archive db")?;
        Ok(conn)
    }

    fn dir_path(&self, id: &str) -> PathBuf {
        self.root.join(SESSIONS_DIR).join(id)
    }

    fn map_segment(&self, row: &Row) -> rusqlite::Result<Segment> {
        let session_id: String = row.get(1)?;
        let file: String = row.get(5)?;
        Ok(Segment {
            id: row.get(0)?,
            path: self
                .dir_path(&session_id)
                .join(file)
                .to_string_lossy()
                .into_owned(),
            session_id,
            track: row.get(2)?,
            start_sample: row.get::<_, i64>(3)? as u64,
            sample_count: row.get::<_, i64>(4)? as u64,
            text: row.get(6)?,
            asr_error: row.get(7)?,
            asr_status: row.get(8)?,
        })
    }

    /// Recovers sessions a crash or a failed write left unfinished: verified temp
    /// files are renamed into place, unindexed WAVs are indexed, and `recording`
    /// sessions become `interrupted`.
    fn recover(&self, conn: &mut Connection) -> Result<(), String> {
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM sessions WHERE status IN (?1, ?2)")
            .and_then(|mut stmt| {
                stmt.query_map([STATUS_RECORDING, STATUS_FAILED], |row| row.get(0))?
                    .collect()
            })
            .ctx("list unfinished sessions")?;
        for id in ids {
            let dir = self.dir_path(&id);
            if !dir.is_dir() {
                continue;
            }
            for name in list_files(&dir)? {
                let Some(final_name) = name
                    .strip_prefix('.')
                    .and_then(|n| n.strip_suffix(".tmp"))
                    .filter(|n| parse_segment_file(n).is_some())
                else {
                    continue;
                };
                let tmp = dir.join(&name);
                let target = dir.join(final_name);
                if verify_wav(&tmp).is_ok() && !target.exists() {
                    File::open(&tmp)
                        .and_then(|f| f.sync_all())
                        .and_then(|()| fs::rename(&tmp, &target))
                        .map_err(|e| format!("recover {}: {e}", tmp.display()))?;
                    sync_dir(&dir)?;
                } else {
                    fs::remove_file(&tmp).map_err(|e| format!("discard {}: {e}", tmp.display()))?;
                }
            }
            let indexed: HashSet<String> = conn
                .prepare("SELECT file FROM segments WHERE session_id = ?1")
                .and_then(|mut stmt| stmt.query_map([&id], |row| row.get(0))?.collect())
                .ctx("list indexed segments")?;
            let tx = conn.transaction().ctx("index orphan segments")?;
            for name in list_files(&dir)? {
                let Some((track, index)) = parse_segment_file(&name) else {
                    continue;
                };
                if indexed.contains(&name) {
                    continue;
                }
                match verify_wav(&dir.join(&name)) {
                    Ok(count) => {
                        tx.execute(
                            "INSERT INTO segments (session_id, track, start_sample, sample_count, file)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![id, track, to_i64(index * WINDOW_SAMPLES)?, to_i64(count)?, name],
                        )
                        .ctx("index orphan segment")?;
                    }
                    Err(e) => log::warn!("Skipping unreadable meeting segment {name}: {e}"),
                }
            }
            tx.commit().ctx("index orphan segments")?;
        }
        conn.execute(
            "UPDATE sessions SET status = ?1, duration_ms = MAX(duration_ms, (
                 SELECT COALESCE(MAX(start_sample + sample_count), 0) * 1000 / ?3
                 FROM segments WHERE session_id = sessions.id))
             WHERE status = ?2",
            params![STATUS_INTERRUPTED, STATUS_RECORDING, SAMPLE_RATE],
        )
        .ctx("mark interrupted sessions")?;
        Ok(())
    }
}

const SESSION_COLUMNS: &str =
    "id, started_at_ms, duration_ms, status, microphone, system, ai_enabled, error";
const SEGMENT_COLUMNS: &str =
    "seg.id, seg.session_id, seg.track, seg.start_sample, seg.sample_count, seg.file, seg.text, seg.asr_error, seg.asr_status";

fn map_session(row: &Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        started_at_ms: row.get(1)?,
        duration_ms: row.get::<_, i64>(2)? as u64,
        status: row.get(3)?,
        microphone: row.get(4)?,
        system: row.get(5)?,
        ai_enabled: row.get(6)?,
        error: row.get(7)?,
    })
}

fn query_session(conn: &Connection, id: &str) -> Result<Session, String> {
    conn.query_row(
        &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
        [id],
        map_session,
    )
    .optional()
    .ctx("look up session")?
    .ok_or_else(|| format!("unknown session {id}"))
}

fn query_track_stats(conn: &Connection, id: &str, track: u8) -> Result<TrackStats, String> {
    let stats = conn
        .query_row(
            "SELECT delivered_samples, padded_samples, overlapped_samples, dropped_samples
             FROM tracks WHERE session_id = ?1 AND track = ?2",
            params![id, track],
            |row| {
                Ok(TrackStats {
                    track,
                    delivered_samples: row.get::<_, i64>(0)? as u64,
                    padded_samples: row.get::<_, i64>(1)? as u64,
                    overlapped_samples: row.get::<_, i64>(2)? as u64,
                    dropped_samples: row.get::<_, i64>(3)? as u64,
                })
            },
        )
        .optional()
        .ctx("read track stats")?;
    Ok(stats.unwrap_or(TrackStats {
        track,
        ..Default::default()
    }))
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// One track's unsealed audio: samples `[sealed, sealed + samples.len())` of the
/// timeline. Holds at most two windows; the oldest is sealed when it fills up.
#[derive(Default)]
struct TrackBuffer {
    /// Start of the first unsealed window (always window-aligned).
    sealed: u64,
    samples: Vec<f32>,
    /// Delivered (non-padding) samples in the first and second buffered window.
    window_delivered: [u64; 2],
    stats: TrackStats,
}

impl TrackBuffer {
    fn cursor(&self) -> u64 {
        self.sealed + self.samples.len() as u64
    }
}

/// Turns timestamped capture chunks into sealed segments. Lives on the archive
/// writer thread; memory stays bounded at two windows per track.
pub struct ArchiveWriter {
    session_id: String,
    dir: PathBuf,
    origin_ns: u64,
    conn: Connection,
    tracks: [TrackBuffer; TRACKS],
    /// Timeline position (samples) of the last `flush_until`.
    watermark: u64,
    finished: bool,
}

impl ArchiveWriter {
    /// Places `samples` on the track at `timestamp_ns` (uptime of the first sample).
    /// Gaps beyond 20 ms are padded with silence; samples overlapping audio already
    /// written on this track are dropped and counted.
    pub fn push(&mut self, track: u8, timestamp_ns: u64, samples: &[f32]) -> Result<(), String> {
        self.ensure_open()?;
        let t = track_index(track)?;
        let buf = &mut self.tracks[t];
        let (start, mut samples) = match timestamp_ns.checked_sub(self.origin_ns) {
            Some(offset) => (ns_to_samples(offset), samples),
            None => {
                let skip =
                    (ns_to_samples(self.origin_ns - timestamp_ns) as usize).min(samples.len());
                buf.stats.dropped_samples += skip as u64;
                (0, &samples[skip..])
            }
        };
        if samples.is_empty() {
            return Ok(());
        }
        let cursor = buf.cursor();
        if start > cursor.max(self.watermark) + MAX_LEAD_SAMPLES {
            buf.stats.dropped_samples += samples.len() as u64;
            return Ok(());
        }
        let mut padding = 0;
        if start + JITTER_SAMPLES < cursor {
            let overlap = ((cursor - start) as usize).min(samples.len());
            let late = (buf.sealed.saturating_sub(start) as usize).min(overlap);
            buf.stats.dropped_samples += late as u64;
            buf.stats.overlapped_samples += (overlap - late) as u64;
            samples = &samples[overlap..];
        } else if start > cursor + JITTER_SAMPLES {
            padding = start - cursor;
        }
        self.append(t, padding, samples)
    }

    /// Seals every complete window ending at or before `elapsed_ms` on both tracks,
    /// padding tracks that received no audio. Callers pass a time slightly behind
    /// the clock so late callbacks still land in their window.
    pub fn flush_until(&mut self, elapsed_ms: u64) -> Result<(), String> {
        self.ensure_open()?;
        let elapsed = ms_to_samples(elapsed_ms);
        self.watermark = self.watermark.max(elapsed);
        let target = elapsed / WINDOW_SAMPLES * WINDOW_SAMPLES;
        for t in 0..TRACKS {
            while self.tracks[t].sealed + WINDOW_SAMPLES <= target {
                let missing = WINDOW.saturating_sub(self.tracks[t].samples.len());
                self.append(t, missing as u64, &[])?;
                self.seal(t, WINDOW)?;
            }
        }
        Ok(())
    }

    /// Pads both tracks to the same length (at least `elapsed_ms`), seals everything,
    /// and records the final status and duration. The writer is closed afterwards.
    pub fn finish(&mut self, elapsed_ms: u64, status: &str) -> Result<(), String> {
        self.ensure_open()?;
        if ![STATUS_COMPLETED, STATUS_INTERRUPTED, STATUS_FAILED].contains(&status) {
            return Err(format!("invalid final status {status}"));
        }
        let total = self
            .tracks
            .iter()
            .map(TrackBuffer::cursor)
            .fold(ms_to_samples(elapsed_ms), u64::max);
        for t in 0..TRACKS {
            let missing = total - self.tracks[t].cursor();
            self.append(t, missing, &[])?;
            while self.tracks[t].sealed < total {
                let len = (total - self.tracks[t].sealed).min(WINDOW_SAMPLES) as usize;
                self.seal(t, len)?;
            }
        }
        self.conn
            .execute(
                "UPDATE sessions SET status = ?2, duration_ms = ?3 WHERE id = ?1",
                params![self.session_id, status, to_i64(samples_to_ms(total))?],
            )
            .ctx("finish session")?;
        self.finished = true;
        Ok(())
    }

    pub fn stats(&self, track: u8) -> Result<TrackStats, String> {
        Ok(self.tracks[track_index(track)?].stats)
    }

    /// Samples currently held in memory across both tracks.
    pub fn buffered_samples(&self) -> usize {
        self.tracks.iter().map(|buf| buf.samples.len()).sum()
    }

    fn ensure_open(&self) -> Result<(), String> {
        if self.finished {
            Err(format!("session {} is already finished", self.session_id))
        } else {
            Ok(())
        }
    }

    /// Appends `padding` zeros then `samples`, sealing the oldest window whenever the
    /// buffer holds two. If sealing fails, the unbuffered rest is counted as dropped.
    fn append(&mut self, t: usize, mut padding: u64, mut samples: &[f32]) -> Result<(), String> {
        while padding > 0 || !samples.is_empty() {
            let buf = &mut self.tracks[t];
            let room = 2 * WINDOW - buf.samples.len();
            if room == 0 {
                if let Err(e) = self.seal(t, WINDOW) {
                    self.tracks[t].stats.dropped_samples += padding + samples.len() as u64;
                    return Err(e);
                }
            } else if padding > 0 {
                let n = (padding as usize).min(room);
                buf.samples.resize(buf.samples.len() + n, 0.0);
                buf.stats.padded_samples += n as u64;
                padding -= n as u64;
            } else {
                let n = samples.len().min(room);
                let in_first = WINDOW.saturating_sub(buf.samples.len()).min(n);
                buf.window_delivered[0] += in_first as u64;
                buf.window_delivered[1] += (n - in_first) as u64;
                buf.samples.extend_from_slice(&samples[..n]);
                buf.stats.delivered_samples += n as u64;
                samples = &samples[n..];
            }
        }
        Ok(())
    }

    /// Writes the first `len` buffered samples of a track as a segment and indexes it.
    /// The buffer is only drained once both the file and the index are durable.
    fn seal(&mut self, t: usize, len: usize) -> Result<(), String> {
        let track = t as u8;
        let buf = &self.tracks[t];
        let start = buf.sealed;
        let file = segment_file(track, start / WINDOW_SAMPLES);
        write_segment(&self.dir, &file, &buf.samples[..len])?;

        let delivered = buf.window_delivered[0];
        // Pure padding (no callback delivered anything) has nothing to transcribe.
        let text = (delivered == 0).then_some("");
        let end_ms = samples_to_ms(start + len as u64);
        let stats = buf.stats;
        let tx = self.conn.transaction().ctx("index segment")?;
        tx.execute(
            "INSERT INTO segments (session_id, track, start_sample, sample_count, delivered_samples, file, text, asr_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CASE WHEN ?5 = 0 THEN 'no_speech' ELSE 'pending' END)",
            params![self.session_id, track, to_i64(start)?, len, to_i64(delivered)?, file, text],
        )
        .ctx("index segment")?;
        tx.execute(
            "INSERT OR REPLACE INTO tracks
             (session_id, track, delivered_samples, padded_samples, overlapped_samples, dropped_samples)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                self.session_id,
                track,
                to_i64(stats.delivered_samples)?,
                to_i64(stats.padded_samples)?,
                to_i64(stats.overlapped_samples)?,
                to_i64(stats.dropped_samples)?
            ],
        )
        .ctx("update track stats")?;
        tx.execute(
            "UPDATE sessions SET duration_ms = MAX(duration_ms, ?2) WHERE id = ?1",
            params![self.session_id, to_i64(end_ms)?],
        )
        .ctx("update session duration")?;
        tx.commit().ctx("index segment")?;

        let buf = &mut self.tracks[t];
        buf.samples.drain(..len);
        buf.sealed += len as u64;
        buf.window_delivered = [buf.window_delivered[1], 0];
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

fn wav_spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

fn segment_file(track: u8, index: u64) -> String {
    format!("t{track}-{index:06}.wav")
}

/// Parses `t<track>-<index>.wav` back into `(track, window index)`.
fn parse_segment_file(name: &str) -> Option<(u8, u64)> {
    let (track, index) = name
        .strip_prefix('t')?
        .strip_suffix(".wav")?
        .split_once('-')?;
    let track: u8 = track.parse().ok().filter(|t| (*t as usize) < TRACKS)?;
    let index: u64 = index.parse().ok()?;
    (segment_file(track, index) == name).then_some((track, index))
}

/// Writes a segment atomically: temp file, WAV finalize, fsync, rename, directory fsync.
fn write_segment(dir: &Path, file: &str, samples: &[f32]) -> Result<(), String> {
    let tmp = dir.join(format!(".{file}.tmp"));
    let result = (|| -> Result<(), String> {
        let mut out = File::create(&tmp).ctx("create segment")?;
        let mut wav =
            hound::WavWriter::new(BufWriter::new(&mut out), wav_spec()).ctx("write segment")?;
        for &sample in samples {
            wav.write_sample(to_i16(sample)).ctx("write segment")?;
        }
        wav.finalize().ctx("finalize segment")?;
        out.sync_all().ctx("sync segment")?;
        fs::rename(&tmp, dir.join(file)).ctx("publish segment")?;
        sync_dir(dir)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| format!("{e} ({})", dir.join(file).display()))
}

fn read_samples(path: &Path) -> Result<Vec<f32>, String> {
    let reader =
        hound::WavReader::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    reader
        .into_samples::<i16>()
        .map(|s| s.map(|s| s as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("read {}: {e}", path.display()))
}

/// Checks that a file is a complete, readable segment and returns its sample count.
fn verify_wav(path: &Path) -> Result<u64, String> {
    let reader = hound::WavReader::open(path).ctx("open")?;
    if reader.spec() != wav_spec() {
        return Err("unexpected WAV format".into());
    }
    let expected = reader.len() as u64;
    let mut read = 0u64;
    for sample in reader.into_samples::<i16>() {
        sample.ctx("read")?;
        read += 1;
    }
    if read == 0 || read != expected || read > WINDOW_SAMPLES {
        return Err(format!("{read} of {expected} samples"));
    }
    Ok(read)
}

fn list_files(dir: &Path) -> Result<Vec<String>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("list {}: {e}", dir.display()))?;
    entries
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("list {}: {e}", dir.display()))
}

fn sync_dir(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| format!("sync {}: {e}", dir.display()))?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

trait Context<T> {
    fn ctx(self, what: &str) -> Result<T, String>;
}

impl<T, E: std::fmt::Display> Context<T> for Result<T, E> {
    fn ctx(self, what: &str) -> Result<T, String> {
        self.map_err(|e| format!("{what}: {e}"))
    }
}

fn expect_one(changed: usize, what: &str) -> Result<(), String> {
    if changed == 1 {
        Ok(())
    } else {
        Err(format!("unknown {what}"))
    }
}

fn track_index(track: u8) -> Result<usize, String> {
    let t = track as usize;
    if t < TRACKS {
        Ok(t)
    } else {
        Err(format!("invalid track {track}"))
    }
}

fn to_i64(value: u64) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("value {value} out of range"))
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

fn ns_to_samples(ns: u64) -> u64 {
    ((ns as u128 * SAMPLE_RATE as u128 + 500_000_000) / 1_000_000_000) as u64
}

fn ms_to_samples(ms: u64) -> u64 {
    ms * (SAMPLE_RATE as u64 / 1000)
}

fn samples_to_ms(samples: u64) -> u64 {
    samples / (SAMPLE_RATE as u64 / 1000)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests;

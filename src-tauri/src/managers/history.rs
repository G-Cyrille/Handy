use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use log::{debug, error, info};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_specta::Event;

/// Database migrations for transcription history.
/// Each migration is applied in order. The library tracks which migrations
/// have been applied using SQLite's user_version pragma.
///
/// Note: For users upgrading from tauri-plugin-sql, migrate_from_tauri_plugin_sql()
/// converts the old _sqlx_migrations table tracking to the user_version pragma,
/// ensuring migrations don't re-run on existing databases.
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );",
    ),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN screenshot_file_name TEXT;"),
];

/// Column list shared by every `SELECT` that maps to [`HistoryEntry`].
const ENTRY_COLUMNS: &str = "id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested, screenshot_file_name";

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedHistory {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action")]
pub enum HistoryUpdatePayload {
    #[serde(rename = "added")]
    Added { entry: HistoryEntry },
    #[serde(rename = "updated")]
    Updated { entry: HistoryEntry },
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
    #[serde(rename = "toggled")]
    Toggled { id: i64 },
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct HistoryEntry {
    pub id: i64,
    pub file_name: String,
    pub timestamp: i64,
    pub saved: bool,
    pub title: String,
    pub transcription_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
    pub post_process_requested: bool,
    /// Basename of the JPEG screenshot context stored beside the WAV in the
    /// recordings directory.
    pub screenshot_file_name: Option<String>,
}

pub struct HistoryManager {
    app_handle: AppHandle,
    recordings_dir: PathBuf,
    db_path: PathBuf,
}

impl HistoryManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Create recordings directory in app data dir
        let app_data_dir = crate::portable::app_data_dir(app_handle)?;
        let recordings_dir = app_data_dir.join("recordings");
        let db_path = app_data_dir.join("history.db");

        // Ensure recordings directory exists
        if !recordings_dir.exists() {
            fs::create_dir_all(&recordings_dir)?;
            debug!("Created recordings directory: {:?}", recordings_dir);
        }

        let manager = Self {
            app_handle: app_handle.clone(),
            recordings_dir,
            db_path,
        };

        // Initialize database and run migrations synchronously
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing database at {:?}", self.db_path);

        let mut conn = Connection::open(&self.db_path)?;

        // Handle migration from tauri-plugin-sql to rusqlite_migration
        // tauri-plugin-sql used _sqlx_migrations table, rusqlite_migration uses user_version pragma
        self.migrate_from_tauri_plugin_sql(&conn)?;

        // Create migrations object and run to latest version
        let migrations = Migrations::new(MIGRATIONS.to_vec());

        // Validate migrations in debug builds
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid migrations");

        // Get current version before migration
        let version_before: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        debug!("Database version before migration: {}", version_before);

        // Apply any pending migrations
        migrations.to_latest(&mut conn)?;

        // Get version after migration
        let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if version_after > version_before {
            info!(
                "Database migrated from version {} to {}",
                version_before, version_after
            );
        } else {
            debug!("Database already at latest version {}", version_after);
        }

        Ok(())
    }

    /// Migrate from tauri-plugin-sql's migration tracking to rusqlite_migration's.
    /// tauri-plugin-sql used a _sqlx_migrations table, while rusqlite_migration uses
    /// SQLite's user_version pragma. This function checks if the old system was in use
    /// and sets the user_version accordingly so migrations don't re-run.
    fn migrate_from_tauri_plugin_sql(&self, conn: &Connection) -> Result<()> {
        // Check if the old _sqlx_migrations table exists
        let has_sqlx_migrations: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_sqlx_migrations {
            return Ok(());
        }

        // Check current user_version
        let current_version: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if current_version > 0 {
            // Already migrated to rusqlite_migration system
            return Ok(());
        }

        // Get the highest version from the old migrations table
        let old_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if old_version > 0 {
            info!(
                "Migrating from tauri-plugin-sql (version {}) to rusqlite_migration",
                old_version
            );

            // Set user_version to match the old migration state
            conn.pragma_update(None, "user_version", old_version)?;

            // Optionally drop the old migrations table (keeping it doesn't hurt)
            // conn.execute("DROP TABLE IF EXISTS _sqlx_migrations", [])?;

            info!(
                "Migration tracking converted: user_version set to {}",
                old_version
            );
        }

        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    fn map_history_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        Ok(HistoryEntry {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            timestamp: row.get("timestamp")?,
            saved: row.get("saved")?,
            title: row.get("title")?,
            transcription_text: row.get("transcription_text")?,
            post_processed_text: row.get("post_processed_text")?,
            post_process_prompt: row.get("post_process_prompt")?,
            post_process_requested: row.get("post_process_requested")?,
            screenshot_file_name: row.get("screenshot_file_name")?,
        })
    }

    pub fn recordings_dir(&self) -> &Path {
        &self.recordings_dir
    }

    /// Path of an entry's screenshot, or `None` when it has none. The stored
    /// name must be a plain basename so a tampered row can't escape the
    /// recordings directory.
    pub fn get_screenshot_file_path(&self, entry: &HistoryEntry) -> Option<PathBuf> {
        let file_name = entry.screenshot_file_name.as_deref()?;
        if !is_plain_basename(file_name) {
            error!(
                "Ignoring screenshot name for history entry {}: not a plain file name",
                entry.id
            );
            return None;
        }
        Some(self.recordings_dir.join(file_name))
    }

    /// Save a new history entry to the database. The WAV file must already be
    /// in the recordings directory; the screenshot JPEG (if any) is written
    /// here, paired with the row (see [`insert_with_screenshot`]).
    pub fn save_entry(
        &self,
        file_name: String,
        transcription_text: String,
        post_process_requested: bool,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
        screenshot: Option<&[u8]>,
    ) -> Result<HistoryEntry> {
        let timestamp = Utc::now().timestamp();
        let title = self.format_timestamp_title(timestamp);

        let conn = self.get_connection()?;
        let entry = insert_with_screenshot(
            &self.recordings_dir,
            &file_name,
            screenshot,
            |screenshot_file_name| {
                conn.execute(
                    "INSERT INTO transcription_history (
                        file_name,
                        timestamp,
                        saved,
                        title,
                        transcription_text,
                        post_processed_text,
                        post_process_prompt,
                        post_process_requested,
                        screenshot_file_name
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        &file_name,
                        timestamp,
                        false,
                        &title,
                        &transcription_text,
                        &post_processed_text,
                        &post_process_prompt,
                        post_process_requested,
                        &screenshot_file_name,
                    ],
                )?;

                Ok(HistoryEntry {
                    id: conn.last_insert_rowid(),
                    file_name: file_name.clone(),
                    timestamp,
                    saved: false,
                    title: title.clone(),
                    transcription_text,
                    post_processed_text,
                    post_process_prompt,
                    post_process_requested,
                    screenshot_file_name,
                })
            },
        )?;

        debug!("Saved history entry with id {}", entry.id);

        self.cleanup_old_entries()?;

        // Emit typed event for real-time frontend updates
        if let Err(e) = (HistoryUpdatePayload::Added {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    /// Update an existing history entry with new transcription results (used by retry).
    pub fn update_transcription(
        &self,
        id: i64,
        transcription_text: String,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
    ) -> Result<HistoryEntry> {
        let conn = self.get_connection()?;
        let updated = conn.execute(
            "UPDATE transcription_history
             SET transcription_text = ?1,
                 post_processed_text = ?2,
                 post_process_prompt = ?3
             WHERE id = ?4",
            params![
                transcription_text,
                post_processed_text,
                post_process_prompt,
                id
            ],
        )?;

        if updated == 0 {
            return Err(anyhow!("History entry {} not found", id));
        }

        let entry = conn.query_row(
            &format!("SELECT {ENTRY_COLUMNS} FROM transcription_history WHERE id = ?1"),
            params![id],
            Self::map_history_entry,
        )?;

        debug!("Updated transcription for history entry {}", id);

        if let Err(e) = (HistoryUpdatePayload::Updated {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    pub fn cleanup_old_entries(&self) -> Result<()> {
        let retention_period = crate::settings::get_recording_retention_period(&self.app_handle);

        match retention_period {
            crate::settings::RecordingRetentionPeriod::Never => {
                // Don't delete anything
                Ok(())
            }
            crate::settings::RecordingRetentionPeriod::PreserveLimit => {
                // Use the old count-based logic with history_limit
                let limit = crate::settings::get_history_limit(&self.app_handle);
                self.cleanup_by_count(limit)
            }
            _ => {
                // Use time-based logic
                self.cleanup_by_time(retention_period)
            }
        }
    }

    /// Delete rows and their recording files. Returns how many entries had a
    /// WAV file removed.
    fn delete_entries_and_files(&self, entries: &[EntryFiles]) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }

        let conn = self.get_connection()?;
        let mut deleted_count = 0;

        for entry in entries {
            // Delete database entry
            conn.execute(
                "DELETE FROM transcription_history WHERE id = ?1",
                params![entry.id],
            )?;

            if remove_entry_files(
                &self.recordings_dir,
                &entry.file_name,
                entry.screenshot_file_name.as_deref(),
            ) {
                deleted_count += 1;
            }
        }

        Ok(deleted_count)
    }

    fn cleanup_by_count(&self, limit: usize) -> Result<()> {
        let conn = self.get_connection()?;

        // Get all entries that are not saved, ordered by timestamp desc
        let mut stmt = conn.prepare(
            "SELECT id, file_name, screenshot_file_name FROM transcription_history WHERE saved = 0 ORDER BY timestamp DESC"
        )?;

        let rows = stmt.query_map([], EntryFiles::from_row)?;

        let mut entries: Vec<EntryFiles> = Vec::new();
        for row in rows {
            entries.push(row?);
        }

        if entries.len() > limit {
            let entries_to_delete = &entries[limit..];
            let deleted_count = self.delete_entries_and_files(entries_to_delete)?;

            if deleted_count > 0 {
                debug!("Cleaned up {} old history entries by count", deleted_count);
            }
        }

        Ok(())
    }

    fn cleanup_by_time(
        &self,
        retention_period: crate::settings::RecordingRetentionPeriod,
    ) -> Result<()> {
        let conn = self.get_connection()?;

        // Calculate cutoff timestamp (current time minus retention period)
        let now = Utc::now().timestamp();
        let cutoff_timestamp = match retention_period {
            crate::settings::RecordingRetentionPeriod::Days3 => now - (3 * 24 * 60 * 60), // 3 days in seconds
            crate::settings::RecordingRetentionPeriod::Weeks2 => now - (2 * 7 * 24 * 60 * 60), // 2 weeks in seconds
            crate::settings::RecordingRetentionPeriod::Months3 => now - (3 * 30 * 24 * 60 * 60), // 3 months in seconds (approximate)
            _ => unreachable!("Should not reach here"),
        };

        // Get all unsaved entries older than the cutoff timestamp
        let mut stmt = conn.prepare(
            "SELECT id, file_name, screenshot_file_name FROM transcription_history WHERE saved = 0 AND timestamp < ?1",
        )?;

        let rows = stmt.query_map(params![cutoff_timestamp], EntryFiles::from_row)?;

        let mut entries_to_delete: Vec<EntryFiles> = Vec::new();
        for row in rows {
            entries_to_delete.push(row?);
        }

        let deleted_count = self.delete_entries_and_files(&entries_to_delete)?;

        if deleted_count > 0 {
            debug!(
                "Cleaned up {} old history entries based on retention period",
                deleted_count
            );
        }

        Ok(())
    }

    pub async fn get_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(100));

        let mut entries: Vec<HistoryEntry> = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(&format!(
                    "SELECT {ENTRY_COLUMNS}
                     FROM transcription_history
                     WHERE id < ?1
                     ORDER BY id DESC
                     LIMIT ?2"
                ))?;
                let result = stmt
                    .query_map(params![cursor_id, fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (None, Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(&format!(
                    "SELECT {ENTRY_COLUMNS}
                     FROM transcription_history
                     ORDER BY id DESC
                     LIMIT ?1"
                ))?;
                let result = stmt
                    .query_map(params![fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (_, None) => {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {ENTRY_COLUMNS}
                     FROM transcription_history
                     ORDER BY id DESC"
                ))?;
                let result = stmt
                    .query_map([], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedHistory { entries, has_more })
    }

    #[cfg(test)]
    fn get_latest_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS}
             FROM transcription_history
             ORDER BY timestamp DESC
             LIMIT 1"
        ))?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    /// Get the latest entry with non-empty transcription text.
    pub fn get_latest_completed_entry(&self) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        Self::get_latest_completed_entry_with_conn(&conn)
    }

    fn get_latest_completed_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS}
             FROM transcription_history
             WHERE transcription_text != ''
             ORDER BY timestamp DESC
             LIMIT 1"
        ))?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    pub async fn toggle_saved_status(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get current saved status
        let current_saved: bool = conn.query_row(
            "SELECT saved FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get("saved"),
        )?;

        let new_saved = !current_saved;

        conn.execute(
            "UPDATE transcription_history SET saved = ?1 WHERE id = ?2",
            params![new_saved, id],
        )?;

        debug!("Toggled saved status for entry {}: {}", id, new_saved);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Toggled { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    pub fn get_audio_file_path(&self, file_name: &str) -> PathBuf {
        self.recordings_dir.join(file_name)
    }

    pub async fn get_entry_by_id(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLUMNS}
             FROM transcription_history
             WHERE id = ?1"
        ))?;

        let entry = stmt.query_row([id], Self::map_history_entry).optional()?;

        Ok(entry)
    }

    pub async fn delete_entry(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Delete the recording files first; database deletion continues even
        // if a file removal fails.
        if let Some(entry) = self.get_entry_by_id(id).await? {
            remove_entry_files(
                &self.recordings_dir,
                &entry.file_name,
                entry.screenshot_file_name.as_deref(),
            );
        }

        // Delete from database
        conn.execute(
            "DELETE FROM transcription_history WHERE id = ?1",
            params![id],
        )?;

        debug!("Deleted history entry with id: {}", id);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    fn format_timestamp_title(&self, timestamp: i64) -> String {
        if let Some(utc_datetime) = DateTime::from_timestamp(timestamp, 0) {
            // Convert UTC to local timezone
            let local_datetime = utc_datetime.with_timezone(&Local);
            local_datetime.format("%B %e, %Y - %l:%M%p").to_string()
        } else {
            format!("Recording {}", timestamp)
        }
    }
}

/// The files a history row owns, as needed by the cleanup paths.
struct EntryFiles {
    id: i64,
    file_name: String,
    screenshot_file_name: Option<String>,
}

impl EntryFiles {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            screenshot_file_name: row.get("screenshot_file_name")?,
        })
    }
}

/// Screenshot basename derived from the WAV it was captured with.
fn screenshot_file_name_for(wav_file_name: &str) -> String {
    let stem = wav_file_name.strip_suffix(".wav").unwrap_or(wav_file_name);
    format!("{stem}.jpg")
}

/// Insert a row together with its screenshot so the two only ever exist as a
/// pair. The JPEG is written first and `insert` receives its basename; if the
/// insert fails the JPEG is removed again, and if the JPEG cannot be written
/// the row is inserted without one. Errors from `insert` are passed through.
fn insert_with_screenshot<T>(
    recordings_dir: &Path,
    wav_file_name: &str,
    screenshot: Option<&[u8]>,
    insert: impl FnOnce(Option<String>) -> Result<T>,
) -> Result<T> {
    let screenshot_file_name = screenshot.and_then(|jpeg| {
        let file_name = screenshot_file_name_for(wav_file_name);
        match fs::write(recordings_dir.join(&file_name), jpeg) {
            Ok(()) => {
                debug!("Saved screenshot {}", file_name);
                Some(file_name)
            }
            Err(err) => {
                error!("Failed to save screenshot {}: {}", file_name, err);
                None
            }
        }
    });

    let result = insert(screenshot_file_name.clone());
    if result.is_err() {
        if let Some(file_name) = screenshot_file_name {
            remove_recording_file(recordings_dir, &file_name);
        }
    }
    result
}

fn is_plain_basename(file_name: &str) -> bool {
    !file_name.is_empty()
        && Path::new(file_name)
            .file_name()
            .is_some_and(|name| name == file_name)
}

/// Remove an entry's WAV and screenshot from the recordings directory.
/// Returns whether the WAV was removed. A screenshot name that is not a plain
/// basename is ignored rather than resolved outside the directory.
fn remove_entry_files(
    recordings_dir: &Path,
    file_name: &str,
    screenshot_file_name: Option<&str>,
) -> bool {
    let wav_removed = remove_recording_file(recordings_dir, file_name);
    if let Some(screenshot) = screenshot_file_name.filter(|name| is_plain_basename(name)) {
        remove_recording_file(recordings_dir, screenshot);
    }
    wav_removed
}

fn remove_recording_file(recordings_dir: &Path, file_name: &str) -> bool {
    let path = recordings_dir.join(file_name);
    if !path.exists() {
        return false;
    }
    match fs::remove_file(&path) {
        Ok(()) => {
            debug!("Deleted recording file: {}", file_name);
            true
        }
        Err(e) => {
            error!("Failed to delete recording file {}: {}", file_name, e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn setup_conn() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        Migrations::new(MIGRATIONS.to_vec())
            .to_latest(&mut conn)
            .expect("apply migrations");
        conn
    }

    fn insert_entry(conn: &Connection, timestamp: i64, text: &str, post_processed: Option<&str>) {
        insert_entry_with_screenshot(conn, timestamp, text, post_processed, None);
    }

    fn insert_entry_with_screenshot(
        conn: &Connection,
        timestamp: i64,
        text: &str,
        post_processed: Option<&str>,
        screenshot_file_name: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested,
                screenshot_file_name
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                format!("handy-{}.wav", timestamp),
                timestamp,
                false,
                format!("Recording {}", timestamp),
                text,
                post_processed,
                Option::<String>::None,
                false,
                screenshot_file_name,
            ],
        )
        .expect("insert history entry");
    }

    #[test]
    fn get_latest_entry_returns_none_when_empty() {
        let conn = setup_conn();
        let entry = HistoryManager::get_latest_entry_with_conn(&conn).expect("fetch latest entry");
        assert!(entry.is_none());
    }

    #[test]
    fn get_latest_entry_returns_newest_entry() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "first", None);
        insert_entry(&conn, 200, "second", Some("processed"));

        let entry = HistoryManager::get_latest_entry_with_conn(&conn)
            .expect("fetch latest entry")
            .expect("entry exists");

        assert_eq!(entry.timestamp, 200);
        assert_eq!(entry.transcription_text, "second");
        assert_eq!(entry.post_processed_text.as_deref(), Some("processed"));
    }

    #[test]
    fn get_latest_completed_entry_skips_empty_entries() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "completed", None);
        insert_entry(&conn, 200, "", None);

        let entry = HistoryManager::get_latest_completed_entry_with_conn(&conn)
            .expect("fetch latest completed entry")
            .expect("completed entry exists");

        assert_eq!(entry.timestamp, 100);
        assert_eq!(entry.transcription_text, "completed");
    }

    #[test]
    fn migration_adds_nullable_screenshot_column() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "no screenshot", None);
        insert_entry_with_screenshot(&conn, 200, "with screenshot", None, Some("handy-200.jpg"));

        let latest = HistoryManager::get_latest_entry_with_conn(&conn)
            .expect("fetch latest entry")
            .expect("entry exists");
        assert_eq!(
            latest.screenshot_file_name.as_deref(),
            Some("handy-200.jpg")
        );

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {ENTRY_COLUMNS} FROM transcription_history WHERE timestamp = 100"
            ))
            .unwrap();
        let older = stmt
            .query_row([], HistoryManager::map_history_entry)
            .unwrap();
        assert_eq!(older.screenshot_file_name, None);
    }

    #[test]
    fn screenshot_name_follows_the_wav_stem() {
        assert_eq!(screenshot_file_name_for("handy-123.wav"), "handy-123.jpg");
    }

    #[test]
    fn only_plain_basenames_are_accepted_for_screenshots() {
        assert!(is_plain_basename("handy-123.jpg"));
        assert!(!is_plain_basename(""));
        assert!(!is_plain_basename("../handy-123.jpg"));
        assert!(!is_plain_basename("/etc/passwd"));
        assert!(!is_plain_basename("sub/handy-123.jpg"));
    }

    #[test]
    fn insert_failure_removes_the_screenshot_it_wrote() {
        let dir = tempfile::tempdir().unwrap();

        let result: Result<()> = insert_with_screenshot(
            dir.path(),
            "handy-1.wav",
            Some(b"jpg"),
            |screenshot_file_name| {
                assert_eq!(screenshot_file_name.as_deref(), Some("handy-1.jpg"));
                assert!(dir.path().join("handy-1.jpg").exists());
                Err(anyhow!("insert failed"))
            },
        );

        assert!(result.is_err());
        assert!(!dir.path().join("handy-1.jpg").exists());
    }

    #[test]
    fn insert_success_keeps_the_screenshot() {
        let dir = tempfile::tempdir().unwrap();

        let stored = insert_with_screenshot(dir.path(), "handy-1.wav", Some(b"jpg"), Ok).unwrap();

        assert_eq!(stored.as_deref(), Some("handy-1.jpg"));
        assert_eq!(fs::read(dir.path().join("handy-1.jpg")).unwrap(), b"jpg");
    }

    #[test]
    fn unwritable_screenshot_inserts_the_row_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");

        let stored = insert_with_screenshot(&missing, "handy-1.wav", Some(b"jpg"), Ok).unwrap();

        assert_eq!(stored, None);
    }

    #[test]
    fn removing_entry_files_deletes_wav_and_screenshot_together() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("handy-1.wav");
        let jpg = dir.path().join("handy-1.jpg");
        fs::write(&wav, b"wav").unwrap();
        fs::write(&jpg, b"jpg").unwrap();

        assert!(remove_entry_files(
            dir.path(),
            "handy-1.wav",
            Some("handy-1.jpg")
        ));

        assert!(!wav.exists());
        assert!(!jpg.exists());
    }

    #[test]
    fn removing_entry_files_ignores_screenshot_paths_outside_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.jpg");
        let recordings = dir.path().join("recordings");
        fs::create_dir(&recordings).unwrap();
        fs::write(recordings.join("handy-1.wav"), b"wav").unwrap();
        fs::write(&outside, b"jpg").unwrap();

        assert!(remove_entry_files(
            &recordings,
            "handy-1.wav",
            Some("../outside.jpg")
        ));

        assert!(outside.exists());
    }
}

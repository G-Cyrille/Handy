//! Meeting preparation and conversations live beside the immutable audio archive.
use super::{insights::write_json, lock, MeetingManager};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingMessage {
    pub role: String,
    pub text: String,
    pub at_ms: u64,
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct QueuedTurn {
    pub request: super::agents::MeetingRequest,
    pub message_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct Discussion {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub effort: String,
    pub fast: bool,
    pub status: String,
    pub draft: String,
    pub messages: Vec<MeetingMessage>,
    pub native_id: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub queued: bool,
    #[serde(default)]
    pub live_text: String,
    #[serde(default)]
    pub turns: Vec<QueuedTurn>,
}

impl Discussion {
    pub fn new(id: String, title: String, provider: String) -> Self {
        Self {
            id,
            title,
            provider,
            model: String::new(),
            effort: "medium".into(),
            fast: false,
            status: "idle".into(),
            draft: String::new(),
            messages: vec![],
            native_id: None,
            error: None,
            queued: false,
            live_text: String::new(),
            turns: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct TimelineEntry {
    pub id: String,
    pub at_ms: u64,
    /// `note` or `capture`. Capture paths are relative to the session directory.
    pub kind: String,
    pub text: String,
    pub image: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Type)]
#[serde(default)]
pub struct MeetingWorkspace {
    pub title: String,
    pub folder: String,
    pub goal: String,
    pub context: String,
    pub participants: String,
    pub working_directory: String,
    pub documents: Vec<String>,
    pub related_meetings: Vec<String>,
    pub entries: Vec<TimelineEntry>,
    pub discussions: Vec<Discussion>,
    pub dismissed_suggestions: Vec<String>,
    pub proposed_actions: Vec<String>,
    pub favorites: Vec<String>,
    pub corrections: HashMap<String, String>,
    pub speakers: HashMap<String, String>,
    pub imported_video: Option<String>,
    pub media_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceEdit {
    Preparation {
        title: String,
        folder: String,
        goal: String,
        context: String,
        participants: String,
        working_directory: String,
        documents: Vec<String>,
        related_meetings: Vec<String>,
    },
    Note {
        id: Option<String>,
        at_ms: u64,
        text: String,
    },
    RemoveEntry {
        id: String,
    },
    Draft {
        chat_id: String,
        text: String,
    },
    DismissSuggestion {
        text: String,
    },
    Favorites {
        actions: Vec<String>,
    },
    Speaker {
        segment_id: String,
        name: String,
    },
    CloseDiscussion {
        chat_id: String,
    },
}

pub fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
pub fn identifier() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        now(),
        std::process::id(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

pub fn read(dir: &Path) -> Result<MeetingWorkspace, String> {
    let mut data: MeetingWorkspace = match std::fs::read(dir.join("workspace.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Meeting workspace: {e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => MeetingWorkspace::default(),
        Err(e) => return Err(e.to_string()),
    };
    data.folder = data
        .folder
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if data.discussions.is_empty() {
        data.discussions.push(Discussion::new(
            "main".into(),
            "Principal".into(),
            "codex".into(),
        ));
    }
    Ok(data)
}

pub fn write(dir: &Path, data: &MeetingWorkspace) -> Result<(), String> {
    write_json(&dir.join("workspace.json"), data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_survives_restart_and_reuses_the_recording_archive() {
        let temp = tempfile::tempdir().unwrap();
        let manager = MeetingManager::test_manager(temp.path().into());
        let session = manager
            .create_preparation("Point novembre".into(), "Tibria/Interne".into())
            .unwrap();
        manager
            .edit_workspace(
                &session.id,
                WorkspaceEdit::Note {
                    id: None,
                    at_ms: 0,
                    text: "Vérifier les profils terminés".into(),
                },
            )
            .unwrap();
        manager
            .edit_workspace(
                &session.id,
                WorkspaceEdit::Draft {
                    chat_id: "main".into(),
                    text: "À discuter".into(),
                },
            )
            .unwrap();
        drop(manager);
        let manager = MeetingManager::test_manager(temp.path().into());
        let data = manager.workspace(&session.id).unwrap();
        assert_eq!(data.entries[0].text, "Vérifier les profils terminés");
        assert_eq!(data.discussions[0].draft, "À discuter");
        assert_eq!(
            manager.store.session(&session.id).unwrap().status,
            "prepared"
        );
        let running = manager
            .store
            .activate_preparation(&session.id, "mic", "system", true, 123)
            .unwrap();
        assert_eq!(running.id, session.id);
        assert_eq!(running.status, "recording");
        assert!(manager
            .store
            .activate_preparation(&session.id, "mic", "system", true, 123)
            .is_err());
    }

    #[test]
    fn stopping_one_agent_keeps_history_main_and_recording() {
        let temp = tempfile::tempdir().unwrap();
        let manager = MeetingManager::test_manager(temp.path().into());
        let session = manager
            .store
            .create_session("mic".into(), "system".into(), false, 0)
            .unwrap();
        let dir = manager.store.session_dir(&session.id).unwrap();
        let mut data = read(&dir).unwrap();
        for id in ["a", "b"] {
            let mut chat = Discussion::new(id.into(), id.into(), "codex".into());
            chat.status = "running".into();
            chat.live_text = "Sources trouvées".into();
            data.discussions.push(chat);
        }
        write(&dir, &data).unwrap();
        let data = manager.cancel_reply(&session.id, "a").unwrap();
        assert_eq!(data.discussions[1].status, "stopped");
        assert_eq!(data.discussions[1].messages[0].text, "Sources trouvées");
        assert_eq!(data.discussions[2].status, "running");
        assert_eq!(
            manager.store.session(&session.id).unwrap().status,
            "recording"
        );
        assert!(manager
            .edit_workspace(
                &session.id,
                WorkspaceEdit::CloseDiscussion {
                    chat_id: "main".into()
                }
            )
            .is_err());
        manager.recover_discussions().unwrap();
        assert_eq!(
            manager.workspace(&session.id).unwrap().discussions[2].status,
            "interrupted"
        );
    }
}

impl MeetingManager {
    pub fn workspace(&self, id: &str) -> Result<MeetingWorkspace, String> {
        let _guard = lock(&self.workspace_lock);
        read(&self.store.session_dir(id)?)
    }

    pub fn edit_workspace(
        &self,
        id: &str,
        edit: WorkspaceEdit,
    ) -> Result<MeetingWorkspace, String> {
        let _guard = lock(&self.workspace_lock);
        let dir = self.store.session_dir(id)?;
        let mut data = read(&dir)?;
        match edit {
            WorkspaceEdit::Preparation {
                title,
                folder,
                goal,
                context,
                participants,
                working_directory,
                documents,
                related_meetings,
            } => {
                if title.len() > 300
                    || folder.len() > 1000
                    || goal.len() > 16_000
                    || context.len() > 64_000
                    || documents.len() > 100
                {
                    return Err("Meeting context is too large".into());
                }
                data.title = title;
                data.folder = folder;
                data.goal = goal;
                data.context = context;
                data.participants = participants;
                data.working_directory = working_directory;
                data.documents = documents;
                if related_meetings.len() > 10 {
                    return Err("Attach at most 10 previous meetings".into());
                }
                data.related_meetings = related_meetings;
            }
            WorkspaceEdit::Note { id, at_ms, text } => {
                if text.trim().is_empty() || text.len() > 32_000 {
                    return Err("A note must contain 1–32000 bytes".into());
                }
                if let Some(entry) = id.and_then(|id| {
                    data.entries
                        .iter_mut()
                        .find(|e| e.id == id && e.kind == "note")
                }) {
                    entry.text = text;
                } else {
                    data.entries.push(TimelineEntry {
                        id: identifier(),
                        at_ms,
                        kind: "note".into(),
                        text,
                        image: None,
                    });
                }
            }
            WorkspaceEdit::RemoveEntry { id } => data.entries.retain(|e| e.id != id),
            WorkspaceEdit::Draft { chat_id, text } => {
                if text.len() > 64_000 {
                    return Err("Message too long".into());
                }
                data.discussions
                    .iter_mut()
                    .find(|c| c.id == chat_id)
                    .ok_or("Unknown discussion")?
                    .draft = text;
            }
            WorkspaceEdit::DismissSuggestion { text } => {
                if !data.dismissed_suggestions.contains(&text) {
                    data.dismissed_suggestions.push(text);
                }
            }
            WorkspaceEdit::Favorites { actions } => {
                if actions.len() > 30 || actions.iter().any(|a| a.len() > 1000) {
                    return Err("Too many favorite actions".into());
                }
                data.favorites = actions;
            }
            WorkspaceEdit::Speaker { segment_id, name } => {
                data.speakers.insert(segment_id, name);
            }
            WorkspaceEdit::CloseDiscussion { chat_id } => {
                if chat_id == "main" {
                    return Err("The principal conversation cannot be closed".into());
                }
                if data
                    .discussions
                    .iter()
                    .any(|c| c.id == chat_id && c.status == "running")
                {
                    return Err("Stop this agent first".into());
                }
                data.discussions.retain(|c| c.id != chat_id);
            }
        }
        write(&dir, &data)?;
        Ok(data)
    }

    pub fn create_preparation(
        &self,
        title: String,
        folder: String,
    ) -> Result<super::archive::Session, String> {
        let session = self.store.create_preparation()?;
        let _guard = lock(&self.workspace_lock);
        let dir = self.store.session_dir(&session.id)?;
        let mut data = read(&dir)?;
        data.title = title.chars().take(300).collect();
        data.folder = folder.chars().take(1000).collect();
        write(&dir, &data)?;
        Ok(session)
    }

    pub fn recover_discussions(&self) -> Result<(), String> {
        for session in self.store.list_sessions()? {
            let dir = self.store.session_dir(&session.id)?;
            let mut data = read(&dir)?;
            let mut changed = false;
            for chat in &mut data.discussions {
                if chat.status == "running" {
                    chat.status = "interrupted".into();
                    chat.queued = false;
                    chat.turns.clear();
                    changed = true;
                }
            }
            if changed {
                write(&dir, &data)?;
            }
        }
        Ok(())
    }
}

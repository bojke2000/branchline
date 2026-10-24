use crate::{
    app::{EditorEdit, PreparedEdit},
    file_identity::{FileIdentity, FileIdentityState, OpenedFile},
    interchange::{ImportReport, read_native},
    session::{MapSession, SessionId, SessionState},
    storage::atomic_write_opened,
};
use std::{collections::BTreeMap, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct WorkspaceError(pub String);

pub struct Workspace {
    order: Vec<SessionId>,
    entries: BTreeMap<SessionId, SessionState>,
    active: SessionId,
    pending_activation: Option<SessionId>,
    storage_limit: usize,
    history_limit: usize,
    next_stamp: u64,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn from_restore(
        restore: crate::recovery::WorkspaceRestore,
        recover: bool,
    ) -> Result<Self, WorkspaceError> {
        use crate::recovery::RestoreContent;
        let mut workspace = Self::new();
        workspace.order.clear();
        workspace.entries.clear();
        let active = restore.active;
        for restored in restore.sessions {
            let tab = restored.tab;
            if !recover && tab.saved_path.is_none() {
                continue;
            }
            let outcome = (|| -> Result<Option<MapSession>, WorkspaceError> {
                let mut map = MapSession::new_blank(tab.id);
                let (document, clean, identity) = match restored.content {
                    RestoreContent::Recovered(document) if recover => {
                        let identity = match &tab.saved_path {
                            Some(path) => match FileIdentity::open(path) {
                                Ok(identity) => FileIdentityState::Known(identity),
                                Err(error) => FileIdentityState::Unresolved(error.to_string()),
                            },
                            None => FileIdentityState::Unsaved,
                        };
                        (document, false, identity)
                    }
                    RestoreContent::Failed(reason) if recover => {
                        return Err(WorkspaceError(reason));
                    }
                    _ => {
                        let Some(path) = &tab.saved_path else {
                            return Ok(None);
                        };
                        let opened = crate::file_identity::read_identified(path)
                            .map_err(|error| WorkspaceError(error.to_string()))?;
                        let document = read_native(&opened.bytes)
                            .map_err(|error| WorkspaceError(error.to_string()))?;
                        (document, true, FileIdentityState::Known(opened.identity))
                    }
                };
                map.editor
                    .replace_document(
                        document,
                        tab.saved_path.clone(),
                        clean,
                        Some(tab.document_id.to_string()),
                    )
                    .map_err(|error| WorkspaceError(error.to_string()))?;
                map.file_identity = identity;
                map.pristine_blank = false;
                if let Some(selected) = &tab.view.selected
                    && map.editor.document.nodes.contains_key(selected)
                {
                    map.editor
                        .select(selected)
                        .map_err(|error| WorkspaceError(error.to_string()))?;
                }
                map.viewport.pan = crate::layout::Point {
                    x: tab.view.pan[0],
                    y: tab.view.pan[1],
                };
                map.viewport.zoom = tab.view.zoom;
                Ok(Some(map))
            })();
            let failure = match outcome {
                Ok(Some(map)) => match workspace.insert_session(map) {
                    Ok(_) => continue,
                    Err(error) => error.to_string(),
                },
                Ok(None) => continue,
                Err(error) => error.to_string(),
            };
            let entry = SessionState::RestoreError(crate::session::RestoreFailure {
                path: tab.saved_path,
                reason: failure,
                retain_recovery: recover && tab.snapshot.is_some(),
            });
            let estimated = match &entry {
                SessionState::RestoreError(error) => error
                    .reason
                    .len()
                    .saturating_add(error.path.as_ref().map_or(0, |path| path.as_os_str().len()))
                    .saturating_add(256),
                _ => unreachable!(),
            };
            if workspace.estimated_bytes().saturating_add(estimated) > workspace.storage_limit {
                return Err(WorkspaceError("Restored workspace metadata exceeds storage capacity; the recovery store was retained.".into()));
            }
            workspace.order.push(tab.id);
            workspace.entries.insert(tab.id, entry);
        }
        if workspace.order.is_empty() {
            workspace.new_map()?;
        }
        workspace.active = active
            .filter(|id| workspace.entries.contains_key(id))
            .unwrap_or(workspace.order[0]);
        workspace.pending_activation = None;
        Ok(workspace)
    }

    fn manifest_bound(&self) -> usize {
        self.entries.values().fold(256usize, |total, entry| {
            total.saturating_add(match entry {
                SessionState::Map(map) => crate::recovery::editor_manifest_bound(&map.editor),
                SessionState::RestoreError(failure) => 2048usize
                    .saturating_add(failure.reason.len().saturating_mul(6))
                    .saturating_add(
                        failure
                            .path
                            .as_ref()
                            .map_or(0, |path| path.as_os_str().len().saturating_mul(6)),
                    ),
            })
        })
    }

    pub fn recovery_state(&self) -> crate::recovery::WorkspaceRecoveryState<'_> {
        crate::recovery::WorkspaceRecoveryState {
            active: Some(self.active),
            sessions: self
                .order
                .iter()
                .map(|id| match &self.entries[id] {
                    SessionState::Map(map) => crate::recovery::RecoverySession::Map(map),
                    SessionState::RestoreError(error) => crate::recovery::RecoverySession::Failed {
                        id: *id,
                        path: &error.path,
                        reason: &error.reason,
                        retain_snapshot: error.retain_recovery,
                    },
                })
                .collect(),
        }
    }

    pub fn new() -> Self {
        let id = SessionId::new();
        Self {
            order: vec![id],
            entries: BTreeMap::from([(id, SessionState::Map(Box::new(MapSession::new_blank(id))))]),
            active: id,
            pending_activation: None,
            storage_limit: 256 * 1024 * 1024,
            history_limit: 64 * 1024 * 1024,
            next_stamp: 0,
        }
    }

    pub fn active_id(&self) -> SessionId {
        self.active
    }
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn order(&self) -> &[SessionId] {
        &self.order
    }
    pub fn pending_activation(&self) -> Option<SessionId> {
        self.pending_activation
    }
    pub fn cancel_activation(&mut self) {
        self.pending_activation = None;
    }

    pub fn entry(&self, id: SessionId) -> Result<&SessionState, WorkspaceError> {
        self.entries
            .get(&id)
            .ok_or_else(|| WorkspaceError("The requested tab is no longer open.".into()))
    }

    pub fn session(&self, id: SessionId) -> Result<&MapSession, WorkspaceError> {
        match self.entry(id)? {
            SessionState::Map(map) => Ok(map),
            SessionState::RestoreError(error) => Err(WorkspaceError(error.reason.clone())),
        }
    }

    pub fn active(&self) -> Result<&MapSession, WorkspaceError> {
        self.session(self.active)
    }

    pub fn map(&self) -> &MapSession {
        self.active()
            .expect("map rendering requires a loaded active session")
    }

    pub fn map_mut(&mut self) -> &mut MapSession {
        match self.entries.get_mut(&self.active) {
            Some(SessionState::Map(map)) => map,
            _ => panic!("map editing requires a loaded active session"),
        }
    }

    pub fn session_mut(&mut self, id: SessionId) -> Result<&mut MapSession, WorkspaceError> {
        match self.entries.get_mut(&id) {
            Some(SessionState::Map(map)) => Ok(map),
            Some(SessionState::RestoreError(error)) => Err(WorkspaceError(error.reason.clone())),
            None => Err(WorkspaceError(
                "The requested tab is no longer open.".into(),
            )),
        }
    }

    pub fn estimated_bytes(&self) -> usize {
        self.entries
            .values()
            .map(|entry| match entry {
                SessionState::Map(map) => map.estimated_bytes() + 128,
                SessionState::RestoreError(error) => {
                    std::mem::size_of::<SessionState>()
                        + error.reason.len()
                        + error.path.as_ref().map_or(0, |path| path.as_os_str().len())
                        + 128
                }
            })
            .sum::<usize>()
            + self.order.len() * std::mem::size_of::<SessionId>()
            + std::mem::size_of::<Self>()
    }

    pub fn apply(&mut self, id: SessionId, edit: EditorEdit) -> Result<(), WorkspaceError> {
        let session = self.session(id)?;
        let prepared = session
            .editor
            .prepare(&edit)
            .map_err(|error| WorkspaceError(error.to_string()))?;
        let next_stamp = self.admit_edit(id, &prepared)?;
        self.commit_edit(id, prepared, next_stamp);
        Ok(())
    }

    fn admit_edit(&self, id: SessionId, prepared: &PreparedEdit) -> Result<u64, WorkspaceError> {
        let original = &self.session(id)?.editor;
        if self
            .manifest_bound()
            .saturating_sub(crate::recovery::editor_manifest_bound(original))
            .saturating_add(prepared.manifest_bound(original))
            > crate::document::MAX_FILE_BYTES
        {
            return Err(WorkspaceError("Workspace recovery metadata capacity exceeded; the previous map and drafts were retained.".into()));
        }
        let growth = self.estimated_bytes() - self.session(id)?.editor.estimated_bytes()
            + prepared.estimated_bytes();
        if growth > self.storage_limit {
            return Err(WorkspaceError(
                "Workspace storage capacity exceeded; the previous map and drafts were retained."
                    .into(),
            ));
        }
        self.next_stamp
            .checked_add(prepared.history_entries())
            .ok_or_else(|| WorkspaceError("History retention stamp capacity exceeded.".into()))
    }

    fn commit_edit(&mut self, id: SessionId, prepared: PreparedEdit, next_stamp: u64) {
        let stamp = self.next_stamp;
        let session = self.session_mut(id).expect("admitted session");
        session.editor.commit_prepared(prepared, stamp);
        if session.editor.revision != 0
            || session
                .editor
                .has_drafts()
                .expect("prepared draft owners are valid")
        {
            session.pristine_blank = false;
        }
        self.next_stamp = next_stamp;
        self.enforce_history_budget();
    }

    pub fn destination_owner(&self, path: &Path) -> Result<Option<SessionId>, WorkspaceError> {
        let destination = match std::fs::symlink_metadata(path) {
            Ok(_) => {
                Some(FileIdentity::open(path).map_err(|error| WorkspaceError(error.to_string()))?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(WorkspaceError(format!("{}: {error}", path.display()))),
        };
        for (id, entry) in &self.entries {
            let SessionState::Map(map) = entry else {
                continue;
            };
            let Some(selected) = &map.editor.path else {
                continue;
            };
            let loaded = match &map.file_identity {
                FileIdentityState::Known(identity) => identity,
                FileIdentityState::Unresolved(reason) => {
                    return Err(WorkspaceError(reason.clone()));
                }
                FileIdentityState::Unsaved => {
                    return Err(WorkspaceError(format!(
                        "{}: Saved-file identity is unresolved.",
                        selected.display()
                    )));
                }
            };
            let current =
                FileIdentity::open(selected).map_err(|error| WorkspaceError(error.to_string()))?;
            if &current != loaded {
                return Err(WorkspaceError(format!(
                    "{}: This file was replaced outside BMM. Its open tab was retained; resolve the changed file before overwriting or opening it again.",
                    selected.display()
                )));
            }
            if destination.as_ref() == Some(loaded) {
                return Ok(Some(*id));
            }
        }
        Ok(None)
    }

    pub fn open_native(&mut self, file: OpenedFile) -> Result<SessionId, WorkspaceError> {
        let current = FileIdentity::open(&file.selected_path)
            .map_err(|error| WorkspaceError(error.to_string()))?;
        if current != file.identity {
            return Err(WorkspaceError(
                "The selected file changed while it was being opened; no tab was added.".into(),
            ));
        }
        if let Some(owner) = self.destination_owner(&file.selected_path)? {
            self.request_activate(owner)?;
            return Ok(owner);
        }
        let document =
            read_native(&file.bytes).map_err(|error| WorkspaceError(error.to_string()))?;
        let mut map = MapSession::new_blank(SessionId::new());
        map.editor
            .replace_document(document, Some(file.selected_path), true, None)
            .map_err(|error| WorkspaceError(error.to_string()))?;
        map.file_identity = FileIdentityState::Known(file.identity);
        self.insert_session(map)
    }

    pub fn accept_import(&mut self, report: ImportReport) -> Result<SessionId, WorkspaceError> {
        let mut map = MapSession::new_blank(SessionId::new());
        map.editor
            .replace_document(report.document, None, false, None)
            .map_err(|error| WorkspaceError(error.to_string()))?;
        self.insert_session(map)
    }

    pub fn save(&mut self, id: SessionId, path: &Path) -> Result<(), WorkspaceError> {
        if self
            .destination_owner(path)?
            .is_some_and(|owner| owner != id)
        {
            return Err(WorkspaceError(
                "Another open tab owns this destination. Choose a different file.".into(),
            ));
        }
        let save = self
            .session(id)?
            .editor
            .prepare_save(path)
            .map_err(|error| WorkspaceError(error.to_string()))?;
        let next_stamp = self.admit_edit(id, &save.edit)?;
        let published = atomic_write_opened(path, &save.bytes)
            .map_err(|error| WorkspaceError(error.to_string()))?;
        let identity = FileIdentity::from_file(published).and_then(|published| {
            if FileIdentity::open(path)? == published {
                Ok(published)
            } else {
                Err(crate::storage::StorageError(
                    "The saved destination was replaced before its identity could be confirmed."
                        .into(),
                ))
            }
        });
        self.commit_edit(id, save.edit, next_stamp);
        match identity {
            Ok(identity) => {
                self.session_mut(id)?.file_identity = FileIdentityState::Known(identity)
            }
            Err(error) => {
                let reason = format!(
                    "The map was saved, but its new file identity could not be verified: {error}"
                );
                self.session_mut(id)?.file_identity = FileIdentityState::Unresolved(reason.clone());
                return Err(WorkspaceError(reason));
            }
        }
        Ok(())
    }

    fn enforce_history_budget(&mut self) {
        loop {
            let bytes: usize = self
                .entries
                .values()
                .filter_map(|entry| match entry {
                    SessionState::Map(map) => Some(map.editor.history.retained_bytes()),
                    _ => None,
                })
                .sum();
            if bytes <= self.history_limit {
                return;
            }
            let oldest = self
                .entries
                .iter()
                .filter_map(|(id, entry)| match entry {
                    SessionState::Map(map) => map
                        .editor
                        .history
                        .oldest_retained()
                        .map(|(stamp, _)| (*id, stamp)),
                    _ => None,
                })
                .min_by_key(|(id, stamp)| (*stamp, *id));
            let (id, stamp) = oldest.expect("nonempty retained history exceeds budget");
            self.session_mut(id)
                .expect("known map owner")
                .editor
                .history
                .evict_oldest(stamp);
        }
    }

    pub fn new_map(&mut self) -> Result<SessionId, WorkspaceError> {
        let id = SessionId::new();
        self.insert_session(MapSession::new_blank(id))
    }

    pub fn insert_session(&mut self, mut map: MapSession) -> Result<SessionId, WorkspaceError> {
        let id = map.id();
        if map.editor.revision != 0 {
            map.pristine_blank = false;
        }
        if self.entries.contains_key(&id) {
            return Err(WorkspaceError(
                "A map with this session identity is already open.".into(),
            ));
        }
        if self
            .manifest_bound()
            .saturating_add(crate::recovery::editor_manifest_bound(&map.editor))
            > crate::document::MAX_FILE_BYTES
        {
            return Err(WorkspaceError(
                "Workspace recovery metadata capacity exceeded; no tab was added.".into(),
            ));
        }
        if self.estimated_bytes() + map.estimated_bytes() + 128 + std::mem::size_of::<SessionId>()
            > self.storage_limit
        {
            return Err(WorkspaceError(
                "Workspace storage capacity exceeded; no tab was added.".into(),
            ));
        }
        self.entries.insert(id, SessionState::Map(Box::new(map)));
        self.order.push(id);
        self.request_activate(id)?;
        Ok(id)
    }

    pub fn request_activate(&mut self, id: SessionId) -> Result<(), WorkspaceError> {
        self.entry(id)?;
        self.pending_activation = (id != self.active).then_some(id);
        Ok(())
    }

    pub fn cycle(&mut self, backwards: bool) -> Result<(), WorkspaceError> {
        let current = self
            .order
            .iter()
            .position(|id| *id == self.active)
            .expect("active tab exists");
        let next = if backwards {
            (current + self.order.len() - 1) % self.order.len()
        } else {
            (current + 1) % self.order.len()
        };
        self.request_activate(self.order[next])
    }

    pub fn apply_pending_activation(&mut self) -> Result<(), WorkspaceError> {
        if let Some(id) = self.pending_activation.take() {
            self.entry(id)?;
            self.active = id;
        }
        Ok(())
    }

    pub fn close_clean(&mut self, id: SessionId) -> Result<(), WorkspaceError> {
        let map = self.session(id)?;
        if map.editor.is_dirty()
            || map
                .editor
                .has_drafts()
                .map_err(|error| WorkspaceError(error.to_string()))?
        {
            return Err(WorkspaceError(
                "Cannot close a tab with unsaved work without a decision.".into(),
            ));
        }
        if self.order.len() == 1 {
            self.new_map()?;
            self.apply_pending_activation()?;
        } else if self.active == id {
            let index = self
                .order
                .iter()
                .position(|candidate| *candidate == id)
                .expect("tab exists");
            self.active = self.order[if index == 0 { 1 } else { index - 1 }];
        }
        self.order.retain(|candidate| *candidate != id);
        self.entries.remove(&id);
        if self.pending_activation == Some(id) {
            self.pending_activation = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{EditorAction, EditorEdit},
        document::Command,
    };

    #[test]
    fn blocked_undo_retains_its_entry() {
        let mut workspace = Workspace::new();
        let id = workspace.active_id();
        let root = workspace.map().editor.selected.clone();
        workspace
            .apply(
                id,
                EditorEdit::Command(Command::SetText {
                    id: root.clone(),
                    text: "x".repeat(4096),
                }),
            )
            .unwrap();
        workspace
            .apply(
                id,
                EditorEdit::Command(Command::SetText {
                    id: root,
                    text: "small".into(),
                }),
            )
            .unwrap();
        workspace.storage_limit = workspace.estimated_bytes();
        assert!(
            workspace
                .apply(id, EditorEdit::Action(EditorAction::Undo))
                .is_err()
        );
        assert_eq!(
            workspace.map().editor.document.nodes[&workspace.map().editor.document.root].text,
            "small"
        );
        workspace.storage_limit += 4096;
        workspace
            .apply(id, EditorEdit::Action(EditorAction::Undo))
            .unwrap();
        assert_eq!(
            workspace.map().editor.document.nodes[&workspace.map().editor.document.root]
                .text
                .len(),
            4096
        );
    }

    #[test]
    fn history_eviction_is_global_oldest_first() {
        let mut workspace = Workspace::new();
        let first = workspace.active_id();
        let root = workspace.map().editor.selected.clone();
        let retained = workspace.map().editor.document.estimated_bytes();
        workspace
            .apply(
                first,
                EditorEdit::Command(Command::SetText {
                    id: root.clone(),
                    text: "first".into(),
                }),
            )
            .unwrap();
        let second = workspace.new_map().unwrap();
        workspace.apply_pending_activation().unwrap();
        workspace.history_limit = retained;
        let root2 = workspace.map().editor.selected.clone();
        workspace
            .apply(
                second,
                EditorEdit::Command(Command::SetText {
                    id: root2,
                    text: "second".into(),
                }),
            )
            .unwrap();
        assert!(workspace.session(first).unwrap().editor.history.evicted);
        assert_eq!(
            workspace
                .session(first)
                .unwrap()
                .editor
                .history
                .retained_bytes(),
            0
        );
        assert!(
            workspace
                .session(second)
                .unwrap()
                .editor
                .history
                .retained_bytes()
                > 0
        );
        workspace
            .apply(first, EditorEdit::Action(EditorAction::Undo))
            .unwrap();
        assert_eq!(
            workspace.session(first).unwrap().editor.document.nodes[&root].text,
            "first"
        );
    }

    #[test]
    fn exact_storage_boundary_accepts_and_one_byte_growth_rejects() {
        let mut workspace = Workspace::new();
        assert_eq!(workspace.storage_limit, 256 * 1024 * 1024);
        assert_eq!(workspace.history_limit, 64 * 1024 * 1024);
        let id = workspace.active_id();
        let root = workspace.map().editor.selected.clone();
        workspace
            .apply(id, EditorEdit::Action(EditorAction::Edit))
            .unwrap();
        workspace.storage_limit = workspace.estimated_bytes() + 1;
        let previous = workspace
            .map()
            .editor
            .editing
            .as_ref()
            .unwrap()
            .text
            .clone();
        workspace
            .apply(
                id,
                EditorEdit::SetDraft {
                    field: crate::app::DraftField::Inline(root.clone()),
                    text: format!("{previous}x"),
                },
            )
            .unwrap();
        assert!(
            workspace
                .apply(
                    id,
                    EditorEdit::SetDraft {
                        field: crate::app::DraftField::Inline(root),
                        text: format!("{previous}xx")
                    }
                )
                .is_err()
        );
        assert_eq!(
            workspace.map().editor.editing.as_ref().unwrap().text,
            format!("{previous}x")
        );
    }
}

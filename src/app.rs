use crate::{
    document::{
        Command, Document, DocumentError, History, MAX_FIELD_BYTES, MAX_TEXT_BYTES, NodeId, Side,
    },
    interchange::{FormatError, ImportReport, write_native},
    storage::{SaveCheckpoint, StorageError, atomic_write},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EditorError {
    #[error("{0}")]
    Document(#[from] DocumentError),
    #[error("{0}")]
    Format(#[from] FormatError),
    #[error("{0}")]
    Storage(#[from] StorageError),
    #[error("{0}")]
    State(String),
    #[error("{0}")]
    Workspace(#[from] crate::workspace::WorkspaceError),
}

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}
#[derive(Clone, Copy, Debug)]
pub enum EditorKey {
    Tab,
    Enter,
    Escape,
    Delete,
    Space,
}
#[derive(Clone, Copy, Debug)]
pub enum EditorAction {
    AddChild,
    AddSibling,
    Edit,
    Delete,
    Fold,
    Undo,
    Redo,
    MoveUp,
    MoveDown,
    Indent,
    Outdent,
    Navigate(Direction),
}
#[derive(Clone, Debug)]
pub enum PendingAction {
    Quit,
}

#[derive(Clone)]
pub struct EditBuffer {
    pub id: NodeId,
    pub text: String,
}

#[derive(Clone)]
pub struct DetailsDraft {
    pub id: NodeId,
    pub notes: String,
    pub link: String,
}

pub enum DraftField {
    Inline(NodeId),
    Notes(NodeId),
    Link(NodeId),
}

pub enum EditorEdit {
    Action(EditorAction),
    Key(EditorKey),
    Command(Command),
    Select(NodeId),
    ToggleSelection(NodeId),
    ActivateSelection(NodeId),
    BeginReplacement,
    BeginDetails(NodeId),
    FlushDetails,
    CommitDrafts,
    SetDraft { field: DraftField, text: String },
}

enum PreparedKind {
    Editor(Box<Editor>),
    Draft { field: DraftField, text: String },
}

pub struct PreparedEdit {
    kind: PreparedKind,
    estimated_bytes: usize,
    original_serial: u64,
    history_entries: u64,
}

pub struct PreparedSave {
    pub(crate) edit: PreparedEdit,
    pub(crate) bytes: Vec<u8>,
}

impl PreparedEdit {
    pub(crate) fn manifest_bound(&self, original: &Editor) -> usize {
        crate::recovery::editor_manifest_bound(match &self.kind {
            PreparedKind::Editor(editor) => editor,
            PreparedKind::Draft { .. } => original,
        })
    }

    pub fn estimated_bytes(&self) -> usize {
        self.estimated_bytes
    }
    pub fn history_entries(&self) -> u64 {
        self.history_entries
    }
}

pub struct Editor {
    pub document: Document,
    pub selected: NodeId,
    selection: BTreeSet<NodeId>,
    pub history: History,
    pub editing: Option<EditBuffer>,
    pub details: Option<DetailsDraft>,
    pub path: Option<PathBuf>,
    pub document_id: String,
    pub pending: Option<PendingAction>,
    pub staged_import: Option<ImportReport>,
    pub revision: u64,
    pub draft_revision: u64,
    checkpoint: SaveCheckpoint,
    dirty: bool,
    document_bytes: usize,
    text_bytes: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    pub fn new() -> Self {
        let document = Document::new("Central idea");
        let mut checkpoint = SaveCheckpoint::default();
        checkpoint.mark_saved(&document);
        Self {
            selected: document.root.clone(),
            selection: BTreeSet::from([document.root.clone()]),
            document_bytes: document.estimated_bytes(),
            text_bytes: document.decoded_text_bytes(),
            document,
            history: History::default(),
            editing: None,
            details: None,
            path: None,
            document_id: Uuid::new_v4().to_string(),
            pending: None,
            staged_import: None,
            revision: 0,
            draft_revision: 0,
            checkpoint,
            dirty: false,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn prepare_save(&self, path: &Path) -> Result<PreparedSave, EditorError> {
        let mut edit = self.prepare(&EditorEdit::CommitDrafts)?;
        let PreparedKind::Editor(candidate) = &mut edit.kind else {
            unreachable!("draft commit prepares a complete editor");
        };
        let bytes = write_native(&candidate.document)?;
        candidate.path = Some(path.into());
        candidate.checkpoint.mark_saved(&candidate.document);
        candidate.dirty = false;
        edit.estimated_bytes = candidate.estimated_bytes();
        Ok(PreparedSave { edit, bytes })
    }

    pub fn estimated_bytes(&self) -> usize {
        self.document_bytes
            + self.checkpoint.estimated_bytes()
            + std::mem::size_of::<Self>()
            + self.document_id.len()
            + self.selected.len()
            + self.selection.iter().map(|id| id.len() + 64).sum::<usize>()
            + self.path.as_ref().map_or(0, |path| path.as_os_str().len())
            + self.editing.as_ref().map_or(0, |draft| {
                draft.id.len() + draft.text.len() + std::mem::size_of::<EditBuffer>()
            })
            + self.details.as_ref().map_or(0, |draft| {
                draft.id.len()
                    + draft.notes.len()
                    + draft.link.len()
                    + std::mem::size_of::<DetailsDraft>()
            })
    }

    fn draft_value(&self, field: &DraftField) -> Result<&str, EditorError> {
        match field {
            DraftField::Inline(id) => self
                .editing
                .as_ref()
                .filter(|draft| &draft.id == id)
                .map(|draft| draft.text.as_str()),
            DraftField::Notes(id) => self
                .details
                .as_ref()
                .filter(|draft| &draft.id == id)
                .map(|draft| draft.notes.as_str()),
            DraftField::Link(id) => self
                .details
                .as_ref()
                .filter(|draft| &draft.id == id)
                .map(|draft| draft.link.as_str()),
        }
        .ok_or_else(|| {
            EditorError::State("The requested text draft is no longer owned by this field.".into())
        })
    }

    fn effective_text_bytes(&self) -> Result<usize, EditorError> {
        self.validate_draft_owners()?;
        let mut bytes = self.text_bytes;
        if let Some(draft) = &self.editing {
            bytes = bytes - self.document.nodes[&draft.id].text.len() + draft.text.len();
        }
        if let Some(draft) = &self.details {
            let node = &self.document.nodes[&draft.id];
            bytes =
                bytes - node.notes.len() - node.link.len() + draft.notes.len() + draft.link.len();
        }
        Ok(bytes)
    }

    pub fn prepare(&self, edit: &EditorEdit) -> Result<PreparedEdit, EditorError> {
        let original_serial = self.history.next_serial();
        if let EditorEdit::SetDraft { field, text } = edit {
            let previous = self.draft_value(field)?;
            if text.len() > MAX_FIELD_BYTES
                || self.effective_text_bytes()? - previous.len() + text.len() > MAX_TEXT_BYTES
            {
                return Err(EditorError::State("Text exceeds the document field or decoded-text limit; the previous draft was retained.".into()));
            }
            let field = match field {
                DraftField::Inline(id) => DraftField::Inline(id.clone()),
                DraftField::Notes(id) => DraftField::Notes(id.clone()),
                DraftField::Link(id) => DraftField::Link(id.clone()),
            };
            return Ok(PreparedEdit {
                estimated_bytes: self.estimated_bytes() - previous.len() + text.len(),
                kind: PreparedKind::Draft {
                    field,
                    text: text.clone(),
                },
                original_serial,
                history_entries: 0,
            });
        }
        let mut candidate = Self {
            document: self.document.clone(),
            selected: self.selected.clone(),
            selection: self.selection.clone(),
            history: self.history.clone(),
            editing: self.editing.clone(),
            details: self.details.clone(),
            path: self.path.clone(),
            document_id: self.document_id.clone(),
            pending: self.pending.clone(),
            staged_import: None,
            revision: self.revision,
            draft_revision: self.draft_revision,
            checkpoint: self.checkpoint.clone(),
            dirty: self.dirty,
            document_bytes: self.document_bytes,
            text_bytes: self.text_bytes,
        };
        match edit {
            EditorEdit::Action(action) => {
                candidate.flush_details()?;
                candidate.dispatch(*action)?;
            }
            EditorEdit::Key(key) => {
                candidate.handle_key(*key, false, false)?;
            }
            EditorEdit::Command(command) => {
                candidate.flush_details()?;
                candidate.commit_edit()?;
                candidate.execute(command.clone())?;
            }
            EditorEdit::Select(id) => {
                candidate.flush_details()?;
                candidate.select(id)?;
            }
            EditorEdit::ToggleSelection(id) => {
                candidate.flush_details()?;
                candidate.toggle_selection(id)?;
            }
            EditorEdit::ActivateSelection(id) => {
                candidate.flush_details()?;
                candidate.activate_selection(id)?;
            }
            EditorEdit::BeginReplacement => {
                candidate.flush_details()?;
                candidate.begin_replacement()?;
            }
            EditorEdit::BeginDetails(id) => {
                candidate.flush_details()?;
                let node = candidate
                    .document
                    .nodes
                    .get(id)
                    .ok_or_else(|| EditorError::State("Unknown notes owner".into()))?;
                candidate.details = Some(DetailsDraft {
                    id: id.clone(),
                    notes: node.notes.clone(),
                    link: node.link.clone(),
                });
                candidate.draft_revision = candidate.draft_revision.wrapping_add(1);
            }
            EditorEdit::FlushDetails => candidate.flush_details()?,
            EditorEdit::CommitDrafts => {
                candidate.flush_details()?;
                candidate.commit_edit()?;
            }
            EditorEdit::SetDraft { .. } => unreachable!("draft updates use narrow preparation"),
        }
        candidate.validate_draft_owners()?;
        Ok(PreparedEdit {
            estimated_bytes: candidate.estimated_bytes(),
            history_entries: candidate.history.next_serial() - original_serial,
            original_serial,
            kind: PreparedKind::Editor(Box::new(candidate)),
        })
    }

    pub fn commit_prepared(&mut self, edit: PreparedEdit, stamp: u64) {
        match edit.kind {
            PreparedKind::Editor(mut candidate) => {
                candidate.history.stamp_since(edit.original_serial, stamp);
                candidate.staged_import = self.staged_import.take();
                *self = *candidate;
            }
            PreparedKind::Draft { field, text } => {
                match field {
                    DraftField::Inline(_) => {
                        self.editing.as_mut().expect("admitted draft").text = text
                    }
                    DraftField::Notes(_) => {
                        self.details.as_mut().expect("admitted draft").notes = text
                    }
                    DraftField::Link(_) => {
                        self.details.as_mut().expect("admitted draft").link = text
                    }
                }
                self.draft_revision = self.draft_revision.wrapping_add(1);
            }
        }
    }

    fn validate_draft_owners(&self) -> Result<(), EditorError> {
        for id in self
            .editing
            .iter()
            .map(|draft| &draft.id)
            .chain(self.details.iter().map(|draft| &draft.id))
        {
            if !self.document.nodes.contains_key(id) {
                return Err(EditorError::State(format!(
                    "Draft owner no longer exists: {id}"
                )));
            }
        }
        Ok(())
    }

    pub fn has_drafts(&self) -> Result<bool, EditorError> {
        self.validate_draft_owners()?;
        Ok(self
            .editing
            .as_ref()
            .is_some_and(|draft| self.document.nodes[&draft.id].text != draft.text)
            || self.details.as_ref().is_some_and(|draft| {
                let node = &self.document.nodes[&draft.id];
                node.notes != draft.notes || node.link != draft.link
            }))
    }

    pub fn snapshot_with_drafts(&self) -> Result<Document, EditorError> {
        self.validate_draft_owners()?;
        let mut document = self.document.clone();
        if let Some(draft) = &self.editing {
            document
                .nodes
                .get_mut(&draft.id)
                .expect("validated owner")
                .text = draft.text.clone();
        }
        if let Some(draft) = &self.details {
            let node = document.nodes.get_mut(&draft.id).expect("validated owner");
            node.notes = draft.notes.clone();
            node.link = draft.link.clone();
        }
        document.validate()?;
        Ok(document)
    }

    pub fn flush_details(&mut self) -> Result<(), EditorError> {
        if let Some(draft) = self.details.clone() {
            self.execute(Command::SetDetails {
                id: draft.id,
                notes: draft.notes,
                link: draft.link,
            })?;
            self.details = None;
            self.draft_revision = self.draft_revision.wrapping_add(1);
        }
        Ok(())
    }

    pub fn search(&self, query: &str) -> Vec<NodeId> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.document
            .nodes
            .values()
            .filter(|node| {
                node.text.to_lowercase().contains(&query)
                    || node.notes.to_lowercase().contains(&query)
            })
            .map(|node| node.id.clone())
            .collect()
    }

    pub fn reveal_selection(&mut self, id: &str) -> Result<(), EditorError> {
        self.commit_edit()?;
        self.execute(Command::Reveal { id: id.into() })
    }

    fn changed(&mut self) {
        self.document_bytes = self.document.estimated_bytes();
        self.text_bytes = self.document.decoded_text_bytes();
        self.revision = self.revision.wrapping_add(1);
        self.dirty = self.checkpoint.is_dirty(&self.document);
        self.reconcile_selection();
    }

    fn reconcile_selection(&mut self) {
        self.selected = self
            .document
            .visible_ancestor(&self.selected)
            .unwrap_or_else(|| self.document.root.clone());
        let visible = self.document.visible_nodes();
        self.selection.retain(|id| visible.contains(id));
        self.selection.insert(self.selected.clone());
    }

    pub fn execute(&mut self, command: Command) -> Result<(), EditorError> {
        let generation = self.history.generation;
        if let Some(selected) = self.history.execute(&mut self.document, command)?
            && selected != self.selected
        {
            self.replace_selection(selected);
        }
        if self.history.generation != generation {
            self.changed();
        } else {
            self.reconcile_selection();
        }
        Ok(())
    }

    pub fn commit_edit(&mut self) -> Result<(), EditorError> {
        if let Some(buffer) = self.editing.take() {
            let command = Command::SetText {
                id: buffer.id.clone(),
                text: buffer.text.clone(),
            };
            if let Err(error) = self.execute(command) {
                self.editing = Some(buffer);
                return Err(error);
            }
            self.draft_revision = self.draft_revision.wrapping_add(1);
        }
        Ok(())
    }

    pub fn select(&mut self, id: &str) -> Result<(), EditorError> {
        self.commit_edit()?;
        if !self.document.nodes.contains_key(id) {
            return Err(EditorError::State("Unknown selection".into()));
        }
        let selected = self
            .document
            .visible_ancestor(id)
            .expect("validated selection");
        self.replace_selection(selected);
        Ok(())
    }

    fn replace_selection(&mut self, id: NodeId) {
        self.selection = BTreeSet::from([id.clone()]);
        self.selected = id;
    }

    pub fn selection(&self) -> &BTreeSet<NodeId> {
        &self.selection
    }

    pub fn is_selected(&self, id: &str) -> bool {
        self.selection.contains(id)
    }

    pub fn activate_selection(&mut self, id: &str) -> Result<(), EditorError> {
        if self.selection.contains(id) {
            self.commit_edit()?;
            self.selected = id.into();
            Ok(())
        } else {
            self.select(id)
        }
    }

    pub fn toggle_selection(&mut self, id: &str) -> Result<(), EditorError> {
        self.commit_edit()?;
        let id = self
            .document
            .visible_ancestor(id)
            .ok_or_else(|| EditorError::State("Unknown selection".into()))?;
        if self.selection.contains(&id) {
            if self.selection.len() > 1 {
                self.selection.remove(&id);
                if self.selected == id {
                    self.selected = self
                        .selection
                        .iter()
                        .next()
                        .expect("nonempty selection")
                        .clone();
                }
            }
        } else {
            self.selection.insert(id.clone());
            self.selected = id;
        }
        Ok(())
    }

    pub fn dispatch(&mut self, action: EditorAction) -> Result<(), EditorError> {
        self.commit_edit()?;
        let id = self.selected.clone();
        let command = match action {
            EditorAction::AddChild => Some(Command::AddChild {
                parent: id,
                text: "New idea".into(),
            }),
            EditorAction::AddSibling => {
                if id == self.document.root {
                    Some(Command::AddChild {
                        parent: id,
                        text: "New idea".into(),
                    })
                } else {
                    Some(Command::AddSibling {
                        sibling: id,
                        text: "New idea".into(),
                    })
                }
            }
            EditorAction::Edit => {
                self.begin_edit();
                None
            }
            EditorAction::Delete => Some(Command::DeleteMany {
                ids: self.selection.iter().cloned().collect(),
            }),
            EditorAction::Fold => Some(Command::ToggleFold { id }),
            EditorAction::MoveUp => Some(Command::MoveSibling { id, delta: -1 }),
            EditorAction::MoveDown => Some(Command::MoveSibling { id, delta: 1 }),
            EditorAction::Indent => Some(Command::Indent { id }),
            EditorAction::Outdent => Some(Command::Outdent { id }),
            EditorAction::Undo => {
                if self.history.undo(&mut self.document) {
                    self.changed();
                }
                None
            }
            EditorAction::Redo => {
                if self.history.redo(&mut self.document) {
                    self.changed();
                }
                None
            }
            EditorAction::Navigate(direction) => {
                self.navigate(direction);
                None
            }
        };
        if let Some(command) = command {
            self.execute(command)?;
        }
        if matches!(action, EditorAction::AddChild | EditorAction::AddSibling) {
            self.begin_edit();
        }
        Ok(())
    }

    fn begin_edit(&mut self) {
        self.start_edit(self.document.nodes[&self.selected].text.clone());
    }

    pub fn begin_replacement(&mut self) -> Result<(), EditorError> {
        self.commit_edit()?;
        self.replace_selection(self.selected.clone());
        self.start_edit(String::new());
        Ok(())
    }

    fn start_edit(&mut self, text: String) {
        self.draft_revision = self.draft_revision.wrapping_add(1);
        self.editing = Some(EditBuffer {
            id: self.selected.clone(),
            text,
        });
    }

    pub fn handle_key(
        &mut self,
        key: EditorKey,
        auxiliary_focus: bool,
        composing: bool,
    ) -> Result<bool, EditorError> {
        if auxiliary_focus || composing {
            return Ok(false);
        }
        if self.editing.is_some() {
            match key {
                EditorKey::Escape => {
                    self.editing = None;
                    self.draft_revision = self.draft_revision.wrapping_add(1);
                }
                EditorKey::Enter => self.commit_edit()?,
                EditorKey::Tab => {
                    self.commit_edit()?;
                    self.dispatch(EditorAction::AddChild)?;
                }
                _ => return Ok(false),
            }
        } else {
            match key {
                EditorKey::Enter => self.dispatch(EditorAction::AddSibling)?,
                EditorKey::Tab => self.dispatch(EditorAction::AddChild)?,
                EditorKey::Delete => self.dispatch(EditorAction::Delete)?,
                EditorKey::Space => self.dispatch(EditorAction::Fold)?,
                EditorKey::Escape => return Ok(false),
            }
        }
        Ok(true)
    }

    fn navigate(&mut self, direction: Direction) {
        let node = &self.document.nodes[&self.selected];
        let parent = self.document.parent(&self.selected);
        let target = match direction {
            Direction::Left | Direction::Right => {
                let side = if matches!(direction, Direction::Left) {
                    Side::Left
                } else {
                    Side::Right
                };
                if node.id == self.document.root {
                    if node.folded {
                        None
                    } else {
                        node.children
                            .iter()
                            .find(|id| self.document.nodes[*id].side == side)
                            .cloned()
                    }
                } else if node.side != side {
                    parent.map(str::to_owned)
                } else if !node.folded {
                    node.children.first().cloned()
                } else {
                    None
                }
            }
            Direction::Up | Direction::Down => parent.and_then(|parent| {
                let siblings: Vec<_> = self.document.nodes[parent]
                    .children
                    .iter()
                    .filter(|id| {
                        parent != self.document.root || self.document.nodes[*id].side == node.side
                    })
                    .collect();
                let index = siblings.iter().position(|id| **id == self.selected)?;
                let next = if matches!(direction, Direction::Up) {
                    index.checked_sub(1)
                } else {
                    index.checked_add(1)
                }?;
                siblings.get(next).map(|id| (*id).clone())
            }),
        };
        if let Some(target) = target {
            self.replace_selection(target);
        }
    }

    pub fn request(&mut self, action: PendingAction) -> Result<bool, EditorError> {
        self.commit_edit()?;
        self.pending = Some(action);
        Ok(!self.dirty)
    }

    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }
    pub fn stage_import(&mut self, report: ImportReport) {
        self.staged_import = Some(report);
    }
    pub fn cancel_import(&mut self) {
        self.staged_import = None;
        self.pending = None;
    }

    pub fn accept_import(&mut self) -> Result<(), EditorError> {
        let report = self
            .staged_import
            .take()
            .ok_or_else(|| EditorError::State("No import is staged".into()))?;
        self.replace_document(report.document, None, false, None)
    }

    pub fn replace_document(
        &mut self,
        document: Document,
        path: Option<PathBuf>,
        clean: bool,
        document_id: Option<String>,
    ) -> Result<(), EditorError> {
        document.validate()?;
        self.document = document;
        self.replace_selection(self.document.root.clone());
        self.history = History::default();
        self.editing = None;
        self.details = None;
        self.path = path;
        self.pending = None;
        self.staged_import = None;
        self.document_id = document_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        self.checkpoint = SaveCheckpoint::default();
        if clean {
            self.checkpoint.mark_saved(&self.document);
        }
        self.changed();
        Ok(())
    }

    pub fn save_to(&mut self, path: &Path) -> Result<(), EditorError> {
        self.commit_edit()?;
        let bytes = write_native(&self.document)?;
        atomic_write(path, &bytes)?;
        self.path = Some(path.into());
        self.checkpoint.mark_saved(&self.document);
        self.dirty = false;
        Ok(())
    }
}

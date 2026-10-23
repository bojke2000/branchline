use crate::{
    document::{Command, Document, DocumentError, History, NodeId, Side},
    interchange::{FormatError, ImportReport, write_native},
    storage::{SaveCheckpoint, StorageError, atomic_write},
};
use std::path::{Path, PathBuf};
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
    New,
    Open(PathBuf),
    Quit,
}

pub struct EditBuffer {
    pub id: NodeId,
    pub text: String,
}

pub struct Editor {
    pub document: Document,
    pub selected: NodeId,
    pub history: History,
    pub editing: Option<EditBuffer>,
    pub path: Option<PathBuf>,
    pub document_id: String,
    pub pending: Option<PendingAction>,
    pub staged_import: Option<ImportReport>,
    pub revision: u64,
    pub draft_revision: u64,
    checkpoint: SaveCheckpoint,
    dirty: bool,
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
            document,
            history: History::default(),
            editing: None,
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
        self.revision = self.revision.wrapping_add(1);
        self.dirty = self.checkpoint.is_dirty(&self.document);
        self.reconcile_selection();
    }

    fn reconcile_selection(&mut self) {
        self.selected = self
            .document
            .visible_ancestor(&self.selected)
            .unwrap_or_else(|| self.document.root.clone());
    }

    pub fn execute(&mut self, command: Command) -> Result<(), EditorError> {
        let generation = self.history.generation;
        if let Some(selected) = self.history.execute(&mut self.document, command)? {
            self.selected = selected;
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
        self.selected = self
            .document
            .visible_ancestor(id)
            .expect("validated selection");
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
            EditorAction::Delete => Some(Command::Delete { id }),
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
        self.draft_revision = self.draft_revision.wrapping_add(1);
        self.editing = Some(EditBuffer {
            id: self.selected.clone(),
            text: self.document.nodes[&self.selected].text.clone(),
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
            self.selected = target;
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
        self.selected = self.document.root.clone();
        self.history = History::default();
        self.editing = None;
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

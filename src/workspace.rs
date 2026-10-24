use crate::session::{MapSession, SessionId, SessionState};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct WorkspaceError(pub String);

pub struct Workspace {
    order: Vec<SessionId>,
    entries: BTreeMap<SessionId, SessionState>,
    active: SessionId,
    pending_activation: Option<SessionId>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn new() -> Self {
        let id = SessionId::new();
        Self {
            order: vec![id],
            entries: BTreeMap::from([(id, SessionState::Map(Box::new(MapSession::new_blank(id))))]),
            active: id,
            pending_activation: None,
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

    pub fn new_map(&mut self) -> Result<SessionId, WorkspaceError> {
        let id = SessionId::new();
        self.entries
            .insert(id, SessionState::Map(Box::new(MapSession::new_blank(id))));
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

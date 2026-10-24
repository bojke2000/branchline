use super::*;
use crate::recovery::{RecoveryCoordinator, RecoverySession, WorkspaceRecoveryState};

#[derive(Clone, Copy)]
pub enum CloseDecision {
    Save,
    Discard,
    Cancel,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TransitionProgress {
    AwaitingDecision(SessionId),
    AwaitingSave(SessionId),
    ReadyToPublish,
    Finished {
        quit: bool,
        closed: Vec<SessionId>,
        cleanup_error: Option<String>,
    },
}

#[derive(Clone, Copy)]
enum Scope {
    Close(SessionId),
    Quit,
}

pub(super) struct Transition {
    scope: Scope,
    targets: Vec<SessionId>,
    cursor: usize,
    discarded: BTreeSet<SessionId>,
    saving: Option<SessionId>,
    replacement: Option<Box<MapSession>>,
}

use std::collections::BTreeSet;

impl Workspace {
    pub(super) fn require_save_target(&self, id: SessionId) -> Result<(), WorkspaceError> {
        if let Some(transition) = &self.transition
            && transition.saving != Some(id)
        {
            return Err(WorkspaceError(
                "This map is not the captured save target of the pending operation.".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn require_idle(&self) -> Result<(), WorkspaceError> {
        if self.transition.is_some() {
            Err(WorkspaceError(
                "Finish or cancel the pending close/quit operation first.".into(),
            ))
        } else {
            Ok(())
        }
    }

    pub fn transition_pending(&self) -> bool {
        self.transition.is_some()
    }

    fn needs_decision(&self, id: SessionId) -> Result<bool, WorkspaceError> {
        match self.entry(id)? {
            SessionState::Map(map) => Ok(map.editor.is_dirty()
                || map
                    .editor
                    .has_drafts()
                    .map_err(|error| WorkspaceError(error.to_string()))?),
            SessionState::RestoreError(failure) => Ok(failure.retain_recovery),
        }
    }

    fn begin_transition(&mut self, scope: Scope) -> Result<TransitionProgress, WorkspaceError> {
        self.require_idle()?;
        let targets = match scope {
            Scope::Close(id) => {
                self.entry(id)?;
                vec![id]
            }
            Scope::Quit => self.order.clone(),
        };
        for id in &targets {
            self.needs_decision(*id)?;
        }
        let replacement = matches!(scope, Scope::Close(_))
            .then(|| (self.len() == 1).then(|| Box::new(MapSession::new_blank(SessionId::new()))))
            .flatten();
        self.pending_activation = None;
        self.transition = Some(Transition {
            scope,
            targets,
            cursor: 0,
            discarded: BTreeSet::new(),
            saving: None,
            replacement,
        });
        self.advance_transition()
    }

    pub fn request_close(&mut self, id: SessionId) -> Result<TransitionProgress, WorkspaceError> {
        self.begin_transition(Scope::Close(id))
    }

    pub fn request_quit(&mut self) -> Result<TransitionProgress, WorkspaceError> {
        self.begin_transition(Scope::Quit)
    }

    fn advance_transition(&mut self) -> Result<TransitionProgress, WorkspaceError> {
        loop {
            let transition = self
                .transition
                .as_ref()
                .ok_or_else(|| WorkspaceError("No close/quit operation is pending.".into()))?;
            if let Some(id) = transition.saving {
                return Ok(TransitionProgress::AwaitingSave(id));
            }
            let Some(id) = transition.targets.get(transition.cursor).copied() else {
                return Ok(TransitionProgress::ReadyToPublish);
            };
            if self.needs_decision(id)? {
                return Ok(TransitionProgress::AwaitingDecision(id));
            }
            self.transition.as_mut().expect("checked transition").cursor += 1;
        }
    }

    pub fn resolve(
        &mut self,
        decision: CloseDecision,
    ) -> Result<TransitionProgress, WorkspaceError> {
        if self.transition.is_none() {
            return Err(WorkspaceError("No close/quit operation is pending.".into()));
        }
        if matches!(decision, CloseDecision::Cancel) {
            self.transition = None;
            return Ok(TransitionProgress::Finished {
                quit: false,
                closed: vec![],
                cleanup_error: None,
            });
        }
        let progress = self.advance_transition()?;
        let TransitionProgress::AwaitingDecision(id) = progress else {
            return Err(WorkspaceError(
                "The pending operation is not awaiting a decision.".into(),
            ));
        };
        if matches!(decision, CloseDecision::Save) {
            self.session(id)?;
            self.transition.as_mut().expect("checked transition").saving = Some(id);
            return Ok(TransitionProgress::AwaitingSave(id));
        }
        let transition = self.transition.as_mut().expect("checked transition");
        transition.discarded.insert(id);
        transition.cursor += 1;
        self.advance_transition()
    }

    pub fn resume_after_save(
        &mut self,
        id: SessionId,
    ) -> Result<TransitionProgress, WorkspaceError> {
        if self
            .transition
            .as_ref()
            .and_then(|transition| transition.saving)
            != Some(id)
        {
            return Err(WorkspaceError(
                "This save does not belong to the pending close/quit operation.".into(),
            ));
        }
        let map = self.session(id)?;
        if map.editor.path.is_none()
            || map.editor.is_dirty()
            || map
                .editor
                .has_drafts()
                .map_err(|error| WorkspaceError(error.to_string()))?
        {
            return Err(WorkspaceError(
                "The requested map still has unsaved work.".into(),
            ));
        }
        let transition = self.transition.as_mut().expect("checked transition");
        transition.saving = None;
        transition.cursor += 1;
        self.advance_transition()
    }

    fn transition_recovery_state(&self) -> Result<WorkspaceRecoveryState<'_>, WorkspaceError> {
        let transition = self
            .transition
            .as_ref()
            .ok_or_else(|| WorkspaceError("No close/quit operation is pending.".into()))?;
        if transition.saving.is_some() || transition.cursor != transition.targets.len() {
            return Err(WorkspaceError(
                "Not every map has approved the pending operation.".into(),
            ));
        }
        let mut sessions = Vec::new();
        for id in &self.order {
            if matches!(transition.scope, Scope::Close(target) if target == *id) {
                continue;
            }
            let discard = transition.discarded.contains(id);
            match self.entry(*id)? {
                SessionState::Map(map) => {
                    if matches!(transition.scope, Scope::Quit) && map.editor.path.is_none() {
                        continue;
                    }
                    sessions.push(if discard {
                        RecoverySession::SavedMap(map)
                    } else {
                        RecoverySession::Map(map)
                    });
                }
                SessionState::RestoreError(failure) => {
                    if matches!(transition.scope, Scope::Quit) && failure.path.is_none() && discard
                    {
                        continue;
                    }
                    sessions.push(RecoverySession::Failed {
                        id: *id,
                        path: &failure.path,
                        reason: &failure.reason,
                        retain_snapshot: failure.retain_recovery && !discard,
                    });
                }
            }
        }
        if let Some(replacement) = &transition.replacement {
            sessions.push(RecoverySession::Map(replacement));
        }
        let ids: Vec<_> = sessions.iter().map(|session| session.id()).collect();
        let active = if ids.contains(&self.active) {
            Some(self.active)
        } else {
            let index = self
                .order
                .iter()
                .position(|id| *id == self.active)
                .expect("active tab exists");
            self.order[..index]
                .iter()
                .rev()
                .copied()
                .find(|id| ids.contains(id))
                .or_else(|| ids.first().copied())
        };
        Ok(WorkspaceRecoveryState { active, sessions })
    }

    pub fn publish_transition(
        &mut self,
        recovery: &mut RecoveryCoordinator,
    ) -> Result<TransitionProgress, WorkspaceError> {
        let state = self.transition_recovery_state()?;
        let next_active = state.active;
        let publication = match recovery.publish(&state) {
            Ok(publication) => publication,
            Err(error) => {
                self.transition = None;
                return Err(WorkspaceError(error.to_string()));
            }
        };
        let transition = self.transition.take().expect("published transition");
        let quit = matches!(transition.scope, Scope::Quit);
        let mut closed = Vec::new();
        if let Scope::Close(id) = transition.scope {
            if let Some(replacement) = transition.replacement {
                self.order.push(replacement.id());
                self.entries
                    .insert(replacement.id(), SessionState::Map(replacement));
            }
            self.entries.remove(&id);
            self.order.retain(|candidate| *candidate != id);
            self.active = next_active.expect("closing the last map has an admitted replacement");
            self.pending_activation = None;
            closed.push(id);
        }
        Ok(TransitionProgress::Finished {
            quit,
            closed,
            cleanup_error: publication.cleanup_error.map(|error| error.to_string()),
        })
    }
}

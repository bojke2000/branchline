use super::*;

impl NativeApp {
    pub(super) fn queue_quit(&mut self) {
        self.cancel_canvas_drag();
        self.pending_quit = true;
        self.pending_close = None;
    }

    pub(super) fn abort_transition(&mut self) {
        if self.workspace.transition_pending()
            && let Err(cause) = self.workspace.resolve(CloseDecision::Cancel)
        {
            self.error = Some(cause.to_string());
        }
        self.pending_quit = false;
        self.pending_close = None;
        self.edit_focus = None;
    }

    pub(super) fn drive_transition(
        &mut self,
        progress: Result<TransitionProgress, EditorError>,
        ctx: &egui::Context,
    ) {
        match progress {
            Ok(TransitionProgress::AwaitingDecision(id)) => {
                self.dialog = Some(Dialog::CloseDecision(id))
            }
            Ok(TransitionProgress::AwaitingSave(id)) => self.save_session(id, false, ctx),
            Ok(TransitionProgress::ReadyToPublish) => {
                let finished = self
                    .workspace
                    .publish_transition(&mut self.store)
                    .map_err(EditorError::from);
                self.drive_transition(finished, ctx);
            }
            Ok(TransitionProgress::Finished {
                quit,
                closed,
                cleanup_error,
            }) => {
                if let Some(cause) = cleanup_error {
                    self.error = Some(format!(
                        "Workspace decisions were published; cleanup remains pending: {cause}"
                    ));
                }
                for id in &closed {
                    if let Some(widgets) = self.owned_widgets.remove(id) {
                        ctx.data_mut(|data| {
                            for widget in widgets {
                                data.remove::<egui::text_edit::TextEditState>(widget);
                            }
                        });
                    }
                }
                self.edit_focus = None;
                if !closed.is_empty() {
                    self.reset_tab_renderer(ctx);
                }
                if quit {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Err(cause) => {
                self.abort_transition();
                self.error = Some(cause.to_string());
            }
        }
    }
}

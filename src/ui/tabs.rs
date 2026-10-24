use super::*;

impl NativeApp {
    pub(super) fn tab_strip(&mut self, root: &mut egui::Ui, blocked: bool) {
        let mut activate = None;
        let mut close = None;
        let mut create = false;
        egui::Panel::top("map-tabs").show(root, |ui| {
            ui.add_enabled_ui(!blocked, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (index, id) in self.workspace.order().iter().copied().enumerate() {
                            let (name, dirty) = match self.workspace.session(id) {
                                Ok(map) => {
                                    let name = map
                                        .editor
                                        .path
                                        .as_ref()
                                        .and_then(|path| path.file_name())
                                        .map(|name| name.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| format!("Untitled {}", index + 1));
                                    let dirty = map.editor.is_dirty()
                                        || map.editor.has_drafts().unwrap_or(true);
                                    (name, dirty)
                                }
                                Err(error) => (error.to_string(), true),
                            };
                            ui.push_id(id, |ui| {
                                if ui
                                    .selectable_label(
                                        self.workspace.active_id() == id,
                                        format!("{name}{}", if dirty { " *" } else { "" }),
                                    )
                                    .clicked()
                                {
                                    activate = Some(id);
                                }
                                if ui
                                    .small_button("x")
                                    .on_hover_text("Close map tab")
                                    .clicked()
                                {
                                    close = Some(id);
                                }
                            });
                            ui.separator();
                        }
                        create = ui.button("+").on_hover_text("New mind map").clicked();
                    });
                });
                if (self.workspace.pending_activation().is_some()
                    || self.pending_close.is_some()
                    || self.pending_quit)
                    && self.composing.is_some()
                {
                    ui.horizontal(|ui| {
                        ui.label(
                            "Finish the current text composition before changing tabs or closing.",
                        );
                        if ui.small_button("Cancel pending action").clicked() {
                            self.workspace.cancel_activation();
                            self.pending_close = None;
                            self.pending_quit = false;
                        }
                    });
                }
            });
        });
        if let Some(id) = activate {
            let result = self
                .workspace
                .request_activate(id)
                .map_err(EditorError::from);
            self.report(result);
        }
        if create {
            let result = self
                .workspace
                .new_map()
                .map(|_| ())
                .map_err(EditorError::from);
            self.report(result);
        }
        if let Some(id) = close {
            self.close_tab(id, root.ctx());
        }
    }

    pub(super) fn finish_tab_transition(&mut self, ctx: &egui::Context) {
        if let Some(owner) = self.composing {
            if self.workspace.pending_activation().is_some()
                || self.pending_close.is_some()
                || self.pending_quit
            {
                ctx.memory_mut(|memory| memory.request_focus(owner));
            }
            return;
        }
        if self.dialog.is_some() {
            return;
        }
        let original = self.workspace.active_id();
        let progress = if self.pending_quit {
            self.pending_quit = false;
            Some(self.workspace.request_quit())
        } else {
            self.pending_close
                .take()
                .map(|id| self.workspace.request_close(id))
        };
        if let Some(progress) = progress {
            self.drive_transition(progress.map_err(EditorError::from), ctx);
            if self.dialog.is_some() {
                self.dialogs(ctx);
                return;
            }
            if self.allow_close {
                return;
            }
        }
        if self.workspace.pending_activation().is_none() && original == self.workspace.active_id() {
            return;
        }
        let result = self
            .workspace
            .apply_pending_activation()
            .map_err(EditorError::from);
        if result.is_ok() {
            self.reset_tab_renderer(ctx);
        }
        self.report(result);
    }

    pub(super) fn close_tab(&mut self, id: SessionId, _ctx: &egui::Context) {
        if let Err(cause) = self.workspace.entry(id) {
            self.error = Some(cause.to_string());
            return;
        }
        self.cancel_canvas_drag();
        if !self.pending_quit {
            self.pending_close = Some(id);
        }
    }

    pub(super) fn reset_tab_renderer(&mut self, ctx: &egui::Context) {
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        self.drag = None;
        self.palette_open = false;
        self.edit_focus = None;
        self.layout_revision = None;
        self.layout_session = None;
        self.galleys.clear();
        self.branch_colors.clear();
        self.placed = Layout::default();
        self.input_after_switch = true;
    }

    pub(super) fn cancel_canvas_drag(&mut self) {
        if let Some(CanvasDrag::Pan { initial_pan, .. }) = self.drag.take() {
            match self.workspace.session_mut(self.workspace.active_id()) {
                Ok(map) => map.viewport.pan = initial_pan,
                Err(cause) => self.error = Some(format!("The pan owner is unavailable: {cause}")),
            }
        }
    }
}

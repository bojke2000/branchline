use super::*;

impl NativeApp {
    pub(super) fn tab_strip(&mut self, root: &mut egui::Ui, blocked: bool) {
        let mut activate = None;
        let mut close = None;
        let mut rename = None;
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
                                let response = ui.selectable_label(
                                    self.workspace.active_id() == id,
                                    format!("{name}{}", if dirty { " *" } else { "" }),
                                );
                                if response.clicked() {
                                    activate = Some(id);
                                }
                                response.context_menu(|ui| {
                                    if ui
                                        .add_enabled(
                                            self.workspace.session(id).is_ok(),
                                            egui::Button::new("Rename..."),
                                        )
                                        .clicked()
                                    {
                                        rename = Some(id);
                                        ui.close();
                                    }
                                });
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
        if let Some(id) = rename {
            self.begin_rename(id, root.ctx());
        }
    }

    pub(super) fn begin_rename(&mut self, session: SessionId, ctx: &egui::Context) {
        let path = match self.workspace.session(session) {
            Ok(map) => map.editor.path.clone(),
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let Some(path) = path else {
            self.save_session(session, true, ctx);
            return;
        };
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
            self.error = Some("This filename cannot be edited as UTF-8 text. Rename it in the file manager instead.".into());
            return;
        };
        self.cancel_canvas_drag();
        self.dialog = Some(Dialog::Rename {
            session,
            name: name.into(),
            focus: true,
        });
    }

    pub(super) fn rename_session(&mut self, session: SessionId, name: &str) -> bool {
        let name = name.trim();
        let filename = if name.ends_with(".branchline") {
            name.to_owned()
        } else {
            format!("{name}.branchline")
        };
        if name.is_empty() {
            self.error = Some("Enter a filename.".into());
            return false;
        }
        if let Err(error) = self.workspace.rename_file(session, &filename) {
            self.error = Some(error.to_string());
            return false;
        }
        self.error = None;
        self.status = format!("Renamed to {filename}");
        if let Err(error) = self.checkpoint_recovery() {
            self.error = Some(format!(
                "The file was renamed, but recovery could not be updated: {error}"
            ));
        }
        true
    }

    pub(super) fn rename_dialog(
        &mut self,
        ui: &mut egui::Ui,
        session: SessionId,
        name: &mut String,
        focus: &mut bool,
    ) -> bool {
        ui.heading("Rename mind map file");
        ui.label("Renames the file in its current folder. Unsaved edits stay unsaved.");
        let response = ui
            .horizontal(|ui| {
                let response = ui.text_edit_singleline(name);
                ui.label(".branchline");
                response
            })
            .inner;
        if *focus {
            response.request_focus();
            *focus = false;
        }
        let enter = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        let mut keep = true;
        ui.horizontal(|ui| {
            if (ui.button("Rename").clicked() || enter) && self.rename_session(session, name) {
                keep = false;
            }
            if ui.button("Cancel").clicked() || ui.input(|input| input.key_pressed(Key::Escape)) {
                keep = false;
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::from_rgb(210, 75, 70), error);
        }
        keep
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
        self.clipboard_gesture = None;
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

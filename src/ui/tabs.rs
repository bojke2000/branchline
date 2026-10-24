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
                if self.workspace.pending_activation().is_some() && self.composing.is_some() {
                    ui.horizontal(|ui| {
                        ui.label("Finish the current text composition to switch maps.");
                        if ui.small_button("Cancel switch").clicked() {
                            self.workspace.cancel_activation();
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
            if self.workspace.pending_activation().is_some() || self.pending_close.is_some() {
                ctx.memory_mut(|memory| memory.request_focus(owner));
            }
            return;
        }
        if self.dialog.is_some() {
            return;
        }
        let original = self.workspace.active_id();
        if let Some(id) = self.pending_close.take() {
            let result = self.workspace.close_clean(id).map_err(EditorError::from);
            if result.is_ok()
                && let Some(widgets) = self.owned_widgets.remove(&id)
            {
                ctx.data_mut(|data| {
                    for widget in widgets {
                        data.remove::<egui::text_edit::TextEditState>(widget);
                    }
                });
            }
            self.report(result);
        }
        if self.workspace.pending_activation().is_none() && original == self.workspace.active_id() {
            return;
        }
        let result = self
            .workspace
            .apply_pending_activation()
            .map_err(EditorError::from);
        if result.is_ok() {
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
        self.report(result);
    }

    pub(super) fn close_tab(&mut self, id: SessionId, _ctx: &egui::Context) {
        let result = self
            .workspace
            .session(id)
            .map_err(EditorError::from)
            .and_then(|map| {
                if map.editor.is_dirty() || map.editor.has_drafts()? {
                    Ok(false)
                } else {
                    Ok(true)
                }
            });
        match result {
            Ok(true) => {
                self.pending_close = Some(id);
            }
            Ok(false) => self.dialog = Some(Dialog::CloseTab(id)),
            Err(error) => self.report(Err(error)),
        }
    }
}

use super::*;

impl NativeApp {
    pub(super) fn debounce_recovery(&mut self, ctx: &egui::Context) {
        if !self.allow_close
            && self.dialog.is_none()
            && self
                .store
                .checkpoint_due(&self.workspace.recovery_state(), Instant::now())
        {
            match self.checkpoint_recovery() {
                Ok(()) => self.status = "Workspace recovery updated locally.".into(),
                Err(cause) => self.error = Some(format!("Recovery unavailable: {cause}")),
            }
        }
        ctx.request_repaint_after(Duration::from_millis(700));
    }

    pub(super) fn failed_restore_view(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.dialog.is_none() {
                self.queue_quit();
            }
        }
        if self.dialog.is_none() {
            let command = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => commands::shortcut(*key, *modifiers)
                        .filter(|command| {
                            matches!(
                                command,
                                AppCommand::New
                                    | AppCommand::Open
                                    | AppCommand::NextTab
                                    | AppCommand::PreviousTab
                                    | AppCommand::CloseTab
                                    | AppCommand::Quit
                            )
                        })
                        .map(|command| (*key, *modifiers, command)),
                    _ => None,
                })
            });
            if let Some((key, modifiers, command)) = command {
                ctx.input_mut(|input| input.consume_key(modifiers, key));
                self.run_command(command, &ctx);
            }
        }
        self.tab_strip(root, self.dialog.is_some());
        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("This mind map could not be restored");
            if let Err(cause) = self.workspace.active() {
                ui.label(cause.to_string());
            }
            ui.label("The saved file and recovery resources have not been replaced with an empty map. Other tabs remain available.");
            ui.label("Retry reads the same source. Locate opens a chosen native file in this failed tab; old recovery is retired only after publication succeeds.");
            ui.horizontal(|ui| {
                let id = self.workspace.active_id();
                if ui.add_enabled(self.dialog.is_none(), egui::Button::new("Retry")).clicked() { self.retry_failed_tab(id, false, &ctx); }
                if ui.add_enabled(self.dialog.is_none(), egui::Button::new("Locate")).clicked() { self.retry_failed_tab(id, true, &ctx); }
                if ui.add_enabled(self.dialog.is_none(), egui::Button::new("Close tab")).clicked() { self.close_tab(id, &ctx); }
                if ui.add_enabled(self.dialog.is_none(), egui::Button::new("New mind map")).clicked() {
                    let result = self.workspace.new_map().map(|_| ()).map_err(EditorError::from);
                    self.report(result);
                }
                if ui.add_enabled(self.dialog.is_none(), egui::Button::new("Open another map")).clicked() { self.open_file(&ctx); }
            });
            if let Some(cause) = &self.error { ui.colored_label(Color32::from_rgb(210, 75, 70), cause); }
        });
        self.dialogs(&ctx);
        self.debounce_recovery(&ctx);
        self.finish_tab_transition(&ctx);
    }

    fn retry_failed_tab(&mut self, id: SessionId, locate: bool, ctx: &egui::Context) {
        let result = (|| -> Result<(), EditorError> {
            let mut restored = self.store.retry_session(id)?;
            if locate {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("BMM mind map", &["branchline"])
                    .pick_file()
                else {
                    return Ok(());
                };
                restored.tab.saved_path = Some(path);
                restored.content = crate::recovery::RestoreContent::Saved;
            }
            self.workspace.retry_restore(restored)?;
            self.error = None;
            self.checkpoint_recovery()?;
            self.reset_tab_renderer(ctx);
            Ok(())
        })();
        self.report(result);
    }
}

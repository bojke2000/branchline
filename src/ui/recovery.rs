use super::*;

impl NativeApp {
    pub(super) fn debounce_recovery(&mut self, ctx: &egui::Context) {
        if !self.recovery_pending
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
            self.error =
                Some("Switch to a loaded map before quitting; all open tabs were retained.".into());
        }
        self.tab_strip(root, self.dialog.is_some());
        egui::CentralPanel::default().show(root, |ui| {
            ui.heading("This mind map could not be restored");
            if let Err(cause) = self.workspace.active() {
                ui.label(cause.to_string());
            }
            ui.label("The saved file and recovery resources have not been replaced with an empty map. Other tabs remain available.");
            ui.horizontal(|ui| {
                if ui.button("New mind map").clicked() {
                    let result = self.workspace.new_map().map(|_| ()).map_err(EditorError::from);
                    self.report(result);
                }
                if ui.button("Open another map").clicked() { self.open_file(&ctx); }
            });
            if let Some(cause) = &self.error { ui.colored_label(Color32::from_rgb(210, 75, 70), cause); }
        });
        self.dialogs(&ctx);
        self.debounce_recovery(&ctx);
        self.finish_tab_transition(&ctx);
    }
}

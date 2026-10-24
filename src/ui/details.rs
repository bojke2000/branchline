use super::*;

impl NativeApp {
    pub(super) fn details_panel(&mut self, ui: &mut egui::Ui) {
        let editor = &self.workspace.map().editor;
        if editor
            .details
            .as_ref()
            .is_none_or(|details| details.id != editor.selected)
        {
            let id = editor.selected.clone();
            if let Err(error) = self.edit(EditorEdit::BeginDetails(id)) {
                self.report(Err(error));
                return;
            }
        }
        let session = self.workspace.active_id();
        let mut details = self
            .workspace
            .map()
            .editor
            .details
            .clone()
            .expect("owned details");
        let mut commit = false;
        egui::Panel::right("notes").default_size(290.0).min_size(220.0).show(ui, |ui| {
            ui.add_space(12.0);
            ui.heading("A little more context");
            ui.weak("Notes for the selected idea");
            ui.add_space(12.0);
            let response = ui.add(egui::TextEdit::multiline(&mut details.notes)
                .id(Id::unique(("notes", session, &details.id)))
                .desired_width(f32::INFINITY).desired_rows(14)
                .hint_text("Capture details here...").char_limit(262_144));
            self.owned_widgets.entry(session).or_default().insert(response.id);
            if response.changed() {
                let result = self.edit(EditorEdit::SetDraft {
                    field: DraftField::Notes(details.id.clone()),
                    text: std::mem::take(&mut details.notes),
                });
                self.report(result);
            }
            commit |= response.lost_focus();
            ui.add_space(12.0);
            ui.label("Link");
            let response = ui.add(egui::TextEdit::singleline(&mut details.link)
                .id(Id::unique(("link", session, &details.id)))
                .hint_text("Paste a URL or path").char_limit(262_144));
            self.owned_widgets.entry(session).or_default().insert(response.id);
            if response.changed() {
                let result = self.edit(EditorEdit::SetDraft {
                    field: DraftField::Link(details.id.clone()),
                    text: std::mem::take(&mut details.link),
                });
                self.report(result);
            }
            commit |= response.lost_focus();
            if ui.button("Open link").clicked() {
                let link = &self.workspace.map().editor.details.as_ref().expect("owned details").link;
                if !link.is_empty() {
                    if ["https://", "http://", "mailto:"].iter().any(|prefix| link.starts_with(prefix)) {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(link));
                    } else {
                        self.error = Some("Only explicit http, https and mailto links can be opened here; copy other paths manually.".into());
                    }
                }
            }
            ui.weak("Notes are plain text. No scripts or formulas run.");
        });
        if commit
            && self.dialog.is_none()
            && self.workspace.pending_activation().is_none()
            && self.pending_close.is_none()
        {
            let result = self.edit(EditorEdit::FlushDetails);
            self.report(result);
        }
    }
}

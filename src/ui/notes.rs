use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    notes_font_size: f32,
}

pub(super) fn load_font_size(path: &std::path::Path) -> Result<Option<f32>, StorageError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(StorageError(format!("{}: {error}", path.display()))),
        Ok(_) => {}
    }
    let preferences: Preferences = serde_json::from_slice(&read_bounded(path)?)
        .map_err(|error| StorageError(format!("{}: {error}", path.display())))?;
    if !preferences.notes_font_size.is_finite()
        || !(10.0..=32.0).contains(&preferences.notes_font_size)
    {
        return Err(StorageError(
            "Saved Notes font size must be between 10 and 32.".into(),
        ));
    }
    Ok(Some(preferences.notes_font_size))
}

pub(super) struct NotesPopup {
    session: SessionId,
    target: NodeId,
    text: String,
    focus: bool,
    composing: bool,
    error: Option<String>,
}

impl NativeApp {
    pub(super) fn open_notes(&mut self) {
        let map = self.workspace.map();
        let target = map.editor.selected.clone();
        let text = map
            .editor
            .details
            .as_ref()
            .filter(|draft| draft.id == target)
            .map_or_else(
                || map.editor.document.nodes[&target].notes.clone(),
                |draft| draft.notes.clone(),
            );
        self.dialog = Some(Dialog::Notes(NotesPopup {
            session: self.workspace.active_id(),
            target,
            text,
            focus: true,
            composing: false,
            error: None,
        }));
    }

    pub(super) fn notes_dialog(&mut self, ui: &mut egui::Ui, popup: &mut NotesPopup) -> bool {
        let ctx = ui.ctx().clone();
        let viewport = ui.input(|input| input.viewport_rect());
        ui.set_width((viewport.width() - 64.0).min(640.0));
        let id = Id::unique(("notes-popup", popup.session, &popup.target));
        if popup.focus {
            egui::text_edit::TextEditState::default().store(&ctx, id);
        }
        let composition_frame = popup.composing
            || ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Ime(_)))
            });
        ui.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                        popup.composing = !text.is_empty()
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(_)) => popup.composing = false,
                    _ => {}
                }
            }
        });
        let (shortcut_apply, escape) = if composition_frame {
            (false, false)
        } else {
            ui.input_mut(|input| {
                let modifiers = input.modifiers;
                let apply = (modifiers.command || modifiers.ctrl || modifiers.mac_cmd)
                    && !modifiers.alt
                    && input.consume_key(modifiers, Key::Enter);
                (apply, input.consume_key(egui::Modifiers::NONE, Key::Escape))
            })
        };
        ui.heading("Notes");
        ui.label("Write your notes below.");
        let mut font = egui::FontSelection::Default.resolve(ui.style());
        font.size = self
            .notes_font_size
            .unwrap_or((font.size + 1.0).clamp(10.0, 32.0));
        let mut requested_size = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !composition_frame && font.size > 10.0,
                    egui::Button::new("-"),
                )
                .on_hover_text("Decrease notes font size")
                .clicked()
            {
                requested_size = Some((font.size - 1.0).max(10.0));
            }
            ui.label("fn");
            if ui
                .add_enabled(
                    !composition_frame && font.size < 32.0,
                    egui::Button::new("+"),
                )
                .on_hover_text("Increase notes font size")
                .clicked()
            {
                requested_size = Some((font.size + 1.0).min(32.0));
            }
        });
        let mut font_changed = false;
        if let Some(size) = requested_size {
            let result = crate::storage::serialize_bounded(&Preferences {
                notes_font_size: size,
            })
            .and_then(|bytes| atomic_write(&self.store.preferences_path(), &bytes));
            match result {
                Ok(()) => {
                    self.notes_font_size = Some(size);
                    font.size = size;
                    font_changed = true;
                }
                Err(error) => {
                    let message =
                        format!("Could not save Notes font size; previous size retained: {error}");
                    popup.error = Some(message.clone());
                    self.error = Some(message);
                }
            }
        }
        let previous = popup.text.clone();
        let height = (viewport.height() * 0.336).clamp(126.0, 308.0);
        let output = egui::ScrollArea::vertical()
            .id_salt("notes-source-scroll")
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::TextEdit::multiline(&mut popup.text)
                    .id(id)
                    .font(font)
                    .min_size(egui::vec2(0.0, height))
                    .desired_width(f32::INFINITY)
                    .char_limit(DRAFT_CHAR_PROBE_LIMIT)
                    .show(ui)
            })
            .inner;
        if popup.text.len() > crate::document::MAX_FIELD_BYTES {
            popup.text = previous;
            let message =
                "Notes exceed the 1 MiB UTF-8 field limit; the previous input was retained.";
            popup.error = Some(message.into());
            self.error = Some(message.into());
        }
        if popup.focus || font_changed {
            output.response.request_focus();
            popup.focus = false;
        }
        if let Some(error) = &popup.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let mut apply = shortcut_apply;
        let mut cancel = escape;
        ui.horizontal(|ui| {
            apply |= ui
                .add_enabled(!composition_frame, egui::Button::new("Apply"))
                .clicked();
            cancel |= ui.button("Cancel").clicked();
            ui.weak("Cmd/Ctrl+Enter applies");
        });
        if apply {
            let result = if self.workspace.active_id() != popup.session {
                Err(EditorError::State(
                    "The notes popup belongs to a different map.".into(),
                ))
            } else {
                self.edit(EditorEdit::ApplyNotes {
                    id: popup.target.clone(),
                    notes: popup.text.clone(),
                })
            };
            if let Err(error) = result {
                popup.error = Some(error.to_string());
                self.report(Err(error));
                return true;
            }
            self.status = "Notes applied. Undo restores the previous notes.".into();
        }
        if apply || cancel {
            self.notes_focus = Some((popup.session, popup.target.clone()));
            ctx.data_mut(|data| {
                data.remove::<egui::text_edit::TextEditState>(id);
            });
            ctx.request_repaint();
            return false;
        }
        true
    }
}

use super::*;
use egui::text::{CCursor, CCursorRange};

pub(super) struct NotesPopup {
    session: SessionId,
    target: NodeId,
    text: String,
    focus: bool,
    composing: bool,
    font_adjustment: f32,
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
            font_adjustment: 0.0,
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
        ui.label("Write Markdown below. Use Bold or Bullet list to format your notes.");
        let mut format = None;
        let mut font = egui::FontSelection::Default.resolve(ui.style());
        font.size = (font.size + 1.0 + popup.font_adjustment).clamp(10.0, 32.0);
        let mut font_changed = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!composition_frame, egui::Button::new("Bold"))
                .clicked()
            {
                format = Some(Format::Bold);
            }
            if ui
                .add_enabled(!composition_frame, egui::Button::new("Bullet list"))
                .clicked()
            {
                format = Some(Format::Bullets);
            }
            ui.separator();
            if ui
                .add_enabled(
                    !composition_frame && font.size > 10.0,
                    egui::Button::new("-"),
                )
                .on_hover_text("Decrease notes font size")
                .clicked()
            {
                popup.font_adjustment -= 1.0;
                font.size = (font.size - 1.0).max(10.0);
                font_changed = true;
            }
            if ui
                .add_enabled(
                    !composition_frame && font.size < 32.0,
                    egui::Button::new("+"),
                )
                .on_hover_text("Increase notes font size")
                .clicked()
            {
                popup.font_adjustment += 1.0;
                font.size = (font.size + 1.0).min(32.0);
                font_changed = true;
            }
        });
        let previous = popup.text.clone();
        let mut selection = None;
        if let Some(format) = format {
            let range = egui::text_edit::TextEditState::load(&ctx, id)
                .and_then(|state| state.cursor.char_range())
                .unwrap_or_else(|| CCursorRange::one(CCursor::new(popup.text.chars().count())));
            let (text, range) = format_text(&popup.text, range, format);
            popup.text = text;
            selection = Some(range);
        }
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
            selection = None;
        }
        if popup.focus || format.is_some() || font_changed {
            output.response.request_focus();
            popup.focus = false;
        }
        if let Some(range) = selection {
            let mut state = output.state;
            state.cursor.set_char_range(Some(range));
            state.store(&ctx, id);
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

#[derive(Clone, Copy)]
enum Format {
    Bold,
    Bullets,
}

fn byte_index(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map_or(text.len(), |(byte, _)| byte)
}

fn format_text(text: &str, range: CCursorRange, format: Format) -> (String, CCursorRange) {
    let [start, end] = range.sorted_cursors();
    let collapsed = start.index == end.index;
    let caret = usize::from(start.index);
    let mut start = byte_index(text, start.index.into());
    let mut end = byte_index(text, end.index.into());
    let replacement = match format {
        Format::Bold => {
            let selected = &text[start..end];
            format!("**{selected}**")
        }
        Format::Bullets => {
            start = text[..start].rfind('\n').map_or(0, |index| index + 1);
            if end > start && text[..end].ends_with('\n') {
                end -= 1;
            } else {
                end = text[end..]
                    .find('\n')
                    .map_or(text.len(), |index| end + index);
            }
            text[start..end]
                .split('\n')
                .map(|line| {
                    if collapsed && line.trim().is_empty() {
                        format!("{line}- ")
                    } else if line.trim().is_empty() || line.trim_start().starts_with("- ") {
                        line.to_owned()
                    } else {
                        let indent = line.len() - line.trim_start().len();
                        format!("{}- {}", &line[..indent], &line[indent..])
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let mut result = String::with_capacity(text.len() + replacement.len());
    result.push_str(&text[..start]);
    result.push_str(&replacement);
    result.push_str(&text[end..]);
    let first = text[..start].chars().count();
    let last = first + replacement.chars().count();
    let range = if matches!(format, Format::Bold) {
        CCursorRange::two(CCursor::new(first + 2), CCursor::new(last - 2))
    } else if collapsed {
        CCursorRange::one(CCursor::new(
            caret + replacement.chars().count() - text[start..end].chars().count(),
        ))
    } else {
        CCursorRange::two(CCursor::new(first), CCursor::new(last))
    };
    (result, range)
}

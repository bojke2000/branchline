use super::*;
use crate::{
    outline_format::{
        OutlineCopy, OutlineWarning, OutlineWarningKind, ParsedOutline, PasteInterpretation,
        parse_clipboard, serialize_outline,
    },
    workspace::PasteTarget,
};

pub struct PendingPaste {
    pub target: PasteTarget,
    pub interpretation: PasteInterpretation,
}

pub(super) struct ClipboardGesture {
    pub key: Key,
    pub plain: Option<String>,
}

fn backend_agrees(native: Option<&str>, backend: &str) -> bool {
    native.is_some_and(|native| {
        native
            .bytes()
            .enumerate()
            .filter_map(|(index, byte)| {
                // egui-winit 0.37 converts CRLF to LF before emitting Event::Paste.
                if byte == b'\r' && native.as_bytes().get(index + 1) == Some(&b'\n') {
                    None
                } else {
                    Some(byte)
                }
            })
            .eq(backend.bytes())
    })
}

fn warning_label(warning: &OutlineWarning) -> String {
    let message = match warning.category {
        OutlineWarningKind::AmbiguousIndentation => {
            "Indentation or list levels are inconsistent; review the proposed nesting"
        }
        OutlineWarningKind::UnsupportedOfficeMetadata => {
            "Office list metadata is not supported; no nesting was inferred from its class name"
        }
        OutlineWarningKind::OutsideListText => {
            "Paragraphs outside lists are proposed as separate top-level ideas"
        }
        OutlineWarningKind::RepresentationLoss => {
            "Plain text cannot preserve every multiline/whitespace label, or HTML required recovery"
        }
        OutlineWarningKind::IgnoredContent => "Only supported text/list content is represented",
        OutlineWarningKind::ConflictingFormats => {
            "Rich and plain representations disagree; choose the intended outline"
        }
    };
    format!(
        "{message} ({} occurrence(s)).{}",
        warning.count,
        warning
            .detail
            .as_ref()
            .map_or(String::new(), |detail| format!(" {detail}"))
    )
}

fn candidates(interpretation: &PasteInterpretation) -> &[ParsedOutline] {
    match interpretation {
        PasteInterpretation::Ready(parsed) => std::slice::from_ref(parsed),
        PasteInterpretation::Preview(parsed) => parsed,
    }
}

impl NativeApp {
    pub(super) fn copy_outline(&mut self) {
        let copy = self
            .workspace
            .map()
            .editor
            .copy_forest()
            .map_err(|error| error.to_string())
            .and_then(|forest| serialize_outline(&forest).map_err(|error| error.to_string()));
        match copy {
            Ok(copy) if !copy.warnings.is_empty() => self.dialog = Some(Dialog::CopyOutline(copy)),
            Ok(copy) => self.publish_copy(copy.payload),
            Err(error) => self.error = Some(error),
        }
    }

    fn publish_copy(&mut self, payload: crate::outline_format::ClipboardPayload) {
        match self.clipboard.write(payload) {
            Ok(()) => {
                self.status = "Copied hierarchy as nested HTML and tab-indented bullets.".into()
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub(super) fn begin_paste(&mut self, new_map: bool, backend: Option<&str>) -> Option<String> {
        let result = (|| {
            let map = self.workspace.active()?;
            let target = if new_map {
                PasteTarget::NewMap
            } else {
                PasteTarget::Append {
                    session: map.id(),
                    node: map.editor.selected.clone(),
                    revision: map.editor.revision,
                }
            };
            let mut payload = self
                .clipboard
                .read()
                .map_err(|error| EditorError::State(error.to_string()))?;
            payload
                .validate()
                .map_err(|error| EditorError::State(error.to_string()))?;
            if backend.is_some_and(|text| !backend_agrees(payload.plain.as_deref(), text)) {
                return Err(EditorError::State("Clipboard changed between the backend paste event and native acquisition. No outline was inserted; invoke paste again.".into()));
            }
            let mut interpretation =
                parse_clipboard(&payload).map_err(|error| EditorError::State(error.to_string()))?;
            if let Some(limitation) = self.clipboard.read_limitation() {
                let mut choices = match interpretation {
                    PasteInterpretation::Ready(parsed) => vec![parsed],
                    PasteInterpretation::Preview(choices) => choices,
                };
                for choice in &mut choices {
                    choice.warnings.push(OutlineWarning {
                        category: OutlineWarningKind::IgnoredContent,
                        count: 1,
                        locations: vec![],
                        detail: Some(limitation.into()),
                    });
                }
                interpretation = PasteInterpretation::Preview(choices);
            }
            match interpretation {
                PasteInterpretation::Ready(parsed) => self.publish_outline(target, parsed)?,
                interpretation @ PasteInterpretation::Preview(_) => {
                    self.cancel_canvas_drag();
                    self.dialog = Some(Dialog::Paste(PendingPaste {
                        target,
                        interpretation,
                    }));
                }
            }
            Ok(payload.plain.take())
        })();
        match result {
            Ok(plain) => plain,
            Err(error) => {
                self.error = Some(error.to_string());
                None
            }
        }
    }

    fn publish_outline(
        &mut self,
        target: PasteTarget,
        parsed: ParsedOutline,
    ) -> Result<(), EditorError> {
        let new_map = matches!(&target, PasteTarget::NewMap);
        let id = self.workspace.paste_outline(target, parsed.forest)?;
        self.workspace.request_activate(id)?;
        self.focus_selected = true;
        self.fit_requested |= new_map;
        self.status = if parsed.warnings.is_empty() {
            "Outline pasted. Undo restores the insertion.".into()
        } else {
            format!(
                "Outline pasted. {}",
                parsed
                    .warnings
                    .iter()
                    .map(warning_label)
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        };
        Ok(())
    }

    pub(super) fn paste_dialog(&mut self, ui: &mut egui::Ui, pending: &mut PendingPaste) -> bool {
        ui.heading("Review clipboard outline");
        ui.label("Nothing changes until you accept. Labels and hierarchy transfer; notes, links and formatting do not.");
        let mut selected = None;
        for (index, parsed) in candidates(&pending.interpretation).iter().enumerate() {
            ui.group(|ui| {
                ui.strong(format!("{:?} outline", parsed.source));
                for warning in &parsed.warnings { ui.label(warning_label(warning)); }
                egui::ScrollArea::both().id_salt(("outline-preview", index)).max_height(220.0).show(ui, |ui| {
                    let mut levels = vec![parsed.forest.roots.iter()];
                    let mut shown = 0;
                    while let Some(level) = levels.last_mut() {
                        let Some(node) = level.next() else { levels.pop(); continue; };
                        if shown == 200 { ui.weak("Preview limited to the first 200 ideas; acceptance pastes the full outline."); break; }
                        let depth = levels.len() - 1;
                        let mut label: String = node.label.chars().take(160).collect();
                        if node.label.chars().nth(160).is_some() { label.push_str("..."); }
                        ui.label(format!("{}- {label}", "  ".repeat(depth)));
                        shown += 1;
                        if !node.children.is_empty() { levels.push(node.children.iter()); }
                    }
                });
                if ui.button("Use this outline").clicked() { selected = Some(index); }
            });
        }
        if ui.button("Cancel paste").clicked() {
            return false;
        }
        if let Some(index) = selected {
            let parsed = match &mut pending.interpretation {
                PasteInterpretation::Ready(parsed) => parsed,
                PasteInterpretation::Preview(choices) => &mut choices[index],
            };
            let parsed = ParsedOutline {
                forest: crate::outline::OutlineForest {
                    roots: std::mem::take(&mut parsed.forest.roots),
                },
                source: parsed.source,
                warnings: std::mem::take(&mut parsed.warnings),
            };
            let result = self.publish_outline(pending.target.clone(), parsed);
            self.report(result);
            return false;
        }
        true
    }

    pub(super) fn copy_dialog(&mut self, ui: &mut egui::Ui, copy: &mut OutlineCopy) -> bool {
        ui.heading("Review clipboard representation");
        ui.label("The rich HTML retains full labels. A plain-text editor may normalize their line breaks or leading whitespace.");
        for warning in &copy.warnings {
            ui.label(warning_label(warning));
        }
        if ui.button("Copy outline").clicked() {
            self.publish_copy(std::mem::take(&mut copy.payload));
            return false;
        }
        !ui.button("Cancel copy").clicked()
    }

    pub(super) fn clipboard_shortcut(
        &mut self,
        ctx: &egui::Context,
        bound: Option<(Key, AppCommand)>,
        events: &[egui::Event],
    ) -> bool {
        let requested = bound.filter(|(_, command)| {
            matches!(
                command,
                AppCommand::Copy | AppCommand::Paste | AppCommand::PasteAsNewMap
            )
        });
        let copy_event = events
            .iter()
            .any(|event| matches!(event, egui::Event::Copy));
        let mut paste_events = events.iter().filter_map(|event| match event {
            egui::Event::Paste(text) => Some(text.as_str()),
            _ => None,
        });
        let backend = paste_events.next();
        if paste_events.any(|text| Some(text) != backend) {
            self.error = Some("Different backend clipboard payloads arrived in one gesture; no outline was inserted.".into());
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_) | egui::Event::Copy))
            });
            return true;
        }
        if let Some(gesture) = &self.clipboard_gesture {
            let repeated_key = requested.is_some_and(|(key, _)| key == gesture.key)
                && events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            repeat: true,
                            pressed: true,
                            ..
                        }
                    )
                });
            let late_backend = requested.is_none()
                && (gesture.key == Key::V && backend.is_some()
                    || gesture.key == Key::C && copy_event);
            if repeated_key || late_backend {
                if gesture.key == Key::V
                    && backend.is_some_and(|text| !backend_agrees(gesture.plain.as_deref(), text))
                {
                    self.error = Some("Clipboard changed in a late backend event; the earlier captured outline was not supplemented or replaced.".into());
                }
                ctx.input_mut(|input| {
                    input
                        .events
                        .retain(|event| !matches!(event, egui::Event::Paste(_) | egui::Event::Copy))
                });
                if !ctx.input(|input| input.key_down(gesture.key)) {
                    self.clipboard_gesture = None;
                }
                return true;
            }
            if !ctx.input(|input| input.key_down(gesture.key)) {
                self.clipboard_gesture = None;
            }
        }
        let command = requested.map(|(_, command)| command).or_else(|| {
            if backend.is_some() {
                Some(AppCommand::Paste)
            } else if copy_event {
                Some(AppCommand::Copy)
            } else {
                None
            }
        });
        let Some(command) = command else { return false };
        ctx.input_mut(|input| {
            if let Some((key, _)) = requested {
                input.consume_key(input.modifiers, key);
            }
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Paste(_) | egui::Event::Copy));
        });
        if copy_event && (backend.is_some() || command != AppCommand::Copy)
            || backend.is_some() && command == AppCommand::Copy
        {
            self.error = Some("Conflicting clipboard operations arrived in one gesture; invoke the intended command again.".into());
            return true;
        }
        let plain = if command == AppCommand::Copy {
            self.copy_outline();
            None
        } else {
            self.begin_paste(command == AppCommand::PasteAsNewMap, backend)
        };
        self.clipboard_gesture = requested.map(|(key, _)| ClipboardGesture { key, plain });
        true
    }
}

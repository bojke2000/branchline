use branchline::{
    app::{DraftField, EditorAction, EditorEdit},
    clipboard::{ClipboardError, OutlineClipboard},
    outline_format::ClipboardPayload,
};
use branchline::{
    app::{Editor, SelectionCoverage},
    interchange::import_mm,
    recovery::RecoveryCoordinator,
    ui::NativeApp,
};
use eframe::egui::{self, Pos2, Shape};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct ClipboardState {
    payload: ClipboardPayload,
    reads: usize,
    writes: Vec<ClipboardPayload>,
    fail_read: bool,
    fail_write: bool,
}
struct FakeClipboard(Arc<Mutex<ClipboardState>>);

#[test]
fn standalone_backend_clipboard_commands_work_immediately_after_tab_switch() {
    for copy in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let payload = ClipboardPayload {
            plain: Some("- Pasted after switch".into()),
            html: None,
        };
        let (mut app, state) = app_with_clipboard(temp.path(), payload);
        let ctx = egui::Context::default();
        render(&mut app, &ctx, vec![]);
        let source = app.workspace.active_id();
        key(&mut app, &ctx, egui::Key::N, egui::Modifiers::COMMAND);
        let target = app.workspace.active_id();
        key(&mut app, &ctx, egui::Key::Tab, egui::Modifiers::CTRL);
        assert_eq!(app.workspace.active_id(), source);
        key(&mut app, &ctx, egui::Key::Tab, egui::Modifiers::CTRL);
        assert_eq!(app.workspace.active_id(), target);
        let event = if copy {
            egui::Event::Copy
        } else {
            egui::Event::Paste("- Pasted after switch".into())
        };
        render(&mut app, &ctx, vec![event]);
        let clipboard = state.lock().unwrap();
        if copy {
            assert_eq!(clipboard.writes.len(), 1);
            assert_eq!(
                clipboard.writes[0].plain.as_deref(),
                Some("- Central idea\n")
            );
        } else {
            assert_eq!(clipboard.reads, 1);
            let document = &app.workspace.session(target).unwrap().editor.document;
            assert_eq!(document.nodes[&document.root].text, "Pasted after switch");
        }
        let untouched = &app.workspace.session(source).unwrap().editor.document;
        assert_eq!(untouched.nodes[&untouched.root].text, "Central idea");
    }
}

#[test]
fn draft_widgets_admit_complete_large_pastes_and_reject_overflow_atomically() {
    for field in ["inline", "notes", "link"] {
        let temp = tempfile::tempdir().unwrap();
        let (mut app, state) = app_with_clipboard(temp.path(), ClipboardPayload::default());
        let ctx = egui::Context::default();
        let session = app.workspace.active_id();
        let owner = app.workspace.map().editor.selected.clone();
        if field == "inline" {
            app.workspace
                .apply(session, EditorEdit::Action(EditorAction::Edit))
                .unwrap();
        } else {
            app.workspace.map_mut().notes_open = true;
        }
        render(&mut app, &ctx, vec![]);
        render(&mut app, &ctx, vec![]);
        let widget = egui::Id::unique((field, session, &owner));
        let before = app.workspace.map().editor.document.clone();
        fn value(app: &NativeApp, field: &str) -> String {
            let editor = &app.workspace.map().editor;
            match field {
                "inline" => editor.editing.as_ref().unwrap().text.clone(),
                "notes" => editor.details.as_ref().unwrap().notes.clone(),
                _ => editor.details.as_ref().unwrap().link.clone(),
            }
        }
        fn select_all(ctx: &egui::Context, widget: egui::Id, count: usize) {
            ctx.memory_mut(|memory| memory.request_focus(widget));
            let mut state = egui::TextEdit::load_state(ctx, widget).unwrap();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(count),
                )));
            state.store(ctx, widget);
        }
        let original = value(&app, field);
        select_all(&ctx, widget, original.chars().count());
        let accepted = "a".repeat(300_000);
        render(&mut app, &ctx, vec![egui::Event::Paste(accepted.clone())]);
        assert_eq!(
            value(&app, field).len(),
            accepted.len(),
            "{field} silently clipped accepted text"
        );
        assert_eq!(value(&app, field), accepted);
        for overflow in [
            "b".repeat(branchline::document::MAX_FIELD_BYTES + 1),
            "🚀".repeat(branchline::document::MAX_FIELD_BYTES / 4 + 1),
        ] {
            select_all(&ctx, widget, accepted.len());
            render(&mut app, &ctx, vec![egui::Event::Paste(overflow)]);
            assert_eq!(
                value(&app, field),
                accepted,
                "{field} replaced the draft with truncated overflow"
            );
        }
        let shapes = render(&mut app, &ctx, vec![]);
        assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text.contains("previous draft was retained"))));
        assert_eq!(app.workspace.map().editor.document, before);
        key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        assert_eq!(
            value(&app, field),
            original,
            "{field} retained rejected text in local undo"
        );
        let clipboard = state.lock().unwrap();
        assert_eq!(clipboard.reads, 0);
        assert!(clipboard.writes.is_empty());
    }
}

impl OutlineClipboard for FakeClipboard {
    fn read(&mut self) -> Result<ClipboardPayload, ClipboardError> {
        let mut state = self.0.lock().unwrap();
        state.reads += 1;
        if state.fail_read {
            return Err(ClipboardError("Clipboard acquisition denied".into()));
        }
        Ok(state.payload.clone())
    }
    fn write(&mut self, payload: ClipboardPayload) -> Result<(), ClipboardError> {
        let mut state = self.0.lock().unwrap();
        if state.fail_write {
            return Err(ClipboardError("Clipboard publication denied".into()));
        }
        state.writes.push(payload);
        Ok(())
    }
}

fn app_with_clipboard(
    path: &std::path::Path,
    payload: ClipboardPayload,
) -> (NativeApp, Arc<Mutex<ClipboardState>>) {
    let state = Arc::new(Mutex::new(ClipboardState {
        payload,
        ..Default::default()
    }));
    (
        NativeApp::with_clipboard(
            RecoveryCoordinator::open(path.into()).unwrap(),
            Box::new(FakeClipboard(state.clone())),
        )
        .unwrap(),
        state,
    )
}

fn click_label(app: &mut NativeApp, ctx: &egui::Context, label: &str) {
    let shapes = render(app, ctx, vec![]);
    let position = shapes
        .iter()
        .rev()
        .find_map(|shape| match shape {
            Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing UI action {label:?}"));
    for pressed in [true, false] {
        render(
            app,
            ctx,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn physical_ctrl_and_cmd_copy_the_map() {
    for modifiers in [
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::Modifiers::COMMAND,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (mut app, state) = app_with_clipboard(temp.path(), ClipboardPayload::default());
        app.workspace.map_mut().editor = Editor::from_document(import_mm(br#"<map><node ID="r" TEXT="Root"><node ID="a" TEXT="A" FOLDED="true"><node ID="b" TEXT="B"/></node></node></map>"#).unwrap().document, None, true, None).unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, vec![]);
        render(&mut app, &ctx, vec![]);
        key(&mut app, &ctx, egui::Key::A, modifiers);
        let before = app.workspace.map().editor.document.clone();
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key {
                    key: egui::Key::C,
                    physical_key: Some(egui::Key::C),
                    pressed: true,
                    repeat: false,
                    modifiers,
                },
                egui::Event::Copy,
            ],
        );
        let state = state.lock().unwrap();
        assert_eq!(state.reads, 0);
        assert_eq!(state.writes.len(), 1);
        assert_eq!(
            state.writes[0].plain.as_deref(),
            Some("- Root\n\t- A\n\t\t- B\n")
        );
        assert!(state.writes[0].html.as_ref().unwrap().contains("<li>"));
        assert_eq!(app.workspace.map().editor.document, before);
    }
}

#[test]
fn paste_builds_blank_or_appends_under_active_node() {
    let temp = tempfile::tempdir().unwrap();
    let payload = ClipboardPayload {
        plain: Some("- A\n\t- B\n\t- C".into()),
        html: None,
    };
    let (mut app, state) = app_with_clipboard(temp.path(), payload.clone());
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    assert_eq!(state.lock().unwrap().reads, 0);
    let modifiers = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key: egui::Key::V,
                physical_key: Some(egui::Key::V),
                pressed: true,
                repeat: false,
                modifiers,
            },
            egui::Event::Paste(payload.plain.clone().unwrap()),
        ],
    );
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 3);
    assert_eq!(app.workspace.map().editor.history.generation, 1);
    assert_eq!(state.lock().unwrap().reads, 1);
    let root = app.workspace.map().editor.document.root.clone();
    assert_eq!(app.workspace.map().editor.document.nodes[&root].text, "A");
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::V,
            physical_key: Some(egui::Key::V),
            pressed: false,
            repeat: false,
            modifiers,
        }],
    );
    key(&mut app, &ctx, egui::Key::V, modifiers);
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 6);
    assert_eq!(state.lock().unwrap().reads, 2);
}

#[test]
fn paste_as_new_map_and_cancel_are_non_destructive() {
    let temp = tempfile::tempdir().unwrap();
    let payload = ClipboardPayload {
        plain: Some("- A\n  - B\n      - C".into()),
        html: None,
    };
    let (mut app, state) = app_with_clipboard(temp.path(), payload);
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let original = app.workspace.active_id();
    let before = app.workspace.map().editor.document.clone();
    let modifiers = egui::Modifiers {
        ctrl: true,
        shift: true,
        ..Default::default()
    };
    key(&mut app, &ctx, egui::Key::V, modifiers);
    click_label(&mut app, &ctx, "Cancel paste");
    assert_eq!(app.workspace.len(), 1);
    assert_eq!(app.workspace.map().editor.document, before);
    assert!(state.lock().unwrap().writes.is_empty());
    key(&mut app, &ctx, egui::Key::V, modifiers);
    click_label(&mut app, &ctx, "Use this outline");
    assert_eq!(app.workspace.len(), 2);
    assert_ne!(app.workspace.active_id(), original);
    assert_eq!(
        app.workspace.session(original).unwrap().editor.document,
        before
    );
    assert!(app.workspace.map().editor.is_dirty());
}

#[test]
fn preview_keeps_its_original_session_target() {
    let temp = tempfile::tempdir().unwrap();
    let payload = ClipboardPayload {
        plain: Some("- A\n  - B\n      - C".into()),
        html: None,
    };
    let (mut app, _) = app_with_clipboard(temp.path(), payload);
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let original = app.workspace.active_id();
    key(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    let other = app.workspace.new_map().unwrap();
    app.workspace.apply_pending_activation().unwrap();
    let untouched = app.workspace.map().editor.document.clone();
    click_label(&mut app, &ctx, "Use this outline");
    assert_eq!(
        app.workspace
            .session(original)
            .unwrap()
            .editor
            .document
            .nodes
            .len(),
        3
    );
    assert_eq!(
        app.workspace.session(other).unwrap().editor.document,
        untouched
    );
}

#[test]
fn changed_clipboard_does_not_mix_with_backend_paste() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(
        temp.path(),
        ClipboardPayload {
            plain: Some("- New".into()),
            html: Some("<ul><li>New</li></ul>".into()),
        },
    );
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let before = app.workspace.map().editor.document.clone();
    render(&mut app, &ctx, vec![egui::Event::Paste("- Old".into())]);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, 0);
    assert_eq!(state.lock().unwrap().reads, 1);
    let shapes = render(&mut app, &ctx, vec![]);
    assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text.to_lowercase().contains("clipboard changed"))));
}

#[test]
fn backend_crlf_normalization_is_not_mistaken_for_a_clipboard_change() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(
        temp.path(),
        ClipboardPayload {
            plain: Some("- A\r\n\t- B\r\n".into()),
            html: Some("<ul><li>A<ul><li>B</li></ul></li></ul>".into()),
        },
    );
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("- A\n\t- B\n".into())],
    );
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 2);
    assert_eq!(app.workspace.map().editor.history.generation, 1);
    assert_eq!(state.lock().unwrap().reads, 1);
}

#[test]
fn text_fields_keep_native_clipboard_and_type_replace_ignores_paste() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(temp.path(), ClipboardPayload::default());
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let id = app.workspace.active_id();
    app.workspace
        .apply(id, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    render(&mut app, &ctx, vec![]);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("Label from clipboard".into())],
    );
    assert_eq!(state.lock().unwrap().reads, 0);
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 1);
    assert!(
        app.workspace
            .map()
            .editor
            .editing
            .as_ref()
            .unwrap()
            .text
            .contains("Label from clipboard")
    );
    key(&mut app, &ctx, egui::Key::C, egui::Modifiers::COMMAND);
    assert!(state.lock().unwrap().writes.is_empty());
    app.workspace
        .apply(
            id,
            EditorEdit::SetDraft {
                field: DraftField::Inline(app.workspace.map().editor.selected.clone()),
                text: "Committed".into(),
            },
        )
        .unwrap();
    app.workspace.apply(id, EditorEdit::CommitDrafts).unwrap();
    ctx.memory_mut(|memory| {
        if let Some(focused) = memory.focused() {
            memory.surrender_focus(focused);
        }
    });
    state.lock().unwrap().payload.plain = Some("- Branch".into());
    render(&mut app, &ctx, vec![egui::Event::Paste("- Branch".into())]);
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 2);
    assert!(app.workspace.map().editor.editing.is_none());
}

#[test]
fn held_shortcut_and_late_backend_paste_insert_only_the_captured_outline() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(
        temp.path(),
        ClipboardPayload {
            plain: Some("- A".into()),
            html: None,
        },
    );
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let modifiers = egui::Modifiers::CTRL;
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key: egui::Key::V,
                physical_key: Some(egui::Key::V),
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
    );
    let before = app.workspace.map().editor.document.clone();
    render(&mut app, &ctx, vec![egui::Event::Paste("- A".into())]);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::V,
            physical_key: Some(egui::Key::V),
            pressed: true,
            repeat: true,
            modifiers,
        }],
    );
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("- Different".into())],
    );
    assert_eq!(state.lock().unwrap().reads, 1);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, 1);
}

#[test]
fn paste_preserves_the_unrelated_notes_owner_through_rendering() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, _) = app_with_clipboard(
        temp.path(),
        ClipboardPayload {
            plain: Some("- Branch".into()),
            html: None,
        },
    );
    let ctx = egui::Context::default();
    let id = app.workspace.active_id();
    let root = app.workspace.map().editor.selected.clone();
    app.workspace.map_mut().notes_open = true;
    render(&mut app, &ctx, vec![]);
    app.workspace
        .apply(
            id,
            EditorEdit::SetDraft {
                field: DraftField::Notes(root.clone()),
                text: "Unfinished notes".into(),
            },
        )
        .unwrap();
    let generation = app.workspace.map().editor.history.generation;
    render(&mut app, &ctx, vec![egui::Event::Paste("- Branch".into())]);
    render(&mut app, &ctx, vec![]);
    let editor = &app.workspace.map().editor;
    assert_eq!(editor.document.nodes[&root].notes, "");
    assert_eq!(editor.details.as_ref().unwrap().id, root);
    assert_eq!(editor.details.as_ref().unwrap().notes, "Unfinished notes");
    assert_eq!(editor.history.generation, generation + 1);
    key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
    let editor = &app.workspace.map().editor;
    assert_eq!(editor.document.nodes.len(), 1);
    assert_eq!(editor.document.nodes[&root].notes, "");
    assert_eq!(editor.details.as_ref().unwrap().notes, "Unfinished notes");
}

#[test]
fn search_and_notes_fields_do_not_read_or_publish_the_map_clipboard() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(
        temp.path(),
        ClipboardPayload {
            plain: Some("- Do not import".into()),
            html: None,
        },
    );
    let ctx = egui::Context::default();
    app.workspace.map_mut().search.open = true;
    app.workspace.map_mut().search.request_focus = true;
    render(&mut app, &ctx, vec![]);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("Search label".into())],
    );
    assert_eq!(app.workspace.map().search.query, "Search label");
    key(&mut app, &ctx, egui::Key::C, egui::Modifiers::COMMAND);
    app.workspace.map_mut().search.open = false;
    app.workspace.map_mut().notes_open = true;
    render(&mut app, &ctx, vec![]);
    let notes = egui::Id::unique((
        "notes",
        app.workspace.active_id(),
        &app.workspace.map().editor.selected,
    ));
    ctx.memory_mut(|memory| memory.request_focus(notes));
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("Notes label".into())],
    );
    assert_eq!(
        app.workspace.map().editor.details.as_ref().unwrap().notes,
        "Notes label"
    );
    key(&mut app, &ctx, egui::Key::C, egui::Modifiers::COMMAND);
    let state = state.lock().unwrap();
    assert_eq!(state.reads, 0);
    assert!(state.writes.is_empty());
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 1);
}

#[test]
fn clipboard_failures_and_oversized_backend_events_do_not_change_the_map() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(temp.path(), ClipboardPayload::default());
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let before = app.workspace.map().editor.document.clone();
    state.lock().unwrap().fail_read = true;
    key(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    state.lock().unwrap().fail_write = true;
    key(&mut app, &ctx, egui::Key::C, egui::Modifiers::CTRL);
    let shapes = render(&mut app, &ctx, vec![]);
    assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text.contains("publication denied"))));
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Paste(
            "x".repeat(branchline::document::MAX_FILE_BYTES + 1),
        )],
    );
    assert_eq!(state.lock().unwrap().reads, 1);
    assert!(state.lock().unwrap().writes.is_empty());
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, 0);
}

#[test]
fn copy_representation_warning_precedes_publication_and_can_be_cancelled() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(temp.path(), ClipboardPayload::default());
    let ctx = egui::Context::default();
    let root = app.workspace.map().editor.selected.clone();
    app.workspace
        .apply(
            app.workspace.active_id(),
            EditorEdit::Command(branchline::document::Command::SetText {
                id: root,
                text: "A\nB".into(),
            }),
        )
        .unwrap();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, egui::Key::C, egui::Modifiers::CTRL);
    assert!(state.lock().unwrap().writes.is_empty());
    click_label(&mut app, &ctx, "Cancel copy");
    assert!(state.lock().unwrap().writes.is_empty());
    key(&mut app, &ctx, egui::Key::C, egui::Modifiers::CTRL);
    click_label(&mut app, &ctx, "Copy outline");
    assert_eq!(
        state.lock().unwrap().writes[0].plain.as_deref(),
        Some("- A B\n")
    );
    assert!(
        state.lock().unwrap().writes[0]
            .html
            .as_ref()
            .unwrap()
            .contains("A<br>B")
    );
    assert_eq!(state.lock().unwrap().reads, 0);
}
fn render(app: &mut NativeApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<Shape> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    fn flatten(shape: Shape, result: &mut Vec<Shape>) {
        match shape {
            Shape::Vec(children) => children
                .into_iter()
                .for_each(|child| flatten(child, result)),
            other => result.push(other),
        }
    }
    let mut result = vec![];
    for clipped in output.shapes {
        flatten(clipped.shape, &mut result);
    }
    result
}

#[test]
fn stale_preview_destination_does_not_edit_its_surviving_neighbor() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(
        temp.path(),
        ClipboardPayload {
            plain: Some("- A\n  - B\n      - C".into()),
            html: None,
        },
    );
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let original = app.workspace.active_id();
    key(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    let neighbor = app.workspace.new_map().unwrap();
    app.workspace.apply_pending_activation().unwrap();
    let before = app.workspace.map().editor.document.clone();
    let mut finalizer =
        RecoveryCoordinator::open(temp.path().join("test-close-publication")).unwrap();
    app.workspace.request_close(original).unwrap();
    app.workspace.publish_transition(&mut finalizer).unwrap();
    click_label(&mut app, &ctx, "Use this outline");
    assert_eq!(app.workspace.active_id(), neighbor);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.len(), 1);
    assert_eq!(state.lock().unwrap().reads, 1);
    assert!(state.lock().unwrap().writes.is_empty());
    let shapes = render(&mut app, &ctx, vec![]);
    assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text.contains("no longer open"))));
}

#[test]
fn pending_paste_does_not_deliver_late_input_to_a_disabled_notes_field() {
    let temp = tempfile::tempdir().unwrap();
    let (mut app, state) = app_with_clipboard(
        temp.path(),
        ClipboardPayload {
            plain: Some("- A\n  - B\n      - C".into()),
            html: None,
        },
    );
    let ctx = egui::Context::default();
    app.workspace.map_mut().notes_open = true;
    render(&mut app, &ctx, vec![]);
    render(&mut app, &ctx, vec![]);
    let id = app.workspace.active_id();
    let root = app.workspace.map().editor.selected.clone();
    key(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique(("notes", id, &root))));
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Paste("Not a notes edit".into())],
    );
    assert_eq!(
        app.workspace.map().editor.details.as_ref().unwrap().notes,
        ""
    );
    assert_eq!(app.workspace.map().editor.history.generation, 0);
    assert_eq!(state.lock().unwrap().reads, 1);
}
fn key(
    app: &mut NativeApp,
    ctx: &egui::Context,
    key: egui::Key,
    modifiers: egui::Modifiers,
) -> Vec<Shape> {
    let result = render(
        app,
        ctx,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
    );
    render(
        app,
        ctx,
        vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        }],
    );
    result
}

#[test]
fn physical_ctrl_and_cmd_select_the_folded_map_and_delete_requires_confirmation() {
    for modifiers in [
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        },
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
        app.workspace.map_mut().editor = Editor::from_document(import_mm(br#"<map><node ID="r" TEXT="Root" FOLDED="true"><node ID="a" TEXT="A"><node ID="b" TEXT="B"/></node><node ID="c" TEXT="C"/></node></map>"#).unwrap().document, None, true, None).unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, vec![]);
        render(&mut app, &ctx, vec![]);
        key(&mut app, &ctx, egui::Key::A, modifiers);
        assert_eq!(
            app.workspace.map().editor.selection_coverage(),
            SelectionCoverage::WholeMap
        );
        let shapes = render(&mut app, &ctx, vec![]);
        assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text.contains("4 selected (3 hidden)"))));
        key(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
        let shapes = render(&mut app, &ctx, vec![]);
        assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text == "The central idea will be kept.")));
        assert_eq!(app.workspace.map().editor.document.nodes.len(), 4);
    }
}

use super::*;
use crate::document::MAX_FIELD_BYTES;

fn frame(app: &mut NativeApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    frame_at_size(app, ctx, events, egui::vec2(1280.0, 840.0))
}

fn frame_at_size(
    app: &mut NativeApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    size: Vec2,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    output
}

fn position(output: &egui::FullOutput, value: &str) -> Pos2 {
    output
        .shapes
        .iter()
        .rev()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == value => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing visible UI text: {value}"))
}

fn click(app: &mut NativeApp, ctx: &egui::Context, pos: Pos2, button: egui::PointerButton) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

fn key(app: &mut NativeApp, ctx: &egui::Context, key: Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                },
            ],
        );
    }
}

fn replace_text(app: &mut NativeApp, ctx: &egui::Context, text: &str) {
    key(
        app,
        ctx,
        Key::A,
        egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        },
    );
    if text.is_empty() {
        key(app, ctx, Key::Backspace, egui::Modifiers::NONE);
        return;
    }
    frame(
        app,
        ctx,
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::Paste(text.into()),
        ],
    );
}

fn open_notes(app: &mut NativeApp, ctx: &egui::Context, label: &str) {
    let output = frame(app, ctx, vec![]);
    let target = position(&output, label);
    click(app, ctx, target, egui::PointerButton::Secondary);
    let output = frame(app, ctx, vec![]);
    let notes = position(&output, "Notes...");
    click(app, ctx, notes, egui::PointerButton::Primary);
    for _ in 0..3 {
        frame(app, ctx, vec![]);
    }
}

fn fixture() -> (tempfile::TempDir, NativeApp, NodeId, NodeId) {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let root = app.workspace.map().editor.document.root.clone();
    app.edit(EditorEdit::Command(Command::AddChild {
        parent: root.clone(),
        text: "Popup target".into(),
    }))
    .unwrap();
    let target = app.workspace.map().editor.selected.clone();
    app.edit(EditorEdit::Command(Command::SetDetails {
        id: target.clone(),
        notes: "Existing notes".into(),
        link: "https://example.com".into(),
    }))
    .unwrap();
    (temp, app, root, target)
}

#[test]
fn notes_font_buttons_resize_text_without_moving_controls_or_editing_notes() {
    let (_temp, mut app, _root, _target) = fixture();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    open_notes(&mut app, &ctx, "Popup target");
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let output = frame(&mut app, &ctx, vec![]);
    let font_size = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Existing notes" => {
                    Some(text.galley.job.sections[0].format.font_id.size)
                }
                _ => None,
            })
            .expect("visible notes input")
    };
    let initial_size = font_size(&output);
    assert!(position(&output, "-").x < position(&output, "fn").x);
    assert!(position(&output, "fn").x < position(&output, "+").x);
    assert!(!output.shapes.iter().any(|shape| matches!(
        &shape.shape, egui::Shape::Text(text) if ["Bold", "Bullet list"].contains(&text.galley.job.text.as_str())
    )));
    let apply = position(&output, "Apply");
    click(
        &mut app,
        &ctx,
        position(&output, "+"),
        egui::PointerButton::Primary,
    );
    let output = frame(&mut app, &ctx, vec![]);
    assert_eq!(font_size(&output), initial_size + 1.0);
    assert!(position(&output, "Apply").distance(apply) <= 1.0);
    click(
        &mut app,
        &ctx,
        position(&output, "-"),
        egui::PointerButton::Primary,
    );
    let output = frame(&mut app, &ctx, vec![]);
    assert_eq!(font_size(&output), initial_size);
    assert!(position(&output, "Apply").distance(apply) <= 1.0);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
    frame(&mut app, &ctx, vec![egui::Event::Text("+-".into())]);
    position(&frame(&mut app, &ctx, vec![]), "Existing notes+-");
}

#[test]
fn notes_font_size_is_shared_between_topics_and_survives_restart_after_cancel() {
    let (temp, mut app, root, _target) = fixture();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    open_notes(&mut app, &ctx, "Popup target");
    let input_size = |output: &egui::FullOutput, text: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => {
                    Some(shape.galley.job.sections[0].format.font_id.size)
                }
                _ => None,
            })
            .unwrap()
    };
    let output = frame(&mut app, &ctx, vec![]);
    let initial = input_size(&output, "Existing notes");
    click(
        &mut app,
        &ctx,
        position(&output, "+"),
        egui::PointerButton::Primary,
    );
    key(&mut app, &ctx, Key::Escape, egui::Modifiers::NONE);
    frame(&mut app, &ctx, vec![]);
    app.edit(EditorEdit::Select(root)).unwrap();
    app.run_command(AppCommand::EditNotes, &ctx);
    frame(&mut app, &ctx, vec![]);
    replace_text(&mut app, &ctx, "Other topic");
    assert_eq!(
        input_size(&frame(&mut app, &ctx, vec![]), "Other topic"),
        initial + 1.0
    );
    drop(app);
    let mut restarted =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let ctx = egui::Context::default();
    frame(&mut restarted, &ctx, vec![]);
    restarted.run_command(AppCommand::EditNotes, &ctx);
    frame(&mut restarted, &ctx, vec![]);
    frame(&mut restarted, &ctx, vec![]);
    replace_text(&mut restarted, &ctx, "After restart");
    assert_eq!(
        input_size(&frame(&mut restarted, &ctx, vec![]), "After restart"),
        initial + 1.0
    );
}

#[test]
fn notes_font_preferences_report_invalid_data_and_failed_writes_without_losing_notes() {
    let (temp, mut app, _root, _target) = fixture();
    let preferences = temp.path().join("recovery/preferences.json");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    open_notes(&mut app, &ctx, "Popup target");
    replace_text(&mut app, &ctx, "Retained draft");
    std::fs::create_dir(&preferences).unwrap();
    let output = frame(&mut app, &ctx, vec![]);
    click(
        &mut app,
        &ctx,
        position(&output, "+"),
        egui::PointerButton::Primary,
    );
    assert!(
        app.error
            .as_ref()
            .unwrap()
            .contains("Could not save Notes font size")
    );
    assert!(app.dialog.is_some());
    assert!(app.notes_font_size.is_none());
    position(&frame(&mut app, &ctx, vec![]), "Retained draft");
    drop(app);
    std::fs::remove_dir(&preferences).unwrap();
    for data in [
        r#"{"notes_font_size":999}"#,
        r#"{"notes_font_size":0}"#,
        "{invalid",
    ] {
        std::fs::write(&preferences, data).unwrap();
        let app = NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap())
            .unwrap();
        assert!(
            app.error
                .as_ref()
                .unwrap()
                .contains("Could not restore Notes font size")
        );
        assert!(app.notes_font_size.is_none());
        assert_eq!(std::fs::read_to_string(&preferences).unwrap(), data);
    }
}

#[test]
fn f4_focuses_selected_branch_and_f5_restores_the_map_without_editing() {
    let (_temp, mut app, _root, target) = fixture();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let viewport = app.workspace.map().viewport;
    key(&mut app, &ctx, Key::F4, egui::Modifiers::NONE);
    assert_eq!(
        app.workspace.map().branch_focus.as_ref().unwrap().root,
        target
    );
    key(&mut app, &ctx, Key::F5, egui::Modifiers::NONE);
    assert!(app.workspace.map().branch_focus.is_none());
    assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
    assert_eq!(app.workspace.map().viewport.zoom, viewport.zoom);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
    app.action(EditorAction::Edit);
    frame(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F4, egui::Modifiers::NONE);
    assert!(app.workspace.map().branch_focus.is_none());
    assert!(app.workspace.map().editor.editing.is_some());
}

#[test]
fn f3_opens_selected_topic_notes_without_intercepting_text_editing() {
    let (_temp, mut app, _root, target) = fixture();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F3, egui::Modifiers::NONE);
    assert!(matches!(app.dialog, Some(Dialog::Notes(_))));
    position(&frame(&mut app, &ctx, vec![]), "Existing notes");
    key(&mut app, &ctx, Key::Escape, egui::Modifiers::NONE);
    frame(&mut app, &ctx, vec![]);
    app.action(EditorAction::Edit);
    frame(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F3, egui::Modifiers::NONE);
    assert!(app.dialog.is_none());
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().id,
        target
    );
}

#[test]
fn notes_popup_stays_in_place_when_typing_and_scrolling_long_notes() {
    for size in [egui::vec2(1280.0, 840.0), egui::vec2(850.0, 560.0)] {
        let (_temp, mut app, _root, _target) = fixture();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        open_notes(&mut app, &ctx, "Popup target");
        replace_text(&mut app, &ctx, "");
        for _ in 0..3 {
            frame_at_size(&mut app, &ctx, vec![], size);
        }
        let before = frame_at_size(&mut app, &ctx, vec![], size);
        let controls = ["fn", "Apply", "Cancel"];
        let session = app.workspace.active_id();
        let target = app.workspace.map().editor.selected.clone();
        let input = ctx
            .read_response(Id::unique(("notes-popup", session, &target)))
            .unwrap();
        let span = position(&before, "Apply").y - position(&before, "Notes").y;
        let previous_span = if size.y > 600.0 { 491.7 } else { 357.3 };
        assert!(
            (0.65..=0.75).contains(&(span / previous_span)),
            "popup span: {span}"
        );
        assert!(
            input.rect.height() >= span * 0.65,
            "input must fill the recovered space"
        );
        assert!(!before.shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Preview"
        )));
        let positions = controls.map(|label| position(&before, label));
        for event in [
            egui::Event::Text("x".into()),
            egui::Event::Paste(
                "A long note that wraps across the editor and preview.\n".repeat(60),
            ),
        ] {
            frame_at_size(&mut app, &ctx, vec![event], size);
            for _ in 0..3 {
                let output = frame_at_size(&mut app, &ctx, vec![], size);
                for (label, before) in controls.into_iter().zip(positions) {
                    let after = position(&output, label);
                    assert!(
                        before.distance(after) <= 1.0,
                        "{label} moved from {before:?} to {after:?} while typing at {size:?}"
                    );
                    assert!(egui::Rect::from_min_size(Pos2::ZERO, size).contains(after));
                }
            }
        }
    }
}

#[test]
fn notes_popup_targets_the_clicked_node_and_preserves_view_and_unrelated_drafts() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let (_temp, mut app, root, target) = fixture();
        app.edit(EditorEdit::Select(root.clone())).unwrap();
        app.edit(EditorEdit::Action(EditorAction::Edit)).unwrap();
        app.edit(EditorEdit::SetDraft {
            field: DraftField::Inline(root.clone()),
            text: "Unfinished title".into(),
        })
        .unwrap();
        app.edit(EditorEdit::BeginDetails(root.clone())).unwrap();
        app.edit(EditorEdit::SetDraft {
            field: DraftField::Notes(root.clone()),
            text: "Unfinished other notes".into(),
        })
        .unwrap();
        app.workspace.map_mut().notes_open = true;
        app.workspace.map_mut().viewport.zoom = 0.85;
        app.workspace.map_mut().viewport.pan = Point { x: -40.0, y: 19.0 };
        let before = app.workspace.map().editor.document.clone();
        let generation = app.workspace.map().editor.history.generation;
        let viewport = app.workspace.map().viewport;
        let ctx = egui::Context::default();
        crate::desktop::configure_context(&ctx);
        frame(&mut app, &ctx, vec![]);
        ctx.set_theme(theme);
        open_notes(&mut app, &ctx, "Popup target");
        let output = frame(&mut app, &ctx, vec![]);
        position(&output, "Existing notes");
        replace_text(&mut app, &ctx, "**New notes**\n\n- First\n- Second");
        assert_eq!(app.workspace.map().editor.document, before);
        let output = frame(&mut app, &ctx, vec![]);
        let apply = position(&output, "Apply");
        click(&mut app, &ctx, apply, egui::PointerButton::Primary);
        frame(&mut app, &ctx, vec![]);
        assert!(app.dialog.is_none());
        let editor = &app.workspace.map().editor;
        assert_eq!(
            editor.document.nodes[&target].notes,
            "**New notes**\n\n- First\n- Second"
        );
        assert_eq!(editor.document.nodes[&target].link, "https://example.com");
        assert_eq!(editor.document.nodes[&root], before.nodes[&root]);
        assert_eq!(editor.editing.as_ref().unwrap().text, "Unfinished title");
        assert_eq!(
            editor.details.as_ref().unwrap().notes,
            "Unfinished other notes"
        );
        assert_eq!(editor.history.generation, generation + 1);
        assert!(app.workspace.map().notes_open);
        assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
        assert_eq!(app.workspace.map().viewport.zoom, viewport.zoom);
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(Id::unique(("idea", app.workspace.active_id(), &target)))
        );
        app.action(EditorAction::Undo);
        assert_eq!(
            app.workspace.map().editor.document.nodes[&target].notes,
            before.nodes[&target].notes
        );
        assert_eq!(
            app.workspace.map().editor.editing.as_ref().unwrap().text,
            "Unfinished title"
        );
        assert_eq!(
            app.workspace.map().editor.details.as_ref().unwrap().notes,
            "Unfinished other notes"
        );
        app.action(EditorAction::Redo);
        assert_eq!(
            app.workspace.map().editor.document.nodes[&target].notes,
            "**New notes**\n\n- First\n- Second"
        );
        assert_eq!(
            app.workspace.map().editor.editing.as_ref().unwrap().text,
            "Unfinished title"
        );
    }
}

#[test]
fn notes_popup_cancel_preserves_an_existing_notes_draft_and_escape_only_closes_popup() {
    for escape in [true, false] {
        let (_temp, mut app, _root, target) = fixture();
        app.edit(EditorEdit::BeginDetails(target.clone())).unwrap();
        app.edit(EditorEdit::SetDraft {
            field: DraftField::Notes(target.clone()),
            text: "Unfinished notes".into(),
        })
        .unwrap();
        app.edit(EditorEdit::SetDraft {
            field: DraftField::Link(target.clone()),
            text: "https://draft.example".into(),
        })
        .unwrap();
        let before = app.workspace.map().editor.document.clone();
        let generation = app.workspace.map().editor.history.generation;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        open_notes(&mut app, &ctx, "Popup target");
        position(&frame(&mut app, &ctx, vec![]), "Unfinished notes");
        replace_text(&mut app, &ctx, "Canceled notes");
        if escape {
            key(&mut app, &ctx, Key::Escape, egui::Modifiers::NONE);
        } else {
            let output = frame(&mut app, &ctx, vec![]);
            click(
                &mut app,
                &ctx,
                position(&output, "Cancel"),
                egui::PointerButton::Primary,
            );
        }
        assert!(app.dialog.is_none());
        assert_eq!(app.workspace.map().editor.document, before);
        assert_eq!(app.workspace.map().editor.history.generation, generation);
        assert_eq!(
            app.workspace.map().editor.details.as_ref().unwrap().notes,
            "Unfinished notes"
        );
        assert_eq!(
            app.workspace.map().editor.details.as_ref().unwrap().link,
            "https://draft.example"
        );
        assert!(!app.workspace.map().notes_open);
    }
}

#[test]
fn notes_popup_apply_is_one_undo_step() {
    let (_temp, mut app, _root, target) = fixture();
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    open_notes(&mut app, &ctx, "Popup target");
    replace_text(&mut app, &ctx, "**Important**\n\n- First\n- Second");
    let output = frame(&mut app, &ctx, vec![]);
    position(&output, "**Important**\n\n- First\n- Second");
    key(
        &mut app,
        &ctx,
        Key::Enter,
        egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        },
    );
    assert!(app.dialog.is_none());
    assert_eq!(
        app.workspace.map().editor.document.nodes[&target].notes,
        "**Important**\n\n- First\n- Second"
    );
    assert_eq!(
        app.workspace.map().editor.history.generation,
        generation + 1
    );
    app.action(EditorAction::Undo);
    assert_eq!(app.workspace.map().editor.document, before);
}

#[test]
fn notes_popup_keeps_typed_text_and_newlines_in_event_order() {
    let (_temp, mut app, _root, target) = fixture();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    open_notes(&mut app, &ctx, "Popup target");
    replace_text(&mut app, &ctx, "First");
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text(" next".into()),
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("Second".into()),
        ],
    );
    let output = frame(&mut app, &ctx, vec![]);
    click(
        &mut app,
        &ctx,
        position(&output, "Apply"),
        egui::PointerButton::Primary,
    );
    assert_eq!(
        app.workspace.map().editor.document.nodes[&target].notes,
        "First next\nSecond"
    );
}

#[test]
fn notes_popup_ime_confirmation_does_not_apply_or_close_the_dialog() {
    let (_temp, mut app, _root, target) = fixture();
    let before = app.workspace.map().editor.document.clone();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    open_notes(&mut app, &ctx, "Popup target");
    replace_text(&mut app, &ctx, "");
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "研究".into(),
            active_range_chars: None,
        })],
    );
    key(&mut app, &ctx, Key::Enter, egui::Modifiers::COMMAND);
    assert!(app.dialog.is_some());
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Ime(egui::ImeEvent::Commit("研究".into())),
            egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
        ],
    );
    assert!(app.dialog.is_some());
    assert_eq!(app.workspace.map().editor.document, before);
    let output = frame(&mut app, &ctx, vec![]);
    click(
        &mut app,
        &ctx,
        position(&output, "Apply"),
        egui::PointerButton::Primary,
    );
    assert!(app.dialog.is_none());
    assert!(
        app.workspace.map().editor.document.nodes[&target]
            .notes
            .contains("研究")
    );
}

#[test]
fn notes_popup_rejects_oversized_utf8_paste_without_losing_previous_input() {
    let (_temp, mut app, _root, _target) = fixture();
    let before = app.workspace.map().editor.document.clone();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    open_notes(&mut app, &ctx, "Popup target");
    replace_text(&mut app, &ctx, "Keep this");
    replace_text(&mut app, &ctx, &"é".repeat(MAX_FIELD_BYTES / 2 + 1));
    let output = frame(&mut app, &ctx, vec![]);
    position(&output, "Keep this");
    assert!(app.error.is_some());
    assert!(app.dialog.is_some());
    assert_eq!(app.workspace.map().editor.document, before);
}

use branchline::{
    app::{Direction, Editor, EditorAction},
    storage::{RecoveryRecord, RecoveryStore},
    ui::NativeApp,
};
use eframe::egui;
use std::time::Duration;

fn render(app: &mut NativeApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.show(ui));
    output.textures_delta.clear();
}

fn key(app: &mut NativeApp, ctx: &egui::Context, key: egui::Key) {
    for pressed in [true, false] {
        render(
            app,
            ctx,
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
}

#[test]
fn inline_creation_keeps_editing_reachable_beyond_the_viewport() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    for _ in 0..14 {
        key(&mut app, &ctx, egui::Key::Tab);
    }
    assert_eq!(app.editor.document.nodes.len(), 15);
    key(&mut app, &ctx, egui::Key::Enter);
    assert!(
        app.editor.editing.is_none(),
        "The active inline editor must remain reachable"
    );
}

#[test]
fn folded_root_cannot_navigate_into_invisible_children() {
    let mut editor = Editor::new();
    editor.dispatch(EditorAction::AddChild).unwrap();
    let child = editor.selected.clone();
    editor.commit_edit().unwrap();
    let root = editor.document.root.clone();
    editor.select(&root).unwrap();
    editor.dispatch(EditorAction::Fold).unwrap();
    editor
        .dispatch(EditorAction::Navigate(Direction::Right))
        .unwrap();
    assert_eq!(editor.selected, root);
    editor
        .execute(branchline::document::Command::SetNotes {
            id: child,
            notes: String::new(),
        })
        .unwrap();
    assert_eq!(
        editor.selected, root,
        "No-op commands must also preserve visible selection"
    );
}

#[test]
fn undoing_reveal_selects_a_visible_ancestor() {
    let mut editor = Editor::new();
    editor.dispatch(EditorAction::AddChild).unwrap();
    let parent = editor.selected.clone();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.commit_edit().unwrap();
    let hidden = editor.selected.clone();
    editor.select(&parent).unwrap();
    editor.dispatch(EditorAction::Fold).unwrap();
    editor.reveal_selection(&hidden).unwrap();
    editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(editor.selected, parent);
    assert!(editor.document.nodes[&parent].folded);
}

#[test]
fn abandoning_ime_composition_releases_map_keyboard_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, egui::Key::F2);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "idea".into(),
            active_range_chars: Some(0..4),
        })],
    );
    let pos = egui::pos2(40.0, 250.0);
    for pressed in [true, false] {
        render(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert!(app.editor.editing.is_none());
    key(&mut app, &ctx, egui::Key::Tab);
    assert_eq!(
        app.editor.document.nodes.len(),
        2,
        "IME state must not outlive its widget"
    );
}

#[test]
fn cancelled_draft_is_removed_from_debounced_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, egui::Key::F2);
    app.editor.editing.as_mut().unwrap().text = "Canceled draft".into();
    std::thread::sleep(Duration::from_millis(750));
    render(&mut app, &ctx, vec![]);
    let path = temp.path().join("recovery.json");
    let record: RecoveryRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        record.document.nodes[&record.document.root].text,
        "Canceled draft"
    );
    key(&mut app, &ctx, egui::Key::Escape);
    assert_eq!(
        app.editor.document.nodes[&app.editor.document.root].text,
        "Central idea"
    );
    std::thread::sleep(Duration::from_millis(750));
    render(&mut app, &ctx, vec![]);
    assert!(
        !path.exists(),
        "An explicitly canceled draft must not be restored later"
    );
}

#[test]
fn palette_ime_confirmation_does_not_dispatch_a_map_command() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    };
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key: egui::Key::K,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
    );
    render(
        &mut app,
        &ctx,
        vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
    );
    render(&mut app, &ctx, vec![egui::Event::Text("Add child".into())]);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "idea".into(),
            active_range_chars: Some(0..4),
        })],
    );
    key(&mut app, &ctx, egui::Key::Enter);
    assert_eq!(
        app.editor.document.nodes.len(),
        1,
        "IME confirmation belongs to the text field, not the command palette"
    );
}

use branchline::{
    app::{DetailsDraft, EditorAction},
    storage::RecoveryStore,
    ui::NativeApp,
};
use eframe::egui::{self, Event, Key, Modifiers};

fn render(app: &mut NativeApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<egui::Shape> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    output.shapes.into_iter().map(|shape| shape.shape).collect()
}

fn key(app: &mut NativeApp, ctx: &egui::Context, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        render(
            app,
            ctx,
            vec![
                Event::ModifiersChanged(modifiers),
                Event::Key {
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

fn command() -> Modifiers {
    Modifiers {
        command: true,
        mac_cmd: true,
        ..Default::default()
    }
}

#[test]
fn new_tab_retains_source_drafts_and_view() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let source = app.workspace.active_id();
    let map = app.workspace.map_mut();
    map.editor.dispatch(EditorAction::Edit).unwrap();
    map.editor.editing.as_mut().unwrap().text = "Source draft".into();
    map.editor.details = Some(DetailsDraft {
        id: map.editor.selected.clone(),
        notes: "Source notes".into(),
        link: String::new(),
    });
    map.viewport.pan.x = 137.0;
    map.viewport.zoom = 0.8;
    map.search.query = "source search".into();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::N, command());
    assert_eq!(app.workspace.len(), 2);
    assert_ne!(app.workspace.active_id(), source);
    let old = app.workspace.session(source).unwrap();
    assert_eq!(old.editor.editing.as_ref().unwrap().text, "Source draft");
    assert_eq!(old.editor.details.as_ref().unwrap().notes, "Source notes");
    assert_eq!(old.search.query, "source search");
    assert_eq!(old.viewport.pan.x, 137.0);
    assert_eq!(old.viewport.zoom, 0.8);
    assert!(app.workspace.active().unwrap().editor.editing.is_none());
}

#[test]
fn equal_node_ids_and_revisions_are_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let first = app.workspace.active_id();
    let root = app.workspace.map().editor.document.root.clone();
    app.workspace
        .map_mut()
        .editor
        .execute(branchline::document::Command::SetText {
            id: root.clone(),
            text: "First loaded".into(),
        })
        .unwrap();
    let mut duplicate = app.workspace.active().unwrap().editor.document.clone();
    duplicate.nodes.get_mut(&root).unwrap().text = "Second loaded".into();
    let ctx = egui::Context::default();
    let shapes = render(&mut app, &ctx, vec![]);
    fn contains(shapes: &[egui::Shape], label: &str) -> bool {
        shapes.iter().any(|shape| match shape {
            egui::Shape::Text(text) => text.galley.job.text == label,
            egui::Shape::Vec(shapes) => contains(shapes, label),
            _ => false,
        })
    }
    assert!(contains(&shapes, "First loaded"));
    key(&mut app, &ctx, Key::N, command());
    let second = app.workspace.active_id();
    app.workspace
        .map_mut()
        .editor
        .replace_document(duplicate, None, true, None)
        .unwrap();
    let shapes = render(&mut app, &ctx, vec![]);
    assert_eq!(
        app.workspace.session(first).unwrap().editor.revision,
        app.workspace.session(second).unwrap().editor.revision
    );
    assert!(contains(&shapes, "Second loaded"));
    assert!(!contains(&shapes, "First loaded"));
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("Second map".into())]);
    key(&mut app, &ctx, Key::Enter, Modifiers::NONE);
    assert_eq!(
        app.workspace
            .session(second)
            .unwrap()
            .editor
            .document
            .nodes
            .values()
            .next()
            .unwrap()
            .text,
        "Second map"
    );
    assert_eq!(
        app.workspace
            .session(first)
            .unwrap()
            .editor
            .document
            .nodes
            .values()
            .next()
            .unwrap()
            .text,
        "First loaded"
    );
    key(
        &mut app,
        &ctx,
        Key::Tab,
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    );
    assert_eq!(app.workspace.active_id(), first);
    assert!(ctx.memory(|memory| memory.focused()).is_none());
    assert!(app.workspace.map().editor.editing.is_none());
}

#[test]
fn late_input_does_not_follow_a_tab_switch() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::N, command());
    let before = app.workspace.map().editor.document.clone();
    render(&mut app, &ctx, vec![Event::Text("Late source text".into())]);
    render(
        &mut app,
        &ctx,
        vec![Event::Ime(egui::ImeEvent::Commit("Late source IME".into()))],
    );
    assert!(app.workspace.map().editor.editing.is_none());
    assert_eq!(app.workspace.map().editor.document, before);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("Destination edit".into())]);
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().text,
        "Destination edit"
    );
}

#[test]
fn pointer_tab_switch_preserves_inline_draft_instead_of_committing_on_blur() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::N, command());
    let source = app.workspace.active_id();
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(
        &mut app,
        &ctx,
        vec![Event::Text("Do not commit this".into())],
    );
    let before = app.workspace.map().editor.document.clone();
    let shapes = render(&mut app, &ctx, vec![]);
    fn find(shapes: &[egui::Shape]) -> Option<egui::Pos2> {
        shapes.iter().find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.job.text == "Untitled 1" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(shapes) => find(shapes),
            _ => None,
        })
    }
    let pos = find(&shapes).expect("first tab is visible");
    for pressed in [true, false] {
        render(
            &mut app,
            &ctx,
            vec![
                Event::PointerMoved(pos),
                Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
    }
    assert_ne!(app.workspace.active_id(), source);
    let old = app.workspace.session(source).unwrap();
    assert_eq!(old.editor.document, before);
    assert_eq!(
        old.editor.editing.as_ref().unwrap().text,
        "Do not commit this"
    );
}

#[test]
fn composing_source_finishes_before_queued_tab_switch() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let source = app.workspace.active_id();
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(
        &mut app,
        &ctx,
        vec![Event::Ime(egui::ImeEvent::Preedit {
            text: "\u{65b0}".into(),
            active_range_chars: Some(0..1),
        })],
    );
    key(&mut app, &ctx, Key::N, command());
    assert_eq!(app.workspace.len(), 2);
    assert_eq!(app.workspace.active_id(), source);
    assert!(app.workspace.pending_activation().is_some());
    render(
        &mut app,
        &ctx,
        vec![
            Event::ModifiersChanged(Modifiers::NONE),
            Event::Ime(egui::ImeEvent::Commit("\u{65b0}".into())),
        ],
    );
    assert_ne!(app.workspace.active_id(), source);
    assert_eq!(
        app.workspace
            .session(source)
            .unwrap()
            .editor
            .editing
            .as_ref()
            .unwrap()
            .text,
        "\u{65b0}"
    );
    assert!(app.workspace.map().editor.editing.is_none());
}

#[test]
fn closing_a_clean_tab_releases_its_text_widget_state() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let old = app.workspace.active_id();
    let root = app.workspace.map().editor.selected.clone();
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    let widget = egui::Id::unique(("inline", old, &root));
    assert!(egui::TextEdit::load_state(&ctx, widget).is_some());
    key(&mut app, &ctx, Key::Escape, Modifiers::NONE);
    key(&mut app, &ctx, Key::W, command());
    assert_eq!(app.workspace.len(), 1);
    assert_ne!(app.workspace.active_id(), old);
    assert!(app.workspace.session(old).is_err());
    assert!(egui::TextEdit::load_state(&ctx, widget).is_none());
}

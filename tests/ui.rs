use branchline::{
    app::{Editor, EditorAction},
    document::Command,
    layout::{Point, Rect, Size},
    session::Viewport,
    ui::*,
};

#[test]
fn palette_filtering_dispatches_real_actions() {
    let commands = palette_matches("child");
    assert_eq!(commands.len(), 1);
    let mut editor = Editor::new();
    editor
        .dispatch(commands[0].action.editor_action().unwrap())
        .unwrap();
    assert_eq!(editor.document.nodes.len(), 2);
    assert!(palette_matches("does-not-exist").is_empty());
}

#[test]
fn search_reveals_ancestors() {
    let mut editor = Editor::new();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.commit_edit().unwrap();
    let parent = editor.selected.clone();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.editing.as_mut().unwrap().text = "Hidden research".into();
    editor.commit_edit().unwrap();
    let found = editor.selected.clone();
    editor
        .execute(Command::ToggleFold { id: parent.clone() })
        .unwrap();
    assert_eq!(editor.search("RESEARCH"), vec![found.clone()]);
    editor.reveal_selection(&found).unwrap();
    assert!(!editor.document.nodes[&parent].folded);
    assert_eq!(editor.selected, found);
    editor.dispatch(EditorAction::Undo).unwrap();
    assert!(editor.document.nodes[&parent].folded);
}

#[test]
fn zoom_preserves_anchor_and_fit_contains_bounds() {
    let mut viewport = Viewport::default();
    let center = Point { x: 500.0, y: 300.0 };
    let pointer = Point { x: 800.0, y: 400.0 };
    let world = viewport.to_world(pointer, center);
    viewport.zoom_at(pointer, center, 2.0);
    let mapped = viewport.to_screen(world, center);
    assert!((mapped.x - pointer.x).abs() < 0.01);
    assert!((mapped.y - pointer.y).abs() < 0.01);
    let bounds = Rect {
        min: Point {
            x: -1000.0,
            y: -200.0,
        },
        max: Point {
            x: 1000.0,
            y: 200.0,
        },
    };
    viewport.fit(
        bounds,
        Size {
            width: 1000.0,
            height: 600.0,
        },
    );
    assert!(viewport.to_screen(bounds.min, center).x >= 20.0);
    assert!(viewport.to_screen(bounds.max, center).x <= 980.0);
    viewport.focus(Point {
        x: 300.0,
        y: -100.0,
    });
    assert_eq!(
        viewport.to_screen(
            Point {
                x: 300.0,
                y: -100.0
            },
            center
        ),
        center
    );
}

#[test]
fn real_ui_renders_and_keeps_auxiliary_typing_out_of_canvas() {
    use eframe::egui;
    let temp = tempfile::tempdir().unwrap();
    let store = branchline::recovery::RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut app = NativeApp::new(store).unwrap();
    let context = egui::Context::default();
    let mut output = context.run_ui(egui::RawInput::default(), |ui| app.show(ui));
    assert!(!output.shapes.is_empty());
    output.textures_delta.clear();
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 1);
    let frame = |app: &mut NativeApp, key: Option<egui::Key>, command: bool| {
        let modifiers = egui::Modifiers {
            command,
            mac_cmd: command,
            ..Default::default()
        };
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::ModifiersChanged(modifiers));
        if let Some(key) = key {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
        }

        let mut output = context.run_ui(input, |ui| app.show(ui));
        output.textures_delta.clear();
    };
    context.memory_mut(|memory| {
        memory.request_focus(egui::Id::unique((
            "idea",
            app.workspace.active_id(),
            &app.workspace.map().editor.selected,
        )))
    });
    frame(&mut app, Some(egui::Key::Tab), false);
    frame(&mut app, None, false);
    assert_eq!(app.workspace.map().editor.document.nodes.len(), 2);
    frame(&mut app, Some(egui::Key::Enter), false);
    assert!(app.workspace.map().editor.editing.is_none());
    assert_eq!(
        app.workspace.map().editor.document.nodes.len(),
        2,
        "Commit must not also create a sibling"
    );
    frame(&mut app, Some(egui::Key::F), true);
    frame(&mut app, None, false);
    frame(&mut app, Some(egui::Key::Tab), false);
    assert_eq!(
        app.workspace.map().editor.document.nodes.len(),
        2,
        "Search text field owns Tab"
    );
}

#[test]
fn recovery_keeps_uncommitted_inline_text() {
    let temp = tempfile::tempdir().unwrap();
    let store = branchline::recovery::RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut app = NativeApp::new(store).unwrap();
    app.workspace
        .map_mut()
        .editor
        .dispatch(EditorAction::Edit)
        .unwrap();
    app.workspace
        .map_mut()
        .editor
        .editing
        .as_mut()
        .unwrap()
        .text = "Still typing 研究".into();
    app.checkpoint_recovery().unwrap();
    assert_eq!(
        app.workspace.map().editor.document.nodes[&app.workspace.map().editor.document.root].text,
        "Central idea"
    );
    assert!(app.workspace.map().editor.editing.is_some());
    drop(app);
    let restore = branchline::recovery::RecoveryCoordinator::open(temp.path().into())
        .unwrap()
        .load()
        .unwrap();
    let branchline::recovery::RestoreContent::Recovered(document) = &restore.sessions[0].content
    else {
        panic!("inline recovery");
    };
    assert_eq!(document.nodes[&document.root].text, "Still typing 研究");
}

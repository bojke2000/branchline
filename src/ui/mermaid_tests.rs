use super::*;

fn application() -> (tempfile::TempDir, NativeApp) {
    let temp = tempfile::tempdir().unwrap();
    let app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    (temp, app)
}

#[test]
fn mermaid_export_uses_uncommitted_session_snapshot() {
    let (temp, mut app) = application();
    let source = app.workspace.active_id();
    let native = temp.path().join("source.branchline");
    app.workspace.save(source, &native).unwrap();
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
        .text = "Captured draft".into();
    app.workspace.map_mut().editor.details = Some(crate::app::DetailsDraft {
        id: app.workspace.map().editor.selected.clone(),
        notes: "Unfinished notes".into(),
        link: String::new(),
    });
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let dirty = app.workspace.map().editor.is_dirty();
    let ctx = egui::Context::default();
    app.run_command(AppCommand::ExportMermaid, &ctx);
    let Some(Dialog::MermaidExport { session, bytes }) = app.dialog.take() else {
        panic!("missing export scope preview")
    };
    assert_eq!(session, source);
    assert!(String::from_utf8_lossy(&bytes).contains("Captured draft"));
    let second = app.workspace.new_map().unwrap();
    app.workspace.apply_pending_activation().unwrap();
    assert_eq!(app.workspace.active_id(), second);
    let output = temp.path().join("captured.mmd");
    app.stage_export(session, Some(output.clone()), bytes, &ctx);
    assert!(
        std::fs::read_to_string(output)
            .unwrap()
            .contains("Captured draft")
    );
    let editor = &app.workspace.session(source).unwrap().editor;
    assert_eq!(editor.document, before);
    assert_eq!(editor.history.generation, generation);
    assert_eq!(editor.path.as_ref(), Some(&native));
    assert_eq!(editor.editing.as_ref().unwrap().text, "Captured draft");
    assert_eq!(editor.is_dirty(), dirty);
    assert_eq!(editor.details.as_ref().unwrap().notes, "Unfinished notes");
    app.workspace.request_activate(source).unwrap();
    app.workspace.apply_pending_activation().unwrap();
    app.workspace
        .map_mut()
        .editor
        .editing
        .as_mut()
        .unwrap()
        .text = "\0".into();
    app.run_command(AppCommand::ExportMermaid, &ctx);
    assert!(app.dialog.is_none());
    assert!(app.error.take().unwrap().contains("label"));
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
}

#[test]
fn mermaid_export_does_not_replace_or_save_the_map() {
    let (temp, mut app) = application();
    let source = app.workspace.active_id();
    let native = temp.path().join("source.branchline");
    app.workspace.save(source, &native).unwrap();
    let original_bytes = std::fs::read(&native).unwrap();
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let ctx = egui::Context::default();
    let bytes = crate::mermaid::export_mermaid(&before).unwrap();
    app.stage_export(source, None, bytes.clone(), &ctx);
    assert!(app.dialog.is_none());
    assert!(!app.status.starts_with("Exported"));
    let existing = temp.path().join("existing.mmd");
    std::fs::write(&existing, b"untouched").unwrap();
    app.stage_export(source, Some(existing.clone()), bytes.clone(), &ctx);
    assert!(matches!(app.dialog, Some(Dialog::Overwrite { .. })));
    // Reject the staged overwrite through the real modal's Cancel button.
    click_dialog_cancel(&mut app, &ctx);
    assert_eq!(std::fs::read(&existing).unwrap(), b"untouched");
    app.stage_export(
        source,
        Some(temp.path().join("missing/export.mmd")),
        bytes.clone(),
        &ctx,
    );
    assert!(app.error.take().is_some());
    let second = app.workspace.new_map().unwrap();
    app.workspace.apply_pending_activation().unwrap();
    for destination in [native.clone(), temp.path().join("alias.branchline")] {
        if destination != native {
            std::fs::hard_link(&native, &destination).unwrap();
        }
        app.publish(second, destination, Some(bytes.clone()), &ctx);
        assert!(app.error.take().unwrap().contains("open native"));
    }
    assert_eq!(std::fs::read(&native).unwrap(), original_bytes);
    let editor = &app.workspace.session(source).unwrap().editor;
    assert_eq!(editor.document, before);
    assert_eq!(editor.history.generation, generation);
    assert!(!editor.is_dirty());
    assert_eq!(editor.path.as_ref(), Some(&native));
}

fn click_dialog_cancel(app: &mut NativeApp, ctx: &egui::Context) {
    fn frame(
        app: &mut NativeApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
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
        output.shapes
    }
    frame(app, ctx, vec![]);
    let shapes = frame(app, ctx, vec![]);
    let pos = shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Cancel" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("Cancel button");
    for pressed in [true, false] {
        frame(
            app,
            ctx,
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
    assert!(app.dialog.is_none());
}

use super::*;

fn frame(app: &mut NativeApp, context: &egui::Context) -> egui::FullOutput {
    context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1280.0, 840.0),
            )),
            ..Default::default()
        },
        |ui| app.show(ui),
    )
}

fn titles(output: &egui::FullOutput) -> Vec<String> {
    output.viewport_output[&egui::ViewportId::ROOT]
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::ViewportCommand::Title(title) => Some(title.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn idle_frames_do_not_republish_the_title_or_request_continuous_repaints() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let context = egui::Context::default();
    assert_eq!(titles(&frame(&mut app, &context)).len(), 1);
    for _ in 0..16 {
        assert!(titles(&frame(&mut app, &context)).is_empty());
    }
    let settled = frame(&mut app, &context);
    assert!(settled.viewport_output[&egui::ViewportId::ROOT].repaint_delay > Duration::ZERO);
}

#[test]
fn title_changes_still_reflect_save_paths_and_dirty_state() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let context = egui::Context::default();
    let _ = frame(&mut app, &context);
    let path = temp.path().join("Saved.branchline");
    app.workspace
        .save(app.workspace.active_id(), &path)
        .unwrap();
    assert_eq!(
        titles(&frame(&mut app, &context)),
        ["Saved.branchline \u{2014} BMM"]
    );
    assert!(titles(&frame(&mut app, &context)).is_empty());
    let root = app.workspace.map().editor.selected.clone();
    app.workspace
        .apply(
            app.workspace.active_id(),
            EditorEdit::Command(Command::SetText {
                id: root,
                text: "Changed".into(),
            }),
        )
        .unwrap();
    assert_eq!(
        titles(&frame(&mut app, &context)),
        ["* Saved.branchline \u{2014} BMM"],
    );
    app.workspace
        .save(app.workspace.active_id(), &path)
        .unwrap();
    assert_eq!(
        titles(&frame(&mut app, &context)),
        ["Saved.branchline \u{2014} BMM"]
    );
}

use super::*;

#[test]
fn rename_input_is_visible_and_accepts_text_in_both_themes() {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap())
                .unwrap();
        let id = app.workspace.active_id();
        app.workspace
            .save(id, &temp.path().join("Readable.branchline"))
            .unwrap();
        let ctx = egui::Context::default();
        crate::desktop::configure_context(&ctx);
        frame(&mut app, &ctx, vec![]);
        ctx.set_theme(theme);
        app.workspace
            .apply(id, EditorEdit::Action(EditorAction::Edit))
            .unwrap();
        let before = app.workspace.map().editor.document.clone();
        let draft = app
            .workspace
            .map()
            .editor
            .editing
            .as_ref()
            .unwrap()
            .text
            .clone();
        app.begin_rename(id, &ctx);
        for _ in 0..12 {
            frame(&mut app, &ctx, vec![]);
        }
        let output = frame(&mut app, &ctx, vec![]);
        let filename = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Readable" => {
                    Some((shape, text))
                }
                _ => None,
            })
            .expect("filename must be painted");
        assert!(
            filename
                .0
                .clip_rect
                .contains_rect(filename.1.galley.rect.translate(filename.1.pos.to_vec2()))
        );
        frame(&mut app, &ctx, vec![egui::Event::Text("X".into())]);
        let Some(Dialog::Rename { name, .. }) = &app.dialog else {
            panic!("rename dialog closed while typing");
        };
        assert!(
            name.contains('X'),
            "rename field did not receive typed text: {name}"
        );
        frame(&mut app, &ctx, vec![egui::Event::Paste("Pasted".into())]);
        let Some(Dialog::Rename { name, .. }) = &app.dialog else {
            panic!("rename dialog closed while pasting");
        };
        assert!(name.contains("Pasted"));
        let renamed = temp.path().join(format!("{name}.branchline"));
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.dialog.is_none(), "Enter must submit the rename");
        assert!(renamed.is_file());
        assert!(!temp.path().join("Readable.branchline").exists());
        assert_eq!(app.workspace.map().editor.path.as_ref(), Some(&renamed));
        assert_eq!(app.workspace.map().editor.document, before);
        assert_eq!(
            app.workspace.map().editor.editing.as_ref().unwrap().text,
            draft
        );
    }
}

#[test]
fn rename_targets_the_clicked_tab_and_checkpoints_its_new_path() {
    let temp = tempfile::tempdir().unwrap();
    let recovery = temp.path().join("recovery");
    let mut app = NativeApp::new(RecoveryCoordinator::open(recovery.clone()).unwrap()).unwrap();
    let id = app.workspace.active_id();
    let old = temp.path().join("old.branchline");
    let new = temp.path().join("new.branchline");
    app.workspace.save(id, &old).unwrap();
    let other = app.workspace.new_map().unwrap();
    app.workspace.apply_pending_activation().unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    let tab = text_position(&output, "old.branchline");
    click(&mut app, &ctx, tab, egui::PointerButton::Secondary);
    let output = frame(&mut app, &ctx, vec![]);
    let rename = text_position(&output, "Rename...");
    click(&mut app, &ctx, rename, egui::PointerButton::Primary);
    let Some(Dialog::Rename { session, name, .. }) = app.dialog.take() else {
        panic!("missing rename dialog");
    };
    assert_eq!(session, id);
    assert_eq!(name, "old");
    assert!(app.rename_session(session, "new"));
    assert_eq!(app.workspace.active_id(), other);
    assert_eq!(
        app.workspace.session(id).unwrap().editor.path.as_ref(),
        Some(&new)
    );
    assert!(!old.exists());
    drop(app);
    let restored = NativeApp::new(RecoveryCoordinator::open(recovery).unwrap()).unwrap();
    assert_eq!(
        restored.workspace.session(id).unwrap().editor.path.as_ref(),
        Some(&new)
    );
}

fn frame(app: &mut NativeApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1280.0, 840.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    output
}

fn text_position(output: &egui::FullOutput, label: &str) -> Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing UI text: {label}"))
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

fn replace_recovery_snapshot(directory: &std::path::Path) -> PathBuf {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("workspace.json")).unwrap()).unwrap();
    let id = manifest["tabs"][0]["snapshot"]["id"].as_str().unwrap();
    let path = directory.join(format!("snapshot-{id}.json"));
    std::fs::rename(&path, path.with_extension("held")).unwrap();
    std::fs::write(&path, b"foreign replacement").unwrap();
    path
}

fn edit_recovery_text(app: &mut NativeApp, text: &str) {
    let root = app.workspace.map().editor.document.root.clone();
    app.edit(EditorEdit::Command(Command::SetText {
        id: root,
        text: text.into(),
    }))
    .unwrap();
    app.checkpoint_recovery().unwrap();
}

fn dismiss_visible_error(app: &mut NativeApp) {
    let ctx = egui::Context::default();
    frame(app, &ctx, vec![]);
    let output = frame(app, &ctx, vec![]);
    let dismiss = text_position(&output, "Dismiss");
    click(app, &ctx, dismiss, egui::PointerButton::Primary);
}

#[test]
fn dismissed_cleanup_warning_stays_hidden_after_checkpoints_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    edit_recovery_text(&mut app, "Original");
    let replacement = replace_recovery_snapshot(temp.path());
    edit_recovery_text(&mut app, "Current");
    assert!(app.error.as_ref().unwrap().contains("identity changed"));
    dismiss_visible_error(&mut app);
    assert!(app.error.is_none());
    edit_recovery_text(&mut app, "Next edit");
    assert!(app.error.is_none(), "dismissed cleanup warning returned");
    drop(app);

    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    assert!(app.error.is_none(), "dismissal did not survive restart");
    edit_recovery_text(&mut app, "After restart");
    assert!(app.error.is_none());
    assert_eq!(std::fs::read(replacement).unwrap(), b"foreign replacement");
    app.error = Some("An unrelated error".into());
    app.checkpoint_recovery().unwrap();
    assert_eq!(app.error.as_deref(), Some("An unrelated error"));

    let new_replacement = replace_recovery_snapshot(temp.path());
    edit_recovery_text(&mut app, "New cleanup failure");
    assert!(
        app.error
            .as_ref()
            .unwrap()
            .contains(new_replacement.to_str().unwrap()),
        "new cleanup failures must still be shown"
    );
    drop(app);
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    assert!(
        app.error
            .as_ref()
            .unwrap()
            .contains(new_replacement.to_str().unwrap())
    );
    dismiss_visible_error(&mut app);
    drop(app);
    let app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    assert!(
        app.error.is_none(),
        "startup warnings must be dismissible too"
    );
}

#[test]
fn dismissing_cleanup_warning_does_not_disable_cleanup_retries() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    edit_recovery_text(&mut app, "Original");
    let replacement = replace_recovery_snapshot(temp.path());
    edit_recovery_text(&mut app, "Current");
    dismiss_visible_error(&mut app);
    std::fs::remove_file(&replacement).unwrap();
    std::fs::rename(replacement.with_extension("held"), &replacement).unwrap();
    app.checkpoint_recovery().unwrap();
    assert!(!replacement.exists(), "dismissal must not skip cleanup");
    assert!(app.error.is_none());
}

#[test]
fn cleanup_warning_dismissal_reports_persistence_failure() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    edit_recovery_text(&mut app, "Original");
    replace_recovery_snapshot(temp.path());
    edit_recovery_text(&mut app, "Current");
    std::fs::create_dir(temp.path().join("dismissed-recovery-warnings.json")).unwrap();
    dismiss_visible_error(&mut app);
    let error = app
        .error
        .as_ref()
        .expect("failed dismissal must be reported");
    assert!(error.contains("Could not remember recovery warning dismissal"));
    app.checkpoint_recovery().unwrap();
    assert!(
        app.error.is_some(),
        "failed persistence must not mute warnings"
    );
}

#[test]
fn dismissed_cleanup_warnings_stay_hidden_when_only_one_failure_remains() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    edit_recovery_text(&mut app, "First");
    let first = replace_recovery_snapshot(temp.path());
    edit_recovery_text(&mut app, "Second");
    let second = replace_recovery_snapshot(temp.path());
    edit_recovery_text(&mut app, "Third");
    let error = app.error.as_ref().unwrap();
    assert!(error.contains(first.to_str().unwrap()));
    assert!(error.contains(second.to_str().unwrap()));
    dismiss_visible_error(&mut app);

    std::fs::remove_file(&second).unwrap();
    std::fs::rename(second.with_extension("held"), &second).unwrap();
    app.checkpoint_recovery().unwrap();
    assert!(
        app.error.is_none(),
        "a previously dismissed failure returned"
    );
    assert!(!second.exists(), "resolved cleanup must still complete");
    drop(app);
    let app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    assert!(app.error.is_none());
    assert_eq!(std::fs::read(first).unwrap(), b"foreign replacement");
}

#[test]
fn rename_conflict_surfaces_error_without_changing_the_tab() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let id = app.workspace.active_id();
    let old = temp.path().join("old.branchline");
    app.workspace.save(id, &old).unwrap();
    std::fs::write(temp.path().join("existing.branchline"), b"keep").unwrap();
    assert!(!app.rename_session(id, "existing"));
    assert!(app.error.is_some());
    assert_eq!(app.workspace.map().editor.path.as_ref(), Some(&old));
    assert!(app.rename_session(id, "available"));
    assert!(app.error.is_none());
}

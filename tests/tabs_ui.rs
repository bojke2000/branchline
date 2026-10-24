use branchline::{
    app::{DetailsDraft, EditorAction},
    recovery::RecoveryCoordinator,
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
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
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
fn palette_preserves_drafts_for_mermaid_export_and_cancel() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(
        &mut app,
        &ctx,
        vec![Event::Text("Uncommitted label".into())],
    );
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    key(&mut app, &ctx, Key::K, command());
    render(&mut app, &ctx, vec![Event::Text("Export Mermaid".into())]);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().text,
        "Uncommitted label"
    );
    key(&mut app, &ctx, Key::Enter, Modifiers::NONE);
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().text,
        "Uncommitted label"
    );
}

#[test]
fn equal_node_ids_and_revisions_are_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
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
    app.workspace.map_mut().editor =
        branchline::app::Editor::from_document(duplicate, None, true, None).unwrap();
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
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
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
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
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
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
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
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
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

#[test]
fn import_cancel_adds_no_tab_and_retains_drafts() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::Tab, Modifiers::NONE);
    key(&mut app, &ctx, Key::Enter, Modifiers::NONE);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(
        &mut app,
        &ctx,
        vec![Event::Text("Keep original draft".into())],
    );
    let source = app.workspace.active_id();
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let report = || {
        branchline::interchange::import_mm(
            br#"<map><node TEXT="Imported"><node TEXT="Child"/></node></map>"#,
        )
        .unwrap()
    };
    fn position(shapes: &[egui::Shape], label: &str) -> Option<egui::Pos2> {
        shapes.iter().find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(children) => position(children, label),
            _ => None,
        })
    }
    for label in ["Cancel", "Accept supported data"] {
        app.stage_import(report()).unwrap();
        render(&mut app, &ctx, vec![]);
        let shapes = render(&mut app, &ctx, vec![]);
        let pos = position(&shapes, label).expect("import dialog action");
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
        if label == "Cancel" {
            assert_eq!(app.workspace.len(), 1);
        }
    }
    assert_eq!(app.workspace.len(), 2);
    assert_ne!(app.workspace.active_id(), source);
    let original = app.workspace.session(source).unwrap();
    assert_eq!(original.editor.document, before);
    assert_eq!(original.editor.history.generation, generation);
    assert_eq!(
        original.editor.editing.as_ref().unwrap().text,
        "Keep original draft"
    );
    assert!(app.workspace.map().editor.is_dirty());
    assert!(app.workspace.map().editor.path.is_none());
}

fn text_positions(shapes: &[egui::Shape], label: &str) -> Vec<egui::Pos2> {
    let mut positions = Vec::new();
    for shape in shapes {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                positions.push(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(children) => positions.extend(text_positions(children, label)),
            _ => {}
        }
    }
    positions
}

fn click_label(app: &mut NativeApp, ctx: &egui::Context, label: &str) {
    render(app, ctx, vec![]);
    let shapes = render(app, ctx, vec![]);
    let pos = *text_positions(&shapes, label).last().expect(label);
    click_at(app, ctx, pos);
}

fn click_at(app: &mut NativeApp, ctx: &egui::Context, pos: egui::Pos2) {
    for pressed in [true, false] {
        render(
            app,
            ctx,
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
}

#[test]
fn native_quit_cancel_on_later_tab_preserves_every_draft_and_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("First draft".into())]);
    let first = app.workspace.active_id();
    key(&mut app, &ctx, Key::N, command());
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("Second draft".into())]);
    let second = app.workspace.active_id();
    app.checkpoint_recovery().unwrap();
    key(&mut app, &ctx, Key::Q, command());
    click_label(&mut app, &ctx, "Discard changes");
    assert_eq!(app.workspace.order(), &[first, second]);
    click_label(&mut app, &ctx, "Cancel");
    assert_eq!(app.workspace.active_id(), second);
    assert_eq!(
        app.workspace
            .session(first)
            .unwrap()
            .editor
            .editing
            .as_ref()
            .unwrap()
            .text,
        "First draft"
    );
    assert_eq!(
        app.workspace
            .session(second)
            .unwrap()
            .editor
            .editing
            .as_ref()
            .unwrap()
            .text,
        "Second draft"
    );
    assert!(!app.workspace.transition_pending());
    drop(app);
    let restore = branchline::recovery::RecoveryCoordinator::open(temp.path().into())
        .unwrap()
        .load()
        .unwrap();
    assert_eq!(restore.sessions.len(), 2);
    assert!(
        restore
            .sessions
            .iter()
            .all(|session| session.tab.snapshot.is_some())
    );
}

#[test]
fn inactive_tab_close_button_targets_its_owner_without_committing_the_active_draft() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("Inactive draft".into())]);
    let first = app.workspace.active_id();
    let first_widget = egui::Id::unique(("inline", first, &app.workspace.map().editor.selected));
    key(&mut app, &ctx, Key::N, command());
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("Active draft".into())]);
    let active = app.workspace.active_id();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    let close = *text_positions(&shapes, "x")
        .first()
        .expect("inactive first tab close button");
    click_at(&mut app, &ctx, close);
    assert_eq!(app.workspace.active_id(), active);
    click_label(&mut app, &ctx, "Discard changes");
    assert_eq!(app.workspace.order(), &[active]);
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().text,
        "Active draft"
    );
    assert!(egui::TextEdit::load_state(&ctx, first_widget).is_none());
}

#[test]
fn successful_quit_does_not_republish_discarded_drafts_during_final_frame() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(
        &mut app,
        &ctx,
        vec![Event::Text("Explicitly discarded".into())],
    );
    app.checkpoint_recovery().unwrap();
    key(&mut app, &ctx, Key::Q, command());
    click_label(&mut app, &ctx, "Discard changes");
    render(&mut app, &ctx, vec![]);
    drop(app);
    let restore = RecoveryCoordinator::open(temp.path().into())
        .unwrap()
        .load()
        .unwrap();
    assert!(restore.sessions.is_empty());
    assert!(!restore.has_recovery());
}

#[test]
fn saving_during_quit_updates_the_same_file_without_replacement_confirmation() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let path = temp.path().join("saved.branchline");
    app.workspace
        .save(app.workspace.active_id(), &path)
        .unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(
        &mut app,
        &ctx,
        vec![Event::Text("Saved without another prompt".into())],
    );
    key(&mut app, &ctx, Key::Q, command());
    click_label(&mut app, &ctx, "Save");
    assert!(!app.workspace.transition_pending());
    let saved = branchline::interchange::read_native(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved.nodes[&saved.root].text,
        "Saved without another prompt"
    );
    assert!(app.workspace.map().editor.editing.is_none());
    assert!(!app.workspace.map().editor.is_dirty());
}

#[test]
fn ordinary_save_updates_its_file_without_a_replace_prompt() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let path = temp.path().join("saved.branchline");
    app.workspace
        .save(app.workspace.active_id(), &path)
        .unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("Updated directly".into())]);
    key(&mut app, &ctx, Key::S, command());
    let saved = branchline::interchange::read_native(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved.nodes[&saved.root].text, "Updated directly");
    assert_eq!(app.workspace.map().editor.path.as_ref(), Some(&path));
    assert!(!app.workspace.map().editor.is_dirty());
    assert!(app.workspace.map().editor.editing.is_none());
}

#[test]
fn ordinary_save_still_rejects_an_externally_replaced_file() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let path = temp.path().join("saved.branchline");
    app.workspace
        .save(app.workspace.active_id(), &path)
        .unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(&mut app, &ctx, vec![Event::Text("Retained draft".into())]);
    std::fs::rename(&path, temp.path().join("original.branchline")).unwrap();
    std::fs::write(&path, b"foreign replacement").unwrap();
    key(&mut app, &ctx, Key::S, command());
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign replacement");
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().text,
        "Retained draft"
    );
    assert!(app.workspace.map().editor.has_drafts().unwrap());
}

#[test]
fn final_native_save_failure_aborts_quit_without_committing_the_draft() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let parent = temp.path().join("maps");
    std::fs::create_dir(&parent).unwrap();
    let path = parent.join("saved.branchline");
    app.workspace
        .save(app.workspace.active_id(), &path)
        .unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    key(&mut app, &ctx, Key::F2, Modifiers::NONE);
    render(
        &mut app,
        &ctx,
        vec![Event::Text("Retain after save failure".into())],
    );
    std::fs::rename(&parent, temp.path().join("retained-maps")).unwrap();
    std::fs::write(&parent, b"foreign path obstacle").unwrap();
    key(&mut app, &ctx, Key::Q, command());
    click_label(&mut app, &ctx, "Save");
    assert!(!app.workspace.transition_pending());
    assert_eq!(app.workspace.len(), 1);
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().text,
        "Retain after save failure"
    );
    assert_eq!(std::fs::read(&parent).unwrap(), b"foreign path obstacle");
}

#[test]
fn closing_a_failed_saved_tab_uses_the_same_published_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().join("recovery")).unwrap();
    let mut workspace = branchline::workspace::Workspace::new();
    let old = workspace.active_id();
    let missing = temp.path().join("missing.branchline");
    workspace.save(old, &missing).unwrap();
    store.publish(&workspace.recovery_state()).unwrap();
    drop(store);
    std::fs::remove_file(&missing).unwrap();
    let mut app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    let ctx = egui::Context::default();
    assert!(app.workspace.active().is_err());
    click_label(&mut app, &ctx, "Close tab");
    assert_ne!(app.workspace.active_id(), old);
    assert!(app.workspace.map().is_pristine_blank());
    assert!(!app.workspace.transition_pending());
    drop(app);
    let restore = RecoveryCoordinator::open(temp.path().join("recovery"))
        .unwrap()
        .load()
        .unwrap();
    assert!(restore.sessions.iter().all(|session| session.tab.id != old));
}

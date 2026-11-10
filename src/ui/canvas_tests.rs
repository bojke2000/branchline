use super::*;

fn app() -> (tempfile::TempDir, NativeApp) {
    let temp = tempfile::tempdir().unwrap();
    let app =
        NativeApp::new(RecoveryCoordinator::open(temp.path().join("recovery")).unwrap()).unwrap();
    (temp, app)
}

fn topic(app: &mut NativeApp, parent: &str, text: &str) -> NodeId {
    app.workspace
        .map_mut()
        .editor
        .execute(Command::AddChild {
            parent: parent.into(),
            text: text.into(),
        })
        .unwrap();
    app.workspace.map().editor.selected.clone()
}

fn frame(app: &mut NativeApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(1200.0, 840.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.show(ui),
    );
    output.textures_delta.clear();
    output
}

fn label(output: &egui::FullOutput, value: &str) -> Option<Pos2> {
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
}

fn pointer(
    app: &mut NativeApp,
    ctx: &egui::Context,
    pos: Pos2,
    pressed: bool,
    modifiers: egui::Modifiers,
) {
    frame(
        app,
        ctx,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            },
        ],
    );
}

fn box_drag(
    app: &mut NativeApp,
    ctx: &egui::Context,
    from: Pos2,
    to: Pos2,
    modifiers: egui::Modifiers,
) {
    pointer(app, ctx, from, true, modifiers);
    frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(from + egui::vec2(12.0, 0.0))],
    );
    frame(app, ctx, vec![egui::Event::PointerMoved(to)]);
}

fn focus_branch(app: &mut NativeApp, ctx: &egui::Context) {
    let command = palette_matches("Focus branch")
        .into_iter()
        .find(|spec| spec.label == "Focus branch")
        .expect("branch focus is discoverable in the command palette");
    app.run_command(command.action, ctx);
}

#[test]
fn box_selection_replaces_or_adds_without_panning_and_escape_cancels() {
    let (_temp, mut app) = app();
    let root = app.workspace.map().editor.document.root.clone();
    let first = topic(&mut app, &root, "Marquee first");
    let outside = topic(&mut app, &root, "Outside rectangle");
    app.workspace
        .map_mut()
        .editor
        .execute(Command::AddSibling {
            sibling: first.clone(),
            text: "Marquee second".into(),
        })
        .unwrap();
    let second = app.workspace.map().editor.selected.clone();
    app.edit(EditorEdit::Select(outside.clone())).unwrap();
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let viewport = app.workspace.map().viewport;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    let a = label(&output, "Marquee first").unwrap();
    let b = label(&output, "Marquee second").unwrap();
    let from = Pos2::new(a.x.min(b.x) - 140.0, a.y.min(b.y) - 35.0);
    let to = Pos2::new(a.x.max(b.x) + 140.0, a.y.max(b.y) + 35.0);
    box_drag(&mut app, &ctx, from, to, egui::Modifiers::SHIFT);
    assert!(
        app.workspace.map().editor.is_selected(&outside),
        "selection commits only on release"
    );
    pointer(&mut app, &ctx, to, false, egui::Modifiers::SHIFT);
    assert_eq!(
        app.workspace.map().editor.selection(),
        &HashSet::from([first.clone(), second.clone()])
            .into_iter()
            .collect()
    );
    assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
    assert_eq!(app.workspace.map().viewport.zoom, viewport.zoom);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
    app.edit(EditorEdit::Select(outside.clone())).unwrap();
    let additive = egui::Modifiers {
        shift: true,
        ctrl: true,
        command: true,
        ..Default::default()
    };
    box_drag(&mut app, &ctx, from, to, additive);
    pointer(&mut app, &ctx, to, false, additive);
    assert_eq!(app.workspace.map().editor.selection_count(), 3);
    assert!(app.workspace.map().editor.is_selected(&outside));
    box_drag(&mut app, &ctx, from, to, egui::Modifiers::SHIFT);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(to + egui::vec2(60.0, 40.0))],
    );
    assert_eq!(
        app.workspace.map().viewport.pan,
        viewport.pan,
        "canceling a rectangle must not turn its remaining pointer movement into panning"
    );
    pointer(&mut app, &ctx, to, false, egui::Modifiers::NONE);
    assert_eq!(app.workspace.map().editor.selection_count(), 3);
    assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
    app.run_command(AppCommand::SelectAll, &ctx);
    box_drag(&mut app, &ctx, from, to, additive);
    pointer(&mut app, &ctx, to, false, additive);
    assert_eq!(
        app.workspace.map().editor.selection_coverage(),
        crate::app::SelectionCoverage::WholeMap
    );
}

#[test]
fn branch_focus_hides_other_topics_scopes_select_all_and_restores_the_full_view() {
    let (_temp, mut app) = app();
    let root = app.workspace.map().editor.document.root.clone();
    let branch = topic(&mut app, &root, "Focused branch");
    let child = topic(&mut app, &branch, "Focused child");
    topic(&mut app, &child, "Folded leaf");
    app.edit(EditorEdit::Command(Command::ToggleFold {
        id: child.clone(),
    }))
    .unwrap();
    let outside = topic(&mut app, &root, "Unrelated branch");
    app.edit(EditorEdit::Select(branch.clone())).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.workspace.map_mut().viewport.pan = Point { x: 81.0, y: -27.0 };
    app.workspace.map_mut().viewport.zoom = 0.8;
    let viewport = app.workspace.map().viewport;
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    focus_branch(&mut app, &ctx);
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    assert!(label(&output, "Unrelated branch").is_none());
    assert!(label(&output, "Focused child").is_some());
    assert_eq!(app.placed.nodes.len(), 2);
    app.checkpoint_recovery().unwrap();
    let restored = Workspace::from_restore(app.store.load().unwrap(), true).unwrap();
    assert_eq!(restored.map().viewport.pan, viewport.pan);
    assert_eq!(restored.map().viewport.zoom, viewport.zoom);
    app.run_command(AppCommand::SelectAll, &ctx);
    assert!(app.workspace.map().editor.is_selected(&branch));
    assert!(app.workspace.map().editor.is_selected(&child));
    assert!(!app.workspace.map().editor.is_selected(&outside));
    assert!(!app.workspace.map().editor.is_selected(&root));
    let forest = app.workspace.map().editor.copy_forest().unwrap();
    assert_eq!(forest.roots.len(), 1);
    assert_eq!(forest.roots[0].label, "Focused branch");
    assert_eq!(forest.roots[0].children[0].children[0].label, "Folded leaf");
    app.edit(EditorEdit::Select(child)).unwrap();
    focus_branch(&mut app, &ctx);
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    let back = label(&output, "Full map").expect("visible way to leave focus mode");
    pointer(&mut app, &ctx, back, true, egui::Modifiers::NONE);
    pointer(&mut app, &ctx, back, false, egui::Modifiers::NONE);
    let output = frame(&mut app, &ctx, vec![]);
    assert!(label(&output, "Unrelated branch").is_some());
    assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
    assert_eq!(app.workspace.map().viewport.zoom, viewport.zoom);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
}

#[test]
fn branch_focus_drag_keeps_children_inside_the_visible_branch_and_blank_drop_cancels() {
    let (_temp, mut app) = app();
    let root = app.workspace.map().editor.document.root.clone();
    let branch = topic(&mut app, &root, "Drag focus");
    let child = topic(&mut app, &branch, "Drag child");
    let outside = topic(&mut app, &root, "Hidden drag target");
    app.edit(EditorEdit::Select(branch.clone())).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    focus_branch(&mut app, &ctx);
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    let start = label(&output, "Drag child").unwrap();
    let target = label(&output, "Drag focus").unwrap() - egui::vec2(0.0, 18.0);
    pointer(&mut app, &ctx, start, true, egui::Modifiers::NONE);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(start + egui::vec2(12.0, 0.0))],
    );
    assert!(matches!(app.drag, Some(CanvasDrag::Branch(_))));
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(target)]);
    pointer(&mut app, &ctx, target, false, egui::Modifiers::NONE);
    assert_eq!(
        app.workspace.map().editor.document.parent(&child),
        Some(branch.as_str())
    );
    assert!(!app.placed.nodes.contains_key(&outside));
    app.edit(EditorEdit::Select(branch)).unwrap();
    let output = frame(&mut app, &ctx, vec![]);
    let start = label(&output, "Drag focus").unwrap();
    let blank = Pos2::new(40.0, 700.0);
    let before = app.workspace.map().editor.document.clone();
    pointer(&mut app, &ctx, start, true, egui::Modifiers::NONE);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(start + egui::vec2(12.0, 0.0))],
    );
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(blank)]);
    pointer(&mut app, &ctx, blank, false, egui::Modifiers::NONE);
    assert_eq!(app.workspace.map().editor.document, before);
}

#[test]
fn branch_focus_preserves_drafts_is_per_tab_and_exits_for_search_or_deletion() {
    let (_temp, mut app) = app();
    let owner = app.workspace.active_id();
    let root = app.workspace.map().editor.document.root.clone();
    let branch = topic(&mut app, &root, "Local focus");
    topic(&mut app, &branch, "Local child");
    let outside = topic(&mut app, &root, "Other topic");
    app.edit(EditorEdit::Select(branch.clone())).unwrap();
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
        .text = "Retained draft".into();
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    focus_branch(&mut app, &ctx);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
    assert_eq!(
        app.workspace.map().editor.editing.as_ref().unwrap().text,
        "Retained draft"
    );
    app.workspace.new_map().unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(
        app.placed
            .nodes
            .contains_key(&app.workspace.map().editor.document.root)
    );
    app.workspace.request_activate(owner).unwrap();
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    assert!(label(&output, "Other topic").is_none());
    app.select_search_result(outside);
    frame(&mut app, &ctx, vec![]);
    assert!(app.placed.nodes.contains_key(&root));
    app.edit(EditorEdit::Select(branch.clone())).unwrap();
    focus_branch(&mut app, &ctx);
    app.edit(EditorEdit::Command(Command::Delete { id: branch }))
        .unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(app.placed.nodes.contains_key(&root));
}

#[test]
fn branch_focus_breadcrumbs_refocus_ancestors_and_navigation_can_leave_focus() {
    let (_temp, mut app) = app();
    let root = app.workspace.map().editor.document.root.clone();
    let branch = topic(&mut app, &root, "Ancestor branch");
    let child = topic(&mut app, &branch, "Nested branch");
    topic(&mut app, &child, "Nested leaf");
    let sibling = topic(&mut app, &branch, "Ancestor sibling");
    app.edit(EditorEdit::Select(child)).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    focus_branch(&mut app, &ctx);
    frame(&mut app, &ctx, vec![]);
    let output = frame(&mut app, &ctx, vec![]);
    assert!(!app.placed.nodes.contains_key(&sibling));
    let ancestor = label(&output, "Ancestor branch").unwrap();
    pointer(&mut app, &ctx, ancestor, true, egui::Modifiers::NONE);
    pointer(&mut app, &ctx, ancestor, false, egui::Modifiers::NONE);
    frame(&mut app, &ctx, vec![]);
    assert!(app.placed.nodes.contains_key(&sibling));
    assert!(!app.placed.nodes.contains_key(&root));
    assert_eq!(app.workspace.map().editor.selected, branch);
    app.run_command(AppCommand::Left, &ctx);
    frame(&mut app, &ctx, vec![]);
    assert!(app.placed.nodes.contains_key(&root));
    assert_eq!(app.workspace.map().editor.document, before);
    assert_eq!(app.workspace.map().editor.history.generation, generation);
}

#[test]
fn box_selection_with_pan_zoom_and_branch_focus_can_drag_the_result_as_one_group() {
    let (_temp, mut app) = app();
    let root = app.workspace.map().editor.document.root.clone();
    let branch = topic(&mut app, &root, "Selection scope");
    let first = topic(&mut app, &branch, "Box one");
    let second = topic(&mut app, &branch, "Box two");
    let target = topic(&mut app, &branch, "Group destination");
    let hidden = topic(&mut app, &root, "Hidden selection");
    app.edit(EditorEdit::Select(branch)).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    focus_branch(&mut app, &ctx);
    frame(&mut app, &ctx, vec![]);
    app.workspace.map_mut().viewport.zoom = 0.75;
    app.workspace.map_mut().viewport.pan = Point { x: -180.0, y: 25.0 };
    let output = frame(&mut app, &ctx, vec![]);
    let rect = |id: &NodeId| {
        let world = app.placed.nodes[id];
        let text = &app.workspace.map().editor.document.nodes[id].text;
        egui::Rect::from_center_size(
            label(&output, text).unwrap(),
            egui::vec2(world.max.x - world.min.x, world.max.y - world.min.y)
                * app.workspace.map().viewport.zoom,
        )
    };
    let bounds = rect(&first).union(rect(&second)).expand(2.0);
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let viewport = app.workspace.map().viewport;
    box_drag(
        &mut app,
        &ctx,
        bounds.min,
        bounds.max,
        egui::Modifiers::SHIFT,
    );
    pointer(&mut app, &ctx, bounds.max, false, egui::Modifiers::SHIFT);
    assert_eq!(app.workspace.map().editor.selection_count(), 2);
    assert!(app.workspace.map().editor.is_selected(&first));
    assert!(app.workspace.map().editor.is_selected(&second));
    assert!(!app.workspace.map().editor.is_selected(&hidden));
    assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
    let output = frame(&mut app, &ctx, vec![]);
    let start = label(&output, "Box one").unwrap();
    let destination = label(&output, "Group destination").unwrap();
    pointer(&mut app, &ctx, start, true, egui::Modifiers::NONE);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(start + egui::vec2(12.0, 0.0))],
    );
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(destination)]);
    assert!(matches!(&app.drag, Some(CanvasDrag::Branch(drag)) if drag.grouped));
    assert_eq!(app.workspace.map().editor.document, before);
    pointer(&mut app, &ctx, destination, false, egui::Modifiers::NONE);
    assert_eq!(
        app.workspace.map().editor.document.nodes[&target].children,
        [first, second]
    );
    assert_eq!(
        app.workspace.map().editor.history.generation,
        generation + 1
    );
    app.run_command(AppCommand::Undo, &ctx);
    assert_eq!(app.workspace.map().editor.document, before);
}

#[test]
fn box_selection_cancels_outside_or_on_focus_loss_and_plain_drag_still_pans() {
    let (_temp, mut app) = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let from = Pos2::new(30.0, 180.0);
    let to = Pos2::new(220.0, 320.0);
    let selected = app.workspace.map().editor.selected.clone();
    let viewport = app.workspace.map().viewport;
    box_drag(&mut app, &ctx, from, to, egui::Modifiers::SHIFT);
    assert!(matches!(app.drag, Some(CanvasDrag::Selection { .. })));
    pointer(
        &mut app,
        &ctx,
        Pos2::new(-20.0, -20.0),
        false,
        egui::Modifiers::SHIFT,
    );
    assert!(app.drag.is_none());
    assert_eq!(app.workspace.map().editor.selected, selected);
    box_drag(&mut app, &ctx, from, to, egui::Modifiers::SHIFT);
    let mut input = egui::RawInput::default();
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(false);
    let mut output = ctx.run_ui(input, |ui| app.show(ui));
    output.textures_delta.clear();
    assert!(app.drag.is_none());
    assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
    pointer(&mut app, &ctx, to, false, egui::Modifiers::NONE);
    pointer(&mut app, &ctx, from, true, egui::Modifiers::NONE);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    pointer(&mut app, &ctx, to, false, egui::Modifiers::NONE);
    assert_ne!(app.workspace.map().viewport.pan, viewport.pan);
}

#[test]
fn box_selection_cancels_on_document_changes_without_panning_the_remaining_drag() {
    let (_temp, mut app) = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let viewport = app.workspace.map().viewport;
    let root = app.workspace.map().editor.selected.clone();
    let to = Pos2::new(220.0, 320.0);
    box_drag(
        &mut app,
        &ctx,
        Pos2::new(30.0, 180.0),
        to,
        egui::Modifiers::SHIFT,
    );
    app.edit(EditorEdit::Command(Command::SetText {
        id: root,
        text: "External change".into(),
    }))
    .unwrap();
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(to + egui::vec2(60.0, 40.0))],
    );
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(to + egui::vec2(90.0, 60.0))],
    );
    assert_eq!(app.workspace.map().viewport.pan, viewport.pan);
    assert!(app.drag.is_none());
}

#[test]
fn branch_focus_context_menu_preserves_unfinished_labels_and_notes() {
    let (_temp, mut app) = app();
    let root = app.workspace.map().editor.document.root.clone();
    let branch = topic(&mut app, &root, "Context focus");
    topic(&mut app, &branch, "Context child");
    app.edit(EditorEdit::Select(root.clone())).unwrap();
    app.edit(EditorEdit::Action(EditorAction::Edit)).unwrap();
    app.edit(EditorEdit::SetDraft {
        field: DraftField::Inline(root.clone()),
        text: "Unfinished root label".into(),
    })
    .unwrap();
    app.edit(EditorEdit::BeginDetails(root.clone())).unwrap();
    app.edit(EditorEdit::SetDraft {
        field: DraftField::Notes(root.clone()),
        text: "Unfinished root notes".into(),
    })
    .unwrap();
    app.workspace.map_mut().notes_open = true;
    let before = app.workspace.map().editor.document.clone();
    let generation = app.workspace.map().editor.history.generation;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let mut output = frame(&mut app, &ctx, vec![]);
    for command in ["Focus branch", "Show full map"] {
        let target = label(&output, "Context focus").unwrap();
        for pressed in [true, false] {
            frame(
                &mut app,
                &ctx,
                vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton {
                        pos: target,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            app.workspace.map().editor.document,
            before,
            "opening a context menu must not publish unfinished drafts"
        );
        output = frame(&mut app, &ctx, vec![]);
        let action = label(&output, command).expect("view command in topic context menu");
        pointer(&mut app, &ctx, action, true, egui::Modifiers::NONE);
        pointer(&mut app, &ctx, action, false, egui::Modifiers::NONE);
        output = frame(&mut app, &ctx, vec![]);
        assert_eq!(app.workspace.map().editor.document, before);
        assert_eq!(app.workspace.map().editor.history.generation, generation);
        assert_eq!(
            app.workspace.map().editor.editing.as_ref().unwrap().text,
            "Unfinished root label"
        );
        assert_eq!(
            app.workspace.map().editor.details.as_ref().unwrap().notes,
            "Unfinished root notes"
        );
    }
}

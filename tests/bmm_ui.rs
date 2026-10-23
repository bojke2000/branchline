use branchline::{app::EditorAction, storage::RecoveryStore, ui::NativeApp};
use eframe::egui::{self, Color32, Pos2, Shape};

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
    fn flatten(shape: Shape, shapes: &mut Vec<Shape>) {
        match shape {
            Shape::Vec(children) => {
                for child in children {
                    flatten(child, shapes);
                }
            }
            other => shapes.push(other),
        }
    }
    let mut shapes = Vec::new();
    for clipped in output.shapes {
        flatten(clipped.shape, &mut shapes);
    }
    shapes
}

fn text_center(shapes: &[Shape], label: &str) -> Option<Pos2> {
    shapes.iter().find_map(|shape| match shape {
        Shape::Text(text) if text.galley.job.text == label => {
            Some(text.pos + text.galley.size() * 0.5)
        }
        _ => None,
    })
}

fn click(
    app: &mut NativeApp,
    ctx: &egui::Context,
    pos: Pos2,
    button: egui::PointerButton,
) -> Vec<Shape> {
    click_with_modifiers(app, ctx, pos, button, egui::Modifiers::NONE)
}

fn click_with_modifiers(
    app: &mut NativeApp,
    ctx: &egui::Context,
    pos: Pos2,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) -> Vec<Shape> {
    let mut shapes = Vec::new();
    for pressed in [true, false] {
        shapes = render(
            app,
            ctx,
            vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                },
            ],
        );
    }
    shapes
}

#[test]
fn bmm_defaults_to_dark_mode_even_on_a_light_host() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Light);
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    assert!(
        shapes.iter().any(|shape| matches!(shape, Shape::Rect(rect)
        if rect.rect.width() > 900.0 && rect.rect.height() > 400.0 && rect.fill == Color32::from_rgb(22, 25, 33))),
        "The editor canvas must start dark, independently of host theme"
    );
    assert!(
        shapes
            .iter()
            .any(|shape| matches!(shape, Shape::Ellipse(ellipse)
        if ellipse.radius.x > ellipse.radius.y && ellipse.fill == Color32::from_rgb(22, 25, 33))),
        "The central topic must be an outlined oval"
    );
    assert!(
        text_center(&shapes, "BMM").is_some(),
        "The top-left logo must use the new identity"
    );
    assert!(text_center(&shapes, "branchline").is_none());
    click(
        &mut app,
        &ctx,
        text_center(&shapes, "More").unwrap(),
        egui::PointerButton::Primary,
    );
    let shapes = render(&mut app, &ctx, vec![]);
    let toggle = text_center(&shapes, "Switch light / dark theme").unwrap();
    click(&mut app, &ctx, toggle, egui::PointerButton::Primary);
    let shapes = render(&mut app, &ctx, vec![]);
    assert!(
        shapes.iter().any(|shape| matches!(shape, Shape::Rect(rect)
        if rect.rect.width() > 900.0 && rect.rect.height() > 400.0 && rect.fill == Color32::WHITE)),
        "The reference-style white editor must remain available"
    );
}

#[test]
fn dragging_the_selected_root_translates_the_entire_map_without_editing_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    for label in ["Pan right branch", "Pan left branch"] {
        let root = app.editor.document.root.clone();
        app.editor.select(&root).unwrap();
        app.editor.dispatch(EditorAction::AddChild).unwrap();
        app.editor.editing.as_mut().unwrap().text = label.into();
        app.editor.commit_edit().unwrap();
    }
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    click(
        &mut app,
        &ctx,
        text_center(&shapes, "Central idea").unwrap(),
        egui::PointerButton::Primary,
    );
    assert_eq!(app.editor.selected, app.editor.document.root);
    let before = app.editor.document.clone();
    let generation = app.editor.history.generation;
    for delta in [egui::vec2(160.0, 90.0), egui::vec2(-230.0, -150.0)] {
        let shapes = render(&mut app, &ctx, vec![]);
        let labels = ["Central idea", "Pan right branch", "Pan left branch"];
        let positions = labels.map(|label| text_center(&shapes, label).unwrap());
        let to = positions[0] + delta;
        begin_drag(&mut app, &ctx, positions[0], to);
        release_drag(&mut app, &ctx, to);
        let shapes = render(&mut app, &ctx, vec![]);
        for (label, position) in labels.into_iter().zip(positions) {
            let actual = text_center(&shapes, label).unwrap();
            assert!(
                (actual - position - delta).length() < 0.5,
                "{label} must follow the root drag by {delta:?}, not stay at {actual:?}"
            );
        }
        assert_eq!(app.editor.document, before);
        assert_eq!(
            app.editor.history.generation, generation,
            "Panning must not add document undo steps"
        );
    }
}

#[test]
fn root_panning_survives_leaving_the_canvas_and_escape_restores_the_view() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Central idea").unwrap();
    begin_drag(&mut app, &ctx, from, egui::pos2(1500.0, 1000.0));
    let to = from + egui::vec2(-170.0, 70.0);
    render(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);
    let shapes = render(&mut app, &ctx, vec![]);
    assert!((text_center(&shapes, "Central idea").unwrap() - to).length() < 0.5);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    release_drag(&mut app, &ctx, to);
    let shapes = render(&mut app, &ctx, vec![]);
    assert!(
        (text_center(&shapes, "Central idea").unwrap() - from).length() < 0.5,
        "Cancel must restore the view without changing the root's tree identity"
    );
    assert!(!app.editor.is_dirty());
}

#[test]
fn branch_underlines_share_their_root_branch_color() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    let first = app.editor.selected.clone();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.commit_edit().unwrap();
    let root = app.editor.document.root.clone();
    app.editor.select(&root).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.commit_edit().unwrap();
    assert_eq!(app.editor.document.nodes[&first].children.len(), 1);
    let ctx = egui::Context::default();
    let shapes = render(&mut app, &ctx, vec![]);
    let colors: Vec<_> = shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::LineSegment { points, stroke }
                if (points[0].y - points[1].y).abs() < 0.01
                    && (points[0].x - points[1].x).abs() > 60.0
                    && stroke.color.r().abs_diff(stroke.color.g()) > 35 =>
            {
                Some(stroke.color)
            }
            _ => None,
        })
        .collect();
    assert!(
        colors.len() >= 3,
        "Every visible non-root topic needs a colored underline"
    );
    assert!(
        colors
            .iter()
            .any(|color| colors.iter().filter(|other| *other == color).count() >= 2),
        "Descendants must inherit their branch color"
    );
}

#[test]
fn right_click_targets_that_node_and_executes_the_shared_creation_command() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "First target".into();
    app.editor.commit_edit().unwrap();
    let first = app.editor.selected.clone();
    app.editor.dispatch(EditorAction::AddSibling).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Second target".into();
    app.editor.commit_edit().unwrap();
    let second = app.editor.selected.clone();
    let ctx = egui::Context::default();
    let shapes = render(&mut app, &ctx, vec![]);
    let node_pos = text_center(&shapes, "First target").unwrap();
    click(&mut app, &ctx, node_pos, egui::PointerButton::Secondary);
    let shapes = render(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.selected, first);
    let add_pos = text_center(&shapes, "Add child")
        .expect("Right-click menu must offer keyboard creation commands");
    click(&mut app, &ctx, add_pos, egui::PointerButton::Primary);
    assert_eq!(app.editor.document.nodes[&first].children.len(), 1);
    assert!(app.editor.document.nodes[&second].children.is_empty());
    assert!(app.editor.editing.is_some());
}

#[test]
fn shift_arrows_reorder_siblings_but_not_text_editing() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.commit_edit().unwrap();
    let first = app.editor.selected.clone();
    app.editor.dispatch(EditorAction::AddSibling).unwrap();
    app.editor.commit_edit().unwrap();
    let second = app.editor.selected.clone();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shift = egui::Modifiers::SHIFT;
    let send = |app: &mut NativeApp, key, modifiers| {
        for pressed in [true, false] {
            render(
                app,
                &ctx,
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
    };
    send(&mut app, egui::Key::ArrowUp, shift);
    let root = app.editor.document.root.clone();
    assert_eq!(
        app.editor.document.nodes[&root].children,
        [second.clone(), first.clone()]
    );
    send(&mut app, egui::Key::ArrowDown, shift);
    assert_eq!(app.editor.document.nodes[&root].children, [first, second]);
    app.editor.dispatch(EditorAction::Edit).unwrap();
    render(&mut app, &ctx, vec![]);
    let before = app.editor.document.clone();
    send(&mut app, egui::Key::ArrowUp, shift);
    assert_eq!(
        app.editor.document, before,
        "Shift+arrows must stay inside text fields"
    );
}

#[test]
fn shift_click_batch_delete_retains_root_and_undoes_as_one_operation() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Select first".into();
    app.editor.commit_edit().unwrap();
    let first = app.editor.selected.clone();
    app.editor.dispatch(EditorAction::AddSibling).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Select second".into();
    app.editor.commit_edit().unwrap();
    let ctx = egui::Context::default();
    let shapes = render(&mut app, &ctx, vec![]);
    click_with_modifiers(
        &mut app,
        &ctx,
        text_center(&shapes, "Select first").unwrap(),
        egui::PointerButton::Primary,
        egui::Modifiers::SHIFT,
    );
    assert_eq!(app.editor.selection().len(), 2);
    assert!(app.editor.is_selected(&first));
    let shapes = render(&mut app, &ctx, vec![]);
    click_with_modifiers(
        &mut app,
        &ctx,
        text_center(&shapes, "Central idea").unwrap(),
        egui::PointerButton::Primary,
        egui::Modifiers::SHIFT,
    );
    assert_eq!(app.editor.selection().len(), 3);
    let before = app.editor.document.clone();
    render(
        &mut app,
        &ctx,
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::Key {
                key: egui::Key::Delete,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    let shapes = render(&mut app, &ctx, vec![]);
    assert_eq!(
        app.editor.document, before,
        "Batch delete must wait for confirmation"
    );
    assert!(text_center(&shapes, "The central idea will be kept.").is_some());
    let confirm = text_center(&shapes, "Delete selected branches").unwrap();
    click(&mut app, &ctx, confirm, egui::PointerButton::Primary);
    assert_eq!(app.editor.document.nodes.len(), 1);
    app.editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(app.editor.document, before);
}

fn begin_drag(app: &mut NativeApp, ctx: &egui::Context, from: Pos2, to: Pos2) -> Vec<Shape> {
    render(
        app,
        ctx,
        vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
    );
    render(
        app,
        ctx,
        vec![egui::Event::PointerMoved(from + egui::vec2(12.0, 0.0))],
    );
    render(app, ctx, vec![egui::Event::PointerMoved(to)])
}

fn release_drag(app: &mut NativeApp, ctx: &egui::Context, to: Pos2) {
    render(
        app,
        ctx,
        vec![egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    render(app, ctx, vec![]);
}

#[test]
fn dragging_previews_without_mutation_then_moves_and_undoes_a_subtree() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Drag source".into();
    app.editor.commit_edit().unwrap();
    let source = app.editor.selected.clone();
    let root = app.editor.document.root.clone();
    app.editor.select(&root).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Drop destination".into();
    app.editor.commit_edit().unwrap();
    let target = app.editor.selected.clone();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Drag source").unwrap();
    let to = text_center(&shapes, "Drop destination").unwrap();
    let before = app.editor.document.clone();
    let preview = begin_drag(&mut app, &ctx, from, to);
    assert_eq!(
        app.editor.document, before,
        "Hover previews must not change history or the map"
    );
    assert!(
        text_center(&preview, "Move inside topic").is_some(),
        "A drop needs an explicit placement preview"
    );
    release_drag(&mut app, &ctx, to);
    assert_eq!(app.editor.document.parent(&source), Some(target.as_str()));
    assert_eq!(app.editor.selected, source);
    app.editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(app.editor.document, before);
}

#[test]
fn drag_reordering_can_be_cancelled_and_cycles_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Drag parent".into();
    app.editor.commit_edit().unwrap();
    let parent = app.editor.selected.clone();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Drag descendant".into();
    app.editor.commit_edit().unwrap();
    app.editor.select(&parent).unwrap();
    app.editor.dispatch(EditorAction::AddSibling).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Drag sibling".into();
    app.editor.commit_edit().unwrap();
    let sibling = app.editor.selected.clone();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Drag sibling").unwrap();
    let to = text_center(&shapes, "Drag parent").unwrap() - egui::vec2(0.0, 20.0);
    let before = app.editor.document.clone();
    begin_drag(&mut app, &ctx, from, to);
    render(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    release_drag(&mut app, &ctx, to);
    assert_eq!(
        app.editor.document, before,
        "Escape must cancel before release"
    );
    let shapes = render(&mut app, &ctx, vec![]);
    let preview = begin_drag(
        &mut app,
        &ctx,
        text_center(&shapes, "Drag sibling").unwrap(),
        to,
    );
    assert!(text_center(&preview, "Insert before topic").is_some());
    release_drag(&mut app, &ctx, to);
    let root = app.editor.document.root.clone();
    assert_eq!(
        app.editor.document.nodes[&root].children,
        [sibling, parent.clone()]
    );
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Drag parent").unwrap();
    let to = text_center(&shapes, "Drag descendant").unwrap();
    let before = app.editor.document.clone();
    let preview = begin_drag(&mut app, &ctx, from, to);
    assert!(
        text_center(
            &preview,
            "A branch cannot be moved into itself or its descendants"
        )
        .is_some()
    );
    release_drag(&mut app, &ctx, to);
    assert_eq!(
        app.editor.document, before,
        "Dropping onto a descendant must never corrupt the tree"
    );
}

#[test]
fn dragging_a_top_level_branch_across_the_root_changes_its_side() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Switch side".into();
    app.editor.commit_edit().unwrap();
    let source = app.editor.selected.clone();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Switch side").unwrap();
    let to = egui::pos2(160.0, 600.0);
    begin_drag(&mut app, &ctx, from, to);
    release_drag(&mut app, &ctx, to);
    assert_eq!(
        app.editor.document.nodes[&source].side,
        branchline::document::Side::Left
    );
    assert_eq!(
        app.editor.document.parent(&source),
        Some(app.editor.document.root.as_str())
    );
}

#[test]
fn releasing_outside_the_canvas_cancels_without_leaking_drag_state() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Release outside".into();
    app.editor.commit_edit().unwrap();
    let source = app.editor.selected.clone();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Release outside").unwrap();
    let to = egui::pos2(-50.0, -50.0);
    let before = app.editor.document.clone();
    begin_drag(&mut app, &ctx, from, to);
    release_drag(&mut app, &ctx, to);
    assert_eq!(app.editor.document, before);
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Release outside").unwrap();
    let to = egui::pos2(150.0, 600.0);
    begin_drag(&mut app, &ctx, from, to);
    release_drag(&mut app, &ctx, to);
    assert_eq!(
        app.editor.document.nodes[&source].side,
        branchline::document::Side::Left
    );
}

#[test]
fn right_click_preserves_a_multi_selection_and_cancel_keeps_the_map() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Menu first".into();
    app.editor.commit_edit().unwrap();
    let first = app.editor.selected.clone();
    app.editor.dispatch(EditorAction::AddSibling).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Menu second".into();
    app.editor.commit_edit().unwrap();
    app.editor.toggle_selection(&first).unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    click(
        &mut app,
        &ctx,
        text_center(&shapes, "Menu second").unwrap(),
        egui::PointerButton::Secondary,
    );
    let shapes = render(&mut app, &ctx, vec![]);
    assert_eq!(app.editor.selection().len(), 2);
    assert!(text_center(&shapes, "2 selected ideas").is_some());
    click(
        &mut app,
        &ctx,
        text_center(&shapes, "Delete idea").unwrap(),
        egui::PointerButton::Primary,
    );
    let shapes = render(&mut app, &ctx, vec![]);
    let before = app.editor.document.clone();
    click(
        &mut app,
        &ctx,
        text_center(&shapes, "Cancel").unwrap(),
        egui::PointerButton::Primary,
    );
    assert_eq!(app.editor.document, before);
    assert_eq!(app.editor.selection().len(), 2);
}

#[test]
fn reordering_root_branches_does_not_recolor_them() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Stable branch color".into();
    app.editor.commit_edit().unwrap();
    app.editor.dispatch(EditorAction::AddSibling).unwrap();
    app.editor.commit_edit().unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let before = render(&mut app, &ctx, vec![]);
    let stroke = |shapes: &[Shape]| {
        let center = text_center(shapes, "Stable branch color").unwrap();
        shapes
            .iter()
            .find_map(|shape| match shape {
                Shape::LineSegment { points, stroke }
                    if points[0].y == points[1].y
                        && (points[0].x - points[1].x).abs() > 60.0
                        && points[0].x.min(points[1].x) < center.x
                        && points[0].x.max(points[1].x) > center.x
                        && points[0].y > center.y
                        && points[0].y - center.y < 30.0 =>
                {
                    Some(stroke.color)
                }
                _ => None,
            })
            .unwrap()
    };
    app.editor.dispatch(EditorAction::MoveUp).unwrap();
    let after = render(&mut app, &ctx, vec![]);
    assert_eq!(stroke(&before), stroke(&after));
}

#[test]
fn opening_an_unsaved_dialog_cancels_a_drag_before_pointer_release() {
    let temp = tempfile::tempdir().unwrap();
    let mut app = NativeApp::new(RecoveryStore::open(temp.path().into()).unwrap()).unwrap();
    app.editor.dispatch(EditorAction::AddChild).unwrap();
    app.editor.editing.as_mut().unwrap().text = "Close during drag".into();
    app.editor.commit_edit().unwrap();
    let ctx = egui::Context::default();
    render(&mut app, &ctx, vec![]);
    let shapes = render(&mut app, &ctx, vec![]);
    let from = text_center(&shapes, "Close during drag").unwrap();
    let to = egui::pos2(150.0, 600.0);
    let before = app.editor.document.clone();
    begin_drag(&mut app, &ctx, from, to);
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1200.0, 800.0),
        )),
        ..Default::default()
    };
    input
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    let mut output = ctx.run_ui(input, |ui| app.show(ui));
    output.textures_delta.clear();
    let shapes = render(&mut app, &ctx, vec![]);
    assert!(text_center(&shapes, "Keep these ideas?").is_some());
    release_drag(&mut app, &ctx, to);
    assert_eq!(
        app.editor.document, before,
        "A modal dialog must prevent a pending drop from committing"
    );
}

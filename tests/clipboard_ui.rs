use branchline::{
    app::{Editor, SelectionCoverage},
    interchange::import_mm,
    recovery::RecoveryCoordinator,
    ui::NativeApp,
};
use eframe::egui::{self, Pos2, Shape};

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
    fn flatten(shape: Shape, result: &mut Vec<Shape>) {
        match shape {
            Shape::Vec(children) => children
                .into_iter()
                .for_each(|child| flatten(child, result)),
            other => result.push(other),
        }
    }
    let mut result = vec![];
    for clipped in output.shapes {
        flatten(clipped.shape, &mut result);
    }
    result
}

fn key(
    app: &mut NativeApp,
    ctx: &egui::Context,
    key: egui::Key,
    modifiers: egui::Modifiers,
) -> Vec<Shape> {
    let result = render(
        app,
        ctx,
        vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            },
        ],
    );
    render(
        app,
        ctx,
        vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        }],
    );
    result
}

#[test]
fn physical_ctrl_and_cmd_select_the_folded_map_and_delete_requires_confirmation() {
    for modifiers in [
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..Default::default()
        },
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap()).unwrap();
        app.workspace.map_mut().editor = Editor::from_document(import_mm(br#"<map><node ID="r" TEXT="Root" FOLDED="true"><node ID="a" TEXT="A"><node ID="b" TEXT="B"/></node><node ID="c" TEXT="C"/></node></map>"#).unwrap().document, None, true, None).unwrap();
        let ctx = egui::Context::default();
        render(&mut app, &ctx, vec![]);
        render(&mut app, &ctx, vec![]);
        key(&mut app, &ctx, egui::Key::A, modifiers);
        assert_eq!(
            app.workspace.map().editor.selection_coverage(),
            SelectionCoverage::WholeMap
        );
        let shapes = render(&mut app, &ctx, vec![]);
        assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text.contains("4 selected (3 hidden)"))));
        key(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
        let shapes = render(&mut app, &ctx, vec![]);
        assert!(shapes.iter().any(|shape| matches!(shape, Shape::Text(text) if text.galley.job.text == "The central idea will be kept.")));
        assert_eq!(app.workspace.map().editor.document.nodes.len(), 4);
    }
}

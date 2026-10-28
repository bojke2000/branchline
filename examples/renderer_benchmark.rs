use branchline::{
    app::{DraftField, Editor, EditorAction, EditorEdit},
    desktop,
    recovery::RecoveryCoordinator,
    ui::NativeApp,
};
use eframe::{App, egui};
use std::{cell::Cell, rc::Rc, time::Instant};

struct Benchmark {
    app: NativeApp,
    frames: usize,
    previous: Option<Instant>,
    started: Instant,
    intervals: Vec<f64>,
    renderer: String,
    completed: Rc<Cell<bool>>,
    last_frame: Option<u64>,
    measured_scale: Option<f32>,
    preview: bool,
}

impl Benchmark {
    fn show(&mut self, ui: &mut egui::Ui) {
        if self.preview {
            self.app.show(ui);
            if ui.ctx().input(|input| {
                input.viewport().title.as_deref() != Some("BMM renderer preview - temporary data")
            }) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(
                    "BMM renderer preview - temporary data".into(),
                ));
            }
            return;
        }
        if self.completed.get() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let frame = ui.ctx().cumulative_frame_nr();
        self.app.show(ui);
        if self.last_frame == Some(frame) {
            return;
        }
        self.last_frame = Some(frame);
        assert!(
            self.started.elapsed().as_secs() < 180,
            "Benchmark timed out"
        );
        let now = Instant::now();
        if self.frames >= 20 {
            let scale = ui.ctx().pixels_per_point();
            if let Some(previous) = self.measured_scale {
                assert_eq!(previous, scale, "Display scale changed during benchmark");
            }
            self.measured_scale = Some(scale);
            self.intervals
                .push(now.duration_since(self.previous.unwrap()).as_secs_f64() * 1000.0);
        }
        self.previous = Some(now);
        self.frames += 1;
        if self.frames == 240 {
            assert_eq!(
                self.app
                    .workspace
                    .map()
                    .editor
                    .editing
                    .as_ref()
                    .unwrap()
                    .text,
                "x".repeat(239),
            );
            self.intervals.sort_by(f64::total_cmp);
            println!(
                "{}",
                serde_json::json!({
                    "renderer": self.renderer,
                    "samples": self.intervals.len(),
                    "median_ms": self.intervals[110],
                    "p95_ms": self.intervals[208],
                    "max_ms": self.intervals.last(),
                    "scale_factor": ui.ctx().pixels_per_point(),
                    "workload": "Scripted inline editing in the production egui UI; not input-to-present latency",
                })
            );
            self.completed.set(true);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            let id = self.app.workspace.map().editor.selected.clone();
            self.app
                .workspace
                .apply(
                    self.app.workspace.active_id(),
                    EditorEdit::SetDraft {
                        field: DraftField::Inline(id),
                        text: "x".repeat(self.frames),
                    },
                )
                .unwrap();
            ui.ctx().request_repaint();
        }
    }
}

impl App for Benchmark {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.show(ui);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let renderer = args.next().unwrap_or_else(|| "wgpu".into());
    let preview = match args.next().as_deref() {
        None => false,
        Some("--preview") => true,
        _ => return Err("Only --preview may follow the renderer".into()),
    };
    if args.next().is_some() {
        return Err("Unexpected renderer benchmark arguments".into());
    }
    if renderer != "wgpu" && !(cfg!(windows) && renderer == "software") {
        return Err("Use wgpu, or software on Windows".into());
    }
    let _directory = tempfile::tempdir()?;
    let mut app = NativeApp::new(RecoveryCoordinator::open(_directory.path().into())?)?;
    if preview {
        let report = branchline::interchange::import_mm(include_bytes!("brainstorm.mm"))?;
        app.workspace.map_mut().editor = Editor::from_document(report.document, None, true, None)?;
    }
    if !preview {
        app.workspace.apply(
            app.workspace.active_id(),
            EditorEdit::Action(EditorAction::Edit),
        )?;
        let id = app.workspace.map().editor.selected.clone();
        app.workspace.apply(
            app.workspace.active_id(),
            EditorEdit::SetDraft {
                field: DraftField::Inline(id),
                text: String::new(),
            },
        )?;
    }
    let completed = Rc::new(Cell::new(false));
    let mut benchmark = Benchmark {
        app,
        frames: 0,
        previous: None,
        started: Instant::now(),
        intervals: vec![],
        renderer: renderer.clone(),
        completed: Rc::clone(&completed),
        last_frame: None,
        measured_scale: None,
        preview,
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title("BMM renderer benchmark - temporary map")
        .with_inner_size([1280.0, 840.0]);
    #[cfg(windows)]
    if renderer == "software" {
        let context = egui::Context::default();
        desktop::configure_context(&context);
        desktop::software::run(context, viewport, |ui| benchmark.show(ui))?;
        if !preview && !completed.get() {
            return Err("Benchmark closed before completing its samples".into());
        }
        return Ok(());
    }
    eframe::run_native(
        "BMM renderer benchmark - temporary map",
        eframe::NativeOptions {
            viewport,
            ..Default::default()
        },
        Box::new(move |context| {
            desktop::configure_context(&context.egui_ctx);
            println!(
                "adapter={:?}",
                context
                    .wgpu_render_state
                    .as_ref()
                    .unwrap()
                    .adapter
                    .get_info()
            );
            Ok(Box::new(benchmark))
        }),
    )?;
    if !preview && !completed.get() {
        return Err("Benchmark closed before completing its samples".into());
    }
    Ok(())
}

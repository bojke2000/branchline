pub use crate::commands::palette_matches;
use crate::{
    app::{DraftField, EditorAction, EditorEdit, EditorError, EditorKey},
    appearance,
    commands::{self, AppCommand, FILE_COMMANDS, NAVIGATION_COMMANDS, VIEW_COMMANDS},
    document::{Command, DeletionPlan, MovePlacement, NodeId},
    drag::DragSession,
    file_identity::read_identified,
    interchange::{ImportReport, export_mm, import_mm},
    layout::{self, Layout, Point, Rect, Size},
    recovery::RecoveryCoordinator,
    session::{SessionId, Viewport},
    storage::{StorageError, atomic_write, read_bounded},
    workspace::{CloseDecision, TransitionProgress, Workspace},
};
use eframe::egui::{self, Color32, FontId, Id, Key, Pos2, Sense, Stroke, Vec2};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

enum CanvasDrag {
    Pan { origin: Point, initial_pan: Point },
    Branch(DragSession),
}

enum Dialog {
    CloseDecision(SessionId),
    Paste(paste::PendingPaste),
    CopyOutline(crate::outline_format::OutlineCopy),
    Import,
    Delete(DeletionPlan),
    MermaidExport {
        session: SessionId,
        bytes: Vec<u8>,
    },
    Overwrite {
        session: SessionId,
        path: PathBuf,
        export: Option<Vec<u8>>,
    },
}

mod details;
#[cfg(test)]
mod mermaid_tests;
mod paste;
mod recovery;
mod tabs;
mod workspace_dialogs;

pub struct NativeApp {
    pub workspace: Workspace,
    store: RecoveryCoordinator,
    dialog: Option<Dialog>,
    error: Option<String>,
    status: String,
    placed: Layout,
    galleys: BTreeMap<NodeId, Arc<egui::Galley>>,
    branch_colors: BTreeMap<NodeId, Color32>,
    layout_revision: Option<u64>,
    layout_session: Option<SessionId>,
    cached_dark: bool,
    theme_initialized: bool,
    focus_selected: bool,
    fit_requested: bool,
    palette_open: bool,
    palette_query: String,
    palette_focus: bool,
    help_open: bool,
    edit_focus: Option<NodeId>,
    composing: Option<Id>,
    drag: Option<CanvasDrag>,
    allow_close: bool,
    input_after_switch: bool,
    owned_widgets: BTreeMap<SessionId, HashSet<Id>>,
    pending_close: Option<SessionId>,
    pending_quit: bool,
    pending_import: Option<ImportReport>,
    clipboard: Box<dyn crate::clipboard::OutlineClipboard>,
    clipboard_gesture: Option<paste::ClipboardGesture>,
}

impl NativeApp {
    pub fn new(store: RecoveryCoordinator) -> Result<Self, StorageError> {
        Self::with_clipboard(store, Box::<crate::clipboard::NativeClipboard>::default())
    }

    pub fn with_clipboard(
        mut store: RecoveryCoordinator,
        clipboard: Box<dyn crate::clipboard::OutlineClipboard>,
    ) -> Result<Self, StorageError> {
        let restore = store.load()?;
        let notices = restore.warnings.join("\n");
        let workspace = Workspace::from_restore(restore, true)
            .map_err(|cause| StorageError(cause.to_string()))?;
        Ok(Self {
            workspace,
            store,
            dialog: None,
            error: (!notices.is_empty()).then_some(notices),
            status: "Local by design. Your ideas stay on this computer.".into(),
            placed: Layout::default(),
            galleys: BTreeMap::new(),
            branch_colors: BTreeMap::new(),
            layout_revision: None,
            layout_session: None,
            cached_dark: false,
            theme_initialized: false,
            focus_selected: false,
            fit_requested: false,
            palette_open: false,
            palette_query: String::new(),
            palette_focus: false,
            help_open: false,
            edit_focus: None,
            composing: None,
            drag: None,
            allow_close: false,
            input_after_switch: false,
            owned_widgets: BTreeMap::new(),
            pending_close: None,
            pending_quit: false,
            pending_import: None,
            clipboard,
            clipboard_gesture: None,
        })
    }

    fn report(&mut self, result: Result<(), EditorError>) {
        if let Err(error) = result {
            self.error = Some(error.to_string());
        }
    }

    pub fn stage_import(&mut self, report: ImportReport) -> Result<(), EditorError> {
        report.document.validate()?;
        if self.dialog.is_some() {
            return Err(EditorError::State(
                "Finish the current dialog before staging another import.".into(),
            ));
        }
        self.pending_import = Some(report);
        self.dialog = Some(Dialog::Import);
        Ok(())
    }

    pub fn checkpoint_recovery(&mut self) -> Result<(), EditorError> {
        let publication = self.store.publish(&self.workspace.recovery_state())?;
        if let Some(cause) = publication.cleanup_error {
            self.error = Some(format!(
                "Workspace recovery was published; snapshot cleanup remains pending: {cause}"
            ));
        }
        Ok(())
    }

    fn prepare(&mut self) -> Result<(), EditorError> {
        self.edit(EditorEdit::CommitDrafts)
    }

    fn edit(&mut self, edit: EditorEdit) -> Result<(), EditorError> {
        self.workspace
            .apply(self.workspace.active_id(), edit)
            .map_err(EditorError::from)
    }

    fn action(&mut self, action: EditorAction) {
        let result = (|| {
            if matches!(action, EditorAction::Delete) {
                let plan = self.workspace.map().editor.deletion_plan()?;
                if self.workspace.map().editor.selection_count() > 1 || plan.node_count > 1 {
                    self.dialog = Some(Dialog::Delete(plan));
                    return Ok(());
                }
            }
            self.edit(EditorEdit::Action(action))?;
            self.focus_selected = true;
            Ok(())
        })();
        self.report(result);
    }

    fn run_command(&mut self, command: AppCommand, ctx: &egui::Context) {
        if let Some(action) = command.editor_action() {
            self.action(action);
            return;
        }
        match command {
            AppCommand::Copy => self.copy_outline(),
            AppCommand::Paste => {
                self.begin_paste(false, None);
            }
            AppCommand::PasteAsNewMap => {
                self.begin_paste(true, None);
            }
            AppCommand::SelectAll => {
                let result = self.edit(EditorEdit::SelectAll);
                self.report(result);
            }
            AppCommand::New => {
                let result = self
                    .workspace
                    .new_map()
                    .map(|_| ())
                    .map_err(EditorError::from);
                self.report(result);
            }
            AppCommand::NextTab | AppCommand::PreviousTab => {
                let result = self
                    .workspace
                    .cycle(command == AppCommand::PreviousTab)
                    .map_err(EditorError::from);
                self.report(result);
            }
            AppCommand::CloseTab => {
                self.close_tab(self.workspace.active_id(), ctx);
            }
            AppCommand::Open => self.open_file(ctx),
            AppCommand::Save => self.save_file(false, ctx),
            AppCommand::SaveAs => self.save_file(true, ctx),
            AppCommand::Export => self.export_file(ctx),
            AppCommand::ExportMermaid => {
                let session = self.workspace.active_id();
                let result = self
                    .workspace
                    .session(session)
                    .map_err(EditorError::from)
                    .and_then(|map| map.editor.snapshot_with_drafts())
                    .and_then(|document| {
                        crate::mermaid::export_mermaid(&document).map_err(EditorError::from)
                    });
                match result {
                    Ok(bytes) => self.dialog = Some(Dialog::MermaidExport { session, bytes }),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            AppCommand::Quit => self.queue_quit(),
            AppCommand::Search => self.open_search(),
            AppCommand::Palette => self.open_palette(),
            AppCommand::Notes => {
                if self.workspace.map().notes_open {
                    let result = self.edit(EditorEdit::FlushDetails);
                    if let Err(error) = result {
                        self.report(Err(error));
                        return;
                    }
                }
                self.workspace.map_mut().notes_open = !self.workspace.map().notes_open;
            }
            AppCommand::Fit => self.fit_requested = true,
            AppCommand::Focus => {
                if let Some(rect) = self.placed.nodes.get(&self.workspace.map().editor.selected) {
                    self.workspace.map_mut().viewport.focus(rect.center());
                }
            }
            AppCommand::PanLeft => self.workspace.map_mut().viewport.pan.x -= 40.0,
            AppCommand::PanRight => self.workspace.map_mut().viewport.pan.x += 40.0,
            AppCommand::PanUp => self.workspace.map_mut().viewport.pan.y -= 40.0,
            AppCommand::PanDown => self.workspace.map_mut().viewport.pan.y += 40.0,
            AppCommand::Theme => ctx.set_theme(if ctx.global_style().visuals.dark_mode {
                egui::Theme::Light
            } else {
                egui::Theme::Dark
            }),
            AppCommand::Help => self.help_open = true,
            _ => unreachable!("Editor commands are dispatched above"),
        }
    }

    fn command_enabled(&self, command: AppCommand, target: &NodeId) -> bool {
        if command == AppCommand::Delete
            && self.workspace.map().editor.selection_coverage()
                == crate::app::SelectionCoverage::WholeMap
        {
            return self.workspace.map().editor.document.nodes.len() > 1;
        }
        if command == AppCommand::Delete
            && self.workspace.map().editor.is_selected(target)
            && self
                .workspace
                .map()
                .editor
                .selection()
                .iter()
                .any(|id| id != &self.workspace.map().editor.document.root)
        {
            return true;
        }
        if matches!(
            command,
            AppCommand::Delete
                | AppCommand::Indent
                | AppCommand::Outdent
                | AppCommand::MoveUp
                | AppCommand::MoveDown
        ) && target == &self.workspace.map().editor.document.root
        {
            return false;
        }
        command != AppCommand::Fold
            || !self.workspace.map().editor.document.nodes[target]
                .children
                .is_empty()
    }

    fn menu_commands(
        &self,
        ui: &mut egui::Ui,
        commands: &[AppCommand],
        target: &NodeId,
    ) -> Option<AppCommand> {
        let mut chosen = None;
        for command in commands {
            let spec = command.spec();
            if ui
                .add_enabled(
                    self.command_enabled(*command, target),
                    egui::Button::new(spec.label).shortcut_text(spec.shortcut),
                )
                .clicked()
            {
                chosen = Some(*command);
                ui.close();
            }
        }
        chosen
    }

    fn context_menu(
        &self,
        ui: &mut egui::Ui,
        target: &NodeId,
        on_node: bool,
    ) -> Option<AppCommand> {
        ui.set_min_width(245.0);
        if self.workspace.map().editor.is_selected(target)
            && self.workspace.map().editor.selection_count() > 1
        {
            ui.weak(format!(
                "{} selected ideas",
                self.workspace.map().editor.selection_count()
            ));
        } else {
            ui.weak(if on_node {
                "This idea"
            } else {
                "Current selection"
            });
        }
        ui.separator();
        let mut chosen = self.menu_commands(
            ui,
            &[
                AppCommand::AddChild,
                AppCommand::AddSibling,
                AppCommand::Edit,
                AppCommand::Fold,
            ],
            target,
        );
        ui.menu_button("Organize branch", |ui| {
            chosen = self
                .menu_commands(
                    ui,
                    &[
                        AppCommand::MoveUp,
                        AppCommand::MoveDown,
                        AppCommand::Indent,
                        AppCommand::Outdent,
                    ],
                    target,
                )
                .or(chosen);
        });
        ui.menu_button("Navigate", |ui| {
            chosen = self
                .menu_commands(ui, NAVIGATION_COMMANDS, target)
                .or(chosen);
        });
        chosen = self
            .menu_commands(
                ui,
                &[
                    AppCommand::SelectAll,
                    AppCommand::Copy,
                    AppCommand::Paste,
                    AppCommand::PasteAsNewMap,
                    AppCommand::Delete,
                ],
                target,
            )
            .or(chosen);
        ui.separator();
        chosen = self
            .menu_commands(ui, &[AppCommand::Undo, AppCommand::Redo], target)
            .or(chosen);
        ui.separator();
        ui.menu_button("File", |ui| {
            chosen = self.menu_commands(ui, FILE_COMMANDS, target).or(chosen);
        });
        ui.menu_button("View and tools", |ui| {
            chosen = self.menu_commands(ui, VIEW_COMMANDS, target).or(chosen);
        });
        chosen
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, blocked: bool) {
        let ctx = ui.ctx().clone();
        ui.add_enabled_ui(!blocked, |ui| {
            ui.horizontal_wrapped(|ui| {
                appearance::logo(ui);
                ui.separator();
                for command in [AppCommand::New, AppCommand::Open, AppCommand::Save] {
                    if appearance::toolbar_button(ui, command, true, false).clicked() {
                        self.run_command(command, &ctx);
                    }
                }
                ui.menu_button("More", |ui| {
                    let commands = [
                        AppCommand::SaveAs,
                        AppCommand::Export,
                        AppCommand::ExportMermaid,
                        AppCommand::Theme,
                        AppCommand::Help,
                    ];
                    if let Some(command) =
                        self.menu_commands(ui, &commands, &self.workspace.map().editor.selected)
                    {
                        self.run_command(command, &ctx);
                    }
                });
                ui.separator();
                for command in [AppCommand::Search, AppCommand::Palette, AppCommand::Notes] {
                    let active = match command {
                        AppCommand::Search => self.workspace.map().search.open,
                        AppCommand::Palette => self.palette_open,
                        _ => self.workspace.map().notes_open,
                    };
                    if appearance::toolbar_button(ui, command, true, active).clicked() {
                        self.run_command(command, &ctx);
                    }
                }
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for group in [
                    &[AppCommand::AddChild, AppCommand::AddSibling][..],
                    &[AppCommand::Edit, AppCommand::Fold, AppCommand::Delete],
                    &[AppCommand::Undo, AppCommand::Redo],
                    &[AppCommand::Fit, AppCommand::Focus],
                ] {
                    if group[0] != AppCommand::AddChild {
                        ui.separator();
                    }
                    for command in group {
                        let enabled =
                            self.command_enabled(*command, &self.workspace.map().editor.selected);
                        if appearance::toolbar_button(ui, *command, enabled, false).clicked() {
                            self.run_command(*command, &ctx);
                        }
                    }
                }
            });
        });
    }

    fn open_file(&mut self, _ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Mind maps", &["branchline", "mm"])
            .pick_file()
        {
            let result = if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("mm"))
            {
                read_bounded(&path)
                    .map_err(EditorError::from)
                    .and_then(|bytes| self.stage_import(import_mm(&bytes)?))
            } else {
                read_identified(&path)
                    .map_err(EditorError::from)
                    .and_then(|file| {
                        self.workspace
                            .open_native(file)
                            .map(|_| ())
                            .map_err(EditorError::from)
                    })
            };
            self.report(result);
        }
    }

    fn save_file(&mut self, save_as: bool, ctx: &egui::Context) {
        self.save_session(self.workspace.active_id(), save_as, ctx);
    }

    fn save_session(&mut self, session: SessionId, save_as: bool, ctx: &egui::Context) {
        let selected = match self.workspace.session(session) {
            Ok(map) => map.editor.path.clone(),
            Err(cause) => {
                self.abort_transition();
                self.error = Some(cause.to_string());
                return;
            }
        };
        let choosing_destination = save_as || selected.is_none();
        let path = if choosing_destination {
            rfd::FileDialog::new()
                .add_filter("BMM mind map", &["branchline"])
                .set_file_name("Ideas.branchline")
                .save_file()
        } else {
            selected
        };
        if let Some(path) = path {
            if choosing_destination && path.exists() {
                self.dialog = Some(Dialog::Overwrite {
                    session,
                    path,
                    export: None,
                });
            } else {
                self.publish(session, path, None, ctx);
            }
        } else if self.workspace.transition_pending() {
            self.abort_transition();
        }
    }

    fn publish(
        &mut self,
        session: SessionId,
        path: PathBuf,
        export: Option<Vec<u8>>,
        ctx: &egui::Context,
    ) {
        let is_export = export.is_some();
        let result = if let Some(bytes) = export {
            self.workspace.destination_owner(&path).map_err(EditorError::from).and_then(|owner| {
                if owner.is_some() {
                    return Err(EditorError::State("Export cannot overwrite an open native mind map. Choose a different destination.".into()));
                }
                atomic_write(&path, &bytes).map_err(EditorError::from)
            })
        } else {
            self.workspace
                .save(session, &path)
                .map_err(EditorError::from)
                .and_then(|_| self.checkpoint_recovery())
        };
        match result {
            Ok(()) => {
                self.status = format!(
                    "{} {}",
                    if is_export { "Exported" } else { "Saved" },
                    path.display()
                );
                if !is_export && self.workspace.transition_pending() {
                    let progress = self
                        .workspace
                        .resume_after_save(session)
                        .map_err(EditorError::from);
                    self.drive_transition(progress, ctx);
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.abort_transition();
            }
        }
    }

    fn export_file(&mut self, ctx: &egui::Context) {
        let result = self.prepare().and_then(|_| {
            export_mm(&self.workspace.map().editor.document).map_err(EditorError::from)
        });
        match result {
            Ok(report) => {
                let accepted = rfd::MessageDialog::new().set_title("Export supported Freeplane data")
                    .set_description("Exports node text, order, IDs, folding, branch sides, plain notes and links only. Formatting, rich content, icons, cross-links and scripts from earlier imports are not recoverable. The current BMM document will not be replaced.")
                    .set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
                if accepted
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Freeplane", &["mm"])
                        .set_file_name("Ideas.mm")
                        .save_file()
                {
                    self.stage_export(self.workspace.active_id(), Some(path), report.bytes, ctx);
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn stage_export(
        &mut self,
        session: SessionId,
        path: Option<PathBuf>,
        bytes: Vec<u8>,
        ctx: &egui::Context,
    ) {
        let Some(path) = path else { return };
        if path.exists() {
            self.dialog = Some(Dialog::Overwrite {
                session,
                path,
                export: Some(bytes),
            });
        } else {
            self.publish(session, path, Some(bytes), ctx);
        }
    }

    pub fn show(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        if self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.theme_initialized {
            ctx.set_theme(egui::Theme::Dark);
            root_ui.style_mut().visuals = egui::Visuals::dark();
            self.theme_initialized = true;
        }
        if self.workspace.active().is_err() {
            self.failed_restore_view(root_ui);
            return;
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.dialog.is_none() {
                self.queue_quit();
            }
        }
        if self.dialog.is_none() {
            self.shortcuts(&ctx);
        }
        let text_pending = self.workspace.map().editor.editing.is_some();
        let blocked = self.dialog.is_some();
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(ctx.global_style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(12, 8)),
            )
            .show(root_ui, |ui| self.toolbar(ui, blocked));
        self.tab_strip(root_ui, blocked);
        let palette_events = if self.palette_open {
            ctx.input_mut(|input| {
                let (text, other) = std::mem::take(&mut input.events)
                    .into_iter()
                    .partition::<Vec<_>, _>(|event| {
                        matches!(
                            event,
                            egui::Event::Text(_)
                                | egui::Event::Paste(_)
                                | egui::Event::Copy
                                | egui::Event::Cut
                                | egui::Event::Ime(_)
                        )
                    });
                input.events = other;
                text
            })
        } else {
            Vec::new()
        };
        if self.dialog.is_some() {
            ctx.input_mut(|input| {
                input.events.retain(|event| {
                    !matches!(
                        event,
                        egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Ime(_)
                    )
                })
            });
        }
        egui::Panel::bottom("status").show(root_ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} ideas  /  {}%{}",
                    self.workspace.map().editor.document.nodes.len(),
                    (self.workspace.map().viewport.zoom * 100.0).round(),
                    if self.workspace.map().editor.is_dirty() {
                        "  /  Unsaved changes"
                    } else {
                        ""
                    }
                ));
                ui.separator();
                let editor = &self.workspace.map().editor;
                let hidden = editor.hidden_selection_count();
                ui.weak(format!(
                    "{} selected{}  /  Shift-click: multi-select  /  Right-click: commands",
                    editor.selection_count(),
                    if hidden > 0 {
                        format!(" ({hidden} hidden)")
                    } else {
                        String::new()
                    }
                ));
            });
            if let Some(error) = self.error.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::from_rgb(210, 75, 70), error);
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            } else {
                ui.weak(&self.status);
            }
            if self.workspace.map().editor.history.evicted {
                ui.weak("Old undo steps were evicted to keep history within its memory budget.");
            }
            ui.add_space(4.0);
        });
        if self.workspace.map().search.open {
            self.search_panel(root_ui);
        }
        if self.workspace.map().notes_open {
            self.details_panel(root_ui);
        }
        egui::CentralPanel::default().show(root_ui, |ui| {
            ui.add_enabled_ui(self.dialog.is_none() && !self.palette_open, |ui| {
                self.canvas(ui)
            });
        });
        ctx.input_mut(|input| input.events.extend(palette_events));
        if !blocked {
            self.palette(&ctx);
        }
        self.dialogs(&ctx);
        if self.workspace.active().is_err() {
            ctx.request_repaint();
            self.finish_tab_transition(&ctx);
            self.clipboard_gesture = None;
            return;
        }
        if self.help_open {
            egui::Window::new("BMM keyboard and mouse guide")
                .open(&mut self.help_open)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label("Capture ideas without leaving the keyboard.");
                    ui.separator();
                    for (key, purpose) in [
                        ("Tab", "Add child / commit and continue with a child"),
                        ("Enter", "Add sibling / commit text while editing"),
                        ("F2 or double-click", "Edit selected idea"),
                        ("Start typing", "Replace the active idea; Enter commits"),
                        ("Escape", "Cancel inline edit or close search/commands"),
                        ("Arrows", "Navigate the map (text fields keep cursor keys)"),
                        ("Space", "Fold / unfold"),
                        (
                            "Alt + arrows",
                            "Up/down: reorder; right/left: indent/outdent",
                        ),
                        ("Shift + Up / Down", "Reorder among same-side siblings"),
                        ("Alt + Shift + arrows", "Pan the whole map"),
                        ("Cmd/Ctrl + Z / Shift+Z", "Undo / redo"),
                        ("Cmd/Ctrl + K / F", "Commands / search"),
                        ("Cmd/Ctrl + N / O / S", "New / open / save"),
                        ("Cmd/Ctrl + Shift + S", "Save as"),
                        ("Drag blank canvas", "Pan"),
                        (
                            "Drag the central root",
                            "Reposition the entire map without editing it",
                        ),
                        ("Drag a topic", "Preview a branch move; Escape cancels"),
                        ("Shift + click", "Toggle nodes in the selection"),
                        (
                            "Delete / Backspace",
                            "Delete selected branches; one undo restores them",
                        ),
                        (
                            "Right-click",
                            "Commands for the clicked idea or current selection",
                        ),
                        ("Scroll / pinch", "Zoom around pointer"),
                    ] {
                        ui.horizontal(|ui| {
                            ui.strong(key);
                            ui.label(purpose);
                        });
                    }
                    ui.separator();
                    ui.weak(
                        "Freeplane import/export is a supported subset, not a lossless conversion.",
                    );
                });
        }
        self.debounce_recovery(&ctx);
        if text_pending || self.workspace.map().editor.editing.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let title = self
            .workspace
            .map()
            .editor
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "{}{} — BMM",
            if self.workspace.map().editor.is_dirty() {
                "* "
            } else {
                ""
            },
            title
        )));
        self.finish_tab_transition(&ctx);
        if self
            .clipboard_gesture
            .as_ref()
            .is_some_and(|gesture| !ctx.input(|input| input.key_down(gesture.key)))
        {
            self.clipboard_gesture = None;
        }
    }

    fn open_palette(&mut self) {
        self.palette_open = true;
        self.palette_query.clear();
        self.palette_focus = true;
    }

    fn open_search(&mut self) {
        let result = self.prepare();
        if result.is_ok() {
            self.workspace.map_mut().search.open = true;
            self.workspace.map_mut().search.request_focus = true;
        }
        self.report(result);
    }

    fn search_panel(&mut self, ui: &mut egui::Ui) {
        let mut selected = None;
        let session = self.workspace.active_id();
        egui::Panel::top("search").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Find an idea");
                let response = ui.add_enabled(
                    self.dialog.is_none() && !self.palette_open,
                    egui::TextEdit::singleline(&mut self.workspace.map_mut().search.query)
                        .id(Id::unique(("search", session)))
                        .hint_text("Search text and notes")
                        .char_limit(1024),
                );
                self.owned_widgets
                    .entry(session)
                    .or_default()
                    .insert(response.id);
                if self.workspace.map().search.request_focus {
                    response.request_focus();
                    self.workspace.map_mut().search.request_focus = false;
                }
                if ui.button("Close").clicked() {
                    self.workspace.map_mut().search.open = false;
                    response.surrender_focus();
                }
            });
            let results = self
                .workspace
                .map()
                .editor
                .search(&self.workspace.map().search.query);
            if !self.workspace.map().search.query.is_empty() {
                ui.weak(format!("{} matches", results.len()));
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for id in results.iter().take(100) {
                            if ui
                                .selectable_label(
                                    self.workspace.map().editor.selected == *id,
                                    &self.workspace.map().editor.document.nodes[id].text,
                                )
                                .clicked()
                            {
                                selected = Some(id.clone());
                            }
                        }
                        if results.len() > 100 {
                            ui.weak("Showing the first 100 matches. Refine your search.");
                        }
                    });
            }
        });
        if let Some(id) = selected {
            let result = self.edit(EditorEdit::Command(Command::Reveal { id }));
            self.report(result);
            self.focus_selected = true;
        }
    }

    fn palette(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }
        let mut action = None;
        egui::Modal::new(Id::unique("command-palette")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("What would you like to do?");
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.palette_query)
                    .desired_width(f32::INFINITY)
                    .hint_text("Type a command...")
                    .char_limit(512),
            );
            if self.palette_focus {
                response.request_focus();
                self.palette_focus = false;
            }
            let matches = palette_matches(&self.palette_query);
            if ctx.input(|input| input.key_pressed(Key::Enter)) {
                action = matches
                    .iter()
                    .find(|command| {
                        self.command_enabled(command.action, &self.workspace.map().editor.selected)
                    })
                    .map(|command| command.action);
            }
            for command in matches {
                if ui
                    .add_enabled(
                        self.command_enabled(command.action, &self.workspace.map().editor.selected),
                        egui::Button::new(command.label).shortcut_text(command.shortcut),
                    )
                    .clicked()
                {
                    action = Some(command.action);
                }
            }
            if ui.button("Cancel").clicked() || ctx.input(|input| input.key_pressed(Key::Escape)) {
                self.palette_open = false;
                response.surrender_focus();
            }
            if action.is_some() {
                response.surrender_focus();
            }
        });
        if let Some(action) = action {
            self.palette_open = false;
            self.run_command(action, ctx);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.drag.is_some() {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, Key::Escape)) {
                self.cancel_canvas_drag();
                self.status = "Drag canceled. The map was not changed.".into();
            }
            return;
        }
        let focused = ctx.memory(|memory| memory.focused());
        if self.composing.is_some() && self.composing != focused {
            self.composing = None;
        }
        let paste_bytes = ctx.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Paste(text) => Some(text.len()),
                    _ => None,
                })
                .fold(0usize, usize::saturating_add)
        });
        if paste_bytes > crate::document::MAX_FILE_BYTES {
            self.error = Some(
                "Backend clipboard paste exceeds 32 MiB; no text or outline was inserted.".into(),
            );
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)))
            });
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        if self.input_after_switch {
            let fresh = events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            });
            if fresh {
                self.input_after_switch = false;
            } else {
                ctx.input_mut(|input| {
                    input.events.retain(|event| {
                        !matches!(
                            event,
                            egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Ime(_)
                        )
                    })
                });
                return;
            }
        }
        let mut preedit = false;
        for event in &events {
            match event {
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    preedit = !text.is_empty();
                    self.composing = if preedit { focused } else { None };
                }
                egui::Event::Ime(egui::ImeEvent::Commit(_)) => {
                    preedit = false;
                    self.composing = None;
                }
                _ => {}
            }
        }
        let blocked = self.palette_open || self.help_open || egui::Popup::is_any_open(ctx);
        let modifiers = ctx.input(|input| input.modifiers);
        let map_focused = focused.is_some_and(|focused| {
            self.placed
                .nodes
                .keys()
                .any(|id| focused == Id::unique(("idea", self.workspace.active_id(), id)))
        });
        let map_owns_input = !ctx.egui_wants_keyboard_input() || map_focused;
        if !blocked
            && self.workspace.map().editor.editing.is_none()
            && map_owns_input
            && !modifiers.command
            && (!modifiers.ctrl || modifiers.alt)
            && events.iter().any(|event| match event {
                egui::Event::Text(text)
                | egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    text.chars().any(|character| !character.is_control())
                }
                _ => false,
            })
        {
            let result = self.edit(EditorEdit::BeginReplacement);
            if result.is_ok() {
                self.edit_focus = None;
                self.focus_selected = true;
                if preedit {
                    self.composing = Some(Id::unique((
                        "inline",
                        self.workspace.active_id(),
                        &self.workspace.map().editor.selected,
                    )));
                }
            }
            self.report(result);
            return;
        }
        let bound = events.iter().find_map(|event| match event {
            egui::Event::Key {
                key, pressed: true, ..
            } => commands::shortcut(*key, modifiers).map(|command| (*key, command)),
            _ => None,
        });
        if self.composing.is_some() || blocked {
            if !blocked
                && let Some((
                    keycode,
                    command @ (AppCommand::NextTab
                    | AppCommand::PreviousTab
                    | AppCommand::New
                    | AppCommand::CloseTab
                    | AppCommand::Quit),
                )) = bound
            {
                ctx.input_mut(|input| input.consume_key(modifiers, keycode));
                self.run_command(command, ctx);
            }
            return;
        }
        let key = |key| ctx.input(|input| input.key_pressed(key));
        if let Some((_, command)) = bound
            && command.is_global_shortcut()
        {
            if let Some((keycode, _)) = bound {
                ctx.input_mut(|input| input.consume_key(modifiers, keycode));
            }
            self.run_command(command, ctx);
            return;
        }
        if key(Key::Escape)
            && self.workspace.map().search.open
            && self.workspace.map().editor.editing.is_none()
        {
            self.workspace.map_mut().search.open = false;
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            return;
        }
        if self.workspace.map().editor.editing.is_some() {
            return;
        }
        if !map_owns_input {
            return;
        }
        if self.clipboard_shortcut(ctx, bound, &events) {
            return;
        }
        if let Some((keycode, command)) = bound {
            ctx.input_mut(|input| input.consume_key(modifiers, keycode));
            self.run_command(command, ctx);
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        let dark = ui.visuals().dark_mode;
        let background = if dark {
            Color32::from_rgb(22, 25, 33)
        } else {
            Color32::WHITE
        };
        ui.painter().rect_filled(available, 0.0, background);
        let canvas_response = ui.allocate_rect(available, Sense::click_and_drag());
        let center = point(available.center());
        if matches!(self.drag, Some(CanvasDrag::Pan { .. })) {
            self.update_drag(ui, available, center);
        }
        if canvas_response.dragged_by(egui::PointerButton::Primary) && self.drag.is_none() {
            let delta = ui.input(|input| input.pointer.delta());
            self.workspace.map_mut().viewport.pan.x += delta.x;
            self.workspace.map_mut().viewport.pan.y += delta.y;
        }
        if canvas_response.hovered()
            && self.workspace.map().editor.editing.is_none()
            && self.drag.is_none()
        {
            let zoom = ui.input(|input| {
                let pinch = input.zoom_delta();
                if (pinch - 1.0).abs() > 0.001 {
                    pinch
                } else {
                    (input.smooth_scroll_delta.y * 0.002).exp()
                }
            });
            if (zoom - 1.0).abs() > 0.0001 {
                let pointer = ui
                    .input(|input| input.pointer.hover_pos())
                    .unwrap_or(available.center());
                self.workspace
                    .map_mut()
                    .viewport
                    .zoom_at(point(pointer), center, zoom);
            }
        }
        if self.layout_revision != Some(self.workspace.map().editor.revision)
            || self.layout_session != Some(self.workspace.active_id())
            || self.cached_dark != dark
        {
            self.galleys.clear();
            let mut sizes = BTreeMap::new();
            for (id, node) in &self.workspace.map().editor.document.nodes {
                let root = id == &self.workspace.map().editor.document.root;
                let galley = ui.painter().layout(
                    node.text.clone(),
                    FontId::proportional(if root { 22.0 } else { 16.0 }),
                    if dark {
                        Color32::from_gray(240)
                    } else {
                        Color32::BLACK
                    },
                    260.0,
                );
                sizes.insert(
                    id.clone(),
                    Size {
                        width: (galley.size().x + 44.0).max(if root { 190.0 } else { 110.0 }),
                        height: (galley.size().y + 26.0).max(48.0),
                    },
                );
                self.galleys.insert(id.clone(), galley);
            }
            match layout::layout(&self.workspace.map().editor.document, &sizes) {
                Ok(placed) => {
                    self.branch_colors.clear();
                    for child in &self.workspace.map().editor.document.nodes
                        [&self.workspace.map().editor.document.root]
                        .children
                    {
                        self.branch_colors
                            .insert(child.clone(), appearance::branch_color(child));
                    }
                    for (parent, child) in &placed.edges {
                        if parent != &self.workspace.map().editor.document.root {
                            self.branch_colors
                                .insert(child.clone(), self.branch_colors[parent]);
                        }
                    }
                    self.placed = placed;
                }
                Err(error) => self.error = Some(error.to_string()),
            }
            self.layout_revision = Some(self.workspace.map().editor.revision);
            self.cached_dark = dark;
            self.layout_session = Some(self.workspace.active_id());
        }
        if self.fit_requested {
            self.workspace.map_mut().viewport.fit(
                self.placed.bounds,
                Size {
                    width: available.width(),
                    height: available.height(),
                },
            );
            self.fit_requested = false;
        }
        if self.focus_selected {
            if let Some(rect) = self.placed.nodes.get(&self.workspace.map().editor.selected) {
                let screen = screen_rect(self.workspace.map().viewport, *rect, center);
                if !available.shrink(60.0).contains_rect(screen) {
                    self.workspace.map_mut().viewport.focus(rect.center());
                }
            }
            self.focus_selected = false;
        }
        for (parent, child) in &self.placed.edges {
            let a = screen_rect(
                self.workspace.map().viewport,
                self.placed.nodes[parent],
                center,
            );
            let b = screen_rect(
                self.workspace.map().viewport,
                self.placed.nodes[child],
                center,
            );
            let right = b.center().x > a.center().x;
            let mut start = if right {
                a.right_center()
            } else {
                a.left_center()
            };
            if parent != &self.workspace.map().editor.document.root {
                start.y = a.bottom() - 3.0 * self.workspace.map().viewport.zoom;
            }
            let end = Pos2::new(
                if right { b.left() } else { b.right() },
                b.bottom() - 3.0 * self.workspace.map().viewport.zoom,
            );
            if !a.union(b).intersects(available) {
                continue;
            }
            let middle = (start.x + end.x) * 0.5;
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [
                        start,
                        Pos2::new(middle, start.y),
                        Pos2::new(middle, end.y),
                        end,
                    ],
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(
                        1.5 * self.workspace.map().viewport.zoom,
                        self.branch_colors[child],
                    ),
                ));
        }
        let mut clicked = None;
        let mut edit = false;
        let mut fold = None;
        let mut context_command = None;
        let mut drag_started = None;
        let inline_composing = self.composing.is_some()
            || ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Ime(_)))
            });
        let mut edit_key = self
            .workspace
            .map()
            .editor
            .editing
            .as_ref()
            .and_then(|buffer| {
                if inline_composing || !ui.is_enabled() {
                    return None;
                }
                let key = ui.input(|input| {
                    if input.key_pressed(Key::Escape) {
                        Some(EditorKey::Escape)
                    } else if input.key_pressed(Key::Tab) {
                        Some(EditorKey::Tab)
                    } else if input.key_pressed(Key::Enter) {
                        Some(EditorKey::Enter)
                    } else {
                        None
                    }
                });
                key.map(|key| {
                    (
                        key,
                        Id::unique(("inline", self.workspace.active_id(), &buffer.id)),
                    )
                })
            });
        let ids: Vec<_> = self.placed.nodes.keys().cloned().collect();
        for id in ids {
            let rect = screen_rect(
                self.workspace.map().viewport,
                self.placed.nodes[&id],
                center,
            );
            if !rect.intersects(available) {
                continue;
            }
            let selected = self.workspace.map().editor.is_selected(&id);
            let root = id == self.workspace.map().editor.document.root;
            if root {
                ui.painter().add(egui::epaint::EllipseShape {
                    center: rect.center(),
                    radius: rect.size() * 0.5,
                    angle: 0.0,
                    fill: background,
                    stroke: Stroke::new(1.4, Color32::from_gray(if dark { 155 } else { 140 })),
                });
            }
            if selected {
                let highlight = if root {
                    egui::Rect::from_center_size(
                        rect.center(),
                        self.galleys[&id].size() * self.workspace.map().viewport.zoom
                            + Vec2::splat(8.0),
                    )
                } else {
                    rect.shrink(3.0 * self.workspace.map().viewport.zoom)
                };
                ui.painter().rect_filled(
                    highlight,
                    2.0,
                    if dark {
                        Color32::from_rgb(35, 62, 87)
                    } else {
                        Color32::from_rgb(218, 236, 255)
                    },
                );
            }
            if !root {
                let y = rect.bottom() - 3.0 * self.workspace.map().viewport.zoom;
                ui.painter().line_segment(
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                    Stroke::new(
                        1.5 * self.workspace.map().viewport.zoom,
                        self.branch_colors[&id],
                    ),
                );
            }
            if self
                .workspace
                .map()
                .editor
                .editing
                .as_ref()
                .is_some_and(|buffer| buffer.id == id)
            {
                let first_focus = self.edit_focus.as_ref() != Some(&id);
                let zoom = self.workspace.map().viewport.zoom;
                let inline_id = Id::unique(("inline", self.workspace.active_id(), &id));
                self.owned_widgets
                    .entry(self.workspace.active_id())
                    .or_default()
                    .insert(inline_id);
                let buffer = self
                    .workspace
                    .map()
                    .editor
                    .editing
                    .as_ref()
                    .expect("editing node");
                let mut text = buffer.text.clone();
                if first_focus {
                    ui.memory_mut(|memory| memory.request_focus(inline_id));
                    let mut state = egui::text_edit::TextEditState::default();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(text.chars().count()),
                        )));
                    state.store(ui.ctx(), inline_id);
                    self.edit_focus = Some(id.clone());
                }
                let response = ui.put(
                    rect.shrink(7.0),
                    egui::TextEdit::singleline(&mut text)
                        .id(inline_id)
                        .font(FontId::proportional(16.0 * zoom))
                        .char_limit(262_144),
                );
                if response.changed() {
                    let result = self.edit(EditorEdit::SetDraft {
                        field: DraftField::Inline(id.clone()),
                        text,
                    });
                    self.report(result);
                }
                if !inline_composing
                    && ui.is_enabled()
                    && edit_key.is_none()
                    && response.lost_focus()
                    && !first_focus
                    && self.workspace.pending_activation().is_none()
                    && self.pending_close.is_none()
                    && !self.pending_quit
                {
                    edit_key = Some((EditorKey::Enter, response.id));
                }
            } else {
                let node = &self.workspace.map().editor.document.nodes[&id];
                let response = ui.interact(
                    rect,
                    Id::unique(("idea", self.workspace.active_id(), &id)),
                    Sense::click_and_drag(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::TreeItem,
                        ui.is_enabled(),
                        selected,
                        &node.text,
                    )
                });
                if response.clicked() {
                    clicked = Some((id.clone(), ui.input(|input| input.modifiers.shift), false));
                }
                if response.secondary_clicked() {
                    clicked = Some((id.clone(), false, true));
                }
                if response.double_clicked() {
                    clicked = Some((id.clone(), false, false));
                    edit = true;
                }
                if self.drag.is_none() && response.drag_started_by(egui::PointerButton::Primary) {
                    response.request_focus();
                    drag_started = Some(id.clone());
                }
                response.context_menu(|ui| {
                    if let Some(command) = self.context_menu(ui, &id, true) {
                        context_command = Some(command);
                        clicked = Some((id.clone(), false, true));
                    }
                });
                response.on_hover_text(if root {
                    "Drag: pan the whole map. Double-click: edit. Right-click: commands."
                } else {
                    "Double-click: edit. Drag: reorganize. Shift-click: multi-select. Right-click: commands."
                });
                if let Some(galley) = self.galleys.get(&id) {
                    let text_pos =
                        rect.center() - galley.size() * self.workspace.map().viewport.zoom * 0.5;
                    let mut shape = egui::epaint::TextShape::new(
                        Pos2::ZERO,
                        galley.clone(),
                        if dark {
                            Color32::from_gray(240)
                        } else {
                            Color32::BLACK
                        },
                    );
                    shape.transform(egui::emath::TSTransform::new(
                        text_pos.to_vec2(),
                        self.workspace.map().viewport.zoom,
                    ));
                    ui.painter().add(shape);
                }
            }
            let node = &self.workspace.map().editor.document.nodes[&id];
            if !node.children.is_empty() {
                let right =
                    rect.center().x >= available.center().x + self.workspace.map().viewport.pan.x;
                let edge = if right {
                    rect.right_center()
                } else {
                    rect.left_center()
                };
                let fold_rect = egui::Rect::from_center_size(
                    edge + Vec2::new(if right { 11.0 } else { -11.0 }, 0.0),
                    Vec2::splat(20.0),
                );
                let response = ui.put(
                    fold_rect,
                    egui::Button::new(if node.folded { "+" } else { "-" }).small(),
                );
                if response.clicked() {
                    fold = Some(id.clone());
                }
            }
        }
        if self.workspace.map().editor.editing.is_none() {
            self.edit_focus = None;
        }
        if let Some((key, focus)) = edit_key {
            let result = self.edit(EditorEdit::Key(key));
            self.report(result);
            if self.workspace.map().editor.editing.is_none() {
                ui.memory_mut(|memory| memory.surrender_focus(focus));
            }
            self.edit_focus = None;
            self.focus_selected = true;
        }
        if let Some((id, toggle, preserve)) = clicked {
            let result = self.edit(if toggle {
                EditorEdit::ToggleSelection(id)
            } else if preserve {
                EditorEdit::ActivateSelection(id)
            } else {
                EditorEdit::Select(id)
            });
            self.report(result);
            if edit {
                self.action(EditorAction::Edit);
            }
        } else if canvas_response.clicked() {
            let result = self.prepare();
            self.report(result);
            ui.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        if let Some(id) = fold {
            let result = self.edit(EditorEdit::Command(Command::ToggleFold { id }));
            self.report(result);
        }
        if let Some(id) = drag_started {
            let result = self.edit(EditorEdit::Select(id.clone()));
            if result.is_ok() {
                if id == self.workspace.map().editor.document.root {
                    if let Some(origin) = ui.input(|input| input.pointer.press_origin()) {
                        self.drag = Some(CanvasDrag::Pan {
                            origin: point(origin),
                            initial_pan: self.workspace.map().viewport.pan,
                        });
                        self.status =
                            "Repositioning the whole map. Escape restores the starting view."
                                .into();
                    } else {
                        self.error = Some(
                            "Cannot start map panning without the pointer's press position.".into(),
                        );
                    }
                } else {
                    self.drag = Some(CanvasDrag::Branch(DragSession::new(
                        &self.workspace.map().editor.document,
                        id,
                        self.workspace.map().editor.revision,
                    )));
                    self.status = "Drag to the middle of a topic for a child, or its top/bottom to reorder. Escape cancels.".into();
                }
            }
            self.report(result);
        }
        self.update_drag(ui, available, center);
        canvas_response.context_menu(|ui| {
            context_command = self
                .context_menu(ui, &self.workspace.map().editor.selected, false)
                .or(context_command);
        });
        if let Some(command) = context_command {
            self.run_command(command, ui.ctx());
        }
        if self.workspace.map().editor.document.nodes.len() == 1
            && self.workspace.map().editor.editing.is_none()
        {
            ui.painter().text(
                available.center() + Vec2::new(0.0, 100.0),
                egui::Align2::CENTER_TOP,
                "Start with one idea. Press Tab to let it grow.",
                FontId::proportional(15.0),
                ui.visuals().weak_text_color(),
            );
        }
    }

    fn update_drag(&mut self, ui: &mut egui::Ui, available: egui::Rect, center: Point) {
        if !ui.is_enabled()
            || self
                .drag
                .as_ref()
                .is_some_and(|drag| matches!(drag, CanvasDrag::Branch(branch) if branch.revision != self.workspace.map().editor.revision))
            || ui.input(|input| input.viewport().focused == Some(false))
        {
            if self.drag.take().is_some() {
                self.status =
                    "Move canceled because the map, window focus or active dialog changed.".into();
            }
            return;
        }
        if let Some(CanvasDrag::Pan {
            origin,
            initial_pan,
        }) = &self.drag
        {
            if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
                let next = Point {
                    x: initial_pan.x + pointer.x - origin.x,
                    y: initial_pan.y + pointer.y - origin.y,
                };
                if next != self.workspace.map().viewport.pan {
                    self.workspace.map_mut().viewport.pan = next;
                    ui.ctx().request_repaint();
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if !ui.input(|input| input.pointer.button_down(egui::PointerButton::Primary)) {
                self.drag = None;
                self.status = "Map repositioned. Its structure is unchanged.".into();
            }
            return;
        }
        let Some(CanvasDrag::Branch(drag)) = &mut self.drag else {
            return;
        };
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let world = pointer
            .filter(|pos| available.contains(*pos))
            .map(|pos| self.workspace.map().viewport.to_world(point(pos), center));
        drag.update(
            &self.workspace.map().editor.document,
            &self.placed,
            world,
            24.0 / self.workspace.map().viewport.zoom,
        );
        if ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary)) {
            let Some(CanvasDrag::Branch(drag)) = self.drag.take() else {
                unreachable!("Active branch drag");
            };
            match drag.preview {
                Some(preview) if preview.rejection.is_none() => {
                    let before = self.workspace.map().editor.revision;
                    let result = self.edit(EditorEdit::Command(Command::MoveNode {
                        id: drag.source,
                        placement: preview.placement,
                    }));
                    if result.is_ok() {
                        self.focus_selected = true;
                        self.status = if before == self.workspace.map().editor.revision {
                            "Already in that position."
                        } else {
                            "Branch moved. Undo restores its previous position."
                        }
                        .into();
                    }
                    self.report(result);
                }
                Some(preview) => self.error = preview.rejection,
                None => {
                    self.status =
                        "Move canceled: drop near a topic, or across the root to switch sides."
                            .into()
                }
            }
            return;
        }
        if !ui.input(|input| input.pointer.button_down(egui::PointerButton::Primary)) {
            self.drag = None;
            self.status =
                "Move canceled because the pointer was released outside the window.".into();
            return;
        }
        let Some(CanvasDrag::Branch(drag)) = &self.drag else {
            unreachable!("Active branch drag");
        };
        let rejected = drag
            .preview
            .as_ref()
            .is_none_or(|preview| preview.rejection.is_some());
        let accent = if rejected {
            Color32::from_rgb(207, 67, 76)
        } else {
            Color32::from_rgb(38, 105, 220)
        };
        ui.ctx().set_cursor_icon(if rejected {
            egui::CursorIcon::NoDrop
        } else {
            egui::CursorIcon::Grabbing
        });
        if let Some(preview) = &drag.preview {
            let rect = screen_rect(
                self.workspace.map().viewport,
                self.placed.nodes[&preview.target],
                center,
            );
            match preview.placement {
                MovePlacement::Before(_) | MovePlacement::After(_) => {
                    let y = if matches!(preview.placement, MovePlacement::Before(_)) {
                        rect.top() - 5.0
                    } else {
                        rect.bottom() + 5.0
                    };
                    ui.painter().line_segment(
                        [
                            Pos2::new(rect.left() - 5.0, y),
                            Pos2::new(rect.right() + 5.0, y),
                        ],
                        Stroke::new(3.0, accent),
                    );
                    ui.painter()
                        .circle_filled(Pos2::new(rect.left() - 5.0, y), 3.0, accent);
                }
                MovePlacement::Child(_) | MovePlacement::RootSide(_) => {
                    ui.painter().rect_stroke(
                        rect.expand(4.0),
                        6.0,
                        Stroke::new(2.5, accent),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
        if let Some(pointer) = pointer {
            let title = ui.painter().layout(
                drag.title.clone(),
                FontId::proportional(14.0),
                ui.visuals().strong_text_color(),
                260.0,
            );
            let label = ui.painter().layout(
                drag.preview
                    .as_ref()
                    .map_or("Drop near a topic; Escape cancels", |preview| {
                        preview.label(&self.workspace.map().editor.document)
                    })
                    .into(),
                FontId::proportional(12.0),
                accent,
                260.0,
            );
            let size = Vec2::new(
                title.size().x.max(label.size().x) + 24.0,
                title.size().y + label.size().y + 28.0,
            );
            let position = Pos2::new(
                (pointer.x + 18.0)
                    .min(available.right() - size.x)
                    .max(available.left()),
                (pointer.y + 22.0)
                    .min(available.bottom() - size.y)
                    .max(available.top()),
            );
            let rect = egui::Rect::from_min_size(position, size);
            ui.painter().rect_filled(
                rect.translate(Vec2::new(0.0, 3.0)),
                8.0,
                Color32::from_black_alpha(18),
            );
            ui.painter().rect_filled(rect, 8.0, ui.visuals().panel_fill);
            ui.painter().rect_stroke(
                rect,
                8.0,
                Stroke::new(1.0, accent),
                egui::StrokeKind::Inside,
            );
            let label_y = title.size().y + 16.0;
            ui.painter().galley(
                position + Vec2::splat(12.0),
                title,
                ui.visuals().strong_text_color(),
            );
            ui.painter()
                .galley(position + Vec2::new(12.0, label_y), label, accent);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        egui::Modal::new(Id::unique("file-dialog")).show(ctx, |ui| {
            ui.set_width(470.0);
            match &mut dialog {
                Dialog::MermaidExport { session, bytes } => {
                    ui.heading("Export Mermaid mindmap");
                    ui.label("Exports the captured labels and complete hierarchy, including unfinished text and folded topics. Notes, links, IDs, styling and BMM layout do not transfer. Mermaid determines presentation; the native map and its edit history stay unchanged.");
                    ui.horizontal(|ui| {
                        if ui.button("Choose .mmd file...").clicked() {
                            keep = false;
                            let path = rfd::FileDialog::new().add_filter("Mermaid mindmap", &["mmd"]).set_file_name("Ideas.mmd").save_file();
                            self.stage_export(*session, path, std::mem::take(bytes), ctx);
                        }
                        if ui.button("Cancel").clicked() { keep = false; }
                    });
                }
                Dialog::Paste(pending) => { keep = self.paste_dialog(ui, pending); }
                Dialog::CopyOutline(copy) => { keep = self.copy_dialog(ui, copy); }
                Dialog::CloseDecision(id) => {
                    ui.heading("Keep these ideas?");
                    let title = self.workspace.order().iter().position(|candidate| candidate == id)
                        .map(|index| format!("Untitled {}", index + 1)).unwrap_or_else(|| "Missing map".into());
                    let path = match self.workspace.entry(*id) {
                        Ok(crate::session::SessionState::Map(map)) => map.editor.path.as_ref(),
                        Ok(crate::session::SessionState::RestoreError(error)) => error.path.as_ref(),
                        Err(_) => None,
                    };
                    ui.strong(path.map_or(title, |path| path.display().to_string()));
                    ui.label("Save this map, discard its unsaved work, or cancel the whole operation.");
                    ui.horizontal(|ui| {
                        for (label, decision) in [("Save", CloseDecision::Save), ("Discard changes", CloseDecision::Discard), ("Cancel", CloseDecision::Cancel)] {
                            let enabled = label != "Save" || self.workspace.session(*id).is_ok();
                            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                                keep = false;
                                let progress = self.workspace.resolve(decision).map_err(EditorError::from);
                                self.drive_transition(progress, ctx);
                            }
                        }
                    });
                }
                Dialog::Import => {
                    ui.heading("Import Freeplane map");
                    ui.label("Original files stay untouched. The imported document saves as BMM JSON (.branchline).");
                    if let Some(report) = &self.pending_import {
                        if report.warnings.is_empty() { ui.label("No unsupported fields detected within this supported subset."); }
                        else {
                            ui.colored_label(Color32::from_rgb(210, 140, 60), "These fields will not be carried into the new document:");
                            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                                for warning in &report.warnings {
                                    ui.label(format!("{}: {} occurrence(s); locations: {}", warning.category, warning.count, warning.locations.join(", ")));
                                    if warning.count > warning.locations.len() { ui.weak("Location preview limited to 8 occurrences."); }
                                }
                            });
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Accept supported data").clicked() {
                            let result = self.pending_import.clone().ok_or_else(|| EditorError::State("No import preview is staged.".into())).and_then(|report| self.workspace.accept_import(report).map(|_| ()).map_err(EditorError::from));
                            if result.is_ok() { keep = false; self.pending_import = None; }
                            self.report(result);
                        }
                        if ui.button("Cancel").clicked() { keep = false; self.pending_import = None; }
                    });
                }
                Dialog::Delete(plan) => {
                    ui.heading("Delete selected branches?");
                    ui.label(format!("Remove {} branch(es), containing {} idea(s) in total. Undo restores the entire operation.", plan.roots.len(), plan.node_count));
                    if plan.root_retained { ui.label("The central idea will be kept."); }
                    ui.horizontal(|ui| {
                        if ui.button("Delete selected branches").clicked() {
                            let result = self.edit(EditorEdit::Command(Command::DeleteMany { ids: plan.roots.clone() }));
                            if result.is_ok() {
                                self.status = if plan.root_retained { "Selected branches deleted. The central idea was kept. Undo restores the deletion." }
                                    else { "Selected branches deleted. Undo restores the deletion." }.into();
                            }
                            self.report(result);
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() { keep = false; }
                    });
                }
                Dialog::Overwrite { session, path, export } => {
                    ui.heading("Replace this file?");
                    ui.label(path.display().to_string());
                    ui.label("The existing file will be atomically replaced after the new data is written.");
                    ui.horizontal(|ui| {
                        if ui.button("Replace file").clicked() { keep = false; self.publish(*session, path.clone(), export.clone(), ctx); }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                            self.abort_transition();
                        }
                    });
                }
            }
        });
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }
}

impl eframe::App for NativeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

fn point(position: Pos2) -> Point {
    Point {
        x: position.x,
        y: position.y,
    }
}
fn screen_rect(viewport: Viewport, rect: Rect, center: Point) -> egui::Rect {
    let min = viewport.to_screen(rect.min, center);
    let max = viewport.to_screen(rect.max, center);
    egui::Rect::from_min_max(Pos2::new(min.x, min.y), Pos2::new(max.x, max.y))
}

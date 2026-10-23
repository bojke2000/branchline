use crate::{
    app::{Direction, Editor, EditorAction, EditorError, EditorKey, PendingAction},
    document::{Command, Document, NodeId},
    interchange::{export_mm, import_mm, read_native},
    layout::{self, Layout, Point, Rect, Size},
    storage::{RecoveryRecord, RecoveryStore, StorageError, atomic_write, read_bounded},
};
use eframe::egui::{self, Color32, FontId, Id, Key, Pos2, Sense, Stroke, Vec2};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct Viewport {
    pub pan: Point,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            pan: Point::default(),
            zoom: 1.0,
        }
    }
}

impl Viewport {
    pub fn to_screen(self, point: Point, center: Point) -> Point {
        Point {
            x: center.x + self.pan.x + point.x * self.zoom,
            y: center.y + self.pan.y + point.y * self.zoom,
        }
    }
    pub fn to_world(self, point: Point, center: Point) -> Point {
        Point {
            x: (point.x - center.x - self.pan.x) / self.zoom,
            y: (point.y - center.y - self.pan.y) / self.zoom,
        }
    }
    pub fn zoom_at(&mut self, pointer: Point, center: Point, factor: f32) {
        let world = self.to_world(pointer, center);
        self.zoom = (self.zoom * factor).clamp(0.05, 3.0);
        self.pan = Point {
            x: pointer.x - center.x - world.x * self.zoom,
            y: pointer.y - center.y - world.y * self.zoom,
        };
    }
    pub fn fit(&mut self, bounds: Rect, available: Size) {
        let width = (bounds.max.x - bounds.min.x).max(1.0);
        let height = (bounds.max.y - bounds.min.y).max(1.0);
        self.zoom = ((available.width - 80.0).max(1.0) / width)
            .min((available.height - 80.0).max(1.0) / height)
            .clamp(0.01, 1.2);
        self.focus(bounds.center());
    }
    pub fn focus(&mut self, point: Point) {
        self.pan = Point {
            x: -point.x * self.zoom,
            y: -point.y * self.zoom,
        };
    }
}

#[derive(Clone, Copy)]
pub struct PaletteCommand {
    pub label: &'static str,
    pub shortcut: &'static str,
    pub action: EditorAction,
}

pub fn palette_matches(query: &str) -> Vec<PaletteCommand> {
    let query = query.to_lowercase();
    [
        ("Add child", "Tab", EditorAction::AddChild),
        ("Add sibling", "Enter", EditorAction::AddSibling),
        ("Edit idea", "F2", EditorAction::Edit),
        ("Fold / unfold branch", "Space", EditorAction::Fold),
        ("Move earlier", "Alt+Up", EditorAction::MoveUp),
        ("Move later", "Alt+Down", EditorAction::MoveDown),
        (
            "Indent under previous sibling",
            "Alt+Right",
            EditorAction::Indent,
        ),
        ("Outdent to parent level", "Alt+Left", EditorAction::Outdent),
        ("Undo", "Cmd/Ctrl+Z", EditorAction::Undo),
        ("Redo", "Cmd/Ctrl+Shift+Z", EditorAction::Redo),
        ("Delete idea", "Delete", EditorAction::Delete),
    ]
    .into_iter()
    .filter(|(label, _, _)| label.to_lowercase().contains(&query))
    .map(|(label, shortcut, action)| PaletteCommand {
        label,
        shortcut,
        action,
    })
    .collect()
}

enum Dialog {
    Unsaved,
    Recovery,
    Import,
    Delete(NodeId),
    Overwrite {
        path: PathBuf,
        export: Option<Vec<u8>>,
    },
}

#[derive(Clone)]
struct Details {
    id: NodeId,
    notes: String,
    link: String,
}

pub struct NativeApp {
    pub editor: Editor,
    store: RecoveryStore,
    recovery: Option<RecoveryRecord>,
    dialog: Option<Dialog>,
    error: Option<String>,
    status: String,
    viewport: Viewport,
    placed: Layout,
    galleys: BTreeMap<NodeId, Arc<egui::Galley>>,
    layout_revision: Option<u64>,
    cached_dark: bool,
    focus_selected: bool,
    fit_requested: bool,
    notes_open: bool,
    details: Option<Details>,
    search_open: bool,
    search_query: String,
    search_focus: bool,
    palette_open: bool,
    palette_query: String,
    palette_focus: bool,
    help_open: bool,
    edit_focus: Option<NodeId>,
    composing: Option<Id>,
    allow_close: bool,
    recovery_revision: Option<(u64, u64)>,
    last_change: Instant,
}

impl NativeApp {
    pub fn new(store: RecoveryStore) -> Result<Self, StorageError> {
        let recovery = store.load()?;
        let dialog = recovery.as_ref().map(|_| Dialog::Recovery);
        Ok(Self {
            editor: Editor::new(),
            store,
            recovery,
            dialog,
            error: None,
            status: "Local by design. Your ideas stay on this computer.".into(),
            viewport: Viewport::default(),
            placed: Layout::default(),
            galleys: BTreeMap::new(),
            layout_revision: None,
            cached_dark: false,
            focus_selected: false,
            fit_requested: false,
            notes_open: false,
            details: None,
            search_open: false,
            search_query: String::new(),
            search_focus: false,
            palette_open: false,
            palette_query: String::new(),
            palette_focus: false,
            help_open: false,
            edit_focus: None,
            composing: None,
            allow_close: false,
            recovery_revision: None,
            last_change: Instant::now(),
        })
    }

    fn report(&mut self, result: Result<(), EditorError>) {
        if let Err(error) = result {
            self.error = Some(error.to_string());
        }
    }

    fn flush_details(&mut self) -> Result<(), EditorError> {
        if let Some(details) = self.details.clone() {
            let selected = self.editor.selected.clone();
            if self.editor.document.nodes.contains_key(&details.id) {
                self.editor.execute(Command::SetNotes {
                    id: details.id.clone(),
                    notes: details.notes,
                })?;
                self.editor.execute(Command::SetLink {
                    id: details.id,
                    link: details.link,
                })?;
            }
            if self.editor.document.nodes.contains_key(&selected) {
                self.editor.selected = selected;
            }
            self.details = None;
        }
        Ok(())
    }

    fn has_drafts(&self) -> bool {
        self.editor
            .editing
            .as_ref()
            .is_some_and(|buffer| self.editor.document.nodes[&buffer.id].text != buffer.text)
            || self.details.as_ref().is_some_and(|details| {
                self.editor
                    .document
                    .nodes
                    .get(&details.id)
                    .is_some_and(|node| node.notes != details.notes || node.link != details.link)
            })
    }

    pub fn checkpoint_recovery(&mut self) -> Result<(), EditorError> {
        if !self.editor.is_dirty() && !self.has_drafts() {
            return self.clear_recovery();
        }
        let mut document = self.editor.document.clone();
        if let Some(buffer) = &self.editor.editing
            && let Some(node) = document.nodes.get_mut(&buffer.id)
        {
            node.text = buffer.text.clone();
        }
        if let Some(details) = &self.details
            && let Some(node) = document.nodes.get_mut(&details.id)
        {
            node.notes = details.notes.clone();
            node.link = details.link.clone();
        }
        let record = RecoveryRecord {
            version: 1,
            document_id: self.editor.document_id.clone(),
            original_path: self.editor.path.clone(),
            document,
        };
        self.store.write(&record)?;
        Ok(())
    }

    fn prepare(&mut self) -> Result<(), EditorError> {
        self.flush_details()?;
        self.editor.commit_edit()
    }

    fn action(&mut self, action: EditorAction) {
        let result = self.flush_details().and_then(|_| {
            if matches!(action, EditorAction::Delete) {
                if self.editor.selected == self.editor.document.root {
                    return Err(EditorError::State(
                        "The central idea cannot be deleted".into(),
                    ));
                }
                if !self.editor.document.nodes[&self.editor.selected]
                    .children
                    .is_empty()
                {
                    self.dialog = Some(Dialog::Delete(self.editor.selected.clone()));
                    return Ok(());
                }
            }
            self.editor.dispatch(action)?;
            self.focus_selected = true;
            Ok(())
        });
        self.report(result);
    }

    fn request(&mut self, action: PendingAction, ctx: &egui::Context) {
        let result = self.prepare().and_then(|_| self.editor.request(action));
        match result {
            Ok(true) => self.perform_pending(ctx),
            Ok(false) => self.dialog = Some(Dialog::Unsaved),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    fn clear_recovery(&self) -> Result<(), EditorError> {
        self.store
            .discard(&self.editor.document_id)
            .map_err(EditorError::from)
    }

    fn reset_view(&mut self) {
        self.viewport = Viewport::default();
        self.layout_revision = None;
        self.details = None;
        self.recovery_revision = None;
        self.fit_requested = true;
    }

    fn perform_pending(&mut self, ctx: &egui::Context) {
        let Some(action) = self.editor.pending.clone() else {
            return;
        };
        let result = (|| -> Result<(), EditorError> {
            match action {
                PendingAction::New => {
                    self.clear_recovery()?;
                    self.editor.replace_document(
                        Document::new("Central idea"),
                        None,
                        true,
                        None,
                    )?;
                    self.reset_view();
                }
                PendingAction::Open(path) => {
                    let bytes = read_bounded(&path)?;
                    if path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("mm"))
                    {
                        let report = import_mm(&bytes)?;
                        self.editor.stage_import(report);
                        self.dialog = Some(Dialog::Import);
                    } else {
                        let document = read_native(&bytes)?;
                        self.clear_recovery()?;
                        self.editor
                            .replace_document(document, Some(path), true, None)?;
                        self.reset_view();
                    }
                }
                PendingAction::Quit => {
                    self.clear_recovery()?;
                    self.editor.pending = None;
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.editor.cancel_pending();
        }
        self.report(result);
    }

    fn open_file(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Mind maps", &["branchline", "mm"])
            .pick_file()
        {
            self.request(PendingAction::Open(path), ctx);
        }
    }

    fn save_file(&mut self, save_as: bool, ctx: &egui::Context) {
        if let Err(error) = self.prepare() {
            self.error = Some(error.to_string());
            return;
        }
        let path = if save_as || self.editor.path.is_none() {
            rfd::FileDialog::new()
                .add_filter("Branchline", &["branchline"])
                .set_file_name("Ideas.branchline")
                .save_file()
        } else {
            self.editor.path.clone()
        };
        if let Some(path) = path {
            if path.exists() {
                self.dialog = Some(Dialog::Overwrite { path, export: None });
            } else {
                self.publish(path, None, ctx);
            }
        }
    }

    fn publish(&mut self, path: PathBuf, export: Option<Vec<u8>>, ctx: &egui::Context) {
        let is_export = export.is_some();
        let result = if let Some(bytes) = export {
            atomic_write(&path, &bytes).map_err(EditorError::from)
        } else {
            self.editor
                .save_to(&path)
                .and_then(|_| self.clear_recovery())
        };
        match result {
            Ok(()) => {
                self.status = format!(
                    "{} {}",
                    if is_export { "Exported" } else { "Saved" },
                    path.display()
                );
                if !is_export && self.editor.pending.is_some() {
                    self.perform_pending(ctx);
                }
            }
            Err(error) => {
                self.error = Some(error.to_string());
                if self.editor.pending.is_some() {
                    self.dialog = Some(Dialog::Unsaved);
                }
            }
        }
    }

    fn export_file(&mut self, ctx: &egui::Context) {
        let result = self
            .prepare()
            .and_then(|_| export_mm(&self.editor.document).map_err(EditorError::from));
        match result {
            Ok(report) => {
                let accepted = rfd::MessageDialog::new().set_title("Export supported Freeplane data")
                    .set_description("Exports node text, order, IDs, folding, branch sides, plain notes and links only. Formatting, rich content, icons, cross-links and scripts from earlier imports are not recoverable. The current Branchline document will not be replaced.")
                    .set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
                if accepted
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Freeplane", &["mm"])
                        .set_file_name("Ideas.mm")
                        .save_file()
                {
                    if path.exists() {
                        self.dialog = Some(Dialog::Overwrite {
                            path,
                            export: Some(report.bytes),
                        });
                    } else {
                        self.publish(path, Some(report.bytes), ctx);
                    }
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub fn show(&mut self, root_ui: &mut egui::Ui) {
        let ctx = root_ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.dialog.is_none() {
                self.request(PendingAction::Quit, &ctx);
            }
        }
        let before = self.editor.revision;
        let draft_before = self.editor.draft_revision;
        if self.dialog.is_none() {
            self.shortcuts(&ctx);
        }
        let text_pending = self.editor.editing.is_some();
        let blocked = self.dialog.is_some();
        egui::Panel::top("toolbar").show(root_ui, |ui| {
            ui.add_space(8.0);
            ui.add_enabled_ui(!blocked, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("branchline").strong().size(21.0));
                    ui.add_space(18.0);
                    if ui.button("New").clicked() {
                        self.request(PendingAction::New, &ctx);
                    }
                    if ui.button("Open").clicked() {
                        self.open_file(&ctx);
                    }
                    if ui.button("Save").clicked() {
                        self.save_file(false, &ctx);
                    }
                    ui.menu_button("More", |ui| {
                        if ui.button("Save as...").clicked() {
                            self.save_file(true, &ctx);
                            ui.close();
                        }
                        if ui.button("Export Freeplane...").clicked() {
                            self.export_file(&ctx);
                            ui.close();
                        }
                        if ui.button("Keyboard guide").clicked() {
                            self.help_open = true;
                            ui.close();
                        }
                    });
                    ui.separator();
                    if ui.button("Undo").clicked() {
                        self.action(EditorAction::Undo);
                    }
                    if ui.button("Redo").clicked() {
                        self.action(EditorAction::Redo);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        if ui
                            .button(if ui.visuals().dark_mode {
                                "Light"
                            } else {
                                "Dark"
                            })
                            .clicked()
                        {
                            ctx.set_theme(if ui.visuals().dark_mode {
                                egui::Theme::Light
                            } else {
                                egui::Theme::Dark
                            });
                        }
                        if ui.button("Notes").clicked() {
                            if self.notes_open {
                                let result = self.flush_details();
                                self.report(result);
                            }
                            self.notes_open = !self.notes_open;
                        }
                        if ui.button("Commands").clicked() {
                            self.open_palette();
                        }
                        if ui.button("Search").clicked() {
                            self.open_search();
                        }
                    });
                });
            });
            ui.add_space(8.0);
        });
        egui::Panel::bottom("status").show(root_ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} ideas  /  {}%{}",
                    self.editor.document.nodes.len(),
                    (self.viewport.zoom * 100.0).round(),
                    if self.editor.is_dirty() {
                        "  /  Unsaved changes"
                    } else {
                        ""
                    }
                ));
                ui.separator();
                ui.weak("Tab: child   Enter: sibling   F2: edit   Space: fold");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Fit map").clicked() {
                        self.fit_requested = true;
                    }
                    if ui.button("Focus").clicked()
                        && let Some(rect) = self.placed.nodes.get(&self.editor.selected)
                    {
                        self.viewport.focus(rect.center());
                    }
                });
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
            if self.editor.history.evicted {
                ui.weak("Old undo steps were evicted to keep history within its memory budget.");
            }
            ui.add_space(4.0);
        });
        if self.search_open {
            self.search_panel(root_ui);
        }
        if self.notes_open {
            self.details_panel(root_ui);
        }
        egui::CentralPanel::default().show(root_ui, |ui| {
            ui.add_enabled_ui(!blocked, |ui| self.canvas(ui));
        });
        if !blocked {
            self.palette(&ctx);
        }
        self.dialogs(&ctx);
        if self.help_open {
            egui::Window::new("A little less friction")
                .open(&mut self.help_open)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label("Capture ideas without leaving the keyboard.");
                    ui.separator();
                    for (key, purpose) in [
                        ("Tab", "Add child / commit and continue with a child"),
                        ("Enter", "Add sibling / commit text while editing"),
                        ("F2 or double-click", "Edit selected idea"),
                        ("Escape", "Cancel inline edit or close search/commands"),
                        ("Arrows", "Navigate the map (text fields keep cursor keys)"),
                        ("Space", "Fold / unfold"),
                        (
                            "Alt + arrows",
                            "Up/down: reorder; right/left: indent/outdent",
                        ),
                        ("Cmd/Ctrl + Z / Shift+Z", "Undo / redo"),
                        ("Cmd/Ctrl + K / F", "Commands / search"),
                        ("Cmd/Ctrl + N / O / S", "New / open / save"),
                        ("Cmd/Ctrl + Shift + S", "Save as"),
                        ("Drag blank canvas", "Pan"),
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
        if self.editor.revision != before || self.editor.draft_revision != draft_before {
            self.last_change = Instant::now();
            self.recovery_revision = None;
        }
        if self.dialog.is_none()
            && self.recovery.is_none()
            && self.recovery_revision != Some((self.editor.revision, self.editor.draft_revision))
        {
            if self.last_change.elapsed() >= Duration::from_millis(700)
                && self.recovery_revision
                    != Some((self.editor.revision, self.editor.draft_revision))
            {
                match self.checkpoint_recovery() {
                    Ok(()) => self.status = "Recovery snapshot updated locally.".into(),
                    Err(error) => self.error = Some(format!("Recovery unavailable: {error}")),
                }
                self.recovery_revision = Some((self.editor.revision, self.editor.draft_revision));
            }
            ctx.request_repaint_after(Duration::from_millis(700));
        }
        if text_pending || self.editor.editing.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let title = self
            .editor
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
            "{}{} — Branchline",
            if self.editor.is_dirty() { "* " } else { "" },
            title
        )));
    }

    fn open_palette(&mut self) {
        let result = self.prepare();
        if result.is_ok() {
            self.palette_open = true;
            self.palette_query.clear();
            self.palette_focus = true;
        }
        self.report(result);
    }

    fn open_search(&mut self) {
        let result = self.prepare();
        if result.is_ok() {
            self.search_open = true;
            self.search_focus = true;
        }
        self.report(result);
    }

    fn details_panel(&mut self, ui: &mut egui::Ui) {
        if self
            .details
            .as_ref()
            .is_none_or(|details| details.id != self.editor.selected)
        {
            let result = self.flush_details();
            self.report(result);
            let node = &self.editor.document.nodes[&self.editor.selected];
            self.details = Some(Details {
                id: node.id.clone(),
                notes: node.notes.clone(),
                link: node.link.clone(),
            });
        }
        let mut commit = false;
        egui::Panel::right("notes").default_size(290.0).min_size(220.0).show(ui, |ui| {
            ui.add_space(12.0);
            ui.heading("A little more context");
            ui.weak("Notes for the selected idea");
            ui.add_space(12.0);
            if let Some(details) = &mut self.details {
                let response = ui.add(egui::TextEdit::multiline(&mut details.notes).desired_width(f32::INFINITY).desired_rows(14).hint_text("Capture details here...").char_limit(262_144));
                if response.changed() { self.recovery_revision = None; self.last_change = Instant::now(); }
                commit |= response.lost_focus();
                ui.add_space(12.0);
                ui.label("Link");
                let response = ui.add(egui::TextEdit::singleline(&mut details.link).hint_text("Paste a URL or path").char_limit(262_144));
                if response.changed() { self.recovery_revision = None; self.last_change = Instant::now(); }
                commit |= response.lost_focus();
                if ui.button("Open link").clicked() && !details.link.is_empty() {
                    let allowed = ["https://", "http://", "mailto:"].iter().any(|prefix| details.link.starts_with(prefix));
                    if allowed {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&details.link));
                    } else {
                        self.error = Some("Only explicit http, https and mailto links can be opened here; copy other paths manually.".into());
                    }
                }
                ui.weak("Notes are plain text. No scripts or formulas run.");
            }
        });
        if commit {
            let result = self.flush_details();
            self.report(result);
        }
    }

    fn search_panel(&mut self, ui: &mut egui::Ui) {
        let mut selected = None;
        egui::Panel::top("search").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Find an idea");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Search text and notes")
                        .char_limit(1024),
                );
                if self.search_focus {
                    response.request_focus();
                    self.search_focus = false;
                }
                if ui.button("Close").clicked() {
                    self.search_open = false;
                    response.surrender_focus();
                }
            });
            let results = self.editor.search(&self.search_query);
            if !self.search_query.is_empty() {
                ui.weak(format!("{} matches", results.len()));
                egui::ScrollArea::vertical()
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for id in results.iter().take(100) {
                            if ui
                                .selectable_label(
                                    self.editor.selected == *id,
                                    &self.editor.document.nodes[id].text,
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
            let result = self
                .flush_details()
                .and_then(|_| self.editor.reveal_selection(&id));
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
                action = matches.first().map(|command| command.action);
            }
            for command in matches {
                if ui
                    .button(format!("{}    {}", command.label, command.shortcut))
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
            self.action(action);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let focused = ctx.memory(|memory| memory.focused());
        if self.composing.is_some() && self.composing != focused {
            self.composing = None;
        }
        for event in ctx.input(|input| input.events.clone()) {
            match event {
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                    self.composing = if text.is_empty() { None } else { focused };
                }
                egui::Event::Ime(egui::ImeEvent::Commit(_)) => self.composing = None,
                _ => {}
            }
        }
        if self.composing.is_some() || self.palette_open || self.help_open {
            return;
        }
        let (modifiers, pressed) = ctx.input(|input| (input.modifiers, input.keys_down.clone()));
        let key = |key| ctx.input(|input| input.key_pressed(key));
        if modifiers.command {
            if key(Key::K) {
                self.open_palette();
                return;
            }
            if key(Key::F) {
                self.open_search();
                return;
            }
            if key(Key::S) {
                self.save_file(modifiers.shift, ctx);
                return;
            }
            if key(Key::O) {
                self.open_file(ctx);
                return;
            }
            if key(Key::N) {
                self.request(PendingAction::New, ctx);
                return;
            }
            if key(Key::Q) {
                self.request(PendingAction::Quit, ctx);
                return;
            }
        }
        if key(Key::Escape) && self.search_open && self.editor.editing.is_none() {
            self.search_open = false;
            ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            return;
        }
        if self.editor.editing.is_some() {
            return;
        }
        let map_focused = ctx
            .memory(|memory| memory.focused())
            .is_some_and(|focused| {
                self.placed
                    .nodes
                    .keys()
                    .any(|id| focused == Id::unique(("idea", id)))
            });
        if ctx.egui_wants_keyboard_input() && !map_focused {
            return;
        }
        if modifiers.command && key(Key::Z) {
            self.action(if modifiers.shift {
                EditorAction::Redo
            } else {
                EditorAction::Undo
            });
            return;
        }
        if modifiers.command {
            return;
        }
        for (keycode, direction, movement) in [
            (Key::ArrowLeft, Direction::Left, EditorAction::Outdent),
            (Key::ArrowRight, Direction::Right, EditorAction::Indent),
            (Key::ArrowUp, Direction::Up, EditorAction::MoveUp),
            (Key::ArrowDown, Direction::Down, EditorAction::MoveDown),
        ] {
            if key(keycode) {
                ctx.input_mut(|input| input.consume_key(modifiers, keycode));
                self.action(if modifiers.alt {
                    movement
                } else {
                    EditorAction::Navigate(direction)
                });
                return;
            }
        }
        for (keycode, action) in [
            (Key::Tab, EditorAction::AddChild),
            (Key::Enter, EditorAction::AddSibling),
            (Key::F2, EditorAction::Edit),
            (Key::Space, EditorAction::Fold),
            (Key::Backspace, EditorAction::Delete),
            (Key::Delete, EditorAction::Delete),
        ] {
            if pressed.contains(&keycode) && key(keycode) {
                ctx.input_mut(|input| input.consume_key(modifiers, keycode));
                self.action(action);
                break;
            }
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        let dark = ui.visuals().dark_mode;
        let background = if dark {
            Color32::from_rgb(22, 25, 33)
        } else {
            Color32::from_rgb(247, 248, 251)
        };
        ui.painter().rect_filled(available, 0.0, background);
        let canvas_response = ui.allocate_rect(available, Sense::click_and_drag());
        let center = point(available.center());
        if canvas_response.dragged() {
            let delta = ui.input(|input| input.pointer.delta());
            self.viewport.pan.x += delta.x;
            self.viewport.pan.y += delta.y;
        }
        if canvas_response.hovered() && self.editor.editing.is_none() {
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
                self.viewport.zoom_at(point(pointer), center, zoom);
            }
        }
        if self.layout_revision != Some(self.editor.revision) || self.cached_dark != dark {
            self.galleys.clear();
            let mut sizes = BTreeMap::new();
            for (id, node) in &self.editor.document.nodes {
                let root = id == &self.editor.document.root;
                let galley = ui.painter().layout(
                    node.text.clone(),
                    FontId::proportional(if root { 22.0 } else { 16.0 }),
                    ui.visuals().text_color(),
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
            match layout::layout(&self.editor.document, &sizes) {
                Ok(placed) => self.placed = placed,
                Err(error) => self.error = Some(error.to_string()),
            }
            self.layout_revision = Some(self.editor.revision);
            self.cached_dark = dark;
        }
        if self.fit_requested {
            self.viewport.fit(
                self.placed.bounds,
                Size {
                    width: available.width(),
                    height: available.height(),
                },
            );
            self.fit_requested = false;
        }
        if self.focus_selected {
            if let Some(rect) = self.placed.nodes.get(&self.editor.selected) {
                let screen = screen_rect(self.viewport, *rect, center);
                if !available.shrink(60.0).contains_rect(screen) {
                    self.viewport.focus(rect.center());
                }
            }
            self.focus_selected = false;
        }
        let palette = [
            Color32::from_rgb(100, 135, 230),
            Color32::from_rgb(76, 165, 147),
            Color32::from_rgb(188, 130, 208),
            Color32::from_rgb(222, 162, 91),
        ];
        for (index, (parent, child)) in self.placed.edges.iter().enumerate() {
            let a = screen_rect(self.viewport, self.placed.nodes[parent], center);
            let b = screen_rect(self.viewport, self.placed.nodes[child], center);
            let right = b.center().x > a.center().x;
            let start = if right {
                a.right_center()
            } else {
                a.left_center()
            };
            let end = if right {
                b.left_center()
            } else {
                b.right_center()
            };
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
                        1.8 * self.viewport.zoom,
                        palette[index % palette.len()].gamma_multiply(0.65),
                    ),
                ));
        }
        let mut clicked = None;
        let mut edit = false;
        let mut fold = None;
        let inline_composing = self.composing.is_some()
            || ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Ime(_)))
            });
        let mut edit_key = self.editor.editing.as_ref().and_then(|buffer| {
            if inline_composing {
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
            key.map(|key| (key, Id::unique(("inline", &buffer.id))))
        });
        let ids: Vec<_> = self.placed.nodes.keys().cloned().collect();
        for id in ids {
            let rect = screen_rect(self.viewport, self.placed.nodes[&id], center);
            if !rect.intersects(available) {
                continue;
            }
            let selected = id == self.editor.selected;
            let node = &self.editor.document.nodes[&id];
            let stroke = if selected {
                Stroke::new(2.0, Color32::from_rgb(105, 139, 244))
            } else {
                Stroke::new(
                    1.0,
                    if dark {
                        Color32::from_gray(61)
                    } else {
                        Color32::from_gray(220)
                    },
                )
            };
            let fill = if selected {
                if dark {
                    Color32::from_rgb(38, 48, 72)
                } else {
                    Color32::from_rgb(235, 240, 255)
                }
            } else if dark {
                Color32::from_rgb(31, 35, 45)
            } else {
                Color32::WHITE
            };
            ui.painter().rect_filled(rect, 10.0, fill);
            ui.painter()
                .rect_stroke(rect, 10.0, stroke, egui::StrokeKind::Inside);
            if self
                .editor
                .editing
                .as_ref()
                .is_some_and(|buffer| buffer.id == id)
            {
                let first_focus = self.edit_focus.as_ref() != Some(&id);
                let buffer = self.editor.editing.as_mut().expect("editing node");
                let response = ui.put(
                    rect.shrink(7.0),
                    egui::TextEdit::singleline(&mut buffer.text)
                        .id(Id::unique(("inline", &id)))
                        .font(FontId::proportional(16.0 * self.viewport.zoom))
                        .char_limit(262_144),
                );
                if response.changed() {
                    self.recovery_revision = None;
                    self.last_change = Instant::now();
                }
                if first_focus {
                    response.request_focus();
                    self.edit_focus = Some(id.clone());
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(buffer.text.chars().count()),
                            )));
                        state.store(ui.ctx(), response.id);
                    }
                }
                if !inline_composing && edit_key.is_none() && response.lost_focus() && !first_focus
                {
                    edit_key = Some((EditorKey::Enter, response.id));
                }
            } else {
                let response = ui.interact(rect, Id::unique(("idea", &id)), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::TreeItem,
                        ui.is_enabled(),
                        selected,
                        &node.text,
                    )
                });
                if response.clicked() {
                    clicked = Some(id.clone());
                }
                if response.double_clicked() {
                    clicked = Some(id.clone());
                    edit = true;
                }
                response.on_hover_text("Double-click to edit. Tab adds a child.");
                if let Some(galley) = self.galleys.get(&id) {
                    let text_pos = rect.center() - galley.size() * self.viewport.zoom * 0.5;
                    let mut shape = egui::epaint::TextShape::new(
                        Pos2::ZERO,
                        galley.clone(),
                        ui.visuals().text_color(),
                    );
                    shape.transform(egui::emath::TSTransform::new(
                        text_pos.to_vec2(),
                        self.viewport.zoom,
                    ));
                    ui.painter().add(shape);
                }
            }
            if !node.children.is_empty() {
                let fold_rect = egui::Rect::from_center_size(
                    rect.right_center() + Vec2::new(11.0, 0.0),
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
        if self.editor.editing.is_none() {
            self.edit_focus = None;
        }
        if let Some((key, focus)) = edit_key {
            let result = self.editor.handle_key(key, false, false).map(|_| ());
            self.report(result);
            if self.editor.editing.is_none() {
                ui.memory_mut(|memory| memory.surrender_focus(focus));
            }
            self.edit_focus = None;
            self.focus_selected = true;
        }
        if let Some(id) = clicked {
            let result = self.flush_details().and_then(|_| self.editor.select(&id));
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
            let result = self
                .prepare()
                .and_then(|_| self.editor.execute(Command::ToggleFold { id }));
            self.report(result);
        }
        if self.editor.document.nodes.len() == 1 && self.editor.editing.is_none() {
            ui.painter().text(
                available.center() + Vec2::new(0.0, 100.0),
                egui::Align2::CENTER_TOP,
                "Start with one idea. Press Tab to let it grow.",
                FontId::proportional(15.0),
                ui.visuals().weak_text_color(),
            );
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let mut keep = true;
        egui::Modal::new(Id::unique("file-dialog")).show(ctx, |ui| {
            ui.set_width(470.0);
            match &dialog {
                Dialog::Unsaved => {
                    ui.heading("Keep these ideas?");
                    ui.label("Save your changes before continuing.");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() { keep = false; self.save_file(false, ctx); if self.dialog.is_none() && self.editor.pending.is_some() { self.dialog = Some(Dialog::Unsaved); } }
                        if ui.button("Discard changes").clicked() { keep = false; self.perform_pending(ctx); }
                        if ui.button("Cancel").clicked() { keep = false; self.editor.cancel_pending(); }
                    });
                }
                Dialog::Recovery => {
                    ui.heading("Your ideas are still here");
                    ui.label("A local recovery snapshot is available. Restore it or explicitly discard it before starting another document.");
                    ui.horizontal(|ui| {
                        if ui.button("Restore").clicked()
                            && let Some(record) = self.recovery.clone() {
                                let result = self.editor.replace_document(record.document, record.original_path, false, Some(record.document_id));
                                if result.is_ok() { self.recovery = None; keep = false; self.reset_view(); }
                                self.report(result);
                            }
                        if ui.button("Discard snapshot").clicked()
                            && let Some(record) = &self.recovery {
                                match self.store.discard(&record.document_id) {
                                    Ok(()) => { self.recovery = None; keep = false; }
                                    Err(error) => self.error = Some(error.to_string()),
                                }
                            }
                    });
                }
                Dialog::Import => {
                    ui.heading("Import Freeplane map");
                    ui.label("Original files stay untouched. The imported document saves as Branchline JSON.");
                    if let Some(report) = &self.editor.staged_import {
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
                            let result = self.clear_recovery().and_then(|_| self.editor.accept_import());
                            if result.is_ok() { keep = false; self.reset_view(); }
                            self.report(result);
                        }
                        if ui.button("Cancel").clicked() { keep = false; self.editor.cancel_import(); }
                    });
                }
                Dialog::Delete(id) => {
                    ui.heading("Delete this branch?");
                    ui.label("This removes the selected idea and all its descendants. You can undo it.");
                    ui.horizontal(|ui| {
                        if ui.button("Delete branch").clicked() {
                            let result = self.editor.execute(Command::Delete { id: id.clone() });
                            self.report(result);
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() { keep = false; }
                    });
                }
                Dialog::Overwrite { path, export } => {
                    ui.heading("Replace this file?");
                    ui.label(path.display().to_string());
                    ui.label("The existing file will be atomically replaced after the new data is written.");
                    ui.horizontal(|ui| {
                        if ui.button("Replace file").clicked() { keep = false; self.publish(path.clone(), export.clone(), ctx); }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                            if self.editor.pending.is_some() { self.dialog = Some(Dialog::Unsaved); }
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

use super::*;

impl NativeApp {
    fn focused_ids(&self) -> Option<HashSet<NodeId>> {
        let map = self.workspace.map();
        let focus = map.branch_focus.as_ref()?;
        let mut ids = HashSet::new();
        let mut pending = vec![focus.root.clone()];
        while let Some(id) = pending.pop() {
            if let Some(node) = map.editor.document.nodes.get(&id) {
                pending.extend(node.children.iter().cloned());
                ids.insert(id);
            }
        }
        Some(ids)
    }

    pub(super) fn reconcile_branch_focus(&mut self) {
        if let Some(ids) = self.focused_ids() {
            let editor = &self.workspace.map().editor;
            if !ids.contains(&editor.selected)
                || editor.selection().iter().any(|id| !ids.contains(id))
            {
                self.show_full_map();
            }
        }
    }

    pub(super) fn focus_branch(&mut self, root: NodeId) -> Result<(), EditorError> {
        if root == self.workspace.map().editor.document.root {
            self.show_full_map();
            return Ok(());
        }
        self.cancel_canvas_drag();
        self.workspace.focus_branch(root)?;
        self.layout_revision = None;
        self.fit_requested = true;
        self.focus_selected = false;
        self.edit_focus = None;
        self.status =
            "Showing only this branch. Use the breadcrumbs to return to the full map.".into();
        Ok(())
    }

    pub(super) fn show_full_map(&mut self) {
        if let Some(focus) = self.workspace.map_mut().branch_focus.take() {
            self.cancel_canvas_drag();
            self.workspace.map_mut().viewport = focus.full_map_viewport;
            self.layout_revision = None;
            self.fit_requested = false;
            self.focus_selected = false;
            self.edit_focus = None;
            self.status = "Full map restored.".into();
        }
    }

    pub(super) fn select_scope(&mut self) -> Result<(), EditorError> {
        if let Some(ids) = self.focused_ids() {
            let visible = self.workspace.map().editor.document.visible_nodes();
            let selection = ids.into_iter().filter(|id| visible.contains(id)).collect();
            self.edit(EditorEdit::SetSelection(selection))
        } else {
            self.edit(EditorEdit::SelectAll)
        }
    }

    pub(super) fn filter_focused_layout(&self, placed: &mut Layout) {
        let Some(ids) = self.focused_ids() else {
            return;
        };
        placed.nodes.retain(|id, _| ids.contains(id));
        placed
            .edges
            .retain(|(parent, child)| ids.contains(parent) && ids.contains(child));
        if let Some(bounds) = placed.nodes.values().copied().reduce(|a, b| Rect {
            min: Point {
                x: a.min.x.min(b.min.x),
                y: a.min.y.min(b.min.y),
            },
            max: Point {
                x: a.max.x.max(b.max.x),
                y: a.max.y.max(b.max.y),
            },
        }) {
            placed.bounds = bounds;
        }
    }

    pub(super) fn focus_breadcrumbs(&mut self, ui: &mut egui::Ui) {
        let Some(focus) = &self.workspace.map().branch_focus else {
            return;
        };
        let doc = &self.workspace.map().editor.document;
        let parents: BTreeMap<_, _> = doc
            .nodes
            .iter()
            .flat_map(|(id, node)| {
                node.children
                    .iter()
                    .map(move |child| (child.as_str(), id.as_str()))
            })
            .collect();
        let mut ancestors = vec![focus.root.clone()];
        let mut current = focus.root.as_str();
        while let Some(parent) = parents.get(current).copied() {
            if parent == doc.root {
                break;
            }
            ancestors.push(parent.into());
            current = parent;
        }
        ancestors.reverse();
        let mut chosen = None;
        let mut full_map = false;
        egui::ScrollArea::horizontal()
            .id_salt(("branch-breadcrumbs", self.workspace.active_id()))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    full_map = ui.button("Full map").clicked();
                    for id in &ancestors {
                        ui.weak(">");
                        let is_current = id == &focus.root;
                        if ui
                            .add_enabled(
                                !is_current,
                                egui::Button::new(&doc.nodes[id].text).truncate(),
                            )
                            .clicked()
                        {
                            chosen = Some(id.clone());
                        }
                    }
                });
            });
        ui.separator();
        if full_map {
            self.show_full_map();
        } else if let Some(root) = chosen {
            let result = self.focus_branch(root);
            self.report(result);
        }
    }

    pub(super) fn update_box_selection(
        &mut self,
        ui: &mut egui::Ui,
        available: egui::Rect,
        center: Point,
    ) {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        let Some(CanvasDrag::Selection {
            origin, current, ..
        }) = &mut self.drag
        else {
            return;
        };
        if let Some(pointer) = pointer {
            *current = pointer;
        }
        let rect = egui::Rect::from_two_pos(*origin, *current).intersect(available);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary)) {
            let Some(CanvasDrag::Selection { additive, .. }) = self.drag.take() else {
                unreachable!("active rectangle selection")
            };
            if !pointer.is_some_and(|pos| available.contains(pos)) {
                self.status = "Rectangle selection canceled: release inside the canvas.".into();
                return;
            }
            let editor = &self.workspace.map().editor;
            let mut selected: Vec<_> = self
                .placed
                .nodes
                .iter()
                .filter(|(_, bounds)| {
                    screen_rect(self.workspace.map().viewport, **bounds, center).intersects(rect)
                })
                .map(|(id, _)| id.clone())
                .collect();
            if additive {
                if editor.selection_coverage() == crate::app::SelectionCoverage::WholeMap {
                    self.status = "All ideas remain selected.".into();
                    return;
                }
                selected.extend(editor.selection().iter().cloned());
            }
            if selected.is_empty() {
                self.status = "No ideas inside the rectangle. Previous selection retained.".into();
                return;
            }
            let result = self.edit(EditorEdit::SetSelection(selected));
            if result.is_ok() {
                self.status = format!(
                    "{} ideas selected.",
                    self.workspace.map().editor.selection_count()
                );
            }
            self.report(result);
        } else if !ui.input(|input| input.pointer.button_down(egui::PointerButton::Primary)) {
            self.drag = None;
            self.status =
                "Rectangle selection canceled because the pointer left the window.".into();
        }
    }

    pub(super) fn paint_box_selection(&self, ui: &egui::Ui, available: egui::Rect) {
        if let Some(CanvasDrag::Selection {
            origin, current, ..
        }) = &self.drag
        {
            let rect = egui::Rect::from_two_pos(*origin, *current).intersect(available);
            let accent = ui.visuals().selection.stroke.color;
            ui.painter().rect(
                rect,
                0.0,
                accent.gamma_multiply(0.12),
                Stroke::new(1.0, accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}

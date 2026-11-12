use branchline::{app::*, interchange::import_mm};

fn folded_map() -> Editor {
    Editor::from_document(import_mm(br#"<map><node ID="r" TEXT="Root"><node ID="a" TEXT="A" FOLDED="true"><node ID="b" TEXT="B"/></node><node ID="c" TEXT="C"/></node></map>"#).unwrap().document, None, true, None).unwrap()
}

#[test]
fn applying_notes_preserves_unrelated_drafts_and_updates_the_matching_sidebar_only() {
    use branchline::{document::Command, workspace::Workspace};
    let mut workspace = Workspace::new();
    let session = workspace.active_id();
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(
            session,
            EditorEdit::Command(Command::AddChild {
                parent: root.clone(),
                text: "Notes target".into(),
            }),
        )
        .unwrap();
    let target = workspace.map().editor.selected.clone();
    workspace
        .apply(session, EditorEdit::Select(root.clone()))
        .unwrap();
    workspace
        .apply(session, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    workspace
        .apply(
            session,
            EditorEdit::SetDraft {
                field: DraftField::Inline(root.clone()),
                text: "Retained label".into(),
            },
        )
        .unwrap();
    workspace
        .apply(session, EditorEdit::BeginDetails(root.clone()))
        .unwrap();
    workspace
        .apply(
            session,
            EditorEdit::SetDraft {
                field: DraftField::Notes(root.clone()),
                text: "Retained root notes".into(),
            },
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    let generation = workspace.map().editor.history.generation;
    workspace
        .apply(
            session,
            EditorEdit::ApplyNotes {
                id: target.clone(),
                notes: "**Target** notes".into(),
            },
        )
        .unwrap();
    assert_eq!(
        workspace.map().editor.document.nodes[&root],
        before.nodes[&root]
    );
    assert_eq!(
        workspace.map().editor.editing.as_ref().unwrap().text,
        "Retained label"
    );
    assert_eq!(
        workspace.map().editor.details.as_ref().unwrap().notes,
        "Retained root notes"
    );
    assert_eq!(
        workspace.map().editor.document.nodes[&target].notes,
        "**Target** notes"
    );
    assert_eq!(workspace.map().editor.history.generation, generation + 1);
    workspace
        .apply(
            session,
            EditorEdit::ApplyNotes {
                id: root.clone(),
                notes: "Applied root notes".into(),
            },
        )
        .unwrap();
    assert_eq!(
        workspace.map().editor.details.as_ref().unwrap().notes,
        "Applied root notes"
    );
    assert_eq!(
        workspace.map().editor.snapshot_with_drafts().unwrap().nodes[&root].notes,
        "Applied root notes"
    );
    let before = workspace.map().editor.document.clone();
    let generation = workspace.map().editor.history.generation;
    assert!(
        workspace
            .apply(
                session,
                EditorEdit::ApplyNotes {
                    id: root.clone(),
                    notes: "é".repeat(branchline::document::MAX_FIELD_BYTES / 2 + 1)
                }
            )
            .is_err()
    );
    assert_eq!(workspace.map().editor.document, before);
    assert_eq!(workspace.map().editor.history.generation, generation);
    workspace
        .apply(
            session,
            EditorEdit::ApplyNotes {
                id: root,
                notes: "Applied root notes".into(),
            },
        )
        .unwrap();
    assert_eq!(workspace.map().editor.history.generation, generation);
}

#[test]
fn notes_undo_updates_the_sidebar_without_committing_its_unfinished_link() {
    use branchline::workspace::Workspace;
    let mut workspace = Workspace::new();
    let session = workspace.active_id();
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(session, EditorEdit::BeginDetails(root.clone()))
        .unwrap();
    workspace
        .apply(
            session,
            EditorEdit::SetDraft {
                field: DraftField::Link(root.clone()),
                text: "https://unfinished.example".into(),
            },
        )
        .unwrap();
    workspace
        .apply(
            session,
            EditorEdit::ApplyNotes {
                id: root.clone(),
                notes: "**Applied**".into(),
            },
        )
        .unwrap();
    workspace
        .apply(session, EditorEdit::Action(EditorAction::Undo))
        .unwrap();
    assert_eq!(
        workspace.map().editor.snapshot_with_drafts().unwrap().nodes[&root].notes,
        ""
    );
    assert_eq!(
        workspace.map().editor.details.as_ref().unwrap().link,
        "https://unfinished.example"
    );
    workspace
        .apply(session, EditorEdit::Action(EditorAction::Redo))
        .unwrap();
    assert_eq!(
        workspace.map().editor.snapshot_with_drafts().unwrap().nodes[&root].notes,
        "**Applied**"
    );
    assert_eq!(workspace.map().editor.document.nodes[&root].link, "");
}

#[test]
fn undo_does_not_remove_an_unfinished_inline_draft_owner() {
    use branchline::workspace::Workspace;
    let mut workspace = Workspace::new();
    let session = workspace.active_id();
    workspace
        .apply(session, EditorEdit::Action(EditorAction::AddChild))
        .unwrap();
    workspace.apply(session, EditorEdit::CommitDrafts).unwrap();
    let child = workspace.map().editor.selected.clone();
    workspace
        .apply(session, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    workspace
        .apply(
            session,
            EditorEdit::SetDraft {
                field: DraftField::Inline(child),
                text: "Unfinished child label".into(),
            },
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    let generation = workspace.map().editor.history.generation;
    assert!(
        workspace
            .apply(session, EditorEdit::Action(EditorAction::Undo))
            .is_err()
    );
    assert_eq!(workspace.map().editor.document, before);
    assert_eq!(workspace.map().editor.history.generation, generation);
    assert_eq!(
        workspace.map().editor.editing.as_ref().unwrap().text,
        "Unfinished child label"
    );
}

#[test]
fn whole_map_selection_includes_folded_root_tree() {
    let mut editor = folded_map();
    editor.select_all();
    assert_eq!(editor.selection_coverage(), SelectionCoverage::WholeMap);
    assert_eq!(editor.selected, "r");
    assert_eq!(editor.selection_count(), 4);
    assert_eq!(editor.hidden_selection_count(), 1);
    assert_eq!(editor.selection().len(), 1);
    assert!(editor.is_selected("b"));
    editor.dispatch(EditorAction::Fold).unwrap();
    assert_eq!(editor.selection_coverage(), SelectionCoverage::WholeMap);
    assert_eq!(editor.selection_count(), 4);
    assert_eq!(editor.hidden_selection_count(), 3);
    let forest = editor.copy_forest().unwrap();
    assert_eq!(forest.roots[0].label, "Root");
    assert_eq!(forest.roots[0].children[0].children[0].label, "B");
    for action in [
        EditorAction::Edit,
        EditorAction::Navigate(Direction::Down),
        EditorAction::Undo,
    ] {
        editor.select_all();
        editor.dispatch(action).unwrap();
        assert_eq!(editor.selection_coverage(), SelectionCoverage::VisibleNodes);
        assert!(
            editor
                .selection()
                .iter()
                .all(|id| editor.document.visible_nodes().contains(id))
        );
    }
}

#[test]
fn copy_selection_deduplicates_in_document_order() {
    let mut editor = folded_map();
    editor.select("a").unwrap();
    editor.dispatch(EditorAction::Fold).unwrap();
    editor.select("c").unwrap();
    editor.toggle_selection("b").unwrap();
    editor.toggle_selection("a").unwrap();
    let before = editor.document.clone();
    let generation = editor.history.generation;
    let forest = editor.copy_forest().unwrap();
    assert_eq!(
        forest
            .roots
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>(),
        ["A", "C"]
    );
    assert_eq!(forest.roots[0].children[0].label, "B");
    assert_eq!(editor.document, before);
    assert_eq!(editor.history.generation, generation);
    editor.select_all();
    editor.select("a").unwrap();
    assert_eq!(editor.selection_coverage(), SelectionCoverage::VisibleNodes);
    assert_eq!(editor.selection_count(), 1);
}

#[test]
fn whole_map_delete_retains_root_and_one_undo_restores_hidden_nodes() {
    let mut editor = folded_map();
    let original = editor.document.clone();
    editor.select_all();
    let plan = editor.deletion_plan().unwrap();
    assert_eq!(plan.node_count, 3);
    assert!(plan.root_retained);
    let generation = editor.history.generation;
    editor.dispatch(EditorAction::Delete).unwrap();
    assert_eq!(editor.document.nodes.len(), 1);
    assert_eq!(editor.document.nodes["r"].text, "Root");
    assert_eq!(editor.history.generation, generation + 1);
    editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(editor.document, original);
    assert_eq!(editor.selection_coverage(), SelectionCoverage::VisibleNodes);
    assert!(!editor.is_selected("b"));
}

#[test]
fn map_undo_releases_unchanged_details_but_does_not_discard_an_unfinished_owner() {
    use branchline::workspace::Workspace;
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    workspace
        .apply(id, EditorEdit::Action(EditorAction::AddChild))
        .unwrap();
    workspace.apply(id, EditorEdit::CommitDrafts).unwrap();
    let child = workspace.map().editor.selected.clone();
    workspace
        .apply(id, EditorEdit::BeginDetails(child.clone()))
        .unwrap();
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Undo))
        .unwrap();
    assert_eq!(workspace.map().editor.document.nodes.len(), 1);
    assert!(workspace.map().editor.details.is_none());
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Redo))
        .unwrap();
    workspace
        .apply(id, EditorEdit::BeginDetails(child.clone()))
        .unwrap();
    workspace
        .apply(
            id,
            EditorEdit::SetDraft {
                field: DraftField::Notes(child),
                text: "Unfinished".into(),
            },
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    let generation = workspace.map().editor.history.generation;
    assert!(
        workspace
            .apply(id, EditorEdit::Action(EditorAction::Undo))
            .is_err()
    );
    assert_eq!(workspace.map().editor.document, before);
    assert_eq!(workspace.map().editor.history.generation, generation);
    assert_eq!(
        workspace.map().editor.details.as_ref().unwrap().notes,
        "Unfinished"
    );
}

#[test]
fn edit_modes_route_keys() {
    let mut editor = Editor::new();
    let root = editor.selected.clone();
    assert!(!editor.handle_key(EditorKey::Tab, true, false).unwrap());
    assert!(!editor.handle_key(EditorKey::Tab, false, true).unwrap());
    assert_eq!(editor.document.nodes.len(), 1);
    editor.handle_key(EditorKey::Tab, false, false).unwrap();
    let child = editor.selected.clone();
    assert_ne!(child, root);
    editor.editing.as_mut().unwrap().text = "研究".into();
    editor.handle_key(EditorKey::Enter, false, false).unwrap();
    assert_eq!(editor.document.nodes[&child].text, "研究");
    assert!(editor.editing.is_none());
    editor.dispatch(EditorAction::Edit).unwrap();
    editor.editing.as_mut().unwrap().text = "cancel me".into();
    editor.handle_key(EditorKey::Escape, false, false).unwrap();
    assert_eq!(editor.document.nodes[&child].text, "研究");
    editor.handle_key(EditorKey::Enter, false, false).unwrap();
    assert_eq!(editor.document.nodes.len(), 3);
}

#[test]
fn failed_save_blocks_close() {
    use branchline::workspace::{CloseDecision, TransitionProgress, Workspace};
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    workspace
        .apply(id, EditorEdit::Action(EditorAction::AddChild))
        .unwrap();
    workspace.apply(id, EditorEdit::CommitDrafts).unwrap();
    assert_eq!(
        workspace.request_quit().unwrap(),
        TransitionProgress::AwaitingDecision(id)
    );
    workspace.resolve(CloseDecision::Save).unwrap();
    let temp = tempfile::tempdir().unwrap();
    assert!(
        workspace
            .save(id, &temp.path().join("missing/map.branchline"))
            .is_err()
    );
    assert!(workspace.map().editor.is_dirty());
    assert!(workspace.transition_pending());
    workspace.resolve(CloseDecision::Cancel).unwrap();
    assert!(!workspace.transition_pending());
    let path = temp.path().join("map.branchline");
    workspace.save(id, &path).unwrap();
    assert!(!workspace.map().editor.is_dirty());
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Undo))
        .unwrap();
    assert!(workspace.map().editor.is_dirty());
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Redo))
        .unwrap();
    assert!(!workspace.map().editor.is_dirty());
}

#[test]
fn imported_tab_has_independent_history_and_no_native_save_path() {
    let mut workspace = branchline::workspace::Workspace::new();
    let original = workspace.active_id();
    workspace
        .apply(original, EditorEdit::Action(EditorAction::AddChild))
        .unwrap();
    workspace.apply(original, EditorEdit::CommitDrafts).unwrap();
    workspace
        .accept_import(import_mm(br#"<map><node ID="r" TEXT="Imported"/></map>"#).unwrap())
        .unwrap();
    workspace.apply_pending_activation().unwrap();
    workspace
        .apply(
            workspace.active_id(),
            EditorEdit::Action(EditorAction::Undo),
        )
        .unwrap();
    let editor = &workspace.map().editor;
    assert_eq!(editor.document.root, "r");
    assert_eq!(editor.document.nodes["r"].text, "Imported");
    assert_eq!(editor.document.nodes.len(), 1);
    assert!(editor.path.is_none());
    assert!(editor.is_dirty());
    assert_eq!(
        workspace
            .session(original)
            .unwrap()
            .editor
            .document
            .nodes
            .len(),
        2
    );
}

#[test]
fn no_op_metadata_does_not_invalidate_layout() {
    let mut editor = Editor::new();
    let revision = editor.revision;
    editor
        .execute(branchline::document::Command::SetNotes {
            id: editor.selected.clone(),
            notes: String::new(),
        })
        .unwrap();
    assert_eq!(editor.revision, revision);
    assert!(!editor.is_dirty());
}

#[test]
fn multi_selection_toggles_and_drops_hidden_nodes_after_undo() {
    let mut editor = Editor::new();
    editor.dispatch(EditorAction::AddChild).unwrap();
    let parent = editor.selected.clone();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.commit_edit().unwrap();
    let hidden = editor.selected.clone();
    let root = editor.document.root.clone();
    editor.select(&root).unwrap();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.commit_edit().unwrap();
    let other = editor.selected.clone();
    editor.select(&parent).unwrap();
    editor.dispatch(EditorAction::Fold).unwrap();
    editor.reveal_selection(&hidden).unwrap();
    editor.toggle_selection(&other).unwrap();
    assert_eq!(editor.selection().len(), 2);
    editor.toggle_selection(&other).unwrap();
    assert_eq!(editor.selection().len(), 1);
    assert_eq!(editor.selected, hidden);
    editor.toggle_selection(&other).unwrap();
    editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(editor.selected, other);
    assert!(
        !editor.is_selected(&hidden),
        "Invisible nodes must not remain batch-delete targets"
    );
    editor.dispatch(EditorAction::Delete).unwrap();
    assert!(editor.document.nodes.contains_key(&parent));
    assert!(editor.document.nodes.contains_key(&hidden));
    assert!(!editor.document.nodes.contains_key(&other));
}

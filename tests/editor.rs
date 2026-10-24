use branchline::{app::*, interchange::import_mm};

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

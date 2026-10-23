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
fn cancelled_import_is_transactional() {
    let mut editor = Editor::new();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.commit_edit().unwrap();
    let before = editor.document.clone();
    let selected = editor.selected.clone();
    editor.stage_import(
        import_mm(br#"<map><node ID="r" TEXT="Imported"><font SIZE="24"/></node></map>"#).unwrap(),
    );
    editor.cancel_import();
    assert_eq!(editor.document, before);
    assert_eq!(editor.selected, selected);
    assert!(editor.path.is_none());
    editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(editor.document.nodes.len(), 1);
}

#[test]
fn failed_save_blocks_close() {
    let mut editor = Editor::new();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.commit_edit().unwrap();
    assert!(!editor.request(PendingAction::Quit).unwrap());
    let temp = tempfile::tempdir().unwrap();
    assert!(
        editor
            .save_to(&temp.path().join("missing/map.branchline"))
            .is_err()
    );
    assert!(editor.is_dirty());
    assert!(matches!(editor.pending, Some(PendingAction::Quit)));
    editor.cancel_pending();
    assert!(editor.pending.is_none());
    let path = temp.path().join("map.branchline");
    editor.save_to(&path).unwrap();
    assert!(!editor.is_dirty());
    editor.dispatch(EditorAction::Undo).unwrap();
    assert!(editor.is_dirty());
    editor.dispatch(EditorAction::Redo).unwrap();
    assert!(!editor.is_dirty());
}

#[test]
fn replacing_document_clears_old_history_and_save_path() {
    let mut editor = Editor::new();
    editor.dispatch(EditorAction::AddChild).unwrap();
    editor.commit_edit().unwrap();
    editor.stage_import(import_mm(br#"<map><node ID="r" TEXT="Imported"/></map>"#).unwrap());
    editor.accept_import().unwrap();
    editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(editor.document.root, "r");
    assert_eq!(editor.document.nodes["r"].text, "Imported");
    assert_eq!(editor.document.nodes.len(), 1);
    assert!(editor.path.is_none());
    assert!(editor.is_dirty());
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

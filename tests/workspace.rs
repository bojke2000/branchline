use branchline::{
    app::{DetailsDraft, DraftField, EditorAction, EditorEdit},
    document::MAX_FIELD_BYTES,
    session::{MapSession, SessionId},
    workspace::Workspace,
};

#[test]
fn session_snapshot_preserves_drafts_without_committing() {
    let mut session = MapSession::new_blank(SessionId::new());
    let root = session.editor.document.root.clone();
    session.editor.dispatch(EditorAction::Edit).unwrap();
    session.editor.editing.as_mut().unwrap().text = "Unfinished".into();
    session.editor.details = Some(DetailsDraft {
        id: root.clone(),
        notes: "Unfinished notes".into(),
        link: "https://example.org/idea".into(),
    });
    let before = session.editor.document.clone();
    let generation = session.editor.history.generation;
    let snapshot = session.editor.snapshot_with_drafts().unwrap();
    assert_eq!(snapshot.nodes[&root].text, "Unfinished");
    assert_eq!(snapshot.nodes[&root].notes, "Unfinished notes");
    assert_eq!(snapshot.nodes[&root].link, "https://example.org/idea");
    assert_eq!(session.editor.document, before);
    assert_eq!(session.editor.history.generation, generation);
    assert!(session.editor.editing.is_some());
    assert!(session.editor.details.is_some());
    assert!(!session.editor.is_dirty());
}

#[test]
fn snapshot_rejects_a_stale_draft_owner() {
    let mut session = MapSession::new_blank(SessionId::new());
    session.editor.details = Some(DetailsDraft {
        id: "missing-node".into(),
        notes: "Must not disappear".into(),
        link: String::new(),
    });
    assert!(session.editor.snapshot_with_drafts().is_err());
    assert!(session.editor.has_drafts().is_err());
    session.editor.details = None;
    session.editor.dispatch(EditorAction::Edit).unwrap();
    session.editor.editing.as_mut().unwrap().id = "missing-inline-owner".into();
    assert!(session.editor.snapshot_with_drafts().is_err());
}

#[test]
fn details_flush_does_not_publish_half_an_invalid_draft() {
    let mut session = MapSession::new_blank(SessionId::new());
    let before = session.editor.document.clone();
    let generation = session.editor.history.generation;
    session.editor.details = Some(DetailsDraft {
        id: session.editor.selected.clone(),
        notes: "Valid new notes".into(),
        link: "x".repeat(MAX_FIELD_BYTES + 1),
    });
    assert!(session.editor.flush_details().is_err());
    assert_eq!(session.editor.document, before);
    assert_eq!(session.editor.history.generation, generation);
    assert!(session.editor.details.is_some());
    session.editor.details.as_mut().unwrap().link = "https://example.org".into();
    session.editor.flush_details().unwrap();
    assert!(session.editor.details.is_none());
    session.editor.dispatch(EditorAction::Undo).unwrap();
    assert_eq!(session.editor.document, before);
}

#[test]
fn workspace_growth_is_admitted_atomically() {
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    let node = workspace.map().editor.selected.clone();
    workspace
        .apply(
            id,
            EditorEdit::SetDraft {
                field: DraftField::Inline(node.clone()),
                text: "A retained draft".into(),
            },
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    let generation = workspace.map().editor.history.generation;
    assert!(
        workspace
            .apply(
                id,
                EditorEdit::SetDraft {
                    field: DraftField::Inline(node),
                    text: "x".repeat(MAX_FIELD_BYTES + 1),
                }
            )
            .is_err()
    );
    assert_eq!(workspace.map().editor.document, before);
    assert_eq!(workspace.map().editor.history.generation, generation);
    assert_eq!(
        workspace.map().editor.editing.as_ref().unwrap().text,
        "A retained draft"
    );
    workspace.apply(id, EditorEdit::CommitDrafts).unwrap();
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Undo))
        .unwrap();
    assert_eq!(workspace.map().editor.document, before);
}

#[test]
fn maximum_supported_map_opens_beside_pristine_blank() {
    use branchline::document::{Command, Document, MAX_FILE_BYTES, MAX_NODES, Node, Side};
    use branchline::interchange::write_native;
    let mut document = Document::new("");
    let root = document.root.clone();
    for index in 1..MAX_NODES {
        let id = format!("n{index}");
        document
            .nodes
            .get_mut(&root)
            .unwrap()
            .children
            .push(id.clone());
        document.nodes.insert(
            id.clone(),
            Node {
                id,
                text: String::new(),
                notes: String::new(),
                link: String::new(),
                children: Vec::new(),
                side: Side::Right,
                folded: false,
            },
        );
    }
    let mut remaining = MAX_FILE_BYTES - write_native(&document).unwrap().len();
    for node in document.nodes.values_mut() {
        let bytes = remaining.min(MAX_FIELD_BYTES);
        node.text = "x".repeat(bytes);
        remaining -= bytes;
        if remaining == 0 {
            break;
        }
    }
    assert_eq!(remaining, 0);
    assert_eq!(write_native(&document).unwrap().len(), MAX_FILE_BYTES);
    let mut workspace = Workspace::new();
    let blank = workspace.active_id();
    let mut loaded = MapSession::new_blank(SessionId::new());
    loaded
        .editor
        .replace_document(document, None, true, None)
        .unwrap();
    workspace.insert_session(loaded).unwrap();
    assert_eq!(workspace.len(), 2);
    assert!(workspace.session(blank).unwrap().is_pristine_blank());
    assert!(workspace.estimated_bytes() <= 256 * 1024 * 1024);
    workspace.apply_pending_activation().unwrap();
    let root = workspace.map().editor.document.root.clone();
    let text = workspace.map().editor.document.nodes[&root]
        .text
        .replacen('x', "y", 1);
    workspace
        .apply(
            workspace.active_id(),
            EditorEdit::Command(Command::SetText { id: root, text }),
        )
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut recovery =
        branchline::recovery::RecoveryCoordinator::open(directory.path().into()).unwrap();
    recovery.publish(&workspace.recovery_state()).unwrap();
    let snapshot = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("snapshot-")
        })
        .unwrap();
    assert_eq!(
        std::fs::metadata(snapshot).unwrap().len(),
        MAX_FILE_BYTES as u64
    );
}

#[test]
fn opening_same_file_focuses_existing_tab() {
    use branchline::{
        document::Document, file_identity::read_identified, interchange::write_native,
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("original.branchline");
    std::fs::write(&path, write_native(&Document::new("Loaded")).unwrap()).unwrap();
    let mut workspace = Workspace::new();
    let opened = workspace
        .open_native(read_identified(&path).unwrap())
        .unwrap();
    workspace.apply_pending_activation().unwrap();
    assert_eq!(workspace.map().editor.path.as_ref(), Some(&path));
    let alias = temp.path().join("alias.branchline");
    std::fs::hard_link(&path, &alias).unwrap();
    assert_eq!(
        workspace
            .open_native(read_identified(&alias).unwrap())
            .unwrap(),
        opened
    );
    assert_eq!(workspace.len(), 2);
    assert_eq!(
        workspace.session(opened).unwrap().editor.path.as_ref(),
        Some(&path)
    );
    #[cfg(unix)]
    {
        let symlink = temp.path().join("symlink.branchline");
        std::os::unix::fs::symlink(&path, &symlink).unwrap();
        assert_eq!(
            workspace
                .open_native(read_identified(&symlink).unwrap())
                .unwrap(),
            opened
        );
    }
    assert!(read_identified(temp.path()).is_err());
    assert!(read_identified(&temp.path().join("missing")).is_err());
}

#[test]
fn save_as_rejects_another_tabs_destination() {
    use branchline::{
        document::Document, file_identity::read_identified, interchange::write_native,
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("owned.branchline");
    let bytes = write_native(&Document::new("Owner")).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut workspace = Workspace::new();
    let blank = workspace.active_id();
    let owner = workspace
        .open_native(read_identified(&path).unwrap())
        .unwrap();
    assert_eq!(workspace.destination_owner(&path).unwrap(), Some(owner));
    assert!(workspace.save(blank, &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(workspace.session(blank).unwrap().editor.path.is_none());
}

#[test]
fn external_replacement_refreshes_identity() {
    use branchline::{
        document::{Command, Document},
        file_identity::read_identified,
        interchange::{read_native, write_native},
        storage::atomic_write,
    };
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("map.branchline");
    std::fs::write(&path, write_native(&Document::new("Before")).unwrap()).unwrap();
    let mut workspace = Workspace::new();
    let id = workspace
        .open_native(read_identified(&path).unwrap())
        .unwrap();
    let root = workspace.session(id).unwrap().editor.selected.clone();
    workspace
        .apply(
            id,
            EditorEdit::Command(Command::SetText {
                id: root,
                text: "After save".into(),
            }),
        )
        .unwrap();
    workspace.save(id, &path).unwrap();
    assert_eq!(
        workspace
            .open_native(read_identified(&path).unwrap())
            .unwrap(),
        id
    );
    let bytes = std::fs::read(&path).unwrap();
    let document = read_native(&bytes).unwrap();
    assert_eq!(document.nodes[&document.root].text, "After save");
    let captured = read_identified(&path).unwrap();
    atomic_write(
        &path,
        &write_native(&Document::new("External replacement")).unwrap(),
    )
    .unwrap();
    assert!(workspace.open_native(captured).is_err());
    assert!(
        workspace
            .open_native(read_identified(&path).unwrap())
            .is_err()
    );
    assert_eq!(workspace.len(), 2);
}

#[test]
fn import_acceptance_adds_dirty_tab_and_retains_source_drafts() {
    use branchline::interchange::import_mm;
    let mut workspace = Workspace::new();
    let id = workspace.active_id();
    workspace
        .apply(id, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    let node = workspace.map().editor.selected.clone();
    workspace
        .apply(
            id,
            EditorEdit::SetDraft {
                field: DraftField::Inline(node),
                text: "Original draft".into(),
            },
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    assert_eq!(workspace.len(), 1);
    assert_eq!(workspace.map().editor.document, before);
    assert_eq!(
        workspace.map().editor.editing.as_ref().unwrap().text,
        "Original draft"
    );
    let report = import_mm(br#"<map><node TEXT="Imported"/></map>"#).unwrap();
    let imported = workspace.accept_import(report).unwrap();
    assert_eq!(workspace.len(), 2);
    assert!(workspace.session(imported).unwrap().editor.is_dirty());
    assert_eq!(
        workspace
            .session(id)
            .unwrap()
            .editor
            .editing
            .as_ref()
            .unwrap()
            .text,
        "Original draft"
    );
}

#[test]
fn failed_save_retains_uncommitted_drafts_and_checkpoint() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::new();
    let session = workspace.active_id();
    workspace
        .apply(session, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(
            session,
            EditorEdit::SetDraft {
                field: DraftField::Inline(root),
                text: "Do not lose the draft".into(),
            },
        )
        .unwrap();
    let before = workspace.map().editor.document.clone();
    assert!(
        workspace
            .save(session, &temp.path().join("missing-parent/map.branchline"))
            .is_err()
    );
    assert_eq!(workspace.map().editor.document, before);
    assert!(!workspace.map().editor.is_dirty());
    assert_eq!(
        workspace.map().editor.editing.as_ref().unwrap().text,
        "Do not lose the draft"
    );
    assert!(workspace.map().editor.path.is_none());
}

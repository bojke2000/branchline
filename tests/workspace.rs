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
    use branchline::document::{Document, MAX_FILE_BYTES, MAX_NODES, Node, Side};
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
}

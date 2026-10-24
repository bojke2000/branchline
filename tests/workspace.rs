use branchline::{
    app::{DetailsDraft, EditorAction},
    document::MAX_FIELD_BYTES,
    session::{MapSession, SessionId},
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

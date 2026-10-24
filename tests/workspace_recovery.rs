use branchline::{
    app::{DraftField, EditorAction, EditorEdit},
    document::{Command, Document},
    recovery::{RecoveryCoordinator, RestoreContent},
    storage::{RecoveryRecord, RecoveryStore},
    workspace::Workspace,
};
use std::fs;

fn edited(workspace: &mut Workspace, text: &str) {
    let id = workspace.active_id();
    let root = workspace.map().editor.document.root.clone();
    workspace
        .apply(
            id,
            EditorEdit::Command(Command::SetText {
                id: root,
                text: text.into(),
            }),
        )
        .unwrap();
}

fn snapshot_paths(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("snapshot-")
        })
        .collect()
}

#[test]
fn manifest_failure_retains_previous_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    edited(&mut workspace, "Original recovery");
    store.publish(&workspace.recovery_state()).unwrap();
    let before = fs::read(temp.path().join("workspace.json")).unwrap();
    let snapshots = snapshot_paths(temp.path());
    edited(&mut workspace, "Newer draft");
    let mut invalid = workspace.recovery_state();
    invalid.active = Some(branchline::session::SessionId::new());
    assert!(store.publish(&invalid).is_err());
    assert_eq!(
        fs::read(temp.path().join("workspace.json")).unwrap(),
        before
    );
    for path in snapshots {
        let bytes = fs::read(path).unwrap();
        let document = branchline::interchange::read_native(&bytes).unwrap();
        assert_eq!(document.nodes[&document.root].text, "Original recovery");
    }
}

#[test]
fn unchanged_snapshots_are_reused_and_inactive_drafts_survive() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    let source = workspace.active_id();
    workspace
        .apply(source, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    let root = workspace.map().editor.selected.clone();
    workspace
        .apply(
            source,
            EditorEdit::SetDraft {
                field: DraftField::Inline(root),
                text: "Unfinished".into(),
            },
        )
        .unwrap();
    let second = workspace.new_map().unwrap();
    workspace.apply_pending_activation().unwrap();
    edited(&mut workspace, "Second map");
    store.publish(&workspace.recovery_state()).unwrap();
    let mut before = snapshot_paths(temp.path());
    before.sort();
    store.publish(&workspace.recovery_state()).unwrap();
    let mut after = snapshot_paths(temp.path());
    after.sort();
    assert_eq!(before, after);
    assert!(workspace.session(source).unwrap().editor.editing.is_some());
    drop(store);
    let restored = RecoveryCoordinator::open(temp.path().into())
        .unwrap()
        .load()
        .unwrap();
    assert_eq!(restored.active, Some(second));
    assert_eq!(restored.sessions.len(), 2);
    let RestoreContent::Recovered(document) = &restored.sessions[0].content else {
        panic!("draft recovery");
    };
    assert_eq!(document.nodes[&document.root].text, "Unfinished");
}

#[test]
fn malformed_snapshot_retains_valid_siblings_and_can_be_cleaned_when_owned() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    edited(&mut workspace, "First");
    workspace.new_map().unwrap();
    workspace.apply_pending_activation().unwrap();
    edited(&mut workspace, "Second");
    store.publish(&workspace.recovery_state()).unwrap();
    let broken = snapshot_paths(temp.path()).pop().unwrap();
    fs::write(&broken, b"malformed owned document").unwrap();
    let restore = store.load().unwrap();
    assert_eq!(
        restore
            .sessions
            .iter()
            .filter(|session| matches!(session.content, RestoreContent::Recovered(_)))
            .count(),
        1
    );
    assert_eq!(
        restore
            .sessions
            .iter()
            .filter(|session| matches!(session.content, RestoreContent::Failed(_)))
            .count(),
        1
    );
    let empty = branchline::recovery::WorkspaceRecoveryState {
        active: None,
        sessions: vec![],
    };
    store.publish(&empty).unwrap();
    assert!(!broken.exists(), "ownership, not validity, permits removal");
}

#[test]
fn snapshot_cleanup_failure_retries_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    edited(&mut workspace, "Old snapshot");
    store.publish(&workspace.recovery_state()).unwrap();
    let old = snapshot_paths(temp.path()).pop().unwrap();
    let held = temp.path().join("held-owned-file");
    fs::rename(&old, &held).unwrap();
    fs::create_dir(&old).unwrap();
    edited(&mut workspace, "Current snapshot");
    assert!(
        store
            .publish(&workspace.recovery_state())
            .unwrap()
            .cleanup_error
            .is_some()
    );
    assert!(old.is_dir());
    drop(store);
    fs::remove_dir(&old).unwrap();
    fs::rename(&held, &old).unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    store.load().unwrap();
    store.retry_cleanup().unwrap();
    assert!(!old.exists());
    let restore = store.load().unwrap();
    let RestoreContent::Recovered(document) = &restore.sessions[0].content else {
        panic!("current recovery");
    };
    assert_eq!(document.nodes[&document.root].text, "Current snapshot");
}

#[test]
fn replaced_snapshot_is_not_cleanup_authority() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    edited(&mut workspace, "Owned");
    store.publish(&workspace.recovery_state()).unwrap();
    let old = snapshot_paths(temp.path()).pop().unwrap();
    fs::rename(&old, temp.path().join("retained-original")).unwrap();
    fs::write(&old, b"foreign replacement").unwrap();
    edited(&mut workspace, "Updated");
    assert!(
        store
            .publish(&workspace.recovery_state())
            .unwrap()
            .cleanup_error
            .is_some()
    );
    assert_eq!(fs::read(&old).unwrap(), b"foreign replacement");
    assert!(store.retry_cleanup().is_err());
}

#[test]
fn malformed_store_is_retained() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("workspace.json");
    fs::write(&path, b"malformed").unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    assert!(store.load().is_err());
    assert!(store.publish(&Workspace::new().recovery_state()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"malformed");
}

#[test]
fn replaced_manifest_blocks_publication_and_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    edited(&mut workspace, "Retain");
    store.publish(&workspace.recovery_state()).unwrap();
    let path = temp.path().join("workspace.json");
    let replacement = fs::read(&path).unwrap();
    fs::rename(&path, temp.path().join("previous-manifest")).unwrap();
    fs::write(&path, &replacement).unwrap();
    assert!(store.publish(&workspace.recovery_state()).is_err());
    assert!(store.retry_cleanup().is_err());
    assert_eq!(snapshot_paths(temp.path()).len(), 1);
}

#[test]
fn legacy_migration_is_idempotent_and_nonoverwriting() {
    let temp = tempfile::tempdir().unwrap();
    let record = RecoveryRecord {
        version: 1,
        document_id: uuid::Uuid::new_v4().to_string(),
        original_path: None,
        document: Document::new("Legacy recovery"),
    };
    let legacy = RecoveryStore::open(temp.path().into()).unwrap();
    legacy.write(&record).unwrap();
    drop(legacy);
    let original = fs::read(temp.path().join("recovery.json")).unwrap();
    let backup = temp.path().join("recovery-v1.backup.json");
    fs::write(&backup, b"unrelated backup").unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(&backup).unwrap(), b"unrelated backup");
    assert_eq!(
        fs::read(temp.path().join("recovery.json")).unwrap(),
        original
    );
    fs::remove_file(&backup).unwrap();
    let first = store.load().unwrap();
    assert_eq!(first.sessions.len(), 1);
    assert_eq!(fs::read(&backup).unwrap(), original);
    let manifest = fs::read(temp.path().join("workspace.json")).unwrap();
    drop(store);
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    store.load().unwrap();
    assert_eq!(
        fs::read(temp.path().join("workspace.json")).unwrap(),
        manifest
    );
    assert_eq!(fs::read(&backup).unwrap(), original);
}

#[test]
fn exclusive_lock_is_workspace_wide() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    assert!(RecoveryCoordinator::open(temp.path().into()).is_err());
    assert!(RecoveryStore::open(temp.path().into()).is_err());
    drop(store);
    assert!(RecoveryCoordinator::open(temp.path().into()).is_ok());
}

#[test]
fn session_admission_rejects_unpublishable_manifest_metadata() {
    let mut workspace = Workspace::new();
    let mut map = branchline::session::MapSession::new_blank(branchline::session::SessionId::new());
    map.editor.path = Some(std::path::PathBuf::from("\u{0001}".repeat(6 * 1024 * 1024)));
    assert!(workspace.insert_session(map).is_err());
    assert_eq!(workspace.len(), 1);
}

#[test]
fn saved_tabs_restart_from_fresh_disk_and_missing_tabs_remain_errors() {
    let temp = tempfile::tempdir().unwrap();
    let mut workspace = Workspace::new();
    let first = workspace.active_id();
    let path = temp.path().join("saved.branchline");
    workspace.save(first, &path).unwrap();
    let second = workspace.new_map().unwrap();
    workspace.apply_pending_activation().unwrap();
    let missing = temp.path().join("missing.branchline");
    workspace.save(second, &missing).unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().join("recovery")).unwrap();
    store.publish(&workspace.recovery_state()).unwrap();
    fs::write(
        &path,
        branchline::interchange::write_native(&Document::new("Fresh disk")).unwrap(),
    )
    .unwrap();
    fs::remove_file(&missing).unwrap();
    let restored = Workspace::from_restore(store.load().unwrap(), true).unwrap();
    assert_eq!(restored.order(), &[first, second]);
    let first_map = restored.session(first).unwrap();
    assert_eq!(
        first_map.editor.document.nodes[&first_map.editor.document.root].text,
        "Fresh disk"
    );
    assert!(!first_map.editor.is_dirty());
    assert!(matches!(
        restored.entry(second).unwrap(),
        branchline::session::SessionState::RestoreError(_)
    ));
}

#[test]
fn bounded_serialization_accounts_for_escaping_before_allocating_output() {
    let exact = "a".repeat(branchline::document::MAX_FILE_BYTES - 2);
    assert_eq!(
        branchline::storage::serialize_bounded(&exact)
            .unwrap()
            .len(),
        branchline::document::MAX_FILE_BYTES
    );
    assert!(branchline::storage::serialize_bounded(&format!("{exact}a")).is_err());
    let expanded = "\u{0001}".repeat(6 * 1024 * 1024);
    assert!(branchline::storage::serialize_bounded(&expanded).is_err());
}

#[test]
fn native_app_checkpoints_every_tab_without_committing_drafts() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        branchline::ui::NativeApp::new(RecoveryCoordinator::open(temp.path().into()).unwrap())
            .unwrap();
    let source = app.workspace.active_id();
    app.workspace
        .apply(source, EditorEdit::Action(EditorAction::Edit))
        .unwrap();
    let root = app.workspace.map().editor.selected.clone();
    app.workspace
        .apply(
            source,
            EditorEdit::SetDraft {
                field: DraftField::Inline(root),
                text: "Inactive draft".into(),
            },
        )
        .unwrap();
    app.workspace.new_map().unwrap();
    app.workspace.apply_pending_activation().unwrap();
    edited(&mut app.workspace, "Active draft");
    app.checkpoint_recovery().unwrap();
    assert!(
        app.workspace
            .session(source)
            .unwrap()
            .editor
            .editing
            .is_some()
    );
    drop(app);
    let restore = RecoveryCoordinator::open(temp.path().into())
        .unwrap()
        .load()
        .unwrap();
    assert_eq!(restore.sessions.len(), 2);
    let RestoreContent::Recovered(document) = &restore.sessions[0].content else {
        panic!("inactive recovery");
    };
    assert_eq!(document.nodes[&document.root].text, "Inactive draft");
}

#[test]
fn native_app_renders_missing_saved_tab_without_fabricating_a_map() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().join("recovery")).unwrap();
    let mut workspace = Workspace::new();
    let missing = temp.path().join("missing.branchline");
    workspace.save(workspace.active_id(), &missing).unwrap();
    store.publish(&workspace.recovery_state()).unwrap();
    drop(store);
    fs::remove_file(&missing).unwrap();
    let mut app = branchline::ui::NativeApp::new(
        RecoveryCoordinator::open(temp.path().join("recovery")).unwrap(),
    )
    .unwrap();
    assert!(app.workspace.active().is_err());
    let ctx = eframe::egui::Context::default();
    let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ui| app.show(ui));
    output.textures_delta.clear();
    assert!(app.workspace.active().is_err());
}

#[cfg(unix)]
#[test]
fn snapshot_and_manifest_io_failures_retain_committed_bytes() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    edited(&mut workspace, "Committed recovery");
    store.publish(&workspace.recovery_state()).unwrap();
    let before = fs::read(temp.path().join("workspace.json")).unwrap();
    let original_permissions = fs::metadata(temp.path()).unwrap().permissions();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let probe = fs::File::create(temp.path().join("permission-probe"));
    if probe.is_ok() {
        fs::set_permissions(temp.path(), original_permissions).unwrap();
        panic!(
            "Permission fault injection is unavailable for this account; this coverage was not exercised"
        );
    }
    edited(&mut workspace, "Unpublished newer recovery");
    let snapshot_error = store.publish(&workspace.recovery_state());
    let empty = branchline::recovery::WorkspaceRecoveryState {
        active: None,
        sessions: vec![],
    };
    let manifest_error = store.publish(&empty);
    fs::set_permissions(temp.path(), original_permissions).unwrap();
    assert!(
        snapshot_error.is_err(),
        "snapshot creation must report filesystem denial"
    );
    assert!(
        manifest_error.is_err(),
        "manifest publication must report filesystem denial"
    );
    assert_eq!(
        fs::read(temp.path().join("workspace.json")).unwrap(),
        before
    );
    let restore = store.load().unwrap();
    let RestoreContent::Recovered(document) = &restore.sessions[0].content else {
        panic!("committed recovery");
    };
    assert_eq!(document.nodes[&document.root].text, "Committed recovery");
}

#[test]
fn exact_manifest_input_bound_is_accepted_and_one_extra_byte_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let mut bytes = br#"{"version":2,"tabs":[],"active":null,"cleanup":[]}"#.to_vec();
    bytes.resize(branchline::document::MAX_FILE_BYTES, b' ');
    let path = temp.path().join("workspace.json");
    fs::write(&path, &bytes).unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    assert!(store.load().unwrap().sessions.is_empty());
    drop(store);
    bytes.push(b' ');
    fs::write(&path, &bytes).unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::metadata(&path).unwrap().len(), bytes.len() as u64);
}

#[test]
fn unreferenced_snapshot_is_reported_and_never_glob_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let orphan = temp
        .path()
        .join(format!("snapshot-{}.json", uuid::Uuid::new_v4()));
    fs::write(&orphan, b"unknown ownership").unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    assert!(!store.load().unwrap().warnings.is_empty());
    store.publish(&Workspace::new().recovery_state()).unwrap();
    store.retry_cleanup().unwrap();
    assert_eq!(fs::read(&orphan).unwrap(), b"unknown ownership");
}

#[test]
fn restore_prompt_handles_a_failed_active_tab_and_keeps_valid_siblings() {
    use eframe::egui::{self, Event};
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().join("recovery")).unwrap();
    let mut workspace = Workspace::new();
    let first = workspace.active_id();
    let missing = temp.path().join("missing.branchline");
    workspace.save(first, &missing).unwrap();
    let second = workspace.new_map().unwrap();
    workspace.apply_pending_activation().unwrap();
    edited(&mut workspace, "Preserved sibling");
    workspace.request_activate(first).unwrap();
    workspace.apply_pending_activation().unwrap();
    store.publish(&workspace.recovery_state()).unwrap();
    drop(store);
    fs::remove_file(&missing).unwrap();
    let mut app = branchline::ui::NativeApp::new(
        RecoveryCoordinator::open(temp.path().join("recovery")).unwrap(),
    )
    .unwrap();
    let ctx = egui::Context::default();
    let mut render = |events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.show(ui),
        );
        output.textures_delta.clear();
        output
            .shapes
            .into_iter()
            .map(|shape| shape.shape)
            .collect::<Vec<_>>()
    };
    render(vec![]);
    let shapes = render(vec![]);
    fn restore_position(shapes: &[egui::Shape]) -> Option<egui::Pos2> {
        shapes.iter().find_map(|shape| match shape {
            egui::Shape::Text(text) if text.galley.job.text == "Restore" => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            egui::Shape::Vec(children) => restore_position(children),
            _ => None,
        })
    }
    let pos = restore_position(&shapes).expect("recovery prompt");
    for pressed in [true, false] {
        render(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    assert_eq!(app.workspace.active_id(), first);
    assert!(app.workspace.session(first).is_err());
    let map = app.workspace.session(second).unwrap();
    assert_eq!(
        map.editor.document.nodes[&map.editor.document.root].text,
        "Preserved sibling"
    );
    assert!(map.editor.is_dirty());
}

#[test]
fn failed_later_snapshot_reuses_unpublished_owned_siblings_on_retry() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    edited(&mut workspace, "Retain unpublished sibling");
    let field = "\u{0001}".repeat(branchline::document::MAX_FIELD_BYTES);
    let mut document = Document::new(&field);
    let root = document.root.clone();
    let node = document.nodes.get_mut(&root).unwrap();
    node.notes = field.clone();
    node.link = field;
    let mut child = node.clone();
    child.id = "child".into();
    node.children.push(child.id.clone());
    document.nodes.insert(child.id.clone(), child);
    let mut map = branchline::session::MapSession::new_blank(branchline::session::SessionId::new());
    map.editor
        .replace_document(document, None, false, None)
        .unwrap();
    workspace.insert_session(map).unwrap();
    assert!(store.publish(&workspace.recovery_state()).is_err());
    let before = snapshot_paths(temp.path());
    assert_eq!(before.len(), 1);
    assert!(store.publish(&workspace.recovery_state()).is_err());
    assert_eq!(
        snapshot_paths(temp.path()),
        before,
        "retry must not multiply unpublished siblings"
    );
}

#[test]
fn discarding_recovery_does_not_retain_snapshots_for_missing_saved_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().join("recovery")).unwrap();
    let mut workspace = Workspace::new();
    let saved = temp.path().join("missing.branchline");
    workspace.save(workspace.active_id(), &saved).unwrap();
    edited(&mut workspace, "Explicitly discarded edits");
    store.publish(&workspace.recovery_state()).unwrap();
    fs::remove_file(&saved).unwrap();
    let discarded = Workspace::from_restore(store.load().unwrap(), false).unwrap();
    assert!(discarded.active().is_err());
    store.publish(&discarded.recovery_state()).unwrap();
    assert!(
        !store.load().unwrap().has_recovery(),
        "discarded snapshots must not return at next startup"
    );
}

#[test]
fn per_session_debounce_is_not_reset_by_other_maps_and_tracks_view_only_changes() {
    use std::time::{Duration, Instant};
    let temp = tempfile::tempdir().unwrap();
    let mut store = RecoveryCoordinator::open(temp.path().into()).unwrap();
    let mut workspace = Workspace::new();
    let source = workspace.active_id();
    workspace.new_map().unwrap();
    workspace.apply_pending_activation().unwrap();
    let start = Instant::now();
    assert!(!store.checkpoint_due(&workspace.recovery_state(), start));
    store.publish(&workspace.recovery_state()).unwrap();
    let root = workspace.session(source).unwrap().editor.selected.clone();
    workspace
        .apply(
            source,
            EditorEdit::Command(Command::SetText {
                id: root,
                text: "Earlier change".into(),
            }),
        )
        .unwrap();
    assert!(!store.checkpoint_due(
        &workspace.recovery_state(),
        start + Duration::from_millis(100)
    ));
    edited(&mut workspace, "Later change");
    assert!(!store.checkpoint_due(
        &workspace.recovery_state(),
        start + Duration::from_millis(600)
    ));
    assert!(store.checkpoint_due(
        &workspace.recovery_state(),
        start + Duration::from_millis(850)
    ));
    store.publish(&workspace.recovery_state()).unwrap();
    let before = snapshot_paths(temp.path());
    workspace.map_mut().viewport.pan.x = 120.0;
    assert!(!store.checkpoint_due(
        &workspace.recovery_state(),
        start + Duration::from_millis(1600)
    ));
    assert!(store.checkpoint_due(
        &workspace.recovery_state(),
        start + Duration::from_millis(2400)
    ));
    store.publish(&workspace.recovery_state()).unwrap();
    assert_eq!(
        snapshot_paths(temp.path()),
        before,
        "view changes must not rewrite document snapshots"
    );
    let restore = store.load().unwrap();
    assert_eq!(restore.sessions[1].tab.view.pan[0], 120.0);
}

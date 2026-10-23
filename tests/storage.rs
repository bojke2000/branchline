use branchline::{document::{Command, Document, History}, storage::*};
use std::fs;
use uuid::Uuid;

#[test]
fn publication_failure_preserves_original() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("ideas.branchline");
    fs::write(&destination, b"original").unwrap();
    atomic_write(&destination, b"updated").unwrap();
    assert_eq!(fs::read(&destination).unwrap(), b"updated");
    let obstacle = temp.path().join("not-a-directory");
    fs::write(&obstacle, b"untouched").unwrap();
    assert!(atomic_write(&obstacle.join("map"), b"bad").is_err());
    assert_eq!(fs::read(&obstacle).unwrap(), b"untouched");
    let directory = temp.path().join("directory");
    fs::create_dir(&directory).unwrap();
    assert!(atomic_write(&directory, b"bad").is_err());
    assert!(directory.is_dir());
}

#[test]
fn recovery_lock_and_identity() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecoveryStore::open(temp.path().into()).unwrap();
    assert!(RecoveryStore::open(temp.path().into()).is_err());
    let record = RecoveryRecord { version: 1, document_id: Uuid::new_v4().to_string(), original_path: None, document: Document::new("Recovered") };
    store.write(&record).unwrap();
    assert_eq!(store.load().unwrap().unwrap(), record);
    assert!(store.discard("wrong").is_err());
    let other = RecoveryRecord { document_id: Uuid::new_v4().to_string(), ..record.clone() };
    assert!(store.write(&other).is_err());
    store.discard(&record.document_id).unwrap();
    assert!(store.load().unwrap().is_none());
    fs::write(temp.path().join("recovery.json"), b"broken").unwrap();
    assert!(store.load().is_err());
    assert!(temp.path().join("recovery.json").exists());
}

#[test]
fn checkpoint_tracks_undo() {
    let mut doc = Document::new("saved");
    let mut checkpoint = SaveCheckpoint::default();
    checkpoint.mark_saved(&doc);
    let mut history = History::default();
    let root = doc.root.clone();
    history.execute(&mut doc, Command::SetText { id: root, text: "changed".into() }).unwrap();
    assert!(checkpoint.is_dirty(&doc));
    let temp = tempfile::tempdir().unwrap();
    assert!(atomic_write(&temp.path().join("missing/file"), b"x").is_err());
    assert!(checkpoint.is_dirty(&doc));
    history.undo(&mut doc);
    assert!(!checkpoint.is_dirty(&doc));
}

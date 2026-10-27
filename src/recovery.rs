use crate::{
    document::{Document, NodeId},
    file_identity::{FileIdentity, read_identified},
    interchange::{read_native, write_native},
    session::{MapSession, SessionId},
    storage::{
        RecoveryLock, StorageError, atomic_write_opened, read_bounded, read_opened_bounded,
        serialize_bounded, serialized_size_bounded,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;

fn error(cause: impl std::fmt::Display) -> StorageError {
    StorageError(cause.to_string())
}

pub(crate) fn editor_manifest_bound(editor: &crate::app::Editor) -> usize {
    2048usize
        .saturating_add(editor.selected.len().saturating_mul(6))
        .saturating_add(
            editor
                .path
                .as_ref()
                .map_or(0, |path| path.as_os_str().len().saturating_mul(6)),
        )
        .saturating_add(editor.document_id.len().saturating_mul(6))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyRecoveryRecord {
    pub version: u32,
    pub document_id: String,
    pub original_path: Option<PathBuf>,
    pub document: Document,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum FileProof {
    Unix {
        device: u64,
        inode: u64,
        created_ns: u128,
    },
    Windows {
        volume: u64,
        file_id: [u8; 16],
        created: u64,
    },
    Unavailable,
}

impl FileProof {
    fn capture(file: &File) -> Result<Self, StorageError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let metadata = file.metadata().map_err(error)?;
            let created_ns = metadata
                .created()
                .map_err(error)?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(error)?
                .as_nanos();
            Ok(Self::Unix {
                device: metadata.dev(),
                inode: metadata.ino(),
                created_ns,
            })
        }
        #[cfg(windows)]
        {
            use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
            };
            let mut info = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
            // The borrowed file handle stays live and the buffer has the required size.
            if unsafe {
                GetFileInformationByHandleEx(
                    file.as_raw_handle(),
                    FileIdInfo,
                    info.as_mut_ptr().cast(),
                    std::mem::size_of::<FILE_ID_INFO>() as u32,
                )
            } == 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
            // A successful call initializes FILE_ID_INFO.
            let info = unsafe { info.assume_init() };
            Ok(Self::Windows {
                volume: info.VolumeSerialNumber,
                file_id: info.FileId.Identifier,
                created: file.metadata().map_err(error)?.creation_time(),
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = file;
            Ok(Self::Unavailable)
        }
    }

    fn verify(&self, path: &Path) -> Result<(), StorageError> {
        self.open_verified(path).map(|_| ())
    }

    fn open_verified(&self, path: &Path) -> Result<File, StorageError> {
        if matches!(self, Self::Unavailable) {
            return Err(error(format!(
                "{}: Snapshot ownership was not recorded by the creating build; the file was retained.",
                path.display()
            )));
        }
        let metadata = fs::symlink_metadata(path).map_err(error)?;
        if !metadata.is_file() {
            return Err(error(format!(
                "{}: Snapshot ownership is unresolved; not a regular file.",
                path.display()
            )));
        }
        let file = File::open(path).map_err(error)?;
        if &Self::capture(&file)? != self {
            return Err(error(format!(
                "{}: Snapshot identity changed; its replacement was retained.",
                path.display()
            )));
        }
        Ok(file)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRef {
    id: Uuid,
    owner: FileProof,
}

impl SnapshotRef {
    fn path(&self, directory: &Path) -> PathBuf {
        directory.join(format!("snapshot-{}.json", self.id))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedView {
    pub selected: Option<NodeId>,
    pub pan: [f32; 2],
    pub zoom: f32,
}

impl Default for SavedView {
    fn default() -> Self {
        Self {
            selected: None,
            pan: [0.0, 0.0],
            zoom: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryTab {
    pub id: SessionId,
    pub document_id: Uuid,
    pub saved_path: Option<PathBuf>,
    pub snapshot: Option<SnapshotRef>,
    pub view: SavedView,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceManifest {
    version: u32,
    tabs: Vec<RecoveryTab>,
    active: Option<SessionId>,
    cleanup: Vec<SnapshotRef>,
}

impl Default for WorkspaceManifest {
    fn default() -> Self {
        Self {
            version: 2,
            tabs: vec![],
            active: None,
            cleanup: vec![],
        }
    }
}

impl WorkspaceManifest {
    fn validate(&self) -> Result<(), StorageError> {
        if self.version != 2 {
            return Err(error("Unsupported workspace recovery version."));
        }
        let ids: BTreeSet<_> = self.tabs.iter().map(|tab| tab.id).collect();
        if ids.len() != self.tabs.len()
            || self.active.is_some_and(|id| !ids.contains(&id))
            || (self.tabs.is_empty() != self.active.is_none())
        {
            return Err(error("Malformed workspace tab identities or active tab."));
        }
        let mut snapshots = BTreeSet::new();
        for tab in &self.tabs {
            if !tab.view.pan.iter().all(|value| value.is_finite())
                || !tab.view.zoom.is_finite()
                || !(0.01..=3.0).contains(&tab.view.zoom)
            {
                return Err(error("Malformed workspace viewport."));
            }
            if let Some(snapshot) = &tab.snapshot
                && !snapshots.insert(snapshot.id)
            {
                return Err(error("A recovery snapshot has multiple owners."));
            }
        }
        for item in &self.cleanup {
            if !snapshots.insert(item.id) {
                return Err(error("Conflicting snapshot cleanup ownership."));
            }
        }
        Ok(())
    }
}

pub enum RecoverySession<'a> {
    Map(&'a MapSession),
    SavedMap(&'a MapSession),
    Failed {
        id: SessionId,
        path: &'a Option<PathBuf>,
        reason: &'a str,
        retain_snapshot: bool,
    },
}

impl RecoverySession<'_> {
    pub fn id(&self) -> SessionId {
        match self {
            Self::Map(map) | Self::SavedMap(map) => map.id(),
            Self::Failed { id, .. } => *id,
        }
    }
}

pub struct WorkspaceRecoveryState<'a> {
    pub active: Option<SessionId>,
    pub sessions: Vec<RecoverySession<'a>>,
}

pub enum RestoreContent {
    Saved,
    Recovered(Document),
    Failed(String),
}

pub struct RestoredSession {
    pub tab: RecoveryTab,
    pub content: RestoreContent,
}

#[derive(Default)]
pub struct WorkspaceRestore {
    pub active: Option<SessionId>,
    pub sessions: Vec<RestoredSession>,
    pub warnings: Vec<String>,
}

pub struct Publication {
    pub cleanup_error: Option<StorageError>,
}

impl WorkspaceRestore {
    pub fn has_recovery(&self) -> bool {
        self.sessions
            .iter()
            .any(|session| session.tab.snapshot.is_some())
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Revision {
    document: String,
    revision: u64,
    draft: u64,
}

impl Revision {
    fn of(map: &MapSession) -> Self {
        Self {
            document: map.editor.document_id.clone(),
            revision: map.editor.revision,
            draft: map.editor.draft_revision,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Observation {
    revision: Revision,
    path: Option<PathBuf>,
    selected: NodeId,
    view: [u32; 3],
}

impl Observation {
    fn of(map: &MapSession) -> Self {
        Self {
            revision: Revision::of(map),
            path: map.editor.path.clone(),
            selected: map.editor.selected.clone(),
            view: [
                map.viewport.pan.x.to_bits(),
                map.viewport.pan.y.to_bits(),
                map.viewport.zoom.to_bits(),
            ],
        }
    }
}

pub struct RecoveryCoordinator {
    store: RecoveryLock,
    committed: Option<(WorkspaceManifest, FileIdentity, Vec<u8>)>,
    pending_owned: Vec<SnapshotRef>,
    pending_snapshots: BTreeMap<SessionId, (Revision, SnapshotRef)>,
    reused: BTreeMap<SessionId, Revision>,
    observed: BTreeMap<SessionId, (Observation, Instant)>,
    published_observed: BTreeMap<SessionId, Observation>,
    last_attempt: Option<Instant>,
}

impl RecoveryCoordinator {
    pub fn open(directory: PathBuf) -> Result<Self, StorageError> {
        Ok(Self {
            store: RecoveryLock::open(directory)?,
            committed: None,
            pending_owned: vec![],
            pending_snapshots: BTreeMap::new(),
            reused: BTreeMap::new(),
            observed: BTreeMap::new(),
            published_observed: BTreeMap::new(),
            last_attempt: None,
        })
    }

    fn path(&self) -> PathBuf {
        self.store.directory().join("workspace.json")
    }

    fn read_manifest(&mut self) -> Result<WorkspaceManifest, StorageError> {
        let path = self.path();
        match fs::symlink_metadata(&path) {
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                if self.committed.is_some() {
                    return Err(error(
                        "The committed workspace manifest disappeared; recovery writes are blocked.",
                    ));
                }
                return Ok(WorkspaceManifest::default());
            }
            Err(cause) => return Err(error(format!("{}: {cause}", path.display()))),
            Ok(metadata) if !metadata.is_file() => {
                return Err(error("Workspace manifest must be a regular file."));
            }
            Ok(_) => {}
        }
        let opened = read_identified(&path)?;
        if let Some((_, identity, bytes)) = &self.committed {
            if &opened.identity != identity || &opened.bytes != bytes {
                return Err(error(
                    "The workspace manifest changed outside BMM; recovery writes and cleanup are blocked.",
                ));
            }
            return Ok(self.committed.as_ref().expect("checked manifest").0.clone());
        }
        let manifest: WorkspaceManifest = serde_json::from_slice(&opened.bytes).map_err(error)?;
        manifest.validate()?;
        self.committed = Some((manifest.clone(), opened.identity, opened.bytes));
        Ok(manifest)
    }

    fn publish_manifest(&mut self, manifest: &WorkspaceManifest) -> Result<(), StorageError> {
        manifest.validate()?;
        let bytes = serialize_bounded(manifest)?;
        self.read_manifest()?;
        let published = atomic_write_opened(&self.path(), &bytes)?;
        let identity = FileIdentity::from_file(published)?;
        self.committed = Some((manifest.clone(), identity.clone(), bytes));
        if FileIdentity::open(&self.path())? != identity {
            return Err(error(
                "Workspace publication completed but the manifest path was replaced; recovery writes are blocked.",
            ));
        }
        Ok(())
    }

    fn snapshot(&mut self, document: &Document) -> Result<SnapshotRef, StorageError> {
        let bytes = write_native(document).map_err(error)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(self.store.directory()).map_err(error)?;
        temporary.write_all(&bytes).map_err(error)?;
        temporary.as_file().sync_all().map_err(error)?;
        let id = Uuid::new_v4();
        let path = self.store.directory().join(format!("snapshot-{id}.json"));
        let owner = FileProof::capture(temporary.as_file())?;
        temporary.persist_noclobber(&path).map_err(error)?;
        let snapshot = SnapshotRef { id, owner };
        self.pending_owned.push(snapshot.clone());
        Ok(snapshot)
    }

    pub fn load(&mut self) -> Result<WorkspaceRestore, StorageError> {
        let mut manifest = self.read_manifest()?;
        if self.committed.is_none() {
            self.migrate_legacy()?;
            manifest = self.read_manifest()?;
        }
        let mut restore = WorkspaceRestore {
            active: manifest.active,
            ..Default::default()
        };
        let known: BTreeSet<_> = manifest
            .tabs
            .iter()
            .filter_map(|tab| tab.snapshot.as_ref())
            .chain(&manifest.cleanup)
            .map(|item| item.path(self.store.directory()))
            .collect();
        for entry in fs::read_dir(self.store.directory()).map_err(error)? {
            let path = entry.map_err(error)?.path();
            if path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.strip_prefix("snapshot-")
                    .and_then(|name| name.strip_suffix(".json"))
                    .is_some_and(|id| Uuid::parse_str(id).is_ok())
            }) && !known.contains(&path)
            {
                restore.warnings.push(format!(
                    "Unreferenced recovery file retained without cleanup authority: {}",
                    path.display()
                ));
            }
        }
        if !manifest.cleanup.is_empty() {
            restore.warnings.push("Snapshot cleanup is pending; owned files will be retried without deleting replacements.".into());
        }
        let mut retained_bytes = 0usize;
        for tab in manifest.tabs {
            let mut session = self.restore_tab(tab);
            if let RestoreContent::Recovered(document) = &session.content {
                let bytes = document.estimated_bytes();
                if retained_bytes.saturating_add(bytes) > 256 * 1024 * 1024 {
                    session.content = RestoreContent::Failed("Recovery document storage exceeds workspace capacity; this snapshot was retained on disk.".into());
                } else {
                    retained_bytes += bytes;
                }
            }
            restore.sessions.push(session);
        }
        Ok(restore)
    }

    fn restore_tab(&self, tab: RecoveryTab) -> RestoredSession {
        let content = if let Some(snapshot) = &tab.snapshot {
            let path = snapshot.path(self.store.directory());
            match snapshot
                .owner
                .open_verified(&path)
                .and_then(|file| read_opened_bounded(&path, &file))
                .and_then(|bytes| read_native(&bytes).map_err(error))
            {
                Ok(document) => RestoreContent::Recovered(document),
                Err(cause) => RestoreContent::Failed(format!("{}: {cause}", path.display())),
            }
        } else if tab.saved_path.is_some() {
            RestoreContent::Saved
        } else if let Some(reason) = &tab.failure {
            RestoreContent::Failed(reason.clone())
        } else {
            RestoreContent::Saved
        };
        RestoredSession { tab, content }
    }

    pub fn retry_session(&mut self, id: SessionId) -> Result<RestoredSession, StorageError> {
        let manifest = self.read_manifest()?;
        let tab = manifest
            .tabs
            .into_iter()
            .find(|tab| tab.id == id)
            .ok_or_else(|| error("The failed tab has no committed restore source."))?;
        Ok(self.restore_tab(tab))
    }

    fn migrate_legacy(&mut self) -> Result<(), StorageError> {
        let source = self.store.directory().join("recovery.json");
        match fs::symlink_metadata(&source) {
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(cause) => return Err(error(cause)),
            Ok(metadata) if !metadata.is_file() => {
                return Err(error("Legacy recovery must be a regular file."));
            }
            Ok(_) => {}
        }
        let opened = read_identified(&source)?;
        let original = &opened.bytes;
        let record: LegacyRecoveryRecord = serde_json::from_slice(original).map_err(error)?;
        if record.version != 1 || Uuid::parse_str(&record.document_id).is_err() {
            return Err(error("Unsupported or malformed legacy recovery identity."));
        }
        record.document.validate().map_err(error)?;
        let backup = self.store.directory().join("recovery-v1.backup.json");
        match fs::symlink_metadata(&backup) {
            Ok(metadata) if metadata.is_file() => {
                if &read_bounded(&backup)? != original {
                    return Err(error(
                        "The v1 recovery backup contains different bytes; migration was not published.",
                    ));
                }
            }
            Ok(_) => {
                return Err(error(
                    "The v1 backup is not a regular file; migration was not published.",
                ));
            }
            Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
                let mut temporary =
                    tempfile::NamedTempFile::new_in(self.store.directory()).map_err(error)?;
                temporary.write_all(original).map_err(error)?;
                temporary.as_file().sync_all().map_err(error)?;
                temporary.persist_noclobber(&backup).map_err(error)?;
                if &read_bounded(&backup)? != original {
                    return Err(error("Could not verify the v1 recovery backup."));
                }
            }
            Err(cause) => return Err(error(cause)),
        }
        let id = SessionId::new();
        let current = read_identified(&source)?;
        if current.identity != opened.identity || &current.bytes != original {
            return Err(error(
                "Legacy recovery changed during backup verification; migration was not published.",
            ));
        }
        let snapshot = self.snapshot(&record.document)?;
        let manifest = WorkspaceManifest {
            tabs: vec![RecoveryTab {
                id,
                document_id: Uuid::parse_str(&record.document_id).map_err(error)?,
                saved_path: record.original_path,
                snapshot: Some(snapshot),
                view: SavedView::default(),
                failure: None,
            }],
            active: Some(id),
            ..Default::default()
        };
        self.publish_manifest(&manifest)?;
        self.pending_owned.clear();
        Ok(())
    }

    pub fn publish(
        &mut self,
        state: &WorkspaceRecoveryState<'_>,
    ) -> Result<Publication, StorageError> {
        let previous = self.read_manifest()?;
        if self.committed.is_none() {
            match fs::symlink_metadata(self.store.directory().join("recovery.json")) {
                Ok(_) => {
                    return Err(error(
                        "Resolve legacy recovery before publishing another workspace.",
                    ));
                }
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {}
                Err(cause) => return Err(error(cause)),
            }
        }
        let mut manifest = WorkspaceManifest {
            active: state.active,
            ..Default::default()
        };
        for session in &state.sessions {
            let tab = match session {
                RecoverySession::Map(map) | RecoverySession::SavedMap(map) => RecoveryTab {
                    id: map.id(),
                    document_id: Uuid::parse_str(&map.editor.document_id).map_err(error)?,
                    saved_path: map.editor.path.clone(),
                    snapshot: None,
                    failure: None,
                    view: SavedView {
                        selected: Some(map.editor.selected.clone()),
                        pan: [map.viewport.pan.x, map.viewport.pan.y],
                        zoom: map.viewport.zoom,
                    },
                },
                RecoverySession::Failed {
                    id,
                    path,
                    reason,
                    retain_snapshot,
                } => {
                    let Some(prior) = previous.tabs.iter().find(|tab| tab.id == *id) else {
                        return Err(error(
                            "A failed restore tab has no retained recovery ownership.",
                        ));
                    };
                    let mut tab = prior.clone();
                    tab.saved_path = (*path).clone();
                    tab.failure = Some((*reason).into());
                    if !retain_snapshot {
                        tab.snapshot = None;
                    }
                    tab
                }
            };
            manifest.tabs.push(tab);
        }
        manifest.validate()?;
        let mut preflight = manifest.clone();
        preflight.cleanup = previous
            .cleanup
            .iter()
            .cloned()
            .chain(previous.tabs.iter().filter_map(|tab| tab.snapshot.clone()))
            .chain(self.pending_owned.iter().cloned())
            .collect();
        let projected_bytes = serialized_size_bounded(&preflight)?
            .saturating_add(state.sessions.len().saturating_mul(512));
        if projected_bytes > crate::document::MAX_FILE_BYTES {
            return Err(error(
                "Workspace manifest and retained cleanup ownership exceed 32 MiB; no snapshots were published.",
            ));
        }
        let mut next_reused = BTreeMap::new();
        for (session, tab) in state.sessions.iter().zip(&mut manifest.tabs) {
            let RecoverySession::Map(map) = session else {
                continue;
            };
            if !map.editor.is_dirty() && !map.editor.has_drafts().map_err(error)? {
                continue;
            }
            let revision = Revision::of(map);
            if self.reused.get(&map.id()) == Some(&revision)
                && let Some(snapshot) = previous
                    .tabs
                    .iter()
                    .find(|prior| prior.id == map.id())
                    .and_then(|prior| prior.snapshot.clone())
            {
                snapshot
                    .owner
                    .verify(&snapshot.path(self.store.directory()))?;
                tab.snapshot = Some(snapshot);
            } else if let Some((pending_revision, snapshot)) = self.pending_snapshots.get(&map.id())
                && pending_revision == &revision
            {
                snapshot
                    .owner
                    .verify(&snapshot.path(self.store.directory()))?;
                tab.snapshot = Some(snapshot.clone());
            } else {
                let document = map.editor.snapshot_with_drafts().map_err(error)?;
                let snapshot = self.snapshot(&document)?;
                self.pending_snapshots
                    .insert(map.id(), (revision.clone(), snapshot.clone()));
                tab.snapshot = Some(snapshot);
            }
            next_reused.insert(map.id(), revision);
        }
        let referenced: BTreeSet<_> = manifest
            .tabs
            .iter()
            .filter_map(|tab| tab.snapshot.as_ref())
            .map(|snapshot| snapshot.id)
            .collect();
        let mut debts = BTreeMap::new();
        for owned in previous
            .cleanup
            .into_iter()
            .chain(previous.tabs.into_iter().filter_map(|tab| tab.snapshot))
            .chain(self.pending_owned.iter().cloned())
        {
            if !referenced.contains(&owned.id) {
                debts.insert(owned.id, owned);
            }
        }
        manifest.cleanup = debts.into_values().collect();
        self.publish_manifest(&manifest)?;
        self.pending_owned.clear();
        self.pending_snapshots.clear();
        self.reused = next_reused;
        self.published_observed = state
            .sessions
            .iter()
            .filter_map(|session| match session {
                RecoverySession::Map(map) | RecoverySession::SavedMap(map) => {
                    Some((map.id(), Observation::of(map)))
                }
                _ => None,
            })
            .collect();
        Ok(Publication {
            cleanup_error: self.retry_cleanup().err(),
        })
    }

    pub fn retry_cleanup(&mut self) -> Result<(), StorageError> {
        let mut manifest = self.read_manifest()?;
        let mut failures = Vec::new();
        let mut remaining = Vec::new();
        for snapshot in &manifest.cleanup {
            let path = snapshot.path(self.store.directory());
            let removed = match fs::symlink_metadata(&path) {
                Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(cause) => Err(error(cause)),
                Ok(_) => snapshot
                    .owner
                    .verify(&path)
                    .and_then(|_| fs::remove_file(&path).map_err(error)),
            };
            if let Err(cause) = removed {
                failures.push(cause.to_string());
                remaining.push(snapshot.clone());
            }
        }
        if remaining != manifest.cleanup {
            manifest.cleanup = remaining;
            self.publish_manifest(&manifest)?;
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(error(failures.join("\n")))
        }
    }

    pub fn checkpoint_due(&mut self, state: &WorkspaceRecoveryState<'_>, now: Instant) -> bool {
        let mut live = BTreeSet::new();
        let mut due = false;
        for session in &state.sessions {
            let RecoverySession::Map(map) = session else {
                continue;
            };
            live.insert(map.id());
            let revision = Observation::of(map);
            let observed = self
                .observed
                .entry(map.id())
                .or_insert_with(|| (revision.clone(), now));
            if observed.0 != revision {
                *observed = (revision.clone(), now);
            }
            if self.published_observed.get(&map.id()) != Some(&revision)
                && now.saturating_duration_since(observed.1) >= Duration::from_millis(700)
            {
                due = true;
            }
        }
        self.observed.retain(|id, _| live.contains(id));
        let prior_order = self.committed.as_ref().map(|(manifest, _, _)| {
            (
                manifest.active,
                manifest.tabs.iter().map(|tab| tab.id).collect::<Vec<_>>(),
            )
        });
        let order = state
            .sessions
            .iter()
            .map(RecoverySession::id)
            .collect::<Vec<_>>();
        let due = due || prior_order.is_some_and(|prior| prior != (state.active, order));
        if due
            && self.last_attempt.is_none_or(|last| {
                now.saturating_duration_since(last) >= Duration::from_millis(700)
            })
        {
            self.last_attempt = Some(now);
            true
        } else {
            false
        }
    }
}

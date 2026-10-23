use crate::document::{Document, MAX_FILE_BYTES};
use serde::{Deserialize, Serialize};
use std::{fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct StorageError(pub String);

fn failure(path: &Path, cause: impl std::fmt::Display) -> StorageError {
    StorageError(format!("{}: {cause}", path.display()))
}

pub fn read_bounded(path: &Path) -> Result<Vec<u8>, StorageError> {
    let file = File::open(path).map_err(|e| failure(path, e))?;
    if file.metadata().map_err(|e| failure(path, e))?.len() > MAX_FILE_BYTES as u64 {
        return Err(failure(path, "File exceeds 32 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| failure(path, e))?;
    if bytes.len() > MAX_FILE_BYTES { return Err(failure(path, "File exceeds 32 MiB")); }
    Ok(bytes)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    if bytes.len() > MAX_FILE_BYTES { return Err(failure(path, "File exceeds 32 MiB")); }
    let parent = path.parent().ok_or_else(|| failure(path, "No parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| failure(path, e))?;
    temporary.write_all(bytes).map_err(|e| failure(path, e))?;
    temporary.as_file().sync_all().map_err(|e| failure(path, e))?;
    temporary.persist(path).map_err(|e| failure(path, e.error))?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRecord {
    pub version: u32,
    pub document_id: String,
    pub original_path: Option<PathBuf>,
    pub document: Document,
}

pub struct RecoveryStore {
    path: PathBuf,
    _lock: File,
}

impl RecoveryStore {
    pub fn open(directory: PathBuf) -> Result<Self, StorageError> {
        fs::create_dir_all(&directory).map_err(|e| failure(&directory, e))?;
        let lock_path = directory.join("instance.lock");
        let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true)
            .open(&lock_path).map_err(|e| failure(&lock_path, e))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|e| failure(&lock_path, format!("Another Branchline instance may be running; recovery store is locked: {e}")))?;
        Ok(Self { path: directory.join("recovery.json"), _lock: lock })
    }

    pub fn load(&self) -> Result<Option<RecoveryRecord>, StorageError> {
        match fs::symlink_metadata(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(failure(&self.path, e)),
            Ok(metadata) if !metadata.is_file() => return Err(failure(&self.path, "Recovery must be a regular file")),
            Ok(_) => {}
        }
        let bytes = read_bounded(&self.path)?;
        let record: RecoveryRecord = serde_json::from_slice(&bytes).map_err(|e| failure(&self.path, e))?;
        Self::validate(&record).map_err(|e| failure(&self.path, e))?;
        Ok(Some(record))
    }

    fn validate(record: &RecoveryRecord) -> Result<(), StorageError> {
        if record.version != 1 || uuid::Uuid::parse_str(&record.document_id).is_err() {
            return Err(StorageError("Unsupported or malformed recovery identity".into()));
        }
        record.document.validate().map_err(|e| StorageError(e.to_string()))
    }

    pub fn write(&self, record: &RecoveryRecord) -> Result<(), StorageError> {
        Self::validate(record)?;
        if let Some(existing) = self.load()? {
            if existing.document_id != record.document_id {
                return Err(failure(&self.path, "Unresolved recovery belongs to another document"));
            }
        }
        let bytes = serde_json::to_vec(record).map_err(|e| failure(&self.path, e))?;
        atomic_write(&self.path, &bytes)
    }

    pub fn discard(&self, document_id: &str) -> Result<(), StorageError> {
        if let Some(record) = self.load()? {
            if record.document_id != document_id {
                return Err(failure(&self.path, "Cannot discard another document's recovery"));
            }
            fs::remove_file(&self.path).map_err(|e| failure(&self.path, e))?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct SaveCheckpoint {
    saved: Option<Document>,
}

impl SaveCheckpoint {
    pub fn mark_saved(&mut self, document: &Document) {
        self.saved = Some(document.clone());
    }

    pub fn is_dirty(&self, document: &Document) -> bool {
        self.saved.as_ref() != Some(document)
    }
}

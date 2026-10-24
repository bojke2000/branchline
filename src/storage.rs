use crate::document::{Document, MAX_FILE_BYTES};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{0}")]
pub struct StorageError(pub String);

pub fn serialize_bounded<T: Serialize>(value: &T) -> Result<Vec<u8>, StorageError> {
    let size = serialized_size_bounded(value)?;
    let mut bytes = Vec::with_capacity(size);
    serde_json::to_writer(&mut bytes, value).map_err(|error| StorageError(error.to_string()))?;
    Ok(bytes)
}

pub fn serialized_size_bounded<T: Serialize>(value: &T) -> Result<usize, StorageError> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .filter(|size| *size <= MAX_FILE_BYTES)
                .ok_or_else(|| std::io::Error::other("Serialized JSON exceeds 32 MiB"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, value).map_err(|error| StorageError(error.to_string()))?;
    Ok(count.0)
}

fn failure(path: &Path, cause: impl std::fmt::Display) -> StorageError {
    StorageError(format!("{}: {cause}", path.display()))
}

pub fn read_bounded(path: &Path) -> Result<Vec<u8>, StorageError> {
    let file = File::open(path).map_err(|e| failure(path, e))?;
    read_opened_bounded(path, &file)
}

pub(crate) fn read_opened_bounded(path: &Path, file: &File) -> Result<Vec<u8>, StorageError> {
    if file.metadata().map_err(|e| failure(path, e))?.len() > MAX_FILE_BYTES as u64 {
        return Err(failure(path, "File exceeds 32 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| failure(path, e))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(failure(path, "File exceeds 32 MiB"));
    }
    Ok(bytes)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    atomic_write_opened(path, bytes).map(|_| ())
}

pub(crate) fn atomic_write_opened(path: &Path, bytes: &[u8]) -> Result<File, StorageError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(failure(path, "File exceeds 32 MiB"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| failure(path, "No parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| failure(path, e))?;
    temporary.write_all(bytes).map_err(|e| failure(path, e))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| failure(path, e))?;
    temporary.persist(path).map_err(|e| failure(path, e.error))
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
    pub(crate) fn directory(&self) -> &Path {
        self.path
            .parent()
            .expect("recovery store has a parent directory")
    }

    pub fn open(directory: PathBuf) -> Result<Self, StorageError> {
        fs::create_dir_all(&directory).map_err(|e| failure(&directory, e))?;
        let lock_path = directory.join("instance.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| failure(&lock_path, e))?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|e| {
            failure(
                &lock_path,
                format!(
                    "Another Branchline instance may be running; recovery store is locked: {e}"
                ),
            )
        })?;
        Ok(Self {
            path: directory.join("recovery.json"),
            _lock: lock,
        })
    }

    pub fn load(&self) -> Result<Option<RecoveryRecord>, StorageError> {
        match fs::symlink_metadata(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(failure(&self.path, e)),
            Ok(metadata) if !metadata.is_file() => {
                return Err(failure(&self.path, "Recovery must be a regular file"));
            }
            Ok(_) => {}
        }
        let bytes = read_bounded(&self.path)?;
        let record: RecoveryRecord =
            serde_json::from_slice(&bytes).map_err(|e| failure(&self.path, e))?;
        Self::validate(&record).map_err(|e| failure(&self.path, e))?;
        Ok(Some(record))
    }

    pub(crate) fn validate(record: &RecoveryRecord) -> Result<(), StorageError> {
        if record.version != 1 || uuid::Uuid::parse_str(&record.document_id).is_err() {
            return Err(StorageError(
                "Unsupported or malformed recovery identity".into(),
            ));
        }
        record
            .document
            .validate()
            .map_err(|e| StorageError(e.to_string()))
    }

    pub fn write(&self, record: &RecoveryRecord) -> Result<(), StorageError> {
        Self::validate(record)?;
        if let Some(existing) = self.load()?
            && existing.document_id != record.document_id
        {
            return Err(failure(
                &self.path,
                "Unresolved recovery belongs to another document",
            ));
        }
        let bytes = serde_json::to_vec(record).map_err(|e| failure(&self.path, e))?;
        atomic_write(&self.path, &bytes)
    }

    pub fn discard(&self, document_id: &str) -> Result<(), StorageError> {
        if let Some(record) = self.load()? {
            if record.document_id != document_id {
                return Err(failure(
                    &self.path,
                    "Cannot discard another document's recovery",
                ));
            }
            fs::remove_file(&self.path).map_err(|e| failure(&self.path, e))?;
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct SaveCheckpoint {
    saved: Option<Arc<Document>>,
}

impl SaveCheckpoint {
    pub fn mark_saved(&mut self, document: &Document) {
        self.saved = Some(Arc::new(document.clone()));
    }

    pub fn is_dirty(&self, document: &Document) -> bool {
        self.saved.as_deref() != Some(document)
    }

    pub fn estimated_bytes(&self) -> usize {
        self.saved
            .as_ref()
            .map_or(0, |document| document.estimated_bytes())
    }
}

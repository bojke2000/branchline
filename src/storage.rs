use crate::document::{Document, MAX_FILE_BYTES};
use serde::Serialize;
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
    #[cfg(windows)]
    {
        use std::os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        };
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            Storage::FileSystem::{
                FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
                ReOpenFile, ReplaceFileW, SetFileAttributesW,
            },
        };
        // Keep identity without retaining read/write access that blocks Windows replacement.
        let handle = unsafe {
            ReOpenFile(
                temporary.as_file().as_raw_handle(),
                0,
                FILE_SHARE_DELETE | FILE_SHARE_READ | FILE_SHARE_WRITE,
                0,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(failure(path, std::io::Error::last_os_error()));
        }
        // The reopened handle is independently owned and refers to the same file.
        let published = unsafe { File::from_raw_handle(handle) };
        let (file, temporary_path) = temporary.into_parts();
        drop(file);
        if !path.try_exists().map_err(|e| failure(path, e))? {
            temporary_path.persist(path).map_err(|e| failure(path, e))?;
            return Ok(published);
        }
        let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let source: Vec<u16> = temporary_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // Match tempfile's persistence semantics before publishing the replacement.
        if unsafe { SetFileAttributesW(source.as_ptr(), FILE_ATTRIBUTE_NORMAL) } == 0 {
            return Err(failure(path, std::io::Error::last_os_error()));
        }
        // Unlike MoveFileExW, ReplaceFileW supports a destination held open for identity checks.
        if unsafe {
            ReplaceFileW(
                destination.as_ptr(),
                source.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(failure(path, std::io::Error::last_os_error()));
        }
        return Ok(published);
    }
    #[cfg(not(windows))]
    temporary.persist(path).map_err(|e| failure(path, e.error))
}

pub(crate) struct RecoveryLock {
    directory: PathBuf,
    _lock: File,
}

impl RecoveryLock {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
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
            directory,
            _lock: lock,
        })
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

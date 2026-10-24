use crate::storage::{StorageError, read_opened_bounded};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileIdentity(Arc<same_file::Handle>);

impl FileIdentity {
    pub(crate) fn from_file(file: std::fs::File) -> Result<Self, StorageError> {
        same_file::Handle::from_file(file)
            .map(|handle| Self(Arc::new(handle)))
            .map_err(|error| {
                StorageError(format!("Cannot establish published-file identity: {error}"))
            })
    }

    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let handle = same_file::Handle::from_path(path)
            .map_err(|error| StorageError(format!("{}: {error}", path.display())))?;
        if !handle
            .as_file()
            .metadata()
            .map_err(|error| StorageError(format!("{}: {error}", path.display())))?
            .is_file()
        {
            return Err(StorageError(format!(
                "{}: Expected a regular document file",
                path.display()
            )));
        }
        Ok(Self(Arc::new(handle)))
    }
}

pub enum FileIdentityState {
    Unsaved,
    Known(FileIdentity),
    Unresolved(String),
}

pub struct OpenedFile {
    pub selected_path: PathBuf,
    pub identity: FileIdentity,
    pub bytes: Vec<u8>,
}

pub fn read_identified(path: &Path) -> Result<OpenedFile, StorageError> {
    let identity = FileIdentity::open(path)?;
    let bytes = read_opened_bounded(path, identity.0.as_file())?;
    Ok(OpenedFile {
        selected_path: path.into(),
        identity,
        bytes,
    })
}

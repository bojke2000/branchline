use super::*;
use std::fs;

impl Workspace {
    pub fn rename_file(&mut self, id: SessionId, name: &str) -> Result<(), WorkspaceError> {
        self.require_idle()?;
        if name.trim().is_empty()
            || name.contains(['/', '\\', '\0'])
            || Path::new(name)
                .extension()
                .is_none_or(|ext| ext != "branchline")
            || Path::new(name)
                .file_stem()
                .is_none_or(|stem| stem.is_empty())
        {
            return Err(WorkspaceError(
                "Enter a filename ending in .branchline, without directory separators.".into(),
            ));
        }
        let map = self.session(id)?;
        let source = map.editor.path.clone().ok_or_else(|| {
            WorkspaceError("Save the untitled map before renaming its file.".into())
        })?;
        let destination = source.with_file_name(name);
        if source == destination {
            return Ok(());
        }
        let expected = match &map.file_identity {
            FileIdentityState::Known(identity) => identity.clone(),
            _ => {
                return Err(WorkspaceError(
                    "The source file identity is unresolved; save the map before renaming.".into(),
                ));
            }
        };
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| WorkspaceError(format!("{}: {error}", source.display())))?;
        if !metadata.file_type().is_file() {
            return Err(WorkspaceError(
                "Rename requires a regular file, not a symbolic link. Open the original file instead.".into(),
            ));
        }
        let current =
            FileIdentity::open(&source).map_err(|error| WorkspaceError(error.to_string()))?;
        if current != expected {
            return Err(WorkspaceError(
                "The source file was replaced outside BMM; it was not renamed.".into(),
            ));
        }
        if self.destination_owner(&destination)?.is_some() {
            return Err(WorkspaceError(
                "Another open tab owns this destination.".into(),
            ));
        }
        let old_len = source.as_os_str().len();
        let new_len = destination.as_os_str().len();
        if self
            .estimated_bytes()
            .saturating_sub(old_len)
            .saturating_add(new_len)
            > self.storage_limit
            || self
                .manifest_bound()
                .saturating_sub(old_len.saturating_mul(6))
                .saturating_add(new_len.saturating_mul(6))
                > crate::document::MAX_FILE_BYTES
        {
            return Err(WorkspaceError(
                "The new filename exceeds workspace capacity.".into(),
            ));
        }
        // Link creation refuses an existing destination, including one created concurrently.
        fs::hard_link(&source, &destination).map_err(|error| {
            WorkspaceError(format!(
                "Cannot rename to {}: {error}",
                destination.display()
            ))
        })?;
        let removal = FileIdentity::open(&destination)
            .and_then(|identity| {
                if identity == expected {
                    Ok(())
                } else {
                    Err(crate::storage::StorageError(
                        "The source changed during rename.".into(),
                    ))
                }
            })
            .and_then(|_| {
                fs::remove_file(&source)
                    .map_err(|error| crate::storage::StorageError(error.to_string()))
            });
        if let Err(error) = removal {
            return match fs::remove_file(&destination) {
                Ok(()) => Err(WorkspaceError(format!("Rename failed: {error}"))),
                Err(cleanup) => Err(WorkspaceError(format!(
                    "Rename failed: {error}. Could not remove the additional link {}: {cleanup}",
                    destination.display()
                ))),
            };
        }
        self.session_mut(id)?.editor.path = Some(destination);
        Ok(())
    }
}

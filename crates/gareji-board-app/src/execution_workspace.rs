use std::fs;
use std::path::Path;

use gareji_board_domain::{ExecutionWorkspaceConnection, ExecutionWorkspaceKind};

/// UI-facing connection seam for existing local Execution workspaces.
pub struct ExecutionWorkspaceConnector;

impl ExecutionWorkspaceConnector {
    /// Resolve one existing directory into the local Board connection shape.
    ///
    /// # Errors
    ///
    /// Returns a bounded diagnostic when the supplied location is not an
    /// existing directory with a displayable canonical path.
    pub fn connect_local_directory(
        project_id: &str,
        location: &str,
    ) -> Result<ExecutionWorkspaceConnection, String> {
        let supplied = location.trim();
        if supplied.is_empty() {
            return Err("Choose an existing local directory first.".to_owned());
        }
        let canonical = fs::canonicalize(Path::new(supplied)).map_err(|_| {
            "That local directory is unavailable. Check that it still exists and is readable."
                .to_owned()
        })?;
        if !canonical.is_dir() {
            return Err("Choose a directory, not an individual file.".to_owned());
        }
        let location = displayable_canonical_path(&canonical)
            .ok_or_else(|| "That local directory cannot be stored on this system.".to_owned())?;
        Ok(ExecutionWorkspaceConnection {
            project_id: project_id.to_owned(),
            kind: ExecutionWorkspaceKind::LocalDirectory,
            location: Some(location),
        })
    }
}

fn displayable_canonical_path(path: &Path) -> Option<String> {
    let location = path.to_str()?.trim();
    if location.is_empty() {
        return None;
    }
    #[cfg(windows)]
    {
        if let Some(location) = location.strip_prefix(r"\\?\UNC\") {
            return Some(format!(r"\\{location}"));
        }
        if let Some(location) = location.strip_prefix(r"\\?\") {
            return Some(location.to_owned());
        }
    }
    Some(location.to_owned())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn connects_an_existing_directory_using_its_canonical_path() {
        let root = temporary_directory("workspace-connector");
        let connection = ExecutionWorkspaceConnector::connect_local_directory(
            "gareji-board",
            root.to_str().unwrap(),
        )
        .unwrap();

        assert_eq!(connection.project_id, "gareji-board");
        assert_eq!(connection.kind, ExecutionWorkspaceKind::LocalDirectory);
        assert!(Path::new(connection.location.as_deref().unwrap()).is_absolute());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_missing_directories_and_files() {
        let root = temporary_directory("workspace-connector-file");
        let file = root.join("file.txt");
        fs::write(&file, "fixture").unwrap();

        assert!(
            ExecutionWorkspaceConnector::connect_local_directory(
                "gareji-board",
                &root.join("missing").display().to_string(),
            )
            .is_err()
        );
        assert!(
            ExecutionWorkspaceConnector::connect_local_directory(
                "gareji-board",
                &file.display().to_string(),
            )
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory(prefix: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("gareji-board-{prefix}-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}

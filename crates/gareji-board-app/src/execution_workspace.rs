use std::fs;
use std::path::{Path, PathBuf};

use gareji_board_domain::{ExecutionWorkspaceConnection, ExecutionWorkspaceKind};

/// UI-facing connection seam for existing local Execution workspaces.
pub struct ExecutionWorkspaceConnector;

/// Read-only local facts currently observable for one workspace connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionWorkspaceInspection {
    pub availability: WorkspaceAvailability,
    pub repository_root: Option<String>,
}

/// Availability is separate from Runner eligibility and execution authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceAvailability {
    BundledSample,
    Available,
    Unavailable,
}

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

    /// Inspect a stored connection without reading workspace contents or Git state.
    #[must_use]
    pub fn inspect_connection(
        connection: Option<&ExecutionWorkspaceConnection>,
    ) -> ExecutionWorkspaceInspection {
        let Some(connection) = connection else {
            return unavailable_inspection();
        };
        match (connection.kind, connection.location.as_deref()) {
            (ExecutionWorkspaceKind::BundledSample, None) => ExecutionWorkspaceInspection {
                availability: WorkspaceAvailability::BundledSample,
                repository_root: None,
            },
            (ExecutionWorkspaceKind::LocalDirectory, Some(location)) => {
                inspect_local_directory(Path::new(location))
            }
            _ => unavailable_inspection(),
        }
    }
}

fn inspect_local_directory(location: &Path) -> ExecutionWorkspaceInspection {
    let canonical = fs::canonicalize(location)
        .ok()
        .filter(|candidate| candidate.is_dir());
    let Some(canonical) = canonical else {
        return unavailable_inspection();
    };
    ExecutionWorkspaceInspection {
        availability: WorkspaceAvailability::Available,
        repository_root: nearest_git_root(&canonical)
            .and_then(|root| displayable_canonical_path(&root)),
    }
}

fn unavailable_inspection() -> ExecutionWorkspaceInspection {
    ExecutionWorkspaceInspection {
        availability: WorkspaceAvailability::Unavailable,
        repository_root: None,
    }
}

fn nearest_git_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|candidate| {
        let marker = candidate.join(".git");
        (marker.is_dir() || marker.is_file()).then(|| candidate.to_path_buf())
    })
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

    #[test]
    fn inspection_reports_availability_and_the_nearest_git_root() {
        let root = temporary_directory("workspace-inspection");
        let repository = root.join("repository");
        let nested = repository.join("nested/project");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(repository.join(".git")).unwrap();
        let connection = ExecutionWorkspaceConnection {
            project_id: "gareji-board".to_owned(),
            kind: ExecutionWorkspaceKind::LocalDirectory,
            location: Some(nested.display().to_string()),
        };

        let inspection = ExecutionWorkspaceConnector::inspect_connection(Some(&connection));
        assert_eq!(inspection.availability, WorkspaceAvailability::Available);
        assert_eq!(
            inspection.repository_root.as_deref(),
            displayable_canonical_path(&fs::canonicalize(repository).unwrap()).as_deref()
        );

        let unavailable = ExecutionWorkspaceConnector::inspect_connection(None);
        assert_eq!(unavailable.availability, WorkspaceAvailability::Unavailable);
        assert_eq!(unavailable.repository_root, None);
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

//! `SQLite` implementation for Board-owned coordination state.

use std::fs;
use std::path::Path;
use std::time::Duration;

use directories::ProjectDirs;
use gareji_board_domain::{
    ActiveWorkAssessment, PortfolioSnapshot, ProjectHealth, ProjectSummary, WorkItemCounts,
    WorkItemState,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;

/// Deep Module that owns Board schema creation, seeding, and portfolio queries.
pub struct SqliteBoardStore {
    connection: Connection,
}

/// Resolve the shared local Board database used by the desktop app and bridge.
#[must_use]
pub fn default_board_database_path() -> std::path::PathBuf {
    ProjectDirs::from("dev", "Gareji", "Gareji Board").map_or_else(
        || std::path::PathBuf::from("gareji-board.sqlite3"),
        |directories| directories.data_local_dir().join("board.sqlite3"),
    )
}

impl SqliteBoardStore {
    /// Open or create the local Board database.
    ///
    /// # Errors
    ///
    /// Returns a bounded storage error when the directory, database, or schema
    /// cannot be initialized.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(StoreError::CreateDirectory)?;
        }
        let connection = Connection::open(path).map_err(StoreError::Sqlite)?;
        Self::from_connection(connection)
    }

    /// Use the same implementation without filesystem state in tests.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the in-memory database cannot be initialized.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory().map_err(StoreError::Sqlite)?)
    }

    fn from_connection(connection: Connection) -> Result<Self, StoreError> {
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(StoreError::Sqlite)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS board_projects (
                   id TEXT PRIMARY KEY,
                   name TEXT NOT NULL,
                   health TEXT NOT NULL CHECK (health IN ('healthy', 'blocked', 'idle')),
                   execution_cap INTEGER NOT NULL CHECK (execution_cap > 0)
                 );
                 CREATE TABLE IF NOT EXISTS board_work_items (
                   id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   title TEXT NOT NULL,
                   state TEXT NOT NULL CHECK (
                     state IN ('backlog', 'todo', 'in_progress', 'in_review', 'blocked', 'done', 'cancelled')
                   ),
                   FOREIGN KEY (project_id) REFERENCES board_projects(id)
                 );
                 CREATE INDEX IF NOT EXISTS board_work_items_by_project_state
                   ON board_work_items(project_id, state);
                 PRAGMA user_version = 1;",
            )
            .map_err(StoreError::Sqlite)?;
        Ok(Self { connection })
    }

    /// Seed the disposable first-run portfolio only when the database is empty.
    ///
    /// # Errors
    ///
    /// Returns a storage error when the seed transaction cannot complete.
    pub fn seed_sample_if_empty(&mut self) -> Result<bool, StoreError> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM board_projects", [], |row| row.get(0))
            .map_err(StoreError::Sqlite)?;
        if count != 0 {
            return Ok(false);
        }

        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let projects = [
            ("gareji-board", "Gareji Board", "healthy", 2_i64),
            ("gareji-core", "Gareji Core", "healthy", 1_i64),
            (
                "zettelkasten-plugin",
                "Zettelkasten Plugin",
                "blocked",
                1_i64,
            ),
        ];
        for project in projects {
            transaction
                .execute(
                    "INSERT INTO board_projects (id, name, health, execution_cap)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![project.0, project.1, project.2, project.3],
                )
                .map_err(StoreError::Sqlite)?;
        }

        let work_items = [
            (
                "BOARD-1",
                "gareji-board",
                "Build portfolio screen",
                WorkItemState::InProgress,
            ),
            (
                "BOARD-2",
                "gareji-board",
                "Connect existing project",
                WorkItemState::Todo,
            ),
            (
                "CORE-1",
                "gareji-core",
                "Stabilize Runner seam",
                WorkItemState::InReview,
            ),
            (
                "CORE-2",
                "gareji-core",
                "Add active-work registry",
                WorkItemState::Todo,
            ),
            (
                "ZETTEL-1",
                "zettelkasten-plugin",
                "Resolve write destination",
                WorkItemState::Blocked,
            ),
        ];
        for work_item in work_items {
            transaction
                .execute(
                    "INSERT INTO board_work_items (id, project_id, title, state)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![work_item.0, work_item.1, work_item.2, work_item.3.as_str()],
                )
                .map_err(StoreError::Sqlite)?;
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(true)
    }

    /// Load the complete initial portfolio read model in one query.
    ///
    /// # Errors
    ///
    /// Returns a storage error for query failures or corrupt persisted values.
    pub fn load_portfolio(&self) -> Result<PortfolioSnapshot, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT
                   p.id,
                   p.name,
                   p.health,
                   p.execution_cap,
                   COUNT(w.id) AS total,
                   COALESCE(SUM(CASE WHEN w.state = 'todo' THEN 1 ELSE 0 END), 0) AS todo,
                   COALESCE(SUM(CASE WHEN w.state = 'in_progress' THEN 1 ELSE 0 END), 0) AS in_progress,
                   COALESCE(SUM(CASE WHEN w.state = 'in_review' THEN 1 ELSE 0 END), 0) AS in_review,
                   COALESCE(SUM(CASE WHEN w.state = 'blocked' THEN 1 ELSE 0 END), 0) AS blocked,
                   COALESCE(SUM(CASE WHEN w.state = 'done' THEN 1 ELSE 0 END), 0) AS done
                 FROM board_projects p
                 LEFT JOIN board_work_items w ON w.project_id = p.id
                 GROUP BY p.id, p.name, p.health, p.execution_cap
                 ORDER BY p.name",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| {
                Ok(RawProjectSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    health: row.get(2)?,
                    execution_cap: row.get(3)?,
                    total: row.get(4)?,
                    todo: row.get(5)?,
                    in_progress: row.get(6)?,
                    in_review: row.get(7)?,
                    blocked: row.get(8)?,
                    done: row.get(9)?,
                })
            })
            .map_err(StoreError::Sqlite)?;

        let mut projects = Vec::new();
        for row in rows {
            projects.push(row.map_err(StoreError::Sqlite)?.try_into()?);
        }
        Ok(PortfolioSnapshot { projects })
    }

    /// Assess one explicit Work item through Board-owned state and project authority.
    ///
    /// Unknown or unrelated Work item identities share one not-found result so this
    /// Interface does not reveal records from another project.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, storage, or corrupt-state error.
    pub fn assess_active_work(
        &self,
        project_id: &str,
        work_item_id: &str,
    ) -> Result<ActiveWorkAssessment, StoreError> {
        validate_id(project_id)?;
        validate_id(work_item_id)?;
        let row = self
            .connection
            .query_row(
                "SELECT w.id, w.state
                 FROM board_projects p
                 LEFT JOIN board_work_items w
                   ON w.project_id = p.id AND w.id = ?2
                 WHERE p.id = ?1",
                params![project_id, work_item_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .ok_or(StoreError::ProjectNotFound)?;
        let (Some(stored_work_item_id), Some(state)) = row else {
            return Err(StoreError::WorkItemNotFound);
        };
        let state = WorkItemState::try_from(state.as_str())
            .map_err(|_| StoreError::CorruptState("unknown Work item state"))?;
        Ok(ActiveWorkAssessment {
            project_id: project_id.to_owned(),
            work_item_id: stored_work_item_id,
            state,
            eligibility: state.active_work_eligibility(),
        })
    }
}

struct RawProjectSummary {
    id: String,
    name: String,
    health: String,
    execution_cap: i64,
    total: i64,
    todo: i64,
    in_progress: i64,
    in_review: i64,
    blocked: i64,
    done: i64,
}

impl TryFrom<RawProjectSummary> for ProjectSummary {
    type Error = StoreError;

    fn try_from(raw: RawProjectSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            id: raw.id,
            name: raw.name,
            health: ProjectHealth::try_from(raw.health.as_str())
                .map_err(|_| StoreError::CorruptState("unknown project health"))?,
            execution_cap: bounded_u32(raw.execution_cap)?,
            work_items: WorkItemCounts {
                total: bounded_u32(raw.total)?,
                todo: bounded_u32(raw.todo)?,
                in_progress: bounded_u32(raw.in_progress)?,
                in_review: bounded_u32(raw.in_review)?,
                blocked: bounded_u32(raw.blocked)?,
                done: bounded_u32(raw.done)?,
            },
        })
    }
}

fn bounded_u32(value: i64) -> Result<u32, StoreError> {
    u32::try_from(value).map_err(|_| StoreError::CorruptState("numeric value out of range"))
}

fn validate_id(value: &str) -> Result<(), StoreError> {
    let length = value.chars().count();
    if length == 0 || length > 128 {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("request did not satisfy the Board contract")]
    InvalidRequest,
    #[error("Board project was not found")]
    ProjectNotFound,
    #[error("Work item was not found in the requested project")]
    WorkItemNotFound,
    #[error("could not create the Board data directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("Board storage operation failed")]
    Sqlite(#[source] rusqlite::Error),
    #[error("Board storage is corrupt: {0}")]
    CorruptState(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_seed_is_idempotent_and_loads_one_query_read_model() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();

        assert!(store.seed_sample_if_empty().unwrap());
        assert!(!store.seed_sample_if_empty().unwrap());

        let snapshot = store.load_portfolio().unwrap();
        assert_eq!(snapshot.projects.len(), 3);
        assert_eq!(snapshot.active_runs(), 1);
        assert_eq!(snapshot.blocked_items(), 1);
    }

    #[test]
    fn active_work_assessment_preserves_board_state_authority() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();

        let eligible = store.assess_active_work("gareji-core", "CORE-2").unwrap();
        assert_eq!(eligible.state, WorkItemState::Todo);
        assert_eq!(
            eligible.eligibility,
            gareji_board_domain::ActiveWorkEligibility::Eligible
        );

        let blocked = store
            .assess_active_work("zettelkasten-plugin", "ZETTEL-1")
            .unwrap();
        assert_eq!(blocked.state, WorkItemState::Blocked);
        assert_eq!(
            blocked.eligibility,
            gareji_board_domain::ActiveWorkEligibility::Ineligible {
                reason: gareji_board_domain::ActiveWorkIneligibleReason::Blocked
            }
        );
    }

    #[test]
    fn active_work_assessment_hides_unrelated_work_items() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();

        assert!(matches!(
            store.assess_active_work("gareji-board", "CORE-2"),
            Err(StoreError::WorkItemNotFound)
        ));
        assert!(matches!(
            store.assess_active_work("missing", "CORE-2"),
            Err(StoreError::ProjectNotFound)
        ));
    }
}

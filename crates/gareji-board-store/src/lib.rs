//! `SQLite` implementation for Board-owned coordination state.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use directories::ProjectDirs;
use gareji_board_domain::{
    ActiveWorkAssessment, ActivityTimeline, AttachmentReceipt, AttachmentRequest, AttachmentTarget,
    CheckpointAttachment, CheckpointReconciliation, PortfolioSnapshot, ProjectHealth,
    ProjectSummary, ReconciliationDecision, ReconciliationReceipt, ReconciliationRequest,
    WorkItemCounts, WorkItemState, WorkItemSummary, WorkItemTransitionReceipt,
    WorkItemTransitionRequest,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};
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
                   priority INTEGER NOT NULL DEFAULT 100 CHECK (priority > 0),
                   state TEXT NOT NULL CHECK (
                     state IN ('backlog', 'todo', 'in_progress', 'in_review', 'blocked', 'done', 'cancelled')
                   ),
                   FOREIGN KEY (project_id) REFERENCES board_projects(id)
                 );
                 CREATE INDEX IF NOT EXISTS board_work_items_by_project_state
                   ON board_work_items(project_id, state);
                 CREATE TABLE IF NOT EXISTS board_checkpoint_reconciliations (
                   checkpoint_id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   work_item_id TEXT NOT NULL,
                   recommended_state TEXT NOT NULL CHECK (
                     recommended_state IN ('backlog', 'todo', 'in_progress', 'in_review', 'blocked', 'done', 'cancelled')
                   ),
                   decision TEXT NOT NULL CHECK (decision IN ('accepted', 'dismissed')),
                   previous_state TEXT NOT NULL CHECK (
                     previous_state IN ('backlog', 'todo', 'in_progress', 'in_review', 'blocked', 'done', 'cancelled')
                   ),
                   resulting_state TEXT NOT NULL CHECK (
                     resulting_state IN ('backlog', 'todo', 'in_progress', 'in_review', 'blocked', 'done', 'cancelled')
                   ),
                   decided_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                   FOREIGN KEY (project_id) REFERENCES board_projects(id),
                   FOREIGN KEY (work_item_id) REFERENCES board_work_items(id)
                 );
                  CREATE INDEX IF NOT EXISTS board_reconciliations_by_work_item
                    ON board_checkpoint_reconciliations(project_id, work_item_id, decided_at);
                  CREATE TABLE IF NOT EXISTS board_checkpoint_attachments (
                    checkpoint_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    work_item_id TEXT NOT NULL,
                    attached_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    FOREIGN KEY (project_id) REFERENCES board_projects(id),
                    FOREIGN KEY (work_item_id) REFERENCES board_work_items(id)
                  );
                  CREATE INDEX IF NOT EXISTS board_attachments_by_work_item
                    ON board_checkpoint_attachments(project_id, work_item_id, attached_at);",
            )
            .map_err(StoreError::Sqlite)?;
        let has_priority: bool = connection
            .query_row(
                "SELECT EXISTS(
                   SELECT 1
                   FROM pragma_table_info('board_work_items')
                   WHERE name = 'priority'
                 )",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::Sqlite)?;
        if !has_priority {
            connection
                .execute(
                    "ALTER TABLE board_work_items
                     ADD COLUMN priority INTEGER NOT NULL DEFAULT 100 CHECK (priority > 0)",
                    [],
                )
                .map_err(StoreError::Sqlite)?;
        }
        connection
            .execute_batch("PRAGMA user_version = 4;")
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
                1_i64,
                WorkItemState::InProgress,
            ),
            (
                "BOARD-2",
                "gareji-board",
                "Connect existing project",
                2_i64,
                WorkItemState::Todo,
            ),
            (
                "CORE-1",
                "gareji-core",
                "Stabilize Runner seam",
                1_i64,
                WorkItemState::InReview,
            ),
            (
                "CORE-2",
                "gareji-core",
                "Add active-work registry",
                1_i64,
                WorkItemState::Todo,
            ),
            (
                "ZETTEL-1",
                "zettelkasten-plugin",
                "Resolve write destination",
                1_i64,
                WorkItemState::Blocked,
            ),
        ];
        for work_item in work_items {
            transaction
                .execute(
                    "INSERT INTO board_work_items (id, project_id, title, priority, state)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        work_item.0,
                        work_item.1,
                        work_item.2,
                        work_item.3,
                        work_item.4.as_str()
                    ],
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

    /// Load existing Work items available to the Activity Inbox.
    ///
    /// # Errors
    ///
    /// Returns a storage or corrupt-state error.
    pub fn load_work_items(&self) -> Result<Vec<WorkItemSummary>, StoreError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, project_id, title, priority, state
                 FROM board_work_items
                 ORDER BY project_id, priority, id",
            )
            .map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(StoreError::Sqlite)?;
        let mut work_items = Vec::new();
        for row in rows {
            let (id, project_id, title, priority, state) = row.map_err(StoreError::Sqlite)?;
            work_items.push(WorkItemSummary {
                id,
                project_id,
                title,
                priority: positive_u32(priority)?,
                state: parse_work_item_state(&state)?,
            });
        }
        Ok(work_items)
    }

    /// Apply one explicit human Work item transition against observed state.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, concurrent-change, storage, or
    /// corrupt-state error. Selecting the stored state is a successful no-op.
    pub fn transition_work_item(
        &mut self,
        request: &WorkItemTransitionRequest,
    ) -> Result<WorkItemTransitionReceipt, StoreError> {
        validate_id(&request.project_id)?;
        validate_id(&request.work_item_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;
        let stored_state = transaction
            .query_row(
                "SELECT w.state
                 FROM board_projects p
                 LEFT JOIN board_work_items w
                   ON w.project_id = p.id AND w.id = ?2
                 WHERE p.id = ?1",
                params![request.project_id, request.work_item_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .ok_or(StoreError::ProjectNotFound)?
            .ok_or(StoreError::WorkItemNotFound)?;
        let previous_state = parse_work_item_state(&stored_state)?;
        if previous_state != request.expected_state {
            return Err(StoreError::ConcurrentChange);
        }
        if previous_state == request.target_state {
            return Ok(WorkItemTransitionReceipt {
                work_item_id: request.work_item_id.clone(),
                previous_state,
                resulting_state: previous_state,
                changed: false,
            });
        }
        let changed = transaction
            .execute(
                "UPDATE board_work_items
                 SET state = ?1
                 WHERE project_id = ?2 AND id = ?3 AND state = ?4",
                params![
                    request.target_state.as_str(),
                    request.project_id,
                    request.work_item_id,
                    request.expected_state.as_str()
                ],
            )
            .map_err(StoreError::Sqlite)?;
        if changed != 1 {
            return Err(StoreError::ConcurrentChange);
        }
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(WorkItemTransitionReceipt {
            work_item_id: request.work_item_id.clone(),
            previous_state,
            resulting_state: request.target_state,
            changed: true,
        })
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

    /// Attach one project-only Checkpoint to an existing or atomically-created Work item.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, conflict, storage, or corrupt-state
    /// error. Retrying the identical request is successful.
    pub fn attach_checkpoint(
        &mut self,
        request: &AttachmentRequest,
    ) -> Result<AttachmentReceipt, StoreError> {
        validate_id(&request.checkpoint_id)?;
        validate_id(&request.project_id)?;
        let work_item_id = request.target.work_item_id();
        validate_id(work_item_id)?;
        if let AttachmentTarget::New { title, .. } = &request.target {
            validate_title(title)?;
        }
        if let Some(work_item_id) = &request.checkpoint_work_item_id {
            validate_id(work_item_id)?;
            return Err(StoreError::CheckpointAlreadyLinked);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;

        if let Some(existing) = load_attachment(&transaction, &request.checkpoint_id)? {
            if existing.project_id == request.project_id
                && existing.attachment.work_item_id == work_item_id
            {
                return Ok(AttachmentReceipt {
                    checkpoint_id: request.checkpoint_id.clone(),
                    duplicate: true,
                    created_work_item: false,
                    attachment: existing.attachment,
                });
            }
            return Err(StoreError::AlreadyAttached);
        }

        let created_work_item = match &request.target {
            AttachmentTarget::Existing { .. } => {
                transaction
                    .query_row(
                        "SELECT w.id
                         FROM board_projects p
                         LEFT JOIN board_work_items w
                           ON w.project_id = p.id AND w.id = ?2
                         WHERE p.id = ?1",
                        params![request.project_id, work_item_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()
                    .map_err(StoreError::Sqlite)?
                    .ok_or(StoreError::ProjectNotFound)?
                    .ok_or(StoreError::WorkItemNotFound)?;
                false
            }
            AttachmentTarget::New { title, .. } => {
                transaction
                    .query_row(
                        "SELECT 1 FROM board_projects WHERE id = ?1",
                        [&request.project_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(StoreError::Sqlite)?
                    .ok_or(StoreError::ProjectNotFound)?;
                let existing = transaction
                    .query_row(
                        "SELECT 1 FROM board_work_items WHERE id = ?1",
                        [work_item_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(StoreError::Sqlite)?;
                if existing.is_some() {
                    return Err(StoreError::WorkItemAlreadyExists);
                }
                transaction
                    .execute(
                        "INSERT INTO board_work_items (id, project_id, title, state)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![
                            work_item_id,
                            request.project_id,
                            title.trim(),
                            WorkItemState::Todo.as_str()
                        ],
                    )
                    .map_err(StoreError::Sqlite)?;
                true
            }
        };

        transaction
            .execute(
                "INSERT INTO board_checkpoint_attachments (
                   checkpoint_id, project_id, work_item_id
                 ) VALUES (?1, ?2, ?3)",
                params![request.checkpoint_id, request.project_id, work_item_id],
            )
            .map_err(StoreError::Sqlite)?;
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(AttachmentReceipt {
            checkpoint_id: request.checkpoint_id.clone(),
            duplicate: false,
            created_work_item,
            attachment: CheckpointAttachment {
                work_item_id: work_item_id.to_owned(),
            },
        })
    }

    /// Record one final Checkpoint judgment and atomically apply an accepted state.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, lookup, conflict, transition, storage, or
    /// corrupt-state error. Retrying the identical request is successful.
    pub fn reconcile_checkpoint(
        &mut self,
        request: &ReconciliationRequest,
    ) -> Result<ReconciliationReceipt, StoreError> {
        validate_id(&request.checkpoint_id)?;
        validate_id(&request.project_id)?;
        validate_id(&request.work_item_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::Sqlite)?;

        if let Some(existing) = load_reconciliation(&transaction, &request.checkpoint_id)? {
            if existing.project_id == request.project_id
                && existing.work_item_id == request.work_item_id
                && existing.reconciliation.recommended_state == request.recommended_state
                && existing.reconciliation.decision == request.decision
            {
                return Ok(ReconciliationReceipt {
                    checkpoint_id: request.checkpoint_id.clone(),
                    duplicate: true,
                    reconciliation: existing.reconciliation,
                });
            }
            return Err(StoreError::AlreadyReconciled);
        }

        let current_state = transaction
            .query_row(
                "SELECT w.state
                 FROM board_projects p
                 LEFT JOIN board_work_items w
                   ON w.project_id = p.id AND w.id = ?2
                 WHERE p.id = ?1",
                params![request.project_id, request.work_item_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(StoreError::Sqlite)?
            .ok_or(StoreError::ProjectNotFound)?
            .ok_or(StoreError::WorkItemNotFound)?;
        let previous_state = parse_work_item_state(&current_state)?;
        let resulting_state = match request.decision {
            ReconciliationDecision::Accepted => {
                if !previous_state.can_accept_recommendation(request.recommended_state) {
                    return Err(StoreError::UnsupportedReconciliation);
                }
                request.recommended_state
            }
            ReconciliationDecision::Dismissed => previous_state,
        };

        if resulting_state != previous_state {
            let updated = transaction
                .execute(
                    "UPDATE board_work_items
                     SET state = ?3
                     WHERE project_id = ?1 AND id = ?2 AND state = ?4",
                    params![
                        request.project_id,
                        request.work_item_id,
                        resulting_state.as_str(),
                        previous_state.as_str()
                    ],
                )
                .map_err(StoreError::Sqlite)?;
            if updated != 1 {
                return Err(StoreError::ConcurrentChange);
            }
        }
        transaction
            .execute(
                "INSERT INTO board_checkpoint_reconciliations (
                   checkpoint_id, project_id, work_item_id, recommended_state,
                   decision, previous_state, resulting_state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    request.checkpoint_id,
                    request.project_id,
                    request.work_item_id,
                    request.recommended_state.as_str(),
                    request.decision.as_str(),
                    previous_state.as_str(),
                    resulting_state.as_str()
                ],
            )
            .map_err(StoreError::Sqlite)?;
        transaction.commit().map_err(StoreError::Sqlite)?;
        Ok(ReconciliationReceipt {
            checkpoint_id: request.checkpoint_id.clone(),
            duplicate: false,
            reconciliation: CheckpointReconciliation {
                decision: request.decision,
                recommended_state: request.recommended_state,
                previous_state,
                resulting_state,
            },
        })
    }

    /// Add Board-owned attachments and reconciliation decisions to Core activity.
    ///
    /// # Errors
    ///
    /// Returns a bounded validation, storage, or corrupt-state error.
    pub fn hydrate_activity(&self, timeline: &mut ActivityTimeline) -> Result<(), StoreError> {
        if timeline.activities.is_empty() {
            return Ok(());
        }
        if timeline.activities.len() > 100 {
            return Err(StoreError::InvalidRequest);
        }
        for activity in &timeline.activities {
            validate_id(&activity.checkpoint_id)?;
        }
        self.hydrate_activity_attachments(timeline)?;
        self.hydrate_activity_reconciliations(timeline)
    }

    fn hydrate_activity_attachments(
        &self,
        timeline: &mut ActivityTimeline,
    ) -> Result<(), StoreError> {
        let placeholders = std::iter::repeat_n("?", timeline.activities.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT checkpoint_id, project_id, work_item_id
             FROM board_checkpoint_attachments
             WHERE checkpoint_id IN ({placeholders})"
        );
        let mut statement = self.connection.prepare(&sql).map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map(
                params_from_iter(
                    timeline
                        .activities
                        .iter()
                        .map(|activity| &activity.checkpoint_id),
                ),
                |row| {
                    Ok(StoredAttachment {
                        checkpoint_id: row.get(0)?,
                        project_id: row.get(1)?,
                        attachment: CheckpointAttachment {
                            work_item_id: row.get(2)?,
                        },
                    })
                },
            )
            .map_err(StoreError::Sqlite)?;
        let mut attachments = HashMap::new();
        for row in rows {
            let stored = row.map_err(StoreError::Sqlite)?;
            attachments.insert(stored.checkpoint_id.clone(), stored);
        }
        for activity in &mut timeline.activities {
            let Some(stored) = attachments.remove(&activity.checkpoint_id) else {
                continue;
            };
            if stored.project_id != activity.project_id || activity.work_item_id.is_some() {
                return Err(StoreError::CorruptState(
                    "attachment does not match its Checkpoint",
                ));
            }
            activity.attachment = Some(stored.attachment);
        }
        Ok(())
    }

    fn hydrate_activity_reconciliations(
        &self,
        timeline: &mut ActivityTimeline,
    ) -> Result<(), StoreError> {
        let placeholders = std::iter::repeat_n("?", timeline.activities.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT checkpoint_id, project_id, work_item_id, recommended_state,
                    decision, previous_state, resulting_state
             FROM board_checkpoint_reconciliations
             WHERE checkpoint_id IN ({placeholders})"
        );
        let mut statement = self.connection.prepare(&sql).map_err(StoreError::Sqlite)?;
        let rows = statement
            .query_map(
                params_from_iter(
                    timeline
                        .activities
                        .iter()
                        .map(|activity| &activity.checkpoint_id),
                ),
                |row| {
                    Ok(RawReconciliation {
                        checkpoint_id: row.get(0)?,
                        project_id: row.get(1)?,
                        work_item_id: row.get(2)?,
                        recommended_state: row.get(3)?,
                        decision: row.get(4)?,
                        previous_state: row.get(5)?,
                        resulting_state: row.get(6)?,
                    })
                },
            )
            .map_err(StoreError::Sqlite)?;
        let mut reconciliations = HashMap::new();
        for row in rows {
            let stored = StoredReconciliation::try_from(row.map_err(StoreError::Sqlite)?)?;
            reconciliations.insert(stored.checkpoint_id.clone(), stored);
        }
        for activity in &mut timeline.activities {
            let Some(stored) = reconciliations.remove(&activity.checkpoint_id) else {
                continue;
            };
            if stored.project_id != activity.project_id
                || activity.effective_work_item_id() != Some(stored.work_item_id.as_str())
                || activity.recommended_state != Some(stored.reconciliation.recommended_state)
            {
                return Err(StoreError::CorruptState(
                    "reconciliation does not match its Checkpoint",
                ));
            }
            activity.reconciliation = Some(stored.reconciliation);
        }
        Ok(())
    }
}

struct StoredAttachment {
    checkpoint_id: String,
    project_id: String,
    attachment: CheckpointAttachment,
}

fn load_attachment(
    connection: &Connection,
    checkpoint_id: &str,
) -> Result<Option<StoredAttachment>, StoreError> {
    connection
        .query_row(
            "SELECT checkpoint_id, project_id, work_item_id
             FROM board_checkpoint_attachments
             WHERE checkpoint_id = ?1",
            [checkpoint_id],
            |row| {
                Ok(StoredAttachment {
                    checkpoint_id: row.get(0)?,
                    project_id: row.get(1)?,
                    attachment: CheckpointAttachment {
                        work_item_id: row.get(2)?,
                    },
                })
            },
        )
        .optional()
        .map_err(StoreError::Sqlite)
}

struct RawReconciliation {
    checkpoint_id: String,
    project_id: String,
    work_item_id: String,
    recommended_state: String,
    decision: String,
    previous_state: String,
    resulting_state: String,
}

struct StoredReconciliation {
    checkpoint_id: String,
    project_id: String,
    work_item_id: String,
    reconciliation: CheckpointReconciliation,
}

impl TryFrom<RawReconciliation> for StoredReconciliation {
    type Error = StoreError;

    fn try_from(raw: RawReconciliation) -> Result<Self, Self::Error> {
        Ok(Self {
            checkpoint_id: raw.checkpoint_id,
            project_id: raw.project_id,
            work_item_id: raw.work_item_id,
            reconciliation: CheckpointReconciliation {
                decision: ReconciliationDecision::try_from(raw.decision.as_str())
                    .map_err(|_| StoreError::CorruptState("unknown reconciliation decision"))?,
                recommended_state: parse_work_item_state(&raw.recommended_state)?,
                previous_state: parse_work_item_state(&raw.previous_state)?,
                resulting_state: parse_work_item_state(&raw.resulting_state)?,
            },
        })
    }
}

fn load_reconciliation(
    connection: &Connection,
    checkpoint_id: &str,
) -> Result<Option<StoredReconciliation>, StoreError> {
    let raw = connection
        .query_row(
            "SELECT checkpoint_id, project_id, work_item_id, recommended_state,
                    decision, previous_state, resulting_state
             FROM board_checkpoint_reconciliations
             WHERE checkpoint_id = ?1",
            [checkpoint_id],
            |row| {
                Ok(RawReconciliation {
                    checkpoint_id: row.get(0)?,
                    project_id: row.get(1)?,
                    work_item_id: row.get(2)?,
                    recommended_state: row.get(3)?,
                    decision: row.get(4)?,
                    previous_state: row.get(5)?,
                    resulting_state: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::Sqlite)?;
    raw.map(StoredReconciliation::try_from).transpose()
}

fn parse_work_item_state(value: &str) -> Result<WorkItemState, StoreError> {
    WorkItemState::try_from(value).map_err(|_| StoreError::CorruptState("unknown Work item state"))
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

fn positive_u32(value: i64) -> Result<u32, StoreError> {
    match bounded_u32(value)? {
        0 => Err(StoreError::CorruptState("numeric value must be positive")),
        value => Ok(value),
    }
}

fn validate_id(value: &str) -> Result<(), StoreError> {
    let length = value.chars().count();
    if length == 0 || length > 128 {
        return Err(StoreError::InvalidRequest);
    }
    Ok(())
}

fn validate_title(value: &str) -> Result<(), StoreError> {
    let length = value.trim().chars().count();
    if length == 0 || length > 256 {
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
    #[error("a Work item with this ID already exists")]
    WorkItemAlreadyExists,
    #[error("this Checkpoint already has a Work item link")]
    CheckpointAlreadyLinked,
    #[error("this Checkpoint is already attached to another Work item")]
    AlreadyAttached,
    #[error("this Checkpoint already has a final reconciliation decision")]
    AlreadyReconciled,
    #[error("the recommended Work item transition requires a separate explicit action")]
    UnsupportedReconciliation,
    #[error("the Work item changed before the operation was applied")]
    ConcurrentChange,
    #[error("could not create the Board data directory")]
    CreateDirectory(#[source] std::io::Error),
    #[error("Board storage operation failed")]
    Sqlite(#[source] rusqlite::Error),
    #[error("Board storage is corrupt: {0}")]
    CorruptState(&'static str),
}

#[cfg(test)]
mod tests {
    use gareji_board_domain::{CheckpointOutcome, CheckpointSource, ProgressActivity};

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
    fn explicit_transition_updates_state_and_rejects_stale_observations() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let mut request = WorkItemTransitionRequest {
            project_id: "gareji-core".to_owned(),
            work_item_id: "CORE-2".to_owned(),
            expected_state: WorkItemState::Todo,
            target_state: WorkItemState::Cancelled,
        };

        let receipt = store.transition_work_item(&request).unwrap();
        assert!(receipt.changed);
        assert_eq!(receipt.previous_state, WorkItemState::Todo);
        assert_eq!(receipt.resulting_state, WorkItemState::Cancelled);

        request.target_state = WorkItemState::Done;
        assert!(matches!(
            store.transition_work_item(&request),
            Err(StoreError::ConcurrentChange)
        ));

        request.expected_state = WorkItemState::Cancelled;
        request.target_state = WorkItemState::Backlog;
        assert!(store.transition_work_item(&request).unwrap().changed);
        request.expected_state = WorkItemState::Backlog;
        let no_change = store.transition_work_item(&request).unwrap();
        assert!(!no_change.changed);
        assert_eq!(no_change.resulting_state, WorkItemState::Backlog);

        let stored = store
            .load_work_items()
            .unwrap()
            .into_iter()
            .find(|work_item| work_item.id == "CORE-2")
            .unwrap();
        assert_eq!(stored.state, WorkItemState::Backlog);
    }

    #[test]
    fn explicit_transition_hides_cross_project_work_items() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();

        assert!(matches!(
            store.transition_work_item(&WorkItemTransitionRequest {
                project_id: "gareji-board".to_owned(),
                work_item_id: "CORE-1".to_owned(),
                expected_state: WorkItemState::InReview,
                target_state: WorkItemState::Done,
            }),
            Err(StoreError::WorkItemNotFound)
        ));
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

    #[test]
    fn accepted_reconciliation_updates_state_and_hydrates_activity() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let request = ReconciliationRequest {
            checkpoint_id: "cp-accept".to_owned(),
            project_id: "gareji-core".to_owned(),
            work_item_id: "CORE-2".to_owned(),
            recommended_state: WorkItemState::InReview,
            decision: ReconciliationDecision::Accepted,
        };

        let receipt = store.reconcile_checkpoint(&request).unwrap();
        assert!(!receipt.duplicate);
        assert_eq!(receipt.reconciliation.previous_state, WorkItemState::Todo);
        assert_eq!(
            receipt.reconciliation.resulting_state,
            WorkItemState::InReview
        );
        assert_eq!(
            store
                .assess_active_work("gareji-core", "CORE-2")
                .unwrap()
                .state,
            WorkItemState::InReview
        );
        assert!(store.reconcile_checkpoint(&request).unwrap().duplicate);

        let mut timeline = ActivityTimeline {
            activities: vec![activity(
                "cp-accept",
                "gareji-core",
                Some("CORE-2"),
                Some(WorkItemState::InReview),
            )],
            has_older: false,
        };
        store.hydrate_activity(&mut timeline).unwrap();
        assert_eq!(
            timeline.activities[0]
                .reconciliation
                .as_ref()
                .unwrap()
                .decision,
            ReconciliationDecision::Accepted
        );
    }

    #[test]
    fn dismissed_reconciliation_preserves_state_and_is_final() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let mut request = ReconciliationRequest {
            checkpoint_id: "cp-dismiss".to_owned(),
            project_id: "gareji-core".to_owned(),
            work_item_id: "CORE-1".to_owned(),
            recommended_state: WorkItemState::Done,
            decision: ReconciliationDecision::Dismissed,
        };

        let receipt = store.reconcile_checkpoint(&request).unwrap();
        assert_eq!(
            receipt.reconciliation.resulting_state,
            WorkItemState::InReview
        );
        request.decision = ReconciliationDecision::Accepted;
        assert!(matches!(
            store.reconcile_checkpoint(&request),
            Err(StoreError::AlreadyReconciled)
        ));
        assert_eq!(
            store
                .assess_active_work("gareji-core", "CORE-1")
                .unwrap()
                .state,
            WorkItemState::InReview
        );
    }

    #[test]
    fn unsupported_recommendation_does_not_write_a_decision() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let mut request = ReconciliationRequest {
            checkpoint_id: "cp-unsupported".to_owned(),
            project_id: "gareji-core".to_owned(),
            work_item_id: "CORE-1".to_owned(),
            recommended_state: WorkItemState::Todo,
            decision: ReconciliationDecision::Accepted,
        };

        assert!(matches!(
            store.reconcile_checkpoint(&request),
            Err(StoreError::UnsupportedReconciliation)
        ));
        request.decision = ReconciliationDecision::Dismissed;
        assert!(store.reconcile_checkpoint(&request).is_ok());
    }

    #[test]
    fn attachment_resolves_inbox_activity_and_is_idempotent() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let request = AttachmentRequest {
            checkpoint_id: "cp-inbox".to_owned(),
            project_id: "gareji-core".to_owned(),
            checkpoint_work_item_id: None,
            target: AttachmentTarget::Existing {
                work_item_id: "CORE-2".to_owned(),
            },
        };

        let receipt = store.attach_checkpoint(&request).unwrap();
        assert!(!receipt.duplicate);
        assert!(!receipt.created_work_item);
        assert_eq!(receipt.attachment.work_item_id, "CORE-2");
        assert!(store.attach_checkpoint(&request).unwrap().duplicate);

        let mut timeline = ActivityTimeline {
            activities: vec![activity("cp-inbox", "gareji-core", None, None)],
            has_older: false,
        };
        store.hydrate_activity(&mut timeline).unwrap();
        assert_eq!(timeline.inbox_count(), 0);
        assert_eq!(
            timeline.activities[0].effective_work_item_id(),
            Some("CORE-2")
        );
        assert_eq!(timeline.activities[0].work_item_id, None);
    }

    #[test]
    fn new_work_item_is_created_and_attached_atomically() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let request = AttachmentRequest {
            checkpoint_id: "cp-new-work".to_owned(),
            project_id: "gareji-core".to_owned(),
            checkpoint_work_item_id: None,
            target: AttachmentTarget::New {
                work_item_id: "CORE-3".to_owned(),
                title: "Capture Activity Inbox follow-up".to_owned(),
            },
        };

        let receipt = store.attach_checkpoint(&request).unwrap();
        assert!(!receipt.duplicate);
        assert!(receipt.created_work_item);
        assert_eq!(receipt.attachment.work_item_id, "CORE-3");

        let work_item = store
            .load_work_items()
            .unwrap()
            .into_iter()
            .find(|work_item| work_item.id == "CORE-3")
            .unwrap();
        assert_eq!(work_item.project_id, "gareji-core");
        assert_eq!(work_item.title, "Capture Activity Inbox follow-up");
        assert_eq!(work_item.state, WorkItemState::Todo);

        let duplicate = store.attach_checkpoint(&request).unwrap();
        assert!(duplicate.duplicate);
        assert!(!duplicate.created_work_item);
        assert_eq!(
            store
                .load_work_items()
                .unwrap()
                .iter()
                .filter(|work_item| work_item.id == "CORE-3")
                .count(),
            1
        );

        let mut timeline = ActivityTimeline {
            activities: vec![activity("cp-new-work", "gareji-core", None, None)],
            has_older: false,
        };
        store.hydrate_activity(&mut timeline).unwrap();
        assert_eq!(timeline.inbox_count(), 0);
        assert_eq!(
            timeline.activities[0].effective_work_item_id(),
            Some("CORE-3")
        );
    }

    #[test]
    fn failed_work_item_creation_leaves_checkpoint_available_for_attachment() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let mut request = AttachmentRequest {
            checkpoint_id: "cp-atomic".to_owned(),
            project_id: "gareji-core".to_owned(),
            checkpoint_work_item_id: None,
            target: AttachmentTarget::New {
                work_item_id: "CORE-1".to_owned(),
                title: "Conflicting identity".to_owned(),
            },
        };

        assert!(matches!(
            store.attach_checkpoint(&request),
            Err(StoreError::WorkItemAlreadyExists)
        ));

        request.target = AttachmentTarget::Existing {
            work_item_id: "CORE-2".to_owned(),
        };
        assert!(store.attach_checkpoint(&request).is_ok());
    }

    #[test]
    fn attachment_is_final_and_rejects_linked_or_cross_project_activity() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        let mut request = AttachmentRequest {
            checkpoint_id: "cp-final".to_owned(),
            project_id: "gareji-core".to_owned(),
            checkpoint_work_item_id: None,
            target: AttachmentTarget::Existing {
                work_item_id: "CORE-1".to_owned(),
            },
        };
        store.attach_checkpoint(&request).unwrap();

        request.target = AttachmentTarget::Existing {
            work_item_id: "CORE-2".to_owned(),
        };
        assert!(matches!(
            store.attach_checkpoint(&request),
            Err(StoreError::AlreadyAttached)
        ));

        request.checkpoint_id = "cp-linked".to_owned();
        request.checkpoint_work_item_id = Some("CORE-1".to_owned());
        assert!(matches!(
            store.attach_checkpoint(&request),
            Err(StoreError::CheckpointAlreadyLinked)
        ));

        request.checkpoint_id = "cp-cross-project".to_owned();
        request.checkpoint_work_item_id = None;
        request.project_id = "gareji-board".to_owned();
        assert!(matches!(
            store.attach_checkpoint(&request),
            Err(StoreError::WorkItemNotFound)
        ));
    }

    #[test]
    fn attached_activity_can_be_reconciled_through_its_effective_link() {
        let mut store = SqliteBoardStore::open_in_memory().unwrap();
        store.seed_sample_if_empty().unwrap();
        store
            .attach_checkpoint(&AttachmentRequest {
                checkpoint_id: "cp-attached-review".to_owned(),
                project_id: "gareji-core".to_owned(),
                checkpoint_work_item_id: None,
                target: AttachmentTarget::Existing {
                    work_item_id: "CORE-2".to_owned(),
                },
            })
            .unwrap();
        store
            .reconcile_checkpoint(&ReconciliationRequest {
                checkpoint_id: "cp-attached-review".to_owned(),
                project_id: "gareji-core".to_owned(),
                work_item_id: "CORE-2".to_owned(),
                recommended_state: WorkItemState::InReview,
                decision: ReconciliationDecision::Accepted,
            })
            .unwrap();
        let mut timeline = ActivityTimeline {
            activities: vec![activity(
                "cp-attached-review",
                "gareji-core",
                None,
                Some(WorkItemState::InReview),
            )],
            has_older: false,
        };

        store.hydrate_activity(&mut timeline).unwrap();
        assert_eq!(
            timeline.activities[0]
                .reconciliation
                .as_ref()
                .unwrap()
                .decision,
            ReconciliationDecision::Accepted
        );
    }

    #[test]
    fn opening_a_v2_database_adds_current_coordination_storage() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE board_projects (
                   id TEXT PRIMARY KEY,
                   name TEXT NOT NULL,
                   health TEXT NOT NULL,
                   execution_cap INTEGER NOT NULL
                 );
                  CREATE TABLE board_work_items (
                   id TEXT PRIMARY KEY,
                   project_id TEXT NOT NULL,
                   title TEXT NOT NULL,
                    state TEXT NOT NULL
                  );
                  CREATE TABLE board_checkpoint_reconciliations (
                    checkpoint_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    work_item_id TEXT NOT NULL,
                    recommended_state TEXT NOT NULL,
                    decision TEXT NOT NULL,
                    previous_state TEXT NOT NULL,
                    resulting_state TEXT NOT NULL,
                    decided_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                  );
                  INSERT INTO board_projects VALUES ('core', 'Core', 'healthy', 1);
                  INSERT INTO board_work_items VALUES ('CORE-1', 'core', 'Work', 'in_review');
                  PRAGMA user_version = 2;",
            )
            .unwrap();
        let mut store = SqliteBoardStore::from_connection(connection).unwrap();

        let receipt = store
            .attach_checkpoint(&AttachmentRequest {
                checkpoint_id: "cp-v2".to_owned(),
                project_id: "core".to_owned(),
                checkpoint_work_item_id: None,
                target: AttachmentTarget::Existing {
                    work_item_id: "CORE-1".to_owned(),
                },
            })
            .unwrap();
        assert_eq!(receipt.attachment.work_item_id, "CORE-1");
        let version: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
        let priority: i64 = store
            .connection
            .query_row(
                "SELECT priority FROM board_work_items WHERE id = 'CORE-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(priority, 100);
    }

    fn activity(
        checkpoint_id: &str,
        project_id: &str,
        work_item_id: Option<&str>,
        recommended_state: Option<WorkItemState>,
    ) -> ProgressActivity {
        ProgressActivity {
            checkpoint_id: checkpoint_id.to_owned(),
            recorded_at: "2026-07-17T12:00:00+09:00".to_owned(),
            project_id: project_id.to_owned(),
            work_item_id: work_item_id.map(str::to_owned),
            source: CheckpointSource::Runner,
            outcome: CheckpointOutcome::Progress,
            summary: "Progress".to_owned(),
            recommended_state,
            deliveries: Vec::new(),
            attachment: None,
            reconciliation: None,
        }
    }
}
